use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub addon_dir: Option<PathBuf>,
    pub user_agent: String,
    pub max_concurrent: u8,
    pub nerd_fonts: bool,
    pub check_interval_secs: u64,
    pub auto_check_enabled: bool,
    pub auto_check_interval_secs: u64,
    pub backup_wtf: bool,
    pub backup_retention: u16,
    pub debug: bool,
    pub show_libs: bool,
    pub theme: ThemeMode,
    pub terminal_progress: bool,
    pub wago_api_key: Option<String>,
}

impl AppConfig {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn new_unconfigured() -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            addon_dir: None,
            user_agent: "LemonUp/2 (+https://github.com/archcorsair/lemonup)".to_string(),
            max_concurrent: 3,
            nerd_fonts: true,
            check_interval_secs: 300,
            auto_check_enabled: true,
            auto_check_interval_secs: 3600,
            backup_wtf: true,
            backup_retention: 5,
            debug: false,
            show_libs: false,
            theme: ThemeMode::Dark,
            terminal_progress: true,
            wago_api_key: None,
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
        config.debug = true;
        store.write_new_config(&config).expect("write config");

        let loaded = store.load().expect("load config");
        assert_eq!(loaded, ConfigLoad::Loaded(config));
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
}
