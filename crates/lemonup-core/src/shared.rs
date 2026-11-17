//! Settings every game shares.
//!
//! Each profile (one per game) keeps its own `config.toml` for what belongs to that game: its
//! AddOns folder, backups of game settings and how many to keep, show libraries and the
//! welcome card. The Wago.io key, the look (theme, symbols, motion) and how checks run are
//! the same for every game, so they live once in `shared.toml` next to the default profile's
//! config, and they win over the copies in each profile's config.
//!
//! Migration: the first time a profile's settings are read or written and `shared.toml` does
//! not exist, it is created from the default profile's config (or, when there is none, from
//! the config being opened, else from the defaults). Nothing is removed from any profile's
//! config, and every later save still writes the shared values into the profile's own config
//! too, so an older LemonUp that knows nothing of `shared.toml` keeps working with the values
//! that profile last saved.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, ThemeMode, write_private_file};
use crate::error::{LemonupError, Result};
use crate::paths::ProfileRoots;

/// How often the games you don't have open are checked for updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OtherGamesCheck {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "3h")]
    ThreeHours,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "12h")]
    TwelveHours,
    #[default]
    #[serde(rename = "1d")]
    OneDay,
    #[serde(rename = "3d")]
    ThreeDays,
}

impl OtherGamesCheck {
    /// Every choice, shortest first after Off.
    pub const ALL: [Self; 7] = [
        Self::Off,
        Self::OneHour,
        Self::ThreeHours,
        Self::SixHours,
        Self::TwelveHours,
        Self::OneDay,
        Self::ThreeDays,
    ];

    /// Time between checks; `None` when the checks are off.
    pub fn interval(self) -> Option<Duration> {
        const HOUR: u64 = 60 * 60;
        let hours = match self {
            Self::Off => return None,
            Self::OneHour => 1,
            Self::ThreeHours => 3,
            Self::SixHours => 6,
            Self::TwelveHours => 12,
            Self::OneDay => 24,
            Self::ThreeDays => 72,
        };
        Some(Duration::from_secs(hours * HOUR))
    }

    /// The choice in words, as Settings shows it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::OneHour => "1 hour",
            Self::ThreeHours => "3 hours",
            Self::SixHours => "6 hours",
            Self::TwelveHours => "12 hours",
            Self::OneDay => "1 day",
            Self::ThreeDays => "3 days",
        }
    }
}

/// The contents of `shared.toml`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedSettings {
    pub schema_version: u32,
    pub wago_api_key: Option<String>,
    pub theme: ThemeMode,
    pub simple_symbols: bool,
    pub reduce_motion: bool,
    pub auto_check_enabled: bool,
    pub auto_check_interval_secs: u64,
    /// How long a check result counts as fresh before an addon is checked again.
    pub check_interval_secs: u64,
    pub check_other_games: OtherGamesCheck,
    /// The profile the TUI opened last; plain `lemonup` opens it again.
    pub last_used_profile: Option<String>,
}

impl SharedSettings {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    /// The shared values of `config`, with no last used game.
    pub fn from_config(config: &AppConfig) -> Self {
        let mut settings = Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            wago_api_key: None,
            theme: config.theme,
            simple_symbols: false,
            reduce_motion: false,
            auto_check_enabled: false,
            auto_check_interval_secs: 0,
            check_interval_secs: 0,
            check_other_games: OtherGamesCheck::default(),
            last_used_profile: None,
        };
        settings.update_from(config);
        settings
    }

    /// Takes the shared values from `config`; the last used game is kept.
    pub fn update_from(&mut self, config: &AppConfig) {
        self.wago_api_key = config.wago_api_key.clone();
        self.theme = config.theme;
        self.simple_symbols = config.simple_symbols;
        self.reduce_motion = config.reduce_motion;
        self.auto_check_enabled = config.auto_check_enabled;
        self.auto_check_interval_secs = config.auto_check_interval_secs;
        self.check_interval_secs = config.check_interval_secs;
        self.check_other_games = config.check_other_games;
    }

    /// Puts the shared values into a profile's `config`.
    pub fn apply_to(&self, config: &mut AppConfig) {
        config.wago_api_key = self.wago_api_key.clone();
        config.theme = self.theme;
        config.simple_symbols = self.simple_symbols;
        config.reduce_motion = self.reduce_motion;
        config.auto_check_enabled = self.auto_check_enabled;
        config.auto_check_interval_secs = self.auto_check_interval_secs;
        config.check_interval_secs = self.check_interval_secs;
        config.check_other_games = self.check_other_games;
    }
}

impl Default for SharedSettings {
    fn default() -> Self {
        Self::from_config(&AppConfig::new_unconfigured())
    }
}

