use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{LemonupError, Result};
use crate::paths::AppPaths;
use crate::shared::{
    OtherGamesCheck, SharedFields, SharedSecret, SharedSettings, SharedSettingsStore, Writes,
    parse_profile_config,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Dark,
    Light,
    /// The terminal's own background and 16 colors.
    Terminal,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Raises GitHub's limit of 60 requests an hour. Shared by all games.
    pub github_token: Option<String>,
    /// The welcome card on the addon list was hidden with `d`. Configs
    /// written before it existed load as `false`.
    pub hide_first_run_card: bool,
    /// Plain ASCII symbols and the word LEMONUP in place of the block logo.
    pub simple_symbols: bool,
    /// A still symbol in place of the spinner, and no shimmer.
    pub reduce_motion: bool,
    /// How often the games that aren't open are checked. Shared by all games.
    pub check_other_games: OtherGamesCheck,
    /// The TUI looks on GitHub once a day for a newer LemonUp. Shared by all games.
    pub check_lemonup_updates: bool,
    /// Progress shows in the terminal's own progress bar, on terminals that draw one.
    /// Shared by all games. Configs written before it existed load as `true`.
    pub terminal_progress: bool,
    /// The sources the Find sheet looks at in this game: `wago`, `wowinterface`, `tukui`.
    /// Missing means all of them. Only this game's `config.toml` holds it. A value that isn't
    /// a list of names reads as missing, and names LemonUp doesn't know are ignored, so a
    /// newer LemonUp's choice never stops this one from loading.
    #[serde(
        deserialize_with = "lenient_names",
        skip_serializing_if = "Option::is_none"
    )]
    pub find_sources: Option<Vec<String>>,
}

/// The names in a list, whatever else the value holds. Anything that is not a list of strings
/// reads as missing instead of failing the whole config.
fn lenient_names<'de, D>(deserializer: D) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Name(String),
        Other(serde::de::IgnoredAny),
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        List(Vec<Entry>),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::List(entries) => Some(
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    Entry::Name(name) => Some(name),
                    Entry::Other(_) => None,
                })
                .collect(),
        ),
        Value::Other(_) => None,
    })
}

