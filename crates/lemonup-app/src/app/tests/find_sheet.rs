//! The Find your addons online sheet (`F`, and `f` on one addon). Lookups are
//! injected as finished reports; nothing here asks the network.

use super::*;
use crate::app::dialog::Question;
use crate::app::find_sheet::{
    FindFocus, FindLinkOutcome, FindOutcome, FindResults, FindStep, link_found, read_find_inputs,
    search_name,
};
use crate::app::{FindAction, FindMessage, FindTaskEvent};
use crate::find::{LocalNames, SourceProblem};
use lemonup_core::GameFlavor;
use lemonup_core::find::{
    FindChoice, FindInputs, FindLabel, FindReport, FindResult, FindSource, FoundAddon, LocalFolder,
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

/// The Start step, every source checked.
pub(super) fn open_find_start(app: &mut App) {
    open_find(app);
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
    assert!(shown.contains("Look for 1 Not linked addon on"), "{shown}");
    assert!(
        shown.contains("Sends folder names and file hashes to Wago.io."),
        "{shown}"
    );
    assert!(!shown.contains("Add a Wago.io key"), "{shown}");

    app.wago_api_key = None;
    let shown = screen(&mut app, 120, 30);
    assert!(
        shown.contains("Add a Wago.io key in Settings to look there too."),
        "{shown}"
    );
    assert!(!shown.contains("Sends folder names"), "{shown}");

    // Help lists F on the list and beside docked Details.
    for context in [HelpContext::Overview, HelpContext::DetailsDocked] {
        assert!(
            crate::app::keymap::help_page(context)
                .sections()
                .flat_map(|section| section.rows.iter())
                .any(|row| row.keys == [KeyEvent::from(KeyCode::Char('F'))]),
            "{context:?}"
        );
    }

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
    assert!(screen(&mut app, 120, 30).contains("Look for 1 Not linked addon on"));

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

    // A new lookup started since: the old answer still lands nowhere.
    open_find(&mut app);
    app.find.generation += 1;
    app.find.step = Some(FindStep::Looking {
        folders: 3,
        asking: Vec::new(),
    });
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Looked {
            generation: 1,
            result: Ok(Box::new(outcome(report()))),
        })),
    );
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Read {
            generation: 1,
            folders: 9,
            asking: vec![FindSource::Tukui],
        })),
    );
    assert_eq!(
        app.find.step,
        Some(FindStep::Looking {
            folders: 3,
            asking: Vec::new(),
        })
    );
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
fn search_name_drops_tag_brackets_and_color_codes_but_keeps_the_words() {
    assert_eq!(
        search_name("<DBM Core> |cffff0000Main|r  Core"),
        "DBM Core Main Core"
    );
    assert_eq!(search_name("Mythic Dungeon Tools"), "Mythic Dungeon Tools");
    assert_eq!(search_name("<Tag>"), "Tag");
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
            query: "DBM Core Main Core".to_string(),
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
    assert_eq!(app.search_pane.query, "DBM Core Main Core");
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

/// The sheet with Questie (a partial match) picked as the only change, its
/// question answered yes and its Wago.io install started.
fn replace_questie(app: &mut App) {
    open_find_results(app);
    key(app, KeyCode::Char(' ')); // Clique off
    for _ in 0..2 {
        key(app, KeyCode::Down);
    }
    key(app, KeyCode::Char(' '));
    key(app, KeyCode::End);
    key(app, KeyCode::Enter);
    app.note_question_shown(Instant::now());
    key(app, KeyCode::Char('y'));
    let actions = app.find_replacement_actions();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, AppAction::StartWagoInstall { .. })),
        "{actions:?}"
    );
    for action in actions {
        if !matches!(action, AppAction::StartWagoInstall { .. }) {
            app.apply(action);
        }
    }
}

#[test]
fn a_failed_replacement_says_which_addon_and_stops_the_rest() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    replace_questie(&mut app);
    assert!(app.wago_install_in_progress);
    assert!(app.status_line.text.contains("Replacing Questie"));
    // Another replacement waits behind it.
    app.find.replacements.push_back(found(
        FindLabel::Partial,
        FindSource::Wago,
        "Details",
        &["Details"],
    ));

    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::WagoInstallFinished(Err(SourceError::new(
            SourceErrorKind::NoConnection,
            "offline",
        )))),
    );
    let text = app.status_line.text.clone();
    assert_eq!(app.status_line.kind, StatusKind::Problem, "{text}");
    assert!(
        text.contains("Questie wasn't replaced: LemonUp couldn't reach Wago.io."),
        "{text}"
    );
    assert!(text.contains("Its folders weren't changed."), "{text}");
    assert!(
        text.contains("LemonUp stopped there, so Details wasn't replaced either."),
        "{text}"
    );
    assert!(!text.contains("Replacing"), "{text}");
    assert!(!app.wago_install_in_progress);
    // Nothing runs after it, and the Search pane keeps no error of it.
    assert!(app.find.replacements.is_empty());
    assert!(app.find_replacement_actions().is_empty());
    assert!(
        !matches!(
            app.search_pane.install_state,
            SearchInstallState::Error { .. }
        ),
        "{:?}",
        app.search_pane.install_state
    );
    let shown = screen(&mut app, 80, 24);
    assert!(shown.contains("Questie wasn't replaced"), "{shown}");
}

#[test]
fn a_failed_link_stops_the_replacements_and_says_so() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    // Clique to link, Questie to replace.
    for _ in 0..2 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(results(&app).counts(), (1, 1));
    // As yes leaves it, without running the link.
    app.apply(AppAction::Find(FindAction::Close));
    app.find.linking = true;
    app.find.replacements = vec![found(
        FindLabel::Partial,
        FindSource::Wago,
        "Questie",
        &["Questie"],
    )]
    .into();

    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Err(
            "LemonUp couldn't read your AddOns folder (boom)".to_string(),
        )))),
    );
    assert!(!app.find.linking);
    assert!(app.find.replacements.is_empty());
    assert!(
        !app.find_replacement_actions()
            .iter()
            .any(|action| matches!(action, AppAction::StartWagoInstall { .. }))
    );
    assert_eq!(app.status_line.kind, StatusKind::Problem);
    assert!(
        app.status_line.text.contains(
            "| Nothing was linked or replaced: LemonUp couldn't read your AddOns folder (boom). |"
        ),
        "{}",
        app.status_line.text
    );
}

