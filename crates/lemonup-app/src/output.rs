//! How CLI commands print. A command builds a report; [`Output`] prints it as readable lines
//! on stdout, or with `--json` as one JSON object on one line. Warnings and notes go to stderr.
//! Colors are always the terminal's own 16 colors, whatever theme is saved: the CLI draws no
//! background, so a theme's text color could be close to invisible on the terminal's own.

mod check;
mod export;
mod import;
mod install;
mod profiles;
mod restore;
mod sync;
#[cfg(test)]
mod tests;
mod toc;
mod update;

pub(crate) use check::{CheckReport, UpdateHint};
pub(crate) use export::ExportReport;
pub(crate) use import::ImportReport;
pub(crate) use install::{
    InstallReport, InstallRequest, InstalledAddon, failure as install_failure,
};
pub(crate) use profiles::ProfilesReport;
pub(crate) use restore::RestoreReport;
pub(crate) use sync::SyncReport;
pub(crate) use toc::{TocChange, TocReport, TocRow};
pub(crate) use update::{BackupLine, UpdateFlags, UpdateReport};

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crossterm::Command;
use crossterm::style::{Attribute, Colored, SetAttribute, SetForegroundColor};
use lemonup_core::{AppConfig, SourceKind};
use ratatui::backend::IntoCrossterm;
use ratatui::style::{Color, Modifier, Style};
use serde::{Serialize, Serializer};

use crate::source_error::SourceErrorKind;
use crate::theme::{ColorRoles, DisplayEnv, Symbols};

/// Longest line of a wrapped list or sentence when the terminal's width is unknown. Table rows
/// are never wrapped or cut.
const WIDTH: usize = 78;
/// The narrowest and widest a report is wrapped to on a terminal.
const MIN_WIDTH: usize = 60;
const MAX_WIDTH: usize = 120;
pub(crate) const INDENT: &str = "  ";
pub(crate) const SUB_INDENT: &str = "      ";

/// What a heading means, which picks its color role.
#[derive(Clone, Copy)]
pub(crate) enum Tone {
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

/// Whether a stream is drawn in color: a terminal, and `NO_COLOR` not set.
pub(crate) fn color_wanted(is_terminal: bool, display: DisplayEnv) -> bool {
    is_terminal && !display.no_color
}

/// The columns a report is wrapped to: the terminal's width less a column of margin, kept
/// between 60 and 120. Without a terminal, or when its size can't be read, 78.
pub(crate) fn report_width(is_terminal: bool, columns: Option<u16>) -> usize {
    match columns {
        Some(columns) if is_terminal => {
            usize::from(columns.saturating_sub(1)).clamp(MIN_WIDTH, MAX_WIDTH)
        }
        _ => WIDTH,
    }
}

/// How a readable report is drawn: whether in color, its symbols and the columns it wraps to.
/// Colors are the Terminal theme's roles, the terminal's own 16 colors, whatever theme is
/// saved. It is a value, not a look at the environment, so tests render both ways and at any
/// width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint {
    colored: bool,
    symbols: Symbols,
    width: usize,
    /// Paths are links to click (OSC 8).
    links: bool,
}

impl Paint {
    /// No escape codes at all, fancy symbols, wrapped at 78 columns.
    pub(crate) const PLAIN: Self = Self {
        colored: false,
        symbols: Symbols::FANCY,
        width: WIDTH,
        links: false,
    };

    /// The same drawing, wrapped to `width` columns.
    pub(crate) fn with_width(self, width: usize) -> Self {
        Self { width, ..self }
    }

    /// The same drawing with `symbols`.
    pub(crate) fn with_symbols(self, symbols: Symbols) -> Self {
        Self { symbols, ..self }
    }

    /// The same drawing, with paths as links to click or not.
    pub(crate) fn with_links(self, links: bool) -> Self {
        Self { links, ..self }
    }

    /// In the terminal's own 16 colors.
    pub(crate) fn terminal() -> Self {
        // The decision to use color was made by the caller. Without this, crossterm would drop
        // every color again whenever `NO_COLOR` is set in the environment it runs in.
        Colored::set_ansi_color_disabled(false);
        Self {
            colored: true,
            ..Self::PLAIN
        }
    }

