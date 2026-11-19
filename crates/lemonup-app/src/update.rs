use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;

use lemonup_core::{
    AddonRecord, GameFlavor, LemonupError, SourceKind, StateDatabase, UpdateStatus,
};
#[cfg(test)]
use lemonup_core::{ScannedAddon, scan_addons_dir};
use time::OffsetDateTime;

use crate::update_targets::ReplaceNotLinked;

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
    /// Held because the update would replace "Not linked" folders nobody agreed to.
    pub(crate) held: usize,
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
        self.held += other.held;
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
    /// Not updated yet: it would replace "Not linked" folders nobody agreed to.
    Held,
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

#[derive(Debug, Default)]
pub(crate) struct LiveUpdateRun {
    pub(crate) summary: LiveUpdateSummary,
    pub(crate) results: Vec<LiveUpdateResult>,
    /// Updates held for a yes, with their downloaded packages kept.
    pub(crate) held: Vec<HeldUpdate>,
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

/// Records what a check found online for `addon` and says what it means for the row as it is
/// stored now. Only the check's own fields are written, and only onto a row that still is
/// `addon` (same folder, source and installed version): a row that was deleted, linked to
/// another source or updated while the check waited for the network is left as it is, and
/// has no result. `source_name` is the addon's name as the source gives it; without one the
/// stored name stays.
fn record_check(
    database: &StateDatabase,
    addon: &AddonRecord,
    remote_version: Option<String>,
    source_url: Option<String>,
    source_name: Option<String>,
) -> Result<Option<CheckResult>, LemonupError> {
    let source_name = source_name
        .as_deref()
        .and_then(lemonup_core::domain::source_name);
    let Some(stored) = database.record_check_result(
        addon,
        remote_version.as_deref(),
        source_url.as_deref(),
        source_name.as_deref(),
        OffsetDateTime::now_utc(),
    )?
    else {
        return Ok(None);
    };
    let (status, message) = determine_update_status(&stored);
    Ok(Some(CheckResult {
        addon_name: stored.folder.clone(),
        status,
        remote_version: stored.remote_version.clone(),
        message,
        error_kind: None,
    }))
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
                        results.extend(record_check(
                            database,
                            &addon,
                            remote.version,
                            remote.source_url,
                            remote.name,
                        )?);
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
                    results.extend(record_check(
                        database,
                        &addon,
                        remote.version,
                        remote.source_url,
                        remote.name,
                    )?);
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
                        results.extend(record_check(
                            database,
                            &addon,
                            remote.version,
                            remote.source_url,
                            remote.name,
                        )?);
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
            SourceKind::GitHub => match crate::github::fetch_github_remote_version(
                &addon,
                addon_dir.map_or(addon.flavor, GameFlavor::for_addons_dir),
            )
            .await
            {
                Ok(remote) => {
                    // GitHub gives only a repository name, so GitHub addons keep their TOC Title.
                    results.extend(record_check(
                        database,
                        &addon,
                        remote.version,
                        remote.source_url,
                        None,
                    )?);
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

/// Updates the selected addons one at a time. An addon whose new package would replace "Not
/// linked" folders is held (see [`HeldUpdate`]) unless `replace_not_linked` agrees to replace
/// them; the held updates come back in `LiveUpdateRun::held` with their packages kept.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_live_updates(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selectors: &[String],
    wago_api_key: Option<&str>,
    force: bool,
    dry_run: bool,
    replace_not_linked: bool,
    mut on_progress: impl FnMut(usize, usize, String),
) -> Result<LiveUpdateRun, LemonupError> {
    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let agree = if replace_not_linked {
        ReplaceNotLinked::Any
    } else {
        ReplaceNotLinked::NONE
    };

    let mut summary = LiveUpdateSummary {
        target_addons: selected.len(),
        ..LiveUpdateSummary::default()
    };
    let mut results = Vec::with_capacity(selected.len());
    let mut held = Vec::new();

    let total = selected.len();
    for (index, addon) in selected.into_iter().enumerate() {
        on_progress(index + 1, total, addon.folder.clone());
        let outcome = match addon.source {
            SourceKind::Manual => {
                summary.skipped_manual += 1;
                results.push(LiveUpdateResult {
                    addon_name: addon.folder.clone(),
                    source: addon.source,
                    status: LiveUpdateStatus::SkippedManual,
                    previous_version: addon.version.clone(),
                    remote_version: addon.remote_version.clone(),
                    message: Some("addons that are not linked cannot be updated yet".to_string()),
                });
                continue;
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
                crate::wago::update_wago_addon(
                    database, addon_dir, &addon, api_key, force, dry_run, agree,
                )
                .await
                .map(|(result, package)| (result.outcome(), package))
            }
            SourceKind::Tukui => {
                crate::tukui::update_tukui_addon(database, addon_dir, &addon, force, dry_run, agree)
                    .await
                    .map(|(result, package)| (result.outcome(), package))
            }
            SourceKind::WowInterface => crate::wowinterface::update_wowinterface_addon(
                database, addon_dir, &addon, force, dry_run, agree,
            )
            .await
            .map(|(result, package)| (result.outcome(), package)),
            SourceKind::GitHub => crate::github::update_github_addon(
                database, addon_dir, &addon, force, dry_run, agree,
            )
            .await
            .map(|(result, package)| (result.outcome(), package)),
        };
        record_provider_result(
            &mut summary,
            &mut results,
            &mut held,
            &addon,
            addon_dir,
            outcome,
        );
    }

    Ok(LiveUpdateRun {
        summary,
        results,
        held,
    })
}

/// What finishing a provider update from a downloaded package did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinishedUpdate {
    pub(crate) previous_version: Option<String>,
    pub(crate) remote_version: Option<String>,
    /// "Not linked" folders the package would replace that nobody agreed to; the update did
    /// nothing while this is not empty.
    pub(crate) not_linked: Vec<String>,
    pub(crate) warning: Option<String>,
}

type FinishHeld = Box<
    dyn FnOnce(
            &mut StateDatabase,
            &Path,
            &AddonRecord,
            &[u8],
            ReplaceNotLinked<'_>,
        ) -> Result<FinishedUpdate, String>
        + Send,
>;

/// A downloaded package whose update waits for a yes, kept in its own temp folder until the
/// update finishes or is skipped. Dropping it removes the folder.
pub(crate) struct HeldPackage {
    dir: tempfile::TempDir,
    finish: FinishHeld,
}

impl std::fmt::Debug for HeldPackage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HeldPackage")
            .field("dir", &self.dir.path())
            .finish_non_exhaustive()
    }
}

