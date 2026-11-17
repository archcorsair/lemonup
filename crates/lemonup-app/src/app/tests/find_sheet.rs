//! The Find your addons online sheet (`F`, and `f` on one addon). Lookups are
//! injected as finished reports; nothing here asks the network.

use super::*;
use crate::app::dialog::Question;
use crate::app::find_sheet::{
    FindFocus, FindLinkOutcome, FindOutcome, FindResults, FindStep, link_found, search_name,
};
use crate::app::{FindAction, FindMessage, FindTaskEvent};
use crate::find::{LocalNames, SourceProblem};
use lemonup_core::GameFlavor;
use lemonup_core::find::{
    FindInputs, FindLabel, FindReport, FindResult, FindSource, FoundAddon, LocalFolder,
    SourceAnswer, SourceLookups, TocIds, TukuiPackage,
};

fn found(label: FindLabel, source: FindSource, name: &str, folders: &[&str]) -> FoundAddon {
    FoundAddon {
        label,
        source,
        source_id: format!("{name}-id"),
        source_url: format!("https://example.invalid/{name}"),
        name: name.to_string(),
        author: None,
        parent_folder: folders[0].to_string(),
        folders: folders.iter().map(|folder| folder.to_string()).collect(),
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

fn result(label: FindLabel, name: &str, candidates: Vec<FoundAddon>) -> FindResult {
    FindResult {
        label,
        name: name.to_string(),
        folder: name.to_string(),
        folders: vec![name.to_string()],
        candidates,
        curseforge_id: None,
        not_checked_by: Vec::new(),
    }
}

/// One result of every kind, in an order unlike the report's.
fn report() -> FindReport {
    let mut blocked = found(FindLabel::Exact, FindSource::Wago, "Blocked", &["Blocked"]);
    blocked.blocked = vec![lemonup_core::find::BlockedFolder {
        folder: "Blocked".to_string(),
        reason: lemonup_core::find::BlockReason::DeveloperCopy,
    }];
    let mut questie = found(
        FindLabel::Partial,
        FindSource::Wago,
        "Questie",
        &["Questie", "QuestieDB"],
    );
    questie.differing = vec!["QuestieDB".to_string()];
    let mut not_checked = result(FindLabel::NotChecked, "Elephant", Vec::new());
    not_checked.not_checked_by = vec![FindSource::Tukui];
    FindReport {
        results: vec![
            result(FindLabel::NotFound, "Lonely", Vec::new()),
            result(
                FindLabel::Likely,
                "Pawn",
                vec![found(
                    FindLabel::Likely,
                    FindSource::WowInterface,
                    "Pawn",
                    &["Pawn"],
                )],
            ),
            result(
                FindLabel::Exact,
                "Clique",
                vec![found(
                    FindLabel::Exact,
                    FindSource::WowInterface,
                    "Clique",
                    &["Clique"],
                )],
            ),
            result(FindLabel::Partial, "Questie", vec![questie]),
            result(
                FindLabel::ChooseOne,
                "QuestieDev",
                vec![
                    found(
                        FindLabel::Likely,
                        FindSource::Wago,
                        "QuestieDev",
                        &["QuestieDev"],
                    ),
                    found(
                        FindLabel::Exact,
                        FindSource::WowInterface,
                        "QuestieDev",
                        &["QuestieDev"],
                    ),
                ],
            ),
            result(FindLabel::Exact, "Blocked", vec![blocked]),
            not_checked,
            result(FindLabel::CurseForgeOnly, "Leatrix", Vec::new()),
            result(FindLabel::DeveloperCopy, "WowButler", Vec::new()),
        ],
        failed: vec![FindSource::Tukui],
        skipped: Vec::new(),
    }
}

fn outcome(report: FindReport) -> FindOutcome {
    FindOutcome {
        report,
        problems: vec![SourceProblem {
            source: FindSource::Tukui,
            error: SourceError::new(SourceErrorKind::NoConnection, "offline"),
        }],
        names: LocalNames::default(),
    }
}

fn key(app: &mut App, code: KeyCode) {
    for message in app.messages_for_key(KeyEvent::from(code)) {
        apply_all(app, message);
    }
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    render_rows(app, width, height).join("\n")
}

fn open_find(app: &mut App) {
    app.effective_addon_dir = Some(std::env::temp_dir().join("lemonup-find-test/AddOns"));
    apply_all(app, AppMessage::Find(FindMessage::Open));
}

fn set_results(app: &mut App, report: FindReport) {
    app.apply(AppAction::Find(FindAction::SetStep(FindStep::Results(
        FindResults::new(outcome(report)),
    ))));
}

/// The sheet with every kind of result, the cursor on the first match.
pub(super) fn open_find_results(app: &mut App) {
    open_find(app);
    set_results(app, report());
}

/// The results with the cursor on the action, something picked.
pub(super) fn open_find_action(app: &mut App) {
    open_find_results(app);
    apply_all(app, AppMessage::Find(FindMessage::Last));
}

/// The link question over a folder list too long to show at once.
pub(super) fn open_find_question(app: &mut App) {
    open_find_action(app);
    apply_all(app, AppMessage::Find(FindMessage::Enter));
    app.find.question_max.set(3);
}

fn results(app: &App) -> &FindResults {
    app.find.results().expect("results")
}

#[test]
fn capital_f_opens_the_sheet_and_says_what_goes_to_wago() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('F'))),
        vec![AppMessage::Find(FindMessage::Open)]
    );
    open_find(&mut app);
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Find));
    assert_eq!(app.find.step, Some(FindStep::Start));
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Find your addons online"), "{shown}");
    assert!(
        shown.contains("LemonUp looks up 1 Not linked addon"),
        "{shown}"
    );
    assert!(
        shown.contains("Sends folder names and file hashes to Wago.io."),
        "{shown}"
    );
    assert!(!shown.contains("A Wago.io key finds more"), "{shown}");

    app.wago_api_key = None;
    let shown = screen(&mut app, 120, 30);
    assert!(
        shown.contains("A Wago.io key finds more of your addons. Add one in Settings."),
        "{shown}"
    );
    assert!(!shown.contains("Sends folder names"), "{shown}");

    // Esc closes it; nothing ran.
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.find.step, None);
}

