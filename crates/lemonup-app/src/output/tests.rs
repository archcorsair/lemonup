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
    for theme in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Terminal] {
        assert_eq!(
            Paint::for_stream(true, truecolor, &config_in(theme)),
            Paint::terminal(),
            "{theme:?}"
        );
        // A terminal known to have links gets them as well.
        assert_eq!(
            Paint::for_stream(true, DisplayEnv::FULL, &config_in(theme)),
            Paint::terminal().with_links(true),
            "{theme:?}"
        );
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
    fn command(&self) -> &'static str {
        "sample"
    }

    fn human(&self, paint: Paint) -> Vec<String> {
        self.names
            .iter()
            .map(|name| paint.heading(Tone::Warn, name))
            .collect()
    }
}

#[cfg(unix)]
#[test]
fn a_path_is_bold_and_a_link_only_where_the_terminal_has_links() {
    let path = Path::new("/games/My Addons/caf\u{e9}\x1b]8;;evil\x07.json");
    // Without color: the path as it is, less its control characters.
    assert_eq!(
        Paint::PLAIN.path(path),
        "/games/My Addons/caf\u{e9} ]8;;evil .json"
    );
    assert_eq!(
        Paint::PLAIN.with_links(true).path(path),
        Paint::PLAIN.path(path)
    );
    let bold = "\x1b[1m/games/My Addons/caf\u{e9} ]8;;evil .json\x1b[0m";
    assert_eq!(Paint::terminal().path(path), bold);
    // With links: the address is plain ASCII, so nothing in a name can end the link.
    assert_eq!(
        Paint::terminal().with_links(true).path(path),
        format!(
            "\x1b]8;;file:///games/My%20Addons/caf%C3%A9%1B]8;;evil%07.json\x1b\\{bold}\x1b]8;;\x1b\\"
        )
    );
    // A path given relative to the working folder still links to the whole path.
    let relative = Paint::terminal()
        .with_links(true)
        .path(Path::new("list.json"));
    assert!(relative.starts_with("\x1b]8;;file:///"), "{relative:?}");
    assert!(
        relative.ends_with("/list.json\x1b\\\x1b[1mlist.json\x1b[0m\x1b]8;;\x1b\\"),
        "{relative:?}"
    );
}

#[test]
fn links_go_only_to_a_colored_stream_of_a_terminal_that_has_them() {
    let config = config_in(ThemeMode::Terminal);
    let linked = |paint: Paint| paint.path(Path::new("/tmp/list.json")).contains("\x1b]8;;");
    assert!(linked(Paint::for_stream(true, DisplayEnv::FULL, &config)));
    // A pipe, `NO_COLOR`, and a terminal that isn't known to have links.
    assert!(!linked(Paint::for_stream(false, DisplayEnv::FULL, &config)));
    let no_color = DisplayEnv {
        no_color: true,
        ..DisplayEnv::FULL
    };
    assert!(!linked(Paint::for_stream(true, no_color, &config)));
    let unknown = display_with(&[("LANG", "en_US.UTF-8"), ("TERM", "xterm-256color")]);
    assert!(!linked(Paint::for_stream(true, unknown, &config)));
}

/// A command as the terminal gets it: bold cyan.
fn command(text: &str) -> String {
    format!("\x1b[1m\x1b[38;5;6m{text}\x1b[0m")
}

#[test]
fn a_command_is_bold_cyan_with_color_and_keeps_its_backticks_without() {
    let sentence =
        "1 update ready. Run `lemonup update-all` with the same `--addon-dir` to update.";
    assert_eq!(Paint::PLAIN.commands(sentence, Paint::text), sentence);
    assert_eq!(
        Paint::terminal().commands(sentence, Paint::text),
        format!(
            "1 update ready. Run {} with the same {} to update.",
            command("lemonup update-all"),
            command("--addon-dir")
        )
    );
    assert_eq!(Paint::PLAIN.command("lemonup"), "`lemonup`");
    assert_eq!(Paint::terminal().command("lemonup"), command("lemonup"));
}

