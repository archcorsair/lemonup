//! The readable report `lemonup find` prints by default: sections by how sure each result is,
//! an aligned line per addon, and a last line that says what happened. `--plain` prints the
//! key=value lines from the parent module instead.

use std::collections::{HashMap, HashSet};

use lemonup_core::find::{
    FindInputs, FindLabel, FindReport, FindResult, FindSource, FolderMove, FoundAddon, LikelyReason,
};
use lemonup_core::{AddonRecord, UpdateStatus, toc_edit};

use super::{FindTarget, LinkOutcome, SourceProblem, exact_picks};
use crate::output::{Entry, INDENT, Paint, Tone, section, table, wrap_units, wrap_words};
use crate::source_error::SourceErrorKind;

/// The names LemonUp shows for the addons in the AddOns folder, by parent folder: the name of
/// its row (its .toc Title, or the one the person set), else the Title of its folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalNames(HashMap<String, String>);

impl LocalNames {
    pub(crate) fn new(rows: &[AddonRecord], inputs: &FindInputs) -> Self {
        let mut names = HashMap::new();
        for local in &inputs.folders {
            let title = local
                .title
                .as_deref()
                .map(toc_edit::display_value)
                .filter(|title| !title.is_empty());
            if let Some(title) = title {
                names.insert(local.folder.to_lowercase(), title);
            }
        }
        for row in rows {
            let name = collapse_spaces(&row.name);
            if !name.is_empty() {
                names.insert(row.folder.to_lowercase(), name);
            }
        }
        Self(names)
    }

    /// The name shown for the addon whose parent folder is `folder`, else `fallback`.
    pub(crate) fn name(&self, folder: &str, fallback: &str) -> String {
        self.0
            .get(&folder.to_lowercase())
            .cloned()
            .unwrap_or_else(|| collapse_spaces(fallback))
    }
}

/// A name with runs of whitespace as one space, so a name never shows doubled spaces.
fn collapse_spaces(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line of an `Exact`, `Likely` or `Partial` result in the words of the report, for the
/// TUI's Find sheet.
pub(crate) fn result_entry(result: &FindResult, names: &LocalNames) -> Option<Entry> {
    let found = result.found()?;
    Some(match result.label {
        FindLabel::Exact if result.preselected() => ready_entry(found, names),
        FindLabel::Exact => cant_link_entry(found, names),
        FindLabel::Likely => likely_entry(found, names),
        _ => partial_entry(found, names),
    })
}

/// One choice of a "Choose one" result, as the report names it: its folder, the source when
/// two choices share a folder, and why it is a choice.
pub(crate) fn choice_text(result: &FindResult, candidate: &FoundAddon) -> String {
    let repeated = result
        .candidates
        .iter()
        .filter(|other| {
            other
                .parent_folder
                .eq_ignore_ascii_case(&candidate.parent_folder)
        })
        .count()
        > 1;
    let mut text = candidate.parent_folder.clone();
    if repeated {
        text.push_str(&format!(" on {}", candidate.source.name()));
    }
    if let Some(why) = choice_why(candidate) {
        text.push_str(&format!(" ({why})"));
    }
    text
}

/// The names of name-only results, as the report lists them.
pub(crate) fn listed_names(results: &[&FindResult], names: &LocalNames) -> Vec<String> {
    name_list(results, names)
}

/// What a failed source means, in the words of the report.
pub(crate) fn problem_text(problem: &SourceProblem) -> String {
    let provider = problem.source.name();
    let mut text = format!(
        "{provider}: {}",
        crate::app::render_install_search::source_error_advice(
            problem.error.kind,
            provider,
            crate::github::github_token_source()
        )
    );
    if problem.error.kind == SourceErrorKind::Other && !problem.error.detail.is_empty() {
        text.push_str(&format!(" Details: {}", problem.error.detail));
    }
    text
}

/// `items` as a wrapped, comma separated list. The items are names from files and from the
/// sources, so each line is drawn as text.
fn wrapped_list(items: Vec<String>, paint: Paint) -> Vec<String> {
    let last = items.len().saturating_sub(1);
    let units = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            if index == last {
                item
            } else {
                format!("{item},")
            }
        })
        .collect::<Vec<_>>();
    wrap_units(INDENT, &units, paint.width())
        .iter()
        .map(|line| paint.text(line))
        .collect()
}

