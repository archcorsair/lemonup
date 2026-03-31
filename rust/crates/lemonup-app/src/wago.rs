use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use lemonup_core::{
    AddonRecord, ConfigLoad, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase,
    scan_addons_dir,
};
use reqwest::{Client, Url};
use serde::Deserialize;
use time::OffsetDateTime;
use zip::ZipArchive;

const API_BASE: &str = "https://addons.wago.io";
const EXTERNAL_PATH: &str = "/api/external";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WagoStability {
    Stable,
    Beta,
    Alpha,
}

impl WagoStability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WagoInstallSummary {
    pub addon_id: String,
    pub addon_name: String,
    pub parent_folder: String,
    pub installed_folders: Vec<String>,
    pub stability: WagoStability,
    pub version: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WagoSearchResult {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) summary: Option<String>,
    pub(crate) owner: Option<String>,
    pub(crate) authors: Vec<String>,
    pub(crate) website_url: Option<String>,
    pub(crate) download_count: Option<u64>,
    pub(crate) version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WagoInstallInspection {
    pub(crate) addon_id: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) stability: WagoStability,
    pub(crate) version: Option<String>,
    pub(crate) existing_folders: Vec<String>,
    pub(crate) tracked_parent: Option<String>,
}

impl WagoInstallInspection {
    pub(crate) fn requires_confirmation(&self) -> bool {
        self.tracked_parent.is_some() || !self.existing_folders.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WagoUpdateSummary {
    pub(crate) addon_id: String,
    pub(crate) addon_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) stability: WagoStability,
    pub(crate) previous_version: Option<String>,
    pub(crate) remote_version: Option<String>,
    pub(crate) updated: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WagoRemoteVersion {
    pub(crate) source_url: Option<String>,
    pub(crate) version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WagoRelease {
    label: Option<String>,
    download_link: Option<String>,
    link: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WagoReleases {
    stable: Option<WagoRelease>,
    beta: Option<WagoRelease>,
    alpha: Option<WagoRelease>,
}

#[derive(Debug, Clone, Deserialize)]
struct WagoAddonSummaryRaw {
    id: String,
    display_name: String,
    #[serde(default)]
    summary: Option<String>,
    owner: Option<String>,
    authors: Option<Vec<String>>,
    website_url: Option<String>,
    download_count: Option<u64>,
    releases: Option<WagoReleases>,
    recent_release: Option<WagoReleases>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WagoAddonDetailsResponse {
    Wrapped { data: WagoAddonSummaryRaw },
    Direct(WagoAddonSummaryRaw),
}

#[derive(Debug, Clone)]
struct WagoAddonSummary {
    id: String,
    display_name: String,
    summary: Option<String>,
    owner: Option<String>,
    authors: Vec<String>,
    website_url: Option<String>,
    download_count: Option<u64>,
    releases: WagoReleases,
}

#[derive(Debug, Clone, Deserialize)]
struct WagoSearchResponse {
    data: Vec<WagoAddonSummaryRaw>,
}

#[derive(Debug, Clone)]
struct ExtractedFolders {
    source_root: PathBuf,
    folders: Vec<String>,
}

#[derive(Debug, Clone)]
struct WagoResolvedRelease {
    addon: WagoAddonSummary,
    stability: WagoStability,
    download_url: Url,
    version: Option<String>,
}

impl WagoAddonSummary {
    fn from_response(response: WagoAddonDetailsResponse) -> Self {
        let raw = match response {
            WagoAddonDetailsResponse::Wrapped { data } => data,
            WagoAddonDetailsResponse::Direct(data) => data,
        };

        Self {
            id: raw.id,
            display_name: raw.display_name,
            summary: raw.summary,
            owner: raw.owner,
            authors: raw.authors.unwrap_or_default(),
            website_url: raw.website_url,
            download_count: raw.download_count,
            releases: raw.releases.or(raw.recent_release).unwrap_or_default(),
        }
    }

    fn release(&self, stability: WagoStability) -> Option<&WagoRelease> {
        match stability {
            WagoStability::Stable => self.releases.stable.as_ref(),
            WagoStability::Beta => self.releases.beta.as_ref(),
            WagoStability::Alpha => self.releases.alpha.as_ref(),
        }
    }

    fn best_available_stability(&self) -> Option<WagoStability> {
        if self.releases.stable.is_some() {
            Some(WagoStability::Stable)
        } else if self.releases.beta.is_some() {
            Some(WagoStability::Beta)
        } else if self.releases.alpha.is_some() {
            Some(WagoStability::Alpha)
        } else {
            None
        }
    }
}

impl From<WagoAddonSummary> for WagoSearchResult {
    fn from(value: WagoAddonSummary) -> Self {
        let version = value
            .release(WagoStability::Stable)
            .and_then(|release| release.label.clone());
        Self {
            id: value.id,
            display_name: value.display_name,
            summary: value.summary,
            owner: value.owner,
            authors: value.authors,
            website_url: value.website_url,
            download_count: value.download_count,
            version,
        }
    }
}

pub fn parse_wago_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Wago addon target cannot be empty".to_string());
    }

    if trimmed.len() > 200 {
        return Err("Wago addon target is too long".to_string());
    }

    if trimmed.contains("..") || trimmed.chars().any(|character| character.is_control()) {
        return Err("Wago addon target contains forbidden characters".to_string());
    }

    if let Ok(url) = Url::parse(trimmed) {
        let Some(host) = url.host_str() else {
            return Err("Wago URL is missing a host".to_string());
        };
        if !host.ends_with("wago.io") {
            return Err("Wago URL must point to wago.io".to_string());
        }

        let segments = url
            .path_segments()
            .map(|values| values.collect::<Vec<_>>())
            .unwrap_or_default();
        if segments.len() < 2 || segments[0] != "addons" {
            return Err("Wago URL must look like https://addons.wago.io/addons/<slug>".to_string());
        }

        let slug = segments[1].trim();
        if !is_valid_wago_slug(slug) {
            return Err("Wago URL contains an invalid addon slug".to_string());
        }
        return Ok(slug.to_string());
    }

    if !is_valid_wago_slug(trimmed) {
        return Err("Wago addon slug contains unsupported characters".to_string());
    }

    Ok(trimmed.to_string())
}

pub async fn install_wago_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    api_key: &str,
    preferred_stability: WagoStability,
    dry_run: bool,
) -> Result<WagoInstallSummary, String> {
    install_wago_addon_with_replace(
        state_database,
        addon_dir,
        target,
        api_key,
        preferred_stability,
        dry_run,
        false,
    )
    .await
}

pub(crate) async fn install_wago_addon_with_replace(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    api_key: &str,
    preferred_stability: WagoStability,
    dry_run: bool,
    replace_existing: bool,
) -> Result<WagoInstallSummary, String> {
    let addon_id = parse_wago_target(target)?;
    let client = build_wago_client()?;
    let resolved =
        resolve_wago_release(&client, &addon_id, api_key, Some(preferred_stability)).await?;
    let bytes = download_wago_release_bytes(&client, &resolved.download_url, api_key).await?;

    install_downloaded_wago_addon(
        state_database,
        addon_dir,
        &resolved.addon,
        &bytes,
        resolved.stability,
        dry_run,
        replace_existing,
    )
}

pub(crate) async fn search_wago_addons(
    query: &str,
    api_key: &str,
) -> Result<Vec<WagoSearchResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Wago search query cannot be empty".to_string());
    }

