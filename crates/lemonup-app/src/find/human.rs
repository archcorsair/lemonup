//! The readable report `lemonup find` prints by default: sections by how sure each result is,
//! an aligned line per addon, and a last line that says what happened. `--plain` prints the
//! key=value lines from the parent module instead.

use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;

use crossterm::Command;
use crossterm::style::{Attribute, Colored, SetAttribute, SetForegroundColor};
use lemonup_core::find::{
    FindInputs, FindLabel, FindReport, FindResult, FindSource, FolderMove, FoundAddon, LikelyReason,
};
use lemonup_core::{AddonRecord, AppConfig, LemonupError, UpdateStatus, toc_edit};
use ratatui::backend::IntoCrossterm;
use ratatui::style::Color;

use super::{FindTarget, LinkOutcome, SourceProblem, exact_picks};
use crate::source_error::SourceErrorKind;
use crate::theme::{ColorRoles, DisplayEnv, UiTheme};

/// Longest line of a wrapped list or sentence when the terminal's width is unknown. Addon lines
/// are never wrapped or cut.
const WIDTH: usize = 78;
/// The narrowest and widest the report is wrapped to on a terminal.
const MIN_WIDTH: usize = 60;
const MAX_WIDTH: usize = 120;
const INDENT: &str = "  ";
const SUB_INDENT: &str = "      ";

/// What a section heading means, which picks its color role.
#[derive(Clone, Copy)]
enum Tone {
    /// Ready to link: the success role.
    Good,
    /// Needs a look or a choice: the warning role.
    Warn,
    /// A source failed: the problem role.
    Problem,
    /// Nothing LemonUp can do: bold in the normal text role, so it stays readable.
    Quiet,
}

/// Whether this console understands escape codes. Windows needs them switched on first.
#[cfg(windows)]
fn ansi_supported() -> bool {
    crossterm::ansi_support::supports_ansi()
}

#[cfg(not(windows))]
fn ansi_supported() -> bool {
    true
}

/// Whether the report is drawn in color: a terminal, and `NO_COLOR` not set.
pub(crate) fn color_wanted(is_terminal: bool, display: DisplayEnv) -> bool {
    is_terminal && !display.no_color
}

/// The columns the report is wrapped to: the terminal's width less a column of margin, kept
/// between 60 and 120. Without a terminal, or when its size can't be read, 78.
pub(crate) fn report_width(is_terminal: bool, columns: Option<u16>) -> usize {
    match columns {
        Some(columns) if is_terminal => {
            usize::from(columns.saturating_sub(1)).clamp(MIN_WIDTH, MAX_WIDTH)
        }
        _ => WIDTH,
    }
}

/// How the readable report is drawn: its color and the columns it wraps to. Colors are the
/// color roles of the person's theme, as the TUI shows them. It is a
/// value, not a look at the environment, so tests render both ways and at any width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint(Option<ColorRoles>, usize);

impl Paint {
    /// No escape codes at all, wrapped at 78 columns.
    pub(crate) const PLAIN: Self = Self(None, WIDTH);

    /// The same drawing, wrapped to `width` columns.
    pub(crate) fn with_width(self, width: usize) -> Self {
        Self(self.0, width)
    }

    /// The Terminal theme's colors.
    #[cfg(test)]
    pub(crate) fn terminal() -> Self {
        Self::with_roles(ColorRoles::terminal())
    }

    /// Drawn with `roles`.
    pub(crate) fn with_roles(roles: ColorRoles) -> Self {
        // The decision to use color was made by the caller. Without this, crossterm would drop
        // every color again whenever `NO_COLOR` is set in the environment it runs in.
        Colored::set_ansi_color_disabled(false);
        Self(Some(roles), WIDTH)
    }

    /// Colored for a stream that `is_terminal` (and that `display` allows), else plain. The
    /// colors are the ones the TUI shows for `config`, including its fall back to the
    /// terminal's own 16 colors when `display` has no 24-bit color.
    pub(crate) fn for_stream(is_terminal: bool, display: DisplayEnv, config: &AppConfig) -> Self {
        if color_wanted(is_terminal, display) {
            let theme = UiTheme::for_config(config, display, !display.truecolor);
            Self::with_roles(theme.roles)
        } else {
            Self::PLAIN
        }
    }

