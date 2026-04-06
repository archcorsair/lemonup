use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lemonup_core::{
    AddonRecord, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, scan_addons_dir,
};
use reqwest::{Client, Url};
use serde::Deserialize;
use time::OffsetDateTime;
use zip::ZipArchive;

const API_URL: &str = "https://api.tukui.org/v1/addons";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TukuiInstallSummary {
    pub(crate) addon_slug: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) version: Option<String>,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TukuiUpdateSummary {
    pub(crate) addon_slug: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) previous_version: Option<String>,
    pub(crate) remote_version: Option<String>,
    pub(crate) updated: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TukuiRemoteVersion {
    pub(crate) source_url: Option<String>,
    pub(crate) version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct TukuiAddon {
    slug: String,
    author: String,
    name: String,
    url: String,
    version: String,
    web_url: String,
    directories: Vec<String>,
}

#[derive(Debug, Clone)]
struct TukuiFeedCache {
    fetched_at: Instant,
    addons: Vec<TukuiAddon>,
}

static TUKUI_CACHE: OnceLock<Mutex<Option<TukuiFeedCache>>> = OnceLock::new();

pub(crate) fn parse_tukui_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("TukUI addon target cannot be empty".to_string());
    }

    if trimmed.len() > 200 {
        return Err("TukUI addon target is too long".to_string());
    }

    if trimmed.contains("..") || trimmed.chars().any(|character| character.is_control()) {
        return Err("TukUI addon target contains forbidden characters".to_string());
    }

    if let Ok(url) = Url::parse(trimmed) {
        let Some(host) = url.host_str() else {
            return Err("TukUI URL is missing a host".to_string());
        };
        if !host.ends_with("tukui.org") {
            return Err("TukUI URL must point to tukui.org".to_string());
        }
        let segments = url
            .path_segments()
            .map(|values| values.collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(slug) = segments
            .iter()
            .find_map(|segment| canonical_tukui_slug(segment))
        {
            return Ok(slug.to_string());
        }
        return Err("TukUI URL must point to the canonical ElvUI or Tukui package".to_string());
    }

    canonical_tukui_slug(trimmed)
        .map(str::to_string)
        .ok_or_else(|| {
            "Only the canonical ElvUI and Tukui targets are supported right now".to_string()
        })
}

pub(crate) async fn install_tukui_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    dry_run: bool,
) -> Result<TukuiInstallSummary, String> {
    let slug = parse_tukui_target(target)?;
    let client = build_tukui_client()?;
    let addon = resolve_tukui_addon(&client, &slug, false).await?;
    let bytes = download_tukui_release_bytes(&client, &addon).await?;
    install_downloaded_tukui_addon(state_database, addon_dir, &addon, &bytes, dry_run)
}

pub(crate) async fn fetch_tukui_remote_version(
    addon: &AddonRecord,
) -> Result<TukuiRemoteVersion, String> {
    let slug = resolve_tracked_tukui_target(addon)?;
    let client = build_tukui_client()?;
    let entry = resolve_tukui_addon(&client, &slug, false).await?;
    Ok(TukuiRemoteVersion {
        source_url: Some(entry.web_url.clone()),
        version: Some(entry.version.clone()),
    })
}

pub(crate) async fn update_tukui_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    force: bool,
    dry_run: bool,
) -> Result<TukuiUpdateSummary, String> {
    if tracked.source != SourceKind::Tukui {
        return Err(format!(
            "addon '{}' is not tracked as TukUI",
            tracked.folder
        ));
    }

    let slug = resolve_tracked_tukui_target(tracked)?;
    let client = build_tukui_client()?;
    let addon = resolve_tukui_addon(&client, &slug, false).await?;
    let remote_version = Some(addon.version.clone());

    if !force && versions_match(tracked.version.as_deref(), remote_version.as_deref()) {
        return Ok(TukuiUpdateSummary {
            addon_slug: addon.slug.clone(),
            addon_name: addon.name.clone(),
            parent_folder: canonical_parent_folder(&addon.slug).to_string(),
            installed_folders: canonical_install_folders(&addon.slug),
            previous_version: tracked.version.clone(),
            remote_version,
            updated: false,
            dry_run,
        });
    }

    let bytes = download_tukui_release_bytes(&client, &addon).await?;
    update_downloaded_tukui_addon(state_database, addon_dir, tracked, &addon, &bytes, dry_run)
}

fn build_tukui_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("LemonUp/2 (+https://github.com/archcorsair/lemonup)")
        .build()
        .map_err(|error| error.to_string())
}

