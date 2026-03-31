use std::collections::{HashMap, HashSet};
use std::path::Path;

use lemonup_core::{AddonRecord, LemonupError, SourceKind, StateDatabase, UpdateStatus};
#[cfg(test)]
use lemonup_core::{GameFlavor, ScannedAddon, scan_addons_dir};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckResult {
    pub(crate) addon_name: String,
    pub(crate) status: UpdateStatus,
    pub(crate) remote_version: Option<String>,
    pub(crate) message: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiveUpdateSummary {
    pub(crate) target_addons: usize,
    pub(crate) updated_addons: usize,
    pub(crate) up_to_date: usize,
    pub(crate) skipped_manual: usize,
    pub(crate) skipped_unmanaged: usize,
    pub(crate) skipped_unsupported: usize,
    pub(crate) errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveUpdateStatus {
    Updated,
    UpToDate,
    SkippedManual,
    SkippedUnmanaged,
    SkippedUnsupported,
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
            }
        })
        .collect())
}

pub(crate) async fn refresh_live_update_checks(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
) -> Result<Vec<CheckResult>, LemonupError> {
    refresh_live_update_checks_with(database, selectors, wago_api_key, |addon, api_key| async move {
        crate::wago::fetch_wago_remote_version(&addon, &api_key).await
    })
    .await
}

async fn refresh_live_update_checks_with<F, Fut>(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    fetch_wago_remote_version: F,
) -> Result<Vec<CheckResult>, LemonupError>
where
    F: Fn(AddonRecord, String) -> Fut,
    Fut: Future<Output = Result<crate::wago::WagoRemoteVersion, String>>,
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
                });
            }
            SourceKind::Wago => {
                let Some(api_key) = wago_api_key else {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Unknown,
                        remote_version: addon.remote_version.clone(),
                        message: Some("Wago API key not configured".to_string()),
                    });
                    continue;
                };

                match fetch_wago_remote_version(addon.clone(), api_key.to_string()).await {
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
                        });
                    }
                    Err(error) => {
                        results.push(CheckResult {
                            addon_name: addon.folder.clone(),
                            status: UpdateStatus::Error,
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
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
                    });
                }
                Err(error) => {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Error,
                        remote_version: addon.remote_version.clone(),
                        message: Some(error),
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
                        });
                    }
                    Err(error) => {
                        results.push(CheckResult {
                            addon_name: addon.folder.clone(),
                            status: UpdateStatus::Error,
                            remote_version: addon.remote_version.clone(),
                            message: Some(error),
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
                    });
                }
                Err(error) => {
                    results.push(CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Error,
                        remote_version: addon.remote_version.clone(),
                        message: Some(error),
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
        skipped_unmanaged: 0,
        skipped_unsupported: 0,
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
                if !addon.has_authoritative_owned_folders() {
                    summary.skipped_unmanaged += 1;
                    results.push(LiveUpdateResult {
                        addon_name: addon.folder.clone(),
                        source: addon.source,
                        status: LiveUpdateStatus::SkippedUnmanaged,
                        previous_version: addon.version.clone(),
                        remote_version: addon.remote_version.clone(),
                        message: Some(
                            "tracked addon does not have authoritative managed ownership"
                                .to_string(),
                        ),
                    });
                    continue;
                }
                let Some(api_key) = wago_api_key else {
                    summary.errors += 1;
                    results.push(LiveUpdateResult {
                        addon_name: addon.folder.clone(),
                        source: addon.source,
                        status: LiveUpdateStatus::Error,
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
        UpdateStatus::Unknown => "unknown",
        UpdateStatus::Error => "error",
    }
}

pub(crate) fn serialize_live_update_status(status: LiveUpdateStatus) -> &'static str {
    match status {
        LiveUpdateStatus::Updated => "updated",
        LiveUpdateStatus::UpToDate => "up_to_date",
        LiveUpdateStatus::SkippedManual => "skipped_manual",
        LiveUpdateStatus::SkippedUnmanaged => "skipped_unmanaged",
        LiveUpdateStatus::SkippedUnsupported => "skipped_unsupported",
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
    let scanned = scan_addons_dir(addon_dir, GameFlavor::Retail)?;
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
        LiveUpdateStatus, apply_live_updates, determine_update_status,
        refresh_live_update_checks_with, serialize_live_update_status,
    };
    use lemonup_core::{AddonRecord, SourceKind, StateDatabase, UpdateStatus};
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
        assert_eq!(run.summary.skipped_unsupported, 0);
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
            serialize_live_update_status(LiveUpdateStatus::SkippedUnsupported),
            "skipped_unsupported"
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
}
