use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lemonup_core::{
    AddonRecord, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, scan_addons_dir,
};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use time::OffsetDateTime;
use zip::ZipArchive;

const API_BASE: &str = "https://api.mmoui.com/v3/game/WOW/filedetails";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WowinterfaceInstallSummary {
    pub(crate) addon_id: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) version: Option<String>,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WowinterfaceUpdateSummary {
    pub(crate) addon_id: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) previous_version: Option<String>,
    pub(crate) remote_version: Option<String>,
    pub(crate) updated: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WowinterfaceRemoteVersion {
    pub(crate) source_url: Option<String>,
    pub(crate) version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct WowinterfaceAddon {
    #[serde(rename = "UID")]
    uid: String,
    #[serde(rename = "UIName")]
    ui_name: String,
    #[serde(rename = "UIVersion")]
    ui_version: String,
    #[serde(rename = "UIDownload")]
    ui_download: String,
    #[serde(rename = "UIAuthorName")]
    ui_author_name: String,
    #[serde(rename = "UIFileName")]
    ui_file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct WowinterfaceError {
    #[serde(rename = "ERROR")]
    error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum WowinterfaceResponse {
    Addons(Vec<WowinterfaceAddon>),
    Error(WowinterfaceError),
}

pub(crate) fn parse_wowinterface_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("WoWInterface addon target cannot be empty".to_string());
    }

    if trimmed.len() > 300 {
        return Err("WoWInterface addon target is too long".to_string());
    }

    if trimmed.chars().any(|character| character.is_control()) {
        return Err("WoWInterface addon target contains forbidden characters".to_string());
    }

    let url = Url::parse(trimmed)
        .map_err(|_| "WoWInterface install currently requires a full addon page URL".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "WoWInterface URL is missing a host".to_string())?;
    if !host.ends_with("wowinterface.com") {
        return Err("WoWInterface URL must point to wowinterface.com".to_string());
    }

    parse_wowinterface_id_from_url(trimmed)
}

pub(crate) async fn install_wowinterface_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    dry_run: bool,
) -> Result<WowinterfaceInstallSummary, String> {
    let addon_id = normalize_wowinterface_install_target(target)?;
    let client = build_wowinterface_client()?;
    let addon = fetch_wowinterface_addon(&client, &addon_id).await?;
    let bytes = download_wowinterface_release_bytes(&client, &addon).await?;
    install_downloaded_wowinterface_addon(state_database, addon_dir, &addon, &bytes, dry_run)
}

pub(crate) async fn fetch_wowinterface_remote_version(
    addon: &AddonRecord,
) -> Result<WowinterfaceRemoteVersion, String> {
    let addon_id = resolve_tracked_wowinterface_id(addon)?;
    let client = build_wowinterface_client()?;
    let remote = fetch_wowinterface_addon(&client, &addon_id).await?;
    Ok(WowinterfaceRemoteVersion {
        source_url: Some(canonical_source_url(&remote.uid)),
        version: Some(remote.ui_version.clone()),
    })
}

pub(crate) async fn update_wowinterface_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    force: bool,
    dry_run: bool,
) -> Result<WowinterfaceUpdateSummary, String> {
    if tracked.source != SourceKind::WowInterface {
        return Err(format!(
            "addon '{}' is not tracked as WoWInterface",
            tracked.folder
        ));
    }

    let addon_id = resolve_tracked_wowinterface_id(tracked)?;
    let client = build_wowinterface_client()?;
    let remote = fetch_wowinterface_addon(&client, &addon_id).await?;
    let remote_version = Some(remote.ui_version.clone());

    if !force && versions_match(tracked.version.as_deref(), remote_version.as_deref()) {
        return Ok(WowinterfaceUpdateSummary {
            addon_id: remote.uid.clone(),
            addon_name: remote.ui_name.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: std::iter::once(tracked.folder.clone())
                .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
                .collect(),
            previous_version: tracked.version.clone(),
            remote_version,
            updated: false,
            dry_run,
        });
    }

    let bytes = download_wowinterface_release_bytes(&client, &remote).await?;
    update_downloaded_wowinterface_addon(
        state_database,
        addon_dir,
        tracked,
        &remote,
        &bytes,
        dry_run,
    )
}

