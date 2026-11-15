//! Which on-disk folders a provider update replaces, shared by every provider.

pub(crate) mod folder_swap;

use std::path::Path;

use lemonup_core::{AddonRecord, SourceKind, StateDatabase};

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
            if !replaced.contains(&owned.name) {
                replaced.push(owned.name.clone());
            }
        }
    }

    let mut conflicts = Vec::new();
    for folder in package_folders {
        if replaced.contains(folder) {
            continue;
        }
        if let Some(owner) = tracked_owner_of(rows, &tracked.folder, folder) {
            conflicts.push(if owner == *folder {
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
        .find(|row| {
            row.folder == folder || row.owned_folders.iter().any(|owned| owned.name == folder)
        })
        .map(|row| row.folder.clone())
}

#[cfg(test)]
mod tests {
    use super::plan_update_replacements_from_rows;
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
}