#[test]
fn a_link_that_cant_read_the_folder_says_nothing_was_linked_once() {
    let temp = tempdir().expect("tempdir");
    let addons = temp
        .path()
        .join("_retail_")
        .join("Interface")
        .join("AddOns");
    let db = temp.path().join("state.sqlite");
    let clique = found(FindLabel::Exact, FindSource::Wago, "Clique", &["Clique"]);
    let error = link_found(&db, &addons, &[clique]).expect_err("no AddOns folder");
    assert!(
        error.starts_with("LemonUp couldn't read your AddOns folder"),
        "{error}"
    );
    assert!(!error.contains("nothing was linked"), "{error}");

    let mut app = app_for_tests(ShellMode::Dashboard);
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Linked(Err(error)))),
    );
    let text = app.status_line.text.to_lowercase();
    assert_eq!(text.matches("nothing was linked").count(), 1, "{text}");
}

#[test]
fn start_waits_for_a_job_that_changes_the_addons() {
    for (busy, note) in [
        (
            (|app: &mut App| app.dashboard.update_in_progress = true) as fn(&mut App),
            "Wait for the update to finish first.",
        ),
        (
            |app: &mut App| app.wago_install_in_progress = true,
            "Wait for the install to finish first.",
        ),
        (
            |app: &mut App| app.delete_in_progress = true,
            "Wait for the delete to finish first.",
        ),
    ] {
        let mut app = app_for_tests(ShellMode::Dashboard);
        open_find(&mut app);
        busy(&mut app);
        let generation = app.find.generation;
        let actions = app.update(AppMessage::Find(FindMessage::Start));
        assert!(
            !actions.contains(&AppAction::Find(FindAction::StartLookup)),
            "{actions:?}"
        );
        for action in actions {
            app.apply(action);
        }
        assert_eq!(app.find.step, Some(FindStep::Start));
        assert_eq!(app.find.generation, generation);
        assert!(
            app.status_line.text.contains(note),
            "{}",
            app.status_line.text
        );
    }
}

#[test]
fn auto_checks_wait_while_find_links_or_replaces() {
    let ready = || {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.scan_state = ScanState::Succeeded {
            path: PathBuf::from("AddOns"),
            summary: ScanSummary::default(),
        };
        app.effective_addon_dir = Some(PathBuf::from("AddOns"));
        app.config_pane.draft.auto_check_enabled = true;
        app
    };
    assert!(ready().auto_check_due_targets().is_some());
    let mut app = ready();
    app.find.linking = true;
    assert!(app.auto_check_due_targets().is_none());
    let mut app = ready();
    app.find.replacements = vec![found(FindLabel::Partial, FindSource::Wago, "Q", &["Q"])].into();
    assert!(app.auto_check_due_targets().is_none());
}

#[test]
fn no_to_a_replace_question_says_nothing_was_changed() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    key(&mut app, KeyCode::Char(' ')); // Clique off
    for _ in 0..2 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char(' ')); // Questie on
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Enter);
    app.note_question_shown(Instant::now());
    key(&mut app, KeyCode::Char('n'));
    assert!(
        app.status_line.text.contains("| Nothing was changed. |"),
        "{}",
        app.status_line.text
    );

    // Only links: nothing was linked.
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_question(&mut app);
    app.note_question_shown(Instant::now());
    key(&mut app, KeyCode::Char('n'));
    assert!(
        app.status_line.text.contains("| Nothing was linked. |"),
        "{}",
        app.status_line.text
    );
}

#[test]
fn a_failed_lookup_says_what_couldnt_be_read() {
    let temp = tempdir().expect("tempdir");
    let addons = temp.path().join("AddOns");
    // The database file is no database: LemonUp's list can't be read.
    let broken = temp.path().join("broken.sqlite");
    std::fs::write(
        &broken,
        "not a database, just text that goes on for a while",
    )
    .expect("file");
    let error = read_find_inputs(&broken, &addons).expect_err("no database");
    assert!(
        error.starts_with("LemonUp couldn't read its list of your addons"),
        "{error}"
    );
    // The database reads; the AddOns folder doesn't exist.
    let db = temp.path().join("state.sqlite");
    StateDatabase::open(&db).expect("database");
    let error = read_find_inputs(&db, &addons).expect_err("no AddOns folder");
    assert!(
        error.starts_with("LemonUp couldn't read your AddOns folder"),
        "{error}"
    );

    // The sheet shows the reason as it is.
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    app.find.generation = 3;
    app.find.step = Some(FindStep::Looking {
        folders: 1,
        asking: Vec::new(),
    });
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Looked {
            generation: 3,
            result: Err("LemonUp couldn't read its list of your addons: locked".to_string()),
        })),
    );
    let shown = screen(&mut app, 120, 30);
    assert!(
        shown.contains("LemonUp couldn't read its list of your addons: locked"),
        "{shown}"
    );
    assert!(!shown.contains("AddOns folder"), "{shown}");
}

#[test]
fn capital_f_says_why_there_is_nothing_to_look_up() {
    let note = |app: &App| {
        app.update(AppMessage::Find(FindMessage::Open))
            .into_iter()
            .find_map(|action| match action {
                AppAction::SetStatus(status) => Some(status.text),
                _ => None,
            })
            .unwrap_or_default()
    };
    let says = |app: &App, text: &str| {
        let note = note(app);
        assert!(note.contains(&format!("| {text} |")), "{note}");
    };
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(Vec::new());
    app.scan_state = ScanState::Succeeded {
        path: PathBuf::from("AddOns"),
        summary: ScanSummary::default(),
    };
    says(&app, "Your AddOns folder has no addons to look up.");
    app.scan_state = ScanState::Failed("denied".to_string());
    says(
        &app,
        "LemonUp couldn't read your AddOns folder, so there is nothing to look up.",
    );
    app.scan_state = ScanState::Running(PathBuf::from("AddOns"));
    says(&app, "Wait for LemonUp to read your AddOns folder first.");
    app.scan_state = ScanState::Idle;
    app.dashboard =
        DashboardState::from_addons(vec![AddonRecord::new("Gamma", "Gamma", SourceKind::Wago)]);
    says(&app, "Every addon is linked already.");
}

#[test]
fn check_failed_comes_right_after_not_checked() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let mut outcome = outcome(report());
    // WoWInterface failed without leaving an addon Not checked.
    outcome.problems.push(SourceProblem {
        source: FindSource::WowInterface,
        error: SourceError::new(SourceErrorKind::SlowDown, "429"),
    });
    app.apply(AppAction::Find(FindAction::SetStep(FindStep::Results(
        FindResults::new(outcome),
    ))));
    let shown = screen(&mut app, 138, 80);
    let at = |heading: &str| {
        shown
            .find(heading)
            .unwrap_or_else(|| panic!("{heading}:\n{shown}"))
    };
    assert!(at("Not checked (1)") < at("Check failed (1)"), "{shown}");
    assert!(at("Check failed (1)") < at("On CurseForge only"), "{shown}");
    assert!(
        at("WoWInterface: WoWInterface asked LemonUp to slow down") < at("On CurseForge only"),
        "{shown}"
    );
}