fn build_wowinterface_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("LemonUp/2 (+https://github.com/archcorsair/lemonup)")
        .build()
        .map_err(|error| error.to_string())
}

async fn fetch_wowinterface_addon(
    client: &Client,
    addon_id: &str,
) -> Result<WowinterfaceAddon, String> {
    let url = format!("{API_BASE}/{addon_id}.json");
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(format!("WoWInterface addon '{addon_id}' was not found"));
    }

    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    match response
        .json::<WowinterfaceResponse>()
        .await
        .map_err(|error| error.to_string())?
    {
        WowinterfaceResponse::Addons(addons) => addons.into_iter().next().ok_or_else(|| {
            format!("WoWInterface addon '{addon_id}' returned an empty details payload")
        }),
        WowinterfaceResponse::Error(error) => {
            if error.error.contains("No AddOn found") {
                Err(format!("WoWInterface addon '{addon_id}' was not found"))
            } else {
                Err(format!("WoWInterface API error: {}", error.error))
            }
        }
    }
}

async fn download_wowinterface_release_bytes(
    client: &Client,
    addon: &WowinterfaceAddon,
) -> Result<Vec<u8>, String> {
    let download_url = validate_wowinterface_download_url(&addon.ui_download)?;
    client
        .get(download_url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| error.to_string())
}

