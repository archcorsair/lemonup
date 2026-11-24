//! The terminal's own progress bar in the TUI: what it shows while a check or an update
//! runs, for a moment after, and while something waits with no count to show.

use super::*;
use crate::app::find_sheet::FindStep;
use crate::app::{
    CheckFailure, ConfigRowKind, DashboardJobStep, SearchInstallState, TERMINAL_BAR_HOLD,
};
use crate::source_error::SourceErrorKind;
use crate::terminal_progress::Bar;
use crate::update::LiveUpdateStatus;

/// The addon list in a terminal that draws a progress bar.
fn app() -> App {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.runtime.display.progress_bar = true;
    app
}

fn job(app: &mut App, kind: DashboardJobKind, finished: usize, total: usize) {
    app.apply(AppAction::SetDashboardJobUi(Some(DashboardJobUiState {
        kind,
        finished,
        total,
        active: Vec::new(),
    })));
}

fn check_failed(app: &mut App, folder: &str) {
    app.apply(AppAction::ApplyDashboardJobStep(
        DashboardJobStep::Checked {
            folder: folder.to_string(),
            result: Err(CheckFailure {
                kind: SourceErrorKind::NoConnection,
                source: SourceKind::GitHub,
                detail: "timed out".to_string(),
                github_token: None,
            }),
        },
    ));
}

#[test]
fn the_bar_follows_a_job_and_stays_full_for_a_moment_after_it() {
    let mut app = app();
    let now = Instant::now();
    assert_eq!(app.terminal_bar(now), Bar::Off);

    job(&mut app, DashboardJobKind::Check, 0, 4);
    assert_eq!(app.terminal_bar(now), Bar::of(0, 4, false));
    job(&mut app, DashboardJobKind::Check, 3, 4);
    assert_eq!(app.terminal_bar(now), Bar::of(3, 4, false));
    // A running job keeps the ticks coming.
    assert!(app.has_active_animation());

    let before = Instant::now();
    app.apply(AppAction::SetDashboardJobUi(None));
    // Full, then gone after half a second. The ticks go on until then.
    let (held, until) = app.dashboard.bar_hold.expect("a bar is held");
    assert_eq!(held, Bar::of(1, 1, false));
    assert!(until >= before + TERMINAL_BAR_HOLD);
    assert!(until <= Instant::now() + TERMINAL_BAR_HOLD);
    assert!(app.has_active_animation());
    assert_eq!(
        app.terminal_bar(until - StdDuration::from_millis(1)),
        Bar::of(1, 1, false)
    );
    assert_eq!(app.terminal_bar(until), Bar::Off);

    // Where no bar is in use, a job that ended asks for no more ticks.
    app.config_pane.draft.terminal_progress = false;
    assert!(!app.has_active_animation());
}

#[test]
fn the_bar_is_yellow_from_the_first_failed_addon_to_the_end() {
    let mut app = app();
    let now = Instant::now();
    job(&mut app, DashboardJobKind::Check, 1, 4);
    check_failed(&mut app, "Second");
    assert_eq!(app.terminal_bar(now), Bar::of(1, 4, true));
    job(&mut app, DashboardJobKind::Check, 4, 4);
    assert_eq!(app.terminal_bar(now), Bar::of(4, 4, true));
    app.apply(AppAction::SetDashboardJobUi(None));
    assert_eq!(
        app.dashboard.bar_hold.map(|(bar, _)| bar),
        Some(Bar::of(1, 1, true))
    );

    // The next job starts clean.
    job(&mut app, DashboardJobKind::Check, 0, 4);
    assert_eq!(app.terminal_bar(Instant::now()), Bar::of(0, 4, false));

    // An addon that fails to update counts the same.
    let mut app = self::app();
    job(&mut app, DashboardJobKind::Update, 0, 2);
    app.apply(AppAction::ApplyDashboardJobStep(
        DashboardJobStep::Updated {
            folder: "Second".to_string(),
            status: LiveUpdateStatus::Error,
        },
    ));
    assert_eq!(app.terminal_bar(Instant::now()), Bar::of(0, 2, true));
    // One that is held for a yes, or up to date, is no trouble.
    let mut app = self::app();
    job(&mut app, DashboardJobKind::Update, 0, 2);
    for status in [LiveUpdateStatus::Held, LiveUpdateStatus::UpToDate] {
        app.apply(AppAction::ApplyDashboardJobStep(
            DashboardJobStep::Updated {
                folder: "Second".to_string(),
                status,
            },
        ));
    }
    assert_eq!(app.terminal_bar(Instant::now()), Bar::of(0, 2, false));
}

