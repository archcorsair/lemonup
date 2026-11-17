//! Settings every game shares.
//!
//! Each profile (one per game) keeps its own `config.toml` for what belongs to that game: its
//! AddOns folder, backups of game settings and how many to keep, show libraries and the
//! welcome card. The Wago.io key, the look (theme, symbols, motion) and how checks run are
//! the same for every game, so they live once in `shared.toml` next to the default profile's
//! config, and they win over the copies in each profile's config.
//!
//! Migration: the first time a profile's settings are read or written and `shared.toml` does
//! not exist, it is created from the configs of every game. Each value comes from the default
//! profile's config when that config has it; else from the first other profile's config that
//! has it, by profile name (`profiles/<name>`, sorted). A value a config doesn't have, or a
//! Wago.io key it doesn't have, never replaces one another config has. When no config exists
//! yet, the defaults are used without writing `shared.toml`: it is never written from the
//! defaults alone. Nothing is removed from any profile's config.
//!
//! Every save still writes the shared values into the profile's own config too, so an older
//! LemonUp that knows nothing of `shared.toml` keeps working with the values that profile last
//! saved. `shared.toml` also records, per profile, the shared values LemonUp last wrote into
//! that profile's config. When the config holds other values later, an older LemonUp changed
//! them there, and they are taken as the newer ones. Removing a game drops its record, and
//! removing the Wago.io key removes the copies of it LemonUp wrote into every game's config.
//!
//! While another game's config can't be read, `shared.toml` isn't created, and a change to a
//! shared value waits: the save is refused, naming that file.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, ThemeMode, write_private_file};
use crate::error::{LemonupError, Result};
use crate::paths::{DEFAULT_PROFILE, ProfileRoots};

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

/// The shared values as one profile's `config.toml` holds them. A value the file doesn't
/// have is `None`, and so is a missing or empty Wago.io key.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wago_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simple_symbols: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_check_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_check_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_other_games: Option<OtherGamesCheck>,
}

impl SharedFields {
    /// The shared values in a profile's config file.
    pub fn parse(raw: &str) -> std::result::Result<Self, toml::de::Error> {
        let mut fields = toml::from_str::<Self>(raw)?;
        fields.wago_api_key = fields.wago_api_key.filter(|key| !key.trim().is_empty());
        Ok(fields)
    }

    /// Every shared value of `settings`, as a save writes them into a profile's config.
    pub fn of(settings: &SharedSettings) -> Self {
        Self {
            wago_api_key: settings.wago_api_key.clone(),
            theme: Some(settings.theme),
            simple_symbols: Some(settings.simple_symbols),
            reduce_motion: Some(settings.reduce_motion),
            auto_check_enabled: Some(settings.auto_check_enabled),
            auto_check_interval_secs: Some(settings.auto_check_interval_secs),
            check_interval_secs: Some(settings.check_interval_secs),
            check_other_games: Some(settings.check_other_games),
        }
    }

    /// Takes each value `self` doesn't have from `other`.
    fn fill_from(&mut self, other: &Self) {
        fn fill<T: Clone>(mine: &mut Option<T>, theirs: &Option<T>) {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
        fill(&mut self.wago_api_key, &other.wago_api_key);
        fill(&mut self.theme, &other.theme);
        fill(&mut self.simple_symbols, &other.simple_symbols);
        fill(&mut self.reduce_motion, &other.reduce_motion);
        fill(&mut self.auto_check_enabled, &other.auto_check_enabled);
        fill(
            &mut self.auto_check_interval_secs,
            &other.auto_check_interval_secs,
        );
        fill(&mut self.check_interval_secs, &other.check_interval_secs);
        fill(&mut self.check_other_games, &other.check_other_games);
    }

    /// The values `self` has that differ from `written`. A value `self` doesn't have is not
    /// a change: an older LemonUp leaves out what it doesn't know, and a key is never
    /// removed by guessing.
    fn changed_since(&self, written: &Self) -> Self {
        fn changed<T: Clone + PartialEq>(mine: &Option<T>, written: &Option<T>) -> Option<T> {
            mine.clone().filter(|value| written.as_ref() != Some(value))
        }
        Self {
            wago_api_key: changed(&self.wago_api_key, &written.wago_api_key),
            theme: changed(&self.theme, &written.theme),
            simple_symbols: changed(&self.simple_symbols, &written.simple_symbols),
            reduce_motion: changed(&self.reduce_motion, &written.reduce_motion),
            auto_check_enabled: changed(&self.auto_check_enabled, &written.auto_check_enabled),
            auto_check_interval_secs: changed(
                &self.auto_check_interval_secs,
                &written.auto_check_interval_secs,
            ),
            check_interval_secs: changed(&self.check_interval_secs, &written.check_interval_secs),
            check_other_games: changed(&self.check_other_games, &written.check_other_games),
        }
    }

    /// Puts the values `self` has into `settings`.
    fn apply_to(&self, settings: &mut SharedSettings) {
        if let Some(key) = &self.wago_api_key {
            settings.wago_api_key = Some(key.clone());
        }
        if let Some(theme) = self.theme {
            settings.theme = theme;
        }
        if let Some(value) = self.simple_symbols {
            settings.simple_symbols = value;
        }
        if let Some(value) = self.reduce_motion {
            settings.reduce_motion = value;
        }
        if let Some(value) = self.auto_check_enabled {
            settings.auto_check_enabled = value;
        }
        if let Some(value) = self.auto_check_interval_secs {
            settings.auto_check_interval_secs = value;
        }
        if let Some(value) = self.check_interval_secs {
            settings.check_interval_secs = value;
        }
        if let Some(value) = self.check_other_games {
            settings.check_other_games = value;
        }
    }

    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Never prints the Wago.io key, only whether there is one.
impl fmt::Debug for SharedFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedFields")
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
            .finish()
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
    /// Per profile, the shared values LemonUp last wrote into its `config.toml`. Values
    /// there that differ were changed by an older LemonUp.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub profile_copies: BTreeMap<String, SharedFields>,
}

impl SharedSettings {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    /// The shared values of `config`, with no last used game.
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            wago_api_key: config.wago_api_key.clone(),
            theme: config.theme,
            simple_symbols: config.simple_symbols,
            reduce_motion: config.reduce_motion,
            auto_check_enabled: config.auto_check_enabled,
            auto_check_interval_secs: config.auto_check_interval_secs,
            check_interval_secs: config.check_interval_secs,
            check_other_games: config.check_other_games,
            last_used_profile: None,
            profile_copies: BTreeMap::new(),
        }
    }

    /// Takes from `config` each shared value that differs from `seen`, the values `config`
    /// started from: those are the ones changed since. Without `seen`, takes them all. Every
    /// other value stays as it is here, even when `config` still holds an older one.
    pub fn take_changes(&mut self, config: &AppConfig, seen: Option<&SharedSettings>) {
        let changed = |differs: fn(&SharedSettings, &AppConfig) -> bool| {
            seen.is_none_or(|seen| differs(seen, config))
        };
        if changed(|seen, config| seen.wago_api_key != config.wago_api_key) {
            self.wago_api_key = config.wago_api_key.clone();
        }
        if changed(|seen, config| seen.theme != config.theme) {
            self.theme = config.theme;
        }
        if changed(|seen, config| seen.simple_symbols != config.simple_symbols) {
            self.simple_symbols = config.simple_symbols;
        }
        if changed(|seen, config| seen.reduce_motion != config.reduce_motion) {
            self.reduce_motion = config.reduce_motion;
        }
        if changed(|seen, config| seen.auto_check_enabled != config.auto_check_enabled) {
            self.auto_check_enabled = config.auto_check_enabled;
        }
        if changed(|seen, config| seen.auto_check_interval_secs != config.auto_check_interval_secs)
        {
            self.auto_check_interval_secs = config.auto_check_interval_secs;
        }
        if changed(|seen, config| seen.check_interval_secs != config.check_interval_secs) {
            self.check_interval_secs = config.check_interval_secs;
        }
        if changed(|seen, config| seen.check_other_games != config.check_other_games) {
            self.check_other_games = config.check_other_games;
        }
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
            .field("profile_copies", &self.profile_copies)
            .finish()
    }
}

