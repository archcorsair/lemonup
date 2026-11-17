//! The games LemonUp manages, one profile each.
//!
//! The default profile lives in the base folders; every other profile in `profiles/<name>`
//! (see [`crate::paths`]). A profile exists when its `config.toml` does. Users see games, not
//! profiles: a profile is shown by its game version ("Retail", "WoW Forever"), with its name
//! added when two profiles are for the same version.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::config::{AppConfig, ConfigLoad, ConfigStore};
use crate::domain::GameFlavor;
use crate::error::LemonupError;
use crate::paths::{
    AppPaths, DEFAULT_PROFILE, ProfileRoots, same_addons_folder, validate_profile_name,
};
use crate::shared::SharedSettingsStore;
use crate::storage::StateDatabase;
use crate::wow::validate_addons_path;

/// The game version's name as users see it.
pub fn game_name(flavor: GameFlavor) -> &'static str {
    match flavor {
        GameFlavor::Retail => "Retail",
        GameFlavor::Forever => "WoW Forever",
        GameFlavor::Classic => "Classic",
        GameFlavor::Cata => "Cataclysm Classic",
    }
}

/// The profile name a new game of this version gets unless the caller gives one.
pub fn profile_name_for(flavor: GameFlavor) -> &'static str {
    match flavor {
        GameFlavor::Retail => "retail",
        GameFlavor::Forever => "forever",
        GameFlavor::Classic => "classic",
        GameFlavor::Cata => "cata",
    }
}

/// One profile as the list of games shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    /// The profile name (`default`, `forever`, ...), for `--profile`.
    pub name: String,
    /// What users see: the game version, plus the profile name when two profiles share it.
    /// A profile without a known game shows its name.
    pub display_name: String,
    /// The AddOns folder its addon list is bound to, else the one in its config.
    pub addon_dir: Option<PathBuf>,
    pub flavor: Option<GameFlavor>,
    /// Addons in its list, libraries not counted; `None` when the list couldn't be read.
    pub addon_count: Option<usize>,
    /// Why part of it couldn't be read. Such a profile is listed, never left out.
    pub problem: Option<String>,
}

impl ProfileSummary {
    pub fn is_default(&self) -> bool {
        self.name == DEFAULT_PROFILE
    }

    /// It has an AddOns folder and nothing failed to read.
    pub fn is_set_up(&self) -> bool {
        self.addon_dir.is_some() && self.problem.is_none()
    }
}

/// What a profile's files say about its AddOns folder.
struct ProfileFolders {
    configured: Option<PathBuf>,
    bound: Option<PathBuf>,
    addon_count: Option<usize>,
    problems: Vec<String>,
}

fn read_profile_folders(paths: &AppPaths) -> ProfileFolders {
    let mut problems = Vec::new();
    let configured = match ConfigStore::new(paths.config_file.clone()).load() {
        Ok(ConfigLoad::Loaded(config)) => config.addon_dir,
        Ok(ConfigLoad::Missing(_)) => None,
        Err(error) => {
            problems.push(format!("couldn't read its settings: {error}"));
            None
        }
    };
    let bound = match StateDatabase::read_bound_addon_dir(&paths.state_db_file) {
        Ok(bound) => bound,
        Err(error) => {
            problems.push(format!("couldn't read its addon list: {error}"));
            None
        }
    };
    let addon_count = match StateDatabase::read_addon_count(&paths.state_db_file) {
        Ok(count) => Some(count),
        Err(error) => {
            if problems
                .iter()
                .all(|problem| !problem.contains("addon list"))
            {
                problems.push(format!("couldn't read its addon list: {error}"));
            }
            None
        }
    };
    ProfileFolders {
        configured,
        bound,
        addon_count,
        problems,
    }
}

/// Every profile: the default one first (it always exists, set up or not), then each
/// `profiles/<name>` that holds a config, by name. Nothing is created or changed.
pub fn list_profiles(roots: &ProfileRoots) -> crate::Result<Vec<ProfileSummary>> {
    let mut names = vec![DEFAULT_PROFILE.to_string()];
    names.extend(other_profile_names(roots)?);

    let mut summaries = Vec::with_capacity(names.len());
    for name in names {
        let paths = roots.paths(&name)?;
        let folders = read_profile_folders(&paths);
        let addon_dir = folders.bound.or(folders.configured);
        let flavor = addon_dir.as_deref().and_then(GameFlavor::detect_from_path);
        summaries.push(ProfileSummary {
            display_name: name.clone(),
            name,
            addon_dir,
            flavor,
            addon_count: folders.addon_count,
            problem: (!folders.problems.is_empty()).then(|| folders.problems.join("; ")),
        });
    }

    let flavors = summaries
        .iter()
        .map(|summary| summary.flavor)
        .collect::<Vec<_>>();
    for summary in &mut summaries {
        if let Some(flavor) = summary.flavor {
            let same_version = flavors
                .iter()
                .filter(|other| **other == Some(flavor))
                .count();
            summary.display_name = if same_version > 1 {
                format!("{} ({})", game_name(flavor), summary.name)
            } else {
                game_name(flavor).to_string()
            };
        }
    }
    Ok(summaries)
}