#[test]
fn capital_f_with_a_selection_looks_up_only_the_selected_not_linked_addons() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(vec![
        AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
        AddonRecord::new("Beta", "Beta", SourceKind::Manual),
        AddonRecord::new("Gamma", "Gamma", SourceKind::Wago),
    ]);
    let select = |app: &mut App, folder: &str| {
        let index = app
            .dashboard
            .rows
            .iter()
            .position(|row| row.folder == folder)
            .expect("row");
        app.dashboard.list_state.select(Some(index));
        apply_all(app, AppMessage::DashboardToggleSelected);
    };
    select(&mut app, "Gamma");
    let actions = app.update(AppMessage::Find(FindMessage::Open));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, AppAction::SetStatus(status)
            if status.text.contains("None of the selected addons is Not linked."))),
        "{actions:?}"
    );
    select(&mut app, "Alpha");
    open_find(&mut app);
    assert_eq!(app.find.scope, Some(vec!["Alpha".to_string()]));
    assert!(screen(&mut app, 120, 30).contains("looks up 1 Not linked addon"));

    // With none selected, every Not linked addon.
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    assert_eq!(app.find.scope, None);
}

#[test]
fn esc_while_looking_changes_nothing_and_a_late_answer_is_dropped() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    // As enter on Start leaves it, without the network.
    app.find.generation = 1;
    app.find.step = Some(FindStep::Looking {
        folders: 3,
        asking: Vec::new(),
    });
    let before = app.dashboard.items.clone();
    assert!(screen(&mut app, 120, 30).contains("Reading 3 folders"));
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Read {
            generation: 1,
            folders: 3,
            asking: vec![
                FindSource::Wago,
                FindSource::WowInterface,
                FindSource::Tukui,
            ],
        })),
    );
    let shown = screen(&mut app, 120, 30);
    assert!(
        shown.contains("Asking Wago.io, WoWInterface, TukUI"),
        "{shown}"
    );

    key(&mut app, KeyCode::Esc);
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.find.step, None);
    for event in [
        FindTaskEvent::Read {
            generation: 1,
            folders: 3,
            asking: Vec::new(),
        },
        FindTaskEvent::Looked {
            generation: 1,
            result: Ok(Box::new(outcome(report()))),
        },
    ] {
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::Find(event)),
        );
    }
    assert_eq!(app.find.step, None);
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.dashboard.items, before);
}

