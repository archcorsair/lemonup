//! The Find your addons online sheet (`F`, and `f` on one addon). Lookups are
//! injected as finished reports or as the answers of single sources; the jobs the sheet
//! starts are recorded, not run, so nothing here reads a folder or asks the network.

use super::*;
use crate::app::dialog::Question;
use crate::app::find_sheet::{
    FindFocus, FindJob, FindLinkOutcome, FindOutcome, FindRead, FindResults, FindStep,
    WAGO_HASH_NOTICE, link_found, read_find_inputs, search_name,
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
        not_asked: Vec::new(),
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
fn capital_f_opens_the_sheet_and_starts_looking_at_once() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('F'))),
        vec![AppMessage::Find(FindMessage::Open)]
    );
    open_find(&mut app);
    assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Find));
    // No step before the lookup: the folders are read at once.
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));
    assert_eq!(app.find.jobs, vec![FindJob::Read]);
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Find your addons online"), "{shown}");
    assert!(shown.contains("Reading 1 folders"), "{shown}");
    assert!(!shown.contains("Look for"), "{shown}");

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
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));

    // With none selected, every Not linked addon.
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    assert_eq!(app.find.scope, None);
}

/// What the lookup read: no rows, and `folders` as read from disk.
fn read_of(inputs: FindInputs) -> Box<FindRead> {
    Box::new(FindRead {
        rows: Vec::new(),
        inputs,
    })
}

fn no_folders() -> FindInputs {
    FindInputs {
        flavor: GameFlavor::Retail,
        folders: Vec::new(),
    }
}

/// The answer of `source` alone: the others not asked.
fn answer_of(source: FindSource, lookups: SourceLookups) -> Box<crate::find::Lookups> {
    let mut only = SourceLookups::not_asked();
    only.take_answer(source, lookups);
    Box::new(crate::find::Lookups {
        lookups: only,
        problems: Vec::new(),
    })
}

fn nothing_found() -> SourceLookups {
    SourceLookups {
        wago: SourceAnswer::Found(Vec::new()),
        wowinterface: SourceAnswer::Found(Vec::new()),
        tukui: SourceAnswer::Found(Vec::new()),
    }
}

fn deliver(app: &mut App, event: FindTaskEvent) {
    apply_all(app, AppMessage::BackgroundTask(AppTaskEvent::Find(event)));
}

fn deliver_read(app: &mut App, inputs: FindInputs) {
    let generation = app.find.generation;
    deliver(
        app,
        FindTaskEvent::Read {
            generation,
            read: Ok(read_of(inputs)),
        },
    );
}

fn deliver_answer(app: &mut App, source: FindSource, lookups: SourceLookups) {
    let generation = app.find.generation;
    deliver(
        app,
        FindTaskEvent::Answered {
            generation,
            source,
            lookups: answer_of(source, lookups),
        },
    );
}

/// The sources the sheet asked, one request each, in order.
fn asked(app: &App) -> Vec<FindSource> {
    app.find
        .jobs
        .iter()
        .filter_map(|job| match job {
            FindJob::Ask(source) => Some(*source),
            FindJob::Read => None,
        })
        .collect()
}

#[test]
fn esc_while_looking_changes_nothing_and_a_late_answer_is_dropped() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let generation = app.find.generation;
    let before = app.dashboard.items.clone();
    assert!(screen(&mut app, 120, 30).contains("Reading 1 folders"));

    key(&mut app, KeyCode::Esc);
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.find.step, None);
    for event in [
        FindTaskEvent::Read {
            generation,
            read: Ok(read_of(no_folders())),
        },
        FindTaskEvent::Answered {
            generation,
            source: FindSource::Tukui,
            lookups: answer_of(FindSource::Tukui, nothing_found()),
        },
    ] {
        deliver(&mut app, event);
    }
    assert_eq!(app.find.step, None);
    assert_eq!(app.shell_ui.overlay.active, None);
    assert_eq!(app.dashboard.items, before);
    assert!(asked(&app).is_empty());

    // A new lookup started since: the old answers still land nowhere.
    open_find(&mut app);
    deliver(
        &mut app,
        FindTaskEvent::Read {
            generation,
            read: Err("old".to_string()),
        },
    );
    deliver(
        &mut app,
        FindTaskEvent::Answered {
            generation,
            source: FindSource::Tukui,
            lookups: answer_of(FindSource::Tukui, nothing_found()),
        },
    );
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));
    assert!(app.find.answered.is_empty());
}

#[test]
fn results_are_grouped_like_the_report_and_only_ready_rows_start_selected() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    set_results(&mut app, report());
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
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.wago_api_key = None;
    open_find(&mut app);
    let generation = app.find.generation;
    deliver(
        &mut app,
        FindTaskEvent::Read {
            generation,
            read: Ok(Box::new(FindRead { rows, inputs })),
        },
    );
    // Without a key Wago.io is not asked.
    assert_eq!(
        asked(&app),
        vec![FindSource::WowInterface, FindSource::Tukui]
    );
    // The first answer shows the results; the other source is still asked.
    let mut tukui = nothing_found();
    tukui.tukui = SourceAnswer::Found(vec![TukuiPackage {
        id: "-2".to_string(),
        name: "ElvUI".to_string(),
        version: "13.80".to_string(),
        web_url: "https://tukui.org/elvui".to_string(),
        author: None,
        directories: vec!["ElvUI".to_string()],
    }]);
    deliver_answer(&mut app, FindSource::Tukui, tukui);
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
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));
    assert_eq!(app.find.jobs, vec![FindJob::Read]);

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
        not_asked: Vec::new(),
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
            not_asked: Vec::new(),
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
            app.find.step = Some(FindStep::Looking { folders: 4 })
        }),
        ("[x] WoWInterface", &|app: &mut App| {
            app.find.read = Some(read_of(no_folders()));
            app.find.asking = vec![FindSource::Wago, FindSource::WowInterface];
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
fn capital_f_waits_for_a_job_that_changes_the_addons() {
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
        app.effective_addon_dir = Some(std::env::temp_dir().join("lemonup-find-test/AddOns"));
        busy(&mut app);
        let generation = app.find.generation;
        apply_all(&mut app, AppMessage::Find(FindMessage::Open));
        assert_eq!(app.find.step, None);
        assert_eq!(app.shell_ui.overlay.active, None);
        assert_eq!(app.find.generation, generation);
        assert!(app.find.jobs.is_empty());
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
    let generation = app.find.generation;
    deliver(
        &mut app,
        FindTaskEvent::Read {
            generation,
            read: Err("LemonUp couldn't read its list of your addons: locked".to_string()),
        },
    );
    // Nothing to ask without the folders.
    assert!(asked(&app).is_empty());
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
        line("QuestieDev on Wago.io").contains("(QuestieDev differs from the release; picking"),
        "{shown}"
    );
    assert!(
        shown.contains("it replaces your copy with the one from Wago.io)"),
        "{shown}"
    );
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
            not_asked: Vec::new(),
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
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 4 }));
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
    set_results(&mut app, lookup);
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
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));
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
        not_asked: Vec::new(),
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