#[test]
fn a_partial_choice_says_picking_it_replaces_your_copy() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    let mut report = report();
    let mut partial = found(
        FindLabel::Partial,
        FindSource::Wago,
        "QuestieDev",
        &["QuestieDev"],
    );
    partial.differing = vec!["QuestieDev".to_string()];
    report.results[4].candidates[0] = partial;
    open_find(&mut app);
    set_results(&mut app, report);
    let shown = screen(&mut app, 138, 80);
    let line = |needle: &str| {
        shown
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle}:\n{shown}"))
            .to_string()
    };
    assert!(
        line("QuestieDev on Wago.io")
            .contains("(QuestieDev differs from the release; picking it replaces your copy"),
        "{shown}"
    );
    assert!(shown.contains("with the one from Wago.io)"), "{shown}");
    assert!(
        !line("QuestieDev on WoWInterface").contains("picking it replaces"),
        "{shown}"
    );
}

#[test]
fn the_question_lists_added_folders_apart_from_replaced_ones() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    let mut report = report();
    report.results[3].candidates[0].missing = vec!["QuestieLoc".to_string()];
    open_find(&mut app);
    set_results(&mut app, report);
    key(&mut app, KeyCode::Char(' ')); // Clique off
    for _ in 0..2 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char(' ')); // Questie on
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Enter);
    let shown = screen(&mut app, 80, 24);
    let replaced = shown
        .find("These folders are replaced now with the copy from Wago.io:")
        .unwrap_or_else(|| panic!("{shown}"));
    let added = shown
        .find("These folders are added from Wago.io:")
        .unwrap_or_else(|| panic!("{shown}"));
    let loc = shown
        .find("QuestieLoc")
        .unwrap_or_else(|| panic!("{shown}"));
    assert!(replaced < added && added < loc, "{shown}");
}

#[test]
fn the_welcome_card_shows_capital_f_in_the_color_of_f() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(vec![
        AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
        AddonRecord::new("Beta", "Beta", SourceKind::Manual),
    ]);
    let roles = app.ui_theme.roles;
    assert_ne!(roles.lemon, roles.blue);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let key_color = |text: &str, offset: u16| {
        (0..24)
            .find_map(|y| {
                let row = (0..80)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>();
                let column = row[..row.find(text)?].chars().count();
                Some(buffer[(u16::try_from(column).expect("column") + offset, y)].fg)
            })
            .unwrap_or_else(|| panic!("{text}"))
    };
    let small = key_color("press f to find it online", 6);
    let capital = key_color("Press F to find your addons online", 6);
    assert_eq!(small, roles.lemon);
    assert_eq!(capital, small);
}

/// A report whose rows are too long for one line at 80 columns.
fn long_report() -> (FindReport, Vec<SourceProblem>) {
    let mut likely = found(
        FindLabel::Likely,
        FindSource::Wago,
        "A Very Long Addon Name Indeed",
        &["LongOne"],
    );
    likely.matched_version = Some("v5.1.5-release-with-a-long-tail".to_string());
    likely.likely_reason = Some(lemonup_core::find::LikelyReason::TocNamesIt);
    let mut partial = found(
        FindLabel::Partial,
        FindSource::Wago,
        "Questie",
        &["Questie", "QuestieDB"],
    );
    partial.differing = (0..8).map(|n| format!("Questie_Module_{n}")).collect();
    let mut choice = partial.clone();
    choice.parent_folder = "QuestieDev".to_string();
    let mut not_checked = (0..14)
        .map(|n| {
            let mut result = result(
                FindLabel::NotChecked,
                &format!("NotCheckedAddon{n}"),
                Vec::new(),
            );
            result.not_checked_by = vec![FindSource::Tukui];
            result
        })
        .collect::<Vec<_>>();
    let mut results = vec![
        result(FindLabel::Likely, "LongOne", vec![likely]),
        result(FindLabel::Partial, "Questie", vec![partial]),
        result(
            FindLabel::ChooseOne,
            "QuestieDev",
            vec![
                choice,
                found(
                    FindLabel::Exact,
                    FindSource::WowInterface,
                    "QuestieDev",
                    &["QuestieDev"],
                ),
            ],
        ),
    ];
    results.append(&mut not_checked);
    let problems = vec![
        SourceProblem {
            source: FindSource::Tukui,
            error: SourceError::new(SourceErrorKind::Other, "a long technical detail ".repeat(4)),
        },
        SourceProblem {
            source: FindSource::WowInterface,
            error: SourceError::new(SourceErrorKind::SlowDown, "429"),
        },
    ];
    (
        FindReport {
            results,
            failed: vec![FindSource::Tukui],
            skipped: Vec::new(),
        },
        problems,
    )
}

