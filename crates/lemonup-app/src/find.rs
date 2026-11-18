//! "Find your addons online": asks Wago.io (fingerprint `_match`), the WoWInterface file list
//! and the TukUI feed where the Not linked addons come from. Matching and linking live in
//! `lemonup_core::find`; this module does the network part and the CLI's `lemonup find`.
//!
//! Wago.io gets folder names and file hashes under the person's key; the key is never logged.
//! The WoWInterface list (no key) is cached for an hour in LemonUp's cache folder; Wago.io and
//! TukUI are always asked fresh.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lemonup_core::find::{
    self, FindChoice, FindLabel, FindReport, FindResult, FindSource, FoundAddon, SourceAnswer,
    SourceLookups, TukuiPackage, WagoLookup, WagoMatch, WowiFile, wire,
};
use lemonup_core::{AddonRecord, GameFlavor, LemonupError, StateDatabase, scan_addons_dir_report};

use crate::source_error::{SourceError, SourceErrorKind};

const WAGO_MATCH_URL: &str = "https://addons.wago.io/api/external/addons/_match";
const WOWI_FILELIST_URL: &str = "https://api.mmoui.com/v4/game/WOW/filelist.json";
/// How long the WoWInterface file list is reused before it is downloaded again.
pub(crate) const WOWI_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const WOWI_CACHE_FILE: &str = "wowinterface-filelist.json";
/// The WoWInterface file list lists thousands of files (7405 in November 2026). A list shorter
/// than this is a broken answer: it is a failure and is never cached, so it can't hide every
/// addon from WoWInterface for an hour.
pub(crate) const WOWI_MIN_FILES: usize = 1000;

/// A source that could not answer, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceProblem {
    pub(crate) source: FindSource,
    pub(crate) error: SourceError,
}

/// The sources `lemonup find` asks: the ones named with `--source`, else all of them. The
/// choice the TUI remembers is not used.
pub(crate) fn cli_choice(sources: &[FindSource]) -> FindChoice {
    if sources.is_empty() {
        FindChoice::all()
    } else {
        FindChoice::only(sources)
    }
}

/// What the sources answered, ready for `lemonup_core::find::match_addons`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lookups {
    pub(crate) lookups: SourceLookups,
    pub(crate) problems: Vec<SourceProblem>,
}

/// Asks the sources in `choice` at once. A source left out is never asked: no request goes out
/// and its answer is `NotAsked`. Without a key Wago.io is skipped (not a failure); a source
/// that fails is reported in `problems` and its answer is `Failed`, so the addons only it could
/// find stay "Not checked".
pub(crate) async fn lookup_sources(
    wago_lookups: &[WagoLookup],
    flavor: GameFlavor,
    wago_api_key: Option<&str>,
    cache_dir: &Path,
    choice: FindChoice,
) -> Lookups {
    lookup_with(
        choice,
        wago_api_key,
        wago_lookups.is_empty(),
        |key| async move { fetch_wago_matches(&key, flavor, wago_lookups).await },
        || fetch_wowi_files(cache_dir, flavor),
        || crate::tukui::fetch_tukui_packages(flavor),
    )
    .await
}

