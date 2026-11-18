use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::domain::{OwnershipSource, UnrecognizedValues};

struct Fixture {
    _dir: tempfile::TempDir,
    addons: PathBuf,
    database: StateDatabase,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let addons = dir.path().join("AddOns");
    fs::create_dir_all(&addons).expect("addons dir");
    let database = StateDatabase::open(dir.path().join("state.sqlite")).expect("database");
    Fixture {
        _dir: dir,
        addons,
        database,
    }
}

/// Writes `<folder>/<folder>.toc` with `lines` and one Lua file the TOC lists.
fn addon(addons: &Path, folder: &str, lines: &[&str]) {
    let path = addons.join(folder);
    fs::create_dir_all(&path).expect("folder");
    let mut toc = lines.join("\n");
    toc.push_str("\nmain.lua\n");
    fs::write(path.join(format!("{folder}.toc")), toc).expect("toc");
    fs::write(path.join("main.lua"), format!("-- {folder}\n")).expect("lua");
}

fn hash_of(inputs: &FindInputs, folder: &str) -> String {
    inputs
        .get(folder)
        .and_then(|local| local.hash())
        .expect("hash")
        .to_string()
}

fn manual_row(folder: &str, owned: &[&str]) -> AddonRecord {
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

fn wago(id: &str, name: &str, modules: Vec<(String, String)>) -> WagoMatch {
    WagoMatch {
        id: id.to_string(),
        name: name.to_string(),
        website_url: Some(format!(
            "https://addons.wago.io/addons/{}",
            id.to_lowercase()
        )),
        author: Some("Author".to_string()),
        modules,
        matched_version: Some("v1.0.0".to_string()),
        latest_version: Some("v1.1.0".to_string()),
        wowinterface_id: None,
    }
}

fn lookups(wago: SourceAnswer<WagoMatch>) -> SourceLookups {
    SourceLookups {
        wago,
        wowinterface: SourceAnswer::Found(Vec::new()),
        tukui: SourceAnswer::Found(Vec::new()),
    }
}

fn result<'a>(report: &'a FindReport, folder: &str) -> &'a FindResult {
    report
        .results
        .iter()
        .find(|result| result.folder == folder)
        .unwrap_or_else(|| panic!("no result for {folder}: {:#?}", report.results))
}

#[test]
fn inputs_skip_files_and_mark_symlinks_and_git_checkouts_as_developer_copies() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Plain",
        &["## Version: 2.0", "## X-Wago-ID: abc123"],
    );
    addon(&fixture.addons, "Checkout", &["## Version: 1.0"]);
    fs::create_dir_all(fixture.addons.join("Checkout").join(".git")).expect(".git");
    fs::write(fixture.addons.join("AtlasLoot\\Data.lua"), "stray file").expect("file");
    let elsewhere = fixture.addons.parent().expect("parent").join("dev");
    addon(&elsewhere, "Linked", &["## Version: 1.0"]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(elsewhere.join("Linked"), fixture.addons.join("Linked"))
        .expect("symlink");

    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");

    let names = inputs
        .folders
        .iter()
        .map(|local| local.folder.as_str())
        .collect::<Vec<_>>();
    #[cfg(unix)]
    assert_eq!(names, ["Checkout", "Linked", "Plain"]);
    let plain = inputs.get("plain").expect("plain");
    assert_eq!(plain.developer_copy, None);
    assert_eq!(plain.version.as_deref(), Some("2.0"));
    assert_eq!(plain.ids.wago.as_deref(), Some("abc123"));
    assert_eq!(plain.fingerprint.as_ref().map(|fp| fp.files), Some(2));
    let checkout = inputs.get("Checkout").expect("checkout");
    assert_eq!(checkout.developer_copy, Some(DeveloperCopy::GitCheckout));
    assert_eq!(checkout.fingerprint, None);
    #[cfg(unix)]
    assert_eq!(
        inputs.get("Linked").expect("linked").developer_copy,
        Some(DeveloperCopy::Symlink)
    );

    // Developer copies never go to Wago.io.
    let sent = inputs.wago_lookups(&[], None);
    assert_eq!(
        sent.iter()
            .map(|lookup| lookup.folder.as_str())
            .collect::<Vec<_>>(),
        ["Plain"]
    );
    assert_eq!(sent[0].wago_id.as_deref(), Some("abc123"));
}

#[test]
fn a_title_with_a_color_code_in_the_middle_has_one_space() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Decursive",
        &["## Title: Decursive |cff00ff00|r -Ace3-"],
    );

    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");

    assert_eq!(
        inputs
            .get("Decursive")
            .and_then(|local| local.title.as_deref()),
        Some("Decursive -Ace3-")
    );
}

#[test]
fn toc_ids_and_version_follow_the_game_version() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Cond",
        &[
            "## Version: 1.0",
            "## Version: 9.9 [AllowLoadGameType classic]",
            "## X-WoWI-ID: 42",
        ],
    );

    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");

    let local = inputs.get("Cond").expect("cond");
    assert_eq!(local.version.as_deref(), Some("1.0"));
    assert_eq!(local.ids.wowinterface.as_deref(), Some("42"));
}

#[test]
fn a_full_hash_match_is_exact_and_preselected() {
    let fixture = fixture();
    addon(&fixture.addons, "Narcissus", &["## Version: v1.0.0"]);
    addon(&fixture.addons, "Narcissus_Achievements", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = vec![
        manual_row("Narcissus", &[]),
        manual_row("Narcissus_Achievements", &[]),
    ];
    let modules = vec![
        ("Narcissus".to_string(), hash_of(&inputs, "Narcissus")),
        (
            "Narcissus_Achievements".to_string(),
            hash_of(&inputs, "Narcissus_Achievements").to_uppercase(),
        ),
    ];

    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![wago("WL6", "Narcissus", modules)])),
    );

    assert_eq!(report.results.len(), 1);
    let found = result(&report, "Narcissus");
    assert_eq!(found.label, FindLabel::Exact);
    assert!(found.preselected());
    let candidate = found.found().expect("candidate");
    assert_eq!(candidate.folders, ["Narcissus", "Narcissus_Achievements"]);
    assert_eq!(candidate.matched_version.as_deref(), Some("v1.0.0"));
    assert_eq!(candidate.latest_version.as_deref(), Some("v1.1.0"));
    assert_eq!(
        candidate.joining,
        [FolderMove {
            folder: "Narcissus_Achievements".to_string(),
            from: "Narcissus_Achievements".to_string(),
        }]
    );
    assert!(candidate.leaving.is_empty());
}

#[test]
fn a_package_with_a_differing_folder_is_partial_and_names_it() {
    let fixture = fixture();
    addon(&fixture.addons, "Questie", &[]);
    addon(&fixture.addons, "QuestieDB", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let modules = vec![
        ("Questie".to_string(), hash_of(&inputs, "Questie")),
        ("QuestieDB".to_string(), "0".repeat(32)),
        ("QuestieTrace".to_string(), "1".repeat(32)),
    ];

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(vec![wago("QID", "Questie", modules)])),
    );

    let found = result(&report, "Questie");
    assert_eq!(found.label, FindLabel::Partial);
    assert!(!found.preselected());
    let candidate = found.found().expect("candidate");
    assert_eq!(candidate.differing, ["QuestieDB"]);
    assert_eq!(candidate.missing, ["QuestieTrace"]);
}

