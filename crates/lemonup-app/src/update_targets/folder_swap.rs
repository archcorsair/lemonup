//! Rename-based replacement of addon folders with rollback, shared by every provider.
//!
//! A swap runs in three steps, all inside one hidden staging directory created in the AddOns
//! folder itself (so every rename stays on the same volume and is atomic per folder):
//!
//! 1. the new package folders are copied into `<staging>/new`, and an optional prepare step
//!    edits those staged copies (for example to write the user's TOC overrides);
//! 2. each existing target folder is renamed aside into `<staging>/old`;
//! 3. each new folder is renamed from `<staging>/new` into the AddOns folder.
//!
//! Any error in any step puts every placed folder back into staging and renames every
//! moved-aside folder back into place before the error is returned. Once the caller's database
//! write succeeds, [`FolderSwap::commit`] deletes the staging directory (and with it the old
//! folders); if that write fails, [`FolderSwap::rollback`] restores the previous state. Dropping
//! an unfinished swap rolls it back.
//!
//! Nothing here follows symlinks: an existing symlinked addon folder is renamed as the link
//! itself, and deleting staging removes links without touching their targets.
//!
//! The staging directory is named `.lemonup-staging-*` and holds only the `new` and `old`
//! subdirectories, never a `.toc` at its root, so neither the LemonUp scan (which skips folders
//! without root TOC files) nor the game treats it as an addon.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Edits one staged copy before anything live moves: called with the folder name and the path of
/// its copy inside staging. An error aborts the swap with nothing live changed.
pub(crate) type PrepareStaged<'a> = dyn FnMut(&str, &Path) -> Result<(), String> + 'a;

/// Name prefix of the per-operation staging directory inside the AddOns folder. Shared with the
/// scan, which reports leftovers by this name.
pub(crate) use lemonup_core::UPDATE_STAGING_PREFIX as STAGING_PREFIX;

#[derive(Debug)]
pub(crate) struct FolderSwap {
    addon_dir: PathBuf,
    staging: PathBuf,
    /// Existing folders renamed into `<staging>/old`, in move order.
    moved_aside: Vec<String>,
    /// New folders renamed from `<staging>/new` into the AddOns folder, in placement order.
    placed: Vec<String>,
    finished: bool,
}

impl FolderSwap {
    /// Replaces `replaced` (those that exist) with `new_folders` copied from `source_root`.
    ///
    /// On error everything is already rolled back; the returned message also says when the
    /// rollback itself was incomplete and where the old folders were kept.
    pub(crate) fn apply(
        addon_dir: &Path,
        source_root: &Path,
        new_folders: &[String],
        replaced: &[String],
    ) -> Result<Self, String> {
        Self::apply_prepared(
            addon_dir,
            source_root,
            new_folders,
            replaced,
            &mut |_, _| Ok(()),
        )
    }

    /// [`FolderSwap::apply`] with a step that edits each staged copy after all of them are copied
    /// and before any live folder moves.
    pub(crate) fn apply_prepared(
        addon_dir: &Path,
        source_root: &Path,
        new_folders: &[String],
        replaced: &[String],
        prepare: &mut PrepareStaged<'_>,
    ) -> Result<Self, String> {
        for folder in new_folders.iter().chain(replaced) {
            validate_folder_name(folder)?;
        }
        fs::create_dir_all(addon_dir).map_err(|error| {
            format!(
                "could not create the AddOns folder {}: {error}",
                addon_dir.display()
            )
        })?;
        let staging = create_staging_dir(addon_dir)?;
        let mut swap = FolderSwap {
            addon_dir: addon_dir.to_path_buf(),
            staging,
            moved_aside: Vec::new(),
            placed: Vec::new(),
            finished: false,
        };
        match swap.run(source_root, new_folders, replaced, prepare) {
            Ok(()) => Ok(swap),
            Err(error) => Err(match swap.restore() {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; {rollback_error}"),
            }),
        }
    }

    /// Keeps the new folders and deletes the moved-aside old ones.
    ///
    /// A staging directory that cannot be fully deleted (for example a file locked by another
    /// process) is left behind; it is inert and holds only replaced folders. The update itself
    /// succeeded, so this only logs a warning; the next scan reports the leftover folder.
    pub(crate) fn commit(mut self) {
        self.finished = true;
        if let Err(error) = fs::remove_dir_all(&self.staging) {
            tracing::warn!(
                staging = %self.staging.display(),
                %error,
                "could not delete the update staging folder; it may contain previous addon versions"
            );
        }
    }

