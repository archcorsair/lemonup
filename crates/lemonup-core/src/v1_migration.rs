//! One-time, read-only migration of LemonUp v1 (Bun/TypeScript) settings and
//! tracked-addon state into the v2 default profile.
//!
//! v1 stored everything in `<home>/.config/lemonup/` on every platform:
//! `config.json` (written by the `conf` package with an explicit `cwd`, so
//! `env-paths` was never consulted) and `lemonup.db` (bun:sqlite, next to the
//! config file). This module never modifies those files.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use rusqlite::{Connection, OpenFlags, Row};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::{AppConfig, ConfigStore, ThemeMode};
use crate::domain::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, SourceKind};
use crate::error::{LemonupError, Result};
use crate::paths::{AppPaths, DEFAULT_PROFILE, same_addons_folder};
use crate::storage::StateDatabase;
use crate::transfer::TransferSourceKind;
use crate::wow::validate_addons_path;

pub const V1_CONFIG_FILE_NAME: &str = "config.json";
pub const V1_DATABASE_FILE_NAME: &str = "lemonup.db";
const V1_UNCONFIGURED_DEST_DIR: &str = "NOT_CONFIGURED";
const SCRATCH_DIR_PREFIX: &str = "v1-migration-";

/// Where the v1 config and database live. Injectable so tests never touch the
/// real home directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Location {
    pub dir: PathBuf,
}

impl V1Location {
    /// v1 location for the current user: `<home>/.config/lemonup` on every OS.
    pub fn discover() -> Option<Self> {
        let base_dirs = BaseDirs::new()?;
        Some(Self::from_dir(
            base_dirs.home_dir().join(".config").join("lemonup"),
        ))
    }

    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn config_file(&self) -> PathBuf {
        self.dir.join(V1_CONFIG_FILE_NAME)
    }

