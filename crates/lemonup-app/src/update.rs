use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use lemonup_core::{
    AddonRecord, GameFlavor, LemonupError, SourceKind, StateDatabase, UpdateStatus,
};
#[cfg(test)]
use lemonup_core::{ScannedAddon, scan_addons_dir};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckResult {
    pub(crate) addon_name: String,
    pub(crate) status: UpdateStatus,
    pub(crate) remote_version: Option<String>,
    pub(crate) message: Option<String>,
    /// What kind of trouble a failed live check ran into.
    pub(crate) error_kind: Option<crate::source_error::SourceErrorKind>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateRefreshSummary {
    pub(crate) target_addons: usize,
    pub(crate) scanned_addons: usize,
    pub(crate) refreshed_addons: usize,
    pub(crate) skipped_unmanaged: usize,
    pub(crate) missing_on_disk: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct LiveUpdateSummary {
    pub(crate) target_addons: usize,
    pub(crate) updated_addons: usize,
    pub(crate) up_to_date: usize,
    pub(crate) skipped_manual: usize,
    pub(crate) skipped_no_api_key: usize,
    /// Remote version is older than the installed one; not updated without `--force`.
    pub(crate) remote_older: usize,
    pub(crate) errors: usize,
}

impl LiveUpdateSummary {
    /// Adds the counts of another run, as if both ran together.
    pub(crate) fn add(&mut self, other: &Self) {
        self.target_addons += other.target_addons;
        self.updated_addons += other.updated_addons;
        self.up_to_date += other.up_to_date;
        self.skipped_manual += other.skipped_manual;
        self.skipped_no_api_key += other.skipped_no_api_key;
        self.remote_older += other.remote_older;
        self.errors += other.errors;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveUpdateStatus {
    Updated,
    UpToDate,
    SkippedManual,
    SkippedNoApiKey,
    RemoteOlder,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveUpdateResult {
    pub(crate) addon_name: String,
    pub(crate) source: SourceKind,
    pub(crate) status: LiveUpdateStatus,
    pub(crate) previous_version: Option<String>,
    pub(crate) remote_version: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveUpdateRun {
    pub(crate) summary: LiveUpdateSummary,
    pub(crate) results: Vec<LiveUpdateResult>,
}

pub(crate) fn build_update_checks(
    installed: &[AddonRecord],
    selectors: &[String],
) -> Result<Vec<CheckResult>, LemonupError> {
    let selected = resolve_selected_addons(installed, selectors)?;

    Ok(selected
        .into_iter()
        .map(|addon| {
            let (status, message) = determine_update_status(addon);
            CheckResult {
                addon_name: addon.folder.clone(),
                status,
                remote_version: addon.remote_version.clone(),
                message,
                error_kind: None,
            }
        })
        .collect())
}

pub(crate) async fn refresh_live_update_checks(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    addon_dir: Option<&Path>,
) -> Result<Vec<CheckResult>, LemonupError> {
    refresh_live_update_checks_with(
        database,
        selectors,
        wago_api_key,
        addon_dir,
        |addon, api_key| async move {
            crate::wago::fetch_wago_remote_version(&addon, &api_key).await
        },
    )
    .await
}

async fn refresh_live_update_checks_with<F, Fut>(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    addon_dir: Option<&Path>,
    fetch_wago_remote_version: F,
) -> Result<Vec<CheckResult>, LemonupError>
where
    F: Fn(AddonRecord, String) -> Fut,
    Fut: Future<Output = Result<crate::wago::WagoRemoteVersion, crate::source_error::SourceError>>,
{
    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut results = Vec::with_capacity(selected.len());

    for addon in selected {
        match addon.source {
            SourceKind::Manual => {
                let (status, message) = determine_update_status(&addon);
                results.push(CheckResult {
                    addon_name: addon.folder.clone(),
                    status,
                    remote_version: addon.remote_version.clone(),
                    message,
                    error_kind: None,
                });
            }
            SourceKind::Wago => {
                let Some(api_key) = wago_api_key else {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Unknown,
                        remote_version: addon.remote_version.clone(),
                        message: Some("Wago API key not configured".to_string()),
                        error_kind: None,
                    });
                    continue;
                };

                // Updates query Wago with the AddOns folder's flavor, so checks must too; the
                // stored record flavor is only a fallback when no AddOns dir is known.
                let mut query_addon = addon.clone();
                if let Some(addon_dir) = addon_dir {
                    query_addon.flavor = GameFlavor::for_addons_dir(addon_dir);
                }

                match fetch_wago_remote_version(query_addon, api_key.to_string()).await {
                    Ok(remote) => {
                        let mut refreshed = addon.clone();
                        refreshed.remote_version = remote.version;
                        refreshed.source_url = remote.source_url.or(refreshed.source_url);
                        refreshed.last_checked_at = Some(OffsetDateTime::now_utc());
                        database.upsert_addon(&refreshed)?;

                        let (status, message) = determine_update_status(&refreshed);
                        results.push(CheckResult {
                            addon_name: refreshed.folder.clone(),
                            status,
                            remote_version: refreshed.remote_version.clone(),
                            message,
                            error_kind: None,
                        });
                    }
                    Err(error) => {
                        results.push(CheckResult {
                            addon_name: addon.folder.clone(),
                            status: UpdateStatus::Error,
                            remote_version: addon.remote_version.clone(),
                            error_kind: Some(error.kind),
                            message: Some(error.detail),
                        });
                    }
                }
            }
            SourceKind::Tukui => match crate::tukui::fetch_tukui_remote_version(&addon).await {
                Ok(remote) => {
                    let mut refreshed = addon.clone();
                    refreshed.remote_version = remote.version;
                    refreshed.source_url = remote.source_url.or(refreshed.source_url);
                    refreshed.last_checked_at = Some(OffsetDateTime::now_utc());
                    database.upsert_addon(&refreshed)?;

                    let (status, message) = determine_update_status(&refreshed);
                    results.push(CheckResult {
                        addon_name: refreshed.folder.clone(),
                        status,
                        remote_version: refreshed.remote_version.clone(),
                        message,
                        error_kind: None,
                    });
                }
                Err(error) => {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Error,
                        remote_version: addon.remote_version.clone(),
                        error_kind: Some(error.kind),
                        message: Some(error.detail),
                    });
                }
            },
            SourceKind::WowInterface => {
                match crate::wowinterface::fetch_wowinterface_remote_version(&addon).await {
                    Ok(remote) => {
                        let mut refreshed = addon.clone();
                        refreshed.remote_version = remote.version;
                        refreshed.source_url = remote.source_url.or(refreshed.source_url);
                        refreshed.last_checked_at = Some(OffsetDateTime::now_utc());
                        database.upsert_addon(&refreshed)?;

                        let (status, message) = determine_update_status(&refreshed);
                        results.push(CheckResult {
                            addon_name: refreshed.folder.clone(),
                            status,
                            remote_version: refreshed.remote_version.clone(),
                            message,
                            error_kind: None,
                        });
                    }
                    Err(error) => {
                        results.push(CheckResult {
                            addon_name: addon.folder.clone(),
                            status: UpdateStatus::Error,
                            remote_version: addon.remote_version.clone(),
                            error_kind: Some(error.kind),
                            message: Some(error.detail),
                        });
                    }
                }
            }
            SourceKind::GitHub => match crate::github::fetch_github_remote_version(&addon).await {
                Ok(remote) => {
                    let mut refreshed = addon.clone();
                    refreshed.remote_version = remote.version;
                    refreshed.source_url = remote.source_url.or(refreshed.source_url);
                    refreshed.last_checked_at = Some(OffsetDateTime::now_utc());
                    database.upsert_addon(&refreshed)?;

                    let (status, message) = determine_update_status(&refreshed);
                    results.push(CheckResult {
                        addon_name: refreshed.folder.clone(),
                        status,
                        remote_version: refreshed.remote_version.clone(),
                        message,
                        error_kind: None,
                    });
                }
                Err(error) => {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Error,
                        remote_version: addon.remote_version.clone(),
                        error_kind: Some(error.kind),
                        message: Some(error.detail),
                    });
                }
            },
        }
    }

    Ok(results)
}

