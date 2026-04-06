use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use tempfile::tempdir;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]T[hour][minute][second][subsecond digits:3]Z");
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

pub fn backup_root(data_dir: &Path) -> PathBuf {
    data_dir.join("backups").join("wtf")
}

pub fn derive_wtf_dir(addon_dir: &Path) -> Result<PathBuf, String> {
    let interface_dir = addon_dir
        .parent()
        .ok_or_else(|| format!("invalid addon directory: {}", addon_dir.display()))?;
    let flavor_dir = interface_dir
        .parent()
        .ok_or_else(|| format!("invalid addon directory: {}", addon_dir.display()))?;
    let wow_root = flavor_dir
        .parent()
        .ok_or_else(|| format!("invalid addon directory: {}", addon_dir.display()))?;
    let wtf_dir = wow_root.join("WTF");
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

    let timestamp = OffsetDateTime::now_utc()
        .format(BACKUP_TIMESTAMP_FORMAT)
        .map_err(|error| error.to_string())?;
    let file_name = format!("WTF-{timestamp}.zip");
    let archive_path = backup_dir.join(&file_name);
    write_wtf_archive(&wtf_dir, &archive_path)?;

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

pub fn restore_wtf_backup(
    addon_dir: &Path,
    archive_path: &Path,
) -> Result<BackupRestoreOutcome, String> {
    let wtf_dir = derive_wtf_dir(addon_dir)?;
    let backup = backup_entry_from_path(archive_path)?;
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

fn write_wtf_archive(wtf_dir: &Path, archive_path: &Path) -> Result<(), String> {
    let file = File::create(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    add_directory_to_zip(&mut zip, wtf_dir, "WTF", options)?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok(())
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
    let enclosed = entry
        .enclosed_name()
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
    use zip::ZipArchive;

    use super::{backup_root, create_wtf_backup, derive_wtf_dir, list_backups, restore_wtf_backup};

    #[test]
    fn derive_wtf_dir_moves_from_addons_to_wtf() {
        let root = PathBuf::from(r"D:\Sandbox\WoW\_retail_\Interface\AddOns");
        let wtf = derive_wtf_dir(&root).expect_err("missing wtf should fail");
        let normalized = wtf.replace('\\', "/");
        assert!(normalized.contains("D:/Sandbox/WoW/WTF"));
    }

    #[test]
    fn backup_root_is_profile_scoped_under_data_dir() {
        let path = backup_root(Path::new(r"C:\Users\archc\AppData\Roaming\lemonup\data"));
        assert!(path.ends_with(Path::new("backups").join("wtf")));
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
        let wtf_dir = temp.path().join("WoW").join("WTF");
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
        let wtf_dir = temp.path().join("WoW").join("WTF");
        let backup_dir = temp.path().join("data").join("backups").join("wtf");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(wtf_dir.join("Account")).expect("create wtf dir");
        fs::write(wtf_dir.join("Config.wtf"), "SET current \"1\"\n").expect("write current");

        let outcome = create_wtf_backup(&addon_dir, &backup_dir, 3).expect("backup");
        fs::write(wtf_dir.join("Config.wtf"), "SET current \"2\"\n").expect("mutate current");
        fs::write(wtf_dir.join("Layout-local.txt"), "transient\n").expect("write transient");

        restore_wtf_backup(&addon_dir, &outcome.backup.path).expect("restore");

        let config = fs::read_to_string(wtf_dir.join("Config.wtf")).expect("restored config");
        assert!(config.contains("SET current \"1\""));
        assert!(!wtf_dir.join("Layout-local.txt").exists());
    }
}