    pub fn database_file(&self) -> PathBuf {
        self.dir.join(V1_DATABASE_FILE_NAME)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1MigrationReport {
    pub v1_dir: PathBuf,
    /// AddOns directory carried over from v1 `destDir`, if it validated.
    pub addon_dir: Option<PathBuf>,
    /// Why a configured v1 `destDir` was not carried over.
    pub addon_dir_rejected: Option<String>,
    pub database_found: bool,
    /// Why addons were not (fully) imported although the AddOns path migrated,
    /// for example an unreadable v1 database. Settings are still migrated.
    pub addons_not_imported: Option<String>,
    /// Folders of v1 addons written to v2 state.
    pub migrated_addons: Vec<String>,
    /// Folders of v1 addons not imported because the folder is missing or the
    /// name is not a plain folder name.
    pub skipped_addons: Vec<String>,
    /// Folders already present in v2 state; v2 data wins.
    pub already_tracked: Vec<String>,
    /// `(child, parent)`: v1 rows folded into a parent's owned folders instead
    /// of becoming separate v2 rows.
    pub owned_by_parent: Vec<(String, String)>,
}

impl V1MigrationReport {
    pub fn summary_line(&self) -> String {
        let mut parts = vec!["Migrated LemonUp v1 settings".to_string()];
        if self.addon_dir.is_some() {
            if let Some(reason) = self.addons_not_imported.as_ref() {
                parts.push(format!("addons not imported: {reason}"));
            } else {
                let mut counts = format!(
                    "{} addon{} imported, {} skipped",
                    self.migrated_addons.len(),
                    plural(self.migrated_addons.len()),
                    self.skipped_addons.len()
                );
                if !self.already_tracked.is_empty() {
                    counts.push_str(&format!(", {} already tracked", self.already_tracked.len()));
                }
                parts.push(counts);
            }
        } else if self.addon_dir_rejected.is_some() {
            parts.push("v1 AddOns path not valid, addons not imported".to_string());
        } else {
            parts.push("no v1 AddOns path, addons not imported".to_string());
        }
        parts.push("v1 files left untouched".to_string());
        parts.join(" | ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V1MigrationOutcome {
    /// Not the default profile, a v2 config already exists, or no v1 config.
    NotApplicable,
    Migrated(V1MigrationReport),
}

/// Runs the v1 migration when it applies: default profile, no v2 config file
/// yet, and a v1 `config.json` present.
///
/// It never modifies, moves or deletes v1's files. On Linux the v2 config
/// directory is the same `~/.config/lemonup` directory, so the only file it
/// may add there is v2's own `config.toml`.
///
/// An unreadable v1 config is an error and nothing is written, so the next
/// start retries. Once the config parsed, the v2 config is always written:
/// addon import problems (for example a corrupt v1 database) are reported in
/// `addons_not_imported` instead of discarding the migrated settings.
pub fn migrate_v1_if_needed(paths: &AppPaths, v1: &V1Location) -> Result<V1MigrationOutcome> {
    if paths.profile != DEFAULT_PROFILE || paths.config_file.exists() {
        return Ok(V1MigrationOutcome::NotApplicable);
    }

    let v1_config_file = v1.config_file();
    if !v1_config_file.is_file() {
        return Ok(V1MigrationOutcome::NotApplicable);
    }

    let raw = fs::read_to_string(&v1_config_file)?;
    let value: Value = serde_json::from_str(&raw)?;
    if !value.is_object() {
        return Err(LemonupError::InvalidArgument(format!(
            "v1 config is not a JSON object: {}",
            v1_config_file.display()
        )));
    }
    let mapped = map_v1_config(&value);

    let database_file = v1.database_file();
    let mut report = V1MigrationReport {
        v1_dir: v1.dir.clone(),
        addon_dir: mapped.config.addon_dir.clone(),
        addon_dir_rejected: mapped.addon_dir_rejected,
        database_found: database_file.is_file(),
        addons_not_imported: None,
        migrated_addons: Vec::new(),
        skipped_addons: Vec::new(),
        already_tracked: Vec::new(),
        owned_by_parent: Vec::new(),
    };

    if let Some(addon_dir) = mapped.config.addon_dir.as_deref()
        && report.database_found
        && let Err(error) = import_v1_addons(paths, &database_file, addon_dir, &mut report)
    {
        tracing::warn!(%error, database = %database_file.display(), "v1 addons not imported");
        report.addons_not_imported = Some(error.to_string());
    }

    ConfigStore::new(paths.config_file.clone()).write_new_config(&mapped.config)?;
    Ok(V1MigrationOutcome::Migrated(report))
}

fn import_v1_addons(
    paths: &AppPaths,
    database_file: &Path,
    addon_dir: &Path,
    report: &mut V1MigrationReport,
) -> Result<()> {
    // The v2 config can be gone while its state database stays (for example a deleted
    // config.toml). That database may already track another game folder; importing v1's rows
    // into it would mix two installs before the binding check refuses the folder.
    if let Some(bound) = StateDatabase::read_bound_addon_dir(&paths.state_db_file)?
        && !same_addons_folder(&bound, addon_dir)
    {
        tracing::warn!(
            bound = %bound.display(),
            v1_addon_dir = %addon_dir.display(),
            "v1 addons not imported: existing v2 state belongs to another AddOns folder"
        );
        report.addons_not_imported = Some(bound_elsewhere_reason(&bound, addon_dir));
        return Ok(());
    }

    let rows = read_v1_addons(database_file, &paths.cache_dir)?;
    if rows.is_empty() {
        return Ok(());
    }

    let database = StateDatabase::open(&paths.state_db_file)?;
    let existing = database
        .list_addons()?
        .into_iter()
        .flat_map(|addon| {
            std::iter::once(addon.folder)
                .chain(addon.owned_folders.into_iter().map(|owned| owned.name))
        })
        .collect::<HashSet<_>>();

    // Plain upserts: the plan already keeps owned folders out of separate rows
    // and away from existing v2 rows. `record_managed_addon` would also reset
    // `updated_at` to now and mark empty owned-folder lists authoritative.
    for record in plan_v1_addons(rows, addon_dir, &existing, report) {
        database.upsert_addon(&record)?;
        report.migrated_addons.push(record.folder);
    }
    Ok(())
}

/// Why v1 addons were left out when the v2 state is bound to another AddOns folder.
fn bound_elsewhere_reason(bound: &Path, v1_addon_dir: &Path) -> String {
    format!(
        "existing LemonUp state is bound to the AddOns folder {}, not v1's {}; v1 addons were left in v1 so the two are not mixed",
        bound.display(),
        v1_addon_dir.display()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedV1Config {
    pub config: AppConfig,
    pub addon_dir_rejected: Option<String>,
}

/// Maps a v1 `config.json` object onto a fresh v2 config. Missing or
/// wrongly-typed keys keep the v2 default; numbers are clamped to v2 ranges.
pub fn map_v1_config(value: &Value) -> MappedV1Config {
    let mut config = AppConfig::new_unconfigured();
    let mut addon_dir_rejected = None;

    if let Some(dest_dir) = value.get("destDir").and_then(Value::as_str) {
        let trimmed = dest_dir.trim();
        if !trimmed.is_empty() && trimmed != V1_UNCONFIGURED_DEST_DIR {
            let candidate = PathBuf::from(trimmed);
            match validate_addons_path(&candidate) {
                Ok(()) => config.addon_dir = Some(candidate),
                Err(reason) => addon_dir_rejected = Some(reason),
            }
        }
    }

    if let Some(key) = value.get("wagoApiKey").and_then(Value::as_str) {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            config.wago_api_key = Some(trimmed.to_string());
        }
    }

    match value.get("theme").and_then(Value::as_str) {
        Some("dark") => config.theme = ThemeMode::Dark,
        Some("light") => config.theme = ThemeMode::Light,
        _ => {}
    }

    if let Some(enabled) = value.get("autoCheckEnabled").and_then(Value::as_bool) {
        config.auto_check_enabled = enabled;
    }
    if let Some(secs) = millis_to_secs(value.get("autoCheckInterval")) {
        config.auto_check_interval_secs = secs.max(1);
    }
    if let Some(secs) = millis_to_secs(value.get("checkInterval")) {
        config.check_interval_secs = secs.max(1);
    }
    if let Some(enabled) = value.get("backupWTF").and_then(Value::as_bool) {
        config.backup_wtf = enabled;
    }
    if let Some(retention) = value.get("backupRetention").and_then(Value::as_f64)
        && retention.is_finite()
    {
        config.backup_retention = retention.round().clamp(1.0, f64::from(u16::MAX)) as u16;
    }
    if let Some(show) = value.get("showLibs").and_then(Value::as_bool) {
        config.show_libs = show;
    }

    MappedV1Config {
        config,
        addon_dir_rejected,
    }
}

fn millis_to_secs(value: Option<&Value>) -> Option<u64> {
    let millis = value?.as_f64()?;
    if !millis.is_finite() || millis < 0.0 {
        return None;
    }
    Some((millis / 1000.0).round().min(u64::MAX as f64) as u64)
}

/// One row of the v1 `addons` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1AddonRow {
    pub name: String,
    pub folder: String,
    pub owned_folders: Vec<String>,
    pub kind: Option<String>,
    pub kind_override: bool,
    pub version: Option<String>,
    pub git_commit: Option<String>,
    pub author: Option<String>,
    pub interface: Option<String>,
    pub url: Option<String>,
    pub source: String,
    pub required_deps: Vec<String>,
    pub optional_deps: Vec<String>,
    pub embedded_libs: Vec<String>,
    pub install_date: Option<String>,
    pub last_updated: Option<String>,
    pub last_checked: Option<String>,
    pub remote_version: Option<String>,
}

/// Reads the v1 database without touching the v1 directory.
///
/// v1 ran bun:sqlite in WAL mode, and SQLite creates `-wal`/`-shm` sidecar
/// files next to a WAL database even for a `SQLITE_OPEN_READ_ONLY`
/// connection. So the database (plus any un-checkpointed `-wal`) is copied
/// into a fresh, uniquely named scratch directory under `scratch_parent` (a
/// v2-owned cache directory), and only the copy is opened, with
/// `query_only`. The scratch directory is removed when reading finishes.
pub fn read_v1_addons(database_file: &Path, scratch_parent: &Path) -> Result<Vec<V1AddonRow>> {
    let scratch = ScratchDir::create(scratch_parent)?;
    read_v1_addons_copy(database_file, scratch.path())
}

fn read_v1_addons_copy(database_file: &Path, scratch_dir: &Path) -> Result<Vec<V1AddonRow>> {
    let copy = scratch_dir.join(V1_DATABASE_FILE_NAME);
    fs::copy(database_file, &copy)?;
    let wal = sidecar(database_file, "-wal");
    if wal.is_file() {
        fs::copy(&wal, sidecar(&copy, "-wal"))?;
    }

    let connection = Connection::open_with_flags(
        &copy,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("PRAGMA query_only = ON;")?;
    let mut statement = connection.prepare("SELECT * FROM addons ORDER BY name ASC")?;
    let rows = statement.query_map([], v1_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// Uniquely named directory removed on drop, so concurrent starts never share
/// or delete each other's copy.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn create(parent: &Path) -> Result<Self> {
        fs::create_dir_all(parent)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        for attempt in 0..100_u32 {
            let path = parent.join(format!(
                "{SCRATCH_DIR_PREFIX}{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(LemonupError::InvalidArgument(format!(
            "could not create a v1 migration scratch directory in {}",
            parent.display()
        )))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            tracing::debug!(%error, path = %self.path.display(), "could not remove v1 migration scratch dir");
        }
    }
}

fn sidecar(database_file: &Path, suffix: &str) -> PathBuf {
    let mut name = database_file.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn v1_row(row: &Row<'_>) -> rusqlite::Result<V1AddonRow> {
    // Optional columns tolerate older v1 schemas (schema V1 lacked
    // last_checked / remote_version).
    let optional = |column: &str| row.get::<_, Option<String>>(column).ok().flatten();
    let list = |column: &str| {
        optional(column)
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default()
    };

    Ok(V1AddonRow {
        name: row.get("name")?,
        folder: row.get("folder")?,
        owned_folders: list("owned_folders"),
        kind: optional("kind"),
        kind_override: row
            .get::<_, Option<i64>>("kind_override")
            .ok()
            .flatten()
            .is_some_and(|value| value != 0),
        version: optional("version"),
        git_commit: optional("git_commit"),
        author: optional("author"),
        interface: optional("interface"),
        url: optional("url"),
        source: row.get("type")?,
        required_deps: list("required_deps"),
        optional_deps: list("optional_deps"),
        embedded_libs: list("embedded_libs"),
        install_date: optional("install_date"),
        last_updated: optional("last_updated"),
        last_checked: optional("last_checked"),
        remote_version: optional("remote_version"),
    })
}

/// Turns v1 rows into v2 records.
///
/// - rows whose folder is missing from `addon_dir` (or is not a plain folder
///   name) are skipped
/// - rows whose folder is already represented in v2 state are left to v2
/// - owned folders must exist on disk and must not already be represented in
///   v2 state; each is claimed by the first parent listing it
/// - a v1 row whose folder is claimed as another row's owned folder is folded
///   into that parent instead of becoming its own row (owned folders are
///   never separate rows); claims by a parent that is itself claimed are
///   dropped, which breaks chains and cycles
fn plan_v1_addons(
    rows: Vec<V1AddonRow>,
    addon_dir: &Path,
    existing: &HashSet<String>,
    report: &mut V1MigrationReport,
) -> Vec<AddonRecord> {
    let on_disk = |folder: &str| is_plain_folder_name(folder) && addon_dir.join(folder).is_dir();
    let flavor = GameFlavor::for_addons_dir(addon_dir);

    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for row in rows {
        if !on_disk(&row.folder) {
            report.skipped_addons.push(row.folder);
        } else if existing.contains(&row.folder) {
            report.already_tracked.push(row.folder);
        } else if seen.insert(row.folder.clone()) {
            kept.push(row);
        }
    }

    let mut claims: HashMap<String, String> = HashMap::new();
    for row in &kept {
        for owned in &row.owned_folders {
            if owned != &row.folder
                && on_disk(owned)
                && !existing.contains(owned)
                && !claims.contains_key(owned)
            {
                claims.insert(owned.clone(), row.folder.clone());
            }
        }
    }
    let nested = claims
        .iter()
        .filter(|(_, parent)| claims.contains_key(*parent))
        .map(|(child, _)| child.clone())
        .collect::<Vec<_>>();
    for child in nested {
        claims.remove(&child);
    }

    let mut records = Vec::new();
    for row in kept {
        if let Some(parent) = claims.get(&row.folder) {
            report
                .owned_by_parent
                .push((row.folder.clone(), parent.clone()));
            continue;
        }
        let mut owned = row
            .owned_folders
            .iter()
            .filter(|owned| claims.get(*owned) == Some(&row.folder))
            .cloned()
            .collect::<Vec<_>>();
        owned.sort();
        owned.dedup();
        records.push(v1_row_to_record(row, owned, flavor));
    }
    records
}

fn v1_row_to_record(
    row: V1AddonRow,
    owned_folders: Vec<String>,
    flavor: GameFlavor,
) -> AddonRecord {
    let source = parse_v1_source(&row.source);
    let now = OffsetDateTime::now_utc();
    let mut record = AddonRecord::new(row.name, row.folder, source);

    let owned_folders = owned_folders
        .into_iter()
        .map(|name| OwnedFolder { name })
        .collect::<Vec<_>>();
    // v1 only recorded owned folders from its own installs, so they are
    // authoritative for tracked sources. Empty lists stay unset so a v2 scan
    // can still infer children.
    if owned_folders.is_empty() {
        record.clear_owned_folders();
    } else if source == SourceKind::Manual {
        record.set_scan_owned_folders(owned_folders);
    } else {
        record.set_managed_owned_folders(owned_folders);
    }

    record.kind = match row.kind.as_deref() {
        Some("library") => AddonKind::Library,
        _ => AddonKind::Addon,
    };
    record.kind_override = row.kind_override;
    // v1 always stored "retail"; the migrated AddOns dir is the truth.
    record.flavor = flavor;
    record.version = non_empty(row.version);
    record.git_commit = non_empty(row.git_commit);
    record.author = non_empty(row.author);
    record.interface = non_empty(row.interface);
    record.source_url = non_empty(row.url);
    record.required_deps = row.required_deps;
    record.optional_deps = row.optional_deps;
    record.embedded_libs = row.embedded_libs;
    record.installed_at = parse_v1_timestamp(row.install_date.as_deref()).unwrap_or(now);
    record.updated_at = parse_v1_timestamp(row.last_updated.as_deref()).unwrap_or(now);
    record.last_checked_at = parse_v1_timestamp(row.last_checked.as_deref());
    record.remote_version = non_empty(row.remote_version);
    record
}

/// v1 `type` strings are the same ones v1 wrote to its export file, so reuse
/// the transfer format's mapping. Unknown values fall back to manual (Not linked), like v1's
/// own unlinked addons.
fn parse_v1_source(raw: &str) -> SourceKind {
    let source = TransferSourceKind::from_name(&raw.to_ascii_lowercase());
    source.as_source_kind().unwrap_or_else(|| {
        tracing::info!(
            source = raw,
            "v1 addon linked to a source this version doesn't know; imported as Not linked"
        );
        SourceKind::Manual
    })
}

fn parse_v1_timestamp(raw: Option<&str>) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(raw?.trim(), &Rfc3339).ok()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|inner| !inner.trim().is_empty())
}

fn is_plain_folder_name(folder: &str) -> bool {
    !folder.is_empty() && folder != "." && folder != ".." && !folder.contains(['/', '\\', ':'])
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::{Connection, params};
    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    use super::{
        SCRATCH_DIR_PREFIX, V1Location, V1MigrationOutcome, V1MigrationReport, map_v1_config,
        migrate_v1_if_needed, read_v1_addons,
    };
    use crate::config::{AppConfig, ConfigLoad, ConfigStore, ThemeMode};
    use crate::domain::{
        AddonKind, AddonRecord, GameFlavor, OwnedFolder, OwnershipSource, SourceKind,
    };
    use crate::paths::{AppPaths, DEFAULT_PROFILE};
    use crate::storage::StateDatabase;

    const V1_SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS addons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            folder TEXT NOT NULL,
            owned_folders TEXT NOT NULL DEFAULT '[]',
            kind TEXT NOT NULL DEFAULT 'addon',
            kind_override INTEGER NOT NULL DEFAULT 0,
            flavor TEXT NOT NULL DEFAULT 'retail',
            version TEXT,
            git_commit TEXT,
            author TEXT,
            interface TEXT,
            url TEXT,
            type TEXT NOT NULL,
            required_deps TEXT NOT NULL DEFAULT '[]',
            optional_deps TEXT NOT NULL DEFAULT '[]',
            embedded_libs TEXT NOT NULL DEFAULT '[]',
            install_date TEXT NOT NULL,
            last_updated TEXT NOT NULL,
            last_checked TEXT,
            remote_version TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_addons_folder ON addons(folder);
        PRAGMA user_version = 2;
    ";

    /// v1 schema version 1: no last_checked / remote_version columns.
    const V1_SCHEMA_OLD: &str = "
        CREATE TABLE addons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            folder TEXT NOT NULL,
            owned_folders TEXT NOT NULL DEFAULT '[]',
            kind TEXT NOT NULL DEFAULT 'addon',
            kind_override INTEGER NOT NULL DEFAULT 0,
            flavor TEXT NOT NULL DEFAULT 'retail',
            version TEXT,
            git_commit TEXT,
            author TEXT,
            interface TEXT,
            url TEXT,
            type TEXT NOT NULL,
            required_deps TEXT NOT NULL DEFAULT '[]',
            optional_deps TEXT NOT NULL DEFAULT '[]',
            embedded_libs TEXT NOT NULL DEFAULT '[]',
            install_date TEXT NOT NULL,
            last_updated TEXT NOT NULL
        );
        CREATE UNIQUE INDEX idx_addons_folder ON addons(folder);
        PRAGMA user_version = 1;
    ";

    struct Fixture {
        _temp: TempDir,
        v1: V1Location,
        paths: AppPaths,
        addon_dir: PathBuf,
    }

    fn fixture(profile: &str) -> Fixture {
        fixture_with_client(profile, "_retail_")
    }

    fn fixture_with_client(profile: &str, client: &str) -> Fixture {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let wow_root = root.join("Games").join("World of Warcraft");
        let addon_dir = wow_root.join(client).join("Interface").join("AddOns");
        fs::create_dir_all(&addon_dir).expect("addons dir");
        fs::create_dir_all(wow_root.join("Data")).expect("data dir");
        fs::write(wow_root.join(".build.info"), "build").expect("build info");

        let v1_dir = root.join("home").join(".config").join("lemonup");
        fs::create_dir_all(&v1_dir).expect("v1 dir");

        let v2_root = root.join("v2");
        let paths = AppPaths {
            profile: profile.to_string(),
            config_dir: v2_root.join("config"),
            data_dir: v2_root.join("data"),
            cache_dir: v2_root.join("cache"),
            log_dir: v2_root.join("data").join("logs"),
            config_file: v2_root.join("config").join("config.toml"),
            state_db_file: v2_root.join("data").join("state.sqlite"),
            roots: crate::paths::ProfileRoots::new(
                v2_root.join("config"),
                v2_root.join("data"),
                v2_root.join("cache"),
            ),
        };

        Fixture {
            _temp: temp,
            v1: V1Location::from_dir(v1_dir),
            paths,
            addon_dir,
        }
    }

    fn write_v1_config(fixture: &Fixture, dest_dir: &str) {
        let config = json!({
            "destDir": dest_dir,
            "userAgent": "Mozilla/5.0",
            "repositories": [],
            "defaultMenuOption": "update",
            "maxConcurrent": 7,
            "nerdFonts": false,
            "checkInterval": 600000,
            "autoCheckEnabled": false,
            "autoCheckInterval": 7200000,
            "backupWTF": false,
            "backupRetention": 9,
            "debug": true,
            "showLibs": true,
            "theme": "light",
            "terminalProgress": false,
            "wagoApiKey": "wago-key",
            "lastGlobalCheck": 1700000000000_u64
        });
        fs::write(
            fixture.v1.config_file(),
            serde_json::to_vec_pretty(&config).expect("json"),
        )
        .expect("write v1 config");
    }

    fn write_v1_config_for_addon_dir(fixture: &Fixture) {
        write_v1_config(fixture, &fixture.addon_dir.display().to_string());
    }

    struct V1Fixture {
        name: &'static str,
        folder: &'static str,
        owned: &'static str,
        kind: &'static str,
        kind_override: i64,
        version: Option<&'static str>,
        git_commit: Option<&'static str>,
        url: Option<&'static str>,
        source: &'static str,
        required_deps: &'static str,
        last_checked: Option<&'static str>,
        remote_version: Option<&'static str>,
    }

    impl Default for V1Fixture {
        fn default() -> Self {
            Self {
                name: "",
                folder: "",
                owned: "[]",
                kind: "addon",
                kind_override: 0,
                version: None,
                git_commit: None,
                url: None,
                source: "manual",
                required_deps: "[]",
                last_checked: None,
                remote_version: None,
            }
        }
    }

    fn default_rows() -> Vec<V1Fixture> {
        vec![
            V1Fixture {
                name: "Deadly Boss Mods",
                folder: "DBM-Core",
                owned: r#"["DBM-Core","DBM-StatusBarTimers","DBM-Missing"]"#,
                version: Some("11.0.1"),
                url: Some("https://addons.wago.io/addons/dbm"),
                source: "wago",
                last_checked: Some("2026-01-03T00:00:00.000Z"),
                remote_version: Some("11.0.2"),
                ..V1Fixture::default()
            },
            V1Fixture {
                name: "ElvUI",
                folder: "ElvUI",
                version: Some("13.80"),
                url: Some("https://www.tukui.org/elvui"),
                source: "tukui",
                ..V1Fixture::default()
            },
            V1Fixture {
                name: "Plater",
                folder: "Plater",
                git_commit: Some("abc1234"),
                url: Some("https://github.com/Tercioo/Plater-Nameplates"),
                source: "github",
                required_deps: r#"["LibStub"]"#,
                remote_version: Some("abc1234"),
                ..V1Fixture::default()
            },
            V1Fixture {
                name: "LibStub",
                folder: "LibStub",
                kind: "library",
                kind_override: 1,
                version: Some("1.0"),
                source: "manual",
                ..V1Fixture::default()
            },
            V1Fixture {
                name: "Gone",
                folder: "GoneAddon",
                version: Some("1.0"),
                url: Some("https://www.wowinterface.com/downloads/info1-Gone.html"),
                source: "wowinterface",
                ..V1Fixture::default()
            },
        ]
    }

    fn write_v1_database(fixture: &Fixture) {
        drop(populate_v1_database(fixture, &default_rows(), false));
    }

    fn write_v1_database_rows(fixture: &Fixture, rows: &[V1Fixture]) {
        drop(populate_v1_database(fixture, rows, false));
    }

    /// Builds a v1 database with v1's schema. With `keep_wal`, checkpointing is
    /// disabled and the returned connection keeps the `-wal`/`-shm` files
    /// alive, like a v1 process that never shut down cleanly.
    fn populate_v1_database(fixture: &Fixture, rows: &[V1Fixture], keep_wal: bool) -> Connection {
        let connection = Connection::open(fixture.v1.database_file()).expect("open v1 db");
        connection
            .execute_batch("PRAGMA journal_mode = WAL;")
            .expect("wal");
        if keep_wal {
            connection
                .execute_batch("PRAGMA wal_autocheckpoint = 0;")
                .expect("disable checkpoint");
        }
        connection.execute_batch(V1_SCHEMA).expect("v1 schema");
        let insert = "INSERT INTO addons (
                name, folder, owned_folders, kind, kind_override, flavor, version, git_commit,
                author, interface, url, type, required_deps, optional_deps, embedded_libs,
                install_date, last_updated, last_checked, remote_version
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'retail', ?6, ?7, ?8, ?9, ?10, ?11, ?12, '[]', '[]',
                '2025-12-20T10:00:00.000Z', '2026-01-02T03:04:05.000Z', ?13, ?14)";
        for row in rows {
            connection
                .execute(
                    insert,
                    params![
                        row.name,
                        row.folder,
                        row.owned,
                        row.kind,
                        row.kind_override,
                        row.version,
                        row.git_commit,
                        "Author",
                        "110000",
                        row.url,
                        row.source,
                        row.required_deps,
                        row.last_checked,
                        row.remote_version
                    ],
                )
                .expect("insert v1 addon");
        }
        connection
    }

    fn create_addon_folders(fixture: &Fixture, folders: &[&str]) {
        for folder in folders {
            fs::create_dir_all(fixture.addon_dir.join(folder)).expect("addon folder");
        }
    }

    fn snapshot_dir(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut entries = fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                let entry = entry.expect("entry");
                (
                    entry.file_name().to_string_lossy().to_string(),
                    fs::read(entry.path()).expect("read file"),
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn file_names(snapshot: &[(String, Vec<u8>)]) -> Vec<&str> {
        snapshot.iter().map(|(name, _)| name.as_str()).collect()
    }

    fn assert_no_scratch_dirs(paths: &AppPaths) {
        if !paths.cache_dir.exists() {
            return;
        }
        let leftovers = fs::read_dir(&paths.cache_dir)
            .expect("read cache dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(SCRATCH_DIR_PREFIX)
            })
            .count();
        assert_eq!(leftovers, 0, "scratch dirs left in cache");
    }

    fn loaded_config(paths: &AppPaths) -> AppConfig {
        match ConfigStore::new(paths.config_file.clone())
            .load()
            .expect("load v2 config")
        {
            ConfigLoad::Loaded(config) => config,
            ConfigLoad::Missing(_) => panic!("v2 config should have been written"),
        }
    }

    fn migrated(fixture: &Fixture) -> V1MigrationReport {
        match migrate_v1_if_needed(&fixture.paths, &fixture.v1).expect("migrate") {
            V1MigrationOutcome::Migrated(report) => report,
            V1MigrationOutcome::NotApplicable => panic!("migration should apply"),
        }
    }

    fn v2_addon(fixture: &Fixture, folder: &str) -> Option<AddonRecord> {
        StateDatabase::open(&fixture.paths.state_db_file)
            .expect("v2 db")
            .get_addon_by_folder(folder)
            .expect("query")
    }

    fn owned_names(record: &AddonRecord) -> Vec<&str> {
        record
            .owned_folders
            .iter()
            .map(|owned| owned.name.as_str())
            .collect()
    }

    #[test]
    fn maps_v1_config_fields_and_clamps_values() {
        let mapped = map_v1_config(&json!({
            "destDir": "NOT_CONFIGURED",
            "theme": "light",
            "wagoApiKey": "  key-123  ",
            "autoCheckEnabled": false,
            "autoCheckInterval": 400,
            "checkInterval": 600000,
            "backupWTF": false,
            "backupRetention": 0,
            "showLibs": true,
            "maxConcurrent": 9,
            "nerdFonts": false,
            "debug": true
        }));

        let defaults = AppConfig::new_unconfigured();
        let config = mapped.config;
        assert_eq!(config.addon_dir, None);
        assert_eq!(mapped.addon_dir_rejected, None);
        assert_eq!(config.theme, ThemeMode::Light);
        assert_eq!(config.wago_api_key.as_deref(), Some("key-123"));
        assert!(!config.auto_check_enabled);
        assert_eq!(config.auto_check_interval_secs, 1);
        assert_eq!(config.check_interval_secs, 600);
        assert!(!config.backup_wtf);
        assert_eq!(config.backup_retention, 1);
        assert!(config.show_libs);
        assert_eq!(config.user_agent, defaults.user_agent);

        let clamped = map_v1_config(&json!({
            "backupRetention": 1_000_000,
            "checkInterval": -5,
            "autoCheckInterval": "soon",
            "theme": "neon",
            "wagoApiKey": "   "
        }))
        .config;
        assert_eq!(clamped.backup_retention, u16::MAX);
        assert_eq!(clamped.check_interval_secs, defaults.check_interval_secs);
        assert_eq!(
            clamped.auto_check_interval_secs,
            defaults.auto_check_interval_secs
        );
        assert_eq!(clamped.theme, defaults.theme);
        assert_eq!(clamped.wago_api_key, None);
    }

    #[test]
    fn migrates_config_and_existing_addons_without_touching_v1_files() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database(&fixture);
        create_addon_folders(
            &fixture,
            &[
                "DBM-Core",
                "DBM-StatusBarTimers",
                "ElvUI",
                "Plater",
                "LibStub",
            ],
        );
        let before = snapshot_dir(&fixture.v1.dir);

        let report = migrated(&fixture);

        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
        assert_no_scratch_dirs(&fixture.paths);
        assert_eq!(
            report.addon_dir.as_deref(),
            Some(fixture.addon_dir.as_path())
        );
        assert_eq!(report.addons_not_imported, None);
        assert_eq!(report.migrated_addons.len(), 4);
        assert_eq!(report.skipped_addons, vec!["GoneAddon".to_string()]);
        assert!(
            report
                .summary_line()
                .contains("4 addons imported, 1 skipped")
        );
        assert!(!report.summary_line().contains("already tracked"));
        assert!(report.summary_line().contains("v1 files left untouched"));

        let config = loaded_config(&fixture.paths);
        assert_eq!(
            config.addon_dir.as_deref(),
            Some(fixture.addon_dir.as_path())
        );
        assert_eq!(config.wago_api_key.as_deref(), Some("wago-key"));
        assert_eq!(config.theme, ThemeMode::Light);
        assert!(!config.auto_check_enabled);
        assert_eq!(config.auto_check_interval_secs, 7200);
        assert_eq!(config.check_interval_secs, 600);
        assert!(!config.backup_wtf);
        assert_eq!(config.backup_retention, 9);
        assert!(config.show_libs);

        let dbm = v2_addon(&fixture, "DBM-Core").expect("dbm migrated");
        assert_eq!(dbm.source, SourceKind::Wago);
        assert_eq!(
            dbm.source_url.as_deref(),
            Some("https://addons.wago.io/addons/dbm")
        );
        assert_eq!(dbm.version.as_deref(), Some("11.0.1"));
        assert_eq!(dbm.remote_version.as_deref(), Some("11.0.2"));
        assert!(dbm.last_checked_at.is_some());
        assert_eq!(dbm.flavor, GameFlavor::Retail);
        assert_eq!(dbm.ownership_source, OwnershipSource::Managed);
        assert_eq!(owned_names(&dbm), vec!["DBM-StatusBarTimers"]);
        assert_eq!(dbm.installed_at.year(), 2025);
        assert_eq!(dbm.updated_at.year(), 2026);

        let plater = v2_addon(&fixture, "Plater").expect("plater migrated");
        assert_eq!(plater.source, SourceKind::GitHub);
        assert_eq!(plater.git_commit.as_deref(), Some("abc1234"));
        assert_eq!(plater.required_deps, vec!["LibStub".to_string()]);
        assert_eq!(plater.ownership_source, OwnershipSource::None);

        let elvui = v2_addon(&fixture, "ElvUI").expect("elvui migrated");
        assert_eq!(elvui.source, SourceKind::Tukui);

        let libstub = v2_addon(&fixture, "LibStub").expect("libstub migrated");
        assert_eq!(libstub.source, SourceKind::Manual);
        assert_eq!(libstub.kind, AddonKind::Library);
        assert!(libstub.kind_override);

        assert!(v2_addon(&fixture, "GoneAddon").is_none());

        // A v2 config now exists, so a second run is a no-op.
        assert_eq!(
            migrate_v1_if_needed(&fixture.paths, &fixture.v1).expect("second run"),
            V1MigrationOutcome::NotApplicable
        );
    }

    #[test]
    fn state_bound_to_another_addons_folder_imports_no_v1_addons() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database(&fixture);
        create_addon_folders(&fixture, &["DBM-Core", "ElvUI", "Plater", "LibStub"]);
        // config.toml is gone, but the v2 state stays and already tracks Forever.
        let forever = fixture
            .addon_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("wow root")
            .join("_forever_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&forever).expect("forever addons");
        {
            let database = StateDatabase::open(&fixture.paths.state_db_file).expect("v2 db");
            database.bind_addon_dir(&forever).expect("bind forever");
            database
                .upsert_addon(&AddonRecord::new(
                    "Forever Addon",
                    "ForeverAddon",
                    SourceKind::Wago,
                ))
                .expect("seed forever row");
        }
        let before = snapshot_dir(&fixture.v1.dir);

        let report = migrated(&fixture);

        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
        assert!(report.migrated_addons.is_empty());
        let reason = report.addons_not_imported.as_deref().expect("skip reason");
        assert!(reason.contains("_forever_"), "{reason}");
        assert!(reason.contains("not mixed"), "{reason}");
        assert!(
            report
                .summary_line()
                .contains("addons not imported: existing LemonUp state is bound to")
        );

        let database = StateDatabase::open(&fixture.paths.state_db_file).expect("v2 db");
        let folders = database
            .list_addons()
            .expect("list")
            .into_iter()
            .map(|addon| addon.folder)
            .collect::<Vec<_>>();
        assert_eq!(folders, vec!["ForeverAddon".to_string()]);
        assert_eq!(
            database.bound_addon_dir().expect("binding"),
            Some(forever.clone())
        );
    }

    #[test]
    fn state_bound_to_the_same_addons_folder_still_imports_v1_addons() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database(&fixture);
        create_addon_folders(&fixture, &["DBM-Core", "ElvUI", "Plater", "LibStub"]);
        StateDatabase::open(&fixture.paths.state_db_file)
            .expect("v2 db")
            .bind_addon_dir(&fixture.addon_dir)
            .expect("bind retail");

        let report = migrated(&fixture);

        assert_eq!(report.addons_not_imported, None);
        assert_eq!(report.migrated_addons.len(), 4);
    }