pub(crate) async fn apply_live_updates(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selectors: &[String],
    wago_api_key: Option<&str>,
    force: bool,
    dry_run: bool,
    mut on_progress: impl FnMut(usize, usize, String),
) -> Result<LiveUpdateRun, LemonupError> {
    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();

    let mut summary = LiveUpdateSummary {
        target_addons: selected.len(),
        updated_addons: 0,
        up_to_date: 0,
        skipped_manual: 0,
        skipped_no_api_key: 0,
        remote_older: 0,
        errors: 0,
    };
    let mut results = Vec::with_capacity(selected.len());

    let total = selected.len();
    for (index, addon) in selected.into_iter().enumerate() {
        on_progress(index + 1, total, addon.folder.clone());
        match addon.source {
            SourceKind::Manual => {
                summary.skipped_manual += 1;
                results.push(LiveUpdateResult {
                    addon_name: addon.folder.clone(),
                    source: addon.source,
                    status: LiveUpdateStatus::SkippedManual,
                    previous_version: addon.version.clone(),
                    remote_version: addon.remote_version.clone(),
                    message: Some("manual addons cannot be updated yet".to_string()),
                });
            }
            SourceKind::Wago => {
                // Tracked addons without authoritative ownership (for example rows migrated
                // from v1) are updated too: the provider takes the folder set from the package.
                let Some(api_key) = wago_api_key else {
                    summary.skipped_no_api_key += 1;
                    results.push(LiveUpdateResult {
                        addon_name: addon.folder.clone(),
                        source: addon.source,
                        status: LiveUpdateStatus::SkippedNoApiKey,
                        previous_version: addon.version.clone(),
                        remote_version: addon.remote_version.clone(),
                        message: Some("Wago API key not configured".to_string()),
                    });
                    continue;
                };

                match crate::wago::update_wago_addon(
                    database, addon_dir, &addon, api_key, force, dry_run,
                )
                .await
                {
                    Ok(result) if result.remote_older => {
                        summary.remote_older += 1;
                        results.push(remote_older_result(
                            &addon,
                            result.previous_version,
                            result.remote_version,
                        ));
                    }
                    Ok(result) if result.updated => {
                        summary.updated_addons += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Updated,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: None,
                        });
                    }
                    Ok(result) => {
                        summary.up_to_date += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::UpToDate,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: Some(
                                "remote package already matches installed version".to_string(),
                            ),
                        });
                    }
                    Err(error) => {
                        summary.errors += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Error,
                            previous_version: addon.version.clone(),
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
                        });
                    }
                }
            }
            SourceKind::Tukui => {
                match crate::tukui::update_tukui_addon(database, addon_dir, &addon, force, dry_run)
                    .await
                {
                    Ok(result) if result.remote_older => {
                        summary.remote_older += 1;
                        results.push(remote_older_result(
                            &addon,
                            result.previous_version,
                            result.remote_version,
                        ));
                    }
                    Ok(result) if result.updated => {
                        summary.updated_addons += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Updated,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: None,
                        });
                    }
                    Ok(result) => {
                        summary.up_to_date += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::UpToDate,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: Some(
                                "remote package already matches installed version".to_string(),
                            ),
                        });
                    }
                    Err(error) => {
                        summary.errors += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Error,
                            previous_version: addon.version.clone(),
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
                        });
                    }
                }
            }
            SourceKind::WowInterface => {
                match crate::wowinterface::update_wowinterface_addon(
                    database, addon_dir, &addon, force, dry_run,
                )
                .await
                {
                    Ok(result) if result.remote_older => {
                        summary.remote_older += 1;
                        results.push(remote_older_result(
                            &addon,
                            result.previous_version,
                            result.remote_version,
                        ));
                    }
                    Ok(result) if result.updated => {
                        summary.updated_addons += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Updated,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: result.warning,
                        });
                    }
                    Ok(result) => {
                        summary.up_to_date += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::UpToDate,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: Some(
                                "remote package already matches installed version".to_string(),
                            ),
                        });
                    }
                    Err(error) => {
                        summary.errors += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Error,
                            previous_version: addon.version.clone(),
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
                        });
                    }
                }
            }
            SourceKind::GitHub => {
                match crate::github::update_github_addon(
                    database, addon_dir, &addon, force, dry_run,
                )
                .await
                {
                    Ok(result) if result.updated => {
                        summary.updated_addons += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Updated,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: None,
                        });
                    }
                    Ok(result) => {
                        summary.up_to_date += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::UpToDate,
                            previous_version: result.previous_version,
                            remote_version: result.remote_version,
                            message: Some(
                                "remote package already matches installed version".to_string(),
                            ),
                        });
                    }
                    Err(error) => {
                        summary.errors += 1;
                        results.push(LiveUpdateResult {
                            addon_name: addon.folder.clone(),
                            source: addon.source,
                            status: LiveUpdateStatus::Error,
                            previous_version: addon.version.clone(),
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
                        });
                    }
                }
            }
        }
    }

    Ok(LiveUpdateRun { summary, results })
}

