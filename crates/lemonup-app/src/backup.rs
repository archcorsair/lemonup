use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::tempdir;
use time::format_description::FormatItem;
use time::macros::format_description;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use lemonup_core::paths::same_addons_folder;

use crate::archive::safe_entry_relative_path;

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]T[hour][minute][second][subsecond digits:3]Z");
const MAX_ARCHIVE_NAME_ATTEMPTS: usize = 50;
const BACKUP_DISPLAY_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]Z");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    pub file_name: String,
    pub path: PathBuf,
    pub label: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRunOutcome {
    pub backup: BackupEntry,
    pub pruned_files: Vec<String>,
    pub backups: Vec<BackupEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRestoreOutcome {
    pub backup: BackupEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDeleteOutcome {
    pub deleted: BackupEntry,
    pub backups: Vec<BackupEntry>,
}

/// Root of a profile's WTF backup store. Archives live one level down, in a folder per
/// game client (see [`client_backup_dir`]); archives directly in the root predate that
/// split and are moved by [`adopt_legacy_backups`].
pub fn backup_root(data_dir: &Path) -> PathBuf {
    data_dir.join("backups").join("wtf")
}

/// Name of the archive entry that records which client a backup was taken from.
const CLIENT_MARKER_ENTRY: &str = "lemonup-backup.txt";

/// Stable key for the game client that owns `addon_dir`: its client folder name without
/// the surrounding underscores (`_retail_` is `retail`, `_forever_` is `forever`). Keyed
/// by client rather than by full path so backups survive moving the install.
pub fn client_backup_key(addon_dir: &Path) -> String {
    let client = addon_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|name| {
            name.to_string_lossy()
                .trim_matches('_')
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let key = client
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '-' || value == '_' {
                value
            } else {
                '-'
            }
        })
        .collect::<String>();
    if key.is_empty() {
        "client".to_string()
    } else {
        key
    }
}

/// Folder holding the WTF backups of the client that owns `addon_dir`. Listing, pruning
/// and the pre-update throttle all work inside this folder, so clients never share them.
pub fn client_backup_dir(root: &Path, addon_dir: &Path) -> PathBuf {
    root.join(client_backup_key(addon_dir))
}

/// Moves archives left directly in `root` by builds without per-client folders into the
/// folder of `addon_dir`'s client. Callers pass the profile's bound AddOns folder: those
/// archives were taken from the folder the profile managed, which is the bound one.
/// Returns the archives moved; a name already present in the client folder is left alone.
pub fn adopt_legacy_backups(root: &Path, addon_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let legacy = list_backups(root)?;
    if legacy.is_empty() {
        return Ok(Vec::new());
    }
    let client_dir = client_backup_dir(root, addon_dir);
    fs::create_dir_all(&client_dir).map_err(|error| error.to_string())?;
    let mut moved = Vec::new();
    for entry in legacy {
        let target = client_dir.join(&entry.file_name);
        if target.exists() {
            continue;
        }
        fs::rename(&entry.path, &target).map_err(|error| error.to_string())?;
        moved.push(target);
    }
    Ok(moved)
}

pub fn derive_wtf_dir(addon_dir: &Path) -> Result<PathBuf, String> {
    // AddOns -> Interface -> client folder (`_retail_`, `_forever_`, ...), which owns WTF.
    let client_dir = addon_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("invalid addon directory: {}", addon_dir.display()))?;
    let wtf_dir = client_dir.join("WTF");
    if !wtf_dir.is_dir() {
        return Err(format!("WTF folder missing at {}", wtf_dir.display()));
    }
    Ok(wtf_dir)
}

pub fn list_backups(root: &Path) -> Result<Vec<BackupEntry>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut backups = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| backup_entry_from_path(&entry.path()).ok())
        .collect::<Vec<_>>();
    backups.sort_by(|left, right| right.file_name.cmp(&left.file_name));
    Ok(backups)
}

