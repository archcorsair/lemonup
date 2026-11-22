//! A newer LemonUp: the header badge and the setting. The answer is planted in the app;
//! nothing here looks on GitHub.

use super::*;
use crate::app::{ConfigRowKind, Modifier};
use crate::theme::Symbols;
use lemonup_core::lemonup_release::LatestRelease;

/// What the badge says for 9.9.9, after the arrow.
fn words() -> String {
    format!("Update available {} → 9.9.9", env!("CARGO_PKG_VERSION"))
}

/// The whole badge for 9.9.9.
fn badge() -> String {
    format!("↑ {}", words())
}

/// An answer naming `version`, published `age` ago and checked just now.
fn answer(version: &str, age: Duration) -> LatestRelease {
    let now = OffsetDateTime::now_utc();
    LatestRelease {
        version: version.to_string(),
        published_at: now - age,
        checked_at: now,
    }
}

/// The addon list of a Retail game in a long folder, knowing `latest`.
fn app_knowing(latest: Option<LatestRelease>) -> App {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.effective_addon_dir = Some(PathBuf::from(
        "/home/me/Games/battlenet/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
    ));
    app.newer_lemonup.latest = latest;
    app
}

/// A newer LemonUp, two days old.
fn newer_app() -> App {
    app_knowing(Some(answer("9.9.9", Duration::days(2))))
}

fn header(app: &mut App, width: u16, height: u16) -> Vec<String> {
    render_rows(app, width, height)[..3].to_vec()
}

/// The end of Help, drawn tall enough to show it whole.
fn help_text(app: &mut App, width: u16) -> String {
    app.shell_ui.overlay.active = Some(OverlayKind::Help);
    render_rows(app, width, 90).join("\n")
}

#[test]
fn the_header_shows_a_newer_lemonup_at_the_right_end() {
    let mut app = newer_app();
    let rows = header(&mut app, 80, 24);
    assert!(rows[0].ends_with(&badge()), "{rows:#?}");
    assert!(rows[0].starts_with(" LEMONUP  Retail  "), "{rows:#?}");
    // The folder shrank to make room.
    assert!(rows[0].contains("…"), "{rows:#?}");
    assert!(rows[0].contains("Interface/AddOns"), "{rows:#?}");

    let rows = header(&mut app, 120, 36);
    assert!(rows[0].ends_with(&badge()), "{rows:#?}");
    assert!(rows[0].contains("█   █▀▀"), "{rows:#?}");
    assert!(rows[0].contains("Retail"), "{rows:#?}");
    // Only once, on the first row.
    assert!(
        !rows[1..].join("\n").contains("Update available"),
        "{rows:#?}"
    );
}