#[test]
fn the_text_around_a_command_keeps_its_own_drawing() {
    assert_eq!(
        Paint::terminal().commands(
            "Nothing was changed. `lemonup find --link` links it.",
            Paint::bold
        ),
        format!(
            "\x1b[1mNothing was changed. \x1b[0m{}\x1b[1m links it.\x1b[0m",
            command("lemonup find --link")
        )
    );
    // Text of more than one byte a character, on both sides of a command and inside it.
    assert_eq!(
        Paint::terminal().commands(
            "caf\u{e9} \u{2713} `lemonup --addon-dir '/j\u{fc}rgen' sync` \u{4e2d}\u{6587}",
            Paint::text
        ),
        format!(
            "caf\u{e9} \u{2713} {} \u{4e2d}\u{6587}",
            command("lemonup --addon-dir '/j\u{fc}rgen' sync")
        )
    );
}

#[test]
fn only_lemonup_commands_and_flags_are_drawn_as_commands() {
    let paint = Paint::terminal();
    // A name or a source's message can hold backticks of its own.
    for text in [
        "Addon `Pawn` not found",
        "a stray ` backtick",
        "an empty `` pair",
        "`lemonupdate` is not a command",
    ] {
        assert_eq!(paint.commands(text, Paint::text), text, "{text:?}");
    }
    // A backtick that is text doesn't swallow the command after it.
    assert_eq!(
        paint.commands("it's ` odd, run `lemonup check`", Paint::text),
        format!("it's ` odd, run {}", command("lemonup check"))
    );
    // Control characters never reach the terminal, and a span that holds one is no command.
    assert_eq!(
        paint.commands("bad\x1b[31m `lemonup\x07 x` `--yes\x1b`", Paint::text),
        "bad [31m `lemonup  x` `--yes `"
    );
    assert_eq!(
        paint.commands("`lemonup update\n--force` and `lemonup x\ty`", Paint::text),
        "`lemonup update --force` and `lemonup x y`"
    );
    // A flag is one word: words after it are not part of a command.
    assert_eq!(
        paint.commands("`--replace everything here`", Paint::text),
        "`--replace everything here`"
    );
    assert_eq!(
        Paint::PLAIN.commands("bad\x07 `lemonup check`", Paint::text),
        "bad  `lemonup check`"
    );
}