#[test]
fn two_folders_naming_one_project_are_choose_one() {
    let fixture = fixture();
    addon(&fixture.addons, "Questie", &["## X-Wago-ID: QID"]);
    addon(&fixture.addons, "QuestieDev", &["## X-Wago-ID: QID"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let modules = vec![("Questie".to_string(), hash_of(&inputs, "Questie"))];

    let report = match_addons(
        &inputs,
        &[manual_row("Questie", &[]), manual_row("QuestieDev", &[])],
        None,
        &lookups(SourceAnswer::Found(vec![wago("QID", "Questie", modules)])),
    );

    assert_eq!(report.results.len(), 1, "{:#?}", report.results);
    let choose = &report.results[0];
    assert_eq!(choose.label, FindLabel::ChooseOne);
    assert!(!choose.preselected());
    let parents = choose
        .candidates
        .iter()
        .map(|candidate| candidate.parent_folder.as_str())
        .collect::<Vec<_>>();
    assert_eq!(parents, ["Questie", "QuestieDev"]);
    assert_eq!(choose.candidates[0].label, FindLabel::Exact);
    assert_eq!(choose.candidates[1].label, FindLabel::Likely);
}

#[test]
fn two_exact_matches_on_one_folder_are_choose_one() {
    let fixture = fixture();
    addon(&fixture.addons, "Spy", &["## Version: 1.0"]);
    addon(&fixture.addons, "Spy_Extra", &["## Version: 1.0"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "SPY",
        "Spy",
        vec![("Spy".to_string(), hash_of(&inputs, "Spy"))],
    )]));
    // Both match exactly, but not the same folders: the files can't tell them apart.
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "99",
        "Spy fork",
        "1.0",
        &[("Spy", "1.0"), ("Spy_Extra", "1.0")],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    let choose = result(&report, "Spy");
    assert_eq!(choose.label, FindLabel::ChooseOne);
    assert_eq!(choose.candidates.len(), 2);
}

fn wowi_file(id: &str, title: &str, version: &str, folders: &[(&str, &str)]) -> WowiFile {
    WowiFile {
        id: id.to_string(),
        title: title.to_string(),
        version: version.to_string(),
        author: None,
        folders: folders
            .iter()
            .map(|(folder, version)| WowiFolder {
                folder: folder.to_string(),
                version: Some(version.to_string()),
                api_version: Some("16001".to_string()),
            })
            .collect(),
    }
}

#[test]
fn the_same_project_on_wago_and_wowinterface_is_one_candidate() {
    let fixture = fixture();
    addon(&fixture.addons, "Clique", &["## Version: v5.1.5-release"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut project = wago(
        "CLQ",
        "Clique",
        vec![("Clique".to_string(), hash_of(&inputs, "Clique"))],
    );
    project.wowinterface_id = Some("5108".to_string());
    let mut lookups = lookups(SourceAnswer::Found(vec![project]));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "5108",
        "Clique",
        "v5.1.5-release",
        &[("Clique", "v5.1.5-release")],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    let found = result(&report, "Clique");
    assert_eq!(found.label, FindLabel::Exact);
    assert_eq!(found.found().expect("candidate").source, FindSource::Wago);
}

#[test]
fn gathermate2_regroups_by_the_sources_folder_lists() {
    let fixture = fixture();
    addon(&fixture.addons, "GatherMate2", &["## Version: 2.0"]);
    addon(&fixture.addons, "GatherMate2_Data", &["## Version: 1"]);
    addon(
        &fixture.addons,
        "GatherMate2Storage_Midnight",
        &["## Version: 2.0"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    // The scan grouped GatherMate2_Data under GatherMate2 and missed the storage folder.
    let rows = vec![
        manual_row("GatherMate2", &["GatherMate2_Data"]),
        manual_row("GatherMate2Storage_Midnight", &[]),
    ];
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "GMD",
        "GatherMate2 Data",
        vec![(
            "GatherMate2_Data".to_string(),
            hash_of(&inputs, "GatherMate2_Data"),
        )],
    )]));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "7",
        "GatherMate2",
        "2.0",
        &[
            ("GatherMate2", "2.0"),
            ("GatherMate2Storage_Midnight", "2.0"),
        ],
    )]);

    let report = match_addons(&inputs, &rows, None, &lookups);

    let main = result(&report, "GatherMate2");
    assert_eq!(main.label, FindLabel::Exact);
    let candidate = main.found().expect("candidate");
    assert_eq!(candidate.source, FindSource::WowInterface);
    assert_eq!(
        candidate.folders,
        ["GatherMate2", "GatherMate2Storage_Midnight"]
    );
    assert_eq!(
        candidate.joining,
        [FolderMove {
            folder: "GatherMate2Storage_Midnight".to_string(),
            from: "GatherMate2Storage_Midnight".to_string(),
        }]
    );
    assert_eq!(candidate.leaving, ["GatherMate2_Data"]);
    assert!(candidate.regroups());

    let data = result(&report, "GatherMate2_Data");
    assert_eq!(data.label, FindLabel::Exact);
    let candidate = data.found().expect("candidate");
    assert_eq!(candidate.source, FindSource::Wago);
    assert_eq!(
        candidate.joining,
        [FolderMove {
            folder: "GatherMate2_Data".to_string(),
            from: "GatherMate2".to_string(),
        }]
    );
    assert_eq!(report.results.len(), 2);
}

#[test]
fn a_folder_a_linked_addon_owns_is_never_taken_whatever_its_case() {
    let fixture = fixture();
    addon(&fixture.addons, "Pack", &[]);
    addon(&fixture.addons, "SharedLib", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut linked = AddonRecord::new("Other", "Other", SourceKind::Wago);
    linked.set_managed_owned_folders(vec![OwnedFolder {
        name: "SHAREDLIB".to_string(),
    }]);
    let rows = vec![linked, manual_row("Pack", &[])];
    let modules = vec![
        ("Pack".to_string(), hash_of(&inputs, "Pack")),
        ("sharedlib".to_string(), hash_of(&inputs, "SharedLib")),
    ];

    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![wago("PCK", "Pack", modules)])),
    );

    let found = result(&report, "Pack");
    assert_eq!(found.label, FindLabel::Exact);
    assert!(!found.preselected());
    let candidate = found.found().expect("candidate");
    assert!(!candidate.is_linkable());
    assert_eq!(
        candidate.blocked,
        [BlockedFolder {
            folder: "SharedLib".to_string(),
            reason: BlockReason::Linked {
                addon: "Other".to_string()
            },
        }]
    );
    assert!(
        candidate
            .not_linkable_reason()
            .expect("reason")
            .contains("SharedLib belongs to Other")
    );
    // The linked addon's folder is never sent to Wago.io either.
    let sent = inputs.wago_lookups(&rows, None);
    assert!(sent.iter().all(|lookup| lookup.folder != "SharedLib"));
}