const HELD_PACKAGE_FILE: &str = "package.zip";

impl HeldPackage {
    /// Keeps `bytes` on disk; `finish` runs the provider's update from them later, with the
    /// provider's resolved data it captured.
    pub(crate) fn keep(
        bytes: &[u8],
        finish: impl FnOnce(
            &mut StateDatabase,
            &Path,
            &AddonRecord,
            &[u8],
            ReplaceNotLinked<'_>,
        ) -> Result<FinishedUpdate, String>
        + Send
        + 'static,
    ) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix("lemonup-held-update-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        std::fs::write(dir.path().join(HELD_PACKAGE_FILE), bytes)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            dir,
            finish: Box::new(finish),
        })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Runs the update from the kept package, then removes it.
    fn finish(
        self,
        database: &mut StateDatabase,
        addon_dir: &Path,
        tracked: &AddonRecord,
        agree: ReplaceNotLinked<'_>,
    ) -> Result<FinishedUpdate, String> {
        let Self { dir, finish } = self;
        let outcome = std::fs::read(dir.path().join(HELD_PACKAGE_FILE))
            .map_err(|error| error.to_string())
            .and_then(|bytes| finish(database, addon_dir, tracked, &bytes, agree));
        let _ = dir.close();
        outcome
    }

    /// Removes the kept package now.
    fn discard(self) {
        let _ = self.dir.close();
    }
}

/// An update held back because its package would replace "Not linked" folders nobody agreed
/// to. Nothing on disk or in the database changed for it.
#[derive(Debug)]
pub(crate) struct HeldUpdate {
    /// The linked addon's folder, as `LiveUpdateResult::addon_name`.
    pub(crate) folder: String,
    /// The name shown for the addon.
    pub(crate) name: String,
    pub(crate) source: SourceKind,
    /// Every "Not linked" folder the update would replace.
    pub(crate) folders: Vec<String>,
    /// Those of `folders` that hold a `.git` entry (likely a developer's copy).
    pub(crate) dev_copies: Vec<String>,
    previous_version: Option<String>,
    remote_version: Option<String>,
    package: HeldPackage,
}

impl HeldUpdate {
    pub(crate) fn new(
        tracked: &AddonRecord,
        addon_dir: &Path,
        finished: &FinishedUpdate,
        package: HeldPackage,
    ) -> Self {
        let name = tracked.name.trim();
        Self {
            folder: tracked.folder.clone(),
            name: if name.is_empty() {
                tracked.folder.clone()
            } else {
                name.to_string()
            },
            source: tracked.source,
            dev_copies: dev_copies(addon_dir, &finished.not_linked),
            folders: finished.not_linked.clone(),
            previous_version: finished.previous_version.clone(),
            remote_version: finished.remote_version.clone(),
            package,
        }
    }

    #[cfg(test)]
    pub(crate) fn package_path(&self) -> &Path {
        self.package.path()
    }