#[test]
fn warnings_notes_and_table_hints_draw_their_commands() {
    let mut colored = output(OutputMode::Human, Paint::terminal());
    colored.warn("Run `lemonup sync` first").expect("warn");
    colored.note("Then `lemonup check`").expect("note");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr),
        format!(
            "\x1b[38;5;3mwarning:\x1b[0m Run {} first\nThen {}\n",
            command("lemonup sync"),
            command("lemonup check")
        )
    );

    // The JSON document keeps the words as they were written.
    let mut json = output(OutputMode::Json, Paint::terminal());
    json.warn("Run `lemonup sync` first").expect("warn");
    json.print(&sample(), None).expect("print");
    let (stdout, _) = json.into_writers();
    assert!(
        text(stdout).contains(r#""warnings":["Run `lemonup sync` first"]"#),
        "json keeps the backticks"
    );

    let entry = Entry {
        name: "Pawn".to_string(),
        source: "Wago.io".to_string(),
        version: "1.0".to_string(),
        update: None,
        // A note names folders and holds a source's words: one that reads like a command is
        // not drawn as one, and an escape code in it never gets through.
        notes: vec!["Would replace `lemonup sync`\x1b[31m and `--yes`.".to_string()],
        hint: Some("Run again with `--force` to install it.".to_string()),
        from: None,
        done: false,
    };
    let lines = table(vec![entry.clone()], Paint::terminal());
    assert_eq!(
        lines[1..],
        [
            "\x1b[2m      Would replace `lemonup sync` [31m and `--yes`.\x1b[0m".to_string(),
            format!(
                "\x1b[2m      Run again with \x1b[0m{}\x1b[2m to install it.\x1b[0m",
                command("--force")
            ),
        ]
    );
    assert_eq!(
        table(vec![entry.clone()], Paint::PLAIN)[1..],
        [
            "      Would replace `lemonup sync` [31m and `--yes`.",
            "      Run again with `--force` to install it."
        ]
    );
    // A command of several words stays on one line when the hint wraps.
    let long = Entry {
        notes: Vec::new(),
        hint: Some("Run `lemonup update-all --dry-run --replace` first, then again.".to_string()),
        ..entry
    };
    assert_eq!(
        table(vec![long], Paint::PLAIN.with_width(48))[1..],
        [
            "      Run",
            "      `lemonup update-all --dry-run --replace`",
            "      first, then again.",
        ]
    );
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
    // Warnings and notes are on stderr as well: a run that fails prints no document.
    assert_eq!(
        text(stderr),
        "warning: WoWInterface didn't answer\nChecking 2 addons\nwarning: Wago.io key is missing\n"
    );
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
fn a_failure_is_the_failed_mark_and_what_failed_then_the_reason_under_it() {
    for mode in [OutputMode::Human, OutputMode::Json] {
        let mut plain = output(mode, Paint::PLAIN);
        plain
            .fail("Sync failed", "the folder is gone", None)
            .expect("fail");
        let (stdout, stderr) = plain.into_writers();
        assert_eq!(text(stdout), "");
        assert_eq!(text(stderr), "✗ Sync failed\n  the folder is gone\n");
    }
    // What failed is bold red; the reason is plain.
    let mut colored = output(OutputMode::Human, Paint::terminal());
    colored
        .fail("Sync failed", "the folder is gone", None)
        .expect("fail");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr),
        "\x1b[38;5;1m✗\x1b[0m \x1b[1m\x1b[38;5;1mSync failed\x1b[0m\n  the folder is gone\n"
    );
    let mut simple = output(
        OutputMode::Human,
        Paint::PLAIN.with_symbols(Symbols::SIMPLE),
    );
    simple
        .fail("Sync failed", "the folder is gone", None)
        .expect("fail");
    let (_, stderr) = simple.into_writers();
    assert_eq!(text(stderr), "x Sync failed\n  the folder is gone\n");
}

#[test]
fn the_hint_is_a_line_of_its_own_and_only_its_commands_are_drawn() {
    let reason = "Kui Nameplates is already installed.";
    let hint = "Run `lemonup update Kui_Nameplates --force` to install it again.";
    let mut plain = output(OutputMode::Human, Paint::PLAIN.with_width(40));
    plain
        .fail("Install failed", reason, Some(hint))
        .expect("fail");
    let (_, stderr) = plain.into_writers();
    assert_eq!(
        text(stderr),
        "✗ Install failed\n  Kui Nameplates is already installed.\n  \
         Run `lemonup update Kui_Nameplates --force` to install it again.\n"
    );

    let mut colored = output(OutputMode::Human, Paint::terminal().with_width(40));
    colored
        .fail("Install failed", reason, Some(hint))
        .expect("fail");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr).lines().skip(1).collect::<Vec<_>>(),
        [
            "  Kui Nameplates is already installed.".to_string(),
            format!(
                "  Run {} to install it again.",
                command("lemonup update Kui_Nameplates --force")
            ),
        ]
    );
}