fn install_downloaded_wowinterface_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    addon: &WowinterfaceAddon,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<WowinterfaceInstallSummary, String> {
    let temp_root = create_temp_work_dir("wowi-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let target_name = addon.ui_file_name.trim_end_matches(".zip");
        let parent_folder =
            determine_parent_folder(&extracted.folders, target_name, Some(&addon.ui_name))?;
        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == parent_folder)
            .ok_or_else(|| {
                format!(
                    "WoWInterface package for '{}' did not yield a scannable parent folder '{}'",
                    addon.ui_name, parent_folder
                )
            })?;

        if dry_run {
            return Ok(WowinterfaceInstallSummary {
                addon_id: addon.uid.clone(),
                addon_name: addon.ui_name.clone(),
                parent_folder,
                installed_folders: extracted.folders,
                version: Some(addon.ui_version.clone()),
                dry_run: true,
            });
        }

        preflight_install_targets(addon_dir, &extracted.folders)?;
        let mut copied_folders = Vec::new();
        for folder in &extracted.folders {
            copy_dir_recursively(&extracted.source_root.join(folder), &addon_dir.join(folder))?;
            copied_folders.push(folder.clone());
        }

        let record = build_managed_wowinterface_record(addon, parent_metadata, &extracted.folders);
        if let Err(error) = state_database.record_managed_addon(&record) {
            for folder in copied_folders.iter().rev() {
                let _ = fs::remove_dir_all(addon_dir.join(folder));
            }
            return Err(error.to_string());
        }

        Ok(WowinterfaceInstallSummary {
            addon_id: addon.uid.clone(),
            addon_name: addon.ui_name.clone(),
            parent_folder,
            installed_folders: extracted.folders,
            version: Some(addon.ui_version.clone()),
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn update_downloaded_wowinterface_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    addon: &WowinterfaceAddon,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<WowinterfaceUpdateSummary, String> {
    let temp_root = create_temp_work_dir("wowi-update")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let backup_root = temp_root.join("backup");
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let previous_version = tracked.version.clone();
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let target_name = addon.ui_file_name.trim_end_matches(".zip");
        let resolved_parent =
            determine_parent_folder(&extracted.folders, target_name, Some(&addon.ui_name))?;
        if resolved_parent != tracked.folder {
            return Err(format!(
                "WoWInterface update for '{}' resolved parent '{}' but tracked addon folder is '{}'",
                addon.ui_name, resolved_parent, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == tracked.folder)
            .ok_or_else(|| {
                format!(
                    "updated WoWInterface package for '{}' did not yield a scannable tracked parent folder '{}'",
                    addon.ui_name, tracked.folder
                )
            })?;

        let managed_folders = std::iter::once(tracked.folder.clone())
            .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
            .collect::<Vec<_>>();

        preflight_update_targets(addon_dir, &extracted.folders, &managed_folders)?;

        if dry_run {
            return Ok(WowinterfaceUpdateSummary {
                addon_id: addon.uid.clone(),
                addon_name: addon.ui_name.clone(),
                parent_folder: tracked.folder.clone(),
                installed_folders: extracted.folders,
                previous_version,
                remote_version: Some(addon.ui_version.clone()),
                updated: true,
                dry_run: true,
            });
        }

        let mut moved_existing = Vec::new();
        for folder in &managed_folders {
            let source = addon_dir.join(folder);
            if !source.exists() {
                continue;
            }
            let backup = backup_root.join(folder);
            copy_dir_recursively(&source, &backup)?;
            fs::remove_dir_all(&source).map_err(|error| error.to_string())?;
            moved_existing.push(folder.clone());
        }

        let mut copied_new = Vec::new();
        for folder in &extracted.folders {
            copy_dir_recursively(&extracted.source_root.join(folder), &addon_dir.join(folder))?;
            copied_new.push(folder.clone());
        }

        let record = build_managed_wowinterface_record(addon, parent_metadata, &extracted.folders);
        if let Err(error) = state_database.record_managed_addon(&record) {
            rollback_updated_folders(addon_dir, &backup_root, &copied_new, &moved_existing);
            return Err(error.to_string());
        }

        Ok(WowinterfaceUpdateSummary {
            addon_id: addon.uid.clone(),
            addon_name: addon.ui_name.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: extracted.folders,
            previous_version,
            remote_version: Some(addon.ui_version.clone()),
            updated: true,
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

#[derive(Debug, Clone)]
struct ExtractedFolders {
    source_root: PathBuf,
    folders: Vec<String>,
}

fn build_managed_wowinterface_record(
    addon: &WowinterfaceAddon,
    parent_metadata: &ScannedAddon,
    installed_folders: &[String],
) -> AddonRecord {
    let mut record = AddonRecord::new(
        parent_metadata.name.clone(),
        parent_metadata.folder.clone(),
        SourceKind::WowInterface,
    );
    record.kind = parent_metadata.kind;
    record.flavor = parent_metadata.flavor;
    record.version = Some(addon.ui_version.clone()).or_else(|| parent_metadata.version.clone());
    record.author = Some(addon.ui_author_name.clone()).or_else(|| parent_metadata.author.clone());
    record.interface = parent_metadata.interface.clone();
    record.source_url = Some(canonical_source_url(&addon.uid));
    record.required_deps = parent_metadata.required_deps.clone();
    record.optional_deps = parent_metadata.optional_deps.clone();
    record.embedded_libs = parent_metadata.embedded_libs.clone();
    record.remote_version = Some(addon.ui_version.clone());
    record.last_checked_at = Some(OffsetDateTime::now_utc());
    record.set_managed_owned_folders(
        installed_folders
            .iter()
            .filter(|folder| **folder != parent_metadata.folder)
            .cloned()
            .map(|name| OwnedFolder { name })
            .collect(),
    );
    record
}

fn resolve_tracked_wowinterface_id(addon: &AddonRecord) -> Result<String, String> {
    addon
        .source_url
        .as_deref()
        .ok_or_else(|| {
            format!(
                "tracked WoWInterface addon '{}' is missing a source URL",
                addon.folder
            )
        })
        .and_then(parse_wowinterface_id_from_url)
}

fn normalize_wowinterface_install_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }
    parse_wowinterface_target(trimmed)
}

fn parse_wowinterface_id_from_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "WoWInterface URL is invalid".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "WoWInterface URL is missing a host".to_string())?;
    if !host.ends_with("wowinterface.com") {
        return Err("WoWInterface URL must point to wowinterface.com".to_string());
    }
    let rendered = format!(
        "{}{}",
        url.path(),
        url.query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    let Some(index) = rendered.find("info") else {
        return Err("WoWInterface URL must contain an addon id like info25687".to_string());
    };
    let digits = rendered[index + 4..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return Err("WoWInterface URL is missing a numeric addon id".to_string());
    }
    Ok(digits)
}

fn canonical_source_url(addon_id: &str) -> String {
    format!("https://www.wowinterface.com/downloads/info{addon_id}.html")
}

fn validate_wowinterface_download_url(value: &str) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|error| error.to_string())?;
    let Some(host) = parsed.host_str() else {
        return Err("WoWInterface download URL is missing a host".to_string());
    };
    if !(host.ends_with("wowinterface.com")
        || host.ends_with("mmoui.com")
        || host.ends_with("cdn.wowinterface.com"))
    {
        return Err(
            "WoWInterface download URL must stay on a WoWInterface-controlled host".to_string(),
        );
    }
    Ok(parsed)
}

fn discover_extracted_folders(root: &Path) -> Result<ExtractedFolders, String> {
    let direct = discover_first_level_addon_folders(root)?;
    if !direct.is_empty() {
        return Ok(ExtractedFolders {
            source_root: root.to_path_buf(),
            folders: direct,
        });
    }

    let first_level_dirs = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry)
        })
        .collect::<Vec<_>>();

    if first_level_dirs.len() == 1 {
        let nested_root = first_level_dirs[0].path();
        let nested = discover_first_level_addon_folders(&nested_root)?;
        if !nested.is_empty() {
            return Ok(ExtractedFolders {
                source_root: nested_root,
                folders: nested,
            });
        }
    }

    Err(
        "no top-level addon folders with root .toc files were found in the WoWInterface archive"
            .to_string(),
    )
}