async fn resolve_tukui_addon(
    client: &Client,
    slug: &str,
    force_refresh: bool,
) -> Result<TukuiAddon, String> {
    let addons = fetch_tukui_addons(client, force_refresh).await?;
    addons
        .into_iter()
        .find(|addon| {
            addon.slug.eq_ignore_ascii_case(slug) || addon.name.eq_ignore_ascii_case(slug)
        })
        .ok_or_else(|| {
            format!("could not find canonical TukUI package '{slug}' in the TukUI API feed")
        })
}

async fn fetch_tukui_addons(
    client: &Client,
    force_refresh: bool,
) -> Result<Vec<TukuiAddon>, String> {
    let cache = TUKUI_CACHE.get_or_init(|| Mutex::new(None));
    if !force_refresh {
        let guard = cache
            .lock()
            .map_err(|_| "TukUI cache lock poisoned".to_string())?;
        if let Some(existing) = guard.as_ref()
            && existing.fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(existing.addons.clone());
        }
    }

    let response = client
        .get(API_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    let addons = response
        .json::<Vec<TukuiAddon>>()
        .await
        .map_err(|error| error.to_string())?;

    let mut guard = cache
        .lock()
        .map_err(|_| "TukUI cache lock poisoned".to_string())?;
    *guard = Some(TukuiFeedCache {
        fetched_at: Instant::now(),
        addons: addons.clone(),
    });
    Ok(addons)
}