    /// Finishes the update from the kept package, replacing exactly the folders the person
    /// agreed to, then removes the package.
    pub(crate) fn apply(self, database: &mut StateDatabase, addon_dir: &Path) -> LiveUpdateResult {
        let mut result = LiveUpdateResult {
            addon_name: self.folder.clone(),
            source: self.source,
            status: LiveUpdateStatus::Error,
            previous_version: self.previous_version.clone(),
            remote_version: self.remote_version.clone(),
            message: None,
        };
        let tracked = match database.list_addons() {
            Ok(rows) => rows.into_iter().find(|row| row.folder == self.folder),
            Err(error) => {
                self.package.discard();
                result.message = Some(error.to_string());
                return result;
            }
        };
        let Some(tracked) = tracked.filter(|row| row.source == self.source) else {
            self.package.discard();
            result.message = Some(format!("{} is no longer linked", self.name));
            return result;
        };
        match self.package.finish(
            database,
            addon_dir,
            &tracked,
            ReplaceNotLinked::Only(&self.folders),
        ) {
            Ok(finished) if finished.not_linked.is_empty() => {
                result.status = LiveUpdateStatus::Updated;
                result.previous_version = finished.previous_version;
                result.remote_version = finished.remote_version;
                result.message = finished.warning;
            }
            Ok(finished) => {
                result.message = Some(format!(
                    "it would also replace these Not linked folders: {}. Press u to update it again.",
                    finished.not_linked.join(", ")
                ));
            }
            Err(error) => result.message = Some(error),
        }
        result
    }

    /// Skips the update and removes the kept package.
    pub(crate) fn discard(self) {
        self.package.discard();
    }
}

fn dev_copies(addon_dir: &Path, folders: &[String]) -> Vec<String> {
    folders
        .iter()
        .filter(|folder| crate::update_targets::is_dev_copy(addon_dir, folder))
        .cloned()
        .collect()
}

