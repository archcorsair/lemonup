use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, params};
use time::OffsetDateTime;

use crate::domain::{
    AddonKind, AddonRecord, GameFlavor, OwnedFolder, OwnershipSource, SourceKind,
    UnrecognizedValues,
};
use crate::error::{LemonupError, Result};
use crate::scan::{GroupingGuard, ScanReport, ScanSummary, ScannedAddon};
use crate::toc_edit::{self, TocField};

pub struct StateDatabase {
    connection: Connection,
}

const BOUND_ADDON_DIR_KEY: &str = "bound_addon_dir";
/// The `addons` schema this version creates and migrates to (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 3;

/// How long a connection waits for another process's write lock before failing, so a
/// TUI session and a scheduled `lemonup sync` overlap instead of erroring with
/// "database is locked".
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

impl StateDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;
        // Before migrate(): switching to WAL needs a lock too.
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let database = Self { connection };
        database.migrate()?;
        Ok(database)
    }

    /// Reads the bound AddOns folder from the database at `path` without creating or
    /// modifying anything: a missing file (or one without binding metadata) is `None`.
    pub fn read_bound_addon_dir(path: &Path) -> Result<Option<PathBuf>> {
        if !path.is_file() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let value = connection.query_row(
            "SELECT value FROM lemonup_meta WHERE key = ?1",
            [BOUND_ADDON_DIR_KEY],
            |row| row.get::<_, String>(0),
        );
        match value {
            Ok(value) => Ok(Some(PathBuf::from(value))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            // A database from before binding existed has no metadata table yet.
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("no such table") =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// How many addons (not libraries) the database at `path` lists, read without creating
    /// or modifying anything: a missing file, or one without the addons table, lists none.
    pub fn read_addon_count(path: &Path) -> Result<usize> {
        if !path.is_file() {
            return Ok(0);
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM addons WHERE kind != 'library'",
            [],
            |row| row.get::<_, i64>(0),
        );
        match count {
            Ok(count) => Ok(usize::try_from(count).unwrap_or(0)),
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("no such table") =>
            {
                Ok(0)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The addons the database at `path` lists, read without creating or modifying anything:
    /// a missing file, or one without the addons table, lists none.
    pub fn read_addons(path: &Path) -> Result<Vec<AddonRecord>> {
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        match Self::list_addons_from_connection(&connection) {
            Ok(addons) => Ok(addons),
            Err(LemonupError::Sqlite(rusqlite::Error::SqliteFailure(_, Some(message))))
                if message.contains("no such table") =>
            {
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }

    /// Milliseconds this connection waits on a locked database (test hook).
    #[cfg(test)]
    fn busy_timeout_millis(&self) -> i64 {
        self.connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("read busy_timeout")
    }

    pub fn list_addons(&self) -> Result<Vec<AddonRecord>> {
        Self::list_addons_from_connection(&self.connection)
    }

    /// The AddOns folder this state was built from, or `None` for a database created
    /// before folder binding existed (callers adopt the profile's folder on first use).
    pub fn bound_addon_dir(&self) -> Result<Option<PathBuf>> {
        let mut statement = self
            .connection
            .prepare("SELECT value FROM lemonup_meta WHERE key = ?1")?;
        let mut rows = statement.query([BOUND_ADDON_DIR_KEY])?;
        let value = rows
            .next()?
            .map(|row| row.get::<_, String>(0))
            .transpose()?;
        Ok(value.map(PathBuf::from))
    }

    /// Records `addon_dir` as the folder this state belongs to.
    pub fn bind_addon_dir(&self, addon_dir: &Path) -> Result<()> {
        let value = addon_dir.to_str().ok_or_else(|| {
            LemonupError::InvalidArgument(format!(
                "AddOns path is not valid UTF-8: {}",
                addon_dir.display()
            ))
        })?;
        self.connection.execute(
            "INSERT INTO lemonup_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![BOUND_ADDON_DIR_KEY, value],
        )?;
        Ok(())
    }

    /// Writes a consistent copy of the whole database (WAL included) to `path`, which must
    /// not exist yet. Used to run read-only work that would otherwise write rows.
    pub fn copy_to(&self, path: &Path) -> Result<()> {
        let target = path.to_str().ok_or_else(|| {
            LemonupError::InvalidArgument(format!(
                "database copy path is not valid UTF-8: {}",
                path.display()
            ))
        })?;
        self.connection.execute("VACUUM INTO ?1", [target])?;
        Ok(())
    }

    pub fn get_addon_by_folder(&self, folder: &str) -> Result<Option<AddonRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT
                id, name, folder, owned_folders, ownership_source, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version, toc_overrides
             FROM addons
             WHERE folder = ?1",
        )?;

        let mut rows = statement.query([folder])?;
        let maybe_row = rows.next()?;
        Ok(maybe_row.map(Self::row_to_addon).transpose()?)
    }

    /// Replaces the stored TOC overrides of the addon whose parent folder is `folder`. Values of
    /// known fields must pass `toc_edit::validate_override_value`; unknown keys are stored as
    /// they are. This changes only the database, not the addon's files.
    pub fn set_toc_overrides(
        &self,
        folder: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<()> {
        toc_edit::validate_stored_overrides(overrides)?;
        Self::write_toc_overrides_with_connection(&self.connection, folder, overrides)
    }

    /// The only write of the `toc_overrides` column on an existing row: upserts leave it alone,
    /// so a stale copy of a row never overwrites what the user set.
    fn write_toc_overrides_with_connection(
        connection: &Connection,
        folder: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<()> {
        let changed = connection.execute(
            "UPDATE addons SET toc_overrides = ?1 WHERE folder = ?2",
            params![serialize_toc_overrides(overrides)?, folder],
        )?;
        if changed == 0 {
            return Err(LemonupError::InvalidArgument(format!(
                "no addon is stored for folder '{folder}'"
            )));
        }
        Ok(())
    }

    /// Sets (`Some`) or clears (`None`) one TOC override of the addon whose parent folder is
    /// `folder` and returns all of its overrides. The value is validated and stored trimmed.
    /// This changes only the database, not the addon's files.
    pub fn set_toc_override(
        &self,
        folder: &str,
        field: TocField,
        value: Option<&str>,
    ) -> Result<BTreeMap<String, String>> {
        let addon = self.get_addon_by_folder(folder)?.ok_or_else(|| {
            LemonupError::InvalidArgument(format!("no addon is stored for folder '{folder}'"))
        })?;
        let mut overrides = addon.toc_overrides;
        toc_edit::set_override(&mut overrides, field, value)?;
        self.set_toc_overrides(folder, &overrides)?;
        Ok(overrides)
    }

    pub fn upsert_addon(&self, addon: &AddonRecord) -> Result<()> {
        Self::upsert_addon_with_connection(&self.connection, addon)
    }

    /// Writes what a check found online (the remote version, the source URL when it knows
    /// one, and when it checked) onto the stored row of `checked`, and nothing else of the
    /// row: a check waits for the network, and meanwhile another LemonUp may have updated
    /// or deleted the addon, which a whole-row write would undo. Returns the row as it is
    /// now, or `None` and writes nothing when the row is gone or no longer has the folder,
    /// source and installed version `checked` had.
    pub fn record_check_result(
        &self,
        checked: &AddonRecord,
        remote_version: Option<&str>,
        source_url: Option<&str>,
        checked_at: OffsetDateTime,
    ) -> Result<Option<AddonRecord>> {
        let changed = self.connection.execute(
            "UPDATE addons
             SET remote_version = ?1,
                 source_url = COALESCE(?2, source_url),
                 last_checked_at = ?3
             WHERE folder = ?4 AND source = ?5 AND version IS ?6",
            params![
                remote_version,
                source_url,
                checked_at.unix_timestamp(),
                checked.folder,
                serialize_source(checked.source),
                checked.version,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_addon_by_folder(&checked.folder)
    }

    /// The TOC overrides [`Self::record_managed_addon`] would store for `addon` right now: the
    /// stored row's (or, without one, `addon`'s own) plus those of the rows it absorbs. Installs
    /// and updates write these into the staged files, so the files and the row agree.
    pub fn managed_toc_overrides(&self, addon: &AddonRecord) -> Result<BTreeMap<String, String>> {
        let existing_addons = Self::list_addons_from_connection(&self.connection)?;
        let plan = ManagedRecordPlan::new(addon, &existing_addons);
        Ok(merge_toc_overrides(
            addon,
            plan.existing_parent,
            &plan.absorbed,
        ))
    }

    pub fn record_managed_addon(&mut self, addon: &AddonRecord) -> Result<()> {
        let tx = self.connection.transaction()?;
        Self::record_managed_addon_with_connection(&tx, addon)?;
        tx.commit()?;
        Ok(())
    }

    /// Records every addon of `addons` as [`Self::record_managed_addon`] does, in one
    /// transaction: if `guard` (given the rows inside the transaction) or any write fails,
    /// nothing is written.
    pub fn record_managed_addons(
        &mut self,
        addons: &[AddonRecord],
        guard: impl FnOnce(&[AddonRecord]) -> Result<()>,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        guard(&Self::list_addons_from_connection(&tx)?)?;
        for addon in addons {
            Self::record_managed_addon_with_connection(&tx, addon)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_managed_addon_with_connection(tx: &Connection, addon: &AddonRecord) -> Result<()> {
        let existing_addons = Self::list_addons_from_connection(tx)?;
        let ManagedRecordPlan {
            existing_parent,
            incoming_owned_folders,
            represented_folders,
            absorbed_folders,
            absorbed,
        } = ManagedRecordPlan::new(addon, &existing_addons);

        // Another addon linked by a newer LemonUp is never deleted or stripped here: its folders
        // are not this addon's to take (compared without case, like every ownership check).
        let taken = represented_folders
            .iter()
            .map(|folder| folder.to_lowercase())
            .collect::<HashSet<_>>();
        if let Some(newer) = existing_addons.iter().find(|existing| {
            existing.folder != addon.folder
                && existing.unrecognized.is_some()
                && std::iter::once(&existing.folder)
                    .chain(existing.owned_folders.iter().map(|owned| &owned.name))
                    .any(|folder| taken.contains(&folder.to_lowercase()))
        }) {
            return Err(LemonupError::InvalidArgument(format!(
                "{} can't take folders of {}, which a newer LemonUp linked",
                addon.folder, newer.folder
            )));
        }
        // Absorbing deletes a row, so a linked addon is never absorbed, and a linked addon never
        // loses its folders to this one.
        let parent_key = addon.folder.to_lowercase();
        let lists_parent = |existing: &AddonRecord| {
            existing
                .owned_folders
                .iter()
                .any(|owned| owned.name.to_lowercase() == parent_key)
        };
        if let Some(linked) = absorbed
            .iter()
            .find(|existing| existing.holds_its_folders())
            .copied()
            .or_else(|| {
                existing_addons.iter().find(|existing| {
                    existing.folder != addon.folder
                        && existing.holds_its_folders()
                        && lists_parent(existing)
                })
            })
        {
            return Err(LemonupError::InvalidArgument(format!(
                "{} can't take folders of {}, which is linked",
                addon.folder, linked.folder
            )));
        }

        // Every folder this addon now holds, its own included, leaves the other rows.
        for mut existing in existing_addons
            .iter()
            .filter(|existing| existing.folder != addon.folder)
            .cloned()
        {
            let filtered = existing
                .owned_folders
                .iter()
                .filter(|owned_folder| {
                    !incoming_owned_folders.contains(&owned_folder.name)
                        && owned_folder.name.to_lowercase() != parent_key
                })
                .cloned()
                .collect::<Vec<_>>();
            if filtered.len() != existing.owned_folders.len() {
                if filtered.is_empty() {
                    existing.clear_owned_folders();
                } else {
                    existing.owned_folders = filtered;
                }
                Self::upsert_addon_with_connection(tx, &existing)?;
            }
        }

        for folder in &absorbed_folders {
            Self::remove_addon_with_connection(tx, folder)?;
        }

        let merged = merge_managed_addon(addon, existing_parent, &absorbed);
        Self::upsert_addon_with_connection(tx, &merged)?;
        if existing_parent.is_some_and(|parent| parent.toc_overrides != merged.toc_overrides) {
            Self::write_toc_overrides_with_connection(tx, &merged.folder, &merged.toc_overrides)?;
        }
        Ok(())
    }

    pub fn reconcile_scanned_addons(
        &mut self,
        scanned_addons: &[ScannedAddon],
    ) -> Result<ScanSummary> {
        self.reconcile_scanned_addons_skipping(scanned_addons, &[])
    }

    /// Reconciles a scan report, treating its skipped (unreadable) folders as present.
    pub fn reconcile_scan(&mut self, report: &ScanReport) -> Result<ScanSummary> {
        let mut summary =
            self.reconcile_scanned_addons_skipping(&report.addons, &report.skipped_folders)?;
        summary.leftover_staging = report.leftover_staging.clone();
        Ok(summary)
    }

    /// `skipped_folders` are on disk but could not be read: any existing row for them is kept
    /// exactly as it is (no field updates, no deletion, authoritative children left alone) and no
    /// row is created for one that has none.
    fn reconcile_scanned_addons_skipping(
        &mut self,
        scanned_addons: &[ScannedAddon],
        skipped_folders: &[String],
    ) -> Result<ScanSummary> {
        let skipped = skipped_folders.iter().cloned().collect::<HashSet<_>>();
        let tx = self.connection.transaction()?;
        let existing_addons = Self::list_addons_from_connection(&tx)?;
        let existing_by_folder = existing_addons
            .iter()
            .map(|addon| (addon.folder.clone(), addon.clone()))
            .collect::<HashMap<_, _>>();
        let existing_folder_names = existing_addons
            .iter()
            .map(|addon| addon.folder.clone())
            .collect::<HashSet<_>>();
        let live_scanned_folders = scanned_addons
            .iter()
            .map(|addon| addon.folder.clone())
            .chain(skipped.iter().cloned())
            .collect::<HashSet<_>>();
        let active_authoritative_parents = existing_addons
            .iter()
            .filter(|addon| {
                addon.has_authoritative_owned_folders()
                    && live_scanned_folders.contains(&addon.folder)
            })
            .map(|addon| addon.folder.clone())
            .collect::<HashSet<_>>();
        let authoritative_owned_children = existing_addons
            .iter()
            .filter(|addon| active_authoritative_parents.contains(&addon.folder))
            .flat_map(|addon| addon.owned_folders.iter().map(|child| child.name.clone()))
            .collect::<HashSet<_>>();
        // An addon whose owned folders a newer LemonUp assigned in a way this version can't read
        // keeps its stored list, and its children stay listed under it as they were stored. Only
        // folders without a row of their own or with a plain "Not linked" one are listed there;
        // nothing is deleted from disk or taken from a linked addon on the strength of it.
        let unrecognized_ownership_parents = existing_addons
            .iter()
            .filter(|addon| {
                addon.has_unrecognized_ownership() && live_scanned_folders.contains(&addon.folder)
            })
            .map(|addon| addon.folder.clone())
            .collect::<HashSet<_>>();
        let unrecognized_ownership_children = existing_addons
            .iter()
            .filter(|addon| unrecognized_ownership_parents.contains(&addon.folder))
            .flat_map(|addon| addon.owned_folders.iter().map(|child| child.name.clone()))
            .filter(|child| {
                existing_by_folder.get(child).is_none_or(|row| {
                    row.source == SourceKind::Manual && row.unrecognized.is_none()
                })
            })
            .collect::<HashSet<_>>();
        // Folders the scan saw on disk, whether as top-level entries or grouped children.
        let on_disk_folders = scanned_addons
            .iter()
            .flat_map(|addon| {
                std::iter::once(addon.folder.clone())
                    .chain(addon.owned_folders.iter().map(|owned| owned.name.clone()))
            })
            .chain(skipped.iter().cloned())
            .collect::<HashSet<_>>();
        // A tracked non-manual row whose folder is still on disk is never deleted here: at worst
        // it is kept as its own top-level row. A row from a newer LemonUp counts as tracked.
        let protected_rows = existing_addons
            .iter()
            .filter(|addon| {
                (addon.source != SourceKind::Manual || addon.unrecognized.is_some())
                    && on_disk_folders.contains(&addon.folder)
            })
            .map(|addon| addon.folder.clone())
            .collect::<HashSet<_>>();
        let grouping_guard = GroupingGuard::from_tracked(&existing_addons);
        let mut visible_parent_folders = HashSet::new();
        let mut represented_child_folders = HashSet::new();

        for scanned_addon in scanned_addons {
            if authoritative_owned_children.contains(&scanned_addon.folder)
                || unrecognized_ownership_children.contains(&scanned_addon.folder)
            {
                continue;
            }

            let preserve_owned_folders = active_authoritative_parents
                .contains(&scanned_addon.folder)
                || unrecognized_ownership_parents.contains(&scanned_addon.folder);
            // Defense in depth for scans grouped without tracked-row knowledge: drop inferred
            // children that the tracked rows say must stay standalone.
            let mut guarded_addon = scanned_addon.clone();
            if !preserve_owned_folders {
                guarded_addon
                    .owned_folders
                    .retain(|owned| grouping_guard.may_group(&scanned_addon.folder, &owned.name));
            }

            let merged = merge_scanned_addon(
                &guarded_addon,
                existing_by_folder.get(&scanned_addon.folder),
                preserve_owned_folders,
            );
            visible_parent_folders.insert(merged.folder.clone());
            represented_child_folders.extend(
                merged
                    .owned_folders
                    .iter()
                    .map(|owned_folder| owned_folder.name.clone()),
            );
            Self::upsert_addon_with_connection(&tx, &merged)?;
        }

        let stale_folders = existing_addons
            .into_iter()
            .filter(|addon| {
                !visible_parent_folders.contains(&addon.folder)
                    && !skipped.contains(&addon.folder)
                    && !represented_child_folders.contains(&addon.folder)
            })
            .map(|addon| addon.folder)
            .collect::<Vec<_>>();
        let represented_child_rows = represented_child_folders
            .iter()
            .filter(|folder| !visible_parent_folders.contains(*folder))
            .cloned()
            .collect::<Vec<_>>();
        let removed_folders = stale_folders
            .iter()
            .cloned()
            .chain(represented_child_rows.iter().cloned())
            .filter(|folder| !protected_rows.contains(folder))
            .collect::<HashSet<_>>();
        let removed_existing_rows = removed_folders
            .iter()
            .filter(|folder| existing_folder_names.contains(*folder))
            .count();

        for folder in &removed_folders {
            Self::remove_addon_with_connection(&tx, folder)?;
        }

        tx.commit()?;

        Ok(ScanSummary {
            scanned_addons: scanned_addons.len(),
            upserted_addons: visible_parent_folders.len(),
            removed_addons: removed_existing_rows,
            skipped_folders: skipped_folders.to_vec(),
            leftover_staging: Vec::new(),
        })
    }

    pub fn remove_addon(&self, folder: &str) -> Result<()> {
        let existing_addons = Self::list_addons_from_connection(&self.connection)?;
        let descendants = collect_owned_descendants(&existing_addons, folder, true);
        let removed_folders = std::iter::once(folder.to_string())
            .chain(descendants)
            .collect::<HashSet<_>>();

        for removed_folder in &removed_folders {
            Self::remove_addon_with_connection(&self.connection, removed_folder)?;
        }

        for mut addon in Self::list_addons_from_connection(&self.connection)? {
            let filtered = addon
                .owned_folders
                .iter()
                .filter(|owned_folder| !removed_folders.contains(&owned_folder.name))
                .cloned()
                .collect::<Vec<_>>();
            if filtered.len() != addon.owned_folders.len() {
                addon.owned_folders = filtered;
                Self::upsert_addon_with_connection(&self.connection, &addon)?;
            }
        }

        Ok(())
    }

    pub fn planned_removal_folders(&self, folder: &str) -> Result<Vec<String>> {
        let existing_addons = Self::list_addons_from_connection(&self.connection)?;
        let mut planned = std::iter::once(folder.to_string())
            .chain(collect_owned_descendants(&existing_addons, folder, true))
            .collect::<Vec<_>>();
        planned.sort();
        planned.dedup();
        Ok(planned)
    }

    fn migrate(&self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA journal_mode = WAL;")?;
        // Key/value metadata (the bound AddOns folder). Added without a version bump so
        // older databases gain it in place and older builds simply ignore it.
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS lemonup_meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );",
        )?;
        if Self::user_version(&self.connection)? >= SCHEMA_VERSION {
            return Ok(());
        }
        // One transaction for the whole schema change, so a crash or a failed step leaves the
        // database as it was. IMMEDIATE takes the write lock before the version is read again,
        // so two processes opening an old database never both migrate it.
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        Self::migrate_with_connection(&tx)?;
        tx.commit()?;
        Ok(())
    }

    fn user_version(connection: &Connection) -> Result<i64> {
        Ok(connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?)
    }

    fn migrate_with_connection(connection: &Connection) -> Result<()> {
        let user_version = Self::user_version(connection)?;

        if user_version == 0 {
            connection.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS addons (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    folder TEXT NOT NULL UNIQUE,
                    owned_folders TEXT NOT NULL,
                    ownership_source TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    kind_override INTEGER NOT NULL,
                    flavor TEXT NOT NULL,
                    version TEXT,
                    git_commit TEXT,
                    author TEXT,
                    interface TEXT,
                    source TEXT NOT NULL,
                    source_url TEXT,
                    required_deps TEXT NOT NULL,
                    optional_deps TEXT NOT NULL,
                    embedded_libs TEXT NOT NULL,
                    installed_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    last_checked_at INTEGER,
                    remote_version TEXT,
                    toc_overrides TEXT
                );
                PRAGMA user_version = 3;
                ",
            )?;
            return Ok(());
        }

        if user_version == 1 {
            connection.execute_batch(
                "
                ALTER TABLE addons ADD COLUMN ownership_source TEXT NOT NULL DEFAULT 'none';
                UPDATE addons
                SET ownership_source = CASE
                    WHEN owned_folders = '[]' THEN 'none'
                    ELSE 'scan_inferred'
                END
                WHERE ownership_source = 'none';
                PRAGMA user_version = 2;
                ",
            )?;
        }

        if user_version == 1 || user_version == 2 {
            connection.execute_batch(
                "
                ALTER TABLE addons ADD COLUMN toc_overrides TEXT;
                PRAGMA user_version = 3;
                ",
            )?;
        }
        Ok(())
    }

    fn list_addons_from_connection(connection: &Connection) -> Result<Vec<AddonRecord>> {
        let mut statement = connection.prepare(
            "SELECT
                id, name, folder, owned_folders, ownership_source, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version, toc_overrides
             FROM addons
             ORDER BY name ASC",
        )?;

        let rows = statement.query_map([], Self::row_to_addon)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn upsert_addon_with_connection(connection: &Connection, addon: &AddonRecord) -> Result<()> {
        let owned_folders = serde_json::to_string(&addon.owned_folders)?;
        // Values a newer LemonUp stored are written back unchanged while the addon stays not
        // linked here; linking it to a source in this version replaces them.
        let kept = addon
            .unrecognized
            .as_ref()
            .filter(|_| addon.source == SourceKind::Manual);
        let ownership_source = kept
            .and_then(|kept| kept.ownership_source.as_deref())
            .unwrap_or_else(|| serialize_ownership_source(addon.effective_ownership_source()));
        let kind = kept
            .and_then(|kept| kept.kind.as_deref())
            .unwrap_or_else(|| serialize_addon_kind(addon.kind));
        let flavor = kept
            .and_then(|kept| kept.flavor.as_deref())
            .unwrap_or_else(|| serialize_flavor(addon.flavor));
        let source = kept.map_or_else(|| serialize_source(addon.source), |kept| &kept.source);
        let required_deps = serde_json::to_string(&addon.required_deps)?;
        let optional_deps = serde_json::to_string(&addon.optional_deps)?;
        let embedded_libs = serde_json::to_string(&addon.embedded_libs)?;
        let toc_overrides = serialize_toc_overrides(&addon.toc_overrides)?;

        connection.execute(
            "INSERT INTO addons (
                name, folder, owned_folders, ownership_source, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version, toc_overrides
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
             ON CONFLICT(folder) DO UPDATE SET
                name = excluded.name,
                owned_folders = excluded.owned_folders,
                ownership_source = excluded.ownership_source,
                kind = excluded.kind,
                kind_override = excluded.kind_override,
                flavor = excluded.flavor,
                version = excluded.version,
                git_commit = excluded.git_commit,
                author = excluded.author,
                interface = excluded.interface,
                source = excluded.source,
                source_url = excluded.source_url,
                required_deps = excluded.required_deps,
                optional_deps = excluded.optional_deps,
                embedded_libs = excluded.embedded_libs,
                installed_at = excluded.installed_at,
                updated_at = excluded.updated_at,
                last_checked_at = excluded.last_checked_at,
                remote_version = excluded.remote_version",
            params![
                addon.name,
                addon.folder,
                owned_folders,
                ownership_source,
                kind,
                addon.kind_override,
                flavor,
                addon.version,
                addon.git_commit,
                addon.author,
                addon.interface,
                source,
                addon.source_url,
                required_deps,
                optional_deps,
                embedded_libs,
                addon.installed_at.unix_timestamp(),
                addon.updated_at.unix_timestamp(),
                addon.last_checked_at.map(|value| value.unix_timestamp()),
                addon.remote_version,
                toc_overrides,
            ],
        )?;

        Ok(())
    }

    fn remove_addon_with_connection(connection: &Connection, folder: &str) -> Result<()> {
        connection.execute("DELETE FROM addons WHERE folder = ?1", [folder])?;
        Ok(())
    }

    fn row_to_addon(row: &rusqlite::Row<'_>) -> rusqlite::Result<AddonRecord> {
        let owned_folders = parse_json::<Vec<OwnedFolder>>(row.get::<_, String>(3)?)?;
        let required_deps = parse_json::<Vec<String>>(row.get::<_, String>(14)?)?;
        let optional_deps = parse_json::<Vec<String>>(row.get::<_, String>(15)?)?;
        let embedded_libs = parse_json::<Vec<String>>(row.get::<_, String>(16)?)?;
        let stored_ownership_source = row.get::<_, String>(4)?;
        let stored_kind = row.get::<_, String>(5)?;
        let stored_flavor = row.get::<_, String>(7)?;
        let stored_source = row.get::<_, String>(12)?;
        let ownership_source = parse_ownership_source(&stored_ownership_source);
        let kind = parse_addon_kind(&stored_kind);
        let flavor = parse_flavor(&stored_flavor);
        let source = parse_source(&stored_source);
        let unrecognized = UnrecognizedValues {
            source: stored_source.clone(),
            flavor: unrecognized_value("flavor", flavor, &stored_flavor),
            kind: unrecognized_value("kind", kind, &stored_kind),
            ownership_source: unrecognized_value(
                "ownership_source",
                ownership_source,
                &stored_ownership_source,
            ),
        };
        let any_unrecognized = unrecognized_value("source", source, &stored_source).is_some()
            || unrecognized.flavor.is_some()
            || unrecognized.kind.is_some()
            || unrecognized.ownership_source.is_some();

        // Fallbacks for values a newer LemonUp wrote: the addon is not linked (never checked or
        // updated), listed as an addon rather than hidden as a library, and its owned folders are
        // not authoritative, so nothing is grouped or deleted on the strength of them. The
        // placeholder flavor is never acted on because the addon is not linked.
        Ok(AddonRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            folder: row.get(2)?,
            owned_folders,
            ownership_source: ownership_source.unwrap_or(OwnershipSource::None),
            kind: kind.unwrap_or(AddonKind::Addon),
            kind_override: row.get(6)?,
            flavor: flavor.unwrap_or(GameFlavor::Retail),
            version: row.get(8)?,
            git_commit: row.get(9)?,
            author: row.get(10)?,
            interface: row.get(11)?,
            source: match source {
                Some(source) if !any_unrecognized => source,
                _ => SourceKind::Manual,
            },
            source_url: row.get(13)?,
            required_deps,
            optional_deps,
            embedded_libs,
            installed_at: parse_timestamp(row.get::<_, i64>(17)?)?,
            updated_at: parse_timestamp(row.get::<_, i64>(18)?)?,
            last_checked_at: row
                .get::<_, Option<i64>>(19)?
                .map(parse_timestamp)
                .transpose()?,
            remote_version: row.get(20)?,
            toc_overrides: row
                .get::<_, Option<String>>(21)?
                .map(parse_json::<BTreeMap<String, String>>)
                .transpose()?
                .unwrap_or_default(),
            unrecognized: any_unrecognized.then_some(unrecognized),
        })
    }
}

fn merge_scanned_addon(
    scanned_addon: &ScannedAddon,
    existing: Option<&AddonRecord>,
    preserve_existing_owned_folders: bool,
) -> AddonRecord {
    let now = OffsetDateTime::now_utc();
    let kind_override = existing.is_some_and(|addon| addon.kind_override);
    let merged_source = merge_scanned_source(scanned_addon, existing);
    let ownership_source = if preserve_existing_owned_folders {
        existing
            .expect("preserving owned folders requires existing addon")
            .effective_ownership_source()
    } else if scanned_addon.owned_folders.is_empty() {
        OwnershipSource::None
    } else {
        OwnershipSource::ScanInferred
    };

    let toc_overrides = existing
        .map(|addon| addon.toc_overrides.clone())
        .unwrap_or_default();
    AddonRecord {
        id: existing.and_then(|addon| addon.id),
        // An addon with a Title override is listed under it, the same as after an update.
        name: toc_edit::title_override_name(&toc_overrides)
            .unwrap_or_else(|| scanned_addon.name.clone()),
        folder: scanned_addon.folder.clone(),
        owned_folders: if preserve_existing_owned_folders {
            existing
                .expect("preserving owned folders requires existing addon")
                .owned_folders
                .clone()
        } else {
            scanned_addon.owned_folders.clone()
        },
        ownership_source,
        kind: if kind_override {
            existing
                .expect("kind override requires existing addon")
                .kind
        } else {
            scanned_addon.kind
        },
        kind_override,
        flavor: scanned_addon.flavor,
        version: merge_scanned_version(scanned_addon, existing, merged_source),
        git_commit: scanned_addon
            .git_commit
            .clone()
            .or_else(|| existing.and_then(|addon| addon.git_commit.clone())),
        author: scanned_addon.author.clone(),
        interface: scanned_addon.interface.clone(),
        source: merged_source,
        source_url: existing.and_then(|addon| addon.source_url.clone()),
        required_deps: scanned_addon.required_deps.clone(),
        optional_deps: scanned_addon.optional_deps.clone(),
        embedded_libs: scanned_addon.embedded_libs.clone(),
        installed_at: existing.map(|addon| addon.installed_at).unwrap_or(now),
        updated_at: now,
        last_checked_at: existing.and_then(|addon| addon.last_checked_at),
        remote_version: existing.and_then(|addon| addon.remote_version.clone()),
        toc_overrides,
        unrecognized: existing.and_then(|addon| addon.unrecognized.clone()),
    }
}

/// A tracked GitHub row without a source URL cannot be updated (`update-all` would fail on it
/// every run). When the scan finds a `.git` checkout there (a development clone that an older
/// scan mislabeled as GitHub), the row is demoted to manual. Every other tracked row keeps its
/// source.
fn merge_scanned_source(
    scanned_addon: &ScannedAddon,
    existing: Option<&AddonRecord>,
) -> SourceKind {
    let Some(existing) = existing else {
        return scanned_addon.source;
    };
    let missing_url = existing
        .source_url
        .as_deref()
        .is_none_or(|url| url.trim().is_empty());
    if existing.source == SourceKind::GitHub && missing_url && scanned_addon.git_commit.is_some() {
        return SourceKind::Manual;
    }
    existing.source
}

/// Picks the version to store when a scan sees an already-known folder.
///
/// Provider-tracked rows (Wago, WoWInterface, Tukui) keep the version recorded at install/update
/// time: that label is what the provider's remote version is compared against, while the TOC
/// version often differs in shape (`Pawn 2.14.3` vs TOC `2.14.3`, `v3.4.1` vs `12.0.5`) or is
/// missing. Overwriting it on every sync made such addons look permanently outdated or
/// "remote older". The TOC version only fills in a row that has no recorded version.
///
/// Rows from a newer LemonUp keep their recorded version too, so the link it restores on upgrade
/// compares against the label it recorded.
///
/// Manual rows always take the TOC version. GitHub rows track by commit (`git_commit`, falling
/// back to `version`), so they keep taking the scanned version, except for unexpanded packager
/// placeholders such as `@project-version@`, which never replace a recorded one.
fn merge_scanned_version(
    scanned_addon: &ScannedAddon,
    existing: Option<&AddonRecord>,
    source: SourceKind,
) -> Option<String> {
    let recorded = existing
        .and_then(|addon| addon.version.clone())
        .filter(|version| !version.trim().is_empty());
    if existing.is_some_and(|addon| addon.unrecognized.is_some()) {
        return recorded.or_else(|| scanned_addon.version.clone());
    }

    match source {
        SourceKind::Tukui | SourceKind::WowInterface | SourceKind::Wago => {
            recorded.or_else(|| scanned_addon.version.clone())
        }
        SourceKind::GitHub
            if scanned_addon
                .version
                .as_deref()
                .is_some_and(is_placeholder_version) =>
        {
            recorded.or_else(|| scanned_addon.version.clone())
        }
        SourceKind::GitHub | SourceKind::Manual => scanned_addon.version.clone(),
    }
}

fn is_placeholder_version(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('@') && trimmed.ends_with('@')
}

/// What [`StateDatabase::record_managed_addon`] reads from the stored rows before it writes.
struct ManagedRecordPlan<'a> {
    existing_parent: Option<&'a AddonRecord>,
    incoming_owned_folders: HashSet<String>,
    represented_folders: HashSet<String>,
    /// Folders the addon now owns whose own rows are removed, sorted so the result does not
    /// depend on hash order.
    absorbed_folders: Vec<String>,
    /// The stored rows of `absorbed_folders`.
    absorbed: Vec<&'a AddonRecord>,
}

impl<'a> ManagedRecordPlan<'a> {
    fn new(addon: &AddonRecord, existing_addons: &'a [AddonRecord]) -> Self {
        let existing_parent = existing_addons
            .iter()
            .find(|existing| existing.folder == addon.folder);
        let incoming_owned_folders = addon
            .owned_folders
            .iter()
            .map(|owned_folder| owned_folder.name.clone())
            .collect::<HashSet<_>>();
        let prior_owned_folders = existing_parent
            .iter()
            .flat_map(|existing| {
                existing
                    .owned_folders
                    .iter()
                    .map(|owned_folder| owned_folder.name.clone())
            })
            .collect::<HashSet<_>>();
        let represented_folders = incoming_owned_folders
            .union(&prior_owned_folders)
            .cloned()
            .collect::<HashSet<_>>();
        // Rows of folders this addon now owns are absorbed into it.
        let mut absorbed_folders = represented_folders
            .iter()
            .filter(|folder| **folder != addon.folder)
            .cloned()
            .collect::<Vec<_>>();
        absorbed_folders.sort();
        let absorbed = absorbed_folders
            .iter()
            .filter_map(|folder| {
                existing_addons
                    .iter()
                    .find(|existing| existing.folder == *folder)
            })
            .collect::<Vec<_>>();
        Self {
            existing_parent,
            incoming_owned_folders,
            represented_folders,
            absorbed_folders,
            absorbed,
        }
    }
}

/// The TOC overrides a managed addon ends up with. A stored row keeps its overrides (the upsert
/// never writes them); a new row starts with the incoming record's, which is how an undone
/// removal gets them back. `absorbed` are the rows of folders this addon now owns, which are
/// removed: their overrides carry over for fields the addon does not set itself.
fn merge_toc_overrides(
    incoming: &AddonRecord,
    existing: Option<&AddonRecord>,
    absorbed: &[&AddonRecord],
) -> BTreeMap<String, String> {
    let mut toc_overrides = existing.map_or_else(
        || incoming.toc_overrides.clone(),
        |addon| addon.toc_overrides.clone(),
    );
    for child in absorbed {
        for (field, value) in &child.toc_overrides {
            toc_overrides
                .entry(field.clone())
                .or_insert_with(|| value.clone());
        }
    }
    toc_overrides
}

/// `absorbed` are the rows of folders this addon now owns, which are removed; see
/// [`merge_toc_overrides`] for what happens to their TOC overrides.
fn merge_managed_addon(
    incoming: &AddonRecord,
    existing: Option<&AddonRecord>,
    absorbed: &[&AddonRecord],
) -> AddonRecord {
    let now = OffsetDateTime::now_utc();
    let preserve_kind_override = existing.is_some_and(|addon| addon.kind_override);
    let toc_overrides = merge_toc_overrides(incoming, existing, absorbed);
    let mut merged = AddonRecord {
        id: existing.and_then(|addon| addon.id),
        // An addon with a Title override is listed under it, the same as after a scan.
        name: toc_edit::title_override_name(&toc_overrides)
            .unwrap_or_else(|| incoming.name.clone()),
        folder: incoming.folder.clone(),
        owned_folders: incoming.owned_folders.clone(),
        ownership_source: incoming.effective_ownership_source(),
        kind: if preserve_kind_override {
            existing
                .expect("kind override requires existing addon")
                .kind
        } else {
            incoming.kind
        },
        kind_override: incoming.kind_override || preserve_kind_override,
        flavor: incoming.flavor,
        version: incoming.version.clone(),
        git_commit: incoming.git_commit.clone(),
        author: incoming.author.clone(),
        interface: incoming.interface.clone(),
        source: incoming.source,
        source_url: incoming
            .source_url
            .clone()
            .or_else(|| existing.and_then(|addon| addon.source_url.clone())),
        required_deps: incoming.required_deps.clone(),
        optional_deps: incoming.optional_deps.clone(),
        embedded_libs: incoming.embedded_libs.clone(),
        installed_at: existing
            .map(|addon| addon.installed_at)
            .unwrap_or(incoming.installed_at),
        updated_at: now,
        last_checked_at: incoming
            .last_checked_at
            .or_else(|| existing.and_then(|addon| addon.last_checked_at)),
        remote_version: incoming
            .remote_version
            .clone()
            .or_else(|| existing.and_then(|addon| addon.remote_version.clone())),
        toc_overrides,
        unrecognized: incoming
            .unrecognized
            .clone()
            .or_else(|| existing.and_then(|addon| addon.unrecognized.clone())),
    };

    merged.ownership_source = OwnershipSource::Managed;

    merged
}

fn collect_owned_descendants(
    existing_addons: &[AddonRecord],
    folder: &str,
    authoritative_only: bool,
) -> HashSet<String> {
    let owned_by_folder = existing_addons
        .iter()
        .filter(|addon| !authoritative_only || addon.has_authoritative_owned_folders())
        .map(|addon| {
            (
                addon.folder.clone(),
                addon
                    .owned_folders
                    .iter()
                    .map(|owned_folder| owned_folder.name.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut descendants = HashSet::new();
    let mut queue = vec![folder.to_string()];

    while let Some(current) = queue.pop() {
        if let Some(children) = owned_by_folder.get(&current) {
            for child in children {
                if descendants.insert(child.clone()) {
                    queue.push(child.clone());
                }
            }
        }
    }

    descendants
}

/// No overrides are stored as NULL, so rows without any look the same as before the column.
fn serialize_toc_overrides(overrides: &BTreeMap<String, String>) -> Result<Option<String>> {
    if overrides.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::to_string(overrides)?))
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn parse_timestamp(value: i64) -> rusqlite::Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            std::mem::size_of::<i64>(),
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn parse_source(value: &str) -> Option<SourceKind> {
    match value {
        "github" => Some(SourceKind::GitHub),
        "tukui" => Some(SourceKind::Tukui),
        "wowinterface" => Some(SourceKind::WowInterface),
        "wago" => Some(SourceKind::Wago),
        "manual" => Some(SourceKind::Manual),
        _ => None,
    }
}

fn parse_ownership_source(value: &str) -> Option<OwnershipSource> {
    match value {
        "none" => Some(OwnershipSource::None),
        "scan_inferred" => Some(OwnershipSource::ScanInferred),
        "managed" => Some(OwnershipSource::Managed),
        _ => None,
    }
}

fn parse_flavor(value: &str) -> Option<GameFlavor> {
    match value {
        "retail" => Some(GameFlavor::Retail),
        "classic" => Some(GameFlavor::Classic),
        "cata" => Some(GameFlavor::Cata),
        "forever" => Some(GameFlavor::Forever),
        _ => None,
    }
}

fn parse_addon_kind(value: &str) -> Option<AddonKind> {
    match value {
        "addon" => Some(AddonKind::Addon),
        "library" => Some(AddonKind::Library),
        _ => None,
    }
}

/// The stored value when this version could not parse it, warning once per column and value
/// for the life of the process (the addon list is reloaded often).
fn unrecognized_value<T>(column: &'static str, parsed: Option<T>, stored: &str) -> Option<String> {
    if parsed.is_some() {
        return None;
    }
    static WARNED: OnceLock<Mutex<HashSet<(&'static str, String)>>> = OnceLock::new();
    let first_time = WARNED
        .get_or_init(Default::default)
        .lock()
        .map(|mut warned| warned.insert((column, stored.to_string())))
        .unwrap_or(false);
    if first_time {
        tracing::info!(
            column,
            value = stored,
            "state database has a value this LemonUp version does not know (written by a newer \
             version); the addon shows as Not linked and the value is kept"
        );
    }
    Some(stored.to_string())
}

fn serialize_source(value: SourceKind) -> &'static str {
    match value {
        SourceKind::GitHub => "github",
        SourceKind::Tukui => "tukui",
        SourceKind::WowInterface => "wowinterface",
        SourceKind::Wago => "wago",
        SourceKind::Manual => "manual",
    }
}

fn serialize_ownership_source(value: OwnershipSource) -> &'static str {
    match value {
        OwnershipSource::None => "none",
        OwnershipSource::ScanInferred => "scan_inferred",
        OwnershipSource::Managed => "managed",
    }
}

fn serialize_flavor(value: GameFlavor) -> &'static str {
    match value {
        GameFlavor::Retail => "retail",
        GameFlavor::Classic => "classic",
        GameFlavor::Cata => "cata",
        GameFlavor::Forever => "forever",
    }
}

fn serialize_addon_kind(value: AddonKind) -> &'static str {
    match value {
        AddonKind::Addon => "addon",
        AddonKind::Library => "library",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use time::OffsetDateTime;

    use crate::domain::{AddonRecord, OwnedFolder, SourceKind};
    use crate::scan::ScannedAddon;

    use super::StateDatabase;
    use crate::domain::{AddonKind, GameFlavor};

    #[test]
    fn read_addons_lists_without_creating_anything() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        assert!(
            StateDatabase::read_addons(&path)
                .expect("missing")
                .is_empty()
        );
        assert!(!path.exists(), "reading must not create the database");

        let database = StateDatabase::open(&path).expect("open");
        database
            .upsert_addon(&AddonRecord::new("Bagnon", "Bagnon", SourceKind::Wago))
            .expect("upsert");
        drop(database);
        let addons = StateDatabase::read_addons(&path).expect("read");
        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].folder, "Bagnon");
    }
    use crate::toc_edit::TocField;

    #[test]
    fn open_sets_a_busy_timeout_so_concurrent_writers_wait() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        let database = StateDatabase::open(&path).expect("open db");
        assert_eq!(database.busy_timeout_millis(), 5_000);

        // A second connection holds the write lock briefly; a bind issued meanwhile waits
        // for it instead of failing with "database is locked".
        let blocker = rusqlite::Connection::open(&path).expect("open blocker");
        blocker
            .execute_batch("BEGIN IMMEDIATE;")
            .expect("take write lock");
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            blocker.execute_batch("COMMIT;").expect("release lock");
        });
        database
            .bind_addon_dir(temp.path())
            .expect("bind waits for the lock");
        release.join().expect("blocker thread");
    }

    #[test]
    fn read_bound_addon_dir_never_creates_or_changes_the_database() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("data").join("state.sqlite");

        // Missing database: None, and nothing created.
        assert_eq!(
            StateDatabase::read_bound_addon_dir(&path).expect("read"),
            None
        );
        assert!(!path.exists());
        assert!(!temp.path().join("data").exists());

        // Existing database without a binding, then with one.
        let database = StateDatabase::open(&path).expect("open db");
        assert_eq!(
            StateDatabase::read_bound_addon_dir(&path).expect("read"),
            None
        );
        database.bind_addon_dir(temp.path()).expect("bind");
        assert_eq!(
            StateDatabase::read_bound_addon_dir(&path).expect("read"),
            Some(temp.path().to_path_buf())
        );

        // A database from before binding existed has no metadata table at all.
        let legacy = temp.path().join("legacy.sqlite");
        rusqlite::Connection::open(&legacy)
            .expect("legacy db")
            .execute_batch("CREATE TABLE addons (id INTEGER);")
            .expect("legacy schema");
        assert_eq!(
            StateDatabase::read_bound_addon_dir(&legacy).expect("read"),
            None
        );
    }

    #[test]
    fn upserts_and_lists_addons() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        addon.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
        ]);
        addon.required_deps = vec!["LibStub".to_string()];
        database.upsert_addon(&addon).expect("upsert");

        let stored = database.list_addons().expect("list");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "ElvUI");
        assert_eq!(stored[0].owned_folders.len(), 2);
        assert!(stored[0].has_authoritative_owned_folders());
        assert_eq!(stored[0].required_deps, vec!["LibStub".to_string()]);
    }

    #[test]
    fn round_trips_forever_flavor() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        addon.flavor = GameFlavor::Forever;
        database.upsert_addon(&addon).expect("upsert");

        let stored = database.list_addons().expect("list");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].flavor, GameFlavor::Forever);
    }

    #[test]
    fn reconcile_preserves_tracked_metadata_and_removes_stale_rows() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let installed_at =
            OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("installed_at");
        let checked_at = OffsetDateTime::from_unix_timestamp(1_700_000_123).expect("checked_at");

        let mut details = AddonRecord::new("Details", "Details", SourceKind::Wago);
        details.installed_at = installed_at;
        details.updated_at = installed_at;
        details.source_url = Some("https://addons.wago.io/addons/details".to_string());
        details.remote_version = Some("11.0.0".to_string());
        details.last_checked_at = Some(checked_at);
        database.upsert_addon(&details).expect("seed details");

        let stale_owned = AddonRecord::new(
            "Details DataStorage",
            "Details_DataStorage",
            SourceKind::Manual,
        );
        database
            .upsert_addon(&stale_owned)
            .expect("seed stale owned");

        let stale_missing = AddonRecord::new("OldAddon", "OldAddon", SourceKind::Manual);
        database
            .upsert_addon(&stale_missing)
            .expect("seed stale missing");

        let mut overridden = AddonRecord::new("CustomLib", "CustomLib", SourceKind::Manual);
        overridden.kind = AddonKind::Addon;
        overridden.kind_override = true;
        database.upsert_addon(&overridden).expect("seed overridden");

        let scanned = vec![
            ScannedAddon {
                name: "Details! Damage Meter".to_string(),
                folder: "Details".to_string(),
                owned_folders: vec![OwnedFolder {
                    name: "Details_DataStorage".to_string(),
                }],
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: Some("11.0.2".to_string()),
                git_commit: None,
                author: Some("Tercio".to_string()),
                interface: Some("110002".to_string()),
                source: SourceKind::Manual,
                required_deps: vec!["Ace3".to_string()],
                optional_deps: vec!["ElvUI".to_string()],
                embedded_libs: vec!["LibStub".to_string()],
                notes: None,
                category: None,
                toc_title: None,
            },
            ScannedAddon {
                name: "GitAddon".to_string(),
                folder: "GitAddon".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: Some("abcdef0".to_string()),
                git_commit: Some("abcdef0123456789".to_string()),
                author: Some("Coder".to_string()),
                interface: Some("110002".to_string()),
                source: SourceKind::GitHub,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
                notes: None,
                category: None,
                toc_title: None,
            },
            ScannedAddon {
                name: "CustomLib".to_string(),
                folder: "CustomLib".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Library,
                flavor: GameFlavor::Retail,
                version: Some("1.0.0".to_string()),
                git_commit: None,
                author: None,
                interface: Some("110002".to_string()),
                source: SourceKind::Manual,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
                notes: None,
                category: None,
                toc_title: None,
            },
        ];

        let summary = database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile scanned addons");

        assert_eq!(summary.scanned_addons, 3);
        assert_eq!(summary.upserted_addons, 3);
        assert_eq!(summary.removed_addons, 2);

        let stored = database.list_addons().expect("list addons");
        assert_eq!(stored.len(), 3);

        let details = database
            .get_addon_by_folder("Details")
            .expect("get details")
            .expect("details exists");
        assert_eq!(details.installed_at, installed_at);
        assert_eq!(details.source, SourceKind::Wago);
        assert_eq!(
            details.source_url.as_deref(),
            Some("https://addons.wago.io/addons/details")
        );
        assert_eq!(details.remote_version.as_deref(), Some("11.0.0"));
        assert_eq!(details.last_checked_at, Some(checked_at));
        assert_eq!(details.version.as_deref(), Some("11.0.2"));
        assert_eq!(details.author.as_deref(), Some("Tercio"));
        assert_eq!(details.interface.as_deref(), Some("110002"));
        assert_eq!(details.required_deps, vec!["Ace3"]);
        assert_eq!(details.optional_deps, vec!["ElvUI"]);
        assert_eq!(details.embedded_libs, vec!["LibStub"]);
        assert_eq!(details.owned_folders.len(), 1);
        assert_eq!(details.owned_folders[0].name, "Details_DataStorage");

        assert!(
            database
                .get_addon_by_folder("Details_DataStorage")
                .expect("get stale owned")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("OldAddon")
                .expect("get stale missing")
                .is_none()
        );

        let git_addon = database
            .get_addon_by_folder("GitAddon")
            .expect("get git addon")
            .expect("git addon exists");
        assert_eq!(git_addon.source, SourceKind::GitHub);
        assert_eq!(git_addon.git_commit.as_deref(), Some("abcdef0123456789"));

        let custom_lib = database
            .get_addon_by_folder("CustomLib")
            .expect("get custom lib")
            .expect("custom lib exists");
        assert_eq!(custom_lib.kind, AddonKind::Addon);
        assert!(custom_lib.kind_override);
    }

    #[test]
    fn reconcile_preserves_authoritative_owned_children_for_managed_parents() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut parent = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        parent.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Vault".to_string(),
            },
        ]);
        database.upsert_addon(&parent).expect("seed parent");

        let child_row = AddonRecord::new("DBM Naxx", "DBM-Naxx", SourceKind::Manual);
        database.upsert_addon(&child_row).expect("seed child");

        let scanned = vec![
            ScannedAddon {
                name: "Deadly Boss Mods".to_string(),
                folder: "DBM-Core".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: Some("11.0.0".to_string()),
                git_commit: None,
                author: Some("MysticalOS".to_string()),
                interface: Some("110002".to_string()),
                source: SourceKind::Manual,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
                notes: None,
                category: None,
                toc_title: None,
            },
            ScannedAddon {
                name: "DBM Naxx".to_string(),
                folder: "DBM-Naxx".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: Some("11.0.0".to_string()),
                git_commit: None,
                author: Some("MysticalOS".to_string()),
                interface: Some("110002".to_string()),
                source: SourceKind::Manual,
                required_deps: vec!["DBM-Core".to_string()],
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
                notes: None,
                category: None,
                toc_title: None,
            },
        ];

        let summary = database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile managed parent");

        assert_eq!(summary.upserted_addons, 1);
        assert_eq!(summary.removed_addons, 1);

        let stored_parent = database
            .get_addon_by_folder("DBM-Core")
            .expect("get parent")
            .expect("parent exists");
        assert!(stored_parent.has_authoritative_owned_folders());
        assert_eq!(stored_parent.owned_folders.len(), 2);
        assert_eq!(stored_parent.owned_folders[0].name, "DBM-Naxx");
        assert_eq!(stored_parent.owned_folders[1].name, "DBM-Vault");

        assert!(
            database
                .get_addon_by_folder("DBM-Naxx")
                .expect("get child row")
                .is_none()
        );
    }

    #[test]
    fn reconcile_preserves_managed_github_commit_when_scan_has_no_commit() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut existing = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        existing.version = Some("364f625".to_string());
        existing.git_commit = Some("364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string());
        existing.remote_version = Some("364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string());
        existing.source_url = Some("https://github.com/WeakAuras/WeakAuras2".to_string());
        existing.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "WeakAurasArchive".to_string(),
            },
            OwnedFolder {
                name: "WeakAurasModelPaths".to_string(),
            },
        ]);
        database
            .record_managed_addon(&existing)
            .expect("seed github addon");

        let scanned = vec![ScannedAddon {
            name: "WeakAuras".to_string(),
            folder: "WeakAuras".to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: Some("@project-version@".to_string()),
            git_commit: None,
            author: Some("The WeakAuras Team".to_string()),
            interface: Some("110205".to_string()),
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
            notes: None,
            category: None,
            toc_title: None,
        }];

        database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile scanned addons");

        let stored = database
            .get_addon_by_folder("WeakAuras")
            .expect("get github addon")
            .expect("github addon exists");
        assert_eq!(stored.version.as_deref(), Some("364f625"));
        assert_eq!(
            stored.git_commit.as_deref(),
            Some("364f625cf8c4f2f1c0785ab12da2121e880ec560")
        );
        assert_eq!(
            stored.remote_version.as_deref(),
            Some("364f625cf8c4f2f1c0785ab12da2121e880ec560")
        );
    }

    #[test]
    fn remove_addon_cascades_to_owned_descendants() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut parent = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        parent.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
        ]);
        database.upsert_addon(&parent).expect("seed parent");

        let mut sibling = AddonRecord::new("Independent", "Independent", SourceKind::Manual);
        sibling.set_scan_owned_folders(vec![OwnedFolder {
            name: "Independent_Child".to_string(),
        }]);
        database.upsert_addon(&sibling).expect("seed sibling");

        let mut nested = AddonRecord::new("ElvUI_Options", "ElvUI_Options", SourceKind::Manual);
        nested.set_managed_owned_folders(vec![OwnedFolder {
            name: "ElvUI_Options_Theme".to_string(),
        }]);
        database.upsert_addon(&nested).expect("seed nested child");

        database
            .upsert_addon(&AddonRecord::new(
                "ElvUI_Options_Theme",
                "ElvUI_Options_Theme",
                SourceKind::Manual,
            ))
            .expect("seed nested grandchild");
        database
            .upsert_addon(&AddonRecord::new(
                "ElvUI_Libraries",
                "ElvUI_Libraries",
                SourceKind::Manual,
            ))
            .expect("seed second child");
        database
            .upsert_addon(&AddonRecord::new(
                "Independent_Child",
                "Independent_Child",
                SourceKind::Manual,
            ))
            .expect("seed independent child");

        database.remove_addon("ElvUI").expect("cascade remove");

        assert!(
            database
                .get_addon_by_folder("ElvUI")
                .expect("get parent")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_Options")
                .expect("get child")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_Options_Theme")
                .expect("get grandchild")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_Libraries")
                .expect("get second child")
                .is_none()
        );

        let remaining = database
            .get_addon_by_folder("Independent")
            .expect("get sibling")
            .expect("sibling remains");
        assert_eq!(remaining.owned_folders.len(), 1);
        assert_eq!(remaining.owned_folders[0].name, "Independent_Child");
    }

    #[test]
    fn planned_removal_folders_returns_parent_and_authoritative_descendants_only() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut parent = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        parent.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
        ]);
        database.upsert_addon(&parent).expect("seed parent");

        let mut managed_child =
            AddonRecord::new("ElvUI_Options", "ElvUI_Options", SourceKind::Manual);
        managed_child.set_managed_owned_folders(vec![OwnedFolder {
            name: "ElvUI_Options_Theme".to_string(),
        }]);
        database
            .upsert_addon(&managed_child)
            .expect("seed managed child");

        let mut scan_only = AddonRecord::new("Independent", "Independent", SourceKind::Manual);
        scan_only.set_scan_owned_folders(vec![OwnedFolder {
            name: "Independent_Child".to_string(),
        }]);
        database.upsert_addon(&scan_only).expect("seed scan-only");

        let planned = database
            .planned_removal_folders("ElvUI")
            .expect("planned removal");

        assert_eq!(
            planned,
            vec![
                "ElvUI".to_string(),
                "ElvUI_Libraries".to_string(),
                "ElvUI_Options".to_string(),
                "ElvUI_Options_Theme".to_string(),
            ]
        );
    }

    #[test]
    fn remove_addon_does_not_cascade_scan_inferred_relationships() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut parent = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Manual);
        parent.set_scan_owned_folders(vec![OwnedFolder {
            name: "ElvUI_WindTools".to_string(),
        }]);
        database.upsert_addon(&parent).expect("seed parent");

        let child = AddonRecord::new("WindTools", "ElvUI_WindTools", SourceKind::Manual);
        database.upsert_addon(&child).expect("seed child");

        database.remove_addon("ElvUI").expect("remove parent only");

        assert!(
            database
                .get_addon_by_folder("ElvUI")
                .expect("get parent")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_WindTools")
                .expect("get child")
                .is_some()
        );
    }

    #[test]
    fn record_managed_addon_removes_standalone_child_rows() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        database
            .upsert_addon(&AddonRecord::new(
                "DBM Naxx",
                "DBM-Naxx",
                SourceKind::Manual,
            ))
            .expect("seed existing child");
        database
            .upsert_addon(&AddonRecord::new(
                "DBM Vault",
                "DBM-Vault",
                SourceKind::Manual,
            ))
            .expect("seed second child");

        let mut managed = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        managed.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Vault".to_string(),
            },
        ]);
        managed.version = Some("11.0.1".to_string());
        managed.remote_version = Some("11.0.1".to_string());
        managed.source_url = Some("https://addons.wago.io/addons/dbm".to_string());

        database
            .record_managed_addon(&managed)
            .expect("record managed addon");

        let parent = database
            .get_addon_by_folder("DBM-Core")
            .expect("get parent")
            .expect("parent exists");
        assert!(parent.has_authoritative_owned_folders());
        assert_eq!(parent.owned_folders.len(), 2);
        assert_eq!(parent.remote_version.as_deref(), Some("11.0.1"));
        assert!(
            database
                .get_addon_by_folder("DBM-Naxx")
                .expect("get child")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("DBM-Vault")
                .expect("get second child")
                .is_none()
        );
    }

    fn manual_with_owned(folder: &str, owned: &[&str]) -> AddonRecord {
        let mut row = AddonRecord::new(folder, folder, SourceKind::Manual);
        row.set_scan_owned_folders(
            owned
                .iter()
                .map(|name| OwnedFolder {
                    name: name.to_string(),
                })
                .collect(),
        );
        row
    }

    #[test]
    fn record_managed_addon_takes_its_own_folder_out_of_other_rows_whatever_its_case() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&manual_with_owned("GatherMate2", &["GatherMate2_Data"]))
            .expect("seed parent");
        database
            .upsert_addon(&manual_with_owned("Other", &["gathermate2_data"]))
            .expect("seed other");

        database
            .record_managed_addon(&AddonRecord::new(
                "GatherMate2 Data",
                "GatherMate2_Data",
                SourceKind::Wago,
            ))
            .expect("record data");
        // GatherMate2 no longer lists the folder, so linking it later can't absorb that row.
        database
            .record_managed_addon(&AddonRecord::new(
                "GatherMate2",
                "GatherMate2",
                SourceKind::WowInterface,
            ))
            .expect("record parent");

        let rows = database.list_addons().expect("rows");
        assert_eq!(
            rows.iter()
                .map(|row| (row.folder.as_str(), row.source, row.owned_folders.len()))
                .collect::<Vec<_>>(),
            [
                ("GatherMate2", SourceKind::WowInterface, 0),
                ("GatherMate2_Data", SourceKind::Wago, 0),
                ("Other", SourceKind::Manual, 0),
            ]
        );
    }

    #[test]
    fn record_managed_addon_never_absorbs_or_strips_a_linked_addon() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut linked = AddonRecord::new("Child", "Child", SourceKind::Wago);
        linked.set_managed_owned_folders(vec![OwnedFolder {
            name: "PARENT".to_string(),
        }]);
        database.upsert_addon(&linked).expect("seed linked");
        // A Not linked row that still lists the linked addon's folder.
        database
            .upsert_addon(&manual_with_owned("Other", &["Child"]))
            .expect("seed stale");
        let before = database.list_addons().expect("rows");

        let absorbing = database
            .record_managed_addon(&AddonRecord::new("Other", "Other", SourceKind::Wago))
            .expect_err("absorbing a linked row is refused");
        let stripping = database
            .record_managed_addon(&AddonRecord::new("Parent", "Parent", SourceKind::Wago))
            .expect_err("taking a linked addon's folder is refused");

        assert!(absorbing.to_string().contains("Child"), "{absorbing}");
        assert!(stripping.to_string().contains("Child"), "{stripping}");
        assert_eq!(database.list_addons().expect("rows"), before);
    }

    #[test]
    fn record_managed_addon_update_replaces_previous_owned_folders_and_preserves_install_time() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let installed_at =
            OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("installed_at");
        let checked_at = OffsetDateTime::from_unix_timestamp(1_700_000_123).expect("checked_at");

        let mut existing = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        existing.installed_at = installed_at;
        existing.last_checked_at = Some(checked_at);
        existing.kind = AddonKind::Addon;
        existing.kind_override = true;
        existing.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Vault".to_string(),
            },
        ]);
        database
            .record_managed_addon(&existing)
            .expect("seed managed parent");

        database
            .upsert_addon(&AddonRecord::new(
                "DBM Vault",
                "DBM-Vault",
                SourceKind::Manual,
            ))
            .expect("seed prior child row");
        database
            .upsert_addon(&AddonRecord::new(
                "DBM Ulduar",
                "DBM-Ulduar",
                SourceKind::Manual,
            ))
            .expect("seed new child row");

        let mut updated = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        updated.kind = AddonKind::Library;
        updated.version = Some("11.0.2".to_string());
        updated.source_url = Some("https://addons.wago.io/addons/dbm".to_string());
        updated.remote_version = Some("11.0.2".to_string());
        updated.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Ulduar".to_string(),
            },
        ]);

        database
            .record_managed_addon(&updated)
            .expect("update managed addon");

        let parent = database
            .get_addon_by_folder("DBM-Core")
            .expect("get parent")
            .expect("parent exists");
        assert!(parent.has_authoritative_owned_folders());
        assert_eq!(parent.installed_at, installed_at);
        assert_eq!(parent.last_checked_at, Some(checked_at));
        assert_eq!(parent.kind, AddonKind::Addon);
        assert!(parent.kind_override);
        assert_eq!(parent.version.as_deref(), Some("11.0.2"));
        assert_eq!(parent.remote_version.as_deref(), Some("11.0.2"));
        assert_eq!(parent.owned_folders.len(), 2);
        assert_eq!(parent.owned_folders[0].name, "DBM-Naxx");
        assert_eq!(parent.owned_folders[1].name, "DBM-Ulduar");
        assert!(
            database
                .get_addon_by_folder("DBM-Vault")
                .expect("get prior child")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("DBM-Ulduar")
                .expect("get new child row")
                .is_none()
        );
    }

    #[test]
    fn record_managed_single_folder_addon_remains_authoritative() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut managed = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        managed.version = Some("5.21.1".to_string());
        managed.remote_version = Some("5.21.1".to_string());
        managed.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        managed.set_managed_owned_folders(Vec::new());

        database
            .record_managed_addon(&managed)
            .expect("record managed addon");

        let stored = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert!(stored.has_authoritative_owned_folders());
        assert!(stored.owned_folders.is_empty());
    }

    fn write_addon_folders(addons: &std::path::Path, folders: &[&str]) {
        for folder in folders {
            let folder_path = addons.join(folder);
            std::fs::create_dir_all(&folder_path).expect("create addon dir");
            std::fs::write(
                folder_path.join(format!("{folder}.toc")),
                format!("## Title: {folder}\n## Version: 1.0"),
            )
            .expect("write toc");
        }
    }

    fn seed_elvui_with_plugins(database: &StateDatabase) {
        let mut elvui = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        elvui.source_url = Some("https://tukui.org/elvui".to_string());
        elvui.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
        ]);
        database.upsert_addon(&elvui).expect("seed elvui");

        let mut fct = AddonRecord::new("ElvUI FCT", "ElvUI_FCT", SourceKind::GitHub);
        fct.source_url = Some("https://github.com/example/ElvUI_FCT".to_string());
        fct.version = Some("2.0.0".to_string());
        database.upsert_addon(&fct).expect("seed fct");

        let mut windtools = AddonRecord::new(
            "ElvUI WindTools",
            "ElvUI_WindTools",
            SourceKind::WowInterface,
        );
        windtools.source_url = Some("https://www.wowinterface.com/downloads/info1".to_string());
        windtools.version = Some("3.0.0".to_string());
        database.upsert_addon(&windtools).expect("seed windtools");
    }

    fn assert_elvui_plugins_survive(database: &StateDatabase) {
        let fct = database
            .get_addon_by_folder("ElvUI_FCT")
            .expect("get fct")
            .expect("fct row survives");
        assert_eq!(fct.source, SourceKind::GitHub);
        assert_eq!(
            fct.source_url.as_deref(),
            Some("https://github.com/example/ElvUI_FCT")
        );

        let windtools = database
            .get_addon_by_folder("ElvUI_WindTools")
            .expect("get windtools")
            .expect("windtools row survives");
        assert_eq!(windtools.source, SourceKind::WowInterface);
        assert_eq!(
            windtools.source_url.as_deref(),
            Some("https://www.wowinterface.com/downloads/info1")
        );

        let elvui = database
            .get_addon_by_folder("ElvUI")
            .expect("get elvui")
            .expect("elvui exists");
        assert!(elvui.has_authoritative_owned_folders());
        assert_eq!(
            elvui
                .owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["ElvUI_Libraries", "ElvUI_Options"]
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_Libraries")
                .expect("get libraries")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("ElvUI_Options")
                .expect("get options")
                .is_none()
        );
    }

    const ELVUI_TREE: [&str; 5] = [
        "ElvUI",
        "ElvUI_Libraries",
        "ElvUI_Options",
        "ElvUI_FCT",
        "ElvUI_WindTools",
    ];

    #[test]
    fn reconcile_keeps_separately_tracked_elvui_plugins() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        write_addon_folders(&addons, &ELVUI_TREE);
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        seed_elvui_with_plugins(&database);

        let tracked = database.list_addons().expect("list tracked");
        let scanned =
            crate::scan::scan_addons_dir_with_tracked(&addons, GameFlavor::Retail, &tracked)
                .expect("scan addons");
        let summary = database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile");

        assert_eq!(summary.removed_addons, 0);
        assert_elvui_plugins_survive(&database);
        let fct = database
            .get_addon_by_folder("ElvUI_FCT")
            .expect("get fct")
            .expect("fct exists");
        assert_eq!(fct.version.as_deref(), Some("1.0"));
        assert!(fct.owned_folders.is_empty());
        assert_eq!(database.list_addons().expect("list").len(), 3);
    }

    #[test]
    fn reconcile_never_deletes_tracked_rows_from_an_unguarded_scan() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        write_addon_folders(&addons, &ELVUI_TREE);
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        seed_elvui_with_plugins(&database);

        // The unguarded heuristic folds every ElvUI_* sibling under ElvUI.
        let scanned =
            crate::scan::scan_addons_dir(&addons, GameFlavor::Retail).expect("scan addons");
        assert_eq!(scanned.len(), 1);
        let summary = database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile");

        assert_eq!(summary.removed_addons, 0);
        assert_elvui_plugins_survive(&database);
    }

    #[test]
    fn reconcile_drops_tracked_children_from_unguarded_inferred_groups() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        write_addon_folders(&addons, &["Foo", "Foo_Plugin", "Foo_Options"]);
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut plugin = AddonRecord::new("Foo Plugin", "Foo_Plugin", SourceKind::GitHub);
        plugin.source_url = Some("https://github.com/example/Foo_Plugin".to_string());
        database.upsert_addon(&plugin).expect("seed plugin");

        let scanned =
            crate::scan::scan_addons_dir(&addons, GameFlavor::Retail).expect("scan addons");
        database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile");

        let foo = database
            .get_addon_by_folder("Foo")
            .expect("get foo")
            .expect("foo exists");
        assert_eq!(
            foo.owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Foo_Options"]
        );
        let plugin = database
            .get_addon_by_folder("Foo_Plugin")
            .expect("get plugin")
            .expect("plugin survives");
        assert_eq!(plugin.source, SourceKind::GitHub);
        assert_eq!(
            plugin.source_url.as_deref(),
            Some("https://github.com/example/Foo_Plugin")
        );
    }

    #[test]
    fn reconcile_keeps_untracked_sibling_of_authoritative_parent_as_manual_row() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        write_addon_folders(&addons, &["Parent", "Parent_Options", "Parent_Extra"]);
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut parent = AddonRecord::new("Parent", "Parent", SourceKind::Wago);
        parent.set_managed_owned_folders(vec![OwnedFolder {
            name: "Parent_Options".to_string(),
        }]);
        database.upsert_addon(&parent).expect("seed parent");

        let tracked = database.list_addons().expect("list tracked");
        let scanned =
            crate::scan::scan_addons_dir_with_tracked(&addons, GameFlavor::Retail, &tracked)
                .expect("scan addons");
        database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile");

        let extra = database
            .get_addon_by_folder("Parent_Extra")
            .expect("get extra")
            .expect("extra is its own row");
        assert_eq!(extra.source, SourceKind::Manual);
        assert_eq!(extra.name, "Parent_Extra");
        assert_eq!(extra.version.as_deref(), Some("1.0"));
        let parent = database
            .get_addon_by_folder("Parent")
            .expect("get parent")
            .expect("parent exists");
        assert_eq!(parent.owned_folders.len(), 1);
        assert_eq!(parent.owned_folders[0].name, "Parent_Options");
    }

    #[test]
    fn reconcile_still_groups_manual_siblings_without_authoritative_parent() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        write_addon_folders(&addons, &["Foo", "Foo_Options"]);
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&AddonRecord::new("Foo", "Foo", SourceKind::Manual))
            .expect("seed foo");
        database
            .upsert_addon(&AddonRecord::new(
                "Foo_Options",
                "Foo_Options",
                SourceKind::Manual,
            ))
            .expect("seed foo options");

        let tracked = database.list_addons().expect("list tracked");
        let scanned =
            crate::scan::scan_addons_dir_with_tracked(&addons, GameFlavor::Retail, &tracked)
                .expect("scan addons");
        database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile");

        let stored = database.list_addons().expect("list");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].folder, "Foo");
        assert_eq!(stored[0].owned_folders.len(), 1);
        assert_eq!(stored[0].owned_folders[0].name, "Foo_Options");
    }
    fn scanned_with(folder: &str, version: Option<&str>, git_commit: Option<&str>) -> ScannedAddon {
        ScannedAddon {
            name: folder.to_string(),
            folder: folder.to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: version.map(str::to_string),
            git_commit: git_commit.map(str::to_string),
            author: None,
            interface: None,
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
            notes: None,
            category: None,
            toc_title: None,
        }
    }

    fn reconciled_version(
        source: SourceKind,
        recorded: Option<&str>,
        scanned: Option<&str>,
    ) -> Option<String> {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut existing = AddonRecord::new("Foo", "Foo", source);
        existing.version = recorded.map(str::to_string);
        existing.source_url = Some("https://example.invalid/foo".to_string());
        database.upsert_addon(&existing).expect("seed row");

        database
            .reconcile_scanned_addons(&[scanned_with("Foo", scanned, None)])
            .expect("reconcile");

        database
            .get_addon_by_folder("Foo")
            .expect("get row")
            .expect("row exists")
            .version
    }

    #[test]
    fn reconcile_keeps_provider_recorded_version_over_toc_version() {
        for (source, recorded, toc) in [
            (SourceKind::Wago, "Pawn 2.14.3", "2.14.3"),
            (SourceKind::WowInterface, "v3.4.1", "12.0.5"),
            (SourceKind::Tukui, "15.10", "15.11"),
        ] {
            assert_eq!(
                reconciled_version(source, Some(recorded), Some(toc)).as_deref(),
                Some(recorded),
                "{source:?}"
            );
            // A TOC without a version must not blank the recorded one either.
            assert_eq!(
                reconciled_version(source, Some(recorded), None).as_deref(),
                Some(recorded),
                "{source:?} without toc version"
            );
        }
    }

    #[test]
    fn reconcile_fills_missing_provider_version_from_toc() {
        assert_eq!(
            reconciled_version(SourceKind::Wago, None, Some("1.2.3")).as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            reconciled_version(SourceKind::Wago, Some("  "), Some("1.2.3")).as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn reconcile_manual_and_github_rows_still_follow_the_scanned_version() {
        assert_eq!(
            reconciled_version(SourceKind::Manual, Some("1.0"), Some("2.0")).as_deref(),
            Some("2.0")
        );
        assert_eq!(
            reconciled_version(SourceKind::GitHub, Some("1.0"), Some("2.0")).as_deref(),
            Some("2.0")
        );
        assert_eq!(
            reconciled_version(
                SourceKind::GitHub,
                Some("364f625"),
                Some("@project-version@")
            )
            .as_deref(),
            Some("364f625")
        );
    }

    #[test]
    fn reconcile_new_dev_clone_is_stored_as_manual_with_commit() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        database
            .reconcile_scanned_addons(&[scanned_with("Dev", None, Some("0123456789abcdef"))])
            .expect("reconcile");

        let stored = database
            .get_addon_by_folder("Dev")
            .expect("get row")
            .expect("row exists");
        assert_eq!(stored.source, SourceKind::Manual);
        assert_eq!(stored.git_commit.as_deref(), Some("0123456789abcdef"));
    }

    #[test]
    fn reconcile_keeps_tracked_github_source_and_demotes_urlless_dev_clone_rows() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut tracked = AddonRecord::new("Tracked", "Tracked", SourceKind::GitHub);
        tracked.source_url = Some("https://github.com/owner/tracked".to_string());
        database.upsert_addon(&tracked).expect("seed tracked");
        // Left behind by an older scan that mislabeled a `.git` folder as GitHub.
        let broken = AddonRecord::new("Broken", "Broken", SourceKind::GitHub);
        database.upsert_addon(&broken).expect("seed broken");

        database
            .reconcile_scanned_addons(&[
                scanned_with("Tracked", None, Some("abcdef0123456789")),
                scanned_with("Broken", None, Some("fedcba9876543210")),
            ])
            .expect("reconcile");

        let tracked = database
            .get_addon_by_folder("Tracked")
            .expect("get tracked")
            .expect("tracked exists");
        assert_eq!(tracked.source, SourceKind::GitHub);
        assert_eq!(
            tracked.source_url.as_deref(),
            Some("https://github.com/owner/tracked")
        );
        let broken = database
            .get_addon_by_folder("Broken")
            .expect("get broken")
            .expect("broken exists");
        assert_eq!(broken.source, SourceKind::Manual);
    }
    /// Writes `folder` with a readable TOC, then locks it; returns false when the lock has no
    /// effect (running as root), in which case the caller has nothing to prove.
    #[cfg(unix)]
    fn write_locked_addon(addons: &std::path::Path, folder: &str) -> Option<std::path::PathBuf> {
        use std::os::unix::fs::PermissionsExt;

        let dir = addons.join(folder);
        fs::create_dir_all(&dir).expect("create addon dir");
        let toc = dir.join(format!("{folder}.toc"));
        fs::write(&toc, format!("## Title: {folder}\n## Version: 9.9.9\n")).expect("write toc");
        fs::set_permissions(&toc, fs::Permissions::from_mode(0o000)).expect("lock toc");
        fs::read(&toc).is_err().then_some(toc)
    }

    #[cfg(unix)]
    fn unlock(toc: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(toc, fs::Permissions::from_mode(0o644)).expect("unlock toc");
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_keeps_tracked_row_unchanged_when_its_toc_is_unreadable() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        fs::create_dir_all(&addons).expect("create addons");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut wago = AddonRecord::new("Pawn", "Pawn", SourceKind::Wago);
        wago.version = Some("Pawn 2.14.3".to_string());
        wago.remote_version = Some("Pawn 2.14.4".to_string());
        wago.source_url = Some("https://addons.wago.io/addons/pawn".to_string());
        wago.set_managed_owned_folders(vec![OwnedFolder {
            name: "PawnChild".to_string(),
        }]);
        database.record_managed_addon(&wago).expect("seed wago");
        let before = database
            .get_addon_by_folder("Pawn")
            .expect("get")
            .expect("row exists");

        let Some(toc) = write_locked_addon(&addons, "Pawn") else {
            return;
        };
        write_addon_folders(&addons, &["Healthy"]);
        let tracked = database.list_addons().expect("list");
        let report = crate::scan::scan_addons_dir_report(&addons, GameFlavor::Retail, &tracked)
            .expect("scan survives");
        assert_eq!(report.skipped_folders, vec!["Pawn".to_string()]);
        let summary = database.reconcile_scan(&report).expect("reconcile");
        unlock(&toc);

        let after = database
            .get_addon_by_folder("Pawn")
            .expect("get")
            .expect("row survives");
        assert_eq!(after, before);
        assert_eq!(summary.removed_addons, 0);
        // Readable folders still reconcile as usual.
        assert!(
            database
                .get_addon_by_folder("Healthy")
                .expect("get")
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_creates_no_row_for_a_new_unreadable_folder() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        fs::create_dir_all(&addons).expect("create addons");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let Some(toc) = write_locked_addon(&addons, "Locked") else {
            return;
        };
        write_addon_folders(&addons, &["Healthy"]);
        let report = crate::scan::scan_addons_dir_report(&addons, GameFlavor::Retail, &[])
            .expect("scan survives");
        database.reconcile_scan(&report).expect("reconcile");
        unlock(&toc);

        assert!(
            database
                .get_addon_by_folder("Locked")
                .expect("get")
                .is_none()
        );
        assert!(
            database
                .get_addon_by_folder("Healthy")
                .expect("get")
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_keeps_the_row_of_a_folder_whose_metadata_cannot_be_read() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        fs::create_dir_all(&addons).expect("create addons");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut linked = AddonRecord::new("MyAddon", "MyAddon", SourceKind::GitHub);
        linked.source_url = Some("https://github.com/example/myaddon".to_string());
        database.upsert_addon(&linked).expect("seed linked");
        let before = database
            .get_addon_by_folder("MyAddon")
            .expect("get")
            .expect("row exists");

        // The AddOns folder lists the entry, but reading its metadata fails (here a symlink
        // whose target is gone, for example an unmounted development checkout).
        std::os::unix::fs::symlink(temp.path().join("missing-checkout"), addons.join("MyAddon"))
            .expect("dangling symlink");
        write_addon_folders(&addons, &["Healthy"]);

        let tracked = database.list_addons().expect("list");
        let report = crate::scan::scan_addons_dir_report(&addons, GameFlavor::Retail, &tracked)
            .expect("scan survives");
        assert_eq!(report.skipped_folders, vec!["MyAddon".to_string()]);
        let summary = database.reconcile_scan(&report).expect("reconcile");

        assert_eq!(summary.removed_addons, 0);
        assert_eq!(
            database
                .get_addon_by_folder("MyAddon")
                .expect("get")
                .expect("row survives"),
            before
        );
    }

    #[test]
    fn reconcile_still_removes_rows_for_folders_that_are_neither_scanned_nor_skipped() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&AddonRecord::new("Gone", "Gone", SourceKind::Manual))
            .expect("seed gone");
        database
            .upsert_addon(&AddonRecord::new("Kept", "Kept", SourceKind::Manual))
            .expect("seed kept");

        let report = crate::scan::ScanReport {
            addons: Vec::new(),
            skipped_folders: vec!["Kept".to_string()],
            leftover_staging: vec![".lemonup-staging-1-2-0".to_string()],
        };
        let summary = database.reconcile_scan(&report).expect("reconcile");
        assert_eq!(
            summary.leftover_staging,
            vec![".lemonup-staging-1-2-0".to_string()]
        );

        assert_eq!(summary.removed_addons, 1);
        assert_eq!(summary.skipped_folders, vec!["Kept".to_string()]);
        assert!(database.get_addon_by_folder("Gone").expect("get").is_none());
        assert!(database.get_addon_by_folder("Kept").expect("get").is_some());
    }

    fn stored_values(database: &StateDatabase, folder: &str) -> [String; 4] {
        database
            .connection
            .query_row(
                "SELECT source, flavor, kind, ownership_source FROM addons WHERE folder = ?1",
                [folder],
                |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?]),
            )
            .expect("stored row")
    }

    /// Stands in for a newer LemonUp writing a value this version does not know.
    fn store_value(database: &StateDatabase, folder: &str, column: &str, value: &str) {
        database
            .connection
            .execute(
                &format!("UPDATE addons SET {column} = ?1 WHERE folder = ?2"),
                [value, folder],
            )
            .expect("store value");
    }

    #[test]
    fn unknown_source_loads_as_not_linked_and_is_kept_through_scans_and_writes() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut curse = AddonRecord::new("Curse", "CurseAddon", SourceKind::Wago);
        curse.source_url = Some("https://www.curseforge.com/wow/addons/curse".to_string());
        curse.version = Some("1.2.3".to_string());
        database.upsert_addon(&curse).expect("seed curse");
        database
            .upsert_addon(&AddonRecord::new("Plain", "Plain", SourceKind::Manual))
            .expect("seed plain");
        store_value(&database, "CurseAddon", "source", "curseforge");

        let addons = database.list_addons().expect("list loads");
        assert_eq!(addons.len(), 2);
        let loaded = addons
            .iter()
            .find(|addon| addon.folder == "CurseAddon")
            .expect("curse row");
        assert_eq!(loaded.source, SourceKind::Manual);
        let unrecognized = loaded.unrecognized.as_ref().expect("unrecognized values");
        assert_eq!(unrecognized.source, "curseforge");
        assert_eq!(unrecognized.flavor, None);
        assert_eq!(unrecognized.kind, None);
        assert_eq!(unrecognized.ownership_source, None);

        // A scan (sync) keeps the stored source and the version that source recorded.
        database
            .reconcile_scanned_addons(&[
                scanned_with("CurseAddon", Some("9.9.9"), None),
                scanned_with("Plain", None, None),
            ])
            .expect("reconcile");
        assert_eq!(stored_values(&database, "CurseAddon")[0], "curseforge");
        let rescanned = database
            .get_addon_by_folder("CurseAddon")
            .expect("get")
            .expect("row kept");
        assert_eq!(rescanned.version.as_deref(), Some("1.2.3"));

        // Re-recording the loaded row keeps it too.
        database.upsert_addon(&rescanned).expect("upsert again");
        assert_eq!(stored_values(&database, "CurseAddon")[0], "curseforge");
        database
            .record_managed_addon(&rescanned)
            .expect("record again");
        assert_eq!(stored_values(&database, "CurseAddon")[0], "curseforge");

        // Linking it to a source this version knows replaces the stored value.
        let mut linked = AddonRecord::new("Curse", "CurseAddon", SourceKind::GitHub);
        linked.source_url = Some("https://github.com/example/curse".to_string());
        database.record_managed_addon(&linked).expect("link");
        assert_eq!(stored_values(&database, "CurseAddon")[0], "github");
        let relinked = database
            .get_addon_by_folder("CurseAddon")
            .expect("get")
            .expect("row");
        assert_eq!(relinked.source, SourceKind::GitHub);
        assert_eq!(relinked.unrecognized, None);
    }

    #[test]
    fn unknown_flavor_kind_and_ownership_load_without_breaking_the_list() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut flavored = AddonRecord::new("Flavored", "Flavored", SourceKind::GitHub);
        flavored.source_url = Some("https://github.com/example/flavored".to_string());
        database.upsert_addon(&flavored).expect("seed flavored");
        database
            .upsert_addon(&AddonRecord::new("Kinded", "Kinded", SourceKind::Manual))
            .expect("seed kinded");
        let mut owned = AddonRecord::new("Owned", "Owned", SourceKind::Manual);
        owned.set_managed_owned_folders(vec![OwnedFolder {
            name: "OwnedChild".to_string(),
        }]);
        database.upsert_addon(&owned).expect("seed owned");
        let mut plain = AddonRecord::new("Plain", "Plain", SourceKind::Wago);
        plain.source_url = Some("https://addons.wago.io/addons/plain".to_string());
        database.upsert_addon(&plain).expect("seed plain");
        store_value(&database, "Flavored", "flavor", "mists");
        store_value(&database, "Kinded", "kind", "plugin");
        store_value(&database, "Owned", "ownership_source", "exclusive");

        let addons = database.list_addons().expect("list loads");
        assert_eq!(addons.len(), 4);
        let by_folder = |folder: &str| {
            addons
                .iter()
                .find(|addon| addon.folder == folder)
                .expect("row")
                .clone()
        };

        // An unknown flavor leaves even a GitHub addon not linked, so nothing acts on it.
        let flavored = by_folder("Flavored");
        assert_eq!(flavored.source, SourceKind::Manual);
        let values = flavored.unrecognized.expect("flavor kept");
        assert_eq!(values.source, "github");
        assert_eq!(values.flavor.as_deref(), Some("mists"));

        let kinded = by_folder("Kinded");
        assert_eq!(kinded.kind, AddonKind::Addon);
        assert_eq!(
            kinded.unrecognized.expect("kind kept").kind.as_deref(),
            Some("plugin")
        );

        // Unknown ownership is not authoritative: removing the addon plans no child folders.
        let owned = by_folder("Owned");
        assert_eq!(owned.source, SourceKind::Manual);
        assert!(!owned.has_authoritative_owned_folders());
        assert_eq!(
            database.planned_removal_folders("Owned").expect("plan"),
            vec!["Owned".to_string()]
        );

        let plain = by_folder("Plain");
        assert_eq!(plain.source, SourceKind::Wago);
        assert_eq!(plain.unrecognized, None);

        database
            .reconcile_scanned_addons(&[
                scanned_with("Flavored", None, None),
                scanned_with("Kinded", None, None),
                scanned_with("Owned", None, None),
                scanned_with("Plain", None, None),
            ])
            .expect("reconcile");
        assert_eq!(
            stored_values(&database, "Flavored"),
            ["github", "mists", "addon", "none"].map(str::to_string)
        );
        assert_eq!(stored_values(&database, "Kinded")[2], "plugin");
        assert_eq!(stored_values(&database, "Owned")[3], "exclusive");
        assert_eq!(stored_values(&database, "Plain")[0], "wago");
    }

    #[test]
    fn recording_another_addon_never_deletes_or_strips_a_row_a_newer_version_linked() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut pawn = AddonRecord::new("Pawn", "Pawn", SourceKind::Wago);
        pawn.source_url = Some("https://addons.wago.io/addons/pawn".to_string());
        pawn.set_managed_owned_folders(vec![OwnedFolder {
            name: "Pawn_Extra".to_string(),
        }]);
        database.upsert_addon(&pawn).expect("seed pawn");
        store_value(&database, "Pawn", "source", "curseforge");
        let before = database
            .get_addon_by_folder("Pawn")
            .expect("get")
            .expect("row");
        assert!(before.unrecognized.is_some());

        for taken in ["pawn", "Pawn_Extra"] {
            let mut other = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
            other.source_url = Some("https://github.com/example/foo".to_string());
            other.set_managed_owned_folders(vec![
                OwnedFolder {
                    name: "Foo".to_string(),
                },
                OwnedFolder {
                    name: taken.to_string(),
                },
            ]);
            database
                .record_managed_addon(&other)
                .expect_err("folders of an addon a newer version linked are not taken");
        }

        // An unrelated addon is recorded and leaves the row alone.
        let mut other = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        other.source_url = Some("https://github.com/example/foo".to_string());
        database.record_managed_addon(&other).expect("record foo");

        let after = database
            .get_addon_by_folder("Pawn")
            .expect("get")
            .expect("row kept");
        assert_eq!(after.owned_folders, before.owned_folders);
        assert_eq!(after.unrecognized, before.unrecognized);
        assert_eq!(
            stored_values(&database, "Pawn"),
            ["curseforge", "retail", "addon", "managed"].map(str::to_string)
        );
    }

    #[test]
    fn unknown_ownership_keeps_its_owned_folders_through_syncs_and_saves() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut owned = AddonRecord::new("Owned", "Owned", SourceKind::Manual);
        owned.set_managed_owned_folders(vec![OwnedFolder {
            name: "OwnedChild".to_string(),
        }]);
        database.upsert_addon(&owned).expect("seed owned");
        store_value(&database, "Owned", "ownership_source", "exclusive");
        let child_folder = || {
            vec![OwnedFolder {
                name: "OwnedChild".to_string(),
            }]
        };

        // A sync that infers no children: the child folder is scanned on its own.
        database
            .reconcile_scanned_addons(&[
                scanned_with("Owned", None, None),
                scanned_with("OwnedChild", None, None),
            ])
            .expect("reconcile");
        let synced = database
            .get_addon_by_folder("Owned")
            .expect("get")
            .expect("row kept");
        assert_eq!(synced.owned_folders, child_folder());
        assert!(!synced.has_authoritative_owned_folders());
        assert_eq!(stored_values(&database, "Owned")[3], "exclusive");
        // Listed under its addon as stored, not as a separate row.
        assert!(
            database
                .get_addon_by_folder("OwnedChild")
                .expect("get")
                .is_none()
        );
        // Removing the addon still plans only its own folder.
        assert_eq!(
            database.planned_removal_folders("Owned").expect("plan"),
            vec!["Owned".to_string()]
        );

        // A sync where the child is not on disk keeps the stored list too.
        database
            .reconcile_scanned_addons(&[scanned_with("Owned", None, None)])
            .expect("reconcile");
        database.upsert_addon(&synced).expect("save");
        let saved = database
            .get_addon_by_folder("Owned")
            .expect("get")
            .expect("row kept");
        assert_eq!(saved.owned_folders, child_folder());
        assert_eq!(stored_values(&database, "Owned")[3], "exclusive");
    }

    #[test]
    fn unknown_ownership_never_lists_a_linked_addon_under_it() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut owned = AddonRecord::new("Owned", "Owned", SourceKind::Manual);
        owned.set_managed_owned_folders(vec![OwnedFolder {
            name: "Linked".to_string(),
        }]);
        database.upsert_addon(&owned).expect("seed owned");
        store_value(&database, "Owned", "ownership_source", "exclusive");
        let mut linked = AddonRecord::new("Linked", "Linked", SourceKind::Wago);
        linked.source_url = Some("https://addons.wago.io/addons/linked".to_string());
        database.upsert_addon(&linked).expect("seed linked");

        database
            .reconcile_scanned_addons(&[
                scanned_with("Owned", None, None),
                scanned_with("Linked", Some("2.0"), None),
            ])
            .expect("reconcile");

        let linked = database
            .get_addon_by_folder("Linked")
            .expect("get")
            .expect("linked row kept");
        assert_eq!(linked.source, SourceKind::Wago);
        assert_eq!(linked.version.as_deref(), Some("2.0"));
        let owned = database
            .get_addon_by_folder("Owned")
            .expect("get")
            .expect("row kept");
        assert_eq!(owned.owned_folders.len(), 1);
    }

    fn toc_overrides(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn user_version(path: &std::path::Path) -> i64 {
        rusqlite::Connection::open(path)
            .expect("open raw")
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version")
    }

    /// The `addons` table as schema version 2 created it.
    const V2_ADDONS_TABLE: &str = "
        CREATE TABLE addons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            folder TEXT NOT NULL UNIQUE,
            owned_folders TEXT NOT NULL,
            ownership_source TEXT NOT NULL,
            kind TEXT NOT NULL,
            kind_override INTEGER NOT NULL,
            flavor TEXT NOT NULL,
            version TEXT,
            git_commit TEXT,
            author TEXT,
            interface TEXT,
            source TEXT NOT NULL,
            source_url TEXT,
            required_deps TEXT NOT NULL,
            optional_deps TEXT NOT NULL,
            embedded_libs TEXT NOT NULL,
            installed_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_checked_at INTEGER,
            remote_version TEXT
        );";

    #[test]
    fn fresh_database_is_schema_version_3() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        let database = StateDatabase::open(&path).expect("open db");
        let mut addon = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        addon.toc_overrides = toc_overrides(&[("Title", "Bar")]);
        database.upsert_addon(&addon).expect("upsert");
        drop(database);
        assert_eq!(user_version(&path), 3);
    }

    #[test]
    fn version_2_and_version_1_databases_migrate_to_3_and_keep_their_rows() {
        let temp = tempdir().expect("tempdir");
        for (version, create) in [
            (2, V2_ADDONS_TABLE.to_string()),
            (
                1,
                V2_ADDONS_TABLE.replace("ownership_source TEXT NOT NULL,\n", ""),
            ),
        ] {
            let path = temp.path().join(format!("v{version}.sqlite"));
            let raw = rusqlite::Connection::open(&path).expect("open raw");
            raw.execute_batch(&create).expect("create old table");
            let columns = if version == 2 {
                "name, folder, owned_folders, ownership_source, kind, kind_override, flavor, source, required_deps, optional_deps, embedded_libs, installed_at, updated_at"
            } else {
                "name, folder, owned_folders, kind, kind_override, flavor, source, required_deps, optional_deps, embedded_libs, installed_at, updated_at"
            };
            let values = if version == 2 {
                "'Foo', 'Foo', '[]', 'none', 'addon', 0, 'retail', 'github', '[]', '[]', '[]', 1, 1"
            } else {
                "'Foo', 'Foo', '[]', 'addon', 0, 'retail', 'github', '[]', '[]', '[]', 1, 1"
            };
            raw.execute_batch(&format!(
                "INSERT INTO addons ({columns}) VALUES ({values});
                 PRAGMA user_version = {version};"
            ))
            .expect("seed old row");
            drop(raw);

            let database = StateDatabase::open(&path).expect("migrate");
            let stored = database.list_addons().expect("list");
            assert_eq!(stored.len(), 1, "v{version}");
            assert_eq!(stored[0].folder, "Foo");
            assert_eq!(stored[0].source, SourceKind::GitHub);
            assert!(stored[0].toc_overrides.is_empty());
            database
                .set_toc_override("Foo", TocField::Title, Some("Bar"))
                .expect("set after migration");
            drop(database);
            assert_eq!(user_version(&path), 3, "v{version}");

            let reopened = StateDatabase::open(&path).expect("reopen");
            assert_eq!(
                reopened
                    .get_addon_by_folder("Foo")
                    .expect("get")
                    .expect("row")
                    .toc_overrides,
                toc_overrides(&[("Title", "Bar")])
            );
        }
    }

    #[test]
    fn toc_overrides_round_trip_and_keep_unknown_keys() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        let database = StateDatabase::open(&path).expect("open db");
        let mut addon = AddonRecord::new("Foo", "Foo", SourceKind::Manual);
        addon.toc_overrides = toc_overrides(&[("Title", "Bar"), ("X-Future", "kept")]);
        database.upsert_addon(&addon).expect("upsert");
        assert_eq!(
            database.list_addons().expect("list")[0].toc_overrides,
            addon.toc_overrides
        );

        let updated = database
            .set_toc_override("Foo", TocField::Category, Some("  Mine "))
            .expect("set category");
        assert_eq!(
            updated,
            toc_overrides(&[("Category", "Mine"), ("Title", "Bar"), ("X-Future", "kept")])
        );
        assert!(
            database
                .set_toc_override("Foo", TocField::Notes, Some("two\nlines"))
                .is_err()
        );
        assert!(
            database
                .set_toc_override("Missing", TocField::Notes, Some("x"))
                .is_err()
        );
        database
            .set_toc_override("Foo", TocField::Title, None)
            .expect("clear title");
        database
            .set_toc_override("Foo", TocField::Category, None)
            .expect("clear category");
        assert_eq!(
            database
                .get_addon_by_folder("Foo")
                .expect("get")
                .expect("row")
                .toc_overrides,
            toc_overrides(&[("X-Future", "kept")])
        );

        database
            .set_toc_overrides("Foo", &std::collections::BTreeMap::new())
            .expect("clear all");
        let raw: Option<String> = rusqlite::Connection::open(&path)
            .expect("open raw")
            .query_row(
                "SELECT toc_overrides FROM addons WHERE folder = 'Foo'",
                [],
                |row| row.get(0),
            )
            .expect("raw value");
        assert_eq!(raw, None);
    }

    #[test]
    fn scans_and_managed_updates_keep_toc_overrides() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        tracked.source_url = Some("https://github.com/example/Foo".to_string());
        database.upsert_addon(&tracked).expect("seed");
        database
            .upsert_addon(&AddonRecord::new("Bar", "Bar", SourceKind::Manual))
            .expect("seed manual");
        let expected = toc_overrides(&[("Notes", "Mine"), ("X-Future", "kept")]);
        database
            .set_toc_overrides("Foo", &expected)
            .expect("set foo");
        database
            .set_toc_overrides("Bar", &toc_overrides(&[("Title", "Baz")]))
            .expect("set bar");

        database
            .reconcile_scanned_addons(&[
                scanned_with("Foo", Some("2.0"), None),
                scanned_with("Bar", Some("1.0"), None),
            ])
            .expect("reconcile");
        let get = |database: &StateDatabase, folder: &str| {
            database
                .get_addon_by_folder(folder)
                .expect("get")
                .expect("row")
        };
        assert_eq!(get(&database, "Foo").toc_overrides, expected);
        assert_eq!(
            get(&database, "Bar").toc_overrides,
            toc_overrides(&[("Title", "Baz")])
        );

        // An install or update records a fresh record without overrides.
        let mut refreshed = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        refreshed.version = Some("3.0".to_string());
        database
            .record_managed_addon(&refreshed)
            .expect("record managed");
        let stored = get(&database, "Foo");
        assert_eq!(stored.version.as_deref(), Some("3.0"));
        assert_eq!(stored.toc_overrides, expected);
    }

    #[test]
    fn an_upsert_of_an_old_copy_of_a_row_keeps_the_stored_toc_overrides() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut addon = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        addon.toc_overrides = toc_overrides(&[("Notes", "First")]);
        database.upsert_addon(&addon).expect("insert");
        let stale = database
            .get_addon_by_folder("Foo")
            .expect("get")
            .expect("row");

        database
            .set_toc_override("Foo", TocField::Title, Some("Mine"))
            .expect("set title");
        let mut changed = stale.clone();
        changed.version = Some("2.0".to_string());
        database.upsert_addon(&changed).expect("stale upsert");
        let mut emptied = stale;
        emptied.toc_overrides.clear();
        database
            .upsert_addon(&emptied)
            .expect("stale upsert without overrides");

        let stored = database
            .get_addon_by_folder("Foo")
            .expect("get")
            .expect("row");
        assert_eq!(
            stored.toc_overrides,
            toc_overrides(&[("Notes", "First"), ("Title", "Mine")])
        );
    }

    #[test]
    fn a_title_override_is_the_stored_name_after_scans_and_updates() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        tracked.source_url = Some("https://github.com/example/Foo".to_string());
        database.upsert_addon(&tracked).expect("seed");
        database
            .upsert_addon(&AddonRecord::new("Bar", "Bar", SourceKind::Manual))
            .expect("seed bar");
        database
            .set_toc_override("Foo", TocField::Title, Some("|cff00ff00My|r Foo"))
            .expect("set title");
        let name = |database: &StateDatabase, folder: &str| {
            database
                .get_addon_by_folder(folder)
                .expect("get")
                .expect("row")
                .name
        };

        let mut scanned_foo = scanned_with("Foo", Some("2.0"), None);
        scanned_foo.name = "Foo Original".to_string();
        let mut scanned_bar = scanned_with("Bar", Some("1.0"), None);
        scanned_bar.name = "Bar Original".to_string();
        database
            .reconcile_scanned_addons(&[scanned_foo, scanned_bar])
            .expect("reconcile");
        assert_eq!(name(&database, "Foo"), "My Foo");
        assert_eq!(name(&database, "Bar"), "Bar Original");

        let mut refreshed = AddonRecord::new("Foo Original", "Foo", SourceKind::GitHub);
        refreshed.version = Some("3.0".to_string());
        database
            .record_managed_addon(&refreshed)
            .expect("record managed");
        assert_eq!(name(&database, "Foo"), "My Foo");

        // Without a usable Title override the scanned name is used.
        database
            .set_toc_override("Foo", TocField::Title, None)
            .expect("clear title");
        database
            .record_managed_addon(&refreshed)
            .expect("record managed again");
        assert_eq!(name(&database, "Foo"), "Foo Original");
    }

    #[test]
    fn absorbed_child_rows_carry_their_toc_overrides_into_the_parent() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&AddonRecord::new("DBM", "DBM-Core", SourceKind::Manual))
            .expect("seed parent");
        database
            .upsert_addon(&AddonRecord::new(
                "DBM Naxx",
                "DBM-Naxx",
                SourceKind::Manual,
            ))
            .expect("seed child");
        database
            .set_toc_overrides("DBM-Core", &toc_overrides(&[("Notes", "Parent notes")]))
            .expect("parent overrides");
        database
            .set_toc_overrides(
                "DBM-Naxx",
                &toc_overrides(&[("Notes", "Child notes"), ("Category", "Raids")]),
            )
            .expect("child overrides");

        let mut managed = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        managed.set_managed_owned_folders(vec![OwnedFolder {
            name: "DBM-Naxx".to_string(),
        }]);
        let planned = database
            .managed_toc_overrides(&managed)
            .expect("planned overrides");
        database
            .record_managed_addon(&managed)
            .expect("record managed");

        assert!(
            database
                .get_addon_by_folder("DBM-Naxx")
                .expect("get child")
                .is_none()
        );
        assert_eq!(
            database
                .get_addon_by_folder("DBM-Core")
                .expect("get parent")
                .expect("parent")
                .toc_overrides,
            toc_overrides(&[("Category", "Raids"), ("Notes", "Parent notes")])
        );
        assert_eq!(
            planned,
            toc_overrides(&[("Category", "Raids"), ("Notes", "Parent notes")])
        );

        // A parent without a row yet gets them on insert.
        database
            .upsert_addon(&AddonRecord::new("Bags", "Bags_Config", SourceKind::Manual))
            .expect("seed lone child");
        database
            .set_toc_overrides("Bags_Config", &toc_overrides(&[("Title", "My Bags")]))
            .expect("lone child overrides");
        let mut bags = AddonRecord::new("Bags", "Bags", SourceKind::GitHub);
        bags.set_managed_owned_folders(vec![OwnedFolder {
            name: "Bags_Config".to_string(),
        }]);
        let planned = database
            .managed_toc_overrides(&bags)
            .expect("planned overrides");
        database.record_managed_addon(&bags).expect("record bags");
        let stored = database
            .get_addon_by_folder("Bags")
            .expect("get bags")
            .expect("bags");
        assert_eq!(stored.toc_overrides, toc_overrides(&[("Title", "My Bags")]));
        assert_eq!(planned, stored.toc_overrides);
        assert_eq!(stored.name, "My Bags");
    }

    #[test]
    fn a_failed_migration_leaves_the_database_as_it_was() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("v1.sqlite");
        // A version 1 table that already has the version 3 column makes the last step fail
        // after the version 2 step ran.
        let raw = rusqlite::Connection::open(&path).expect("open raw");
        raw.execute_batch(&format!(
            "{}
             ALTER TABLE addons ADD COLUMN toc_overrides TEXT;
             PRAGMA user_version = 1;",
            V2_ADDONS_TABLE.replace("ownership_source TEXT NOT NULL,\n", "")
        ))
        .expect("create v1 table");
        drop(raw);

        assert!(StateDatabase::open(&path).is_err());
        assert_eq!(user_version(&path), 1);
        let has_ownership_source: i64 = rusqlite::Connection::open(&path)
            .expect("open raw")
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('addons') WHERE name = 'ownership_source'",
                [],
                |row| row.get(0),
            )
            .expect("table info");
        assert_eq!(has_ownership_source, 0);
    }
}
