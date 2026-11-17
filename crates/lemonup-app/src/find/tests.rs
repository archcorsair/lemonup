use std::fs::File;
use std::path::PathBuf;

use lemonup_core::find::{FindInputs, match_addons};
use lemonup_core::{OwnedFolder, SourceKind, UpdateStatus};

use super::*;

/// A WoWInterface file list of `count` files, numbered from `first`.
fn wowi_list(first: usize, count: usize) -> String {
    let files = (first..first + count)
        .map(|id| {
            format!(
                r#"{{"id":{id},"title":"A{id}","version":"1","addons":[{{"path":"A{id}","addOnVersion":"1","apiVersion":"16001"}}]}}"#
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", files.join(","))
}

fn list_a() -> String {
    wowi_list(1, WOWI_MIN_FILES)
}

fn list_b() -> String {
    wowi_list(5001, WOWI_MIN_FILES)
}

fn cached_list(dir: &Path, age: Duration) -> (PathBuf, SystemTime) {
    write_cached(dir, &list_a(), age)
}

fn write_cached(dir: &Path, list: &str, age: Duration) -> (PathBuf, SystemTime) {
    let path = dir.join(WOWI_CACHE_FILE);
    fs::write(&path, list).expect("cache");
    let modified = SystemTime::now() - age;
    File::options()
        .write(true)
        .open(&path)
        .expect("open cache")
        .set_modified(modified)
        .expect("set mtime");
    (path, modified)
}

#[tokio::test]
async fn the_wowinterface_list_is_reused_within_an_hour() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, modified) = cached_list(dir.path(), Duration::from_secs(10 * 60));

    let files = load_wowi_files(&path, modified + Duration::from_secs(50 * 60), || async {
        panic!("a fresh cache must not be downloaded again")
    })
    .await
    .expect("cached list");

    assert_eq!(files.len(), WOWI_MIN_FILES);
    assert_eq!(files[0].id, "1");
}

#[tokio::test]
async fn an_hour_old_wowinterface_list_is_downloaded_again_and_cached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, modified) = cached_list(dir.path(), Duration::from_secs(2 * 60 * 60));

    let files = load_wowi_files(&path, modified + WOWI_CACHE_TTL, || async {
        Ok(list_b().into_bytes())
    })
    .await
    .expect("downloaded list");

    assert_eq!(files[0].id, "5001");
    assert_eq!(fs::read_to_string(&path).expect("cache"), list_b());
}

#[tokio::test]
async fn an_empty_or_short_wowinterface_list_is_an_error_and_is_not_cached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache").join(WOWI_CACHE_FILE);

    for list in ["[]".to_string(), wowi_list(1, WOWI_MIN_FILES - 1)] {
        let error = load_wowi_files(&path, SystemTime::now(), || async { Ok(list.into_bytes()) })
            .await
            .expect_err("too short");

        assert_eq!(error.kind, SourceErrorKind::Other);
        assert!(error.detail.contains("only"), "{}", error.detail);
        assert!(!path.exists());
    }
}

#[tokio::test]
async fn a_short_cached_wowinterface_list_is_downloaded_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, modified) = write_cached(dir.path(), "[]", Duration::from_secs(60));

    let files = load_wowi_files(&path, modified + Duration::from_secs(60), || async {
        Ok(list_b().into_bytes())
    })
    .await
    .expect("downloaded list");

    assert_eq!(files[0].id, "5001");
}

#[tokio::test]
async fn a_failed_download_is_an_error_and_an_unreadable_one_is_not_cached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache").join(WOWI_CACHE_FILE);

    let offline = load_wowi_files(&path, SystemTime::now(), || async {
        Err(SourceError::new(SourceErrorKind::NoConnection, "offline"))
    })
    .await
    .expect_err("offline");
    let garbage = load_wowi_files(&path, SystemTime::now(), || async {
        Ok(b"<html>".to_vec())
    })
    .await
    .expect_err("not a list");

    assert_eq!(offline.kind, SourceErrorKind::NoConnection);
    assert_eq!(garbage.kind, SourceErrorKind::Other);
    assert!(!path.exists());
}

struct Sandbox {
    _dir: tempfile::TempDir,
    addons: PathBuf,
    database: StateDatabase,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().expect("tempdir");
    let addons = dir.path().join("AddOns");
    fs::create_dir_all(&addons).expect("addons");
    let database = StateDatabase::open(dir.path().join("state.sqlite")).expect("database");
    Sandbox {
        _dir: dir,
        addons,
        database,
    }
}

fn addon(addons: &Path, folder: &str, version: &str) {
    let path = addons.join(folder);
    fs::create_dir_all(&path).expect("folder");
    fs::write(
        path.join(format!("{folder}.toc")),
        format!("## Title: {folder}\n## Version: {version}\nmain.lua\n"),
    )
    .expect("toc");
    fs::write(path.join("main.lua"), format!("-- {folder}\n")).expect("lua");
}

fn manual(folder: &str, owned: &[&str]) -> AddonRecord {
    let mut row = AddonRecord::new(folder, folder, SourceKind::Manual);
    row.set_scan_owned_folders(
        owned
            .iter()
            .map(|name| OwnedFolder {
                name: name.to_string(),
            })
            .collect(),
    );
    row
}

fn wago_match(inputs: &FindInputs, id: &str, folders: &[&str], latest: &str) -> WagoMatch {
    WagoMatch {
        id: id.to_string(),
        name: folders[0].to_string(),
        website_url: None,
        author: None,
        modules: folders
            .iter()
            .map(|folder| {
                let local = inputs.get(folder).expect("folder");
                (
                    folder.to_string(),
                    local.fingerprint.as_ref().expect("hash").hash.clone(),
                )
            })
            .collect(),
        matched_version: Some("1.0".to_string()),
        latest_version: Some(latest.to_string()),
        wowinterface_id: None,
    }
}