/// The CLI and status words for the folders an update would replace: `A, B (has .git, likely a
/// dev copy)`.
pub(crate) fn describe_not_linked_folders(folders: &[String], dev_copies: &[String]) -> String {
    folders
        .iter()
        .map(|folder| {
            if dev_copies.contains(folder) {
                format!("{folder} (has .git, likely a dev copy)")
            } else {
                folder.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Applies updates the person agreed to after they were held, one at a time, from their kept
/// packages (no download).
pub(crate) fn apply_held_updates(
    database: &mut StateDatabase,
    addon_dir: &Path,
    held: Vec<HeldUpdate>,
) -> LiveUpdateRun {
    let mut summary = LiveUpdateSummary {
        target_addons: held.len(),
        ..LiveUpdateSummary::default()
    };
    let mut results = Vec::with_capacity(held.len());
    for update in held {
        let result = update.apply(database, addon_dir);
        match result.status {
            LiveUpdateStatus::Updated => summary.updated_addons += 1,
            _ => summary.errors += 1,
        }
        results.push(result);
    }
    LiveUpdateRun {
        summary,
        results,
        held: Vec::new(),
    }
}

/// Records one provider result: held back (with its kept package unless `dry_run`), updated,
/// already up to date, newer than the source, or failed.
fn record_provider_result(
    summary: &mut LiveUpdateSummary,
    results: &mut Vec<LiveUpdateResult>,
    held: &mut Vec<HeldUpdate>,
    addon: &AddonRecord,
    addon_dir: &Path,
    outcome: Result<(ProviderOutcome, Option<HeldPackage>), String>,
) {
    let base = |status, previous_version, remote_version, message| LiveUpdateResult {
        addon_name: addon.folder.clone(),
        source: addon.source,
        status,
        previous_version,
        remote_version,
        message,
    };
    match outcome {
        Ok((outcome, package)) if !outcome.finished.not_linked.is_empty() => {
            summary.held += 1;
            let dev = dev_copies(addon_dir, &outcome.finished.not_linked);
            results.push(base(
                LiveUpdateStatus::Held,
                outcome.finished.previous_version.clone(),
                outcome.finished.remote_version.clone(),
                Some(format!(
                    "would replace Not linked folders: {}",
                    describe_not_linked_folders(&outcome.finished.not_linked, &dev)
                )),
            ));
            if let Some(package) = package {
                held.push(HeldUpdate::new(
                    addon,
                    addon_dir,
                    &outcome.finished,
                    package,
                ));
            }
        }
        Ok((outcome, _)) if outcome.remote_older => {
            summary.remote_older += 1;
            results.push(remote_older_result(
                addon,
                outcome.finished.previous_version,
                outcome.finished.remote_version,
            ));
        }
        Ok((outcome, _)) if outcome.updated => {
            summary.updated_addons += 1;
            results.push(base(
                LiveUpdateStatus::Updated,
                outcome.finished.previous_version,
                outcome.finished.remote_version,
                outcome.finished.warning,
            ));
        }
        Ok((outcome, _)) => {
            summary.up_to_date += 1;
            results.push(base(
                LiveUpdateStatus::UpToDate,
                outcome.finished.previous_version,
                outcome.finished.remote_version,
                Some("remote package already matches installed version".to_string()),
            ));
        }
        Err(error) => {
            summary.errors += 1;
            results.push(base(
                LiveUpdateStatus::Error,
                addon.version.clone(),
                addon.remote_version.clone(),
                Some(error),
            ));
        }
    }
}

/// A provider update result in the shape every provider shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderOutcome {
    pub(crate) updated: bool,
    pub(crate) remote_older: bool,
    pub(crate) finished: FinishedUpdate,
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

/// Saves what an update that installed nothing (remote older or already current) learned from
/// the provider, the same way a check does: the fresh remote version, the source URL, the
/// addon's name at the source (`source_name`, as the source gives it; without one the stored
/// name stays) and the check time. Without it the row keeps an older remote version and can
/// show an update again.
///
/// A dry run writes nothing, and a row removed in the meantime is not recreated.
pub(crate) fn record_skipped_update_check(
    database: &StateDatabase,
    tracked: &AddonRecord,
    remote_version: Option<&str>,
    source_url: Option<String>,
    source_name: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    let Some(mut refreshed) = database
        .get_addon_by_folder(&tracked.folder)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    refreshed.remote_version = remote_version.map(str::to_string);
    refreshed.source_url = source_url.or(refreshed.source_url);
    refreshed.source_name = source_name
        .and_then(lemonup_core::domain::source_name)
        .or(refreshed.source_name);
    refreshed.last_checked_at = Some(OffsetDateTime::now_utc());
    database
        .upsert_addon(&refreshed)
        .map_err(|error| error.to_string())
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
            Some("addons that are not linked cannot be checked yet".to_string()),
        );
    }

    if addon.source == lemonup_core::SourceKind::GitHub {
        return determine_github_update_status(addon);
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
            Some("the source has a different version than the installed one".to_string()),
        ),
        (_, None) => (UpdateStatus::Unknown, Some("not checked yet".to_string())),
        (None, Some(_)) => (
            UpdateStatus::Unknown,
            Some("the installed version is unknown".to_string()),
        ),
    }
}

/// GitHub addons compare what is installed with what an update would install: a release tag
/// when the repo's latest release has a zip for the game (see `github::GithubPackage`), else the
/// head commit. A source install is never up to date with a release, so it moves to the release
/// zip even when the tag points at the installed commit.
fn determine_github_update_status(addon: &AddonRecord) -> (UpdateStatus, Option<String>) {
    let Some(remote) = addon
        .remote_version
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return (UpdateStatus::Unknown, Some("not checked yet".to_string()));
    };

    if let Some(installed_tag) = addon.installed_release_tag() {
        return if installed_tag.trim() == remote.trim() {
            (UpdateStatus::UpToDate, None)
        } else {
            (
                UpdateStatus::UpdateAvailable,
                Some("the source has a different version than the installed one".to_string()),
            )
        };
    }

    // A release tag for a source install, even one whose commit is unknown (v1 rows). A tag
    // that looks like a commit (`20241118`) never matches the installed one, so it is Update
    // ready below all the same.
    if !is_commit_hash(remote) {
        return (
            UpdateStatus::UpdateAvailable,
            Some("the source has a release for this game".to_string()),
        );
    }
    match installed_github_commit(addon) {
        None => (
            UpdateStatus::Unknown,
            Some("the installed commit is unknown".to_string()),
        ),
        Some(installed) if crate::github::commits_match(Some(installed), Some(remote)) => {
            (UpdateStatus::UpToDate, None)
        }
        Some(_) => (
            UpdateStatus::UpdateAvailable,
            Some("the source has a different commit than the installed one".to_string()),
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
        LiveUpdateStatus::Held => "held",
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
        .collect::<std::collections::HashMap<_, _>>();

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
        toc_overrides: existing.toc_overrides.clone(),
        source_name: existing.source_name.clone(),
        release_tag: existing.release_tag.clone(),
        unrecognized: existing.unrecognized.clone(),
    };
    refreshed.set_managed_owned_folders(existing.owned_folders.clone());
    refreshed
}

pub(crate) fn resolve_selected_addons<'a>(
    installed: &'a [AddonRecord],
    selectors: &[String],
) -> Result<Vec<&'a AddonRecord>, LemonupError> {
    if selectors.is_empty() {
        return Ok(installed.iter().collect());
    }

    let mut selected = Vec::new();
    let mut seen_folders = HashSet::new();
    let mut missing = Vec::new();

    for selector in selectors {
        let Some(addon) = resolve_one_addon(installed, selector)? else {
            missing.push(selector.clone());
            continue;
        };

        if seen_folders.insert(addon.folder.clone()) {
            selected.push(addon);
        }
    }

    if !missing.is_empty() {
        let noun = if missing.len() == 1 {
            "No linked addon matches"
        } else {
            "No linked addons match"
        };
        return Err(LemonupError::InvalidArgument(format!(
            "{noun}: {}. Use exact addon names or folder names, or run `lemonup check` to see the overall summary.",
            missing.join(", ")
        )));
    }

    Ok(selected)
}

