//! The readable report `lemonup find` prints by default: sections by how sure each result is,
//! an aligned line per addon, and a last line that says what happened. `--plain` prints the
//! key=value lines from the parent module instead.

use std::collections::{HashMap, HashSet};

use lemonup_core::find::{FindInputs, FindLabel, FindReport, FindResult, FindSource, FoundAddon};
use lemonup_core::{AddonRecord, LemonupError, UpdateStatus, toc_edit};

use super::{LinkOutcome, SourceProblem, exact_picks};
use crate::source_error::SourceErrorKind;

/// Longest line of a wrapped list or sentence. Addon lines are never wrapped or cut.
const WIDTH: usize = 78;
const INDENT: &str = "  ";
const SUB_INDENT: &str = "      ";

/// The names LemonUp shows for the addons in the AddOns folder, by parent folder: the name of
/// its row (its .toc Title, or the one the person set), else the Title of its folder.
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
            let name = row.name.trim();
            if !name.is_empty() {
                names.insert(row.folder.to_lowercase(), name.to_string());
            }
        }
        Self(names)
    }

    /// The name shown for the addon whose parent folder is `folder`, else `fallback`.
    fn name(&self, folder: &str, fallback: &str) -> String {
        self.0
            .get(&folder.to_lowercase())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }
}

/// One addon line: name, source and version in columns, then lines under it.
struct Entry {
    name: String,
    source: String,
    version: String,
    notes: Vec<String>,
}

fn pad(text: &str, width: usize) -> String {
    let missing = width.saturating_sub(text.chars().count());
    format!("{text}{}", " ".repeat(missing))
}

