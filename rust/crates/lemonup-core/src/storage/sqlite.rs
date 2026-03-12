use std::fs;
use std::path::Path;

use rusqlite::{Connection, params};
use time::OffsetDateTime;

use crate::domain::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, SourceKind};
use crate::error::Result;

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
        let mut statement = self.connection.prepare(
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
        let owned_folders = serde_json::to_string(&addon.owned_folders)?;
        let required_deps = serde_json::to_string(&addon.required_deps)?;
        let optional_deps = serde_json::to_string(&addon.optional_deps)?;
        let embedded_libs = serde_json::to_string(&addon.embedded_libs)?;

        self.connection.execute(
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

    pub fn remove_addon(&self, folder: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM addons WHERE folder = ?1", [folder])?;
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

    use crate::domain::{AddonRecord, OwnedFolder, SourceKind};

    use super::StateDatabase;

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
}