    /// Colored for a stream that `is_terminal` (and that `display` allows), else plain. Simple
    /// symbols when `config` asks for them or the locale can't show others, as in the TUI.
    pub(crate) fn for_stream(is_terminal: bool, display: DisplayEnv, config: &AppConfig) -> Self {
        // A link is an escape code too, so it goes only where color goes.
        let paint = if color_wanted(is_terminal, display) {
            Self::terminal().with_links(display.hyperlinks)
        } else {
            Self::PLAIN
        };
        let symbols = if config.simple_symbols || !display.utf8 {
            Symbols::SIMPLE
        } else {
            Symbols::FANCY
        };
        paint.with_symbols(symbols)
    }

    pub(crate) fn for_stdout(config: &AppConfig, display: DisplayEnv) -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        let colors = is_terminal && ansi_supported();
        // Asked only of a terminal: without one crossterm falls back to running `tput`.
        let columns = is_terminal
            .then(|| crossterm::terminal::size().ok())
            .flatten()
            .map(|(columns, _)| columns);
        Self::for_stream(colors, display, config).with_width(report_width(is_terminal, columns))
    }

    pub(crate) fn for_stderr(config: &AppConfig, display: DisplayEnv) -> Self {
        let colors = std::io::stderr().is_terminal() && ansi_supported();
        Self::for_stream(colors, display, config)
    }

    /// The columns lines are wrapped to.
    pub(crate) fn width(self) -> usize {
        self.width
    }

    /// The mark of a finished action, in green: `✓`, or `ok` with simple symbols.
    pub(crate) fn done(self) -> String {
        self.added(self.symbols.done)
    }

    /// The mark of a command that failed, in red: `✗`, or `x` with simple symbols.
    pub(crate) fn failed(self) -> String {
        self.removed(self.symbols.failed)
    }

    /// From the installed version to the new one: `→`, or `->` with simple symbols.
    pub(crate) fn arrow(self) -> &'static str {
        self.symbols.arrow
    }

    fn apply(self, text: &str, pick: fn(&ColorRoles) -> Style, bold: bool) -> String {
        let text = &printable(text);
        if !self.colored {
            return text.to_string();
        }
        let style = pick(&ColorRoles::terminal());
        let color = style.fg.unwrap_or(Color::Reset);
        let dim = style.add_modifier.contains(Modifier::DIM);
        if text.is_empty() || (color == Color::Reset && !bold && !dim) {
            return text.to_string();
        }
        let mut out = String::new();
        if bold {
            let _ = SetAttribute(Attribute::Bold).write_ansi(&mut out);
        }
        if dim {
            let _ = SetAttribute(Attribute::Dim).write_ansi(&mut out);
        }
        if color != Color::Reset {
            let _ = SetForegroundColor(color.into_crossterm()).write_ansi(&mut out);
        }
        out.push_str(text);
        let _ = SetAttribute(Attribute::Reset).write_ansi(&mut out);
        out
    }

    pub(crate) fn text(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.text), false)
    }

    pub(crate) fn muted(self, text: &str) -> String {
        self.apply(text, |roles| roles.muted, false)
    }

    /// The role of "Update ready" in the TUI.
    pub(crate) fn update_ready(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.amber), false)
    }

    /// Something to look at, such as the word `warning:`: yellow.
    pub(crate) fn attention(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.amber), false)
    }

    /// What an update leaves behind, such as the old version: red.
    pub(crate) fn removed(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.red), false)
    }

    /// What an update brings, such as the new version, and the done mark: green.
    pub(crate) fn added(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.green), false)
    }

    pub(crate) fn bold(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.text), true)
    }

    /// A file or folder a command wrote or read: bold, and a link to it where the terminal
    /// has links.
    pub(crate) fn path(self, path: &Path) -> String {
        let shown = self.bold(&crate::app::friendly_path(path));
        match (self.colored && self.links)
            .then(|| file_url(path))
            .flatten()
        {
            // OSC 8: the link opens, the text, the link closes.
            Some(url) => format!("\x1b]8;;{url}\x1b\\{shown}\x1b]8;;\x1b\\"),
            None => shown,
        }
    }

    /// A command to run, such as `lemonup update-all`, or a flag: bold cyan. Plain output keeps
    /// the backticks, which are all that marks it there.
    pub(crate) fn command(self, command: &str) -> String {
        if self.colored {
            self.apply(command, |roles| Style::new().fg(roles.blue), true)
        } else {
            format!("`{}`", printable(command))
        }
    }

    /// `text` with each command it names between backticks drawn as a command, and the rest
    /// drawn by `rest`. Only LemonUp's own commands and flags count, so a backtick in a name or
    /// in a source's message stays as it is.
    pub(crate) fn commands(self, text: &str, rest: fn(Self, &str) -> String) -> String {
        if !self.colored {
            return rest(self, text);
        }
        pieces(text)
            .into_iter()
            .map(|(command, piece)| {
                if command {
                    self.command(piece)
                } else {
                    rest(self, piece)
                }
            })
            .collect()
    }

    pub(crate) fn heading(self, tone: Tone, text: &str) -> String {
        let pick: fn(&ColorRoles) -> Style = match tone {
            Tone::Good => |roles| Style::new().fg(roles.green),
            Tone::Warn => |roles| Style::new().fg(roles.amber),
            Tone::Problem => |roles| Style::new().fg(roles.red),
            Tone::Quiet => |roles| Style::new().fg(roles.text),
        };
        self.apply(text, pick, true)
    }
}

