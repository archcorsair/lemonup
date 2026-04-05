use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::domain::{AddonKind, GameFlavor, OwnedFolder, SourceKind};
use crate::error::{LemonupError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocSelectionConfidence {
    Exact,
    Fallback,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocSelectionResult {
    pub selected: String,
    pub confidence: TocSelectionConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocMetadata {
    pub title: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub interface: Option<String>,
    pub required_deps: Vec<String>,
    pub optional_deps: Vec<String>,
    pub x_library: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedAddon {
    pub name: String,
    pub folder: String,
    pub owned_folders: Vec<OwnedFolder>,
    pub kind: AddonKind,
    pub flavor: GameFlavor,
    pub version: Option<String>,
    pub git_commit: Option<String>,
    pub author: Option<String>,
    pub interface: Option<String>,
    pub source: SourceKind,
    pub required_deps: Vec<String>,
    pub optional_deps: Vec<String>,
    pub embedded_libs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    pub scanned_addons: usize,
    pub upserted_addons: usize,
    pub removed_addons: usize,
}

pub fn scan_addons_dir(addons_dir: &Path, flavor: GameFlavor) -> Result<Vec<ScannedAddon>> {
    if !addons_dir.exists() {
        return Err(LemonupError::InvalidArgument(format!(
            "addons directory does not exist: {}",
            addons_dir.display()
        )));
    }
    if !addons_dir.is_dir() {
        return Err(LemonupError::InvalidArgument(format!(
            "addons directory is not a directory: {}",
            addons_dir.display()
        )));
    }

    let mut scanned = Vec::new();

    for entry in fs::read_dir(addons_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let folder = entry.file_name().to_string_lossy().to_string();
        let addon_path = entry.path();
        let toc_files = list_root_toc_files(&addon_path)?;
        if toc_files.is_empty() {
            continue;
        }

        let selected_toc = select_toc_file(&folder, &toc_files, flavor)?;
        let content = fs::read_to_string(addon_path.join(&selected_toc.selected))?;
        let toc = parse_toc_content(&content, &folder);
        let kind = detect_addon_kind(&addon_path, &toc);
        let git_commit = read_git_commit(&addon_path);
        let version = toc
            .version
            .clone()
            .or_else(|| git_commit.as_ref().map(|commit| short_commit(commit)));

        scanned.push(ScannedAddon {
            name: toc.title,
            folder,
            owned_folders: Vec::new(),
            kind,
            flavor,
            version,
            git_commit: git_commit.clone(),
            author: toc.author,
            interface: toc.interface,
            source: if git_commit.is_some() {
                SourceKind::GitHub
            } else {
                SourceKind::Manual
            },
            required_deps: toc.required_deps,
            optional_deps: toc.optional_deps,
            embedded_libs: detect_embedded_libs(&addon_path)?,
        });
    }

    let mut collapsed = collapse_owned_folders(scanned);
    collapsed.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| {
                left.folder
                    .to_ascii_lowercase()
                    .cmp(&right.folder.to_ascii_lowercase())
            })
    });

    Ok(collapsed)
}

pub fn select_toc_file(
    addon_folder: &str,
    toc_files: &[String],
    target_flavor: GameFlavor,
) -> Result<TocSelectionResult> {
    if toc_files.is_empty() {
        return Err(LemonupError::InvalidArgument(format!(
            "no TOC files found for addon folder '{addon_folder}'"
        )));
    }

    if toc_files.len() == 1 {
        return Ok(TocSelectionResult {
            selected: toc_files[0].clone(),
            confidence: TocSelectionConfidence::Exact,
        });
    }

    let base_toc = format!("{addon_folder}.toc");
    for suffix in flavor_suffixes(target_flavor) {
        let flavor_toc = format!("{addon_folder}{suffix}.toc");
        if let Some(found) = toc_files
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(&flavor_toc))
        {
            return Ok(TocSelectionResult {
                selected: found.clone(),
                confidence: TocSelectionConfidence::Exact,
            });
        }
    }

    if let Some(found) = toc_files
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(&base_toc))
    {
        return Ok(TocSelectionResult {
            selected: found.clone(),
            confidence: TocSelectionConfidence::Fallback,
        });
    }

    let mut sorted = toc_files.to_vec();
    sorted.sort_by_key(|candidate| candidate.to_ascii_lowercase());

    Ok(TocSelectionResult {
        selected: sorted[0].clone(),
        confidence: TocSelectionConfidence::Ambiguous,
    })
}

