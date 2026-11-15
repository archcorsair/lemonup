//! Binds a profile's tracked state to the AddOns folder it was built from.
//!
//! The state database (and the backup store next to it) belongs to one profile, and a
//! profile manages one AddOns folder. Pointing it at another folder (for example
//! `--addon-dir` at the WoW Forever client while the profile tracks retail) would
//! reconcile retail's rows against Forever and drop their tracking. The bound folder is
//! stored in the database itself, so the binding travels with the state it protects.

use std::fmt;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;
use time::macros::format_description;

use crate::DEFAULT_PROFILE;
use crate::error::Result;
use crate::paths::same_addons_folder;
use crate::storage::StateDatabase;
use crate::transfer::export_addons;

/// Outcome of comparing a requested AddOns folder with the profile's bound folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingCheck {
    /// The requested folder is the bound one.
    Bound,
    /// The requested folder is a different one; the caller must not touch state for it.
    Mismatch(AddonDirMismatch),
}

/// A command asked for a folder other than the one the profile's state belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonDirMismatch {
    pub profile: String,
    pub bound: PathBuf,
    pub requested: PathBuf,
}

impl fmt::Display for AddonDirMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile_flag = if self.profile == DEFAULT_PROFILE {
            String::new()
        } else {
            format!("--profile {} ", self.profile)
        };
        let suggested_profile = suggested_profile_name(&self.requested);
        write!(
            formatter,
            "profile '{}' is bound to the AddOns folder {} but this run targets {} | refusing to touch tracked addons or backups for a different folder | to manage that install, give it its own profile: `lemonup --profile {} --addon-dir '{}' sync` | to move this profile to that folder on purpose, open `lemonup {}tui`, press r in Config to rerun setup and pick it",
            self.profile,
            self.bound.display(),
            self.requested.display(),
            suggested_profile,
            self.requested.display(),
            profile_flag,
        )
    }
}

impl std::error::Error for AddonDirMismatch {}