#[test]
fn a_run_that_fails_as_a_whole_leaves_a_red_bar_for_a_moment() {
    let mut app = app();
    job(&mut app, DashboardJobKind::Check, 1, 4);
    let actions = app.update(AppMessage::BackgroundTask(
        AppTaskEvent::DashboardCheckFinished(Err("the list could not be read".to_string())),
    ));
    assert!(
        actions.contains(&AppAction::TerminalBarFailed),
        "{actions:?}"
    );
    for action in actions {
        app.apply(action);
    }
    let (held, until) = app.dashboard.bar_hold.expect("a bar is held");
    assert_eq!(held, Bar::Failed);
    assert_eq!(
        app.terminal_bar(until - StdDuration::from_millis(1)),
        Bar::Failed
    );
    assert_eq!(app.terminal_bar(until), Bar::Off);

    // A run can fail before it counted a single addon: it is red all the same.
    let mut app = self::app();
    let actions = app.update(AppMessage::BackgroundTask(
        AppTaskEvent::DashboardCheckFinished(Err("the list could not be read".to_string())),
    ));
    for action in actions {
        app.apply(action);
    }
    let (held, _) = app.dashboard.bar_hold.expect("a bar is held");
    assert_eq!(held, Bar::Failed);
}

#[test]
fn work_with_no_count_to_show_pulses() {
    let now = Instant::now();
    let mut app = app();
    app.import_running = true;
    assert_eq!(app.terminal_bar(now), Bar::Pulse);
    app.import_running = false;
    assert_eq!(app.terminal_bar(now), Bar::Off);

    app.find.step = Some(FindStep::Looking { folders: 12 });
    assert_eq!(app.terminal_bar(now), Bar::Pulse);
    // A job's count wins over the pulse.
    job(&mut app, DashboardJobKind::Check, 1, 4);
    assert_eq!(app.terminal_bar(now), Bar::of(1, 4, false));
    app.apply(AppAction::SetDashboardJobUi(None));
    app.find.step = None;

    // The pulse is an animation, which Reduce motion turns off. A count is not.
    let mut still = self::app();
    still.ui_theme.reduce_motion = true;
    still.import_running = true;
    assert_eq!(still.terminal_bar(now), Bar::Off);
    job(&mut still, DashboardJobKind::Check, 1, 4);
    assert_eq!(still.terminal_bar(now), Bar::of(1, 4, false));
}

#[test]
fn an_install_pulses_while_it_waits_and_not_while_it_asks() {
    let now = Instant::now();
    let named = || ("id".to_string(), "Pawn".to_string());
    for (state, pulses) in [
        (SearchInstallState::Idle, false),
        (
            {
                let (addon_id, addon_name) = named();
                SearchInstallState::Checking {
                    addon_id,
                    addon_name,
                }
            },
            true,
        ),
        (
            {
                let (addon_id, addon_name) = named();
                SearchInstallState::Installing {
                    addon_id,
                    addon_name,
                }
            },
            true,
        ),
        // A question waits for a key, not for a source.
        (
            {
                let (addon_id, addon_name) = named();
                SearchInstallState::ConfirmReplace {
                    addon_id,
                    addon_name,
                    tracked_parent: "Pawn".to_string(),
                }
            },
            false,
        ),
    ] {
        let mut app = app();
        app.search_pane.install_state = state.clone();
        let expected = if pulses { Bar::Pulse } else { Bar::Off };
        assert_eq!(app.terminal_bar(now), expected, "{state:?}");
    }
}

#[test]
fn an_update_keeps_its_trouble_from_its_check_to_its_end() {
    // An update checks first, then updates: two phases of one job.
    let mut app = app();
    job(&mut app, DashboardJobKind::Check, 0, 2);
    check_failed(&mut app, "Second");
    job(&mut app, DashboardJobKind::Update, 0, 1);
    // The new phase counts from the start, and is still yellow. Nothing is held: the job
    // goes on.
    assert_eq!(app.terminal_bar(Instant::now()), Bar::of(0, 1, true));
    job(&mut app, DashboardJobKind::Update, 1, 1);
    let actions = app.update(AppMessage::BackgroundTask(
        AppTaskEvent::DashboardUpdateFinished(Err("the folder is gone".to_string())),
    ));
    for action in actions {
        app.apply(action);
    }
    // The update failed as a whole: red for a moment.
    assert_eq!(
        app.dashboard.bar_hold.map(|(bar, _)| bar),
        Some(Bar::Failed)
    );

    // A failure that comes in when no job runs is not the next job's trouble.
    let mut app = self::app();
    check_failed(&mut app, "Second");
    job(&mut app, DashboardJobKind::Check, 0, 2);
    assert_eq!(app.terminal_bar(Instant::now()), Bar::of(0, 2, false));
}