#[test]
fn a_reason_is_one_line_whatever_it_holds_and_names_no_command() {
    // A reason can hold a source's or an addon's own words: a line break in them never
    // starts a line, and a command they name is not drawn as one of LemonUp's.
    let forged = "failed\n  Run `lemonup update-all --force --replace`\r\nnow\x1b[1m";
    for paint in [Paint::PLAIN, Paint::terminal()] {
        let mut out = output(OutputMode::Human, paint);
        out.fail("Install failed", forged, None).expect("fail");
        let (_, stderr) = out.into_writers();
        let printed = text(stderr);
        assert_eq!(printed.lines().count(), 2, "{printed:?}");
        assert!(
            printed.ends_with("  failed   Run `lemonup update-all --force --replace`  now [1m\n"),
            "{printed:?}"
        );
    }
    // A path with a space in it stays in one piece: a reason is not wrapped.
    let mut path = output(OutputMode::Human, Paint::PLAIN.with_width(40));
    path.fail(
        "Sync failed",
        "path does not exist: /games/World of Warcraft/_retail_/Interface/AddOns",
        None,
    )
    .expect("fail");
    let (_, stderr) = path.into_writers();
    assert_eq!(
        text(stderr),
        "✗ Sync failed\n  path does not exist: /games/World of Warcraft/_retail_/Interface/AddOns\n"
    );
}

#[test]
fn a_failure_with_only_advice_has_no_empty_line_for_the_reason() {
    let hint = "give a new Category value, or `--clear` to restore the addon's own";
    for reason in ["", "  "] {
        let mut plain = output(OutputMode::Human, Paint::PLAIN);
        plain.fail("Save failed", reason, Some(hint)).expect("fail");
        let (_, stderr) = plain.into_writers();
        assert_eq!(
            text(stderr),
            "✗ Save failed\n  give a new Category value, or `--clear` to restore the addon's own\n"
        );
    }
    let mut colored = output(OutputMode::Human, Paint::terminal());
    colored.fail("Save failed", "", Some(hint)).expect("fail");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr).lines().nth(1),
        Some(
            format!(
                "  give a new Category value, or {} to restore the addon's own",
                command("--clear")
            )
            .as_str()
        )
    );
}