#[test]
fn the_arrow_is_amber_and_the_words_shimmer_in_amber() {
    let mut app = newer_app();
    let now = OffsetDateTime::now_utc();
    let roles = app.ui_theme.roles;
    let line = &app.dashboard_header_lines(80, now)[0];
    let arrow = line
        .spans
        .iter()
        .position(|span| span.content == "↑")
        .expect("arrow");
    assert_eq!(line.spans[arrow].style.fg, Some(roles.amber));
    assert!(
        line.spans[arrow]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    let shimmer = &line.spans[arrow + 2..];
    // One span a letter, so the sweep can move across it.
    assert!(shimmer.len() > 1);
    let text: String = shimmer.iter().map(|span| span.content.as_ref()).collect();
    assert_eq!(text, words());
    assert!(app.has_active_animation());

    // Reduce motion: still, plain amber, and no redraws for it.
    app.ui_theme.reduce_motion = true;
    assert!(!app.has_active_animation());
    let line = &app.dashboard_header_lines(80, now)[0];
    let last = line.spans.last().expect("words");
    assert_eq!(last.content, words());
    assert_eq!(last.style.fg, Some(roles.amber));

    // Nothing newer: nothing to redraw for.
    let app = app_knowing(None);
    assert!(!app.has_active_animation());
}

#[test]
fn simple_symbols_show_a_caret() {
    let mut app = newer_app();
    app.ui_theme.symbols = Symbols::SIMPLE;
    let simple = format!("^ Update available {} -> 9.9.9", env!("CARGO_PKG_VERSION"));
    let rows = header(&mut app, 80, 24);
    assert!(rows[0].ends_with(&simple), "{rows:#?}");
    let rows = header(&mut app, 120, 36);
    assert!(rows[0].ends_with(&simple), "{rows:#?}");
}

/// Every case in which nothing newer is said: none known, the same or an older version, a
/// pre-release, a release younger than six hours, and the setting off.
fn quiet_apps() -> Vec<(&'static str, App)> {
    let mut off = newer_app();
    off.config_pane.draft.check_lemonup_updates = false;
    vec![
        ("no answer", app_knowing(None)),
        (
            "same version",
            app_knowing(Some(answer(env!("CARGO_PKG_VERSION"), Duration::days(2)))),
        ),
        (
            "older version",
            app_knowing(Some(answer("1.0.0", Duration::days(2)))),
        ),
        (
            "pre-release",
            app_knowing(Some(answer("9.9.9-rc.1", Duration::days(2)))),
        ),
        (
            "five hours old",
            app_knowing(Some(answer("9.9.9", Duration::hours(5)))),
        ),
        ("setting off", off),
    ]
}

#[test]
fn nothing_newer_shows_no_badge() {
    for (case, mut app) in quiet_apps() {
        for (width, height) in [(80, 24), (120, 36)] {
            let rows = header(&mut app, width, height);
            let text = rows.join("\n");
            assert!(!text.contains("Update available"), "{case}:\n{text}");
            assert!(!rows[0].contains('↑'), "{case}:\n{text}");
            // The folder ends the row, as before.
            assert!(
                rows[0].trim_end().ends_with("Interface/AddOns"),
                "{case}:\n{text}"
            );
        }
    }
}

#[test]
fn a_release_shows_once_it_is_six_hours_old() {
    let mut app = app_knowing(Some(answer(
        "9.9.9",
        Duration::hours(6) + Duration::minutes(1),
    )));
    assert!(header(&mut app, 80, 24)[0].ends_with(&badge()));
}

#[test]
fn the_game_name_never_clips_and_the_badge_goes_first() {
    let mut app = newer_app();
    app.games.games = two_test_games();
    app.games.games[1].update_ready = Some(3);
    // At 80 the folder, the other game and the badge all fit.
    let rows = header(&mut app, 80, 24);
    assert!(
        rows[0].contains(" LEMONUP  Retail   WoW Forever ↑3  "),
        "{rows:#?}"
    );
    assert!(rows[0].ends_with(&badge()), "{rows:#?}");

    let now = OffsetDateTime::now_utc();
    let first = |app: &App, width: u16| line_text(&app.dashboard_header_lines(width, now)[0]);
    let lead = " LEMONUP  Retail   WoW Forever ↑3";
    let lead_width = lead.chars().count();
    let badge_width = badge().chars().count();
    // Too narrow for the folder beside the badge: the folder goes.
    let width = lead_width + 2 + badge_width + 3;
    let row = first(&app, u16::try_from(width).expect("width"));
    assert_eq!(row, format!("{lead}     {}", badge()), "{row}");
    // Too narrow for the badge: it goes, and the game name stays whole.
    assert!(app.has_active_animation());
    let row = first(
        &app,
        u16::try_from(lead_width + 1 + badge_width).expect("width"),
    );
    assert!(row.starts_with(lead), "{row}");
    assert!(!row.contains("Update available"), "{row}");
    // No badge drawn, so nothing shimmers and the frame doesn't redraw for it.
    assert!(!app.has_active_animation());
}

#[test]
fn help_does_not_mention_a_newer_lemonup() {
    let mut app = newer_app();
    // At 80 columns Help covers the whole screen, header included.
    let text = help_text(&mut app, 80);
    assert!(!text.contains("9.9.9"), "{text}");
    assert!(text.contains("lemonup.org/keyboard-shortcuts"), "{text}");
}

#[test]
fn settings_has_the_toggle_for_all_games_on_by_default() {
    let mut app = newer_app();
    assert!(app.config_pane.draft.check_lemonup_updates);
    app.apply(AppAction::SetDetailMode(DetailMode::Config));
    app.config_pane.selected_field = ConfigField::ALL
        .iter()
        .position(|field| *field == ConfigField::CheckLemonupUpdates)
        .expect("row");
    let text = render_rows(&mut app, 138, 40).join("\n");
    let all = text.find("For all games").expect("For all games");
    let this = text.find("For Retail").expect("For Retail");
    let row = text.find("Check for LemonUp updates").expect("row");
    assert!(all < row && row < this, "{text}");
    let row_line = text
        .lines()
        .find(|line| line.contains("Check for LemonUp updates") && line.contains("On"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(row_line.contains("On"), "{row_line}");
    let description = render_rows(&mut app, 160, 45).join("\n");
    assert!(
        description.contains(
            "Looks on GitHub once a day for a newer LemonUp and shows it at the top of the addon list."
        ),
        "{description}"
    );
    assert_eq!(
        ConfigField::CheckLemonupUpdates.kind(),
        ConfigRowKind::Toggle
    );
}

#[test]
fn turning_the_setting_off_hides_the_badge_right_away() {
    let mut app = newer_app();
    app.config_pane.selected_field = ConfigField::ALL
        .iter()
        .position(|field| *field == ConfigField::CheckLemonupUpdates)
        .expect("row");
    let next = app.config_pane.toggle_selected();
    assert!(!next.draft.check_lemonup_updates);
    app.config_pane = next;
    assert!(!header(&mut app, 80, 24)[0].contains("Update available"));
}

#[test]
fn a_launch_looks_once_and_only_with_the_setting_on() {
    // Off: no look, and it waits for the setting.
    let mut app = app_knowing(Some(answer("2.0.0", Duration::days(2))));
    let now = app
        .newer_lemonup
        .latest
        .as_ref()
        .expect("answer")
        .checked_at
        + Duration::hours(1);
    app.config_pane.draft.check_lemonup_updates = false;
    app.start_newer_lemonup_check(now);
    assert!(!app.newer_lemonup.started);

    // Turned on, with an answer from an hour ago: no look is due, and none starts later
    // this launch.
    app.config_pane.draft.check_lemonup_updates = true;
    app.start_newer_lemonup_check(now);
    assert!(app.newer_lemonup.started);
    // A look is due now, but `started` keeps a launch to one: a second look would call
    // `tokio::spawn`, which panics here, outside a runtime.
    app.newer_lemonup.latest = None;
    app.start_newer_lemonup_check(now + Duration::days(2));
    assert!(app.newer_lemonup.started);
}

#[test]
fn setup_never_looks() {
    let mut app = app_for_tests(ShellMode::Onboarding);
    app.config_pane.draft.check_lemonup_updates = true;
    // No answer, so a look is due; outside the addon list none starts (and `tokio::spawn`
    // would panic here, outside a runtime).
    app.start_newer_lemonup_check(OffsetDateTime::now_utc());
    assert!(!app.newer_lemonup.started);
}

#[test]
fn the_answer_of_the_look_shows_when_it_lands() {
    let mut app = app_knowing(None);
    assert!(!header(&mut app, 80, 24)[0].contains("Update available"));
    app.newer_lemonup
        .sender
        .send(answer("9.9.9", Duration::days(2)))
        .expect("send");
    app.process_background_events();
    assert!(header(&mut app, 80, 24)[0].ends_with(&badge()));
}
