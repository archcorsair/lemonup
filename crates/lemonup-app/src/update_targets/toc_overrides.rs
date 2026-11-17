//! Writes the user's TOC overrides (`Title`, `Notes`, `Category`) into an addon's folders, always
//! through [`FolderSwap`]: the edit happens on the staged copy, so a failure never leaves a live
//! folder half written.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use lemonup_core::StateDatabase;
use lemonup_core::toc_edit;

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
///
/// A stored value this version can't use (an older or newer LemonUp may have stored it) is
/// skipped with a logged warning instead of blocking the install or update.
pub(crate) fn swap_with_toc_overrides(
    addon_dir: &Path,
    source_root: &Path,
    new_folders: &[String],
    replaced: &[String],
    overrides: &BTreeMap<String, String>,
) -> Result<FolderSwap, String> {
    let (overrides, skipped) = toc_edit::usable_overrides(overrides);
    for warning in &skipped {
        tracing::warn!(folders = ?new_folders, "{warning}");
    }
    let overrides = &overrides;
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
pub(crate) fn apply_toc_overrides_now(
    database: &mut StateDatabase,
    addon_dir: &Path,
    folder: &str,
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<String>, TocApplyError> {
    apply_toc_overrides_now_storing_with(
        database,
        addon_dir,
        folder,
        overrides,
        |database, folder, overrides| {
            database
                .set_toc_overrides(folder, overrides)
                .map_err(|error| error.to_string())
        },
    )
}

/// Why [`apply_toc_overrides_now`] left an addon's files alone, so callers can word it without
/// reading the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TocApplyError {
    /// `folder` has a symbolic link at or below it.
    Symlink { folder: String, link: String },
    /// `folder` belongs to the linked addon in `owner`.
    HeldByOther { folder: String, owner: String },
    /// The addon's parent folder is not in the AddOns folder.
    Missing { folder: String, addon_dir: String },
    /// Anything else: a refused value, a read, swap or database failure.
    Other(String),
}

impl std::fmt::Display for TocApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Symlink { folder, link } => write!(
                formatter,
                "{folder} contains a symbolic link ({link}); LemonUp can't rewrite its TOC files safely"
            ),
            Self::HeldByOther { folder, owner } => write!(
                formatter,
                "{folder} belongs to {owner}, so its TOC files were left alone"
            ),
            Self::Missing { folder, addon_dir } => {
                write!(
                    formatter,
                    "{folder} is not in the AddOns folder {addon_dir}"
                )
            }
            Self::Other(message) => formatter.write_str(message),
        }
    }
}

impl From<String> for TocApplyError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

/// [`apply_toc_overrides_now`] with the database write passed in, so tests can make it fail.
fn apply_toc_overrides_now_storing_with(
    database: &mut StateDatabase,
    addon_dir: &Path,
    folder: &str,
    overrides: &BTreeMap<String, String>,
    store: impl FnOnce(&mut StateDatabase, &str, &BTreeMap<String, String>) -> Result<(), String>,
) -> Result<Vec<String>, TocApplyError> {
    // The same check the database write makes, so nothing is swapped for a value it refuses.
    toc_edit::validate_stored_overrides(overrides).map_err(|error| error.to_string())?;
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
        return Err(TocApplyError::Missing {
            folder: addon.folder.clone(),
            addon_dir: addon_dir.display().to_string(),
        });
    }
    // Owned folders that are gone are skipped, like an update skips them.
    folders.retain(|name| fs::symlink_metadata(addon_dir.join(name)).is_ok());
    for name in &folders {
        if let Some(owner) = tracked_owner_of(&rows, &addon.folder, name) {
            return Err(TocApplyError::HeldByOther {
                folder: name.clone(),
                owner: owner.folder.clone(),
            });
        }
        if let Some(link) = find_symlink(&addon_dir.join(name))? {
            return Err(TocApplyError::Symlink {
                folder: name.clone(),
                link: link.display().to_string(),
            });
        }
    }

    let swap = FolderSwap::apply_prepared(
        addon_dir,
        addon_dir,
        &folders,
        &folders,
        &mut |_, staged| write_overrides(staged, overrides),
    )?;
    if let Err(error) = store(database, &addon.folder, overrides) {
        return Err(swap.abort(error).into());
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

    use super::{
        apply_toc_overrides_now, apply_toc_overrides_now_storing_with, swap_with_toc_overrides,
    };
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
        let error = apply_toc_overrides_now(
            &mut database,
            &addon_dir,
            "Foo",
            &overrides(&[("Title", "  My Foo ")]),
        )
        .expect_err("untrimmed value refused");
        assert!(error.to_string().contains("Title override"), "{error}");
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
        assert!(error.to_string().contains("symbolic link"), "{error}");
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

    #[test]
    fn apply_now_refuses_a_folder_another_linked_addon_holds() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = seed(temp.path());
        let mut other = AddonRecord::new("Bar", "Bar", SourceKind::Wago);
        other.set_managed_owned_folders(vec![OwnedFolder {
            name: "foo_options".to_string(),
        }]);
        database.upsert_addon(&other).expect("other linked addon");

        let error = apply_toc_overrides_now(
            &mut database,
            &addon_dir,
            "Foo",
            &overrides(&[("Title", "My Foo")]),
        )
        .expect_err("refused");
        assert!(error.to_string().contains("belongs to Bar"), "{error}");
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
        assert!(!staging_left(&addon_dir));
    }

    #[test]
    fn apply_now_rolls_back_the_files_when_the_database_write_fails() {
        let temp = tempdir().expect("tempdir");
        let (addon_dir, mut database) = seed(temp.path());
        let before = overrides(&[("Notes", "Before")]);
        database.set_toc_overrides("Foo", &before).expect("seed");

        let error = apply_toc_overrides_now_storing_with(
            &mut database,
            &addon_dir,
            "Foo",
            &overrides(&[("Title", "My Foo")]),
            |_, _, _| Err("refused for the test".to_string()),
        )
        .expect_err("database write fails");
        assert!(
            error.to_string().contains("refused for the test"),
            "{error}"
        );
        assert_eq!(
            fs::read(addon_dir.join("Foo/Foo.toc")).expect("foo"),
            FOO_TOC
        );
        assert_eq!(
            fs::read(addon_dir.join("Foo_Options/Foo_Options.toc")).expect("options"),
            OPTIONS_TOC
        );
        assert!(addon_dir.join("Foo/Libs/Lib.toc").exists());
        assert_eq!(
            database
                .get_addon_by_folder("Foo")
                .expect("get")
                .expect("row")
                .toc_overrides,
            before
        );
        assert!(!staging_left(&addon_dir));
    }

    #[test]
    fn an_install_skips_a_stored_value_this_version_cannot_use() {
        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("package");
        let addon_dir = temp.path().join("AddOns");
        fs::create_dir_all(source_root.join("Foo")).expect("package");
        fs::create_dir_all(&addon_dir).expect("addons");
        fs::write(
            source_root.join("Foo").join("Foo.toc"),
            b"## Title: Foo\n## Category: Combat\n",
        )
        .expect("toc");

        let swap = swap_with_toc_overrides(
            &addon_dir,
            &source_root,
            &["Foo".to_string()],
            &[],
            &overrides(&[("Title", ""), ("Category", "Mine"), ("X-Future", "kept")]),
        )
        .expect("an unusable value does not block the swap");
        swap.commit();
        assert_eq!(
            fs::read_to_string(addon_dir.join("Foo/Foo.toc")).expect("toc"),
            "## Title: Foo\n# LemonUp: ## Category: Combat\n# LemonUp-managed\n## Category: Mine\n"
        );
    }
}