#[test]
fn results_are_grouped_like_the_report_and_only_ready_rows_start_selected() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    app.find.generation = 4;
    app.find.step = Some(FindStep::Looking {
        folders: 9,
        asking: Vec::new(),
    });
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Looked {
            generation: 4,
            result: Ok(Box::new(outcome(report()))),
        })),
    );
    let picks = &results(&app).picks;
    // Only Clique (Exact, linkable) is selected.
    assert_eq!(picks[2], Some(0));
    for (index, pick) in picks.iter().enumerate() {
        if index != 2 {
            assert_eq!(*pick, None, "{index}");
        }
    }
    let shown = screen(&mut app, 138, 80);
    let order = [
        "Ready to link (1)",
        "Can't link yet (1)",
        "Likely matches (1)",
        "Partial matches (1)",
        "Choose one (1)",
        "Not checked (1)",
        "On CurseForge only",
        "Not found (1)",
        "Skipped, developer copy (1)",
    ];
    let mut at = 0;
    for heading in order {
        let found = shown[at..]
            .find(heading)
            .unwrap_or_else(|| panic!("{heading} after {at}:\n{shown}"));
        at += found;
    }
    assert!(shown.contains("TukUI: "), "{shown}");
    assert!(shown.contains("files differ in QuestieDB"), "{shown}");
    assert!(shown.contains("Replace with the source's copy"), "{shown}");
    let line = |name: &str| {
        shown
            .lines()
            .find(|line| line.contains(name) && line.contains("1.0"))
            .unwrap_or_else(|| panic!("{name}:\n{shown}"))
            .to_string()
    };
    assert!(line("Clique").contains("● Clique"), "{shown}");
    assert!(line("Pawn").contains("○ Pawn"), "{shown}");
    assert!(shown.contains("Link 1 addon"), "{shown}");
}

#[test]
fn space_selects_and_unselects_a_match() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    assert_eq!(results(&app).cursor, FindFocus::Result(2));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(results(&app).picks[2], None);
    assert!(screen(&mut app, 138, 60).contains("Nothing selected"));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(results(&app).picks[2], Some(0));
    // The blocked match can't be selected: the cursor skips it.
    key(&mut app, KeyCode::Down);
    assert_eq!(results(&app).cursor, FindFocus::Result(1));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(results(&app).picks[1], Some(0));
    assert!(screen(&mut app, 138, 60).contains("Link 2 addons"));
}

#[test]
fn enter_on_a_choose_one_row_picks_each_candidate_then_none() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(results(&app).cursor, FindFocus::Result(4));
    assert_eq!(results(&app).picks[4], None);
    // Space never picks for you here.
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(results(&app).picks[4], None);
    key(&mut app, KeyCode::Enter);
    assert_eq!(results(&app).picks[4], Some(0));
    key(&mut app, KeyCode::Enter);
    assert_eq!(results(&app).picks[4], Some(1));
    key(&mut app, KeyCode::Enter);
    assert_eq!(results(&app).picks[4], None);
}

#[test]
fn a_partial_match_asks_first_then_replaces_through_the_wago_install() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    // Only Questie, as a replacement.
    key(&mut app, KeyCode::Char(' '));
    for _ in 0..2 {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(results(&app).cursor, FindFocus::Result(3));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::End);
    assert!(screen(&mut app, 138, 60).contains("Replace 1 addon"));
    key(&mut app, KeyCode::Enter);
    let Some(Question::LinkFound {
        links,
        replaces,
        replaced,
        ..
    }) = app.active_question()
    else {
        panic!("no question: {:?}", app.active_question());
    };
    assert_eq!((links, replaces), (0, 1));
    assert_eq!(
        replaced,
        vec!["Questie".to_string(), "QuestieDB".to_string()]
    );
    let shown = screen(&mut app, 80, 24);
    assert!(shown.contains("Replace 1 addon?"), "{shown}");
    assert!(shown.contains("QuestieDB"), "{shown}");

    // Nothing runs before the yes.
    assert!(app.find_replacement_actions().is_empty());
    app.note_question_shown(Instant::now());
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(app.active_question(), None);
    assert_eq!(app.shell_ui.overlay.active, None);
    let actions = app.find_replacement_actions();
    assert!(
        actions.iter().any(|action| matches!(action,
            AppAction::StartWagoInstall { replace_folders: Some(folders), request, .. }
                if *folders == vec!["Questie".to_string(), "QuestieDB".to_string()]
                    && request.target == "https://example.invalid/Questie")),
        "{actions:?}"
    );
    // One at a time: not while that install runs.
    for action in actions {
        if !matches!(action, AppAction::StartWagoInstall { .. }) {
            app.apply(action);
        }
    }
    assert!(app.find.replacements.is_empty());
}