/// The source rail as drawn: its lines from "Sources" down, from its text's left edge.
fn rail_of(shown: &str) -> Vec<String> {
    let rows = shown.lines().collect::<Vec<_>>();
    let border = |row: &str, column: usize| matches!(row.chars().nth(column), Some('│' | '|'));
    let (at, column) = rows
        .iter()
        .enumerate()
        .find_map(|(at, row)| {
            let column = row[..row.find("Sources")?].chars().count();
            (column >= 2 && border(row, column - 2)).then_some((at, column))
        })
        .unwrap_or_else(|| panic!("no rail:\n{shown}"));
    rows[at..]
        .iter()
        .take_while(|row| border(row, column - 2))
        .map(|row| {
            row.chars()
                .skip(column)
                .take(22)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The rail's lines of the three sources.
fn rail_sources(shown: &str) -> Vec<String> {
    rail_of(shown)
        .into_iter()
        .filter(|line| line.contains(" ["))
        .collect()
}

/// The Wago.io notice in the rail, its lines joined: the lines after the rail's blank line.
fn rail_notice(shown: &str) -> String {
    let rail = rail_of(shown);
    let blank = rail
        .iter()
        .position(String::is_empty)
        .unwrap_or_else(|| panic!("no notice:\n{shown}"));
    rail[blank + 1..].join(" ")
}

fn has_notice(shown: &str) -> bool {
    let rail = rail_of(shown);
    rail.iter()
        .position(String::is_empty)
        .is_some_and(|blank| rail[blank + 1..].join(" ") == WAGO_HASH_NOTICE)
}

/// The rail's line for `name`.
fn rail_line(shown: &str, name: &str) -> String {
    rail_of(shown)
        .into_iter()
        .find(|line| line.contains(name))
        .unwrap_or_else(|| panic!("no {name} in the rail:\n{shown}"))
}

/// The cell of the screen `offset` characters into the first `text` on it.
fn cell_at(
    app: &mut App,
    width: u16,
    height: u16,
    text: &str,
    offset: u16,
) -> ratatui::buffer::Cell {
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
            Some(buffer[(u16::try_from(column).expect("column") + offset, y)].clone())
        })
        .unwrap_or_else(|| panic!("{text}"))
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

/// An app whose config is saved in its own folder, so saving the sources touches no other test.
fn app_with_config() -> (tempfile::TempDir, std::path::PathBuf, App) {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("config.toml");
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = ConfigStore::new(path.clone());
    (temp, path, app)
}

/// An AddOns folder with Spy (and Spy_Extra), Clique and Lonely, as the lookup reads it.
fn spy_inputs() -> (tempfile::TempDir, FindInputs) {
    let temp = tempdir().expect("tempdir");
    let addons = temp.path().join("AddOns");
    for folder in ["Spy", "Spy_Extra", "Clique", "Lonely"] {
        let path = addons.join(folder);
        std::fs::create_dir_all(&path).expect("folder");
        std::fs::write(
            path.join(format!("{folder}.toc")),
            format!("## Title: {folder}\n## Version: 1.0\nmain.lua\n"),
        )
        .expect("toc");
        std::fs::write(path.join("main.lua"), format!("-- {folder}\n")).expect("lua");
    }
    let inputs = FindInputs::read(&addons, GameFlavor::Forever).expect("inputs");
    (temp, inputs)
}

/// Wago.io knows Spy and Clique by their files.
fn wago_answer(inputs: &FindInputs) -> SourceLookups {
    let lookups = inputs.wago_lookups(&[], None);
    let hash = |folder: &str| {
        lookups
            .iter()
            .find(|lookup| lookup.folder == folder)
            .map(|lookup| lookup.hash.clone())
            .expect("hash")
    };
    let project = |id: &str, folder: &str| lemonup_core::find::WagoMatch {
        id: id.to_string(),
        name: folder.to_string(),
        website_url: Some(format!("https://addons.wago.io/addons/{id}")),
        author: None,
        modules: vec![(folder.to_string(), hash(folder))],
        matched_version: Some("1.0".to_string()),
        latest_version: Some("1.0".to_string()),
        wowinterface_id: None,
    };
    let mut answer = nothing_found();
    answer.wago = SourceAnswer::Found(vec![project("spy", "Spy"), project("clq", "Clique")]);
    answer
}

/// WoWInterface has a fork of Spy that matches it too, with Spy_Extra: a "Choose one" with
/// Wago.io's Spy.
fn wowinterface_answer() -> SourceLookups {
    let folder = |name: &str| lemonup_core::find::WowiFolder {
        folder: name.to_string(),
        version: Some("1.0".to_string()),
        api_version: Some("16001".to_string()),
    };
    let mut answer = nothing_found();
    answer.wowinterface = SourceAnswer::Found(vec![lemonup_core::find::WowiFile {
        id: "99".to_string(),
        title: "Spy fork".to_string(),
        version: "1.0".to_string(),
        author: None,
        folders: vec![folder("Spy"), folder("Spy_Extra")],
    }]);
    answer
}

fn index_of(app: &App, folder: &str) -> usize {
    results(app)
        .report
        .results
        .iter()
        .position(|result| result.folder == folder)
        .unwrap_or_else(|| panic!("{folder}"))
}

fn label_of(app: &App, folder: &str) -> FindLabel {
    results(app).report.results[index_of(app, folder)].label
}

fn pick_of(app: &App, folder: &str) -> Option<FindSource> {
    let results = results(app);
    let index = index_of(app, folder);
    results.picks[index].map(|pick| results.report.results[index].candidates[pick].source)
}

/// Picks the candidate of `folder` from `source`.
fn pick(app: &mut App, folder: &str, source: FindSource) {
    let index = index_of(app, folder);
    let Some(FindStep::Results(results)) = app.find.step.as_mut() else {
        panic!("results");
    };
    results.picks[index] = results.report.results[index]
        .candidates
        .iter()
        .position(|candidate| candidate.source == source);
}

#[test]
fn f_opens_straight_into_looking_and_asks_only_the_sources_that_are_on() {
    let (_config, _path, mut app) = app_with_config();
    let (_addons, inputs) = spy_inputs();
    app.config_pane.persisted.find_sources = Some(names(&["wowinterface"]));
    app.dashboard =
        DashboardState::from_addons(vec![AddonRecord::new("Spy", "Spy", SourceKind::Manual)]);
    app.dashboard.list_state.select(Some(0));
    app.effective_addon_dir = Some(std::env::temp_dir().join("lemonup-find-test/AddOns"));
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(app.find.step, Some(FindStep::Looking { folders: 1 }));
    assert_eq!(app.find.jobs, vec![FindJob::Read]);
    let shown = screen(&mut app, 80, 24);
    assert_eq!(
        rail_line(&shown, "WoWInterface"),
        "2 [x] WoWInterface   ⠋",
        "{shown}"
    );
    assert_eq!(
        rail_line(&shown, "Wago.io"),
        "1 [ ] Wago.io      off",
        "{shown}"
    );

    // While the folders are read, 3 turns TukUI on; it is asked with the others once they are.
    key(&mut app, KeyCode::Char('3'));
    assert!(asked(&app).is_empty());
    deliver_read(&mut app, inputs);
    assert_eq!(
        asked(&app),
        vec![FindSource::WowInterface, FindSource::Tukui]
    );
    // Looking until one of them answers.
    assert!(app.find.looking());
    deliver_answer(&mut app, FindSource::WowInterface, wowinterface_answer());
    let shown = screen(&mut app, 80, 24);
    assert_eq!(
        rail_sources(&shown),
        [
            "1 [ ] Wago.io      off",
            "2 [x] WoWInterface   1",
            "3 [x] TukUI          ⠋"
        ],
        "{shown}"
    );
    deliver_answer(&mut app, FindSource::Tukui, nothing_found());
    assert_eq!(
        rail_sources(&screen(&mut app, 80, 24)),
        [
            "1 [ ] Wago.io      off",
            "2 [x] WoWInterface   1",
            "3 [x] TukUI          0"
        ]
    );
    assert_eq!(asked(&app).len(), 2);
}

#[test]
fn the_rail_shows_each_source_with_its_state() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    app.shell_ui.motion.tick_count = 0;
    for (width, height) in [(80, 24), (138, 36)] {
        // TukUI's request failed in this report.
        let shown = screen(&mut app, width, height);
        assert_eq!(
            rail_of(&shown),
            [
                "Sources",
                "1 [x] Wago.io        3",
                "2 [x] WoWInterface   3",
                "3 [x] TukUI",
                "      Check failed",
            ],
            "{shown}"
        );
        // "Sources" is on the row of the first group heading.
        let row = shown
            .lines()
            .find(|row| row.contains("Sources"))
            .expect("Sources");
        assert!(row.contains("Ready to link"), "{shown}");
    }
    app.find.sources.toggle(FindSource::WowInterface);
    app.find.asking = vec![FindSource::Tukui];
    app.wago_api_key = None;
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        assert_eq!(
            rail_of(&shown),
            [
                "Sources",
                "1 [-] Wago.io",
                "      Needs a key",
                "2 [ ] WoWInterface off",
                "3 [x] TukUI          ⠋",
            ],
            "{shown}"
        );
    }
}