#[test]
fn a_row_from_a_newer_lemonup_keeps_its_folders() {
    let fixture = fixture();
    addon(&fixture.addons, "Pack", &[]);
    addon(&fixture.addons, "Future", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut newer = manual_row("Future", &[]);
    newer.unrecognized = Some(UnrecognizedValues {
        source: "curseforge".to_string(),
        ..UnrecognizedValues::default()
    });
    let rows = vec![newer, manual_row("Pack", &[])];
    let modules = vec![
        ("Pack".to_string(), hash_of(&inputs, "Pack")),
        ("Future".to_string(), hash_of(&inputs, "Future")),
    ];

    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![wago("PCK", "Pack", modules)])),
    );

    let candidate = result(&report, "Pack").found().expect("candidate").clone();
    assert_eq!(
        candidate.blocked,
        [BlockedFolder {
            folder: "Future".to_string(),
            reason: BlockReason::NewerVersion {
                addon: "Future".to_string()
            },
        }]
    );
    assert!(
        report
            .results
            .iter()
            .all(|result| result.folder != "Future")
    );
}

#[test]
fn curseforge_only_developer_copy_and_not_found() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Leatrix_Plus",
        &["## X-Curse-Project-ID: 12345"],
    );
    addon(&fixture.addons, "WowButler", &[]);
    fs::create_dir_all(fixture.addons.join("WowButler").join(".git")).expect(".git");
    addon(&fixture.addons, "Mystery", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(Vec::new())),
    );

    let curse = result(&report, "Leatrix_Plus");
    assert_eq!(curse.label, FindLabel::CurseForgeOnly);
    assert_eq!(curse.curseforge_id.as_deref(), Some("12345"));
    assert_eq!(result(&report, "WowButler").label, FindLabel::DeveloperCopy);
    assert_eq!(result(&report, "Mystery").label, FindLabel::NotFound);
    assert!(report.results.iter().all(|result| !result.preselected()));
}

#[test]
fn a_failed_source_leaves_what_only_it_could_find_not_checked() {
    let fixture = fixture();
    addon(&fixture.addons, "Mystery", &["## X-Curse-Project-ID: 1"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");

    let failed = match_addons(&inputs, &[], None, &lookups(SourceAnswer::Failed));
    let skipped = match_addons(&inputs, &[], None, &lookups(SourceAnswer::Skipped));

    let mystery = result(&failed, "Mystery");
    assert_eq!(mystery.label, FindLabel::NotChecked);
    assert_eq!(mystery.not_checked_by, [FindSource::Wago]);
    assert_eq!(failed.failed, [FindSource::Wago]);
    // Without a key Wago.io is not asked, which is not a failure.
    assert_eq!(result(&skipped, "Mystery").label, FindLabel::CurseForgeOnly);
    assert_eq!(skipped.skipped, [FindSource::Wago]);
    assert!(skipped.failed.is_empty());
}

#[test]
fn a_failed_tukui_only_leaves_tukui_folders_not_checked() {
    let fixture = fixture();
    addon(&fixture.addons, "ElvUI", &[]);
    addon(&fixture.addons, "Mystery", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Found(Vec::new()));
    lookups.tukui = SourceAnswer::Failed;

    let report = match_addons(&inputs, &[], None, &lookups);

    assert_eq!(result(&report, "ElvUI").label, FindLabel::NotChecked);
    assert_eq!(result(&report, "Mystery").label, FindLabel::NotFound);
}

#[test]
fn tukui_directories_and_version_make_an_exact_match() {
    let fixture = fixture();
    addon(&fixture.addons, "ElvUI", &["## Version: 15.10"]);
    addon(&fixture.addons, "ElvUI_Options", &["## Version: 15.10"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Skipped);
    lookups.tukui = SourceAnswer::Found(vec![TukuiPackage {
        id: "elvui".to_string(),
        name: "ElvUI".to_string(),
        version: "15.10".to_string(),
        web_url: "https://tukui.org/elvui".to_string(),
        author: Some("Elv".to_string()),
        directories: vec!["ElvUI".to_string(), "ElvUI_Options".to_string()],
    }]);

    let report = match_addons(&inputs, &[], None, &lookups);

    let found = result(&report, "ElvUI");
    assert_eq!(found.label, FindLabel::Exact);
    assert_eq!(found.found().expect("candidate").source, FindSource::Tukui);
}

#[test]
fn a_tukui_package_with_an_older_folder_is_only_likely() {
    let fixture = fixture();
    addon(&fixture.addons, "ElvUI", &["## Version: 15.10"]);
    addon(&fixture.addons, "ElvUI_Options", &["## Version: 15.02"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Skipped);
    lookups.tukui = SourceAnswer::Found(vec![TukuiPackage {
        id: "elvui".to_string(),
        name: "ElvUI".to_string(),
        version: "15.10".to_string(),
        web_url: "https://tukui.org/elvui".to_string(),
        author: Some("Elv".to_string()),
        directories: vec!["ElvUI".to_string(), "ElvUI_Options".to_string()],
    }]);

    let report = match_addons(&inputs, &[], None, &lookups);

    let found = result(&report, "ElvUI");
    assert_eq!(found.label, FindLabel::Likely);
    assert!(!found.preselected());
}

#[test]
fn a_scope_looks_up_only_the_named_addons() {
    let fixture = fixture();
    addon(&fixture.addons, "One", &[]);
    addon(&fixture.addons, "Two", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = vec![manual_row("One", &[]), manual_row("Two", &[])];
    let scope = vec!["two".to_string()];

    let sent = inputs.wago_lookups(&rows, Some(&scope));
    let report = match_addons(
        &inputs,
        &rows,
        Some(&scope),
        &lookups(SourceAnswer::Found(Vec::new())),
    );

    assert_eq!(
        sent.iter()
            .map(|lookup| lookup.folder.as_str())
            .collect::<Vec<_>>(),
        ["Two"]
    );
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].folder, "Two");
}

#[test]
fn wowinterface_files_are_filtered_by_interface_number() {
    let file = |id: &str, api: &str| WowiFile {
        id: id.to_string(),
        title: id.to_string(),
        version: "1".to_string(),
        author: None,
        folders: vec![WowiFolder {
            folder: id.to_string(),
            version: None,
            api_version: Some(api.to_string()),
        }],
    };
    let mut files = vec![
        file("forever", "16001"),
        file("classic", "11507 40400"),
        file("old-retail", "110205"),
        file("broken", "####### 40400"),
    ];
    for index in 0..RETAIL_MAJOR_MIN_FILES {
        files.push(file(&format!("retail{index}"), "120005 11507"));
    }
    files.push(file("future-typo", "990000"));

    let ids = |flavor| {
        wowi_files_for_flavor(files.clone(), flavor)
            .into_iter()
            .map(|file| file.id)
            .collect::<Vec<_>>()
    };

    assert_eq!(ids(GameFlavor::Forever), ["forever"]);
    let retail = ids(GameFlavor::Retail);
    assert_eq!(retail.len(), RETAIL_MAJOR_MIN_FILES);
    assert!(retail.iter().all(|id| id.starts_with("retail")));
    assert!(ids(GameFlavor::Classic).contains(&"classic".to_string()));
}

fn exact_wago_pick(fixture: &Fixture) -> (FindInputs, Vec<AddonRecord>, FindReport) {
    addon(
        &fixture.addons,
        "Narcissus",
        &["## Version: 1.8.7", "## Title: Narcissus"],
    );
    addon(
        &fixture.addons,
        "Narcissus_Achievements",
        &["## Version: 1.8.7"],
    );
    let mut parent = manual_row("Narcissus", &[]);
    parent.toc_overrides = BTreeMap::from([("Notes".to_string(), "mine".to_string())]);
    let mut child = manual_row("Narcissus_Achievements", &[]);
    child.toc_overrides = BTreeMap::from([("Category".to_string(), "Fun".to_string())]);
    fixture.database.upsert_addon(&parent).expect("parent");
    fixture.database.upsert_addon(&child).expect("child");
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = fixture.database.list_addons().expect("rows");
    let mut project = wago(
        "WL6",
        "Narcissus",
        vec![
            ("Narcissus".to_string(), hash_of(&inputs, "Narcissus")),
            (
                "Narcissus_Achievements".to_string(),
                hash_of(&inputs, "Narcissus_Achievements"),
            ),
        ],
    );
    project.matched_version = Some("v1.8.7-c".to_string());
    project.latest_version = Some("v1.8.8".to_string());
    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![project])),
    );
    (inputs, rows, report)
}

