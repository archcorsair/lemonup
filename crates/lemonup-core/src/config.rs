use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

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

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<ConfigLoad> {
        if !self.path.exists() {
            return Ok(ConfigLoad::Missing(self.path.clone()));
        }

        let raw = fs::read_to_string(&self.path)?;
        let config = toml::from_str::<AppConfig>(&raw)?;
        Ok(ConfigLoad::Loaded(config))
    }

    pub fn write_config(&self, config: &AppConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let rendered = toml::to_string_pretty(config)?;
        fs::write(&self.path, rendered)?;
        // The config can hold API keys; keep it private to the owner on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn write_new_config(&self, config: &AppConfig) -> Result<()> {
        self.write_config(config)
    }
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