/// `path` as a `file://` address, with everything but plain ASCII percent-encoded, so no
/// character of a file name can end the link or reach the terminal as an escape code.
fn file_url(path: &Path) -> Option<String> {
    let absolute = std::path::absolute(path).ok()?;
    reqwest::Url::from_file_path(absolute)
        .ok()
        .map(String::from)
}

/// `text` cut where it names a command between backticks: `(true, the command)` without its
/// backticks, and `(false, text)` for everything around it, other backticks included.
fn pieces(text: &str) -> Vec<(bool, &str)> {
    let mut pieces = Vec::new();
    // Where the text not yet given out starts, and where the search for a backtick goes on.
    let (mut plain_from, mut at) = (0, 0);
    while let Some(found) = text[at..].find('`') {
        let open = at + found;
        let after = open + 1;
        match text[after..].find('`') {
            Some(length) if is_command(&text[after..after + length]) => {
                if open > plain_from {
                    pieces.push((false, &text[plain_from..open]));
                }
                pieces.push((true, &text[after..after + length]));
                at = after + length + 1;
                plain_from = at;
            }
            // Not a command: this backtick is text, and the next one may open a command.
            _ => at = after,
        }
    }
    if plain_from < text.len() {
        pieces.push((false, &text[plain_from..]));
    }
    pieces
}

/// Whether the words between two backticks are a LemonUp command or one of its flags.
fn is_command(span: &str) -> bool {
    span == "lemonup" || span.starts_with("lemonup ") || span.starts_with("--")
}

