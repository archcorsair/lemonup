use crate::update::PreparedUpdate;
use crate::update_targets::ReplaceNotLinked;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::source_error::{SourceError, SourceErrorKind};
use lemonup_core::github_release::{
    ReleaseAsset, ReleaseJson, choose_release_asset, release_json_asset,
};
use lemonup_core::known_addons::{KnownAddon, KnownSource, find_known_github_repo, known_addons};
use lemonup_core::{
    AddonRecord, GameFlavor, OwnedFolder, ScannedAddon, SourceKind, StateDatabase, scan_addons_dir,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode, Url};
use serde::Deserialize;
use time::OffsetDateTime;
use zip::ZipArchive;

use crate::archive::safe_entry_relative_path;

const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubInstallSummary {
    pub(crate) repo_url: String,
    pub(crate) repo_name: String,
    pub(crate) parent_folder: String,
    pub(crate) installed_folders: Vec<String>,
    pub(crate) version: Option<String>,
    /// The source zipball's commit; `None` for a release zip.
    pub(crate) git_commit: Option<String>,
    /// The release whose zip was installed; `None` for the source zipball.
    pub(crate) release_tag: Option<String>,
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
    /// "Not linked" folders the new package would replace that nobody agreed to. While this is
    /// not empty nothing was updated.
    pub(crate) not_linked: Vec<String>,
}

impl GithubUpdateSummary {
    fn finished(self) -> crate::update::FinishedUpdate {
        crate::update::FinishedUpdate {
            previous_version: self.previous_version,
            remote_version: self.remote_version,
            not_linked: self.not_linked,
            warning: None,
        }
    }

    pub(crate) fn outcome(self) -> crate::update::ProviderOutcome {
        crate::update::ProviderOutcome {
            updated: self.updated,
            remote_older: false,
            finished: self.finished(),
        }
    }
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
    /// GitHub's numeric id for the repository.
    id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubResolvedRepo {
    metadata: GithubRepoMetadata,
    package: GithubPackage,
}

/// What a GitHub install or update downloads.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GithubPackage {
    /// A zip of the repo's latest release, made for the game (see
    /// `lemonup_core::github_release`).
    Release { tag: String, asset: ReleaseAsset },
    /// The source zipball of the default branch's head commit, when no release zip fits.
    Source { head_sha: String },
}

impl GithubPackage {
    /// What a check stores as the remote version: the release tag, or the commit.
    fn remote_version(&self) -> &str {
        match self {
            Self::Release { tag, .. } => tag,
            Self::Source { head_sha } => head_sha,
        }
    }

    fn git_commit(&self) -> Option<&str> {
        match self {
            Self::Release { .. } => None,
            Self::Source { head_sha } => Some(head_sha),
        }
    }

    fn release_tag(&self) -> Option<&str> {
        match self {
            Self::Release { tag, .. } => Some(tag),
            Self::Source { .. } => None,
        }
    }

    /// Whether `tracked` already has this package: the same release, or (for a source
    /// install) the same commit. A source install never matches a release, so the next update
    /// moves it to the release zip even when the tag points at the installed commit.
    fn is_installed(&self, tracked: &AddonRecord) -> bool {
        match (self, tracked.installed_release_tag()) {
            (Self::Release { tag, .. }, Some(installed)) => installed.trim() == tag.trim(),
            (Self::Release { .. }, None) | (Self::Source { .. }, Some(_)) => false,
            (Self::Source { head_sha }, None) => commits_match(
                tracked.git_commit.as_deref().or(tracked.version.as_deref()),
                Some(head_sha),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubOwnerResponse {
    login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubRepoResponse {
    id: u64,
    name: String,
    default_branch: String,
    html_url: String,
    owner: GithubOwnerResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubCommitResponse {
    sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubReleaseAssetResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubReleaseAssetResponse {
    name: String,
    browser_download_url: String,
}

impl GithubReleaseResponse {
    /// Drafts and prereleases are never the latest release; skipped all the same.
    fn is_installable(&self) -> bool {
        !self.draft && !self.prerelease && !self.tag_name.trim().is_empty()
    }

    /// The release's files whose download starts on github.com, so the GitHub token is only
    /// ever sent there (reqwest drops it on the redirect to GitHub's file host).
    fn assets(&self) -> Vec<ReleaseAsset> {
        self.assets
            .iter()
            .filter(|asset| is_github_download_url(&asset.browser_download_url))
            .map(|asset| ReleaseAsset {
                name: asset.name.clone(),
                download_url: asset.browser_download_url.clone(),
            })
            .collect()
    }
}

fn is_github_download_url(value: &str) -> bool {
    Url::parse(value)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some("github.com"))
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
    install_github_addon_checked(state_database, addon_dir, target, dry_run, &[])
        .await
        .map_err(String::from)
}

/// `install_github_addon` with the error kind kept. `replace` is the list of
/// folders the person agreed to replace: those of them already on disk are
/// taken over (moved aside and restored on failure, like an update). Any other
/// folder in the way refuses the install (see `plan_install_replacements`).
pub(crate) async fn install_github_addon_checked(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    target: &str,
    dry_run: bool,
    replace: &[String],
) -> Result<GithubInstallSummary, SourceError> {
    let repo = parse_github_repo_url(target)?;
    let client = build_github_client()?;
    let resolved = fetch_github_repo(&client, &repo, GameFlavor::for_addons_dir(addon_dir)).await?;
    let bytes = download_github_package_bytes(&client, &resolved).await?;
    install_downloaded_github_addon(
        state_database,
        addon_dir,
        &resolved,
        &bytes,
        dry_run,
        replace,
    )
}

/// The version an update for `flavor` would install: the latest release's tag when it has a zip
/// for the game, otherwise the default branch's head commit.
pub(crate) async fn fetch_github_remote_version(
    client: &Client,
    addon: &AddonRecord,
    flavor: GameFlavor,
) -> Result<GithubRemoteVersion, SourceError> {
    let repo = resolve_tracked_github_repo(addon)?;
    // A listed repository is checked before its version counts, so the list
    // never says Update ready for one the update would refuse.
    let checked = if needs_known_repository_check(known_addons(), &repo.owner, &repo.repo) {
        let metadata = fetch_github_repo_metadata(client, &repo).await?;
        check_known_repository(
            known_addons(),
            &repo.owner,
            &repo.repo,
            metadata.id,
            &metadata.owner_login,
        )?;
        Some(metadata)
    } else {
        None
    };
    if let Some(package) = fetch_github_release_package(client, &repo, flavor).await? {
        return Ok(GithubRemoteVersion {
            source_url: Some(canonical_github_url(&repo.owner, &repo.repo)),
            version: Some(package.remote_version().to_string()),
        });
    }
    let metadata = match checked {
        Some(metadata) => metadata,
        None => fetch_github_repo_metadata(client, &repo).await?,
    };
    let head_sha = fetch_github_head_commit(client, &metadata).await?;
    Ok(GithubRemoteVersion {
        source_url: Some(metadata.html_url),
        version: Some(head_sha),
    })
}

/// The network part of a GitHub update: what the repo offers for the AddOns folder's
/// `flavor`, then the package unless it is already installed. Touches neither the database
/// nor the AddOns folder.
pub(crate) async fn prepare_github_update(
    client: Client,
    tracked: AddonRecord,
    flavor: GameFlavor,
    force: bool,
    dry_run: bool,
) -> Result<PreparedUpdate, String> {
    if tracked.source != SourceKind::GitHub {
        return Err(format!(
            "addon '{}' is not linked to GitHub",
            tracked.folder
        ));
    }

    let repo = resolve_tracked_github_repo(&tracked)?;
    let resolved = fetch_github_repo(&client, &repo, flavor).await?;
    let remote_version = Some(resolved.package.remote_version().to_string());

    if !force && resolved.package.is_installed(&tracked) {
        let summary = GithubUpdateSummary {
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
            not_linked: Vec::new(),
        };
        return Ok(PreparedUpdate::Skipped {
            outcome: summary.outcome(),
            source_url: Some(resolved.metadata.html_url),
            // GitHub gives only a repository name, so GitHub addons keep their TOC Title.
            source_name: None,
        });
    }

    let bytes = download_github_package_bytes(&client, &resolved).await?;
    Ok(PreparedUpdate::downloaded(
        bytes,
        move |database, addon_dir, tracked, bytes, agree, dry_run| {
            update_downloaded_github_addon(
                database, addon_dir, tracked, &resolved, bytes, agree, dry_run,
            )
            .map(GithubUpdateSummary::outcome)
        },
    ))
}

const GITHUB_TOKEN_ENV_VARS: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];

/// Where the GitHub token in use comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GithubTokenSource {
    /// Saved in Settings (`shared.toml`, shared by all games).
    Settings,
    /// The environment variable it was read from: `GITHUB_TOKEN` or `GH_TOKEN`.
    Environment(&'static str),
}

/// A GitHub token and where it comes from. Debug output never shows the token.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct GithubToken {
    value: String,
    pub(crate) source: GithubTokenSource,
}

impl std::fmt::Debug for GithubToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GithubToken")
            .field("value", &"<hidden>")
            .field("source", &self.source)
            .finish()
    }
}

/// The token to use: the one saved in Settings, else `GITHUB_TOKEN`, then `GH_TOKEN`. Values
/// are trimmed and empty or whitespace-only ones are skipped.
fn resolve_github_token(
    saved: Option<&str>,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<GithubToken> {
    let saved = saved.map(str::trim).filter(|token| !token.is_empty());
    if let Some(saved) = saved {
        return Some(GithubToken {
            value: saved.to_string(),
            source: GithubTokenSource::Settings,
        });
    }
    GITHUB_TOKEN_ENV_VARS.iter().find_map(|name| {
        let value = lookup(name)?;
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| GithubToken {
            value: trimmed.to_string(),
            source: GithubTokenSource::Environment(name),
        })
    })
}

/// The variable a GitHub token comes from when none is saved in Settings, as `lookup` sees
/// the environment.
pub(crate) fn environment_github_token_var(
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<&'static str> {
    match resolve_github_token(None, lookup)?.source {
        GithubTokenSource::Environment(name) => Some(name),
        GithubTokenSource::Settings => None,
    }
}

/// Where the token in use comes from, given whether one is saved in Settings and the
/// variable a token comes from otherwise.
pub(crate) fn github_token_source_of(
    saved: Option<&str>,
    environment_var: Option<&'static str>,
) -> Option<GithubTokenSource> {
    if saved.is_some_and(|token| !token.trim().is_empty()) {
        Some(GithubTokenSource::Settings)
    } else {
        environment_var.map(GithubTokenSource::Environment)
    }
}

/// The token saved in Settings, for every GitHub client this process builds: the CLI sets it
/// once after it reads the settings, the TUI whenever its settings are loaded or saved, so a
/// change in Settings counts from the next check on.
#[cfg(not(test))]
static SAVED_GITHUB_TOKEN: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

// Each test thread has its own, so a test that saves a token never changes what another
// test sends.
#[cfg(test)]
thread_local! {
    static SAVED_GITHUB_TOKEN: std::sync::RwLock<Option<String>> =
        const { std::sync::RwLock::new(None) };
}

#[cfg(not(test))]
fn with_saved_github_token<R>(use_it: impl FnOnce(&std::sync::RwLock<Option<String>>) -> R) -> R {
    use_it(&SAVED_GITHUB_TOKEN)
}

#[cfg(test)]
fn with_saved_github_token<R>(use_it: impl FnOnce(&std::sync::RwLock<Option<String>>) -> R) -> R {
    SAVED_GITHUB_TOKEN.with(use_it)
}

/// Makes `token` the one saved in Settings for the GitHub clients built from now on.
pub(crate) fn use_saved_github_token(token: Option<&str>) {
    let token = token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    with_saved_github_token(|saved| {
        *saved
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = token;
    });
}

fn saved_github_token() -> Option<String> {
    with_saved_github_token(|saved| {
        saved
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    })
}

/// The token every GitHub request sends now, if any.
fn github_token_in_use() -> Option<GithubToken> {
    resolve_github_token(saved_github_token().as_deref(), |name| {
        std::env::var(name).ok()
    })
}

/// Where the token every GitHub request sends now comes from, if one is sent.
pub(crate) fn github_token_source() -> Option<GithubTokenSource> {
    github_token_in_use().map(|token| token.source)
}

pub(crate) fn build_github_client() -> Result<Client, String> {
    build_github_client_with_token(
        github_token_in_use()
            .as_ref()
            .map(|token| token.value.as_str()),
    )
}

/// Whether `token` can go in the header every GitHub request sends it in, as plain visible
/// ASCII like every token GitHub makes. A control character fails the client itself; a
/// character a copy can bring along, such as a smart quote or a zero-width space, would go
/// out and be turned down. Asks nothing of GitHub.
pub(crate) fn github_token_can_be_sent(token: &str) -> bool {
    HeaderValue::from_str(&format!("Bearer {token}")).is_ok_and(|value| value.to_str().is_ok())
}

fn build_github_client_with_token(token: Option<&str>) -> Result<Client, String> {
    let mut builder = crate::http::client_builder();

    if let Some(token) = token {
        // Never include the token in the error: it is not a valid header value.
        let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
            "GitHub token contains characters not allowed in HTTP headers".to_string()
        })?;
        // Marks the value as sensitive so it is redacted from `Debug` output and
        // is not kept in HTTP/2 header compression tables.
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }

