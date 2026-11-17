use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{LemonupError, Result};
use crate::paths::AppPaths;
use crate::shared::{
    OtherGamesCheck, SharedFields, SharedSettings, SharedSettingsStore, parse_profile_config,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Dark,
    Light,
    /// The terminal's own background and 16 colors.
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub addon_dir: Option<PathBuf>,
    pub user_agent: String,
    pub check_interval_secs: u64,
    pub auto_check_enabled: bool,
    pub auto_check_interval_secs: u64,
    pub backup_wtf: bool,
    pub backup_retention: u16,
    pub show_libs: bool,
    pub theme: ThemeMode,
    pub wago_api_key: Option<String>,
    /// The welcome card on the addon list was hidden with `d`. Configs
    /// written before it existed load as `false`.
    pub hide_first_run_card: bool,
    /// Plain ASCII symbols and the word LEMONUP in place of the block logo.
    pub simple_symbols: bool,
    /// A still symbol in place of the spinner, and no shimmer.
    pub reduce_motion: bool,
    /// How often the games that aren't open are checked. Shared by all games.
    pub check_other_games: OtherGamesCheck,
}

impl AppConfig {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn new_unconfigured() -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            addon_dir: None,
            user_agent: "LemonUp/2 (+https://github.com/archcorsair/lemonup)".to_string(),
            check_interval_secs: 300,
            auto_check_enabled: true,
            auto_check_interval_secs: 3600,
            backup_wtf: true,
            backup_retention: 5,
            show_libs: false,
            theme: ThemeMode::Dark,
            wago_api_key: None,
            hide_first_run_card: false,
            simple_symbols: false,
            reduce_motion: false,
            check_other_games: OtherGamesCheck::default(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::new_unconfigured()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoad {
    Missing(PathBuf),
    Loaded(AppConfig),
}

/// A profile's `config.toml`. Built with [`ConfigStore::for_profile`], it also reads the
/// settings all games share from `shared.toml` (they win over the profile's copies) and
/// writes them there; built with [`ConfigStore::new`], it is the bare file.
///
/// It remembers the shared values it last read or saved. A save writes to `shared.toml` only
/// the shared values that differ from those: the ones changed in this game. Values another
/// game (or another LemonUp window) saved since are kept, not replaced by the older copies
/// this game still holds.
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    shared: Option<SharedLink>,
}

#[derive(Debug)]
struct SharedLink {
    store: SharedSettingsStore,
    profile: String,
    /// The shared values the caller's config started from.
    seen: Mutex<Option<SharedSettings>>,
}

impl SharedLink {
    fn seen(&self) -> Option<SharedSettings> {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn remember(&self, settings: SharedSettings) {
        *self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(settings);
    }
}

impl Clone for ConfigStore {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            shared: self.shared.as_ref().map(|link| SharedLink {
                store: link.store.clone(),
                profile: link.profile.clone(),
                seen: Mutex::new(link.seen()),
            }),
        }
    }
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, shared: None }
    }

    /// The config of the profile at `paths`, with the settings all games share.
    pub fn for_profile(paths: &AppPaths) -> Self {
        Self {
            path: paths.config_file.clone(),
            shared: Some(SharedLink {
                store: SharedSettingsStore::for_roots(&paths.roots),
                profile: paths.profile.clone(),
                seen: Mutex::new(None),
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn shared_store(&self) -> Option<&SharedSettingsStore> {
        self.shared.as_ref().map(|link| &link.store)
    }

    /// The profile's settings. A file that can't be read or parsed is an error naming it.
    pub fn load(&self) -> Result<ConfigLoad> {
        if !self.path.try_exists()? {
            return Ok(ConfigLoad::Missing(self.path.clone()));
        }

        let raw =
            fs::read_to_string(&self.path).map_err(|error| LemonupError::SettingsUnreadable {
                path: self.path.clone(),
                reason: error.to_string(),
            })?;
        let (mut config, in_file) = parse_profile_config(&self.path, &raw)?;
        if let Some(link) = &self.shared {
            let settings = link.store.load_for_profile(&link.profile, &in_file)?;
            settings.apply_to(&mut config);
            link.remember(settings);
        }
        Ok(ConfigLoad::Loaded(config))
    }

    /// The settings of a profile that isn't set up yet: the defaults, with the shared values.
    pub fn unconfigured(&self) -> Result<AppConfig> {
        let mut config = AppConfig::new_unconfigured();
        if let Some(link) = &self.shared {
            let settings = link.store.load_or_migrate()?;
            settings.apply_to(&mut config);
            link.remember(settings);
        }
        Ok(config)
    }

    /// The loaded config, or [`ConfigStore::unconfigured`] when there is none.
    pub fn effective(&self, state: &ConfigLoad) -> Result<AppConfig> {
        match state {
            ConfigLoad::Loaded(config) => Ok(config.clone()),
            ConfigLoad::Missing(_) => self.unconfigured(),
        }
    }

    /// Saves `config`.
    ///
    /// `shared.toml` is read again and gets only the shared values changed in this game
    /// since this store last read or saved them (all of them when it hasn't). The profile's
    /// file gets every field, the shared ones as `shared.toml` now holds them, so an older
    /// LemonUp can read it.
    ///
    /// `shared.toml` is written first. When the profile's file then can't be written, the
    /// old `shared.toml` is put back and the save fails, so nothing reports a save that
    /// didn't happen. A crash between the two writes, or a failed put back, can still leave
    /// the new shared values with the old profile file; the next load then keeps the shared
    /// ones.
    pub fn write_config(&self, config: &AppConfig) -> Result<()> {
        let Some(link) = &self.shared else {
            let rendered = toml::to_string_pretty(config)?;
            return write_private_file(&self.path, &rendered);
        };

        let current = link.store.current()?;
        let mut merged = current.settings.clone();
        merged.take_changes(config, link.seen().as_ref());
        let mut file_config = config.clone();
        merged.apply_to(&mut file_config);
        let rendered = toml::to_string_pretty(&file_config)?;

        if current.may_write {
            merged
                .profile_copies
                .insert(link.profile.clone(), SharedFields::of(&merged));
            link.store.write(&merged)?;
            if let Err(error) = write_private_file(&self.path, &rendered) {
                link.store
                    .restore(current.on_disk.then_some(&current.settings));
                return Err(error);
            }
        } else {
            write_private_file(&self.path, &rendered)?;
        }
        link.remember(SharedSettings::from_config(config));
        Ok(())
    }

    pub fn write_new_config(&self, config: &AppConfig) -> Result<()> {
        self.write_config(config)
    }
}

/// Writes `contents` to `path` in one step: to a file beside it first, then renamed over it,
/// so a crash or a full disk never leaves a half-written settings file. The file can hold
/// API keys, so on Unix only its owner can read it.
pub(crate) fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = temp_path_for(path);
    let result = write_and_sync(&temp, contents).and_then(|()| fs::rename(&temp, path));
    if result.is_err() && temp.is_file() {
        let _ = fs::remove_file(&temp);
    }
    Ok(result?)
}

fn write_and_sync(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // A leftover temp file keeps its old mode; make it private before the key goes in.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// The file a write of `path` goes to before it is renamed into place.
pub(crate) fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{AppConfig, ConfigLoad, ConfigStore, ThemeMode};

    #[test]
    fn round_trips_config() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path);

        let mut config = AppConfig::new_unconfigured();
        config.theme = ThemeMode::Light;
        config.show_libs = true;
        store.write_new_config(&config).expect("write config");

        let loaded = store.load().expect("load config");
        assert_eq!(loaded, ConfigLoad::Loaded(config));
    }

    #[test]
    fn hidden_first_run_card_round_trips_and_defaults_to_shown() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());

        let mut config = AppConfig::new_unconfigured();
        assert!(!config.hide_first_run_card);
        config.hide_first_run_card = true;
        store.write_config(&config).expect("write config");
        assert_eq!(
            store.load().expect("load config"),
            ConfigLoad::Loaded(config)
        );

        // A config written before the flag existed shows the card.
        std::fs::write(&path, "schema_version = 1\ntheme = \"dark\"\n").expect("write old config");
        match store.load().expect("load old config") {
            ConfigLoad::Loaded(config) => assert!(!config.hide_first_run_card),
            ConfigLoad::Missing(_) => panic!("old config should load"),
        }
    }

    #[test]
    fn terminal_theme_and_look_toggles_round_trip_and_old_configs_load() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());

        let mut config = AppConfig::new_unconfigured();
        assert!(!config.simple_symbols);
        assert!(!config.reduce_motion);
        config.theme = ThemeMode::Terminal;
        config.simple_symbols = true;
        config.reduce_motion = true;
        store.write_config(&config).expect("write config");
        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(raw.contains("theme = \"terminal\""), "{raw}");
        assert_eq!(
            store.load().expect("load config"),
            ConfigLoad::Loaded(config)
        );

        // Written before Terminal and the two toggles existed.
        for theme in ["dark", "light"] {
            std::fs::write(
                &path,
                format!("schema_version = 1\ntheme = \"{theme}\"\nshow_libs = true\n"),
            )
            .expect("write old config");
            match store.load().expect("load old config") {
                ConfigLoad::Loaded(config) => {
                    assert!(!config.simple_symbols);
                    assert!(!config.reduce_motion);
                    assert!(config.show_libs);
                }
                ConfigLoad::Missing(_) => panic!("old config should load"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn writes_config_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "").expect("precreate");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let store = ConfigStore::new(path.clone());
        store
            .write_new_config(&AppConfig::new_unconfigured())
            .expect("write config");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn loads_partial_config_using_defaults() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());

        std::fs::write(&path, "wago_api_key = \"test-key\"\n").expect("write partial config");

        let loaded = store.load().expect("load config");
        match loaded {
            ConfigLoad::Loaded(config) => {
                let mut expected = AppConfig::new_unconfigured();
                expected.wago_api_key = Some("test-key".to_string());
                assert_eq!(config, expected);
            }
            ConfigLoad::Missing(_) => panic!("partial config should load"),
        }
    }

    #[test]
    fn loads_config_with_removed_legacy_keys() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());

        std::fs::write(
            &path,
            "schema_version = 1\n\
             user_agent = \"Custom/1\"\n\
             max_concurrent = 5\n\
             nerd_fonts = false\n\
             terminal_progress = false\n\
             debug = true\n\
             backup_retention = 9\n\
             theme = \"light\"\n",
        )
        .expect("write legacy config");

        match store.load().expect("load config") {
            ConfigLoad::Loaded(config) => {
                assert_eq!(config.user_agent, "Custom/1");
                assert_eq!(config.backup_retention, 9);
                assert_eq!(config.theme, ThemeMode::Light);
                assert_eq!(config.schema_version, 1);
            }
            ConfigLoad::Missing(_) => panic!("legacy config should load"),
        }
    }
}