/// The addon `selector` names, ignoring case: the one whose folder it is, else the one whose
/// shown name (the source's name for a linked addon) or TOC Title it is. `None` when no addon
/// matches; an error when it names more than one addon and no folder, so a command never acts
/// on a guess.
pub(crate) fn resolve_one_addon<'a>(
    installed: &'a [AddonRecord],
    selector: &str,
) -> Result<Option<&'a AddonRecord>, LemonupError> {
    let normalized = normalize_selector(selector);
    if let Some(addon) = installed
        .iter()
        .find(|addon| normalize_selector(&addon.folder) == normalized)
    {
        return Ok(Some(addon));
    }
    let named = installed
        .iter()
        .filter(|addon| {
            normalize_selector(addon.display_name()) == normalized
                || normalize_selector(&addon.name) == normalized
        })
        .collect::<Vec<_>>();
    match named.as_slice() {
        [] => Ok(None),
        [addon] => Ok(Some(addon)),
        several => Err(LemonupError::InvalidArgument(format!(
            "More than one addon is named {}: {}. Use the folder name instead.",
            selector.trim(),
            several
                .iter()
                .map(|addon| addon.folder.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn normalize_selector(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn normalize_version(value: &str) -> String {
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
        record_skipped_update_check, refresh_live_update_checks_with, remote_is_older,
        remote_older_result, resolve_selected_addons, serialize_live_update_status,
        serialize_update_status,
    };
    use lemonup_core::{AddonRecord, SourceKind, StateDatabase, UpdateStatus};
    use std::cmp::Ordering;
    use tempfile::tempdir;

    /// `lemonup update`, `lemonup check` and `lemonup toc` select an addon by its shown name
    /// (the source's name) or its TOC Title, whatever the case.
    #[test]
    fn selectors_match_the_source_name_and_the_toc_title() {
        let mut dbm = AddonRecord::new("<DBM Core> Main Core", "DBM-Core", SourceKind::Wago);
        dbm.source_name = Some("Deadly Boss Mods (DBM)".to_string());
        let installed = vec![
            dbm,
            AddonRecord::new("Details", "Details", SourceKind::Manual),
        ];

        for selector in [
            "<DBM Core> Main Core",
            "deadly boss mods (dbm)",
            "Deadly Boss Mods (DBM)",
            "DBM-Core",
        ] {
            let selected =
                resolve_selected_addons(&installed, &[selector.to_string()]).expect(selector);
            assert_eq!(
                selected
                    .iter()
                    .map(|addon| addon.folder.as_str())
                    .collect::<Vec<_>>(),
                ["DBM-Core"],
                "{selector}"
            );
        }
        assert!(resolve_selected_addons(&installed, &["Deadly Boss".to_string()]).is_err());
    }

    /// A Not linked `Details` folder and a plugin whose Wago.io name is "Details": the folder
    /// wins, whatever the order of the rows.
    #[test]
    fn a_folder_selector_wins_over_another_addons_name() {
        let mut plugin = AddonRecord::new("Details Streamer", "Details_Streamer", SourceKind::Wago);
        plugin.source_name = Some("Details".to_string());
        let details = AddonRecord::new("Details! Damage Meter", "Details", SourceKind::Manual);
        for installed in [
            vec![plugin.clone(), details.clone()],
            vec![details.clone(), plugin.clone()],
        ] {
            let selected =
                resolve_selected_addons(&installed, &["details".to_string()]).expect("select");
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].folder, "Details");
        }
    }

    /// A name two addons have is refused for `lemonup check` and `lemonup update`, and the
    /// message names both folders; `lemonup toc` is covered in `main.rs`.
    #[tokio::test]
    async fn a_name_two_addons_have_is_refused_for_check_and_update() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let one = AddonRecord::new("Twin", "One", SourceKind::Manual);
        let mut two = AddonRecord::new("Two Toc", "Two", SourceKind::Wago);
        two.source_name = Some("Twin".to_string());
        database.upsert_addon(&one).expect("one");
        database.upsert_addon(&two).expect("two");
        let installed = database.list_addons().expect("list");
        let selectors = ["twin".to_string()];

        let check = super::build_update_checks(&installed, &selectors).expect_err("check");
        let update = apply_live_updates(
            &mut database,
            temp.path(),
            &selectors,
            None,
            false,
            true,
            false,
            |_, _, _| {},
        )
        .await
        .expect_err("update");
        for message in [check.to_string(), update.to_string()] {
            assert!(
                message.contains("More than one addon is named twin: One, Two."),
                "{message}"
            );
            assert!(message.contains("Use the folder name instead"), "{message}");
        }

        // A name only one addon has still works.
        let selected = resolve_selected_addons(&installed, &["two toc".to_string()]).expect("one");
        assert_eq!(selected[0].folder, "Two");
    }

    #[test]
    fn skipped_update_saves_the_fresh_remote_version_so_the_row_stops_offering_it() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");

        // An earlier check saw 4.4.0; the author pulled it and the provider now serves 4.3.0.
        let mut addon = AddonRecord::new("Postal", "Postal", SourceKind::WowInterface);
        addon.version = Some("4.3.5".to_string());
        addon.remote_version = Some("4.4.0".to_string());
        addon.source_url = Some("https://www.wowinterface.com/downloads/info1.html".to_string());
        database.upsert_addon(&addon).expect("seed addon");
        let addon = database
            .get_addon_by_folder("Postal")
            .expect("get")
            .expect("row");
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );

        // A dry run learns the same thing but saves nothing.
        record_skipped_update_check(&database, &addon, Some("4.3.0"), None, None, true)
            .expect("dry run");
        let unchanged = database
            .get_addon_by_folder("Postal")
            .expect("get")
            .expect("row");
        assert_eq!(unchanged, addon);

        record_skipped_update_check(
            &database,
            &addon,
            Some("4.3.0"),
            Some("https://www.wowinterface.com/downloads/info2.html".to_string()),
            None,
            false,
        )
        .expect("record");
        let stored = database
            .get_addon_by_folder("Postal")
            .expect("get")
            .expect("row");
        assert_eq!(stored.remote_version.as_deref(), Some("4.3.0"));
        assert_eq!(
            stored.source_url.as_deref(),
            Some("https://www.wowinterface.com/downloads/info2.html")
        );
        assert!(stored.last_checked_at.is_some());
        assert_eq!(stored.version, addon.version);
        assert_eq!(
            determine_update_status(&stored).0,
            UpdateStatus::RemoteOlder
        );

        // Already current: the stale remote version is replaced too, and no source URL keeps
        // the stored one.
        let mut current = stored.clone();
        current.remote_version = Some("4.4.0".to_string());
        database.upsert_addon(&current).expect("stale again");
        record_skipped_update_check(&database, &current, Some("4.3.5"), None, None, false)
            .expect("record");
        let stored = database
            .get_addon_by_folder("Postal")
            .expect("get")
            .expect("row");
        assert_eq!(stored.remote_version.as_deref(), Some("4.3.5"));
        assert_eq!(
            stored.source_url.as_deref(),
            Some("https://www.wowinterface.com/downloads/info2.html")
        );
        assert_eq!(determine_update_status(&stored).0, UpdateStatus::UpToDate);
    }

    #[test]
    fn skipped_update_does_not_recreate_a_removed_row() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let addon = AddonRecord::new("Gone", "Gone", SourceKind::Tukui);

        record_skipped_update_check(&database, &addon, Some("1.0"), None, None, false)
            .expect("record");

        assert!(database.get_addon_by_folder("Gone").expect("get").is_none());
    }

    #[test]
    fn skipped_update_fills_the_source_name_and_keeps_it_when_the_source_gives_none() {
        let temp = tempdir().expect("tempdir");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut addon = AddonRecord::new("<DBM Core> Main Core", "DBM-Core", SourceKind::Wago);
        addon.version = Some("1.0".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        record_skipped_update_check(
            &database,
            &addon,
            Some("1.0"),
            None,
            Some(" |cff00ff00Deadly|r Boss Mods &amp; Co "),
            false,
        )
        .expect("record");
        let stored = database
            .get_addon_by_folder("DBM-Core")
            .expect("get")
            .expect("row");
        assert_eq!(stored.source_name.as_deref(), Some("Deadly Boss Mods & Co"));

        record_skipped_update_check(&database, &stored, Some("1.0"), None, None, false)
            .expect("record without a name");
        let kept = database
            .get_addon_by_folder("DBM-Core")
            .expect("get")
            .expect("row");
        assert_eq!(kept.source_name.as_deref(), Some("Deadly Boss Mods & Co"));
    }

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
                        name: None,
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

    /// Addons linked before LemonUp stored source names get theirs on the next check, from
    /// the check's own response; a check whose source gives no name keeps the stored one.
    #[tokio::test]
    async fn live_checks_fill_the_source_name_and_keep_it_when_the_source_gives_none() {
        let temp = tempdir().expect("tempdir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut addon = AddonRecord::new("<DBM Core> Main Core", "DBM-Core", SourceKind::Wago);
        addon.version = Some("1.0".to_string());
        addon.source_url = Some("https://addons.wago.io/addons/dbm".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        for name in [Some("|cffff0000Deadly|r Boss Mods (DBM)"), None] {
            refresh_live_update_checks_with(
                &mut database,
                &[],
                Some("fake-key"),
                None,
                |tracked, _api_key| async move {
                    Ok(crate::wago::WagoRemoteVersion {
                        source_url: tracked.source_url.clone(),
                        version: Some("1.1".to_string()),
                        name: name.map(str::to_string),
                    })
                },
            )
            .await
            .expect("refresh checks");

            let stored = database
                .get_addon_by_folder("DBM-Core")
                .expect("get addon")
                .expect("addon exists");
            assert_eq!(
                stored.source_name.as_deref(),
                Some("Deadly Boss Mods (DBM)"),
                "{name:?}"
            );
            assert_eq!(stored.display_name(), "Deadly Boss Mods (DBM)");
            assert_eq!(stored.name, "<DBM Core> Main Core");
        }
    }

    /// Checks `WeakAuras` while `during_the_check` changes the same database, like another
    /// LemonUp would while this one waits for the network.
    async fn check_while(
        during_the_check: impl Fn(&StateDatabase) + Clone,
    ) -> (Vec<crate::update::CheckResult>, Option<AddonRecord>) {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        let mut database = StateDatabase::open(&path).expect("open db");
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.version = Some("5.20.0".to_string());
        database.upsert_addon(&addon).expect("seed addon");

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            |_tracked, _api_key| {
                let other = StateDatabase::open(&path).expect("another LemonUp");
                during_the_check(&other);
                async move {
                    Ok(crate::wago::WagoRemoteVersion {
                        source_url: None,
                        version: Some("5.21.1".to_string()),
                        name: None,
                    })
                }
            },
        )
        .await
        .expect("refresh checks");
        let stored = database
            .get_addon_by_folder("WeakAuras")
            .expect("get addon");
        (checks, stored)
    }

    #[tokio::test]
    async fn a_check_does_not_bring_back_an_addon_deleted_meanwhile() {
        let (checks, stored) = check_while(|other| {
            other.remove_addon("WeakAuras").expect("delete");
        })
        .await;
        assert_eq!(stored, None);
        assert!(checks.is_empty(), "{checks:?}");
    }

    #[tokio::test]
    async fn a_check_does_not_undo_an_update_made_meanwhile() {
        let (checks, stored) = check_while(|other| {
            let mut updated = other
                .get_addon_by_folder("WeakAuras")
                .expect("get")
                .expect("row");
            updated.version = Some("5.21.1".to_string());
            other.upsert_addon(&updated).expect("update");
        })
        .await;
        let stored = stored.expect("still there");
        assert_eq!(stored.version.as_deref(), Some("5.21.1"));
        assert_eq!(stored.last_checked_at, None);
        assert!(checks.is_empty(), "{checks:?}");
    }

    #[tokio::test]
    async fn a_check_writes_only_its_own_fields() {
        let (checks, stored) = check_while(|other| {
            let mut changed = other
                .get_addon_by_folder("WeakAuras")
                .expect("get")
                .expect("row");
            changed.author = Some("Someone".to_string());
            other.upsert_addon(&changed).expect("change");
        })
        .await;
        let stored = stored.expect("still there");
        assert_eq!(stored.author.as_deref(), Some("Someone"));
        assert_eq!(stored.remote_version.as_deref(), Some("5.21.1"));
        assert!(stored.last_checked_at.is_some());
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::UpdateAvailable);
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
                name: None,
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
                    name: None,
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
                    name: None,
                })
            },
        )
        .await
        .expect("refresh checks");

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::Error);
        assert_eq!(
            checks[0].message.as_deref(),
            Some("linked GitHub addon 'DBM-Core' has no source URL")
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
            false,
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
    fn an_update_that_would_replace_not_linked_folders_is_held_and_named() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("BigWigs_Core").join(".git")).expect("git");
        std::fs::create_dir_all(temp.path().join("BigWigs_Options")).expect("folder");
        let addon = AddonRecord::new("BigWigs", "BigWigs", SourceKind::GitHub);
        let outcome = super::ProviderOutcome {
            updated: false,
            remote_older: false,
            finished: super::FinishedUpdate {
                previous_version: Some("1".to_string()),
                remote_version: Some("2".to_string()),
                not_linked: vec!["BigWigs_Core".to_string(), "BigWigs_Options".to_string()],
                warning: None,
            },
        };
        let mut summary = super::LiveUpdateSummary::default();
        let mut results = Vec::new();
        let mut held = Vec::new();

        // A dry run keeps no package, but still reports what would be held.
        super::record_provider_result(
            &mut summary,
            &mut results,
            &mut held,
            &addon,
            temp.path(),
            Ok((outcome.clone(), None)),
        );
        assert_eq!(summary.held, 1);
        assert_eq!(summary.updated_addons, 0);
        assert!(held.is_empty());
        assert_eq!(results[0].status, super::LiveUpdateStatus::Held);
        assert_eq!(
            results[0].message.as_deref(),
            Some(
                "would replace Not linked folders: BigWigs_Core (has .git, likely a dev copy), \
                 BigWigs_Options"
            )
        );

        let package = super::HeldPackage::keep(b"zip", |_, _, _, _, _| Err("unused".to_string()))
            .expect("keep");
        super::record_provider_result(
            &mut summary,
            &mut results,
            &mut held,
            &addon,
            temp.path(),
            Ok((outcome, Some(package))),
        );
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].name, "BigWigs");
        assert_eq!(held[0].dev_copies, vec!["BigWigs_Core".to_string()]);
        assert_eq!(
            super::serialize_live_update_status(super::LiveUpdateStatus::Held),
            "held"
        );
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
    fn a_github_source_install_is_update_ready_when_the_source_has_a_release() {
        let mut addon = AddonRecord::new("OpenMap", "OpenMap", SourceKind::GitHub);
        addon.git_commit = Some("0f4d2e4b6123456789".to_string());
        addon.version = Some("v0.8.10".to_string());
        addon.remote_version = Some("v0.8.10".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::UpdateAvailable);
        assert_eq!(
            message.as_deref(),
            Some("the source has a release for this game")
        );
    }

    #[test]
    fn a_github_row_without_a_known_commit_is_update_ready_for_a_release() {
        let mut addon = AddonRecord::new("OpenMap", "OpenMap", SourceKind::GitHub);
        addon.version = Some("@project-version@".to_string());
        addon.remote_version = Some("v0.8.10".to_string());

        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );
    }

    #[test]
    fn a_github_release_install_compares_release_tags() {
        let mut addon = AddonRecord::new("BigWigs", "BigWigs", SourceKind::GitHub);
        addon.version = Some("v427.9".to_string());
        addon.release_tag = Some("v427.9".to_string());
        addon.remote_version = Some("v427.9".to_string());
        assert_eq!(determine_update_status(&addon).0, UpdateStatus::UpToDate);

        addon.remote_version = Some("v428.0".to_string());
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );

        // The release lost its zip for this game, so an update goes back to the source.
        addon.remote_version = Some("c2a8e4612345678".to_string());
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );

        addon.remote_version = None;
        assert_eq!(determine_update_status(&addon).0, UpdateStatus::Unknown);

        // A tag that looks like a commit is still compared as a tag.
        addon.release_tag = Some("20241118".to_string());
        addon.remote_version = Some("20241118".to_string());
        assert_eq!(determine_update_status(&addon).0, UpdateStatus::UpToDate);
    }

    #[test]
    fn determine_update_status_for_github_requires_real_commit_metadata() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        addon.version = Some("@project-version@".to_string());
        addon.remote_version = Some("364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string());

        let (status, message) = determine_update_status(&addon);

        assert_eq!(status, UpdateStatus::Unknown);
        assert_eq!(message.as_deref(), Some("the installed commit is unknown"));
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
            false,
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

    #[tokio::test]
    async fn addon_from_newer_version_is_checked_and_updated_like_a_not_linked_one() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut curse = AddonRecord::new("Curse", "CurseAddon", SourceKind::Manual);
        curse.source_url = Some("https://www.curseforge.com/wow/addons/curse".to_string());
        curse.version = Some("1.0.0".to_string());
        curse.remote_version = Some("2.0.0".to_string());
        curse.unrecognized = Some(lemonup_core::UnrecognizedValues {
            source: "curseforge".to_string(),
            ..Default::default()
        });
        database.upsert_addon(&curse).expect("seed curseforge row");

        let checks = refresh_live_update_checks_with(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            |_, _| async { panic!("an addon from a newer version is never fetched") },
        )
        .await
        .expect("refresh checks");
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, UpdateStatus::Unknown);
        assert_eq!(
            checks[0].message.as_deref(),
            Some("addons that are not linked cannot be checked yet")
        );

        let run = apply_live_updates(
            &mut database,
            &addon_dir,
            &[],
            Some("unused"),
            false,
            true,
            false,
            |_, _, _| {},
        )
        .await
        .expect("apply live updates");
        assert_eq!(run.summary.skipped_manual, 1);
        assert_eq!(run.results[0].status, LiveUpdateStatus::SkippedManual);

        let stored = database
            .get_addon_by_folder("CurseAddon")
            .expect("get")
            .expect("row kept");
        assert_eq!(stored.source, SourceKind::Manual);
        assert_eq!(
            stored.unrecognized.map(|values| values.source).as_deref(),
            Some("curseforge")
        );
    }
}
