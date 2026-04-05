use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{LemonupError, Result};

pub const DEFAULT_PROFILE: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub profile: String,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub config_file: PathBuf,
    pub state_db_file: PathBuf,
}

impl AppPaths {
    pub fn discover(profile: impl Into<String>) -> Result<Self> {
        let profile = profile.into();
        validate_profile_name(&profile)?;

        let project_dirs = ProjectDirs::from("org", "archcorsair", "lemonup")
            .ok_or(LemonupError::PathsUnavailable)?;

        let config_dir = scoped_dir(project_dirs.config_dir(), &profile);
        let data_dir = scoped_dir(project_dirs.data_dir(), &profile);
        let cache_dir = scoped_dir(project_dirs.cache_dir(), &profile);
        let log_dir = data_dir.join("logs");

        Ok(Self {
            profile,
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

pub fn paths_match(left: &Path, right: &Path) -> bool {
    normalize_for_compare(left) == normalize_for_compare(right)
}

fn validate_profile_name(profile: &str) -> Result<()> {
    if profile.is_empty() {
        return Err(LemonupError::InvalidArgument(
            "profile name must not be empty".to_string(),
        ));
    }

    if profile
        .chars()
        .all(|value| value.is_ascii_alphanumeric() || value == '-' || value == '_')
    {
        return Ok(());
    }

    Err(LemonupError::InvalidArgument(format!(
        "profile name contains unsupported characters: {profile}"
    )))
}

fn scoped_dir(base: &Path, profile: &str) -> PathBuf {
    if profile == DEFAULT_PROFILE {
        return base.to_path_buf();
    }

    base.join("profiles").join(profile)
}

fn normalize_for_compare(path: &Path) -> String {
    let rendered = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join("/");

    if cfg!(windows) {
        rendered.to_ascii_lowercase()
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{AppPaths, DEFAULT_PROFILE, paths_match, scoped_dir, validate_profile_name};

    #[test]
    fn default_profile_uses_base_directories() {
        let base = Path::new("/tmp/lemonup");
        assert_eq!(
            scoped_dir(base, DEFAULT_PROFILE),
            PathBuf::from("/tmp/lemonup")
        );
    }

    #[test]
    fn non_default_profile_uses_scoped_directories() {
        let base = Path::new("/tmp/lemonup");
        assert_eq!(
            scoped_dir(base, "dev"),
            PathBuf::from("/tmp/lemonup/profiles/dev")
        );
    }

    #[test]
    fn invalid_profile_names_are_rejected() {
        let error = validate_profile_name("dev/profile").expect_err("invalid profile");
        assert!(error.to_string().contains("unsupported characters"));
    }

    #[test]
    fn discover_sets_profile_on_paths() {
        let paths = AppPaths::discover("dev").expect("discover paths");
        assert_eq!(paths.profile, "dev");
        assert!(paths.config_dir.ends_with("profiles/dev"));
    }

    #[test]
    fn path_matching_normalizes_separators() {
        assert!(paths_match(
            Path::new("/tmp/lemonup/Interface/AddOns"),
            Path::new("/tmp/lemonup//Interface/AddOns/")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn path_matching_is_case_insensitive_on_windows() {
        assert!(paths_match(
            Path::new(r"D:\World of Warcraft\_retail_\Interface\AddOns"),
            Path::new(r"d:\world of warcraft\_retail_\interface\addons")
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn path_matching_is_case_sensitive_off_windows() {
        assert!(!paths_match(
            Path::new("/tmp/LemonUp/Interface/AddOns"),
            Path::new("/tmp/lemonup/Interface/AddOns")
        ));
    }
}