    pub(crate) fn for_stdout(config: &AppConfig) -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        let colors = is_terminal && ansi_supported();
        let columns = crossterm::terminal::size().ok().map(|(columns, _)| columns);
        Self::for_stream(colors, DisplayEnv::detect(), config)
            .with_width(report_width(is_terminal, columns))
    }

    pub(crate) fn for_stderr(config: &AppConfig) -> Self {
        let colors = std::io::stderr().is_terminal() && ansi_supported();
        Self::for_stream(colors, DisplayEnv::detect(), config)
    }

    fn apply(self, text: &str, pick: fn(&ColorRoles) -> Color, bold: bool) -> String {
        let Some(roles) = self.0 else {
            return text.to_string();
        };
        let color = pick(&roles);
        if text.is_empty() || (color == Color::Reset && !bold) {
            return text.to_string();
        }
        let mut out = String::new();
        if bold {
            let _ = SetAttribute(Attribute::Bold).write_ansi(&mut out);
        }
        if color != Color::Reset {
            let _ = SetForegroundColor(color.into_crossterm()).write_ansi(&mut out);
        }
        out.push_str(text);
        let _ = SetAttribute(Attribute::Reset).write_ansi(&mut out);
        out
    }

    fn text(self, text: &str) -> String {
        self.apply(text, |roles| roles.text, false)
    }

    fn muted(self, text: &str) -> String {
        self.apply(text, |roles| roles.muted, false)
    }

    /// The role of "Update ready" in the TUI.
    fn update_ready(self, text: &str) -> String {
        self.apply(text, |roles| roles.amber, false)
    }

    pub(crate) fn bold(self, text: &str) -> String {
        self.apply(text, |roles| roles.text, true)
    }

    fn heading(self, tone: Tone, text: &str) -> String {
        let pick: fn(&ColorRoles) -> Color = match tone {
            Tone::Good => |roles| roles.green,
            Tone::Warn => |roles| roles.amber,
            Tone::Problem => |roles| roles.red,
            Tone::Quiet => |roles| roles.text,
        };
        self.apply(text, pick, true)
    }
}

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

/// One addon line: name, source and version in columns, then lines under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) version: String,
    /// "update ready (x)", shown after the version in its own color.
    pub(crate) update: Option<String>,
    pub(crate) notes: Vec<String>,
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
        crate::app::render_install_search::source_error_advice(problem.error.kind, provider)
    );
    if problem.error.kind == SourceErrorKind::Other && !problem.error.detail.is_empty() {
        text.push_str(&format!(" Details: {}", problem.error.detail));
    }
    text
}

fn pad(text: &str, width: usize) -> String {
    let missing = width.saturating_sub(text.chars().count());
    format!("{text}{}", " ".repeat(missing))
}

/// The lines of a section's addons, sorted by name, with the name and source columns aligned.
fn table(mut entries: Vec<Entry>, paint: Paint) -> Vec<String> {
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
        let mut version = paint.text(&entry.version);
        if let Some(update) = &entry.update {
            version.push_str(&paint.text(", "));
            version.push_str(&paint.update_ready(update));
        }
        lines.push(format!(
            "{INDENT}{}  {}  {version}",
            paint.text(&pad(&entry.name, name_width)),
            paint.muted(&pad(&entry.source, source_width)),
        ));
        for note in &entry.notes {
            lines.extend(
                wrap_words(SUB_INDENT, note, paint.1)
                    .iter()
                    .map(|line| paint.muted(line)),
            );
        }
    }
    lines
}

