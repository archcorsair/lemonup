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

const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubInstallSummary {
    pub(crate) repo_url: String,
    pub(crate) repo_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) version: Option<String>,
    pub(crate) git_commit: String,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubUpdateSummary {
    pub(crate) repo_url: String,
    pub(crate) repo_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) previous_version: Option<String>,
    pub(crate) previous_commit: Option<String>,
    pub(crate) remote_version: Option<String>,
    pub(crate) updated: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubRemoteVersion {
    pub(crate) source_url: Option<String>,
    pub(crate) version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubRepoRef {
    owner: String,
    repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubRepoMetadata {
    owner: String,
    repo: String,
    default_branch: String,
    html_url: String,
    owner_login: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubResolvedRepo {
    metadata: GithubRepoMetadata,
    head_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubOwnerResponse {
    login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubRepoResponse {
    name: String,
    default_branch: String,
    html_url: String,
    owner: GithubOwnerResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubCommitResponse {
    sha: String,
}

pub(crate) fn parse_github_target(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("GitHub repo target cannot be empty".to_string());
    }

    if trimmed.len() > 300 {
        return Err("GitHub repo target is too long".to_string());
    }

    if trimmed.chars().any(|character| character.is_control()) {
        return Err("GitHub repo target contains forbidden characters".to_string());
    }

    let parsed = parse_github_repo_url(trimmed)?;
    Ok(canonical_github_url(&parsed.owner, &parsed.repo))
}

pub(crate) async fn install_github_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    dry_run: bool,
) -> Result<GithubInstallSummary, String> {
    let repo = parse_github_repo_url(target)?;
    let client = build_github_client()?;
    let resolved = fetch_github_repo(&client, &repo).await?;
    let bytes = download_github_zipball_bytes(&client, &resolved).await?;
    install_downloaded_github_addon(state_database, addon_dir, &resolved, &bytes, dry_run)
}

pub(crate) async fn fetch_github_remote_version(
    addon: &AddonRecord,
) -> Result<GithubRemoteVersion, String> {
    let repo = resolve_tracked_github_repo(addon)?;
    let client = build_github_client()?;
    let resolved = fetch_github_repo(&client, &repo).await?;
    Ok(GithubRemoteVersion {
        source_url: Some(resolved.metadata.html_url),
        version: Some(resolved.head_sha),
    })
}

pub(crate) async fn update_github_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    force: bool,
    dry_run: bool,
) -> Result<GithubUpdateSummary, String> {
    if tracked.source != SourceKind::GitHub {
        return Err(format!(
            "addon '{}' is not tracked as GitHub",
            tracked.folder
        ));
    }

    let repo = resolve_tracked_github_repo(tracked)?;
    let client = build_github_client()?;
    let resolved = fetch_github_repo(&client, &repo).await?;
    let remote_version = Some(resolved.head_sha.clone());
    let installed_commit = tracked.git_commit.as_deref().or(tracked.version.as_deref());

    if !force && commits_match(installed_commit, remote_version.as_deref()) {
        return Ok(GithubUpdateSummary {
            repo_url: resolved.metadata.html_url.clone(),
            repo_name: resolved.metadata.repo.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: std::iter::once(tracked.folder.clone())
                .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
                .collect(),
            previous_version: tracked.version.clone(),
            previous_commit: tracked.git_commit.clone(),
            remote_version,
            updated: false,
            dry_run,
        });
    }

    let bytes = download_github_zipball_bytes(&client, &resolved).await?;
    update_downloaded_github_addon(
        state_database,
        addon_dir,
        tracked,
        &resolved,
        &bytes,
        dry_run,
    )
}

fn build_github_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("LemonUp/2 (+https://github.com/archcorsair/lemonup)")
        .build()
        .map_err(|error| error.to_string())
}

async fn fetch_github_repo(
    client: &Client,
    repo: &GithubRepoRef,
) -> Result<GithubResolvedRepo, String> {
    let metadata = fetch_github_repo_metadata(client, repo).await?;
    let head_sha = fetch_github_head_commit(client, &metadata).await?;
    Ok(GithubResolvedRepo { metadata, head_sha })
}

async fn fetch_github_repo_metadata(
    client: &Client,
    repo: &GithubRepoRef,
) -> Result<GithubRepoMetadata, String> {
    let url = format!("{API_BASE}/repos/{}/{}", repo.owner, repo.repo);
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(format!(
            "GitHub repo '{}/{}' was not found",
            repo.owner, repo.repo
        ));
    }

    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let parsed = response
        .json::<GithubRepoResponse>()
        .await
        .map_err(|error| error.to_string())?;

    Ok(GithubRepoMetadata {
        owner: repo.owner.clone(),
        repo: parsed.name,
        default_branch: parsed.default_branch,
        html_url: canonical_github_url(&repo.owner, &repo.repo),
        owner_login: parsed.owner.login,
    })
}

async fn fetch_github_head_commit(
    client: &Client,
    metadata: &GithubRepoMetadata,
) -> Result<String, String> {
    let url = format!(
        "{API_BASE}/repos/{}/{}/commits/{}",
        metadata.owner, metadata.repo, metadata.default_branch
    );
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(format!(
            "GitHub repo '{}/{}' is missing default branch '{}'",
            metadata.owner, metadata.repo, metadata.default_branch
        ));
    }

    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let parsed = response
        .json::<GithubCommitResponse>()
        .await
        .map_err(|error| error.to_string())?;
    Ok(parsed.sha)
}