/// The lines of a section's addons, sorted by name, with the name and source columns aligned.
fn table(mut entries: Vec<Entry>) -> Vec<String> {
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    let name_width = entries
        .iter()
        .map(|entry| entry.name.chars().count())
        .max()
        .unwrap_or(0);
    let source_width = entries
        .iter()
        .map(|entry| entry.source.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for entry in entries {
        lines.push(format!(
            "{INDENT}{}  {}  {}",
            pad(&entry.name, name_width),
            pad(&entry.source, source_width),
            entry.version
        ));
        lines.extend(entry.notes.iter().map(|note| format!("{SUB_INDENT}{note}")));
    }
    lines
}

/// Fills lines of at most [`WIDTH`] columns with `units`, each kept whole.
fn wrap_units(units: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for unit in units {
        if line.is_empty() {
            line = format!("{INDENT}{unit}");
        } else if line.chars().count() + 1 + unit.chars().count() > WIDTH {
            lines.push(std::mem::take(&mut line));
            line = format!("{INDENT}{unit}");
        } else {
            line.push(' ');
            line.push_str(unit);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `items` as a wrapped, comma separated list.
fn wrapped_list(items: Vec<String>) -> Vec<String> {
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
    wrap_units(&units)
}

fn wrapped_text(text: &str) -> Vec<String> {
    wrap_units(
        &text
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    )
}

fn capitalize(text: &str) -> String {
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

fn notes_for_moves(found: &FoundAddon) -> Vec<String> {
    let mut notes = Vec::new();
    if !found.joining.is_empty() {
        let folders = found
            .joining
            .iter()
            .map(|moved| moved.folder.as_str())
            .collect::<Vec<_>>();
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
    let mut version = found
        .matched_version
        .clone()
        .unwrap_or_else(|| "unknown version".to_string());
    if let Some(latest) = update_ready(found) {
        version.push_str(&format!(", update ready ({latest})"));
    }
    let mut notes = notes_for_moves(found);
    notes.extend(notes_for_missing(found));
    Entry {
        name: names.name(&found.parent_folder, &found.name),
        source: found.source.name().to_string(),
        version,
        notes,
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

/// Why a `Likely` candidate is only likely.
fn likely_reason(found: &FoundAddon) -> String {
    if !found.differing.is_empty() {
        "its files differ from the release".to_string()
    } else if found.matched_version.is_none() {
        match found.latest_version.as_deref() {
            Some(latest) => format!("the installed version differs from the release ({latest})"),
            None => "the installed version differs from the release".to_string(),
        }
    } else {
        "its .toc names it, but its files were not checked".to_string()
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
        version: match found.matched_version.as_deref() {
            Some(installed) => format!("{installed}, {reason}"),
            None => reason,
        },
        notes,
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
        notes,
    }
}

/// `Questie: Questie (QuestieDB differs from the release) or QuestieDev`.
fn choose_one_line(result: &FindResult, names: &LocalNames) -> String {
    let mut seen = HashSet::new();
    let repeated = result
        .candidates
        .iter()
        .any(|candidate| !seen.insert(candidate.name.to_lowercase()));
    let candidates = result
        .candidates
        .iter()
        .map(|candidate| {
            let mut text = candidate.name.clone();
            if repeated {
                text.push_str(&format!(" on {}", candidate.source.name()));
            }
            if !candidate.differing.is_empty() {
                text.push_str(&format!(
                    " ({} {} from the release)",
                    candidate.differing.join(", "),
                    plural(candidate.differing.len(), "differs", "differ")
                ));
            }
            text
        })
        .collect::<Vec<_>>();
    let joined = match candidates.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    };
    format!(
        "{INDENT}{}: {joined}",
        names.name(&result.folder, &result.name)
    )
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
fn problem_line(problem: &SourceProblem) -> Vec<String> {
    let provider = problem.source.name();
    let mut text = format!(
        "{provider}: {}",
        crate::app::render_install_search::source_error_advice(problem.error.kind, provider)
    );
    if problem.error.kind == SourceErrorKind::Other && !problem.error.detail.is_empty() {
        text.push_str(&format!(" Details: {}", problem.error.detail));
    }
    wrapped_text(&text)
}

fn found_of<'a>(results: &[&'a FindResult]) -> Vec<&'a FoundAddon> {
    results.iter().filter_map(|result| result.found()).collect()
}

fn section(lines: &mut Vec<String>, title: &str, count: usize, body: Vec<String>) {
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{title} ({count})"));
    lines.extend(body);
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
pub(crate) fn human_report(
    report: &FindReport,
    problems: &[SourceProblem],
    names: &LocalNames,
    wago_folders: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(note) = note(report, problems, wago_folders) {
        lines.push(note);
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
        section(&mut lines, "Ready to link", entries.len(), table(entries));
    }
    if !blocked.is_empty() {
        let entries = found_of(&blocked)
            .into_iter()
            .map(|found| cant_link_entry(found, names))
            .collect::<Vec<_>>();
        section(&mut lines, "Can't link yet", entries.len(), table(entries));
    }
    let likely = with(FindLabel::Likely);
    if !likely.is_empty() {
        let entries = found_of(&likely)
            .into_iter()
            .map(|found| likely_entry(found, names))
            .collect::<Vec<_>>();
        section(&mut lines, "Likely matches", entries.len(), table(entries));
    }
    let partial = with(FindLabel::Partial);
    if !partial.is_empty() {
        let entries = found_of(&partial)
            .into_iter()
            .map(|found| partial_entry(found, names))
            .collect::<Vec<_>>();
        section(&mut lines, "Partial matches", entries.len(), table(entries));
    }
    let choose = with(FindLabel::ChooseOne);
    if !choose.is_empty() {
        let mut body = choose
            .iter()
            .map(|result| {
                (
                    names.name(&result.folder, &result.name).to_lowercase(),
                    choose_one_line(result, names),
                )
            })
            .collect::<Vec<_>>();
        body.sort();
        section(
            &mut lines,
            "Choose one in LemonUp",
            choose.len(),
            body.into_iter().map(|(_, line)| line).collect(),
        );
    }
    let not_checked = with(FindLabel::NotChecked);
    let asked_in_vain = not_checked
        .iter()
        .flat_map(|result| result.not_checked_by.iter().copied())
        .collect::<HashSet<_>>();
    if !not_checked.is_empty() {
        let mut body = wrapped_list(name_list(&not_checked, names));
        for problem in problems
            .iter()
            .filter(|problem| asked_in_vain.contains(&problem.source))
        {
            body.extend(problem_line(problem));
        }
        section(&mut lines, "Not checked", not_checked.len(), body);
    }
    let unexplained = problems
        .iter()
        .filter(|problem| !asked_in_vain.contains(&problem.source))
        .collect::<Vec<_>>();
    if !unexplained.is_empty() {
        let body = unexplained
            .iter()
            .flat_map(|problem| problem_line(problem))
            .collect();
        section(&mut lines, "Check failed", unexplained.len(), body);
    }
    for (label, title) in [
        (
            FindLabel::CurseForgeOnly,
            "On CurseForge only, LemonUp can't update these yet",
        ),
        (FindLabel::NotFound, "Not found"),
        (FindLabel::DeveloperCopy, "Skipped, developer copy"),
    ] {
        let results = with(label);
        if !results.is_empty() {
            section(
                &mut lines,
                title,
                results.len(),
                wrapped_list(name_list(&results, names)),
            );
        }
    }
    if report.results.is_empty() {
        let text = "No addons to look up.".to_string();
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(text);
    }
    lines
}

/// The words for what a link did to the addons that were linked.
fn linked_summary(rows: &[AddonRecord], unconfirmed: &[String]) -> Vec<String> {
    let mut up_to_date = 0;
    let mut newer = 0;
    let mut unchecked = 0;
    let mut failed = 0;
    let mut ready = Vec::new();
    for row in rows {
        match crate::update::determine_update_status(row).0 {
            UpdateStatus::UpToDate => up_to_date += 1,
            UpdateStatus::UpdateAvailable => ready.push(row.name.clone()),
            UpdateStatus::RemoteOlder => newer += 1,
            UpdateStatus::Unknown => unchecked += 1,
            UpdateStatus::Error => failed += 1,
        }
    }
    ready.sort_by_key(|name| name.to_lowercase());
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
    let mut lines = Vec::new();
    if !rows.is_empty() {
        let mut line = format!(
            "Linked {} {}: {}.",
            rows.len(),
            plural(rows.len(), "addon", "addons"),
            parts.join(", ")
        );
        if !ready.is_empty() {
            line.push_str(&format!(
                " `lemonup update-all` updates {}.",
                plural(ready.len(), "it", "them")
            ));
        }
        lines.push(line);
    }
    if !unconfirmed.is_empty() {
        let leading = if rows.is_empty() {
            "LemonUp could not confirm that these were linked"
        } else {
            "LemonUp could not confirm that these were also linked"
        };
        lines.push(format!(
            "{leading}: {}. Run `lemonup find` again to see where they stand.",
            unconfirmed.join(", ")
        ));
    }
    lines
}

/// "GatherMate2 also takes A, B; C becomes its own addon" for each match that moves folders.
fn moves_text(regrouping: &[FoundAddon], names: &LocalNames) -> String {
    let mut clauses = Vec::new();
    for pick in regrouping {
        let name = names.name(&pick.parent_folder, &pick.name);
        if !pick.joining.is_empty() {
            let folders = pick
                .joining
                .iter()
                .map(|moved| moved.folder.as_str())
                .collect::<Vec<_>>();
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

/// The lines that end the report, saying what happened: nothing was changed, nothing was
/// linked, or what was linked. A refusal (`--link` without `--yes` when matches move folders)
/// is an error that prints as it is.
pub(crate) fn human_closing(
    report: &FindReport,
    names: &LocalNames,
    link: Option<LinkOutcome>,
) -> Result<Vec<String>, LemonupError> {
    let ready = exact_picks(report).len();
    let Some(outcome) = link else {
        let mut line = "Nothing was changed.".to_string();
        if ready > 0 {
            line.push_str(&format!(
                " `lemonup find --link` links the {}ready to link.",
                if ready == 1 {
                    "addon ".to_string()
                } else {
                    format!("{ready} addons ")
                }
            ));
        }
        return Ok(vec![line]);
    };
    match outcome {
        LinkOutcome::NothingToLink => Ok(vec![
            "Nothing was linked. No addon is ready to link.".to_string(),
        ]),
        LinkOutcome::NeedsYes(regrouping) => Err(LemonupError::Refused(format!(
            "{} of these {} folders between addons: {}. Nothing was linked. \
             Run `lemonup find --link --yes` to link the {}.",
            regrouping.len(),
            plural(regrouping.len(), "moves", "move"),
            moves_text(&regrouping, names),
            if ready == 1 {
                "addon ready to link".to_string()
            } else {
                format!("{ready} addons ready to link")
            }
        ))),
        LinkOutcome::Linked { rows, unconfirmed } => Ok(linked_summary(&rows, &unconfirmed)),
    }
}