    #[test]
    fn forever_dest_dir_migrates_records_as_forever() {
        let fixture = fixture_with_client(DEFAULT_PROFILE, "_forever_");
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database(&fixture);
        create_addon_folders(&fixture, &["DBM-Core", "ElvUI"]);

        let report = migrated(&fixture);
        assert_eq!(
            report.addon_dir.as_deref(),
            Some(fixture.addon_dir.as_path())
        );
        assert_eq!(report.migrated_addons.len(), 2);
        for folder in ["DBM-Core", "ElvUI"] {
            let record = v2_addon(&fixture, folder).expect("migrated");
            assert_eq!(record.flavor, GameFlavor::Forever, "{folder}");
        }
    }

    #[test]
    fn parent_with_owned_folders_keeps_them_managed() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database_rows(
            &fixture,
            &[V1Fixture {
                name: "ElvUI",
                folder: "ElvUI",
                owned: r#"["ElvUI_Options","ElvUI_Libraries","ElvUI"]"#,
                url: Some("https://www.tukui.org/elvui"),
                source: "tukui",
                ..V1Fixture::default()
            }],
        );
        create_addon_folders(&fixture, &["ElvUI", "ElvUI_Options", "ElvUI_Libraries"]);

        let report = migrated(&fixture);
        assert_eq!(report.migrated_addons, vec!["ElvUI".to_string()]);