/// `text` wrapped and drawn as text: it can hold names, or a source's own words.
fn wrapped_text(text: &str, paint: Paint) -> Vec<String> {
    wrap_words(INDENT, text, paint.width())
        .iter()
        .map(|line| paint.text(line))
        .collect()
}

pub(crate) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// The latest version when it is known to differ from the installed one.
fn update_ready(found: &FoundAddon) -> Option<&str> {
    match (
        found.matched_version.as_deref(),
        found.latest_version.as_deref(),
    ) {
        (Some(installed), Some(latest))
            if crate::update::normalize_version(installed)
                != crate::update::normalize_version(latest) =>
        {
            Some(latest)
        }
        _ => None,
    }
}

/// Whether `moved` is the addon's own folder, which another row holds now.
fn is_own_folder(found: &FoundAddon, moved: &FolderMove) -> bool {
    moved.folder.to_lowercase() == found.parent_folder.to_lowercase()
}

fn notes_for_moves(found: &FoundAddon, names: &LocalNames) -> Vec<String> {
    let mut notes = Vec::new();
    for moved in &found.joining {
        if is_own_folder(found, moved) && !moved.is_own_addon() {
            notes.push(format!(
                "moves out of {}",
                names.name(&moved.from, &moved.from)
            ));
        }
    }
    let folders = found
        .joining
        .iter()
        .filter(|moved| !is_own_folder(found, moved))
        .map(|moved| moved.folder.as_str())
        .collect::<Vec<_>>();
    if !folders.is_empty() {
        notes.push(format!("also takes {}", folders.join(", ")));
    }
    notes.extend(
        found
            .leaving
            .iter()
            .map(|folder| format!("{folder} becomes its own addon")),
    );
    notes
}

fn notes_for_missing(found: &FoundAddon) -> Vec<String> {
    if found.missing.is_empty() {
        Vec::new()
    } else {
        vec![format!("{} missing on disk", found.missing.join(", "))]
    }
}

fn ready_entry(found: &FoundAddon, names: &LocalNames) -> Entry {
    let version = found
        .matched_version
        .as_deref()
        .map_or_else(|| "unknown version".to_string(), collapse_spaces);
    let mut notes = notes_for_moves(found, names);
    notes.extend(notes_for_missing(found));
    Entry {
        name: names.name(&found.parent_folder, &found.name),
        source: found.source.name().to_string(),
        version,
        update: update_ready(found)
            .map(|latest| format!("update ready ({})", collapse_spaces(latest))),
        notes,
        hint: None,
        from: None,
        done: false,
    }
}

fn cant_link_entry(found: &FoundAddon, names: &LocalNames) -> Entry {
    let mut entry = ready_entry(found, names);
    entry.notes.extend(
        found
            .not_linkable_reason()
            .map(|reason| capitalize(&reason)),
    );
    entry
}

/// Why a `Likely` candidate is only likely: what was checked, from the facts core carries.
fn likely_reason(found: &FoundAddon) -> String {
    let source = found.source.name();
    match &found.likely_reason {
        Some(LikelyReason::FilesMatchNoRelease) => {
            format!("its .toc names it; its files match no release on {source}")
        }
        Some(LikelyReason::VersionDiffers {
            folder,
            installed,
            listed,
        }) => {
            let installed = match installed {
                Some(installed) => format!("installed {installed}"),
                None => "no installed version".to_string(),
            };
            let folder = if folder.eq_ignore_ascii_case(&found.parent_folder) {
                String::new()
            } else {
                format!(" in {folder}")
            };
            match listed {
                Some(listed) => format!("{installed}{folder}, {source} has {listed}"),
                None => format!("{installed}{folder}, {source} lists no version for it"),
            }
        }
        Some(LikelyReason::TocNamesIt) => {
            "its .toc names it; its files weren't checked".to_string()
        }
        None => "it is not confirmed".to_string(),
    }
}

fn likely_entry(found: &FoundAddon, names: &LocalNames) -> Entry {
    let mut notes = notes_for_missing(found);
    notes.extend(
        found
            .not_linkable_reason()
            .map(|reason| capitalize(&reason)),
    );
    let reason = likely_reason(found);
    Entry {
        name: names.name(&found.parent_folder, &found.name),
        source: found.source.name().to_string(),
        // A version difference already names the installed version.
        version: match (&found.likely_reason, found.matched_version.as_deref()) {
            (Some(LikelyReason::VersionDiffers { .. }), _) | (_, None) => reason,
            (_, Some(installed)) => format!("{installed}, {reason}"),
        },
        update: None,
        notes,
        hint: None,
        from: None,
        done: false,
    }
}