#[test]
fn linking_writes_the_row_an_install_would_and_touches_no_files() {
    let mut fixture = fixture();
    let (_, _, report) = exact_wago_pick(&fixture);
    let pick = result(&report, "Narcissus").found().expect("pick").clone();
    let toc_before = fs::read(fixture.addons.join("Narcissus").join("Narcissus.toc")).expect("toc");

    link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect("link");

    let rows = fixture.database.list_addons().expect("rows");
    assert_eq!(rows.len(), 1, "the child row is absorbed: {rows:#?}");
    let row = &rows[0];
    assert_eq!(row.folder, "Narcissus");
    assert_eq!(row.source, SourceKind::Wago);
    assert_eq!(
        row.source_url.as_deref(),
        Some("https://addons.wago.io/addons/wl6")
    );
    assert_eq!(row.version.as_deref(), Some("v1.8.7-c"));
    assert_eq!(row.remote_version.as_deref(), Some("v1.8.8"));
    assert!(row.last_checked_at.is_some());
    assert_eq!(row.flavor, GameFlavor::Forever);
    assert_eq!(row.author.as_deref(), Some("Author"));
    assert_eq!(row.ownership_source, OwnershipSource::Managed);
    assert_eq!(
        row.owned_folders,
        [OwnedFolder {
            name: "Narcissus_Achievements".to_string()
        }]
    );
    assert_eq!(
        row.toc_overrides,
        BTreeMap::from([
            ("Category".to_string(), "Fun".to_string()),
            ("Notes".to_string(), "mine".to_string()),
        ])
    );
    assert_eq!(
        fs::read(fixture.addons.join("Narcissus").join("Narcissus.toc")).expect("toc"),
        toc_before
    );
}

#[test]
fn linking_refuses_a_blocked_pick_and_writes_nothing() {
    let mut fixture = fixture();
    let (_, _, report) = exact_wago_pick(&fixture);
    let pick = result(&report, "Narcissus").found().expect("pick").clone();
    // Meanwhile another addon was linked with one of the pick's folders, in another case.
    let mut linked = AddonRecord::new("Other", "Other", SourceKind::Wago);
    linked.set_managed_owned_folders(vec![OwnedFolder {
        name: "narcissus_achievements".to_string(),
    }]);
    fixture.database.upsert_addon(&linked).expect("linked");
    let before = fixture.database.list_addons().expect("rows");

    let error = link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect_err("refused");

    assert!(error.to_string().contains("belongs to Other"), "{error}");
    assert_eq!(fixture.database.list_addons().expect("rows"), before);
}

#[test]
fn linking_refuses_two_picks_that_share_a_folder() {
    let mut fixture = fixture();
    let (_, _, report) = exact_wago_pick(&fixture);
    let pick = result(&report, "Narcissus").found().expect("pick").clone();
    let before = fixture.database.list_addons().expect("rows");

    let error = link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick.clone(), pick],
    )
    .expect_err("refused");

    assert!(error.to_string().contains("both need"), "{error}");
    assert_eq!(fixture.database.list_addons().expect("rows"), before);
}

#[test]
fn a_write_that_fails_part_way_leaves_the_database_unchanged() {
    let mut fixture = fixture();
    let (_, _, report) = exact_wago_pick(&fixture);
    let pick = result(&report, "Narcissus").found().expect("pick").clone();
    let mut newer = manual_row("Future", &[]);
    newer.unrecognized = Some(UnrecognizedValues {
        source: "curseforge".to_string(),
        ..UnrecognizedValues::default()
    });
    fixture.database.upsert_addon(&newer).expect("newer");
    let before = fixture.database.list_addons().expect("rows");
    let parent = scan_addon_folder(
        &fixture.addons.join("Narcissus"),
        "Narcissus",
        GameFlavor::Forever,
    )
    .expect("scan")
    .expect("toc");
    let good = link_record(&pick, &parent, OffsetDateTime::now_utc());
    // The second record would take the newer LemonUp's folder: recording it fails after the
    // first one was written inside the transaction.
    let mut bad = AddonRecord::new("Thief", "Thief", SourceKind::Wago);
    bad.set_managed_owned_folders(vec![OwnedFolder {
        name: "Future".to_string(),
    }]);

    let error = fixture
        .database
        .record_managed_addons(&[good, bad], |_| Ok(()))
        .expect_err("second write fails");

    assert!(error.to_string().contains("newer LemonUp"), "{error}");
    assert_eq!(fixture.database.list_addons().expect("rows"), before);
}

fn rows_by_folder(database: &StateDatabase) -> Vec<(String, SourceKind, Vec<String>)> {
    database
        .list_addons()
        .expect("rows")
        .into_iter()
        .map(|row| {
            (
                row.folder,
                row.source,
                row.owned_folders
                    .into_iter()
                    .map(|owned| owned.name)
                    .collect(),
            )
        })
        .collect()
}

/// The scan grouped GatherMate2_Data under GatherMate2. Wago.io matches GatherMate2_Data on its
/// own (it joins from GatherMate2) and WoWInterface matches GatherMate2 alone (Data leaves).
fn gathermate2_split(fixture: &Fixture) -> Vec<FoundAddon> {
    addon(&fixture.addons, "GatherMate2", &["## Version: 2.0"]);
    addon(&fixture.addons, "GatherMate2_Data", &["## Version: 1"]);
    fixture
        .database
        .upsert_addon(&manual_row("GatherMate2", &["GatherMate2_Data"]))
        .expect("row");
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = fixture.database.list_addons().expect("rows");
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "GMD",
        "AAA GatherMate2 Data",
        vec![(
            "GatherMate2_Data".to_string(),
            hash_of(&inputs, "GatherMate2_Data"),
        )],
    )]));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "7",
        "GatherMate2",
        "2.0",
        &[("GatherMate2", "2.0")],
    )]);
    let report = match_addons(&inputs, &rows, None, &lookups);
    let picks = report
        .results
        .iter()
        .filter(|result| result.preselected())
        .filter_map(FindResult::found)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        picks
            .iter()
            .map(|pick| pick.parent_folder.as_str())
            .collect::<Vec<_>>(),
        ["GatherMate2_Data", "GatherMate2"]
    );
    picks
}

