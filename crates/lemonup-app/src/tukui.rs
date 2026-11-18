use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use lemonup_core::{
    AddonRecord, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, scan_addons_dir,
};
use reqwest::{Client, Url};
use serde::Deserialize;
use time::OffsetDateTime;
use zip::ZipArchive;

use crate::archive::safe_entry_relative_path;
use crate::source_error::{SourceError, SourceErrorKind};

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
    /// Not updated because the remote version is older than the installed one.
    pub(crate) remote_older: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TukuiRemoteVersion {
    pub(crate) source_url: Option<String>,
    pub(crate) version: Option<String>,
    /// The addon's name on TukUI, as TukUI gives it.
    pub(crate) name: Option<String>,
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
    /// Game patches the package lists as supported (for example `12.0.5`, `1.60.1`).
    #[serde(default, deserialize_with = "deserialize_patch_list")]
    patch: Vec<String>,
}

/// Lenient view of the feed's `patch` field. Anything that is not a string or a list of
/// strings is ignored so a malformed value can never break parsing of the whole feed.
#[derive(Deserialize)]
#[serde(untagged)]
enum TukuiPatchField {
    One(String),
    Many(Vec<TukuiPatchEntry>),
    Other(serde::de::IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TukuiPatchEntry {
    Text(String),
    Other(serde::de::IgnoredAny),
}

fn deserialize_patch_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(
        match Option::<TukuiPatchField>::deserialize(deserializer)? {
            Some(TukuiPatchField::One(value)) => vec![value],
            Some(TukuiPatchField::Many(entries)) => entries
                .into_iter()
                .filter_map(|entry| match entry {
                    TukuiPatchEntry::Text(value) => Some(value),
                    TukuiPatchEntry::Other(_) => None,
                })
                .collect(),
            Some(TukuiPatchField::Other(_)) | None => Vec::new(),
        },
    )
}

/// Parses the `major.minor` prefix of a patch string such as `1.60.1`; the patch component and
/// anything unparsable is ignored.
fn parse_patch_major_minor(patch: &str) -> Option<(u32, u32)> {
    let mut parts = patch.trim().split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next()?.trim().parse().ok()?;
    Some((major, minor))
}

/// WoW Forever runs client patch 1.60 or newer within major 1; refuse TukUI packages that do
/// not list such a patch.
fn ensure_tukui_supports_flavor(
    addon: &TukuiAddon,
    flavor: GameFlavor,
    operation: &str,
) -> Result<(), String> {
    if flavor != GameFlavor::Forever {
        return Ok(());
    }
    if addon
        .patch
        .iter()
        .filter_map(|patch| parse_patch_major_minor(patch))
        .any(|(major, minor)| major == 1 && minor >= 60)
    {
        return Ok(());
    }
    Err(format!(
        "TukUI addon '{}' does not list WoW Forever support (no 1.60 or newer 1.x patch); refusing to {operation} it into a WoW Forever AddOns folder",
        addon.name
    ))
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
    install_tukui_addon_checked(state_database, addon_dir, target, dry_run, &[])
        .await
        .map_err(String::from)
}

/// `install_tukui_addon` with the error kind kept. `replace` is the list of
/// folders the person agreed to replace (see `plan_install_replacements`).
pub(crate) async fn install_tukui_addon_checked(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    dry_run: bool,
    replace: &[String],
) -> Result<TukuiInstallSummary, SourceError> {
    let slug = parse_tukui_target(target)?;
    let client = build_tukui_client()?;
    let addon = resolve_tukui_addon(&client, &slug, false).await?;
    ensure_tukui_supports_flavor(&addon, GameFlavor::for_addons_dir(addon_dir), "install")?;
    let bytes = download_tukui_release_bytes(&client, &addon).await?;
    install_downloaded_tukui_addon(state_database, addon_dir, &addon, &bytes, dry_run, replace)
}

pub(crate) async fn fetch_tukui_remote_version(
    addon: &AddonRecord,
) -> Result<TukuiRemoteVersion, SourceError> {
    let slug = resolve_tracked_tukui_target(addon)?;
    let client = build_tukui_client()?;
    let entry = resolve_tukui_addon(&client, &slug, false).await?;
    Ok(TukuiRemoteVersion {
        source_url: Some(entry.web_url.clone()),
        version: Some(entry.version.clone()),
        name: Some(entry.name.clone()),
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
        return Err(format!("addon '{}' is not linked to TukUI", tracked.folder));
    }

    let slug = resolve_tracked_tukui_target(tracked)?;
    let client = build_tukui_client()?;
    let addon = resolve_tukui_addon(&client, &slug, false).await?;
    let plan = plan_tukui_update(
        tracked,
        &addon,
        GameFlavor::for_addons_dir(addon_dir),
        force,
        dry_run,
    )?;
    if let TukuiUpdatePlan::UpToDate(summary) = plan {
        crate::update::record_skipped_update_check(
            state_database,
            tracked,
            summary.remote_version.as_deref(),
            Some(addon.web_url.clone()),
            Some(&addon.name),
            dry_run,
        )?;
        return Ok(summary);
    }

    let bytes = download_tukui_release_bytes(&client, &addon).await?;
    update_downloaded_tukui_addon(state_database, addon_dir, tracked, &addon, &bytes, dry_run)
}

enum TukuiUpdatePlan {
    UpToDate(TukuiUpdateSummary),
    Download,
}

/// Decides whether an update needs a download. The up-to-date comparison runs first so an
/// already current addon reports up to date; the flavor support check then gates any download.
fn plan_tukui_update(
    tracked: &AddonRecord,
    addon: &TukuiAddon,
    flavor: GameFlavor,
    force: bool,
    dry_run: bool,
) -> Result<TukuiUpdatePlan, String> {
    let remote_version = Some(addon.version.clone());

    let remote_older = !force
        && crate::update::remote_is_older(tracked.version.as_deref(), remote_version.as_deref());
    if remote_older
        || (!force && versions_match(tracked.version.as_deref(), remote_version.as_deref()))
    {
        return Ok(TukuiUpdatePlan::UpToDate(TukuiUpdateSummary {
            addon_slug: addon.slug.clone(),
            addon_name: addon.name.clone(),
            parent_folder: canonical_parent_folder(&addon.slug).to_string(),
            installed_folders: canonical_install_folders(&addon.slug),
            previous_version: tracked.version.clone(),
            remote_version,
            updated: false,
            remote_older,
            dry_run,
        }));
    }

    ensure_tukui_supports_flavor(addon, flavor, "update")?;
    Ok(TukuiUpdatePlan::Download)
}

/// The canonical TukUI packages (ElvUI, Tukui) that support `flavor`, fresh from the feed, for
/// "find your addons online".
pub(crate) async fn fetch_tukui_packages(
    flavor: GameFlavor,
) -> Result<Vec<lemonup_core::find::TukuiPackage>, SourceError> {
    let client = build_tukui_client()?;
    let addons = fetch_tukui_addons(&client, true).await?;
    Ok(tukui_packages_for(addons, flavor))
}

fn tukui_packages_for(
    addons: Vec<TukuiAddon>,
    flavor: GameFlavor,
) -> Vec<lemonup_core::find::TukuiPackage> {
    addons
        .into_iter()
        .filter(|addon| canonical_tukui_slug(&addon.slug).is_some())
        .filter(|addon| ensure_tukui_supports_flavor(addon, flavor, "link").is_ok())
        .map(|addon| lemonup_core::find::TukuiPackage {
            id: addon.slug,
            name: addon.name,
            version: addon.version,
            web_url: addon.web_url,
            author: Some(addon.author).filter(|author| !author.trim().is_empty()),
            directories: addon.directories,
        })
        .collect()
}

fn build_tukui_client() -> Result<Client, String> {
    crate::http::client_builder()
        .build()
        .map_err(|error| error.to_string())
}

async fn resolve_tukui_addon(
    client: &Client,
    slug: &str,
    force_refresh: bool,
) -> Result<TukuiAddon, SourceError> {
    let addons = fetch_tukui_addons(client, force_refresh).await?;
    addons
        .into_iter()
        .find(|addon| {
            addon.slug.eq_ignore_ascii_case(slug) || addon.name.eq_ignore_ascii_case(slug)
        })
        .ok_or_else(|| {
            SourceError::new(
                SourceErrorKind::NotFound,
                format!("could not find canonical TukUI package '{slug}' in the TukUI API feed"),
            )
        })
}

async fn fetch_tukui_addons(
    client: &Client,
    force_refresh: bool,
) -> Result<Vec<TukuiAddon>, SourceError> {
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
        .map_err(SourceError::from_keyless_reqwest)?
        .error_for_status()
        .map_err(SourceError::from_keyless_reqwest)?;

    let addons = response
        .json::<Vec<TukuiAddon>>()
        .await
        .map_err(SourceError::from_keyless_reqwest)?;

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
) -> Result<Vec<u8>, SourceError> {
    let download_url = validate_tukui_download_url(&addon.url)?;
    client
        .get(download_url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(SourceError::from_keyless_reqwest)?
        .error_for_status()
        .map_err(SourceError::from_keyless_reqwest)?
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(SourceError::from_keyless_reqwest)
}

fn install_downloaded_tukui_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    addon: &TukuiAddon,
    archive_bytes: &[u8],
    dry_run: bool,
    replace: &[String],
) -> Result<TukuiInstallSummary, SourceError> {
    let temp_root = create_temp_work_dir("tukui-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| -> Result<TukuiInstallSummary, SourceError> {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let layout = inspect_tukui_package_layout(&extract_root, addon)?;
        let scanned = scan_addons_dir(&layout.source_root, GameFlavor::for_addons_dir(addon_dir))
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

        // Only folders on `replace` that are not linked are taken over.
        let replaced = crate::update_targets::plan_install_replacements(
            state_database,
            addon_dir,
            &layout.folders,
            &layout.parent_folder,
            replace,
            None,
        )?;
        let record = build_managed_tukui_record(addon, parent_metadata, &layout.folders);
        // The user's TOC overrides go into the staged copies before anything live moves.
        let toc_overrides = crate::update_targets::stored_toc_overrides(state_database, &record)?;
        let swap = crate::update_targets::swap_with_toc_overrides(
            addon_dir,
            &layout.source_root,
            &layout.folders,
            &replaced,
            &toc_overrides,
        )?;

        if let Err(error) = state_database.record_managed_addon(&record) {
            return Err(swap.abort(error.to_string()).into());
        }
        swap.commit();

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
    let previous_version = tracked.version.clone();

    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let layout = inspect_tukui_package_layout(&extract_root, addon)?;
        if layout.parent_folder != tracked.folder {
            return Err(format!(
                "TukUI update for '{}' resolved parent '{}' but linked addon folder is '{}'",
                addon.name, layout.parent_folder, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(&layout.source_root, GameFlavor::for_addons_dir(addon_dir))
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

        // Parent plus owned folders (authoritative ownership only) and any package folders
        // taken over from manual rows; refuses folders of other tracked addons.
        let managed_folders = crate::update_targets::plan_update_replacements(
            state_database,
            addon_dir,
            tracked,
            &layout.folders,
        )?;

        if dry_run {
            return Ok(TukuiUpdateSummary {
                addon_slug: addon.slug.clone(),
                addon_name: addon.name.clone(),
                parent_folder: layout.parent_folder,
                installed_folders: layout.folders,
                previous_version,
                remote_version: Some(addon.version.clone()),
                updated: true,
                remote_older: false,
                dry_run: true,
            });
        }

        let record = build_managed_tukui_record(addon, parent_metadata, &layout.folders);
        // Stage, move aside and place by same-volume rename; any failure restores the old
        // folders before the error is returned.
        // The user's TOC overrides go into the staged copies before anything live moves.
        let toc_overrides = crate::update_targets::stored_toc_overrides(state_database, &record)?;
        let swap = crate::update_targets::swap_with_toc_overrides(
            addon_dir,
            &layout.source_root,
            &layout.folders,
            &managed_folders,
            &toc_overrides,
        )?;

        if let Err(error) = state_database.record_managed_addon(&record) {
            return Err(swap.abort(error.to_string()));
        }
        swap.commit();

        Ok(TukuiUpdateSummary {
            addon_slug: addon.slug.clone(),
            addon_name: addon.name.clone(),
            parent_folder: layout.parent_folder,
            installed_folders: layout.folders,
            previous_version,
            remote_version: Some(addon.version.clone()),
            updated: true,
            remote_older: false,
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
    record.source_name = lemonup_core::domain::source_name(&addon.name);
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
                "linked TukUI addon '{}' does not resolve to the canonical ElvUI or Tukui package",
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

fn extract_zip_archive(zip_path: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let file = File::open(zip_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative_path) = safe_entry_relative_path(entry.name()) else {
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

/// A fresh, uniquely named work directory under the system temp dir; the caller removes it.
///
/// `tempfile` creates it atomically with a random suffix, so parallel operations (and tests)
/// can never share one. A millisecond-timestamp name with an `exists()` check could hand the
/// same directory to two callers in the same millisecond, and one would delete the other's
/// extracted package.
fn create_temp_work_dir(prefix: &str) -> Result<PathBuf, String> {
    tempfile::Builder::new()
        .prefix(&format!("lemonup-{prefix}-"))
        .tempdir()
        .map(tempfile::TempDir::keep)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        TukuiAddon, TukuiUpdatePlan, build_managed_tukui_record, canonical_install_folders,
        ensure_tukui_supports_flavor, inspect_tukui_package_layout, parse_tukui_target,
        plan_tukui_update, update_downloaded_tukui_addon, versions_match,
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
    fn forever_requires_a_1_60_patch_entry() {
        let mut addon: TukuiAddon = serde_json::from_str(
            r#"{
                "slug": "elvui",
                "author": "Elv",
                "name": "ElvUI",
                "url": "https://api.tukui.org/v1/download/elvui/token",
                "version": "15.08",
                "web_url": "https://tukui.org/elvui",
                "directories": ["ElvUI", "ElvUI_Libraries", "ElvUI_Options"],
                "patch": ["12.0.5", "1.60.1", "5.5.4", "1.15.9"]
            }"#,
        )
        .expect("parse feed entry");
        assert!(
            ensure_tukui_supports_flavor(&addon, lemonup_core::GameFlavor::Forever, "install")
                .is_ok()
        );

        addon.patch = vec!["12.0.5".to_string(), "5.5.4".to_string()];
        let error =
            ensure_tukui_supports_flavor(&addon, lemonup_core::GameFlavor::Forever, "install")
                .expect_err("missing 1.60 patch");
        assert!(error.contains("ElvUI"));
        assert!(error.contains("does not list WoW Forever support"));
        assert!(
            ensure_tukui_supports_flavor(&addon, lemonup_core::GameFlavor::Retail, "install")
                .is_ok()
        );
    }

    #[test]
    fn find_offers_only_canonical_packages_for_the_game_version() {
        let mut plugin = forever_addon_with_patch(&["1.60.1"]);
        plugin.slug = "elvui-windtools".to_string();
        let addons = vec![
            forever_addon_with_patch(&["1.60.1"]),
            forever_addon_with_patch(&["12.0.5"]),
            plugin,
        ];

        let forever = super::tukui_packages_for(addons.clone(), lemonup_core::GameFlavor::Forever);
        let retail = super::tukui_packages_for(addons, lemonup_core::GameFlavor::Retail);

        assert_eq!(forever.len(), 1);
        assert_eq!(forever[0].id, "elvui");
        assert_eq!(forever[0].directories, ["ElvUI"]);
        assert_eq!(retail.len(), 2);
    }

    fn forever_addon_with_patch(patch: &[&str]) -> TukuiAddon {
        let mut addon: TukuiAddon = serde_json::from_str(
            r#"{"slug": "elvui", "author": "Elv", "name": "ElvUI", "url": "u",
                "version": "15.08", "web_url": "w", "directories": ["ElvUI"]}"#,
        )
        .expect("parse feed entry");
        addon.patch = patch.iter().map(|value| value.to_string()).collect();
        addon
    }

    #[test]
    fn forever_accepts_1_60_or_newer_1x_patches() {
        let forever = lemonup_core::GameFlavor::Forever;
        for patch in [
            &["1.61.0"][..],
            &["1.60"][..],
            &["12.0.5", "1.75.2"][..],
            &["not-a-patch", " 1.60.1 "][..],
        ] {
            assert!(
                ensure_tukui_supports_flavor(&forever_addon_with_patch(patch), forever, "install")
                    .is_ok(),
                "{patch:?} should be accepted"
            );
        }

        for patch in [
            &["1.15.9"][..],
            &["2.60.0", "12.0.5"][..],
            &["1", "garbage", ""][..],
            &[][..],
        ] {
            assert!(
                ensure_tukui_supports_flavor(&forever_addon_with_patch(patch), forever, "install")
                    .is_err(),
                "{patch:?} should be rejected"
            );
        }
    }

    #[test]
    fn tukui_patch_ignores_non_string_entries_and_values() {
        let feed: Vec<TukuiAddon> = serde_json::from_str(
            r#"[
                {"slug": "tukui", "author": "Tukz", "name": "Tukui", "url": "u", "version": "1",
                 "web_url": "w", "directories": ["Tukui"],
                 "patch": ["12.0.5", 7, null, {"a": 1}, ["1.60.1"], "1.60.2", 1.5]},
                {"slug": "elvui", "author": "Elv", "name": "ElvUI", "url": "u", "version": "2",
                 "web_url": "w", "directories": ["ElvUI"], "patch": null},
                {"slug": "weird", "author": "W", "name": "Weird", "url": "u", "version": "3",
                 "web_url": "w", "directories": ["Weird"], "patch": {"unexpected": true}},
                {"slug": "number", "author": "N", "name": "Number", "url": "u", "version": "4",
                 "web_url": "w", "directories": ["Number"], "patch": 12}
            ]"#,
        )
        .expect("a weird patch field must not break the feed");

        assert_eq!(feed.len(), 4);
        assert_eq!(
            feed[0].patch,
            vec!["12.0.5".to_string(), "1.60.2".to_string()]
        );
        assert!(feed[1].patch.is_empty());
        assert!(feed[2].patch.is_empty());
        assert!(feed[3].patch.is_empty());

        let retail = lemonup_core::GameFlavor::Retail;
        for addon in &feed {
            assert!(ensure_tukui_supports_flavor(addon, retail, "install").is_ok());
        }
    }

    #[test]
    fn tukui_update_reports_up_to_date_before_forever_support_check() {
        let forever = lemonup_core::GameFlavor::Forever;
        let unsupported = forever_addon_with_patch(&["12.0.5"]);
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        tracked.version = Some("v15.08".to_string());

        match plan_tukui_update(&tracked, &unsupported, forever, false, false)
            .expect("up to date must not error")
        {
            TukuiUpdatePlan::UpToDate(summary) => {
                assert!(!summary.updated);
                assert_eq!(summary.remote_version.as_deref(), Some("15.08"));
            }
            TukuiUpdatePlan::Download => panic!("expected up to date"),
        }

        // An outdated record still refuses before any download.
        tracked.version = Some("15.01".to_string());
        let error = plan_tukui_update(&tracked, &unsupported, forever, false, false)
            .err()
            .expect("unsupported addon must refuse");
        assert!(error.contains("does not list WoW Forever support"));

        // Force skips the up-to-date shortcut and must still be refused.
        tracked.version = Some("15.08".to_string());
        assert!(plan_tukui_update(&tracked, &unsupported, forever, true, false).is_err());

        let supported = forever_addon_with_patch(&["1.61.0"]);
        tracked.version = Some("15.01".to_string());
        assert!(matches!(
            plan_tukui_update(&tracked, &supported, forever, false, false),
            Ok(TukuiUpdatePlan::Download)
        ));
    }

    #[test]
    fn tukui_patch_accepts_string_or_missing_field() {
        let single: TukuiAddon = serde_json::from_str(
            r#"{"slug": "tukui", "author": "Tukz", "name": "Tukui", "url": "u", "version": "1",
                "web_url": "w", "directories": ["Tukui"], "patch": "1.60.1"}"#,
        )
        .expect("parse single patch");
        assert_eq!(single.patch, vec!["1.60.1".to_string()]);

        let missing: TukuiAddon = serde_json::from_str(
            r#"{"slug": "tukui", "author": "Tukz", "name": "Tukui", "url": "u", "version": "1",
                "web_url": "w", "directories": ["Tukui"]}"#,
        )
        .expect("parse missing patch");
        assert!(missing.patch.is_empty());
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
            patch: Vec::new(),
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
            notes: None,
            category: None,
            toc_title: None,
        };
        let addon = super::TukuiAddon {
            slug: "elvui".to_string(),
            author: "Elv".to_string(),
            name: "|cff1784d1ElvUI|r &amp; Friends".to_string(),
            url: "https://api.tukui.org/v1/download/elvui/token".to_string(),
            version: "15.09".to_string(),
            web_url: "https://tukui.org/elvui".to_string(),
            directories: canonical_install_folders("elvui"),
            patch: Vec::new(),
        };

        let record =
            build_managed_tukui_record(&addon, &scanned, &canonical_install_folders("elvui"));
        assert_eq!(record.source, SourceKind::Tukui);
        assert_eq!(record.remote_version.as_deref(), Some("15.09"));
        assert_eq!(record.name, "ElvUI");
        assert_eq!(record.source_name.as_deref(), Some("ElvUI & Friends"));
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
            patch: Vec::new(),
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
    fn update_downloaded_tukui_addon_writes_toc_overrides_and_skips_unusable_ones() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("Tukui")).expect("create addon dir");
        std::fs::write(
            addon_dir.join("Tukui").join("Tukui.toc"),
            "## Title: Tukui\n## Version: 20.462\n## Interface: 110205\n",
        )
        .expect("write toc");

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("Tukui", "Tukui", SourceKind::Tukui);
        tracked.version = Some("20.462".to_string());
        tracked.source_url = Some("https://tukui.org/tukui".to_string());
        tracked.set_managed_owned_folders(Vec::<OwnedFolder>::new());
        // An empty Category is a value this version refuses; it must not block the update.
        tracked.toc_overrides = [
            ("Title", "|cff00ff00My|r Tukui"),
            ("Notes", "Mine"),
            ("Category", ""),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
        database.upsert_addon(&tracked).expect("seed tracked addon");

        let addon = super::TukuiAddon {
            slug: "tukui".to_string(),
            author: "Tukz".to_string(),
            name: "Tukui".to_string(),
            url: "https://api.tukui.org/v1/download/tukui/token".to_string(),
            version: "20.463".to_string(),
            web_url: "https://tukui.org/tukui".to_string(),
            directories: canonical_install_folders("tukui"),
            patch: Vec::new(),
        };
        let bytes = build_zip(&[(
            "wrapper/Tukui/Tukui.toc",
            "## Title: Tukui\n## Version: 20.463\n## Category: UI\n## Interface: 110205\n",
        )]);

        update_downloaded_tukui_addon(&mut database, &addon_dir, &tracked, &addon, &bytes, false)
            .expect("update addon");

        assert_eq!(
            std::fs::read_to_string(addon_dir.join("Tukui").join("Tukui.toc")).expect("toc"),
            "# LemonUp: ## Title: Tukui\n## Version: 20.463\n## Category: UI\n## Interface: 110205\n# LemonUp-managed\n## Title: |cff00ff00My|r Tukui\n# LemonUp-managed\n## Notes: Mine\n"
        );
        let stored = database
            .get_addon_by_folder("Tukui")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("20.463"));
        assert_eq!(stored.name, "My Tukui");
        assert_eq!(stored.toc_overrides, tracked.toc_overrides);
    }

    #[test]
    fn update_downloaded_tukui_addon_adopts_unowned_parent_and_manual_package_folders() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        for folder in ["ElvUI", "ElvUI_Libraries", "ElvUI_Options"] {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(addon_dir.join(folder).join("stale.lua"), "old").expect("stale");
        }
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        tracked.version = Some("13.80".to_string());
        tracked.source_url = Some("https://tukui.org/elvui".to_string());
        database.upsert_addon(&tracked).expect("seed tracked addon");
        for folder in ["ElvUI_Libraries", "ElvUI_Options"] {
            database
                .upsert_addon(&AddonRecord::new(folder, folder, SourceKind::Manual))
                .expect("seed manual row");
        }

        let addon = super::TukuiAddon {
            slug: "elvui".to_string(),
            author: "Elv".to_string(),
            name: "ElvUI".to_string(),
            url: "https://api.tukui.org/v1/download/elvui/token".to_string(),
            version: "13.81".to_string(),
            web_url: "https://tukui.org/elvui".to_string(),
            directories: canonical_install_folders("elvui"),
            patch: Vec::new(),
        };
        let toc = "## Title: ElvUI\n## Version: 13.81\n## Interface: 110205\n";
        let bytes = build_zip(&[
            ("ElvUI/ElvUI.toc", toc),
            ("ElvUI_Libraries/ElvUI_Libraries.toc", toc),
            ("ElvUI_Options/ElvUI_Options.toc", toc),
        ]);

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
        for folder in ["ElvUI", "ElvUI_Libraries", "ElvUI_Options"] {
            assert!(!addon_dir.join(folder).join("stale.lua").exists());
        }
        let rows = database.list_addons().expect("list");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].has_authoritative_owned_folders());
        assert_eq!(rows[0].owned_folders.len(), 2);
    }

    #[test]
    fn tukui_update_holds_back_an_older_remote_unless_forced() {
        let retail = lemonup_core::GameFlavor::Retail;
        let mut addon = forever_addon_with_patch(&["11.0.5"]);
        addon.version = "13.10".to_string();
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        tracked.version = Some("v13.80".to_string());

        match plan_tukui_update(&tracked, &addon, retail, false, false).expect("plan") {
            TukuiUpdatePlan::UpToDate(summary) => {
                assert!(!summary.updated);
                assert!(summary.remote_older);
            }
            TukuiUpdatePlan::Download => panic!("an older remote must not be downloaded"),
        }
        assert!(matches!(
            plan_tukui_update(&tracked, &addon, retail, true, false),
            Ok(TukuiUpdatePlan::Download)
        ));
    }

    #[test]
    fn versions_match_ignores_leading_v_prefix() {
        assert!(versions_match(Some("v15.10"), Some("15.10")));
        assert!(versions_match(Some("15.10"), Some("V15.10")));
        assert!(!versions_match(Some("v15.09"), Some("15.10")));
    }
}