        let elvui = v2_addon(&fixture, "ElvUI").expect("elvui migrated");
        assert_eq!(elvui.ownership_source, OwnershipSource::Managed);
        assert_eq!(
            owned_names(&elvui),
            vec!["ElvUI_Libraries", "ElvUI_Options"]
        );
        assert!(v2_addon(&fixture, "ElvUI_Options").is_none());
        assert!(v2_addon(&fixture, "ElvUI_Libraries").is_none());
    }

    #[test]
    fn manual_addon_with_owned_folders_is_scan_inferred() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database_rows(
            &fixture,
            &[V1Fixture {
                name: "My Pack",
                folder: "MyPack",
                owned: r#"["MyPack_Extra"]"#,
                source: "manual",
                ..V1Fixture::default()
            }],
        );
        create_addon_folders(&fixture, &["MyPack", "MyPack_Extra"]);

        migrated(&fixture);
        let pack = v2_addon(&fixture, "MyPack").expect("pack migrated");
        assert_eq!(pack.source, SourceKind::Manual);
        assert_eq!(pack.ownership_source, OwnershipSource::ScanInferred);
        assert_eq!(owned_names(&pack), vec!["MyPack_Extra"]);
    }

    #[test]
    fn traversal_folder_names_are_skipped() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database_rows(
            &fixture,
            &[
                V1Fixture {
                    name: "Up",
                    folder: "../x",
                    ..V1Fixture::default()
                },
                V1Fixture {
                    name: "Drive",
                    folder: "C:\\x",
                    ..V1Fixture::default()
                },
                V1Fixture {
                    name: "Nested",
                    folder: "a/b",
                    ..V1Fixture::default()
                },
                V1Fixture {
                    name: "Safe",
                    folder: "Safe",
                    owned: r#"["../x","a/b","C:\\x"]"#,
                    url: Some("https://github.com/example/safe"),
                    source: "github",
                    ..V1Fixture::default()
                },
            ],
        );
        // Make every traversal target exist so only the name check rejects it.
        fs::create_dir_all(fixture.addon_dir.join("..").join("x")).expect("sibling x");
        create_addon_folders(&fixture, &["a/b", "Safe"]);
        if cfg!(not(windows)) {
            create_addon_folders(&fixture, &["C:\\x"]);
        }

        let report = migrated(&fixture);
        let mut skipped = report.skipped_addons.clone();
        skipped.sort();
        assert_eq!(skipped, vec!["../x", "C:\\x", "a/b"]);
        assert_eq!(report.migrated_addons, vec!["Safe".to_string()]);

        let safe = v2_addon(&fixture, "Safe").expect("safe migrated");
        assert!(safe.owned_folders.is_empty());
        assert_eq!(safe.ownership_source, OwnershipSource::None);
        for folder in ["../x", "C:\\x", "a/b"] {
            assert!(v2_addon(&fixture, folder).is_none(), "{folder}");
        }
    }

    #[test]
    fn owned_folder_rows_fold_into_parent_and_respect_v2_state() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database_rows(
            &fixture,
            &[
                // Sorts before its parent, so ordering must not matter.
                V1Fixture {
                    name: "AAA Child",
                    folder: "Child",
                    source: "manual",
                    ..V1Fixture::default()
                },
                V1Fixture {
                    name: "Parent",
                    folder: "Parent",
                    owned: r#"["Child","TrackedInV2"]"#,
                    url: Some("https://addons.wago.io/addons/parent"),
                    source: "wago",
                    ..V1Fixture::default()
                },
                V1Fixture {
                    name: "Tracked",
                    folder: "TrackedInV2",
                    source: "manual",
                    ..V1Fixture::default()
                },
            ],
        );
        create_addon_folders(&fixture, &["Parent", "Child", "TrackedInV2"]);
        let seeded = AddonRecord::new("Tracked v2", "TrackedInV2", SourceKind::GitHub);
        StateDatabase::open(&fixture.paths.state_db_file)
            .expect("v2 db")
            .upsert_addon(&seeded)
            .expect("seed v2 row");

        let report = migrated(&fixture);
        assert_eq!(report.migrated_addons, vec!["Parent".to_string()]);
        assert_eq!(
            report.owned_by_parent,
            vec![("Child".to_string(), "Parent".to_string())]
        );
        assert_eq!(report.already_tracked, vec!["TrackedInV2".to_string()]);
        assert!(
            report
                .summary_line()
                .contains("1 addon imported, 0 skipped, 1 already tracked")
        );

        let parent = v2_addon(&fixture, "Parent").expect("parent migrated");
        assert_eq!(owned_names(&parent), vec!["Child"]);
        assert_eq!(parent.ownership_source, OwnershipSource::Managed);
        assert!(v2_addon(&fixture, "Child").is_none());

        let tracked = v2_addon(&fixture, "TrackedInV2").expect("v2 row kept");
        assert_eq!(tracked.name, "Tracked v2");
        assert_eq!(tracked.source, SourceKind::GitHub);
        assert!(
            !StateDatabase::open(&fixture.paths.state_db_file)
                .expect("v2 db")
                .list_addons()
                .expect("list")
                .iter()
                .any(|addon| addon.owned_folders.contains(&OwnedFolder {
                    name: "TrackedInV2".to_string()
                }))
        );
    }

    #[test]
    fn older_v1_schema_without_check_columns_migrates() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        let connection = Connection::open(fixture.v1.database_file()).expect("open v1 db");
        connection.execute_batch(V1_SCHEMA_OLD).expect("old schema");
        connection
            .execute(
                "INSERT INTO addons (name, folder, version, url, type, install_date, last_updated)
                 VALUES ('Details', 'Details', '1.2.3', 'https://addons.wago.io/addons/details',
                    'wago', '2025-12-20T10:00:00.000Z', '2025-12-21T10:00:00.000Z')",
                [],
            )
            .expect("insert");
        drop(connection);
        create_addon_folders(&fixture, &["Details"]);

        let report = migrated(&fixture);
        assert_eq!(report.addons_not_imported, None);
        let details = v2_addon(&fixture, "Details").expect("details migrated");
        assert_eq!(details.source, SourceKind::Wago);
        assert_eq!(details.version.as_deref(), Some("1.2.3"));
        assert_eq!(details.last_checked_at, None);
        assert_eq!(details.remote_version, None);
    }

    #[test]
    fn database_without_addons_table_still_migrates_config() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        let connection = Connection::open(fixture.v1.database_file()).expect("open v1 db");
        connection
            .execute_batch("CREATE TABLE other (id INTEGER);")
            .expect("other table");
        drop(connection);

        let report = migrated(&fixture);
        let reason = report.addons_not_imported.clone().expect("import failed");
        assert!(reason.contains("no such table"), "{reason}");
        assert!(report.summary_line().contains("addons not imported:"));
        assert!(report.migrated_addons.is_empty());
        assert_no_scratch_dirs(&fixture.paths);

        let config = loaded_config(&fixture.paths);
        assert_eq!(
            config.addon_dir.as_deref(),
            Some(fixture.addon_dir.as_path())
        );
        assert_eq!(config.wago_api_key.as_deref(), Some("wago-key"));
    }

    #[test]
    fn reads_v1_database_read_only() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_database(&fixture);
        let before = snapshot_dir(&fixture.v1.dir);

        let rows =
            read_v1_addons(&fixture.v1.database_file(), &fixture.paths.cache_dir).expect("read");
        assert_eq!(rows.len(), 5);
        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
        assert_no_scratch_dirs(&fixture.paths);
    }

    #[test]
    fn reads_uncheckpointed_v1_wal_without_touching_it() {
        let fixture = fixture(DEFAULT_PROFILE);
        let connection = populate_v1_database(&fixture, &default_rows(), true);
        assert!(super::sidecar(&fixture.v1.database_file(), "-wal").is_file());
        let before = snapshot_dir(&fixture.v1.dir);

        let rows =
            read_v1_addons(&fixture.v1.database_file(), &fixture.paths.cache_dir).expect("read");
        assert_eq!(rows.len(), 5);
        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
        assert_no_scratch_dirs(&fixture.paths);
        drop(connection);
    }

    #[test]
    fn shared_config_dir_only_gains_config_toml() {
        // On Linux v2's config dir is v1's ~/.config/lemonup.
        let mut fixture = fixture(DEFAULT_PROFILE);
        fixture.paths.config_dir = fixture.v1.dir.clone();
        fixture.paths.config_file = fixture.v1.dir.join("config.toml");
        write_v1_config_for_addon_dir(&fixture);
        let connection = populate_v1_database(&fixture, &default_rows(), true);
        create_addon_folders(&fixture, &["DBM-Core", "ElvUI"]);
        let before = snapshot_dir(&fixture.v1.dir);
        assert_eq!(
            file_names(&before),
            vec![
                "config.json",
                "lemonup.db",
                "lemonup.db-shm",
                "lemonup.db-wal"
            ]
        );

        let report = migrated(&fixture);
        assert_eq!(report.migrated_addons.len(), 2);

        let after = snapshot_dir(&fixture.v1.dir);
        let without_v2 = after
            .iter()
            .filter(|(name, _)| name != "config.toml")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(without_v2, before);
        assert_eq!(
            file_names(&after),
            vec![
                "config.json",
                "config.toml",
                "lemonup.db",
                "lemonup.db-shm",
                "lemonup.db-wal"
            ]
        );
        drop(connection);
    }

    #[test]
    fn concurrent_reads_use_separate_scratch_dirs() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_database(&fixture);
        let database = fixture.v1.database_file();
        let cache = fixture.paths.cache_dir.clone();

        let handles = (0..4)
            .map(|_| {
                let database = database.clone();
                let cache = cache.clone();
                std::thread::spawn(move || read_v1_addons(&database, &cache).map(|rows| rows.len()))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert_eq!(handle.join().expect("thread").expect("read"), 5);
        }
        assert_no_scratch_dirs(&fixture.paths);
    }

    #[test]
    fn no_v1_config_is_a_no_op() {
        let fixture = fixture(DEFAULT_PROFILE);

        let outcome = migrate_v1_if_needed(&fixture.paths, &fixture.v1).expect("migrate");
        assert_eq!(outcome, V1MigrationOutcome::NotApplicable);
        assert!(!fixture.paths.config_file.exists());
        assert!(!fixture.paths.state_db_file.exists());
    }

    #[test]
    fn existing_v2_config_is_a_no_op() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        write_v1_database(&fixture);
        create_addon_folders(&fixture, &["DBM-Core", "ElvUI"]);
        let store = ConfigStore::new(fixture.paths.config_file.clone());
        let existing = AppConfig::new_unconfigured();
        store.write_new_config(&existing).expect("write v2 config");

        let outcome = migrate_v1_if_needed(&fixture.paths, &fixture.v1).expect("migrate");
        assert_eq!(outcome, V1MigrationOutcome::NotApplicable);
        assert_eq!(loaded_config(&fixture.paths), existing);
        assert!(!fixture.paths.state_db_file.exists());
    }

    #[test]
    fn non_default_profile_is_a_no_op() {
        let fixture = fixture("dev");
        write_v1_config_for_addon_dir(&fixture);

        let outcome = migrate_v1_if_needed(&fixture.paths, &fixture.v1).expect("migrate");
        assert_eq!(outcome, V1MigrationOutcome::NotApplicable);
        assert!(!fixture.paths.config_file.exists());
    }

    #[test]
    fn invalid_dest_dir_migrates_config_without_addon_dir_or_addons() {
        let fixture = fixture(DEFAULT_PROFILE);
        let bogus = fixture.addon_dir.parent().expect("parent").join("Missing");
        write_v1_config(&fixture, &bogus.display().to_string());
        write_v1_database(&fixture);

        let report = migrated(&fixture);
        assert_eq!(report.addon_dir, None);
        assert!(report.addon_dir_rejected.is_some());
        assert_eq!(report.addons_not_imported, None);
        assert!(report.migrated_addons.is_empty());
        assert!(report.summary_line().contains("addons not imported"));

        let config = loaded_config(&fixture.paths);
        assert_eq!(config.addon_dir, None);
        assert_eq!(config.wago_api_key.as_deref(), Some("wago-key"));
        assert_eq!(config.theme, ThemeMode::Light);
        // The database is not even read without a valid AddOns dir.
        assert!(!fixture.paths.state_db_file.exists());
        assert!(!fixture.paths.cache_dir.exists());
    }

    #[test]
    fn corrupt_v1_config_fails_without_writing_v2_config() {
        let fixture = fixture(DEFAULT_PROFILE);
        fs::write(fixture.v1.config_file(), "{ not json").expect("write corrupt config");
        let before = snapshot_dir(&fixture.v1.dir);

        assert!(migrate_v1_if_needed(&fixture.paths, &fixture.v1).is_err());
        assert!(!fixture.paths.config_file.exists());
        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
    }

    #[test]
    fn corrupt_v1_database_still_migrates_config() {
        let fixture = fixture(DEFAULT_PROFILE);
        write_v1_config_for_addon_dir(&fixture);
        fs::write(fixture.v1.database_file(), b"definitely not sqlite").expect("corrupt db");
        let before = snapshot_dir(&fixture.v1.dir);

        let report = migrated(&fixture);
        assert!(report.addons_not_imported.is_some());
        assert!(report.summary_line().contains("addons not imported:"));
        assert_eq!(snapshot_dir(&fixture.v1.dir), before);
        assert_no_scratch_dirs(&fixture.paths);

        let config = loaded_config(&fixture.paths);
        assert_eq!(
            config.addon_dir.as_deref(),
            Some(fixture.addon_dir.as_path())
        );
        assert_eq!(config.theme, ThemeMode::Light);
    }

    #[test]
    fn unknown_v1_source_imports_as_not_linked() {
        assert_eq!(
            super::parse_v1_source("CurseForge"),
            crate::SourceKind::Manual
        );
        assert_eq!(super::parse_v1_source("Wago"), crate::SourceKind::Wago);
    }
}