fn partial_entry(found: &FoundAddon, names: &LocalNames) -> Entry {
    let installed = found
        .matched_version
        .as_deref()
        .unwrap_or("unknown version");
    let version = if found.differing.is_empty() {
        installed.to_string()
    } else {
        format!(
            "{installed}, files differ in {}",
            found.differing.join(", ")
        )
    };
    let mut notes = notes_for_missing(found);
    notes.extend(
        found
            .not_linkable_reason()
            .map(|reason| capitalize(&reason)),
    );
    Entry {
        name: names.name(&found.parent_folder, &found.name),
        source: found.source.name().to_string(),
        version,
        update: None,
        notes,
        hint: None,
        from: None,
        done: false,
    }
}

/// Why a candidate is one of the choices, in a few words.
fn choice_why(candidate: &FoundAddon) -> Option<String> {
    if !candidate.differing.is_empty() {
        return Some(format!(
            "{} {} from the release",
            candidate.differing.join(", "),
            plural(candidate.differing.len(), "differs", "differ")
        ));
    }
    match candidate.label {
        FindLabel::Exact => Some("exact match".to_string()),
        FindLabel::Likely => Some(format!("likely, same {} project", candidate.source.name())),
        _ => None,
    }
}

/// `QuestieDev (likely, same Wago.io project) or Questie (QuestieDB differs from the release)`,
/// wrapped. The choices are named by their folders.
fn choose_one_lines(result: &FindResult, paint: Paint) -> Vec<String> {
    let candidates = result
        .candidates
        .iter()
        .map(|candidate| choice_text(result, candidate))
        .collect::<Vec<_>>();
    let joined = match candidates.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    };
    wrapped_text(&joined, paint)
}

/// The names of name-only results, sorted. Results that share one CurseForge project are one
/// name with the number of folders.
fn name_list(results: &[&FindResult], names: &LocalNames) -> Vec<String> {
    struct Group {
        name: String,
        results: usize,
        folders: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut by_project: HashMap<&str, usize> = HashMap::new();
    for result in results {
        let folders = result.folders.len().max(1);
        let project = result.curseforge_id.as_deref().filter(|id| !id.is_empty());
        if let Some(index) = project.and_then(|id| by_project.get(id)).copied() {
            groups[index].results += 1;
            groups[index].folders += folders;
            continue;
        }
        if let Some(id) = project {
            by_project.insert(id, groups.len());
        }
        groups.push(Group {
            name: names.name(&result.folder, &result.name),
            results: 1,
            folders,
        });
    }
    groups.sort_by_key(|group| group.name.to_lowercase());
    groups
        .into_iter()
        .map(|group| {
            if group.results > 1 {
                format!("{} ({} folders)", group.name, group.folders)
            } else {
                group.name
            }
        })
        .collect()
}

/// What a failed source means for the addons it couldn't check.
fn problem_line(problem: &SourceProblem, paint: Paint) -> Vec<String> {
    wrapped_text(&problem_text(problem), paint)
}

fn found_of<'a>(results: &[&'a FindResult]) -> Vec<&'a FoundAddon> {
    results.iter().filter_map(|result| result.found()).collect()
}

/// The note that opens the report: what was sent to Wago.io, or that a key finds more.
fn note(report: &FindReport, problems: &[SourceProblem], wago_folders: usize) -> Option<String> {
    if report.skipped.contains(&FindSource::Wago) {
        return Some("A Wago.io key finds more of your addons. Add one in Settings.".to_string());
    }
    // A request that got no answer may not have arrived, so it is not claimed as sent.
    let unreachable = problems.iter().any(|problem| {
        problem.source == FindSource::Wago && problem.error.kind == SourceErrorKind::NoConnection
    });
    (wago_folders > 0 && !unreachable)
        .then(|| format!("Sent {wago_folders} folder names and file hashes to Wago.io."))
}

/// What `lemonup find` prints for a report: a note, then a section for each way a result can
/// be labeled that has results. `wago_folders` is how many folder names went to Wago.io.
/// The first line of the readable report: the game and its AddOns folder, and the profile
/// unless it is the default one.
pub(crate) fn target_line(target: &FindTarget, paint: Paint) -> String {
    crate::output::context_line(paint, &target.game, &target.addon_dir, &target.profile)
}

