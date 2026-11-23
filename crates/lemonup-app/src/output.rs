//! How CLI commands print. A command builds a report; [`Output`] prints it as readable lines
//! on stdout, or with `--json` as one JSON object on one line. Warnings and notes go to stderr.
//! Colors are always the terminal's own 16 colors, whatever theme is saved: the CLI draws no
//! background, so a theme's text color could be close to invisible on the terminal's own.

mod profiles;
#[cfg(test)]
mod tests;

pub(crate) use profiles::ProfilesReport;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crossterm::Command;
use crossterm::style::{Attribute, Colored, SetAttribute, SetForegroundColor};
use lemonup_core::AppConfig;
use ratatui::backend::IntoCrossterm;
use ratatui::style::{Color, Modifier, Style};
use serde::{Serialize, Serializer};

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
}

impl Paint {
    /// No escape codes at all, fancy symbols, wrapped at 78 columns.
    pub(crate) const PLAIN: Self = Self {
        colored: false,
        symbols: Symbols::FANCY,
        width: WIDTH,
    };

    /// The same drawing, wrapped to `width` columns.
    pub(crate) fn with_width(self, width: usize) -> Self {
        Self { width, ..self }
    }

    /// The same drawing with `symbols`.
    pub(crate) fn with_symbols(self, symbols: Symbols) -> Self {
        Self { symbols, ..self }
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
        let paint = if color_wanted(is_terminal, display) {
            Self::terminal()
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

    /// From the installed version to the new one: `→`, or `->` with simple symbols.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the reports of later commands use it")
    )]
    pub(crate) fn arrow(self) -> &'static str {
        self.symbols.arrow
    }

    fn apply(self, text: &str, pick: fn(&ColorRoles) -> Style, bold: bool) -> String {
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

    pub(crate) fn bold(self, text: &str) -> String {
        self.apply(text, |roles| Style::new().fg(roles.text), true)
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

/// The label of an error that ends a run: `Error:`, bold red when stderr has color.
pub(crate) fn error_label(paint: Paint) -> String {
    paint.heading(Tone::Problem, "Error:")
}

/// The columns `text` takes on screen. Wide characters, such as Chinese ones, take two.
pub(crate) fn columns(text: &str) -> usize {
    ratatui::text::Span::raw(text).width()
}

pub(crate) fn pad(text: &str, width: usize) -> String {
    let missing = width.saturating_sub(columns(text));
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
                wrap_words(SUB_INDENT, note, paint.width)
                    .iter()
                    .map(|line| paint.muted(line)),
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
    lines.push(format!(
        "{} {}",
        paint.heading(tone, title),
        paint.muted(&format!("({count})"))
    ));
    lines.extend(body);
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

/// A path as JSON: a string, or `null`. A path that isn't valid Unicode is written with
/// replacement characters rather than failing the whole document.
fn lossy_path<S: Serializer>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => serializer.serialize_some(&path.to_string_lossy()),
        None => serializer.serialize_none(),
    }
}

/// What a command prints: its readable report, or its fields in the JSON document.
pub(crate) trait Report: Serialize {
    /// The `command` value of the JSON envelope, for example "check".
    const COMMAND: &'static str;
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
        }
    }

    /// Prints `report`: its lines, or the JSON document with the warnings given so far.
    /// `addon_dir` is the AddOns folder the command acted on, if it has one.
    pub(crate) fn print<R: Report>(
        &mut self,
        report: &R,
        addon_dir: Option<&Path>,
    ) -> std::io::Result<()> {
        match self.mode {
            OutputMode::Human => {
                for line in report.human(self.stdout_paint) {
                    writeln!(self.stdout, "{line}")?;
                }
            }
            OutputMode::Json => {
                let warnings = std::mem::take(&mut self.warnings);
                let envelope = Envelope {
                    schema: 1,
                    command: R::COMMAND,
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
                self.stdout.write_all(&line)?;
            }
        }
        self.stdout.flush()
    }

    /// A warning: on stderr, or in the `warnings` of the JSON document.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "later commands warn through it")
    )]
    pub(crate) fn warn(&mut self, text: &str) -> std::io::Result<()> {
        match self.mode {
            OutputMode::Human => writeln!(
                self.stderr,
                "{} {text}",
                self.stderr_paint.attention("warning:")
            ),
            OutputMode::Json => {
                self.warnings.push(text.to_string());
                Ok(())
            }
        }
    }

    /// A notice on stderr. The JSON document leaves it out.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "later commands note through it")
    )]
    pub(crate) fn note(&mut self, text: &str) -> std::io::Result<()> {
        match self.mode {
            OutputMode::Human => writeln!(self.stderr, "{text}"),
            OutputMode::Json => Ok(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn into_writers(self) -> (O, E) {
        (self.stdout, self.stderr)
    }
}