fn remote_older_result(
    addon: &AddonRecord,
    previous_version: Option<String>,
    remote_version: Option<String>,
) -> LiveUpdateResult {
    let message = remote_older_message(
        previous_version.as_deref().unwrap_or("<unknown>"),
        remote_version.as_deref().unwrap_or("<unknown>"),
    );
    LiveUpdateResult {
        addon_name: addon.folder.clone(),
        source: addon.source,
        status: LiveUpdateStatus::RemoteOlder,
        previous_version,
        remote_version,
        message: Some(message),
    }
}

fn remote_older_message(installed: &str, remote: &str) -> String {
    format!(
        "remote version {remote} is older than installed {installed}; use --force to install it anyway"
    )
}

/// Best-effort release ordering of two provider version labels.
///
/// Each side is reduced to its first dotted numeric run that starts at a token boundary: the
/// start of the string, right after a non-alphanumeric character, or right after a single
/// leading `v`/`V`. So `GatherMate2 1.50` parses as `1.50` (the `2` is glued to a word and is
/// skipped), `Pawn 2.14.3` as `2.14.3`, `Plater-v661-Retail` as `661` and
/// `DetailsRetail-v8.2.5.7229.140` as `8.2.5.7229.140`. Runs are compared component-wise as
/// integers with missing trailing components treated as zero, so `1.09` is `1.9` and compares
/// greater than `1.1`: dotted versions are read as component lists, never as decimals.
///
/// Labels of different shape are incomparable and return `None` (so nothing is reported as
/// older): a lone large integer (a build or date number such as `20261015`) against anything
/// that is not one, and labels with no boundary-aligned number at all (`r123`, `release`).
/// Two dotted labels always compare, even when one is date-style
/// (`#Details.20261015.15339.172` is greater than `DetailsRetail-v8.2.5.7229.140`): that pairing
/// is exactly the stale-remote downgrade the check exists to hold back.
pub(crate) fn compare_release_versions(installed: &str, remote: &str) -> Option<Ordering> {
    let installed = release_components(installed)?;
    let remote = release_components(remote)?;
    if is_build_number(&installed) != is_build_number(&remote) {
        return None;
    }
    let width = installed.len().max(remote.len());
    for index in 0..width {
        let left = installed.get(index).copied().unwrap_or(0);
        let right = remote.get(index).copied().unwrap_or(0);
        match left.cmp(&right) {
            Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

/// True only when both versions parse and the remote one is strictly lower. Updates that would
/// install it are held back unless forced.
pub(crate) fn remote_is_older(installed: Option<&str>, remote: Option<&str>) -> bool {
    match (installed, remote) {
        (Some(installed), Some(remote)) => {
            compare_release_versions(installed, remote) == Some(Ordering::Greater)
        }
        _ => false,
    }
}

/// A single integer at or above this is a build or date number, not a release component.
const BUILD_NUMBER_FLOOR: u64 = 10_000;

fn is_build_number(components: &[u64]) -> bool {
    matches!(components, [only] if *only >= BUILD_NUMBER_FLOOR)
}

/// Whether the digit at `index` starts a token: string start, after a non-alphanumeric byte, or
/// after a single `v`/`V` that itself starts a token.
fn starts_version_token(bytes: &[u8], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).map(|i| bytes[i]) else {
        return true;
    };
    if !previous.is_ascii_alphanumeric() {
        return true;
    }
    matches!(previous, b'v' | b'V')
        && index
            .checked_sub(2)
            .is_none_or(|i| !bytes[i].is_ascii_alphanumeric())
}

fn release_components(value: &str) -> Option<Vec<u64>> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let start = loop {
        let digit = index + bytes.get(index..)?.iter().position(u8::is_ascii_digit)?;
        if starts_version_token(bytes, digit) {
            break digit;
        }
        // Skip the whole glued run (`GatherMate2.5` must not resume at `5`).
        index = digit;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            index += 1;
        }
    };
    let mut components = Vec::new();
    for part in value[start..].split('.') {
        let digits_len = part
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(part.len());
        if digits_len == 0 {
            break;
        }
        components.push(part[..digits_len].parse::<u64>().ok()?);
        if digits_len < part.len() {
            break;
        }
    }
    (!components.is_empty()).then_some(components)
}