    builder.build().map_err(|error| error.to_string())
}

fn unix_now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// Returns a descriptive error when a response is a GitHub API rate-limit
/// rejection (403/429 with `x-ratelimit-remaining: 0`, or 403/429 with `retry-after`,
/// which is how GitHub sends a secondary rate limit).
/// The hint to add a token is left off when `token_sent`: GitHub's higher limit is used up.
fn classify_rate_limit(
    status: StatusCode,
    headers: &HeaderMap,
    now_unix: i64,
    token_sent: bool,
) -> Option<String> {
    if status != StatusCode::FORBIDDEN && status != StatusCode::TOO_MANY_REQUESTS {
        return None;
    }

    let header_str = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let header_i64 =
        |name: &str| header_str(name).and_then(|value| value.trim().parse::<i64>().ok());

    let remaining_zero =
        header_str("x-ratelimit-remaining").is_some_and(|value| value.trim() == "0");
    let retry_after = header_i64("retry-after");
    let throttled = retry_after.is_some();
    if !(remaining_zero || throttled) {
        return None;
    }

    let reset_unix = header_i64("x-ratelimit-reset")
        .or_else(|| retry_after.map(|seconds| now_unix.saturating_add(seconds.max(0))));
    let reset_note = reset_unix
        .map(|reset| format!(" and {}", describe_reset(reset, now_unix)))
        .unwrap_or_default();

    let hint = if token_sent {
        ""
    } else {
        "; add a GitHub token in Settings or set GITHUB_TOKEN to raise the limit"
    };
    Some(format!(
        "GitHub API rate limit reached (HTTP {}){reset_note}{hint}",
        status.as_u16()
    ))
}

fn describe_reset(reset_unix: i64, now_unix: i64) -> String {
    let remaining_secs = reset_unix.saturating_sub(now_unix);
    if remaining_secs <= 0 {
        return "resets momentarily".to_string();
    }
    let minutes = (remaining_secs + 59) / 60;
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    match OffsetDateTime::from_unix_timestamp(reset_unix) {
        Ok(at) => format!(
            "resets in {minutes} {unit} (at {:02}:{:02} UTC)",
            at.hour(),
            at.minute()
        ),
        Err(_) => format!("resets in {minutes} {unit}"),
    }
}

/// A 401 while a token was sent means GitHub rejected that token. Names where the token came
/// from (`sent`), never its value.
fn classify_rejected_token(status: StatusCode, sent: Option<GithubTokenSource>) -> Option<String> {
    let source = sent.filter(|_| status == StatusCode::UNAUTHORIZED)?;
    Some(match source {
        GithubTokenSource::Settings => "GitHub rejected the GitHub token saved in Settings (HTTP \
                                        401); check it in Settings"
            .to_string(),
        GithubTokenSource::Environment(name) => format!(
            "GitHub rejected the token from {name} (HTTP 401); unset it or replace it with a \
             valid token"
        ),
    })
}

/// Maps a non-success response to an error, with a dedicated message for rate limits. The
/// token named is the one in use now. That is usually the one the client was built with,
/// but a token saved in Settings while the request was out makes them differ.
pub(crate) fn ensure_github_success(response: Response) -> Result<Response, SourceError> {
    let sent = github_token_source();
    if let Some(message) = classify_rejected_token(response.status(), sent) {
        return Err(SourceError::new(SourceErrorKind::KeyRejected, message));
    }
    if let Some(message) = classify_rate_limit(
        response.status(),
        response.headers(),
        unix_now(),
        sent.is_some(),
    ) {
        return Err(SourceError::new(SourceErrorKind::SlowDown, message));
    }
    response.error_for_status().map_err(|error| {
        let mut error = SourceError::from_reqwest(error);
        // GitHub only rejects a token as a 401, handled above.
        if error.kind == SourceErrorKind::KeyRejected {
            error.kind = SourceErrorKind::Other;
        }
        error
    })
}

/// The repo and the package to install for `flavor`: a release zip when one fits, otherwise
/// the source zipball of the head commit.
async fn fetch_github_repo(
    client: &Client,
    repo: &GithubRepoRef,
    flavor: GameFlavor,
) -> Result<GithubResolvedRepo, SourceError> {
    let metadata = fetch_github_repo_metadata(client, repo).await?;
    check_known_repository(
        known_addons(),
        &repo.owner,
        &repo.repo,
        metadata.id,
        &metadata.owner_login,
    )?;
    let release_repo = GithubRepoRef {
        owner: metadata.owner.clone(),
        repo: metadata.repo.clone(),
    };
    let package = match fetch_github_release_package(client, &release_repo, flavor).await? {
        Some(package) => package,
        None => GithubPackage::Source {
            head_sha: fetch_github_head_commit(client, &metadata).await?,
        },
    };
    Ok(GithubResolvedRepo { metadata, package })
}

/// Whether `owner/repo` is on the known addons list, so GitHub's metadata for it has to be read
/// and checked before anything else about it counts.
fn needs_known_repository_check(list: &[KnownAddon], owner: &str, repo: &str) -> bool {
    find_known_github_repo(list, owner, repo).is_some()
}

/// Refuses a repository the known addons list names unless GitHub reports the listed repository
/// id for it and the listed owner (ignoring case), so a repository that changed owners is not
/// installed or updated: a transferred repository keeps its id, and GitHub sends the old path to
/// the new owner. `owner` and `repo` are the ones asked for, `owner_login` the owner GitHub
/// reports. A repository the list does not name passes.
fn check_known_repository(
    list: &[KnownAddon],
    owner: &str,
    repo: &str,
    id: u64,
    owner_login: &str,
) -> Result<(), SourceError> {
    let Some(known) = find_known_github_repo(list, owner, repo) else {
        return Ok(());
    };
    match &known.source {
        KnownSource::GitHub {
            id: known_id,
            owner: known_owner,
            ..
        } if *known_id != id || !known_owner.eq_ignore_ascii_case(owner_login) => {
            Err(SourceError::new(
                SourceErrorKind::WrongRepository,
                format!(
                    "github.com/{owner}/{repo} is not the repository LemonUp lists for {}",
                    known.name
                ),
            ))
        }
        _ => Ok(()),
    }
}

/// The latest release's zip for `flavor`, or `None` when the repo has no release (drafts and
/// prereleases are never the latest) or none of its zips fits the game.
async fn fetch_github_release_package(
    client: &Client,
    repo: &GithubRepoRef,
    flavor: GameFlavor,
) -> Result<Option<GithubPackage>, SourceError> {
    let url = format!(
        "{API_BASE}/repos/{}/{}/releases/latest",
        repo.owner, repo.repo
    );
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let release = ensure_github_success(response)?
        .json::<GithubReleaseResponse>()
        .await
        .map_err(SourceError::from_reqwest)?;
    if !release.is_installable() {
        return Ok(None);
    }
    let assets = release.assets();
    let release_json = match release_json_asset(&assets) {
        Some(asset) => read_release_json(download_github_asset_bytes(client, asset).await)?,
        None => None,
    };
    Ok(release_package(
        &release,
        &assets,
        flavor,
        release_json.as_ref(),
    ))
}

/// The release's `release.json` from its download. A failed download fails the check or update,
/// so a network error never picks another package (the source zip, say) than the metadata would;
/// the installed copy stays as it is until the metadata can be read. A file that isn't in the
/// packager's format counts as none.
fn read_release_json(
    download: Result<Vec<u8>, SourceError>,
) -> Result<Option<ReleaseJson>, SourceError> {
    Ok(ReleaseJson::parse(&String::from_utf8_lossy(&download?)))
}

/// The package `release` offers for `flavor`, given its files and its `release.json`.
fn release_package(
    release: &GithubReleaseResponse,
    assets: &[ReleaseAsset],
    flavor: GameFlavor,
    release_json: Option<&ReleaseJson>,
) -> Option<GithubPackage> {
    if !release.is_installable() {
        return None;
    }
    choose_release_asset(flavor, assets, release_json).map(|asset| GithubPackage::Release {
        tag: release.tag_name.trim().to_string(),
        asset: asset.clone(),
    })
}

async fn download_github_asset_bytes(
    client: &Client,
    asset: &ReleaseAsset,
) -> Result<Vec<u8>, SourceError> {
    let response = client
        .get(&asset.download_url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;
    ensure_github_success(response)?
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(SourceError::from_reqwest)
}

async fn fetch_github_repo_metadata(
    client: &Client,
    repo: &GithubRepoRef,
) -> Result<GithubRepoMetadata, SourceError> {
    let url = format!("{API_BASE}/repos/{}/{}", repo.owner, repo.repo);
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(SourceError::new(
            SourceErrorKind::NotFound,
            format!("GitHub repo '{}/{}' was not found", repo.owner, repo.repo),
        ));
    }

    let response = ensure_github_success(response)?;
    let parsed = response
        .json::<GithubRepoResponse>()
        .await
        .map_err(SourceError::from_reqwest)?;

    Ok(GithubRepoMetadata {
        owner: repo.owner.clone(),
        repo: parsed.name,
        default_branch: parsed.default_branch,
        html_url: canonical_github_url(&repo.owner, &repo.repo),
        owner_login: parsed.owner.login,
        id: parsed.id,
    })
}

async fn fetch_github_head_commit(
    client: &Client,
    metadata: &GithubRepoMetadata,
) -> Result<String, SourceError> {
    let url = format!(
        "{API_BASE}/repos/{}/{}/commits/{}",
        metadata.owner, metadata.repo, metadata.default_branch
    );
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(SourceError::new(
            SourceErrorKind::NotFound,
            format!(
                "GitHub repo '{}/{}' is missing default branch '{}'",
                metadata.owner, metadata.repo, metadata.default_branch
            ),
        ));
    }

    let response = ensure_github_success(response)?;
    let parsed = response
        .json::<GithubCommitResponse>()
        .await
        .map_err(SourceError::from_reqwest)?;
    Ok(parsed.sha)
}

async fn download_github_package_bytes(
    client: &Client,
    resolved: &GithubResolvedRepo,
) -> Result<Vec<u8>, SourceError> {
    let head_sha = match &resolved.package {
        GithubPackage::Release { asset, .. } => {
            return download_github_asset_bytes(client, asset).await;
        }
        GithubPackage::Source { head_sha } => head_sha,
    };
    let url = format!(
        "{API_BASE}/repos/{}/{}/zipball/{head_sha}",
        resolved.metadata.owner, resolved.metadata.repo
    );
    let response = client
        .get(url)
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;
    ensure_github_success(response)?
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(SourceError::from_reqwest)
}