/// Never prints the Wago.io key, only whether there is one.
impl fmt::Debug for SharedSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedSettings")
            .field("schema_version", &self.schema_version)
            .field(
                "wago_api_key",
                &self.wago_api_key.as_ref().map(|_| "<hidden>"),
            )
            .field("theme", &self.theme)
            .field("simple_symbols", &self.simple_symbols)
            .field("reduce_motion", &self.reduce_motion)
            .field("auto_check_enabled", &self.auto_check_enabled)
            .field("auto_check_interval_secs", &self.auto_check_interval_secs)
            .field("check_interval_secs", &self.check_interval_secs)
            .field("check_other_games", &self.check_other_games)
            .field("last_used_profile", &self.last_used_profile)
            .finish()
    }
}

/// Reads and writes `shared.toml`, creating it from the existing settings the first time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSettingsStore {
    path: PathBuf,
    /// The default profile's `config.toml`: the first source of the shared values.
    default_config: PathBuf,
}

impl SharedSettingsStore {
    pub fn new(path: PathBuf, default_config: PathBuf) -> Self {
        Self {
            path,
            default_config,
        }
    }

    pub fn for_roots(roots: &ProfileRoots) -> Self {
        Self::new(roots.shared_settings_file(), roots.default_config_file())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The shared settings, or `None` when `shared.toml` doesn't exist yet. Never writes.
    pub fn read(&self) -> Result<Option<SharedSettings>> {
        if !self.path.try_exists()? {
            return Ok(None);
        }
        read_toml::<SharedSettings>(&self.path).map(Some)
    }

    /// The shared settings, creating `shared.toml` first when it doesn't exist.
    ///
    /// Its values come from the default profile's config; when that doesn't exist, from
    /// `opening` (the config of the profile being opened) when it exists; else from the
    /// defaults. A file that exists but can't be read is an error, and then nothing is
    /// written: defaults never replace real values.
    pub fn load_or_migrate(&self, opening: Option<&Path>) -> Result<SharedSettings> {
        if let Some(settings) = self.read()? {
            return Ok(settings);
        }

        let mut source = None;
        for candidate in std::iter::once(self.default_config.as_path()).chain(opening) {
            if candidate.try_exists()? {
                source = Some(read_toml::<AppConfig>(candidate)?);
                break;
            }
        }
        let settings = source
            .as_ref()
            .map(SharedSettings::from_config)
            .unwrap_or_default();
        self.write(&settings)?;
        tracing::info!(path = %self.path.display(), "created the settings shared by all games");
        Ok(settings)
    }

    /// Replaces `shared.toml` in one step (written beside it, then renamed), readable only by
    /// its owner on Unix because it can hold the Wago.io key.
    pub fn write(&self, settings: &SharedSettings) -> Result<()> {
        let rendered = toml::to_string_pretty(settings)?;
        write_private_file(&self.path, &rendered)
    }

    /// Remembers `profile` as the game the TUI opened last.
    pub fn remember_last_used(&self, profile: &str) -> Result<()> {
        let mut settings = self.load_or_migrate(None)?;
        if settings.last_used_profile.as_deref() == Some(profile) {
            return Ok(());
        }
        settings.last_used_profile = Some(profile.to_string());
        self.write(&settings)
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let unreadable = |reason: String| LemonupError::SettingsUnreadable {
        path: path.to_path_buf(),
        reason,
    };
    let raw = std::fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
    toml::from_str::<T>(&raw).map_err(|error| unreadable(error.message().to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{OtherGamesCheck, SharedSettings, SharedSettingsStore};
    use crate::config::{AppConfig, ConfigLoad, ConfigStore, ThemeMode, temp_path_for};
    use crate::paths::ProfileRoots;

    const SECRET: &str = "wago-secret-for-tests";

    fn test_roots(base: &Path) -> ProfileRoots {
        ProfileRoots::new(base.join("config"), base.join("data"), base.join("cache"))
    }

    fn every_shared_field_set() -> AppConfig {
        let mut config = AppConfig::new_unconfigured();
        config.wago_api_key = Some(SECRET.to_string());
        config.theme = ThemeMode::Terminal;
        config.simple_symbols = true;
        config.reduce_motion = true;
        config.auto_check_enabled = false;
        config.auto_check_interval_secs = 7200;
        config.check_interval_secs = 900;
        config.check_other_games = OtherGamesCheck::ThreeHours;
        config.addon_dir = Some("/games/_retail_/Interface/AddOns".into());
        config.backup_retention = 9;
        config
    }

    #[test]
    fn migration_copies_every_shared_value_from_the_default_config() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let default_config = every_shared_field_set();
        ConfigStore::new(roots.default_config_file())
            .write_config(&default_config)
            .expect("write default config");
        let before = fs::read(roots.default_config_file()).expect("read default");

        let store = SharedSettingsStore::for_roots(&roots);
        let settings = store.load_or_migrate(None).expect("migrate");

        let mut expected = SharedSettings::from_config(&default_config);
        expected.last_used_profile = None;
        // `assert!` rather than `assert_eq!`: a failure must not print the key.
        assert!(settings == expected, "{settings:?}");
        assert!(settings.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(settings.theme, ThemeMode::Terminal);
        assert!(settings.simple_symbols && settings.reduce_motion);
        assert!(!settings.auto_check_enabled);
        assert_eq!(settings.auto_check_interval_secs, 7200);
        assert_eq!(settings.check_interval_secs, 900);
        assert_eq!(settings.check_other_games, OtherGamesCheck::ThreeHours);
        assert!(store.read().expect("reread") == Some(expected));
        // The default profile's config is left exactly as it was.
        assert_eq!(fs::read(roots.default_config_file()).expect("read"), before);
        assert!(!format!("{settings:?}").contains(SECRET));
    }

    #[test]
    fn migration_without_a_default_config_uses_the_opened_config_then_the_defaults() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let store = SharedSettingsStore::for_roots(&roots);
        assert!(store.load_or_migrate(None).expect("migrate") == SharedSettings::default());
        assert_eq!(
            store
                .read()
                .expect("read")
                .expect("written")
                .check_other_games,
            OtherGamesCheck::OneDay
        );

        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = roots.paths("forever").expect("paths");
        let mut config = AppConfig::new_unconfigured();
        config.theme = ThemeMode::Light;
        config.wago_api_key = Some(SECRET.to_string());
        ConfigStore::new(forever.config_file.clone())
            .write_config(&config)
            .expect("write forever config");
        let store = SharedSettingsStore::for_roots(&roots);
        let settings = store
            .load_or_migrate(Some(&forever.config_file))
            .expect("migrate");
        assert_eq!(settings.theme, ThemeMode::Light);
        assert!(settings.wago_api_key.as_deref() == Some(SECRET));
    }

    #[test]
    fn migration_runs_once() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let store = SharedSettingsStore::for_roots(&roots);
        let mut config = AppConfig::new_unconfigured();
        config.theme = ThemeMode::Light;
        ConfigStore::new(roots.default_config_file())
            .write_config(&config)
            .expect("write default");
        store.load_or_migrate(None).expect("first");
        let first = fs::read(store.path()).expect("read shared");

        // A later change to the default profile's own file doesn't redo the migration.
        config.theme = ThemeMode::Dark;
        ConfigStore::new(roots.default_config_file())
            .write_config(&config)
            .expect("rewrite default");
        let settings = store.load_or_migrate(None).expect("second");
        assert_eq!(settings.theme, ThemeMode::Light);
        assert_eq!(fs::read(store.path()).expect("read shared"), first);
    }

    #[test]
    fn an_unreadable_config_stops_the_migration_and_changes_nothing() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        fs::create_dir_all(&roots.config_dir).expect("config dir");
        let broken = "wago_api_key = \"half a key\ntheme = 7\n";
        fs::write(roots.default_config_file(), broken).expect("write broken");
        let store = SharedSettingsStore::for_roots(&roots);

        let error = store.load_or_migrate(None).expect_err("must not migrate");
        assert!(error.to_string().contains("couldn't read"), "{error}");
        assert!(!store.path().exists(), "no shared file over real values");
        assert_eq!(
            fs::read_to_string(roots.default_config_file()).expect("read"),
            broken
        );

        // A broken shared file is never replaced either.
        fs::write(store.path(), "theme = [").expect("write broken shared");
        fs::write(roots.default_config_file(), "theme = \"light\"\n").expect("fix default");
        assert!(store.load_or_migrate(None).is_err());
        assert_eq!(fs::read_to_string(store.path()).expect("read"), "theme = [");
        let profile = ConfigStore::for_profile(&roots.paths("default").expect("paths"));
        assert!(profile.load().is_err());
        assert!(
            profile
                .write_config(&AppConfig::new_unconfigured())
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(roots.default_config_file()).expect("read"),
            "theme = \"light\"\n"
        );
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_the_old_file_whole() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let store = SharedSettingsStore::for_roots(&roots);
        let mut settings = SharedSettings {
            theme: ThemeMode::Light,
            ..SharedSettings::default()
        };
        store.write(&settings).expect("first write");
        let before = fs::read(store.path()).expect("read");

        // The new contents go to a file beside it first; when that can't be written, the
        // old file must not have been emptied or half written.
        fs::create_dir_all(temp_path_for(store.path())).expect("block the temp file");
        settings.theme = ThemeMode::Dark;
        assert!(store.write(&settings).is_err());
        assert_eq!(fs::read(store.path()).expect("read"), before);
    }

    #[test]
    fn remembers_the_last_used_game_and_keeps_it_across_saves() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let store = SharedSettingsStore::for_roots(&roots);
        store.remember_last_used("forever").expect("remember");
        let paths = roots.paths("forever").expect("paths");
        ConfigStore::for_profile(&paths)
            .write_config(&AppConfig::new_unconfigured())
            .expect("save");
        assert_eq!(
            store
                .read()
                .expect("read")
                .expect("exists")
                .last_used_profile
                .as_deref(),
            Some("forever")
        );
    }