#[test]
fn the_link_question_names_every_folder_scrolls_and_only_y_confirms() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    let folders = (0..30).map(|n| format!("Pack_{n:02}")).collect::<Vec<_>>();
    let refs = folders.iter().map(String::as_str).collect::<Vec<_>>();
    let mut report = report();
    report.results[2] = result(
        FindLabel::Exact,
        "Pack_00",
        vec![found(FindLabel::Exact, FindSource::Wago, "Pack", &refs)],
    );
    open_find(&mut app);
    set_results(&mut app, report);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Enter);
    let Some(Question::LinkFound { taken, links, .. }) = app.active_question() else {
        panic!("no question");
    };
    assert_eq!(links, 1);
    assert_eq!(taken, folders);
    app.note_question_shown(Instant::now());

    // Every folder shows as the list scrolls; never "and N more".
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        let shown = screen(&mut app, 80, 24);
        assert!(shown.contains("Link 1 addon?"), "{shown}");
        assert!(!shown.contains("more\n") || shown.contains("↓"), "{shown}");
        assert!(!shown.contains("and 2"), "{shown}");
        for folder in &folders {
            if shown.contains(folder.as_str()) {
                seen.insert(folder.clone());
            }
        }
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(seen.len(), folders.len());
    assert!(app.key_table(80).iter().any(|hint| hint.label == "scroll"));

    // Enter never says yes; n says no and links nothing.
    key(&mut app, KeyCode::Enter);
    assert!(app.active_question().is_some());
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
        vec![AppMessage::Find(FindMessage::ConfirmLink)]
    );
    key(&mut app, KeyCode::Char('n'));
    assert!(app.active_question().is_none());
    assert!(!app.find.linking);
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Find));
}

fn write_addon(addons: &std::path::Path, folder: &str, version: &str) {
    let dir = addons.join(folder);
    std::fs::create_dir_all(&dir).expect("addon dir");
    std::fs::write(
        dir.join(format!("{folder}.toc")),
        format!("## Interface: 110005\n## Title: {folder}\n## Version: {version}\n"),
    )
    .expect("toc");
}