/// Fills lines of at most `width` columns with `units`, each kept whole, after `indent`.
fn wrap_units(indent: &str, units: &[String], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for unit in units {
        if line.is_empty() {
            line = format!("{indent}{unit}");
        } else if line.chars().count() + 1 + unit.chars().count() > width {
            lines.push(std::mem::take(&mut line));
            line = format!("{indent}{unit}");
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
fn wrapped_list(items: Vec<String>, width: usize) -> Vec<String> {
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
    wrap_units(INDENT, &units, width)
}

/// `text` wrapped at spaces, every line after `indent`.
fn wrap_words(indent: &str, text: &str, width: usize) -> Vec<String> {
    wrap_units(
        indent,
        &text
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        width,
    )
}

fn wrapped_text(text: &str, width: usize) -> Vec<String> {
    wrap_words(INDENT, text, width)
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
fn choose_one_lines(result: &FindResult, width: usize) -> Vec<String> {
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
    wrap_words(INDENT, &joined, width)
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
fn problem_line(problem: &SourceProblem, width: usize) -> Vec<String> {
    wrapped_text(&problem_text(problem), width)
}

fn found_of<'a>(results: &[&'a FindResult]) -> Vec<&'a FoundAddon> {
    results.iter().filter_map(|result| result.found()).collect()
}

fn section(
    lines: &mut Vec<String>,
    paint: Paint,
    tone: Tone,
    title: &str,
    count: usize,
    body: Vec<String>,
) {
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!(
        "{} {}",
        paint.heading(tone, title),
        paint.muted(&format!("({count})"))
    ));
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
/// The first line of the readable report: the game and its AddOns folder, and the profile
/// unless it is the default one.
pub(crate) fn target_line(target: &FindTarget, paint: Paint) -> String {
    let mut line = format!(
        "{}  {}",
        paint.bold(&target.game),
        paint.muted(&crate::app::friendly_path(&target.addon_dir))
    );
    if target.profile != lemonup_core::DEFAULT_PROFILE {
        line.push_str("  ");
        line.push_str(&paint.muted(&format!("(profile {})", target.profile)));
    }
    line
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
                    choose_one_lines(result, paint.1),
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
        let mut body = wrapped_list(name_list(&not_checked, names), paint.1);
        for problem in problems
            .iter()
            .filter(|problem| asked_in_vain.contains(&problem.source))
        {
            body.extend(problem_line(problem, paint.1));
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
            .flat_map(|problem| problem_line(problem, paint.1))
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
                wrapped_list(name_list(&results, names), paint.1),
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

/// How the linked addons stand, as the database holds them: "2 up to date", "1 with an update
/// ready (X)", and the rest; and how many have an update ready.
pub(crate) fn linked_parts(rows: &[AddonRecord]) -> (Vec<String>, usize) {
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
    (parts, ready.len())
}

/// The words for what a link did to the addons that were linked.
fn linked_summary(rows: &[AddonRecord], unconfirmed: &[String]) -> Vec<String> {
    let (parts, ready) = linked_parts(rows);
    let mut lines = Vec::new();
    if !rows.is_empty() {
        let mut line = format!(
            "Linked {} {}: {}.",
            rows.len(),
            plural(rows.len(), "addon", "addons"),
            parts.join(", ")
        );
        if ready > 0 {
            line.push_str(&format!(
                " `lemonup update-all` updates {}.",
                plural(ready, "it", "them")
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

/// The lines that end the report, saying what happened: nothing was changed, nothing was
/// linked, or what was linked. A refusal (`--link` without `--yes` when matches move folders)
/// is an error that prints as it is.
pub(crate) fn human_closing(
    report: &FindReport,
    names: &LocalNames,
    link: Option<LinkOutcome>,
    paint: Paint,
    error_paint: Paint,
) -> Result<Vec<String>, LemonupError> {
    let lines = closing_text(report, names, link, error_paint)?;
    Ok(lines.iter().map(|line| paint.bold(line)).collect())
}

/// The closing lines as text. `error_paint` draws the refusal, which goes to stderr.
fn closing_text(
    report: &FindReport,
    names: &LocalNames,
    link: Option<LinkOutcome>,
    error_paint: Paint,
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
        LinkOutcome::NeedsYes(regrouping) => {
            Err(LemonupError::Refused(error_paint.bold(&format!(
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
            ))))
        }
        LinkOutcome::Linked { rows, unconfirmed } => Ok(linked_summary(&rows, &unconfirmed)),
    }
}