/// Debug output never shows the Wago.io key or the GitHub token, so a log line or a failed
/// assertion can't leak it.
impl fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("schema_version", &self.schema_version)
            .field("addon_dir", &self.addon_dir)
            .field("user_agent", &self.user_agent)
            .field("check_interval_secs", &self.check_interval_secs)
            .field("auto_check_enabled", &self.auto_check_enabled)
            .field("auto_check_interval_secs", &self.auto_check_interval_secs)
            .field("backup_wtf", &self.backup_wtf)
            .field("backup_retention", &self.backup_retention)
            .field("show_libs", &self.show_libs)
            .field("theme", &self.theme)
            .field(
                "wago_api_key",
                &self.wago_api_key.as_ref().map(|_| "<hidden>"),
            )
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "<hidden>"),
            )
            .field("hide_first_run_card", &self.hide_first_run_card)
            .field("simple_symbols", &self.simple_symbols)
            .field("reduce_motion", &self.reduce_motion)
            .field("check_other_games", &self.check_other_games)
            .field("check_lemonup_updates", &self.check_lemonup_updates)
            .field("terminal_progress", &self.terminal_progress)
            .field("find_sources", &self.find_sources)
            .finish()
    }
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
            github_token: None,
            hide_first_run_card: false,
            simple_symbols: false,
            reduce_motion: false,
            check_other_games: OtherGamesCheck::default(),
            check_lemonup_updates: true,
            terminal_progress: true,
            find_sources: None,
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

    /// The same store, giving up on the shared settings lock sooner. For tests.
    #[cfg(test)]
    pub(crate) fn with_shared_lock_wait(mut self, wait: std::time::Duration) -> Self {
        if let Some(link) = &mut self.shared {
            link.store = link.store.clone().with_lock_wait(wait);
        }
        self
    }

    pub fn shared_store(&self) -> Option<&SharedSettingsStore> {
        self.shared.as_ref().map(|link| &link.store)
    }

    /// The profile's settings. A file that can't be read or parsed is an error naming it.
    pub fn load(&self) -> Result<ConfigLoad> {
        self.load_with(Writes::Allowed)
    }

    /// Like [`ConfigStore::load`], but never writes `shared.toml`, for reads in the
    /// background: two writers could undo each other's change there.
    pub fn load_without_writing(&self) -> Result<ConfigLoad> {
        self.load_with(Writes::Never)
    }

    fn load_with(&self, writes: Writes) -> Result<ConfigLoad> {
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
            let settings = link
                .store
                .load_for_profile(&link.profile, &in_file, writes)?;
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

    /// Puts the shared values as they are now into `config`, and starts from them: the
    /// next save takes as changed the shared values that differ from these. For showing
    /// Settings as they are stored. Never writes.
    pub fn refresh_shared(&self, config: &mut AppConfig) -> Result<()> {
        if let Some(link) = &self.shared {
            let settings = link.store.current_without_writing()?;
            settings.apply_to(config);
            link.remember(settings);
        }
        Ok(())
    }

    /// Saves `config` and returns it as stored: callers keep that one, so what they show
    /// and save next is what is stored.
    ///
    /// `shared.toml` is read again and gets only the shared values changed in this game
    /// since this store last read or saved them (all of them when it hasn't). The profile's
    /// file gets every field, the shared ones as `shared.toml` now holds them, so an older
    /// LemonUp can read it.
    ///
    /// While another game's config can't be read, `shared.toml` can't be made, and a save
    /// that changes a shared value is refused, writing nothing: the change would be lost.
    ///
    /// `shared.toml` is written first. When the profile's file then can't be written, the
    /// old `shared.toml` is put back and the save fails, so nothing reports a save that
    /// didn't happen. A crash between the two writes, or a failed put back, can still leave
    /// the new shared values with the old profile file. That file then differs from what
    /// `shared.toml` records LemonUp wrote into it, so the next load takes its values as an
    /// older LemonUp's change and keeps them: the save that never finished is undone.
    ///
    /// Removing the Wago.io key or the GitHub token also removes the copies of it LemonUp
    /// wrote into other games' configs.
    pub fn write_config(&self, config: &AppConfig) -> Result<AppConfig> {
        let Some(link) = &self.shared else {
            let rendered = toml::to_string_pretty(config)?;
            write_private_file(&self.path, &rendered)?;
            return Ok(config.clone());
        };

        // One LemonUp at a time from the read to the last write, the rollback included: two
        // saves that read the same `shared.toml` would drop one of the changes.
        let lock = link.store.lock()?;
        let current = link.store.current_locked(&lock)?;
        let mut merged = current.settings.clone();
        merged.take_changes(config, link.seen().as_ref());
        let mut file_config = config.clone();
        merged.apply_to(&mut file_config);
        let rendered = toml::to_string_pretty(&file_config)?;

        if current.may_write() {
            merged
                .profile_copies
                .insert(link.profile.clone(), SharedFields::of(&merged));
            link.store.write_locked(&lock, &merged)?;
            if let Err(error) = write_private_file(&self.path, &rendered) {
                link.store
                    .restore(&lock, current.on_disk.then_some(&current.settings));
                return Err(error);
            }
            for secret in SharedSecret::ALL {
                if secret.in_settings(&current.settings).is_some()
                    && secret.in_settings(&merged).is_none()
                {
                    link.store.clear_key_copies(&lock, &link.profile, secret);
                }
            }
        } else if SharedFields::of(&merged) != SharedFields::of(&current.settings) {
            return Err(LemonupError::SharedChangeWaits {
                paths: current.waiting_for,
            });
        } else {
            write_private_file(&self.path, &rendered)?;
        }
        link.remember(merged);
        Ok(file_config)
    }

    pub fn write_new_config(&self, config: &AppConfig) -> Result<AppConfig> {
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
    #[test]
    fn debug_output_hides_the_wago_key() {
        let mut config = AppConfig::new_unconfigured();
        config.wago_api_key = Some("super-secret-key".to_string());
        let shown = format!("{config:?} {config:#?}");
        assert!(!shown.contains("super-secret-key"), "{shown}");
        assert!(shown.contains("<hidden>"));
        let none = format!("{:?}", AppConfig::new_unconfigured());
        assert!(none.contains("wago_api_key: None"), "{none}");
    }

    #[test]
    fn debug_output_hides_the_github_token() {
        let mut config = AppConfig::new_unconfigured();
        config.github_token = Some("ghp_super_secret_token".to_string());
        let shown = format!("{config:?} {config:#?}");
        assert!(!shown.contains("ghp_super_secret_token"), "{shown}");
        assert!(
            shown.contains("github_token: Some(\"<hidden>\")"),
            "{shown}"
        );
        let none = format!("{:?}", AppConfig::new_unconfigured());
        assert!(none.contains("github_token: None"), "{none}");
    }

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
                // The terminal progress bar is a setting again: a choice made back then is
                // kept.
                assert!(!config.terminal_progress);
            }
            ConfigLoad::Missing(_) => panic!("legacy config should load"),
        }
    }
}

#[cfg(test)]
mod find_sources_tests {
    use tempfile::tempdir;

    use super::{AppConfig, ConfigLoad, ConfigStore};

    fn loaded(store: &ConfigStore) -> AppConfig {
        match store.load().expect("load config") {
            ConfigLoad::Loaded(config) => config,
            ConfigLoad::Missing(_) => panic!("config should load"),
        }
    }

    #[test]
    fn find_sources_round_trip_as_a_list_and_missing_means_none_saved() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());

        let mut config = AppConfig::new_unconfigured();
        assert_eq!(config.find_sources, None);
        store.write_config(&config).expect("write config");
        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(!raw.contains("find_sources"), "{raw}");

        config.find_sources = Some(vec!["wowinterface".to_string(), "tukui".to_string()]);
        store.write_config(&config).expect("write config");
        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(
            raw.contains("find_sources = [\n    \"wowinterface\",\n    \"tukui\",\n]"),
            "{raw}"
        );
        assert_eq!(loaded(&store), config);

        // A config written before the field existed.
        std::fs::write(&path, "schema_version = 1\ntheme = \"dark\"\n").expect("write old");
        assert_eq!(loaded(&store).find_sources, None);
    }

    #[test]
    fn an_unreadable_find_sources_value_never_breaks_loading_the_config() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let store = ConfigStore::new(path.clone());
        for (value, expected) in [
            ("5", None),
            ("\"wago\"", None),
            ("{ a = 1 }", None),
            ("[1, \"tukui\", true]", Some(vec!["tukui".to_string()])),
            (
                "[\"curseforge\", \"wago\"]",
                Some(vec!["curseforge".to_string(), "wago".to_string()]),
            ),
        ] {
            std::fs::write(
                &path,
                format!("schema_version = 1\nshow_libs = true\nfind_sources = {value}\n"),
            )
            .expect("write config");
            let config = loaded(&store);
            assert_eq!(config.find_sources, expected, "{value}");
            assert!(config.show_libs, "{value}");
        }
    }
}
