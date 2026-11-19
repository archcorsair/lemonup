//! Marking linked addons Ignored in CurseForge. Every record is a synthetic
//! one in a temp folder, and whether CurseForge runs is set by each test.

use super::*;
use crate::app::dialog::Question;
use crate::app::find_sheet::link_found;
use crate::app::{CurseForgeMessage, FindTaskEvent, curseforge_mark};
use lemonup_core::find::{FindLabel, FindSource, FoundAddon};

fn closed() -> bool {
    false
}

fn open() -> bool {
    true
}

/// An app that can mark addons, with CurseForge closed, on a synthetic game
/// whose record manages `managed`.
fn marking_app(managed: &[&str]) -> (App, tempfile::TempDir, PathBuf) {
    let mut app = app_for_tests(ShellMode::Dashboard);
    let (temp, addons, record) = curseforge_game(managed);
    app.effective_addon_dir = Some(addons);
    app.curseforge_files = vec![record.clone()];
    app.curseforge_can_mark = true;
    app.curseforge_running = closed;
    (app, temp, record)
}

fn flags(record: &Path) -> Vec<serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(record).expect("record")).expect("json");
    value[0]["installedAddons"]
        .as_array()
        .expect("addons")
        .iter()
        .map(|entry| entry["preferenceIsIgnored"].clone())
        .collect()
}

fn found(name: &str) -> FoundAddon {
    FoundAddon {
        label: FindLabel::Exact,
        source: FindSource::Wago,
        source_id: format!("{name}-id"),
        source_url: format!("https://example.invalid/{name}"),
        name: name.to_string(),
        source_name: Some(name.to_string()),
        author: None,
        parent_folder: name.to_string(),
        folders: vec![name.to_string()],
        matched_version: Some("1.0".to_string()),
        latest_version: Some("1.0".to_string()),
        joining: Vec::new(),
        leaving: Vec::new(),
        likely_reason: None,
        missing: Vec::new(),
        differing: Vec::new(),
        blocked: Vec::new(),
    }
}

/// Links Clique and Pawn in `app`'s game, as the Find sheet does, and hands
/// the result to `app`.
fn link_two(app: &mut App) {
    let addons = app.effective_addon_dir.clone().expect("addons");
    for folder in ["Clique", "Pawn"] {
        fs::create_dir_all(addons.join(folder)).expect("addon dir");
        fs::write(
            addons.join(folder).join(format!("{folder}.toc")),
            format!("## Interface: 110005\n## Title: {folder}\n## Version: 1.0\n"),
        )
        .expect("toc");
    }
    let db = addons.join("..").join("state.sqlite");
    let outcome = link_found(
        &db,
        &addons,
        &[found("Clique"), found("Pawn")],
        &app.curseforge_files,
    )
    .expect("linked");
    app.find.linking = true;
    apply_all(
        app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Ok(Box::new(
            outcome,
        ))))),
    );
}

/// The keys `code` sends while the question has been on screen.
fn answer(app: &mut App, code: KeyCode) -> Vec<AppMessage> {
    app.note_question_shown(Instant::now());
    let messages = app.messages_for_key(KeyEvent::from(code));
    for message in messages.clone() {
        apply_all(app, message);
    }
    messages
}

