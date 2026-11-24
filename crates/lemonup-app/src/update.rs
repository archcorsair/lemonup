use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;

use lemonup_core::{
    AddonRecord, GameFlavor, LemonupError, SourceKind, StateDatabase, UpdateStatus,
};
#[cfg(test)]
use lemonup_core::{ScannedAddon, scan_addons_dir};
use time::OffsetDateTime;

use crate::source_limits::SourceLimits;
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
    /// For a held update, every "Not linked" folder it would replace. Empty otherwise.
    pub(crate) not_linked: Vec<String>,
    /// Those of `not_linked` that hold a `.git` entry (likely a developer's copy).
    pub(crate) dev_copies: Vec<String>,
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

/// What a running check reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckEvent {
    /// The request for this addon is going out now.
    Started { folder: String },
    /// This addon is done. `result` is `None` when its row changed while the check
    /// waited, so nothing was recorded for it.
    Finished {
        folder: String,
        result: Option<CheckResult>,
    },
}

/// What a source said about one addon, whatever the source.
struct RemoteCheck {
    version: Option<String>,
    source_url: Option<String>,
    /// The addon's name as the source gives it; `None` keeps the stored one.
    name: Option<String>,
}

type RemoteCheckResult = Result<RemoteCheck, crate::source_error::SourceError>;

/// Checks the selected addons online, several at once within [`SourceLimits::standard`], and
/// records what each check found. The caps hold for this run only, so a check never waits
/// behind another one. Results come back in selector order.
pub(crate) async fn refresh_live_update_checks(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    addon_dir: Option<&Path>,
    on_event: impl FnMut(CheckEvent),
) -> Result<Vec<CheckResult>, LemonupError> {
    refresh_live_update_checks_in(
        database,
        selectors,
        wago_api_key,
        addon_dir,
        &SourceLimits::standard(),
        fetch_wago_check,
        on_event,
    )
    .await
}

/// The Wago.io request of a live check, as [`refresh_live_update_checks_in`] takes it.
pub(crate) async fn fetch_wago_check(
    client: reqwest::Client,
    addon: AddonRecord,
    api_key: String,
) -> Result<crate::wago::WagoRemoteVersion, crate::source_error::SourceError> {
    crate::wago::fetch_wago_remote_version(&client, &addon, &api_key).await
}

/// [`refresh_live_update_checks_in`] with its own limits and no events, for tests that only
/// look at the results.
#[cfg(test)]
async fn refresh_live_update_checks_with<F, Fut>(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    addon_dir: Option<&Path>,
    fetch_wago_remote_version: F,
) -> Result<Vec<CheckResult>, LemonupError>
where
    F: Fn(AddonRecord, String) -> Fut,
    Fut: Future<Output = Result<crate::wago::WagoRemoteVersion, crate::source_error::SourceError>>
        + Send
        + 'static,
{
    refresh_live_update_checks_in(
        database,
        selectors,
        wago_api_key,
        addon_dir,
        &SourceLimits::standard(),
        |_client, addon, api_key| fetch_wago_remote_version(addon, api_key),
        |_| {},
    )
    .await
}

/// The check engine. `fetch_wago_remote_version` builds the Wago request for one addon; it
/// runs here, before the request is handed to its own task, so it may use the database. The
/// tasks only wait for the network; every database write happens here, one at a time.
pub(crate) async fn refresh_live_update_checks_in<F, Fut>(
    database: &mut StateDatabase,
    selectors: &[String],
    wago_api_key: Option<&str>,
    addon_dir: Option<&Path>,
    limits: &SourceLimits,
    fetch_wago_remote_version: F,
    mut on_event: impl FnMut(CheckEvent),
) -> Result<Vec<CheckResult>, LemonupError>
where
    F: Fn(reqwest::Client, AddonRecord, String) -> Fut,
    Fut: Future<Output = Result<crate::wago::WagoRemoteVersion, crate::source_error::SourceError>>
        + Send
        + 'static,
{
    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut slots: Vec<Option<CheckResult>> = vec![None; selected.len()];
    let mut pending = vec![false; selected.len()];

    // Built once for the run, the first time an addon needs one: a client for the sources
    // that need no token, and the GitHub one. A client that cannot be built fails every
    // check that needed it.
    let mut general_client: Option<Result<reqwest::Client, String>> = None;
    let mut github_client: Option<Result<reqwest::Client, String>> = None;

    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut join_set = tokio::task::JoinSet::new();
    // Which addon each task checks, so a task that panics still finishes its own addon.
    let mut task_index = std::collections::HashMap::new();

    for (index, addon) in selected.iter().enumerate() {
        let api_key = match addon.source {
            SourceKind::Manual => {
                let (status, message) = determine_update_status(addon);
                let result = CheckResult {
                    addon_name: addon.folder.clone(),
                    status,
                    remote_version: addon.remote_version.clone(),
                    message,
                    error_kind: None,
                };
                finish(&mut slots, &mut on_event, index, addon, Some(result));
                continue;
            }
            SourceKind::Wago => {
                let Some(api_key) = wago_api_key else {
                    let result = CheckResult {
                        addon_name: addon.folder.clone(),
                        status: UpdateStatus::Unknown,
                        remote_version: addon.remote_version.clone(),
                        message: Some("Wago API key not configured".to_string()),
                        error_kind: None,
                    };
                    finish(&mut slots, &mut on_event, index, addon, Some(result));
                    continue;
                };
                Some(api_key)
            }
            SourceKind::Tukui | SourceKind::WowInterface | SourceKind::GitHub => None,
        };

        let client = if addon.source == SourceKind::GitHub {
            github_client.get_or_insert_with(crate::github::build_github_client)
        } else {
            general_client.get_or_insert_with(|| {
                crate::http::client_builder()
                    .build()
                    .map_err(|error| error.to_string())
            })
        };
        let client = match client {
            Ok(client) => client.clone(),
            Err(detail) => {
                // No request went out, so there is no `Started`.
                let error = crate::source_error::SourceError::from(detail.clone());
                let result = error_result(addon, error.kind, error.detail);
                finish(&mut slots, &mut on_event, index, addon, Some(result));
                continue;
            }
        };

        let tracked = addon.clone();
        let task = match (addon.source, api_key) {
            (SourceKind::Wago, Some(api_key)) => {
                // Updates query Wago with the AddOns folder's flavor, so checks must too; the
                // stored record flavor is only a fallback when no AddOns dir is known.
                let mut query_addon = tracked;
                if let Some(addon_dir) = addon_dir {
                    query_addon.flavor = GameFlavor::for_addons_dir(addon_dir);
                }
                let fetch = fetch_wago_remote_version(client, query_addon, api_key.to_string());
                spawn_check(
                    &mut join_set,
                    limits,
                    &started_tx,
                    index,
                    addon,
                    async move {
                        fetch.await.map(|remote| RemoteCheck {
                            version: remote.version,
                            source_url: remote.source_url,
                            name: remote.name,
                        })
                    },
                )
            }
            (SourceKind::Tukui, _) => spawn_check(
                &mut join_set,
                limits,
                &started_tx,
                index,
                addon,
                async move {
                    crate::tukui::fetch_tukui_remote_version(&client, &tracked)
                        .await
                        .map(|remote| RemoteCheck {
                            version: remote.version,
                            source_url: remote.source_url,
                            name: remote.name,
                        })
                },
            ),
            (SourceKind::WowInterface, _) => spawn_check(
                &mut join_set,
                limits,
                &started_tx,
                index,
                addon,
                async move {
                    crate::wowinterface::fetch_wowinterface_remote_version(&client, &tracked)
                        .await
                        .map(|remote| RemoteCheck {
                            version: remote.version,
                            source_url: remote.source_url,
                            name: remote.name,
                        })
                },
            ),
            (SourceKind::GitHub, _) => {
                let flavor = addon_dir.map_or(addon.flavor, GameFlavor::for_addons_dir);
                spawn_check(
                    &mut join_set,
                    limits,
                    &started_tx,
                    index,
                    addon,
                    async move {
                        crate::github::fetch_github_remote_version(&client, &tracked, flavor)
                            .await
                            .map(|remote| RemoteCheck {
                                version: remote.version,
                                source_url: remote.source_url,
                                // GitHub gives only a repository name, so GitHub addons keep
                                // their TOC Title.
                                name: None,
                            })
                    },
                )
            }
            // Manual rows and Wago rows without a key were finished above.
            (SourceKind::Manual, _) | (SourceKind::Wago, None) => continue,
        };
        task_index.insert(task, index);
        pending[index] = true;
    }
    // Each task holds its own sender, so the channel closes once they are all done.
    drop(started_tx);

    loop {
        tokio::select! {
            biased;
            Some(folder) = started_rx.recv() => {
                on_event(CheckEvent::Started { folder });
            }
            joined = join_set.join_next_with_id() => {
                let Some(joined) = joined else {
                    break;
                };
                // A task sends its `Started` before it ends, so it is in the channel by now
                // and goes out before that addon's `Finished`.
                while let Ok(folder) = started_rx.try_recv() {
                    on_event(CheckEvent::Started { folder });
                }
                let (index, remote) = match joined {
                    Ok((_task, (index, remote))) => (index, remote),
                    // A task that panicked or was canceled finishes its addon now, with an
                    // error. One that is somehow unknown gets its result after the loop.
                    Err(error) => {
                        let Some(&index) = task_index.get(&error.id()) else {
                            continue;
                        };
                        (index, Err(stopped_unexpectedly()))
                    }
                };
                pending[index] = false;
                let addon = &selected[index];
                let result = match remote {
                    Ok(remote) => record_check(
                        database,
                        addon,
                        remote.version,
                        remote.source_url,
                        remote.name,
                    )?,
                    Err(error) => Some(error_result(addon, error.kind, error.detail)),
                };
                finish(&mut slots, &mut on_event, index, addon, result);
            }
        }
    }

    // A safety net: every addon whose task never reported gets an error result.
    for (index, addon) in selected.iter().enumerate() {
        if pending[index] {
            let error = stopped_unexpectedly();
            let result = error_result(addon, error.kind, error.detail);
            finish(&mut slots, &mut on_event, index, addon, Some(result));
        }
    }

    Ok(slots.into_iter().flatten().collect())
}