/// The shared settings as they are now, and whether `shared.toml` holds them.
#[derive(Debug, Clone)]
pub(crate) struct CurrentShared {
    pub(crate) settings: SharedSettings,
    /// `shared.toml` exists and holds `settings`.
    pub(crate) on_disk: bool,
    /// The configs of other games that couldn't be read for the migration. While there are
    /// any, `shared.toml` isn't written (their values would be lost), and a change to a
    /// shared value can't be saved.
    pub(crate) waiting_for: Vec<PathBuf>,
}

impl CurrentShared {
    /// A save may write `shared.toml`.
    pub(crate) fn may_write(&self) -> bool {
        self.waiting_for.is_empty()
    }
}

/// Whether reading the shared settings may write `shared.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Writes {
    Allowed,
    /// For reads in the background: two writers could undo each other's change.
    Never,
}

/// What the migration found in the configs of every game.
struct Migration {
    settings: SharedSettings,
    /// At least one game has a config.
    found_any: bool,
    /// Configs of other games that couldn't be read.
    skipped: Vec<PathBuf>,
}

/// Reads and writes `shared.toml`, creating it from the existing settings the first time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSettingsStore {
    path: PathBuf,
    roots: ProfileRoots,
}

impl SharedSettingsStore {
    pub fn for_roots(roots: &ProfileRoots) -> Self {
        Self {
            path: roots.shared_settings_file(),
            roots: roots.clone(),
        }
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

    /// The shared settings, creating `shared.toml` first when it doesn't exist and a game has
    /// a config to create it from (see the module docs for where each value comes from).
    ///
    /// A file that exists but can't be read is an error, and then nothing is written:
    /// defaults never replace real values. The default profile's config must be readable;
    /// another game's config that can't be read is skipped, and then `shared.toml` isn't
    /// written, so the next start tries again.
    pub fn load_or_migrate(&self) -> Result<SharedSettings> {
        Ok(self.current()?.settings)
    }

    pub(crate) fn current(&self) -> Result<CurrentShared> {
        self.current_with(Writes::Allowed)
    }

    /// Like [`Self::current`], but with `Writes::Never` it doesn't create `shared.toml`.
    fn current_with(&self, writes: Writes) -> Result<CurrentShared> {
        if let Some(settings) = self.read()? {
            return Ok(CurrentShared {
                settings,
                on_disk: true,
                waiting_for: Vec::new(),
            });
        }

        let migration = self.migrate()?;
        let complete = migration.skipped.is_empty();
        if writes == Writes::Never {
            return Ok(CurrentShared {
                settings: migration.settings,
                on_disk: false,
                waiting_for: migration.skipped,
            });
        }
        if migration.found_any && complete {
            self.write(&migration.settings)?;
            tracing::info!(path = %self.path.display(), "created the settings shared by all games");
        } else if !complete {
            tracing::warn!(
                skipped = ?migration.skipped,
                "the settings shared by all games wait until every game's settings can be read"
            );
        }
        Ok(CurrentShared {
            settings: migration.settings,
            on_disk: migration.found_any && complete,
            waiting_for: migration.skipped,
        })
    }

    fn migrate(&self) -> Result<Migration> {
        let mut values = SharedFields::default();
        let mut copies = BTreeMap::new();
        let mut skipped = Vec::new();

        let default_config = self.roots.default_config_file();
        if default_config.try_exists()? {
            let fields = read_profile_fields(&default_config).map_err(|error| match error {
                LemonupError::SettingsUnreadable { path, reason } => {
                    LemonupError::DefaultSettingsUnreadable { path, reason }
                }
                other => other,
            })?;
            values.fill_from(&fields);
            copies.insert(DEFAULT_PROFILE.to_string(), fields);
        }
        // Sorted by name, so the same configs always give the same values.
        for name in crate::profiles::other_profile_names(&self.roots)? {
            let config_file = self.roots.paths(&name)?.config_file;
            match read_profile_fields(&config_file) {
                Ok(fields) => {
                    values.fill_from(&fields);
                    copies.insert(name, fields);
                }
                Err(error) => {
                    tracing::warn!(%error, "couldn't read a game's settings for the shared settings");
                    skipped.push(config_file);
                }
            }
        }

        let found_any = !copies.is_empty() || !skipped.is_empty();
        let mut settings = SharedSettings::default();
        values.apply_to(&mut settings);
        settings.profile_copies = copies;
        Ok(Migration {
            settings,
            found_any,
            skipped,
        })
    }

    /// The shared settings for opening `profile`, whose config holds `in_file`. Values there
    /// that differ from what LemonUp last wrote into it were changed by an older LemonUp and
    /// are taken as the newer ones. With `Writes::Never` nothing is written: neither
    /// `shared.toml`'s creation nor the record of what the config holds.
    pub(crate) fn load_for_profile(
        &self,
        profile: &str,
        in_file: &SharedFields,
        writes: Writes,
    ) -> Result<SharedSettings> {
        let current = self.current_with(writes)?;
        let mut settings = current.settings;
        if !current.on_disk || settings.profile_copies.get(profile) == Some(in_file) {
            return Ok(settings);
        }
        // With no record yet, what the file holds now becomes the record.
        let newer = settings
            .profile_copies
            .get(profile)
            .map(|written| in_file.changed_since(written))
            .unwrap_or_default();
        newer.apply_to(&mut settings);
        settings
            .profile_copies
            .insert(profile.to_string(), in_file.clone());
        if writes == Writes::Never {
            return Ok(settings);
        }
        match self.write(&settings) {
            Ok(()) if !newer.is_empty() => {
                tracing::info!(profile, "kept the shared settings an older LemonUp changed");
            }
            Ok(()) => {}
            Err(error) => {
                tracing::warn!(%error, profile, "couldn't record this game's shared settings");
            }
        }
        Ok(settings)
    }

    /// Replaces `shared.toml` in one step (written beside it, then renamed), readable only by
    /// its owner on Unix because it can hold the Wago.io key. Refuses settings a newer
    /// LemonUp saved: it could drop what this one doesn't know.
    pub fn write(&self, settings: &SharedSettings) -> Result<()> {
        if settings.schema_version > SharedSettings::CURRENT_SCHEMA_VERSION {
            return Err(LemonupError::SettingsTooNew {
                path: self.path.clone(),
            });
        }
        let rendered = toml::to_string_pretty(settings)?;
        write_private_file(&self.path, &rendered)
    }

    /// Puts back what `shared.toml` held before a save that couldn't finish: `previous`, or
    /// no file. Best effort; the save already failed.
    pub(crate) fn restore(&self, previous: Option<&SharedSettings>) {
        let restored = match previous {
            Some(settings) => self.write(settings),
            None => std::fs::remove_file(&self.path).map_err(LemonupError::from),
        };
        if let Err(error) = restored {
            tracing::warn!(%error, "couldn't put the shared settings back after a failed save");
        }
    }

    /// The shared settings as they are now, to show them. Never writes.
    pub(crate) fn current_without_writing(&self) -> Result<SharedSettings> {
        Ok(self.current_with(Writes::Never)?.settings)
    }

    /// After a save removed the Wago.io key: removes the copy of it from every other game's
    /// config that still holds the copy LemonUp wrote there, then from the record of that
    /// copy. A config that holds another key (an older LemonUp changed it) keeps it; its
    /// record loses the key either way, since that key differs from both. A config that
    /// can't be read or written keeps its record as it is: a record without the key next to
    /// a config with it would bring the key back as an older LemonUp's change. Best effort;
    /// the save is already done.
    pub(crate) fn clear_key_copies(&self, saved_by: &str) {
        let Ok(Some(mut settings)) = self.read() else {
            return;
        };
        let mut cleared = false;
        for (profile, record) in &mut settings.profile_copies {
            if profile == saved_by || record.wago_api_key.is_none() {
                continue;
            }
            match self.clear_key_copy(profile, record) {
                Ok(()) => {
                    record.wago_api_key = None;
                    cleared = true;
                }
                Err(error) => {
                    tracing::warn!(%error, profile, "kept the Wago.io key in a game's settings");
                }
            }
        }
        if cleared && let Err(error) = self.write(&settings) {
            tracing::warn!(%error, "couldn't record the Wago.io key removal for every game");
        }
    }

    /// Removes the key from `profile`'s config when it holds the copy `record` says LemonUp
    /// wrote there. `Ok` once the config no longer holds that copy.
    fn clear_key_copy(&self, profile: &str, record: &SharedFields) -> Result<()> {
        let path = self.roots.paths(profile)?.config_file;
        if !path.try_exists()? {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&path)?;
        let in_file = parse_profile_config(&path, &raw)?.1;
        if in_file.wago_api_key != record.wago_api_key {
            return Ok(());
        }
        let mut table = raw.parse::<toml::Table>()?;
        table.remove("wago_api_key");
        write_private_file(&path, &toml::to_string_pretty(&table)?)
    }

    /// After `profile` was removed: forgets what LemonUp wrote into its config, the
    /// Wago.io key included. Best effort; the game is already removed.
    pub(crate) fn forget_profile(&self, profile: &str) {
        let forget = || -> Result<()> {
            let Some(mut settings) = self.read()? else {
                return Ok(());
            };
            // Also a record in another case whose config is gone, like Windows and macOS
            // find the folder without case.
            let mut gone = Vec::new();
            for name in settings.profile_copies.keys() {
                if name == profile
                    || (name.eq_ignore_ascii_case(profile)
                        && !self.roots.paths(name)?.config_file.try_exists()?)
                {
                    gone.push(name.clone());
                }
            }
            if gone.is_empty() {
                return Ok(());
            }
            for name in gone {
                settings.profile_copies.remove(&name);
            }
            self.write(&settings)
        };
        if let Err(error) = forget() {
            tracing::warn!(%error, profile, "couldn't forget a removed game's shared settings");
        }
    }

    /// Remembers `profile` as the game the TUI opened last. Writes nothing while no game has
    /// settings yet: `shared.toml` is never made from the defaults alone.
    pub fn remember_last_used(&self, profile: &str) -> Result<()> {
        let current = self.current()?;
        let mut settings = current.settings;
        if !current.on_disk || settings.last_used_profile.as_deref() == Some(profile) {
            return Ok(());
        }
        settings.last_used_profile = Some(profile.to_string());
        self.write(&settings)
    }

    /// After the v1 migration wrote the default profile's config from `config`: when
    /// `shared.toml` already exists, takes the key from `config` if it has none, and every
    /// other value it doesn't have. The values it has stay.
    pub fn adopt_v1_config(&self, config: &AppConfig) -> Result<()> {
        let Some(mut settings) = self.read()? else {
            return Ok(());
        };
        let raw = std::fs::read_to_string(&self.path)?;
        let mut present =
            SharedFields::parse(&raw).map_err(|error| LemonupError::SettingsUnreadable {
                path: self.path.clone(),
                reason: error.message().to_string(),
            })?;
        present.wago_api_key = settings.wago_api_key.clone();
        let mut from_v1 = SharedFields::of(&SharedSettings::from_config(config));
        from_v1.wago_api_key = from_v1.wago_api_key.filter(|key| !key.trim().is_empty());
        present.fill_from(&from_v1);
        present.apply_to(&mut settings);
        settings
            .profile_copies
            .insert(DEFAULT_PROFILE.to_string(), from_v1);
        self.write(&settings)
    }
}

/// The shared values in the profile config at `path`, which must read as a whole config.
fn read_profile_fields(path: &Path) -> Result<SharedFields> {
    let raw = std::fs::read_to_string(path).map_err(|error| LemonupError::SettingsUnreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    parse_profile_config(path, &raw).map(|(_, fields)| fields)
}

/// A profile's config and the shared values it holds, or why it can't be read.
pub(crate) fn parse_profile_config(path: &Path, raw: &str) -> Result<(AppConfig, SharedFields)> {
    let unreadable = |error: toml::de::Error| LemonupError::SettingsUnreadable {
        path: path.to_path_buf(),
        reason: error.message().to_string(),
    };
    let config = toml::from_str::<AppConfig>(raw).map_err(unreadable)?;
    let fields = SharedFields::parse(raw).map_err(unreadable)?;
    Ok((config, fields))
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

    use super::{OtherGamesCheck, SharedFields, SharedSettings, SharedSettingsStore};
    use crate::config::{AppConfig, ConfigLoad, ConfigStore, ThemeMode, temp_path_for};
    use crate::paths::{DEFAULT_PROFILE, ProfileRoots};

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
        let settings = store.load_or_migrate().expect("migrate");

        let mut expected = SharedSettings::from_config(&default_config);
        expected.last_used_profile = None;
        // It records what the default game's config holds, to notice an older LemonUp's
        // changes there later.
        expected.profile_copies.insert(
            DEFAULT_PROFILE.to_string(),
            SharedFields::of(&SharedSettings::from_config(&default_config)),
        );
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
    fn with_no_config_anywhere_the_defaults_are_used_and_nothing_is_written() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let store = SharedSettingsStore::for_roots(&roots);
        assert!(store.load_or_migrate().expect("migrate") == SharedSettings::default());
        assert!(!store.path().exists());
        // Remembering the last game doesn't make it from the defaults either.
        store.remember_last_used("forever").expect("remember");
        assert!(!store.path().exists());
        // Nor does what plain `lemonup` and the Wago.io key lookup read for a game not
        // set up yet.
        let default = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("paths"));
        let state = default.load().expect("load");
        default.effective(&state).expect("effective");
        assert!(!store.path().exists());
    }