pub fn create_wtf_backup(
    addon_dir: &Path,
    backup_dir: &Path,
    retention: u16,
) -> Result<BackupRunOutcome, String> {
    if retention == 0 {
        return Err("backup retention must be at least 1".to_string());
    }

    let wtf_dir = derive_wtf_dir(addon_dir)?;
    fs::create_dir_all(backup_dir).map_err(|error| error.to_string())?;

    let (file_name, archive_path) =
        write_wtf_archive(&wtf_dir, backup_dir, &client_backup_key(addon_dir))?;

    let pruned_files = prune_backups(backup_dir, usize::from(retention))?;
    let backups = list_backups(backup_dir)?;
    let backup = backups
        .iter()
        .find(|entry| entry.file_name == file_name)
        .cloned()
        .ok_or_else(|| format!("backup created but not found at {}", archive_path.display()))?;

    Ok(BackupRunOutcome {
        backup,
        pruned_files,
        backups,
    })
}

/// A pre-update WTF backup is skipped when the newest existing backup is younger than this.
pub const PRE_UPDATE_BACKUP_MIN_AGE_MINUTES: i64 = 60;

/// Settings for the automatic WTF backup taken before an update run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreUpdateBackupSettings {
    pub enabled: bool,
    pub retention: u16,
    pub backup_dir: PathBuf,
}

/// Result of the automatic pre-update backup. Never fatal: callers surface it and carry on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreUpdateBackup {
    Disabled,
    Created { path: PathBuf },
    Skipped { minutes_ago: i64 },
    Warning(String),
}

impl PreUpdateBackup {
    /// One-line CLI output; `None` when the setting is off.
    pub fn cli_line(&self) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::Created { path } => Some(format!("wtf backup: created {}", path.display())),
            Self::Skipped { minutes_ago } => Some(format!(
                "wtf backup: skipped (last backup {minutes_ago} min ago)"
            )),
            Self::Warning(error) => Some(format!("wtf backup warning: {error}")),
        }
    }

    /// Short note for the TUI status line; only worth showing when something happened.
    pub fn status_note(&self) -> Option<String> {
        match self {
            Self::Created { .. } => Some("backed up your game settings first".to_string()),
            Self::Warning(error) => Some(format!(
                "couldn't back up your game settings ({error}), the update went ahead"
            )),
            Self::Disabled | Self::Skipped { .. } => None,
        }
    }
}

/// Backs up WTF before an update run when `backup_wtf` is on, unless the newest existing
/// backup is less than 60 minutes old at `now`. A failure becomes a warning, never an abort.
pub fn run_pre_update_backup(
    settings: &PreUpdateBackupSettings,
    addon_dir: &Path,
    now: OffsetDateTime,
) -> PreUpdateBackup {
    if !settings.enabled {
        return PreUpdateBackup::Disabled;
    }

    // An unreadable backup folder is not a reason to skip: attempt the backup and let
    // `create_wtf_backup` report any real problem.
    let existing = list_backups(&settings.backup_dir).unwrap_or_default();
    if let Some(minutes_ago) = recent_backup_minutes(&existing, now) {
        return PreUpdateBackup::Skipped { minutes_ago };
    }

    match create_wtf_backup(addon_dir, &settings.backup_dir, settings.retention) {
        Ok(outcome) => PreUpdateBackup::Created {
            path: outcome.backup.path,
        },
        Err(error) => PreUpdateBackup::Warning(error),
    }
}

/// Whole minutes since the newest backup when it is younger than the throttle window.
fn recent_backup_minutes(backups: &[BackupEntry], now: OffsetDateTime) -> Option<i64> {
    let newest = backups
        .iter()
        .filter_map(|entry| backup_created_at(&entry.file_name))
        .max()?;
    let age = now - newest;
    // A backup stamped in the future (clock change) does not throttle.
    if age < Duration::ZERO || age >= Duration::minutes(PRE_UPDATE_BACKUP_MIN_AGE_MINUTES) {
        return None;
    }
    Some(age.whole_minutes())
}

fn backup_created_at(file_name: &str) -> Option<OffsetDateTime> {
    let timestamp = file_name.strip_prefix("WTF-")?.strip_suffix(".zip")?;
    PrimitiveDateTime::parse(timestamp, BACKUP_TIMESTAMP_FORMAT)
        .ok()
        .map(PrimitiveDateTime::assume_utc)
}