/// Pawn is an exact match on its own; GatherMate2 is an exact match whose source also lists a
/// folder LemonUp has as a separate addon. Spy is found nowhere.
fn report_for(sandbox: &Sandbox) -> FindReport {
    addon(&sandbox.addons, "Pawn", "1.0");
    addon(&sandbox.addons, "GatherMate2", "2.0");
    addon(&sandbox.addons, "GatherMate2Storage_Midnight", "2.0");
    addon(&sandbox.addons, "Spy", "3.0");
    for row in [
        manual("Pawn", &[]),
        manual("GatherMate2", &[]),
        manual("GatherMate2Storage_Midnight", &[]),
        manual("Spy", &[]),
    ] {
        sandbox.database.upsert_addon(&row).expect("row");
    }
    let inputs = FindInputs::read(&sandbox.addons, GameFlavor::Forever).expect("inputs");
    let rows = sandbox.database.list_addons().expect("rows");
    let lookups = SourceLookups {
        wago: SourceAnswer::Found(vec![
            wago_match(&inputs, "PAWN", &["Pawn"], "1.1"),
            wago_match(
                &inputs,
                "GM2",
                &["GatherMate2", "GatherMate2Storage_Midnight"],
                "1.0",
            ),
        ]),
        wowinterface: SourceAnswer::Failed,
        tukui: SourceAnswer::Found(Vec::new()),
    };
    match_addons(&inputs, &rows, None, &lookups)
}

/// `--link` with `--plain`: what it prints, or the refusal.
fn link_plain(
    sandbox: &mut Sandbox,
    report: &FindReport,
    yes: bool,
) -> Result<Vec<String>, LemonupError> {
    plain_link_lines(link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        report,
        yes,
    )?)
}

#[test]
fn find_lists_results_by_label_with_a_summary_and_source_problems() {
    let sandbox = sandbox();
    let report = report_for(&sandbox);
    let problems = vec![SourceProblem {
        source: FindSource::WowInterface,
        error: SourceError::new(SourceErrorKind::NoConnection, "timed out"),
    }];

    let lines = report_lines(&report, &problems, 4);

    assert_eq!(
        lines[0],
        "find warning: source=WoWInterface, problem=no_connection, detail=timed out"
    );
    assert_eq!(
        lines[1],
        "find summary: folders=4, addons=3, exact=2, likely=0, partial=0, choose_one=0, not_checked=1, curseforge_only=0, not_found=0, developer_copy=0"
    );
    assert!(lines[2].starts_with(
        "find result: label=exact, addon=GatherMate2, folder=GatherMate2, source=wago, id=GM2, version=1.0, latest=1.0, folders=GatherMate2|GatherMate2Storage_Midnight, joining=GatherMate2Storage_Midnight (own addon)"
    ));
    assert!(lines[3].starts_with("find result: label=exact, addon=Pawn, folder=Pawn"));
    assert_eq!(
        lines[4],
        "find result: label=not_checked, addon=Spy, folder=Spy, folders=Spy, not_checked_by=WoWInterface"
    );
}

#[test]
fn without_a_wago_key_find_says_a_key_finds_more() {
    let report = FindReport {
        results: Vec::new(),
        failed: Vec::new(),
        skipped: vec![FindSource::Wago],
    };

    let lines = report_lines(&report, &[], 0);

    assert_eq!(
        lines[0],
        "find note: A Wago.io key finds more of your addons. Add one in Settings."
    );
}

#[test]
fn find_link_refuses_without_yes_when_an_exact_match_takes_other_folders() {
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);
    let before = sandbox.database.list_addons().expect("rows");

    let error = link_plain(&mut sandbox, &report, false).expect_err("needs --yes");

    let message = error.to_string();
    assert!(
        message.contains("GatherMate2 (GatherMate2Storage_Midnight joins (now its own addon))"),
        "{message}"
    );
    assert!(message.contains("--yes"), "{message}");
    assert!(!message.contains("Pawn"), "{message}");
    assert_eq!(sandbox.database.list_addons().expect("rows"), before);
}

#[test]
fn find_link_with_yes_links_exact_matches_with_their_status_right_away() {
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);

    let lines = link_plain(&mut sandbox, &report, true).expect("linked");

    assert_eq!(
        lines,
        [
            "find linked: addon=GatherMate2, source=wago, version=1.0, latest=1.0, status=up_to_date, folders=GatherMate2|GatherMate2Storage_Midnight",
            "find linked: addon=Pawn, source=wago, version=1.0, latest=1.1, status=update_available, folders=Pawn",
            "find link summary: linked=2",
        ]
    );
    let rows = sandbox.database.list_addons().expect("rows");
    let folders = rows
        .iter()
        .map(|row| row.folder.as_str())
        .collect::<Vec<_>>();
    assert_eq!(folders, ["GatherMate2", "Pawn", "Spy"]);
    let pawn = rows.iter().find(|row| row.folder == "Pawn").expect("pawn");
    assert_eq!(
        crate::update::determine_update_status(pawn).0,
        UpdateStatus::UpdateAvailable
    );
    let spy = rows.iter().find(|row| row.folder == "Spy").expect("spy");
    assert_eq!(spy.source, SourceKind::Manual);
}

#[test]
fn find_link_reports_every_pick_the_database_holds_when_one_folder_moves_between_them() {
    let mut sandbox = sandbox();
    addon(&sandbox.addons, "GatherMate2", "2.0");
    addon(&sandbox.addons, "GatherMate2_Data", "1");
    sandbox
        .database
        .upsert_addon(&manual("GatherMate2", &["GatherMate2_Data"]))
        .expect("row");
    let inputs = FindInputs::read(&sandbox.addons, GameFlavor::Forever).expect("inputs");
    let rows = sandbox.database.list_addons().expect("rows");
    let mut data = wago_match(&inputs, "GMD", &["GatherMate2_Data"], "1.0");
    data.name = "AAA GatherMate2 Data".to_string();
    let lookups = SourceLookups {
        wago: SourceAnswer::Found(vec![data]),
        wowinterface: SourceAnswer::Found(vec![WowiFile {
            id: "7".to_string(),
            title: "GatherMate2".to_string(),
            version: "2.0".to_string(),
            author: None,
            folders: vec![lemonup_core::find::WowiFolder {
                folder: "GatherMate2".to_string(),
                version: Some("2.0".to_string()),
                api_version: Some("16001".to_string()),
            }],
        }]),
        tukui: SourceAnswer::Found(Vec::new()),
    };
    let report = match_addons(&inputs, &rows, None, &lookups);

    let lines = link_plain(&mut sandbox, &report, true).expect("linked");

    assert_eq!(
        lines,
        [
            "find linked: addon=GatherMate2_Data, source=wago, version=1.0, latest=1.0, status=up_to_date, folders=GatherMate2_Data",
            "find linked: addon=GatherMate2, source=wowinterface, version=2.0, latest=2.0, status=up_to_date, folders=GatherMate2",
            "find link summary: linked=2",
        ]
    );
    let rows = sandbox.database.list_addons().expect("rows");
    assert_eq!(
        rows.iter()
            .map(|row| (row.folder.as_str(), row.source))
            .collect::<Vec<_>>(),
        [
            ("GatherMate2", SourceKind::WowInterface),
            ("GatherMate2_Data", SourceKind::Wago),
        ]
    );
}