pub(crate) fn determine_update_status(addon: &AddonRecord) -> (UpdateStatus, Option<String>) {
    if addon.source == lemonup_core::SourceKind::Manual {
        return (
            UpdateStatus::Unknown,
            Some("manual addons cannot be checked yet".to_string()),
        );
    }

    if addon.source == lemonup_core::SourceKind::GitHub {
        return match (
            installed_github_commit(addon),
            addon
                .remote_version
                .as_deref()
                .filter(|value| is_commit_hash(value)),
        ) {
            (Some(installed_commit), Some(remote_commit))
                if crate::github::commits_match(Some(installed_commit), Some(remote_commit)) =>
            {
                (UpdateStatus::UpToDate, None)
            }
            (Some(_), Some(_)) => (
                UpdateStatus::UpdateAvailable,
                Some("tracked remote commit differs from installed commit".to_string()),
            ),
            (_, None) => (
                UpdateStatus::Unknown,
                Some("no tracked remote version yet".to_string()),
            ),
            (None, Some(_)) => (
                UpdateStatus::Unknown,
                Some("no installed Git commit metadata is tracked yet".to_string()),
            ),
        };
    }

    match (addon.version.as_deref(), addon.remote_version.as_deref()) {
        (Some(version), Some(remote_version))
            if normalize_version(version) == normalize_version(remote_version) =>
        {
            (UpdateStatus::UpToDate, None)
        }
        (Some(version), Some(remote_version))
            if remote_is_older(Some(version), Some(remote_version)) =>
        {
            (
                UpdateStatus::RemoteOlder,
                Some(remote_older_message(version, remote_version)),
            )
        }
        (Some(_), Some(_)) => (
            UpdateStatus::UpdateAvailable,
            Some("tracked remote version differs from installed version".to_string()),
        ),
        (_, None) => (
            UpdateStatus::Unknown,
            Some("no tracked remote version yet".to_string()),
        ),
        (None, Some(_)) => (
            UpdateStatus::Unknown,
            Some("no installed version metadata is tracked yet".to_string()),
        ),
    }
}

fn installed_github_commit(addon: &AddonRecord) -> Option<&str> {
    addon
        .git_commit
        .as_deref()
        .filter(|value| is_commit_hash(value))
        .or_else(|| {
            addon
                .version
                .as_deref()
                .filter(|value| is_commit_hash(value))
        })
}

fn is_commit_hash(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 7
        && trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn serialize_update_status(status: UpdateStatus) -> &'static str {
    match status {
        UpdateStatus::UpToDate => "up_to_date",
        UpdateStatus::UpdateAvailable => "update_available",
        UpdateStatus::RemoteOlder => "remote_older",
        UpdateStatus::Unknown => "unknown",
        UpdateStatus::Error => "error",
    }
}

pub(crate) fn serialize_live_update_status(status: LiveUpdateStatus) -> &'static str {
    match status {
        LiveUpdateStatus::Updated => "updated",
        LiveUpdateStatus::UpToDate => "up_to_date",
        LiveUpdateStatus::SkippedManual => "skipped_manual",
        LiveUpdateStatus::SkippedNoApiKey => "skipped_no_api_key",
        LiveUpdateStatus::RemoteOlder => "remote_older",
        LiveUpdateStatus::Error => "error",
    }
}

