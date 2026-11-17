use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::domain::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, SourceKind};
use crate::error::{LemonupError, Result};
use crate::toc_conditions;

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
    /// The addon's name: the first `## Title:` line (indented ones included) that applies to the
    /// client, with its inline conditions and color codes removed, or the fallback.
    pub title: String,
    /// `## Title:` as the game reads it (the last base line that applies to the client wins),
    /// without color codes removed.
    pub toc_title: Option<String>,
    /// `## Notes:` as the game reads it (the last base line that applies to the client wins),
    /// without color codes removed.
    pub notes: Option<String>,
    /// `## Category:` as the game reads it (the last base line that applies to the client wins).
    pub category: Option<String>,
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
    /// The selected TOC's `Notes` (see [`TocMetadata::notes`]).
    pub notes: Option<String>,
    /// The selected TOC's `Category` (see [`TocMetadata::category`]).
    pub category: Option<String>,
    /// The selected TOC's `Title` as the game reads it (see [`TocMetadata::toc_title`]).
    pub toc_title: Option<String>,
}

/// Result of a scan that also reports what it could not read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanReport {
    pub addons: Vec<ScannedAddon>,
    /// Addon folders skipped because they could not be read (for example a locked TOC). They are
    /// present on disk, so reconciliation must leave any tracked row for them untouched.
    pub skipped_folders: Vec<String>,
    /// `.lemonup-staging-*` folders found in the AddOns folder: leftovers of an update whose
    /// cleanup failed. They are never scanned as addons but may hold the previous addon versions.
    pub leftover_staging: Vec<String>,
}

/// Name prefix of the hidden per-update staging folder LemonUp creates inside the AddOns folder.
pub const UPDATE_STAGING_PREFIX: &str = ".lemonup-staging-";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanSummary {
    pub scanned_addons: usize,
    pub upserted_addons: usize,
    pub removed_addons: usize,
    /// Folders on disk that could not be read and were left as they were.
    pub skipped_folders: Vec<String>,
    /// Leftover update staging folders seen by the scan (see [`ScanReport::leftover_staging`]).
    pub leftover_staging: Vec<String>,
}

/// Scans an AddOns directory with no knowledge of tracked rows.
///
/// The folder-name grouping heuristic runs unguarded, which is right for scanning a freshly
/// extracted archive. Scans of a live AddOns directory that feed reconciliation should use
/// [`scan_addons_dir_with_tracked`] so separately tracked addons are never absorbed.
pub fn scan_addons_dir(addons_dir: &Path, flavor: GameFlavor) -> Result<Vec<ScannedAddon>> {
    scan_addons_dir_with_tracked(addons_dir, flavor, &[])
}

/// Scans an AddOns directory, letting the tracked rows veto name-heuristic grouping.
///
/// A folder is never grouped under a sibling parent by name alone when it has its own tracked
/// row with a non-manual source, or when the would-be parent has authoritative owned folders that
/// do not list it. Such folders stay top-level scanned addons with their own metadata.
pub fn scan_addons_dir_with_tracked(
    addons_dir: &Path,
    flavor: GameFlavor,
    tracked: &[AddonRecord],
) -> Result<Vec<ScannedAddon>> {
    Ok(scan_addons_dir_report(addons_dir, flavor, tracked)?.addons)
}

/// Like [`scan_addons_dir_with_tracked`], but also reports the folders that were skipped because
/// they could not be read. Scans that feed reconciliation of a live AddOns directory should use
/// this with [`StateDatabase::reconcile_scan`](crate::StateDatabase::reconcile_scan), so a
/// transiently locked folder never costs its tracked row.
pub fn scan_addons_dir_report(
    addons_dir: &Path,
    flavor: GameFlavor,
    tracked: &[AddonRecord],
) -> Result<ScanReport> {
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
    let mut skipped_folders = Vec::new();
    let mut leftover_staging = Vec::new();

    for entry in fs::read_dir(addons_dir)? {
        let entry = entry?;
        let addon_path = entry.path();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(UPDATE_STAGING_PREFIX)
        {
            // Never an addon, but it can hold the previous versions of updated addons.
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                leftover_staging.push(entry.file_name().to_string_lossy().to_string());
            }
            continue;
        }
        let folder = entry.file_name().to_string_lossy().to_string();
        // Follow symlinks so linked development checkouts are scanned; a broken or looping link
        // (or any entry whose metadata cannot be read) is skipped rather than failing the whole
        // scan. It may still be an addon folder, so it is reported as skipped and reconciliation
        // keeps its row. The scan only reads.
        let metadata = match fs::metadata(&addon_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    folder = %folder,
                    path = %addon_path.display(),
                    %error,
                    "skipping AddOns entry whose metadata could not be read"
                );
                skipped_folders.push(folder);
                continue;
            }
        };
        if !metadata.is_dir() {
            continue;
        }

        // One unreadable addon folder must not fail the whole scan (and with it `sync` or a
        // provider package scan): skip it and name it in the log.
        match scan_addon_folder(&addon_path, &folder, flavor) {
            Ok(Some(addon)) => scanned.push(addon),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    folder = %folder,
                    path = %addon_path.display(),
                    %error,
                    "skipping addon folder that could not be read"
                );
                skipped_folders.push(folder);
            }
        }
    }

    let guard = GroupingGuard::from_tracked(tracked);
    let mut collapsed = collapse_owned_folders(scanned, &guard);
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

    skipped_folders.sort();
    leftover_staging.sort();
    Ok(ScanReport {
        addons: collapsed,
        skipped_folders,
        leftover_staging,
    })
}