#[test]
fn find_link_refuses_without_yes_even_before_the_addons_folder_was_scanned() {
    let mut sandbox = sandbox();
    addon(&sandbox.addons, "GatherMate2", "2.0");
    addon(&sandbox.addons, "GatherMate2Storage_Midnight", "2.0");
    let inputs = FindInputs::read(&sandbox.addons, GameFlavor::Forever).expect("inputs");
    let lookups = SourceLookups {
        wago: SourceAnswer::Found(vec![wago_match(
            &inputs,
            "GM2",
            &["GatherMate2", "GatherMate2Storage_Midnight"],
            "1.0",
        )]),
        wowinterface: SourceAnswer::Found(Vec::new()),
        tukui: SourceAnswer::Found(Vec::new()),
    };
    let report = match_addons(&inputs, &[], None, &lookups);

    let error = link_plain(&mut sandbox, &report, false).expect_err("needs --yes");

    let message = error.to_string();
    assert!(
        message.contains("GatherMate2 (GatherMate2Storage_Midnight joins (now its own addon))"),
        "{message}"
    );
    assert!(sandbox.database.list_addons().expect("rows").is_empty());
}

#[test]
fn find_link_scans_first_and_a_failed_scan_keeps_the_rows_and_links_nothing() {
    let mut sandbox = sandbox();
    addon(&sandbox.addons, "Pawn", "1.0");
    sandbox
        .database
        .upsert_addon(&manual("Spy", &[]))
        .expect("row");
    let before = sandbox.database.list_addons().expect("rows");

    let error = scan_before_linking(
        &mut sandbox.database,
        &sandbox.addons.join("Missing"),
        GameFlavor::Forever,
    )
    .expect_err("no AddOns folder");

    assert!(error.to_string().contains("nothing was linked"), "{error}");
    assert_eq!(sandbox.database.list_addons().expect("rows"), before);

    scan_before_linking(&mut sandbox.database, &sandbox.addons, GameFlavor::Forever).expect("scan");
    let folders = sandbox
        .database
        .list_addons()
        .expect("rows")
        .into_iter()
        .map(|row| row.folder)
        .collect::<Vec<_>>();
    assert_eq!(folders, ["Pawn"]);
}

#[test]
fn a_wago_answer_with_unreadable_entries_is_incomplete_and_reported() {
    let mut problems = Vec::new();
    let complete = wago_answer(&mut problems, Ok(wire::WagoAnswer::default()));
    assert_eq!(complete, SourceAnswer::Found(Vec::new()));
    assert!(problems.is_empty());

    let incomplete = wago_answer(
        &mut problems,
        Ok(wire::WagoAnswer {
            matches: Vec::new(),
            unreadable: 2,
        }),
    );

    assert_eq!(incomplete, SourceAnswer::Incomplete(Vec::new()));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].source, FindSource::Wago);
    assert_eq!(problems[0].error.kind, SourceErrorKind::Other);
    assert_eq!(
        problems[0].error.detail,
        "Wago.io sent 2 matches LemonUp can't read"
    );
}

// ---------------------------------------------------------------------------------------------
// The readable report.

fn found(source: FindSource, name: &str, folder: &str, version: &str, latest: &str) -> FoundAddon {
    FoundAddon {
        label: FindLabel::Exact,
        source,
        source_id: "1".to_string(),
        source_url: String::new(),
        name: name.to_string(),
        author: None,
        parent_folder: folder.to_string(),
        folders: vec![folder.to_string()],
        matched_version: Some(version.to_string()),
        latest_version: Some(latest.to_string()),
        joining: Vec::new(),
        leaving: Vec::new(),
        missing: Vec::new(),
        differing: Vec::new(),
        blocked: Vec::new(),
    }
}

fn result_of(
    label: FindLabel,
    name: &str,
    folder: &str,
    candidates: Vec<FoundAddon>,
) -> FindResult {
    FindResult {
        label,
        name: name.to_string(),
        folder: folder.to_string(),
        folders: vec![folder.to_string()],
        candidates,
        curseforge_id: None,
        not_checked_by: Vec::new(),
    }
}

fn exact(found: FoundAddon) -> FindResult {
    result_of(
        FindLabel::Exact,
        &found.name.clone(),
        &found.parent_folder.clone(),
        vec![found],
    )
}

fn only(label: FindLabel, name: &str) -> FindResult {
    result_of(label, name, name, Vec::new())
}

fn folder_move(folder: &str, from: &str) -> lemonup_core::find::FolderMove {
    lemonup_core::find::FolderMove {
        folder: folder.to_string(),
        from: from.to_string(),
    }
}