/// Names of the profiles under `profiles/` that have a config, sorted. Folders whose names
/// `--profile` can't take are not profiles and are skipped.
pub(crate) fn other_profile_names(roots: &ProfileRoots) -> crate::Result<Vec<String>> {
    let entries = match fs::read_dir(roots.profiles_config_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name == DEFAULT_PROFILE || validate_profile_name(&name).is_err() {
            continue;
        }
        // An error here (no permission, say) still lists it, with the problem shown.
        if entry
            .path()
            .join("config.toml")
            .try_exists()
            .unwrap_or(true)
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Whether a profile named `name` exists: the default one always does, others when their
/// config does.
pub fn profile_exists(roots: &ProfileRoots, name: &str) -> bool {
    if name == DEFAULT_PROFILE {
        return true;
    }
    roots
        .paths(name)
        .map(|paths| paths.config_file.is_file())
        .unwrap_or(false)
}

/// Why a game couldn't be added or removed. Callers choose their message by the variant.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("{reason}")]
    NotAnAddonsFolder { reason: String },
    #[error("there is already a game named '{0}' | choose another name")]
    NameTaken(String),
    #[error("{} is already set up as '{profile}'", addon_dir.display())]
    FolderInUse { profile: String, addon_dir: PathBuf },
    #[error("couldn't check the game '{profile}': {reason} | nothing was changed")]
    CantCheck { profile: String, reason: String },
    #[error("'{0}' is the game in use | switch to another game first")]
    InUse(String),
    #[error("the default game can't be removed | commands without --profile use it")]
    DefaultProfile,
    #[error("there is no game named '{0}'")]
    NotFound(String),
    #[error(transparent)]
    Core(#[from] LemonupError),
}

impl From<std::io::Error> for ProfileError {
    fn from(error: std::io::Error) -> Self {
        Self::Core(error.into())
    }
}

/// A game set up by [`create_profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedProfile {
    pub paths: AppPaths,
    pub flavor: GameFlavor,
    pub addon_dir: PathBuf,
}

/// Sets up a new profile for the install whose AddOns folder is `addon_dir`.
///
/// It is named `name`, or after its game version (`retail`, `forever`) when `name` is
/// `None`. It refuses a name already used, and a folder another profile is bound to or
/// configured for (compared as [`same_addons_folder`] does, without case where the system
/// ignores it). It writes the new profile's config, with the shared settings, and binds its
/// addon list to the folder. The game folder is only read.
pub fn create_profile(
    roots: &ProfileRoots,
    addon_dir: &Path,
    name: Option<&str>,
) -> Result<CreatedProfile, ProfileError> {
    validate_addons_path(addon_dir).map_err(|reason| ProfileError::NotAnAddonsFolder { reason })?;
    let addon_dir = std::path::absolute(addon_dir)?;
    let flavor = GameFlavor::detect_from_path(&addon_dir).ok_or_else(|| {
        ProfileError::NotAnAddonsFolder {
            reason: "path must point to a retail or WoW Forever AddOns folder".to_string(),
        }
    })?;
    let name = name.unwrap_or(profile_name_for(flavor)).to_string();
    let paths = roots.paths(&name)?;
    if name == DEFAULT_PROFILE
        || paths.config_file.try_exists()?
        || paths.state_db_file.try_exists()?
    {
        return Err(ProfileError::NameTaken(name));
    }

    let mut others = vec![DEFAULT_PROFILE.to_string()];
    others.extend(other_profile_names(roots)?);
    for other in others {
        let folders = read_profile_folders(&roots.paths(&other)?);
        let uses_folder = folders
            .configured
            .iter()
            .chain(folders.bound.iter())
            .any(|folder| same_addons_folder(folder, &addon_dir));
        if uses_folder {
            return Err(ProfileError::FolderInUse {
                profile: other,
                addon_dir,
            });
        }
        // Not knowing which folder a profile has is not the same as it having none.
        if let Some(problem) = folders.problems.first() {
            return Err(ProfileError::CantCheck {
                profile: other,
                reason: problem.clone(),
            });
        }
    }

    let mut config = AppConfig::new_unconfigured();
    SharedSettingsStore::for_roots(roots)
        .load_or_migrate()?
        .apply_to(&mut config);
    config.addon_dir = Some(addon_dir.clone());

    paths.ensure()?;
    // Through the shared settings, so they record what this game's config now holds.
    let written = ConfigStore::for_profile(&paths)
        .write_config(&config)
        .and_then(|_| StateDatabase::open(&paths.state_db_file)?.bind_addon_dir(&addon_dir));
    if let Err(error) = written {
        forget_profile_files(&paths);
        return Err(error.into());
    }

    Ok(CreatedProfile {
        paths,
        flavor,
        addon_dir,
    })
}

/// A game removed by [`remove_profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedProfile {
    pub name: String,
    /// Its backups of game settings, left on disk.
    pub backups_dir: PathBuf,
}

