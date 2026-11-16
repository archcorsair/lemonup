//! Which on-disk folders a provider update replaces, shared by every provider.

pub(crate) mod folder_swap;

use std::path::Path;

use lemonup_core::{AddonRecord, SourceKind, StateDatabase};

use crate::source_error::{FolderClash, SourceError, SourceErrorKind};

/// Plans the folders an update of `tracked` moves aside before copying `package_folders` in.
///
/// - A tracked addon with authoritative (managed) ownership replaces its parent and every owned
///   folder, as before.
/// - A tracked addon without authoritative ownership (none or scan-inferred, for example a row
///   migrated from v1) replaces only its parent; the rest of the folder set comes from the
///   package, exactly as a fresh install would take it.
/// - A package folder outside that set may be taken over when its existing row is manual
///   (untracked) or it has no row at all. After the update it becomes an owned folder of the
///   updated addon, and `record_managed_addon` drops its separate row.
/// - A package folder that is itself, or is owned by, a different tracked non-manual addon
///   refuses the whole update.
///
/// Returns the folders to move aside: those of the replaced set that exist on disk are moved by
/// the caller (missing ones are skipped there).
pub(crate) fn plan_update_replacements(
    database: &StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    package_folders: &[String],
) -> Result<Vec<String>, String> {
    let rows = database.list_addons().map_err(|error| error.to_string())?;
    plan_update_replacements_from_rows(&rows, addon_dir, tracked, package_folders)
}

fn plan_update_replacements_from_rows(
    rows: &[AddonRecord],
    addon_dir: &Path,
    tracked: &AddonRecord,
    package_folders: &[String],
) -> Result<Vec<String>, String> {
    let mut replaced = vec![tracked.folder.clone()];
    if tracked.has_authoritative_owned_folders() {
        for owned in &tracked.owned_folders {
            if !contains_folder(&replaced, &owned.name) {
                replaced.push(owned.name.clone());
            }
        }
    }

    let mut conflicts = Vec::new();
    for folder in package_folders {
        if contains_folder(&replaced, folder) {
            continue;
        }
        if let Some(owner) = tracked_owner_of(rows, &tracked.folder, folder) {
            conflicts.push(if same_folder_name(&owner, folder) {
                format!("{folder} (tracked separately)")
            } else {
                format!("{folder} (owned by tracked addon {owner})")
            });
            continue;
        }
        if addon_dir.join(folder).exists() {
            replaced.push(folder.clone());
        }
    }

    if conflicts.is_empty() {
        Ok(replaced)
    } else {
        Err(format!(
            "update refused because these addon folders are owned by something else on disk: {}",
            conflicts.join(", ")
        ))
    }
}

/// The folder of a tracked non-manual addon, other than `updating`, that is `folder` itself or
/// lists it among its owned folders.
fn tracked_owner_of(rows: &[AddonRecord], updating: &str, folder: &str) -> Option<String> {
    rows.iter()
        .filter(|row| row.folder != updating && row.source != SourceKind::Manual)
        .find(|row| row_claims_folder(row, folder))
        .map(|row| row.folder.clone())
}

/// Whether two AddOns folder names name the same folder on disk.
///
/// Windows and macOS (by default) have case-insensitive filesystems, so `Shared` and `shared`
/// are one directory there. Ownership checks always compare without case: on a case-sensitive
/// filesystem that only errs toward refusing, never toward replacing another addon's files.
fn same_folder_name(left: &str, right: &str) -> bool {
    left == right || left.to_lowercase() == right.to_lowercase()
}

fn contains_folder(folders: &[String], folder: &str) -> bool {
    folders.iter().any(|known| same_folder_name(known, folder))
}

/// Whether `row` is `folder` itself or lists it among its owned folders (ignoring case).
fn row_claims_folder(row: &AddonRecord, folder: &str) -> bool {
    same_folder_name(&row.folder, folder)
        || row
            .owned_folders
            .iter()
            .any(|owned| same_folder_name(&owned.name, folder))
}

