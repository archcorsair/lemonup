use lemonup_core::{AppConfig, ThemeMode};
use serde::Serialize;

use super::*;
use crate::theme::{DisplayEnv, Symbols};

fn display_with(pairs: &'static [(&'static str, &'static str)]) -> DisplayEnv {
    DisplayEnv::from_vars(
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        },
        false,
    )
}

fn config_in(theme: ThemeMode) -> AppConfig {
    let mut config = AppConfig::new_unconfigured();
    config.theme = theme;
    config
}

#[test]
fn color_is_for_a_terminal_without_no_color() {
    let utf8 = display_with(&[("LANG", "en_US.UTF-8")]);
    let no_color = display_with(&[("LANG", "en_US.UTF-8"), ("NO_COLOR", "1")]);
    assert!(color_wanted(true, utf8));
    assert!(color_wanted(true, DisplayEnv::FULL));
    assert!(!color_wanted(true, no_color));
    assert!(!color_wanted(false, utf8));
    assert!(!color_wanted(false, DisplayEnv::FULL));

    let config = AppConfig::new_unconfigured();
    assert_eq!(Paint::for_stream(true, no_color, &config), Paint::PLAIN);
    assert_eq!(Paint::for_stream(false, utf8, &config), Paint::PLAIN);
    assert_ne!(Paint::terminal(), Paint::PLAIN);
}

#[test]
fn every_saved_theme_paints_in_the_terminal_colors() {
    let truecolor = display_with(&[("LANG", "en_US.UTF-8"), ("COLORTERM", "truecolor")]);
    for display in [DisplayEnv::FULL, truecolor] {
        for theme in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Terminal] {
            assert_eq!(
                Paint::for_stream(true, display, &config_in(theme)),
                Paint::terminal(),
                "{theme:?}"
            );
        }
    }
    // Normal text has no color of its own, so it stays as the terminal draws it.
    let paint = Paint::for_stream(true, DisplayEnv::FULL, &config_in(ThemeMode::Dark));
    assert_eq!(paint.text("Retail"), "Retail");
    assert_eq!(
        paint.heading(Tone::Good, "Ready"),
        "\x1b[1m\x1b[38;5;2mReady\x1b[0m"
    );
}

#[test]
fn no_color_and_pipes_paint_plain_in_every_theme() {
    let no_color = display_with(&[
        ("LANG", "en_US.UTF-8"),
        ("NO_COLOR", "1"),
        ("COLORTERM", "truecolor"),
    ]);
    for theme in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Terminal] {
        let config = config_in(theme);
        assert_eq!(Paint::for_stream(true, no_color, &config), Paint::PLAIN);
        assert_eq!(
            Paint::for_stream(false, DisplayEnv::FULL, &config),
            Paint::PLAIN
        );
        assert_eq!(
            Paint::for_stream(true, no_color, &config).heading(Tone::Problem, "Check failed"),
            "Check failed"
        );
    }
}

#[test]
fn simple_symbols_follow_the_setting_and_the_locale() {
    let mut simple = AppConfig::new_unconfigured();
    simple.simple_symbols = true;
    let fancy = AppConfig::new_unconfigured();
    let not_utf8 = display_with(&[("LANG", "C")]);

    assert_eq!(
        Paint::for_stream(true, DisplayEnv::FULL, &fancy).arrow(),
        "→"
    );
    assert_eq!(
        Paint::for_stream(true, DisplayEnv::FULL, &simple).arrow(),
        "->"
    );
    assert_eq!(Paint::for_stream(true, not_utf8, &fancy).arrow(), "->");
    // A pipe gets the same symbols as a terminal.
    assert_eq!(
        Paint::for_stream(false, DisplayEnv::FULL, &simple),
        Paint::PLAIN.with_symbols(Symbols::SIMPLE)
    );
}

#[test]
fn the_width_follows_the_terminal_within_limits() {
    // A column of margin, kept between 60 and 120.
    assert_eq!(report_width(true, Some(100)), 99);
    assert_eq!(report_width(true, Some(40)), 60);
    assert_eq!(report_width(true, Some(200)), 120);
    // Not a terminal, or a size that can't be read: 78.
    assert_eq!(report_width(false, Some(200)), 78);
    assert_eq!(report_width(true, None), 78);
    assert_eq!(report_width(false, None), 78);
}