    let client = build_wago_client()?;
    let results = fetch_addon_search_results(&client, trimmed, api_key).await?;
    Ok(results.into_iter().map(Into::into).collect())
}

pub(crate) async fn inspect_wago_install_target(
    state_database: &StateDatabase,
    addon_dir: &Path,
    target: &str,
    api_key: &str,
    preferred_stability: WagoStability,
) -> Result<WagoInstallInspection, String> {
    let addon_id = parse_wago_target(target)?;
    let client = build_wago_client()?;
    let resolved =
        resolve_wago_release(&client, &addon_id, api_key, Some(preferred_stability)).await?;
    let bytes = download_wago_release_bytes(&client, &resolved.download_url, api_key).await?;

    inspect_downloaded_wago_addon(
        state_database,
        addon_dir,
        &resolved.addon,
        &bytes,
        resolved.stability,
    )
}

pub(crate) async fn fetch_wago_remote_version(
    addon: &AddonRecord,
    api_key: &str,
) -> Result<WagoRemoteVersion, String> {
    let target = addon
        .source_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "tracked Wago addon '{}' is missing a source URL",
                addon.folder
            )
        })?;
    let addon_id = parse_wago_target(target)?;
    let client = build_wago_client()?;
    let resolved = resolve_wago_release(&client, &addon_id, api_key, None).await?;

    Ok(WagoRemoteVersion {
        source_url: Some(
            resolved
                .addon
                .website_url
                .clone()
                .unwrap_or_else(|| format!("https://addons.wago.io/addons/{}", resolved.addon.id)),
        ),
        version: resolved.version,
    })
}

