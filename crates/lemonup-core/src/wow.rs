use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use crate::domain::GameFlavor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgressUpdate {
    pub dirs_scanned: usize,
    pub current_path: PathBuf,
}

pub fn suggested_scan_roots() -> Vec<PathBuf> {
    let mut suggestions = Vec::new();

    if let Some(home) = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    {
        suggestions.push(home);
    }

    if cfg!(windows) {
        let mut found_drive = false;
        for drive in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", drive as char));
            if root.exists() {
                found_drive = true;
                suggestions.push(root);
            }
        }

        if !found_drive {
            suggestions.extend(
                ["C:\\", "D:\\", "E:\\", "F:\\"]
                    .into_iter()
                    .map(PathBuf::from),
            );
        }
    } else if cfg!(target_os = "macos") {
        suggestions.extend(["/Applications", "/"].into_iter().map(PathBuf::from));
    } else {
        suggestions.extend(["/home", "/mnt", "/"].into_iter().map(PathBuf::from));
    }

    dedupe_paths(suggestions)
}

/// Client folders LemonUp manages: retail and WoW Forever.
const CLIENT_FOLDERS: [&str; 2] = ["_retail_", "_forever_"];

pub fn detect_known_addons_path() -> Option<PathBuf> {
    detect_known_addons_paths().into_iter().next()
}

/// Every valid AddOns folder found at the well-known install locations, retail before
/// WoW Forever for each install.
pub fn detect_known_addons_paths() -> Vec<PathBuf> {
    let candidates = if cfg!(windows) {
        let mut candidates = Vec::new();
        for root in suggested_scan_roots() {
            let root_str = root.display().to_string();
            if root_str.len() == 3 && root_str.ends_with(":\\") {
                let drive = &root_str[0..1];
                for template in windows_relative_candidates() {
                    candidates.push(PathBuf::from(format!(r"{drive}:\{template}")));
                }
            }
        }
        candidates
    } else if cfg!(target_os = "macos") {
        addons_candidates(&[PathBuf::from("/Applications/World of Warcraft")])
    } else {
        let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        addons_candidates(&[
            home.join("Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft"),
            home.join(".wine/drive_c/Program Files (x86)/World of Warcraft"),
            home.join(
                ".var/app/com.usebottles.bottles/data/bottles/bottles/World-of-Warcraft/drive_c/Program Files (x86)/World of Warcraft",
            ),
            home.join(".local/share/Steam/steamapps/common/World of Warcraft"),
            home.join(
                ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/World of Warcraft",
            ),
        ])
    };

    dedupe_paths(
        candidates
            .into_iter()
            .filter(|candidate| validate_addons_path(candidate).is_ok())
            .collect(),
    )
}

pub fn quick_check_common_paths(root: &Path) -> Option<PathBuf> {
    for candidate in common_relative_candidates() {
        let full_path = root.join(candidate);
        if validate_addons_path(&full_path).is_ok() {
            return Some(full_path);
        }
    }

    None
}