#[test]
fn after_a_link_curseforge_still_manages_the_second_question_asks_and_y_marks() {
    let (mut app, _game, record) = marking_app(&["Pawn"]);
    link_two(&mut app);
    assert!(
        app.status_line.text.contains(
            "Linked 2 addons: 2 up to date. CurseForge also manages Pawn. Mark it Ignored there."
        ),
        "{}",
        app.status_line.text
    );
    assert_eq!(
        app.active_question(),
        Some(Question::MarkIgnored {
            names: vec!["Pawn".to_string()]
        })
    );
    for (width, height) in [(80, 24), (160, 40)] {
        let rows = render_rows(&mut app, width, height).join("\n");
        assert!(rows.contains("Mark Pawn Ignored in CurseForge?"), "{rows}");
        assert!(rows.contains("y  mark"), "{rows}");
        assert!(rows.contains("n  skip"), "{rows}");
    }

    // Enter never answers it.
    assert!(
        app.messages_for_key(KeyEvent::from(KeyCode::Enter))
            .is_empty()
    );
    assert_eq!(flags(&record), [serde_json::json!(false)]);

    answer(&mut app, KeyCode::Char('y'));
    assert_eq!(app.active_question(), None);
    assert_eq!(flags(&record), [serde_json::json!(true)]);
    let (text, kind) = app.status_line_message().expect("result line");
    assert_eq!(kind, StatusKind::Done);
    assert_eq!(text, "Marked Pawn Ignored in CurseForge.");
    // CurseForge's state is read again: Pawn is Ignored now.
    assert!(!app.curseforge.manages("Pawn"));
    assert!(
        record
            .with_file_name("AddonGameInstance.json.lemonup-backup")
            .exists()
    );
}

#[test]
fn n_skips_the_question_and_changes_nothing() {
    let (mut app, _game, record) = marking_app(&["Pawn"]);
    let before = fs::read(&record).expect("record");
    link_two(&mut app);
    let status = app.status_line.text.clone();
    for no in [KeyCode::Char('n'), KeyCode::Esc] {
        app.curseforge_mark = Some(curseforge_mark::CurseForgeMark {
            names: vec!["Pawn".to_string()],
            folders: vec!["Pawn".to_string()],
        });
        assert!(!answer(&mut app, no).is_empty());
        assert_eq!(app.active_question(), None);
    }
    assert_eq!(fs::read(&record).expect("record"), before);
    assert_eq!(app.status_line.text, status);
}

#[test]
fn a_y_typed_before_the_question_showed_does_not_answer_it() {
    let (mut app, _game, record) = marking_app(&["Pawn"]);
    app.key_read_at = Some(Instant::now());
    std::thread::sleep(std::time::Duration::from_millis(2));
    link_two(&mut app);
    app.note_question_shown(Instant::now());
    assert!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('y')))
            .is_empty()
    );
    assert_eq!(flags(&record), [serde_json::json!(false)]);
}

#[test]
fn while_curseforge_runs_the_hint_says_to_quit_it_and_nothing_is_asked() {
    let (mut app, _game, record) = marking_app(&["Pawn"]);
    app.curseforge_running = open;
    link_two(&mut app);
    assert!(
        app.status_line.text.contains(
            "CurseForge also manages Pawn. Mark it Ignored there. Quit CurseForge to have \
             LemonUp do it."
        ),
        "{}",
        app.status_line.text
    );
    assert_eq!(app.active_question(), None);

    let (mut app, _game, _) = marking_app(&["WeakAurasOptions"]);
    app.curseforge_running = open;
    apply_all(&mut app, weakauras_installed());
    let (text, _) = app.status_line_message().expect("result line");
    assert_eq!(
        text,
        "WeakAuras is installed. CurseForge also manages it. Mark it Ignored there. Quit \
         CurseForge to have LemonUp do it."
    );
    assert_eq!(app.active_question(), None);
    assert_eq!(flags(&record), [serde_json::json!(false)]);
}

#[test]
fn after_an_install_the_question_names_the_addon_and_marks_its_folders() {
    let (mut app, _game, record) = marking_app(&["WeakAurasOptions"]);
    apply_all(&mut app, weakauras_installed());
    let (text, _) = app.status_line_message().expect("result line");
    assert_eq!(
        text,
        "WeakAuras is installed. CurseForge also manages it. Mark it Ignored there."
    );
    assert_eq!(
        app.active_question(),
        Some(Question::MarkIgnored {
            names: vec!["WeakAuras".to_string()]
        })
    );
    answer(&mut app, KeyCode::Char('y'));
    assert_eq!(flags(&record), [serde_json::json!(true)]);
    assert_eq!(
        app.status_line_message().expect("result line").0,
        "Marked WeakAuras Ignored in CurseForge."
    );

    // Where LemonUp can't mark (Windows, macOS), today's hint only.
    let (mut app, _game, _) = marking_app(&["WeakAurasOptions"]);
    app.curseforge_can_mark = false;
    apply_all(&mut app, weakauras_installed());
    assert_eq!(app.active_question(), None);
}