pub(crate) async fn update_wago_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    api_key: &str,
    force: bool,
    dry_run: bool,
) -> Result<WagoUpdateSummary, String> {
    if tracked.source != SourceKind::Wago {
        return Err(format!("addon '{}' is not tracked as Wago", tracked.folder));
    }
    if !tracked.has_authoritative_owned_folders() {
        return Err(format!(
            "tracked Wago addon '{}' does not have authoritative managed ownership",
            tracked.folder
        ));
    }

    let target = tracked
        .source_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "tracked Wago addon '{}' is missing a source URL",
                tracked.folder
            )
        })?;
    let addon_id = parse_wago_target(target)?;
    let client = build_wago_client()?;
    let resolved = resolve_wago_release(&client, &addon_id, api_key, None).await?;
    let remote_version = resolved.version.clone();

    if !force && versions_match(tracked.version.as_deref(), remote_version.as_deref()) {
        return Ok(WagoUpdateSummary {
            addon_id: resolved.addon.id.clone(),
            addon_name: resolved.addon.display_name.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: tracked
                .owned_folders
                .iter()
                .map(|owned| owned.name.clone())
                .chain(std::iter::once(tracked.folder.clone()))
                .collect(),
            stability: resolved.stability,
            previous_version: tracked.version.clone(),
            remote_version,
            updated: false,
            dry_run,
        });
    }

    let bytes = download_wago_release_bytes(&client, &resolved.download_url, api_key).await?;
    update_downloaded_wago_addon(
        state_database,
        addon_dir,
        tracked,
        &resolved.addon,
        &bytes,
        resolved.stability,
        dry_run,
    )
}