/// Restores `archive_path` over the WTF folder of `addon_dir`'s client. `backup_dir` is
/// that client's own backup folder ([`client_backup_dir`]); an archive taken from another
/// client (its recorded client, or its folder in the same store) is refused.
pub fn restore_wtf_backup(
    addon_dir: &Path,
    backup_dir: &Path,
    archive_path: &Path,
) -> Result<BackupRestoreOutcome, String> {
    let wtf_dir = derive_wtf_dir(addon_dir)?;
    let backup = backup_entry_from_path(archive_path)?;
    let client = client_backup_key(addon_dir);
    if let Some(owner) = archive_client(&backup.path, backup_dir)?
        && owner != client
    {
        return Err(format!(
            "backup {} was taken from the '{owner}' client | refusing to restore it into the '{client}' WTF folder {}",
            backup.path.display(),
            wtf_dir.display()
        ));
    }
    let temp = tempdir().map_err(|error| error.to_string())?;
    let extracted_root = temp.path().join("extracted");
    fs::create_dir_all(&extracted_root).map_err(|error| error.to_string())?;
    extract_wtf_archive(&backup.path, &extracted_root)?;
    let extracted_wtf = extracted_root.join("WTF");
    if !extracted_wtf.is_dir() {
        return Err("backup archive is missing the WTF root".to_string());
    }

    let parent = wtf_dir
        .parent()
        .ok_or_else(|| format!("invalid WTF directory: {}", wtf_dir.display()))?;
    let rollback_dir = parent.join(format!(
        ".lemonup-restore-{}",
        OffsetDateTime::now_utc()
            .format(BACKUP_TIMESTAMP_FORMAT)
            .map_err(|error| error.to_string())?
    ));

    if rollback_dir.exists() {
        fs::remove_dir_all(&rollback_dir).map_err(|error| error.to_string())?;
    }

    fs::rename(&wtf_dir, &rollback_dir).map_err(|error| error.to_string())?;

    if let Err(error) = copy_dir_recursively(&extracted_wtf, &wtf_dir) {
        let _ = fs::remove_dir_all(&wtf_dir);
        let _ = fs::rename(&rollback_dir, &wtf_dir);
        return Err(error);
    }

    fs::remove_dir_all(&rollback_dir).map_err(|error| error.to_string())?;
    Ok(BackupRestoreOutcome { backup })
}

pub fn delete_backup(
    backup_root: &Path,
    archive_path: &Path,
) -> Result<BackupDeleteOutcome, String> {
    let backup = backup_entry_from_path(archive_path)?;
    fs::remove_file(&backup.path).map_err(|error| error.to_string())?;
    let backups = list_backups(backup_root)?;
    Ok(BackupDeleteOutcome {
        deleted: backup,
        backups,
    })
}

/// The client an archive belongs to: the client recorded inside it, else the per-client
/// folder it sits in when that folder is a sibling of `backup_dir` in the same store.
/// `None` for an older archive outside the store (or in the legacy flat root).
fn archive_client(archive_path: &Path, backup_dir: &Path) -> Result<Option<String>, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if let Ok(mut marker) = zip.by_name(CLIENT_MARKER_ENTRY) {
        let mut contents = String::new();
        marker
            .read_to_string(&mut contents)
            .map_err(|error| error.to_string())?;
        if let Some(client) = contents
            .lines()
            .find_map(|line| line.trim().strip_prefix("client="))
        {
            return Ok(Some(client.trim().to_string()));
        }
    }

    let Some(parent) = archive_path.parent() else {
        return Ok(None);
    };
    let (Some(store), Some(own_store)) = (parent.parent(), backup_dir.parent()) else {
        return Ok(None);
    };
    if same_addons_folder(parent, backup_dir) || !same_addons_folder(store, own_store) {
        return Ok(None);
    }
    Ok(parent
        .file_name()
        .map(|name| name.to_string_lossy().into_owned()))
}

fn backup_entry_from_path(path: &Path) -> Result<BackupEntry, String> {
    if path.extension().and_then(|value| value.to_str()) != Some("zip") {
        return Err("not a zip backup".to_string());
    }

    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("not a file".to_string());
    }

    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid backup file name: {}", path.display()))?
        .to_string();

    Ok(BackupEntry {
        label: render_backup_label(&file_name),
        file_name,
        path: path.to_path_buf(),
        size_bytes: metadata.len(),
    })
}