#[cfg(test)]
pub(crate) fn refresh_managed_update_state(
    database: &mut StateDatabase,
    addon_dir: &Path,
    dry_run: bool,
) -> lemonup_core::Result<UpdateRefreshSummary> {
    refresh_managed_update_state_for_selectors(database, addon_dir, &[], dry_run)
}

#[cfg(test)]
pub(crate) fn refresh_managed_update_state_for_selectors(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selectors: &[String],
    dry_run: bool,
) -> lemonup_core::Result<UpdateRefreshSummary> {
    let scanned = scan_addons_dir(addon_dir, GameFlavor::for_addons_dir(addon_dir))?;
    let scanned_by_folder = scanned
        .iter()
        .map(|addon| (addon.folder.as_str(), addon))
        .collect::<HashMap<_, _>>();

    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?;

    let mut refreshed_addons = 0;
    let mut skipped_unmanaged = 0;
    let mut missing_on_disk = 0;

    for addon in &selected {
        if addon.source == lemonup_core::SourceKind::Manual {
            continue;
        }

        if !addon.has_authoritative_owned_folders() {
            skipped_unmanaged += 1;
            continue;
        }

        let Some(scanned_addon) = scanned_by_folder.get(addon.folder.as_str()) else {
            missing_on_disk += 1;
            continue;
        };

        let refreshed = build_managed_update_record(addon, scanned_addon);
        if !dry_run {
            database.record_managed_addon(&refreshed)?;
        }
        refreshed_addons += 1;
    }

    Ok(UpdateRefreshSummary {
        target_addons: selected.len(),
        scanned_addons: scanned.len(),
        refreshed_addons,
        skipped_unmanaged,
        missing_on_disk,
    })
}

#[cfg(test)]
pub(crate) fn build_managed_update_record(
    existing: &AddonRecord,
    scanned: &ScannedAddon,
) -> AddonRecord {
    let mut refreshed = AddonRecord {
        id: existing.id,
        name: scanned.name.clone(),
        folder: existing.folder.clone(),
        owned_folders: existing.owned_folders.clone(),
        ownership_source: existing.effective_ownership_source(),
        kind: if existing.kind_override {
            existing.kind
        } else {
            scanned.kind
        },
        kind_override: existing.kind_override,
        flavor: scanned.flavor,
        version: scanned.version.clone(),
        git_commit: scanned.git_commit.clone(),
        author: scanned.author.clone(),
        interface: scanned.interface.clone(),
        source: existing.source,
        source_url: existing.source_url.clone(),
        required_deps: scanned.required_deps.clone(),
        optional_deps: scanned.optional_deps.clone(),
        embedded_libs: scanned.embedded_libs.clone(),
        installed_at: existing.installed_at,
        updated_at: existing.updated_at,
        last_checked_at: existing.last_checked_at,
        remote_version: existing.remote_version.clone(),
    };
    refreshed.set_managed_owned_folders(existing.owned_folders.clone());
    refreshed
}

fn resolve_selected_addons<'a>(
    installed: &'a [AddonRecord],
    selectors: &[String],
) -> Result<Vec<&'a AddonRecord>, LemonupError> {
    if selectors.is_empty() {
        return Ok(installed.iter().collect());
    }

    let by_name_or_folder = installed
        .iter()
        .flat_map(|addon| {
            [
                (normalize_selector(&addon.folder), addon),
                (normalize_selector(&addon.name), addon),
            ]
        })
        .collect::<HashMap<_, _>>();

    let mut selected = Vec::new();
    let mut seen_folders = HashSet::new();
    let mut missing = Vec::new();

    for selector in selectors {
        let normalized = normalize_selector(selector);
        let Some(addon) = by_name_or_folder.get(&normalized).copied() else {
            missing.push(selector.clone());
            continue;
        };

        if seen_folders.insert(addon.folder.clone()) {
            selected.push(addon);
        }
    }

    if !missing.is_empty() {
        let noun = if missing.len() == 1 {
            "No tracked addon matches"
        } else {
            "No tracked addons match"
        };
        return Err(LemonupError::InvalidArgument(format!(
            "{noun}: {}. Use exact addon names or folder names, or run `lemonup check` to see the overall summary.",
            missing.join(", ")
        )));
    }

    Ok(selected)
}