fn discover_first_level_addon_folders(root: &Path) -> Result<Vec<String>, String> {
    let mut folders = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        if has_root_toc_files(&entry.path())? {
            folders.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    folders.sort_by_key(|folder| folder.to_ascii_lowercase());
    Ok(folders)
}

fn has_root_toc_files(path: &Path) -> Result<bool, String> {
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.to_ascii_lowercase().ends_with(".toc") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn determine_parent_folder(
    folders: &[String],
    target_name: &str,
    ui_name: Option<&str>,
) -> Result<String, String> {
    if folders.is_empty() {
        return Err("no addon folders were discovered in the WoWInterface archive".to_string());
    }
    if folders.len() == 1 {
        return Ok(folders[0].clone());
    }

    let normalized_target_name = normalize_folder_like(target_name);
    if let Some(exact) = folders
        .iter()
        .find(|folder| normalize_folder_like(folder) == normalized_target_name)
    {
        return Ok(exact.clone());
    }

    if let Some(name) = ui_name {
        let normalized_ui_name = normalize_folder_like(name);
        if let Some(exact) = folders
            .iter()
            .find(|folder| normalize_folder_like(folder) == normalized_ui_name)
        {
            return Ok(exact.clone());
        }
    }

    let mut sorted = folders.to_vec();
    sorted.sort_by_key(|folder| folder.len());
    let shortest = sorted[0].clone();
    let prefix_matches = sorted
        .iter()
        .filter(|folder| folder.starts_with(&shortest))
        .count();
    if prefix_matches * 2 >= sorted.len() {
        return Ok(shortest);
    }

    if !target_name.is_empty() {
        let normalized_target = normalize_folder_like(target_name);
        let mut candidates = folders
            .iter()
            .filter(|folder| {
                let normalized = normalize_folder_like(folder);
                normalized_target.contains(&normalized) || normalized.contains(&normalized_target)
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|folder| folder.len());
        if let Some(candidate) = candidates.first() {
            return Ok(candidate.clone());
        }
    }

    Ok(shortest)
}

fn normalize_folder_like(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-' && *character != '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn versions_match(installed: Option<&str>, remote: Option<&str>) -> bool {
    match (installed, remote) {
        (Some(left), Some(right)) => normalize_version(left) == normalize_version(right),
        _ => false,
    }
}

fn normalize_version(value: &str) -> String {
    let trimmed = value.trim();
    let normalized = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .filter(|rest| rest.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .unwrap_or(trimmed);
    normalized.to_ascii_lowercase()
}

fn preflight_install_targets(addon_dir: &Path, folders: &[String]) -> Result<(), String> {
    let collisions = folders
        .iter()
        .filter(|folder| addon_dir.join(folder).exists())
        .cloned()
        .collect::<Vec<_>>();
    if collisions.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "install refused because these addon folders already exist: {}",
            collisions.join(", ")
        ))
    }
}

fn preflight_update_targets(
    addon_dir: &Path,
    new_folders: &[String],
    replaceable_folders: &[String],
) -> Result<(), String> {
    let replaceable = replaceable_folders
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let collisions = new_folders
        .iter()
        .filter(|folder| addon_dir.join(folder).exists() && !replaceable.contains(folder))
        .cloned()
        .collect::<Vec<_>>();
    if collisions.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "update refused because these addon folders are owned by something else on disk: {}",
            collisions.join(", ")
        ))
    }
}