#[test]
fn a_folder_that_joins_one_pick_and_leaves_another_keeps_both_rows_in_either_order() {
    for reversed in [false, true] {
        let mut fixture = fixture();
        let mut picks = gathermate2_split(&fixture);
        if reversed {
            picks.reverse();
        }

        let linked = link_found_addons(
            &mut fixture.database,
            &fixture.addons,
            GameFlavor::Forever,
            &picks,
        )
        .expect("link");

        let mut linked = linked
            .iter()
            .map(|row| (row.folder.as_str(), row.source))
            .collect::<Vec<_>>();
        linked.sort_by_key(|(folder, _)| *folder);
        assert_eq!(
            linked,
            [
                ("GatherMate2", SourceKind::WowInterface),
                ("GatherMate2_Data", SourceKind::Wago),
            ],
            "reversed={reversed}"
        );
        assert_eq!(
            rows_by_folder(&fixture.database),
            [
                (
                    "GatherMate2".to_string(),
                    SourceKind::WowInterface,
                    Vec::new()
                ),
                ("GatherMate2_Data".to_string(), SourceKind::Wago, Vec::new()),
            ],
            "reversed={reversed}"
        );
    }
}

/// A Not linked row that still lists a linked addon's folder, as after a scan that failed.
fn stale_parent(fixture: &Fixture) -> (FindInputs, Vec<AddonRecord>) {
    addon(&fixture.addons, "Parent", &["## Version: 2.0"]);
    addon(&fixture.addons, "Child", &["## Version: 1.0"]);
    fixture
        .database
        .upsert_addon(&AddonRecord::new("Child", "Child", SourceKind::Wago))
        .expect("linked row");
    fixture
        .database
        .upsert_addon(&manual_row("Parent", &["Child"]))
        .expect("stale row");
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = fixture.database.list_addons().expect("rows");
    (inputs, rows)
}

fn parent_on_wowinterface() -> SourceLookups {
    let mut lookups = lookups(SourceAnswer::Found(Vec::new()));
    lookups.wowinterface =
        SourceAnswer::Found(vec![wowi_file("9", "Parent", "2.0", &[("Parent", "2.0")])]);
    lookups
}

#[test]
fn a_not_linked_row_listing_a_linked_addons_folder_is_not_linkable() {
    let fixture = fixture();
    let (inputs, rows) = stale_parent(&fixture);

    let report = match_addons(&inputs, &rows, None, &parent_on_wowinterface());

    let found = result(&report, "Parent");
    assert!(!found.preselected());
    let candidate = found.found().expect("candidate");
    assert!(candidate.leaving.is_empty());
    assert_eq!(
        candidate.blocked,
        [BlockedFolder {
            folder: "Child".to_string(),
            reason: BlockReason::Linked {
                addon: "Child".to_string()
            },
        }]
    );
    // The linked addon's folder is not part of the Not linked addon, so it is never sent.
    assert!(
        inputs
            .wago_lookups(&rows, None)
            .iter()
            .all(|lookup| lookup.folder != "Child")
    );
}

#[test]
fn linking_never_deletes_a_linked_addons_row_a_not_linked_row_still_lists() {
    let mut fixture = fixture();
    let (inputs, rows) = stale_parent(&fixture);
    let mut pick = result(
        &match_addons(&inputs, &rows, None, &parent_on_wowinterface()),
        "Parent",
    )
    .found()
    .expect("candidate")
    .clone();
    // Even a pick that looks linkable (planned before the row went stale) is refused.
    pick.blocked.clear();

    let error = link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect_err("refused");

    assert!(
        error.to_string().contains("Child belongs to Child"),
        "{error}"
    );
    assert_eq!(fixture.database.list_addons().expect("rows"), rows);
}