#[test]
fn only_a_report_that_could_not_be_printed_is_remembered() {
    let mut fine = output(OutputMode::Human, Paint::PLAIN);
    fine.warn("one").expect("warn");
    fine.print(&sample(), None).expect("print");
    assert!(!fine.report_failed());

    // A reader that went away is not a failure of the report.
    let mut closed = Output::new(
        OutputMode::Human,
        Paint::PLAIN,
        Paint::PLAIN,
        "default",
        ClosedPipe,
        ClosedPipe,
    );
    closed.print(&sample(), None).expect("print");
    closed.note("two").expect("note");
    assert!(!closed.report_failed());

    // A warning, a notice or a failure line that can't be written comes before the report or
    // in place of it: the command's work may not be done, so it is not remembered.
    let mut mute = Output::new(
        OutputMode::Human,
        Paint::PLAIN,
        Paint::PLAIN,
        "default",
        Vec::new(),
        FullDisk,
    );
    assert!(mute.warn("one").is_err());
    assert!(mute.note("two").is_err());
    assert!(mute.fail("three", "four", None).is_err());
    assert!(!mute.report_failed());

    // A warning that comes with the report, after the work, is part of the report.
    assert!(mute.warn_with_report("five").is_err());
    assert!(mute.report_failed());
    let mut fine = output(OutputMode::Json, Paint::PLAIN);
    fine.warn_with_report("six").expect("warn");
    assert!(!fine.report_failed());
    fine.print(&sample(), None).expect("print");
    let (stdout, stderr) = fine.into_writers();
    assert!(text(stdout).contains(r#""warnings":["six"]"#));
    assert_eq!(text(stderr), "warning: six\n");

    let mut full = Output::new(
        OutputMode::Human,
        Paint::PLAIN,
        Paint::PLAIN,
        "default",
        FullDisk,
        Vec::new(),
    );
    assert!(!full.report_failed());
    assert!(full.print(&sample(), None).is_err());
    assert!(full.report_failed());
}

#[test]
fn a_sentence_wraps_like_words_but_keeps_each_command_whole() {
    assert_eq!(
        wrap_sentence("  ", "one two three four", 12),
        wrap_words("  ", "one two three four", 12)
    );
    assert_eq!(
        wrap_sentence(
            "",
            "Use `lemonup update-all --dry-run` first, or `--force`.",
            24
        ),
        [
            "Use",
            "`lemonup update-all --dry-run`",
            "first, or `--force`."
        ]
    );
    // What touches a command stays with it.
    assert_eq!(
        wrap_sentence("", "(`lemonup check`), then `lemonup update x`!", 12),
        ["(`lemonup check`),", "then", "`lemonup update x`!"]
    );
    // Nothing in the text is used up as a placeholder.
    assert_eq!(
        wrap_sentence("", "icon \u{e000} and `lemonup check` here", 80),
        ["icon \u{e000} and `lemonup check` here"]
    );
    // A backtick that marks no command holds nothing together.
    assert_eq!(
        wrap_sentence("", "a `quoted phrase` here", 10),
        ["a `quoted", "phrase`", "here"]
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
    // The colored label is for the terminal on stderr, never for the document.
    assert_eq!(
        text(stderr),
        "\x1b[38;5;3mwarning:\x1b[0m Wago.io key is missing\n"
    );
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
                hint: None,
                from: None,
                done: false,
            },
            Entry {
                name: "Pawn".to_string(),
                source: "GitHub".to_string(),
                version: "2.0".to_string(),
                update: None,
                notes: Vec::new(),
                hint: None,
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
    fn command(&self) -> &'static str {
        "broken"
    }

    fn human(&self, _: Paint) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_document_that_fails_to_serialize_prints_nothing() {
    let mut out = output(OutputMode::Json, Paint::PLAIN);
    assert!(out.print(&Broken, None).is_err());
    // It is a failure of the report, like a write that failed: not of the command's work.
    assert!(out.report_failed());
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

        // The same on stderr, for a warning, a notice and a failure.
        let mut closed = Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            Vec::new(),
            ClosedPipe,
        );
        assert!(closed.warn("one").is_ok(), "{mode:?}");
        assert!(closed.note("two").is_ok(), "{mode:?}");
        assert!(
            closed.fail("three", "four", Some("five")).is_ok(),
            "{mode:?}"
        );
        let mut full = Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            Vec::new(),
            FullDisk,
        );
        assert!(full.warn("one").is_err(), "{mode:?}");
        assert!(full.note("two").is_err(), "{mode:?}");
        assert!(
            full.fail("three", "four", Some("five")).is_err(),
            "{mode:?}"
        );
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

#[test]
fn widths_are_measured_on_the_text_as_it_is_printed() {
    // A control character is printed as a space, so it takes a column in the layout too.
    assert_eq!(columns("Pa\x1bwn"), 5);
    assert_eq!(pad("Pa\x1bwn", 7), "Pa wn  ");
    let rows = table(
        vec![
            Entry {
                name: "Pa\x07wn".to_string(),
                source: "Wago.io".to_string(),
                version: "1.0".to_string(),
                update: None,
                notes: Vec::new(),
                hint: None,
                from: None,
                done: false,
            },
            Entry {
                name: "Postal".to_string(),
                source: "GitHub".to_string(),
                version: "2.0".to_string(),
                update: None,
                notes: Vec::new(),
                hint: None,
                from: None,
                done: false,
            },
        ],
        Paint::PLAIN,
    );
    assert_eq!(rows, ["  Pa wn   Wago.io  1.0", "  Postal  GitHub   2.0"]);
}

#[test]
fn a_version_is_shown_short_and_never_blank() {
    assert_eq!(display_version(None, &["Pawn"]), "unknown");
    assert_eq!(display_version(Some("  "), &["Pawn"]), "unknown");
    assert_eq!(display_version(Some("Pawn 2.14.4"), &["Pawn"]), "2.14.4");
    // A full commit hash is shown as its first seven characters.
    assert_eq!(
        display_version(Some("0f4d2e4b61234567890abcdef0123456789abcde"), &["Pawn"]),
        "0f4d2e4"
    );
    // A dated version is all digits but not a commit.
    assert_eq!(display_version(Some("20261122"), &["Pawn"]), "20261122");
}

#[test]
fn every_kind_of_source_failure_has_its_own_name_in_json() {
    use crate::source_error::SourceErrorKind;
    let names = [
        SourceErrorKind::NoConnection,
        SourceErrorKind::KeyRejected,
        SourceErrorKind::NotFound,
        SourceErrorKind::SlowDown,
        SourceErrorKind::WrongRepository,
        SourceErrorKind::FoldersInTheWay,
        SourceErrorKind::FoldersOfLinkedAddon,
        SourceErrorKind::Other,
    ]
    .map(error_kind_key);
    assert_eq!(
        names,
        [
            "no_connection",
            "key_rejected",
            "not_found",
            "slow_down",
            "wrong_repository",
            "folders_in_the_way",
            "folders_of_linked_addon",
            "other",
        ]
    );
}

#[test]
fn a_note_reaches_stderr_in_json_mode_too() {
    let mut out = output(OutputMode::Json, Paint::PLAIN);
    out.note("Made a backup of your game settings: /data/WTF.zip")
        .expect("note");
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "");
    assert_eq!(
        text(stderr),
        "Made a backup of your game settings: /data/WTF.zip\n"
    );
}

/// `line` without its escape codes.
pub(super) fn strip(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// The JSON document `report` prints for the default profile, after `warnings`.
pub(super) fn json_document<R: Report>(
    report: &R,
    addon_dir: Option<&Path>,
    warnings: &[&str],
) -> String {
    let mut output = Output::new(
        OutputMode::Json,
        Paint::terminal(),
        Paint::terminal(),
        "default",
        Vec::new(),
        Vec::new(),
    );
    for warning in warnings {
        output.warn(warning).expect("warn");
    }
    output.print(report, addon_dir).expect("print");
    // Warnings are on stderr as well; the document is what stdout got.
    let (stdout, _) = output.into_writers();
    String::from_utf8(stdout).expect("utf-8")
}

#[test]
fn a_readable_only_report_prints_nothing_with_json() {
    let mut out = output(OutputMode::Human, Paint::PLAIN);
    out.print_readable(&sample()).expect("print");
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "Questie\nPawn\n");
    assert_eq!(text(stderr), "");

    // A run that ends in a refusal or a failure prints no document.
    let mut out = output(OutputMode::Json, Paint::PLAIN);
    out.warn("WoWInterface didn't answer").expect("warn");
    out.print_readable(&sample()).expect("print");
    assert!(!out.report_failed());
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "");
    assert_eq!(text(stderr), "warning: WoWInterface didn't answer\n");
}