/// `text` with every control character as a space. Names, versions and messages come from
/// files and from the sources, and an escape code among them must never reach the terminal.
pub(crate) fn printable(text: &str) -> std::borrow::Cow<'_, str> {
    if text.chars().any(char::is_control) {
        text.chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect()
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// The label of an error that ends a run: `Error:`, bold red when stderr has color.
pub(crate) fn error_label(paint: Paint) -> String {
    paint.heading(Tone::Problem, "Error:")
}

/// The columns `text` takes on screen. Wide characters, such as Chinese ones, take two.
/// The width is of the text as it is printed, with control characters as spaces.
pub(crate) fn columns(text: &str) -> usize {
    ratatui::text::Span::raw(printable(text)).width()
}

/// `text` as it is printed, filled with spaces to `width` columns.
pub(crate) fn pad(text: &str, width: usize) -> String {
    let text = printable(text);
    let missing = width.saturating_sub(columns(&text));
    format!("{text}{}", " ".repeat(missing))
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
    /// The version the addon moves on from. With it the row reads `from → version`, the old
    /// version in red and the new one in green.
    pub(crate) from: Option<String>,
    /// A done mark before the name, in green.
    pub(crate) done: bool,
}

/// The lines of a section's addons, sorted by name, with the name and source columns aligned.
pub(crate) fn table(mut entries: Vec<Entry>, paint: Paint) -> Vec<String> {
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    let name_width = entries
        .iter()
        .map(|entry| columns(&entry.name))
        .max()
        .unwrap_or(0);
    let source_width = entries
        .iter()
        .map(|entry| columns(&entry.source))
        .max()
        .unwrap_or(0);
    // A table with a done row keeps a column for the mark, so every name starts in line.
    let mark = paint.symbols.done;
    let mark_width = if entries.iter().any(|entry| entry.done) {
        columns(mark) + 1
    } else {
        0
    };
    let note_indent = format!("{SUB_INDENT}{}", " ".repeat(mark_width));
    let mut lines = Vec::new();
    for entry in entries {
        let mut version = match &entry.from {
            // A reinstall of the same version is not a change, so it gets no arrow or color.
            Some(from) if *from != entry.version => format!(
                "{} {} {}",
                paint.removed(from),
                paint.text(paint.arrow()),
                paint.added(&entry.version)
            ),
            _ => paint.text(&entry.version),
        };
        if let Some(update) = &entry.update {
            version.push_str(&paint.text(", "));
            version.push_str(&paint.update_ready(update));
        }
        let marked = if entry.done {
            format!("{} ", paint.added(mark))
        } else {
            " ".repeat(mark_width)
        };
        // A row without a version ends with its source, so the line has no trailing spaces.
        let rest = if version.is_empty() {
            paint.muted(&entry.source)
        } else {
            format!(
                "{}  {version}",
                paint.muted(&pad(&entry.source, source_width))
            )
        };
        lines.push(format!(
            "{INDENT}{marked}{}  {rest}",
            paint.text(&pad(&entry.name, name_width)),
        ));
        for note in &entry.notes {
            lines.extend(
                wrap_sentence(&note_indent, note, paint.width)
                    .iter()
                    .map(|line| paint.commands(line, Paint::muted)),
            );
        }
    }
    lines
}

/// Adds a section to a report: a blank line when something is above it, the heading in `tone`
/// with its count, then `body`.
pub(crate) fn section(
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
    lines.push(heading_line(paint, tone, title, count));
    lines.extend(body);
}

/// A heading in `tone` with its muted count.
fn heading_line(paint: Paint, tone: Tone, title: &str, count: usize) -> String {
    format!(
        "{} {}",
        paint.heading(tone, title),
        paint.muted(&format!("({count})"))
    )
}

/// Adds headings that only count, on adjacent lines with one blank line above the group.
/// A heading whose count is 0 is left out, and so is the group when every count is.
pub(crate) fn count_headings(
    lines: &mut Vec<String>,
    paint: Paint,
    tone: Tone,
    counts: &[(&str, usize)],
) {
    let shown = counts
        .iter()
        .filter(|(_, count)| *count > 0)
        .collect::<Vec<_>>();
    if shown.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    for (title, count) in shown {
        lines.push(heading_line(paint, tone, title, *count));
    }
}

/// The first line of a report on one game: the game in bold, its AddOns folder muted, then
/// the profile unless it is the default one.
pub(crate) fn context_line(paint: Paint, game: &str, addon_dir: &Path, profile: &str) -> String {
    let mut line = format!(
        "{}  {}",
        paint.bold(game),
        paint.muted(&crate::app::friendly_path(addon_dir))
    );
    if profile != lemonup_core::DEFAULT_PROFILE {
        line.push_str("  ");
        line.push_str(&paint.muted(&format!("(profile {profile})")));
    }
    line
}

/// A version as a report shows it: `unknown` when there is none or it is blank, a full commit
/// hash as its first seven characters, and without the addon's own name in front (`names` are
/// the addon's name and folder; see [`short_version`]).
pub(crate) fn display_version(version: Option<&str>, names: &[&str]) -> String {
    let Some(version) = version.map(str::trim).filter(|version| !version.is_empty()) else {
        return "unknown".to_string();
    };
    let is_commit = version.len() == 40
        && version
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    if is_commit {
        return version[..7].to_string();
    }
    short_version(version, names).to_string()
}

/// A version without the addon's own name in front. Some sources label a release with it
/// (`Pawn 2.14.4`), and the row already starts with the name. `names` are the addon's name
/// and folder; the longest that fits is taken, so `DBM-Core 11.0` loses all of `DBM-Core`.
/// Only a label is removed: the name, a separator, then something that starts like a version.
pub(crate) fn short_version<'a>(version: &'a str, names: &[&str]) -> &'a str {
    let version = version.trim();
    let mut names = names.to_vec();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for name in names {
        let Some(prefix) = version.get(..name.len()) else {
            continue;
        };
        if name.is_empty() || !prefix.eq_ignore_ascii_case(name) {
            continue;
        }
        let rest = &version[name.len()..];
        let trimmed = rest.trim_start_matches([' ', '-', '_', ':']);
        // "Pawn2" has no separator, and "Bagnon_Config-1.0" names something else.
        let starts_like_a_version = trimmed
            .strip_prefix(['v', 'V'])
            .unwrap_or(trimmed)
            .starts_with(|character: char| character.is_ascii_digit());
        if trimmed.len() < rest.len() && starts_like_a_version {
            return trimmed;
        }
    }
    version
}