fn extract_zip_archive(zip_path: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let file = File::open(zip_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative_path) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        let output_path = destination.join(relative_path);

        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| error.to_string())?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = File::create(&output_path).map_err(|error| error.to_string())?;
        io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn copy_dir_recursively(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_dir_recursively(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn create_temp_work_dir(prefix: &str) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let root = std::env::temp_dir();

    for attempt in 0..100u16 {
        let candidate = if attempt == 0 {
            root.join(format!("lemonup-{prefix}-{timestamp}"))
        } else {
            root.join(format!("lemonup-{prefix}-{timestamp}-{attempt}"))
        };
        if !candidate.exists() {
            fs::create_dir_all(&candidate).map_err(|error| error.to_string())?;
            return Ok(candidate);
        }
    }

    Err("failed to create a unique temporary WoWInterface work directory".to_string())
}

fn rollback_updated_folders(
    addon_dir: &Path,
    backup_root: &Path,
    copied_new: &[String],
    moved_existing: &[String],
) {
    for folder in copied_new.iter().rev() {
        let _ = fs::remove_dir_all(addon_dir.join(folder));
    }

    for folder in moved_existing {
        let backup = backup_root.join(folder);
        if !backup.exists() {
            continue;
        }
        let restored = addon_dir.join(folder);
        let _ = copy_dir_recursively(&backup, &restored);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WowinterfaceResponse, build_managed_wowinterface_record, determine_parent_folder,
        parse_wowinterface_id_from_url, parse_wowinterface_target,
        update_downloaded_wowinterface_addon, versions_match,
    };
    use lemonup_core::{AddonRecord, SourceKind, StateDatabase};
    use tempfile::tempdir;

    fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (path, body) in entries {
                zip.start_file(path, options).expect("start file");
                use std::io::Write;
                zip.write_all(body.as_bytes()).expect("write body");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    #[test]
    fn parse_wowinterface_target_accepts_common_urls() {
        assert_eq!(
            parse_wowinterface_target(
                "https://www.wowinterface.com/downloads/info25687-ElvUI_WindTools.html"
            )
            .expect("url"),
            "25687"
        );
        assert_eq!(
            parse_wowinterface_id_from_url("https://wowinterface.com/downloads/info12345.html")
                .expect("id"),
            "12345"
        );
        assert!(parse_wowinterface_target("12345").is_err());
        assert!(parse_wowinterface_target("https://example.com/info12345.html").is_err());
    }

    #[test]
    fn wowinterface_response_parses_array_and_error_shapes() {
        let array = serde_json::from_str::<WowinterfaceResponse>(
            r#"[{"UID":"25687","UIName":"ElvUI_WindTools","UIVersion":"4.16","UIDownload":"https://cdn.wowinterface.com/downloads/getfile.php?id=25687","UIAuthorName":"fang2hou","UIFileName":"ElvUI_WindTools-4.16.zip"}]"#,
        )
        .expect("array response");
        match array {
            WowinterfaceResponse::Addons(addons) => {
                assert_eq!(addons[0].uid, "25687");
                assert_eq!(addons[0].ui_name, "ElvUI_WindTools");
            }
            other => panic!("expected addon array, got {other:?}"),
        }

        let error = serde_json::from_str::<WowinterfaceResponse>(r#"{"ERROR":"No AddOn found."}"#)
            .expect("error response");
        match error {
            WowinterfaceResponse::Error(error) => {
                assert!(error.error.contains("No AddOn found"));
            }
            other => panic!("expected error object, got {other:?}"),
        }
    }

    #[test]
    fn determine_parent_folder_prefers_target_and_ui_name_matches() {
        let folders = vec![
            "ElvUI_WindTools".to_string(),
            "ElvUI_WindTools_Options".to_string(),
        ];
        assert_eq!(
            determine_parent_folder(&folders, "ElvUI_WindTools-4.16", Some("ElvUI_WindTools"))
                .expect("parent"),
            "ElvUI_WindTools"
        );
    }

    #[test]
    fn build_managed_wowinterface_record_sets_owned_folders() {
        let addon = super::WowinterfaceAddon {
            uid: "25687".to_string(),
            ui_name: "ElvUI_WindTools".to_string(),
            ui_version: "4.16".to_string(),
            ui_download: "https://cdn.wowinterface.com/downloads/getfile.php?id=25687".to_string(),
            ui_author_name: "fang2hou".to_string(),
            ui_file_name: "ElvUI_WindTools-4.16.zip".to_string(),
        };
        let scanned = lemonup_core::ScannedAddon {
            name: "ElvUI WindTools".to_string(),
            folder: "ElvUI_WindTools".to_string(),
            owned_folders: Vec::new(),
            kind: lemonup_core::AddonKind::Addon,
            flavor: lemonup_core::GameFlavor::Retail,
            version: Some("4.16".to_string()),
            git_commit: None,
            author: Some("fang2hou".to_string()),
            interface: Some("110205".to_string()),
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
        };

        let record = build_managed_wowinterface_record(
            &addon,
            &scanned,
            &[
                "ElvUI_WindTools".to_string(),
                "ElvUI_WindTools_Options".to_string(),
            ],
        );
        assert_eq!(record.source, SourceKind::WowInterface);
        assert_eq!(record.remote_version.as_deref(), Some("4.16"));
        assert_eq!(record.owned_folders.len(), 1);
        assert_eq!(record.owned_folders[0].name, "ElvUI_WindTools_Options");
    }

    #[test]
    fn update_downloaded_wowinterface_addon_preserves_parent_folder() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("ElvUI_WindTools")).expect("create addon dir");
        std::fs::write(
            addon_dir.join("ElvUI_WindTools").join("ElvUI_WindTools.toc"),
            "## Title: ElvUI WindTools\n## Version: 4.15\n## Author: fang2hou\n## Interface: 110205\n",
        )
        .expect("write toc");

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new(
            "ElvUI WindTools",
            "ElvUI_WindTools",
            SourceKind::WowInterface,
        );
        tracked.version = Some("4.15".to_string());
        tracked.source_url =
            Some("https://www.wowinterface.com/downloads/info25687.html".to_string());
        database.upsert_addon(&tracked).expect("seed addon");

        let addon = super::WowinterfaceAddon {
            uid: "25687".to_string(),
            ui_name: "ElvUI_WindTools".to_string(),
            ui_version: "4.16".to_string(),
            ui_download: "https://cdn.wowinterface.com/downloads/getfile.php?id=25687".to_string(),
            ui_author_name: "fang2hou".to_string(),
            ui_file_name: "ElvUI_WindTools-4.16.zip".to_string(),
        };
        let bytes = build_zip(&[(
            "wrapper/ElvUI_WindTools/ElvUI_WindTools.toc",
            "## Title: ElvUI WindTools\n## Version: 4.16\n## Author: fang2hou\n## Interface: 110205\n",
        )]);

        let summary = update_downloaded_wowinterface_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &addon,
            &bytes,
            false,
        )
        .expect("update");

        assert!(summary.updated);
        let stored = database
            .get_addon_by_folder("ElvUI_WindTools")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("4.16"));
        assert_eq!(stored.source, SourceKind::WowInterface);
    }

    #[test]
    fn versions_match_ignores_leading_v_prefix() {
        assert!(versions_match(Some("v4.16"), Some("4.16")));
        assert!(versions_match(Some("4.16"), Some("V4.16")));
        assert!(!versions_match(Some("v4.15"), Some("4.16")));
    }
}