    #[test]
    fn other_games_check_choices() {
        assert_eq!(OtherGamesCheck::default(), OtherGamesCheck::OneDay);
        assert_eq!(OtherGamesCheck::Off.interval(), None);
        assert_eq!(
            OtherGamesCheck::ThreeDays.interval(),
            Some(std::time::Duration::from_secs(3 * 24 * 3600))
        );
        let labels = OtherGamesCheck::ALL.map(OtherGamesCheck::label);
        assert_eq!(
            labels,
            [
                "Off", "1 hour", "3 hours", "6 hours", "12 hours", "1 day", "3 days"
            ]
        );
    }

    #[test]
    fn profile_loads_use_the_shared_values() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let default = roots.paths("default").expect("paths");
        let forever = roots.paths("forever").expect("paths");
        ConfigStore::new(default.config_file.clone())
            .write_config(&every_shared_field_set())
            .expect("write default");
        let mut own = AppConfig::new_unconfigured();
        own.addon_dir = Some("/games/_forever_/Interface/AddOns".into());
        own.backup_retention = 2;
        ConfigStore::new(forever.config_file.clone())
            .write_config(&own)
            .expect("write forever");

        let ConfigLoad::Loaded(loaded) = ConfigStore::for_profile(&forever).load().expect("load")
        else {
            panic!("forever is set up");
        };
        assert!(loaded.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(loaded.theme, ThemeMode::Terminal);
        assert_eq!(loaded.check_other_games, OtherGamesCheck::ThreeHours);
        // Its own values stay its own.
        assert_eq!(loaded.addon_dir, own.addon_dir);
        assert_eq!(loaded.backup_retention, 2);

        // A profile that isn't set up yet starts from the shared values too.
        let unset = ConfigStore::for_profile(&roots.paths("cata").expect("paths"));
        let fresh = unset.unconfigured().expect("unconfigured");
        assert_eq!(fresh.theme, ThemeMode::Terminal);
        assert!(fresh.addon_dir.is_none());
    }