#[test]
fn the_error_label_is_bold_red_only_with_color() {
    assert_eq!(error_label(Paint::PLAIN), "Error:");
    assert_eq!(
        error_label(Paint::terminal()),
        "\x1b[1m\x1b[38;5;1mError:\x1b[0m"
    );
}

/// A report whose readable lines are colored, to show JSON never carries them.
#[derive(Serialize)]
struct Sample {
    names: Vec<String>,
}

impl Report for Sample {
    const COMMAND: &'static str = "sample";

    fn human(&self, paint: Paint) -> Vec<String> {
        self.names
            .iter()
            .map(|name| paint.heading(Tone::Warn, name))
            .collect()
    }
}

fn sample() -> Sample {
    Sample {
        names: vec!["Questie".to_string(), "Pawn".to_string()],
    }
}

fn output(mode: OutputMode, paint: Paint) -> Output<Vec<u8>, Vec<u8>> {
    Output::new(mode, paint, paint, "forever", Vec::new(), Vec::new())
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("utf-8")
}

#[test]
fn json_is_one_line_with_the_warnings_given_before_it() {
    let mut out = output(OutputMode::Json, Paint::PLAIN);
    out.warn("WoWInterface didn't answer").expect("warn");
    out.note("Checking 2 addons").expect("note");
    out.warn("Wago.io key is missing").expect("warn");
    out.print(&sample(), Some(Path::new("/games/AddOns")))
        .expect("print");
    let (stdout, stderr) = out.into_writers();
    let stdout = text(stdout);
    assert!(text(stderr).is_empty());
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    assert!(stdout.ends_with('\n'));
    assert_eq!(
        stdout,
        "{\"schema\":1,\"command\":\"sample\",\"profile\":\"forever\",\
         \"addon_dir\":\"/games/AddOns\",\"names\":[\"Questie\",\"Pawn\"],\
         \"warnings\":[\"WoWInterface didn't answer\",\"Wago.io key is missing\"]}\n"
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(parsed["warnings"].as_array().map(Vec::len), Some(2));
}

#[test]
fn readable_output_puts_warnings_and_notes_on_stderr() {
    let mut out = output(OutputMode::Human, Paint::PLAIN);
    out.warn("WoWInterface didn't answer").expect("warn");
    out.note("Checking 2 addons").expect("note");
    out.print(&sample(), None).expect("print");
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "Questie\nPawn\n");
    assert_eq!(
        text(stderr),
        "warning: WoWInterface didn't answer\nChecking 2 addons\n"
    );

    let mut colored = output(OutputMode::Human, Paint::terminal());
    colored.warn("WoWInterface didn't answer").expect("warn");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr),
        "\x1b[38;5;3mwarning:\x1b[0m WoWInterface didn't answer\n"
    );
}

#[test]
fn json_never_carries_escape_codes() {
    let mut out = output(OutputMode::Json, Paint::terminal());
    out.warn("Wago.io key is missing").expect("warn");
    out.print(&sample(), None).expect("print");
    let (stdout, stderr) = out.into_writers();
    let stdout = text(stdout);
    assert!(!stdout.contains('\x1b'), "{stdout:?}");
    assert!(stdout.contains("\"addon_dir\":null"), "{stdout:?}");
    assert!(stderr.is_empty());
}

#[test]
fn wide_characters_count_as_two_columns() {
    assert_eq!(columns("Pawn"), 4);
    assert_eq!(columns("背包整合"), 8);
    assert_eq!(pad("背包", 6), "背包  ");
    let rows = table(
        vec![
            Entry {
                name: "背包整合".to_string(),
                source: "Wago.io".to_string(),
                version: "1.0".to_string(),
                update: None,
                notes: Vec::new(),
                from: None,
                done: false,
            },
            Entry {
                name: "Pawn".to_string(),
                source: "GitHub".to_string(),
                version: "2.0".to_string(),
                update: None,
                notes: Vec::new(),
                from: None,
                done: false,
            },
        ],
        Paint::PLAIN,
    );
    assert_eq!(
        rows,
        ["  Pawn      GitHub   2.0", "  背包整合  Wago.io  1.0"]
    );
}