    /// Restores the folders as they were before [`FolderSwap::apply`].
    pub(crate) fn rollback(mut self) -> Result<(), String> {
        self.restore()
    }

    /// Rolls back after the caller's own step failed with `error`, returning the message to
    /// report (with any rollback problem appended).
    pub(crate) fn abort(self, error: String) -> String {
        match self.rollback() {
            Ok(()) => error,
            Err(rollback_error) => format!("{error}; {rollback_error}"),
        }
    }

    fn new_root(&self) -> PathBuf {
        self.staging.join("new")
    }

    fn old_root(&self) -> PathBuf {
        self.staging.join("old")
    }

    fn run(
        &mut self,
        source_root: &Path,
        new_folders: &[String],
        replaced: &[String],
        prepare: &mut PrepareStaged<'_>,
    ) -> Result<(), String> {
        let new_root = self.new_root();
        let old_root = self.old_root();
        for root in [&new_root, &old_root] {
            fs::create_dir(root).map_err(|error| {
                format!("could not prepare staging {}: {error}", root.display())
            })?;
        }

        for folder in new_folders {
            copy_tree(&source_root.join(folder), &new_root.join(folder))
                .map_err(|error| format!("could not stage new folder '{folder}': {error}"))?;
        }

        for folder in new_folders {
            prepare(folder, &new_root.join(folder))
                .map_err(|error| format!("could not prepare new folder '{folder}': {error}"))?;
        }

        for folder in replaced {
            if self.moved_aside.contains(folder) {
                continue;
            }
            let live = self.addon_dir.join(folder);
            // `symlink_metadata` so a symlink (even a broken one) counts as present and is
            // moved as the link itself.
            if fs::symlink_metadata(&live).is_err() {
                continue;
            }
            fs::rename(&live, old_root.join(folder)).map_err(|error| {
                format!("could not move existing folder '{folder}' aside: {error}")
            })?;
            self.moved_aside.push(folder.clone());
        }

        for folder in new_folders {
            if self.placed.contains(folder) {
                continue;
            }
            let live = self.addon_dir.join(folder);
            if fs::symlink_metadata(&live).is_ok() {
                return Err(format!(
                    "could not place new folder '{folder}': something else already exists there"
                ));
            }
            fs::rename(new_root.join(folder), &live)
                .map_err(|error| format!("could not place new folder '{folder}': {error}"))?;
            self.placed.push(folder.clone());
        }

        Ok(())
    }

    fn restore(&mut self) -> Result<(), String> {
        self.finished = true;
        let new_root = self.new_root();
        let old_root = self.old_root();
        let mut problems = Vec::new();

        for folder in std::mem::take(&mut self.placed).into_iter().rev() {
            let live = self.addon_dir.join(&folder);
            // A placed folder is our own fresh copy (never a symlink), so removing it is safe
            // when it cannot be moved back.
            if fs::rename(&live, new_root.join(&folder)).is_err()
                && let Err(error) = fs::remove_dir_all(&live)
            {
                problems.push(format!("could not remove new folder '{folder}': {error}"));
            }
        }

        for folder in std::mem::take(&mut self.moved_aside).into_iter().rev() {
            let live = self.addon_dir.join(&folder);
            if fs::symlink_metadata(&live).is_ok() {
                problems.push(format!(
                    "could not restore '{folder}': the folder is occupied"
                ));
                continue;
            }
            if let Err(error) = fs::rename(old_root.join(&folder), &live) {
                problems.push(format!("could not restore '{folder}': {error}"));
            }
        }

        if problems.is_empty() {
            let _ = fs::remove_dir_all(&self.staging);
            Ok(())
        } else {
            Err(format!(
                "rollback incomplete ({}); the previous folders are kept in {}",
                problems.join("; "),
                self.old_root().display()
            ))
        }
    }
}

impl Drop for FolderSwap {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.restore();
        }
    }
}

/// Folder names come from archives and the database; only a single plain path component may be
/// joined onto the AddOns folder.
fn validate_folder_name(folder: &str) -> Result<(), String> {
    let mut components = Path::new(folder).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(format!(
            "refusing to touch invalid addon folder name '{folder}'"
        )),
    }
}

fn create_staging_dir(addon_dir: &Path) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let pid = std::process::id();
    let mut last_error = None;
    for attempt in 0..100u32 {
        let candidate = addon_dir.join(format!("{STAGING_PREFIX}{pid}-{nanos}-{attempt}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => {
                return Err(format!(
                    "could not create a staging folder in {}: {error}",
                    addon_dir.display()
                ));
            }
        }
    }
    Err(format!(
        "could not create a unique staging folder in {}: {}",
        addon_dir.display(),
        last_error
            .map(|error| error.to_string())
            .unwrap_or_default()
    ))
}