#[test]
fn the_rail_shows_while_looking() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    app.shell_ui.motion.tick_count = 0;
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        assert_eq!(
            rail_sources(&shown),
            [
                "1 [x] Wago.io        ⠋",
                "2 [x] WoWInterface   ⠋",
                "3 [x] TukUI          ⠋",
            ],
            "{shown}"
        );
        let row = shown
            .lines()
            .find(|row| row.contains("Sources"))
            .expect("Sources");
        assert!(row.contains("Reading"), "{shown}");
    }
}

#[test]
fn the_rail_colors_come_from_the_theme_roles() {
    use ratatui::style::Modifier;
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    app.find.sources.toggle(FindSource::WowInterface);
    let roles = app.ui_theme.roles;
    let sources = cell_at(&mut app, 120, 30, "Sources", 0);
    assert_eq!(sources.fg, roles.muted);
    assert!(sources.modifier.contains(Modifier::BOLD));
    // On: a blue number, a lemon box, the name in text, the count muted.
    let on = "1 [x] Wago.io        3";
    assert_eq!(cell_at(&mut app, 120, 30, on, 0).fg, roles.blue);
    assert_eq!(cell_at(&mut app, 120, 30, on, 2).fg, roles.lemon);
    assert_eq!(cell_at(&mut app, 120, 30, on, 6).fg, roles.text);
    assert_eq!(cell_at(&mut app, 120, 30, on, 21).fg, roles.muted);
    // Off: the box, the name and "off" muted.
    let off = "2 [ ] WoWInterface off";
    assert_eq!(cell_at(&mut app, 120, 30, off, 0).fg, roles.blue);
    for offset in [2, 6, 19] {
        assert_eq!(cell_at(&mut app, 120, 30, off, offset).fg, roles.muted);
    }
    // Failed: "Check failed" in red.
    assert_eq!(cell_at(&mut app, 120, 30, "Check failed", 0).fg, roles.red);
    // Asking: the spinner in lemon.
    app.find.asking = vec![FindSource::Tukui];
    app.shell_ui.motion.tick_count = 0;
    assert_eq!(
        cell_at(&mut app, 120, 30, "3 [x] TukUI          ⠋", 21).fg,
        roles.lemon
    );
    // Needs a key: a muted number, a dim box, the name and the words muted.
    app.wago_api_key = None;
    let key = "1 [-] Wago.io";
    assert_eq!(cell_at(&mut app, 120, 30, key, 0).fg, roles.muted);
    assert_eq!(cell_at(&mut app, 120, 30, key, 2).fg, roles.dim);
    assert_eq!(cell_at(&mut app, 120, 30, key, 6).fg, roles.muted);
    assert_eq!(cell_at(&mut app, 120, 30, "Needs a key", 0).fg, roles.muted);
}