/// [`lookup_sources`] with the three requests given, so a test can count which ones are made.
/// Each runs only for a source in `choice`.
pub(crate) async fn lookup_with<WagoFn, WagoFut, WowiFn, WowiFut, TukuiFn, TukuiFut>(
    choice: FindChoice,
    wago_api_key: Option<&str>,
    nothing_for_wago: bool,
    wago: WagoFn,
    wowinterface: WowiFn,
    tukui: TukuiFn,
) -> Lookups
where
    WagoFn: FnOnce(String) -> WagoFut,
    WagoFut: Future<Output = Result<wire::WagoAnswer, SourceError>>,
    WowiFn: FnOnce() -> WowiFut,
    WowiFut: Future<Output = Result<Vec<WowiFile>, SourceError>>,
    TukuiFn: FnOnce() -> TukuiFut,
    TukuiFut: Future<Output = Result<Vec<TukuiPackage>, SourceError>>,
{
    let wago = async {
        if !choice.wants(FindSource::Wago) {
            return Reply::NotAsked;
        }
        match wago_api_key {
            None => Reply::Skipped,
            Some(_) if nothing_for_wago => Reply::Answer(Ok(wire::WagoAnswer::default())),
            Some(key) => Reply::Answer(wago(key.to_string()).await),
        }
    };
    let wowinterface = async {
        if choice.wants(FindSource::WowInterface) {
            Reply::Answer(wowinterface().await)
        } else {
            Reply::NotAsked
        }
    };
    let tukui = async {
        if choice.wants(FindSource::Tukui) {
            Reply::Answer(tukui().await)
        } else {
            Reply::NotAsked
        }
    };
    let (wago, wowinterface, tukui) = tokio::join!(wago, wowinterface, tukui);

    let mut problems = Vec::new();
    let wago = match wago {
        Reply::NotAsked => SourceAnswer::NotAsked,
        Reply::Skipped => SourceAnswer::Skipped,
        Reply::Answer(result) => wago_answer(&mut problems, result),
    };
    let wowinterface = answer_of(&mut problems, FindSource::WowInterface, wowinterface);
    let tukui = answer_of(&mut problems, FindSource::Tukui, tukui);
    Lookups {
        lookups: SourceLookups {
            wago,
            wowinterface,
            tukui,
        },
        problems,
    }
}

/// What asking one source came to.
enum Reply<T> {
    NotAsked,
    Skipped,
    Answer(Result<T, SourceError>),
}

fn answer_of<T>(
    problems: &mut Vec<SourceProblem>,
    source: FindSource,
    asked: Reply<Vec<T>>,
) -> SourceAnswer<T> {
    match asked {
        Reply::NotAsked => SourceAnswer::NotAsked,
        Reply::Skipped => SourceAnswer::Skipped,
        Reply::Answer(result) => answer(problems, source, result),
    }
}

fn answer<T>(
    problems: &mut Vec<SourceProblem>,
    source: FindSource,
    result: Result<Vec<T>, SourceError>,
) -> SourceAnswer<T> {
    match result {
        Ok(items) => SourceAnswer::Found(items),
        Err(error) => {
            tracing::warn!(source = source.name(), kind = ?error.kind, "find lookup failed");
            problems.push(SourceProblem { source, error });
            SourceAnswer::Failed
        }
    }
}

/// Like [`answer`], but a Wago.io answer with entries LemonUp couldn't read is incomplete: the
/// rest is used, and the problem is reported so the folders only Wago.io could find stay "Not
/// checked".
pub(crate) fn wago_answer(
    problems: &mut Vec<SourceProblem>,
    result: Result<wire::WagoAnswer, SourceError>,
) -> SourceAnswer<WagoMatch> {
    match result {
        Ok(answer) if answer.unreadable > 0 => {
            tracing::warn!(
                unreadable = answer.unreadable,
                "Wago.io sent unreadable matches"
            );
            problems.push(SourceProblem {
                source: FindSource::Wago,
                error: SourceError::new(
                    SourceErrorKind::Other,
                    format!(
                        "Wago.io sent {} matches LemonUp can't read",
                        answer.unreadable
                    ),
                ),
            });
            SourceAnswer::Incomplete(answer.matches)
        }
        result => answer(
            problems,
            FindSource::Wago,
            result.map(|answer| answer.matches),
        ),
    }
}

fn build_client() -> Result<reqwest::Client, SourceError> {
    crate::http::client_builder()
        .build()
        .map_err(|error| SourceError::from(error.to_string()))
}

async fn fetch_wago_matches(
    api_key: &str,
    flavor: GameFlavor,
    lookups: &[WagoLookup],
) -> Result<wire::WagoAnswer, SourceError> {
    let client = build_client()?;
    let game_version = crate::wago::wago_game_version(flavor);
    let url = reqwest::Url::parse_with_params(WAGO_MATCH_URL, [("game_version", game_version)])
        .map_err(|error| SourceError::from(error.to_string()))?;
    tracing::debug!(folders = lookups.len(), "asking Wago.io to match folders");
    let bytes = client
        .post(url)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(wire::wago_match_body(game_version, lookups))
        .send()
        .await
        .map_err(SourceError::from_reqwest)?
        .error_for_status()
        .map_err(SourceError::from_reqwest)?
        .bytes()
        .await
        .map_err(SourceError::from_reqwest)?;
    wire::parse_wago_match(&bytes)
        .map_err(|detail| SourceError::new(SourceErrorKind::Other, detail))
}