async fn download_github_zipball_bytes(
    client: &Client,
    resolved: &GithubResolvedRepo,
) -> Result<Vec<u8>, String> {
    let url = format!(
        "{API_BASE}/repos/{}/{}/zipball/{}",
        resolved.metadata.owner, resolved.metadata.repo, resolved.head_sha
    );
    client
        .get(url)
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

fn install_downloaded_github_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    resolved: &GithubResolvedRepo,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<GithubInstallSummary, String> {
    let temp_root = create_temp_work_dir("github-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let parent_folder = determine_parent_folder(&extracted.folders, &resolved.metadata.repo)?;
        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == parent_folder)
            .ok_or_else(|| {
                format!(
                    "GitHub repo '{}' did not yield a scannable parent folder '{}'",
                    resolved.metadata.repo, parent_folder
                )
            })?;

        if dry_run {
            return Ok(GithubInstallSummary {
                repo_url: resolved.metadata.html_url.clone(),
                repo_name: resolved.metadata.repo.clone(),
                parent_folder,
                installed_folders: extracted.folders,
                version: install_version(parent_metadata.version.as_deref(), &resolved.head_sha),
                git_commit: resolved.head_sha.clone(),
                dry_run: true,
            });
        }

        preflight_install_targets(addon_dir, &extracted.folders)?;
        let mut copied_folders = Vec::new();
        for folder in &extracted.folders {
            copy_dir_recursively(&extracted.source_root.join(folder), &addon_dir.join(folder))?;
            copied_folders.push(folder.clone());
        }

        let record = build_managed_github_record(resolved, parent_metadata, &extracted.folders);
        if let Err(error) = state_database.record_managed_addon(&record) {
            for folder in copied_folders.iter().rev() {
                let _ = fs::remove_dir_all(addon_dir.join(folder));
            }
            return Err(error.to_string());
        }

        Ok(GithubInstallSummary {
            repo_url: resolved.metadata.html_url.clone(),
            repo_name: resolved.metadata.repo.clone(),
            parent_folder,
            installed_folders: extracted.folders,
            version: install_version(parent_metadata.version.as_deref(), &resolved.head_sha),
            git_commit: resolved.head_sha.clone(),
            dry_run: false,
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

fn update_downloaded_github_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    resolved: &GithubResolvedRepo,
    archive_bytes: &[u8],
    dry_run: bool,
) -> Result<GithubUpdateSummary, String> {
    let temp_root = create_temp_work_dir("github-update")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let backup_root = temp_root.join("backup");
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let previous_version = tracked.version.clone();
    let previous_commit = tracked.git_commit.clone();
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root)?;
        let resolved_parent = determine_parent_folder(&extracted.folders, &resolved.metadata.repo)?;
        if resolved_parent != tracked.folder {
            return Err(format!(
                "GitHub update for '{}' resolved parent '{}' but tracked addon folder is '{}'",
                resolved.metadata.repo, resolved_parent, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(&extracted.source_root, GameFlavor::Retail)
            .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == tracked.folder)
            .ok_or_else(|| {
                format!(
                    "updated GitHub repo '{}' did not yield a scannable tracked parent folder '{}'",
                    resolved.metadata.repo, tracked.folder
                )
            })?;

        let managed_folders = std::iter::once(tracked.folder.clone())
            .chain(tracked.owned_folders.iter().map(|owned| owned.name.clone()))
            .collect::<Vec<_>>();

        preflight_update_targets(addon_dir, &extracted.folders, &managed_folders)?;

        if dry_run {
            return Ok(GithubUpdateSummary {
                repo_url: resolved.metadata.html_url.clone(),
                repo_name: resolved.metadata.repo.clone(),
                parent_folder: tracked.folder.clone(),
                installed_folders: extracted.folders,
                previous_version,
                previous_commit,
                remote_version: Some(resolved.head_sha.clone()),
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

        let record = build_managed_github_record(resolved, parent_metadata, &extracted.folders);
        if let Err(error) = state_database.record_managed_addon(&record) {
            rollback_updated_folders(addon_dir, &backup_root, &copied_new, &moved_existing);
            return Err(error.to_string());
        }

        Ok(GithubUpdateSummary {
            repo_url: resolved.metadata.html_url.clone(),
            repo_name: resolved.metadata.repo.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: extracted.folders,
            previous_version,
            previous_commit,
            remote_version: Some(resolved.head_sha.clone()),
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

fn build_managed_github_record(
    resolved: &GithubResolvedRepo,
    parent_metadata: &ScannedAddon,
    installed_folders: &[String],
) -> AddonRecord {
    let mut record = AddonRecord::new(
        parent_metadata.name.clone(),
        parent_metadata.folder.clone(),
        SourceKind::GitHub,
    );
    record.kind = parent_metadata.kind;
    record.flavor = parent_metadata.flavor;
    record.version = install_version(parent_metadata.version.as_deref(), &resolved.head_sha);
    record.git_commit = Some(resolved.head_sha.clone());
    record.author = parent_metadata
        .author
        .clone()
        .or_else(|| Some(resolved.metadata.owner_login.clone()));
    record.interface = parent_metadata.interface.clone();
    record.source_url = Some(resolved.metadata.html_url.clone());
    record.required_deps = parent_metadata.required_deps.clone();
    record.optional_deps = parent_metadata.optional_deps.clone();
    record.embedded_libs = parent_metadata.embedded_libs.clone();
    record.remote_version = Some(resolved.head_sha.clone());
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

fn resolve_tracked_github_repo(addon: &AddonRecord) -> Result<GithubRepoRef, String> {
    addon
        .source_url
        .as_deref()
        .ok_or_else(|| {
            format!(
                "tracked GitHub addon '{}' is missing a source URL",
                addon.folder
            )
        })
        .and_then(parse_github_repo_url)
}

fn parse_github_repo_url(value: &str) -> Result<GithubRepoRef, String> {
    let url = Url::parse(value)
        .map_err(|_| "GitHub install currently requires a full repo URL".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "GitHub URL is missing a host".to_string())?;
    if host != "github.com" && host != "www.github.com" {
        return Err("GitHub URL must point to github.com".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("GitHub URL must not include a query string or fragment".to_string());
    }
    let segments = url
        .path_segments()
        .map(|values| {
            values
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if segments.len() != 2 {
        return Err("GitHub URL must look like https://github.com/<owner>/<repo>".to_string());
    }
    let owner = segments[0].trim();
    let repo = segments[1].trim_end_matches(".git").trim();
    if !is_valid_github_path_part(owner) || !is_valid_github_path_part(repo) {
        return Err("GitHub URL contains an invalid owner or repo name".to_string());
    }

    Ok(GithubRepoRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

fn is_valid_github_path_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn canonical_github_url(owner: &str, repo: &str) -> String {
    format!("https://github.com/{owner}/{repo}")
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
        "no top-level addon folders with root .toc files were found in the GitHub archive"
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

fn determine_parent_folder(folders: &[String], repo_name: &str) -> Result<String, String> {
    if folders.is_empty() {
        return Err("no addon folders were discovered in the GitHub archive".to_string());
    }
    if folders.len() == 1 {
        return Ok(folders[0].clone());
    }

    let normalized_repo = normalize_folder_like(repo_name);
    if let Some(exact) = folders
        .iter()
        .find(|folder| normalize_folder_like(folder) == normalized_repo)
    {
        return Ok(exact.clone());
    }

    let mut sorted = folders.to_vec();
    sorted.sort_by_key(|folder| folder.len());
    let shortest = sorted[0].clone();
    let shortest_normalized = normalize_folder_like(&shortest);
    let prefix_matches = sorted
        .iter()
        .filter(|folder| normalize_folder_like(folder).starts_with(&shortest_normalized))
        .count();
    if prefix_matches * 2 >= sorted.len() {
        return Ok(shortest);
    }

    let mut candidates = folders
        .iter()
        .filter(|folder| {
            let normalized = normalize_folder_like(folder);
            normalized_repo.contains(&normalized) || normalized.contains(&normalized_repo)
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by_key(|folder| folder.len());
    if let Some(candidate) = candidates.first() {
        return Ok(candidate.clone());
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

fn install_version(parent_version: Option<&str>, commit: &str) -> Option<String> {
    parent_version
        .filter(|value| !is_placeholder_version(value))
        .map(ToOwned::to_owned)
        .or_else(|| Some(short_commit(commit)))
}

fn is_placeholder_version(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('@') && trimmed.ends_with('@')
}

pub(crate) fn commits_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left = normalize_commit(left);
            let right = normalize_commit(right);
            !left.is_empty()
                && !right.is_empty()
                && (left == right || left.starts_with(&right) || right.starts_with(&left))
        }
        _ => false,
    }
}

fn normalize_commit(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(7).collect()
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

    Err("failed to create a unique temporary GitHub work directory".to_string())
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
        GithubRepoResponse, build_managed_github_record, commits_match, determine_parent_folder,
        parse_github_repo_url, parse_github_target, update_downloaded_github_addon,
    };
    use lemonup_core::{
        AddonKind, AddonRecord, GameFlavor, ScannedAddon, SourceKind, StateDatabase,
    };
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
    fn parse_github_target_accepts_canonical_repo_urls() {
        assert_eq!(
            parse_github_target("https://github.com/WeakAuras/WeakAuras2").expect("repo url"),
            "https://github.com/WeakAuras/WeakAuras2"
        );
        assert_eq!(
            parse_github_target("https://github.com/WeakAuras/WeakAuras2.git").expect("repo url"),
            "https://github.com/WeakAuras/WeakAuras2"
        );
    }

    #[test]
    fn parse_github_target_rejects_non_root_or_non_github_urls() {
        assert!(parse_github_target("https://example.com/owner/repo").is_err());
        assert!(parse_github_target("https://github.com/owner/repo/tree/main").is_err());
        assert!(parse_github_target("owner/repo").is_err());
    }

    #[test]
    fn parse_github_repo_url_returns_owner_and_repo() {
        let parsed =
            parse_github_repo_url("https://github.com/AdiAddons/AdiBags").expect("github repo");
        assert_eq!(parsed.owner, "AdiAddons");
        assert_eq!(parsed.repo, "AdiBags");
    }

    #[test]
    fn github_repo_response_deserializes() {
        let parsed = serde_json::from_str::<GithubRepoResponse>(
                            r#"{"name":"AdiBags","default_branch":"main","html_url":"https://github.com/AdiAddons/AdiBags","owner":{"login":"AdiAddons"}}"#,
                                    )
                                            .expect("repo response");
        assert_eq!(parsed.name, "AdiBags");
        assert_eq!(parsed.default_branch, "main");
        assert_eq!(parsed.owner.login, "AdiAddons");
        assert_eq!(parsed.html_url, "https://github.com/AdiAddons/AdiBags");
    }

    #[test]
    fn determine_parent_folder_prefers_repo_name_match() {
        let folders = vec!["AdiBags".to_string(), "AdiBags_Config".to_string()];
        assert_eq!(
            determine_parent_folder(&folders, "AdiBags").expect("parent"),
            "AdiBags"
        );
    }

    #[test]
    fn build_managed_github_record_sets_commit_and_owned_folders() {
        let resolved = super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: "AdiAddons".to_string(),
                repo: "AdiBags".to_string(),
                default_branch: "main".to_string(),
                html_url: "https://github.com/AdiAddons/AdiBags".to_string(),
                owner_login: "AdiAddons".to_string(),
            },
            head_sha: "abcdef0123456789".to_string(),
        };
        let scanned = ScannedAddon {
            name: "AdiBags".to_string(),
            folder: "AdiBags".to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: Some("1.0.0".to_string()),
            git_commit: None,
            author: None,
            interface: Some("110205".to_string()),
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
        };

        let record = build_managed_github_record(
            &resolved,
            &scanned,
            &["AdiBags".to_string(), "AdiBags_Config".to_string()],
        );
        assert_eq!(record.source, SourceKind::GitHub);
        assert_eq!(record.git_commit.as_deref(), Some("abcdef0123456789"));
        assert_eq!(record.remote_version.as_deref(), Some("abcdef0123456789"));
        assert_eq!(record.owned_folders.len(), 1);
        assert_eq!(record.owned_folders[0].name, "AdiBags_Config");
    }

    #[test]
    fn build_managed_github_record_uses_short_commit_when_version_is_placeholder() {
        let resolved = super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: "WeakAuras".to_string(),
                repo: "WeakAuras2".to_string(),
                default_branch: "main".to_string(),
                html_url: "https://github.com/WeakAuras/WeakAuras2".to_string(),
                owner_login: "WeakAuras".to_string(),
            },
            head_sha: "364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string(),
        };
        let scanned = ScannedAddon {
            name: "WeakAuras".to_string(),
            folder: "WeakAuras".to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: Some("@project-version@".to_string()),
            git_commit: None,
            author: None,
            interface: Some("110205".to_string()),
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
        };

        let record = build_managed_github_record(&resolved, &scanned, &["WeakAuras".to_string()]);
        assert_eq!(record.version.as_deref(), Some("364f625"));
        assert_eq!(
            record.git_commit.as_deref(),
            Some("364f625cf8c4f2f1c0785ab12da2121e880ec560")
        );
    }

    #[test]
    fn commits_match_accepts_short_and_full_hashes() {
        assert!(commits_match(
            Some("abcdef0"),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        ));
        assert!(commits_match(
            Some("abcdef0123456789abcdef0123456789abcdef01"),
            Some("abcdef0")
        ));
        assert!(!commits_match(Some("abcdef0"), Some("1234567")));
    }

    #[test]
    fn update_downloaded_github_addon_preserves_parent_folder_in_dry_run() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags")).expect("create addon dir");
        std::fs::write(
            addon_dir.join("AdiBags").join("AdiBags.toc"),
            "## Title: AdiBags\n## Version: 1.0.0\n## Interface: 110205\n",
        )
        .expect("write toc");

        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/AdiAddons/AdiBags".to_string());
        database.upsert_addon(&tracked).expect("seed addon");

        let resolved = super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: "AdiAddons".to_string(),
                repo: "AdiBags".to_string(),
                default_branch: "main".to_string(),
                html_url: "https://github.com/AdiAddons/AdiBags".to_string(),
                owner_login: "AdiAddons".to_string(),
            },
            head_sha: "abcdef0123456789".to_string(),
        };
        let bytes = build_zip(&[(
            "wrapper/AdiBags/AdiBags.toc",
            "## Title: AdiBags\n## Version: 1.1.0\n## Interface: 110205\n",
        )]);

        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved,
            &bytes,
            true,
        )
        .expect("dry-run update");

        assert!(summary.updated);
        assert_eq!(summary.parent_folder, "AdiBags");
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.git_commit.as_deref(), Some("abcdef0"));
    }
}
