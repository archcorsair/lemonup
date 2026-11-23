//! Which on-disk folders a provider update replaces, shared by every provider.

pub(crate) mod folder_swap;
pub(crate) mod toc_overrides;

pub(crate) use toc_overrides::{
    TocApplyError, apply_toc_overrides_now, stored_toc_overrides, swap_with_toc_overrides,
};

use std::path::Path;

use lemonup_core::{AddonRecord, SourceKind, StateDatabase};

use crate::source_error::{FolderClash, SourceError, SourceErrorKind};

/// Which "Not linked" folders an update may replace: those the person said yes to, or any
/// (`lemonup update --replace`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReplaceNotLinked<'a> {
    Only(&'a [String]),
    Any,
}

impl ReplaceNotLinked<'_> {
    /// Nobody was asked yet.
    pub(crate) const NONE: ReplaceNotLinked<'static> = ReplaceNotLinked::Only(&[]);

    fn allows(self, folder: &str) -> bool {
        match self {
            Self::Only(agreed) => contains_folder(agreed, folder),
            Self::Any => true,
        }
    }
}

/// What an update of one addon replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateReplacements {
    /// The folders to move aside: those that exist on disk are moved by the caller (missing ones
    /// are skipped there).
    pub(crate) replaced: Vec<String>,
    /// "Not linked" folders on disk the package would replace that nobody agreed to. While this
    /// is not empty the update must not touch anything.
    pub(crate) not_linked: Vec<String>,
}

/// Plans the folders an update of `tracked` moves aside before copying `package_folders` in.
///
/// - A tracked addon with authoritative (managed) ownership replaces its parent and every owned
///   folder, as before.
/// - A tracked addon without authoritative ownership (none or scan-inferred, for example a row
///   migrated from v1) replaces only its parent; the rest of the folder set comes from the
///   package, exactly as a fresh install would take it.
/// - A package folder that is already the addon's own (its parent, or an owned folder whether the
///   ownership is managed or scan-inferred) is replaced without asking, except a scan-inferred
///   folder with a `.git` entry: the scan groups folders by name, so a developer's clone next to
///   the addon lands there too, and it is asked about like a "Not linked" folder.
/// - Any other package folder on disk is "Not linked" (a manual row or no row): it is replaced
///   only when `agree` allows it, and otherwise listed in `not_linked`. After the update it
///   becomes an owned folder of the updated addon, and `record_managed_addon` drops its separate
///   row.
/// - A package folder that is itself, or is owned by, a different tracked non-manual addon, or
///   one a newer LemonUp linked (shown as "Not linked" here), refuses the whole update.
pub(crate) fn plan_update_replacements(
    database: &StateDatabase,
    addon_dir: &Path,
    tracked: &AddonRecord,
    package_folders: &[String],
    agree: ReplaceNotLinked<'_>,
) -> Result<UpdateReplacements, String> {
    let rows = database.list_addons().map_err(|error| error.to_string())?;
    plan_update_replacements_from_rows(&rows, addon_dir, tracked, package_folders, agree)
}

fn plan_update_replacements_from_rows(
    rows: &[AddonRecord],
    addon_dir: &Path,
    tracked: &AddonRecord,
    package_folders: &[String],
    agree: ReplaceNotLinked<'_>,
) -> Result<UpdateReplacements, String> {
    let mut replaced = vec![tracked.folder.clone()];
    if tracked.has_authoritative_owned_folders() {
        for owned in &tracked.owned_folders {
            if !contains_folder(&replaced, &owned.name) {
                replaced.push(owned.name.clone());
            }
        }
    }

    let mut conflicts = Vec::new();
    let mut not_linked = Vec::new();
    for folder in package_folders {
        if contains_folder(&replaced, folder) {
            continue;
        }
        if let Some(owner) = tracked_owner_of(rows, &tracked.folder, folder) {
            conflicts.push(if owner.unrecognized.is_some() {
                format!(
                    "{folder} (belongs to {}, which a newer LemonUp linked)",
                    display_name(owner)
                )
            } else if same_folder_name(&owner.folder, folder) {
                format!("{folder} (linked separately)")
            } else {
                format!("{folder} (owned by tracked addon {})", owner.folder)
            });
            continue;
        }
        if !addon_dir.join(folder).exists() {
            continue;
        }
        // Scan-inferred ownership (a row migrated from v1) still counts as the addon's own, unless
        // the folder is a dev copy. Managed owned folders are already in `replaced`.
        let inferred_own = tracked.claims_folder(folder) && !is_dev_copy(addon_dir, folder);
        if inferred_own || agree.allows(folder) {
            replaced.push(folder.clone());
        } else {
            not_linked.push(folder.clone());
        }
    }

    if conflicts.is_empty() {
        Ok(UpdateReplacements {
            replaced,
            not_linked,
        })
    } else {
        Err(format!(
            "update refused because these addon folders are owned by something else on disk: {}",
            conflicts.join(", ")
        ))
    }
}