async fn fetch_wowi_files(
    cache_dir: &Path,
    flavor: GameFlavor,
) -> Result<Vec<WowiFile>, SourceError> {
    let files = load_wowi_files(
        &cache_dir.join(WOWI_CACHE_FILE),
        SystemTime::now(),
        download_wowi_filelist,
    )
    .await?;
    Ok(find::wowi_files_for_flavor(files, flavor))
}

async fn download_wowi_filelist() -> Result<Vec<u8>, SourceError> {
    let client = build_client()?;
    client
        .get(WOWI_FILELIST_URL)
        .header(reqwest::header::ACCEPT, "application/json")
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

/// The WoWInterface file list: the cached copy when it is younger than [`WOWI_CACHE_TTL`] at
/// `now` and readable, else a fresh download (cached again once it reads). A list with fewer
/// than [`WOWI_MIN_FILES`] files counts as unreadable.
pub(crate) async fn load_wowi_files<F, Fut>(
    cache_file: &Path,
    now: SystemTime,
    download: F,
) -> Result<Vec<WowiFile>, SourceError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<u8>, SourceError>>,
{
    if let Some(bytes) = fresh_cache(cache_file, now)
        && let Ok(files) = read_wowi_list(&bytes)
    {
        return Ok(files);
    }
    let bytes = download().await?;
    let files = read_wowi_list(&bytes)
        .map_err(|detail| SourceError::new(SourceErrorKind::Other, detail))?;
    if let Err(error) = write_cache(cache_file, &bytes) {
        tracing::warn!(%error, "could not cache the WoWInterface file list");
    }
    Ok(files)
}

/// Reads the file list, refusing one with fewer than [`WOWI_MIN_FILES`] files.
fn read_wowi_list(bytes: &[u8]) -> Result<Vec<WowiFile>, String> {
    let files = wire::parse_wowi_filelist(bytes)?;
    if files.len() < WOWI_MIN_FILES {
        return Err(format!(
            "WoWInterface sent a file list of only {} files",
            files.len()
        ));
    }
    Ok(files)
}

fn fresh_cache(path: &Path, now: SystemTime) -> Option<Vec<u8>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let age = now.duration_since(modified).ok()?;
    if age >= WOWI_CACHE_TTL {
        return None;
    }
    fs::read(path).ok()
}

fn write_cache(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("json.partial");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

// ---------------------------------------------------------------------------------------------
// `lemonup find`

fn problem_name(kind: SourceErrorKind) -> &'static str {
    match kind {
        SourceErrorKind::NoConnection => "no_connection",
        SourceErrorKind::KeyRejected => "key_rejected",
        SourceErrorKind::NotFound => "not_found",
        SourceErrorKind::SlowDown => "slow_down",
        SourceErrorKind::FoldersInTheWay
        | SourceErrorKind::FoldersOfLinkedAddon
        | SourceErrorKind::Other => "other",
    }
}

fn source_cli_name(source: FindSource) -> &'static str {
    match source {
        FindSource::Wago => "wago",
        FindSource::WowInterface => "wowinterface",
        FindSource::Tukui => "tukui",
    }
}

fn join(folders: &[String]) -> String {
    if folders.is_empty() {
        "<none>".to_string()
    } else {
        folders.join("|")
    }
}