/// A report that uses an envelope key for a field of its own.
#[derive(Serialize)]
struct TakesProfile {
    profile: String,
}

#[test]
fn a_report_field_named_like_an_envelope_key_is_caught() {
    assert!(keeps_envelope_keys(&sample()));
    assert!(!keeps_envelope_keys(&TakesProfile {
        profile: "mine".to_string()
    }));
    // Only an object can be flattened into the document.
    assert!(!keeps_envelope_keys(&vec!["Questie"]));
}

/// A report that fails partway through its fields.
struct Broken;

impl Serialize for Broken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("first", "written")?;
        Err(serde::ser::Error::custom("the second field failed"))
    }
}

impl Report for Broken {
    const COMMAND: &'static str = "broken";

    fn human(&self, _: Paint) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_document_that_fails_to_serialize_prints_nothing() {
    let mut out = output(OutputMode::Json, Paint::PLAIN);
    assert!(out.print(&Broken, None).is_err());
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "");
    assert_eq!(text(stderr), "");
}

#[test]
fn control_characters_never_reach_the_terminal() {
    let hostile = "Pawn\x1b[31m\x07\r\nv2\t\u{9b}";
    for paint in [Paint::PLAIN, Paint::terminal()] {
        for drawn in [
            paint.text(hostile),
            paint.muted(hostile),
            paint.bold(hostile),
            paint.heading(Tone::Problem, hostile),
        ] {
            // What is left of the name has no control character; the only escape codes are
            // the ones `Paint` wrote around it.
            assert!(drawn.contains("Pawn [31m   v2  "), "{drawn:?}");
            assert!(!drawn.contains("Pawn\x1b"), "{drawn:?}");
            assert!(
                !drawn.contains(['\x07', '\r', '\n', '\t', '\u{9b}']),
                "{drawn:?}"
            );
        }
    }
    assert_eq!(Paint::PLAIN.text("Details!"), "Details!");
}

/// A stream whose reader went away.
struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

/// A stream that fails for another reason.
struct FullDisk;

impl Write for FullDisk {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::StorageFull.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_pipe_ends_printing_quietly_and_other_failures_are_errors() {
    for mode in [OutputMode::Human, OutputMode::Json] {
        let mut closed = Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            ClosedPipe,
            Vec::new(),
        );
        assert!(closed.print(&sample(), None).is_ok(), "{mode:?}");
        let mut full = Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            FullDisk,
            Vec::new(),
        );
        assert!(full.print(&sample(), None).is_err(), "{mode:?}");
    }
}

#[test]
fn a_version_loses_the_addon_name_only_as_a_label() {
    assert_eq!(short_version("Pawn 2.14.4", &["Pawn"]), "2.14.4");
    assert_eq!(short_version("pawn-2.14.4", &["Pawn"]), "2.14.4");
    assert_eq!(
        short_version("ElvUI_WindTools-4.16", &["Wind Tools", "ElvUI_WindTools"]),
        "4.16"
    );
    assert_eq!(short_version("Pawn v2.14.4", &["Pawn"]), "v2.14.4");
    // The longest name that fits is the label, whichever comes first.
    assert_eq!(short_version("DBM-Core 11.0", &["DBM", "DBM-Core"]), "11.0");
    assert_eq!(short_version("DBM-Core 11.0", &["DBM-Core", "DBM"]), "11.0");
    // What follows the name has to start like a version.
    assert_eq!(
        short_version("Bagnon_Config-1.0", &["Bagnon"]),
        "Bagnon_Config-1.0"
    );
    assert_eq!(short_version("Pawn beta", &["Pawn"]), "Pawn beta");
    // Not a label: no separator, nothing after it, or another name.
    assert_eq!(short_version("Pawn2", &["Pawn"]), "Pawn2");
    assert_eq!(short_version("Pawn", &["Pawn"]), "Pawn");
    assert_eq!(short_version("16.02", &["ElvUI"]), "16.02");
    assert_eq!(
        short_version("v1.56", &["ElvUI Floating Combat Text"]),
        "v1.56"
    );
    assert_eq!(short_version("背包 1.0", &["背"]), "背包 1.0");
    assert_eq!(short_version("unknown", &["Pawn", ""]), "unknown");
}