fn install_downloaded_github_addon(
    state_database: &mut StateDatabase,
    addon_dir: &Path,
    resolved: &GithubResolvedRepo,
    archive_bytes: &[u8],
    dry_run: bool,
    replace: &[String],
) -> Result<GithubInstallSummary, SourceError> {
    let temp_root = create_temp_work_dir("github-install")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let outcome = (|| -> Result<GithubInstallSummary, SourceError> {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root, &resolved.metadata.repo)?;
        let parent_folder = extracted.parent_folder(&resolved.metadata.repo)?;
        let scanned = scan_addons_dir(
            &extracted.source_root,
            GameFlavor::for_addons_dir(addon_dir),
        )
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

        // Planned before a dry run returns as well, so it never says an install would work
        // that is then refused. This only reads the list and the AddOns folder.
        // Only folders on `replace` that are not linked are taken over.
        let replaced = crate::update_targets::plan_install_replacements(
            state_database,
            addon_dir,
            &extracted.folders,
            &parent_folder,
            replace,
            None,
        )?;
        if dry_run {
            return Ok(GithubInstallSummary {
                repo_url: resolved.metadata.html_url.clone(),
                repo_name: resolved.metadata.repo.clone(),
                parent_folder,
                installed_folders: extracted.folders,
                version: install_version(parent_metadata.version.as_deref(), &resolved.package),
                git_commit: resolved.package.git_commit().map(ToOwned::to_owned),
                release_tag: resolved.package.release_tag().map(ToOwned::to_owned),
                dry_run: true,
            });
        }
        let record = build_managed_github_record(resolved, parent_metadata, &extracted.folders);
        // The user's TOC overrides go into the staged copies before anything live moves.
        let toc_overrides = crate::update_targets::stored_toc_overrides(state_database, &record)?;
        let swap = crate::update_targets::swap_with_toc_overrides(
            addon_dir,
            &extracted.source_root,
            &extracted.folders,
            &replaced,
            &toc_overrides,
        )?;

        if let Err(error) = state_database.record_managed_addon(&record) {
            return Err(swap.abort(error.to_string()).into());
        }
        swap.commit();

        Ok(GithubInstallSummary {
            repo_url: resolved.metadata.html_url.clone(),
            repo_name: resolved.metadata.repo.clone(),
            parent_folder,
            installed_folders: extracted.folders,
            version: install_version(parent_metadata.version.as_deref(), &resolved.package),
            git_commit: resolved.package.git_commit().map(ToOwned::to_owned),
            release_tag: resolved.package.release_tag().map(ToOwned::to_owned),
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
    agree: ReplaceNotLinked<'_>,
    dry_run: bool,
) -> Result<GithubUpdateSummary, String> {
    let temp_root = create_temp_work_dir("github-update")?;
    let zip_path = temp_root.join("package.zip");
    let extract_root = temp_root.join("extract");
    let previous_version = tracked.version.clone();
    let previous_commit = tracked.git_commit.clone();
    let outcome = (|| {
        fs::write(&zip_path, archive_bytes).map_err(|error| error.to_string())?;
        extract_zip_archive(&zip_path, &extract_root)?;

        let extracted = discover_extracted_folders(&extract_root, &resolved.metadata.repo)?;
        let resolved_parent = extracted.parent_folder(&resolved.metadata.repo)?;
        if resolved_parent != tracked.folder {
            return Err(format!(
                "GitHub update for '{}' resolved parent '{}' but linked addon folder is '{}'",
                resolved.metadata.repo, resolved_parent, tracked.folder
            ));
        }

        let scanned = scan_addons_dir(
            &extracted.source_root,
            GameFlavor::for_addons_dir(addon_dir),
        )
        .map_err(|error| error.to_string())?;
        let parent_metadata = scanned
            .iter()
            .find(|candidate| candidate.folder == tracked.folder)
            .ok_or_else(|| {
                format!(
                    "updated GitHub repo '{}' did not yield a scannable linked parent folder '{}'",
                    resolved.metadata.repo, tracked.folder
                )
            })?;

        // Parent plus owned folders (authoritative ownership only) and any package folders
        // agreed to take over from manual rows; refuses folders of other tracked addons.
        let plan = crate::update_targets::plan_update_replacements(
            state_database,
            addon_dir,
            tracked,
            &extracted.folders,
            agree,
        )?;
        let managed_folders = plan.replaced;
        // A package that would replace "Not linked" folders nobody agreed to changes nothing.
        let held = !plan.not_linked.is_empty();

        if dry_run || held {
            return Ok(GithubUpdateSummary {
                repo_url: resolved.metadata.html_url.clone(),
                repo_name: resolved.metadata.repo.clone(),
                parent_folder: tracked.folder.clone(),
                installed_folders: extracted.folders,
                previous_version,
                previous_commit,
                remote_version: Some(resolved.package.remote_version().to_string()),
                updated: !held,
                dry_run,
                not_linked: plan.not_linked,
            });
        }

        let record = build_managed_github_record(resolved, parent_metadata, &extracted.folders);
        // Stage, move aside and place by same-volume rename; any failure restores the old
        // folders before the error is returned.
        // The user's TOC overrides go into the staged copies before anything live moves.
        let toc_overrides = crate::update_targets::stored_toc_overrides(state_database, &record)?;
        let swap = crate::update_targets::swap_with_toc_overrides(
            addon_dir,
            &extracted.source_root,
            &extracted.folders,
            &managed_folders,
            &toc_overrides,
        )?;

        if let Err(error) = state_database.record_managed_addon(&record) {
            return Err(swap.abort(error.to_string()));
        }
        swap.commit();

        Ok(GithubUpdateSummary {
            repo_url: resolved.metadata.html_url.clone(),
            repo_name: resolved.metadata.repo.clone(),
            parent_folder: tracked.folder.clone(),
            installed_folders: extracted.folders,
            previous_version,
            previous_commit,
            remote_version: Some(resolved.package.remote_version().to_string()),
            updated: true,
            dry_run: false,
            not_linked: Vec::new(),
        })
    })();

    let _ = fs::remove_dir_all(&temp_root);
    outcome
}

#[derive(Debug, Clone)]
struct ExtractedFolders {
    source_root: PathBuf,
    folders: Vec<String>,
    /// The parent addon when the layout itself says which it is (a root-TOC repo); otherwise
    /// [`determine_parent_folder`] picks one from `folders`.
    parent: Option<String>,
}

impl ExtractedFolders {
    fn parent_folder(&self, repo_name: &str) -> Result<String, String> {
        match &self.parent {
            Some(parent) => Ok(parent.clone()),
            None => determine_parent_folder(&self.folders, repo_name),
        }
    }
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
    record.version = install_version(parent_metadata.version.as_deref(), &resolved.package);
    record.git_commit = resolved.package.git_commit().map(ToOwned::to_owned);
    record.release_tag = resolved.package.release_tag().map(ToOwned::to_owned);
    record.author = parent_metadata
        .author
        .clone()
        .or_else(|| Some(resolved.metadata.owner_login.clone()));
    record.interface = parent_metadata.interface.clone();
    record.source_url = Some(resolved.metadata.html_url.clone());
    record.required_deps = parent_metadata.required_deps.clone();
    record.optional_deps = parent_metadata.optional_deps.clone();
    record.embedded_libs = parent_metadata.embedded_libs.clone();
    record.remote_version = Some(resolved.package.remote_version().to_string());
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
        .ok_or_else(|| format!("linked GitHub addon '{}' has no source URL", addon.folder))
        .and_then(parse_github_repo_url)
}

/// `repo` without any `.git` ending, in any case: GitHub ignores case in repository names, so
/// `Foo.GIT` is the repository `Foo` and must meet the known addons check like `Foo` does.
fn strip_git_suffix(mut repo: &str) -> &str {
    while let Some(rest) = repo
        .len()
        .checked_sub(4)
        .filter(|&at| repo.is_char_boundary(at) && repo[at..].eq_ignore_ascii_case(".git"))
        .map(|at| &repo[..at])
    {
        repo = rest;
    }
    repo
}

/// Whether `url` is a GitHub link to `owner/repo`, read the way updates read a linked addon's
/// link (`www.`, a `.git` ending and a trailing slash allowed), ignoring ASCII case.
pub(crate) fn github_url_is_repo(url: &str, owner: &str, repo: &str) -> bool {
    parse_github_repo_url(url.trim()).is_ok_and(|parsed| {
        parsed.owner.eq_ignore_ascii_case(owner) && parsed.repo.eq_ignore_ascii_case(repo)
    })
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
    let repo = strip_git_suffix(segments[1]).trim();
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

/// Finds the addon folders in an extracted GitHub archive.
///
/// GitHub zipballs wrap the repo in a single `<owner>-<repo>-<sha>` directory. Two layouts are
/// supported:
///
/// - the TOC files sit at the repo root (AdiBags, AdvancedInterfaceOptions): the wrapper is the
///   addon, so it is renamed after its TOC base name (see [`root_toc_addon_name`]) because the
///   game only loads a folder named like its TOC. Sub-folders that are addons of their own
///   (`AdiBags_Config/AdiBags_Config.toc`) are lifted out next to it, since the game never loads
///   nested addons; they become owned folders of the root addon;
/// - the repo holds one or more addon folders (WeakAuras): each first-level folder with root TOC
///   files inside the wrapper is installed under its own name.
///
/// Archives without a wrapper whose first level already holds addon folders are taken as is.
fn discover_extracted_folders(root: &Path, repo_name: &str) -> Result<ExtractedFolders, String> {
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

    if first_level_dirs.len() == 1 && has_root_toc_files(&first_level_dirs[0].path())? {
        let wrapper = first_level_dirs[0].path();
        let addon_name = root_toc_addon_name(&wrapper, repo_name)?;
        let target = root.join(&addon_name);
        if wrapper != target {
            fs::rename(&wrapper, &target).map_err(|error| {
                format!("could not name the extracted GitHub addon folder '{addon_name}': {error}")
            })?;
        }
        let mut folders = vec![addon_name.clone()];
        folders.extend(lift_bundled_addon_folders(root, &addon_name)?);
        return Ok(ExtractedFolders {
            source_root: root.to_path_buf(),
            folders,
            parent: Some(addon_name),
        });
    }

    let direct = discover_first_level_addon_folders(root)?;
    if !direct.is_empty() {
        return Ok(ExtractedFolders {
            source_root: root.to_path_buf(),
            folders: direct,
            parent: None,
        });
    }

    if first_level_dirs.len() == 1 {
        let nested_root = first_level_dirs[0].path();
        let nested = discover_first_level_addon_folders(&nested_root)?;
        if !nested.is_empty() {
            return Ok(ExtractedFolders {
                source_root: nested_root,
                folders: nested,
                parent: None,
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

/// TOC name suffixes that select a game flavor (`Foo_Mainline.toc`, `Foo-Classic.toc`, ...).
/// Stripping one yields the addon (folder) name the TOC belongs to.
const TOC_FLAVOR_SUFFIXES: &[&str] = &[
    "mainline",
    "retail",
    "classic",
    "vanilla",
    "era",
    "tbc",
    "bcc",
    "wrath",
    "wotlkc",
    "cata",
    "cataclysm",
    "mists",
    "mop",
    "forever",
    "camelot",
];

/// The addon folder name for a directory whose root holds the addon's TOC files.
///
/// Each TOC contributes its base name with one flavor suffix (`_Mainline`, `-Classic`,
/// `_Forever`, ...) stripped. One distinct base name wins outright; otherwise the base equal to
/// the repo name (ignoring case), then the most common base; a tie is an error.
/// Base names (one flavor suffix stripped) of the `.toc` files at the root of `dir`, with how
/// many TOC files share each, in first-seen order.
fn toc_base_names(dir: &Path) -> Result<Vec<(String, usize)>, String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = file_name
            .len()
            .checked_sub(".toc".len())
            .filter(|_| file_name.to_ascii_lowercase().ends_with(".toc"))
            .map(|end| &file_name[..end])
        else {
            continue;
        };
        let base = strip_toc_flavor_suffix(stem);
        if base.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(name, _)| name == base) {
            Some((_, count)) => *count += 1,
            None => counts.push((base.to_string(), 1)),
        }
    }
    Ok(counts)
}

/// Moves each immediate sub-folder of `root/<addon_name>` that is an addon of its own (a TOC
/// inside it carries the folder's name, flavor suffix aside) up to `root`, next to the root
/// addon. Folders without such a TOC (Libs, Locales, media) stay nested. Returns the moved
/// folder names, sorted.
///
/// An embedded library stays nested even when it carries its own TOC: lifting it would install
/// a second copy of a library the root addon loads from its own folder, or break the root TOC's
/// relative paths. A sub-folder is left alone when any root TOC lists a file inside it, when its
/// own TOC declares `## X-Library: true`, or when its name looks like a library.
fn lift_bundled_addon_folders(root: &Path, addon_name: &str) -> Result<Vec<String>, String> {
    let addon_root = root.join(addon_name);
    let referenced = root_toc_referenced_folders(&addon_root)?;
    let mut lifted = Vec::new();
    for entry in fs::read_dir(&addon_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case(addon_name) {
            continue;
        }
        let is_addon = toc_base_names(&entry.path())?
            .iter()
            .any(|(base, _)| base.eq_ignore_ascii_case(&name));
        if !is_addon {
            continue;
        }
        if referenced.contains(&name.to_ascii_lowercase())
            || looks_like_library_folder(&name)
            || declares_x_library(&entry.path())?
        {
            continue;
        }
        fs::rename(entry.path(), root.join(&name)).map_err(|error| {
            format!("could not lift bundled addon folder '{name}' out of '{addon_name}': {error}")
        })?;
        lifted.push(name);
    }
    lifted.sort_by_key(|name| name.to_ascii_lowercase());
    Ok(lifted)
}

/// Lowercased first path component of every file line in the root addon's TOC files (all
/// flavor variants), so `Libs\\LibStub\\LibStub.lua` and `libs/LibStub/LibStub.lua` both mark
/// `libs`. Comment and metadata lines (starting with `#`) are ignored.
fn root_toc_referenced_folders(addon_root: &Path) -> Result<HashSet<String>, String> {
    let mut referenced = HashSet::new();
    for entry in fs::read_dir(addon_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !file_name.ends_with(".toc")
            || !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
        {
            continue;
        }
        let bytes = fs::read(entry.path()).map_err(|error| error.to_string())?;
        let content = String::from_utf8_lossy(&bytes);
        for line in content.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let normalized = line.replace('\\', "/").to_ascii_lowercase();
            if let Some((first, _)) = normalized.split_once('/') {
                referenced.insert(first.trim_start_matches("./").to_string());
            }
        }
    }
    Ok(referenced)
}

/// True when any TOC at the root of `folder` declares `## X-Library: true`.
fn declares_x_library(folder: &Path) -> Result<bool, String> {
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !file_name.ends_with(".toc")
            || !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
        {
            continue;
        }
        let bytes = fs::read(entry.path()).map_err(|error| error.to_string())?;
        if lemonup_core::parse_toc_content(&String::from_utf8_lossy(&bytes), "", None).x_library {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Folder names that follow library conventions: `LibStub`, `Lib` + an uppercase letter or
/// `-` (`LibDBIcon-1.0`, `Lib-Foo`), `CallbackHandler*`, and `Ace` + an uppercase letter, digit
/// or `-` (`AceGUI-3.0`, `Ace3`). Stricter than the scan's own pattern so an addon that merely
/// starts with "Lib" or "Ace" (`Libram`, `Acerbic`) is still lifted.
fn looks_like_library_folder(name: &str) -> bool {
    fn after(name: &str, prefix: &str, accepts: impl Fn(char) -> bool) -> bool {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.chars().next())
            .is_some_and(accepts)
    }
    name.eq_ignore_ascii_case("LibStub")
        || name.starts_with("CallbackHandler")
        || after(name, "Lib", |ch| ch.is_ascii_uppercase() || ch == '-')
        || after(name, "Ace", |ch| {
            ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-'
        })
}

fn root_toc_addon_name(dir: &Path, repo_name: &str) -> Result<String, String> {
    let mut counts = toc_base_names(dir)?;
    if counts.len() == 1 {
        return Ok(counts.remove(0).0);
    }
    if let Some((name, _)) = counts
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(repo_name))
    {
        return Ok(name.clone());
    }
    let max = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let mut leaders = counts
        .iter()
        .filter(|(_, count)| *count == max)
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    match leaders.len() {
        0 => Err("no usable .toc files were found at the root of the GitHub archive".to_string()),
        1 => Ok(leaders.remove(0)),
        _ => {
            leaders.sort_by_key(|name| name.to_ascii_lowercase());
            Err(format!(
                "cannot tell which addon the GitHub repo '{repo_name}' is: its root .toc files name {}",
                leaders.join(", ")
            ))
        }
    }
}

fn strip_toc_flavor_suffix(stem: &str) -> &str {
    let lower = stem.to_ascii_lowercase();
    for suffix in TOC_FLAVOR_SUFFIXES {
        for separator in ['_', '-'] {
            let tail = format!("{separator}{suffix}");
            if lower.len() > tail.len() && lower.ends_with(&tail) {
                return &stem[..stem.len() - tail.len()];
            }
        }
    }
    stem
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

/// The TOC version, unless it is a packager placeholder; then the release tag or short commit.
fn install_version(parent_version: Option<&str>, package: &GithubPackage) -> Option<String> {
    parent_version
        .filter(|value| !is_placeholder_version(value))
        .map(ToOwned::to_owned)
        .or_else(|| {
            Some(match package {
                GithubPackage::Release { tag, .. } => tag.clone(),
                GithubPackage::Source { head_sha } => short_commit(head_sha),
            })
        })
}

fn is_placeholder_version(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('@') && trimmed.ends_with('@')
}

/// Whether `value` is a commit as a check stores it: GitHub's full 40 hex characters. A shorter
/// hex string next to it is a release tag that looks like a commit (`20241118`).
pub(crate) fn is_full_commit(value: &str) -> bool {
    let value = value.trim();
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
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
        GithubRepoResponse, GithubToken, GithubTokenSource, build_github_client,
        build_github_client_with_token, build_managed_github_record, check_known_repository,
        classify_rate_limit, classify_rejected_token, commits_match, determine_parent_folder,
        discover_extracted_folders, environment_github_token_var, extract_zip_archive,
        github_token_can_be_sent, github_token_in_use, github_token_source, github_token_source_of,
        install_downloaded_github_addon, needs_known_repository_check, parse_github_repo_url,
        parse_github_target, resolve_github_token, update_downloaded_github_addon,
        use_saved_github_token,
    };
    use lemonup_core::{
        AddonKind, AddonRecord, GameFlavor, ScannedAddon, SourceKind, StateDatabase,
    };
    use reqwest::StatusCode;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use tempfile::tempdir;

    fn env_lookup(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs = pairs.to_vec();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn token(saved: Option<&str>, pairs: &[(&'static str, &'static str)]) -> Option<GithubToken> {
        resolve_github_token(saved, env_lookup(pairs))
    }

    fn value_and_source(token: Option<GithubToken>) -> Option<(String, GithubTokenSource)> {
        token.map(|token| (token.value, token.source))
    }

    #[test]
    fn token_resolution_prefers_github_token_over_gh_token() {
        let found = token(
            None,
            &[("GITHUB_TOKEN", "primary"), ("GH_TOKEN", "secondary")],
        );
        assert_eq!(
            value_and_source(found),
            Some((
                "primary".to_string(),
                GithubTokenSource::Environment("GITHUB_TOKEN")
            ))
        );
    }

    #[test]
    fn token_resolution_falls_back_to_gh_token() {
        let found = token(None, &[("GH_TOKEN", "secondary")]);
        assert_eq!(
            value_and_source(found),
            Some((
                "secondary".to_string(),
                GithubTokenSource::Environment("GH_TOKEN")
            ))
        );
    }

    #[test]
    fn token_resolution_trims_and_skips_empty_values() {
        let found = token(
            None,
            &[("GITHUB_TOKEN", "   "), ("GH_TOKEN", "  padded \n")],
        );
        assert_eq!(
            value_and_source(found),
            Some((
                "padded".to_string(),
                GithubTokenSource::Environment("GH_TOKEN")
            ))
        );

        assert_eq!(
            token(None, &[("GITHUB_TOKEN", ""), ("GH_TOKEN", "\t")]),
            None
        );
        assert_eq!(token(None, &[]), None);
        // An empty saved token is no token: the environment's is used.
        assert_eq!(
            value_and_source(token(Some("  "), &[("GH_TOKEN", "env")])),
            Some((
                "env".to_string(),
                GithubTokenSource::Environment("GH_TOKEN")
            ))
        );
    }

    #[test]
    fn a_token_saved_in_settings_wins_over_the_environment() {
        let both = &[("GITHUB_TOKEN", "primary"), ("GH_TOKEN", "secondary")];
        assert_eq!(
            value_and_source(token(Some(" saved "), both)),
            Some(("saved".to_string(), GithubTokenSource::Settings))
        );
        assert_eq!(
            value_and_source(token(Some("saved"), &[])),
            Some(("saved".to_string(), GithubTokenSource::Settings))
        );
        assert_eq!(
            environment_github_token_var(env_lookup(both)),
            Some("GITHUB_TOKEN")
        );
        assert_eq!(environment_github_token_var(env_lookup(&[])), None);
        assert_eq!(
            github_token_source_of(Some("saved"), Some("GH_TOKEN")),
            Some(GithubTokenSource::Settings)
        );
        assert_eq!(
            github_token_source_of(None, Some("GH_TOKEN")),
            Some(GithubTokenSource::Environment("GH_TOKEN"))
        );
        assert_eq!(github_token_source_of(Some(" "), None), None);
    }

    #[test]
    fn the_saved_token_reaches_the_next_client_and_is_never_printed() {
        use_saved_github_token(Some(" ghp_saved_secret "));
        let in_use = github_token_in_use().expect("a token");
        assert_eq!(in_use.source, GithubTokenSource::Settings);
        assert_eq!(in_use.value, "ghp_saved_secret");
        assert!(!format!("{in_use:?}").contains("ghp_saved_secret"));
        assert_eq!(github_token_source(), Some(GithubTokenSource::Settings));

        // Removing it in Settings counts from the next client on.
        use_saved_github_token(None);
        assert_ne!(github_token_source(), Some(GithubTokenSource::Settings));
        use_saved_github_token(Some("   "));
        assert_ne!(github_token_source(), Some(GithubTokenSource::Settings));
    }

    #[tokio::test]
    async fn the_client_sends_the_saved_token() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).expect("read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .expect("respond");
            String::from_utf8(request).expect("utf8")
        });

        // The store is per test thread: the client is built here, on the test's own thread.
        use_saved_github_token(Some("saved-test-token"));
        let client = build_github_client();
        use_saved_github_token(None);
        let response = client
            .expect("client")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("send");
        assert!(response.status().is_success());

        let request = server.join().expect("server");
        let authorization: Vec<&str> = request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.trim().eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.trim())
            .collect();
        assert_eq!(authorization, ["Bearer saved-test-token"]);
    }

    #[test]
    fn only_a_token_of_plain_visible_characters_can_be_sent() {
        assert!(github_token_can_be_sent("ghp_AbC123"));
        assert!(github_token_can_be_sent("github_pat_11AB_cd"));
        for pasted in [
            "ghp_\u{201c}abc\u{201d}",
            "ghp_abc\u{200b}",
            "ghp_\u{1}abc",
            "ghp_\u{7f}",
        ] {
            assert!(!github_token_can_be_sent(pasted), "{pasted:?}");
        }
    }

    #[test]
    fn client_builds_with_and_without_token_and_rejects_invalid_token_without_echoing_it() {
        assert!(build_github_client_with_token(None).is_ok());
        assert!(build_github_client_with_token(Some("ghp_valid")).is_ok());
        let error = build_github_client_with_token(Some("bad\ntoken-secret")).unwrap_err();
        assert!(!error.contains("token-secret"));
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        map
    }

    #[test]
    fn unauthorized_with_token_names_where_the_token_came_from() {
        let message =
            classify_rejected_token(StatusCode::UNAUTHORIZED, Some(GithubTokenSource::Settings))
                .expect("rejected token");
        assert!(message.contains("saved in Settings"), "{message}");
        assert!(message.contains("HTTP 401"), "{message}");

        let message = classify_rejected_token(
            StatusCode::UNAUTHORIZED,
            Some(GithubTokenSource::Environment("GH_TOKEN")),
        )
        .expect("rejected token");
        assert!(message.contains("from GH_TOKEN"), "{message}");
        assert!(!message.contains("GITHUB_TOKEN"), "{message}");
        assert!(message.contains("HTTP 401"), "{message}");
    }

    #[test]
    fn unauthorized_without_token_keeps_generic_error() {
        assert!(classify_rejected_token(StatusCode::UNAUTHORIZED, None).is_none());
        assert!(
            classify_rejected_token(StatusCode::FORBIDDEN, Some(GithubTokenSource::Settings))
                .is_none()
        );
    }

    const HINT: &str = "add a GitHub token in Settings or set GITHUB_TOKEN to raise the limit";

    #[test]
    fn rate_limit_403_with_zero_remaining_is_classified() {
        let message = classify_rate_limit(
            StatusCode::FORBIDDEN,
            &headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1700000600"),
            ]),
            1_700_000_000,
            false,
        )
        .expect("rate limit error");
        assert!(message.contains("rate limit"), "{message}");
        assert!(message.contains("resets in 10 minutes"), "{message}");
        assert!(message.contains("22:23 UTC"), "{message}");
        assert!(message.ends_with(HINT), "{message}");
    }

    #[test]
    fn rate_limit_with_a_token_sent_leaves_off_the_hint_to_add_one() {
        let message = classify_rate_limit(
            StatusCode::FORBIDDEN,
            &headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1700000600"),
            ]),
            1_700_000_000,
            true,
        )
        .expect("rate limit error");
        assert!(message.contains("HTTP 403"), "{message}");
        assert!(message.contains("resets in 10 minutes"), "{message}");
        assert!(!message.contains("GITHUB_TOKEN"), "{message}");
        assert!(!message.contains("Settings"), "{message}");
    }

    #[test]
    fn rate_limit_403_without_reset_header_still_classified() {
        let message = classify_rate_limit(
            StatusCode::FORBIDDEN,
            &headers(&[("x-ratelimit-remaining", "0")]),
            1_700_000_000,
            false,
        )
        .expect("rate limit error");
        assert!(message.contains(HINT), "{message}");
        assert!(!message.contains("resets"), "{message}");
    }

    #[test]
    fn rate_limit_429_with_retry_after_is_classified() {
        let message = classify_rate_limit(
            StatusCode::TOO_MANY_REQUESTS,
            &headers(&[("retry-after", "120")]),
            1_700_000_000,
            false,
        )
        .expect("rate limit error");
        assert!(message.contains("resets in 2 minutes"), "{message}");
        assert!(message.contains(HINT), "{message}");
    }

    #[test]
    fn non_rate_limit_responses_are_not_classified() {
        // Plain 403 (e.g. forbidden repo) with quota remaining.
        assert!(
            classify_rate_limit(
                StatusCode::FORBIDDEN,
                &headers(&[("x-ratelimit-remaining", "42")]),
                1_700_000_000,
                false
            )
            .is_none()
        );
        assert!(classify_rate_limit(StatusCode::FORBIDDEN, &HeaderMap::new(), 0, false).is_none());
        // Other statuses are never rate-limit errors, even with a zero counter.
        assert!(
            classify_rate_limit(
                StatusCode::NOT_FOUND,
                &headers(&[("x-ratelimit-remaining", "0")]),
                0,
                false
            )
            .is_none()
        );
        // 429 without any rate-limit hint stays a normal error.
        assert!(
            classify_rate_limit(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), 0, false)
                .is_none()
        );
    }

    #[test]
    fn secondary_rate_limit_403_with_retry_after_is_a_rate_limit() {
        // GitHub's secondary rate limit: 403, requests still remaining, retry-after set.
        let message = classify_rate_limit(
            StatusCode::FORBIDDEN,
            &headers(&[("x-ratelimit-remaining", "42"), ("retry-after", "60")]),
            1_000,
            false,
        )
        .expect("a 403 with retry-after is a rate limit");
        assert!(message.contains("HTTP 403"), "{message}");
        assert!(message.contains("resets in 1 minute"), "{message}");
    }

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

    fn collect_file_names(root: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(root).expect("read dir") {
            let entry = entry.expect("dir entry");
            out.push(entry.file_name().to_string_lossy().into_owned());
            if entry.file_type().expect("file type").is_dir() {
                collect_file_names(&entry.path(), out);
            }
        }
    }

    #[test]
    fn extract_zip_archive_nests_windows_style_backslash_entries() {
        let workspace = tempdir().expect("tempdir");
        let zip_path = workspace.path().join("addon.zip");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("Foo\\Foo.toc", options).expect("start toc");
            zip.write_all(b"## Title: Foo").expect("write toc");
            zip.add_directory("Foo\\Sub\\", options).expect("add dir");
            zip.start_file("Foo\\Sub\\a.lua", options)
                .expect("start lua");
            zip.write_all(b"print('a')").expect("write lua");
            zip.finish().expect("finish zip");
        }
        std::fs::write(&zip_path, cursor.into_inner()).expect("write zip");

        let destination = workspace.path().join("out");
        extract_zip_archive(&zip_path, &destination).expect("extract");

        assert!(destination.join("Foo").join("Foo.toc").is_file());
        assert!(destination.join("Foo").join("Sub").is_dir());
        assert!(destination.join("Foo").join("Sub").join("a.lua").is_file());
        let mut names = Vec::new();
        collect_file_names(&destination, &mut names);
        assert!(
            names.iter().all(|name| !name.contains('\\')),
            "no extracted name may contain a backslash: {names:?}"
        );
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
    fn parse_github_repo_url_drops_a_git_ending_in_any_case() {
        for url in [
            "https://github.com/svengabr/ForeverLootSparkles.git",
            "https://github.com/svengabr/ForeverLootSparkles.GIT",
            "https://github.com/svengabr/ForeverLootSparkles.Git",
        ] {
            let parsed = parse_github_repo_url(url).expect("github repo");
            assert_eq!(parsed.repo, "ForeverLootSparkles", "{url}");
        }
        // The ending is dropped, so the listed repository is still checked.
        let list = known_list();
        let parsed = parse_github_repo_url("https://github.com/svengabr/ForeverLootSparkles.GIT")
            .expect("github repo");
        assert!(needs_known_repository_check(
            &list,
            &parsed.owner,
            &parsed.repo
        ));
    }

    #[test]
    fn github_repo_response_deserializes() {
        let parsed = serde_json::from_str::<GithubRepoResponse>(
                            r#"{"name":"AdiBags","default_branch":"main","html_url":"https://github.com/AdiAddons/AdiBags","owner":{"login":"AdiAddons"},"id":42}"#,
                                    )
                                            .expect("repo response");
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.name, "AdiBags");
        assert_eq!(parsed.default_branch, "main");
        assert_eq!(parsed.owner.login, "AdiAddons");
        assert_eq!(parsed.html_url, "https://github.com/AdiAddons/AdiBags");
    }

    fn known_list() -> Vec<lemonup_core::known_addons::KnownAddon> {
        lemonup_core::known_addons::parse_known_addons(
            r#"
[[addon]]
name = "Forever Loot Sparkles"
github = "svengabr/ForeverLootSparkles"
github_id = 20
games = ["forever"]

[[addon]]
name = "Leatrix Plus"
curseforge_only = true
games = ["forever"]
"#,
        )
        .expect("test list parses")
    }

    #[test]
    fn a_listed_repository_with_the_listed_id_passes() {
        assert!(
            check_known_repository(
                &known_list(),
                "svengabr",
                "ForeverLootSparkles",
                20,
                "svengabr"
            )
            .is_ok()
        );
    }

    #[test]
    fn a_listed_repository_with_another_id_is_refused() {
        let error = check_known_repository(
            &known_list(),
            "svengabr",
            "ForeverLootSparkles",
            21,
            "svengabr",
        )
        .expect_err("another id");
        assert_eq!(
            error.kind,
            crate::source_error::SourceErrorKind::WrongRepository
        );
        assert_eq!(
            error.detail,
            "github.com/svengabr/ForeverLootSparkles is not the repository LemonUp lists for \
             Forever Loot Sparkles"
        );
    }

    #[test]
    fn the_listed_repository_is_found_whatever_the_case() {
        let list = known_list();
        assert!(
            check_known_repository(&list, "SvenGabr", "foreverlootsparkles", 20, "SVENGABR")
                .is_ok()
        );
        let error =
            check_known_repository(&list, "SVENGABR", "FOREVERLOOTSPARKLES", 99, "svengabr")
                .expect_err("another id");
        assert_eq!(
            error.kind,
            crate::source_error::SourceErrorKind::WrongRepository
        );
        assert!(
            error
                .detail
                .starts_with("github.com/SVENGABR/FOREVERLOOTSPARKLES ")
        );
    }

    #[test]
    fn a_repository_the_list_does_not_name_passes_with_any_id() {
        let list = known_list();
        assert!(check_known_repository(&list, "AdiAddons", "AdiBags", 1, "AdiAddons").is_ok());
        assert!(check_known_repository(&list, "AdiAddons", "AdiBags", u64::MAX, "someone").is_ok());
    }

    #[test]
    fn a_listed_repository_with_the_listed_id_and_another_owner_is_refused() {
        // A transferred repository keeps its id; GitHub reports the new owner.
        let error = check_known_repository(
            &known_list(),
            "svengabr",
            "ForeverLootSparkles",
            20,
            "someone-else",
        )
        .expect_err("another owner");
        assert_eq!(
            error.kind,
            crate::source_error::SourceErrorKind::WrongRepository
        );
        assert_eq!(
            error.detail,
            "github.com/svengabr/ForeverLootSparkles is not the repository LemonUp lists for \
             Forever Loot Sparkles"
        );
    }

    #[test]
    fn only_a_listed_repository_needs_the_metadata_check_first() {
        let list = known_list();
        assert!(needs_known_repository_check(
            &list,
            "svengabr",
            "ForeverLootSparkles"
        ));
        assert!(needs_known_repository_check(
            &list,
            "SVENGABR",
            "foreverlootsparkles"
        ));
        assert!(!needs_known_repository_check(&list, "AdiAddons", "AdiBags"));
        // A CurseForge-only entry has no repository to check.
        assert!(!needs_known_repository_check(
            &list,
            "leatrix",
            "Leatrix Plus"
        ));
    }

    #[test]
    fn every_github_entry_on_the_real_known_addons_list_parses_and_is_found_again() {
        // The one app test that reads the shipped list.
        let list = lemonup_core::known_addons::known_addons();
        let mut github_entries = 0;
        for addon in list {
            let lemonup_core::known_addons::KnownSource::GitHub { owner, repo, .. } = &addon.source
            else {
                continue;
            };
            github_entries += 1;
            let link = format!("https://github.com/{owner}/{repo}");
            let target = parse_github_target(&link).expect("the link parses");
            let parsed = parse_github_repo_url(&target).expect("the target parses");
            let found = lemonup_core::known_addons::find_known_github_repo(
                list,
                &parsed.owner,
                &parsed.repo,
            )
            .expect("found again");
            assert_eq!(found, addon, "{link}");
        }
        assert!(github_entries > 0);
    }

    #[test]
    fn a_curseforge_only_entry_never_blocks_a_repository() {
        assert!(
            check_known_repository(&known_list(), "someone", "Leatrix Plus", 5, "someone").is_ok()
        );
        assert!(
            check_known_repository(&known_list(), "leatrix", "leatrix-plus", 5, "leatrix").is_ok()
        );
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
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "abcdef0123456789".to_string(),
            },
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
            notes: None,
            category: None,
            toc_title: None,
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
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string(),
            },
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
            notes: None,
            category: None,
            toc_title: None,
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
    fn update_downloaded_github_addon_takes_folders_from_the_package_for_inferred_ownership() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        for folder in [
            "AdiBags",
            "AdiBags_Config",
            "AdiBags_Legacy",
            "AdiBags_Extras",
        ] {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(addon_dir.join(folder).join("stale.lua"), "old").expect("stale");
        }
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/AdiAddons/AdiBags".to_string());
        // Scan-inferred grouping is a guess: only package folders are replaced.
        tracked.set_scan_owned_folders(vec![
            lemonup_core::OwnedFolder {
                name: "AdiBags_Config".to_string(),
            },
            lemonup_core::OwnedFolder {
                name: "AdiBags_Legacy".to_string(),
            },
        ]);
        database.upsert_addon(&tracked).expect("seed addon");
        database
            .upsert_addon(&AddonRecord::new(
                "AdiBags_Extras",
                "AdiBags_Extras",
                SourceKind::Manual,
            ))
            .expect("seed manual row");

        let resolved = super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: "AdiAddons".to_string(),
                repo: "AdiBags".to_string(),
                default_branch: "main".to_string(),
                html_url: "https://github.com/AdiAddons/AdiBags".to_string(),
                owner_login: "AdiAddons".to_string(),
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "abcdef0123456789".to_string(),
            },
        };
        let toc = "## Title: AdiBags\n## Version: 1.1.0\n## Interface: 110205\n";
        let bytes = build_zip(&[
            ("wrapper/AdiBags/AdiBags.toc", toc),
            ("wrapper/AdiBags_Config/AdiBags_Config.toc", toc),
            ("wrapper/AdiBags_Extras/AdiBags_Extras.toc", toc),
        ]);

        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved,
            &bytes,
            // The person said yes to replacing the Not linked folders.
            crate::update_targets::ReplaceNotLinked::Any,
            false,
        )
        .expect("update");

        assert!(summary.updated);
        for folder in ["AdiBags", "AdiBags_Config", "AdiBags_Extras"] {
            assert!(!addon_dir.join(folder).join("stale.lua").exists());
        }
        assert!(
            addon_dir.join("AdiBags_Legacy").join("stale.lua").exists(),
            "an inferred child outside the package is left on disk"
        );
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert!(stored.has_authoritative_owned_folders());
        let mut owned = stored
            .owned_folders
            .iter()
            .map(|owned| owned.name.as_str())
            .collect::<Vec<_>>();
        owned.sort();
        assert_eq!(owned, vec!["AdiBags_Config", "AdiBags_Extras"]);
        assert!(
            database
                .get_addon_by_folder("AdiBags_Extras")
                .expect("get row")
                .is_none()
        );
    }

    #[test]
    fn update_over_a_not_linked_folder_is_held_then_applied_after_a_yes() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        for folder in ["AdiBags", "AdiBags_Extras"] {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(addon_dir.join(folder).join("stale.lua"), "old").expect("stale");
        }
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/AdiAddons/AdiBags".to_string());
        database.upsert_addon(&tracked).expect("seed addon");
        database
            .upsert_addon(&AddonRecord::new(
                "AdiBags_Extras",
                "AdiBags_Extras",
                SourceKind::Manual,
            ))
            .expect("seed manual row");
        let rows_before = database.list_addons().expect("rows");

        let resolved = super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: "AdiAddons".to_string(),
                repo: "AdiBags".to_string(),
                default_branch: "main".to_string(),
                html_url: "https://github.com/AdiAddons/AdiBags".to_string(),
                owner_login: "AdiAddons".to_string(),
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "abcdef0123456789".to_string(),
            },
        };
        let toc = "## Title: AdiBags\n## Version: 1.1.0\n## Interface: 110205\n";
        let bytes = build_zip(&[
            ("wrapper/AdiBags/AdiBags.toc", toc),
            ("wrapper/AdiBags_Extras/AdiBags_Extras.toc", toc),
        ]);

        // Nobody agreed yet: held, and nothing on disk or in the database changes.
        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved,
            &bytes,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        )
        .expect("held update");
        assert!(!summary.updated);
        assert_eq!(summary.not_linked, vec!["AdiBags_Extras".to_string()]);
        for folder in ["AdiBags", "AdiBags_Extras"] {
            assert!(
                addon_dir.join(folder).join("stale.lua").exists(),
                "{folder}"
            );
        }
        assert_eq!(database.list_addons().expect("rows"), rows_before);

        // A yes applies it from the kept package, without a download.
        let package = crate::update::HeldPackage::keep(&bytes, {
            let resolved = resolved.clone();
            move |database, addon_dir, tracked, bytes, agree| {
                update_downloaded_github_addon(
                    database, addon_dir, tracked, &resolved, bytes, agree, false,
                )
                .map(super::GithubUpdateSummary::finished)
            }
        })
        .expect("keep package");
        let held =
            crate::update::HeldUpdate::new(&tracked, &addon_dir, &summary.finished(), package);
        let kept = held.package_path().to_path_buf();
        assert!(kept.exists());
        let result = held.apply(&mut database, &addon_dir);
        assert_eq!(
            result.status,
            crate::update::LiveUpdateStatus::Updated,
            "{:?}",
            result.message
        );
        assert!(
            !kept.exists(),
            "the kept package is removed after the update"
        );
        for folder in ["AdiBags", "AdiBags_Extras"] {
            assert!(
                !addon_dir.join(folder).join("stale.lua").exists(),
                "{folder}"
            );
        }
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert!(
            stored
                .owned_folders
                .iter()
                .any(|owned| owned.name == "AdiBags_Extras")
        );
        assert!(
            database
                .get_addon_by_folder("AdiBags_Extras")
                .expect("get manual")
                .is_none()
        );
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
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "abcdef0123456789".to_string(),
            },
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
            crate::update_targets::ReplaceNotLinked::NONE,
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
    fn resolved_repo(owner: &str, repo: &str) -> super::GithubResolvedRepo {
        super::GithubResolvedRepo {
            metadata: super::GithubRepoMetadata {
                owner: owner.to_string(),
                repo: repo.to_string(),
                default_branch: "main".to_string(),
                html_url: format!("https://github.com/{owner}/{repo}"),
                owner_login: owner.to_string(),
                id: 1,
            },
            package: super::GithubPackage::Source {
                head_sha: "10808e8123456789".to_string(),
            },
        }
    }

    fn extract(entries: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("repo.zip");
        std::fs::write(&zip_path, build_zip(entries)).expect("write zip");
        let root = temp.path().join("extract");
        extract_zip_archive(&zip_path, &root).expect("extract");
        (temp, root)
    }

    const ADIBAGS_TOC: &str = "## Title: AdiBags\n## Version: 2.0.0\n## Interface: 110205\n";

    #[test]
    fn root_toc_zipball_is_named_after_its_toc_not_the_wrapper() {
        let (_temp, root) = extract(&[
            ("AdiAddons-AdiBags-10808e8/AdiBags.toc", ADIBAGS_TOC),
            (
                "AdiAddons-AdiBags-10808e8/AdiBags_Mainline.toc",
                ADIBAGS_TOC,
            ),
            ("AdiAddons-AdiBags-10808e8/AdiBags-Classic.toc", ADIBAGS_TOC),
            ("AdiAddons-AdiBags-10808e8/core.lua", "-- core"),
            ("AdiAddons-AdiBags-10808e8/modules/Junk.lua", "-- junk"),
        ]);

        let found = discover_extracted_folders(&root, "AdiBags").expect("discover");

        assert_eq!(found.folders, vec!["AdiBags".to_string()]);
        assert_eq!(found.source_root, root);
        assert!(root.join("AdiBags").join("core.lua").exists());
        assert!(
            root.join("AdiBags")
                .join("modules")
                .join("Junk.lua")
                .exists()
        );
        assert!(!root.join("AdiAddons-AdiBags-10808e8").exists());
    }

    #[test]
    fn root_toc_zipball_with_only_flavor_tocs_strips_the_suffix() {
        let (_temp, root) = extract(&[
            (
                "Stanzilla-AdvancedInterfaceOptions-7fa425b/AdvancedInterfaceOptions_Mainline.toc",
                ADIBAGS_TOC,
            ),
            (
                "Stanzilla-AdvancedInterfaceOptions-7fa425b/AdvancedInterfaceOptions_Forever.toc",
                ADIBAGS_TOC,
            ),
        ]);
        let found =
            discover_extracted_folders(&root, "AdvancedInterfaceOptions").expect("discover");
        assert_eq!(found.folders, vec!["AdvancedInterfaceOptions".to_string()]);
    }

    #[test]
    fn root_toc_zipball_with_several_bases_prefers_repo_name_then_majority_else_errors() {
        let entries = [
            ("o-r-1/Alpha.toc", ADIBAGS_TOC),
            ("o-r-1/Alpha_Mainline.toc", ADIBAGS_TOC),
            ("o-r-1/beta.toc", ADIBAGS_TOC),
        ];
        let (_temp, root) = extract(&entries);
        assert_eq!(
            discover_extracted_folders(&root, "Beta")
                .expect("repo match")
                .folders,
            vec!["beta".to_string()]
        );
        let (_temp, root) = extract(&entries);
        assert_eq!(
            discover_extracted_folders(&root, "Other")
                .expect("majority")
                .folders,
            vec!["Alpha".to_string()]
        );
        let (_temp, root) = extract(&[
            ("o-r-1/Alpha.toc", ADIBAGS_TOC),
            ("o-r-1/Beta.toc", ADIBAGS_TOC),
        ]);
        let error = discover_extracted_folders(&root, "Other").expect_err("tie");
        assert!(error.contains("Alpha, Beta"), "{error}");
    }

    #[test]
    fn nested_addon_folders_inside_the_wrapper_are_unchanged() {
        let (_temp, root) = extract(&[
            ("WeakAuras-WeakAuras2-364f625/README.md", "readme"),
            (
                "WeakAuras-WeakAuras2-364f625/WeakAuras/WeakAuras.toc",
                ADIBAGS_TOC,
            ),
            (
                "WeakAuras-WeakAuras2-364f625/WeakAurasOptions/WeakAurasOptions.toc",
                ADIBAGS_TOC,
            ),
        ]);
        let found = discover_extracted_folders(&root, "WeakAuras2").expect("discover");
        assert_eq!(
            found.folders,
            vec!["WeakAuras".to_string(), "WeakAurasOptions".to_string()]
        );
        assert_eq!(found.source_root, root.join("WeakAuras-WeakAuras2-364f625"));
    }

    #[test]
    fn install_of_a_root_toc_repo_creates_the_toc_named_folder() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let bytes = build_zip(&[
            ("AdiAddons-AdiBags-10808e8/AdiBags.toc", ADIBAGS_TOC),
            (
                "AdiAddons-AdiBags-10808e8/AdiBags_Mainline.toc",
                ADIBAGS_TOC,
            ),
        ]);

        let summary = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved_repo("AdiAddons", "AdiBags"),
            &bytes,
            false,
            &[],
        )
        .expect("install");

        assert_eq!(summary.parent_folder, "AdiBags");
        assert_eq!(summary.installed_folders, vec!["AdiBags".to_string()]);
        assert!(
            addon_dir
                .join("AdiBags")
                .join("AdiBags_Mainline.toc")
                .exists()
        );
        let entries = std::fs::read_dir(&addon_dir)
            .expect("read addons")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec!["AdiBags".to_string()],
            "no wrapper or staging left"
        );
        assert!(
            database
                .get_addon_by_folder("AdiBags")
                .expect("get")
                .is_some()
        );
    }

    #[test]
    fn update_of_a_tracked_root_toc_addon_resolves_to_the_tracked_folder() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags")).expect("create addon dir");
        std::fs::write(addon_dir.join("AdiBags").join("AdiBags.toc"), ADIBAGS_TOC).expect("toc");
        std::fs::write(addon_dir.join("AdiBags").join("stale.lua"), "old").expect("stale");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        // Shaped like a row migrated from v1: no authoritative ownership.
        let mut tracked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/AdiAddons/AdiBags".to_string());
        database.upsert_addon(&tracked).expect("seed addon");
        let bytes = build_zip(&[
            ("AdiAddons-AdiBags-10808e8/AdiBags.toc", ADIBAGS_TOC),
            (
                "AdiAddons-AdiBags-10808e8/AdiBags_Mainline.toc",
                ADIBAGS_TOC,
            ),
            ("AdiAddons-AdiBags-10808e8/core.lua", "-- new"),
        ]);

        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved_repo("AdiAddons", "AdiBags"),
            &bytes,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        )
        .expect("update");

        assert!(summary.updated);
        assert_eq!(summary.parent_folder, "AdiBags");
        assert!(addon_dir.join("AdiBags").join("core.lua").exists());
        assert!(!addon_dir.join("AdiBags").join("stale.lua").exists());
        assert!(!addon_dir.join("AdiAddons-AdiBags-10808e8").exists());
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("2.0.0"));
    }

    fn toc_overrides(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn update_writes_the_stored_toc_overrides_into_every_new_folder() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database, tracked, bytes) = seed_foo_with_options(temp.path());
        let wanted = toc_overrides(&[("Title", "My Foo"), ("Category", "Mine")]);
        database
            .set_toc_overrides("Foo", &wanted)
            .expect("set overrides");

        update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved_repo("x", "Foo"),
            &bytes,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        )
        .expect("update");

        for toc in ["Foo/Foo.toc", "Foo_Options/Foo_Options.toc"] {
            let text = std::fs::read_to_string(addon_dir.join(toc)).expect("toc");
            assert_eq!(
                text,
                "## Title: Foo\n## Version: 1.1.0\n## Interface: 110205\n# LemonUp-managed\n## Title: My Foo\n# LemonUp-managed\n## Category: Mine\n"
                    .replace("## Title: Foo\n", "# LemonUp: ## Title: Foo\n"),
                "{toc}"
            );
        }
        let stored = database
            .get_addon_by_folder("Foo")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(stored.version.as_deref(), Some("1.1.0"));
        assert_eq!(stored.toc_overrides, wanted);
        // The Title override is the stored name, as a scan would store it.
        assert_eq!(stored.name, "My Foo");
    }

    #[test]
    fn install_over_a_not_linked_addon_keeps_its_toc_overrides() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags")).expect("not linked copy");
        std::fs::write(addon_dir.join("AdiBags").join("AdiBags.toc"), ADIBAGS_TOC).expect("toc");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&AddonRecord::new("AdiBags", "AdiBags", SourceKind::Manual))
            .expect("not linked row");
        let wanted = toc_overrides(&[("Notes", "My bags")]);
        database
            .set_toc_overrides("AdiBags", &wanted)
            .expect("set overrides");

        install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved_repo("AdiAddons", "AdiBags"),
            &adibags_shaped_zip("-- v1"),
            false,
            &["AdiBags".to_string()],
        )
        .expect("install");

        for toc in [
            "AdiBags/AdiBags.toc",
            "AdiBags/AdiBags_Vanilla.toc",
            "AdiBags_Config/AdiBags_Config.toc",
        ] {
            let text = std::fs::read_to_string(addon_dir.join(toc)).expect("toc");
            assert!(
                text.contains("# LemonUp-managed\n## Notes: My bags\n"),
                "{toc}: {text}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(addon_dir.join("AdiBags/Libs/LibStub/LibStub.toc"))
                .expect("lib toc"),
            "## Title: LibStub\n"
        );
        let linked = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("linked");
        assert_eq!(linked.source, SourceKind::GitHub);
        assert_eq!(linked.toc_overrides, wanted);
    }

    #[test]
    fn install_that_takes_over_a_not_linked_folder_writes_its_toc_overrides() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags_Config")).expect("not linked copy");
        std::fs::write(
            addon_dir.join("AdiBags_Config").join("AdiBags_Config.toc"),
            "## Title: AdiBags Config\n",
        )
        .expect("toc");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        database
            .upsert_addon(&AddonRecord::new(
                "AdiBags Config",
                "AdiBags_Config",
                SourceKind::Manual,
            ))
            .expect("not linked row");
        let wanted = toc_overrides(&[("Category", "Mine")]);
        database
            .set_toc_overrides("AdiBags_Config", &wanted)
            .expect("set overrides");

        install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved_repo("AdiAddons", "AdiBags"),
            &adibags_shaped_zip("-- v1"),
            false,
            &["AdiBags_Config".to_string()],
        )
        .expect("install");

        for toc in [
            "AdiBags/AdiBags.toc",
            "AdiBags/AdiBags_Vanilla.toc",
            "AdiBags_Config/AdiBags_Config.toc",
        ] {
            let text = std::fs::read_to_string(addon_dir.join(toc)).expect("toc");
            assert!(
                text.contains("# LemonUp-managed\n## Category: Mine\n"),
                "{toc}: {text}"
            );
        }
        assert!(
            database
                .get_addon_by_folder("AdiBags_Config")
                .expect("get child")
                .is_none()
        );
        let linked = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("linked");
        assert_eq!(linked.toc_overrides, wanted);
    }

    fn seed_foo_with_options(
        temp: &std::path::Path,
    ) -> (std::path::PathBuf, StateDatabase, AddonRecord, Vec<u8>) {
        let addon_dir = temp.join("_retail_").join("Interface").join("AddOns");
        for folder in ["Foo", "Foo_Options"] {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(addon_dir.join(folder).join("old.lua"), "old").expect("old");
        }
        let mut database = StateDatabase::open(temp.join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/x/Foo".to_string());
        tracked.set_managed_owned_folders(vec![lemonup_core::OwnedFolder {
            name: "Foo_Options".to_string(),
        }]);
        database.record_managed_addon(&tracked).expect("seed addon");
        let toc = "## Title: Foo\n## Version: 1.1.0\n## Interface: 110205\n";
        let bytes = build_zip(&[
            ("wrapper/Foo/Foo.toc", toc),
            ("wrapper/Foo/new.lua", "new"),
            ("wrapper/Foo_Options/Foo_Options.toc", toc),
        ]);
        (addon_dir, database, tracked, bytes)
    }

    #[cfg(unix)]
    #[test]
    fn failed_update_with_a_locked_owned_folder_leaves_folders_and_db_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database, tracked, bytes) = seed_foo_with_options(temp.path());
        // A folder that cannot be moved (Linux needs write access to a directory to move it to
        // another parent) stands in for one holding a file locked by another process on Windows.
        let locked = addon_dir.join("Foo_Options");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        if std::fs::write(locked.join("probe"), "x").is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
            return; // privileged run: permissions are not enforced
        }

        let result = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved_repo("x", "Foo"),
            &bytes,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("chmod back");

        let error = result.expect_err("update must fail");
        assert!(error.contains("Foo_Options"), "{error}");
        for folder in ["Foo", "Foo_Options"] {
            assert!(
                addon_dir.join(folder).join("old.lua").exists(),
                "{folder} restored"
            );
        }
        assert!(!addon_dir.join("Foo").join("new.lua").exists());
        let stored = database
            .get_addon_by_folder("Foo")
            .expect("get")
            .expect("row");
        assert_eq!(stored.version.as_deref(), Some("1.0.0"));
        assert_eq!(stored.git_commit.as_deref(), Some("abcdef0"));
        let leftovers = std::fs::read_dir(&addon_dir)
            .expect("read addons")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".lemonup-staging-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[cfg(unix)]
    #[test]
    fn update_with_an_undeletable_subfolder_no_longer_loses_folders() {
        // The adversarial reproduction: an owned folder holds a subfolder that cannot be
        // emptied. Copy-then-delete failed halfway and lost `Foo`; renames do not need to empty
        // it, so the update now completes and only the inert backup stays behind.
        use std::os::unix::fs::PermissionsExt;
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database, tracked, bytes) = seed_foo_with_options(temp.path());
        let locked = addon_dir.join("Foo_Options").join("locked");
        std::fs::create_dir_all(&locked).expect("locked dir");
        std::fs::write(locked.join("font.ttf"), "x").expect("locked file");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("chmod");

        let result = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved_repo("x", "Foo"),
            &bytes,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        );

        // Unlock wherever the folder ended up so the temp dir can be cleaned.
        let mut stack = vec![addon_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o755),
                    );
                    stack.push(entry.path());
                }
            }
        }

        assert!(result.expect("update").updated);
        assert!(addon_dir.join("Foo").join("new.lua").exists());
        assert!(
            addon_dir
                .join("Foo_Options")
                .join("Foo_Options.toc")
                .exists()
        );
        let stored = database
            .get_addon_by_folder("Foo")
            .expect("get")
            .expect("row");
        assert_eq!(stored.version.as_deref(), Some("1.1.0"));
    }
    fn adibags_shaped_zip(core: &str) -> Vec<u8> {
        build_zip(&[
            ("AdiAddons-AdiBags-10808e8/AdiBags.toc", ADIBAGS_TOC),
            ("AdiAddons-AdiBags-10808e8/AdiBags_Vanilla.toc", ADIBAGS_TOC),
            ("AdiAddons-AdiBags-10808e8/core.lua", core),
            (
                "AdiAddons-AdiBags-10808e8/AdiBags_Config/AdiBags_Config.toc",
                "## Title: AdiBags Config\n## Interface: 110205\n## LoadOnDemand: 1\n",
            ),
            (
                "AdiAddons-AdiBags-10808e8/AdiBags_Config/AdiBags_Config_Cata.toc",
                "## Title: c\n",
            ),
            ("AdiAddons-AdiBags-10808e8/AdiBags_Config/Options.lua", core),
            (
                "AdiAddons-AdiBags-10808e8/Libs/LibStub/LibStub.toc",
                "## Title: LibStub\n",
            ),
            (
                "AdiAddons-AdiBags-10808e8/Libs/LibStub/LibStub.lua",
                "-- lib",
            ),
            ("AdiAddons-AdiBags-10808e8/Locales/enUS.lua", "-- locale"),
            ("AdiAddons-AdiBags-10808e8/media/icon.tga", "tga"),
        ])
    }

    fn addon_dir_listing(addon_dir: &std::path::Path) -> Vec<String> {
        let mut names = std::fs::read_dir(addon_dir)
            .expect("read addons")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn root_toc_layout_lifts_bundled_addon_folders_and_keeps_libs_nested() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("repo.zip");
        std::fs::write(&zip_path, adibags_shaped_zip("-- core")).expect("write zip");
        let root = temp.path().join("extract");
        extract_zip_archive(&zip_path, &root).expect("extract");

        let found = discover_extracted_folders(&root, "AdiBags").expect("discover");

        assert_eq!(
            found.folders,
            vec!["AdiBags".to_string(), "AdiBags_Config".to_string()]
        );
        assert_eq!(found.parent.as_deref(), Some("AdiBags"));
        assert!(root.join("AdiBags_Config").join("Options.lua").exists());
        assert!(!root.join("AdiBags").join("AdiBags_Config").exists());
        for nested in [
            "Libs/LibStub/LibStub.toc",
            "Locales/enUS.lua",
            "media/icon.tga",
        ] {
            assert!(
                root.join("AdiBags").join(nested).exists(),
                "{nested} stays nested"
            );
        }
        assert!(!root.join("Libs").exists());
    }

    fn discover_zip(
        entries: &[(&str, &str)],
    ) -> (tempfile::TempDir, std::path::PathBuf, Vec<String>) {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("repo.zip");
        std::fs::write(&zip_path, build_zip(entries)).expect("write zip");
        let root = temp.path().join("extract");
        extract_zip_archive(&zip_path, &root).expect("extract");
        let found = discover_extracted_folders(&root, "Foo").expect("discover");
        (temp, root, found.folders)
    }

    #[test]
    fn bundled_library_referenced_by_the_root_toc_stays_nested() {
        // The sub-folder has its own TOC, but the root TOC loads its files by relative path
        // (either separator, any case), so lifting it would break the addon and duplicate the lib.
        for reference in [
            "LibStub\\LibStub.lua",
            "libstub/LibStub.lua",
            "./LibStub/LibStub.lua",
        ] {
            let toc = format!("## Title: Foo\n# comment\n{reference}\nFoo.lua\n");
            let (_temp, root, folders) = discover_zip(&[
                ("repo-1/Foo.toc", &toc),
                ("repo-1/Foo.lua", "-- core"),
                ("repo-1/LibStub/LibStub.toc", "## Title: LibStub\n"),
                ("repo-1/LibStub/LibStub.lua", "-- lib"),
            ]);
            assert_eq!(folders, vec!["Foo".to_string()], "{reference}");
            assert!(
                root.join("Foo")
                    .join("LibStub")
                    .join("LibStub.toc")
                    .exists()
            );
            assert!(!root.join("LibStub").exists());
        }
    }

    #[test]
    fn root_toc_reference_in_any_flavor_toc_keeps_the_folder_nested() {
        // `Shared` is not library-shaped and declares nothing; only the flavor TOC mentions it.
        let (_temp, root, folders) = discover_zip(&[
            ("repo-1/Foo.toc", "## Title: Foo\nFoo.lua\n"),
            (
                "repo-1/Foo_Mainline.toc",
                "## Title: Foo\nShared\\Core.lua\nFoo.lua\n",
            ),
            ("repo-1/Shared/Shared.toc", "## Title: Shared\n"),
            ("repo-1/Shared/Core.lua", "-- shared"),
        ]);
        assert_eq!(folders, vec!["Foo".to_string()]);
        assert!(root.join("Foo").join("Shared").join("Core.lua").exists());
    }

    #[test]
    fn commented_out_reference_does_not_keep_a_real_addon_nested() {
        let (_temp, root, folders) = discover_zip(&[
            (
                "repo-1/Foo.toc",
                "## Title: Foo\n#Foo_Config\\Old.lua\nFoo.lua\n",
            ),
            ("repo-1/Foo_Config/Foo_Config.toc", "## Title: Foo Config\n"),
            ("repo-1/Foo_Config/Options.lua", "-- options"),
        ]);
        assert_eq!(folders, vec!["Foo".to_string(), "Foo_Config".to_string()]);
        assert!(root.join("Foo_Config").join("Options.lua").exists());
    }

    #[test]
    fn bundled_x_library_sub_folder_stays_nested() {
        let (_temp, root, folders) = discover_zip(&[
            ("repo-1/Foo.toc", "## Title: Foo\nFoo.lua\n"),
            (
                "repo-1/Shared/Shared.toc",
                "## Title: Shared\n## X-Library: true\n",
            ),
            ("repo-1/Shared/Shared.lua", "-- shared"),
        ]);
        assert_eq!(folders, vec!["Foo".to_string()]);
        assert!(root.join("Foo").join("Shared").join("Shared.lua").exists());
        assert!(!root.join("Shared").exists());
    }

    #[test]
    fn library_shaped_names_stay_nested_but_lookalike_addons_are_lifted() {
        for name in [
            "LibDBIcon-1.0",
            "Lib-Foo",
            "LibStub",
            "CallbackHandler-1.0",
            "Ace3",
            "AceGUI-3.0",
        ] {
            assert!(super::looks_like_library_folder(name), "{name}");
        }
        for name in [
            "Libram",
            "Acerbic",
            "Library",
            "Foo_Config",
            "AdiBags_Config",
        ] {
            assert!(!super::looks_like_library_folder(name), "{name}");
        }
        let (_temp, root, folders) = discover_zip(&[
            ("repo-1/Foo.toc", "## Title: Foo\nFoo.lua\n"),
            (
                "repo-1/LibDBIcon-1.0/LibDBIcon-1.0.toc",
                "## Title: LibDBIcon\n",
            ),
            ("repo-1/LibDBIcon-1.0/LibDBIcon-1.0.lua", "-- lib"),
            ("repo-1/Libram/Libram.toc", "## Title: Libram\n"),
            ("repo-1/Libram/Libram.lua", "-- addon"),
        ]);
        assert_eq!(folders, vec!["Foo".to_string(), "Libram".to_string()]);
        assert!(root.join("Foo").join("LibDBIcon-1.0").exists());
        assert!(root.join("Libram").join("Libram.lua").exists());
    }

    #[test]
    fn install_over_a_not_linked_folder_asks_first_then_replaces_and_links_it() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags")).expect("not linked copy");
        std::fs::write(addon_dir.join("AdiBags").join("old.lua"), "-- old").expect("old file");
        let wtf = temp.path().join("_retail_").join("WTF");
        std::fs::create_dir_all(&wtf).expect("wtf");
        std::fs::write(wtf.join("AdiBags.lua"), "settings").expect("saved variables");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let resolved = resolved_repo("AdiAddons", "AdiBags");

        // A dry run is refused as the install it stands for would be, and touches nothing.
        let dry_run = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved,
            &adibags_shaped_zip("-- v1"),
            true,
            &[],
        )
        .expect_err("a dry run with a folder in the way");
        assert_eq!(
            dry_run.kind,
            crate::source_error::SourceErrorKind::FoldersInTheWay
        );
        assert!(addon_dir.join("AdiBags").join("old.lua").exists());
        assert!(!addon_dir.join("AdiBags").join("AdiBags.toc").exists());
        assert!(!addon_dir.join("AdiBags_Config").exists());
        assert!(database.list_addons().expect("list").is_empty());

        let refused = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved,
            &adibags_shaped_zip("-- v1"),
            false,
            &[],
        )
        .expect_err("folder in the way");
        assert_eq!(
            refused.kind,
            crate::source_error::SourceErrorKind::FoldersInTheWay
        );
        assert!(addon_dir.join("AdiBags").join("old.lua").exists());
        let confirmed = refused.clash.expect("names the folders").replaceable;
        assert_eq!(confirmed, vec!["AdiBags".to_string()]);

        install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved,
            &adibags_shaped_zip("-- v1"),
            false,
            &confirmed,
        )
        .expect("replace");
        assert!(!addon_dir.join("AdiBags").join("old.lua").exists());
        assert!(addon_dir.join("AdiBags").join("AdiBags.toc").exists());
        let tracked = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("linked");
        assert_eq!(tracked.source, SourceKind::GitHub);
        assert_eq!(
            std::fs::read_to_string(wtf.join("AdiBags.lua")).expect("wtf kept"),
            "settings"
        );
    }

    #[test]
    fn install_never_replaces_a_folder_of_a_linked_addon() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(addon_dir.join("AdiBags")).expect("linked copy");
        std::fs::write(addon_dir.join("AdiBags").join("old.lua"), "-- old").expect("old file");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut linked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::Wago);
        linked.source_url = Some("https://addons.wago.io/addons/adibags".to_string());
        database.upsert_addon(&linked).expect("linked row");

        // A dry run is refused as the install it stands for would be.
        let dry_run = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved_repo("AdiAddons", "AdiBags"),
            &adibags_shaped_zip("-- v1"),
            true,
            &[],
        )
        .expect_err("a dry run over a linked addon");
        assert_eq!(
            dry_run.kind,
            crate::source_error::SourceErrorKind::FoldersOfLinkedAddon
        );
        assert!(addon_dir.join("AdiBags").join("old.lua").exists());
        assert!(!addon_dir.join("AdiBags_Config").exists());

        let refused = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved_repo("AdiAddons", "AdiBags"),
            &adibags_shaped_zip("-- v1"),
            false,
            &["AdiBags".to_string()],
        )
        .expect_err("belongs to a linked addon");
        assert_eq!(
            refused.kind,
            crate::source_error::SourceErrorKind::FoldersOfLinkedAddon
        );
        assert_eq!(
            refused.clash.expect("clash").linked,
            vec![("AdiBags".to_string(), "AdiBags".to_string())]
        );
        assert!(addon_dir.join("AdiBags").join("old.lua").exists());
        assert!(!addon_dir.join("AdiBags_Config").exists());
        let row = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("still there");
        assert_eq!(row.source, SourceKind::Wago);
    }

    #[test]
    fn install_and_update_of_a_root_toc_repo_with_a_bundled_addon_keep_both_top_level() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let resolved = resolved_repo("AdiAddons", "AdiBags");

        let summary = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &resolved,
            &adibags_shaped_zip("-- v1"),
            false,
            &[],
        )
        .expect("install");
        assert_eq!(summary.parent_folder, "AdiBags");
        assert_eq!(
            addon_dir_listing(&addon_dir),
            vec!["AdiBags", "AdiBags_Config"]
        );
        assert!(
            addon_dir
                .join("AdiBags")
                .join("Libs")
                .join("LibStub")
                .exists()
        );
        let tracked = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert!(tracked.has_authoritative_owned_folders());
        assert_eq!(
            tracked
                .owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["AdiBags_Config"]
        );
        assert!(
            database
                .get_addon_by_folder("AdiBags_Config")
                .expect("get row")
                .is_none(),
            "the bundled addon is owned, not tracked separately"
        );

        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved,
            &adibags_shaped_zip("-- v2"),
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        )
        .expect("update");

        assert!(summary.updated);
        assert_eq!(summary.parent_folder, "AdiBags");
        assert_eq!(
            addon_dir_listing(&addon_dir),
            vec!["AdiBags", "AdiBags_Config"]
        );
        for file in ["AdiBags/core.lua", "AdiBags_Config/Options.lua"] {
            assert_eq!(
                std::fs::read_to_string(addon_dir.join(file)).expect("read"),
                "-- v2",
                "{file} updated"
            );
        }
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(
            stored
                .owned_folders
                .iter()
                .map(|owned| owned.name.as_str())
                .collect::<Vec<_>>(),
            vec!["AdiBags_Config"]
        );
    }

    #[test]
    fn update_of_a_migrated_row_with_a_manual_config_folder_takes_it_over() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        for folder in ["AdiBags", "AdiBags_Config"] {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(addon_dir.join(folder).join("stale.lua"), "old").expect("stale");
        }
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut tracked = AddonRecord::new("AdiBags", "AdiBags", SourceKind::GitHub);
        tracked.version = Some("1.0.0".to_string());
        tracked.git_commit = Some("abcdef0".to_string());
        tracked.source_url = Some("https://github.com/AdiAddons/AdiBags".to_string());
        database.upsert_addon(&tracked).expect("seed addon");
        database
            .upsert_addon(&AddonRecord::new(
                "AdiBags_Config",
                "AdiBags_Config",
                SourceKind::Manual,
            ))
            .expect("seed manual row");

        update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &resolved_repo("AdiAddons", "AdiBags"),
            &adibags_shaped_zip("-- new"),
            // The person said yes to replacing the Not linked folders.
            crate::update_targets::ReplaceNotLinked::Any,
            false,
        )
        .expect("update");

        assert_eq!(
            addon_dir_listing(&addon_dir),
            vec!["AdiBags", "AdiBags_Config"]
        );
        assert!(!addon_dir.join("AdiBags_Config").join("stale.lua").exists());
        assert!(
            addon_dir
                .join("AdiBags_Config")
                .join("Options.lua")
                .exists()
        );
        let stored = database
            .get_addon_by_folder("AdiBags")
            .expect("get addon")
            .expect("addon exists");
        assert!(stored.has_authoritative_owned_folders());
        assert!(
            database
                .get_addon_by_folder("AdiBags_Config")
                .expect("get")
                .is_none()
        );
    }

    fn released_repo(owner: &str, repo: &str, tag: &str, asset: &str) -> super::GithubResolvedRepo {
        super::GithubResolvedRepo {
            package: super::GithubPackage::Release {
                tag: tag.to_string(),
                asset: lemonup_core::github_release::ReleaseAsset {
                    name: asset.to_string(),
                    download_url: format!(
                        "https://github.com/{owner}/{repo}/releases/download/{tag}/{asset}"
                    ),
                },
            },
            ..resolved_repo(owner, repo)
        }
    }

    fn forever_addon_dir(root: &std::path::Path) -> std::path::PathBuf {
        let addon_dir = root.join("_forever_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("addon dir");
        addon_dir
    }

    /// The latest release of openmap-world/openmap as the API lists it (v0.8.10), plus a file
    /// whose download is not on github.com.
    const OPENMAP_RELEASE: &str = r#"{
        "tag_name": "v0.8.10",
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "OpenMap-v0.8.10-forever.zip", "browser_download_url": "https://github.com/openmap-world/openmap/releases/download/v0.8.10/OpenMap-v0.8.10-forever.zip"},
            {"name": "OpenMap-v0.8.10-retail.zip", "browser_download_url": "https://github.com/openmap-world/openmap/releases/download/v0.8.10/OpenMap-v0.8.10-retail.zip"},
            {"name": "OpenMap-mirror.zip", "browser_download_url": "https://example.com/OpenMap-mirror.zip"}
        ]
    }"#;

    #[test]
    fn a_release_offers_the_zip_for_the_game_and_only_github_downloads() {
        let release: super::GithubReleaseResponse =
            serde_json::from_str(OPENMAP_RELEASE).expect("release parses");
        let assets = release.assets();
        assert_eq!(assets.len(), 2, "the off-site file is dropped");

        let forever = super::release_package(&release, &assets, GameFlavor::Forever, None)
            .expect("a zip for Forever");
        assert_eq!(forever.release_tag(), Some("v0.8.10"));
        assert_eq!(forever.remote_version(), "v0.8.10");
        assert_eq!(forever.git_commit(), None);
        let super::GithubPackage::Release { asset, .. } = forever else {
            panic!("expected a release package");
        };
        assert_eq!(asset.name, "OpenMap-v0.8.10-forever.zip");

        let retail = super::release_package(&release, &assets, GameFlavor::Retail, None)
            .expect("a zip for Retail");
        let super::GithubPackage::Release { asset, .. } = retail else {
            panic!("expected a release package");
        };
        assert_eq!(asset.name, "OpenMap-v0.8.10-retail.zip");
    }

    #[test]
    fn a_release_json_that_fails_to_download_fails_instead_of_guessing() {
        let failed = super::read_release_json(Err(crate::source_error::SourceError::new(
            crate::source_error::SourceErrorKind::Other,
            "timed out".to_string(),
        )));
        assert_eq!(failed.expect_err("propagated").detail, "timed out");
        assert_eq!(
            super::read_release_json(Ok(b"<html>not json</html>".to_vec())).expect("read"),
            None
        );
        assert!(
            super::read_release_json(Ok(br#"{"releases":[]}"#.to_vec()))
                .expect("read")
                .is_some()
        );
    }

    #[test]
    fn only_https_github_com_downloads_are_used() {
        assert!(super::is_github_download_url(
            "https://github.com/o/r/releases/download/v1/Foo.zip"
        ));
        for url in [
            "http://github.com/o/r/releases/download/v1/Foo.zip",
            "https://api.github.com/o/r/Foo.zip",
            "https://github.com.example.com/Foo.zip",
            "https://example.com/Foo.zip",
            "not a url",
        ] {
            assert!(!super::is_github_download_url(url), "{url}");
        }
    }

    #[test]
    fn drafts_prereleases_and_releases_without_a_fitting_zip_mean_the_source() {
        let mut release: super::GithubReleaseResponse =
            serde_json::from_str(OPENMAP_RELEASE).expect("release parses");
        let assets = release.assets();
        release.prerelease = true;
        assert_eq!(
            super::release_package(&release, &assets, GameFlavor::Retail, None),
            None
        );
        release.prerelease = false;
        release.draft = true;
        assert_eq!(
            super::release_package(&release, &assets, GameFlavor::Retail, None),
            None
        );
        release.draft = false;
        let classic_only = vec![lemonup_core::github_release::ReleaseAsset {
            name: "OpenMap-v0.8.10-classic.zip".to_string(),
            download_url: "https://github.com/openmap-world/openmap/releases/download/v0.8.10/OpenMap-v0.8.10-classic.zip".to_string(),
        }];
        assert_eq!(
            super::release_package(&release, &classic_only, GameFlavor::Retail, None),
            None
        );
    }

    #[test]
    fn a_source_install_is_never_up_to_date_with_a_release_at_its_commit() {
        let mut source_install = AddonRecord::new("OpenMap", "OpenMap", SourceKind::GitHub);
        source_install.git_commit = Some("0f4d2e4b6123456789".to_string());
        source_install.version = Some("v0.8.10".to_string());
        let release = released_repo("openmap-world", "openmap", "v0.8.10", "x.zip").package;
        let source = super::GithubPackage::Source {
            head_sha: "0f4d2e4b6123456789".to_string(),
        };
        assert!(!release.is_installed(&source_install));
        assert!(source.is_installed(&source_install));

        let mut release_install = source_install.clone();
        release_install.git_commit = None;
        release_install.release_tag = Some("v0.8.10".to_string());
        assert!(release.is_installed(&release_install));
        assert!(!source.is_installed(&release_install));
        let newer = released_repo("openmap-world", "openmap", "v0.8.11", "x.zip").package;
        assert!(!newer.is_installed(&release_install));

        // An older LemonUp updated it from source and left the tag behind.
        let mut downgraded = release_install.clone();
        downgraded.git_commit = Some("0f4d2e4b6123456789".to_string());
        assert!(!release.is_installed(&downgraded));
        assert!(source.is_installed(&downgraded));
    }

    const OPENMAP_FOREVER_TOC: &str =
        "## Interface: 16001\n## Title: OpenMap\n## Version: v0.8.10\nOpenMap.lua\n";

    #[test]
    fn install_of_a_release_zip_records_the_tag_and_the_zip_s_toc() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = forever_addon_dir(temp.path());
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        // OpenMap-v0.8.10-forever.zip: the folder at the top, no wrapper, only OpenMap.toc.
        let bytes = build_zip(&[
            ("OpenMap/OpenMap.toc", OPENMAP_FOREVER_TOC),
            ("OpenMap/OpenMap.lua", "-- map"),
        ]);

        let summary = install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &released_repo(
                "openmap-world",
                "openmap",
                "v0.8.10",
                "OpenMap-v0.8.10-forever.zip",
            ),
            &bytes,
            false,
            &[],
        )
        .expect("install");

        assert_eq!(summary.installed_folders, vec!["OpenMap".to_string()]);
        assert_eq!(summary.release_tag.as_deref(), Some("v0.8.10"));
        assert_eq!(summary.git_commit, None);
        assert_eq!(summary.version.as_deref(), Some("v0.8.10"));
        let stored = database
            .get_addon_by_folder("OpenMap")
            .expect("get")
            .expect("row");
        assert_eq!(stored.installed_release_tag(), Some("v0.8.10"));
        assert_eq!(stored.git_commit, None);
        assert_eq!(stored.remote_version.as_deref(), Some("v0.8.10"));
        assert_eq!(stored.interface.as_deref(), Some("16001"));
        assert_eq!(
            crate::update::determine_update_status(&stored).0,
            lemonup_core::UpdateStatus::UpToDate
        );
    }

    #[test]
    fn update_from_a_source_install_to_the_release_zip_takes_the_packaged_folders() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = forever_addon_dir(temp.path());
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        // A source install of BigWigs: one folder with the modules nested under their repo names.
        let source = build_zip(&[
            (
                "BigWigsMods-BigWigs-c2a8e46/BigWigs.toc",
                "## Interface: 120105\n## Version: @project-version@\n",
            ),
            (
                "BigWigsMods-BigWigs-c2a8e46/Core/BigWigs_Core.toc",
                "## Interface: 120105\n",
            ),
        ]);
        let mut source_repo = resolved_repo("BigWigsMods", "BigWigs");
        source_repo.package = super::GithubPackage::Source {
            head_sha: "c2a8e4612345678".to_string(),
        };
        install_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &source_repo,
            &source,
            false,
            &[],
        )
        .expect("source install");
        let tracked = database
            .get_addon_by_folder("BigWigs")
            .expect("get")
            .expect("row");
        assert_eq!(tracked.installed_release_tag(), None);

        // BigWigs-v427.9.zip: the packager names the modules' folders and lifts them out.
        let release = build_zip(&[
            (
                "BigWigs/BigWigs.toc",
                "## Interface: 16001, 120105\n## Version: v427.9\n",
            ),
            (
                "BigWigs_Core/BigWigs_Core.toc",
                "## Interface: 16001, 120105\n",
            ),
            (
                "BigWigs_Options/BigWigs_Options.toc",
                "## Interface: 16001, 120105\n",
            ),
        ]);
        let summary = update_downloaded_github_addon(
            &mut database,
            &addon_dir,
            &tracked,
            &released_repo("BigWigsMods", "BigWigs", "v427.9", "BigWigs-v427.9.zip"),
            &release,
            crate::update_targets::ReplaceNotLinked::NONE,
            false,
        )
        .expect("update to the release zip");

        assert_eq!(
            summary.installed_folders,
            vec![
                "BigWigs".to_string(),
                "BigWigs_Core".to_string(),
                "BigWigs_Options".to_string()
            ]
        );
        assert_eq!(summary.remote_version.as_deref(), Some("v427.9"));
        assert!(!addon_dir.join("BigWigs").join("Core").exists());
        assert!(
            addon_dir
                .join("BigWigs_Core")
                .join("BigWigs_Core.toc")
                .exists()
        );
        let stored = database
            .get_addon_by_folder("BigWigs")
            .expect("get")
            .expect("row");
        assert_eq!(stored.installed_release_tag(), Some("v427.9"));
        assert_eq!(stored.version.as_deref(), Some("v427.9"));
        let owned = stored
            .owned_folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(owned, vec!["BigWigs_Core", "BigWigs_Options"]);
    }
}