/// Sorts the folders a fresh install of a package brings into what is in the way.
///
/// - A package folder that is the parent or an owned folder of a linked (non-manual) addon is
///   `linked`, on disk or not, since recording the install would take it from that addon.
///   `reinstalling` is the parent folder of the very addon being installed again (a Wago.io
///   reinstall); its own folders are not counted as linked.
/// - Any other package folder already in the AddOns folder is `replaceable`: it has no record or
///   a "Not linked" (manual) one.
pub(crate) fn classify_install_folders(
    rows: &[AddonRecord],
    addon_dir: &Path,
    package_folders: &[String],
    parent_folder: &str,
    reinstalling: Option<&str>,
) -> FolderClash {
    let mut clash = FolderClash::default();
    for folder in package_folders {
        let owner = rows
            .iter()
            .filter(|row| {
                row.source != SourceKind::Manual && Some(row.folder.as_str()) != reinstalling
            })
            .find(|row| row_claims_folder(row, folder));
        if let Some(owner) = owner {
            let name = if owner.name.trim().is_empty() {
                owner.folder.clone()
            } else {
                owner.name.clone()
            };
            clash.linked.push((folder.clone(), name));
        } else if addon_dir.join(folder).exists() {
            clash.replaceable.push(folder.clone());
        }
    }
    // The "Not linked" addon already there: the package parent first, then any other folder.
    let manual_name = |folder: &str| {
        rows.iter()
            .find(|row| row.source == SourceKind::Manual && same_folder_name(&row.folder, folder))
            .map(|row| row.name.trim().to_string())
            .filter(|name| !name.is_empty())
    };
    clash.existing_name = clash
        .replaceable
        .iter()
        .find(|folder| folder.as_str() == parent_folder)
        .and_then(|folder| manual_name(folder))
        .or_else(|| {
            clash
                .replaceable
                .iter()
                .find_map(|folder| manual_name(folder))
        });
    clash
}

/// The folders a fresh install may move aside: those in the way that are on `confirmed` (the
/// list the person said yes to; empty when nobody was asked).
///
/// Refuses with `FoldersOfLinkedAddon` when a package folder belongs to a linked addon, and with
/// `FoldersInTheWay` (a new question) when a folder in the way is not on `confirmed`, so a
/// download that changed since the question never replaces a folder nobody agreed to.
pub(crate) fn plan_install_replacements(
    database: &StateDatabase,
    addon_dir: &Path,
    package_folders: &[String],
    parent_folder: &str,
    confirmed: &[String],
    reinstalling: Option<&str>,
) -> Result<Vec<String>, SourceError> {
    let rows = database.list_addons().map_err(|error| error.to_string())?;
    plan_install_replacements_from_rows(
        &rows,
        addon_dir,
        package_folders,
        parent_folder,
        confirmed,
        reinstalling,
    )
}

fn plan_install_replacements_from_rows(
    rows: &[AddonRecord],
    addon_dir: &Path,
    package_folders: &[String],
    parent_folder: &str,
    confirmed: &[String],
    reinstalling: Option<&str>,
) -> Result<Vec<String>, SourceError> {
    let clash = classify_install_folders(
        rows,
        addon_dir,
        package_folders,
        parent_folder,
        reinstalling,
    );
    if !clash.linked.is_empty() {
        return Err(linked_folders_error(clash));
    }
    if clash
        .replaceable
        .iter()
        .any(|folder| !confirmed.contains(folder))
    {
        return Err(SourceError::folders(
            SourceErrorKind::FoldersInTheWay,
            format!(
                "install refused because these addon folders already exist: {}",
                clash.replaceable.join(", ")
            ),
            clash,
        ));
    }
    Ok(clash.replaceable)
}

/// The refusal for package folders that belong to linked addons (`clash.linked`).
pub(crate) fn linked_folders_error(clash: FolderClash) -> SourceError {
    let named = clash
        .linked
        .iter()
        .map(|(folder, owner)| format!("{folder} ({owner})"))
        .collect::<Vec<_>>()
        .join(", ");
    SourceError::folders(
        SourceErrorKind::FoldersOfLinkedAddon,
        format!("install refused because these addon folders belong to linked addons: {named}"),
        clash,
    )
}