pub(crate) fn human_report(
    report: &FindReport,
    problems: &[SourceProblem],
    names: &LocalNames,
    wago_folders: usize,
    paint: Paint,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(note) = note(report, problems, wago_folders) {
        lines.push(paint.muted(&note));
    }
    let with = |label: FindLabel| {
        report
            .results
            .iter()
            .filter(|result| result.label == label)
            .collect::<Vec<_>>()
    };

    let (ready, blocked): (Vec<&FindResult>, Vec<&FindResult>) = with(FindLabel::Exact)
        .into_iter()
        .partition(|result| result.preselected());
    if !ready.is_empty() {
        let entries = found_of(&ready)
            .into_iter()
            .map(|found| ready_entry(found, names))
            .collect::<Vec<_>>();
        section(
            &mut lines,
            paint,
            Tone::Good,
            "Ready to link",
            entries.len(),
            table(entries, paint),
        );
    }
    if !blocked.is_empty() {
        let entries = found_of(&blocked)
            .into_iter()
            .map(|found| cant_link_entry(found, names))
            .collect::<Vec<_>>();
        section(
            &mut lines,
            paint,
            Tone::Warn,
            "Can't link yet",
            entries.len(),
            table(entries, paint),
        );
    }
    let likely = with(FindLabel::Likely);
    if !likely.is_empty() {
        let entries = found_of(&likely)
            .into_iter()
            .map(|found| likely_entry(found, names))
            .collect::<Vec<_>>();
        section(
            &mut lines,
            paint,
            Tone::Warn,
            "Likely matches",
            entries.len(),
            table(entries, paint),
        );
    }
    let partial = with(FindLabel::Partial);
    if !partial.is_empty() {
        let entries = found_of(&partial)
            .into_iter()
            .map(|found| partial_entry(found, names))
            .collect::<Vec<_>>();
        section(
            &mut lines,
            paint,
            Tone::Warn,
            "Partial matches",
            entries.len(),
            table(entries, paint),
        );
    }
    let choose = with(FindLabel::ChooseOne);
    if !choose.is_empty() {
        let mut body = choose
            .iter()
            .map(|result| {
                (
                    names.name(&result.folder, &result.name).to_lowercase(),
                    choose_one_lines(result, paint),
                )
            })
            .collect::<Vec<_>>();
        body.sort();
        section(
            &mut lines,
            paint,
            Tone::Warn,
            "Choose one in LemonUp",
            choose.len(),
            body.into_iter().flat_map(|(_, lines)| lines).collect(),
        );
    }
    let not_checked = with(FindLabel::NotChecked);
    let asked_in_vain = not_checked
        .iter()
        .flat_map(|result| result.not_checked_by.iter().copied())
        .collect::<HashSet<_>>();
    if !not_checked.is_empty() {
        let mut body = wrapped_list(name_list(&not_checked, names), paint);
        for problem in problems
            .iter()
            .filter(|problem| asked_in_vain.contains(&problem.source))
        {
            body.extend(problem_line(problem, paint));
        }
        section(
            &mut lines,
            paint,
            Tone::Problem,
            "Not checked",
            not_checked.len(),
            body,
        );
    }
    let unexplained = problems
        .iter()
        .filter(|problem| !asked_in_vain.contains(&problem.source))
        .collect::<Vec<_>>();
    if !unexplained.is_empty() {
        let body = unexplained
            .iter()
            .flat_map(|problem| problem_line(problem, paint))
            .collect();
        section(
            &mut lines,
            paint,
            Tone::Problem,
            "Check failed",
            unexplained.len(),
            body,
        );
    }
    for (label, title) in [
        (
            FindLabel::CurseForgeOnly,
            "On CurseForge only, LemonUp can't update these yet".to_string(),
        ),
        (FindLabel::NotFound, report.not_found_title()),
        (
            FindLabel::DeveloperCopy,
            "Skipped, developer copy".to_string(),
        ),
    ] {
        let results = with(label);
        if !results.is_empty() {
            section(
                &mut lines,
                paint,
                Tone::Quiet,
                &title,
                results.len(),
                wrapped_list(name_list(&results, names), paint),
            );
        }
    }
    if report.results.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(paint.text("No addons to look up."));
    }
    lines
}

/// How the linked addons stand, as the database holds them.
struct LinkedStatus {
    up_to_date: usize,
    /// The names of the addons with an update ready, sorted.
    ready: Vec<String>,
    newer: usize,
    unchecked: usize,
    failed: usize,
}