/// A source as the TUI names it. An addon that is not linked is "Not linked".
pub(crate) fn source_label(source: SourceKind) -> &'static str {
    match source {
        SourceKind::Manual => "Not linked",
        linked => crate::app::dashboard_source_name(linked, true),
    }
}

/// A source's name in JSON and in `key=value` lines. Written out here, never taken from
/// `SourceKind`'s own serde names (`git_hub`, `wow_interface`).
pub(crate) fn source_key(source: SourceKind) -> &'static str {
    match source {
        SourceKind::Wago => "wago",
        SourceKind::WowInterface => "wowinterface",
        SourceKind::Tukui => "tukui",
        SourceKind::GitHub => "github",
        SourceKind::Manual => "manual",
    }
}

/// The kind of a source failure in JSON and in `key=value` lines.
pub(crate) fn error_kind_key(kind: SourceErrorKind) -> &'static str {
    match kind {
        SourceErrorKind::NoConnection => "no_connection",
        SourceErrorKind::KeyRejected => "key_rejected",
        SourceErrorKind::NotFound => "not_found",
        SourceErrorKind::SlowDown => "slow_down",
        SourceErrorKind::WrongRepository => "wrong_repository",
        SourceErrorKind::FoldersInTheWay => "folders_in_the_way",
        SourceErrorKind::FoldersOfLinkedAddon => "folders_of_linked_addon",
        SourceErrorKind::Other => "other",
    }
}

fn serialize_source<S: Serializer>(source: &SourceKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(source_key(*source))
}

fn serialize_error_kind<S: Serializer>(
    kind: &Option<SourceErrorKind>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match kind {
        Some(kind) => serializer.serialize_some(error_kind_key(*kind)),
        None => serializer.serialize_none(),
    }
}

/// Rows of a report keyed by folder, compared without case as Windows and macOS do.
fn rows_by_folder(
    addons: &[lemonup_core::AddonRecord],
) -> std::collections::HashMap<String, &lemonup_core::AddonRecord> {
    addons
        .iter()
        .map(|addon| (addon.folder.to_lowercase(), addon))
        .collect()
}