/// Why a check whose task stopped without a result failed.
fn stopped_unexpectedly() -> crate::source_error::SourceError {
    crate::source_error::SourceError::new(
        crate::source_error::SourceErrorKind::Other,
        "the check stopped unexpectedly",
    )
}

/// The result of a check that failed for `addon`.
fn error_result(
    addon: &AddonRecord,
    kind: crate::source_error::SourceErrorKind,
    detail: String,
) -> CheckResult {
    CheckResult {
        addon_name: addon.folder.clone(),
        status: UpdateStatus::Error,
        remote_version: addon.remote_version.clone(),
        error_kind: Some(kind),
        message: Some(detail),
    }
}

/// Puts `result` in its slot and tells the caller this addon is done.
fn finish(
    slots: &mut [Option<CheckResult>],
    on_event: &mut impl FnMut(CheckEvent),
    index: usize,
    addon: &AddonRecord,
    result: Option<CheckResult>,
) {
    slots[index] = result.clone();
    on_event(CheckEvent::Finished {
        folder: addon.folder.clone(),
        result,
    });
}

/// Runs one addon's request in its own task: it waits for its permits, says it started,
/// then waits for the source. The task does no database work. Returns the task's id.
fn spawn_check(
    join_set: &mut tokio::task::JoinSet<(usize, RemoteCheckResult)>,
    limits: &SourceLimits,
    started_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    index: usize,
    addon: &AddonRecord,
    fetch: impl Future<Output = RemoteCheckResult> + Send + 'static,
) -> tokio::task::Id {
    let limits = limits.clone();
    let started_tx = started_tx.clone();
    let source = addon.source;
    let folder = addon.folder.clone();
    join_set
        .spawn(async move {
            let permit = limits.acquire(source).await;
            // The engine stops listening only when it gave up on the run; the check still ends.
            let _ = started_tx.send(folder);
            let remote = fetch.await;
            drop(permit);
            (index, remote)
        })
        .id()
}

/// What the network part of one addon's update produced.
pub(crate) enum PreparedUpdate {
    /// Nothing to install: already current, or the source lists an older version.
    Skipped {
        outcome: ProviderOutcome,
        /// What the source said, to record the way a check does.
        source_url: Option<String>,
        source_name: Option<String>,
    },
    /// The package is downloaded and waits for its turn to be applied.
    Downloaded {
        bytes: Vec<u8>,
        apply: ApplyDownloaded,
    },
}

/// Applies a downloaded package: plan, folder swap and database write, or only the plan for
/// a dry run. May be called again later for a held update.
pub(crate) type ApplyDownloaded = Box<
    dyn Fn(
            &mut StateDatabase,
            &Path,
            &AddonRecord,
            &[u8],
            ReplaceNotLinked<'_>,
            bool,
        ) -> Result<ProviderOutcome, String>
        + Send,
>;

impl PreparedUpdate {
    /// A downloaded package that `apply` installs; its last argument is `dry_run`.
    pub(crate) fn downloaded(
        bytes: Vec<u8>,
        apply: impl Fn(
            &mut StateDatabase,
            &Path,
            &AddonRecord,
            &[u8],
            ReplaceNotLinked<'_>,
            bool,
        ) -> Result<ProviderOutcome, String>
        + Send
        + 'static,
    ) -> Self {
        Self::Downloaded {
            bytes,
            apply: Box::new(apply),
        }
    }
}

/// What the network part of one addon's update gets to work with.
pub(crate) struct PrepareRequest {
    /// The addon as it was listed when the run began.
    pub(crate) addon: AddonRecord,
    pub(crate) client: reqwest::Client,
    /// The flavor of the AddOns folder being updated.
    pub(crate) flavor: GameFlavor,
    pub(crate) wago_api_key: Option<String>,
    pub(crate) force: bool,
    pub(crate) dry_run: bool,
}

/// What a running update reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateEvent {
    /// The request for this addon is going out now.
    Started { folder: String },
    /// This addon is done: applied, held, skipped or failed.
    Finished {
        folder: String,
        result: LiveUpdateResult,
    },
}

/// At most this many addons are downloading, or downloaded and waiting for their turn, at
/// once, so the packages that wait in memory stay few.
const UPDATE_WINDOW: usize = 6;

/// Updates the selected addons. Downloads run several at once within
/// [`SourceLimits::standard`], but each addon's folders and database row change one addon at
/// a time, in the order of the selection. An addon whose new package would replace "Not
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
    on_event: impl FnMut(UpdateEvent),
) -> Result<LiveUpdateRun, LemonupError> {
    apply_live_updates_in(
        database,
        addon_dir,
        selectors,
        wago_api_key,
        force,
        dry_run,
        replace_not_linked,
        &SourceLimits::standard(),
        prepare_update,
        on_event,
    )
    .await
}

/// The network part of one addon's update, from the addon's own source.
pub(crate) async fn prepare_update(request: PrepareRequest) -> Result<PreparedUpdate, String> {
    let PrepareRequest {
        addon,
        client,
        flavor,
        wago_api_key,
        force,
        dry_run,
    } = request;
    match addon.source {
        SourceKind::Wago => {
            let api_key = wago_api_key.ok_or_else(|| "Wago API key not configured".to_string())?;
            crate::wago::prepare_wago_update(client, addon, api_key, flavor, force, dry_run).await
        }
        SourceKind::Tukui => {
            crate::tukui::prepare_tukui_update(client, addon, flavor, force, dry_run).await
        }
        SourceKind::WowInterface => {
            crate::wowinterface::prepare_wowinterface_update(client, addon, force, dry_run).await
        }
        SourceKind::GitHub => {
            crate::github::prepare_github_update(client, addon, flavor, force, dry_run).await
        }
        SourceKind::Manual => Err("addons that are not linked cannot be updated yet".to_string()),
    }
}

/// Where one addon stands before its turn: nothing to ask the source, or what its download
/// brought.
enum UpdateTurn {
    Manual,
    NoApiKey,
    Prepared(Result<PreparedUpdate, String>),
}