#[test]
fn a_refusal_is_two_lines_on_stderr_and_only_its_hint_names_commands() {
    const END: &str = "\x1b[0m";
    const COMMAND: &str = "\x1b[1m\x1b[38;5;6m";
    // The reason names addons and folders: an escape code in one never gets through, a line
    // break in one does not start a new line, and a name that reads like a command is not
    // drawn as one.
    let reason = "Nothing linked - Evil\x1b]0;x\x07 also takes `lemonup update --force`,\nOther";
    let hint = "Run `lemonup find --link --yes` to link the addon ready to link";

    for mode in [OutputMode::Human, OutputMode::Json] {
        let mut plain = output(mode, Paint::PLAIN);
        plain.refuse(reason, hint).expect("refuse");
        let (stdout, stderr) = plain.into_writers();
        assert_eq!(text(stdout), "");
        assert_eq!(
            text(stderr),
            "Nothing linked - Evil ]0;x  also takes `lemonup update --force`, Other\n\
             Run `lemonup find --link --yes` to link the addon ready to link\n"
        );
    }

    let mut colored = output(OutputMode::Human, Paint::terminal());
    colored.refuse(reason, hint).expect("refuse");
    let (_, stderr) = colored.into_writers();
    assert_eq!(
        text(stderr),
        format!(
            "Nothing linked - Evil ]0;x  also takes `lemonup update --force`, Other\n\
             Run {COMMAND}lemonup find --link --yes{END} to link the addon ready to link\n"
        )
    );
}