/// A report with a result for every section.
fn full_report() -> FindReport {
    let mut gather = found(
        FindSource::WowInterface,
        "GatherMate 2",
        "GatherMate2",
        "1.52.7",
        "1.52.7",
    );
    gather.joining = vec![
        folder_move(
            "GatherMate2Storage_Dragonflight",
            "GatherMate2Storage_Dragonflight",
        ),
        folder_move("GatherMate2Storage_Midnight", "GatherMate2Storage_Midnight"),
        folder_move(
            "GatherMate2Storage_Shadowlands",
            "GatherMate2Storage_Shadowlands",
        ),
    ];
    gather.leaving = vec!["GatherMate2_Data".to_string()];
    gather.missing = vec!["GatherMate2Storage_Gone".to_string()];

    // An addon whose own folder is grouped under the GatherMate2 row now.
    let mut gather_data = found(
        FindSource::WowInterface,
        "GatherMate2Data",
        "GatherMate2_Data",
        "1.0",
        "1.0",
    );
    gather_data.joining = vec![folder_move("GatherMate2_Data", "GatherMate2")];

    let mut blocked = found(FindSource::Wago, "Details", "Details", "1.0", "1.0");
    blocked.blocked = vec![lemonup_core::find::BlockedFolder {
        folder: "Details_Shared".to_string(),
        reason: lemonup_core::find::BlockReason::Linked {
            addon: "Details Core".to_string(),
        },
    }];

    let mut likely = found(
        FindSource::WowInterface,
        "Bartender4",
        "Bartender4",
        "4.0",
        "4.1",
    );
    likely.label = FindLabel::Likely;
    likely.matched_version = None;

    let mut partial = found(FindSource::Wago, "Plater", "Plater", "2.0", "2.1");
    partial.label = FindLabel::Partial;
    partial.folders = vec!["Plater".to_string(), "Plater_Libs".to_string()];
    partial.differing = vec!["Plater_Libs".to_string()];

    let mut questie = found(FindSource::Wago, "Questie", "Questie", "9.0", "9.0");
    questie.label = FindLabel::Partial;
    questie.differing = vec!["QuestieDB".to_string()];
    let mut questie_dev = found(FindSource::Wago, "Questie Dev", "QuestieDev", "9.0", "9.0");
    questie_dev.label = FindLabel::Likely;

    let mut results = vec![
        exact(found(
            FindSource::Wago,
            "Ask Mr Robot",
            "AskMrRobot",
            "v1003",
            "v1003",
        )),
        exact(found(FindSource::Wago, "APR", "APR", "v7.7.1", "v7.8.0")),
        exact(found(FindSource::Tukui, "ElvUI", "ElvUI", "16.01", "16.01")),
        exact(gather),
        exact(gather_data),
        exact(blocked),
        result_of(FindLabel::Likely, "Bartender4", "Bartender4", vec![likely]),
        result_of(FindLabel::Partial, "Plater", "Plater", vec![partial]),
        result_of(
            FindLabel::ChooseOne,
            "Questie",
            "Questie",
            vec![questie_dev, questie],
        ),
    ];
    let mut spy = only(FindLabel::NotChecked, "Spy");
    spy.not_checked_by = vec![FindSource::WowInterface];
    results.push(spy);
    for folder in [
        "ForeverAuras",
        "ForeverAuras_Options",
        "ForeverAuras_Libs",
        "ForeverAuras_Media",
        "ForeverAuras_Extra",
    ] {
        let mut result = only(FindLabel::CurseForgeOnly, folder);
        result.curseforge_id = Some("9001".to_string());
        results.push(result);
    }
    for name in [
        "Forever Loot Sparkles",
        "Leatrix Plus",
        "Leatrix Sounds",
        "OpenMap",
        "Another Long Addon Name",
        "Yet Another Addon",
    ] {
        let mut result = only(FindLabel::CurseForgeOnly, name);
        result.curseforge_id = Some(format!("id-{name}"));
        results.push(result);
    }
    results.push(only(FindLabel::NotFound, "AzerothGPS"));
    results.push(only(FindLabel::NotFound, "DungeonJournal"));
    results.push(only(FindLabel::DeveloperCopy, "WoW Butler"));
    FindReport {
        results,
        failed: vec![FindSource::WowInterface],
        skipped: Vec::new(),
    }
}

/// Names LemonUp shows for the folders of [`full_report`].
fn full_names() -> LocalNames {
    let rows = [
        ("Ask Mr. Robot Classic", "AskMrRobot"),
        ("Azeroth Pilot Reloaded", "APR"),
        ("ElvUI Forever", "ElvUI"),
        ("GatherMate2", "GatherMate2"),
    ]
    .map(|(name, folder)| AddonRecord::new(name, folder, SourceKind::Manual));
    LocalNames::new(
        &rows,
        &FindInputs {
            flavor: GameFlavor::Forever,
            folders: Vec::new(),
        },
    )
}

fn wowinterface_down() -> Vec<SourceProblem> {
    vec![SourceProblem {
        source: FindSource::WowInterface,
        error: SourceError::new(SourceErrorKind::NoConnection, "timed out"),
    }]
}

/// The lines of one section of the readable report, without its heading.
fn section_lines<'a>(lines: &'a [String], heading: &str) -> Vec<&'a str> {
    lines
        .iter()
        .skip_while(|line| line.as_str() != heading)
        .skip(1)
        .take_while(|line| !line.is_empty())
        .map(String::as_str)
        .collect()
}

#[test]
fn the_readable_report_has_a_section_for_each_label_in_order() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    assert_eq!(lines[0], "Sent 44 folder names and file hashes to Wago.io.");
    assert_eq!(lines[1], "");
    let headings = lines
        .iter()
        .filter(|line| !line.starts_with(' ') && line.ends_with(')'))
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        headings,
        [
            "Ready to link (5)",
            "Can't link yet (1)",
            "Likely matches (1)",
            "Partial matches (1)",
            "Choose one in LemonUp (1)",
            "Not checked (1)",
            "On CurseForge only, LemonUp can't update these yet (11)",
            "Not found (2)",
            "Skipped, developer copy (1)",
        ]
    );
    // One blank line between sections, none at the end, and no key=value lines.
    assert_ne!(lines.last().expect("last"), "");
    assert!(lines.iter().all(|line| !line.contains("find result")));
    assert!(
        lines
            .windows(2)
            .all(|pair| !(pair[0].is_empty() && pair[1].is_empty()))
    );
}

#[test]
fn ready_lines_use_the_name_lemonup_shows_and_align_their_columns() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    assert_eq!(
        section_lines(&lines, "Ready to link (5)"),
        [
            "  Ask Mr. Robot Classic   Wago.io       v1003",
            "  Azeroth Pilot Reloaded  Wago.io       v7.7.1, update ready (v7.8.0)",
            "  ElvUI Forever           TukUI         16.01",
            "  GatherMate2             WoWInterface  1.52.7",
            "      also takes GatherMate2Storage_Dragonflight, GatherMate2Storage_Midnight,",
            "      GatherMate2Storage_Shadowlands",
            "      GatherMate2_Data becomes its own addon",
            "      GatherMate2Storage_Gone missing on disk",
            "  GatherMate2Data         WoWInterface  1.0",
            "      moves out of GatherMate2",
        ]
    );
    let text = lines.join("\n");
    assert!(!text.contains("joins from"), "{text}");
}

#[test]
fn an_addon_whose_own_folder_is_held_by_another_row_moves_out_of_it() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    let text = lines.join("\n");
    assert!(text.contains("      moves out of GatherMate2"), "{text}");
    // "also takes" is only for folders other than the addon's own.
    assert!(!text.contains("also takes GatherMate2_Data"), "{text}");
    assert!(
        lines
            .iter()
            .filter(|line| line.contains("also takes"))
            .all(|line| !line.contains("GatherMate2_Data")),
        "{text}"
    );
}