#[test]
fn the_rail_reads_the_same_without_color_and_is_plain_with_simple_symbols() {
    let expected = [
        "Sources",
        "1 [-] Wago.io",
        "      Needs a key",
        "2 [ ] WoWInterface off",
        "3 [x] TukUI          /",
    ];
    for no_color in [false, true] {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.ui_theme = UiTheme::from_mode(ThemeMode::Terminal);
        app.ui_theme.no_color = no_color;
        app.ui_theme.symbols = crate::theme::Symbols::SIMPLE;
        open_find_results(&mut app);
        app.shell_ui.motion.tick_count = 1;
        app.find.sources.toggle(FindSource::WowInterface);
        app.find.asking = vec![FindSource::Tukui];
        app.wago_api_key = None;
        for (width, height) in [(80, 24), (138, 36)] {
            let shown = screen(&mut app, width, height);
            let rail = rail_of(&shown);
            assert_eq!(rail, expected, "{no_color}:\n{shown}");
            assert!(rail.iter().all(|line| line.is_ascii()), "{shown}");
        }
        // A source that failed reads "Check failed".
        app.find.asking.clear();
        let shown = screen(&mut app, 80, 24);
        assert_eq!(
            rail_of(&shown)[3..],
            [
                "2 [ ] WoWInterface off",
                "3 [x] TukUI",
                "      Check failed"
            ],
            "{shown}"
        );
    }
}

#[test]
fn the_wago_notice_is_in_the_rail_wrapped_to_its_width() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    app.find.hash_notice = true;
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        let rail = rail_of(&shown);
        assert_eq!(
            rail[5..],
            ["", "Sends folder names and", "file hashes to", "Wago.io."],
            "{shown}"
        );
        // Nowhere else.
        assert!(!shown.contains(WAGO_HASH_NOTICE), "{shown}");
    }
    // Not while Wago.io is off.
    app.find.sources.toggle(FindSource::Wago);
    assert!(!has_notice(&screen(&mut app, 80, 24)));
}

/// A report with an update ready and a likely match, as `lemonup find` words them.
fn versions_report() -> FindReport {
    let mut clique = found(FindLabel::Exact, FindSource::Wago, "Clique", &["Clique"]);
    clique.latest_version = Some("2.0".to_string());
    let mut pawn = found(
        FindLabel::Likely,
        FindSource::WowInterface,
        "Pawn",
        &["Pawn"],
    );
    pawn.likely_reason = Some(lemonup_core::find::LikelyReason::TocNamesIt);
    FindReport {
        results: vec![
            result(FindLabel::Exact, "Clique", vec![clique]),
            result(FindLabel::Likely, "Pawn", vec![pawn]),
        ],
        failed: Vec::new(),
        skipped: Vec::new(),
        not_asked: Vec::new(),
    }
}

#[test]
fn rows_show_the_version_only_when_the_sheet_is_wide() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    set_results(&mut app, versions_report());
    let pawn = &results(&app).report.results[1];
    let reason = crate::find::result_entry(pawn, &LocalNames::default())
        .expect("entry")
        .version;
    let row = |shown: &str, name: &str| {
        shown
            .lines()
            .position(|row| row.contains(name))
            .unwrap_or_else(|| panic!("{name}:\n{shown}"))
    };

    // At 80 columns: the name, the source, then the note.
    let shown = screen(&mut app, 80, 24);
    let rows = shown.lines().collect::<Vec<_>>();
    let clique = rows[row(&shown, "Clique")];
    assert!(
        clique.contains("Clique  Wago.io       Update ready (2.0)"),
        "{shown}"
    );
    assert!(!clique.contains("1.0"), "{shown}");
    // A likely match keeps what its version says, on the line under it.
    let pawn = row(&shown, "Pawn");
    assert!(!rows[pawn].contains(reason.as_str()), "{shown}");
    let first = reason.split(';').next().expect("reason");
    assert!(
        rows[pawn + 1].trim_start().starts_with(first),
        "{reason}:\n{shown}"
    );

    // Wide: the version between the source and the note.
    let shown = screen(&mut app, 138, 36);
    let rows = shown.lines().collect::<Vec<_>>();
    assert!(
        rows[row(&shown, "Clique")].contains("Clique  Wago.io       1.0  Update ready (2.0)"),
        "{shown}"
    );
    assert!(rows[row(&shown, "Pawn")].contains(first), "{shown}");
}