/// Copies a directory tree without following symlinks; symlinks in the source are skipped.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FolderSwap, STAGING_PREFIX};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_folder(root: &Path, folder: &str, marker: &str) {
        fs::create_dir_all(root.join(folder)).expect("create folder");
        fs::write(root.join(folder).join("marker.txt"), marker).expect("write marker");
    }

    fn marker(root: &Path, folder: &str) -> Option<String> {
        fs::read_to_string(root.join(folder).join("marker.txt")).ok()
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn staging_dirs(addon_dir: &Path) -> Vec<String> {
        fs::read_dir(addon_dir)
            .expect("read addon dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(STAGING_PREFIX))
            .collect()
    }

    #[test]
    fn successful_swap_replaces_folders_and_cleans_up_staging() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&addon_dir, "Foo_Options", "old");
        write_folder(&addon_dir, "Other", "keep");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Extra", "new");

        let swap = FolderSwap::apply(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Extra"]),
            &names(&["Foo", "Foo_Options"]),
        )
        .expect("swap");
        assert!(
            !staging_dirs(&addon_dir).is_empty(),
            "backups are kept until commit"
        );
        swap.commit();

        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("new"));
        assert_eq!(marker(&addon_dir, "Foo_Extra").as_deref(), Some("new"));
        assert!(!addon_dir.join("Foo_Options").exists());
        assert_eq!(marker(&addon_dir, "Other").as_deref(), Some("keep"));
        assert!(staging_dirs(&addon_dir).is_empty());
        assert_eq!(
            marker(&source, "Foo").as_deref(),
            Some("new"),
            "source kept"
        );
    }

    #[test]
    fn prepare_step_edits_the_staged_copies_only() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Extra", "new");

        let mut seen = Vec::new();
        let swap = FolderSwap::apply_prepared(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Extra"]),
            &names(&["Foo"]),
            &mut |folder, staged| {
                assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
                assert!(staged.starts_with(&addon_dir));
                seen.push(folder.to_string());
                fs::write(staged.join("marker.txt"), "prepared").map_err(|e| e.to_string())
            },
        )
        .expect("swap");
        swap.commit();

        assert_eq!(seen, names(&["Foo", "Foo_Extra"]));
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("prepared"));
        assert_eq!(marker(&addon_dir, "Foo_Extra").as_deref(), Some("prepared"));
        assert_eq!(
            marker(&source, "Foo").as_deref(),
            Some("new"),
            "source kept"
        );
    }

    #[test]
    fn failing_prepare_step_leaves_live_folders_untouched() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Extra", "new");

        let error = FolderSwap::apply_prepared(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Extra"]),
            &names(&["Foo"]),
            &mut |folder, _| {
                if folder == "Foo_Extra" {
                    Err("bad toc".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("prepare fails");

        assert!(
            error.contains("Foo_Extra") && error.contains("bad toc"),
            "{error}"
        );
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert!(!addon_dir.join("Foo_Extra").exists());
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[test]
    fn rollback_after_success_restores_previous_folders() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&addon_dir, "Foo_Options", "old");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Extra", "new");

        let swap = FolderSwap::apply(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Extra"]),
            &names(&["Foo", "Foo_Options"]),
        )
        .expect("swap");
        swap.rollback().expect("rollback");

        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert_eq!(marker(&addon_dir, "Foo_Options").as_deref(), Some("old"));
        assert!(!addon_dir.join("Foo_Extra").exists());
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[test]
    fn dropping_an_unfinished_swap_rolls_back() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&source, "Foo", "new");

        let swap = FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&["Foo"]))
            .expect("swap");
        drop(swap);

        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[test]
    fn failure_while_placing_new_folders_rolls_everything_back() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&addon_dir, "Foo_Options", "old");
        // An unplanned occupant of a package folder name (here a plain file) makes placing
        // the second new folder fail after the first one is already in place.
        fs::write(addon_dir.join("Foo_Zed"), "file").expect("occupant");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Zed", "new");

        let error = FolderSwap::apply(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Zed"]),
            &names(&["Foo", "Foo_Options"]),
        )
        .expect_err("placing must fail");

        assert!(error.contains("Foo_Zed"), "{error}");
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert_eq!(marker(&addon_dir, "Foo_Options").as_deref(), Some("old"));
        assert_eq!(
            fs::read_to_string(addon_dir.join("Foo_Zed")).expect("occupant kept"),
            "file"
        );
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[test]
    fn failure_while_staging_touches_nothing() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");

        let error = FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&["Foo"]))
            .expect_err("missing source must fail");

        assert!(error.contains("could not stage"), "{error}");
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[test]
    fn invalid_folder_names_are_refused() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&source, "Foo", "new");
        for bad in ["../Foo", "a/b", "..", ""] {
            assert!(
                FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&[bad])).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(!addon_dir.join("Foo").exists());
    }

    #[cfg(unix)]
    #[test]
    fn locked_folder_that_cannot_be_moved_aside_rolls_back_earlier_moves() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&addon_dir, "Foo_Options", "old");
        write_folder(&source, "Foo", "new");
        write_folder(&source, "Foo_Options", "new");
        // Moving a directory to another parent needs write access to it (for its `..`
        // entry), so a read-only folder stands in for one locked by another process.
        let locked = addon_dir.join("Foo_Options");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("chmod");
        if fs::write(locked.join("probe"), "x").is_ok() {
            // Running with privileges that ignore permissions; nothing to lock.
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
            return;
        }

        let result = FolderSwap::apply(
            &addon_dir,
            &source,
            &names(&["Foo", "Foo_Options"]),
            &names(&["Foo", "Foo_Options"]),
        );
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");

        let error = result.expect_err("moving the locked folder must fail");
        assert!(error.contains("Foo_Options"), "{error}");
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("old"));
        assert_eq!(marker(&addon_dir, "Foo_Options").as_deref(), Some("old"));
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_target_is_moved_as_a_link_and_its_target_survives() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        let checkout = temp.path().join("dev-checkout");
        write_folder(&checkout, "Foo", "dev");
        fs::create_dir_all(&addon_dir).expect("addon dir");
        std::os::unix::fs::symlink(checkout.join("Foo"), addon_dir.join("Foo")).expect("link");
        write_folder(&source, "Foo", "new");

        // Rollback path: the link comes back as a link.
        let swap = FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&["Foo"]))
            .expect("swap");
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("new"));
        swap.rollback().expect("rollback");
        assert!(
            fs::symlink_metadata(addon_dir.join("Foo"))
                .expect("restored")
                .file_type()
                .is_symlink()
        );
        assert_eq!(marker(&checkout, "Foo").as_deref(), Some("dev"));

        // Commit path: the link is deleted, never what it points at.
        let swap = FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&["Foo"]))
            .expect("swap");
        swap.commit();
        assert!(
            !fs::symlink_metadata(addon_dir.join("Foo"))
                .expect("new folder")
                .file_type()
                .is_symlink()
        );
        assert_eq!(marker(&addon_dir, "Foo").as_deref(), Some("new"));
        assert_eq!(marker(&checkout, "Foo").as_deref(), Some("dev"));
        assert!(staging_dirs(&addon_dir).is_empty());
    }

    /// Collects formatted tracing output so a test can assert on warnings.
    #[derive(Clone, Default)]
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCapture;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[cfg(unix)]
    #[test]
    fn commit_that_cannot_delete_staging_warns_with_the_staging_path_and_still_succeeds() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        let source = temp.path().join("source");
        write_folder(&addon_dir, "Foo", "old");
        write_folder(&source, "Foo", "new");

        let swap = FolderSwap::apply(&addon_dir, &source, &names(&["Foo"]), &names(&["Foo"]))
            .expect("swap");
        let staging = addon_dir.join(
            staging_dirs(&addon_dir)
                .first()
                .expect("staging exists until commit"),
        );
        // A read-only folder with content inside the moved-aside tree cannot be emptied.
        let locked = staging.join("old").join("Foo").join("locked");
        fs::create_dir_all(&locked).expect("locked dir");
        fs::write(locked.join("file.txt"), "x").expect("locked file");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).expect("lock");
        if fs::remove_dir(&locked).is_ok() || fs::File::create(locked.join("probe")).is_ok() {
            // Running as a user that ignores permissions (for example root): nothing to assert.
            return;
        }

        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || swap.commit());

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).expect("unlock");
        let logged = String::from_utf8(capture.0.lock().expect("log lock").clone()).expect("utf8");
        assert!(logged.contains("WARN"), "{logged}");
        assert!(logged.contains(&staging.display().to_string()), "{logged}");
        assert_eq!(
            marker(&addon_dir, "Foo").as_deref(),
            Some("new"),
            "the update itself still succeeded"
        );
        assert_eq!(
            staging_dirs(&addon_dir).len(),
            1,
            "leftover is left in place"
        );
    }
}
