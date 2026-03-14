use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use rusqlite::{Connection, params};
use time::OffsetDateTime;

use crate::domain::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, SourceKind};
use crate::error::Result;
use crate::scan::{ScanSummary, ScannedAddon};

pub struct StateDatabase {
    connection: Connection,
}

impl StateDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;
        let database = Self { connection };
        database.migrate()?;
        Ok(database)
    }

    pub fn list_addons(&self) -> Result<Vec<AddonRecord>> {
        Self::list_addons_from_connection(&self.connection)
    }

    pub fn get_addon_by_folder(&self, folder: &str) -> Result<Option<AddonRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT
                id, name, folder, owned_folders, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version
             FROM addons
             WHERE folder = ?1",
        )?;

        let mut rows = statement.query([folder])?;
        let maybe_row = rows.next()?;
        Ok(maybe_row.map(Self::row_to_addon).transpose()?)
    }

    pub fn upsert_addon(&self, addon: &AddonRecord) -> Result<()> {
        Self::upsert_addon_with_connection(&self.connection, addon)
    }

    pub fn reconcile_scanned_addons(
        &mut self,
        scanned_addons: &[ScannedAddon],
    ) -> Result<ScanSummary> {
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
            .collect::<HashSet<_>>();
        let active_authoritative_parents = existing_addons
            .iter()
            .filter(|addon| {
                has_authoritative_owned_folders(addon)
                    && live_scanned_folders.contains(&addon.folder)
            })
            .map(|addon| addon.folder.clone())
            .collect::<HashSet<_>>();
        let authoritative_owned_children = existing_addons
            .iter()
            .filter(|addon| active_authoritative_parents.contains(&addon.folder))
            .flat_map(|addon| addon.owned_folders.iter().map(|child| child.name.clone()))
            .collect::<HashSet<_>>();
        let mut visible_parent_folders = HashSet::new();
        let mut represented_child_folders = HashSet::new();

        for scanned_addon in scanned_addons {
            if authoritative_owned_children.contains(&scanned_addon.folder) {
                continue;
            }

            let merged = merge_scanned_addon(
                scanned_addon,
                existing_by_folder.get(&scanned_addon.folder),
                active_authoritative_parents.contains(&scanned_addon.folder),
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
        })
    }

    pub fn remove_addon(&self, folder: &str) -> Result<()> {
        let existing_addons = Self::list_addons_from_connection(&self.connection)?;
        let descendants = collect_owned_descendants(&existing_addons, folder);
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

    fn migrate(&self) -> Result<()> {
        self.connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS addons (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                folder TEXT NOT NULL UNIQUE,
                owned_folders TEXT NOT NULL,
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
            );
            PRAGMA user_version = 1;
            ",
        )?;
        Ok(())
    }

    fn list_addons_from_connection(connection: &Connection) -> Result<Vec<AddonRecord>> {
        let mut statement = connection.prepare(
            "SELECT
                id, name, folder, owned_folders, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version
             FROM addons
             ORDER BY name ASC",
        )?;

        let rows = statement.query_map([], Self::row_to_addon)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn upsert_addon_with_connection(connection: &Connection, addon: &AddonRecord) -> Result<()> {
        let owned_folders = serde_json::to_string(&addon.owned_folders)?;
        let required_deps = serde_json::to_string(&addon.required_deps)?;
        let optional_deps = serde_json::to_string(&addon.optional_deps)?;
        let embedded_libs = serde_json::to_string(&addon.embedded_libs)?;

        connection.execute(
            "INSERT INTO addons (
                name, folder, owned_folders, kind, kind_override, flavor,
                version, git_commit, author, interface, source, source_url,
                required_deps, optional_deps, embedded_libs,
                installed_at, updated_at, last_checked_at, remote_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(folder) DO UPDATE SET
                name = excluded.name,
                owned_folders = excluded.owned_folders,
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
                serialize_addon_kind(addon.kind),
                addon.kind_override,
                serialize_flavor(addon.flavor),
                addon.version,
                addon.git_commit,
                addon.author,
                addon.interface,
                serialize_source(addon.source),
                addon.source_url,
                required_deps,
                optional_deps,
                embedded_libs,
                addon.installed_at.unix_timestamp(),
                addon.updated_at.unix_timestamp(),
                addon.last_checked_at.map(|value| value.unix_timestamp()),
                addon.remote_version,
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
        let required_deps = parse_json::<Vec<String>>(row.get::<_, String>(13)?)?;
        let optional_deps = parse_json::<Vec<String>>(row.get::<_, String>(14)?)?;
        let embedded_libs = parse_json::<Vec<String>>(row.get::<_, String>(15)?)?;

        Ok(AddonRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            folder: row.get(2)?,
            owned_folders,
            kind: parse_addon_kind(&row.get::<_, String>(4)?)?,
            kind_override: row.get(5)?,
            flavor: parse_flavor(&row.get::<_, String>(6)?)?,
            version: row.get(7)?,
            git_commit: row.get(8)?,
            author: row.get(9)?,
            interface: row.get(10)?,
            source: parse_source(&row.get::<_, String>(11)?)?,
            source_url: row.get(12)?,
            required_deps,
            optional_deps,
            embedded_libs,
            installed_at: parse_timestamp(row.get::<_, i64>(16)?)?,
            updated_at: parse_timestamp(row.get::<_, i64>(17)?)?,
            last_checked_at: row
                .get::<_, Option<i64>>(18)?
                .map(parse_timestamp)
                .transpose()?,
            remote_version: row.get(19)?,
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

    AddonRecord {
        id: existing.and_then(|addon| addon.id),
        name: scanned_addon.name.clone(),
        folder: scanned_addon.folder.clone(),
        owned_folders: if preserve_existing_owned_folders {
            existing
                .expect("preserving owned folders requires existing addon")
                .owned_folders
                .clone()
        } else {
            scanned_addon.owned_folders.clone()
        },
        kind: if kind_override {
            existing
                .expect("kind override requires existing addon")
                .kind
        } else {
            scanned_addon.kind
        },
        kind_override,
        flavor: scanned_addon.flavor,
        version: scanned_addon.version.clone(),
        git_commit: scanned_addon.git_commit.clone(),
        author: scanned_addon.author.clone(),
        interface: scanned_addon.interface.clone(),
        source: existing
            .map(|addon| addon.source)
            .unwrap_or(scanned_addon.source),
        source_url: existing.and_then(|addon| addon.source_url.clone()),
        required_deps: scanned_addon.required_deps.clone(),
        optional_deps: scanned_addon.optional_deps.clone(),
        embedded_libs: scanned_addon.embedded_libs.clone(),
        installed_at: existing.map(|addon| addon.installed_at).unwrap_or(now),
        updated_at: now,
        last_checked_at: existing.and_then(|addon| addon.last_checked_at),
        remote_version: existing.and_then(|addon| addon.remote_version.clone()),
    }
}

fn has_authoritative_owned_folders(addon: &AddonRecord) -> bool {
    addon.source != SourceKind::Manual && !addon.owned_folders.is_empty()
}

fn collect_owned_descendants(existing_addons: &[AddonRecord], folder: &str) -> HashSet<String> {
    let owned_by_folder = existing_addons
        .iter()
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

fn parse_source(value: &str) -> rusqlite::Result<SourceKind> {
    match value {
        "github" => Ok(SourceKind::GitHub),
        "tukui" => Ok(SourceKind::Tukui),
        "wowinterface" => Ok(SourceKind::WowInterface),
        "wago" => Ok(SourceKind::Wago),
        "manual" => Ok(SourceKind::Manual),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn parse_flavor(value: &str) -> rusqlite::Result<GameFlavor> {
    match value {
        "retail" => Ok(GameFlavor::Retail),
        "classic" => Ok(GameFlavor::Classic),
        "cata" => Ok(GameFlavor::Cata),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn parse_addon_kind(value: &str) -> rusqlite::Result<AddonKind> {
    match value {
        "addon" => Ok(AddonKind::Addon),
        "library" => Ok(AddonKind::Library),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
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

fn serialize_flavor(value: GameFlavor) -> &'static str {
    match value {
        GameFlavor::Retail => "retail",
        GameFlavor::Classic => "classic",
        GameFlavor::Cata => "cata",
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
    use tempfile::tempdir;
    use time::OffsetDateTime;

    use crate::domain::{AddonRecord, OwnedFolder, SourceKind};
    use crate::scan::ScannedAddon;

    use super::StateDatabase;
    use crate::domain::{AddonKind, GameFlavor};

    #[test]
    fn upserts_and_lists_addons() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        addon.owned_folders = vec![
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
        ];
        addon.required_deps = vec!["LibStub".to_string()];
        database.upsert_addon(&addon).expect("upsert");

        let stored = database.list_addons().expect("list");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "ElvUI");
        assert_eq!(stored[0].owned_folders.len(), 2);
        assert_eq!(stored[0].required_deps, vec!["LibStub".to_string()]);
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
        parent.owned_folders = vec![
            OwnedFolder {
                name: "DBM-Naxx".to_string(),
            },
            OwnedFolder {
                name: "DBM-Vault".to_string(),
            },
        ];
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
    fn remove_addon_cascades_to_owned_descendants() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut parent = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        parent.owned_folders = vec![
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
        ];
        database.upsert_addon(&parent).expect("seed parent");

        let mut sibling = AddonRecord::new("Independent", "Independent", SourceKind::Manual);
        sibling.owned_folders = vec![OwnedFolder {
            name: "Independent_Child".to_string(),
        }];
        database.upsert_addon(&sibling).expect("seed sibling");

        let mut nested = AddonRecord::new("ElvUI_Options", "ElvUI_Options", SourceKind::Manual);
        nested.owned_folders = vec![OwnedFolder {
            name: "ElvUI_Options_Theme".to_string(),
        }];
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
}