/// Fills lines of at most `width` columns with `units`, each kept whole, after `indent`.
pub(crate) fn wrap_units(indent: &str, units: &[String], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for unit in units {
        if line.is_empty() {
            line = format!("{indent}{unit}");
        } else if columns(&line) + 1 + columns(unit) > width {
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

/// `text` wrapped at spaces, every line after `indent`.
pub(crate) fn wrap_words(indent: &str, text: &str, width: usize) -> Vec<String> {
    wrap_units(
        indent,
        &text
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        width,
    )
}

/// `text` wrapped like [`wrap_words`], with each command it names kept on one line, so
/// [`Paint::commands`] still finds the command whole on the line it landed on.
pub(crate) fn wrap_sentence(indent: &str, text: &str, width: usize) -> Vec<String> {
    let mut units: Vec<String> = Vec::new();
    // No space stood before what comes next, so it carries on the last unit: the period
    // after a command, or a command right after a bracket.
    let mut joined = false;
    for (command, piece) in pieces(text) {
        if command {
            let unit = format!("`{piece}`");
            match units.last_mut() {
                Some(last) if joined => last.push_str(&unit),
                _ => units.push(unit),
            }
            joined = true;
            continue;
        }
        let spaced = piece.starts_with(char::is_whitespace);
        for (index, word) in piece.split_whitespace().enumerate() {
            match units.last_mut() {
                Some(last) if index == 0 && joined && !spaced => last.push_str(word),
                _ => units.push(word.to_string()),
            }
        }
        joined = !piece.ends_with(char::is_whitespace);
    }
    wrap_units(indent, &units, width)
}

/// A path as JSON: a string, or `null`. A path that isn't valid Unicode is written with
/// replacement characters rather than failing the whole document.
fn lossy_path<S: Serializer>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => serializer.serialize_some(&path.to_string_lossy()),
        None => serializer.serialize_none(),
    }
}

/// A path as a JSON string, written with replacement characters where it isn't valid Unicode.
fn lossy_path_buf<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

/// What a command prints: its readable report, or its fields in the JSON document.
pub(crate) trait Report: Serialize {
    /// The `command` value of the JSON envelope, for example "check".
    fn command(&self) -> &'static str;
    /// The readable report, one string per line.
    fn human(&self, paint: Paint) -> Vec<String>;
}

/// Readable report, or one JSON object (`--json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputMode {
    Human,
    Json,
}

impl OutputMode {
    pub(crate) fn from_json_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

/// The JSON document: the envelope keys, the report's own fields, then the warnings.
#[derive(Serialize)]
struct Envelope<'a, R: Serialize> {
    schema: u32,
    command: &'static str,
    profile: &'a str,
    addon_dir: Option<String>,
    #[serde(flatten)]
    report: &'a R,
    warnings: &'a [String],
}

/// The keys every JSON document starts and ends with. A report must not use them for its own
/// fields: the document would carry the key twice.
const ENVELOPE_KEYS: [&str; 5] = ["schema", "command", "profile", "addon_dir", "warnings"];

/// Whether `report` serializes to an object that leaves the envelope's keys alone. A report
/// that fails to serialize passes: printing it reports the failure.
fn keeps_envelope_keys<R: Serialize>(report: &R) -> bool {
    match serde_json::to_value(report) {
        Ok(serde_json::Value::Object(fields)) => {
            !ENVELOPE_KEYS.iter().any(|key| fields.contains_key(*key))
        }
        Ok(_) => false,
        Err(_) => true,
    }
}

/// Where a command prints: stdout for the result, stderr for everything else. Built once a run
/// knows its profile; the writers are given, so tests read what each stream got.
pub(crate) struct Output<O: Write = std::io::Stdout, E: Write = std::io::Stderr> {
    mode: OutputMode,
    stdout_paint: Paint,
    stderr_paint: Paint,
    profile: String,
    warnings: Vec<String>,
    stdout: O,
    stderr: E,
    /// A write to stdout or stderr failed, other than to a reader that went away.
    write_failed: bool,
}