#[test]
fn long_result_rows_continue_under_their_text() {
    let headings = [
        "Likely matches",
        "Partial matches",
        "Choose one",
        "Not checked",
        "Check failed",
    ];
    for (width, height) in [(80, 90), (138, 90)] {
        let mut app = app_for_tests(ShellMode::Dashboard);
        open_find(&mut app);
        let (report, problems) = long_report();
        app.apply(AppAction::Find(FindAction::SetStep(FindStep::Results(
            FindResults::new(FindOutcome {
                report,
                problems,
                names: LocalNames::default(),
            }),
        ))));
        let rows = render_rows(&mut app, width, height)
            .into_iter()
            .map(|row| row.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let text = |row: &[char]| row.iter().collect::<String>();
        let shown = rows
            .iter()
            .map(|row| text(row))
            .collect::<Vec<_>>()
            .join("\n");
        let top = rows
            .iter()
            .position(|row| text(row).contains("Likely matches"))
            .unwrap_or_else(|| panic!("{shown}"));
        let heading = text(&rows[top]);
        let left = heading[..heading.find("Likely matches").expect("heading")]
            .chars()
            .count();
        let bottom = rows
            .iter()
            .position(|row| text(row).contains("Nothing selected"))
            .unwrap_or_else(|| panic!("{shown}"));
        for row in &rows[top..bottom] {
            let from = text(&row[left..]);
            if row[left] == ' ' {
                assert!(
                    row[left + 1] == ' ' || from.starts_with(" ○") || from.starts_with(" ●"),
                    "{width}x{height}: a row starts one column in:\n{from}\n{shown}"
                );
                continue;
            }
            assert!(
                from.starts_with('▌') || headings.iter().any(|heading| from.starts_with(heading)),
                "{width}x{height}: a row starts at the left edge:\n{from}\n{shown}"
            );
        }
        // The long rows did wrap at 80 columns.
        if width == 80 {
            assert!(
                rows[top..bottom].iter().any(|row| {
                    let line = text(row);
                    line.contains("checked") && !line.contains("A Very Long")
                }),
                "{shown}"
            );
        }
        assert!(shown.contains("Questie_Module_7"), "{shown}");
    }
}

#[test]
fn f_opens_the_sheet_after_a_lookup_closed_with_nothing_picked() {
    // As the visual check typed it: F, enter, the results, G, enter, n, esc, j, j, f.
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.effective_addon_dir = Some(std::env::temp_dir().join("lemonup-find-test/AddOns"));
    app.dashboard = DashboardState::from_addons(vec![
        AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
        AddonRecord::new("AzerothGPS", "AzerothGPS", SourceKind::Manual),
        AddonRecord::new("Beta", "Beta", SourceKind::Manual),
        AddonRecord::new("Gamma", "Gamma", SourceKind::Manual),
    ]);
    app.dashboard.list_state.select(Some(0));
    key(&mut app, KeyCode::Char('F'));
    assert_eq!(app.find.step, Some(FindStep::Start));
    // Enter starts the lookup; here without the network.
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
        vec![AppMessage::Find(FindMessage::Start)]
    );
    app.find.generation += 1;
    app.find.step = Some(FindStep::Looking {
        folders: 4,
        asking: Vec::new(),
    });
    // Every source failed: nothing to link.
    let mut lookup = report();
    lookup.results.retain(|result| {
        matches!(
            result.label,
            FindLabel::Likely
                | FindLabel::ChooseOne
                | FindLabel::NotChecked
                | FindLabel::DeveloperCopy
        )
    });
    let generation = app.find.generation;
    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::Find(FindTaskEvent::Looked {
            generation,
            result: Ok(Box::new(outcome(lookup))),
        })),
    );
    assert!(app.find.results().is_some());
    for code in [
        KeyCode::Char('G'),
        KeyCode::Enter,
        KeyCode::Char('n'),
        KeyCode::Esc,
    ] {
        key(&mut app, code);
    }
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.find.step, None);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    let under = app
        .dashboard
        .selected_item()
        .map(|item| item.folder.clone())
        .expect("a row under the cursor");
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Find));
    assert_eq!(app.find.step, Some(FindStep::Start));
    assert_eq!(app.find.scope, Some(vec![under]));
}

fn questie_summary() -> WagoInstallSummary {
    WagoInstallSummary {
        addon_id: "Questie-id".to_string(),
        addon_name: "Questie".to_string(),
        parent_folder: "Questie".to_string(),
        installed_folders: vec!["Questie".to_string()],
        stability: crate::wago::WagoStability::Stable,
        version: Some("1.0".to_string()),
        dry_run: false,
    }
}

#[test]
fn a_replacement_that_installed_is_not_reported_as_failed() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    replace_questie(&mut app);
    app.find.replacements.push_back(found(
        FindLabel::Partial,
        FindSource::Wago,
        "Details",
        &["Details"],
    ));

    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::WagoInstallFinished(Ok(
            WagoInstallTaskOutcome::InstalledNotRefreshed {
                summary: questie_summary(),
                reason: "database is locked".to_string(),
            },
        ))),
    );
    let text = app.status_line.text.clone();
    assert_eq!(app.status_line.kind, StatusKind::Problem, "{text}");
    assert!(text.contains("Questie was replaced."), "{text}");
    assert!(
        text.contains("couldn't read your addon list again: database is locked."),
        "{text}"
    );
    assert!(text.contains("restart LemonUp"), "{text}");
    assert!(!text.contains("wasn't replaced:"), "{text}");
    assert!(!text.contains("folders weren't changed"), "{text}");
    // The next one waits: LemonUp can't read the folder it would change.
    assert!(
        text.contains("LemonUp stopped there, so Details wasn't replaced yet."),
        "{text}"
    );
    assert!(!app.wago_install_in_progress);
    assert!(app.find.replacing.is_none());
    assert!(app.find.replacements.is_empty());
    assert!(app.find_replacement_actions().is_empty());
}