#[test]
fn linking_writes_rows_and_the_result_line_reads_them_from_the_database() {
    let temp = tempdir().expect("tempdir");
    let addons = temp
        .path()
        .join("_retail_")
        .join("Interface")
        .join("AddOns");
    write_addon(&addons, "Clique", "1.0");
    write_addon(&addons, "Pawn", "2.0");
    let db = temp.path().join("state.sqlite");
    let mut pawn = found(
        FindLabel::Likely,
        FindSource::WowInterface,
        "Pawn",
        &["Pawn"],
    );
    pawn.matched_version = Some("2.0".to_string());
    pawn.latest_version = Some("2.1".to_string());
    let clique = found(FindLabel::Exact, FindSource::Wago, "Clique", &["Clique"]);

    let outcome = link_found(&db, &addons, &[clique, pawn]).expect("linked");
    let stored = StateDatabase::read_addons(&db).expect("rows");
    let row = |folder: &str| {
        stored
            .iter()
            .find(|row| row.folder == folder)
            .unwrap_or_else(|| panic!("{folder}: {stored:?}"))
            .clone()
    };
    assert_eq!(row("Clique").source, SourceKind::Wago);
    assert_eq!(row("Pawn").source, SourceKind::WowInterface);
    assert_eq!(outcome.rows.len(), 2);
    assert!(outcome.unconfirmed.is_empty());

    let mut app = app_for_tests(ShellMode::Dashboard);
    app.find.linking = true;
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Ok(Box::new(
            outcome,
        ))))),
    );
    assert!(!app.find.linking);
    assert!(
        app.status_line
            .text
            .contains("Linked 2 addons: 1 up to date, 1 with an update ready (Pawn)."),
        "{}",
        app.status_line.text
    );
    assert!(
        app.dashboard
            .items
            .iter()
            .any(|item| item.folder == "Pawn" && item.source == SourceKind::WowInterface)
    );

    // A failure says so and links nothing.
    let mut app = app_for_tests(ShellMode::Dashboard);
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Err(
            "the folder Clique belongs to Clique".to_string(),
        )))),
    );
    assert!(
        app.status_line.text.contains("Nothing was linked"),
        "{}",
        app.status_line.text
    );

    // Only what the database holds counts as linked.
    let mut app = app_for_tests(ShellMode::Dashboard);
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Ok(Box::new(
            FindLinkOutcome {
                rows: Vec::new(),
                unconfirmed: vec!["Pawn".to_string()],
                sync: None,
            },
        ))))),
    );
    assert!(
        app.status_line
            .text
            .contains("couldn't confirm that Pawn was linked"),
        "{}",
        app.status_line.text
    );
}

#[test]
fn the_report_comes_from_the_core_matcher_with_injected_lookups() {
    let inputs = FindInputs {
        flavor: GameFlavor::Retail,
        folders: vec![LocalFolder {
            folder: "ElvUI".to_string(),
            developer_copy: None,
            fingerprint: None,
            title: Some("ElvUI".to_string()),
            version: Some("13.80".to_string()),
            required_deps: Vec::new(),
            ids: TocIds::default(),
        }],
    };
    let rows = vec![AddonRecord::new("ElvUI", "ElvUI", SourceKind::Manual)];
    let lookups = crate::find::Lookups {
        lookups: SourceLookups {
            wago: SourceAnswer::Skipped,
            wowinterface: SourceAnswer::Found(Vec::new()),
            tukui: SourceAnswer::Found(vec![TukuiPackage {
                id: "-2".to_string(),
                name: "ElvUI".to_string(),
                version: "13.80".to_string(),
                web_url: "https://tukui.org/elvui".to_string(),
                author: None,
                directories: vec!["ElvUI".to_string()],
            }]),
        },
        problems: Vec::new(),
    };
    let outcome = crate::app::find_sheet::find_outcome(&inputs, &rows, None, lookups);
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    app.find.generation = 2;
    app.find.step = Some(FindStep::Looking {
        folders: 1,
        asking: Vec::new(),
    });
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Looked {
            generation: 2,
            result: Ok(Box::new(outcome)),
        })),
    );
    let shown = screen(&mut app, 138, 36);
    assert!(shown.contains("Ready to link (1)"), "{shown}");
    assert!(shown.contains("A Wago.io key finds more"), "{shown}");
    assert_eq!(results(&app).picks, vec![Some(0)]);
}

#[test]
fn search_name_drops_tags_and_color_codes() {
    assert_eq!(
        search_name("<DBM Core> |cffff0000Main|r  Core"),
        "Main Core"
    );
    assert_eq!(search_name("Mythic Dungeon Tools"), "Mythic Dungeon Tools");
    // A name that is only a tag keeps its words.
    assert_eq!(search_name("<Tag>"), "<Tag>");
}