    #[test]
    fn migration_without_a_default_config_uses_another_games_config() {
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
        let settings = store.load_or_migrate().expect("migrate");
        assert_eq!(settings.theme, ThemeMode::Light);
        assert!(settings.wago_api_key.as_deref() == Some(SECRET));
        assert!(
            store.read().expect("read").expect("written").wago_api_key == settings.wago_api_key
        );
    }

    /// Reviewer's scenario B: only a non-default game is set up, and plain `lemonup` opens
    /// the default game (not set up) first.
    #[test]
    fn opening_the_unset_default_game_first_keeps_another_games_key_and_look() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = roots.paths("forever").expect("paths");
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some("/g/_forever_/Interface/AddOns".into());
        config.wago_api_key = Some(SECRET.to_string());
        config.theme = ThemeMode::Light;
        ConfigStore::new(forever.config_file.clone())
            .write_config(&config)
            .expect("write forever");

        // What App::bootstrap and the Wago.io key lookup do for the default game.
        let default = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("paths"));
        let state = default.load().expect("load");
        let seen = default.effective(&state).expect("effective");
        assert!(seen.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(seen.theme, ThemeMode::Light);

        let loaded = loaded(&ConfigStore::for_profile(&forever));
        assert!(loaded.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(loaded.theme, ThemeMode::Light);
    }

    /// Reviewer's scenario A: the key was saved only in a non-default game.
    #[test]
    fn a_key_saved_only_in_another_game_is_kept_by_the_migration() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let mut retail = AppConfig::new_unconfigured();
        retail.addon_dir = Some("/g/_retail_/Interface/AddOns".into());
        retail.theme = ThemeMode::Terminal;
        ConfigStore::new(roots.default_config_file())
            .write_config(&retail)
            .expect("write default");
        let forever = roots.paths("forever").expect("paths");
        let mut own = AppConfig::new_unconfigured();
        own.addon_dir = Some("/g/_forever_/Interface/AddOns".into());
        own.wago_api_key = Some(SECRET.to_string());
        own.theme = ThemeMode::Light;
        ConfigStore::new(forever.config_file.clone())
            .write_config(&own)
            .expect("write forever");

        let store = ConfigStore::for_profile(&forever);
        let mut config = loaded(&store);
        assert!(config.wago_api_key.as_deref() == Some(SECRET));
        // The default game's set values win.
        assert_eq!(config.theme, ThemeMode::Terminal);
        config.hide_first_run_card = true;
        store.write_config(&config).expect("save");
        assert!(own_key(&forever.config_file).as_deref() == Some(SECRET));
        let shared = SharedSettingsStore::for_roots(&roots)
            .read()
            .expect("read")
            .expect("exists");
        assert!(shared.wago_api_key.as_deref() == Some(SECRET));
    }

    #[test]
    fn migration_takes_a_missing_value_from_the_first_other_game_by_name() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        for (name, key) in [("zeta", "zeta-key"), ("alpha", "alpha-key")] {
            let mut config = AppConfig::new_unconfigured();
            config.wago_api_key = Some(key.to_string());
            ConfigStore::new(roots.paths(name).expect("paths").config_file)
                .write_config(&config)
                .expect("write");
        }
        // The default game's file has only some of the values, and an empty key.
        fs::create_dir_all(&roots.config_dir).expect("config dir");
        fs::write(
            roots.default_config_file(),
            "wago_api_key = \"\"\ntheme = \"terminal\"\n",
        )
        .expect("write default");

        let settings = SharedSettingsStore::for_roots(&roots)
            .load_or_migrate()
            .expect("migrate");
        assert!(settings.wago_api_key.as_deref() == Some("alpha-key"));
        assert_eq!(settings.theme, ThemeMode::Terminal);
    }

    #[test]
    fn an_unreadable_other_game_skips_the_migration_until_it_can_be_read() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let mut retail = AppConfig::new_unconfigured();
        retail.theme = ThemeMode::Light;
        ConfigStore::new(roots.default_config_file())
            .write_config(&retail)
            .expect("write default");
        let forever = roots.paths("forever").expect("paths");
        fs::create_dir_all(&forever.config_dir).expect("dir");
        fs::write(&forever.config_file, "wago_api_key = [").expect("broken");

        let default = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("paths"));
        let mut config = loaded(&default);
        assert_eq!(config.theme, ThemeMode::Light);
        config.show_libs = true;
        default.write_config(&config).expect("save this game");
        let store = SharedSettingsStore::for_roots(&roots);
        assert!(!store.path().exists(), "it waits for WoW Forever");

        let mut fixed = AppConfig::new_unconfigured();
        fixed.wago_api_key = Some(SECRET.to_string());
        ConfigStore::new(forever.config_file.clone())
            .write_config(&fixed)
            .expect("fix forever");
        let settings = store.load_or_migrate().expect("migrate");
        assert!(settings.wago_api_key.as_deref() == Some(SECRET));
        assert!(store.path().exists());
    }

    #[test]
    fn an_unreadable_default_config_says_it_is_the_default_games() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        fs::create_dir_all(&roots.config_dir).expect("config dir");
        fs::write(roots.default_config_file(), "theme = [").expect("broken");
        let forever = roots.paths("forever").expect("paths");
        ConfigStore::new(forever.config_file.clone())
            .write_config(&AppConfig::new_unconfigured())
            .expect("write forever");

        let error = ConfigStore::for_profile(&forever)
            .load()
            .expect_err("must not migrate");
        let shown = error.to_string();
        assert!(
            shown.starts_with(&format!(
                "couldn't read the settings of your default game at {}",
                roots.default_config_file().display()
            )),
            "{shown}"
        );
        assert!(
            shown.ends_with("fix that file, then run LemonUp again"),
            "{shown}"
        );
    }

    /// Reviewer's scenario C: two LemonUp windows, each holding the shared values it read.
    #[test]
    fn a_save_keeps_the_shared_values_another_game_saved_since() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let default_paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let forever_paths = roots.paths("forever").expect("paths");
        let mut retail_own = AppConfig::new_unconfigured();
        retail_own.addon_dir = Some("/g/_retail_/Interface/AddOns".into());
        ConfigStore::new(default_paths.config_file.clone())
            .write_config(&retail_own)
            .expect("write default");
        let mut forever_own = retail_own.clone();
        forever_own.addon_dir = Some("/g/_forever_/Interface/AddOns".into());
        ConfigStore::new(forever_paths.config_file.clone())
            .write_config(&forever_own)
            .expect("write forever");

        let retail_store = ConfigStore::for_profile(&default_paths);
        let forever_store = ConfigStore::for_profile(&forever_paths);
        let mut retail = loaded(&retail_store);
        let mut forever = loaded(&forever_store);
        forever.wago_api_key = Some(SECRET.to_string());
        forever.theme = ThemeMode::Light;
        forever_store.write_config(&forever).expect("save forever");
        retail.hide_first_run_card = true;
        retail.reduce_motion = true;
        retail_store.write_config(&retail).expect("save retail");

        let shared = SharedSettingsStore::for_roots(&roots)
            .read()
            .expect("read")
            .expect("exists");
        assert!(shared.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(shared.theme, ThemeMode::Light);
        // Retail's own change still lands.
        assert!(shared.reduce_motion);
        // Retail's file holds the shared values as they are now, for an older LemonUp.
        assert!(own_key(&default_paths.config_file).as_deref() == Some(SECRET));

        // Removing the key in one game removes it for all.
        let mut forever = loaded(&forever_store);
        forever.wago_api_key = None;
        forever_store.write_config(&forever).expect("remove key");
        assert!(loaded(&retail_store).wago_api_key.is_none());
    }

    /// Reviewer's scenario D: LemonUp 2.2 changed the key in the profile's own file.
    #[test]
    fn a_shared_value_an_older_lemonup_changed_is_kept() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some("/g/_retail_/Interface/AddOns".into());
        config.wago_api_key = Some("old-key".to_string());
        config.check_other_games = OtherGamesCheck::ThreeHours;
        let store = ConfigStore::for_profile(&paths);
        store.write_config(&config).expect("save");

        // LemonUp 2.2 rewrites the file it read, without the settings it doesn't know.
        let mut table = fs::read_to_string(&paths.config_file)
            .expect("read")
            .parse::<toml::Table>()
            .expect("toml");
        table.remove("check_other_games");
        table.insert("wago_api_key".into(), "new-key".into());
        table.insert("theme".into(), "light".into());
        fs::write(&paths.config_file, toml::to_string(&table).expect("toml")).expect("write");

        let store = ConfigStore::for_profile(&paths);
        let mut config = loaded(&store);
        assert!(config.wago_api_key.as_deref() == Some("new-key"));
        assert_eq!(config.theme, ThemeMode::Light);
        // What 2.2 left out is not a change.
        assert_eq!(config.check_other_games, OtherGamesCheck::ThreeHours);
        config.show_libs = true;
        store.write_config(&config).expect("save again");
        assert!(own_key(&paths.config_file).as_deref() == Some("new-key"));

        // Read once, the change is not taken again over a newer one from another game.
        let forever = ConfigStore::for_profile(&roots.paths("forever").expect("paths"));
        let mut other = forever.unconfigured().expect("unconfigured");
        other.wago_api_key = Some(SECRET.to_string());
        forever.write_config(&other).expect("save forever");
        assert!(
            loaded(&ConfigStore::for_profile(&paths))
                .wago_api_key
                .as_deref()
                == Some(SECRET)
        );
    }

    #[test]
    fn a_failed_profile_write_puts_the_shared_settings_back() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let store = ConfigStore::for_profile(&paths);
        store
            .write_config(&AppConfig::new_unconfigured())
            .expect("first save");
        let shared = SharedSettingsStore::for_roots(&roots);
        let before = shared.read().expect("read");

        let mut config = loaded(&store);
        config.theme = ThemeMode::Light;
        fs::create_dir_all(temp_path_for(&paths.config_file)).expect("block the profile file");
        assert!(store.write_config(&config).is_err());
        assert!(shared.read().expect("read") == before);
        assert_eq!(loaded(&store).theme, ThemeMode::Dark);
    }

    #[test]
    fn shared_settings_from_a_newer_lemonup_are_never_rewritten() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        ConfigStore::new(paths.config_file.clone())
            .write_config(&AppConfig::new_unconfigured())
            .expect("write default");
        let shared = SharedSettingsStore::for_roots(&roots);
        fs::create_dir_all(&roots.config_dir).expect("dir");
        let newer = "schema_version = 2\ntheme = \"light\"\nsomething_new = 3\n";
        fs::write(shared.path(), newer).expect("write newer");

        let store = ConfigStore::for_profile(&paths);
        let mut config = loaded(&store);
        assert_eq!(config.theme, ThemeMode::Light);
        config.theme = ThemeMode::Dark;
        let error = store.write_config(&config).expect_err("refused");
        assert!(error.to_string().contains("newer LemonUp"), "{error}");
        assert!(shared.remember_last_used("forever").is_err());
        assert_eq!(fs::read_to_string(shared.path()).expect("read"), newer);
    }

    #[cfg(unix)]
    #[test]
    fn the_shared_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("paths"))
            .write_config(&every_shared_field_set())
            .expect("save");
        let mode = fs::metadata(roots.shared_settings_file())
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    fn loaded(store: &ConfigStore) -> AppConfig {
        match store.load().expect("load") {
            ConfigLoad::Loaded(config) => config,
            ConfigLoad::Missing(path) => panic!("no config at {}", path.display()),
        }
    }

    /// The key in a profile's own file, as an older LemonUp reads it.
    fn own_key(path: &Path) -> Option<String> {
        let raw = fs::read_to_string(path).expect("read");
        toml::from_str::<AppConfig>(&raw)
            .expect("parse")
            .wago_api_key
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
        store.load_or_migrate().expect("first");
        let first = fs::read(store.path()).expect("read shared");

        // A later change to the default profile's own file doesn't redo the migration.
        config.theme = ThemeMode::Dark;
        ConfigStore::new(roots.default_config_file())
            .write_config(&config)
            .expect("rewrite default");
        let settings = store.load_or_migrate().expect("second");
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

        let error = store.load_or_migrate().expect_err("must not migrate");
        assert!(error.to_string().contains("couldn't read"), "{error}");
        assert!(!store.path().exists(), "no shared file over real values");
        assert_eq!(
            fs::read_to_string(roots.default_config_file()).expect("read"),
            broken
        );

        // A broken shared file is never replaced either.
        fs::write(store.path(), "theme = [").expect("write broken shared");
        fs::write(roots.default_config_file(), "theme = \"light\"\n").expect("fix default");
        assert!(store.load_or_migrate().is_err());
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
        let paths = roots.paths("forever").expect("paths");
        let config = ConfigStore::for_profile(&paths);
        config
            .write_config(&AppConfig::new_unconfigured())
            .expect("set up");
        store.remember_last_used("forever").expect("remember");
        config
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

    /// Retail and WoW Forever set up with `key`, both through the bare files, so no
    /// `shared.toml` yet.
    fn two_games(roots: &ProfileRoots, key: &str) {
        let mut retail = AppConfig::new_unconfigured();
        retail.addon_dir = Some("/g/_retail_/Interface/AddOns".into());
        retail.wago_api_key = Some(key.to_string());
        ConfigStore::new(roots.default_config_file())
            .write_config(&retail)
            .expect("write default");
        let mut forever = retail.clone();
        forever.addon_dir = Some("/g/_forever_/Interface/AddOns".into());
        ConfigStore::new(roots.paths("forever").expect("paths").config_file)
            .write_config(&forever)
            .expect("write forever");
    }

    /// Reviewer's scenario F: another game's config can't be read, so `shared.toml` can't
    /// be made. A shared change made then would be lost on the next load.
    #[test]
    fn a_shared_change_waits_while_another_games_settings_cant_be_read() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, "old-key");
        let broken = roots.paths("broken").expect("paths");
        fs::create_dir_all(&broken.config_dir).expect("dir");
        fs::write(&broken.config_file, "theme = [").expect("broken");
        let forever = roots.paths("forever").expect("paths");
        let before = fs::read(&forever.config_file).expect("read");

        let store = ConfigStore::for_profile(&forever);
        let mut config = loaded(&store);
        config.wago_api_key = Some(SECRET.to_string());
        config.theme = ThemeMode::Light;
        let Err(error) = store.write_config(&config) else {
            panic!("must wait");
        };
        let shown = error.to_string();
        assert!(
            shown.contains(&format!(
                "this change waits until {} can be read",
                broken.config_file.display()
            )),
            "{shown}"
        );
        assert!(shown.contains("nothing was changed"), "{shown}");
        assert!(!shown.contains(SECRET));
        // Nothing was written, and the next load shows what was there.
        assert!(fs::read(&forever.config_file).expect("read") == before);
        assert!(!SharedSettingsStore::for_roots(&roots).path().exists());
        let again = loaded(&ConfigStore::for_profile(&forever));
        assert!(again.wago_api_key.as_deref() == Some("old-key"));
        assert_eq!(again.theme, ThemeMode::Dark);

        // What belongs to this game alone still saves.
        let mut own = loaded(&store);
        own.show_libs = true;
        let saved = store.write_config(&own).expect("save this game");
        assert!(saved.show_libs);
        assert!(loaded(&ConfigStore::for_profile(&forever)).show_libs);
    }

    /// Reviewer's scenario G: a crash after `shared.toml` was written and before the
    /// profile's file was. That save never reported done, and the next load undoes it.
    #[test]
    fn a_crash_between_the_two_writes_undoes_the_save() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, "old-key");
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let store = ConfigStore::for_profile(&paths);
        let mut config = loaded(&store);
        let before = fs::read(&paths.config_file).expect("read");
        config.wago_api_key = Some(SECRET.to_string());
        store.write_config(&config).expect("save");
        // As if the profile's file was never written.
        fs::write(&paths.config_file, before).expect("put back");

        let after = loaded(&ConfigStore::for_profile(&paths));
        assert!(after.wago_api_key.as_deref() == Some("old-key"));
    }

    /// Reviewer's scenario J: Retail saw one key, WoW Forever saved another since. Retail
    /// shows the stored key after its next save, and typing the first key again saves it.
    #[test]
    fn after_a_save_this_game_shows_and_changes_the_stored_shared_values() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, "first-key");
        let retail_store = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("p"));
        let forever_store = ConfigStore::for_profile(&roots.paths("forever").expect("p"));
        let mut retail = loaded(&retail_store);
        let mut forever = loaded(&forever_store);
        forever.wago_api_key = Some(SECRET.to_string());
        forever_store.write_config(&forever).expect("save forever");

        retail.reduce_motion = true;
        let mut retail = retail_store.write_config(&retail).expect("save retail");
        assert!(
            retail.wago_api_key.as_deref() == Some(SECRET),
            "shows what is stored"
        );
        assert!(retail.reduce_motion);

        retail.wago_api_key = Some("first-key".to_string());
        retail_store.write_config(&retail).expect("type it again");
        let shared = SharedSettingsStore::for_roots(&roots)
            .read()
            .expect("read")
            .expect("exists");
        assert!(shared.wago_api_key.as_deref() == Some("first-key"));
    }

    /// Scenario J with Settings opened again: it reads the shared values first.
    #[test]
    fn refreshing_shows_another_games_change_and_typing_the_old_value_saves_it() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, "first-key");
        let retail_store = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("p"));
        let forever_store = ConfigStore::for_profile(&roots.paths("forever").expect("p"));
        let mut retail = loaded(&retail_store);
        let mut forever = loaded(&forever_store);
        forever.wago_api_key = Some(SECRET.to_string());
        forever.theme = ThemeMode::Light;
        forever_store.write_config(&forever).expect("save forever");
        let shared = SharedSettingsStore::for_roots(&roots);
        let before = fs::read(shared.path()).expect("read");

        retail_store.refresh_shared(&mut retail).expect("refresh");
        assert!(retail.wago_api_key.as_deref() == Some(SECRET));
        assert_eq!(retail.theme, ThemeMode::Light);
        assert!(
            fs::read(shared.path()).expect("read") == before,
            "never writes"
        );

        retail.wago_api_key = Some("first-key".to_string());
        retail_store.write_config(&retail).expect("save");
        let stored = shared.read().expect("read").expect("exists");
        assert!(stored.wago_api_key.as_deref() == Some("first-key"));
        assert_eq!(stored.theme, ThemeMode::Light);
    }

    #[test]
    fn removing_the_key_removes_the_copies_lemonup_wrote_for_other_games() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, SECRET);
        let classic = roots.paths("classic").expect("paths");
        let retail_store = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("p"));
        let forever = roots.paths("forever").expect("paths");
        // Every game opened once, so each has a record of the key LemonUp wrote there.
        loaded(&ConfigStore::for_profile(&forever));
        let mut classic_config = AppConfig::new_unconfigured();
        classic_config.wago_api_key = Some(SECRET.to_string());
        ConfigStore::new(classic.config_file.clone())
            .write_config(&classic_config)
            .expect("write classic");
        loaded(&ConfigStore::for_profile(&classic));
        // Then an older LemonUp saves another key in classic's own file.
        classic_config.wago_api_key = Some("older-lemonup-key".to_string());
        ConfigStore::new(classic.config_file.clone())
            .write_config(&classic_config)
            .expect("older lemonup");

        let mut retail = loaded(&retail_store);
        retail.wago_api_key = None;
        let saved = retail_store.write_config(&retail).expect("remove key");
        assert!(saved.wago_api_key.is_none());

        let shared = SharedSettingsStore::for_roots(&roots);
        let stored = shared.read().expect("read").expect("exists");
        assert!(stored.wago_api_key.is_none());
        for profile in [DEFAULT_PROFILE, "forever", "classic"] {
            assert!(
                stored.profile_copies[profile].wago_api_key.is_none(),
                "{profile}"
            );
        }
        assert!(
            !fs::read_to_string(shared.path())
                .expect("read")
                .contains(SECRET)
        );
        let forever_raw = fs::read_to_string(&forever.config_file).expect("read");
        assert!(!forever_raw.contains(SECRET), "{forever_raw}");
        assert!(forever_raw.contains("_forever_"), "the rest stays");
        // Classic's file holds a key an older LemonUp saved there: it stays, and classic
        // takes it up as that LemonUp's change, as before.
        assert!(own_key(&classic.config_file).as_deref() == Some("older-lemonup-key"));

        // The key stays removed in WoW Forever.
        assert!(
            loaded(&ConfigStore::for_profile(&forever))
                .wago_api_key
                .is_none()
        );
    }

    #[test]
    fn a_key_copy_that_cant_be_removed_keeps_its_record() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, SECRET);
        let forever = roots.paths("forever").expect("paths");
        let retail_store = ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("p"));
        loaded(&ConfigStore::for_profile(&forever));
        // Forever's file can't be replaced.
        fs::create_dir_all(temp_path_for(&forever.config_file)).expect("block");

        let mut retail = loaded(&retail_store);
        retail.wago_api_key = None;
        retail_store.write_config(&retail).expect("remove key");

        let stored = SharedSettingsStore::for_roots(&roots)
            .read()
            .expect("read")
            .expect("exists");
        assert!(stored.wago_api_key.is_none());
        assert!(stored.profile_copies["forever"].wago_api_key.as_deref() == Some(SECRET));
        // So forever doesn't take its copy back as an older LemonUp's change.
        assert!(
            loaded(&ConfigStore::for_profile(&forever))
                .wago_api_key
                .is_none()
        );
    }

    #[test]
    fn a_background_load_never_writes_the_shared_settings() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        two_games(&roots, "first-key");
        let forever = roots.paths("forever").expect("paths");
        let shared = SharedSettingsStore::for_roots(&roots);

        // Before `shared.toml` exists: not made.
        let config = match ConfigStore::for_profile(&forever)
            .load_without_writing()
            .expect("load")
        {
            ConfigLoad::Loaded(config) => config,
            ConfigLoad::Missing(_) => panic!("forever is set up"),
        };
        assert!(config.wago_api_key.as_deref() == Some("first-key"));
        assert!(!shared.path().exists());

        // An older LemonUp changed forever's file: the change shows, and isn't recorded.
        loaded(&ConfigStore::for_profile(&forever));
        let mut older =
            toml::from_str::<AppConfig>(&fs::read_to_string(&forever.config_file).expect("read"))
                .expect("parse");
        older.theme = ThemeMode::Light;
        ConfigStore::new(forever.config_file.clone())
            .write_config(&older)
            .expect("older lemonup");
        let before = fs::read(shared.path()).expect("read");
        let ConfigLoad::Loaded(config) = ConfigStore::for_profile(&forever)
            .load_without_writing()
            .expect("load")
        else {
            panic!("forever is set up");
        };
        assert_eq!(config.theme, ThemeMode::Light);
        assert!(fs::read(shared.path()).expect("read") == before, "written");
    }
}