#[test]
fn an_install_that_committed_is_not_an_error_in_the_search_pane() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard.detail_mode = DetailMode::Search;
    app.search_pane.install_state = SearchInstallState::Installing {
        addon_id: "Questie-id".to_string(),
        addon_name: "Questie".to_string(),
    };
    app.wago_install_in_progress = true;

    apply_all(
        &mut app,
        AppMessage::BackgroundTask(AppTaskEvent::WagoInstallFinished(Ok(
            WagoInstallTaskOutcome::InstalledNotRefreshed {
                summary: questie_summary(),
                reason: "boom".to_string(),
            },
        ))),
    );
    match &app.search_pane.install_state {
        SearchInstallState::Success {
            addon_name,
            warning,
            ..
        } => {
            assert_eq!(addon_name, "Questie");
            let warning = warning.as_deref().unwrap_or_default();
            assert!(
                warning.contains("couldn't read your addon list again: boom."),
                "{warning}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(!app.wago_install_in_progress);
    assert_eq!(app.status_line.kind, StatusKind::Problem);
    assert!(
        app.status_line.text.contains("Questie is installed."),
        "{}",
        app.status_line.text
    );
}

#[test]
fn a_committed_install_whose_list_cannot_be_read_is_not_an_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let addons = dir.path().join("AddOns");
    fs::create_dir_all(&addons).expect("AddOns");
    // A database file that is really a directory can't be opened.
    let broken = dir.path().join("state.db");
    fs::create_dir_all(&broken).expect("directory");
    let outcome = crate::app::finish_wago_install(questie_summary(), &broken, &addons);
    assert!(
        matches!(
            outcome,
            WagoInstallTaskOutcome::InstalledNotRefreshed { ref summary, .. }
                if summary.addon_name == "Questie"
        ),
        "{outcome:?}"
    );

    let good = dir.path().join("good.db");
    let outcome = crate::app::finish_wago_install(questie_summary(), &good, &addons);
    assert!(
        matches!(outcome, WagoInstallTaskOutcome::Installed(_)),
        "{outcome:?}"
    );
}

/// A report much taller than 80x24: a few matches, then long lists of names
/// below the last row the cursor can reach, and a source that failed.
fn tall_report(selectable: bool) -> FindReport {
    let mut results = Vec::new();
    if selectable {
        results.push(result(
            FindLabel::Exact,
            "Clique",
            vec![found(
                FindLabel::Exact,
                FindSource::WowInterface,
                "Clique",
                &["Clique"],
            )],
        ));
        results.push(result(
            FindLabel::Likely,
            "Pawn",
            vec![found(
                FindLabel::Likely,
                FindSource::WowInterface,
                "Pawn",
                &["Pawn"],
            )],
        ));
    }
    for index in 0..40 {
        results.push(result(
            FindLabel::NotFound,
            &format!("Gone{index:02}Name"),
            Vec::new(),
        ));
    }
    results.push(result(FindLabel::CurseForgeOnly, "Leatrix", Vec::new()));
    results.push(result(FindLabel::DeveloperCopy, "ZzLastCopy", Vec::new()));
    FindReport {
        results,
        failed: vec![FindSource::Wago],
        skipped: Vec::new(),
    }
}

/// The long report open in the sheet, drawn once at 80x24.
fn open_tall_report(app: &mut App, selectable: bool) {
    open_find(app);
    app.apply(AppAction::Find(FindAction::SetStep(FindStep::Results(
        FindResults::new(FindOutcome {
            report: tall_report(selectable),
            problems: vec![SourceProblem {
                source: FindSource::Wago,
                error: SourceError::new(SourceErrorKind::Other, "wago exploded"),
            }],
            names: LocalNames::default(),
        }),
    ))));
    let shown = screen(app, 80, 24);
    assert!(app.sheet_scroll.get().max_offset > 0, "{shown}");
    assert!(!shown.contains("ZzLastCopy"), "{shown}");
}

/// A key, then the sheet drawn again, as the terminal loop does.
fn press_and_draw(app: &mut App, code: KeyCode) -> String {
    key(app, code);
    screen(app, 80, 24)
}

fn wheel_event(kind: MouseEventKind) -> MouseEvent {
    MouseEvent {
        kind,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::NONE,
    }
}

fn wheel_and_draw(app: &mut App, kind: MouseEventKind) -> String {
    for message in app.messages_for_mouse(wheel_event(kind)) {
        apply_all(app, message);
    }
    screen(app, 80, 24)
}

#[test]
fn down_past_the_last_row_scrolls_to_the_end_of_the_report() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    // The cursor walks down to the action row, then the report scrolls.
    let mut shown = String::new();
    for _ in 0..200 {
        shown = press_and_draw(&mut app, KeyCode::Down);
    }
    assert_eq!(results(&app).cursor, FindFocus::Action);
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    assert!(shown.contains("Gone39Name"), "{shown}");
    assert!(
        app.sheet_scroll.get().offset == app.sheet_scroll.get().max_offset,
        "{shown}"
    );
    // Back up: the cursor moves up first, and the top of the report returns.
    for _ in 0..200 {
        shown = press_and_draw(&mut app, KeyCode::Up);
    }
    assert!(shown.contains("Ready to link"), "{shown}");
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
}

#[test]
fn j_and_k_scroll_past_the_ends_like_the_arrows() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    let mut shown = String::new();
    for _ in 0..200 {
        shown = press_and_draw(&mut app, KeyCode::Char('j'));
    }
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    for _ in 0..200 {
        shown = press_and_draw(&mut app, KeyCode::Char('k'));
    }
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
}

#[test]
fn end_and_home_go_to_the_bottom_and_top_of_the_report() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    let shown = press_and_draw(&mut app, KeyCode::End);
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    assert_eq!(results(&app).cursor, FindFocus::Action);
    assert_eq!(
        app.sheet_scroll.get().offset,
        app.sheet_scroll.get().max_offset
    );

    let shown = press_and_draw(&mut app, KeyCode::Home);
    assert!(!shown.contains("ZzLastCopy"), "{shown}");
    assert!(shown.contains("Ready to link"), "{shown}");
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    assert_eq!(results(&app).cursor, FindFocus::Result(0), "{shown}");

    // G and gg do the same.
    let shown = press_and_draw(&mut app, KeyCode::Char('G'));
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    key(&mut app, KeyCode::Char('g'));
    let shown = press_and_draw(&mut app, KeyCode::Char('g'));
    assert!(!shown.contains("ZzLastCopy"), "{shown}");
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
}

#[test]
fn page_keys_move_the_view_past_the_last_row() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    // The first page down takes the cursor to the action row.
    press_and_draw(&mut app, KeyCode::PageDown);
    assert_eq!(results(&app).cursor, FindFocus::Action);
    let before = app.sheet_scroll.get().offset;
    // The next ones scroll a page at a time, to the end.
    let mut last = before;
    for _ in 0..30 {
        let shown = press_and_draw(&mut app, KeyCode::PageDown);
        let offset = app.sheet_scroll.get().offset;
        assert!(offset >= last, "{shown}");
        last = offset;
    }
    let shown = screen(&mut app, 80, 24);
    assert!(last > before, "{shown}");
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    // And back up to the top.
    let mut shown = String::new();
    for _ in 0..30 {
        shown = press_and_draw(&mut app, KeyCode::PageUp);
    }
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    assert!(!shown.contains("ZzLastCopy"), "{shown}");
}

#[test]
fn the_wheel_scrolls_the_report_when_the_cursor_cannot_move() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    // The cursor goes first.
    assert_eq!(
        app.messages_for_mouse(wheel_event(MouseEventKind::ScrollDown)),
        vec![AppMessage::Find(FindMessage::Move(1))]
    );
    press_and_draw(&mut app, KeyCode::End);
    press_and_draw(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::End);
    // On the last row the same wheel scrolls the text.
    let _ = screen(&mut app, 80, 24);
    apply_all(&mut app, AppMessage::SheetScroll(SheetMove::Top));
    let shown = screen(&mut app, 80, 24);
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    assert_eq!(
        app.messages_for_mouse(wheel_event(MouseEventKind::ScrollDown)),
        vec![AppMessage::SheetScroll(SheetMove::LineDown)]
    );
    let mut shown = shown;
    for _ in 0..200 {
        shown = wheel_and_draw(&mut app, MouseEventKind::ScrollDown);
    }
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    assert_eq!(results(&app).cursor, FindFocus::Action);
}