/// The Title, Notes and Category the game reads for the addon in `folder` of `addons_dir`, from
/// the TOC a scan selects for `flavor`. `Ok(None)` when the folder has no TOC for `flavor`.
pub fn read_addon_toc_fields(
    addons_dir: &Path,
    folder: &str,
    flavor: GameFlavor,
) -> Result<Option<crate::toc_edit::TocFieldValues>> {
    Ok(scan_addon_folder(&addons_dir.join(folder), folder, flavor)?
        .map(|addon| crate::toc_edit::TocFieldValues::from_scanned(&addon)))
}

/// The text of the TOC a scan selects for `flavor` in the addon folder at `addon_path`.
/// `Ok(None)` when the folder has no TOC loadable by `flavor`.
pub(crate) fn read_selected_toc(
    addon_path: &Path,
    folder: &str,
    flavor: GameFlavor,
) -> Result<Option<String>> {
    let toc_files = list_root_toc_files(addon_path)?
        .into_iter()
        .filter(|toc| toc_loadable_by_flavor(toc, folder, flavor))
        .collect::<Vec<_>>();
    if toc_files.is_empty() {
        return Ok(None);
    }
    let selected_toc = select_toc_file(folder, &toc_files, flavor)?;
    Ok(Some(read_toc_file(
        &addon_path.join(&selected_toc.selected),
    )?))
}

/// The value of the `## <key>:` field the game uses for `flavor`'s client (the last line that
/// applies), with its inline conditions taken out.
pub(crate) fn toc_field(content: &str, key: &str, flavor: GameFlavor) -> Option<String> {
    last_field_value(content, &[key], Some(flavor))
}

/// Scans one addon folder. `Ok(None)` means the folder has no TOC loadable by `flavor`.
pub(crate) fn scan_addon_folder(
    addon_path: &Path,
    folder: &str,
    flavor: GameFlavor,
) -> Result<Option<ScannedAddon>> {
    let toc_files = list_root_toc_files(addon_path)?
        .into_iter()
        .filter(|toc| toc_loadable_by_flavor(toc, folder, flavor))
        .collect::<Vec<_>>();
    if toc_files.is_empty() {
        return Ok(None);
    }

    let selected_toc = select_toc_file(folder, &toc_files, flavor)?;
    let content = read_toc_file(&addon_path.join(&selected_toc.selected))?;
    let toc = parse_toc_content(&content, folder, Some(flavor));
    let kind = detect_addon_kind(addon_path, &toc);
    let git_commit = read_git_commit(addon_path);
    let version = toc
        .version
        .clone()
        .or_else(|| git_commit.as_ref().map(|commit| short_commit(commit)));

    Ok(Some(ScannedAddon {
        name: toc.title,
        folder: folder.to_string(),
        owned_folders: Vec::new(),
        kind,
        flavor,
        version,
        git_commit,
        author: toc.author,
        interface: toc.interface,
        // A folder with a `.git` directory is a development checkout, not a tracked GitHub
        // install: tracked GitHub rows come from `install`/`update` and carry a source URL.
        // The commit stays as informational metadata.
        source: SourceKind::Manual,
        required_deps: toc.required_deps,
        optional_deps: toc.optional_deps,
        embedded_libs: detect_embedded_libs(addon_path)?,
        notes: toc.notes,
        category: toc.category,
        toc_title: toc.toc_title,
    }))
}

/// Reads a TOC tolerantly. Real-world TOCs are not always valid UTF-8 (Latin-1 titles are
/// common) and some carry a UTF-8 BOM that would otherwise hide the first line; invalid bytes
/// decode lossily and the BOM is dropped. Only genuine I/O failures are errors.
fn read_toc_file(path: &Path) -> Result<String> {
    Ok(decode_toc_bytes(&fs::read(path)?))
}

