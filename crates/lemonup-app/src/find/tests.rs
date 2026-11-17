use std::fs::File;
use std::path::PathBuf;

use lemonup_core::find::{FindInputs, match_addons};
use lemonup_core::{OwnedFolder, SourceKind, UpdateStatus};

use super::*;

const LIST_A: &str = r#"[{"id":1,"title":"A","version":"1","addons":[{"path":"A","addOnVersion":"1","apiVersion":"16001"}]}]"#;
const LIST_B: &str = r#"[{"id":2,"title":"B","version":"2","addons":[{"path":"B","addOnVersion":"2","apiVersion":"16001"}]}]"#;

fn cached_list(dir: &Path, age: Duration) -> (PathBuf, SystemTime) {
    let path = dir.join(WOWI_CACHE_FILE);
    fs::write(&path, LIST_A).expect("cache");
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

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "1");
}

#[tokio::test]
async fn an_hour_old_wowinterface_list_is_downloaded_again_and_cached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, modified) = cached_list(dir.path(), Duration::from_secs(2 * 60 * 60));

    let files = load_wowi_files(&path, modified + WOWI_CACHE_TTL, || async {
        Ok(LIST_B.as_bytes().to_vec())
    })
    .await
    .expect("downloaded list");

    assert_eq!(files[0].id, "2");
    assert_eq!(fs::read_to_string(&path).expect("cache"), LIST_B);
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
        "find result: label=exact, addon=GatherMate2, folder=GatherMate2, source=wago, id=GM2, version=1.0, latest=1.0, folders=GatherMate2|GatherMate2Storage_Midnight, joining=GatherMate2Storage_Midnight (from GatherMate2Storage_Midnight)"
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

    let error = link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        &report,
        false,
    )
    .expect_err("needs --yes");

    let message = error.to_string();
    assert!(
        message.contains(
            "GatherMate2 (GatherMate2Storage_Midnight joins from GatherMate2Storage_Midnight)"
        ),
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

    let lines = link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        &report,
        true,
    )
    .expect("linked");

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

    let lines = link_exact_matches(
        &mut sandbox.database,
        &sandbox.addons,
        GameFlavor::Forever,
        &report,
        true,
    )
    .expect("linked");

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
