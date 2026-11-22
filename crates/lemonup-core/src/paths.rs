use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{LemonupError, Result};

pub const DEFAULT_PROFILE: &str = "default";

/// Settings every game shares (Wago.io key, look, auto-check), next to the default
/// profile's `config.toml`.
pub const SHARED_SETTINGS_FILE: &str = "shared.toml";
/// The newest LemonUp release GitHub named, shared by all games, in the default profile's data
/// folder.
pub const LATEST_RELEASE_FILE: &str = "latest-release.json";
const CONFIG_FILE: &str = "config.toml";
const STATE_DB_FILE: &str = "state.sqlite";
const PROFILES_DIR: &str = "profiles";

/// The base folders every profile lives under: the default profile uses them directly,
/// any other profile uses `profiles/<name>` inside each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRoots {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl ProfileRoots {
    pub fn new(config_dir: PathBuf, data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    pub fn discover() -> Result<Self> {
        let project_dirs = ProjectDirs::from("org", "archcorsair", "lemonup")
            .ok_or(LemonupError::PathsUnavailable)?;
        Ok(Self::new(
            project_dirs.config_dir().to_path_buf(),
            project_dirs.data_dir().to_path_buf(),
            project_dirs.cache_dir().to_path_buf(),
        ))
    }

    /// The folders of `profile`, which must be a valid profile name.
    pub fn paths(&self, profile: &str) -> Result<AppPaths> {
        validate_profile_name(profile)?;
        let config_dir = scoped_dir(&self.config_dir, profile);
        let data_dir = scoped_dir(&self.data_dir, profile);
        let cache_dir = scoped_dir(&self.cache_dir, profile);
        let log_dir = data_dir.join("logs");

        Ok(AppPaths {
            profile: profile.to_string(),
            config_file: config_dir.join(CONFIG_FILE),
            state_db_file: data_dir.join(STATE_DB_FILE),
            config_dir,
            data_dir,
            cache_dir,
            log_dir,
            roots: self.clone(),
        })
    }

    pub fn shared_settings_file(&self) -> PathBuf {
        self.config_dir.join(SHARED_SETTINGS_FILE)
    }

    /// Where the newest LemonUp release GitHub named is kept, for every game.
    pub fn latest_release_file(&self) -> PathBuf {
        self.data_dir.join(LATEST_RELEASE_FILE)
    }

    pub fn default_config_file(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    /// The folder holding the config folders of every profile but the default.
    pub fn profiles_config_dir(&self) -> PathBuf {
        self.config_dir.join(PROFILES_DIR)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub profile: String,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub config_file: PathBuf,
    pub state_db_file: PathBuf,
    /// The base folders this profile lives under, for the settings all games share.
    pub roots: ProfileRoots,
}

impl AppPaths {
    pub fn discover(profile: impl Into<String>) -> Result<Self> {
        let profile = profile.into();
        validate_profile_name(&profile)?;
        ProfileRoots::discover()?.paths(&profile)
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }

    /// Where this profile keeps its backups of game settings.
    pub fn backup_root(&self) -> PathBuf {
        backup_root(&self.data_dir)
    }
}

/// The store of backups of game settings under a profile's data folder.
pub fn backup_root(data_dir: &Path) -> PathBuf {
    data_dir.join("backups").join("wtf")
}

pub fn paths_match(left: &Path, right: &Path) -> bool {
    normalize_for_compare(left) == normalize_for_compare(right)
}

/// Whether two AddOns paths name the same folder on disk: symlinks and `..` are resolved
/// when the folder exists, and the comparison ignores case on Windows and macOS, whose
/// default file systems are case-insensitive.
pub fn same_addons_folder(left: &Path, right: &Path) -> bool {
    comparable_folder(left) == comparable_folder(right)
}

fn comparable_folder(path: &Path) -> String {
    let resolved = fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    let rendered = normalize_for_compare(&resolved);
    if cfg!(any(windows, target_os = "macos")) {
        rendered.to_lowercase()
    } else {
        rendered
    }
}

pub fn validate_profile_name(profile: &str) -> Result<()> {
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

    base.join(PROFILES_DIR).join(profile)
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

    use super::{
        AppPaths, DEFAULT_PROFILE, paths_match, same_addons_folder, scoped_dir,
        validate_profile_name,
    };

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
    fn the_newest_release_is_kept_once_for_every_game_in_the_data_folder() {
        let roots = super::ProfileRoots::new(
            PathBuf::from("/c"),
            PathBuf::from("/d"),
            PathBuf::from("/x"),
        );
        assert_eq!(
            roots.latest_release_file(),
            PathBuf::from("/d/latest-release.json")
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

    #[test]
    fn same_addons_folder_resolves_symlinks_and_dot_segments() {
        let temp = tempfile::tempdir().expect("tempdir");
        let real = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&real).expect("create addons");
        let dotted = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("..")
            .join("Interface")
            .join("AddOns");
        assert!(same_addons_folder(&real, &dotted));
        assert!(!same_addons_folder(
            &real,
            &temp
                .path()
                .join("_forever_")
                .join("Interface")
                .join("AddOns")
        ));

        #[cfg(unix)]
        {
            let link = temp.path().join("wow-link");
            std::os::unix::fs::symlink(temp.path().join("_retail_"), &link).expect("symlink");
            assert!(same_addons_folder(
                &real,
                &link.join("Interface").join("AddOns")
            ));
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn same_addons_folder_ignores_case_on_case_insensitive_platforms() {
        assert!(same_addons_folder(
            Path::new("/Missing/World of Warcraft/_retail_/Interface/AddOns"),
            Path::new("/missing/world of warcraft/_RETAIL_/interface/addons")
        ));
    }
}