/// Forgets the profile `name`: its addon list and its settings. Its backups of game settings
/// stay on disk, and the game's own files are never touched. Refuses the profile in use and
/// the default profile.
pub fn remove_profile(
    roots: &ProfileRoots,
    name: &str,
    in_use: &str,
) -> Result<RemovedProfile, ProfileError> {
    if name == DEFAULT_PROFILE {
        return Err(ProfileError::DefaultProfile);
    }
    // Without case, like Windows and macOS find its folder.
    if name.eq_ignore_ascii_case(in_use) {
        return Err(ProfileError::InUse(name.to_string()));
    }
    let paths = roots.paths(name)?;
    if !paths.config_file.try_exists()? && !paths.state_db_file.try_exists()? {
        return Err(ProfileError::NotFound(name.to_string()));
    }

    // The addon list goes first: a profile left with its config and no list shows as a
    // game with no addons, never as a list with no settings.
    for path in state_db_files(&paths) {
        remove_if_present(&path)?;
    }
    remove_if_present(&paths.config_file)?;
    // Only when empty: anything else someone put there stays.
    let _ = fs::remove_dir(&paths.config_dir);
    // The copy of the shared settings, Wago.io key included, recorded for it.
    SharedSettingsStore::for_roots(roots).forget_profile(name);

    Ok(RemovedProfile {
        name: name.to_string(),
        backups_dir: paths.backup_root(),
    })
}

/// The profile the TUI opens, from [`tui_profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiProfile {
    pub name: String,
    /// Why the game used last didn't open, to say once.
    pub notice: Option<String>,
}

/// The profile plain `lemonup` opens: `requested` when given, else the last one the TUI
/// opened, else (none yet, or it was removed) the default one. When the last used game's
/// settings can't be read, the default game opens and `notice` says why. Never writes.
pub fn tui_profile(roots: &ProfileRoots, requested: Option<&str>) -> crate::Result<TuiProfile> {
    let default = |notice| TuiProfile {
        name: DEFAULT_PROFILE.to_string(),
        notice,
    };
    if let Some(requested) = requested {
        return Ok(TuiProfile {
            name: requested.to_string(),
            notice: None,
        });
    }
    let last_used = SharedSettingsStore::for_roots(roots)
        .read()?
        .and_then(|settings| settings.last_used_profile)
        .filter(|name| name != DEFAULT_PROFILE && profile_exists(roots, name));
    let Some(name) = last_used else {
        return Ok(default(None));
    };
    match ConfigStore::new(roots.paths(&name)?.config_file).load() {
        Ok(_) => Ok(TuiProfile { name, notice: None }),
        Err(error) => Ok(default(Some(format!(
            "Opened your default game: the game you used last ('{name}') couldn't be opened | {error}"
        )))),
    }
}

/// Remembers `profile` as the one the TUI opened last.
pub fn remember_last_used(roots: &ProfileRoots, profile: &str) -> crate::Result<()> {
    SharedSettingsStore::for_roots(roots).remember_last_used(profile)
}