/// Profile name to suggest for another install: its client folder without underscores
/// (`_forever_` becomes `forever`).
fn suggested_profile_name(addon_dir: &Path) -> String {
    let client = addon_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|name| {
            name.to_string_lossy()
                .trim_matches('_')
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let cleaned = client
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '-' || value == '_' {
                value
            } else {
                '-'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() || cleaned == DEFAULT_PROFILE {
        "other".to_string()
    } else {
        cleaned
    }
}

/// Compares `requested` with the folder `database` is bound to.
///
/// A database without a binding (created before binding existed, or brand new) belongs to
/// the profile's `configured` folder, or to `requested` when the profile has none
/// configured. With `adopt` set that folder is written as the binding; read-only callers
/// pass `false` and nothing is written.
pub fn check_addon_dir_binding(
    database: &StateDatabase,
    profile: &str,
    configured: Option<&Path>,
    requested: &Path,
    adopt: bool,
) -> Result<BindingCheck> {
    let bound = match database.bound_addon_dir()? {
        Some(bound) => bound,
        None => {
            let adopted = absolute(configured.unwrap_or(requested));
            if adopt {
                database.bind_addon_dir(&adopted)?;
            }
            adopted
        }
    };

    if same_addons_folder(&bound, requested) {
        Ok(BindingCheck::Bound)
    } else {
        Ok(BindingCheck::Mismatch(AddonDirMismatch {
            profile: profile.to_string(),
            bound,
            requested: requested.to_path_buf(),
        }))
    }
}

/// Result of a deliberate rebind to a new AddOns folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindOutcome {
    /// The folder the profile was bound to before, when it was a different one.
    pub previous: Option<PathBuf>,
    /// Export of the tracked addons as they were before the rebind, written when the
    /// folder changed and something was tracked. `lemonup import-addons <file>` reads it.
    pub snapshot: Option<PathBuf>,
}

/// Binds the profile to `addon_dir` on purpose (onboarding / Location Finder).
///
/// Tracked rows are kept: moving an install keeps every addon's tracking, and the next
/// scan of the new folder drops only the rows whose folders are not there. Because that
/// drop is easy to miss, the old rows are first exported to `snapshot_dir`.
pub fn rebind_addon_dir(
    database: &StateDatabase,
    addon_dir: &Path,
    snapshot_dir: &Path,
    now: OffsetDateTime,
) -> Result<RebindOutcome> {
    let new_dir = absolute(addon_dir);
    let previous = database
        .bound_addon_dir()?
        .filter(|bound| !same_addons_folder(bound, &new_dir));

    let mut snapshot = None;
    if previous.is_some() {
        let addons = database.list_addons()?;
        if !addons.is_empty() {
            let stamp = now
                .format(format_description!(
                    "[year][month][day]T[hour][minute][second]Z"
                ))
                .unwrap_or_else(|_| now.unix_timestamp().to_string());
            let path = snapshot_dir.join(format!("tracked-before-rebind-{stamp}.json"));
            export_addons(&addons, &path)?;
            snapshot = Some(path);
        }
    }

    database.bind_addon_dir(&new_dir)?;
    Ok(RebindOutcome { previous, snapshot })
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use time::OffsetDateTime;

    use super::{BindingCheck, check_addon_dir_binding, rebind_addon_dir};
    use crate::domain::{AddonRecord, SourceKind};
    use crate::storage::StateDatabase;
    use crate::transfer::parse_import_file;

    fn addons_dir(root: &Path, client: &str) -> PathBuf {
        let dir = root.join(client).join("Interface").join("AddOns");
        fs::create_dir_all(&dir).expect("create addons dir");
        dir
    }

    fn tracked(folder: &str) -> AddonRecord {
        let mut addon = AddonRecord::new(folder, folder, SourceKind::GitHub);
        addon.source_url = Some("https://github.com/example/addon".to_string());
        addon.git_commit = Some("abc123".to_string());
        addon
    }

    #[test]
    fn legacy_database_adopts_the_configured_folder() {
        let temp = tempdir().expect("tempdir");
        let retail = addons_dir(temp.path(), "_retail_");
        let forever = addons_dir(temp.path(), "_forever_");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open");
        assert_eq!(database.bound_addon_dir().expect("read"), None);

        let check = check_addon_dir_binding(&database, "default", Some(&retail), &forever, true)
            .expect("check");
        let BindingCheck::Mismatch(mismatch) = check else {
            panic!("forever must not match a retail-configured profile");
        };
        assert_eq!(mismatch.bound, retail);
        assert!(mismatch.to_string().contains("--profile forever"));
        assert_eq!(
            database.bound_addon_dir().expect("read"),
            Some(retail.clone())
        );

        assert_eq!(
            check_addon_dir_binding(&database, "default", Some(&retail), &retail, true)
                .expect("check"),
            BindingCheck::Bound
        );
    }

    #[test]
    fn unconfigured_profile_binds_to_the_first_requested_folder() {
        let temp = tempdir().expect("tempdir");
        let retail = addons_dir(temp.path(), "_retail_");
        let forever = addons_dir(temp.path(), "_forever_");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open");

        assert_eq!(
            check_addon_dir_binding(&database, "default", None, &retail, true).expect("check"),
            BindingCheck::Bound
        );
        assert!(matches!(
            check_addon_dir_binding(&database, "default", None, &forever, true).expect("check"),
            BindingCheck::Mismatch(_)
        ));
    }

    #[test]
    fn read_only_check_does_not_write_a_binding() {
        let temp = tempdir().expect("tempdir");
        let retail = addons_dir(temp.path(), "_retail_");
        let forever = addons_dir(temp.path(), "_forever_");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open");

        assert!(matches!(
            check_addon_dir_binding(&database, "default", Some(&retail), &forever, false)
                .expect("check"),
            BindingCheck::Mismatch(_)
        ));
        assert_eq!(database.bound_addon_dir().expect("read"), None);
    }

    #[test]
    fn binding_survives_reopen() {
        let temp = tempdir().expect("tempdir");
        let retail = addons_dir(temp.path(), "_retail_");
        let db_path = temp.path().join("state.sqlite");
        StateDatabase::open(&db_path)
            .expect("open")
            .bind_addon_dir(&retail)
            .expect("bind");
        let reopened = StateDatabase::open(&db_path).expect("reopen");
        assert_eq!(reopened.bound_addon_dir().expect("read"), Some(retail));
    }

    #[test]
    fn rebind_keeps_rows_and_snapshots_the_old_folder() {
        let temp = tempdir().expect("tempdir");
        let retail = addons_dir(temp.path(), "_retail_");
        let forever = addons_dir(temp.path(), "_forever_");
        let snapshots = temp.path().join("rebind");
        let database = StateDatabase::open(temp.path().join("state.sqlite")).expect("open");
        database.bind_addon_dir(&retail).expect("bind");
        database.upsert_addon(&tracked("GitAddon")).expect("upsert");

        let outcome = rebind_addon_dir(&database, &forever, &snapshots, OffsetDateTime::UNIX_EPOCH)
            .expect("rebind");
        assert_eq!(outcome.previous, Some(retail));
        let snapshot = outcome.snapshot.expect("snapshot written");
        let exported = parse_import_file(&snapshot).expect("parse snapshot");
        assert_eq!(exported.addons.len(), 1);
        assert_eq!(database.list_addons().expect("list").len(), 1);
        assert_eq!(
            database.bound_addon_dir().expect("read"),
            Some(forever.clone())
        );

        let same = rebind_addon_dir(&database, &forever, &snapshots, OffsetDateTime::UNIX_EPOCH)
            .expect("rebind same");
        assert_eq!(same.previous, None);
        assert_eq!(same.snapshot, None);
    }
}