#[test]
fn detail_lines_wrap_to_the_width_of_the_lists_and_keep_their_indent() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    let details = lines
        .iter()
        .filter(|line| line.starts_with("      "))
        .collect::<Vec<_>>();
    assert!(!details.is_empty());
    assert!(
        details.iter().all(|line| line.chars().count() <= 80),
        "{details:?}"
    );
    // The long "also takes" line continues on the next line with the same indent.
    assert!(
        lines.contains(
            &"      also takes GatherMate2Storage_Dragonflight, GatherMate2Storage_Midnight,"
                .to_string()
        )
    );
    assert!(lines.contains(&"      GatherMate2Storage_Shadowlands".to_string()));
}

#[test]
fn a_match_that_cannot_be_linked_is_never_listed_as_ready() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    let ready = section_lines(&lines, "Ready to link (5)").join("\n");
    assert!(!ready.contains("Details"), "{ready}");
    assert_eq!(
        section_lines(&lines, "Can't link yet (1)"),
        [
            "  Details  Wago.io  1.0",
            "      It needs folders LemonUp won't take: Details_Shared belongs to Details",
            "      Core",
        ]
    );
}

#[test]
fn likely_partial_and_choose_one_say_why_they_are_not_linked() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    assert_eq!(
        section_lines(&lines, "Likely matches (1)"),
        ["  Bartender4  WoWInterface  the installed version differs from the release (4.1)"]
    );
    assert_eq!(
        section_lines(&lines, "Partial matches (1)"),
        ["  Plater  Wago.io  2.0, files differ in Plater_Libs"]
    );
    assert_eq!(
        section_lines(&lines, "Choose one in LemonUp (1)"),
        [
            "  QuestieDev (likely, same Wago.io project) or Questie (QuestieDB differs from",
            "  the release)",
        ]
    );
}

#[test]
fn name_only_sections_group_curseforge_projects_and_wrap() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    let curseforge = section_lines(
        &lines,
        "On CurseForge only, LemonUp can't update these yet (11)",
    );
    assert_eq!(
        curseforge,
        [
            "  Another Long Addon Name, Forever Loot Sparkles, ForeverAuras (5 folders),",
            "  Leatrix Plus, Leatrix Sounds, OpenMap, Yet Another Addon",
        ]
    );
    assert!(curseforge.iter().all(|line| line.chars().count() <= 80));
    assert_eq!(
        section_lines(&lines, "Not found (2)"),
        ["  AzerothGPS, DungeonJournal"]
    );
    assert_eq!(
        section_lines(&lines, "Skipped, developer copy (1)"),
        ["  WoW Butler"]
    );
}

#[test]
fn not_checked_names_the_failed_source_and_why() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );

    assert_eq!(
        section_lines(&lines, "Not checked (1)"),
        [
            "  Spy",
            "  WoWInterface: LemonUp couldn't reach WoWInterface. Check your internet",
            "  connection and try again.",
        ]
    );
}

#[test]
fn a_failed_source_that_left_nothing_unchecked_is_still_reported() {
    let report = FindReport {
        results: vec![only(FindLabel::NotFound, "Spy")],
        failed: vec![FindSource::Tukui],
        skipped: Vec::new(),
    };
    let problems = vec![SourceProblem {
        source: FindSource::Tukui,
        error: SourceError::new(SourceErrorKind::SlowDown, "429"),
    }];

    let lines = human_report(&report, &problems, &full_names(), 0, Paint::PLAIN);

    assert_eq!(
        section_lines(&lines, "Check failed (1)"),
        ["  TukUI: TukUI asked LemonUp to slow down. Wait a few minutes, then try again."]
    );
}

#[test]
fn the_report_opens_with_what_was_sent_to_wago_or_that_a_key_finds_more() {
    let report = full_report();
    let names = full_names();

    // A request that got no answer is not claimed as sent.
    let unreachable = vec![SourceProblem {
        source: FindSource::Wago,
        error: SourceError::new(SourceErrorKind::NoConnection, "timed out"),
    }];
    assert_ne!(
        human_report(&report, &unreachable, &names, 44, Paint::PLAIN)[0],
        "Sent 44 folder names and file hashes to Wago.io."
    );

    let without_key = FindReport {
        results: Vec::new(),
        failed: Vec::new(),
        skipped: vec![FindSource::Wago],
    };
    let lines = human_report(&without_key, &[], &names, 0, Paint::PLAIN);
    assert_eq!(
        lines[0],
        "A Wago.io key finds more of your addons. Add one in Settings."
    );
    assert_eq!(lines.last().expect("last"), "No addons to look up.");
}

#[test]
fn the_last_line_always_says_what_happened() {
    let report = full_report();
    let names = full_names();

    assert_eq!(
        human_closing(&report, &names, None, Paint::PLAIN, Paint::PLAIN).expect("closing"),
        ["Nothing was changed. `lemonup find --link` links the 5 addons ready to link."]
    );
    let nothing_ready = FindReport {
        results: vec![only(FindLabel::NotFound, "Spy")],
        failed: Vec::new(),
        skipped: Vec::new(),
    };
    assert_eq!(
        human_closing(&nothing_ready, &names, None, Paint::PLAIN, Paint::PLAIN).expect("closing"),
        ["Nothing was changed."]
    );
    assert_eq!(
        human_closing(
            &nothing_ready,
            &names,
            Some(LinkOutcome::NothingToLink),
            Paint::PLAIN,
            Paint::PLAIN,
        )
        .expect("closing"),
        ["Nothing was linked. No addon is ready to link."]
    );
}