#[test]
fn a_report_with_no_rows_to_pick_scrolls_with_every_key() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, false);
    assert!(
        results(&app)
            .focusables(app.find_offers_search())
            .is_empty()
    );
    // The key line offers the scroll, and the sheet says more is below.
    let shown = screen(&mut app, 80, 24);
    assert!(shown.contains("↓"), "{shown}");
    assert!(shown.contains("more"), "{shown}");
    let keys = shown.lines().last().expect("key line").to_string();
    assert!(keys.contains("↑/↓ scroll"), "{keys}");

    let shown = press_and_draw(&mut app, KeyCode::Down);
    assert_eq!(app.sheet_scroll.get().offset, 1, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::Char('j'));
    assert_eq!(app.sheet_scroll.get().offset, 2, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::Up);
    assert_eq!(app.sheet_scroll.get().offset, 1, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::Char('k'));
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::PageDown);
    assert!(app.sheet_scroll.get().offset > 1, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::PageUp);
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::End);
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    let shown = press_and_draw(&mut app, KeyCode::Home);
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    let shown = wheel_and_draw(&mut app, MouseEventKind::ScrollDown);
    assert_eq!(app.sheet_scroll.get().offset, 1, "{shown}");
    let shown = wheel_and_draw(&mut app, MouseEventKind::ScrollUp);
    assert_eq!(app.sheet_scroll.get().offset, 0, "{shown}");
    // Keys that do nothing here still do nothing.
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char(' '))),
        vec![]
    );
}

#[test]
fn help_says_the_arrows_scroll_past_the_last_row() {
    let help = crate::app::keymap::help_page(HelpContext::Find);
    let row = |label: &str| {
        help.sections()
            .flat_map(|section| section.rows.iter())
            .find(|row| matches!(row.label, crate::app::keymap::HelpLabel::Keys(keys) if keys == label))
            .expect(label)
    };
    assert!(row("↑ ↓  or  j k").text.contains("scroll"));
    assert!(row("PgUp PgDn").text.contains("scroll"));
    assert!(
        row("Home End  or  gg G")
            .text
            .contains("bottom of the report")
    );
}

const CHIPS: &str = "1 ● Wago.io    2 ● WoWInterface    3 ● TukUI";
const SIMPLE_CHIPS: &str = "1 [x] Wago.io    2 [x] WoWInterface    3 [x] TukUI";

/// The Start step's lines as drawn inside the sheet's border, blanks kept, from "Look for"
/// down. Whatever shows beside the sheet is cut off.
fn start_lines(shown: &str) -> Vec<String> {
    let rows = shown
        .lines()
        .map(|row| row.chars().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let is_border = |c: &char| "│|".contains(*c);
    let (first, column) = rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| {
            let text = row.iter().collect::<String>();
            let at = text[..text.find("Look for")?].chars().count();
            Some((index, at))
        })
        .unwrap_or_else(|| panic!("{shown}"));
    // A sheet over the list has a border; a full screen one does not.
    let left = rows[first][..column].iter().rposition(is_border);
    let right = rows[first][column..]
        .iter()
        .position(is_border)
        .map_or(rows[first].len(), |at| column + at);
    rows[first..]
        .iter()
        .map(|row| {
            let end = right.min(row.len());
            row[left.map_or(0, |left| left + 1).min(end)..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect()
}

/// The key colors of the first cell of `text` on the screen, `offset` characters in.
fn color_at(
    app: &mut App,
    width: u16,
    height: u16,
    text: &str,
    offset: u16,
) -> ratatui::style::Color {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .find_map(|y| {
            let row = (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>();
            let column = row[..row.find(text)?].chars().count();
            Some(buffer[(u16::try_from(column).expect("column") + offset, y)].fg)
        })
        .unwrap_or_else(|| panic!("{text}"))
}

#[test]
fn start_shows_the_sources_as_chips_on_one_line_for_one_addon() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    app.find.single = Some("ForeverAuras".to_string());
    for (width, height) in [(80, 24), (120, 30), (138, 36)] {
        let shown = screen(&mut app, width, height);
        let lines = start_lines(&shown);
        assert_eq!(lines[0], "Look for ForeverAuras on", "{shown}");
        assert_eq!(lines[1], "", "{shown}");
        assert_eq!(lines[2], CHIPS, "{shown}");
        assert_eq!(lines[3], "", "{shown}");
        assert_eq!(
            lines[4], "Sends folder names and file hashes to Wago.io.",
            "{shown}"
        );
        // The only sentences: nothing about what is linked.
        assert!(!shown.contains("Nothing is linked"), "{shown}");
        assert!(!shown.contains("Nothing changes until"), "{shown}");
        assert!(
            shown.contains("1 2 3 check   enter look   esc close"),
            "{shown}"
        );
    }
}

#[test]
fn start_counts_the_not_linked_addons_for_several() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.dashboard = DashboardState::from_addons(vec![
        AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
        AddonRecord::new("Beta", "Beta", SourceKind::Manual),
        AddonRecord::new("Gamma", "Gamma", SourceKind::Manual),
    ]);
    open_find(&mut app);
    let shown = screen(&mut app, 80, 24);
    let lines = start_lines(&shown);
    assert_eq!(lines[0], "Look for 3 Not linked addons on", "{shown}");
    assert_eq!(lines[2], CHIPS, "{shown}");
}

#[test]
fn start_draws_the_check_boxes_plain_with_simple_symbols() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.ui_theme.symbols = crate::theme::Symbols::SIMPLE;
    open_find(&mut app);
    key(&mut app, KeyCode::Char('3'));
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        let lines = start_lines(&shown);
        assert_eq!(
            lines[2], "1 [x] Wago.io    2 [x] WoWInterface    3 [ ] TukUI",
            "{shown}"
        );
        // Every cell of the sheet is plain ASCII.
        let sheet = lines.join("\n");
        assert!(sheet.is_ascii(), "{sheet}");
    }
    key(&mut app, KeyCode::Char('3'));
    assert!(screen(&mut app, 80, 24).contains(SIMPLE_CHIPS));
}

#[test]
fn the_chip_line_wraps_between_chips_instead_of_cutting_a_name() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let room = app.find_source_chips(30);
    let rows = room
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0], "  1 ● Wago.io");
    assert_eq!(rows[1], "  2 ● WoWInterface");
    assert_eq!(rows[2], "  3 ● TukUI");
    // At the real width of the sheet at 80x24 they share one line.
    assert_eq!(app.find_source_chips(76).len(), 1);
}