/// Why a command failed, as [`Output::fail`] draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason<'a> {
    /// LemonUp's own words. They can name a command to run, which is drawn as one.
    Ours(&'a str),
    /// The words of a source, an addon or the system: drawn plain, never as a command.
    Theirs(&'a str),
}

impl Output {
    /// Printing to this process's stdout and stderr, each colored only when it is a terminal.
    pub(crate) fn stdio(
        mode: OutputMode,
        config: &AppConfig,
        display: DisplayEnv,
        profile: &str,
    ) -> Self {
        Self::new(
            mode,
            Paint::for_stdout(config, display),
            Paint::for_stderr(config, display),
            profile,
            std::io::stdout(),
            std::io::stderr(),
        )
    }
}

impl<O: Write, E: Write> Output<O, E> {
    pub(crate) fn new(
        mode: OutputMode,
        stdout_paint: Paint,
        stderr_paint: Paint,
        profile: &str,
        stdout: O,
        stderr: E,
    ) -> Self {
        Self {
            mode,
            stdout_paint,
            stderr_paint,
            profile: profile.to_string(),
            warnings: Vec::new(),
            stdout,
            stderr,
            write_failed: false,
        }
    }

    /// Whether a line could not be written. A command whose own work is done has not failed
    /// because its report could not be printed.
    pub(crate) fn write_failed(&self) -> bool {
        self.write_failed
    }

    /// The result of a write: a reader that went away (`| head`) is not an error of the
    /// command, since there is nobody left to tell. Any other failure is remembered.
    fn written(&mut self, written: std::io::Result<()>) -> std::io::Result<()> {
        match written {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => {
                self.write_failed = true;
                Err(error)
            }
            Ok(()) => Ok(()),
        }
    }

    /// Prints `report`: its lines, or the JSON document with the warnings given so far.
    /// `addon_dir` is the AddOns folder the command acted on, if it has one.
    pub(crate) fn print<R: Report>(
        &mut self,
        report: &R,
        addon_dir: Option<&Path>,
    ) -> std::io::Result<()> {
        let written = match self.mode {
            OutputMode::Human => report
                .human(self.stdout_paint)
                .iter()
                .try_for_each(|line| writeln!(self.stdout, "{line}")),
            OutputMode::Json => {
                let warnings = std::mem::take(&mut self.warnings);
                let envelope = Envelope {
                    schema: 1,
                    command: report.command(),
                    profile: &self.profile,
                    addon_dir: addon_dir.map(|dir| dir.to_string_lossy().into_owned()),
                    report,
                    warnings: &warnings,
                };
                debug_assert!(
                    keeps_envelope_keys(report),
                    "a report field has the name of an envelope key"
                );
                // Written whole or not at all: a document that fails to serialize prints nothing.
                let mut line = serde_json::to_vec(&envelope)?;
                line.push(b'\n');
                self.stdout.write_all(&line)
            }
        }
        .and_then(|()| self.stdout.flush());
        self.written(written)
    }

    /// A warning: on stderr in both modes, and also in the `warnings` of the JSON document.
    /// A run that fails prints no document, so stderr is the one place a warning can't be lost.
    pub(crate) fn warn(&mut self, text: &str) -> std::io::Result<()> {
        if self.mode == OutputMode::Json {
            self.warnings.push(text.to_string());
        }
        let written = writeln!(
            self.stderr,
            "{} {}",
            self.stderr_paint.attention("warning:"),
            self.stderr_paint.commands(text, Paint::text)
        );
        self.written(written)
    }

    /// How a command that failed ends, on stderr in both modes: the red mark and `what` failed
    /// on a line of their own, then `reason` under it, then `hint` when there is something to
    /// run next. Nothing goes to stdout after it.
    ///
    /// The reason is one line however it was written. It is not wrapped, since it can hold a
    /// path. Words that are not LemonUp's own are never drawn as a command; the hint, which
    /// LemonUp wrote, always has its commands drawn.
    pub(crate) fn fail(
        &mut self,
        what: &str,
        reason: Reason,
        hint: Option<&str>,
    ) -> std::io::Result<()> {
        let paint = self.stderr_paint;
        let reason = match reason {
            Reason::Ours(words) => paint.commands(words, Paint::text),
            Reason::Theirs(words) => paint.text(words),
        };
        let mut lines = vec![
            format!("{} {}", paint.failed(), paint.heading(Tone::Problem, what)),
            format!("{INDENT}{reason}"),
        ];
        if let Some(hint) = hint {
            lines.push(format!("{INDENT}{}", paint.commands(hint, Paint::text)));
        }
        let written = lines
            .iter()
            .try_for_each(|line| writeln!(self.stderr, "{line}"));
        self.written(written)
    }

    /// A notice on stderr, in both modes. The JSON document leaves it out.
    pub(crate) fn note(&mut self, text: &str) -> std::io::Result<()> {
        let written = writeln!(
            self.stderr,
            "{}",
            self.stderr_paint.commands(text, Paint::text)
        );
        self.written(written)
    }

    #[cfg(test)]
    pub(crate) fn into_writers(self) -> (O, E) {
        (self.stdout, self.stderr)
    }
}