#[test]
fn files_wago_says_differ_stay_a_partial_match_when_wowinterface_versions_match() {
    let fixture = fixture();
    addon(&fixture.addons, "Clique", &["## Version: v5.1.5-release"]);
    addon(
        &fixture.addons,
        "Clique_Options",
        &["## Version: v5.1.5-release"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut project = wago(
        "CLQ",
        "Clique",
        vec![
            ("Clique".to_string(), hash_of(&inputs, "Clique")),
            (
                "Clique_Options".to_string(),
                "00000000000000000000000000000000".to_string(),
            ),
        ],
    );
    project.wowinterface_id = Some("5108".to_string());
    let mut lookups = lookups(SourceAnswer::Found(vec![project]));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "5108",
        "Clique",
        "v5.1.5-release",
        &[
            ("Clique", "v5.1.5-release"),
            ("Clique_Options", "v5.1.5-release"),
        ],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    let found = result(&report, "Clique");
    assert_eq!(found.label, FindLabel::Partial);
    assert!(!found.preselected());
    let candidate = found.found().expect("candidate");
    assert_eq!(candidate.source, FindSource::Wago);
    assert_eq!(candidate.differing, ["Clique_Options"]);
    assert_eq!(report.results.len(), 1);
}

#[test]
fn every_folder_a_match_takes_over_joins_whether_or_not_it_has_a_row() {
    let fixture = fixture();
    addon(&fixture.addons, "GatherMate2", &["## Version: 2.0"]);
    addon(
        &fixture.addons,
        "GatherMate2Storage_Midnight",
        &["## Version: 2.0"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let modules = vec![
        ("GatherMate2".to_string(), hash_of(&inputs, "GatherMate2")),
        (
            "GatherMate2Storage_Midnight".to_string(),
            hash_of(&inputs, "GatherMate2Storage_Midnight"),
        ),
    ];
    let lookups = lookups(SourceAnswer::Found(vec![wago(
        "GM2",
        "GatherMate2",
        modules,
    )]));
    let joining = [FolderMove {
        folder: "GatherMate2Storage_Midnight".to_string(),
        from: "GatherMate2Storage_Midnight".to_string(),
    }];

    // Not scanned yet: no rows at all.
    let unscanned = match_addons(&inputs, &[], None, &lookups);
    // Scanned: GatherMate2 has a row, the storage folder doesn't.
    let partly = match_addons(&inputs, &[manual_row("GatherMate2", &[])], None, &lookups);

    for report in [unscanned, partly] {
        let candidate = result(&report, "GatherMate2")
            .found()
            .expect("candidate")
            .clone();
        assert_eq!(candidate.joining, joining);
        assert!(candidate.regroups());
    }
}

#[test]
fn an_incomplete_wago_answer_leaves_what_it_did_not_match_not_checked() {
    let fixture = fixture();
    addon(&fixture.addons, "Pawn", &["## Version: 1.0"]);
    addon(&fixture.addons, "Spy", &["## Version: 3.0"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let pawn = wago(
        "PAWN",
        "Pawn",
        vec![("Pawn".to_string(), hash_of(&inputs, "Pawn"))],
    );

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Incomplete(vec![pawn])),
    );

    assert_eq!(result(&report, "Pawn").label, FindLabel::Exact);
    let spy = result(&report, "Spy");
    assert_eq!(spy.label, FindLabel::NotChecked);
    assert_eq!(spy.not_checked_by, [FindSource::Wago]);
    assert_eq!(report.failed, [FindSource::Wago]);
}

#[test]
fn a_wago_match_without_a_release_label_claims_no_status() {
    let mut fixture = fixture();
    addon(&fixture.addons, "Pawn", &["## Version: 2.5.0"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let mut project = wago(
        "PAWN",
        "Pawn",
        vec![("Pawn".to_string(), hash_of(&inputs, "Pawn"))],
    );
    project.matched_version = None;
    project.latest_version = Some("v2.5.0".to_string());
    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(vec![project])),
    );
    let pick = result(&report, "Pawn").found().expect("pick").clone();
    assert_eq!(pick.label, FindLabel::Exact);

    let linked = link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect("link");

    let row = &linked[0];
    assert_eq!(row.source, SourceKind::Wago);
    assert_eq!(row.version.as_deref(), Some("2.5.0"));
    assert_eq!(row.remote_version, None);
    assert_eq!(row.last_checked_at, None);
}

#[test]
fn a_folder_linked_on_its_own_leaves_its_old_addon_which_links_later_without_losing_it() {
    let mut fixture = fixture();
    let picks = gathermate2_split(&fixture);
    let data = picks
        .iter()
        .find(|pick| pick.parent_folder == "GatherMate2_Data")
        .expect("data pick")
        .clone();

    link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[data],
    )
    .expect("link data");

    assert_eq!(
        rows_by_folder(&fixture.database),
        [
            ("GatherMate2".to_string(), SourceKind::Manual, Vec::new()),
            ("GatherMate2_Data".to_string(), SourceKind::Wago, Vec::new()),
        ]
    );

    // A later find, before any scan, links GatherMate2 on its own.
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = fixture.database.list_addons().expect("rows");
    let mut lookups = lookups(SourceAnswer::Found(Vec::new()));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "7",
        "GatherMate2",
        "2.0",
        &[("GatherMate2", "2.0")],
    )]);
    let report = match_addons(&inputs, &rows, None, &lookups);
    let main = result(&report, "GatherMate2");
    assert!(main.preselected());
    assert!(!main.found().expect("candidate").regroups());
    let pick = main.found().expect("candidate").clone();
    link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect("link parent");

    // A scan afterwards keeps both.
    let tracked = fixture.database.list_addons().expect("rows");
    let scan = crate::scan::scan_addons_dir_report(&fixture.addons, GameFlavor::Forever, &tracked)
        .expect("scan");
    fixture.database.reconcile_scan(&scan).expect("reconcile");
    assert_eq!(
        rows_by_folder(&fixture.database),
        [
            (
                "GatherMate2".to_string(),
                SourceKind::WowInterface,
                Vec::new()
            ),
            ("GatherMate2_Data".to_string(), SourceKind::Wago, Vec::new()),
        ]
    );
}

#[test]
fn a_folder_that_moves_out_keeps_the_toc_overrides_of_the_addon_it_leaves() {
    let mut fixture = fixture();
    addon(&fixture.addons, "Group", &["## Version: 1.0"]);
    addon(&fixture.addons, "Group_Data", &["## Version: 1.0"]);
    let mut group = manual_row("Group", &["Group_Data"]);
    group.toc_overrides = BTreeMap::from([("Category".to_string(), "Mine".to_string())]);
    fixture.database.upsert_addon(&group).expect("group");
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Forever).expect("inputs");
    let rows = fixture.database.list_addons().expect("rows");
    let mut project = wago(
        "DATA",
        "Group Data",
        vec![("Group_Data".to_string(), hash_of(&inputs, "Group_Data"))],
    );
    project.matched_version = Some("1.0".to_string());
    project.latest_version = Some("1.0".to_string());
    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![project])),
    );
    let pick = report
        .results
        .iter()
        .filter_map(|result| result.found())
        .find(|found| found.parent_folder == "Group_Data")
        .expect("Group_Data pick")
        .clone();

    link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Forever,
        &[pick],
    )
    .expect("link");

    let rows = fixture.database.list_addons().expect("rows");
    let data = rows
        .iter()
        .find(|row| row.folder == "Group_Data")
        .expect("Group_Data row");
    assert_eq!(data.source, SourceKind::Wago);
    assert_eq!(
        data.toc_overrides,
        BTreeMap::from([("Category".to_string(), "Mine".to_string())])
    );
    let group = rows
        .iter()
        .find(|row| row.folder == "Group")
        .expect("Group row");
    assert_eq!(
        group.toc_overrides,
        BTreeMap::from([("Category".to_string(), "Mine".to_string())])
    );
}

/// Deadly Boss Mods as one Wago.io package: DBM-Core is the folder the others require, though
/// DBM-GUI has the shortest name.
fn dbm_package(fixture: &Fixture) -> (FindInputs, Vec<AddonRecord>, WagoMatch) {
    addon(
        &fixture.addons,
        "DBM-Core",
        &["## Title: |cffffe00a<|r|cffff7d0aDBM Core|r|cffffe00a>|r |cff69ccf0Main Core|r"],
    );
    addon(
        &fixture.addons,
        "DBM-GUI",
        &["## Title: <DBM Core> Options GUI"],
    );
    for folder in [
        "DBM-Brawlers",
        "DBM-Raids-Midnight",
        "DBM-StatusBarTimers",
        "DBM-VPVEM",
    ] {
        addon(&fixture.addons, folder, &["## Dependencies: DBM-Core"]);
    }
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let folders = [
        "DBM-Brawlers",
        "DBM-Core",
        "DBM-GUI",
        "DBM-Raids-Midnight",
        "DBM-StatusBarTimers",
        "DBM-VPVEM",
    ];
    let rows = folders
        .iter()
        .map(|folder| manual_row(folder, &[]))
        .collect();
    let modules = folders
        .iter()
        .map(|folder| (folder.to_string(), hash_of(&inputs, folder)))
        .collect();
    (inputs, rows, wago("DBM", "Deadly Boss Mods", modules))
}

#[test]
fn a_packages_main_folder_is_the_one_the_others_require() {
    let fixture = fixture();
    let (inputs, rows, project) = dbm_package(&fixture);

    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![project])),
    );

    assert_eq!(report.results.len(), 1, "{:#?}", report.results);
    let found = &report.results[0];
    assert_eq!(found.label, FindLabel::Exact);
    assert_eq!(found.folder, "DBM-Core");
    let candidate = found.found().expect("candidate");
    assert_eq!(candidate.parent_folder, "DBM-Core");
    assert_eq!(candidate.folders[0], "DBM-Core");
}