fn candidate_fields(found: &FoundAddon) -> String {
    let mut fields = format!(
        "source={}, id={}, version={}, latest={}, folders={}",
        source_cli_name(found.source),
        found.source_id,
        found.matched_version.as_deref().unwrap_or("<unknown>"),
        found.latest_version.as_deref().unwrap_or("<unknown>"),
        join(&found.folders)
    );
    if !found.joining.is_empty() {
        let joining = found
            .joining
            .iter()
            .map(|moved| {
                if moved.is_own_addon() {
                    format!("{} (own addon)", moved.folder)
                } else {
                    format!("{} (from {})", moved.folder, moved.from)
                }
            })
            .collect::<Vec<_>>();
        fields.push_str(&format!(", joining={}", joining.join("|")));
    }
    if !found.leaving.is_empty() {
        fields.push_str(&format!(", leaving={}", join(&found.leaving)));
    }
    if !found.missing.is_empty() {
        fields.push_str(&format!(", missing={}", join(&found.missing)));
    }
    if !found.differing.is_empty() {
        fields.push_str(&format!(", differing={}", join(&found.differing)));
    }
    if let Some(reason) = found.not_linkable_reason() {
        fields.push_str(&format!(", not_linkable={reason}"));
    }
    fields
}

fn result_lines(result: &FindResult) -> Vec<String> {
    let head = format!(
        "find result: label={}, addon={}, folder={}",
        result.label.cli_name(),
        result.name,
        result.folder
    );
    match result.label {
        FindLabel::Exact | FindLabel::Likely | FindLabel::Partial => {
            let found = result.found().expect("a labeled match has a candidate");
            vec![format!("{head}, {}", candidate_fields(found))]
        }
        FindLabel::ChooseOne => {
            std::iter::once(format!("{head}, candidates={}", result.candidates.len()))
                .chain(result.candidates.iter().map(|candidate| {
                    format!(
                        "find candidate: addon={}, label={}, folder={}, {}",
                        candidate.name,
                        candidate.label.cli_name(),
                        candidate.parent_folder,
                        candidate_fields(candidate)
                    )
                }))
                .collect()
        }
        FindLabel::NotChecked => vec![format!(
            "{head}, folders={}, not_checked_by={}",
            join(&result.folders),
            result
                .not_checked_by
                .iter()
                .map(|source| source.name())
                .collect::<Vec<_>>()
                .join("|")
        )],
        FindLabel::CurseForgeOnly => vec![format!(
            "{head}, folders={}, curseforge_id={}",
            join(&result.folders),
            result.curseforge_id.as_deref().unwrap_or("<unknown>")
        )],
        FindLabel::NotFound | FindLabel::DeveloperCopy => {
            vec![format!("{head}, folders={}", join(&result.folders))]
        }
    }
}

/// The game `lemonup find` works on: what the person sees first, so a `--link` with several
/// games never leaves them guessing which one it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindTarget {
    /// The name LemonUp shows for the game, as `lemonup profiles` and the TUI header do.
    pub(crate) game: String,
    pub(crate) profile: String,
    pub(crate) addon_dir: PathBuf,
}

impl FindTarget {
    /// The game of `profile` as `profiles` lists it, else the name of the version `addon_dir`
    /// belongs to (a profile list that can't be read, or an `--addon-dir` of another version).
    pub(crate) fn new(
        profile: &str,
        addon_dir: &Path,
        flavor: GameFlavor,
        profiles: &[lemonup_core::ProfileSummary],
    ) -> Self {
        let game = profiles
            .iter()
            .find(|summary| summary.name == profile && summary.flavor == Some(flavor))
            .map(|summary| summary.display_name.clone())
            .unwrap_or_else(|| lemonup_core::profiles::game_name(flavor).to_string());
        Self {
            game,
            profile: profile.to_string(),
            addon_dir: addon_dir.to_path_buf(),
        }
    }
}

/// The first line of `lemonup find --plain`.
pub(crate) fn target_plain_line(target: &FindTarget) -> String {
    format!(
        "find target: profile={}, game={}, addon_dir={}",
        target.profile,
        target.game,
        target.addon_dir.display()
    )
}