fn decode_toc_bytes(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
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

    let toc_files = toc_files
        .iter()
        .filter(|toc| toc_loadable_by_flavor(toc, addon_folder, target_flavor))
        .cloned()
        .collect::<Vec<_>>();
    if toc_files.is_empty() {
        return Err(LemonupError::InvalidArgument(format!(
            "no TOC files for this game version found for addon folder '{addon_folder}'"
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

/// Reads a TOC's metadata for `flavor`'s client: lines whose inline conditions
/// (`[AllowLoadGameType ...]`, see [`crate::toc_conditions`]) leave this client out are skipped,
/// and the conditions are taken out of the values. With `flavor` `None` (game version not known)
/// only lines without a game type condition count.
pub fn parse_toc_content(
    content: &str,
    fallback_title: &str,
    flavor: Option<GameFlavor>,
) -> TocMetadata {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    // The game uses the last line that applies; only the name keeps the first, so names stay
    // stable.
    let field = |keys: &[&str]| last_field_value(content, keys, flavor);
    let title = field_value(content, &["Title"], flavor)
        .map(strip_wow_color_codes)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_title.to_string());

    TocMetadata {
        title,
        toc_title: directive_value(content, "Title", flavor),
        notes: directive_value(content, "Notes", flavor),
        category: directive_value(content, "Category", flavor),
        version: field(&["Version"]),
        author: field(&["Author"]),
        interface: field(&["Interface"]),
        required_deps: parse_dependency_list(field(&["Dependencies", "RequiredDeps"])),
        optional_deps: parse_dependency_list(field(&["OptionalDeps"])),
        x_library: field(&["X-Library"]).is_some_and(|value| value.eq_ignore_ascii_case("true")),
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

/// Tracked-row knowledge that vetoes the folder-name grouping heuristic.
#[derive(Debug, Default)]
pub(crate) struct GroupingGuard {
    /// Lowercased folders with their own tracked row whose source is not manual.
    standalone: HashSet<String>,
    /// Lowercased parent folder to the lowercased folders it authoritatively owns.
    authoritative: HashMap<String, HashSet<String>>,
}

impl GroupingGuard {
    pub(crate) fn from_tracked(tracked: &[AddonRecord]) -> Self {
        let mut guard = Self::default();
        for addon in tracked {
            let folder = addon.folder.to_ascii_lowercase();
            // A row from a newer LemonUp is linked there, so it stays standalone too.
            if addon.source != SourceKind::Manual || addon.unrecognized.is_some() {
                guard.standalone.insert(folder.clone());
            }
            // An owned-folder list stored by a newer LemonUp is kept as it is, so the name
            // heuristic may not fold any other folder into that addon either.
            if addon.has_authoritative_owned_folders() || addon.has_unrecognized_ownership() {
                guard.authoritative.insert(
                    folder,
                    addon
                        .owned_folders
                        .iter()
                        .map(|owned| owned.name.to_ascii_lowercase())
                        .collect(),
                );
            }
        }
        guard
    }

    /// Whether the name heuristic may fold `child` into `parent`.
    pub(crate) fn may_group(&self, parent: &str, child: &str) -> bool {
        let child = child.to_ascii_lowercase();
        if self.standalone.contains(&child) {
            return false;
        }
        self.authoritative
            .get(&parent.to_ascii_lowercase())
            .is_none_or(|owned| owned.contains(&child))
    }
}

fn collapse_owned_folders(scanned: Vec<ScannedAddon>, guard: &GroupingGuard) -> Vec<ScannedAddon> {
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
        if parent_index == index || !guard.may_group(&scanned[parent_index].folder, &addon.folder) {
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
        GameFlavor::Forever => &[
            "_Forever",
            "-Forever",
            "_Camelot",
            "-Camelot",
            "_Mainline",
            "-Mainline",
        ],
    }
}

/// WoW Forever flavor TOCs (`<Folder>_Forever`, `<Folder>_Camelot`, or the `-` variants) only
/// load in the Forever client, so other flavors must never select them. A TOC only counts as a
/// flavor TOC when its stem is exactly `<folder name><suffix>`, matching how the client resolves
/// them; a folder that is itself named `Foo_Camelot` keeps its base `Foo_Camelot.toc`.
fn toc_loadable_by_flavor(toc_file: &str, addon_folder: &str, flavor: GameFlavor) -> bool {
    if flavor == GameFlavor::Forever {
        return true;
    }

    let stem = toc_file
        .len()
        .checked_sub(".toc".len())
        .and_then(|end| toc_file.get(..end))
        .unwrap_or(toc_file)
        .to_ascii_lowercase();
    let folder = addon_folder.to_ascii_lowercase();
    !["_forever", "-forever", "_camelot", "-camelot"]
        .iter()
        .any(|suffix| stem == format!("{folder}{suffix}"))
}

/// The first `## <key>:` line (indented ones included) that applies to `flavor`'s client, with
/// its inline conditions taken out.
fn field_value(content: &str, keys: &[&str], flavor: Option<GameFlavor>) -> Option<String> {
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
            let line = toc_conditions::evaluate(raw_value, flavor);
            if line.applies && !line.value.is_empty() {
                return Some(line.value);
            }
        }
    }

    None
}

/// The last `## <key>:` line (indented ones included) that applies to `flavor`'s client, with its
/// inline conditions taken out: the line the game uses when a directive is given more than once.
fn last_field_value(content: &str, keys: &[&str], flavor: Option<GameFlavor>) -> Option<String> {
    let mut found = None;
    for line in content.lines() {
        let Some(remainder) = line.trim().strip_prefix("##") else {
            continue;
        };
        let Some((raw_key, raw_value)) = remainder.trim().split_once(':') else {
            continue;
        };
        if keys
            .iter()
            .any(|key| raw_key.trim().eq_ignore_ascii_case(key))
        {
            let line = toc_conditions::evaluate(raw_value, flavor);
            if line.applies && !line.value.is_empty() {
                found = Some(line.value);
            }
        }
    }
    found
}

/// The value the game uses for a display directive: of the `## <key>:` lines that start in
/// column 0, have no locale suffix and apply to `flavor`'s client, the last one wins, with its
/// inline conditions taken out. Lines LemonUp commented out (`# LemonUp: ...`) are not
/// directives, so an override LemonUp wrote is what this returns.
fn directive_value(content: &str, key: &str, flavor: Option<GameFlavor>) -> Option<String> {
    let mut found = None;
    for line in content.lines() {
        let Some(remainder) = line.strip_prefix("##") else {
            continue;
        };
        let Some((raw_key, raw_value)) = remainder.split_once(':') else {
            continue;
        };
        if raw_key.trim().eq_ignore_ascii_case(key) {
            let line = toc_conditions::evaluate(raw_value, flavor);
            if line.applies && !line.value.is_empty() {
                found = Some(line.value);
            }
        }
    }
    found
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

/// `value` without WoW color codes, runs of whitespace collapsed to one space and the ends
/// trimmed.
pub(crate) fn strip_wow_color_codes(value: String) -> String {
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

    // A removed code leaves its neighbors' spaces side by side.
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        GroupingGuard, ScannedAddon, TocSelectionConfidence, UPDATE_STAGING_PREFIX,
        collapse_owned_folders, parse_toc_content, read_addon_toc_fields, scan_addons_dir,
        scan_addons_dir_report, scan_addons_dir_with_tracked, select_toc_file,
    };
    use crate::domain::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, SourceKind};

    #[test]
    fn scan_reports_leftover_staging_folders_without_scanning_them_as_addons() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        fs::create_dir_all(addons.join("Foo")).expect("addon");
        fs::write(addons.join("Foo").join("Foo.toc"), "## Title: Foo\n").expect("toc");
        let staging = format!("{UPDATE_STAGING_PREFIX}123-456-0");
        // Even a TOC inside the leftover must not make it an addon.
        fs::create_dir_all(addons.join(&staging).join("old").join("Foo")).expect("staging");
        fs::write(
            addons
                .join(&staging)
                .join("old")
                .join("Foo")
                .join("Foo.toc"),
            "## Title: Foo\n",
        )
        .expect("old toc");
        fs::write(addons.join(&staging).join("Stray.toc"), "## Title: Stray\n").expect("toc");
        // A plain file with the prefix is not a staging folder.
        fs::write(addons.join(format!("{UPDATE_STAGING_PREFIX}file")), "x").expect("file");

        let report = scan_addons_dir_report(&addons, GameFlavor::Retail, &[]).expect("scan");

        assert_eq!(report.leftover_staging, vec![staging]);
        assert_eq!(
            report
                .addons
                .iter()
                .map(|addon| addon.folder.as_str())
                .collect::<Vec<_>>(),
            vec!["Foo"]
        );
        assert!(report.skipped_folders.is_empty());
    }

    #[test]
    fn a_removed_color_code_leaves_no_doubled_spaces_in_the_name() {
        let parsed = parse_toc_content(
            "## Title:  Decursive |cff00ff00|r -Ace3-  \n## Version: 1",
            "Decursive",
            None,
        );

        assert_eq!(parsed.title, "Decursive -Ace3-");
    }

    #[test]
    fn parses_toc_metadata_and_strips_color_codes() {
        let parsed = parse_toc_content(
            "## Title: |cffabd473Details! Damage Meter|r\n## Version: 1.0\n## Author: Tercio\n## Interface: 110002\n## Dependencies: Ace3, LibStub CallbackHandler-1.0\n## OptionalDeps: ElvUI Masque\n## X-Library: true",
            "Fallback",
            None,
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
    fn display_fields_follow_the_game_last_line_wins_and_skip_lemonup_comments() {
        let parsed = parse_toc_content(
            "## Title: First\r\n## Notes: Old notes\r\n## Category: Combat\r\n## Title-deDE: Erster\r\n## Notes-frFR: Notes\r\n# LemonUp: ## Category: Commented\r\n  ## Title: Indented\r\n## Title: |cff00ff00Last|r\r\n## Notes: New notes\r\n",
            "Fallback",
            None,
        );
        // The name keeps the first Title line, indented or not, as before overrides existed.
        assert_eq!(parsed.title, "First");
        assert_eq!(parsed.toc_title.as_deref(), Some("|cff00ff00Last|r"));
        assert_eq!(parsed.notes.as_deref(), Some("New notes"));
        assert_eq!(parsed.category.as_deref(), Some("Combat"));

        let overridden = parse_toc_content(
            "## Interface: 110002\n# LemonUp: ## Title: Original\n# LemonUp: ## Category: Combat\n# LemonUp-managed\n## Title: Mine\n",
            "Fallback",
            None,
        );
        assert_eq!(overridden.title, "Mine");
        assert_eq!(overridden.toc_title.as_deref(), Some("Mine"));
        assert_eq!(overridden.category, None);
        assert_eq!(overridden.notes, None);
    }

    #[test]
    fn the_name_still_comes_from_the_first_title_line_even_when_indented() {
        let parsed = parse_toc_content(
            "## Interface: 110002\n  ## Title: |cffff0000Indented|r\n## Title: Later\n",
            "Fallback",
            None,
        );
        assert_eq!(parsed.title, "Indented");
        assert_eq!(parsed.toc_title.as_deref(), Some("Later"));
        assert_eq!(
            parse_toc_content("## Notes: x\n", "Fallback", None).title,
            "Fallback"
        );
    }

    #[test]
    fn scan_reports_notes_and_category_of_the_selected_toc() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        fs::create_dir_all(addons.join("Foo")).expect("dir");
        fs::write(
            addons.join("Foo").join("Foo.toc"),
            "## Title: Foo\n## Notes: Does things\n## Category: Bags\n",
        )
        .expect("toc");
        let scanned = scan_addons_dir(&addons, GameFlavor::Retail).expect("scan");
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].notes.as_deref(), Some("Does things"));
        assert_eq!(scanned[0].category.as_deref(), Some("Bags"));
    }

    #[test]
    fn read_addon_toc_fields_reads_what_the_game_reads() {
        let temp = tempdir().expect("temp dir");
        let folder = temp.path().join("Foo");
        fs::create_dir_all(&folder).expect("folder");
        fs::write(
            folder.join("Foo.toc"),
            "## Interface: 110005\n## Title: Foo\n## Category: Bags\n# LemonUp: ## Notes: Old\n",
        )
        .expect("toc");
        let fields = read_addon_toc_fields(temp.path(), "Foo", GameFlavor::Retail)
            .expect("read")
            .expect("fields");
        assert_eq!(fields.title.as_deref(), Some("Foo"));
        assert_eq!(fields.category.as_deref(), Some("Bags"));
        assert_eq!(fields.notes, None);
    }

    /// The Title lines of ElvUI's root TOC (v16.01), one per game version.
    const ELVUI_TITLES: &str = "## Interface: 11509, 16001, 20506, 38002, 50504, 120100, 120105\r\n## Notes: User Interface Replacement\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bMists|r [AllowLoadGameType mists]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bMidnight|r [AllowLoadGameType standard]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bForever|r [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bWrath|r [AllowLoadGameType wrath]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bTBC|r [AllowLoadGameType tbc]\r\n## Title: |cff1784d1ElvUI|r |cfd9b9b9bClassic|r [AllowLoadGameType vanilla]\r\n";

    fn scan_one(flavor: GameFlavor, client: &str, toc: &str) -> ScannedAddon {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join(client).join("Interface").join("AddOns");
        fs::create_dir_all(addons.join("ElvUI")).expect("dir");
        fs::write(addons.join("ElvUI").join("ElvUI.toc"), toc).expect("toc");
        let mut scanned = scan_addons_dir(&addons, flavor).expect("scan");
        assert_eq!(scanned.len(), 1);
        scanned.remove(0)
    }

    #[test]
    fn titles_for_another_game_version_are_skipped_and_their_conditions_never_shown() {
        let forever = scan_one(GameFlavor::Forever, "_forever_", ELVUI_TITLES);
        assert_eq!(forever.name, "ElvUI Forever");
        assert_eq!(
            forever.toc_title.as_deref(),
            Some("|cff1784d1ElvUI|r |cfd9b9b9bForever|r")
        );
        assert_eq!(forever.notes.as_deref(), Some("User Interface Replacement"));

        let retail = scan_one(GameFlavor::Retail, "_retail_", ELVUI_TITLES);
        assert_eq!(retail.name, "ElvUI Midnight");
        assert_eq!(
            retail.toc_title.as_deref(),
            Some("|cff1784d1ElvUI|r |cfd9b9b9bMidnight|r")
        );

        let classic = scan_one(GameFlavor::Classic, "_classic_era_", ELVUI_TITLES);
        assert_eq!(classic.name, "ElvUI Classic");

        // Without a known game version only lines without a game version condition count,
        // and ElvUI has none, so its folder name is the name.
        let unknown = parse_toc_content(ELVUI_TITLES, "ElvUI", None);
        assert_eq!(unknown.title, "ElvUI");
        assert_eq!(unknown.toc_title, None);
        assert_eq!(unknown.notes.as_deref(), Some("User Interface Replacement"));

        for addon in [&forever, &retail, &classic] {
            assert!(!addon.name.contains('['), "{}", addon.name);
            assert!(
                !addon.toc_title.as_deref().unwrap_or_default().contains('['),
                "{:?}",
                addon.toc_title
            );
        }
    }

    #[test]
    fn an_unconditional_title_and_conditional_ones_follow_the_game() {
        let toc = "## Title: Foo\n## Title: Foo Classic [AllowLoadGameType classic]\n## Title: Foo Forever [ExcludeLoadGameType standard]\n";
        // Retail skips both conditional lines: the second is for the Classic family, the third
        // excludes Retail.
        let retail = scan_one(GameFlavor::Retail, "_retail_", toc);
        assert_eq!(retail.name, "Foo");
        assert_eq!(retail.toc_title.as_deref(), Some("Foo"));
        // Forever is in the Mainline family: the Classic line is skipped and the last line that
        // applies wins, while the name keeps the first line that applies.
        let forever = scan_one(GameFlavor::Forever, "_forever_", toc);
        assert_eq!(forever.name, "Foo");
        assert_eq!(forever.toc_title.as_deref(), Some("Foo Forever"));
        // Without a known game version only the unconditional line counts.
        let unknown = parse_toc_content(toc, "Fallback", None);
        assert_eq!(unknown.title, "Foo");
        assert_eq!(unknown.toc_title.as_deref(), Some("Foo"));

        // A Title that only applies elsewhere comes first: it is skipped for the name too.
        let later = scan_one(
            GameFlavor::Retail,
            "_retail_",
            "## Title: Bar Wrath [AllowLoadGameType wrath]\n## Title: Bar\n",
        );
        assert_eq!(later.name, "Bar");
    }

    #[test]
    fn version_interface_and_dependencies_use_the_last_line_for_the_game() {
        let toc = "## Title: Foo\n## Title: Foo Forever [AllowLoadGameType camelot]\n## Interface: 120100\n## Interface: 16001 [AllowLoadGameType camelot]\n## Version: 1.0\n## Version: 2.0 [AllowLoadGameType camelot]\n## Dependencies: LibA\n## Dependencies: LibB [AllowLoadGameType camelot]\n";
        let forever = parse_toc_content(toc, "Foo", Some(GameFlavor::Forever));
        // The name keeps the first line that applies, so names stay stable.
        assert_eq!(forever.title, "Foo");
        assert_eq!(forever.version.as_deref(), Some("2.0"));
        assert_eq!(forever.interface.as_deref(), Some("16001"));
        assert_eq!(forever.required_deps, vec!["LibB".to_string()]);

        let retail = parse_toc_content(toc, "Foo", Some(GameFlavor::Retail));
        assert_eq!(retail.version.as_deref(), Some("1.0"));
        assert_eq!(retail.interface.as_deref(), Some("120100"));
        assert_eq!(retail.required_deps, vec!["LibA".to_string()]);
    }

    #[test]
    fn notes_and_category_with_conditions_follow_the_game() {
        let toc = "## Title: Foo\n## Notes: Everywhere\n## Notes: On Forever [AllowLoadGameType forever]\n## Category: Combat [AllowLoadGameType camelot]\n## Category: Bags [AllowLoadGameType standard]\n";
        let forever = scan_one(GameFlavor::Forever, "_forever_", toc);
        assert_eq!(forever.notes.as_deref(), Some("On Forever"));
        assert_eq!(forever.category.as_deref(), Some("Combat"));
        let retail = scan_one(GameFlavor::Retail, "_retail_", toc);
        assert_eq!(retail.notes.as_deref(), Some("Everywhere"));
        assert_eq!(retail.category.as_deref(), Some("Bags"));

        let temp = tempdir().expect("tempdir");
        let addons = temp
            .path()
            .join("_forever_")
            .join("Interface")
            .join("AddOns");
        fs::create_dir_all(addons.join("ElvUI")).expect("dir");
        fs::write(addons.join("ElvUI").join("ElvUI.toc"), ELVUI_TITLES).expect("toc");
        let fields = read_addon_toc_fields(&addons, "ElvUI", GameFlavor::Forever)
            .expect("read")
            .expect("fields");
        assert_eq!(
            fields.title.as_deref(),
            Some("|cff1784d1ElvUI|r |cfd9b9b9bForever|r")
        );
    }

    #[test]
    fn ignores_localized_metadata_without_utf8_boundary_panics() {
        let parsed = parse_toc_content(
            "## Notes-ruRU: Собирает информацию об ошибках и ловит их в мешок.\n## Title: BugSack\n## Author: Rabbit\n## OptionalDeps: Masque",
            "Fallback",
            None,
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
    fn forever_prefers_forever_then_camelot_then_mainline_then_base() {
        let all = [
            "Addon.toc",
            "Addon_Mainline.toc",
            "Addon_Camelot.toc",
            "Addon_Forever.toc",
            "Addon_Vanilla.toc",
        ];
        let select = |files: &[&str]| {
            select_toc_file(
                "Addon",
                &files.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
                GameFlavor::Forever,
            )
            .expect("select toc")
        };

        let selected = select(&all);
        assert_eq!(selected.selected, "Addon_Forever.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Exact);

        let selected = select(&["Addon.toc", "Addon-Forever.toc", "Addon_Camelot.toc"]);
        assert_eq!(selected.selected, "Addon-Forever.toc");

        let selected = select(&["Addon.toc", "Addon_Mainline.toc", "Addon_Camelot.toc"]);
        assert_eq!(selected.selected, "Addon_Camelot.toc");

        let selected = select(&["Addon.toc", "Addon_Mainline.toc", "Addon_Vanilla.toc"]);
        assert_eq!(selected.selected, "Addon_Mainline.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Exact);

        let selected = select(&["Addon.toc", "Addon_Vanilla.toc"]);
        assert_eq!(selected.selected, "Addon.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Fallback);
    }

    #[test]
    fn retail_never_selects_forever_or_camelot_toc() {
        let selected = select_toc_file(
            "Addon",
            &[
                "Addon_Camelot.toc".to_string(),
                "Addon_Forever.toc".to_string(),
                "Addon_Vanilla.toc".to_string(),
            ],
            GameFlavor::Retail,
        )
        .expect("select toc");
        assert_eq!(selected.selected, "Addon_Vanilla.toc");

        let selected = select_toc_file(
            "Addon",
            &["Addon-Forever.toc".to_string(), "Addon.toc".to_string()],
            GameFlavor::Retail,
        )
        .expect("select toc");
        assert_eq!(selected.selected, "Addon.toc");

        assert!(
            select_toc_file(
                "Addon",
                &["Addon_Forever.toc".to_string()],
                GameFlavor::Retail
            )
            .is_err()
        );
    }

    #[test]
    fn forever_honors_dash_separator_variants() {
        let select = |files: &[&str]| {
            select_toc_file(
                "Addon",
                &files.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
                GameFlavor::Forever,
            )
            .expect("select toc")
        };

        let selected = select(&["Addon.toc", "Addon-Mainline.toc", "Addon-Camelot.toc"]);
        assert_eq!(selected.selected, "Addon-Camelot.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Exact);

        let selected = select(&["Addon.toc", "Addon-Mainline.toc", "Addon_Vanilla.toc"]);
        assert_eq!(selected.selected, "Addon-Mainline.toc");
        assert_eq!(selected.confidence, TocSelectionConfidence::Exact);

        let selected = select(&["Addon-Mainline.toc", "Addon_Camelot.toc"]);
        assert_eq!(selected.selected, "Addon_Camelot.toc");

        let selected = select(&["addon-camelot.toc", "ADDON_FOREVER.toc"]);
        assert_eq!(selected.selected, "ADDON_FOREVER.toc");
    }

    #[test]
    fn retail_scan_keeps_folder_named_like_a_flavor_suffix() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        fs::create_dir_all(addons.join("Foo_Camelot")).expect("create camelot-named addon");
        fs::write(
            addons.join("Foo_Camelot").join("Foo_Camelot.toc"),
            "## Title: Foo Camelot\n## Interface: 120105",
        )
        .expect("write base toc");
        fs::create_dir_all(addons.join("Foo")).expect("create flavor-only addon");
        fs::write(
            addons.join("Foo").join("Foo_Camelot.toc"),
            "## Title: Foo\n## Interface: 16001",
        )
        .expect("write flavor toc");

        let retail = scan_addons_dir(addons, GameFlavor::Retail).expect("scan retail");
        assert_eq!(retail.len(), 1);
        assert_eq!(retail[0].folder, "Foo_Camelot");
        assert_eq!(retail[0].name, "Foo Camelot");

        let selected = select_toc_file(
            "Foo_Camelot",
            &["Foo_Camelot.toc".to_string()],
            GameFlavor::Retail,
        )
        .expect("select toc");
        assert_eq!(selected.selected, "Foo_Camelot.toc");

        assert!(
            select_toc_file("Foo", &["Foo_Camelot.toc".to_string()], GameFlavor::Retail).is_err()
        );

        // Forever loads `Foo_Camelot.toc` for folder `Foo`; `Foo_Camelot` then collapses under it.
        let forever = scan_addons_dir(addons, GameFlavor::Forever).expect("scan forever");
        assert_eq!(forever.len(), 1);
        assert_eq!(forever[0].folder, "Foo");
    }

    #[test]
    fn retail_scan_skips_forever_only_addons() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        fs::create_dir_all(addons.join("ForeverOnly")).expect("create forever addon");
        fs::write(
            addons.join("ForeverOnly").join("ForeverOnly_Forever.toc"),
            "## Title: Forever Only\n## Interface: 16001",
        )
        .expect("write forever toc");
        fs::create_dir_all(addons.join("Shared")).expect("create shared addon");
        fs::write(
            addons.join("Shared").join("Shared.toc"),
            "## Title: Shared\n## Interface: 120105",
        )
        .expect("write base toc");
        fs::write(
            addons.join("Shared").join("Shared_Forever.toc"),
            "## Title: Shared Forever\n## Interface: 16001",
        )
        .expect("write forever toc");

        let retail = scan_addons_dir(addons, GameFlavor::Retail).expect("scan retail");
        assert_eq!(retail.len(), 1);
        assert_eq!(retail[0].name, "Shared");
        assert_eq!(retail[0].interface.as_deref(), Some("120105"));

        let forever = scan_addons_dir(addons, GameFlavor::Forever).expect("scan forever");
        assert_eq!(forever.len(), 2);
        assert!(
            forever
                .iter()
                .all(|addon| addon.flavor == GameFlavor::Forever)
        );
        let shared = forever
            .iter()
            .find(|addon| addon.folder == "Shared")
            .expect("shared");
        assert_eq!(shared.name, "Shared Forever");
        assert_eq!(shared.interface.as_deref(), Some("16001"));
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
                notes: None,
                category: None,
                toc_title: None,
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
                notes: None,
                category: None,
                toc_title: None,
            },
        ];

        let collapsed = collapse_owned_folders(addons, &GroupingGuard::default());
        assert_eq!(collapsed.len(), 2);
    }

    fn write_addon(addons: &std::path::Path, folder: &str) {
        let folder_path = addons.join(folder);
        fs::create_dir_all(&folder_path).expect("create addon dir");
        fs::write(
            folder_path.join(format!("{folder}.toc")),
            format!("## Title: {folder}"),
        )
        .expect("write toc");
    }

    fn owned_names(addon: &ScannedAddon) -> Vec<&str> {
        addon
            .owned_folders
            .iter()
            .map(|owned| owned.name.as_str())
            .collect()
    }

    #[test]
    fn tracked_scan_keeps_separately_tracked_and_unlisted_siblings_top_level() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        for folder in [
            "ElvUI",
            "ElvUI_Libraries",
            "ElvUI_Options",
            "ElvUI_FCT",
            "ElvUI_WindTools",
            "ElvUI_Extra",
        ] {
            write_addon(addons, folder);
        }

        let mut elvui = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Tukui);
        elvui.set_managed_owned_folders(vec![
            OwnedFolder {
                name: "ElvUI_Libraries".to_string(),
            },
            OwnedFolder {
                name: "ElvUI_Options".to_string(),
            },
        ]);
        let fct = AddonRecord::new("ElvUI_FCT", "ElvUI_FCT", SourceKind::GitHub);
        let tracked = vec![elvui, fct];

        let scanned = scan_addons_dir_with_tracked(addons, GameFlavor::Retail, &tracked)
            .expect("scan addons");
        let folders = scanned
            .iter()
            .map(|addon| addon.folder.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            folders,
            vec!["ElvUI", "ElvUI_Extra", "ElvUI_FCT", "ElvUI_WindTools"]
        );
        let parent = scanned
            .iter()
            .find(|addon| addon.folder == "ElvUI")
            .expect("elvui row");
        assert_eq!(
            owned_names(parent),
            vec!["ElvUI_Libraries", "ElvUI_Options"]
        );

        // Without tracked knowledge the heuristic still folds every sibling (archive scans).
        let unguarded = scan_addons_dir(addons, GameFlavor::Retail).expect("scan addons");
        assert_eq!(unguarded.len(), 1);
        assert_eq!(unguarded[0].owned_folders.len(), 5);
    }

    #[test]
    fn tracked_scan_still_groups_manual_siblings_without_authoritative_parent() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_addon(addons, "Foo");
        write_addon(addons, "Foo_Options");

        let tracked = vec![
            AddonRecord::new("Foo", "Foo", SourceKind::Manual),
            AddonRecord::new("Foo_Options", "Foo_Options", SourceKind::Manual),
        ];
        let scanned = scan_addons_dir_with_tracked(addons, GameFlavor::Retail, &tracked)
            .expect("scan addons");

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].folder, "Foo");
        assert_eq!(owned_names(&scanned[0]), vec!["Foo_Options"]);
    }

    #[test]
    fn grouping_guard_vetoes_standalone_and_unlisted_children() {
        let mut parent = AddonRecord::new("Parent", "Parent", SourceKind::Wago);
        parent.set_managed_owned_folders(vec![OwnedFolder {
            name: "Parent_Options".to_string(),
        }]);
        let standalone = AddonRecord::new("Other_Plugin", "Other_Plugin", SourceKind::GitHub);
        let guard = GroupingGuard::from_tracked(&[parent, standalone]);

        assert!(guard.may_group("Parent", "parent_options"));
        assert!(!guard.may_group("Parent", "Parent_Extra"));
        assert!(!guard.may_group("Other", "Other_Plugin"));
        assert!(guard.may_group("Other", "Other_Untracked"));
    }

    #[test]
    fn grouping_guard_keeps_the_owned_folders_a_newer_version_stored() {
        let mut newer = AddonRecord::new("Owned", "Owned", SourceKind::Manual);
        newer.owned_folders = vec![OwnedFolder {
            name: "Owned_Child".to_string(),
        }];
        newer.unrecognized = Some(crate::domain::UnrecognizedValues {
            source: "manual".to_string(),
            ownership_source: Some("exclusive".to_string()),
            ..Default::default()
        });
        let guard = GroupingGuard::from_tracked(&[newer]);

        assert!(guard.may_group("Owned", "Owned_Child"));
        assert!(!guard.may_group("Owned", "Owned_Extra"));
    }

    #[cfg(unix)]
    #[test]
    fn scan_follows_symlinked_addon_folders() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path().join("AddOns");
        let dev = temp.path().join("dev");
        fs::create_dir_all(&addons).expect("create addons");
        write_addon(&dev, "MyAddon");
        std::os::unix::fs::symlink(dev.join("MyAddon"), addons.join("MyAddon"))
            .expect("symlink addon");

        let scanned = scan_addons_dir(&addons, GameFlavor::Retail).expect("scan addons");

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].folder, "MyAddon");
        assert_eq!(scanned[0].name, "MyAddon");
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_broken_and_looping_symlinks() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_addon(addons, "RealAddon");
        std::os::unix::fs::symlink(addons.join("Missing"), addons.join("Broken"))
            .expect("broken symlink");
        std::os::unix::fs::symlink(addons.join("Loop"), addons.join("Loop"))
            .expect("looping symlink");

        let report = scan_addons_dir_report(addons, GameFlavor::Retail, &[]).expect("scan addons");

        assert_eq!(report.addons.len(), 1);
        assert_eq!(report.addons[0].folder, "RealAddon");
        // Their metadata cannot be read, so they are reported as skipped (never dropped).
        assert_eq!(
            report.skipped_folders,
            vec!["Broken".to_string(), "Loop".to_string()]
        );
    }
    fn write_toc_bytes(addons: &std::path::Path, folder: &str, bytes: &[u8]) {
        let dir = addons.join(folder);
        fs::create_dir_all(&dir).expect("create addon dir");
        fs::write(dir.join(format!("{folder}.toc")), bytes).expect("write toc");
    }

    #[test]
    fn scan_decodes_non_utf8_toc_lossily_instead_of_failing() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_addon(addons, "GoodAddon");
        // Latin-1 `é` (0xE9) is not valid UTF-8.
        write_toc_bytes(
            addons,
            "Latin",
            b"## Title: Caf\xE9 Tools\n## Version: 1.2.3\n## Interface: 110002\n",
        );

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan survives");

        assert_eq!(scanned.len(), 2);
        let latin = scanned
            .iter()
            .find(|addon| addon.folder == "Latin")
            .expect("latin addon scanned");
        assert_eq!(latin.name, "Caf\u{fffd} Tools");
        assert_eq!(latin.version.as_deref(), Some("1.2.3"));
        assert_eq!(latin.interface.as_deref(), Some("110002"));
    }

    #[test]
    fn scan_strips_utf8_bom_so_the_first_toc_line_parses() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_toc_bytes(
            addons,
            "Bom",
            b"\xEF\xBB\xBF## Interface: 110002\n## Title: Bom Addon\n## Version: 2.0\n",
        );

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan addons");

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].interface.as_deref(), Some("110002"));
        assert_eq!(scanned[0].name, "Bom Addon");

        // parse_toc_content is BOM-safe for callers that already hold a string.
        let parsed = parse_toc_content("\u{feff}## Interface: 110002\n", "Fallback", None);
        assert_eq!(parsed.interface.as_deref(), Some("110002"));
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_only_the_folder_whose_toc_cannot_be_read() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_addon(addons, "GoodAddon");
        write_addon(addons, "Locked");
        let locked_toc = addons.join("Locked").join("Locked.toc");
        fs::set_permissions(&locked_toc, fs::Permissions::from_mode(0o000)).expect("lock toc");
        // Running as root ignores file modes; nothing to prove then.
        if fs::read(&locked_toc).is_ok() {
            return;
        }

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan survives");

        fs::set_permissions(&locked_toc, fs::Permissions::from_mode(0o644)).expect("unlock toc");
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].folder, "GoodAddon");
    }

    #[test]
    fn scanned_dev_clone_with_git_dir_is_manual_with_commit_metadata() {
        let temp = tempdir().expect("tempdir");
        let addons = temp.path();
        write_addon(addons, "DevClone");
        let git_dir = addons.join("DevClone").join(".git");
        fs::create_dir_all(&git_dir).expect("create .git");
        fs::write(
            git_dir.join("HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .expect("write HEAD");

        let scanned = scan_addons_dir(addons, GameFlavor::Retail).expect("scan addons");

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].source, SourceKind::Manual);
        assert_eq!(
            scanned[0].git_commit.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
    }
}