#[test]
fn a_record_that_cant_be_changed_says_so_and_keeps_the_hint() {
    let (mut app, _game, record) = marking_app(&["Pawn"]);
    link_two(&mut app);
    // A flag LemonUp can't place: it refuses to touch the record.
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&record).expect("record")).expect("json");
    value[0]["preferenceIsIgnored"] = serde_json::json!(false);
    let before = serde_json::to_vec(&value).expect("json");
    fs::write(&record, &before).expect("record");

    answer(&mut app, KeyCode::Char('y'));
    let (text, kind) = app.status_line_message().expect("result line");
    assert_eq!(kind, StatusKind::Problem);
    assert_eq!(text, "Couldn't change CurseForge's list. Nothing changed.");
    assert_eq!(fs::read(&record).expect("record"), before);
    assert!(app.curseforge.manages("Pawn"));
}

#[test]
fn a_question_naming_many_addons_fits_at_80x24() {
    let (mut app, _game, _) = marking_app(&["Pawn"]);
    let names = (0..40)
        .map(|index| format!("Addon{index:02}"))
        .collect::<Vec<_>>();
    app.curseforge_mark = Some(curseforge_mark::CurseForgeMark {
        names: names.clone(),
        folders: names,
    });
    let rows = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        rows.contains("Mark these 40 addons Ignored in CurseForge?"),
        "{rows}"
    );
    assert!(rows.contains("Addon00, Addon01"), "{rows}");
    assert!(rows.contains("more"), "{rows}");
    assert!(rows.contains("y  mark"), "{rows}");
    assert!(rows.contains("n  skip"), "{rows}");
}

/// Details on Second, whose own folder CurseForge manages.
fn details_on_second(app: &mut App) {
    let index = (0..app.dashboard.rows.len())
        .find(|index| {
            app.dashboard.list_state.select(Some(*index));
            app.dashboard
                .selected_item()
                .is_some_and(|item| item.folder == "Second")
        })
        .expect("Second");
    app.dashboard.list_state.select(Some(index));
    app.apply(AppAction::SetInspectOverlay(true));
}

#[test]
fn m_in_details_marks_the_addon_where_curseforge_manages_it() {
    let (mut app, _game, record) = marking_app(&["second_config"]);
    details_on_second(&mut app);
    let line = |app: &App, width| {
        app.key_table(width)
            .iter()
            .any(|hint| hint.key == "m" && hint.label == "mark Ignored in CurseForge")
    };
    assert!(line(&app, u16::MAX));

    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('m'))),
        vec![AppMessage::CurseForge(CurseForgeMessage::MarkFromDetails)]
    );
    apply_all(
        &mut app,
        AppMessage::CurseForge(CurseForgeMessage::MarkFromDetails),
    );
    assert_eq!(
        app.active_question(),
        Some(Question::MarkIgnored {
            names: vec!["Second".to_string()]
        })
    );
    answer(&mut app, KeyCode::Char('y'));
    assert_eq!(flags(&record), [serde_json::json!(true)]);
    assert!(
        app.status_line
            .text
            .contains("Marked Second Ignored in CurseForge."),
        "{}",
        app.status_line.text
    );
    let rows = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        rows.contains("Marked Second Ignored in CurseForge."),
        "{rows}"
    );
    // Ignored now: `m` does nothing and the key line doesn't offer it.
    assert!(!line(&app, u16::MAX));
    assert!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('m')))
            .is_empty()
    );
}