#[test]
fn a_chip_has_the_key_color_the_accent_mark_and_muted_when_unchecked() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    key(&mut app, KeyCode::Char('3'));
    let roles = app.ui_theme.roles;
    assert_ne!(roles.blue, roles.muted);
    assert_ne!(roles.lemon, roles.muted);
    // Number, mark, name of a checked chip.
    assert_eq!(color_at(&mut app, 120, 30, CHIPS_START, 0), roles.blue);
    assert_eq!(color_at(&mut app, 120, 30, CHIPS_START, 2), roles.lemon);
    assert_eq!(color_at(&mut app, 120, 30, CHIPS_START, 4), roles.text);
    // The unchecked one: key color on the number, muted mark and name.
    assert_eq!(color_at(&mut app, 120, 30, "3 ○ TukUI", 0), roles.blue);
    assert_eq!(color_at(&mut app, 120, 30, "3 ○ TukUI", 2), roles.muted);
    assert_eq!(color_at(&mut app, 120, 30, "3 ○ TukUI", 4), roles.muted);

    // Wago.io without a key: all muted, its number too (the key does nothing).
    app.wago_api_key = None;
    for offset in [0, 2, 4] {
        assert_eq!(
            color_at(&mut app, 120, 30, "1 ○ Wago.io", offset),
            roles.muted,
            "{offset}"
        );
    }
    assert_eq!(
        color_at(&mut app, 120, 30, "2 ● WoWInterface", 0),
        roles.blue
    );
}

const CHIPS_START: &str = "1 ● Wago.io";

#[test]
fn without_a_key_wago_is_dimmed_unchecked_and_its_key_does_nothing() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.wago_api_key = None;
    open_find(&mut app);
    let shown = screen(&mut app, 80, 24);
    let lines = start_lines(&shown);
    assert_eq!(
        lines[2], "1 ○ Wago.io    2 ● WoWInterface    3 ● TukUI",
        "{shown}"
    );
    assert_eq!(lines[4], "Add a Wago.io key in Settings to look there too.");
    assert!(!shown.contains("needs a key"), "{shown}");
    assert!(!shown.contains("Sends folder names"), "{shown}");

    // Key 1 does nothing, and neither the sheet nor the key line offers it.
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('1'))),
        vec![]
    );
    let table = app.key_table(80);
    assert!(
        table
            .iter()
            .any(|hint| hint.key == "2 3" && hint.label == "check")
    );
    assert!(!table.iter().any(|hint| hint.key.contains('1')));
    assert!(
        shown.contains("2 3 check   enter look   esc close"),
        "{shown}"
    );
    assert!(!shown.contains("1 2 3 check"), "{shown}");
    key(&mut app, KeyCode::Char('1'));
    assert!(app.find.sources.wants(FindSource::Wago));
    assert!(!app.find_checked(FindSource::Wago));
}

#[test]
fn keys_1_2_3_check_and_uncheck_the_sources_and_the_arrows_do_nothing() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    for (code, source) in [
        ('1', FindSource::Wago),
        ('2', FindSource::WowInterface),
        ('3', FindSource::Tukui),
    ] {
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(code))),
            vec![AppMessage::Find(FindMessage::ToggleSource(source))]
        );
        key(&mut app, KeyCode::Char(code));
        assert!(!app.find_checked(source), "{code}");
        key(&mut app, KeyCode::Char(code));
        assert!(app.find_checked(source), "{code}");
    }
    key(&mut app, KeyCode::Char('2'));
    let shown = screen(&mut app, 80, 24);
    assert_eq!(
        start_lines(&shown)[2],
        "1 ● Wago.io    2 ○ WoWInterface    3 ● TukUI",
        "{shown}"
    );
    // No cursor, so no arrow keys.
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right] {
        assert_eq!(
            app.messages_for_key(KeyEvent::from(code)),
            vec![],
            "{code:?}"
        );
    }
    // The digits do nothing once results are showing.
    set_results(&mut app, report());
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('1'))),
        vec![]
    );
}

#[test]
fn with_nothing_checked_enter_does_nothing_and_the_line_says_to_check_one() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    for code in ['1', '2', '3'] {
        key(&mut app, KeyCode::Char(code));
    }
    let shown = screen(&mut app, 80, 24);
    let lines = start_lines(&shown);
    assert_eq!(
        lines[2], "1 ○ Wago.io    2 ○ WoWInterface    3 ○ TukUI",
        "{shown}"
    );
    assert_eq!(lines[4], "Check at least one source.", "{shown}");
    assert!(!shown.contains("Sends folder names"), "{shown}");
    // No hint for a key that does nothing.
    assert!(shown.contains("1 2 3 check   esc close"), "{shown}");
    assert!(!shown.contains("enter look"), "{shown}");
    assert!(!app.key_table(80).iter().any(|hint| hint.key == "enter"));
    assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Enter)), vec![]);
    assert!(app.update(AppMessage::Find(FindMessage::Start)).is_empty());
    assert_eq!(app.find.step, Some(FindStep::Start));

    // Without a key, with the other two unchecked, it says the same.
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.wago_api_key = None;
    open_find(&mut app);
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Char('3'));
    let shown = screen(&mut app, 80, 24);
    assert_eq!(
        start_lines(&shown)[4],
        "Check at least one source.",
        "{shown}"
    );
    assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Enter)), vec![]);
}

#[test]
fn enter_with_wowinterface_unchecked_asks_only_the_others() {
    let temp = tempdir().expect("tempdir");
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = ConfigStore::new(temp.path().join("config.toml"));
    open_find(&mut app);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.find_asked(), vec![FindSource::Wago, FindSource::Tukui]);
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
        vec![AppMessage::Find(FindMessage::Start)]
    );
    let actions = app.update(AppMessage::Find(FindMessage::Start));
    assert!(actions.contains(&AppAction::Find(FindAction::StartLookup)));
    // What the Looking step then says it asks.
    let asking = crate::find::asking(app.find.sources, true, true);
    app.find.generation = 1;
    app.find.step = Some(FindStep::Looking {
        folders: 3,
        asking: Vec::new(),
    });
    apply_all(
        &mut app,
        AppMessage::Find(FindMessage::Task(FindTaskEvent::Read {
            generation: 1,
            folders: 3,
            asking,
        })),
    );
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Asking Wago.io, TukUI"), "{shown}");
    assert!(!shown.contains("WoWInterface"), "{shown}");
}

/// Presses enter on Start: applies everything but the lookup, which needs the network.
/// Returns whether the lookup started.
fn enter_on_start(app: &mut App) -> bool {
    let mut looked = false;
    for action in app.update(AppMessage::Find(FindMessage::Start)) {
        if action == AppAction::Find(FindAction::StartLookup) {
            looked = true;
        } else {
            app.apply(action);
        }
    }
    looked
}