async fn download_tukui_release_bytes(
    client: &Client,
    addon: &TukuiAddon,
) -> Result<Vec<u8>, String> {
    let download_url = validate_tukui_download_url(&addon.url)?;
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

fn install_downloaded_tukui_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    addon: &TukuiAddon,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<TukuiInstallSummary, String> {
    let temp_root = create_temp_work_dir("tukui-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let layout = inspect_tukui_package_layout(&extract_root, addon)?;
        let scanned = scan_addons_dir(&layout.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == layout.parent_folder)
            .ok_or_else(|| {
                format!(
                    "TukUI package for '{}' did not yield a scannable parent folder '{}'",
                    addon.name, layout.parent_folder
                )
            })?;

        if dry_run {
            return Ok(TukuiInstallSummary {
                addon_slug: addon.slug.clone(),
                addon_name: addon.name.clone(),
                parent_folder: layout.parent_folder,
                installed_folders: layout.folders,
                version: Some(addon.version.clone()),
                dry_run: true,
            });
        }

        preflight_install_targets(addon_dir, &layout.folders)?;
        for folder in &layout.folders {
            copy_dir_recursively(&layout.source_root.join(folder), &addon_dir.join(folder))?;
        }

        let record = build_managed_tukui_record(addon, parent_metadata, &layout.folders);
        state_database
            .record_managed_addon(&record)
            .map_err(|error| error.to_string())?;

        Ok(TukuiInstallSummary {
            addon_slug: addon.slug.clone(),
            addon_name: addon.name.clone(),
            parent_folder: layout.parent_folder,
            installed_folders: layout.folders,
            version: Some(addon.version.clone()),
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn update_downloaded_tukui_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    addon: &TukuiAddon,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<TukuiUpdateSummary, String> {
    let temp_root = create_temp_work_dir("tukui-update")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let backup_root = temp_root.join("backup");
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let previous_version = tracked.version.clone();

    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let layout = inspect_tukui_package_layout(&extract_root, addon)?;
        if layout.parent_folder != tracked.folder {
            return Err(format!(
                "TukUI update for '{}' resolved parent '{}' but tracked addon folder is '{}'",
                addon.name, layout.parent_folder, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(&layout.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == layout.parent_folder)
            .ok_or_else(|| {
                format!(
                    "updated TukUI package for '{}' did not yield a scannable parent folder '{}'",
                    addon.name, layout.parent_folder
                )
            })?;

        let managed_folders = std::iter::once(tracked.folder.clone())
            .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
            .collect::<Vec<_>>();

        preflight_update_targets(addon_dir, &layout.folders, &managed_folders)?;

        if dry_run {
            return Ok(TukuiUpdateSummary {
                addon_slug: addon.slug.clone(),
                addon_name: addon.name.clone(),
                parent_folder: layout.parent_folder,
                installed_folders: layout.folders,
                previous_version,
                remote_version: Some(addon.version.clone()),
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
        for folder in &layout.folders {
            copy_dir_recursively(&layout.source_root.join(folder), &addon_dir.join(folder))?;
            copied_new.push(folder.clone());
        }

        let record = build_managed_tukui_record(addon, parent_metadata, &layout.folders);
        if let Err(error) = state_database.record_managed_addon(&record) {
            rollback_updated_folders(addon_dir, &backup_root, &copied_new, &moved_existing);
            return Err(error.to_string());
        }

        Ok(TukuiUpdateSummary {
            addon_slug: addon.slug.clone(),
            addon_name: addon.name.clone(),
            parent_folder: layout.parent_folder,
            installed_folders: layout.folders,
            previous_version,
            remote_version: Some(addon.version.clone()),
            updated: true,
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

#[derive(Debug, Clone)]
struct TukuiPackageLayout {
    source_root: PathBuf,
    parent_folder: String,
    folders: Vec<String>,
}

fn inspect_tukui_package_layout(
    root: &Path,
    addon: &TukuiAddon,
) -> Result<TukuiPackageLayout, String> {
    let expected_folders = canonical_install_folders(&addon.slug);
    let source_root = resolve_expected_source_root(root, &expected_folders)?;
    let parent_folder = canonical_parent_folder(&addon.slug).to_string();
    for folder in &expected_folders {
        if !source_root.join(folder).exists() {
            return Err(format!(
                "TukUI package for '{}' is missing expected folder '{}'",
                addon.name, folder
            ));
        }
    }
    Ok(TukuiPackageLayout {
        source_root,
        parent_folder,
        folders: expected_folders,
    })
}

fn resolve_expected_source_root(
    root: &Path,
    expected_folders: &[String],
) -> Result<PathBuf, String> {
    if expected_folders
        .iter()
        .all(|folder| root.join(folder).exists())
    {
        return Ok(root.to_path_buf());
    }

    let first_level_dirs = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();

    if first_level_dirs.len() == 1 {
        let nested_root = first_level_dirs[0].clone();
        if expected_folders
            .iter()
            .all(|folder| nested_root.join(folder).exists())
        {
            return Ok(nested_root);
        }
    }

    Err(
        "no top-level TukUI addon folders matching the canonical package layout were found"
            .to_string(),
    )
}

fn build_managed_tukui_record(
    addon: &TukuiAddon,
    parent_metadata: &ScannedAddon,
    installed_folders: &[String],
) -> AddonRecord {
    let mut record = AddonRecord::new(
        parent_metadata.name.clone(),
        parent_metadata.folder.clone(),
        SourceKind::Tukui,
    );
    record.kind = parent_metadata.kind;
    record.flavor = parent_metadata.flavor;
    record.version = Some(addon.version.clone()).or_else(|| parent_metadata.version.clone());
    record.author = Some(addon.author.clone()).or_else(|| parent_metadata.author.clone());
    record.interface = parent_metadata.interface.clone();
    record.source_url = Some(addon.web_url.clone());
    record.required_deps = parent_metadata.required_deps.clone();
    record.optional_deps = parent_metadata.optional_deps.clone();
    record.embedded_libs = parent_metadata.embedded_libs.clone();
    record.remote_version = Some(addon.version.clone());
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

fn resolve_tracked_tukui_target(addon: &AddonRecord) -> Result<String, String> {
    addon
        .source_url
        .as_deref()
        .and_then(|value| parse_tukui_target(value).ok())
        .or_else(|| parse_tukui_target(&addon.folder).ok())
        .or_else(|| parse_tukui_target(&addon.name).ok())
        .ok_or_else(|| {
            format!(
                "tracked TukUI addon '{}' does not resolve to the canonical ElvUI or Tukui package",
                addon.folder
            )
        })
}

fn canonical_tukui_slug(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "elvui" => Some("elvui"),
        "tukui" => Some("tukui"),
        _ => None,
    }
}

fn canonical_parent_folder(slug: &str) -> &'static str {
    match slug {
        "elvui" => "ElvUI",
        "tukui" => "Tukui",
        _ => unreachable!("canonical targets only"),
    }
}

fn canonical_install_folders(slug: &str) -> Vec<String> {
    match slug {
        "elvui" => vec![
            "ElvUI".to_string(),
            "ElvUI_Libraries".to_string(),
            "ElvUI_Options".to_string(),
        ],
        "tukui" => vec!["Tukui".to_string()],
        _ => unreachable!("canonical targets only"),
    }
}

fn validate_tukui_download_url(value: &str) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|error| error.to_string())?;
    let Some(host) = parsed.host_str() else {
        return Err("TukUI download URL is missing a host".to_string());
    };
    if !host.ends_with("tukui.org") {
        return Err("TukUI download URL must stay on a tukui.org host".to_string());
    }
    Ok(parsed)
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

    Err("failed to create a unique temporary TukUI work directory".to_string())
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
        build_managed_tukui_record, canonical_install_folders, inspect_tukui_package_layout,
        parse_tukui_target, update_downloaded_tukui_addon, versions_match,
    };
    use lemonup_core::{AddonRecord, OwnedFolder, SourceKind, StateDatabase};
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
    fn parse_tukui_target_accepts_canonical_names_and_urls() {
        assert_eq!(parse_tukui_target("ElvUI").expect("elvui"), "elvui");
        assert_eq!(
            parse_tukui_target("https://tukui.org/tukui").expect("url"),
            "tukui"
        );
        assert_eq!(
            parse_tukui_target("https://api.tukui.org/v1/download/elvui/token")
                .expect("download url"),
            "elvui"
        );
        assert!(parse_tukui_target("details").is_err());
    }

    #[test]
    fn inspect_tukui_layout_accepts_single_wrapper_directory() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("extract");
        std::fs::create_dir_all(root.join("wrapper").join("ElvUI")).expect("create parent");
        std::fs::create_dir_all(root.join("wrapper").join("ElvUI_Libraries")).expect("create lib");
        std::fs::create_dir_all(root.join("wrapper").join("ElvUI_Options")).expect("create opts");

        let addon = super::TukuiAddon {
            slug: "elvui".to_string(),
            author: "Elv".to_string(),
            name: "ElvUI".to_string(),
            url: "https://api.tukui.org/v1/download/elvui/token".to_string(),
            version: "1.0.0".to_string(),
            web_url: "https://tukui.org/elvui".to_string(),
            directories: canonical_install_folders("elvui"),
        };

        let layout = inspect_tukui_package_layout(&root, &addon).expect("layout");
        assert_eq!(layout.parent_folder, "ElvUI");
        assert_eq!(
            layout.folders,
            vec![
                "ElvUI".to_string(),
                "ElvUI_Libraries".to_string(),
                "ElvUI_Options".to_string()
            ]
        );
    }

    #[test]
    fn build_managed_tukui_record_sets_expected_owned_folders() {
        let scanned = lemonup_core::ScannedAddon {
            name: "ElvUI".to_string(),
            folder: "ElvUI".to_string(),
            owned_folders: Vec::new(),
            kind: lemonup_core::AddonKind::Addon,
            flavor: lemonup_core::GameFlavor::Retail,
            version: Some("15.08".to_string()),
            git_commit: None,
            author: Some("Elv".to_string()),
            interface: Some("110205".to_string()),
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
        };
        let addon = super::TukuiAddon {
            slug: "elvui".to_string(),
            author: "Elv".to_string(),
            name: "ElvUI".to_string(),
            url: "https://api.tukui.org/v1/download/elvui/token".to_string(),
            version: "15.09".to_string(),
            web_url: "https://tukui.org/elvui".to_string(),
            directories: canonical_install_folders("elvui"),
        };

        let record =
            build_managed_tukui_record(&addon, &scanned, &canonical_install_folders("elvui"));
        assert_eq!(record.source, SourceKind::Tukui);
        assert_eq!(record.remote_version.as_deref(), Some("15.09"));
        assert_eq!(
            record
                .owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["ElvUI_Libraries", "ElvUI_Options"]
        );
    }

    #[test]
    fn update_downloaded_tukui_addon_supports_single_folder_managed_package() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("Tukui")).expect("create addon dir");
        std::fs::write(
            addon_dir.join("Tukui").join("Tukui.toc"),
            "## Title: Tukui\n## Version: 20.462\n## Author: Tukz\n## Interface: 110205\n",
        )
        .expect("write toc");

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("Tukui", "Tukui", SourceKind::Tukui);
        tracked.version = Some("20.462".to_string());
        tracked.source_url = Some("https://tukui.org/tukui".to_string());
        tracked.set_managed_owned_folders(Vec::<OwnedFolder>::new());
        database.upsert_addon(&tracked).expect("seed tracked addon");

        let addon = super::TukuiAddon {
            slug: "tukui".to_string(),
            author: "Tukz".to_string(),
            name: "Tukui".to_string(),
            url: "https://api.tukui.org/v1/download/tukui/token".to_string(),
            version: "20.463".to_string(),
            web_url: "https://tukui.org/tukui".to_string(),
            directories: canonical_install_folders("tukui"),
        };

        let bytes = build_zip(&[(
            "wrapper/Tukui/Tukui.toc",
            "## Title: Tukui\n## Version: 20.463\n## Author: Tukz\n## Interface: 110205\n",
        )]);

        let summary = update_downloaded_tukui_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &addon,
            &bytes,
            false,
        )
        .expect("update addon");

        assert!(summary.updated);
        assert_eq!(summary.remote_version.as_deref(), Some("20.463"));
        let stored = database
            .get_addon_by_folder("Tukui")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("20.463"));
        assert_eq!(stored.source, SourceKind::Tukui);
    }

    #[test]
    fn versions_match_ignores_leading_v_prefix() {
        assert!(versions_match(Some("v15.10"), Some("15.10")));
        assert!(versions_match(Some("15.10"), Some("V15.10")));
        assert!(!versions_match(Some("v15.09"), Some("15.10")));
    }
}