/// What `lemonup find` prints for a report: problems, a summary by label, then every result
/// grouped by label.
pub(crate) fn report_lines(
    report: &FindReport,
    problems: &[SourceProblem],
    folders_read: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    if report.skipped.contains(&FindSource::Wago) {
        lines.push(
            "find note: A Wago.io key finds more of your addons. Add one in Settings.".to_string(),
        );
    }
    for problem in problems {
        lines.push(format!(
            "find warning: source={}, problem={}, detail={}",
            problem.source.name(),
            problem_name(problem.error.kind),
            problem.error.detail
        ));
    }
    let count = |label: FindLabel| {
        report
            .results
            .iter()
            .filter(|result| result.label == label)
            .count()
    };
    lines.push(format!(
        "find summary: folders={folders_read}, addons={}, exact={}, likely={}, partial={}, choose_one={}, not_checked={}, curseforge_only={}, not_found={}, developer_copy={}",
        report.results.len(),
        count(FindLabel::Exact),
        count(FindLabel::Likely),
        count(FindLabel::Partial),
        count(FindLabel::ChooseOne),
        count(FindLabel::NotChecked),
        count(FindLabel::CurseForgeOnly),
        count(FindLabel::NotFound),
        count(FindLabel::DeveloperCopy),
    ));
    for result in &report.results {
        lines.extend(result_lines(result));
    }
    lines
}

/// `lemonup find --link` first reads the AddOns folder and brings the rows up to date, as a scan
/// does, so linking starts from how the folders are grouped now (a scan that failed earlier can
/// leave a row listing folders it no longer has). If that read fails, the rows stay as they are
/// and nothing is linked. `lemonup find` without `--link` changes nothing, so it doesn't scan.
pub(crate) fn scan_before_linking(
    database: &mut StateDatabase,
    addon_dir: &Path,
    flavor: GameFlavor,
) -> Result<(), LemonupError> {
    rescan_addons(database, addon_dir, flavor).map_err(|error| {
        LemonupError::InvalidArgument(format!(
            "could not read the AddOns folder, so nothing was linked: {error}"
        ))
    })
}

/// `scan_before_linking` with the scan's own error, for callers that say
/// what didn't happen themselves.
pub(crate) fn rescan_addons(
    database: &mut StateDatabase,
    addon_dir: &Path,
    flavor: GameFlavor,
) -> Result<(), LemonupError> {
    database
        .list_addons()
        .and_then(|tracked| {
            let report = scan_addons_dir_report(addon_dir, flavor, &tracked)?;
            database.reconcile_scan(&report)
        })
        .map(|_| ())
}

/// The candidates `lemonup find --link` links: every preselected (linkable `Exact`) one.
pub(crate) fn exact_picks(report: &FindReport) -> Vec<FoundAddon> {
    report
        .results
        .iter()
        .filter(|result| result.preselected())
        .filter_map(FindResult::found)
        .cloned()
        .collect()
}

/// What `lemonup find --link` did.
#[derive(Debug)]
pub(crate) enum LinkOutcome {
    /// No exact match can be linked.
    NothingToLink,
    /// Some exact matches move folders between addons and `--yes` was not given. Nothing was
    /// linked; these are the matches that move folders.
    NeedsYes(Vec<FoundAddon>),
    /// The rows the database holds after linking, and the names of linked addons it doesn't
    /// hold after all (a read of the AddOns folder after linking can drop one).
    Linked {
        rows: Vec<AddonRecord>,
        unconfirmed: Vec<String>,
    },
}

/// `lemonup find --link`: links the exact matches, then reads the AddOns folder again so
/// folders that left a group are listed as their own Not linked addons. The CLI never asks, so
/// when an exact match takes over folders beyond its own (folders joining from other addons or
/// leaving this one) it refuses without `--yes` and links nothing.
pub(crate) fn link_exact_matches(
    database: &mut StateDatabase,
    addon_dir: &Path,
    flavor: GameFlavor,
    report: &FindReport,
    yes: bool,
) -> Result<LinkOutcome, LemonupError> {
    let picks = exact_picks(report);
    if picks.is_empty() {
        return Ok(LinkOutcome::NothingToLink);
    }
    let regrouping = picks
        .iter()
        .filter(|pick| pick.regroups())
        .cloned()
        .collect::<Vec<_>>();
    if !regrouping.is_empty() && !yes {
        return Ok(LinkOutcome::NeedsYes(regrouping));
    }

    let linked = find::link_found_addons(database, addon_dir, flavor, &picks)?;
    // The list catches up on the next scan if this read fails.
    let rescan = database.list_addons().and_then(|tracked| {
        let report = scan_addons_dir_report(addon_dir, flavor, &tracked)?;
        database.reconcile_scan(&report)
    });
    if let Err(error) = rescan {
        tracing::warn!(%error, "could not read the AddOns folder after linking");
    }
    // Only what the database holds now is reported as linked.
    let rows = database.list_addons()?;
    let mut stored = Vec::new();
    let mut unconfirmed = Vec::new();
    for linked in &linked {
        match rows
            .iter()
            .find(|row| row.folder == linked.folder && row.source == linked.source)
        {
            Some(row) => stored.push(row.clone()),
            None => unconfirmed.push(linked.name.clone()),
        }
    }
    Ok(LinkOutcome::Linked {
        rows: stored,
        unconfirmed,
    })
}