#[test]
fn find_link_without_yes_prints_a_refusal_that_says_what_moves() {
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);
    let inputs = FindInputs::read(&sandbox.addons, GameFlavor::Forever).expect("inputs");
    let names = LocalNames::new(&sandbox.database.list_addons().expect("rows"), &inputs);
    let outcome = link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        &report,
        false,
    )
    .expect("a refusal is an outcome");

    let error = human_closing(&report, &names, Some(outcome), Paint::PLAIN, Paint::PLAIN)
        .expect_err("refused");

    assert!(matches!(error, LemonupError::Refused(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "1 of these moves folders between addons: GatherMate2 also takes \
         GatherMate2Storage_Midnight. Nothing was linked. Run `lemonup find --link --yes` \
         to link the 2 addons ready to link."
    );
    assert!(!error.to_string().contains("invalid argument"));
    assert!(!error.to_string().contains("joins from"));
}

#[test]
fn a_refusal_names_folders_that_become_their_own_addon() {
    let names = full_names();
    let mut gather = found(
        FindSource::WowInterface,
        "GatherMate 2",
        "GatherMate2",
        "1.0",
        "1.0",
    );
    gather.joining = vec![folder_move("GatherMate2Storage_Midnight", "Other")];
    gather.leaving = vec!["GatherMate2_Data".to_string()];
    let report = FindReport {
        results: vec![exact(gather.clone())],
        failed: Vec::new(),
        skipped: Vec::new(),
    };

    let error = human_closing(
        &report,
        &names,
        Some(LinkOutcome::NeedsYes(vec![gather])),
        Paint::PLAIN,
        Paint::PLAIN,
    )
    .expect_err("refused");

    assert_eq!(
        error.to_string(),
        "1 of these moves folders between addons: GatherMate2 also takes \
         GatherMate2Storage_Midnight; GatherMate2_Data becomes its own addon. Nothing was \
         linked. Run `lemonup find --link --yes` to link the addon ready to link."
    );
}

#[test]
fn find_link_with_yes_ends_with_what_was_linked_and_what_is_ready_to_update() {
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);
    let names = full_names();
    let outcome = link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        &report,
        true,
    )
    .expect("linked");

    assert_eq!(
        human_closing(&report, &names, Some(outcome), Paint::PLAIN, Paint::PLAIN).expect("closing"),
        [
            "Linked 2 addons: 1 up to date, 1 with an update ready (Pawn). \
          `lemonup update-all` updates it."
        ]
    );
}

#[test]
fn a_link_the_database_does_not_hold_is_said_not_claimed() {
    let linked = AddonRecord::new("Pawn", "Pawn", SourceKind::Wago);
    let outcome = LinkOutcome::Linked {
        rows: vec![linked],
        unconfirmed: vec!["Spy".to_string()],
    };

    let lines = human_closing(
        &full_report(),
        &full_names(),
        Some(outcome),
        Paint::PLAIN,
        Paint::PLAIN,
    )
    .expect("closing");

    assert!(lines[0].starts_with("Linked 1 addon: "), "{lines:?}");
    assert_eq!(
        lines[1],
        "LemonUp could not confirm that these were also linked: Spy. \
         Run `lemonup find` again to see where they stand."
    );
}

#[test]
fn plain_output_keeps_its_fields_and_fixes_the_three_lines() {
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);

    // A joining folder that has a row of its own is its own addon, not "from" itself.
    let lines = report_lines(&report, &[], 4);
    let gather = lines
        .iter()
        .find(|line| line.contains("addon=GatherMate2,"))
        .expect("gathermate");
    assert!(
        gather.contains("joining=GatherMate2Storage_Midnight (own addon)"),
        "{gather}"
    );
    // One grouped under another addon still says where it comes from.
    let mut grouped = found(FindSource::Wago, "Pawn", "Pawn", "1.0", "1.0");
    grouped.joining = vec![folder_move("Pawn_Options", "Other")];
    let moved = candidate_fields(&grouped);
    assert!(
        moved.contains("joining=Pawn_Options (from Other)"),
        "{moved}"
    );
    assert!(moved.starts_with("source=wago, id=1, version=1.0, latest=1.0, folders=Pawn"));

    // The refusal has no "invalid argument" label.
    let error = link_plain(&mut sandbox, &report, false).expect_err("needs --yes");
    assert!(matches!(error, LemonupError::Refused(_)), "{error:?}");
    assert!(!error.to_string().contains("invalid argument"), "{error}");

    // Plain find always ends by saying nothing was changed.
    let nothing_ready = FindReport {
        results: vec![only(FindLabel::NotFound, "Spy")],
        failed: Vec::new(),
        skipped: Vec::new(),
    };
    assert_eq!(
        plain_closing(&nothing_ready, None).expect("closing"),
        ["find detail: nothing was changed"]
    );
    assert_eq!(
        plain_closing(&report, None).expect("closing"),
        ["find detail: nothing was changed; `lemonup find --link` links the exact matches"]
    );
}

// ---------------------------------------------------------------------------------------------
// Colors: the Terminal theme's roles as the terminal's own palette.

const BOLD: &str = "\x1b[1m";
const END: &str = "\x1b[0m";
const GREEN: &str = "\x1b[38;5;2m";
const AMBER: &str = "\x1b[38;5;3m";
const RED: &str = "\x1b[38;5;1m";
const MUTED: &str = "\x1b[38;5;8m";

fn colored_report() -> Vec<String> {
    human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::terminal(),
    )
}

/// What a colored heading looks like: bold in its role's color, the count muted.
fn heading(color: &str, title: &str, count: usize) -> String {
    format!("{BOLD}{color}{title}{END} {MUTED}({count}){END}")
}

#[test]
fn the_quiet_headings_are_bold_in_the_text_role_not_muted() {
    let lines = colored_report();

    for (title, count) in [
        ("On CurseForge only, LemonUp can't update these yet", 11),
        ("Not found", 2),
        ("Skipped, developer copy", 1),
    ] {
        // The text role is the terminal's own color, so bold is the only escape on the title.
        let expected = format!("{BOLD}{title}{END} {MUTED}({count}){END}");
        assert!(
            lines.contains(&expected),
            "missing {expected:?} in {lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(&format!("{MUTED}{title}"))),
            "{title} is muted"
        );
    }
}

#[test]
fn colored_headings_are_bold_in_the_color_of_what_they_mean() {
    let lines = colored_report();

    for (color, title, count) in [
        (GREEN, "Ready to link", 5),
        (AMBER, "Can't link yet", 1),
        (AMBER, "Likely matches", 1),
        (AMBER, "Partial matches", 1),
        (AMBER, "Choose one in LemonUp", 1),
        (RED, "Not checked", 1),
    ] {
        let expected = heading(color, title, count);
        assert!(
            lines.contains(&expected),
            "missing {expected:?} in {lines:?}"
        );
    }
    let failed = FindReport {
        results: vec![only(FindLabel::NotFound, "Spy")],
        failed: vec![FindSource::Tukui],
        skipped: Vec::new(),
    };
    let problems = vec![SourceProblem {
        source: FindSource::Tukui,
        error: SourceError::new(SourceErrorKind::SlowDown, "429"),
    }];
    let lines = human_report(&failed, &problems, &full_names(), 0, Paint::terminal());
    assert!(
        lines.contains(&heading(RED, "Check failed", 1)),
        "{lines:?}"
    );
}