#[test]
fn linking_a_package_lists_it_under_the_folder_the_others_require() {
    let mut fixture = fixture();
    let (inputs, rows, project) = dbm_package(&fixture);
    let report = match_addons(
        &inputs,
        &rows,
        None,
        &lookups(SourceAnswer::Found(vec![project])),
    );
    let pick = report.results[0].found().expect("pick").clone();

    let linked = link_found_addons(
        &mut fixture.database,
        &fixture.addons,
        GameFlavor::Retail,
        &[pick],
    )
    .expect("link");

    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].folder, "DBM-Core");
    assert_eq!(
        linked[0]
            .owned_folders
            .iter()
            .map(|owned| owned.name.as_str())
            .collect::<Vec<_>>(),
        [
            "DBM-Brawlers",
            "DBM-GUI",
            "DBM-Raids-Midnight",
            "DBM-StatusBarTimers",
            "DBM-VPVEM"
        ]
    );
}

#[test]
fn a_folder_named_like_the_project_is_the_parent_before_any_requirement() {
    let fixture = fixture();
    addon(&fixture.addons, "Bar", &["## Dependencies: Foo"]);
    addon(&fixture.addons, "Foo", &[]);
    addon(&fixture.addons, "FooBar", &["## Dependencies: Bar"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let modules = ["Bar", "Foo", "FooBar"]
        .iter()
        .map(|folder| (folder.to_string(), hash_of(&inputs, folder)))
        .collect();

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(vec![wago("FB", "Foo Bar", modules)])),
    );

    assert_eq!(report.results[0].folder, "FooBar");
}

#[test]
fn an_exact_match_outweighs_a_version_mismatch_on_another_source() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Pawn",
        &["## Version: 3.0", "## X-WoWI-ID: 11"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "PWN",
        "Pawn",
        vec![("Pawn".to_string(), hash_of(&inputs, "Pawn"))],
    )]));
    lookups.wowinterface =
        SourceAnswer::Found(vec![wowi_file("11", "Pawn", "2.0", &[("Pawn", "2.0")])]);

    let report = match_addons(&inputs, &[], None, &lookups);

    assert_eq!(report.results.len(), 1, "{:#?}", report.results);
    let found = result(&report, "Pawn");
    assert_eq!(found.label, FindLabel::Exact);
    assert!(found.preselected());
    assert_eq!(found.candidates.len(), 1);
    assert_eq!(found.found().expect("candidate").source, FindSource::Wago);
}

#[test]
fn an_exact_match_outweighs_a_toc_id_on_another_source() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "HandyNotes",
        &["## Version: 1.0", "## X-WoWI-ID: 22"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "HN",
        "HandyNotes",
        vec![("HandyNotes".to_string(), hash_of(&inputs, "HandyNotes"))],
    )]));
    // The WoWInterface file lists a folder that is not here, so only the .toc's ID links them.
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "22",
        "HandyNotes",
        "1.0",
        &[("HandyNotes", "1.0"), ("HandyNotes_Extra", "1.0")],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    assert_eq!(report.results.len(), 1, "{:#?}", report.results);
    let found = result(&report, "HandyNotes");
    assert_eq!(found.label, FindLabel::Exact);
    assert_eq!(found.candidates.len(), 1);
    assert_eq!(found.found().expect("candidate").source, FindSource::Wago);
}

#[test]
fn a_likely_match_beside_a_partial_one_is_still_a_choice() {
    let fixture = fixture();
    addon(&fixture.addons, "Questie", &["## X-Wago-ID: QID"]);
    addon(&fixture.addons, "QuestieDB", &["## X-Wago-ID: QID"]);
    addon(&fixture.addons, "QuestieDev", &["## X-Wago-ID: QID"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let modules = vec![
        ("Questie".to_string(), hash_of(&inputs, "Questie")),
        ("QuestieDB".to_string(), "0".repeat(32)),
    ];

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(vec![wago("QID", "Questie", modules)])),
    );

    let choose = report
        .results
        .iter()
        .find(|result| result.label == FindLabel::ChooseOne)
        .unwrap_or_else(|| panic!("{:#?}", report.results));
    let parents = choose
        .candidates
        .iter()
        .map(|candidate| (candidate.parent_folder.as_str(), candidate.label))
        .collect::<Vec<_>>();
    assert_eq!(
        parents,
        [
            ("QuestieDev", FindLabel::Likely),
            ("Questie", FindLabel::Partial)
        ]
    );
}

fn likely_reason_of(report: &FindReport, folder: &str) -> Option<LikelyReason> {
    let found = result(report, folder);
    assert_eq!(found.label, FindLabel::Likely, "{:#?}", report.results);
    found.found().expect("candidate").likely_reason.clone()
}

#[test]
fn a_likely_match_says_what_was_checked_on_wago() {
    let fixture = fixture();
    addon(
        &fixture.addons,
        "Hekili",
        &["## Version: 1.0", "## X-Wago-ID: HK"],
    );
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let modules = vec![("Hekili".to_string(), "0".repeat(32))];

    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(vec![wago("HK", "Hekili", modules)])),
    );
    assert_eq!(
        likely_reason_of(&report, "Hekili"),
        Some(LikelyReason::FilesMatchNoRelease)
    );

    // Wago.io answered but returned nothing for it: its hashed files match no release there.
    let report = match_addons(
        &inputs,
        &[],
        None,
        &lookups(SourceAnswer::Found(Vec::new())),
    );
    assert_eq!(
        likely_reason_of(&report, "Hekili"),
        Some(LikelyReason::FilesMatchNoRelease)
    );

    // Without a Wago.io answer nothing but the .toc names it.
    let report = match_addons(&inputs, &[], None, &lookups(SourceAnswer::Skipped));
    assert_eq!(
        likely_reason_of(&report, "Hekili"),
        Some(LikelyReason::TocNamesIt)
    );
}

#[test]
fn a_likely_match_says_which_versions_were_compared() {
    let fixture = fixture();
    addon(&fixture.addons, "ElvUI", &["## Version: v15.17"]);
    addon(&fixture.addons, "Simulationcraft", &["## Version: 1.52.3"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Skipped);
    lookups.tukui = SourceAnswer::Found(vec![TukuiPackage {
        id: "elvui".to_string(),
        name: "ElvUI".to_string(),
        version: "16.01".to_string(),
        web_url: "https://tukui.org/elvui".to_string(),
        author: None,
        directories: vec!["ElvUI".to_string()],
    }]);
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "7",
        "Simulationcraft",
        "1.52.7",
        &[("Simulationcraft", "1.52.7")],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    assert_eq!(
        likely_reason_of(&report, "ElvUI"),
        Some(LikelyReason::VersionDiffers {
            folder: "ElvUI".to_string(),
            installed: Some("v15.17".to_string()),
            listed: Some("16.01".to_string()),
        })
    );
    assert_eq!(
        likely_reason_of(&report, "Simulationcraft"),
        Some(LikelyReason::VersionDiffers {
            folder: "Simulationcraft".to_string(),
            installed: Some("1.52.3".to_string()),
            listed: Some("1.52.7".to_string()),
        })
    );
}

#[test]
fn an_exact_match_that_cannot_be_linked_keeps_the_other_sources_candidate() {
    let fixture = fixture();
    addon(&fixture.addons, "Pack", &["## Version: 3.0"]);
    addon(&fixture.addons, "SharedLib", &[]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut linked = AddonRecord::new("Other", "Other", SourceKind::Wago);
    linked.set_managed_owned_folders(vec![OwnedFolder {
        name: "SharedLib".to_string(),
    }]);
    let rows = vec![linked, manual_row("Pack", &[])];
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "PCK",
        "Pack",
        vec![
            ("Pack".to_string(), hash_of(&inputs, "Pack")),
            ("SharedLib".to_string(), hash_of(&inputs, "SharedLib")),
        ],
    )]));
    lookups.wowinterface =
        SourceAnswer::Found(vec![wowi_file("33", "Pack", "2.0", &[("Pack", "2.0")])]);

    let report = match_addons(&inputs, &rows, None, &lookups);

    let found = result(&report, "Pack");
    assert!(
        found
            .candidates
            .iter()
            .any(|candidate| candidate.source == FindSource::WowInterface),
        "the WoWInterface candidate must stay: {:#?}",
        found.candidates
    );
}