#[test]
fn the_action_stays_on_the_last_row_of_the_sheet_while_the_report_scrolls() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_tall_report(&mut app, true);
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        let rows = shown.lines().collect::<Vec<_>>();
        let action = rows
            .iter()
            .position(|row| row.contains("Link 1 addon"))
            .unwrap_or_else(|| panic!("{shown}"));
        assert_last_inner_row(&rows, action, width, &shown);
        assert_eq!(rail_of(&shown)[0], "Sources", "{shown}");
        // Longer than the sheet at 80x24.
        if width == 80 {
            assert!(app.sheet_scroll.get().max_offset > 0, "{shown}");
            assert!(shown.contains("↓"), "{shown}");
        }
    }
    // Scrolled to the end, the action and the rail stay in place, and the cursor's row shows.
    let mut shown = String::new();
    for _ in 0..200 {
        shown = press_and_draw(&mut app, KeyCode::Down);
    }
    assert!(shown.contains("ZzLastCopy"), "{shown}");
    let rows = shown.lines().collect::<Vec<_>>();
    let action = rows
        .iter()
        .position(|row| row.contains("Link 1 addon"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert_last_inner_row(&rows, action, 80, &shown);
    assert!(rows[action].contains('▌'), "{shown}");
    assert_eq!(rail_of(&shown)[0], "Sources", "{shown}");
}

/// `action` is the sheet's last inner row: the full screen sheet's rule is right under it, a
/// floating sheet's bottom border one blank row under it.
fn assert_last_inner_row(rows: &[&str], action: usize, width: u16, shown: &str) {
    if width < 100 {
        assert!(
            rows[action + 1].trim().chars().all(|ch| ch == '─'),
            "{shown}"
        );
    } else {
        assert!(
            rows[action + 1].replace('│', "").trim().is_empty(),
            "{shown}"
        );
        assert!(rows[action + 2].contains('╰'), "{shown}");
    }
}

#[test]
fn turning_a_source_off_works_the_report_out_again_and_on_brings_it_back() {
    let (_config, _path, mut app) = app_with_config();
    let (_addons, inputs) = spy_inputs();
    open_find(&mut app);
    deliver_read(&mut app, inputs.clone());
    assert_eq!(asked(&app), FindSource::ALL.to_vec());
    deliver_answer(&mut app, FindSource::Wago, wago_answer(&inputs));
    deliver_answer(&mut app, FindSource::WowInterface, wowinterface_answer());
    deliver_answer(&mut app, FindSource::Tukui, nothing_found());
    assert_eq!(label_of(&app, "Spy"), FindLabel::ChooseOne);
    assert_eq!(pick_of(&app, "Clique"), Some(FindSource::Wago));
    let shown = screen(&mut app, 120, 30);
    assert!(shown.contains("Not found (1)"), "{shown}");
    assert!(!shown.contains("Not found on"), "{shown}");

    // The WoWInterface copy of Spy is picked: turning WoWInterface off drops that pick and
    // keeps the others. Spy is a single match from Wago.io now.
    pick(&mut app, "Spy", FindSource::WowInterface);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(label_of(&app, "Spy"), FindLabel::Exact);
    assert_eq!(pick_of(&app, "Spy"), None);
    assert_eq!(pick_of(&app, "Clique"), Some(FindSource::Wago));
    let shown = screen(&mut app, 120, 30);
    assert_eq!(
        rail_sources(&shown),
        [
            "1 [x] Wago.io        2",
            "2 [ ] WoWInterface off",
            "3 [x] TukUI          0"
        ],
        "{shown}"
    );
    // Not found names the sources that are on.
    assert!(
        shown.contains("Not found on Wago.io and TukUI (2)"),
        "{shown}"
    );

    // On again: its matches are back at once, with no new request.
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(asked(&app).len(), 3);
    assert_eq!(label_of(&app, "Spy"), FindLabel::ChooseOne);
    assert_eq!(pick_of(&app, "Clique"), Some(FindSource::Wago));
    assert!(screen(&mut app, 120, 30).contains("Not found (1)"));

    // A pick whose match stays is kept.
    pick(&mut app, "Spy", FindSource::Wago);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(pick_of(&app, "Spy"), Some(FindSource::Wago));
    assert_eq!(asked(&app).len(), 3);
}

#[test]
fn turning_on_a_source_never_asked_asks_it_once_and_works_the_report_out_when_it_answers() {
    let (_config, path, mut app) = app_with_config();
    let (_addons, inputs) = spy_inputs();
    app.config_pane.persisted.find_sources = Some(names(&["wago", "tukui"]));
    open_find(&mut app);
    deliver_read(&mut app, inputs.clone());
    // No request for a source that is off.
    assert_eq!(asked(&app), vec![FindSource::Wago, FindSource::Tukui]);
    deliver_answer(&mut app, FindSource::Wago, wago_answer(&inputs));
    deliver_answer(&mut app, FindSource::Tukui, nothing_found());
    assert_eq!(label_of(&app, "Spy"), FindLabel::Exact);
    app.shell_ui.motion.tick_count = 0;

    key(&mut app, KeyCode::Char('2'));
    assert_eq!(
        asked(&app),
        vec![
            FindSource::Wago,
            FindSource::Tukui,
            FindSource::WowInterface
        ]
    );
    assert_eq!(
        rail_sources(&screen(&mut app, 80, 24)),
        [
            "1 [x] Wago.io        2",
            "2 [x] WoWInterface   ⠋",
            "3 [x] TukUI          0"
        ]
    );
    // The report waits for the answer.
    assert_eq!(label_of(&app, "Spy"), FindLabel::Exact);
    // Off and on again while it is asked: no second request.
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(asked(&app).len(), 3);

    deliver_answer(&mut app, FindSource::WowInterface, wowinterface_answer());
    assert_eq!(label_of(&app, "Spy"), FindLabel::ChooseOne);
    assert_eq!(
        rail_sources(&screen(&mut app, 80, 24)),
        [
            "1 [x] Wago.io        2",
            "2 [x] WoWInterface   1",
            "3 [x] TukUI          0"
        ]
    );
    assert_eq!(
        saved_find_sources(&path),
        Some(names(&["wago", "wowinterface", "tukui"]))
    );
}

#[test]
fn the_choice_is_saved_for_this_game_and_the_sheet_opens_with_it() {
    let (_config, path, mut app) = app_with_config();
    open_find_results(&mut app);
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(
        saved_find_sources(&path),
        Some(names(&["wago", "wowinterface"]))
    );
    key(&mut app, KeyCode::Esc);
    open_find(&mut app);
    assert!(!app.find.sources.wants(FindSource::Tukui));
    deliver_read(&mut app, no_folders());
    assert_eq!(
        asked(&app),
        vec![FindSource::Wago, FindSource::WowInterface]
    );

    // From the file, in a new session.
    let mut fresh = app_for_tests(ShellMode::Dashboard);
    fresh.config_pane =
        fresh
            .config_pane
            .mark_saved(match ConfigStore::new(path.clone()).load().expect("load") {
                ConfigLoad::Loaded(config) => config,
                ConfigLoad::Missing(_) => panic!("saved"),
            });
    open_find(&mut fresh);
    assert!(!fresh.find.sources.wants(FindSource::Tukui));

    // A save that fails says so; the sheet goes on.
    let temp = tempdir().expect("tempdir");
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, "x").expect("file");
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = ConfigStore::new(blocker.join("config.toml"));
    open_find_results(&mut app);
    key(&mut app, KeyCode::Char('3'));
    assert!(!app.find.sources.wants(FindSource::Tukui));
    assert!(
        app.status_line
            .text
            .contains("Couldn't remember which sources"),
        "{}",
        app.status_line.text
    );
}