#[test]
fn everything_printed_takes_the_status_line_off_the_screen_first() {
    use super::progress::testing::{Buffer, progress};
    const LINE_GONE: &str = "\r\x1b[K\x1b[?25h";
    // One terminal shows both streams and the status line, so all three write to one place
    // here and the order is what a person sees.
    let screen = |mode| {
        let (status_line, seen) = progress(Paint::PLAIN, false, 80);
        let output = Output::new(
            mode,
            Paint::PLAIN,
            Paint::PLAIN,
            "forever",
            Buffer::clone(&seen),
            Buffer::clone(&seen),
        )
        .with_progress(Some(status_line));
        (output, seen)
    };
    let frame = |output: &Output<Buffer, Buffer>| output.progress().expect("status line").tick();

    let (mut out, seen) = screen(OutputMode::Human);
    let status = out.status();
    status.say("Checking 1 of 2");
    frame(&out);
    assert_eq!(seen.take(), "\r\x1b[K\x1b[?25l⠋ Checking 1 of 2");

    // A warning and a note come mid-run: the line makes room, then comes back.
    out.warn("WoWInterface didn't answer").expect("warn");
    assert_eq!(
        seen.take(),
        format!("{LINE_GONE}warning: WoWInterface didn't answer\n")
    );
    frame(&out);
    assert_eq!(seen.take(), "\r\x1b[K\x1b[?25l⠙ Checking 1 of 2");
    out.note("Made a backup").expect("note");
    assert_eq!(seen.take(), format!("{LINE_GONE}Made a backup\n"));
    frame(&out);
    seen.take();

    // The report ends the status line for good.
    out.print(&sample(), None).expect("print");
    assert_eq!(seen.take(), format!("{LINE_GONE}Questie\nPawn\n"));
    frame(&out);
    assert_eq!(seen.take(), "");

    // So does a failure, a refusal, and a report shown before the work is done.
    type Ending = fn(&mut Output<Buffer, Buffer>);
    let endings: [(Ending, &str); 3] = [
        (
            |out| out.fail("Check failed", "no", None).expect("fail"),
            "✗ Check failed\n  no\n",
        ),
        (
            |out| {
                out.refuse("Nothing linked", "Run `lemonup find`")
                    .expect("refuse")
            },
            "Nothing linked\nRun `lemonup find`\n",
        ),
        (
            |out| out.print_readable(&sample()).expect("print"),
            "Questie\nPawn\n",
        ),
    ];
    for (end, printed) in endings {
        let (mut out, seen) = screen(OutputMode::Human);
        out.status().say("Asking TukUI");
        frame(&out);
        seen.take();
        end(&mut out);
        assert_eq!(seen.take(), format!("{LINE_GONE}{printed}"));
        frame(&out);
        assert_eq!(seen.take(), "");
    }
}

#[test]
fn a_run_without_a_status_line_shows_none() {
    // In a pipe and with `--json` there is no status line: saying something does nothing.
    let mut out = output(OutputMode::Human, Paint::PLAIN);
    let status = out.status();
    status.say("Checking 1 of 2");
    status.show(
        "Checking 2 of 2",
        "Checking 2 addons",
        crate::terminal_progress::Bar::Off,
    );
    out.warn("careful").expect("warn");
    status.clear();
    out.print(&sample(), None).expect("print");
    let (stdout, stderr) = out.into_writers();
    assert_eq!(text(stdout), "Questie\nPawn\n");
    assert_eq!(text(stderr), "warning: careful\n");
}