pub fn parse_toc_content(content: &str, fallback_title: &str) -> TocMetadata {
    let title = field_value(content, &["Title"])
        .map(strip_wow_color_codes)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_title.to_string());

    TocMetadata {
        title,
        version: field_value(content, &["Version"]),
        author: field_value(content, &["Author"]),
        interface: field_value(content, &["Interface"]),
        required_deps: parse_dependency_list(field_value(
            content,
            &["Dependencies", "RequiredDeps"],
        )),
        optional_deps: parse_dependency_list(field_value(content, &["OptionalDeps"])),
        x_library: field_value(content, &["X-Library"])
            .is_some_and(|value| value.eq_ignore_ascii_case("true")),
    }
}

fn list_root_toc_files(addon_path: &Path) -> Result<Vec<String>> {
    let mut toc_files = Vec::new();

    for entry in fs::read_dir(addon_path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.to_ascii_lowercase().ends_with(".toc") {
            toc_files.push(file_name);
        }
    }

    Ok(toc_files)
}

fn detect_embedded_libs(addon_path: &Path) -> Result<Vec<String>> {
    const EMBEDDED_LIB_DIRS: &[&str] = &["Libs", "libs", "Lib", "lib", "Libraries"];

    let mut embedded = HashSet::new();
    for lib_dir in EMBEDDED_LIB_DIRS {
        let libs_path = addon_path.join(lib_dir);
        if !libs_path.is_dir() {
            continue;
        }

        for entry in fs::read_dir(&libs_path)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let child_name = entry.file_name().to_string_lossy().to_string();
            let child_path = entry.path();
            if has_any_toc_file(&child_path)? {
                embedded.insert(child_name);
            }
        }
    }

    let mut libs = embedded.into_iter().collect::<Vec<_>>();
    libs.sort_by_key(|name| name.to_ascii_lowercase());
    Ok(libs)
}

fn has_any_toc_file(path: &Path) -> Result<bool> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.to_ascii_lowercase().ends_with(".toc") {
            return Ok(true);
        }
    }

    Ok(false)
}

fn detect_addon_kind(addon_path: &Path, toc: &TocMetadata) -> AddonKind {
    if toc.x_library {
        return AddonKind::Library;
    }

    let folder_name = addon_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();

    if matches_library_pattern(&folder_name) {
        return AddonKind::Library;
    }

    AddonKind::Addon
}

fn matches_library_pattern(folder_name: &str) -> bool {
    let lower = folder_name.to_ascii_lowercase();
    folder_name.starts_with("Lib")
        || folder_name.starts_with("Ace")
        || folder_name.starts_with("CallbackHandler")
        || lower == "libstub"
        || lower
            .rsplit_once('-')
            .is_some_and(|(_, suffix)| is_major_minor_suffix(suffix))
}

fn is_major_minor_suffix(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !major.is_empty()
        && !minor.is_empty()
        && major.chars().all(|ch| ch.is_ascii_digit())
        && minor.chars().all(|ch| ch.is_ascii_digit())
}

fn read_git_commit(addon_path: &Path) -> Option<String> {
    let git_dir = addon_path.join(".git");
    if !git_dir.is_dir() {
        return None;
    }

    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let trimmed = head.trim();
    if let Some(reference) = trimmed.strip_prefix("ref:") {
        let reference = reference.trim();
        return fs::read_to_string(git_dir.join(reference))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }

    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(7).collect()
}