/// Whether `folder` in `addon_dir` holds a `.git` entry (a folder or a file), so it is likely a
/// developer's working copy.
pub(crate) fn is_dev_copy(addon_dir: &Path, folder: &str) -> bool {
    std::fs::symlink_metadata(addon_dir.join(folder).join(".git")).is_ok()
}

/// The linked addon (see `AddonRecord::holds_its_folders`), other than `updating`, that is
/// `folder` itself or lists it among its owned folders.
fn tracked_owner_of<'a>(
    rows: &'a [AddonRecord],
    updating: &str,
    folder: &str,
) -> Option<&'a AddonRecord> {
    rows.iter()
        .filter(|row| row.folder != updating && row.holds_its_folders())
        .find(|row| row.claims_folder(folder))
}

fn display_name(row: &AddonRecord) -> String {
    if row.name.trim().is_empty() {
        row.folder.clone()
    } else {
        row.name.clone()
    }
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

/// Sorts the folders a fresh install of a package brings into what is in the way.
///
/// - A package folder that is the parent or an owned folder of a linked (non-manual) addon, or of
///   one a newer LemonUp linked, is `linked`, on disk or not, since recording the install would
///   take it from that addon. `reinstalling` is the parent folder of the very addon being
///   installed again (a Wago.io reinstall); its own folders are not counted as linked. An addon a
///   newer LemonUp linked whose folder is the package parent is the same addon installed here
///   (for example after "find online"), so it counts as "Not linked" and is asked about.
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
    // The linked addons that own a package folder, each by its own folder.
    let mut owners: Vec<&AddonRecord> = Vec::new();
    for folder in package_folders {
        let owner = rows
            .iter()
            .filter(|row| {
                row.holds_its_folders()
                    && Some(row.folder.as_str()) != reinstalling
                    && !(row.unrecognized.is_some() && same_folder_name(&row.folder, parent_folder))
            })
            .find(|row| row.claims_folder(folder));
        if let Some(owner) = owner {
            let name = display_name(owner);
            clash.newer_version_linked |= owner.unrecognized.is_some();
            owners.push(owner);
            clash.linked.push((folder.clone(), name));
        } else if addon_dir.join(folder).exists() {
            clash.replaceable.push(folder.clone());
        }
    }
    // LemonUp knows an addon by its parent folder. A linked addon whose own folder is the
    // package's parent folder is the addon being installed, already there: the one a
    // reinstall leaves out of `linked`, or the only owner in it. With another linked addon in
    // the way there is more to say than that.
    let same_addon = |row: &&AddonRecord| same_folder_name(&row.folder, parent_folder);
    let installed = match reinstalling {
        Some(own) if owners.is_empty() => rows
            .iter()
            .find(|row| row.holds_its_folders() && row.folder == own)
            .filter(same_addon),
        _ => owners
            .first()
            .copied()
            .filter(|_| owners.iter().all(same_addon)),
    };
    clash.installed = installed.map(|row| {
        let shown = row.display_name().trim();
        Box::new(crate::source_error::InstalledAddon {
            name: if shown.is_empty() {
                row.folder.clone()
            } else {
                shown.to_string()
            },
            folder: row.folder.clone(),
            source: row.source,
        })
    });
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
    use super::{
        ReplaceNotLinked, plan_install_replacements_from_rows, plan_update_replacements_from_rows,
    };
    use crate::source_error::SourceErrorKind;
    use lemonup_core::{AddonRecord, OwnedFolder, SourceKind, UnrecognizedValues};
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
            ReplaceNotLinked::NONE,
        )
        .expect("plan")
        .replaced;

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
            ReplaceNotLinked::NONE,
        )
        .expect("plan")
        .replaced;

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
            ReplaceNotLinked::NONE,
        )
        .expect("plan")
        .replaced;

        assert_eq!(
            replaced,
            folders(&["ElvUI", "ElvUI_Options", "ElvUI_Libraries"])
        );
    }

    fn dbm_with_not_linked_folders() -> (tempfile::TempDir, AddonRecord, Vec<AddonRecord>) {
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
        (temp, tracked, rows)
    }

    const DBM_PACKAGE: [&str; 5] = [
        "DBM-Core",
        "DBM-GUI",
        "DBM-Brawlers",
        "DBM-StatusBarTimers",
        "DBM-VPVEM",
    ];

    #[test]
    fn not_linked_package_folders_need_a_yes_before_an_update_replaces_them() {
        let (temp, tracked, rows) = dbm_with_not_linked_folders();

        let plan = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&DBM_PACKAGE),
            ReplaceNotLinked::NONE,
        )
        .expect("plan");

        // DBM-VPVEM is not on disk, so there is nothing to ask about or move aside for it.
        assert_eq!(plan.replaced, folders(&["DBM-Core"]));
        assert_eq!(
            plan.not_linked,
            folders(&["DBM-GUI", "DBM-Brawlers", "DBM-StatusBarTimers"])
        );
    }

    #[test]
    fn agreed_not_linked_folders_are_replaced() {
        let (temp, tracked, rows) = dbm_with_not_linked_folders();
        let expected = folders(&["DBM-Core", "DBM-GUI", "DBM-Brawlers", "DBM-StatusBarTimers"]);

        let agreed = folders(&["dbm-gui", "DBM-Brawlers", "DBM-StatusBarTimers"]);
        let plan = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&DBM_PACKAGE),
            ReplaceNotLinked::Only(&agreed),
        )
        .expect("plan");
        assert_eq!(plan.replaced, expected);
        assert!(plan.not_linked.is_empty());

        let plan = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&DBM_PACKAGE),
            ReplaceNotLinked::Any,
        )
        .expect("plan");
        assert_eq!(plan.replaced, expected);
        assert!(plan.not_linked.is_empty());
    }

    #[test]
    fn a_folder_nobody_agreed_to_is_still_asked_about() {
        let (temp, tracked, rows) = dbm_with_not_linked_folders();

        let agreed = folders(&["DBM-GUI", "DBM-Brawlers"]);
        let plan = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&DBM_PACKAGE),
            ReplaceNotLinked::Only(&agreed),
        )
        .expect("plan");

        assert_eq!(
            plan.replaced,
            folders(&["DBM-Core", "DBM-GUI", "DBM-Brawlers"])
        );
        assert_eq!(plan.not_linked, folders(&["DBM-StatusBarTimers"]));
    }

    #[test]
    fn inferred_owned_folders_of_a_migrated_row_need_no_yes() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["BigWigs", "BigWigs_Core", "BigWigs_Options"]);
        let mut tracked = AddonRecord::new("BigWigs", "BigWigs", SourceKind::GitHub);
        tracked.set_scan_owned_folders(owned(&["BigWigs_Core", "BigWigs_Options"]));

        let plan = plan_update_replacements_from_rows(
            std::slice::from_ref(&tracked),
            temp.path(),
            &tracked,
            &folders(&["BigWigs", "BigWigs_Core", "BigWigs_Options"]),
            ReplaceNotLinked::NONE,
        )
        .expect("plan");

        assert_eq!(
            plan.replaced,
            folders(&["BigWigs", "BigWigs_Core", "BigWigs_Options"])
        );
        assert!(plan.not_linked.is_empty());
    }

    #[test]
    fn an_inferred_folder_with_git_is_asked_about_but_a_managed_one_is_not() {
        let temp = tempdir().expect("tempdir");
        make_dirs(
            temp.path(),
            &[
                "BigWigs",
                "BigWigs_Core",
                "BigWigs_Options",
                "BigWigs_Plugins",
            ],
        );
        std::fs::create_dir(temp.path().join("BigWigs_Core").join(".git")).expect("git dir");
        std::fs::write(
            temp.path().join("BigWigs_Plugins").join(".git"),
            "gitdir: x",
        )
        .expect("git file");
        let package = folders(&[
            "BigWigs",
            "BigWigs_Core",
            "BigWigs_Options",
            "BigWigs_Plugins",
        ]);

        // A v1-migrated row: the scan grouped all three folders under BigWigs by name.
        let mut inferred = AddonRecord::new("BigWigs", "BigWigs", SourceKind::GitHub);
        inferred.set_scan_owned_folders(owned(&[
            "BigWigs_Core",
            "BigWigs_Options",
            "BigWigs_Plugins",
        ]));
        let plan = plan_update_replacements_from_rows(
            std::slice::from_ref(&inferred),
            temp.path(),
            &inferred,
            &package,
            ReplaceNotLinked::NONE,
        )
        .expect("plan");
        assert_eq!(plan.replaced, folders(&["BigWigs", "BigWigs_Options"]));
        assert_eq!(
            plan.not_linked,
            folders(&["BigWigs_Core", "BigWigs_Plugins"])
        );

        let agreed = folders(&["BigWigs_Core", "BigWigs_Plugins"]);
        let plan = plan_update_replacements_from_rows(
            std::slice::from_ref(&inferred),
            temp.path(),
            &inferred,
            &package,
            ReplaceNotLinked::Only(&agreed),
        )
        .expect("plan");
        assert!(plan.not_linked.is_empty());

        // The same folders installed and managed by LemonUp stay silent.
        let mut managed = inferred.clone();
        managed.set_managed_owned_folders(owned(&[
            "BigWigs_Core",
            "BigWigs_Options",
            "BigWigs_Plugins",
        ]));
        let plan = plan_update_replacements_from_rows(
            std::slice::from_ref(&managed),
            temp.path(),
            &managed,
            &package,
            ReplaceNotLinked::NONE,
        )
        .expect("plan");
        assert_eq!(plan.replaced, package);
        assert!(plan.not_linked.is_empty());
    }

    #[test]
    fn a_yes_never_takes_a_folder_of_another_linked_addon() {
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
            ReplaceNotLinked::Any,
        )
        .expect_err("a linked addon's folder is never replaced");

        assert!(
            error.ends_with("Shared_Lib (owned by tracked addon Bar)"),
            "{error}"
        );
    }

    #[test]
    fn a_folder_with_a_git_entry_is_a_dev_copy() {
        let temp = tempdir().expect("tempdir");
        make_dirs(
            temp.path(),
            &["BigWigs_Core/.git", "BigWigs_Options", "Worktree"],
        );
        std::fs::write(temp.path().join("Worktree/.git"), "gitdir: elsewhere").expect("file");

        assert!(super::is_dev_copy(temp.path(), "BigWigs_Core"));
        assert!(super::is_dev_copy(temp.path(), "Worktree"));
        assert!(!super::is_dev_copy(temp.path(), "BigWigs_Options"));
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
            ReplaceNotLinked::NONE,
        )
        .expect_err("tracked folder must not be taken over");

        assert_eq!(
            error,
            "update refused because these addon folders are owned by something else on disk: GatherMate2_Data (linked separately)"
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
            ReplaceNotLinked::NONE,
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
            ReplaceNotLinked::NONE,
        )
        .expect_err("a differently cased folder of another tracked addon must not be taken");

        assert!(
            error.contains("shared (owned by tracked addon Bar)"),
            "{error}"
        );
        assert!(error.contains("LIBTHING (linked separately)"), "{error}");
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
            ReplaceNotLinked::NONE,
        )
        .expect("own folders in another case")
        .replaced;

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

    #[test]
    fn a_refused_install_says_when_that_addon_is_already_installed() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Kui_Nameplates", "Kui_Media", "Other_Lib"]);
        let mut tracked = AddonRecord::new("Kui Title", "Kui_Nameplates", SourceKind::Wago);
        tracked.source_name = Some("Kui Nameplates".to_string());
        tracked.owned_folders = owned(&["Kui_Media"]);
        let other = AddonRecord::new("Other", "Other_Lib", SourceKind::GitHub);
        let rows = [tracked.clone(), other];
        let clash = |package: &[&str], parent: &str, reinstalling: Option<&str>| {
            plan_install_replacements_from_rows(
                &rows,
                temp.path(),
                &folders(package),
                parent,
                &[],
                reinstalling,
            )
            .expect_err("folders in the way")
        };

        // The package's parent folder is the linked addon's own folder, whatever its case: the
        // same addon, by the name LemonUp shows, its own folder and its source.
        let same = clash(&["Kui_Media", "kui_nameplates"], "kui_nameplates", None);
        assert_eq!(same.kind, SourceErrorKind::FoldersOfLinkedAddon);
        let installed = same.clash.expect("clash").installed.expect("installed");
        assert_eq!(installed.name, "Kui Nameplates");
        assert_eq!(installed.folder, "Kui_Nameplates");
        assert_eq!(installed.source, SourceKind::Wago);

        // A reinstall from Wago.io leaves the addon's own folders out of `linked`: they are in
        // the way, and it is still that addon.
        let again = clash(
            &["Kui_Media", "Kui_Nameplates"],
            "Kui_Nameplates",
            Some("Kui_Nameplates"),
        );
        assert_eq!(again.kind, SourceErrorKind::FoldersInTheWay);
        assert_eq!(
            again
                .clash
                .expect("clash")
                .installed
                .map(|installed| installed.folder),
            Some("Kui_Nameplates".to_string())
        );

        // Another addon that only brings one of its folders is not that addon.
        let plugin = clash(&["OtherPlates", "Kui_Media"], "OtherPlates", None);
        assert_eq!(plugin.kind, SourceErrorKind::FoldersOfLinkedAddon);
        assert_eq!(plugin.clash.expect("clash").installed, None);

        // With a second linked addon in the way there is more to say than "already installed".
        let two = clash(&["Kui_Nameplates", "Other_Lib"], "Kui_Nameplates", None);
        assert_eq!(two.kind, SourceErrorKind::FoldersOfLinkedAddon);
        assert_eq!(two.clash.expect("clash").installed, None);
    }

    /// A row a newer LemonUp wrote with a source this version does not know: it loads as
    /// "Not linked" and keeps the stored values.
    fn linked_by_newer_version(name: &str, folder: &str, owned_names: &[&str]) -> AddonRecord {
        let mut row = AddonRecord::new(name, folder, SourceKind::Manual);
        row.owned_folders = owned(owned_names);
        row.unrecognized = Some(UnrecognizedValues {
            source: "curseforge".to_string(),
            ..UnrecognizedValues::default()
        });
        row
    }

    #[test]
    fn update_refuses_folders_of_an_addon_a_newer_version_linked() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Foo", "Pawn", "Pawn_Extra"]);
        let tracked = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        let newer = linked_by_newer_version("Pawn", "Pawn", &["Pawn_Extra"]);
        let rows = vec![tracked.clone(), newer];

        let error = plan_update_replacements_from_rows(
            &rows,
            temp.path(),
            &tracked,
            &folders(&["Foo", "pawn", "Pawn_Extra"]),
            ReplaceNotLinked::NONE,
        )
        .expect_err("folders of an addon a newer version linked must not be taken");

        assert_eq!(
            error,
            "update refused because these addon folders are owned by something else on disk: \
             pawn (belongs to Pawn, which a newer LemonUp linked), \
             Pawn_Extra (belongs to Pawn, which a newer LemonUp linked)"
        );
    }

    #[test]
    fn install_refuses_folders_of_an_addon_a_newer_version_linked() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Pawn", "Pawn_Extra"]);
        let newer = linked_by_newer_version("Pawn", "Pawn", &["Pawn_Extra"]);

        for package in [&["MyPack", "Pawn"][..], &["MyPack", "pawn_extra"][..]] {
            let error = plan_install_replacements_from_rows(
                std::slice::from_ref(&newer),
                temp.path(),
                &folders(package),
                "MyPack",
                // Even a yes never takes them.
                &folders(&package[1..]),
                None,
            )
            .expect_err("folder of an addon a newer version linked");
            assert_eq!(error.kind, SourceErrorKind::FoldersOfLinkedAddon);
            let clash = error.clash.expect("clash");
            assert_eq!(
                clash.linked,
                vec![(package[1].to_string(), "Pawn".to_string())]
            );
            assert!(clash.newer_version_linked);
        }
    }

    #[test]
    fn installing_the_same_addon_over_one_a_newer_version_linked_asks_first() {
        let temp = tempdir().expect("tempdir");
        make_dirs(temp.path(), &["Pawn", "Pawn_Extra"]);
        let newer = linked_by_newer_version("Pawn", "Pawn", &["Pawn_Extra"]);
        let package = folders(&["Pawn", "Pawn_Extra"]);

        // Like any "Not linked" copy: replaced only after a yes that names every folder.
        let error = plan_install_replacements_from_rows(
            std::slice::from_ref(&newer),
            temp.path(),
            &package,
            "Pawn",
            &[],
            None,
        )
        .expect_err("asks first");
        assert_eq!(error.kind, SourceErrorKind::FoldersInTheWay);
        let clash = error.clash.expect("clash");
        assert_eq!(clash.replaceable, package);
        assert_eq!(clash.existing_name.as_deref(), Some("Pawn"));

        let replaced = plan_install_replacements_from_rows(
            std::slice::from_ref(&newer),
            temp.path(),
            &package,
            "Pawn",
            &package,
            None,
        )
        .expect("confirmed");
        assert_eq!(replaced, package);
    }
}