/// The update engine. `prepare` builds the network part of one addon's update; it runs here,
/// before that part is handed to its own task, so it may use the database. The tasks only
/// wait for the network. Every folder change and database write happens here, one addon at a
/// time and in the order of the selection: an addon whose download ended early waits for the
/// addons before it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_live_updates_in<F, Fut>(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selectors: &[String],
    wago_api_key: Option<&str>,
    force: bool,
    dry_run: bool,
    replace_not_linked: bool,
    limits: &SourceLimits,
    mut prepare: F,
    mut on_event: impl FnMut(UpdateEvent),
) -> Result<LiveUpdateRun, LemonupError>
where
    F: FnMut(PrepareRequest) -> Fut,
    Fut: Future<Output = Result<PreparedUpdate, String>> + Send + 'static,
{
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
    let flavor = GameFlavor::for_addons_dir(addon_dir);

    let mut summary = LiveUpdateSummary {
        target_addons: selected.len(),
        ..LiveUpdateSummary::default()
    };
    let mut results = Vec::with_capacity(selected.len());
    let mut held = Vec::new();

    // Built once for the run, the first time an addon needs one, as for checks.
    let mut general_client: Option<Result<reqwest::Client, String>> = None;
    let mut github_client: Option<Result<reqwest::Client, String>> = None;

    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut join_set = tokio::task::JoinSet::new();
    // Which addon each task prepares, so a task that panics still finishes its own addon.
    let mut task_index = std::collections::HashMap::new();
    let mut turns: Vec<Option<UpdateTurn>> = selected.iter().map(|_| None).collect();
    let mut spawned = vec![false; selected.len()];
    // Spawned and not yet applied.
    let mut out = 0;
    let mut next_spawn = 0;
    let mut next_apply = 0;

    loop {
        // Downloads start in list order while the window has room. Rows that need no network
        // do not count against it.
        while next_spawn < selected.len() && out < UPDATE_WINDOW {
            let index = next_spawn;
            next_spawn += 1;
            let addon = &selected[index];
            let api_key = match addon.source {
                SourceKind::Manual => {
                    turns[index] = Some(UpdateTurn::Manual);
                    continue;
                }
                // Tracked addons without authoritative ownership (for example rows migrated
                // from v1) are updated too: the provider takes the folder set from the package.
                SourceKind::Wago => match wago_api_key {
                    Some(api_key) => Some(api_key.to_string()),
                    None => {
                        turns[index] = Some(UpdateTurn::NoApiKey);
                        continue;
                    }
                },
                SourceKind::Tukui | SourceKind::WowInterface | SourceKind::GitHub => None,
            };
            let client = if addon.source == SourceKind::GitHub {
                github_client.get_or_insert_with(crate::github::build_github_client)
            } else {
                general_client.get_or_insert_with(|| {
                    crate::http::client_builder()
                        .build()
                        .map_err(|error| error.to_string())
                })
            };
            let client = match client {
                Ok(client) => client.clone(),
                Err(detail) => {
                    // No request goes out, so there is no `Started`.
                    turns[index] = Some(UpdateTurn::Prepared(Err(detail.clone())));
                    continue;
                }
            };
            let prepared = prepare(PrepareRequest {
                addon: addon.clone(),
                client,
                flavor,
                wago_api_key: api_key,
                force,
                dry_run,
            });
            let task = spawn_prepare(&mut join_set, limits, &started_tx, index, addon, prepared);
            task_index.insert(task, index);
            spawned[index] = true;
            out += 1;
        }

        // Each addon whose turn has come and whose download is done is applied now.
        let mut applied = false;
        while let Some(turn) = turns.get_mut(next_apply).and_then(Option::take) {
            // Its task sent `Started` before it ended, so that goes out first.
            while let Ok(folder) = started_rx.try_recv() {
                on_event(UpdateEvent::Started { folder });
            }
            let addon = &selected[next_apply];
            match turn {
                UpdateTurn::Manual => {
                    summary.skipped_manual += 1;
                    results.push(LiveUpdateResult {
                        addon_name: addon.folder.clone(),
                        source: addon.source,
                        status: LiveUpdateStatus::SkippedManual,
                        previous_version: addon.version.clone(),
                        remote_version: addon.remote_version.clone(),
                        message: Some(
                            "addons that are not linked cannot be updated yet".to_string(),
                        ),
                        not_linked: Vec::new(),
                        dev_copies: Vec::new(),
                    });
                }
                UpdateTurn::NoApiKey => {
                    summary.skipped_no_api_key += 1;
                    results.push(LiveUpdateResult {
                        addon_name: addon.folder.clone(),
                        source: addon.source,
                        status: LiveUpdateStatus::SkippedNoApiKey,
                        previous_version: addon.version.clone(),
                        remote_version: addon.remote_version.clone(),
                        message: Some("Wago API key not configured".to_string()),
                        not_linked: Vec::new(),
                        dev_copies: Vec::new(),
                    });
                }
                UpdateTurn::Prepared(prepared) => {
                    let outcome =
                        apply_prepared(database, addon_dir, addon, prepared, agree, dry_run);
                    record_provider_result(
                        &mut summary,
                        &mut results,
                        &mut held,
                        addon,
                        addon_dir,
                        outcome,
                    );
                }
            }
            if spawned[next_apply] {
                out -= 1;
            }
            if let Some(result) = results.last() {
                on_event(UpdateEvent::Finished {
                    folder: addon.folder.clone(),
                    result: result.clone(),
                });
            }
            next_apply += 1;
            applied = true;
        }
        if next_apply == selected.len() {
            break;
        }
        if applied {
            // The window may have room again.
            continue;
        }

        tokio::select! {
            biased;
            Some(folder) = started_rx.recv() => {
                on_event(UpdateEvent::Started { folder });
            }
            joined = join_set.join_next_with_id() => {
                let Some(joined) = joined else {
                    // A safety net: every addon whose task never reported fails.
                    for index in next_apply..selected.len() {
                        if spawned[index] && turns[index].is_none() {
                            turns[index] = Some(UpdateTurn::Prepared(Err(
                                update_stopped_unexpectedly(),
                            )));
                        }
                    }
                    continue;
                };
                let (index, prepared) = match joined {
                    Ok((_task, (index, prepared))) => (index, prepared),
                    // A task that panicked or was canceled fails its own addon. One that is
                    // somehow unknown is caught by the safety net.
                    Err(error) => {
                        let Some(&index) = task_index.get(&error.id()) else {
                            continue;
                        };
                        (index, Err(update_stopped_unexpectedly()))
                    }
                };
                turns[index] = Some(UpdateTurn::Prepared(prepared));
            }
        }
    }

    Ok(LiveUpdateRun {
        summary,
        results,
        held,
    })
}

/// Why an update whose task stopped without a result failed.
fn update_stopped_unexpectedly() -> String {
    "the update stopped unexpectedly".to_string()
}

/// The name an update shows for `addon`: its name, else its folder.
fn shown_name(addon: &AddonRecord) -> String {
    let name = addon.name.trim();
    if name.is_empty() {
        addon.folder.clone()
    } else {
        name.to_string()
    }
}

/// Applies what one addon's download brought to its row as it is stored now, compared with
/// the row as the run listed it. A row that was deleted or linked to another source meanwhile
/// is not touched, and neither is one whose source URL changed (linked to another project on
/// the same source). A downloaded package is also not applied when the installed `version`,
/// `git_commit` or `release_tag` changed, since something else installed the addon meanwhile.
fn apply_prepared(
    database: &mut StateDatabase,
    addon_dir: &Path,
    snapshot: &AddonRecord,
    prepared: Result<PreparedUpdate, String>,
    agree: ReplaceNotLinked<'_>,
    dry_run: bool,
) -> Result<(ProviderOutcome, Option<HeldPackage>), String> {
    let prepared = prepared?;
    let fresh = database
        .get_addon_by_folder(&snapshot.folder)
        .map_err(|error| error.to_string())?
        .filter(|row| row.source == snapshot.source)
        .ok_or_else(|| format!("{} is no longer linked", shown_name(snapshot)))?;
    let changed = || {
        format!(
            "{} changed during the update; update it again",
            shown_name(snapshot)
        )
    };
    if fresh.source_url != snapshot.source_url {
        return Err(changed());
    }
    match prepared {
        PreparedUpdate::Skipped {
            outcome,
            source_url,
            source_name,
        } => {
            record_skipped_update_check(
                database,
                snapshot,
                outcome.finished.remote_version.as_deref(),
                source_url,
                source_name.as_deref(),
                dry_run,
            )?;
            Ok((outcome, None))
        }
        PreparedUpdate::Downloaded { bytes, apply } => {
            if fresh.version != snapshot.version
                || fresh.git_commit != snapshot.git_commit
                || fresh.release_tag != snapshot.release_tag
            {
                return Err(changed());
            }
            let outcome = apply(database, addon_dir, &fresh, &bytes, agree, dry_run)?;
            if outcome.finished.not_linked.is_empty() || dry_run {
                return Ok((outcome, None));
            }
            let package =
                HeldPackage::keep(&bytes, move |database, addon_dir, tracked, bytes, agree| {
                    apply(database, addon_dir, tracked, bytes, agree, false)
                        .map(|outcome| outcome.finished)
                })?;
            Ok((outcome, Some(package)))
        }
    }
}