fn linked_status(rows: &[AddonRecord]) -> LinkedStatus {
    let mut status = LinkedStatus {
        up_to_date: 0,
        ready: Vec::new(),
        newer: 0,
        unchecked: 0,
        failed: 0,
    };
    for row in rows {
        match crate::update::determine_update_status(row).0 {
            UpdateStatus::UpToDate => status.up_to_date += 1,
            UpdateStatus::UpdateAvailable => status.ready.push(row.name.clone()),
            UpdateStatus::RemoteOlder => status.newer += 1,
            UpdateStatus::Unknown => status.unchecked += 1,
            UpdateStatus::Error => status.failed += 1,
        }
    }
    status.ready.sort_by_key(|name| name.to_lowercase());
    status
}

/// How the linked addons stand, as the database holds them: "2 up to date", "1 with an update
/// ready (X)", and the rest; and how many have an update ready.
pub(crate) fn linked_parts(rows: &[AddonRecord]) -> (Vec<String>, usize) {
    let LinkedStatus {
        up_to_date,
        ready,
        newer,
        unchecked,
        failed,
    } = linked_status(rows);
    let mut parts = Vec::new();
    if up_to_date > 0 {
        parts.push(format!("{up_to_date} up to date"));
    }
    if !ready.is_empty() {
        parts.push(format!(
            "{} with an update ready ({})",
            ready.len(),
            ready.join(", ")
        ));
    }
    if newer > 0 {
        parts.push(format!("{newer} newer than source"));
    }
    if unchecked > 0 {
        parts.push(format!("{unchecked} not checked yet"));
    }
    if failed > 0 {
        parts.push(format!("{failed} check failed"));
    }
    (parts, ready.len())
}

/// One thing the report ends with: what happened, and under it what to run next. `said` can
/// name addons, so nothing in it is shown as a command. Only `hint`, which LemonUp wrote,
/// names commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Closing {
    /// Something was done: the line starts with the done mark.
    done: bool,
    said: String,
    hint: Option<String>,
}

impl Closing {
    fn said(said: impl Into<String>) -> Self {
        Self {
            done: false,
            said: said.into(),
            hint: None,
        }
    }

    fn with_hint(said: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            hint: Some(hint.into()),
            ..Self::said(said)
        }
    }
}

/// How to run LemonUp again on the same game: `--profile` unless it is the default one, and
/// `--addon-dir` again when the run was given one. A command for another game would link or
/// update the wrong addons.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rerun<'a> {
    pub(crate) profile: &'a str,
    pub(crate) addon_dir_given: bool,
}

impl Rerun<'_> {
    /// "Run `lemonup find --link`", for the same game.
    fn run(self, command: &str) -> String {
        let lemonup = if self.profile == lemonup_core::DEFAULT_PROFILE {
            "lemonup".to_string()
        } else {
            format!("lemonup --profile {}", self.profile)
        };
        let with = if self.addon_dir_given {
            " with the same `--addon-dir`"
        } else {
            ""
        };
        format!("Run `{lemonup} {command}`{with}")
    }
}

/// The words for what a link did to the addons that were linked: how many, and how they
/// stand, as `lemonup check` counts them. The report above names the ones with an update.
fn linked_summary(
    rows: &[AddonRecord],
    unconfirmed: &[AddonRecord],
    rerun: Rerun<'_>,
) -> Vec<Closing> {
    let mut lines = Vec::new();
    if !rows.is_empty() {
        let status = linked_status(rows);
        let ready = status.ready.len();
        let parts = [
            (status.up_to_date, "up to date", "up to date"),
            (ready, "update ready", "updates ready"),
            (status.newer, "newer than source", "newer than source"),
            (status.unchecked, "not checked yet", "not checked yet"),
            (status.failed, "check failed", "checks failed"),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, one, many)| format!("{count} {}", plural(count, one, many)))
        .collect::<Vec<_>>();
        lines.push(Closing {
            done: true,
            said: format!(
                "Linked {} {} - {}",
                rows.len(),
                plural(rows.len(), "addon", "addons"),
                parts.join(", ")
            ),
            hint: (ready > 0).then(|| {
                format!(
                    "{} to update {}",
                    rerun.run("update-all"),
                    plural(ready, "it", "them")
                )
            }),
        });
    }
    if !unconfirmed.is_empty() {
        let leading = if rows.is_empty() {
            "Could not confirm these were linked"
        } else {
            "Could not confirm these were also linked"
        };
        let names = unconfirmed
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>();
        lines.push(Closing::with_hint(
            format!("{leading}: {}", names.join(", ")),
            format!("{} again to see where they stand", rerun.run("find")),
        ));
    }
    lines
}