#[test]
fn colored_ready_lines_have_a_plain_name_a_muted_source_and_an_amber_update() {
    let lines = colored_report();

    let apr = lines
        .iter()
        .find(|line| line.contains("Azeroth Pilot Reloaded"))
        .expect("apr line");
    // The name is in the normal text role: the terminal's own color, so no escape.
    assert!(apr.starts_with("  Azeroth Pilot Reloaded  "), "{apr:?}");
    assert!(
        apr.contains(&format!("{MUTED}Wago.io     {END}")),
        "{apr:?}"
    );
    assert!(
        apr.ends_with(&format!("v7.7.1, {AMBER}update ready (v7.8.0){END}")),
        "{apr:?}"
    );
    // An addon with nothing to update has no amber.
    let ask = lines
        .iter()
        .find(|line| line.contains("Ask Mr. Robot Classic"))
        .expect("ask line");
    assert!(!ask.contains(AMBER), "{ask:?}");
}

#[test]
fn colored_detail_lines_are_muted_and_the_note_is_muted() {
    let lines = colored_report();

    for detail in [
        "      also takes GatherMate2Storage_Dragonflight, GatherMate2Storage_Midnight,",
        "      GatherMate2Storage_Shadowlands",
        "      GatherMate2_Data becomes its own addon",
        "      GatherMate2Storage_Gone missing on disk",
        "      moves out of GatherMate2",
        "      Core",
    ] {
        let expected = format!("{MUTED}{detail}{END}");
        assert!(lines.contains(&expected), "missing {expected:?}");
    }
    assert_eq!(
        lines[0],
        format!("{MUTED}Sent 44 folder names and file hashes to Wago.io.{END}")
    );
}

#[test]
fn the_closing_line_and_the_refusal_are_bold() {
    let closing = human_closing(
        &full_report(),
        &full_names(),
        None,
        Paint::terminal(),
        Paint::PLAIN,
    )
    .expect("closing");
    assert_eq!(
        closing,
        [format!(
            "{BOLD}Nothing was changed. `lemonup find --link` links the 5 addons ready to link.{END}"
        )]
    );

    let linked = LinkOutcome::Linked {
        rows: vec![AddonRecord::new("Pawn", "Pawn", SourceKind::Wago)],
        unconfirmed: Vec::new(),
    };
    let lines = human_closing(
        &full_report(),
        &full_names(),
        Some(linked),
        Paint::terminal(),
        Paint::PLAIN,
    )
    .expect("closing");
    assert!(
        lines[0].starts_with(&format!("{BOLD}Linked 1 addon: ")),
        "{lines:?}"
    );
    assert!(lines[0].ends_with(END));

    let mut gather = found(FindSource::Wago, "GatherMate2", "GatherMate2", "1.0", "1.0");
    gather.joining = vec![folder_move("GatherMate2Storage_Midnight", "Other")];
    let report = FindReport {
        results: vec![exact(gather.clone())],
        failed: Vec::new(),
        skipped: Vec::new(),
    };
    let refused = |error_paint| {
        human_closing(
            &report,
            &full_names(),
            Some(LinkOutcome::NeedsYes(vec![gather.clone()])),
            Paint::terminal(),
            error_paint,
        )
        .expect_err("refused")
        .to_string()
    };
    let bold = refused(Paint::terminal());
    assert!(bold.starts_with(BOLD) && bold.ends_with(END), "{bold:?}");
    assert!(!refused(Paint::PLAIN).contains('\x1b'));
}

#[test]
fn uncolored_output_has_no_escape_codes() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );
    assert!(lines.iter().all(|line| !line.contains('\x1b')), "{lines:?}");
    let closing = human_closing(
        &full_report(),
        &full_names(),
        None,
        Paint::PLAIN,
        Paint::PLAIN,
    )
    .expect("closing");
    assert!(closing.iter().all(|line| !line.contains('\x1b')));

    // Colors only add escape codes: with them stripped, the text is the same.
    let strip = |line: &str| {
        let mut text = String::new();
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                text.push(ch);
            }
        }
        text
    };
    let stripped = colored_report()
        .iter()
        .map(|line| strip(line))
        .collect::<Vec<_>>();
    assert_eq!(stripped, lines);
}

#[test]
fn the_plain_key_value_output_is_never_colored() {
    // `--plain` is built by `report_lines` and `plain_closing`, which take no Paint at all.
    let mut sandbox = sandbox();
    let report = report_for(&sandbox);
    for line in report_lines(&report, &[], 4) {
        assert!(!line.contains('\x1b'), "{line:?}");
    }
    let closing = plain_closing(&report, None).expect("closing");
    assert!(closing.iter().all(|line| !line.contains('\x1b')));
    let error = link_plain(&mut sandbox, &report, false).expect_err("needs --yes");
    assert!(!error.to_string().contains('\x1b'), "{error:?}");
}

#[test]
fn color_is_for_a_terminal_without_no_color() {
    use super::human::color_wanted;
    use crate::theme::{ColorRoles, DisplayEnv};
    use lemonup_core::AppConfig;

    let with = |no_color: &'static str| {
        DisplayEnv::from_vars(
            move |name| (name == "NO_COLOR").then(|| no_color.to_string()),
            false,
        )
    };
    assert!(color_wanted(true, with("")));
    assert!(color_wanted(true, DisplayEnv::FULL));
    assert!(!color_wanted(true, with("1")));
    assert!(!color_wanted(false, with("")));
    assert!(!color_wanted(false, DisplayEnv::FULL));

    let config = AppConfig::new_unconfigured();
    assert_eq!(
        Paint::for_stream(true, DisplayEnv::FULL, &config),
        Paint::with_roles(ColorRoles::dark())
    );
    assert_eq!(Paint::for_stream(true, with("1"), &config), Paint::PLAIN);
    assert_eq!(Paint::for_stream(false, with(""), &config), Paint::PLAIN);
    assert_ne!(Paint::terminal(), Paint::PLAIN);
}