pub fn validate_addons_path(path: &Path) -> std::result::Result<(), String> {
    if !path.exists() {
        return Err(format!("path does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("path is not a directory: {}", path.display()));
    }

    let parts = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    // Same rule as flavor detection: the client folder is the AddOns dir's grandparent.
    if GameFlavor::detect_from_path(path).is_none() {
        return Err("path must point to a retail or WoW Forever AddOns folder".to_string());
    }
    if !parts
        .iter()
        .any(|part| part.eq_ignore_ascii_case("Interface"))
        || !parts.iter().any(|part| part.eq_ignore_ascii_case("AddOns"))
    {
        return Err("path must contain Interface/AddOns".to_string());
    }

    if parts.len() < 3 {
        return Err("path is too short to be a WoW AddOns directory".to_string());
    }

    let wow_root =
        parts
            .iter()
            .take(parts.len().saturating_sub(3))
            .fold(PathBuf::new(), |mut acc, part| {
                acc.push(part);
                acc
            });
    let flavor_dir =
        parts
            .iter()
            .take(parts.len().saturating_sub(2))
            .fold(PathBuf::new(), |mut acc, part| {
                acc.push(part);
                acc
            });

    let root_artifacts = ["Data", ".build.info"];
    let flavor_artifacts = ["Wow.exe", "Wow-64.exe", "World of Warcraft.app"];

    let found_root = root_artifacts
        .iter()
        .filter(|artifact| wow_root.join(artifact).exists())
        .count();
    let found_flavor = flavor_artifacts
        .iter()
        .filter(|artifact| flavor_dir.join(artifact).exists())
        .count();

    if found_root + found_flavor < 2 {
        return Err("path is missing expected WoW install artifacts".to_string());
    }

    Ok(())
}

pub fn search_for_wow<F, C>(
    root: &Path,
    should_cancel: C,
    on_progress: F,
) -> std::result::Result<Option<PathBuf>, String>
where
    F: FnMut(ScanProgressUpdate),
    C: FnMut() -> bool,
{
    search_for_wow_installs(root, should_cancel, on_progress).map(|found| found.into_iter().next())
}

/// Deep scan that returns every AddOns folder of the first WoW install it reaches: the
/// retail folder and the WoW Forever folder when both exist. Empty when nothing was found
/// or the scan was cancelled.
pub fn search_for_wow_installs<F, C>(
    root: &Path,
    mut should_cancel: C,
    mut on_progress: F,
) -> std::result::Result<Vec<PathBuf>, String>
where
    F: FnMut(ScanProgressUpdate),
    C: FnMut() -> bool,
{
    if validate_addons_path(root).is_ok() {
        return Ok(vec![root.to_path_buf()]);
    }

    if !root.exists() {
        return Err(format!("cannot read directory: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("scan root is not a directory: {}", root.display()));
    }

    let ignored_dirs = ignored_dirs();
    let mut queue = VecDeque::from([root.to_path_buf()]);
    let max_depth = 10usize;
    let root_depth = root.components().count();
    let mut dirs_scanned = 0usize;

    while let Some(current_dir) = queue.pop_front() {
        if should_cancel() {
            return Ok(Vec::new());
        }

        dirs_scanned += 1;
        on_progress(ScanProgressUpdate {
            dirs_scanned,
            current_path: current_dir.clone(),
        });

        if dirs_scanned.is_multiple_of(50) {
            thread::yield_now();
            if should_cancel() {
                return Ok(Vec::new());
            }
        }

        let current_depth = current_dir.components().count().saturating_sub(root_depth);
        if current_depth > max_depth {
            continue;
        }

        let entries = match fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let mut child_dirs = Vec::new();
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            if !file_type.is_dir() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().to_string();
            if ignored_dirs.contains(file_name.as_str()) {
                continue;
            }

            let full_path = entry.path();
            if let Ok(metadata) = fs::symlink_metadata(&full_path)
                && metadata.file_type().is_symlink()
            {
                continue;
            }

            child_dirs.push((file_name, full_path));
        }

        let found = CLIENT_FOLDERS
            .iter()
            .filter_map(|client| {
                child_dirs
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(client))
            })
            .map(|(_, client_dir)| client_dir.join("Interface").join("AddOns"))
            .filter(|addons_path| validate_addons_path(addons_path).is_ok())
            .collect::<Vec<_>>();
        if !found.is_empty() {
            return Ok(found);
        }

        for (_, child_dir) in child_dirs {
            queue.push_back(child_dir);
        }
    }

    Ok(Vec::new())
}

fn common_relative_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        windows_relative_candidates()
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else if cfg!(target_os = "macos") {
        addons_candidates(&[PathBuf::from("Applications/World of Warcraft")])
    } else {
        addons_candidates(&[
            PathBuf::from("Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft"),
            PathBuf::from(".wine/drive_c/Program Files (x86)/World of Warcraft"),
            PathBuf::from(".local/share/Steam/steamapps/common/World of Warcraft"),
        ])
    }
}

/// Expands WoW install roots into `<root>/<client>/Interface/AddOns` for every client folder.
fn addons_candidates(wow_roots: &[PathBuf]) -> Vec<PathBuf> {
    wow_roots
        .iter()
        .flat_map(|wow_root| {
            CLIENT_FOLDERS
                .iter()
                .map(move |client| wow_root.join(client).join("Interface").join("AddOns"))
        })
        .collect()
}

fn windows_relative_candidates() -> Vec<String> {
    [
        r"Program Files (x86)\World of Warcraft",
        r"Program Files\World of Warcraft",
        r"Games\World of Warcraft",
        r"World of Warcraft",
    ]
    .into_iter()
    .flat_map(|wow_root| {
        CLIENT_FOLDERS
            .iter()
            .map(move |client| format!(r"{wow_root}\{client}\Interface\AddOns"))
    })
    .collect()
}

fn ignored_dirs() -> HashSet<&'static str> {
    HashSet::from([
        "node_modules",
        ".git",
        "Library",
        "System",
        "Applications",
        "private",
        "proc",
        "sys",
        "dev",
        "boot",
        "snap",
        "flatpak",
        "lost+found",
        "run",
        "tmp",
        "Windows",
        "$Recycle.Bin",
        "System Volume Information",
        "ProgramData",
        "AppData",
    ])
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for path in paths {
        let normalized = path.display().to_string();
        if seen.insert(normalized) {
            deduped.push(path);
        }
    }

    deduped
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use tempfile::tempdir;

    use super::{
        quick_check_common_paths, search_for_wow, search_for_wow_installs, suggested_scan_roots,
        validate_addons_path,
    };

    fn create_wow_install(root: &Path) -> PathBuf {
        create_wow_client(root, "_retail_")
    }

    fn create_wow_client(root: &Path, client: &str) -> PathBuf {
        let wow_root = root.join("World of Warcraft");
        let client_dir = wow_root.join(client);
        let addons = client_dir.join("Interface").join("AddOns");

        fs::create_dir_all(wow_root.join("Data")).expect("create Data");
        fs::write(wow_root.join(".build.info"), "build").expect("write .build.info");
        fs::create_dir_all(&addons).expect("create addons dir");
        fs::write(client_dir.join("Wow.exe"), "wow").expect("write Wow.exe");

        addons
    }

    #[test]
    fn validates_a_realistic_addons_path() {
        let temp = tempdir().expect("tempdir");
        let addons = create_wow_install(temp.path());

        assert!(validate_addons_path(&addons).is_ok());
    }

    #[test]
    fn validates_a_forever_addons_path() {
        let temp = tempdir().expect("tempdir");
        let addons = create_wow_client(temp.path(), "_forever_");

        assert!(validate_addons_path(&addons).is_ok());
    }

    #[test]
    fn rejects_paths_whose_grandparent_is_not_a_client_folder() {
        let temp = tempdir().expect("tempdir");
        let addons = create_wow_install(temp.path());

        // A client folder elsewhere in the path is not enough.
        let nested = addons.join("Sub");
        fs::create_dir_all(&nested).expect("create nested dir");
        let error = validate_addons_path(&nested).expect_err("grandparent is Interface");
        assert!(error.contains("retail or WoW Forever"));

        let stuffed = temp
            .path()
            .join("World of Warcraft")
            .join("_retail_")
            .join("stuff")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(&stuffed).expect("create stuffed dir");
        let error = validate_addons_path(&stuffed).expect_err("grandparent is stuff");
        assert!(error.contains("retail or WoW Forever"));
    }

    #[test]
    fn rejects_unknown_client_folder() {
        let temp = tempdir().expect("tempdir");
        let addons = create_wow_client(temp.path(), "_classic_");

        let error = validate_addons_path(&addons).expect_err("classic is not supported");
        assert!(error.contains("retail or WoW Forever"));
    }

    #[test]
    fn deep_scan_lists_retail_and_forever_installs() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("search-root");
        let retail = create_wow_client(&root, "_retail_");
        let forever = create_wow_client(&root, "_forever_");

        let result = search_for_wow_installs(&root, || false, |_| {}).expect("search");

        assert_eq!(result, vec![retail, forever]);
    }

    #[test]
    fn quick_check_finds_current_platform_candidate() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();

        if cfg!(windows) {
            let addons = root
                .join("Program Files (x86)")
                .join("World of Warcraft")
                .join("_retail_")
                .join("Interface")
                .join("AddOns");
            fs::create_dir_all(root.join("Program Files (x86)/World of Warcraft/Data"))
                .expect("create Data");
            fs::write(
                root.join("Program Files (x86)/World of Warcraft/.build.info"),
                "build",
            )
            .expect("write .build.info");
            fs::create_dir_all(&addons).expect("create addons dir");
            fs::write(
                root.join("Program Files (x86)/World of Warcraft/_retail_/Wow.exe"),
                "wow",
            )
            .expect("write Wow.exe");
            assert_eq!(quick_check_common_paths(root), Some(addons));
        } else if cfg!(target_os = "macos") {
            let addons = root
                .join("Applications")
                .join("World of Warcraft")
                .join("_retail_")
                .join("Interface")
                .join("AddOns");
            fs::create_dir_all(root.join("Applications/World of Warcraft/Data"))
                .expect("create Data");
            fs::write(
                root.join("Applications/World of Warcraft/.build.info"),
                "build",
            )
            .expect("write .build.info");
            fs::create_dir_all(&addons).expect("create addons dir");
            fs::create_dir_all(root.join("Applications/World of Warcraft/_retail_"))
                .expect("create retail");
            fs::write(
                root.join("Applications/World of Warcraft/_retail_/World of Warcraft.app"),
                "wow",
            )
            .expect("write app");
            assert_eq!(quick_check_common_paths(root), Some(addons));
        } else {
            let addons = root
                .join(".wine")
                .join("drive_c")
                .join("Program Files (x86)")
                .join("World of Warcraft")
                .join("_retail_")
                .join("Interface")
                .join("AddOns");
            fs::create_dir_all(
                root.join(".wine/drive_c/Program Files (x86)/World of Warcraft/Data"),
            )
            .expect("create Data");
            fs::write(
                root.join(".wine/drive_c/Program Files (x86)/World of Warcraft/.build.info"),
                "build",
            )
            .expect("write .build.info");
            fs::create_dir_all(&addons).expect("create addons dir");
            fs::write(
                root.join(".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Wow.exe"),
                "wow",
            )
            .expect("write Wow.exe");
            assert_eq!(quick_check_common_paths(root), Some(addons));
        }
    }

    #[test]
    fn deep_scan_reports_progress_and_finds_path() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("search-root");
        let addons = create_wow_install(&root);
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut progress_calls = 0usize;

        let result = search_for_wow(
            &root,
            || cancelled.load(Ordering::SeqCst),
            |_| progress_calls += 1,
        )
        .expect("search should succeed");

        assert_eq!(result, Some(addons));
        assert!(progress_calls > 0);
    }

    #[test]
    fn suggestions_are_never_empty() {
        assert!(!suggested_scan_roots().is_empty());
    }
}