#[cfg(test)]
mod tests {
    use super::{plan_install_replacements_from_rows, plan_update_replacements_from_rows};
    use crate::source_error::SourceErrorKind;
    use lemonup_core::{AddonRecord, OwnedFolder, SourceKind};
    use tempfile::tempdir;

    fn owned(names: &[&str]) -> Vec<OwnedFolder> {
        names
            .iter()
            .map(|name| OwnedFolder {
                name: (*name).to_string(),
            })
            .collect()
    }

    fn folders(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn make_dirs(root: &std::path::Path, names: &[&str]) {
        for name in names {
            std::fs::create_dir_all(root.join(name)).expect("create folder");
        }
    }

    #[test]
    fn unowned_tracked_addon_takes_its_folder_set_from_the_package() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["AstralKeys"]);
        let tracked = AddonRecord::new("AstralKeys", "AstralKeys", SourceKind::Wago);

        let replaced = plan_update_replacements_from_rows(
            std::slice::from_ref(&tracked),
            temp.path(),
            &tracked,
            &folders(&["AstralKeys"]),
        )
        .expect("plan");

        assert_eq!(replaced, folders(&["AstralKeys"]));
    }

    #[test]
    fn scan_inferred_children_outside_the_package_are_left_alone() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Pawn", "Pawn_Extra", "Pawn_Old"]);
        let mut tracked = AddonRecord::new("Pawn", "Pawn", SourceKind::Wago);
        tracked.set_scan_owned_folders(owned(&["Pawn_Extra", "Pawn_Old"]));

        let replaced = plan_update_replacements_from_rows(
            std::slice::from_ref(&tracked),
            temp.path(),
            &tracked,
            &folders(&["Pawn", "Pawn_Extra"]),
        )
        .expect("plan");

        assert_eq!(replaced, folders(&["Pawn", "Pawn_Extra"]));
    }

    #[test]
    fn managed_addon_still_replaces_every_owned_folder() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["ElvUI", "ElvUI_Options", "ElvUI_Libraries"]);
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        tracked.set_managed_owned_folders(owned(&["ElvUI_Options", "ElvUI_Libraries"]));

        let replaced = plan_update_replacements_from_rows(
            std::slice::from_ref(&tracked),
            temp.path(),
            &tracked,
            &folders(&["ElvUI", "ElvUI_Options"]),
        )
        .expect("plan");

        assert_eq!(
            replaced,
            folders(&["ElvUI", "ElvUI_Options", "ElvUI_Libraries"])
        );
    }

    #[test]
    fn manual_rows_and_rowless_folders_in_the_package_are_taken_over() {
        let temp = tempdir().expect("tempdir");
        make_dirs(
            temp.path(),
            &["DBM-Core", "DBM-GUI", "DBM-Brawlers", "DBM-StatusBarTimers"],
        );
        let tracked = AddonRecord::new("DBM", "DBM-Core", SourceKind::WowInterface);
        let mut manual_gui = AddonRecord::new("DBM-GUI", "DBM-GUI", SourceKind::Manual);
        // A manual row's scan-inferred children are untracked as well.
        manual_gui.set_scan_owned_folders(owned(&["DBM-Brawlers"]));
        let rows = vec![tracked.clone(), manual_gui];

        let replaced = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&[
                "DBM-Core",
                "DBM-GUI",
                "DBM-Brawlers",
                "DBM-StatusBarTimers",
                "DBM-VPVEM",
            ]),
        )
        .expect("plan");

        // DBM-VPVEM is not on disk, so there is nothing to move aside for it.
        assert_eq!(
            replaced,
            folders(&["DBM-Core", "DBM-GUI", "DBM-Brawlers", "DBM-StatusBarTimers"])
        );
    }

    #[test]
    fn package_folder_tracked_by_another_addon_refuses_the_update() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["GatherMate2", "GatherMate2_Data"]);
        let tracked = AddonRecord::new("GatherMate2", "GatherMate2", SourceKind::WowInterface);
        let other = AddonRecord::new("GatherMate2_Data", "GatherMate2_Data", SourceKind::Wago);
        let rows = vec![tracked.clone(), other];

        let error = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&["GatherMate2", "GatherMate2_Data"]),
        )
        .expect_err("tracked folder must not be taken over");

        assert_eq!(
            error,
            "update refused because these addon folders are owned by something else on disk: GatherMate2_Data (tracked separately)"
        );
    }

    #[test]
    fn package_folder_owned_by_another_tracked_addon_refuses_the_update() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Foo", "Shared_Lib"]);
        let tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        let mut other = AddonRecord::new("Bar", "Bar", SourceKind::Tukui);
        other.set_managed_owned_folders(owned(&["Shared_Lib"]));
        let rows = vec![tracked.clone(), other];

        let error = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&["Foo", "Shared_Lib"]),
        )
        .expect_err("folder owned by another tracked addon must not be taken over");

        assert!(
            error.ends_with("Shared_Lib (owned by tracked addon Bar)"),
            "{error}"
        );
    }

    #[test]
    fn update_refuses_a_folder_another_addon_owns_in_different_case() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Foo", "Shared"]);
        let tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        let mut other = AddonRecord::new("Bar", "Bar", SourceKind::Tukui);
        other.set_managed_owned_folders(owned(&["Shared"]));
        let separate = AddonRecord::new("LibThing", "LibThing", SourceKind::Wago);
        let rows = vec![tracked.clone(), other, separate];

        // On a case-insensitive filesystem `shared` is the `Shared` folder Bar owns, and
        // `LIBTHING` is the separately tracked `LibThing`.
        let error = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&["Foo", "shared", "LIBTHING"]),
        )
        .expect_err("a differently cased folder of another tracked addon must not be taken");

        assert!(
            error.contains("shared (owned by tracked addon Bar)"),
            "{error}"
        );
        assert!(error.contains("LIBTHING (tracked separately)"), "{error}");
    }

    #[test]
    fn update_matches_its_own_folders_in_different_case() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["ElvUI", "ElvUI_Options"]);
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        tracked.set_managed_owned_folders(owned(&["ElvUI_Options"]));

        let replaced = plan_update_replacements_from_rows(
            std::slice::from_ref(&tracked),
            temp.path(),
            &tracked,
            &folders(&["elvui", "ELVUI_OPTIONS"]),
        )
        .expect("own folders in another case");

        assert_eq!(replaced, folders(&["ElvUI", "ElvUI_Options"]));
    }

    #[test]
    fn install_refuses_a_linked_addons_folder_in_different_case() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["DBM-Core", "DBM-Raids", "Details"]);
        let mut dbm = AddonRecord::new("Deadly Boss Mods", "DBM-Core", SourceKind::Wago);
        dbm.set_managed_owned_folders(owned(&["DBM-Raids"]));
        let details = AddonRecord::new("Details!", "Details", SourceKind::GitHub);

        let error = plan_install_replacements_from_rows(
            &[dbm, details],
            temp.path(),
            &folders(&["MyRaidPack", "dbm-raids", "DETAILS"]),
            "MyRaidPack",
            &folders(&["dbm-raids", "DETAILS"]),
            None,
        )
        .expect_err("differently cased folders of linked addons");

        assert_eq!(error.kind, SourceErrorKind::FoldersOfLinkedAddon);
        assert_eq!(
            error.clash.expect("clash").linked,
            vec![
                ("dbm-raids".to_string(), "Deadly Boss Mods".to_string()),
                ("DETAILS".to_string(), "Details!".to_string()),
            ]
        );
    }

    fn manual(folder: &str, name: &str) -> AddonRecord {
        AddonRecord::new(name, folder, SourceKind::Manual)
    }

    #[test]
    fn install_with_nothing_in_the_way_replaces_nothing() {
        let temp = tempdir().expect("tempdir");
        let replaced = plan_install_replacements_from_rows(
            &[],
            temp.path(),
            &folders(&["Hekili"]),
            "Hekili",
            &[],
            None,
        )
        .expect("nothing in the way");
        assert!(replaced.is_empty());
    }

    #[test]
    fn install_asks_about_not_linked_folders_and_names_the_copy() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["hekili", "Hekili_Extra"]);
        let rows = vec![manual("hekili", "Hekili")];
        let error = plan_install_replacements_from_rows(
            &rows,
            temp.path(),
            &folders(&["hekili", "Hekili_Extra", "Hekili_New"]),
            "hekili",
            &[],
            None,
        )
        .expect_err("folders in the way need a yes");
        assert_eq!(error.kind, SourceErrorKind::FoldersInTheWay);
        let clash = error.clash.expect("clash");
        assert_eq!(clash.replaceable, folders(&["hekili", "Hekili_Extra"]));
        assert!(clash.linked.is_empty());
        assert_eq!(clash.existing_name.as_deref(), Some("Hekili"));
    }

    #[test]
    fn install_replaces_exactly_the_confirmed_folders() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["hekili", "Hekili_Extra"]);
        let confirmed = folders(&["hekili", "Hekili_Extra"]);
        let replaced = plan_install_replacements_from_rows(
            &[manual("hekili", "Hekili")],
            temp.path(),
            &folders(&["hekili", "Hekili_Extra"]),
            "hekili",
            &confirmed,
            None,
        )
        .expect("confirmed");
        assert_eq!(replaced, confirmed);
    }

    #[test]
    fn install_asks_again_when_the_download_brings_a_new_folder_in_the_way() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["hekili", "Hekili_Extra"]);
        // The question named only `hekili`; the fresh download also brings `Hekili_Extra`.
        let error = plan_install_replacements_from_rows(
            &[],
            temp.path(),
            &folders(&["hekili", "Hekili_Extra"]),
            "hekili",
            &folders(&["hekili"]),
            None,
        )
        .expect_err("a folder nobody agreed to");
        assert_eq!(error.kind, SourceErrorKind::FoldersInTheWay);
        assert_eq!(
            error.clash.expect("clash").replaceable,
            folders(&["hekili", "Hekili_Extra"])
        );
    }

    #[test]
    fn install_refuses_a_folder_owned_by_a_linked_addon() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["DBM-Core", "DBM-Raids"]);
        let mut linked = AddonRecord::new("Deadly Boss Mods", "DBM-Core", SourceKind::Wago);
        linked.owned_folders = owned(&["DBM-Core", "DBM-Raids"]);
        let error = plan_install_replacements_from_rows(
            &[linked],
            temp.path(),
            &folders(&["MyRaidPack", "DBM-Raids"]),
            "MyRaidPack",
            // Even a yes never takes a linked addon's folder.
            &folders(&["DBM-Raids"]),
            None,
        )
        .expect_err("owned by a linked addon");
        assert_eq!(error.kind, SourceErrorKind::FoldersOfLinkedAddon);
        let clash = error.clash.expect("clash");
        assert_eq!(
            clash.linked,
            vec![("DBM-Raids".to_string(), "Deadly Boss Mods".to_string())]
        );
    }

    #[test]
    fn install_refuses_the_parent_folder_of_a_linked_addon() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Details"]);
        let linked = AddonRecord::new("Details!", "Details", SourceKind::GitHub);
        let error = plan_install_replacements_from_rows(
            &[linked],
            temp.path(),
            &folders(&["Details"]),
            "Details",
            &folders(&["Details"]),
            None,
        )
        .expect_err("parent of a linked addon");
        assert_eq!(error.kind, SourceErrorKind::FoldersOfLinkedAddon);
        assert_eq!(
            error.clash.expect("clash").linked,
            vec![("Details".to_string(), "Details!".to_string())]
        );
    }

    #[test]
    fn reinstalling_the_same_addon_does_not_count_its_own_folders_as_linked() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["AstralKeys"]);
        let tracked = AddonRecord::new("AstralKeys", "AstralKeys", SourceKind::Wago);
        let replaced = plan_install_replacements_from_rows(
            &[tracked],
            temp.path(),
            &folders(&["AstralKeys"]),
            "AstralKeys",
            &folders(&["AstralKeys"]),
            Some("AstralKeys"),
        )
        .expect("own folders");
        assert_eq!(replaced, folders(&["AstralKeys"]));
    }
}