#[test]
fn without_a_key_wago_is_never_saved_as_off_and_its_number_does_nothing() {
    let (_config, path, mut app) = app_with_config();
    app.wago_api_key = None;
    app.config_pane.persisted.wago_api_key = None;
    app.config_pane.draft.wago_api_key = None;
    open_find_results(&mut app);
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('1'))),
        vec![]
    );
    let table = app.key_table(160);
    assert!(
        table
            .iter()
            .any(|hint| hint.key == "2 3" && hint.label == "turn on or off")
    );
    assert!(!table.iter().any(|hint| hint.key.contains('1')));
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(
        saved_find_sources(&path),
        Some(names(&["wago", "wowinterface"]))
    );
    // WoWInterface is the only source on: its number does nothing either.
    assert_eq!(
        app.messages_for_key(KeyEvent::from(KeyCode::Char('2'))),
        vec![]
    );

    // With a key later, Wago.io is on again.
    app.wago_api_key = Some("key".to_string());
    open_find(&mut app);
    assert!(app.find_on(FindSource::Wago));

    // Only Wago.io saved and no key: the others are on, and nothing is saved until changed.
    let (_config, path, mut app) = app_with_config();
    app.wago_api_key = None;
    app.config_pane.persisted.find_sources = Some(names(&["wago"]));
    open_find(&mut app);
    deliver_read(&mut app, no_folders());
    assert_eq!(
        asked(&app),
        vec![FindSource::WowInterface, FindSource::Tukui]
    );
    assert!(!path.exists());
}

/// A game whose settings are saved, so `shared.toml` exists.
fn shared_game(base: &std::path::Path) -> (lemonup_core::ProfileRoots, ConfigStore) {
    let roots =
        lemonup_core::ProfileRoots::new(base.join("config"), base.join("data"), base.join("cache"));
    let paths = roots.paths(DEFAULT_PROFILE).expect("paths");
    let store = ConfigStore::for_profile(&paths);
    store
        .write_config(&AppConfig::new_unconfigured())
        .expect("config");
    (roots, store)
}

fn app_in(store: ConfigStore) -> App {
    let mut app = app_for_tests(ShellMode::Dashboard);
    app.config_store = store;
    app
}

#[test]
fn the_wago_notice_shows_the_first_time_wago_is_asked_and_never_again() {
    let temp = tempdir().expect("tempdir");
    let (roots, store) = shared_game(temp.path());
    let (_addons, inputs) = spy_inputs();
    let shared = lemonup_core::SharedSettingsStore::for_roots(&roots);
    let mut app = app_in(store.clone());

    // Nothing to send to Wago.io: no notice yet.
    open_find(&mut app);
    deliver_read(&mut app, no_folders());
    assert!(!has_notice(&screen(&mut app, 80, 24)));
    assert!(!shared.wago_hash_notice_shown());

    // Folders to send: muted lines at the bottom of the rail, under a blank line.
    key(&mut app, KeyCode::Esc);
    open_find(&mut app);
    deliver_read(&mut app, inputs.clone());
    assert!(asked(&app).contains(&FindSource::Wago));
    for (width, height) in [(80, 24), (138, 36)] {
        let shown = screen(&mut app, width, height);
        assert_eq!(rail_notice(&shown), WAGO_HASH_NOTICE, "{shown}");
    }
    assert_eq!(
        cell_at(&mut app, 120, 30, "Sends folder names", 0).fg,
        app.ui_theme.roles.muted
    );
    assert!(shared.wago_hash_notice_shown());
    // It stays in this sheet while Wago.io is on.
    deliver_answer(&mut app, FindSource::Wago, wago_answer(&inputs));
    assert!(has_notice(&screen(&mut app, 80, 24)));
    key(&mut app, KeyCode::Char('1'));
    assert!(!has_notice(&screen(&mut app, 80, 24)));

    // Never again: in this session, nor in a new one.
    key(&mut app, KeyCode::Esc);
    app.config_pane.persisted.find_sources = None;
    open_find(&mut app);
    deliver_read(&mut app, inputs.clone());
    assert!(asked(&app).contains(&FindSource::Wago));
    assert!(!has_notice(&screen(&mut app, 80, 24)));
    let mut fresh = app_in(store);
    open_find(&mut fresh);
    deliver_read(&mut fresh, inputs);
    assert!(!has_notice(&screen(&mut fresh, 80, 24)));

    // Help keeps a row about it.
    assert!(
        crate::app::keymap::help_page(HelpContext::Find)
            .sections()
            .flat_map(|section| section.rows.iter())
            .any(|row| row.text.contains("sends folder names and file hashes"))
    );
}