#[test]
fn m_in_details_while_curseforge_runs_says_to_quit_it() {
    let (mut app, _game, record) = marking_app(&["second_config"]);
    app.curseforge_running = open;
    details_on_second(&mut app);
    apply_all(
        &mut app,
        AppMessage::CurseForge(CurseForgeMessage::MarkFromDetails),
    );
    assert_eq!(app.active_question(), None);
    assert!(
        app.status_line
            .text
            .contains("CurseForge is running. Quit it, then press m again."),
        "{}",
        app.status_line.text
    );
    assert_eq!(flags(&record), [serde_json::json!(false)]);

    // It started while the question was open: nothing changes.
    app.curseforge_running = closed;
    apply_all(
        &mut app,
        AppMessage::CurseForge(CurseForgeMessage::MarkFromDetails),
    );
    app.curseforge_running = open;
    answer(&mut app, KeyCode::Char('y'));
    assert_eq!(flags(&record), [serde_json::json!(false)]);
    assert!(
        app.status_line
            .text
            .contains("CurseForge is running. Nothing changed. Quit it, then press m in Details."),
        "{}",
        app.status_line.text
    );
}

#[test]
fn m_in_details_does_nothing_where_it_cant_mark() {
    // CurseForge doesn't manage Second.
    let (mut app, _game, _) = marking_app(&["Other"]);
    details_on_second(&mut app);
    assert!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('m')))
            .is_empty()
    );
    assert!(!app.key_table(u16::MAX).iter().any(|hint| hint.key == "m"));

    // Managed, but LemonUp can't change CurseForge's list on this system.
    let (mut app, _game, _) = marking_app(&["second_config"]);
    app.curseforge_can_mark = false;
    details_on_second(&mut app);
    assert!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('m')))
            .is_empty()
    );
    assert!(!app.key_table(u16::MAX).iter().any(|hint| hint.key == "m"));
}

#[test]
fn the_details_key_line_offers_m_at_80x24_and_wide() {
    let (mut app, _game, _) = marking_app(&["second_config"]);
    details_on_second(&mut app);
    for (width, height, want) in [
        (80, 24, "m mark Ignored"),
        (160, 40, "m mark Ignored in CurseForge"),
    ] {
        let rows = render_rows(&mut app, width, height);
        println!("--- Details {width}x{height}\n{}", rows.join("\n"));
        assert!(
            rows.iter().any(|row| row.contains(want)),
            "{width}x{height}"
        );
    }

    // Docked beside the list, `m` works the same.
    let (mut app, _game, _) = marking_app(&["second_config"]);
    app.dashboard.detail_mode = DetailMode::Overview;
    details_on_second(&mut app);
    let rows = render_rows(&mut app, 200, 40);
    if app.details_docked() {
        println!("--- Docked 200x40\n{}", rows.join("\n"));
        assert!(rows.iter().any(|row| row.contains("m mark Ignored")));
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('m'))),
            vec![AppMessage::CurseForge(CurseForgeMessage::MarkFromDetails)]
        );
    }
}

#[test]
fn the_question_renders_at_80x24_and_wide() {
    let (mut app, _game, _) = marking_app(&["Pawn"]);
    app.curseforge_mark = Some(curseforge_mark::CurseForgeMark {
        names: vec!["Pawn".into(), "Clique".into(), "Spy".into()],
        folders: vec!["Pawn".into()],
    });
    for (width, height) in [(80, 24), (160, 40)] {
        let rows = render_rows(&mut app, width, height);
        println!("--- Question {width}x{height}\n{}", rows.join("\n"));
        let rows = rows.join("\n");
        assert!(rows.contains("Mark these 3 addons Ignored in CurseForge?"));
        assert!(rows.contains("Pawn, Clique, Spy"));
        assert!(rows.contains("y  mark"));
    }
}

#[test]
fn after_an_install_from_search_the_sheet_says_what_marking_did() {
    let (mut app, _game, record) = marking_app(&["WeakAurasOptions"]);
    app.apply(AppAction::SetDetailMode(DetailMode::Search));
    apply_all(&mut app, weakauras_installed());
    assert!(app.active_question().is_some());
    answer(&mut app, KeyCode::Char('y'));
    assert_eq!(flags(&record), [serde_json::json!(true)]);
    for (width, height) in [(80, 24), (160, 40)] {
        let rows = render_rows(&mut app, width, height).join("\n");
        assert!(rows.contains("WeakAuras is installed."), "{rows}");
        assert!(
            rows.contains("Marked WeakAuras Ignored in CurseForge."),
            "{rows}"
        );
    }
}