fn render_backup_label(file_name: &str) -> String {
    let trimmed = file_name
        .strip_prefix("WTF-")
        .and_then(|value| value.strip_suffix(".zip"));
    let Some(timestamp) = trimmed else {
        return file_name.to_string();
    };
    let Ok(parsed) = OffsetDateTime::parse(timestamp, BACKUP_TIMESTAMP_FORMAT) else {
        return file_name.to_string();
    };
    parsed
        .format(BACKUP_DISPLAY_FORMAT)
        .unwrap_or_else(|_| file_name.to_string())
}

fn prune_backups(root: &Path, retention: usize) -> Result<Vec<String>, String> {
    let backups = list_backups(root)?;
    if backups.len() <= retention {
        return Ok(Vec::new());
    }

    let mut deleted = Vec::new();
    for entry in backups.iter().skip(retention) {
        fs::remove_file(&entry.path).map_err(|error| error.to_string())?;
        deleted.push(entry.file_name.clone());
    }
    Ok(deleted)
}

fn create_unique_archive_file(backup_dir: &Path) -> Result<(String, PathBuf, File), String> {
    for _ in 0..MAX_ARCHIVE_NAME_ATTEMPTS {
        let timestamp = OffsetDateTime::now_utc()
            .format(BACKUP_TIMESTAMP_FORMAT)
            .map_err(|error| error.to_string())?;
        let file_name = format!("WTF-{timestamp}.zip");
        let archive_path = backup_dir.join(&file_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&archive_path)
        {
            Ok(file) => return Ok((file_name, archive_path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(format!(
        "could not allocate a unique backup name after {MAX_ARCHIVE_NAME_ATTEMPTS} attempts"
    ))
}

fn write_wtf_archive(
    wtf_dir: &Path,
    backup_dir: &Path,
    client: &str,
) -> Result<(String, PathBuf), String> {
    let (file_name, archive_path, file) = create_unique_archive_file(backup_dir)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    add_directory_to_zip(&mut zip, wtf_dir, "WTF", options)?;
    // Outside the WTF root, so restores (including older builds) never copy it into WTF.
    zip.start_file(CLIENT_MARKER_ENTRY, options)
        .map_err(|error| error.to_string())?;
    zip.write_all(format!("client={client}\n").as_bytes())
        .map_err(|error| error.to_string())?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok((file_name, archive_path))
}

fn extract_wtf_archive(archive_path: &Path, destination_root: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let output_path = archive_output_path(&entry, destination_root)?;
        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| error.to_string())?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = File::create(&output_path).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn archive_output_path(
    entry: &zip::read::ZipFile<'_>,
    destination_root: &Path,
) -> Result<PathBuf, String> {
    let enclosed = safe_entry_relative_path(entry.name())
        .ok_or_else(|| format!("backup archive contains unsafe path: {}", entry.name()))?;
    let output_path = destination_root.join(enclosed);
    Ok(output_path)
}

fn add_directory_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    zip_prefix: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    zip.add_directory(format!("{zip_prefix}/"), options)
        .map_err(|error| error.to_string())?;

    let mut entries = fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let zip_path = format!("{zip_prefix}/{name}");
        if path.is_dir() {
            add_directory_to_zip(zip, &path, &zip_path, options)?;
            continue;
        }

        let mut source = File::open(&path).map_err(|error| error.to_string())?;
        zip.start_file(zip_path, options)
            .map_err(|error| error.to_string())?;
        let mut buffer = Vec::new();
        source
            .read_to_end(&mut buffer)
            .map_err(|error| error.to_string())?;
        zip.write_all(&buffer).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn copy_dir_recursively(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;

    let mut entries = fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursively(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Read;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use time::macros::datetime;
    use time::{Duration, OffsetDateTime};
    use zip::ZipArchive;

    use super::{
        BACKUP_TIMESTAMP_FORMAT, PreUpdateBackup, PreUpdateBackupSettings, adopt_legacy_backups,
        backup_root, client_backup_dir, client_backup_key, create_unique_archive_file,
        create_wtf_backup, delete_backup, derive_wtf_dir, list_backups, restore_wtf_backup,
        run_pre_update_backup,
    };

    #[test]
    fn derive_wtf_dir_moves_from_addons_to_wtf() {
        let root = PathBuf::from("D:/Sandbox/WoW")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let wtf = derive_wtf_dir(&root).expect_err("missing wtf should fail");
        let normalized = wtf.replace('\\', "/");
        assert!(normalized.contains("D:/Sandbox/WoW/_retail_/WTF"));
    }

    #[test]
    fn derive_wtf_dir_uses_forever_client_folder() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("WoW")
            .join("_forever_")
            .join("Interface")
            .join("AddOns");
        let wtf_dir = temp.path().join("WoW").join("_forever_").join("WTF");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(&wtf_dir).expect("create wtf dir");
        assert_eq!(derive_wtf_dir(&addon_dir).expect("wtf dir"), wtf_dir);
    }

    #[test]
    fn backup_root_is_profile_scoped_under_data_dir() {
        let path = backup_root(Path::new(r"C:\Users\archc\AppData\Roaming\lemonup\data"));
        assert!(path.ends_with(Path::new("backups").join("wtf")));
    }

    #[test]
    fn rapid_backups_never_overwrite_existing_archives() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("WoW")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let wtf_dir = temp.path().join("WoW").join("_retail_").join("WTF");
        let backup_dir = temp.path().join("data").join("backups").join("wtf");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(&wtf_dir).expect("create wtf dir");
        fs::write(wtf_dir.join("Config.wtf"), "SET test \"1\"\n").expect("write config");

        let mut names = Vec::new();
        for _ in 0..8 {
            let outcome = create_wtf_backup(&addon_dir, &backup_dir, 100).expect("backup");
            names.push(outcome.backup.file_name);
        }
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        assert_eq!(list_backups(&backup_dir).expect("list").len(), 8);

        let pruned = create_wtf_backup(&addon_dir, &backup_dir, 3).expect("pruning backup");
        assert_eq!(pruned.backups.len(), 3);
        assert_eq!(pruned.pruned_files.len(), 6);
    }

    #[test]
    fn create_unique_archive_file_skips_existing_names() {
        let temp = tempdir().expect("tempdir");
        let (first_name, first_path, _file) =
            create_unique_archive_file(temp.path()).expect("first");
        fs::write(&first_path, "sentinel").expect("write sentinel");
        let (second_name, _, _file2) = create_unique_archive_file(temp.path()).expect("second");
        assert_ne!(first_name, second_name);
        assert_eq!(fs::read_to_string(&first_path).expect("read"), "sentinel");
    }

    #[test]
    fn create_backup_archives_wtf_and_prunes_old_entries() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("WoW")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let wtf_dir = temp.path().join("WoW").join("_retail_").join("WTF");
        let backup_dir = temp.path().join("data").join("backups").join("wtf");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(wtf_dir.join("Account")).expect("create wtf subdir");
        fs::write(wtf_dir.join("Config.wtf"), "SET test \"1\"\n").expect("write config");
        fs::write(
            wtf_dir.join("Account").join("bindings-cache.wtf"),
            "bindings\n",
        )
        .expect("write nested config");

        let first = create_wtf_backup(&addon_dir, &backup_dir, 1).expect("first backup");
        fs::write(wtf_dir.join("Config.wtf"), "SET test \"2\"\n").expect("update config");
        let second = create_wtf_backup(&addon_dir, &backup_dir, 1).expect("second backup");

        assert_eq!(second.backups.len(), 1);
        assert_eq!(second.pruned_files.len(), 1);
        assert_ne!(first.backup.file_name, second.backup.file_name);

        let backups = list_backups(&backup_dir).expect("list backups");
        assert_eq!(backups.len(), 1);

        let archive = File::open(&backups[0].path).expect("open archive");
        let mut zip = ZipArchive::new(archive).expect("open zip");
        let mut root_file = String::new();
        zip.by_name("WTF/Config.wtf")
            .expect("backup has root config")
            .read_to_string(&mut root_file)
            .expect("read root file");
        assert!(root_file.contains("SET test \"2\""));

        let mut nested_file = String::new();
        zip.by_name("WTF/Account/bindings-cache.wtf")
            .expect("backup has nested file")
            .read_to_string(&mut nested_file)
            .expect("read nested file");
        assert!(nested_file.contains("bindings"));
    }

    #[test]
    fn restore_backup_replaces_existing_wtf_contents() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("WoW")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let wtf_dir = temp.path().join("WoW").join("_retail_").join("WTF");
        let backup_dir = temp.path().join("data").join("backups").join("wtf");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(wtf_dir.join("Account")).expect("create wtf dir");
        fs::write(wtf_dir.join("Config.wtf"), "SET current \"1\"\n").expect("write current");

        let outcome = create_wtf_backup(&addon_dir, &backup_dir, 3).expect("backup");
        fs::write(wtf_dir.join("Config.wtf"), "SET current \"2\"\n").expect("mutate current");
        fs::write(wtf_dir.join("Layout-local.txt"), "transient\n").expect("write transient");

        restore_wtf_backup(&addon_dir, &backup_dir, &outcome.backup.path).expect("restore");

        let config = fs::read_to_string(wtf_dir.join("Config.wtf")).expect("restored config");
        assert!(config.contains("SET current \"1\""));
        assert!(!wtf_dir.join("Layout-local.txt").exists());
    }

    #[test]
    fn delete_backup_removes_archive_and_refreshes_listing() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("WoW")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let wtf_dir = temp.path().join("WoW").join("_retail_").join("WTF");
        let backup_dir = temp.path().join("data").join("backups").join("wtf");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(&wtf_dir).expect("create wtf dir");
        fs::write(wtf_dir.join("Config.wtf"), "SET test \"1\"\n").expect("write config");

        let first = create_wtf_backup(&addon_dir, &backup_dir, 5).expect("first backup");
        fs::write(wtf_dir.join("Config.wtf"), "SET test \"2\"\n").expect("update config");
        let second = create_wtf_backup(&addon_dir, &backup_dir, 5).expect("second backup");

        let outcome = delete_backup(&backup_dir, &first.backup.path).expect("delete backup");

        assert_eq!(outcome.deleted.file_name, first.backup.file_name);
        assert_eq!(outcome.backups.len(), 1);
        assert_eq!(outcome.backups[0].file_name, second.backup.file_name);
        assert!(!first.backup.path.exists());
    }

    #[test]
    fn extract_wtf_archive_nests_backslash_entries_and_rejects_traversal() {
        use std::io::Write;

        let workspace = tempdir().expect("tempdir");
        let archive_path = workspace.path().join("backup.zip");
        {
            let file = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.add_directory("WTF\\Account\\", options)
                .expect("add dir");
            zip.start_file("WTF\\Account\\config.wtf", options)
                .expect("start file");
            zip.write_all(b"SET x 1").expect("write file");
            zip.finish().expect("finish zip");
        }

        let destination = workspace.path().join("out");
        super::extract_wtf_archive(&archive_path, &destination).expect("extract");
        assert!(
            destination
                .join("WTF")
                .join("Account")
                .join("config.wtf")
                .is_file()
        );

        let evil_path = workspace.path().join("evil.zip");
        {
            let file = File::create(&evil_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("..\\..\\evil", options).expect("start file");
            zip.write_all(b"x").expect("write file");
            zip.finish().expect("finish zip");
        }
        assert!(super::extract_wtf_archive(&evil_path, &workspace.path().join("out2")).is_err());
    }

    fn wtf_layout(root: &Path) -> PathBuf {
        let addons = root.join("_retail_").join("Interface").join("AddOns");
        fs::create_dir_all(&addons).expect("addons dir");
        let wtf = root.join("_retail_").join("WTF");
        fs::create_dir_all(&wtf).expect("wtf dir");
        fs::write(wtf.join("Config.wtf"), "SET x 1").expect("wtf file");
        addons
    }

    fn settings(enabled: bool, backup_dir: &Path) -> PreUpdateBackupSettings {
        PreUpdateBackupSettings {
            enabled,
            retention: 5,
            backup_dir: backup_dir.to_path_buf(),
        }
    }

    fn seed_backup(backup_dir: &Path, at: OffsetDateTime) {
        fs::create_dir_all(backup_dir).expect("backup dir");
        let stamp = at.format(BACKUP_TIMESTAMP_FORMAT).expect("format");
        fs::write(backup_dir.join(format!("WTF-{stamp}.zip")), b"seed").expect("seed backup");
    }

    const NOW: OffsetDateTime = datetime!(2026-03-01 12:00:00 UTC);

    #[test]
    fn pre_update_backup_disabled_does_nothing() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");

        let result = run_pre_update_backup(&settings(false, &backup_dir), &addons, NOW);

        assert_eq!(result, PreUpdateBackup::Disabled);
        assert_eq!(result.cli_line(), None);
        assert!(!backup_dir.exists());
    }

    #[test]
    fn pre_update_backup_creates_when_none_exists() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        let PreUpdateBackup::Created { path } = &result else {
            panic!("expected created, got {result:?}");
        };
        assert!(path.is_file());
        assert!(
            result
                .cli_line()
                .expect("line")
                .starts_with("wtf backup: created ")
        );
        assert_eq!(
            result.status_note().as_deref(),
            Some("backed up your game settings first")
        );
    }

    #[test]
    fn pre_update_backup_throttles_just_under_sixty_minutes() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");
        seed_backup(
            &backup_dir,
            NOW - Duration::minutes(59) - Duration::seconds(59),
        );

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        assert_eq!(result, PreUpdateBackup::Skipped { minutes_ago: 59 });
        assert_eq!(
            result.cli_line().as_deref(),
            Some("wtf backup: skipped (last backup 59 min ago)")
        );
        assert_eq!(result.status_note(), None);
        assert_eq!(list_backups(&backup_dir).expect("list").len(), 1);
    }

    #[test]
    fn pre_update_backup_runs_at_exactly_sixty_minutes() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");
        seed_backup(&backup_dir, NOW - Duration::minutes(60));

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        assert!(matches!(result, PreUpdateBackup::Created { .. }));
        assert_eq!(list_backups(&backup_dir).expect("list").len(), 2);
    }

    #[test]
    fn pre_update_backup_throttle_uses_newest_backup() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");
        seed_backup(&backup_dir, NOW - Duration::hours(5));
        seed_backup(&backup_dir, NOW - Duration::minutes(12));

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        assert_eq!(result, PreUpdateBackup::Skipped { minutes_ago: 12 });
    }

    #[test]
    fn pre_update_backup_failure_is_a_warning_not_an_error() {
        let temp = tempdir().expect("tempdir");
        // No WTF folder next to the client dir.
        let addons = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&addons).expect("addons dir");
        let backup_dir = temp.path().join("backups");

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        let PreUpdateBackup::Warning(message) = &result else {
            panic!("expected warning, got {result:?}");
        };
        assert!(message.contains("WTF folder missing"));
        assert!(
            result
                .cli_line()
                .expect("line")
                .starts_with("wtf backup warning: ")
        );
        assert!(
            result
                .status_note()
                .expect("note")
                .starts_with("couldn't back up your game settings")
        );
    }

    #[test]
    fn pre_update_backup_future_stamped_backup_does_not_throttle() {
        let temp = tempdir().expect("tempdir");
        let addons = wtf_layout(temp.path());
        let backup_dir = temp.path().join("backups");
        seed_backup(&backup_dir, NOW + Duration::minutes(30));

        let result = run_pre_update_backup(&settings(true, &backup_dir), &addons, NOW);

        assert!(matches!(result, PreUpdateBackup::Created { .. }));
    }

    fn client_layout(root: &Path, client: &str) -> PathBuf {
        let addons = root.join(client).join("Interface").join("AddOns");
        fs::create_dir_all(&addons).expect("addons dir");
        let wtf = root.join(client).join("WTF");
        fs::create_dir_all(&wtf).expect("wtf dir");
        fs::write(wtf.join("Config.wtf"), client).expect("wtf file");
        addons
    }

    #[test]
    fn client_backup_key_comes_from_the_client_folder() {
        assert_eq!(
            client_backup_key(Path::new("D:/WoW/_retail_/Interface/AddOns")),
            "retail"
        );
        assert_eq!(
            client_backup_key(Path::new("D:/WoW/_forever_/Interface/AddOns")),
            "forever"
        );
        assert_eq!(
            client_backup_key(Path::new("D:/WoW/_Classic Era_/Interface/AddOns")),
            "classic-era"
        );
    }

    #[test]
    fn clients_list_and_prune_their_own_backups() {
        let temp = tempdir().expect("tempdir");
        let retail = client_layout(temp.path(), "_retail_");
        let forever = client_layout(temp.path(), "_forever_");
        let root = temp.path().join("backups");
        let retail_dir = client_backup_dir(&root, &retail);
        let forever_dir = client_backup_dir(&root, &forever);
        assert_eq!(retail_dir, root.join("retail"));

        create_wtf_backup(&forever, &forever_dir, 1).expect("forever backup");
        create_wtf_backup(&retail, &retail_dir, 1).expect("retail backup");
        let second = create_wtf_backup(&retail, &retail_dir, 1).expect("retail backup 2");

        assert_eq!(second.pruned_files.len(), 1, "retention prunes only retail");
        assert_eq!(list_backups(&retail_dir).expect("list").len(), 1);
        assert_eq!(list_backups(&forever_dir).expect("list").len(), 1);
    }

    #[test]
    fn pre_update_throttle_is_per_client() {
        let temp = tempdir().expect("tempdir");
        let retail = client_layout(temp.path(), "_retail_");
        let forever = client_layout(temp.path(), "_forever_");
        let root = temp.path().join("backups");
        seed_backup(
            &client_backup_dir(&root, &retail),
            NOW - Duration::minutes(3),
        );

        let retail_run = run_pre_update_backup(
            &settings(true, &client_backup_dir(&root, &retail)),
            &retail,
            NOW,
        );
        assert_eq!(retail_run, PreUpdateBackup::Skipped { minutes_ago: 3 });

        let forever_run = run_pre_update_backup(
            &settings(true, &client_backup_dir(&root, &forever)),
            &forever,
            NOW,
        );
        assert!(
            matches!(forever_run, PreUpdateBackup::Created { .. }),
            "{forever_run:?}"
        );
    }

    #[test]
    fn restore_refuses_another_clients_archive() {
        let temp = tempdir().expect("tempdir");
        let retail = client_layout(temp.path(), "_retail_");
        let forever = client_layout(temp.path(), "_forever_");
        let root = temp.path().join("backups");
        let retail_dir = client_backup_dir(&root, &retail);
        let retail_backup = create_wtf_backup(&retail, &retail_dir, 5).expect("retail backup");

        // Refused by the client recorded inside the archive, wherever it is.
        let copied = temp.path().join("elsewhere.zip");
        fs::copy(&retail_backup.backup.path, &copied).expect("copy");
        let error = restore_wtf_backup(&forever, &client_backup_dir(&root, &forever), &copied)
            .expect_err("retail archive into forever");
        assert!(error.contains("'retail' client"), "{error}");

        // Refused by its folder in the store when it carries no client record (older build).
        let unmarked_dir = root.join("retail");
        let unmarked = unmarked_dir.join("WTF-20250101T000000000Z.zip");
        {
            let file = fs::File::create(&unmarked).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            zip.add_directory("WTF/", zip::write::SimpleFileOptions::default())
                .expect("dir");
            zip.finish().expect("finish");
        }
        let error = restore_wtf_backup(&forever, &client_backup_dir(&root, &forever), &unmarked)
            .expect_err("unmarked retail archive into forever");
        assert!(error.contains("'retail' client"), "{error}");

        let forever_wtf = temp.path().join("_forever_").join("WTF").join("Config.wtf");
        assert_eq!(fs::read_to_string(forever_wtf).expect("wtf"), "_forever_");
        restore_wtf_backup(&retail, &retail_dir, &retail_backup.backup.path)
            .expect("own archive restores");
    }

    #[test]
    fn legacy_flat_backups_are_adopted_by_the_given_client() {
        let temp = tempdir().expect("tempdir");
        let retail = client_layout(temp.path(), "_retail_");
        let root = temp.path().join("backups");
        seed_backup(&root, NOW);
        seed_backup(&root, NOW - Duration::hours(2));

        let moved = adopt_legacy_backups(&root, &retail).expect("adopt");
        assert_eq!(moved.len(), 2);
        assert!(list_backups(&root).expect("root").is_empty());
        assert_eq!(list_backups(&root.join("retail")).expect("retail").len(), 2);
        assert!(
            adopt_legacy_backups(&root, &retail)
                .expect("again")
                .is_empty()
        );
    }
}