/// The `--plain` lines that end the report: what `--link` did, or that nothing was changed.
pub(crate) fn plain_closing(
    report: &FindReport,
    link: Option<LinkOutcome>,
) -> Result<Vec<String>, LemonupError> {
    match link {
        Some(outcome) => plain_link_lines(outcome),
        None if !exact_picks(report).is_empty() => Ok(vec![
            "find detail: nothing was changed; `lemonup find --link` links the exact matches"
                .to_string(),
        ]),
        None => Ok(vec!["find detail: nothing was changed".to_string()]),
    }
}

/// The `--plain` lines for what `--link` did. A refusal is an error that prints as it is.
fn plain_link_lines(outcome: LinkOutcome) -> Result<Vec<String>, LemonupError> {
    match outcome {
        LinkOutcome::NothingToLink => Ok(vec![
            "find link: nothing to link, no exact match can be linked".to_string(),
        ]),
        LinkOutcome::NeedsYes(regrouping) => {
            let regrouping = regrouping
                .iter()
                .map(|pick| {
                    let mut moves = pick
                        .joining
                        .iter()
                        .map(|moved| {
                            if moved.is_own_addon() {
                                format!("{} joins (now its own addon)", moved.folder)
                            } else {
                                format!("{} joins from {}", moved.folder, moved.from)
                            }
                        })
                        .collect::<Vec<_>>();
                    moves.extend(pick.leaving.iter().map(|folder| format!("{folder} leaves")));
                    format!("{} ({})", pick.name, moves.join(", "))
                })
                .collect::<Vec<_>>();
            Err(LemonupError::Refused(format!(
                "these exact matches take over folders beyond their own: {}. Nothing was linked. \
                 Run `lemonup find --link --yes` to link them.",
                regrouping.join("; ")
            )))
        }
        LinkOutcome::Linked { rows, unconfirmed } => {
            let mut lines = rows
                .iter()
                .map(|stored| {
                    let (status, _) = crate::update::determine_update_status(stored);
                    format!(
                        "find linked: addon={}, source={}, version={}, latest={}, status={}, folders={}",
                        stored.name,
                        source_name_of(stored),
                        stored.version.as_deref().unwrap_or("<unknown>"),
                        stored.remote_version.as_deref().unwrap_or("<unknown>"),
                        crate::update::serialize_update_status(status),
                        std::iter::once(stored.folder.clone())
                            .chain(stored.owned_folders.iter().map(|owned| owned.name.clone()))
                            .collect::<Vec<_>>()
                            .join("|")
                    )
                })
                .collect::<Vec<_>>();
            lines.push(format!("find link summary: linked={}", rows.len()));
            if !unconfirmed.is_empty() {
                lines.push(format!(
                    "find link warning: not_confirmed={}",
                    join(&unconfirmed)
                ));
            }
            Ok(lines)
        }
    }
}

fn source_name_of(record: &AddonRecord) -> &'static str {
    match record.source {
        lemonup_core::SourceKind::Wago => "wago",
        lemonup_core::SourceKind::WowInterface => "wowinterface",
        lemonup_core::SourceKind::Tukui => "tukui",
        lemonup_core::SourceKind::GitHub => "github",
        lemonup_core::SourceKind::Manual => "manual",
    }
}

mod human;
pub(crate) use human::{
    LocalNames, Paint, choice_text, human_closing, human_report, linked_parts, listed_names,
    problem_text, result_entry, target_line,
};

#[cfg(test)]
mod tests;