/// Runs one addon's download in its own task: it waits for its permits, says it started,
/// then waits for the source, and gives the permits back before it ends. The task does no
/// database work. Returns the task's id.
fn spawn_prepare(
    join_set: &mut tokio::task::JoinSet<(usize, Result<PreparedUpdate, String>)>,
    limits: &SourceLimits,
    started_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    index: usize,
    addon: &AddonRecord,
    prepared: impl Future<Output = Result<PreparedUpdate, String>> + Send + 'static,
) -> tokio::task::Id {
    let limits = limits.clone();
    let started_tx = started_tx.clone();
    let source = addon.source;
    let folder = addon.folder.clone();
    join_set
        .spawn(async move {
            let permit = limits.acquire(source).await;
            // The engine stops listening only when it gave up on the run; the task still ends.
            let _ = started_tx.send(folder);
            let prepared = prepared.await;
            drop(permit);
            (index, prepared)
        })
        .id()
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
            not_linked: Vec::new(),
            dev_copies: Vec::new(),
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
        not_linked: Vec::new(),
        dev_copies: Vec::new(),
    };
    match outcome {
        Ok((outcome, package)) if !outcome.finished.not_linked.is_empty() => {
            summary.held += 1;
            let dev = dev_copies(addon_dir, &outcome.finished.not_linked);
            // The folders ride on the result itself: a dry run keeps no package, so `held`
            // stays empty there, and the report still has to name them.
            results.push(LiveUpdateResult {
                not_linked: outcome.finished.not_linked.clone(),
                dev_copies: dev.clone(),
                ..base(
                    LiveUpdateStatus::Held,
                    outcome.finished.previous_version.clone(),
                    outcome.finished.remote_version.clone(),
                    Some(format!(
                        "would replace Not linked folders: {}",
                        describe_not_linked_folders(&outcome.finished.not_linked, &dev)
                    )),
                )
            });
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
        not_linked: Vec::new(),
        dev_copies: Vec::new(),
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

    // A release tag for a source install, even one whose commit is unknown (v1 rows) or whose
    // own version equals the tag. Checks store a commit as GitHub's full 40 characters, so a
    // tag that only looks like one (`20241118`) is still a tag.
    if !crate::github::is_full_commit(remote) {
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
        return Err(LemonupError::Usage {
            reason: format!("{noun}: {}.", missing.join(", ")),
            hint: "Use exact addon names or folder names, or run `lemonup check` to see the \
                   overall summary."
                .to_string(),
        });
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
    use super::refresh_live_update_checks_in;
    use super::{
        LiveUpdateStatus, apply_live_updates, compare_release_versions, determine_update_status,
        record_skipped_update_check, refresh_live_update_checks_with, remote_is_older,
        remote_older_result, resolve_selected_addons, serialize_live_update_status,
        serialize_update_status,
    };
    use crate::source_limits::SourceLimits;
    use lemonup_core::{AddonRecord, SourceKind, StateDatabase, UpdateStatus};
    use std::cmp::Ordering;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;
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
            |_| {},
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
            |_| {},
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
        // The result itself names the folders, so a dry run's report can too.
        assert_eq!(
            results[0].not_linked,
            vec!["BigWigs_Core".to_string(), "BigWigs_Options".to_string()]
        );
        assert_eq!(results[0].dev_copies, vec!["BigWigs_Core".to_string()]);
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
    fn a_github_row_without_a_known_commit_is_update_ready_for_a_tag_that_looks_like_one() {
        let mut addon = AddonRecord::new("Dated", "Dated", SourceKind::GitHub);
        addon.version = Some("@project-version@".to_string());
        addon.remote_version = Some("20241118".to_string());
        assert_eq!(
            determine_update_status(&addon).0,
            UpdateStatus::UpdateAvailable
        );

        // A full commit still can't be compared with an unknown installed one.
        addon.remote_version = Some("364f625cf8c4f2f1c0785ab12da2121e880ec560".to_string());
        assert_eq!(determine_update_status(&addon).0, UpdateStatus::Unknown);

        // The addon's own version equals the tag, and it still came from source.
        addon.version = Some("20241118".to_string());
        addon.remote_version = Some("20241118".to_string());
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
            |_| {},
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
            |_| {},
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
            |_| {},
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

    /// A Wago check that found `version`.
    fn wago_version(
        version: &str,
    ) -> Result<crate::wago::WagoRemoteVersion, crate::source_error::SourceError> {
        Ok(crate::wago::WagoRemoteVersion {
            source_url: None,
            version: Some(version.to_string()),
            name: None,
        })
    }

    /// A database in `temp` with one Wago addon per folder, each at version 1.0.
    fn database_with_wago_addons(temp: &tempfile::TempDir, folders: &[&str]) -> StateDatabase {
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        for folder in folders {
            let mut addon = AddonRecord::new(*folder, *folder, SourceKind::Wago);
            addon.version = Some("1.0".to_string());
            database.upsert_addon(&addon).expect("seed addon");
        }
        database
    }

    fn folders_of(database: &StateDatabase) -> Vec<String> {
        database
            .list_addons()
            .expect("list")
            .into_iter()
            .map(|addon| addon.folder)
            .collect()
    }

    /// Three checks that each wait for the other two: only checks that run at the same time
    /// get past the barrier.
    #[tokio::test]
    async fn checks_run_at_the_same_time() {
        let temp = tempdir().expect("tempdir");
        let mut database = database_with_wago_addons(&temp, &["A", "B", "C"]);
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let limits = SourceLimits::new(6, 3, 4, 4, 2);

        let run = refresh_live_update_checks_in(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            &limits,
            move |_client, _addon, _api_key| {
                let barrier = Arc::clone(&barrier);
                async move {
                    barrier.wait().await;
                    wago_version("2.0")
                }
            },
            |_| {},
        );
        let checks = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("the checks wait for each other, so they must overlap")
            .expect("refresh checks");

        assert_eq!(checks.len(), 3);
        assert!(
            checks
                .iter()
                .all(|check| check.status == UpdateStatus::UpdateAvailable),
            "{checks:?}"
        );
    }

    /// The most checks that were out at once, checking twelve Wago addons under `limits`.
    async fn most_checks_out_at_once(limits: SourceLimits) -> usize {
        let temp = tempdir().expect("tempdir");
        let folders = (1..=12).map(|n| format!("Addon{n:02}")).collect::<Vec<_>>();
        let folders = folders.iter().map(String::as_str).collect::<Vec<_>>();
        let mut database = database_with_wago_addons(&temp, &folders);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));

        let fetch = {
            let in_flight = Arc::clone(&in_flight);
            let most = Arc::clone(&most);
            move |_client, _addon, _api_key| {
                let in_flight = Arc::clone(&in_flight);
                let most = Arc::clone(&most);
                async move {
                    let now = in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                    most.fetch_max(now, AtomicOrdering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
                    wago_version("2.0")
                }
            }
        };
        let checks = refresh_live_update_checks_in(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            &limits,
            fetch,
            |_| {},
        )
        .await
        .expect("refresh checks");
        assert_eq!(checks.len(), 12);
        most.load(AtomicOrdering::SeqCst)
    }

    #[tokio::test]
    async fn the_total_cap_limits_checks_out_at_once() {
        let most = most_checks_out_at_once(SourceLimits::new(3, 6, 6, 6, 6)).await;
        assert!(1 < most && most <= 3, "{most}");
    }

    #[tokio::test]
    async fn the_source_cap_limits_checks_out_at_once() {
        let most = most_checks_out_at_once(SourceLimits::new(6, 6, 2, 6, 6)).await;
        assert!(1 < most && most <= 2, "{most}");
    }

    /// The last addon's check ends first and each one lets the one before it end, yet the
    /// results come back in the order the addons were selected.
    #[tokio::test]
    async fn results_keep_the_selection_order_when_later_checks_end_first() {
        const COUNT: usize = 3;
        let temp = tempdir().expect("tempdir");
        let mut database = database_with_wago_addons(&temp, &["A", "B", "C"]);
        let selection = folders_of(&database);
        let calls = Arc::new(AtomicUsize::new(0));
        let releases: Arc<std::sync::Mutex<Vec<Option<tokio::sync::oneshot::Sender<()>>>>> =
            Arc::default();
        let limits = SourceLimits::new(6, 3, 4, 4, 2);
        let mut finished = Vec::new();

        let run = refresh_live_update_checks_in(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            &limits,
            |_client, addon: AddonRecord, _api_key| {
                let index = calls.fetch_add(1, AtomicOrdering::SeqCst);
                let (release, released) = tokio::sync::oneshot::channel();
                releases.lock().expect("releases").push(Some(release));
                let releases = Arc::clone(&releases);
                async move {
                    if index + 1 < COUNT {
                        released.await.expect("the next check lets this one end");
                    }
                    if index > 0 {
                        let previous = releases.lock().expect("releases")[index - 1].take();
                        previous.expect("sender").send(()).expect("still waiting");
                    }
                    wago_version(&format!("2.0-{}", addon.folder))
                }
            },
            |event| {
                if let super::CheckEvent::Finished { folder, .. } = event {
                    finished.push(folder);
                }
            },
        );
        let checks = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("each check waits for the next, so they must overlap")
            .expect("refresh checks");

        let mut reversed = selection.clone();
        reversed.reverse();
        assert_eq!(finished, reversed, "the checks ended last to first");
        assert_eq!(
            checks
                .iter()
                .map(|check| check.addon_name.clone())
                .collect::<Vec<_>>(),
            selection
        );
        for check in &checks {
            assert_eq!(
                check.remote_version.as_deref(),
                Some(format!("2.0-{}", check.addon_name).as_str())
            );
        }
    }

    /// Every `Started` comes before its addon's `Finished`; rows that need no network finish
    /// without starting; a row deleted while its check waited finishes with no result.
    #[tokio::test]
    async fn check_events_follow_each_addon() {
        use super::CheckEvent;

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("state.sqlite");
        let mut database = database_with_wago_addons(&temp, &["Kept", "Gone"]);
        database
            .upsert_addon(&AddonRecord::new("Mine", "Mine", SourceKind::Manual))
            .expect("seed manual");
        let limits = SourceLimits::new(6, 3, 4, 4, 2);

        // Without a key nothing goes online.
        let mut events = Vec::new();
        refresh_live_update_checks_in(
            &mut database,
            &[],
            None,
            None,
            &limits,
            |_client, _addon, _api_key| async { wago_version("2.0") },
            |event| events.push(event),
        )
        .await
        .expect("refresh checks");
        assert_eq!(events.len(), 3, "{events:?}");
        assert!(
            events.iter().all(|event| matches!(
                event,
                CheckEvent::Finished {
                    result: Some(_),
                    ..
                }
            )),
            "{events:?}"
        );

        let mut events = Vec::new();
        let checks = refresh_live_update_checks_in(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            &limits,
            |_client, addon: AddonRecord, _api_key| {
                if addon.folder == "Gone" {
                    let other = StateDatabase::open(&path).expect("another LemonUp");
                    other.remove_addon("Gone").expect("delete");
                }
                async { wago_version("2.0") }
            },
            |event| events.push(event),
        )
        .await
        .expect("refresh checks");

        let position = |wanted: &CheckEvent| {
            events
                .iter()
                .position(|event| event == wanted)
                .unwrap_or_else(|| panic!("{wanted:?} in {events:?}"))
        };
        let started = |folder: &str| CheckEvent::Started {
            folder: folder.to_string(),
        };
        let kept = checks
            .iter()
            .find(|check| check.addon_name == "Kept")
            .expect("Kept has a result")
            .clone();
        assert!(
            position(&started("Kept"))
                < position(&CheckEvent::Finished {
                    folder: "Kept".to_string(),
                    result: Some(kept),
                })
        );
        assert!(
            position(&started("Gone"))
                < position(&CheckEvent::Finished {
                    folder: "Gone".to_string(),
                    result: None,
                })
        );
        assert!(
            !events.contains(&started("Mine")),
            "a Manual row sends no request: {events:?}"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            CheckEvent::Finished { folder, result: Some(_) } if folder == "Mine"
        )));
        assert_eq!(events.len(), 5, "{events:?}");
        assert!(checks.iter().all(|check| check.addon_name != "Gone"));
    }

    /// A check that panics gives its addon an error right away, while another check is
    /// still out, and lets the others finish.
    #[tokio::test]
    async fn a_check_that_panics_fails_only_its_own_addon() {
        use super::CheckEvent;

        let temp = tempdir().expect("tempdir");
        let mut database = database_with_wago_addons(&temp, &["A", "Broken", "C"]);
        let limits = SourceLimits::new(6, 3, 4, 4, 2);
        // `C` stays out until `Broken` has finished.
        let (gate, gate_released) = tokio::sync::oneshot::channel::<()>();
        let gate_released = std::sync::Mutex::new(Some(gate_released));
        let mut gate = Some(gate);
        let mut finished = Vec::new();

        let run = refresh_live_update_checks_in(
            &mut database,
            &[],
            Some("fake-key"),
            None,
            &limits,
            |_client, addon: AddonRecord, _api_key| {
                let gate_released = if addon.folder == "C" {
                    gate_released.lock().expect("gate").take()
                } else {
                    None
                };
                async move {
                    if addon.folder == "Broken" {
                        panic!("this check breaks");
                    }
                    if let Some(gate_released) = gate_released {
                        gate_released.await.expect("the gate opens");
                    }
                    wago_version("2.0")
                }
            },
            |event| {
                let CheckEvent::Finished { folder, result } = event else {
                    return;
                };
                if folder == "Broken" {
                    let result = result.expect("Broken has a result");
                    assert_eq!(result.status, UpdateStatus::Error);
                    assert_eq!(
                        result.message.as_deref(),
                        Some("the check stopped unexpectedly")
                    );
                    if let Some(gate) = gate.take() {
                        gate.send(()).expect("C still waits");
                    }
                }
                finished.push(folder);
            },
        );
        let checks = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("Broken finishes while C is still out, which lets C end")
            .expect("the run goes on");

        assert_eq!(finished.len(), 3, "{finished:?}");
        assert_eq!(finished.last().map(String::as_str), Some("C"));
        assert_eq!(checks.len(), 3);
        for check in &checks {
            if check.addon_name == "Broken" {
                assert_eq!(check.status, UpdateStatus::Error);
                assert_eq!(
                    check.error_kind,
                    Some(crate::source_error::SourceErrorKind::Other)
                );
                assert_eq!(
                    check.message.as_deref(),
                    Some("the check stopped unexpectedly")
                );
            } else {
                assert_eq!(check.status, UpdateStatus::UpdateAvailable, "{check:?}");
            }
        }
    }

    /// On a runtime with several threads, each addon's `Started` still reaches the caller
    /// before its `Finished`, however fast the checks end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn started_comes_before_finished_on_several_threads() {
        use super::CheckEvent;

        let temp = tempdir().expect("tempdir");
        let folders = (1..=300)
            .map(|n| format!("Addon{n:03}"))
            .collect::<Vec<_>>();
        let folder_refs = folders.iter().map(String::as_str).collect::<Vec<_>>();
        let mut database = database_with_wago_addons(&temp, &folder_refs);

        for round in 0..5 {
            let limits = SourceLimits::new(64, 64, 64, 64, 64);
            let mut events = Vec::new();
            refresh_live_update_checks_in(
                &mut database,
                &[],
                Some("fake-key"),
                None,
                &limits,
                |_client, _addon, _api_key| async { wago_version("2.0") },
                |event| events.push(event),
            )
            .await
            .expect("refresh checks");

            let mut started = std::collections::HashSet::new();
            for event in &events {
                match event {
                    CheckEvent::Started { folder } => {
                        started.insert(folder.clone());
                    }
                    CheckEvent::Finished { folder, .. } => {
                        assert!(
                            started.contains(folder),
                            "round {round}: {folder} finished before it started"
                        );
                    }
                }
            }
            assert_eq!(started.len(), folders.len(), "round {round}");
        }
    }

    type PrepareFuture = std::pin::Pin<
        Box<dyn Future<Output = Result<super::PreparedUpdate, String>> + Send + 'static>,
    >;

    /// An update that found the addon at 1.0, as the source lists it.
    fn up_to_date() -> super::PreparedUpdate {
        super::PreparedUpdate::Skipped {
            outcome: super::ProviderOutcome {
                updated: false,
                remote_older: false,
                finished: super::FinishedUpdate {
                    previous_version: Some("1.0".to_string()),
                    remote_version: Some("1.0".to_string()),
                    not_linked: Vec::new(),
                    warning: None,
                },
            },
            source_url: None,
            source_name: None,
        }
    }

    /// A downloaded package whose apply only notes, in `applied`, which addon it was
    /// applied to.
    fn noted_package(applied: &Arc<std::sync::Mutex<Vec<String>>>) -> super::PreparedUpdate {
        let applied = Arc::clone(applied);
        super::PreparedUpdate::downloaded(Vec::new(), move |_, _, tracked, _, _, _| {
            applied
                .lock()
                .expect("applied")
                .push(tracked.folder.clone());
            Ok(super::ProviderOutcome {
                updated: true,
                remote_older: false,
                finished: super::FinishedUpdate {
                    previous_version: tracked.version.clone(),
                    remote_version: Some("2.0".to_string()),
                    not_linked: Vec::new(),
                    warning: None,
                },
            })
        })
    }

    fn toc(title: &str, version: &str) -> String {
        format!("## Title: {title}\n## Version: {version}\n## Interface: 110205\n")
    }

    /// A retail AddOns folder in `temp` with one folder per WoWInterface addon at 1.0, each
    /// with an old file, and the database that links them. `not_linked` folders are on disk
    /// too, as Not linked rows.
    fn linked_addons(
        temp: &tempfile::TempDir,
        linked: &[&str],
        not_linked: &[&str],
    ) -> (std::path::PathBuf, StateDatabase) {
        let addon_dir = temp
            .path()
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        for (folder, source) in linked
            .iter()
            .map(|folder| (folder, SourceKind::WowInterface))
            .chain(not_linked.iter().map(|folder| (folder, SourceKind::Manual)))
        {
            std::fs::create_dir_all(addon_dir.join(folder)).expect("create folder");
            std::fs::write(
                addon_dir.join(folder).join(format!("{folder}.toc")),
                toc(folder, "1.0"),
            )
            .expect("write toc");
            std::fs::write(addon_dir.join(folder).join("old.lua"), "old").expect("old file");
            let mut addon = AddonRecord::new(*folder, *folder, source);
            addon.version = Some("1.0".to_string());
            database.upsert_addon(&addon).expect("seed addon");
        }
        (addon_dir, database)
    }

    /// The real WoWInterface package of `folder` at 2.0, with `extra` folders beside it.
    fn package(folder: &str, extra: &[(&str, String)]) -> super::PreparedUpdate {
        let mut entries = vec![(format!("{folder}/{folder}.toc"), toc(folder, "2.0"))];
        entries.extend(
            extra
                .iter()
                .map(|(extra, body)| (format!("{extra}/{extra}.toc"), body.clone())),
        );
        let entries = entries
            .iter()
            .map(|(path, body)| (path.as_str(), body.as_str()))
            .collect::<Vec<_>>();
        crate::wowinterface::test_downloaded_update(folder, "2.0", &entries)
    }

    /// Prepares each addon with its entry in `packages`. For each `(waiter, waited)` pair,
    /// the waiter's download ends only after the waited one's. Each folder is noted in
    /// `ended` as its download ends.
    fn prepare_from(
        packages: Vec<(&str, super::PreparedUpdate)>,
        waits: &[(&str, &str)],
        ended: &Arc<std::sync::Mutex<Vec<String>>>,
    ) -> impl FnMut(super::PrepareRequest) -> PrepareFuture + use<> {
        let mut packages = packages
            .into_iter()
            .map(|(folder, package)| (folder.to_string(), package))
            .collect::<std::collections::HashMap<_, _>>();
        let mut waiting = std::collections::HashMap::new();
        let mut releases = std::collections::HashMap::new();
        for (waiter, waited) in waits {
            let (release, released) = tokio::sync::oneshot::channel::<()>();
            waiting.insert(waiter.to_string(), released);
            releases.insert(waited.to_string(), release);
        }
        let ended = Arc::clone(ended);
        move |request: super::PrepareRequest| {
            let folder = request.addon.folder.clone();
            let package = packages.remove(&folder);
            let wait = waiting.remove(&folder);
            let release = releases.remove(&folder);
            let ended = Arc::clone(&ended);
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.await.expect("the addon it waits for ends");
                }
                ended.lock().expect("ended").push(folder.clone());
                if let Some(release) = release {
                    let _ = release.send(());
                }
                package.ok_or_else(|| format!("no package for {folder}"))
            })
        }
    }

    /// Updates `selectors` (every addon when empty) with the standard limits and no Wago.io
    /// key, noting every event.
    async fn update_with<F, Fut>(
        database: &mut StateDatabase,
        addon_dir: &std::path::Path,
        selectors: &[&str],
        dry_run: bool,
        prepare: F,
        events: &mut Vec<super::UpdateEvent>,
    ) -> super::LiveUpdateRun
    where
        F: FnMut(super::PrepareRequest) -> Fut,
        Fut: Future<Output = Result<super::PreparedUpdate, String>> + Send + 'static,
    {
        let selectors = selectors
            .iter()
            .map(|selector| selector.to_string())
            .collect::<Vec<_>>();
        let limits = SourceLimits::standard();
        let run = super::apply_live_updates_in(
            database,
            addon_dir,
            &selectors,
            None,
            false,
            dry_run,
            false,
            &limits,
            prepare,
            |event| events.push(event),
        );
        tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .expect("the run ends")
            .expect("the run")
    }

    fn statuses(run: &super::LiveUpdateRun) -> Vec<(String, LiveUpdateStatus)> {
        run.results
            .iter()
            .map(|result| (result.addon_name.clone(), result.status))
            .collect()
    }

    fn stored(database: &StateDatabase, folder: &str) -> Option<AddonRecord> {
        database.get_addon_by_folder(folder).expect("get addon")
    }

    /// Three downloads that each wait for the other two: only downloads that run at the
    /// same time get past the barrier.
    #[tokio::test]
    async fn update_downloads_run_at_the_same_time() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["A", "B", "C"], &[]);
        let barrier = Arc::new(tokio::sync::Barrier::new(3));

        let run = update_with(
            &mut database,
            &addon_dir,
            &[],
            false,
            |_request| {
                let barrier = Arc::clone(&barrier);
                async move {
                    barrier.wait().await;
                    Ok(up_to_date())
                }
            },
            &mut Vec::new(),
        )
        .await;

        assert_eq!(
            statuses(&run),
            ["A", "B", "C"].map(|folder| (folder.to_string(), LiveUpdateStatus::UpToDate))
        );
    }

    /// With the first download held back, no more than six addons are ever out or waiting
    /// for their turn; once it ends, all twelve finish.
    #[tokio::test]
    async fn no_more_than_six_updates_wait_at_once() {
        let temp = tempdir().expect("tempdir");
        let folders = (1..=12).map(|n| format!("Addon{n:02}")).collect::<Vec<_>>();
        let folder_refs = folders.iter().map(String::as_str).collect::<Vec<_>>();
        let (addon_dir, mut database) = linked_addons(&temp, &folder_refs, &[]);
        // Room for every request, so only the window holds them back.
        let limits = SourceLimits::new(64, 64, 64, 64, 64);
        let started = AtomicUsize::new(0);
        let (gate, gate_opened) = tokio::sync::oneshot::channel::<()>();
        let gate_opened = std::sync::Mutex::new(Some(gate_opened));

        let run = super::apply_live_updates_in(
            &mut database,
            &addon_dir,
            &[],
            None,
            false,
            false,
            false,
            &limits,
            |request: super::PrepareRequest| {
                started.fetch_add(1, AtomicOrdering::SeqCst);
                let gate_opened = (request.addon.folder == "Addon01")
                    .then(|| gate_opened.lock().expect("gate").take())
                    .flatten();
                async move {
                    if let Some(gate_opened) = gate_opened {
                        gate_opened.await.expect("the gate opens");
                    }
                    Ok(up_to_date())
                }
            },
            |_| {},
        );
        let started = &started;
        let control = async move {
            while started.load(AtomicOrdering::SeqCst) < 6 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Time for more to start, if anything let them.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let before = started.load(AtomicOrdering::SeqCst);
            gate.send(()).expect("the first download still waits");
            before
        };
        let (run, before) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(run, control)
        })
        .await
        .expect("the run ends once the first download does");
        let run = run.expect("the run");

        assert_eq!(before, 6);
        assert_eq!(started.load(AtomicOrdering::SeqCst), 12);
        assert_eq!(
            statuses(&run),
            folders
                .iter()
                .map(|folder| (folder.clone(), LiveUpdateStatus::UpToDate))
                .collect::<Vec<_>>()
        );
    }

    /// The last download ends first and each one lets the one before it end, yet the
    /// packages are applied, finished and returned in list order.
    #[tokio::test]
    async fn updates_are_applied_in_list_order_whatever_order_downloads_end() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["A", "B", "C"], &[]);
        let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let prepare = prepare_from(
            vec![
                ("A", noted_package(&applied)),
                ("B", noted_package(&applied)),
                ("C", noted_package(&applied)),
            ],
            &[("A", "B"), ("B", "C")],
            &ended,
        );
        let mut events = Vec::new();

        let run = update_with(&mut database, &addon_dir, &[], false, prepare, &mut events).await;

        let in_order = ["A", "B", "C"].map(str::to_string).to_vec();
        assert_eq!(*ended.lock().expect("ended"), ["C", "B", "A"]);
        assert_eq!(*applied.lock().expect("applied"), in_order);
        let finished = events
            .iter()
            .filter_map(|event| match event {
                super::UpdateEvent::Finished { folder, .. } => Some(folder.clone()),
                super::UpdateEvent::Started { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(finished, in_order);
        assert_eq!(
            statuses(&run),
            in_order
                .iter()
                .map(|folder| (folder.clone(), LiveUpdateStatus::Updated))
                .collect::<Vec<_>>()
        );
    }

    /// Both addons ship a new `Shared` folder; the second in the list ends its download
    /// first, yet the first in the list gets the folder and the other is refused.
    async fn a_new_folder_goes_to_the_first_in_the_list(first: &str, second: &str) {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Alpha", "Bravo"], &[]);
        let shared = |owner: &str| toc(&format!("Shared from {owner}"), "2.0");
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let prepare = prepare_from(
            vec![
                ("Alpha", package("Alpha", &[("Shared", shared("Alpha"))])),
                ("Bravo", package("Bravo", &[("Shared", shared("Bravo"))])),
            ],
            &[(first, second)],
            &ended,
        );

        let run = update_with(
            &mut database,
            &addon_dir,
            &[first, second],
            false,
            prepare,
            &mut Vec::new(),
        )
        .await;

        assert_eq!(*ended.lock().expect("ended"), [second, first]);
        assert_eq!(
            run.results
                .iter()
                .map(|result| result.addon_name.as_str())
                .collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(run.results[0].status, LiveUpdateStatus::Updated);
        assert_eq!(run.results[1].status, LiveUpdateStatus::Error);
        let message = run.results[1].message.as_deref().unwrap_or_default();
        assert!(
            message.contains("owned by something else on disk: Shared"),
            "{message}"
        );
        assert_eq!(
            std::fs::read_to_string(addon_dir.join("Shared").join("Shared.toc")).expect("toc"),
            shared(first)
        );
        let winner = stored(&database, first).expect("first row");
        assert_eq!(winner.version.as_deref(), Some("2.0"));
        assert!(
            winner
                .owned_folders
                .iter()
                .any(|owned| owned.name == "Shared")
        );
        let refused = stored(&database, second).expect("second row");
        assert_eq!(refused.version.as_deref(), Some("1.0"));
        assert!(addon_dir.join(second).join("old.lua").exists());
    }

    #[tokio::test]
    async fn a_new_folder_two_updates_ship_goes_to_the_first_in_the_list() {
        a_new_folder_goes_to_the_first_in_the_list("Alpha", "Bravo").await;
        a_new_folder_goes_to_the_first_in_the_list("Bravo", "Alpha").await;
    }

    /// Two updates that would replace Not linked folders, ending in reverse order: both are
    /// held in list order, nothing changes, and the kept packages can still be skipped or
    /// applied.
    #[tokio::test]
    async fn updates_over_not_linked_folders_are_held_in_list_order() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) =
            linked_addons(&temp, &["Alpha", "Bravo"], &["LibAlpha", "LibBravo"]);
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let prepare = prepare_from(
            vec![
                (
                    "Alpha",
                    package("Alpha", &[("LibAlpha", toc("LibAlpha", "2.0"))]),
                ),
                (
                    "Bravo",
                    package("Bravo", &[("LibBravo", toc("LibBravo", "2.0"))]),
                ),
            ],
            &[("Alpha", "Bravo")],
            &ended,
        );

        let run = update_with(
            &mut database,
            &addon_dir,
            &["Alpha", "Bravo"],
            false,
            prepare,
            &mut Vec::new(),
        )
        .await;

        assert_eq!(*ended.lock().expect("ended"), ["Bravo", "Alpha"]);
        assert_eq!(
            statuses(&run),
            [
                ("Alpha".to_string(), LiveUpdateStatus::Held),
                ("Bravo".to_string(), LiveUpdateStatus::Held)
            ]
        );
        assert_eq!(run.summary.held, 2);
        assert_eq!(
            run.held
                .iter()
                .map(|held| (held.folder.as_str(), held.folders.clone()))
                .collect::<Vec<_>>(),
            [
                ("Alpha", vec!["LibAlpha".to_string()]),
                ("Bravo", vec!["LibBravo".to_string()])
            ]
        );
        for folder in ["Alpha", "Bravo", "LibAlpha", "LibBravo"] {
            assert!(addon_dir.join(folder).join("old.lua").exists(), "{folder}");
            assert_eq!(
                stored(&database, folder).expect("row").version.as_deref(),
                Some("1.0")
            );
        }
        assert_eq!(
            stored(&database, "LibBravo").expect("row").source,
            SourceKind::Manual
        );

        let mut held = run.held;
        let bravo = held.pop().expect("Bravo held");
        let alpha = held.pop().expect("Alpha held");
        let alpha_package = alpha.package_path().to_path_buf();
        let bravo_package = bravo.package_path().to_path_buf();
        assert!(alpha_package.exists() && bravo_package.exists());

        alpha.discard();
        assert!(!alpha_package.exists());
        assert!(addon_dir.join("LibAlpha").join("old.lua").exists());

        let result = bravo.apply(&mut database, &addon_dir);
        assert_eq!(result.status, LiveUpdateStatus::Updated, "{result:?}");
        assert!(!bravo_package.exists());
        assert!(!addon_dir.join("Bravo").join("old.lua").exists());
        assert!(!addon_dir.join("LibBravo").join("old.lua").exists());
        assert_eq!(
            stored(&database, "Bravo").expect("row").version.as_deref(),
            Some("2.0")
        );
        assert_eq!(stored(&database, "LibBravo"), None);
    }

    /// Updates while `meanwhile` changes the row of `folder` from another LemonUp, after the
    /// run listed the addons and before the package is applied.
    async fn update_while(
        temp: &tempfile::TempDir,
        folders: &[&str],
        changed: &str,
        meanwhile: impl Fn(&StateDatabase),
    ) -> (std::path::PathBuf, StateDatabase, super::LiveUpdateRun) {
        let (addon_dir, mut database) = linked_addons(temp, folders, &[]);
        let path = temp.path().join("state.sqlite");
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut prepare = prepare_from(
            folders
                .iter()
                .map(|folder| (*folder, package(folder, &[])))
                .collect(),
            &[],
            &ended,
        );
        let changed = changed.to_string();
        let run = update_with(
            &mut database,
            &addon_dir,
            &[],
            false,
            |request: super::PrepareRequest| {
                if request.addon.folder == changed {
                    let other = StateDatabase::open(&path).expect("another LemonUp");
                    meanwhile(&other);
                }
                prepare(request)
            },
            &mut Vec::new(),
        )
        .await;
        (addon_dir, database, run)
    }

    #[tokio::test]
    async fn an_addon_unlinked_while_its_download_waited_is_left_alone() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, database, run) =
            update_while(&temp, &["Alpha", "Bravo"], "Alpha", |other| {
                other.remove_addon("Alpha").expect("delete");
            })
            .await;
        assert_eq!(run.results[0].status, LiveUpdateStatus::Error);
        assert_eq!(
            run.results[0].message.as_deref(),
            Some("Alpha is no longer linked")
        );
        assert!(addon_dir.join("Alpha").join("old.lua").exists());
        assert_eq!(stored(&database, "Alpha"), None);
        assert_eq!(run.results[1].status, LiveUpdateStatus::Updated);
        assert_eq!(
            stored(&database, "Bravo").expect("row").version.as_deref(),
            Some("2.0")
        );

        let temp = tempdir().expect("tempdir");
        let (addon_dir, database, run) =
            update_while(&temp, &["Alpha", "Bravo"], "Bravo", |other| {
                let mut relinked = other
                    .get_addon_by_folder("Bravo")
                    .expect("get")
                    .expect("row");
                relinked.source = SourceKind::Wago;
                other.upsert_addon(&relinked).expect("relink");
            })
            .await;
        assert_eq!(run.results[0].status, LiveUpdateStatus::Updated);
        assert_eq!(run.results[1].status, LiveUpdateStatus::Error);
        assert_eq!(
            run.results[1].message.as_deref(),
            Some("Bravo is no longer linked")
        );
        assert!(addon_dir.join("Bravo").join("old.lua").exists());
        let bravo = stored(&database, "Bravo").expect("row");
        assert_eq!(bravo.source, SourceKind::Wago);
        assert_eq!(bravo.version.as_deref(), Some("1.0"));
        assert_eq!(run.summary.updated_addons, 1);
        assert_eq!(run.summary.errors, 1);
    }

    #[tokio::test]
    async fn an_addon_changed_while_its_download_waited_is_left_alone() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, database, run) = update_while(&temp, &["Alpha"], "Alpha", |other| {
            let mut changed = other
                .get_addon_by_folder("Alpha")
                .expect("get")
                .expect("row");
            changed.version = Some("1.5".to_string());
            other.upsert_addon(&changed).expect("change");
        })
        .await;
        assert_eq!(run.results[0].status, LiveUpdateStatus::Error);
        assert_eq!(
            run.results[0].message.as_deref(),
            Some("Alpha changed during the update; update it again")
        );
        assert!(addon_dir.join("Alpha").join("old.lua").exists());
        assert_eq!(
            stored(&database, "Alpha").expect("row").version.as_deref(),
            Some("1.5")
        );
    }

    /// Another LemonUp installed another commit or release meanwhile, with the same TOC
    /// version: the package downloaded for the old state is not applied over it.
    #[tokio::test]
    async fn an_addon_installed_at_another_commit_or_release_meanwhile_is_left_alone() {
        for commit in [true, false] {
            let temp = tempdir().expect("tempdir");
            let (addon_dir, database, run) = update_while(&temp, &["Alpha"], "Alpha", |other| {
                let mut changed = other
                    .get_addon_by_folder("Alpha")
                    .expect("get")
                    .expect("row");
                if commit {
                    changed.git_commit = Some("c3c3c3c3".to_string());
                } else {
                    changed.release_tag = Some("v3".to_string());
                }
                other.upsert_addon(&changed).expect("change");
            })
            .await;
            assert_eq!(run.results[0].status, LiveUpdateStatus::Error, "{commit}");
            assert_eq!(
                run.results[0].message.as_deref(),
                Some("Alpha changed during the update; update it again")
            );
            assert!(addon_dir.join("Alpha").join("old.lua").exists());
            let alpha = stored(&database, "Alpha").expect("row");
            assert_eq!(alpha.version.as_deref(), Some("1.0"));
            if commit {
                assert_eq!(alpha.git_commit.as_deref(), Some("c3c3c3c3"));
            } else {
                assert_eq!(alpha.release_tag.as_deref(), Some("v3"));
            }
            assert!(alpha.owned_folders.is_empty());
        }
    }

    const RELINKED_URL: &str = "https://www.wowinterface.com/downloads/info2.html";

    /// Linked to another project on the same source meanwhile: the package for the old
    /// project is not applied.
    #[tokio::test]
    async fn an_addon_relinked_on_the_same_source_meanwhile_gets_no_package() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, database, run) = update_while(&temp, &["Alpha"], "Alpha", |other| {
            let mut relinked = other
                .get_addon_by_folder("Alpha")
                .expect("get")
                .expect("row");
            relinked.source_url = Some(RELINKED_URL.to_string());
            other.upsert_addon(&relinked).expect("relink");
        })
        .await;
        assert_eq!(run.results[0].status, LiveUpdateStatus::Error);
        assert_eq!(
            run.results[0].message.as_deref(),
            Some("Alpha changed during the update; update it again")
        );
        assert!(addon_dir.join("Alpha").join("old.lua").exists());
        let alpha = stored(&database, "Alpha").expect("row");
        assert_eq!(alpha.version.as_deref(), Some("1.0"));
        assert_eq!(alpha.source_url.as_deref(), Some(RELINKED_URL));
        assert!(alpha.owned_folders.is_empty());
    }

    /// Linked to another project on the same source meanwhile: what the old project's
    /// source said is not recorded on the row.
    #[tokio::test]
    async fn a_skip_for_an_addon_relinked_meanwhile_records_nothing() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Alpha"], &[]);
        let path = temp.path().join("state.sqlite");

        let run = update_with(
            &mut database,
            &addon_dir,
            &[],
            false,
            |_request| {
                let other = StateDatabase::open(&path).expect("another LemonUp");
                let mut relinked = other
                    .get_addon_by_folder("Alpha")
                    .expect("get")
                    .expect("row");
                relinked.source_url = Some(RELINKED_URL.to_string());
                relinked.remote_version = Some("5.0".to_string());
                other.upsert_addon(&relinked).expect("relink");
                async {
                    Ok(super::PreparedUpdate::Skipped {
                        outcome: super::ProviderOutcome {
                            updated: false,
                            remote_older: true,
                            finished: super::FinishedUpdate {
                                previous_version: Some("1.0".to_string()),
                                remote_version: Some("0.9".to_string()),
                                not_linked: Vec::new(),
                                warning: None,
                            },
                        },
                        source_url: Some("https://example.com/old-project".to_string()),
                        source_name: Some("Old project".to_string()),
                    })
                }
            },
            &mut Vec::new(),
        )
        .await;

        assert_eq!(run.results[0].status, LiveUpdateStatus::Error);
        assert_eq!(
            run.results[0].message.as_deref(),
            Some("Alpha changed during the update; update it again")
        );
        let alpha = stored(&database, "Alpha").expect("row");
        assert_eq!(alpha.remote_version.as_deref(), Some("5.0"));
        assert_eq!(alpha.source_url.as_deref(), Some(RELINKED_URL));
        assert_eq!(alpha.source_name, None);
        assert_eq!(alpha.last_checked_at, None);
    }

    /// With a yes to replacing Not linked folders, the update replaces them right away.
    #[tokio::test]
    async fn a_yes_to_not_linked_folders_updates_without_holding() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Alpha"], &["LibAlpha"]);
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let prepare = prepare_from(
            vec![(
                "Alpha",
                package("Alpha", &[("LibAlpha", toc("LibAlpha", "2.0"))]),
            )],
            &[],
            &ended,
        );
        let limits = SourceLimits::standard();
        let selectors = ["Alpha".to_string()];

        let run = super::apply_live_updates_in(
            &mut database,
            &addon_dir,
            &selectors,
            None,
            false,
            false,
            true,
            &limits,
            prepare,
            |_| {},
        );
        let run = tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .expect("the run ends")
            .expect("the run");

        assert_eq!(
            statuses(&run),
            [("Alpha".to_string(), LiveUpdateStatus::Updated)]
        );
        assert!(run.held.is_empty());
        assert_eq!(run.summary.held, 0);
        assert!(!addon_dir.join("LibAlpha").join("old.lua").exists());
        assert_eq!(
            std::fs::read_to_string(addon_dir.join("LibAlpha").join("LibAlpha.toc")).expect("toc"),
            toc("LibAlpha", "2.0")
        );
        assert_eq!(stored(&database, "LibAlpha"), None);
        let alpha = stored(&database, "Alpha").expect("row");
        assert_eq!(alpha.version.as_deref(), Some("2.0"));
        assert!(
            alpha
                .owned_folders
                .iter()
                .any(|owned| owned.name == "LibAlpha")
        );
    }

    /// The network part of each update gets the run's `force` and `dry_run`, and the
    /// flavor of the AddOns folder being updated.
    #[tokio::test]
    async fn a_prepare_request_carries_the_run_settings_and_the_folder_flavor() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp
            .path()
            .join("_forever_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("create addon dir");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open db");
        let mut addon = AddonRecord::new("Alpha", "Alpha", SourceKind::WowInterface);
        addon.version = Some("1.0".to_string());
        database.upsert_addon(&addon).expect("seed addon");
        let limits = SourceLimits::standard();
        let mut seen = Vec::new();

        for (force, dry_run) in [(true, false), (false, true)] {
            let run = super::apply_live_updates_in(
                &mut database,
                &addon_dir,
                &[],
                None,
                force,
                dry_run,
                false,
                &limits,
                |request: super::PrepareRequest| {
                    seen.push((request.force, request.dry_run, request.flavor));
                    async { Ok(up_to_date()) }
                },
                |_| {},
            );
            tokio::time::timeout(Duration::from_secs(10), run)
                .await
                .expect("the run ends")
                .expect("the run");
        }

        assert_eq!(
            seen,
            [
                (true, false, lemonup_core::GameFlavor::Forever),
                (false, true, lemonup_core::GameFlavor::Forever)
            ]
        );
    }

    /// A dry run downloads, but changes nothing and keeps no package, even for an update
    /// that would be held.
    #[tokio::test]
    async fn a_dry_run_with_downloaded_packages_changes_nothing() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Alpha", "Bravo"], &["LibBravo"]);
        let ended = Arc::new(std::sync::Mutex::new(Vec::new()));
        let prepare = prepare_from(
            vec![
                ("Alpha", package("Alpha", &[])),
                (
                    "Bravo",
                    package("Bravo", &[("LibBravo", toc("LibBravo", "2.0"))]),
                ),
            ],
            &[],
            &ended,
        );

        let run = update_with(
            &mut database,
            &addon_dir,
            &["Alpha", "Bravo"],
            true,
            prepare,
            &mut Vec::new(),
        )
        .await;

        assert_eq!(
            statuses(&run),
            [
                ("Alpha".to_string(), LiveUpdateStatus::Updated),
                ("Bravo".to_string(), LiveUpdateStatus::Held)
            ]
        );
        assert!(run.held.is_empty());
        for folder in ["Alpha", "Bravo", "LibBravo"] {
            assert!(addon_dir.join(folder).join("old.lua").exists(), "{folder}");
            assert_eq!(
                stored(&database, folder).expect("row").version.as_deref(),
                Some("1.0")
            );
        }
        assert_eq!(
            stored(&database, "LibBravo").expect("row").source,
            SourceKind::Manual
        );
    }

    /// An update that installed nothing records what the source said, as a check does; a
    /// dry run records nothing.
    #[tokio::test]
    async fn a_skipped_update_records_what_the_source_said() {
        for dry_run in [false, true] {
            let temp = tempdir().expect("tempdir");
            let (addon_dir, mut database) = linked_addons(&temp, &["Alpha"], &[]);
            let run = update_with(
                &mut database,
                &addon_dir,
                &[],
                dry_run,
                |_request| async {
                    Ok(super::PreparedUpdate::Skipped {
                        outcome: super::ProviderOutcome {
                            updated: false,
                            remote_older: true,
                            finished: super::FinishedUpdate {
                                previous_version: Some("1.0".to_string()),
                                remote_version: Some("0.9".to_string()),
                                not_linked: Vec::new(),
                                warning: None,
                            },
                        },
                        source_url: Some("https://example.com/alpha".to_string()),
                        source_name: Some("Alpha at the source".to_string()),
                    })
                },
                &mut Vec::new(),
            )
            .await;

            assert_eq!(run.results[0].status, LiveUpdateStatus::RemoteOlder);
            assert_eq!(run.summary.remote_older, 1);
            let alpha = stored(&database, "Alpha").expect("row");
            if dry_run {
                assert_eq!(alpha.remote_version, None);
                assert_eq!(alpha.source_url, None);
                assert_eq!(alpha.source_name, None);
                assert_eq!(alpha.last_checked_at, None);
            } else {
                assert_eq!(alpha.remote_version.as_deref(), Some("0.9"));
                assert_eq!(
                    alpha.source_url.as_deref(),
                    Some("https://example.com/alpha")
                );
                assert_eq!(alpha.source_name.as_deref(), Some("Alpha at the source"));
                assert!(alpha.last_checked_at.is_some());
            }
            assert_eq!(alpha.version.as_deref(), Some("1.0"));
        }
    }

    /// A download that panics fails only its own addon; the addons after it are still
    /// applied.
    #[tokio::test]
    async fn an_update_that_panics_fails_only_its_own_addon() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Alpha", "Broken", "Charlie"], &[]);
        let applied = Arc::new(std::sync::Mutex::new(Vec::new()));

        let run = update_with(
            &mut database,
            &addon_dir,
            &[],
            false,
            |request: super::PrepareRequest| {
                let package = noted_package(&applied);
                async move {
                    if request.addon.folder == "Broken" {
                        panic!("this download breaks");
                    }
                    Ok(package)
                }
            },
            &mut Vec::new(),
        )
        .await;

        assert_eq!(
            statuses(&run),
            [
                ("Alpha".to_string(), LiveUpdateStatus::Updated),
                ("Broken".to_string(), LiveUpdateStatus::Error),
                ("Charlie".to_string(), LiveUpdateStatus::Updated)
            ]
        );
        assert_eq!(
            run.results[1].message.as_deref(),
            Some("the update stopped unexpectedly")
        );
        assert_eq!(*applied.lock().expect("applied"), ["Alpha", "Charlie"]);
    }

    /// Every `Started` comes before its addon's `Finished`; rows that need no network
    /// finish without starting.
    #[tokio::test]
    async fn update_events_follow_each_addon() {
        use super::UpdateEvent;

        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = linked_addons(&temp, &["Net"], &["Mine"]);
        let mut keyless = AddonRecord::new("Keyless", "Keyless", SourceKind::Wago);
        keyless.version = Some("1.0".to_string());
        database.upsert_addon(&keyless).expect("seed wago");
        let mut events = Vec::new();

        let run = update_with(
            &mut database,
            &addon_dir,
            &["Net", "Mine", "Keyless"],
            false,
            |_request| async { Ok(up_to_date()) },
            &mut events,
        )
        .await;

        let finished = |folder: &str| UpdateEvent::Finished {
            folder: folder.to_string(),
            result: run
                .results
                .iter()
                .find(|result| result.addon_name == folder)
                .expect("result")
                .clone(),
        };
        assert_eq!(
            events,
            [
                UpdateEvent::Started {
                    folder: "Net".to_string()
                },
                finished("Net"),
                finished("Mine"),
                finished("Keyless"),
            ]
        );
        assert_eq!(
            statuses(&run),
            [
                ("Net".to_string(), LiveUpdateStatus::UpToDate),
                ("Mine".to_string(), LiveUpdateStatus::SkippedManual),
                ("Keyless".to_string(), LiveUpdateStatus::SkippedNoApiKey)
            ]
        );
    }
}