fn collapse_owned_folders(scanned: Vec<ScannedAddon>) -> Vec<ScannedAddon> {
    let folder_names = scanned
        .iter()
        .map(|addon| addon.folder.clone())
        .collect::<Vec<_>>();

    let mut folder_to_index = HashMap::new();
    for (index, addon) in scanned.iter().enumerate() {
        folder_to_index.insert(addon.folder.to_ascii_lowercase(), index);
    }

    let mut children_by_parent: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, addon) in scanned.iter().enumerate() {
        let Some(base_folder) = conservative_parent_folder(&addon.folder) else {
            continue;
        };
        let Some(&parent_index) = folder_to_index.get(&base_folder.to_ascii_lowercase()) else {
            continue;
        };
        if parent_index == index {
            continue;
        }

        children_by_parent
            .entry(parent_index)
            .or_default()
            .push(index);
    }

    let child_indexes = children_by_parent
        .values()
        .flat_map(|children| children.iter().copied())
        .collect::<HashSet<_>>();

    let mut collapsed = Vec::new();
    for (index, mut addon) in scanned.into_iter().enumerate() {
        if child_indexes.contains(&index) {
            continue;
        }

        if let Some(children) = children_by_parent.get(&index) {
            let mut owned = children
                .iter()
                .map(|child_index| OwnedFolder {
                    name: folder_names[*child_index].clone(),
                })
                .collect::<Vec<_>>();
            owned.sort_by_key(|folder| folder.name.to_ascii_lowercase());
            addon.owned_folders = owned;
        }

        collapsed.push(addon);
    }

    collapsed
}

fn conservative_parent_folder(folder_name: &str) -> Option<&str> {
    folder_name
        .find(['_', '-'])
        .and_then(|separator| (separator > 0).then(|| &folder_name[..separator]))
}

fn flavor_suffixes(flavor: GameFlavor) -> &'static [&'static str] {
    match flavor {
        GameFlavor::Retail => &["-Retail", "_Mainline", "-Mainline"],
        GameFlavor::Classic => &[
            "-Classic", "_Classic", "-Vanilla", "_Vanilla", "-Era", "_Era",
        ],
        GameFlavor::Cata => &["-Cata", "_Cata", "-Cataclysm", "_Cataclysm"],
    }
}