/// "GatherMate2 also takes A, B; C becomes its own addon" for each match that moves folders.
fn moves_text(regrouping: &[FoundAddon], names: &LocalNames) -> String {
    let mut clauses = Vec::new();
    for pick in regrouping {
        let name = names.name(&pick.parent_folder, &pick.name);
        for moved in &pick.joining {
            if is_own_folder(pick, moved) && !moved.is_own_addon() {
                clauses.push(format!(
                    "{name} moves out of {}",
                    names.name(&moved.from, &moved.from)
                ));
            }
        }
        let folders = pick
            .joining
            .iter()
            .filter(|moved| !is_own_folder(pick, moved))
            .map(|moved| moved.folder.as_str())
            .collect::<Vec<_>>();
        if !folders.is_empty() {
            clauses.push(format!("{name} also takes {}", folders.join(", ")));
        }
        match pick.leaving.as_slice() {
            [] => {}
            [folder] => clauses.push(format!("{folder} becomes its own addon")),
            folders => clauses.push(format!("{} become their own addons", folders.join(", "))),
        }
    }
    clauses.join("; ")
}

/// The closing lines as stdout gets them: the done mark before something that was done,
/// and each hint on a line of its own with the commands it names drawn as commands.
pub(super) fn draw_closing(lines: &[Closing], paint: Paint) -> Vec<String> {
    let mut drawn = Vec::new();
    for line in lines {
        drawn.push(if line.done {
            format!("{} {}", paint.done(), paint.text(&line.said))
        } else {
            paint.text(&line.said)
        });
        if let Some(hint) = &line.hint {
            drawn.push(paint.commands(hint, Paint::text));
        }
    }
    drawn
}

/// How many addons are ready to link, in words: "the addon ready to link".
fn ready_to_link(ready: usize) -> String {
    if ready == 1 {
        "the addon ready to link".to_string()
    } else {
        format!("the {ready} addons ready to link")
    }
}

/// Why `find --link` refuses without `--yes`, and what to run instead. The first names addons
/// and folders, so only the second is shown as naming a command.
pub(crate) fn needs_yes_refusal(
    report: &FindReport,
    names: &LocalNames,
    regrouping: &[FoundAddon],
    rerun: Rerun<'_>,
) -> (String, String) {
    (
        format!(
            "Nothing linked - {} {} folders between addons: {}",
            regrouping.len(),
            plural(regrouping.len(), "match moves", "matches move"),
            moves_text(regrouping, names),
        ),
        format!(
            "{} to link {}",
            rerun.run("find --link --yes"),
            ready_to_link(exact_picks(report).len())
        ),
    )
}

/// What ends the report, saying what happened: nothing changed, nothing was linked, or what
/// was linked. `--link` without `--yes` when matches move folders is a refusal: its words
/// are those of [`needs_yes_refusal`], which `lemonup find` prints on stderr instead.
pub(super) fn closing_text(
    report: &FindReport,
    names: &LocalNames,
    link: Option<LinkOutcome>,
    rerun: Rerun<'_>,
) -> Vec<Closing> {
    let ready = exact_picks(report).len();
    let Some(outcome) = link else {
        let link = rerun.run("find --link");
        return vec![match ready {
            0 => Closing::said("Nothing changed"),
            1 => Closing::with_hint(
                "Nothing changed - 1 addon ready to link",
                format!("{link} to link it"),
            ),
            ready => Closing::with_hint(
                format!("Nothing changed - {ready} addons ready to link"),
                format!("{link} to link them"),
            ),
        }];
    };
    match outcome {
        LinkOutcome::NothingToLink => {
            vec![Closing::said("Nothing linked - no addon is ready to link")]
        }
        LinkOutcome::NeedsYes(regrouping) => {
            let (reason, hint) = needs_yes_refusal(report, names, &regrouping, rerun);
            vec![Closing::with_hint(reason, hint)]
        }
        LinkOutcome::Linked {
            rows,
            unconfirmed,
            curseforge,
        } => {
            let mut lines = linked_summary(&rows, &unconfirmed, rerun);
            if !curseforge.is_empty() {
                lines.push(Closing::said(lemonup_core::curseforge::hint_naming(
                    &super::curseforge_names(&rows, &curseforge),
                )));
            }
            lines
        }
    }
}