fn state_db_files(paths: &AppPaths) -> Vec<PathBuf> {
    let name = paths.state_db_file.as_os_str().to_os_string();
    ["", "-wal", "-shm", "-journal"]
        .iter()
        .map(|suffix| {
            let mut file = name.clone();
            file.push(suffix);
            PathBuf::from(file)
        })
        .collect()
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Undoes a half-made profile: only the files [`create_profile`] writes.
fn forget_profile_files(paths: &AppPaths) {
    for path in state_db_files(paths) {
        let _ = remove_if_present(&path);
    }
    let _ = remove_if_present(&paths.config_file);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        ProfileError, create_profile, list_profiles, remember_last_used, remove_profile,
        tui_profile,
    };
    use crate::config::{AppConfig, ConfigLoad, ConfigStore, ThemeMode};
    use crate::domain::{AddonKind, AddonRecord, GameFlavor, SourceKind};
    use crate::paths::{DEFAULT_PROFILE, ProfileRoots};
    use crate::shared::SharedSettingsStore;
    use crate::storage::StateDatabase;

    fn test_roots(base: &Path) -> ProfileRoots {
        ProfileRoots::new(base.join("config"), base.join("data"), base.join("cache"))
    }

    fn install(base: &Path, client: &str) -> PathBuf {
        let root = base.join("wow");
        let dir = root.join(client).join("Interface").join("AddOns");
        fs::create_dir_all(&dir).expect("addons dir");
        fs::create_dir_all(root.join("Data")).expect("data dir");
        fs::write(root.join(".build.info"), "build").expect("build info");
        dir
    }

    /// Sets up the default profile on `addon_dir` with `addons` addons and one library.
    fn set_up_default(roots: &ProfileRoots, addon_dir: &Path, addons: usize) {
        let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some(addon_dir.to_path_buf());
        ConfigStore::for_profile(&paths)
            .write_config(&config)
            .expect("write config");
        let database = StateDatabase::open(&paths.state_db_file).expect("open");
        database.bind_addon_dir(addon_dir).expect("bind");
        for index in 0..addons {
            let name = format!("Addon{index}");
            database
                .upsert_addon(&AddonRecord::new(&name, &name, SourceKind::Manual))
                .expect("upsert");
        }
        let mut library = AddonRecord::new("LibStub", "LibStub", SourceKind::Manual);
        library.kind = AddonKind::Library;
        database.upsert_addon(&library).expect("upsert library");
    }

    #[test]
    fn lists_games_by_version_with_counts_and_folders() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        let forever = install(temp.path(), "_forever_");
        set_up_default(&roots, &retail, 3);
        let created = create_profile(&roots, &forever, None).expect("create forever");
        assert_eq!(created.paths.profile, "forever");

        let list = list_profiles(&roots).expect("list");
        let rows = list
            .iter()
            .map(|summary| {
                (
                    summary.name.as_str(),
                    summary.display_name.as_str(),
                    summary.flavor,
                    summary.addon_count,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ("default", "Retail", Some(GameFlavor::Retail), Some(3)),
                ("forever", "WoW Forever", Some(GameFlavor::Forever), Some(0)),
            ]
        );
        assert_eq!(list[0].addon_dir.as_deref(), Some(retail.as_path()));
        assert_eq!(list[1].addon_dir.as_deref(), Some(forever.as_path()));
        assert!(list.iter().all(|summary| summary.is_set_up()));
    }

    #[test]
    fn two_games_of_one_version_show_their_names() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        set_up_default(&roots, &install(temp.path(), "_retail_"), 0);
        let ptr = temp.path().join("ptr");
        let second = ptr.join("_retail_").join("Interface").join("AddOns");
        fs::create_dir_all(&second).expect("second install");
        fs::create_dir_all(ptr.join("Data")).expect("data dir");
        fs::write(ptr.join(".build.info"), "build").expect("build info");
        create_profile(&roots, &second, Some("ptr")).expect("create ptr");

        let names = list_profiles(&roots)
            .expect("list")
            .into_iter()
            .map(|summary| summary.display_name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["Retail (default)", "Retail (ptr)"]);
    }

    #[test]
    fn a_profile_that_cannot_be_read_is_listed_with_its_problem() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let broken = roots.paths("broken").expect("paths");
        fs::create_dir_all(&broken.config_dir).expect("dir");
        fs::write(&broken.config_file, "addon_dir = [").expect("write broken");
        fs::create_dir_all(roots.profiles_config_dir().join("no config")).expect("not a profile");

        let list = list_profiles(&roots).expect("list");
        let names = list
            .iter()
            .map(|summary| summary.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["default", "broken"]);
        let problem = list[1].problem.as_deref().expect("problem shown");
        assert!(problem.contains("couldn't read its settings"), "{problem}");
        assert!(!list[1].is_set_up());
        assert_eq!(
            fs::read_to_string(&broken.config_file).expect("read"),
            "addon_dir = ["
        );
    }

    #[test]
    fn create_refuses_a_folder_another_game_uses() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        set_up_default(&roots, &retail, 1);

        let error = create_profile(&roots, &retail, None).expect_err("refused");
        assert!(
            matches!(&error, ProfileError::FolderInUse { profile, .. } if profile == "default"),
            "{error}"
        );
        assert!(!roots.paths("retail").expect("paths").config_file.exists());

        // The same folder, reached another way.
        #[cfg(unix)]
        {
            let link = temp.path().join("wow-link");
            std::os::unix::fs::symlink(temp.path().join("wow"), &link).expect("symlink");
            let linked = link.join("_retail_").join("Interface").join("AddOns");
            assert!(matches!(
                create_profile(&roots, &linked, Some("linked")),
                Err(ProfileError::FolderInUse { .. })
            ));
        }

        let forever = install(temp.path(), "_forever_");
        create_profile(&roots, &forever, None).expect("create forever");
        let error = create_profile(&roots, &forever, Some("again")).expect_err("refused");
        assert!(
            matches!(&error, ProfileError::FolderInUse { profile, .. } if profile == "forever"),
            "{error}"
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn create_refuses_a_folder_another_game_uses_in_other_case() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        set_up_default(&roots, &retail, 0);
        let shouted = PathBuf::from(retail.to_string_lossy().to_uppercase());
        assert!(matches!(
            create_profile(&roots, &shouted, Some("loud")),
            Err(ProfileError::FolderInUse { .. })
        ));
    }

    #[test]
    fn create_refuses_a_taken_name_and_when_another_game_cannot_be_read() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let paths = roots.paths("forever").expect("paths");
        fs::create_dir_all(&paths.config_dir).expect("dir");
        fs::write(&paths.config_file, "addon_dir = [").expect("broken");
        assert!(matches!(
            create_profile(&roots, &forever, None),
            Err(ProfileError::NameTaken(name)) if name == "forever"
        ));
        assert!(matches!(
            create_profile(&roots, &forever, Some("default")),
            Err(ProfileError::NameTaken(_))
        ));
        // `forever` might be bound to this very folder; not knowing is a refusal.
        assert!(matches!(
            create_profile(&roots, &forever, Some("other")),
            Err(ProfileError::CantCheck { profile, .. }) if profile == "forever"
        ));
    }

    #[test]
    fn create_writes_config_with_shared_settings_and_binds_the_list() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        let marker = retail.join("Untouched");
        fs::create_dir_all(&marker).expect("addon");
        let mut shared = AppConfig::new_unconfigured();
        shared.theme = ThemeMode::Light;
        ConfigStore::for_profile(&roots.paths(DEFAULT_PROFILE).expect("paths"))
            .write_config(&shared)
            .expect("default settings");

        let created = create_profile(&roots, &retail, None).expect("create");
        assert_eq!(created.paths.profile, "retail");
        assert_eq!(created.flavor, GameFlavor::Retail);
        let ConfigLoad::Loaded(config) = ConfigStore::new(created.paths.config_file.clone())
            .load()
            .expect("load")
        else {
            panic!("config written");
        };
        assert_eq!(config.addon_dir.as_deref(), Some(retail.as_path()));
        assert_eq!(config.theme, ThemeMode::Light);
        assert_eq!(
            StateDatabase::read_bound_addon_dir(&created.paths.state_db_file).expect("read"),
            Some(retail.clone())
        );
        assert!(marker.is_dir());
        assert_eq!(fs::read_dir(&retail).expect("read").count(), 1);
    }

    #[test]
    fn remove_forgets_the_game_and_keeps_its_backups() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let created = create_profile(&roots, &forever, None).expect("create");
        let backups = created.paths.backup_root().join("forever");
        fs::create_dir_all(&backups).expect("backups");
        fs::write(backups.join("wtf-1.zip"), b"zip").expect("backup");

        let removed = remove_profile(&roots, "forever", DEFAULT_PROFILE).expect("remove");
        assert_eq!(removed.backups_dir, created.paths.backup_root());
        assert!(backups.join("wtf-1.zip").is_file());
        assert!(!created.paths.config_file.exists());
        assert!(!created.paths.state_db_file.exists());
        assert!(forever.is_dir());
        let names = list_profiles(&roots)
            .expect("list")
            .into_iter()
            .map(|summary| summary.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["default"]);

        // It can be added again, and finds its backups where they were.
        create_profile(&roots, &forever, None).expect("add again");
        assert!(backups.join("wtf-1.zip").is_file());
    }

    #[test]
    fn remove_forgets_the_shared_settings_copy_of_the_game() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let retail = install(temp.path(), "_retail_");
        set_up_default(&roots, &retail, 0);
        let forever = install(temp.path(), "_forever_");
        create_profile(&roots, &forever, None).expect("create");
        let shared = SharedSettingsStore::for_roots(&roots);
        let recorded = |shared: &SharedSettingsStore| {
            shared
                .read()
                .expect("read")
                .expect("exists")
                .profile_copies
                .contains_key("forever")
        };
        assert!(recorded(&shared));

        remove_profile(&roots, "forever", DEFAULT_PROFILE).expect("remove");
        assert!(!recorded(&shared));
        assert!(
            shared
                .read()
                .expect("read")
                .expect("exists")
                .profile_copies
                .contains_key(DEFAULT_PROFILE)
        );
    }

    #[test]
    fn remove_refuses_the_game_in_use_and_the_default_one() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let created = create_profile(&roots, &forever, None).expect("create");

        assert!(matches!(
            remove_profile(&roots, "forever", "forever"),
            Err(ProfileError::InUse(_))
        ));
        assert!(matches!(
            remove_profile(&roots, DEFAULT_PROFILE, "forever"),
            Err(ProfileError::DefaultProfile)
        ));
        assert!(matches!(
            remove_profile(&roots, "missing", DEFAULT_PROFILE),
            Err(ProfileError::NotFound(_))
        ));
        assert!(created.paths.config_file.is_file());
        assert!(created.paths.state_db_file.is_file());
    }

    #[test]
    fn plain_launch_opens_the_last_used_game_or_the_default_one() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let opened = |requested: Option<&str>| {
            let profile = tui_profile(&roots, requested).expect("tui profile");
            assert_eq!(profile.notice, None);
            profile.name
        };
        assert_eq!(opened(None), DEFAULT_PROFILE);

        let forever = install(temp.path(), "_forever_");
        create_profile(&roots, &forever, None).expect("create");
        remember_last_used(&roots, "forever").expect("remember");
        assert_eq!(opened(None), "forever");
        assert_eq!(opened(Some("other")), "other");

        remove_profile(&roots, "forever", DEFAULT_PROFILE).expect("remove");
        assert_eq!(opened(None), DEFAULT_PROFILE);
    }

    #[test]
    fn a_last_used_game_that_cannot_be_read_opens_the_default_one_and_says_why() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let created = create_profile(&roots, &forever, None).expect("create");
        remember_last_used(&roots, "forever").expect("remember");
        fs::write(&created.paths.config_file, "addon_dir = [").expect("break it");

        let profile = tui_profile(&roots, None).expect("still opens");
        assert_eq!(profile.name, DEFAULT_PROFILE);
        let notice = profile.notice.expect("says why");
        assert!(notice.contains("Opened your default game"), "{notice}");
        assert!(notice.contains("'forever'"), "{notice}");
        assert!(
            notice.contains(&created.paths.config_file.display().to_string()),
            "{notice}"
        );
        // Asked for by name, it is opened, and its error says which file.
        assert_eq!(
            tui_profile(&roots, Some("forever")).expect("flag").name,
            "forever"
        );
    }

    #[test]
    fn a_game_whose_settings_cannot_be_read_names_the_file() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let created = create_profile(&roots, &forever, None).expect("create");
        fs::write(&created.paths.config_file, "addon_dir = [").expect("break it");

        let error = ConfigStore::for_profile(&created.paths)
            .load()
            .expect_err("unreadable");
        let shown = error.to_string();
        assert!(
            shown.contains(&created.paths.config_file.display().to_string()),
            "{shown}"
        );
        assert!(shown.contains("nothing was changed"), "{shown}");
    }

    #[test]
    fn remove_refuses_the_game_in_use_written_in_another_case() {
        let temp = tempdir().expect("tempdir");
        let roots = test_roots(temp.path());
        let forever = install(temp.path(), "_forever_");
        let created = create_profile(&roots, &forever, None).expect("create");

        assert!(matches!(
            remove_profile(&roots, "Forever", "forever"),
            Err(ProfileError::InUse(_))
        ));
        assert!(matches!(
            remove_profile(&roots, "forever", "FOREVER"),
            Err(ProfileError::InUse(_))
        ));
        assert!(created.paths.config_file.is_file());
    }
}