fn field_value(content: &str, keys: &[&str]) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(remainder) = trimmed.strip_prefix("##") else {
            continue;
        };
        let Some((raw_key, raw_value)) = remainder.trim().split_once(':') else {
            continue;
        };

        if keys
            .iter()
            .any(|key| raw_key.trim().eq_ignore_ascii_case(key))
        {
            let value = raw_value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

fn parse_dependency_list(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };

    raw.split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn strip_wow_color_codes(value: String) -> String {
    let mut result = String::with_capacity(value.len());
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '|' && index + 1 < chars.len() {
            match chars[index + 1] {
                'c' | 'C' if index + 10 <= chars.len() => {
                    index += 10;
                    continue;
                }
                'r' | 'R' => {
                    index += 2;
                    continue;
                }
                _ => {}
            }
        }

        result.push(chars[index]);
        index += 1;
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        ScannedAddon, TocSelectionConfidence, collapse_owned_folders, parse_toc_content,
        scan_addons_dir, select_toc_file,
    };
    use crate::domain::{AddonKind, GameFlavor};

    #[test]
    fn parses_toc_metadata_and_strips_color_codes() {
        let parsed = parse_toc_content(
            "## Title: |cffabd473Details! Damage Meter|r\n## Version: 1.0\n## Author: Tercio\n## Interface: 110002\n## Dependencies: Ace3, LibStub CallbackHandler-1.0\n## OptionalDeps: ElvUI Masque\n## X-Library: true",
            "Fallback",
        );

        assert_eq!(parsed.title, "Details! Damage Meter");
        assert_eq!(parsed.version.as_deref(), Some("1.0"));
        assert_eq!(parsed.author.as_deref(), Some("Tercio"));
        assert_eq!(parsed.interface.as_deref(), Some("110002"));
        assert_eq!(
            parsed.required_deps,
            vec!["Ace3", "LibStub", "CallbackHandler-1.0"]
        );
        assert_eq!(parsed.optional_deps, vec!["ElvUI", "Masque"]);
        assert!(parsed.x_library);
    }

    #[test]
    fn ignores_localized_metadata_without_utf8_boundary_panics() {
        let parsed = parse_toc_content(
            "## Notes-ruRU: Собирает информацию об ошибках и ловит их в мешок.\n## Title: BugSack\n## Author: Rabbit\n## OptionalDeps: Masque",
            "Fallback",
        );

        assert_eq!(parsed.title, "BugSack");
        assert_eq!(parsed.author.as_deref(), Some("Rabbit"));
        assert_eq!(parsed.optional_deps, vec!["Masque"]);
    }

    #[test]
    fn selects_retail_toc_before_base_toc() {
        let selected = select_toc_file(
            "WeakAuras",
            &[
                "WeakAuras.toc".to_string(),
                "WeakAuras-Retail.toc".to_string(),
                "WeakAuras-Classic.toc".to_string(),
            ],
            GameFlavor::Retail,
        )
        .expect("select toc");

        assert_eq!(selected.selected, "WeakAuras-Retail.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Exact);
    }

    #[test]
    fn falls_back_to_alphabetical_when_no_base_or_flavor_match_exists() {
        let selected = select_toc_file(
            "Addon",
            &["Zebra.toc".to_string(), "Alpha.toc".to_string()],
            GameFlavor::Retail,
        )
        .expect("select toc");

        assert_eq!(selected.selected, "Alpha.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Ambiguous);
    }

    #[test]
    fn scan_discovers_only_top_level_addons_and_embedded_libs() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();

        let details = addons.join("Details");
        fs::create_dir_all(details.join("Libs").join("LibStub")).expect("create lib dir");
        fs::create_dir_all(details.join("nested").join("IgnoredAddon")).expect("create nested dir");
        fs::write(
            details.join("Details.toc"),
            "## Title: Details\n## Version: 1.2.3\n## Author: Tercio",
        )
        .expect("write details toc");
        fs::write(
            details.join("Libs").join("LibStub").join("LibStub.toc"),
            "## Title: LibStub",
        )
        .expect("write embedded toc");
        fs::write(
            details
                .join("nested")
                .join("IgnoredAddon")
                .join("IgnoredAddon.toc"),
            "## Title: Ignored",
        )
        .expect("write nested toc");

        let ignored_folder = addons.join("NoTocFolder");
        fs::create_dir_all(&ignored_folder).expect("create ignored folder");

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan addons");

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].folder, "Details");
        assert_eq!(scanned[0].embedded_libs, vec!["LibStub"]);
        assert_eq!(scanned[0].version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn scan_collapses_only_conservative_owned_folder_groups() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();

        for (folder, toc) in [
            ("Details", "## Title: Details"),
            ("Details_DataStorage", "## Title: Details DataStorage"),
            ("DBM-Core", "## Title: DBM Core"),
            ("DBM-Naxx", "## Title: DBM Naxx\n## Dependencies: DBM-Core"),
            ("LibSharedMedia-3.0", "## Title: LibSharedMedia"),
        ] {
            let folder_path = addons.join(folder);
            fs::create_dir_all(&folder_path).expect("create addon dir");
            fs::write(folder_path.join(format!("{folder}.toc")), toc).expect("write toc");
        }

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan addons");
        let details = scanned
            .iter()
            .find(|addon| addon.folder == "Details")
            .expect("details row");
        let dbm_core = scanned
            .iter()
            .find(|addon| addon.folder == "DBM-Core")
            .expect("dbm core row");
        let dbm_naxx = scanned
            .iter()
            .find(|addon| addon.folder == "DBM-Naxx")
            .expect("dbm naxx row");
        let lib = scanned
            .iter()
            .find(|addon| addon.folder == "LibSharedMedia-3.0")
            .expect("library row");

        assert_eq!(details.owned_folders.len(), 1);
        assert_eq!(details.owned_folders[0].name, "Details_DataStorage");
        assert!(
            scanned
                .iter()
                .all(|addon| addon.folder != "Details_DataStorage")
        );
        assert!(dbm_core.owned_folders.is_empty());
        assert!(dbm_naxx.owned_folders.is_empty());
        assert_eq!(lib.kind, AddonKind::Library);
    }

    #[test]
    fn collapse_keeps_unrelated_dash_groups_separate_without_exact_base_folder() {
        let addons = vec![
            ScannedAddon {
                name: "DBM Core".to_string(),
                folder: "DBM-Core".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: None,
                git_commit: None,
                author: None,
                interface: None,
                source: crate::domain::SourceKind::Manual,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
            },
            ScannedAddon {
                name: "DBM Naxx".to_string(),
                folder: "DBM-Naxx".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: None,
                git_commit: None,
                author: None,
                interface: None,
                source: crate::domain::SourceKind::Manual,
                required_deps: vec!["DBM-Core".to_string()],
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
            },
        ];

        let collapsed = collapse_owned_folders(addons);
        assert_eq!(collapsed.len(), 2);
    }
}
