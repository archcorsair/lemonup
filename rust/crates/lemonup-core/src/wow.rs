use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

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

pub fn detect_known_addons_path() -> Option<PathBuf> {
    if cfg!(windows) {
        for root in suggested_scan_roots() {
            let root_str = root.display().to_string();
            if root_str.len() == 3 && root_str.ends_with(":\\") {
                let drive = &root_str[0..1];
                for template in windows_relative_candidates() {
                    let candidate = PathBuf::from(format!("{drive}{template}"));
                    if validate_addons_path(&candidate).is_ok() {
                        return Some(candidate);
                    }
                }
            }
        }
        return None;
    }

    let candidates = if cfg!(target_os = "macos") {
        vec![PathBuf::from(
            "/Applications/World of Warcraft/_retail_/Interface/AddOns",
        )]
    } else {
        let home = env::var_os("HOME").map(PathBuf::from)?;
        vec![
            home.join(
                "Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ),
            home.join(
                ".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ),
            home.join(
                ".var/app/com.usebottles.bottles/data/bottles/bottles/World-of-Warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ),
            home.join(
                ".local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
            ),
            home.join(
                ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
            ),
        ]
    };

    candidates
        .into_iter()
        .find(|candidate| validate_addons_path(candidate).is_ok())
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

    if !parts
        .iter()
        .any(|part| part.eq_ignore_ascii_case("_retail_"))
    {
        return Err("path must point to a retail WoW AddOns folder".to_string());
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
    mut should_cancel: C,
    mut on_progress: F,
) -> std::result::Result<Option<PathBuf>, String>
where
    F: FnMut(ScanProgressUpdate),
    C: FnMut() -> bool,
{
    if validate_addons_path(root).is_ok() {
        return Ok(Some(root.to_path_buf()));
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
            return Ok(None);
        }

        dirs_scanned += 1;
        on_progress(ScanProgressUpdate {
            dirs_scanned,
            current_path: current_dir.clone(),
        });

        if dirs_scanned.is_multiple_of(50) {
            thread::yield_now();
            if should_cancel() {
                return Ok(None);
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

        if let Some((_, retail_dir)) = child_dirs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("_retail_"))
        {
            let addons_path = retail_dir.join("Interface").join("AddOns");
            if validate_addons_path(&addons_path).is_ok() {
                return Ok(Some(addons_path));
            }
        }

        for (_, child_dir) in child_dirs {
            queue.push_back(child_dir);
        }
    }

    Ok(None)
}

fn common_relative_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        windows_relative_candidates()
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from(
            "Applications/World of Warcraft/_retail_/Interface/AddOns",
        )]
    } else {
        vec![
            PathBuf::from(
                "Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ),
            PathBuf::from(
                ".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ),
            PathBuf::from(
                ".local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
            ),
        ]
    }
}

fn windows_relative_candidates() -> Vec<&'static str> {
    vec![
        r":\Program Files (x86)\World of Warcraft\_retail_\Interface\AddOns",
        r":\Program Files\World of Warcraft\_retail_\Interface\AddOns",
        r":\Games\World of Warcraft\_retail_\Interface\AddOns",
        r":\World of Warcraft\_retail_\Interface\AddOns",
    ]
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
        quick_check_common_paths, search_for_wow, suggested_scan_roots, validate_addons_path,
    };

    fn create_wow_install(root: &Path) -> PathBuf {
        let wow_root = root.join("World of Warcraft");
        let retail = wow_root.join("_retail_");
        let addons = retail.join("Interface").join("AddOns");

        fs::create_dir_all(wow_root.join("Data")).expect("create Data");
        fs::write(wow_root.join(".build.info"), "build").expect("write .build.info");
        fs::create_dir_all(&addons).expect("create addons dir");
        fs::write(retail.join("Wow.exe"), "wow").expect("write Wow.exe");

        addons
    }

    #[test]
    fn validates_a_realistic_addons_path() {
        let temp = tempdir().expect("tempdir");
        let addons = create_wow_install(temp.path());

        assert!(validate_addons_path(&addons).is_ok());
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