fn normalize_selector(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_version(value: &str) -> String {
    let trimmed = value.trim();
    let trimmed = trimmed.trim_start_matches(|character: char| !character.is_ascii_alphanumeric());
    let normalized = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .filter(|rest| rest.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .unwrap_or(trimmed);
    normalized.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        LiveUpdateStatus, apply_live_updates, compare_release_versions, determine_update_status,
        refresh_live_update_checks_with, remote_is_older, remote_older_result,
        serialize_live_update_status, serialize_update_status,
    };
    use lemonup_core::{AddonRecord, SourceKind, StateDatabase, UpdateStatus};
    use std::cmp::Ordering;
    use tempfile::tempdir;

    #[tokio::test]
    async fn live_checks_refresh_wago_remote_versions_from_mocked_provider() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.version = Some("5.20.0".to_string());
        addon.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            |tracked, api_key| {
                let tracked = tracked.clone();
                async move {
                    assert_eq!(tracked.folder, "WeakAuras");
                    assert_eq!(api_key, "fake-key");
                    Ok(crate::wago::WagoRemoteVersion {
                        source_url: tracked.source_url.clone(),
                        version: Some("5.21.1".to_string()),
                    })
                }
            },
        )
        .await
        .expect("refresh checks");

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::UpdateAvailable);
        assert_eq!(checks[0].remote_version.as_deref(), Some("5.21.1"));

        let refreshed = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert_eq!(refreshed.remote_version.as_deref(), Some("5.21.1"));
        assert!(refreshed.last_checked_at.is_some());
    }

    #[tokio::test]
    async fn live_checks_query_wago_with_the_addon_dir_flavor() {
        use lemonup_core::GameFlavor;

        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let forever_dir = temp
            .path()
            .join("World of Warcraft")
            .join("_forever_")
            .join("Interface")
            .join("AddOns");

        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.flavor = GameFlavor::Retail;
        addon.version = Some("5.20.0".to_string());
        addon.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        let fetch = |tracked: AddonRecord, _api_key: String| async move {
            let url = crate::wago::addon_details_url("VBNBxKx5", tracked.flavor);
            Ok(crate::wago::WagoRemoteVersion {
                source_url: Some(url),
                version: Some("5.21.1".to_string()),
            })
        };

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            Some("fake-key"),
            Some(&forever_dir),
            fetch,
        )
        .await
        .expect("refresh checks");
        assert_eq!(checks.len(), 1);
        let refreshed = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert!(
            refreshed
                .source_url
                .as_deref()
                .is_some_and(|url| url.ends_with("game_version=forever")),
            "checks under a _forever_ dir must query game_version=forever: {:?}",
            refreshed.source_url
        );
        // The stored record flavor is not rewritten by the check.
        assert_eq!(refreshed.flavor, GameFlavor::Retail);

        // Without an AddOns dir the stored record flavor is used.
        refresh_live_update_checks_with(&mut database, &[], Some("fake-key"), None, fetch)
            .await
            .expect("refresh checks");
        let refreshed = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon")
            .expect("addon exists");
        assert!(
            refreshed
                .source_url
                .as_deref()
                .is_some_and(|url| url.ends_with("game_version=retail"))
        );
    }

    #[tokio::test]
    async fn live_checks_report_missing_wago_api_key_without_fetching() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.version = Some("5.20.0".to_string());
        addon.source_url = Some("https://addons.wago.io/addons/VBNBxKx5".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            None,
            None,
            |_tracked, _api_key| async move {
                panic!("fetcher should not run without an API key");
                #[allow(unreachable_code)]
                Ok(crate::wago::WagoRemoteVersion {
                    source_url: None,
                    version: None,
                })
            },
        )
        .await
        .expect("refresh checks");

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::Unknown);
        assert_eq!(
            checks[0].message.as_deref(),
            Some("Wago API key not configured")
        );
    }

    #[tokio::test]
    async fn live_checks_report_github_provider_errors_when_tracking_is_incomplete() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let mut addon = AddonRecord::new("DBM", "DBM-Core", SourceKind::GitHub);
        addon.version = Some("1.0.0".to_string());
        addon.remote_version = Some("1.1.0".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            Some("unused"),
            None,
            |_tracked, _api_key| async move {
                panic!("Wago fetcher should not run for non-Wago sources");
                #[allow(unreachable_code)]
                Ok(crate::wago::WagoRemoteVersion {
                    source_url: None,
                    version: None,
                })
            },
        )
        .await
        .expect("refresh checks");

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::Error);
        assert_eq!(
            checks[0].message.as_deref(),
            Some("tracked GitHub addon 'DBM-Core' is missing a source URL")
        );
    }

    #[tokio::test]
    async fn apply_live_updates_reports_targeted_result_mix() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let manual = AddonRecord::new("Manual", "Manual", SourceKind::Manual);
        database.upsert_addon(&manual).expect("seed manual");

        let mut github = AddonRecord::new("DBM", "DBM-Core", SourceKind::GitHub);
        github.version = Some("1.0.0".to_string());
        github.remote_version = Some("1.1.0".to_string());
        database.upsert_addon(&github).expect("seed github");

        let run = apply_live_updates(
            &mut database,
            &addon_dir,
            &["Manual".to_string(), "DBM-Core".to_string()],
            Some("unused"),
            false,
            true,
            |_, _, _| {},
        )
        .await
        .expect("apply live updates");

        assert_eq!(run.summary.target_addons, 2);
        assert_eq!(run.summary.skipped_manual, 1);
        assert_eq!(run.summary.errors, 1);
        assert_eq!(run.results.len(), 2);
        assert_eq!(run.results[0].status, LiveUpdateStatus::SkippedManual);
        assert_eq!(run.results[1].status, LiveUpdateStatus::Error);
    }

    #[test]
    fn live_update_status_serializes_for_cli_output() {
        assert_eq!(
            serialize_live_update_status(LiveUpdateStatus::Updated),
            "updated"
        );
        assert_eq!(
            serialize_live_update_status(LiveUpdateStatus::SkippedNoApiKey),
            "skipped_no_api_key"
        );
    }

    #[test]
    fn determine_update_status_ignores_leading_v_for_non_github_versions() {
        let mut addon = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        addon.version = Some("v15.10".to_string());
        addon.remote_version = Some("15.10".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::UpToDate);
        assert_eq!(message, None);
    }

    #[test]
    fn determine_update_status_ignores_leading_hash_prefix_for_non_github_versions() {
        let mut addon = AddonRecord::new("Details", "Details", SourceKind::Wago);
        addon.version = Some("#Details.20260327.14812.171".to_string());
        addon.remote_version = Some("Details.20260327.14812.171".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::UpToDate);
        assert_eq!(message, None);
    }

    #[test]
    fn determine_update_status_for_github_requires_real_commit_metadata() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        addon.version = Some("@project-version@".to_string());
        addon.remote_version = Some("364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(
            message.as_deref(),
            Some("no installed Git commit metadata is tracked yet")
        );
    }

    #[test]
    fn release_comparator_flags_the_stale_wowinterface_examples_as_older() {
        for (installed, remote) in [
            ("12.1.5.3", "9.2.5.1"),
            ("v4.3.5", "v3.6.6"),
            ("v12.1.5.561-Retail", "v8.2.5.384"),
            ("Plater-v661-Retail", "v8.2.5.278"),
            (
                "#Details.20261015.15339.172",
                "DetailsRetail-v8.2.5.7229.140",
            ),
        ] {
            assert_eq!(
                compare_release_versions(installed, remote),
                Some(Ordering::Greater),
                "{installed} vs {remote}"
            );
            assert!(remote_is_older(Some(installed), Some(remote)));
        }
    }

    #[test]
    fn release_comparator_keeps_normal_upgrades_as_updates() {
        for (installed, remote) in [
            ("2.13.17", "2.14.3"),
            ("v4.3.12-release", "v4.4.1-release"),
            ("12.1.002", "12.1.003"),
            ("1.2", "1.2.1"),
        ] {
            assert_eq!(
                compare_release_versions(installed, remote),
                Some(Ordering::Less),
                "{installed} vs {remote}"
            );
            assert!(!remote_is_older(Some(installed), Some(remote)));
        }
        assert_eq!(
            compare_release_versions("1.2", "1.2.0"),
            Some(Ordering::Equal)
        );
        // Unparseable or missing sides keep today's behavior.
        assert_eq!(compare_release_versions("@project-version@", "1.0"), None);
        assert_eq!(compare_release_versions("1.0", "beta"), None);
        assert!(!remote_is_older(Some("2.0"), None));
        assert!(!remote_is_older(None, Some("1.0")));
        assert_eq!(
            compare_release_versions("99999999999999999999999", "1"),
            None
        );
    }

    #[test]
    fn release_comparator_parses_the_first_token_aligned_numeric_run() {
        for (installed, remote, expected) in [
            // The `2` in `GatherMate2` is part of the name, not the version.
            ("GatherMate2 1.50", "1.51", Some(Ordering::Less)),
            (
                "GatherMate2 1.51",
                "GatherMate2 1.51",
                Some(Ordering::Equal),
            ),
            ("Pawn 2.14.3", "2.14.3", Some(Ordering::Equal)),
            ("Pawn 2.14.3", "Pawn 2.14.4", Some(Ordering::Less)),
            (
                "Plater-v661-Retail",
                "Plater-v662-Retail",
                Some(Ordering::Less),
            ),
            (
                "DetailsRetail-v8.2.5.7229.140",
                "DetailsRetail-v8.2.5.7230.1",
                Some(Ordering::Less),
            ),
            // Dotted components compare as integers: 1.09 is 1.9, which is greater than 1.1.
            ("1.09", "1.1", Some(Ordering::Greater)),
            ("1.1", "1.09", Some(Ordering::Less)),
            // Different shapes are incomparable.
            ("r123", "v2.0", None),
            ("v2.0", "r123", None),
            ("r123", "r124", None),
            ("20261015", "1.2.3", None),
            ("1.2.3", "20261015", None),
            ("20261015", "661", None),
            ("20261015", "20261101", Some(Ordering::Less)),
            // Still ordered: the genuine stale-remote downgrades.
            ("12.1.5.3", "9.2.5.1", Some(Ordering::Greater)),
            ("v4.3.5", "v3.6.6", Some(Ordering::Greater)),
            ("v12.1.5.561-Retail", "v8.2.5.384", Some(Ordering::Greater)),
            (
                "#Details.20261015.15339.172",
                "DetailsRetail-v8.2.5.7229.140",
                Some(Ordering::Greater),
            ),
            // Ordinary updates.
            ("2.13.17", "2.14.3", Some(Ordering::Less)),
            ("v4.3.12-release", "v4.4.1-release", Some(Ordering::Less)),
            ("12.1.002", "12.1.003", Some(Ordering::Less)),
        ] {
            assert_eq!(
                compare_release_versions(installed, remote),
                expected,
                "{installed} vs {remote}"
            );
            assert_eq!(
                remote_is_older(Some(installed), Some(remote)),
                expected == Some(Ordering::Greater),
                "remote_is_older {installed} vs {remote}"
            );
        }
    }

    #[test]
    fn release_comparator_only_treats_a_single_leading_v_as_a_prefix() {
        // `v` glued to a word is not a prefix, so there is no boundary-aligned number.
        assert_eq!(compare_release_versions("Pawnv2", "1.0"), None);
        assert_eq!(
            compare_release_versions("Foo-v2.0", "Foo-v1.0"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_release_versions("vv2.0", "1.0"), None);
        assert_eq!(compare_release_versions("GatherMate2.5", "1.0"), None);
    }

    #[tokio::test]
    async fn update_all_over_a_scanned_dev_clone_skips_it_without_errors() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let clone = addon_dir.join("DevClone");
        std::fs::create_dir_all(clone.join(".git")).expect("create clone");
        std::fs::write(clone.join("DevClone.toc"), "## Title: DevClone\n").expect("write toc");
        std::fs::write(
            clone.join(".git").join("HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .expect("write HEAD");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        let scanned = lemonup_core::scan_addons_dir_with_tracked(
            &addon_dir,
            lemonup_core::GameFlavor::Retail,
            &[],
        )
        .expect("scan addons");
        database
            .reconcile_scanned_addons(&scanned)
            .expect("reconcile scanned addons");

        let run = apply_live_updates(
            &mut database,
            &addon_dir,
            &[],
            Some("unused"),
            false,
            true,
            |_, _, _| {},
        )
        .await
        .expect("apply live updates");

        assert_eq!(run.summary.target_addons, 1);
        assert_eq!(run.summary.errors, 0);
        assert_eq!(run.summary.skipped_manual, 1);
        assert_eq!(run.results[0].status, LiveUpdateStatus::SkippedManual);
    }

    #[test]
    fn determine_update_status_reports_remote_older_for_parsed_downgrades() {
        let mut addon = AddonRecord::new("RareScanner", "RareScanner", SourceKind::WowInterface);
        addon.version = Some("12.1.5.3".to_string());
        addon.remote_version = Some("9.2.5.1".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::RemoteOlder);
        assert_eq!(serialize_update_status(status), "remote_older");
        assert_eq!(
            message.as_deref(),
            Some(
                "remote version 9.2.5.1 is older than installed 12.1.5.3; use --force to install it anyway"
            )
        );

        addon.version = Some("2.13.17".to_string());
        addon.remote_version = Some("2.14.3".to_string());
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );

        // Unparseable versions keep the old "differs" behavior.
        addon.version = Some("release".to_string());
        addon.remote_version = Some("beta".to_string());
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );
    }

    #[test]
    fn determine_update_status_never_reports_remote_older_for_github_commits() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        addon.git_commit = Some("9999999aaaaaaa".to_string());
        addon.remote_version = Some("1111111bbbbbbb".to_string());

        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );
    }

    #[test]
    fn remote_older_results_carry_the_force_hint() {
        let addon = AddonRecord::new("Postal", "Postal", SourceKind::WowInterface);
        let result = remote_older_result(
            &addon,
            Some("v4.3.5".to_string()),
            Some("v3.6.6".to_string()),
        );

        assert_eq!(result.status, LiveUpdateStatus::RemoteOlder);
        assert_eq!(serialize_live_update_status(result.status), "remote_older");
        assert_eq!(
            result.message.as_deref(),
            Some(
                "remote version v3.6.6 is older than installed v4.3.5; use --force to install it anyway"
            )
        );
    }

    #[tokio::test]
    async fn wago_updates_without_an_api_key_are_skipped_not_errors_whatever_the_ownership() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        // Migrated from v1: tracked, no ownership. This used to be skipped as unmanaged.
        let mut unowned = AddonRecord::new("Pawn", "Pawn", SourceKind::Wago);
        unowned.version = Some("2.9.0".to_string());
        unowned.source_url = Some("https://addons.wago.io/addons/pawn".to_string());
        database.upsert_addon(&unowned).expect("seed unowned");
        let mut managed = AddonRecord::new("AstralKeys", "AstralKeys", SourceKind::Wago);
        managed.source_url = Some("https://addons.wago.io/addons/astralkeys".to_string());
        managed.set_managed_owned_folders(Vec::new());
        database
            .record_managed_addon(&managed)
            .expect("seed managed");

        let run = apply_live_updates(
            &mut database,
            &addon_dir,
            &[],
            None,
            false,
            false,
            |_, _, _| {},
        )
        .await
        .expect("apply live updates");

        assert_eq!(run.summary.target_addons, 2);
        assert_eq!(run.summary.skipped_no_api_key, 2);
        assert_eq!(run.summary.errors, 0);
        for result in &run.results {
            assert_eq!(result.status, LiveUpdateStatus::SkippedNoApiKey);
            assert_eq!(
                serialize_live_update_status(result.status),
                "skipped_no_api_key"
            );
            assert_eq!(
                result.message.as_deref(),
                Some("Wago API key not configured")
            );
        }
    }
}