fn install_downloaded_wago_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    addon: &WagoAddonSummary,
    archive_bytes: &[u8],
    stability: WagoStability,
    dry_run: bool,
    replace_existing: bool,
) -> Result<WagoInstallSummary, String> {
    let temp_root = create_temp_work_dir("wago-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let backup_root = temp_root.join("replace-backup");
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let parent_folder = determine_parent_folder(&extracted.folders, &addon.display_name)?;
        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == parent_folder)
            .ok_or_else(|| {
                format!(
                    "installed Wago package for '{}' did not yield a scannable parent folder '{}'",
                    addon.display_name, parent_folder
                )
            })?;

        if dry_run {
            return Ok(WagoInstallSummary {
                addon_id: addon.id.clone(),
                addon_name: addon.display_name.clone(),
                parent_folder,
                installed_folders: extracted.folders,
                stability,
                version: release_version(addon, stability),
                dry_run: true,
            });
        }

        let mut replaced_existing = Vec::new();
        if replace_existing {
            fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
            replaced_existing =
                replace_existing_install_targets(addon_dir, &backup_root, &extracted.folders)?;
        } else {
            preflight_install_targets(addon_dir, &extracted.folders)?;
        }

        let mut copied_folders = Vec::new();
        for folder in &extracted.folders {
            copy_dir_recursively(&extracted.source_root.join(folder), &addon_dir.join(folder))?;
            copied_folders.push(folder.clone());
        }

        let record =
            build_managed_wago_record(addon, parent_metadata, &extracted.folders, stability);

        if let Err(error) = state_database.record_managed_addon(&record) {
            for folder in copied_folders.iter().rev() {
                let _ = fs::remove_dir_all(addon_dir.join(folder));
            }
            restore_replaced_install_targets(addon_dir, &backup_root, &replaced_existing);
            return Err(error.to_string());
        }

        Ok(WagoInstallSummary {
            addon_id: addon.id.clone(),
            addon_name: addon.display_name.clone(),
            parent_folder,
            installed_folders: extracted.folders,
            stability,
            version: release_version(addon, stability),
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn inspect_downloaded_wago_addon(
    state_database: &StateDatabase,
    addon_dir: &Path,
    addon: &WagoAddonSummary,
    archive_bytes: &[u8],
    stability: WagoStability,
) -> Result<WagoInstallInspection, String> {
    let temp_root = create_temp_work_dir("wago-inspect")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let parent_folder = determine_parent_folder(&extracted.folders, &addon.display_name)?;
        let existing_folders = extracted
            .folders
            .iter()
            .filter(|folder| addon_dir.join(folder).exists())
            .cloned()
            .collect::<Vec<_>>();
        let tracked_parent = state_database
            .list_addons()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|tracked| {
                tracked.source == SourceKind::Wago
                    && tracked
                        .source_url
                        .as_deref()
                        .and_then(|url| parse_wago_target(url).ok())
                        .is_some_and(|tracked_id| tracked_id == addon.id)
            })
            .map(|tracked| tracked.folder);

        Ok(WagoInstallInspection {
            addon_id: addon.id.clone(),
            addon_name: addon.display_name.clone(),
            parent_folder,
            installed_folders: extracted.folders,
            stability,
            version: release_version(addon, stability),
            existing_folders,
            tracked_parent,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn update_downloaded_wago_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    addon: &WagoAddonSummary,
    archive_bytes: &[u8],
    stability: WagoStability,
    dry_run: bool,
) -> Result<WagoUpdateSummary, String> {
    let temp_root = create_temp_work_dir("wago-update")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let update_root = temp_root.join("backup");
    fs::create_dir_all(&update_root).map_err(|error| error.to_string())?;
    let previous_version = tracked.version.clone();
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        if !extracted
            .folders
            .iter()
            .any(|folder| folder == &tracked.folder)
        {
            return Err(format!(
                "Wago update for '{}' does not contain the tracked parent folder '{}'",
                addon.display_name, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == tracked.folder)
            .ok_or_else(|| {
                format!(
                    "updated Wago package for '{}' did not yield a scannable tracked parent folder '{}'",
                    addon.display_name, tracked.folder
                )
            })?;

        let managed_folders = std::iter::once(tracked.folder.clone())
            .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
            .collect::<Vec<_>>();

        preflight_update_targets(addon_dir, &extracted.folders, &managed_folders)?;

        if dry_run {
            return Ok(WagoUpdateSummary {
                addon_id: addon.id.clone(),
                addon_name: addon.display_name.clone(),
                parent_folder: tracked.folder.clone(),
                installed_folders: extracted.folders,
                stability,
                previous_version,
                remote_version: release_version(addon, stability),
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
            let backup = update_root.join(folder);
            copy_dir_recursively(&source, &backup)?;
            fs::remove_dir_all(&source).map_err(|error| error.to_string())?;
            moved_existing.push(folder.clone());
        }

        let mut copied_new = Vec::new();
        for folder in &extracted.folders {
            copy_dir_recursively(&extracted.source_root.join(folder), &addon_dir.join(folder))?;
            copied_new.push(folder.clone());
        }

        let record =
            build_managed_wago_record(addon, parent_metadata, &extracted.folders, stability);

        if let Err(error) = state_database.record_managed_addon(&record) {
            rollback_updated_folders(addon_dir, &update_root, &copied_new, &moved_existing);
            return Err(error.to_string());
        }

        Ok(WagoUpdateSummary {
            addon_id: addon.id.clone(),
            addon_name: addon.display_name.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: extracted.folders,
            stability,
            previous_version,
            remote_version: release_version(addon, stability),
            updated: true,
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn build_managed_wago_record(
    addon: &WagoAddonSummary,
    parent_metadata: &ScannedAddon,
    installed_folders: &[String],
    stability: WagoStability,
) -> AddonRecord {
    let mut record = AddonRecord::new(
        parent_metadata.name.clone(),
        parent_metadata.folder.clone(),
        SourceKind::Wago,
    );
    record.kind = parent_metadata.kind;
    record.flavor = parent_metadata.flavor;
    record.version = release_version(addon, stability).or_else(|| parent_metadata.version.clone());
    record.author = addon
        .owner
        .clone()
        .or_else(|| addon.authors.first().cloned())
        .or_else(|| parent_metadata.author.clone());
    record.interface = parent_metadata.interface.clone();
    record.source_url = Some(
        addon
            .website_url
            .clone()
            .unwrap_or_else(|| format!("https://addons.wago.io/addons/{}", addon.id)),
    );
    record.required_deps = parent_metadata.required_deps.clone();
    record.optional_deps = parent_metadata.optional_deps.clone();
    record.embedded_libs = parent_metadata.embedded_libs.clone();
    record.remote_version = release_version(addon, stability);
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

fn release_version(addon: &WagoAddonSummary, stability: WagoStability) -> Option<String> {
    addon
        .release(stability)
        .and_then(|release| release.label.clone())
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

fn build_wago_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("LemonUp/2 (+https://github.com/archcorsair/lemonup)")
        .build()
        .map_err(|error| error.to_string())
}

async fn resolve_wago_release(
    client: &Client,
    addon_id: &str,
    api_key: &str,
    preferred_stability: Option<WagoStability>,
) -> Result<WagoResolvedRelease, String> {
    let addon = fetch_addon_details(client, addon_id, api_key).await?;
    let effective_stability = addon.best_available_stability().ok_or_else(|| {
        format!(
            "no stable, beta, or alpha release is available for Wago addon '{}'",
            addon.display_name
        )
    })?;
    let selected_stability = match preferred_stability {
        Some(preferred) if addon.release(preferred).is_some() => preferred,
        _ => effective_stability,
    };
    let release = addon.release(selected_stability).ok_or_else(|| {
        format!(
            "requested Wago release '{}' is not available for '{}'",
            selected_stability.as_str(),
            addon.display_name
        )
    })?;
    let download_url = release
        .download_link
        .as_deref()
        .or(release.link.as_deref())
        .ok_or_else(|| format!("Wago addon '{}' has no download URL", addon.display_name))?;

    Ok(WagoResolvedRelease {
        version: release.label.clone(),
        download_url: validate_download_url(download_url)?,
        stability: selected_stability,
        addon,
    })
}

async fn download_wago_release_bytes(
    client: &Client,
    download_url: &Url,
    api_key: &str,
) -> Result<Vec<u8>, String> {
    client
        .get(download_url.clone())
        .bearer_auth(api_key)
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

async fn fetch_addon_details(
    client: &Client,
    addon_id: &str,
    api_key: &str,
) -> Result<WagoAddonSummary, String> {
    let url = format!("{API_BASE}{EXTERNAL_PATH}/addons/{addon_id}?game_version=retail");
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    let parsed = response
        .json::<WagoAddonDetailsResponse>()
        .await
        .map_err(|error| error.to_string())?;
    Ok(WagoAddonSummary::from_response(parsed))
}

async fn fetch_addon_search_results(
    client: &Client,
    query: &str,
    api_key: &str,
) -> Result<Vec<WagoAddonSummary>, String> {
    let url = Url::parse_with_params(
        &format!("{API_BASE}{EXTERNAL_PATH}/addons/_search"),
        [
            ("query", query),
            ("game_version", "retail"),
            ("stability", "stable"),
        ],
    )
    .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    let parsed = response
        .json::<WagoSearchResponse>()
        .await
        .map_err(|error| error.to_string())?;
    Ok(parsed
        .data
        .into_iter()
        .map(|raw| WagoAddonSummary::from_response(WagoAddonDetailsResponse::Direct(raw)))
        .collect())
}

fn validate_download_url(value: &str) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|error| error.to_string())?;
    let Some(host) = parsed.host_str() else {
        return Err("Wago download URL is missing a host".to_string());
    };
    if !host.ends_with("wago.io") {
        return Err("Wago download URL must stay on a wago.io host".to_string());
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
        "no top-level addon folders with root .toc files were found in the Wago archive"
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

fn determine_parent_folder(folders: &[String], display_name: &str) -> Result<String, String> {
    if folders.is_empty() {
        return Err("no addon folders were discovered in the Wago archive".to_string());
    }
    if folders.len() == 1 {
        return Ok(folders[0].clone());
    }

    let normalized_display_name = normalize_folder_like(display_name);
    if let Some(exact) = folders.iter().find(|folder| {
        let normalized_folder = normalize_folder_like(folder);
        normalized_folder == normalized_display_name
    }) {
        return Ok(exact.clone());
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

    Ok(shortest)
}

fn normalize_folder_like(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-' && *character != '_')
        .collect::<String>()
        .to_ascii_lowercase()
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

fn replace_existing_install_targets(
    addon_dir: &Path,
    backup_root: &Path,
    folders: &[String],
) -> Result<Vec<String>, String> {
    let mut replaced = Vec::new();
    for folder in folders {
        let source = addon_dir.join(folder);
        if !source.exists() {
            continue;
        }
        let backup = backup_root.join(folder);
        copy_dir_recursively(&source, &backup)?;
        fs::remove_dir_all(&source).map_err(|error| error.to_string())?;
        replaced.push(folder.clone());
    }
    Ok(replaced)
}

fn restore_replaced_install_targets(addon_dir: &Path, backup_root: &Path, folders: &[String]) {
    for folder in folders {
        let destination = addon_dir.join(folder);
        if destination.exists() {
            let _ = fs::remove_dir_all(&destination);
        }
        let backup = backup_root.join(folder);
        if backup.exists() {
            let _ = copy_dir_recursively(&backup, &destination);
        }
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

    Err("failed to create a unique temporary Wago install directory".to_string())
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

fn is_valid_wago_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

pub(crate) fn resolve_wago_api_key(config_state: &ConfigLoad) -> Option<String> {
    let from_config = match config_state {
        ConfigLoad::Loaded(config) => config.wago_api_key.clone(),
        ConfigLoad::Missing(_) => None,
    };

    from_config
        .or_else(read_process_wago_api_key)
        .or_else(load_repo_root_wago_api_key)
}

fn read_process_wago_api_key() -> Option<String> {
    std::env::var("WAGO_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn load_repo_root_wago_api_key() -> Option<String> {
    let dotenv_path = repo_root_dotenv_path();
    if !dotenv_path.exists() {
        return None;
    }

    let iter = dotenvy::from_path_iter(&dotenv_path).ok()?;
    for entry in iter {
        let (key, value) = entry.ok()?;
        if key == "WAGO_API_KEY" {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
            return None;
        }
    }

    None
}

fn repo_root_dotenv_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join(".env")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        WagoAddonDetailsResponse, WagoAddonSummary, WagoAddonSummaryRaw, WagoRelease, WagoReleases,
        WagoSearchResult, WagoStability, determine_parent_folder, discover_extracted_folders,
        extract_zip_archive, inspect_downloaded_wago_addon, install_downloaded_wago_addon,
        parse_wago_target, release_version, update_downloaded_wago_addon, versions_match,
    };
    use lemonup_core::{GameFlavor, OwnedFolder, SourceKind, StateDatabase};
    use tempfile::tempdir;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    fn sample_addon() -> WagoAddonSummary {
        WagoAddonSummary {
            id: "details".to_string(),
            display_name: "Details! Damage Meter".to_string(),
            summary: Some("Top meters".to_string()),
            owner: Some("Tercioo".to_string()),
            authors: vec!["Tercioo".to_string()],
            website_url: Some("https://addons.wago.io/addons/details".to_string()),
            download_count: Some(1_000_000),
            releases: WagoReleases {
                stable: Some(WagoRelease {
                    label: Some("v1.2.3".to_string()),
                    download_link: Some("https://addons.wago.io/download/example.zip".to_string()),
                    link: None,
                }),
                beta: None,
                alpha: None,
            },
        }
    }

    fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::<u8>::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        for (name, body) in entries {
            writer
                .start_file(name, options)
                .expect("start zip file entry");
            std::io::Write::write_all(&mut writer, body.as_bytes()).expect("write zip entry");
        }

        writer.finish().expect("finish zip").into_inner()
    }

    #[test]
    fn parse_wago_target_accepts_slug_or_wago_url() {
        assert_eq!(parse_wago_target("details").expect("slug"), "details");
        assert_eq!(
            parse_wago_target("https://addons.wago.io/addons/details").expect("url"),
            "details"
        );
        assert_eq!(
            parse_wago_target("https://wago.io/addons/details").expect("root url"),
            "details"
        );
    }

    #[test]
    fn parse_wago_target_rejects_non_wago_url() {
        assert!(parse_wago_target("https://example.com/addons/details").is_err());
        assert!(parse_wago_target("../details").is_err());
    }

    #[test]
    fn determine_parent_folder_prefers_display_name_match() {
        let folders = vec![
            "Details".to_string(),
            "Details_Compare2".to_string(),
            "Details_DataStorage".to_string(),
        ];
        assert_eq!(
            determine_parent_folder(&folders, "Details! Damage Meter").expect("parent"),
            "Details"
        );
    }

    #[test]
    fn addon_summary_falls_back_to_recent_release() {
        let addon = WagoAddonSummary::from_response(WagoAddonDetailsResponse::Direct(
            WagoAddonSummaryRaw {
                id: "details".to_string(),
                display_name: "Details! Damage Meter".to_string(),
                summary: Some("Top meters".to_string()),
                owner: Some("Tercioo".to_string()),
                authors: Some(vec!["Tercioo".to_string()]),
                website_url: None,
                download_count: Some(1_000_000),
                releases: None,
                recent_release: Some(WagoReleases {
                    stable: Some(WagoRelease {
                        label: Some("v1.2.3".to_string()),
                        download_link: Some(
                            "https://addons.wago.io/download/example.zip".to_string(),
                        ),
                        link: None,
                    }),
                    beta: None,
                    alpha: None,
                }),
            },
        ));

        assert_eq!(
            addon.best_available_stability(),
            Some(WagoStability::Stable)
        );
        assert_eq!(
            release_version(&addon, WagoStability::Stable).as_deref(),
            Some("v1.2.3")
        );
    }

    #[test]
    fn discover_extracted_folders_supports_single_wrapper_directory() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("package.zip");
        let extract_path = temp.path().join("extract");
        let bytes = build_zip(&[
            ("wrapper/Details/Details.toc", "## Title: Details\n"),
            (
                "wrapper/Details_DataStorage/Details_DataStorage.toc",
                "## Title: Details Data Storage\n",
            ),
        ]);
        fs::write(&zip_path, bytes).expect("write zip");

        extract_zip_archive(&zip_path, &extract_path).expect("extract zip");
        let discovered = discover_extracted_folders(&extract_path).expect("discover folders");

        assert!(discovered.source_root.ends_with("wrapper"));
        assert_eq!(discovered.folders, vec!["Details", "Details_DataStorage"]);
    }

    #[test]
    fn search_result_conversion_keeps_summary_downloads_and_stable_version() {
        let result = WagoSearchResult::from(sample_addon());

        assert_eq!(result.id, "details");
        assert_eq!(result.summary.as_deref(), Some("Top meters"));
        assert_eq!(result.download_count, Some(1_000_000));
        assert_eq!(result.version.as_deref(), Some("v1.2.3"));
    }

    #[test]
    fn inspect_downloaded_wago_addon_flags_existing_install_for_confirmation() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(addon_dir.join("Details")).expect("create addon dir");

        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let bytes = build_zip(&[(
            "Details/Details.toc",
            "## Title: Details! Damage Meter\n## Version: 11.2.0\n## Author: Tercioo\n## Interface: 110205\n",
        )]);

        let inspection = inspect_downloaded_wago_addon(
            &database,
            &addon_dir,
            &sample_addon(),
            &bytes,
            WagoStability::Stable,
        )
        .expect("inspect addon");

        assert!(inspection.requires_confirmation());
        assert_eq!(inspection.parent_folder, "Details");
        assert_eq!(inspection.existing_folders, vec!["Details"]);
    }

    #[test]
    fn install_downloaded_wago_addon_records_managed_ownership() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let bytes = build_zip(&[
            (
                "Details/Details.toc",
                "## Title: Details! Damage Meter\n## Version: 11.2.0\n## Author: Tercioo\n## Interface: 110205\n",
            ),
            (
                "Details_DataStorage/Details_DataStorage.toc",
                "## Title: Details Data Storage\n## Version: 11.2.0\n## Author: Tercioo\n## Interface: 110205\n",
            ),
        ]);

        let summary = install_downloaded_wago_addon(
            &mut database,
            &addon_dir,
            &sample_addon(),
            &bytes,
            WagoStability::Stable,
            false,
            false,
        )
        .expect("install addon");

        assert_eq!(summary.parent_folder, "Details");
        assert_eq!(summary.installed_folders.len(), 2);
        assert!(addon_dir.join("Details").exists());
        assert!(addon_dir.join("Details_DataStorage").exists());

        let installed = database.list_addons().expect("list addons");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].source, SourceKind::Wago);
        assert_eq!(installed[0].flavor, GameFlavor::Retail);
        assert_eq!(installed[0].remote_version.as_deref(), Some("v1.2.3"));
        assert_eq!(
            installed[0].owned_folders,
            vec![OwnedFolder {
                name: "Details_DataStorage".to_string(),
            }]
        );
        assert!(installed[0].has_authoritative_owned_folders());
    }

    #[test]
    fn install_downloaded_wago_addon_dry_run_does_not_write_disk_or_state() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let bytes = build_zip(&[(
            "Details/Details.toc",
            "## Title: Details! Damage Meter\n## Version: 11.2.0\n## Author: Tercioo\n## Interface: 110205\n",
        )]);

        let summary = install_downloaded_wago_addon(
            &mut database,
            &addon_dir,
            &sample_addon(),
            &bytes,
            WagoStability::Stable,
            true,
            false,
        )
        .expect("dry run install");

        assert!(summary.dry_run);
        assert!(!addon_dir.join("Details").exists());
        assert!(database.list_addons().expect("list addons").is_empty());
    }

    #[test]
    fn update_downloaded_wago_addon_replaces_owned_folders_and_preserves_install_time() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        fs::create_dir_all(addon_dir.join("WeakAuras")).expect("create parent folder");
        fs::create_dir_all(addon_dir.join("WeakAurasOptions")).expect("create child folder");
        fs::create_dir_all(addon_dir.join("WeakAurasArchive")).expect("create old child folder");

        let mut tracked =
            lemonup_core::AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        tracked.version = Some("5.20.0".to_string());
        tracked.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        tracked.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "WeakAurasArchive".to_string(),
            },
            OwnedFolder {
                name: "WeakAurasOptions".to_string(),
            },
        ]);
        let installed_at = tracked.installed_at;
        database
            .record_managed_addon(&tracked)
            .expect("seed tracked addon");

        let addon = WagoAddonSummary {
            id: "VBNBxKx5".to_string(),
            display_name: "WeakAuras".to_string(),
            summary: Some("Aura framework".to_string()),
            owner: Some("WeakAuras Team".to_string()),
            authors: vec!["WeakAuras Team".to_string()],
            website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
            download_count: Some(500_000),
            releases: WagoReleases {
                stable: Some(WagoRelease {
                    label: Some("5.21.1".to_string()),
                    download_link: Some("https://addons.wago.io/download/example.zip".to_string()),
                    link: None,
                }),
                beta: None,
                alpha: None,
            },
        };

        let bytes = build_zip(&[
            (
                "WeakAuras/WeakAuras.toc",
                "## Title: WeakAuras\n## Version: 5.21.1\n## Author: WeakAuras Team\n## Interface: 110205\n",
            ),
            (
                "WeakAurasOptions/WeakAurasOptions.toc",
                "## Title: WeakAuras Options\n## Version: 5.21.1\n## Author: WeakAuras Team\n## Interface: 110205\n",
            ),
            (
                "WeakAurasTemplates/WeakAurasTemplates.toc",
                "## Title: WeakAuras Templates\n## Version: 5.21.1\n## Author: WeakAuras Team\n## Interface: 110205\n",
            ),
        ]);

        let summary = update_downloaded_wago_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &addon,
            &bytes,
            WagoStability::Stable,
            false,
        )
        .expect("update addon");

        assert!(summary.updated);
        assert_eq!(summary.previous_version.as_deref(), Some("5.20.0"));
        assert_eq!(summary.remote_version.as_deref(), Some("5.21.1"));
        assert!(addon_dir.join("WeakAuras").exists());
        assert!(addon_dir.join("WeakAurasOptions").exists());
        assert!(addon_dir.join("WeakAurasTemplates").exists());
        assert!(!addon_dir.join("WeakAurasArchive").exists());

        let stored = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(
            stored.installed_at.unix_timestamp(),
            installed_at.unix_timestamp()
        );
        assert_eq!(stored.version.as_deref(), Some("5.21.1"));
        assert_eq!(stored.remote_version.as_deref(), Some("5.21.1"));
        assert_eq!(
            stored
                .owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["WeakAurasOptions", "WeakAurasTemplates"]
        );
        assert!(stored.has_authoritative_owned_folders());
    }

    #[test]
    fn update_downloaded_wago_addon_dry_run_does_not_mutate_disk_or_state() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        fs::create_dir_all(addon_dir.join("WeakAuras")).expect("create parent folder");

        let mut tracked =
            lemonup_core::AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        tracked.version = Some("5.20.0".to_string());
        tracked.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        database
            .record_managed_addon(&tracked)
            .expect("seed tracked addon");

        let addon = WagoAddonSummary {
            id: "VBNBxKx5".to_string(),
            display_name: "WeakAuras".to_string(),
            summary: Some("Aura framework".to_string()),
            owner: Some("WeakAuras Team".to_string()),
            authors: vec!["WeakAuras Team".to_string()],
            website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
            download_count: Some(500_000),
            releases: WagoReleases {
                stable: Some(WagoRelease {
                    label: Some("5.21.1".to_string()),
                    download_link: Some("https://addons.wago.io/download/example.zip".to_string()),
                    link: None,
                }),
                beta: None,
                alpha: None,
            },
        };

        let bytes = build_zip(&[(
            "WeakAuras/WeakAuras.toc",
            "## Title: WeakAuras\n## Version: 5.21.1\n## Author: WeakAuras Team\n## Interface: 110205\n",
        )]);

        let summary = update_downloaded_wago_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &addon,
            &bytes,
            WagoStability::Stable,
            true,
        )
        .expect("dry-run update");

        assert!(summary.dry_run);
        let stored = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("5.20.0"));
        assert!(!addon_dir.join("WeakAurasTemplates").exists());
    }

    #[test]
    fn versions_match_ignores_case_and_whitespace() {
        assert!(versions_match(Some(" 5.21.1 "), Some("5.21.1")));
        assert!(versions_match(Some("V5.21.1"), Some("v5.21.1")));
        assert!(versions_match(Some("v5.21.1"), Some("5.21.1")));
        assert!(!versions_match(Some("5.21.0"), Some("5.21.1")));
    }
}