#[test]
fn a_wago_notice_that_cannot_be_saved_still_shows_and_the_lookup_goes_on() {
    let temp = tempdir().expect("tempdir");
    let (roots, store) = shared_game(temp.path());
    // The lock beside shared.toml is a folder: nothing can be saved there.
    let lock = roots
        .shared_settings_file()
        .with_file_name("shared.toml.lock");
    let _ = std::fs::remove_file(&lock);
    std::fs::create_dir_all(&lock).expect("block the lock");
    let (_addons, inputs) = spy_inputs();
    let mut app = app_in(store.clone());
    open_find(&mut app);
    deliver_read(&mut app, inputs.clone());
    assert!(has_notice(&screen(&mut app, 80, 24)));
    deliver_answer(&mut app, FindSource::Wago, wago_answer(&inputs));
    assert_eq!(label_of(&app, "Clique"), FindLabel::Exact);
    // Not saved: a new session shows it again, which is fine.
    assert!(!lemonup_core::SharedSettingsStore::for_roots(&roots).wago_hash_notice_shown());
    let mut fresh = app_in(store);
    open_find(&mut fresh);
    deliver_read(&mut fresh, inputs);
    assert!(has_notice(&screen(&mut fresh, 80, 24)));
}

#[test]
fn the_name_column_fits_the_longest_name_when_there_is_room() {
    let long = "HandyNotes: The War Within Treasures";
    assert_eq!(long.chars().count(), 36);
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    set_results(
        &mut app,
        FindReport {
            results: vec![
                result(
                    FindLabel::Exact,
                    "HandyNotes_TheWarWithin",
                    vec![found(
                        FindLabel::Exact,
                        FindSource::Wago,
                        long,
                        &["HandyNotes_TheWarWithin"],
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
            ],
            failed: Vec::new(),
            skipped: Vec::new(),
            not_asked: Vec::new(),
        },
    );
    let column = |row: &str, text: &str| row[..row.find(text).expect("text")].chars().count();
    for (width, height) in [(138, 36), (120, 30)] {
        let shown = screen(&mut app, width, height);
        let row = shown
            .lines()
            .find(|row| row.contains("HandyNotes"))
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(row.contains(&format!("{long}  Wago.io")), "{shown}");
        // The versions line up under each other.
        let clique = shown
            .lines()
            .find(|row| row.contains("Clique") && row.contains("1.0"))
            .expect("Clique");
        assert_eq!(column(row, "1.0"), column(clique, "1.0"), "{shown}");
    }
    // At 80 columns the rail leaves the name 34 columns; the sources line up.
    let shown = screen(&mut app, 80, 24);
    let row = shown
        .lines()
        .find(|row| row.contains("HandyNotes"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert!(
        row.contains("HandyNotes: The War Within Treasu…  Wago.io"),
        "{shown}"
    );
    let clique = shown
        .lines()
        .find(|row| row.contains("Clique"))
        .expect("Clique");
    assert_eq!(
        column(row, "Wago.io"),
        column(clique, "WoWInterface"),
        "{shown}"
    );
}

#[test]
fn the_action_lines_up_with_the_names_under_a_blank_line() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    for (width, height) in [(120, 30), (80, 24)] {
        let shown = screen(&mut app, width, height);
        let rows = shown.lines().collect::<Vec<_>>();
        let column = |row: &str, text: &str| row[..row.find(text).expect("text")].chars().count();
        let action = rows
            .iter()
            .position(|row| row.contains("Link 1 addon"))
            .unwrap_or_else(|| panic!("{shown}"));
        let clique = rows
            .iter()
            .find(|row| row.contains("Clique") && row.contains("WoWInterface"))
            .expect("Clique");
        assert_eq!(
            column(rows[action], "Link 1 addon"),
            column(clique, "Clique"),
            "{shown}"
        );
        let above = rows[action - 1].replace(['│', '▌'], "");
        assert!(above.trim().is_empty(), "{shown}");
    }

    // A short report: the action stays on the sheet's last row, a blank line above it.
    set_results(
        &mut app,
        FindReport {
            results: vec![result(
                FindLabel::Exact,
                "Clique",
                vec![found(
                    FindLabel::Exact,
                    FindSource::WowInterface,
                    "Clique",
                    &["Clique"],
                )],
            )],
            failed: Vec::new(),
            skipped: Vec::new(),
            not_asked: Vec::new(),
        },
    );
    for (width, height) in [(120, 30), (80, 24)] {
        let shown = screen(&mut app, width, height);
        let rows = shown.lines().collect::<Vec<_>>();
        let action = rows
            .iter()
            .position(|row| row.contains("Link 1 addon"))
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(
            rows[action - 1].replace('│', "").trim().is_empty(),
            "{shown}"
        );
        assert_last_inner_row(&rows, action, width, &shown);
    }
}

#[test]
fn the_sheet_shows_no_key_hints_of_its_own() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let looking = render_rows(&mut app, 120, 30);
    set_results(&mut app, report());
    let on_result = render_rows(&mut app, 120, 30);
    apply_all(&mut app, AppMessage::Find(FindMessage::Last));
    let on_action = render_rows(&mut app, 120, 30);
    app.find.step = Some(FindStep::Failed("It broke.".to_string()));
    let failed = render_rows(&mut app, 120, 30);
    for rows in [looking, on_result, on_action, failed] {
        let (key_line, sheet) = rows.split_last().expect("rows");
        // Below the title, which every sheet has.
        let sheet = sheet
            .iter()
            .filter(|row| !row.contains("Find your addons online"))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        for hint in [
            "esc",
            "space",
            "enter",
            "nothing changes",
            "asks before",
            "selects",
            "press space",
        ] {
            assert!(!sheet.contains(hint), "{hint}:\n{sheet}");
        }
        assert!(key_line.contains("esc"), "{key_line}");
    }
}

#[test]
fn the_key_line_shows_the_keys_of_the_row_under_the_cursor() {
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find(&mut app);
    let pairs = |app: &App| {
        app.key_table(200)
            .iter()
            .map(|hint| format!("{} {}", hint.key, hint.label))
            .collect::<Vec<_>>()
    };
    // Looking: the sources and esc.
    let looking = pairs(&app);
    assert!(
        looking.contains(&"1 2 3 turn on or off".to_string()),
        "{looking:?}"
    );
    assert!(looking.contains(&"esc cancel".to_string()), "{looking:?}");
    assert!(
        !looking.iter().any(|pair| pair.starts_with("enter")),
        "{looking:?}"
    );

    // A match: move, space, the sources, close; enter does nothing there.
    set_results(&mut app, report());
    let on_match = pairs(&app);
    for expected in [
        "↑/↓ move",
        "space unselect",
        "1 2 3 turn on or off",
        "esc close",
    ] {
        assert!(on_match.contains(&expected.to_string()), "{on_match:?}");
    }
    assert!(
        !on_match.iter().any(|pair| pair.starts_with("enter")),
        "{on_match:?}"
    );

    // A "Choose one" row: enter picks.
    let choose = index_of(&app, "QuestieDev");
    let Some(FindStep::Results(results)) = app.find.step.as_mut() else {
        panic!("results");
    };
    results.cursor = FindFocus::Result(choose);
    let on_choice = pairs(&app);
    assert!(
        on_choice.contains(&"enter pick".to_string()),
        "{on_choice:?}"
    );
    assert!(
        !on_choice.iter().any(|pair| pair.starts_with("space")),
        "{on_choice:?}"
    );

    // The action: enter links.
    apply_all(&mut app, AppMessage::Find(FindMessage::Last));
    let on_action = pairs(&app);
    assert!(
        on_action.contains(&"enter link".to_string()),
        "{on_action:?}"
    );
    assert!(
        on_action.contains(&"1 2 3 turn on or off".to_string()),
        "{on_action:?}"
    );
    assert!(
        !on_action.iter().any(|pair| pair.starts_with("space")),
        "{on_action:?}"
    );

    // Each number pulses the hint that answers to it.
    for digit in ['1', '2', '3'] {
        assert!(
            app.footer_hint_for_key(KeyEvent::from(KeyCode::Char(digit)))
                .is_some(),
            "{digit}"
        );
    }
}

#[test]
fn help_lists_the_source_keys() {
    let help = crate::app::keymap::help_page(HelpContext::Find);
    let rows = help
        .sections()
        .flat_map(|section| section.rows.iter())
        .collect::<Vec<_>>();
    for (digit, text) in [
        ('1', "Wago.io on or off"),
        ('2', "WoWInterface on or off"),
        ('3', "TukUI on or off"),
    ] {
        assert!(
            rows.iter()
                .any(|row| row.keys == [KeyEvent::from(KeyCode::Char(digit))]
                    && row.text.starts_with(text)),
            "{digit}"
        );
    }
    let mut app = app_for_tests(ShellMode::Dashboard);
    open_find_results(&mut app);
    // The key line names what the numbers do, and Help has a row for each of them.
    let hint = app
        .key_table(200)
        .into_iter()
        .find(|hint| hint.label == "turn on or off")
        .expect("the sources hint");
    assert_eq!(hint.key, "1 2 3");
    for digit in ['1', '2', '3'] {
        assert!(
            rows.iter()
                .any(|row| row.keys == [KeyEvent::from(KeyCode::Char(digit))]),
            "{digit}"
        );
    }
    key(&mut app, KeyCode::Char('?'));
    let shown = screen(&mut app, 138, 36);
    for line in [
        "Wago.io on or off",
        "WoWInterface on or off",
        "TukUI on or off",
    ] {
        assert!(shown.contains(line), "{shown}");
    }
    assert!(!shown.contains("look up the sources"), "{shown}");
}

fn only_not_found(not_asked: Vec<FindSource>) -> FindReport {
    FindReport {
        results: vec![
            result(FindLabel::NotFound, "Lonely", Vec::new()),
            result(FindLabel::NotFound, "Alone", Vec::new()),
        ],
        failed: Vec::new(),
        skipped: not_asked
            .contains(&FindSource::Wago)
            .then_some(FindSource::Wago)
            .into_iter()
            .collect(),
        not_asked,
    }
}

#[test]
fn not_found_names_the_sources_it_asked_when_it_did_not_ask_them_all() {
    for (not_asked, heading) in [
        (Vec::new(), "Not found (2)"),
        (
            vec![FindSource::WowInterface],
            "Not found on Wago.io and TukUI (2)",
        ),
        // Wago.io without a key was not asked either.
        (
            vec![FindSource::Wago],
            "Not found on WoWInterface and TukUI (2)",
        ),
        (
            vec![FindSource::Wago, FindSource::Tukui],
            "Not found on WoWInterface (2)",
        ),
    ] {
        for (width, height) in [(80, 24), (138, 36)] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_find(&mut app);
            set_results(&mut app, only_not_found(not_asked.clone()));
            let shown = screen(&mut app, width, height);
            assert!(shown.contains(heading), "{heading} at {width}:\n{shown}");
            if not_asked.is_empty() {
                assert!(!shown.contains("Not found on"), "{shown}");
            }
        }
    }
}