#[test]
fn a_refusal_says_an_addon_moves_out_of_the_row_that_holds_its_folder() {
    let mut data = found(
        FindSource::WowInterface,
        "GatherMate2Data",
        "GatherMate2_Data",
        "1.0",
        "1.0",
    );
    data.joining = vec![folder_move("GatherMate2_Data", "GatherMate2")];
    let report = FindReport {
        results: vec![exact(data.clone())],
        failed: Vec::new(),
        skipped: Vec::new(),
    };

    let error = human_closing(
        &report,
        &full_names(),
        Some(LinkOutcome::NeedsYes(vec![data])),
        Paint::PLAIN,
        Paint::PLAIN,
    )
    .expect_err("refused")
    .to_string();

    assert!(
        error.contains("GatherMate2Data moves out of GatherMate2"),
        "{error}"
    );
    assert!(!error.contains("also takes"), "{error}");
}

#[test]
fn the_report_wraps_to_the_width_it_is_given() {
    let wide = |width: usize| {
        human_report(
            &full_report(),
            &wowinterface_down(),
            &full_names(),
            44,
            Paint::PLAIN.with_width(width),
        )
    };
    let at_78 = wide(78);
    let at_100 = wide(100);
    let at_120 = wide(120);

    // At 78 the lines wrap as they always did.
    assert_eq!(
        section_lines(&at_78, "Choose one in LemonUp (1)").len(),
        2,
        "{at_78:?}"
    );
    assert!(
        at_78.contains(
            &"      also takes GatherMate2Storage_Dragonflight, GatherMate2Storage_Midnight,"
                .to_string()
        )
    );
    // At 100 and 120 the Choose one line fits on one line.
    for lines in [&at_100, &at_120] {
        assert_eq!(
            section_lines(lines, "Choose one in LemonUp (1)"),
            [
                "  QuestieDev (likely, same Wago.io project) or Questie (QuestieDB differs from the release)"
            ]
        );
    }
    // The "also takes" folders need 120 to fit on one line, and still wrap at 100.
    assert!(
        !at_100
            .iter()
            .any(|line| line.contains("Midnight, GatherMate2Storage_Shadowlands")),
        "{at_100:?}"
    );
    assert!(
        at_120.contains(
            &"      also takes GatherMate2Storage_Dragonflight, GatherMate2Storage_Midnight, GatherMate2Storage_Shadowlands"
                .to_string()
        ),
        "{at_120:?}"
    );
}

#[test]
fn the_width_follows_the_terminal_within_limits() {
    use super::human::report_width;

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
fn choose_one_lists_folders_with_a_short_reason_and_wraps() {
    let lines = human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::PLAIN,
    );
    let choose = section_lines(&lines, "Choose one in LemonUp (1)");
    assert_eq!(choose.len(), 2, "{choose:?}");
    assert_eq!(
        choose[0],
        "  QuestieDev (likely, same Wago.io project) or Questie (QuestieDB differs from"
    );
    assert_eq!(choose[1], "  the release)");
    // No .toc title or addon name leads the line.
    assert!(
        choose.iter().all(|line| !line.contains("Questie:")),
        "{choose:?}"
    );
    assert!(choose.iter().all(|line| line.chars().count() <= 80));

    // The same folder from two sources is told apart by the source.
    let mut wago = found(FindSource::Wago, "Pawn", "Pawn", "1.0", "1.0");
    wago.label = FindLabel::Likely;
    let wowi = found(FindSource::WowInterface, "Pawn", "Pawn", "1.0", "1.0");
    let report = FindReport {
        results: vec![result_of(
            FindLabel::ChooseOne,
            "Pawn",
            "Pawn",
            vec![wowi, wago],
        )],
        failed: Vec::new(),
        skipped: Vec::new(),
    };
    let lines = human_report(&report, &[], &full_names(), 0, Paint::PLAIN);
    let joined = section_lines(&lines, "Choose one in LemonUp (1)")
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        joined,
        "Pawn on WoWInterface (exact match) or Pawn on Wago.io (likely, same Wago.io project)"
    );
}

/// The report drawn for a terminal with `display`, in the theme `theme` names.
fn report_in_theme(theme: lemonup_core::ThemeMode, display: crate::theme::DisplayEnv) -> String {
    let mut config = lemonup_core::AppConfig::new_unconfigured();
    config.theme = theme;
    human_report(
        &full_report(),
        &wowinterface_down(),
        &full_names(),
        44,
        Paint::for_stream(true, display, &config),
    )
    .join("\n")
}

#[test]
fn the_report_uses_the_colors_of_the_chosen_theme() {
    use crate::theme::{ColorRoles, DisplayEnv};
    use lemonup_core::ThemeMode;

    let rgb = |color: ratatui::style::Color| match color {
        ratatui::style::Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        other => panic!("expected an RGB role, got {other:?}"),
    };

    let dark = report_in_theme(ThemeMode::Dark, DisplayEnv::FULL);
    let roles = ColorRoles::dark();
    assert!(
        dark.contains(&rgb(roles.muted)),
        "dark muted missing: {dark:?}"
    );
    assert!(
        dark.contains(&rgb(roles.green)),
        "dark green missing: {dark:?}"
    );

    let light = report_in_theme(ThemeMode::Light, DisplayEnv::FULL);
    assert!(light.contains(&rgb(ColorRoles::light().muted)), "{light:?}");

    let terminal = report_in_theme(ThemeMode::Terminal, DisplayEnv::FULL);
    assert!(terminal.contains('\u{1b}'), "{terminal:?}");
    assert!(
        !terminal.contains("38;2;"),
        "terminal theme used 24-bit: {terminal:?}"
    );
}

#[test]
fn without_24_bit_color_a_dark_theme_uses_the_16_colors() {
    use crate::theme::DisplayEnv;
    use lemonup_core::ThemeMode;

    let no_truecolor = DisplayEnv::from_vars(|_| None, false);
    assert!(!no_truecolor.truecolor);
    let report = report_in_theme(ThemeMode::Dark, no_truecolor);
    assert!(report.contains('\u{1b}'), "{report:?}");
    assert!(!report.contains("38;2;"), "{report:?}");
}

#[test]
fn no_color_draws_no_color_in_any_theme() {
    use crate::theme::DisplayEnv;
    use lemonup_core::ThemeMode;

    let no_color = DisplayEnv::from_vars(
        |name| match name {
            "NO_COLOR" => Some("1".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        },
        false,
    );
    for theme in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Terminal] {
        let report = report_in_theme(theme, no_color);
        assert!(!report.contains('\u{1b}'), "{theme:?}: {report:?}");
    }
}