#[test]
fn f_checks_one_addon_first_and_offers_a_name_search_when_nothing_is_found() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(vec![AddonRecord::new(
        "<DBM Core> |cffff0000Main|r Core",
        "DBM-Core",
        SourceKind::Manual,
    )]);
    app.dashboard.list_state.select(Some(0));
    app.effective_addon_dir = Some(std::env::temp_dir().join("lemonup-find-test/AddOns"));
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Find));
    assert_eq!(app.find.scope, Some(vec!["DBM-Core".to_string()]));
    assert_eq!(app.find.step, Some(FindStep::Start));
    assert!(screen(&mut app, 120, 30).contains("Sends folder names and file hashes"));

    // Found by its files: the result, no name search.
    let found_report = FindReport {
        results: vec![result(
            FindLabel::Exact,
            "DBM-Core",
            vec![found(
                FindLabel::Exact,
                FindSource::Wago,
                "Deadly Boss Mods",
                &["DBM-Core"],
            )],
        )],
        failed: Vec::new(),
        skipped: Vec::new(),
    };
    set_results(&mut app, found_report);
    assert!(!app.find_offers_search());
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Ready to link (1)"), "{shown}");
    assert!(!shown.contains("Search Wago.io by name"), "{shown}");

    // Found nothing: enter searches Wago.io for the cleaned name.
    set_results(
        &mut app,
        FindReport {
            results: vec![result(FindLabel::NotFound, "DBM-Core", Vec::new())],
            failed: Vec::new(),
            skipped: Vec::new(),
        },
    );
    assert!(app.find_offers_search());
    assert_eq!(results(&app).cursor, FindFocus::Action);
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Search Wago.io by name"), "{shown}");
    let actions = app.update(AppMessage::Find(FindMessage::Enter));
    assert!(
        actions.contains(&AppAction::StartWagoSearch {
            query: "Main Core".to_string(),
            api_key: "test-wago-key".to_string(),
        }),
        "{actions:?}"
    );
    for action in actions {
        if !matches!(action, AppAction::StartWagoSearch { .. }) {
            app.apply(action);
        }
    }
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
    assert_eq!(app.search_pane.query, "Main Core");
}

#[test]
fn the_welcome_card_offers_capital_f_next_to_p_at_80x24() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(vec![
        AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
        AddonRecord::new("Beta", "Beta", SourceKind::Manual),
    ]);
    app.games.games = two_test_games();
    app.games.games.truncate(1);
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        assert!(
            shown.contains("Press F to find your addons online."),
            "{shown}"
        );
        assert!(shown.contains("Press p to add another game."), "{shown}");
        assert!(shown.contains("d hide this"), "{shown}");
        assert!(shown.contains("Alpha"), "{shown}");
    }
    assert!(app.key_table(138).iter().any(|hint| hint.key == "F"));
}

#[test]
fn the_find_sheet_keeps_its_size_and_place_in_every_step() {
    let open = |app: &mut App| open_find(app);
    let steps: [super::sheets::SheetStep<'_>; 5] = [
        ("Reading 4 folders", &|app: &mut App| {
            app.find.step = Some(FindStep::Looking {
                folders: 4,
                asking: Vec::new(),
            })
        }),
        ("Asking Wago.io", &|app: &mut App| {
            app.find.step = Some(FindStep::Looking {
                folders: 4,
                asking: vec![FindSource::Wago, FindSource::WowInterface],
            })
        }),
        ("Ready to link", &|app: &mut App| set_results(app, report())),
        ("Nothing selected", &|app: &mut App| {
            key(app, KeyCode::Char(' '));
        }),
        ("Nothing was changed", &|app: &mut App| {
            app.find.step = Some(FindStep::Failed("It broke.".to_string()))
        }),
    ];
    super::sheets::assert_sheet_holds_still(&[(138, 36), (120, 30), (80, 24)], &open, &steps);
}

#[test]
fn switching_games_waits_while_find_links() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.games.games = two_test_games();
    app.find.linking = true;
    // Updates, deletes and installs wait too.
    assert!(app.install_running_status().is_some());
    assert!(app.install_busy_status().is_some());
    key(&mut app, KeyCode::Char('p'));
    assert_ne!(app.shell_ui.overlay.active, Some(OverlayKind::Games));
    assert!(
        app.status_line
            .text
            .contains("Wait for the find to finish first."),
        "{}",
        app.status_line.text
    );
    assert!(!app.key_table(138).iter().any(|hint| hint.key == "p"));

    // A replacement still to run counts too, and so does an install.
    app.find.linking = false;
    app.find.replacements = vec![found(FindLabel::Partial, FindSource::Wago, "Q", &["Q"])].into();
    key(&mut app, KeyCode::Char('p'));
    assert_ne!(app.shell_ui.overlay.active, Some(OverlayKind::Games));
    app.find.replacements.clear();
    key(&mut app, KeyCode::Char('p'));
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Games));
}