#[test]
fn a_library_both_sources_ship_never_hides_the_other_addon() {
    let fixture = fixture();
    addon(&fixture.addons, "Pack", &["## Version: 3.0"]);
    addon(&fixture.addons, "SharedLib", &["## Version: 1.0"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Found(vec![wago(
        "LIB",
        "SharedLib",
        vec![("SharedLib".to_string(), hash_of(&inputs, "SharedLib"))],
    )]));
    lookups.wowinterface = SourceAnswer::Found(vec![wowi_file(
        "44",
        "Pack",
        "2.0",
        &[("Pack", "2.0"), ("SharedLib", "1.0")],
    )]);

    let report = match_addons(&inputs, &[], None, &lookups);

    assert!(
        report.results.iter().any(|result| result
            .candidates
            .iter()
            .any(|candidate| candidate.source == FindSource::WowInterface
                && candidate.parent_folder == "Pack")),
        "Pack's WoWInterface candidate must stay: {:#?}",
        report.results
    );
}

#[test]
fn a_folder_the_source_lists_no_version_for_says_so() {
    let fixture = fixture();
    addon(&fixture.addons, "Pack", &["## Version: 1.0"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let mut lookups = lookups(SourceAnswer::Skipped);
    let mut file = wowi_file("55", "Pack", "1.0", &[("Pack", "1.0")]);
    file.folders[0].version = None;
    lookups.wowinterface = SourceAnswer::Found(vec![file]);

    let report = match_addons(&inputs, &[], None, &lookups);

    assert_eq!(
        likely_reason_of(&report, "Pack"),
        Some(LikelyReason::VersionDiffers {
            folder: "Pack".to_string(),
            installed: Some("1.0".to_string()),
            listed: None,
        })
    );
}

#[test]
fn the_choice_of_sources_reads_names_without_case_and_ignores_unknown_ones() {
    let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
    // Missing means every source.
    assert_eq!(FindChoice::from_config(None), FindChoice::all());
    assert_eq!(
        FindChoice::from_config(Some(&names(&["Wago", "WOWI", "bogus"]))),
        FindChoice::only(&[FindSource::Wago, FindSource::WowInterface])
    );
    assert_eq!(
        FindChoice::from_config(Some(&names(&["tukui", "curseforge"]))).to_config(),
        names(&["tukui"])
    );
    // Nothing known, or nothing at all, is every source: nothing is saved with none checked.
    assert_eq!(
        FindChoice::from_config(Some(&names(&["curseforge"]))),
        FindChoice::all()
    );
    assert_eq!(FindChoice::from_config(Some(&[])), FindChoice::all());
    assert_eq!(
        FindChoice::all().to_config(),
        names(&["wago", "wowinterface", "tukui"])
    );
}

#[test]
fn wago_stays_wanted_without_a_key_but_is_never_asked() {
    let mut choice = FindChoice::all();
    assert_eq!(choice.asked(true), FindSource::ALL.to_vec());
    assert_eq!(
        choice.asked(false),
        vec![FindSource::WowInterface, FindSource::Tukui]
    );
    // No key does not change what is saved.
    assert!(choice.wants(FindSource::Wago));
    assert_eq!(choice.to_config().len(), 3);
    choice.toggle(FindSource::Wago);
    assert!(!choice.wants(FindSource::Wago));
    assert_eq!(
        choice.asked(true),
        vec![FindSource::WowInterface, FindSource::Tukui]
    );
}

#[test]
fn a_source_that_was_not_asked_finds_nothing_and_does_not_fail() {
    let fixture = fixture();
    addon(&fixture.addons, "Alpha", &["## Title: Alpha"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let report = match_addons(
        &inputs,
        &[],
        None,
        &SourceLookups {
            wago: SourceAnswer::NotAsked,
            wowinterface: SourceAnswer::Found(Vec::new()),
            tukui: SourceAnswer::NotAsked,
        },
    );
    assert!(report.failed.is_empty());
    // Left out by choice is not "a key finds more".
    assert!(report.skipped.is_empty());
    assert!(
        report
            .results
            .iter()
            .all(|result| result.not_checked_by.is_empty())
    );
}

#[test]
fn the_not_found_title_names_the_sources_asked_unless_all_were() {
    let report = |not_asked: &[FindSource]| FindReport {
        results: Vec::new(),
        failed: Vec::new(),
        skipped: Vec::new(),
        not_asked: not_asked.to_vec(),
    };
    assert_eq!(report(&[]).not_found_title(), "Not found");
    assert_eq!(
        report(&[FindSource::WowInterface]).not_found_title(),
        "Not found on Wago.io and TukUI"
    );
    assert_eq!(
        report(&[FindSource::Wago]).not_found_title(),
        "Not found on WoWInterface and TukUI"
    );
    assert_eq!(
        report(&[FindSource::Wago, FindSource::WowInterface]).not_found_title(),
        "Not found on TukUI"
    );
    // Nothing asked: there is nothing to name.
    assert_eq!(report(&FindSource::ALL).not_found_title(), "Not found");
}

#[test]
fn the_report_records_every_source_that_was_not_asked() {
    let fixture = fixture();
    addon(&fixture.addons, "Alpha", &["## Title: Alpha"]);
    let inputs = FindInputs::read(&fixture.addons, GameFlavor::Retail).expect("inputs");
    let report = |wago, wowinterface, tukui| {
        match_addons(
            &inputs,
            &[],
            None,
            &SourceLookups {
                wago,
                wowinterface,
                tukui,
            },
        )
    };
    let asked = report(
        SourceAnswer::Found(Vec::new()),
        SourceAnswer::Failed,
        SourceAnswer::Found(Vec::new()),
    );
    // A source that failed was still asked.
    assert!(asked.not_asked.is_empty());
    assert_eq!(asked.not_found_title(), "Not found");
    let left_out = report(
        SourceAnswer::Skipped,
        SourceAnswer::NotAsked,
        SourceAnswer::Found(Vec::new()),
    );
    assert_eq!(
        left_out.not_asked,
        vec![FindSource::Wago, FindSource::WowInterface]
    );
    assert_eq!(left_out.not_found_title(), "Not found on TukUI");
}