    #[test]
    fn saving_a_profile_writes_shared_values_to_the_shared_file_for_every_game() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let default = ConfigStore::for_profile(&roots.paths("default").expect("paths"));
        let forever = ConfigStore::for_profile(&roots.paths("forever").expect("paths"));
        let mut retail = AppConfig::new_unconfigured();
        retail.addon_dir = Some("/games/_retail_/Interface/AddOns".into());
        default.write_config(&retail).expect("write default");
        let mut other = AppConfig::new_unconfigured();
        other.addon_dir = Some("/games/_forever_/Interface/AddOns".into());
        forever.write_config(&other).expect("write forever");

        // Changing the theme and key while in WoW Forever changes them for Retail too.
        other.theme = ThemeMode::Light;
        other.wago_api_key = Some(SECRET.to_string());
        other.show_libs = true;
        forever.write_config(&other).expect("save forever");

        let ConfigLoad::Loaded(seen_from_retail) = default.load().expect("load") else {
            panic!("default is set up");
        };
        assert_eq!(seen_from_retail.theme, ThemeMode::Light);
        assert!(seen_from_retail.wago_api_key.as_deref() == Some(SECRET));
        // Show libraries belongs to WoW Forever alone.
        assert!(!seen_from_retail.show_libs);
    }

    #[test]
    fn a_profile_config_keeps_every_field_an_older_lemonup_reads() {
        // LemonUp 2.2 reads only the profile's config.toml. After the split it must still
        // find every setting there, including the shared ones as last saved.
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let paths = roots.paths("default").expect("paths");
        let store = ConfigStore::for_profile(&paths);
        store
            .write_config(&every_shared_field_set())
            .expect("write config");

        let raw = fs::read_to_string(&paths.config_file).expect("read");
        let table = raw.parse::<toml::Table>().expect("toml");
        for key in [
            "schema_version",
            "addon_dir",
            "user_agent",
            "check_interval_secs",
            "auto_check_enabled",
            "auto_check_interval_secs",
            "backup_wtf",
            "backup_retention",
            "show_libs",
            "theme",
            "wago_api_key",
            "hide_first_run_card",
            "simple_symbols",
            "reduce_motion",
        ] {
            assert!(table.contains_key(key), "missing {key}");
        }
        assert_eq!(table["theme"].as_str(), Some("terminal"));
        assert_eq!(table["schema_version"].as_integer(), Some(1));
        assert!(table["wago_api_key"].as_str() == Some(SECRET));
    }
}
