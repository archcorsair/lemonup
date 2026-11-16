//! Writes the user's TOC overrides (`Title`, `Notes`, `Category`) into an addon's folders, always
//! through [`FolderSwap`]: the edit happens on the staged copy, so a failure never leaves a live
//! folder half written.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use lemonup_core::StateDatabase;
use lemonup_core::toc_edit::{self, TocField};

use super::folder_swap::FolderSwap;
use super::{same_folder_name, tracked_owner_of};

/// The overrides stored for the addon whose parent folder is `parent_folder`, or none when it has
/// no row. Installs use this too, so linking a "Not linked" addon keeps the overrides set on it.
pub(crate) fn stored_toc_overrides(
    database: &StateDatabase,
    parent_folder: &str,
) -> Result<BTreeMap<String, String>, String> {
    Ok(database
        .get_addon_by_folder(parent_folder)
        .map_err(|error| error.to_string())?
        .map(|addon| addon.toc_overrides)
        .unwrap_or_default())
}

/// [`FolderSwap::apply`] that also writes `overrides` into the root TOC files of every staged
/// folder before anything live moves. Without overrides the package is placed as it is.
pub(crate) fn swap_with_toc_overrides(
    addon_dir: &Path,
    source_root: &Path,
    new_folders: &[String],
    replaced: &[String],
    overrides: &BTreeMap<String, String>,
) -> Result<FolderSwap, String> {
    if overrides.is_empty() {
        return FolderSwap::apply(addon_dir, source_root, new_folders, replaced);
    }
    FolderSwap::apply_prepared(
        addon_dir,
        source_root,
        new_folders,
        replaced,
        &mut |_, staged| write_overrides(staged, overrides),
    )
}

fn write_overrides(staged: &Path, overrides: &BTreeMap<String, String>) -> Result<(), String> {
    toc_edit::apply_overrides_to_folder(staged, overrides)
        .map(|_| ())
        .map_err(|error| format!("could not write the TOC overrides: {error}"))
}

/// Writes `overrides` into the installed addon whose parent folder is `folder` (its parent and
/// every owned folder its row lists) and stores them, all through one [`FolderSwap`] with the
/// AddOns folder itself as the source: the folders are copied to staging, edited there, swapped
/// in, and the swap is committed only after the database write. With no known field set this
/// restores the addon's own TOC lines.
///
/// Returns the folders that were rewritten. It asks nothing itself: the caller asks before
/// rewriting a "Not linked" addon. It refuses a folder with a symlink anywhere inside, because
/// the staged copy would drop it, and a folder another linked addon holds.
#[cfg_attr(not(test), allow(dead_code))] // Called by the TOC overrides sheet, coming next.
pub(crate) fn apply_toc_overrides_now(
    database: &mut StateDatabase,
    addon_dir: &Path,
    folder: &str,
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    for field in TocField::ALL {
        if let Some(value) = overrides.get(field.name()) {
            toc_edit::validate_override_value(value)
                .map_err(|error| format!("{} override: {error}", field.name()))?;
        }
    }
    let rows = database.list_addons().map_err(|error| error.to_string())?;
    let addon = rows
        .iter()
        .find(|row| row.folder == folder)
        .ok_or_else(|| format!("no addon is stored for folder '{folder}'"))?;

    let mut folders = vec![addon.folder.clone()];
    for owned in &addon.owned_folders {
        if !folders
            .iter()
            .any(|known| same_folder_name(known, &owned.name))
        {
            folders.push(owned.name.clone());
        }
    }
    if fs::symlink_metadata(addon_dir.join(&addon.folder)).is_err() {
        return Err(format!(
            "{} is not in the AddOns folder {}",
            addon.folder,
            addon_dir.display()
        ));
    }
    // Owned folders that are gone are skipped, like an update skips them.
    folders.retain(|name| fs::symlink_metadata(addon_dir.join(name)).is_ok());
    for name in &folders {
        if let Some(owner) = tracked_owner_of(&rows, &addon.folder, name) {
            return Err(format!(
                "{name} belongs to {}, so its TOC files were left alone",
                owner.folder
            ));
        }
        if let Some(link) = find_symlink(&addon_dir.join(name))? {
            return Err(format!(
                "{name} contains a symbolic link ({}); LemonUp can't rewrite its TOC files safely",
                link.display()
            ));
        }
    }

    let swap = FolderSwap::apply_prepared(
        addon_dir,
        addon_dir,
        &folders,
        &folders,
        &mut |_, staged| write_overrides(staged, overrides),
    )?;
    if let Err(error) = database.set_toc_overrides(&addon.folder, overrides) {
        return Err(swap.abort(error.to_string()));
    }
    swap.commit();
    Ok(folders)
}