#[test]
fn the_check_an_update_starts_with_pulses_and_only_the_update_fills_the_bar() {
    let now = Instant::now();
    let mut app = app();
    app.dashboard
        .begin_update_targets(&["Second".to_string(), "Third".to_string()]);
    // The check of an update is not the update's progress: a bar that filled for it would
    // say the update is done before it began.
    job(&mut app, DashboardJobKind::Check, 1, 2);
    assert_eq!(app.terminal_bar(now), Bar::Pulse);
    job(&mut app, DashboardJobKind::Check, 2, 2);
    assert_eq!(app.terminal_bar(now), Bar::Pulse);
    job(&mut app, DashboardJobKind::Update, 0, 2);
    assert_eq!(app.terminal_bar(now), Bar::of(0, 2, false));
    job(&mut app, DashboardJobKind::Update, 1, 2);
    assert_eq!(app.terminal_bar(now), Bar::of(1, 2, false));

    // Under Reduce motion there is no pulse, and the update still counts.
    let mut still = self::app();
    still.ui_theme.reduce_motion = true;
    still
        .dashboard
        .begin_update_targets(&["Second".to_string()]);
    job(&mut still, DashboardJobKind::Check, 0, 1);
    assert_eq!(still.terminal_bar(now), Bar::Off);
    job(&mut still, DashboardJobKind::Update, 0, 1);
    assert_eq!(still.terminal_bar(now), Bar::of(0, 1, false));

    // A check on its own fills the bar.
    let mut check = self::app();
    job(&mut check, DashboardJobKind::Check, 1, 2);
    assert_eq!(check.terminal_bar(now), Bar::of(1, 2, false));
}

#[test]
fn no_bar_where_the_terminal_draws_none_or_the_setting_is_off() {
    let now = Instant::now();
    // Other terminals show the sequence as a notification.
    let mut unknown = app();
    unknown.runtime.display.progress_bar = false;
    job(&mut unknown, DashboardJobKind::Check, 1, 4);
    unknown.import_running = true;
    assert_eq!(unknown.terminal_bar(now), Bar::Off);

    let mut off = app();
    off.config_pane.draft.terminal_progress = false;
    job(&mut off, DashboardJobKind::Check, 1, 4);
    assert_eq!(off.terminal_bar(now), Bar::Off);
    off.config_pane.draft.terminal_progress = true;
    assert_eq!(off.terminal_bar(now), Bar::of(1, 4, false));
}

#[test]
fn settings_has_the_terminal_progress_bar_row_and_says_when_the_terminal_draws_none() {
    let mut app = app();
    assert!(app.config_pane.draft.terminal_progress);
    assert_eq!(
        ConfigField::TerminalProgress.label(),
        "Terminal progress bar"
    );
    assert_eq!(ConfigField::TerminalProgress.kind(), ConfigRowKind::Toggle);
    // It sits with the other settings of how LemonUp looks.
    let at = |field| {
        ConfigField::ALL
            .iter()
            .position(|candidate| *candidate == field)
            .expect("row")
    };
    assert_eq!(
        at(ConfigField::TerminalProgress),
        at(ConfigField::ReduceMotion) + 1
    );
    assert!(!app.look_setting_forced(ConfigField::TerminalProgress));
    assert_eq!(
        app.look_value_text(&app.config_pane.draft, ConfigField::TerminalProgress),
        "On"
    );
    app.config_pane.selected_field = at(ConfigField::TerminalProgress);
    let off = app.config_pane.toggle_selected();
    assert!(!off.draft.terminal_progress);

    // In a terminal that draws none, the row says Off whatever is saved.
    app.runtime.display.progress_bar = false;
    assert!(app.look_setting_forced(ConfigField::TerminalProgress));
    assert_eq!(
        app.look_value_text(&app.config_pane.draft, ConfigField::TerminalProgress),
        "Off"
    );
    for action in app.update(AppMessage::SetDetailMode(DetailMode::Config)) {
        app.apply(action);
    }
    // And no key changes it there: the row could not show the change.
    for message in [
        AppMessage::ConfigToggleSelected,
        AppMessage::ConfigAdjustSelected(1),
        AppMessage::ConfigAdjustSelected(-1),
    ] {
        for action in app.update(message) {
            app.apply(action);
        }
        assert!(app.config_pane.draft.terminal_progress);
        assert!(app.config_pane.persisted.terminal_progress);
    }
    // The narrow screen says why in a line, the wide one in the help under the list.
    let narrow = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        narrow.contains("off, because this terminal isn't known to draw one"),
        "{narrow}"
    );
    let wide = render_rows(&mut app, 120, 36).join("\n");
    assert!(wide.contains("It is off now:"), "{wide}");

    // Where the terminal draws one, enter turns it off and on.
    app.runtime.display.progress_bar = true;
    for action in app.update(AppMessage::ConfigToggleSelected) {
        app.apply(action);
    }
    assert!(!app.config_pane.draft.terminal_progress);
    let text = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        text.contains("shows progress in your terminal's own progress bar"),
        "{text}"
    );
}