fn saved_find_sources(path: &std::path::Path) -> Option<Vec<String>> {
    match ConfigStore::new(path.to_path_buf()).load().expect("load") {
        ConfigLoad::Loaded(config) => config.find_sources,
        ConfigLoad::Missing(_) => None,
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn the_choice_is_saved_for_this_game_on_enter_and_restored_next_time() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("config.toml");
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = ConfigStore::new(path.clone());
    open_find(&mut app);
    // Missing means all.
    assert_eq!(app.find.sources, FindChoice::all());

    // Toggling does not save.
    key(&mut app, KeyCode::Char('3'));
    assert!(!path.exists());
    assert!(enter_on_start(&mut app));
    assert_eq!(
        saved_find_sources(&path),
        Some(names(&["wago", "wowinterface"]))
    );
    let raw = std::fs::read_to_string(&path).expect("config");
    assert!(raw.contains("find_sources"), "{raw}");

    // Next time the sheet opens with that choice, in this session and from the file.
    key(&mut app, KeyCode::Esc);
    open_find(&mut app);
    assert_eq!(
        app.find_asked(),
        vec![FindSource::Wago, FindSource::WowInterface]
    );
    assert!(screen(&mut app, 80, 24).contains("3 ○ TukUI"));
    let mut fresh = app_for_tests(ShellMode::Dashboard);
    fresh.config_store = ConfigStore::new(path.clone());
    fresh.config_pane =
        fresh
            .config_pane
            .mark_saved(match fresh.config_store.load().expect("load") {
                ConfigLoad::Loaded(config) => config,
                ConfigLoad::Missing(_) => panic!("saved"),
            });
    open_find(&mut fresh);
    assert!(!fresh.find.sources.wants(FindSource::Tukui));

    // An unchanged choice writes nothing.
    std::fs::remove_file(&path).expect("remove");
    assert!(enter_on_start(&mut app));
    assert!(!path.exists());
}

#[test]
fn saved_names_this_version_does_not_know_are_ignored() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_pane.persisted.find_sources = Some(names(&["tukui", "curseforge", "WOWI"]));
    open_find(&mut app);
    assert_eq!(
        app.find.sources,
        FindChoice::only(&[FindSource::WowInterface, FindSource::Tukui])
    );
    // Only unknown names: all.
    app.config_pane.persisted.find_sources = Some(names(&["curseforge"]));
    open_find(&mut app);
    assert_eq!(app.find.sources, FindChoice::all());
}

#[test]
fn no_wago_key_is_never_saved_as_unchecking_wago_and_a_key_later_checks_it_again() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("config.toml");
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = ConfigStore::new(path.clone());
    app.wago_api_key = None;
    open_find(&mut app);
    // The person unchecks TukUI; Wago.io is only unavailable.
    key(&mut app, KeyCode::Char('3'));
    assert!(enter_on_start(&mut app));
    assert_eq!(
        saved_find_sources(&path),
        Some(names(&["wago", "wowinterface"]))
    );

    // A key added later: Wago.io is checked again.
    app.wago_api_key = Some("key".to_string());
    open_find(&mut app);
    assert!(app.find_checked(FindSource::Wago));
    let shown = screen(&mut app, 80, 24);
    assert_eq!(
        start_lines(&shown)[2],
        "1 ● Wago.io    2 ● WoWInterface    3 ○ TukUI",
        "{shown}"
    );

    // Unless the person unchecked it themselves.
    key(&mut app, KeyCode::Char('1'));
    assert!(enter_on_start(&mut app));
    assert_eq!(saved_find_sources(&path), Some(names(&["wowinterface"])));
    app.wago_api_key = None;
    open_find(&mut app);
    app.wago_api_key = Some("key".to_string());
    assert!(!app.find_checked(FindSource::Wago));
}

#[test]
fn a_save_that_fails_says_so_and_the_lookup_still_starts() {
    let temp = tempdir().expect("tempdir");
    let mut app = app_for_tests(ShellMode::Dashboard);
    // A file where the config's folder should be: the write fails.
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, "x").expect("file");
    app.config_store = ConfigStore::new(blocker.join("config.toml"));
    open_find(&mut app);
    key(&mut app, KeyCode::Char('3'));
    let actions = app.update(AppMessage::Find(FindMessage::Start));
    assert!(actions.contains(&AppAction::Find(FindAction::StartLookup)));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, AppAction::SetStatus(status)
            if status.text.contains("Couldn't remember which sources"))),
        "{actions:?}"
    );
}

#[test]
fn help_lists_the_source_keys_and_the_key_line_shows_them_from_the_same_table() {
    let help = crate::app::keymap::help_page(HelpContext::Find);
    let keys = help
        .sections()
        .flat_map(|section| section.rows.iter())
        .flat_map(|row| row.keys.iter().copied())
        .collect::<Vec<_>>();
    for digit in ['1', '2', '3'] {
        assert!(
            keys.contains(&KeyEvent::from(KeyCode::Char(digit))),
            "{digit}"
        );
    }

    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let table = app.key_table(138);
    let check = table
        .iter()
        .find(|hint| hint.label == "check")
        .expect("check hint");
    assert_eq!(check.key, "1 2 3");
    assert!(
        table
            .iter()
            .any(|hint| hint.key == "enter" && hint.label == "look")
    );
    // Pressing a source key pulses the hint that answers to it.
    for digit in ['1', '2', '3'] {
        assert!(
            app.footer_hint_for_key(KeyEvent::from(KeyCode::Char(digit)))
                .is_some(),
            "{digit}"
        );
    }
    // Help opened from Start lists them.
    key(&mut app, KeyCode::Char('?'));
    let shown = screen(&mut app, 138, 36);
    for line in [
        "Wago.io on or off",
        "WoWInterface on or off",
        "TukUI on or off",
    ] {
        assert!(shown.contains(line), "{shown}");
    }
}

#[test]
fn start_and_looking_keep_the_same_size_and_place() {
    let open = |app: &mut App| open_find(app);
    let steps: [super::sheets::SheetStep<'_>; 2] = [
        ("Look for", &|app: &mut App| {
            app.find.step = Some(FindStep::Start)
        }),
        ("Asking TukUI", &|app: &mut App| {
            app.find.step = Some(FindStep::Looking {
                folders: 4,
                asking: vec![FindSource::Tukui],
            })
        }),
    ];
    super::sheets::assert_sheet_holds_still(&[(138, 36), (120, 30), (80, 24)], &open, &steps);
}