/// The first symlink at or below `path`, never following one.
fn find_symlink(path: &Path) -> Result<Option<std::path::PathBuf>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(Some(path.to_path_buf()));
    }
    if metadata.is_dir() {
        let entries = fs::read_dir(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("could not read {}: {error}", path.display()))?;
            if let Some(link) = find_symlink(&entry.path())? {
                return Ok(Some(link));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use lemonup_core::{AddonRecord, OwnedFolder, SourceKind, StateDatabase};
    use tempfile::tempdir;

    use super::apply_toc_overrides_now;
    use crate::update_targets::folder_swap::STAGING_PREFIX;

    const FOO_TOC: &[u8] =
        b"\xEF\xBB\xBF## Interface: 110205\r\n## Title: Foo\r\n## Category: Combat\r\nFoo.lua";
    const OPTIONS_TOC: &[u8] = b"## Interface: 110205\n## Title: Foo Options\n## Notes: Caf\xE9\n";

    fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn seed(root: &Path) -> (std::path::PathBuf, StateDatabase) {
        let addon_dir = root.join("AddOns");
        fs::create_dir_all(addon_dir.join("Foo").join("Libs")).expect("foo");
        fs::create_dir_all(addon_dir.join("Foo_Options")).expect("options");
        fs::write(addon_dir.join("Foo").join("Foo.toc"), FOO_TOC).expect("toc");
        fs::write(
            addon_dir.join("Foo").join("Libs").join("Lib.toc"),
            b"## Title: Lib\n",
        )
        .expect("lib toc");
        fs::write(
            addon_dir.join("Foo_Options").join("Foo_Options.toc"),
            OPTIONS_TOC,
        )
        .expect("options toc");
        let mut database = StateDatabase::open(root.join("state.sqlite")).expect("open db");
        let mut addon = AddonRecord::new("Foo", "Foo", SourceKind::GitHub);
        addon.source_url = Some("https://github.com/x/Foo".to_string());
        addon.set_managed_owned_folders(vec![OwnedFolder {
            name: "Foo_Options".to_string(),
        }]);
        database.record_managed_addon(&addon).expect("seed");
        (addon_dir, database)
    }

    fn staging_left(addon_dir: &Path) -> bool {
        fs::read_dir(addon_dir)
            .expect("read")
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(STAGING_PREFIX)
            })
    }

    #[test]
    fn apply_now_rewrites_the_live_tocs_through_a_swap_and_clearing_restores_them() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = seed(temp.path());
        let wanted = overrides(&[("Category", "Mine"), ("Title", "My Foo")]);

        let rewritten =
            apply_toc_overrides_now(&mut database, &addon_dir, "Foo", &wanted).expect("apply");
        assert_eq!(rewritten, ["Foo", "Foo_Options"]);
        for toc in ["Foo/Foo.toc", "Foo_Options/Foo_Options.toc"] {
            let text =
                String::from_utf8_lossy(&fs::read(addon_dir.join(toc)).expect("read")).into_owned();
            assert!(text.contains("## Title: My Foo"), "{toc}: {text}");
            assert!(text.contains("## Category: Mine"), "{toc}: {text}");
        }
        assert_eq!(
            fs::read(addon_dir.join("Foo/Libs/Lib.toc")).expect("lib"),
            b"## Title: Lib\n"
        );
        assert!(!staging_left(&addon_dir));
        let stored = database
            .get_addon_by_folder("Foo")
            .expect("get")
            .expect("row");
        assert_eq!(stored.toc_overrides, wanted);

        apply_toc_overrides_now(&mut database, &addon_dir, "Foo", &BTreeMap::new()).expect("clear");
        assert_eq!(
            fs::read(addon_dir.join("Foo/Foo.toc")).expect("foo"),
            FOO_TOC
        );
        assert_eq!(
            fs::read(addon_dir.join("Foo_Options/Foo_Options.toc")).expect("options"),
            OPTIONS_TOC
        );
        assert!(
            database
                .get_addon_by_folder("Foo")
                .expect("get")
                .expect("row")
                .toc_overrides
                .is_empty()
        );
    }

    #[test]
    fn apply_now_refuses_invalid_values_and_unknown_addons_without_touching_files() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = seed(temp.path());
        assert!(
            apply_toc_overrides_now(
                &mut database,
                &addon_dir,
                "Foo",
                &overrides(&[("Notes", "two\nlines")])
            )
            .is_err()
        );
        assert!(
            apply_toc_overrides_now(
                &mut database,
                &addon_dir,
                "Missing",
                &overrides(&[("Notes", "x")])
            )
            .is_err()
        );
        assert_eq!(
            fs::read(addon_dir.join("Foo/Foo.toc")).expect("foo"),
            FOO_TOC
        );
        assert!(!staging_left(&addon_dir));
    }

    #[cfg(unix)]
    #[test]
    fn apply_now_refuses_a_folder_with_a_symlink_inside() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = seed(temp.path());
        let shared = temp.path().join("shared-media");
        fs::create_dir_all(&shared).expect("shared");
        std::os::unix::fs::symlink(&shared, addon_dir.join("Foo_Options").join("Media"))
            .expect("symlink");

        let error = apply_toc_overrides_now(
            &mut database,
            &addon_dir,
            "Foo",
            &overrides(&[("Title", "My Foo")]),
        )
        .expect_err("refused");
        assert!(error.contains("symbolic link"), "{error}");
        assert_eq!(
            fs::read(addon_dir.join("Foo/Foo.toc")).expect("foo"),
            FOO_TOC
        );
        assert!(addon_dir.join("Foo_Options").join("Media").exists());
        assert!(
            database
                .get_addon_by_folder("Foo")
                .expect("get")
                .expect("row")
                .toc_overrides
                .is_empty()
        );
        assert!(!staging_left(&addon_dir));
    }
}
