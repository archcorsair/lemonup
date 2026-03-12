use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;

use crate::error::{LemonupError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub config_file: PathBuf,
    pub state_db_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project_dirs = ProjectDirs::from("org", "archcorsair", "lemonup")
            .ok_or(LemonupError::PathsUnavailable)?;

        let config_dir = project_dirs.config_dir().to_path_buf();
        let data_dir = project_dirs.data_dir().to_path_buf();
        let cache_dir = project_dirs.cache_dir().to_path_buf();
        let log_dir = data_dir.join("logs");

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            state_db_file: data_dir.join("state.sqlite"),
            config_dir,
            data_dir,
            cache_dir,
            log_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}
