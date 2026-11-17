//! "Find your addons online": works out where Not linked addons come from, and links them.
//!
//! Nothing here touches the network. The app reads the AddOns folder with
//! [`FindInputs::read`], sends [`FindInputs::wago_lookups`] to Wago.io and fetches the
//! WoWInterface and TukUI lists, then hands what came back to [`match_addons`]. That produces
//! one [`FindResult`] per addon with a [`FindLabel`]; only `Exact` results are
//! [`FindResult::preselected`]. [`link_found_addons`] links the chosen [`FoundAddon`]s without
//! downloading anything: it only writes rows, in one transaction.
//!
//! The rules that keep people's addons safe:
//! - A folder a linked addon (or a row from a newer LemonUp) owns is never taken; a result that
//!   needs one is not linkable and says why. Folder names compare without case.
//! - Symlinked folders and folders with `.git` are developer copies: never sent, never matched.
//! - A source that failed leaves the addons only it could find `NotChecked`, never `NotFound`.
//! - Two candidates for one folder, or two folders claiming one source project, are
//!   `ChooseOne`: the person picks, nothing is preselected.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use time::OffsetDateTime;

use crate::domain::{AddonRecord, GameFlavor, OwnedFolder, SourceKind};
use crate::error::{LemonupError, Result};
use crate::fingerprint::{FolderFingerprint, folder_fingerprint};
use crate::scan::{
    ScannedAddon, UPDATE_STAGING_PREFIX, read_selected_toc, scan_addon_folder, toc_field,
};
use crate::storage::StateDatabase;

/// The sources "find" asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FindSource {
    Wago,
    WowInterface,
    Tukui,
}

impl FindSource {
    pub const ALL: [Self; 3] = [Self::Wago, Self::WowInterface, Self::Tukui];

    /// The source's name as people know it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wago => "Wago.io",
            Self::WowInterface => "WoWInterface",
            Self::Tukui => "TukUI",
        }
    }

    pub fn source_kind(self) -> SourceKind {
        match self {
            Self::Wago => SourceKind::Wago,
            Self::WowInterface => SourceKind::WowInterface,
            Self::Tukui => SourceKind::Tukui,
        }
    }
}

/// Why a folder counts as a developer copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeveloperCopy {
    /// The folder is a symlink.
    Symlink,
    /// The folder has a `.git` entry.
    GitCheckout,
}

/// The source IDs an addon's TOC names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TocIds {
    /// `X-Wago-ID`
    pub wago: Option<String>,
    /// `X-WoWI-ID`
    pub wowinterface: Option<String>,
    /// `X-Curse-Project-ID`
    pub curseforge: Option<String>,
    /// `X-Tukui-ProjectID`
    pub tukui: Option<String>,
}

/// One top-level folder of the AddOns folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFolder {
    pub folder: String,
    pub developer_copy: Option<DeveloperCopy>,
    /// `None` for a developer copy or a folder whose files could not be read.
    pub fingerprint: Option<FolderFingerprint>,
    /// The TOC Title and Version the game reads for this game version.
    pub title: Option<String>,
    pub version: Option<String>,
    pub ids: TocIds,
}

impl LocalFolder {
    /// The fingerprint to send to Wago.io: only for a folder that has files to hash.
    fn hash(&self) -> Option<&str> {
        self.fingerprint
            .as_ref()
            .filter(|fingerprint| fingerprint.files > 0 && self.developer_copy.is_none())
            .map(|fingerprint| fingerprint.hash.as_str())
    }
}

/// What "find" knows about the AddOns folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindInputs {
    pub flavor: GameFlavor,
    pub folders: Vec<LocalFolder>,
}

/// One folder for the Wago.io `_match` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WagoLookup {
    pub folder: String,
    pub hash: String,
    pub wago_id: Option<String>,
}

impl FindInputs {
    /// Reads every top-level folder of `addons_dir`: real directories are fingerprinted and
    /// their TOC read for `flavor`; symlinks and folders with `.git` are developer copies;
    /// files (and LemonUp's staging folders) are left out. A folder whose files cannot be read
    /// is kept without a fingerprint, so it is never mistaken for a missing one.
    pub fn read(addons_dir: &Path, flavor: GameFlavor) -> Result<Self> {
        let mut folders = Vec::new();
        for entry in fs::read_dir(addons_dir)? {
            let entry = entry?;
            let folder = entry.file_name().to_string_lossy().into_owned();
            if folder.starts_with('.') || folder.starts_with(UPDATE_STAGING_PREFIX) {
                continue;
            }
            let path = entry.path();
            let Ok(link_metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            let developer_copy = if link_metadata.file_type().is_symlink() {
                if !path.is_dir() {
                    continue;
                }
                Some(DeveloperCopy::Symlink)
            } else if !link_metadata.is_dir() {
                continue;
            } else if path.join(".git").exists() {
                Some(DeveloperCopy::GitCheckout)
            } else {
                None
            };
            let fingerprint = if developer_copy.is_some() {
                None
            } else {
                match folder_fingerprint(addons_dir, &folder) {
                    Ok(fingerprint) => Some(fingerprint),
                    Err(error) => {
                        tracing::debug!(folder = %folder, %error, "could not fingerprint folder");
                        None
                    }
                }
            };
            let toc = read_selected_toc(&path, &folder, flavor).unwrap_or_else(|error| {
                tracing::debug!(folder = %folder, %error, "could not read the folder's TOC");
                None
            });
            let field = |key: &str| toc.as_deref().and_then(|text| toc_field(text, key, flavor));
            let title = toc
                .as_deref()
                .map(|text| crate::scan::parse_toc_content(text, &folder, Some(flavor)).title);
            folders.push(LocalFolder {
                version: field("Version"),
                ids: TocIds {
                    wago: field("X-Wago-ID"),
                    wowinterface: field("X-WoWI-ID"),
                    curseforge: field("X-Curse-Project-ID"),
                    tukui: field("X-Tukui-ProjectID"),
                },
                title,
                folder,
                developer_copy,
                fingerprint,
            });
        }
        folders.sort_by_key(|folder| folder.folder.to_lowercase());
        Ok(Self { flavor, folders })
    }

    /// The folder named `folder`, without case.
    pub fn get(&self, folder: &str) -> Option<&LocalFolder> {
        let key = folder.to_lowercase();
        self.folders
            .iter()
            .find(|local| local.folder.to_lowercase() == key)
    }

    /// The folders whose names and hashes go to Wago.io for `scope` (see [`match_addons`]):
    /// those of the Not linked addons looked up, without developer copies and folders with
    /// nothing to hash.
    pub fn wago_lookups(&self, rows: &[AddonRecord], scope: Option<&[String]>) -> Vec<WagoLookup> {
        let groups = Groups::new(self, rows);
        groups
            .units(scope)
            .flat_map(|unit| unit.folders.iter())
            .filter_map(|folder| self.get(folder))
            .filter_map(|local| {
                local.hash().map(|hash| WagoLookup {
                    folder: local.folder.clone(),
                    hash: hash.to_string(),
                    wago_id: local.ids.wago.clone(),
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// What the sources answered.

/// One project Wago.io matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WagoMatch {
    pub id: String,
    pub name: String,
    pub website_url: Option<String>,
    pub author: Option<String>,
    /// The project's folders and the hash Wago.io has for each.
    pub modules: Vec<(String, String)>,
    /// The release whose files matched.
    pub matched_version: Option<String>,
    /// The newest release (stable, else beta, else alpha), as a check would see it.
    pub latest_version: Option<String>,
    /// The project's WoWInterface ID, when Wago.io knows it.
    pub wowinterface_id: Option<String>,
}

/// One folder of a WoWInterface file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WowiFolder {
    pub folder: String,
    pub version: Option<String>,
    /// The interface numbers the folder's TOC lists, separated by spaces.
    pub api_version: Option<String>,
}

/// One entry of the WoWInterface file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WowiFile {
    pub id: String,
    pub title: String,
    pub version: String,
    pub author: Option<String>,
    pub folders: Vec<WowiFolder>,
}

/// One TukUI package (ElvUI or Tukui) that supports this game version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TukuiPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub web_url: String,
    pub author: Option<String>,
    pub directories: Vec<String>,
}

/// What one source answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAnswer<T> {
    Found(Vec<T>),
    /// Not asked (Wago.io without a key).
    Skipped,
    /// Asked, but the answer never came or made no sense.
    Failed,
    /// Answered, but part of the answer made no sense: what could be read is used, and what
    /// only this source could find stays "Not checked", as for `Failed`.
    Incomplete(Vec<T>),
}

impl<T> SourceAnswer<T> {
    fn items(&self) -> &[T] {
        match self {
            Self::Found(items) | Self::Incomplete(items) => items,
            Self::Skipped | Self::Failed => &[],
        }
    }

    fn failed(&self) -> bool {
        matches!(self, Self::Failed | Self::Incomplete(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLookups {
    pub wago: SourceAnswer<WagoMatch>,
    pub wowinterface: SourceAnswer<WowiFile>,
    pub tukui: SourceAnswer<TukuiPackage>,
}

impl SourceLookups {
    fn failed(&self) -> Vec<FindSource> {
        let mut failed = Vec::new();
        if self.wago.failed() {
            failed.push(FindSource::Wago);
        }
        if self.wowinterface.failed() {
            failed.push(FindSource::WowInterface);
        }
        if self.tukui.failed() {
            failed.push(FindSource::Tukui);
        }
        failed
    }
}

/// The WoWInterface files for `flavor`'s client, judged by the interface numbers their folders
/// list (`gameVersions` is not reliable):
/// - Forever: a 5-digit number starting with `16` (client 1.60 and later).
/// - Classic: `11xxx`; Cata: `40xxx`.
/// - Retail: a 6-digit number whose major version is the newest one at least
///   [`RETAIL_MAJOR_MIN_FILES`] files list. Older retail builds rarely load after an expansion
///   changes the API, and a fixed number would go stale with the next expansion.
pub fn wowi_files_for_flavor(files: Vec<WowiFile>, flavor: GameFlavor) -> Vec<WowiFile> {
    let numbers = |file: &WowiFile| -> Vec<u32> {
        file.folders
            .iter()
            .filter_map(|folder| folder.api_version.as_deref())
            .flat_map(str::split_whitespace)
            .filter_map(|token| token.parse::<u32>().ok())
            .collect()
    };
    let accepts: Box<dyn Fn(u32) -> bool> = match flavor {
        GameFlavor::Forever => Box::new(|number| (16_000..17_000).contains(&number)),
        GameFlavor::Classic => Box::new(|number| (11_000..12_000).contains(&number)),
        GameFlavor::Cata => Box::new(|number| (40_000..50_000).contains(&number)),
        GameFlavor::Retail => {
            let mut files_per_major = HashMap::<u32, usize>::new();
            for file in &files {
                let majors = numbers(file)
                    .into_iter()
                    .filter(|number| (100_000..1_000_000).contains(number))
                    .map(|number| number / 10_000)
                    .collect::<HashSet<_>>();
                for major in majors {
                    *files_per_major.entry(major).or_default() += 1;
                }
            }
            let newest = files_per_major
                .into_iter()
                .filter(|(_, count)| *count >= RETAIL_MAJOR_MIN_FILES)
                .map(|(major, _)| major)
                .max();
            match newest {
                Some(major) => Box::new(move |number| {
                    (100_000..1_000_000).contains(&number) && number / 10_000 == major
                }),
                None => Box::new(|_| false),
            }
        }
    };
    files
        .into_iter()
        .filter(|file| numbers(file).into_iter().any(&accepts))
        .collect()
}

/// How many WoWInterface files must list a retail major version before it counts as current,
/// so one mislabeled file cannot move the retail filter.
pub const RETAIL_MAJOR_MIN_FILES: usize = 10;

// ---------------------------------------------------------------------------------------------
// Results.

/// How sure a result is. Declared in the order results are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FindLabel {
    /// Wago.io's hashes match every folder of the project, or the TOC Version of every folder
    /// equals the WoWInterface or TukUI version. Preselected.
    Exact,
    /// A TOC ID names the source, or the folders match a WoWInterface or TukUI package whose
    /// version differs.
    Likely,
    /// Wago.io knows the project, but some of its folders differ or are missing.
    Partial,
    /// More than one candidate: two sources for one folder, or two folders naming one project.
    ChooseOne,
    /// A source that could have found it failed.
    NotChecked,
    /// Only a CurseForge project ID; LemonUp can't update from CurseForge.
    CurseForgeOnly,
    NotFound,
    /// A symlink or a folder with `.git`, skipped.
    DeveloperCopy,
}

impl FindLabel {
    /// The words people see.
    pub fn text(self) -> &'static str {
        match self {
            Self::Exact => "Exact",
            Self::Likely => "Likely",
            Self::Partial => "Partial",
            Self::ChooseOne => "Choose one",
            Self::NotChecked => "Not checked",
            Self::CurseForgeOnly => "On CurseForge only",
            Self::NotFound => "Not found",
            Self::DeveloperCopy => "Developer copy, skipped",
        }
    }

    /// The name the CLI prints (`snake_case`).
    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Likely => "likely",
            Self::Partial => "partial",
            Self::ChooseOne => "choose_one",
            Self::NotChecked => "not_checked",
            Self::CurseForgeOnly => "curseforge_only",
            Self::NotFound => "not_found",
            Self::DeveloperCopy => "developer_copy",
        }
    }
}

/// Why a folder can't be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// A linked addon owns it.
    Linked { addon: String },
    /// An addon a newer LemonUp linked owns it.
    NewerVersion { addon: String },
    /// It is a developer copy (symlink or `.git`).
    DeveloperCopy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedFolder {
    pub folder: String,
    pub reason: BlockReason,
}

impl BlockedFolder {
    pub fn describe(&self) -> String {
        match &self.reason {
            BlockReason::Linked { addon } => format!("{} belongs to {addon}", self.folder),
            BlockReason::NewerVersion { addon } => format!(
                "{} belongs to {addon}, which a newer LemonUp linked",
                self.folder
            ),
            BlockReason::DeveloperCopy => format!("{} is a developer copy", self.folder),
        }
    }
}

/// A folder that moves into a result's addon from another Not linked addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMove {
    pub folder: String,
    /// The folder of the Not linked addon it belongs to now.
    pub from: String,
}

impl FolderMove {
    /// Whether the folder is an addon of its own now (its own row, or a folder with none), as
    /// opposed to a folder grouped under another addon.
    pub fn is_own_addon(&self) -> bool {
        key(&self.folder) == key(&self.from)
    }
}

/// One way to link an addon to a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundAddon {
    /// How sure this candidate is on its own (`Exact`, `Likely` or `Partial`).
    pub label: FindLabel,
    pub source: FindSource,
    pub source_id: String,
    pub source_url: String,
    pub name: String,
    pub author: Option<String>,
    /// The folder the linked row is listed under.
    pub parent_folder: String,
    /// Every folder it takes over (all on disk), the parent first.
    pub folders: Vec<String>,
    /// The version installed now, as the source names it.
    pub matched_version: Option<String>,
    /// The source's newest version.
    pub latest_version: Option<String>,
    /// Folders it takes that are not part of the addon now: they move in from other Not linked
    /// addons, folders LemonUp has no row for yet included.
    pub joining: Vec<FolderMove>,
    /// Folders grouped under the parent now that the source doesn't list; they become their own
    /// Not linked addons.
    pub leaving: Vec<String>,
    /// The source's folders that are not on disk.
    pub missing: Vec<String>,
    /// Folders whose files differ from the source's (Wago.io hashes).
    pub differing: Vec<String>,
    /// Folders of the source's package it can't take.
    pub blocked: Vec<BlockedFolder>,
}

impl FoundAddon {
    pub fn is_linkable(&self) -> bool {
        self.blocked.is_empty()
    }

    /// Why linking this is refused, when it is.
    pub fn not_linkable_reason(&self) -> Option<String> {
        if self.blocked.is_empty() {
            return None;
        }
        Some(format!(
            "it needs folders LemonUp won't take: {}",
            self.blocked
                .iter()
                .map(BlockedFolder::describe)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Whether linking it changes which folders belong to which addon.
    pub fn regroups(&self) -> bool {
        !self.joining.is_empty() || !self.leaving.is_empty()
    }
}

/// One line of the results: an addon and what was found for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindResult {
    pub label: FindLabel,
    /// The source's name for it, or the addon's own name when nothing was found.
    pub name: String,
    /// The folder it is listed under.
    pub folder: String,
    /// The folders of the addon as LemonUp groups them now (for results without a candidate).
    pub folders: Vec<String>,
    /// One candidate for `Exact`, `Likely` and `Partial`; two or more for `ChooseOne`; none for
    /// the rest.
    pub candidates: Vec<FoundAddon>,
    /// The `X-Curse-Project-ID` for `CurseForgeOnly`.
    pub curseforge_id: Option<String>,
    /// The failed sources that could have found it, for `NotChecked`.
    pub not_checked_by: Vec<FindSource>,
}

impl FindResult {
    /// The candidate of an `Exact`, `Likely` or `Partial` result.
    pub fn found(&self) -> Option<&FoundAddon> {
        match self.label {
            FindLabel::Exact | FindLabel::Likely | FindLabel::Partial => self.candidates.first(),
            _ => None,
        }
    }

    /// Whether the result starts selected: only a linkable `Exact` one.
    pub fn preselected(&self) -> bool {
        self.label == FindLabel::Exact && self.found().is_some_and(FoundAddon::is_linkable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindReport {
    pub results: Vec<FindResult>,
    /// Sources that were asked but failed.
    pub failed: Vec<FindSource>,
    /// Sources that were not asked (Wago.io without a key).
    pub skipped: Vec<FindSource>,
}

// ---------------------------------------------------------------------------------------------
// Matching.

/// The TukUI folders only TukUI could find.
const TUKUI_FOLDERS: &[&str] = &["elvui", "elvui_libraries", "elvui_options", "tukui"];

fn key(folder: &str) -> String {
    folder.to_lowercase()
}

/// A Not linked addon as LemonUp groups it now (a row, or a folder with no row yet).
#[derive(Debug, Clone)]
struct Unit {
    name: String,
    folder: String,
    /// The parent first, then its owned folders; only folders on disk, and never one another
    /// addon holds.
    folders: Vec<String>,
    /// Folders the row still lists that a linked addon (or a row from a newer LemonUp) holds,
    /// for example after a scan that failed. Linking this addon would take them, so it is
    /// refused until a scan sorts them out.
    held: Vec<String>,
}

/// Every Not linked addon, and who holds each folder.
struct Groups<'a> {
    units: Vec<Unit>,
    /// Folder key -> index into `units`.
    unit_of: HashMap<String, usize>,
    rows: &'a [AddonRecord],
}

impl<'a> Groups<'a> {
    fn new(inputs: &FindInputs, rows: &'a [AddonRecord]) -> Self {
        let mut units = Vec::new();
        let mut unit_of = HashMap::new();
        let present = |folder: &str| inputs.get(folder).map(|local| local.folder.clone());
        let held = |folder: &str| {
            rows.iter()
                .any(|row| row.holds_its_folders() && row.claims_folder(folder))
        };
        for row in rows.iter().filter(|row| !row.holds_its_folders()) {
            let Some(parent) = present(&row.folder).filter(|parent| !held(parent)) else {
                continue;
            };
            let mut folders = vec![parent];
            let mut held_folders = Vec::new();
            for owned in &row.owned_folders {
                if held(&owned.name) {
                    held_folders.push(owned.name.clone());
                } else if let Some(owned) = present(&owned.name)
                    && !folders.iter().any(|known| key(known) == key(&owned))
                {
                    folders.push(owned);
                }
            }
            let index = units.len();
            for folder in &folders {
                unit_of.entry(key(folder)).or_insert(index);
            }
            units.push(Unit {
                name: if row.name.trim().is_empty() {
                    row.folder.clone()
                } else {
                    row.name.clone()
                },
                folder: folders[0].clone(),
                folders,
                held: held_folders,
            });
        }
        for local in &inputs.folders {
            let folder_key = key(&local.folder);
            if unit_of.contains_key(&folder_key)
                || rows.iter().any(|row| row.claims_folder(&local.folder))
            {
                continue;
            }
            unit_of.insert(folder_key, units.len());
            units.push(Unit {
                name: local.title.clone().unwrap_or_else(|| local.folder.clone()),
                folder: local.folder.clone(),
                folders: vec![local.folder.clone()],
                held: Vec::new(),
            });
        }
        Self {
            units,
            unit_of,
            rows,
        }
    }

    /// The Not linked addons looked up: those whose folder (the parent folder of a row) is named
    /// in `scope`, without case, or all of them.
    fn units(&self, scope: Option<&[String]>) -> impl Iterator<Item = &Unit> {
        let wanted = scope.map(|names| names.iter().map(|name| key(name)).collect::<HashSet<_>>());
        self.units.iter().filter(move |unit| {
            wanted
                .as_ref()
                .is_none_or(|wanted| wanted.contains(&key(&unit.folder)))
        })
    }

    fn unit_containing(&self, folder: &str) -> Option<&Unit> {
        self.unit_of
            .get(&key(folder))
            .map(|index| &self.units[*index])
    }

    /// The row (linked here or by a newer LemonUp) that holds `folder`.
    fn holder(&self, folder: &str) -> Option<&'a AddonRecord> {
        self.rows
            .iter()
            .find(|row| row.holds_its_folders() && row.claims_folder(folder))
    }
}

/// Internal: a candidate before it is placed.
struct Claim {
    found: FoundAddon,
    /// Wago.io's cross reference to the same project on WoWInterface.
    wowinterface_id: Option<String>,
}

fn normalize_version(value: &str) -> String {
    let trimmed = value.trim();
    let trimmed = trimmed.trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
    trimmed
        .strip_prefix(['v', 'V'])
        .filter(|rest| rest.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

fn same_version(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => normalize_version(left) == normalize_version(right),
        _ => false,
    }
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn wowinterface_url(id: &str) -> String {
    format!("https://www.wowinterface.com/downloads/info{id}.html")
}

fn wago_url(id: &str) -> String {
    format!("https://addons.wago.io/addons/{id}")
}

fn empty_found(source: FindSource, source_id: &str, source_url: String, name: &str) -> FoundAddon {
    FoundAddon {
        label: FindLabel::Likely,
        source,
        source_id: source_id.to_string(),
        source_url,
        name: name.to_string(),
        author: None,
        parent_folder: String::new(),
        folders: Vec::new(),
        matched_version: None,
        latest_version: None,
        joining: Vec::new(),
        leaving: Vec::new(),
        missing: Vec::new(),
        differing: Vec::new(),
        blocked: Vec::new(),
    }
}

/// Matches what the sources answered against the Not linked addons in `scope` (their folders,
/// the parent folder for a row, compared without case; `None` for all of them, folders without
/// a row included). Linked addons are never looked up, and their folders are never offered.
pub fn match_addons(
    inputs: &FindInputs,
    rows: &[AddonRecord],
    scope: Option<&[String]>,
    lookups: &SourceLookups,
) -> FindReport {
    let groups = Groups::new(inputs, rows);
    let units = groups.units(scope).collect::<Vec<_>>();
    let in_scope = units
        .iter()
        .flat_map(|unit| unit.folders.iter().map(|folder| key(folder)))
        .collect::<HashSet<_>>();
    let present = |folder: &str| inputs.get(folder).map(|local| local.folder.clone());

    let mut claims = Vec::<Claim>::new();

    for project in lookups.wago.items() {
        let mut found = empty_found(
            FindSource::Wago,
            &project.id,
            project
                .website_url
                .clone()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| wago_url(&project.id)),
            &project.name,
        );
        let mut matched = 0;
        for (folder, hash) in &project.modules {
            match inputs.get(folder) {
                Some(local) => {
                    found.folders.push(local.folder.clone());
                    if local
                        .hash()
                        .is_some_and(|own| own.eq_ignore_ascii_case(hash))
                    {
                        matched += 1;
                    } else {
                        found.differing.push(local.folder.clone());
                    }
                }
                None => found.missing.push(folder.clone()),
            }
        }
        if !found
            .folders
            .iter()
            .any(|folder| in_scope.contains(&key(folder)))
        {
            continue;
        }
        found.label = if matched == 0 {
            FindLabel::Likely
        } else if found.differing.is_empty() && found.missing.is_empty() {
            FindLabel::Exact
        } else {
            FindLabel::Partial
        };
        found.author = project.author.clone();
        found.matched_version = project.matched_version.clone();
        found.latest_version = project.latest_version.clone();
        claims.push(Claim {
            found,
            wowinterface_id: project.wowinterface_id.clone(),
        });
    }

    for file in lookups.wowinterface.items() {
        if file.folders.is_empty() {
            continue;
        }
        let folders = file
            .folders
            .iter()
            .map(|folder| present(&folder.folder))
            .collect::<Option<Vec<_>>>();
        let Some(folders) = folders else {
            continue;
        };
        if !folders.iter().any(|folder| in_scope.contains(&key(folder))) {
            continue;
        }
        let versions_match = file.folders.iter().all(|folder| {
            same_version(
                inputs
                    .get(&folder.folder)
                    .and_then(|local| local.version.as_deref()),
                folder.version.as_deref(),
            )
        });
        let mut found = empty_found(
            FindSource::WowInterface,
            &file.id,
            wowinterface_url(&file.id),
            &file.title,
        );
        found.label = if versions_match {
            FindLabel::Exact
        } else {
            FindLabel::Likely
        };
        found.folders = folders;
        found.author = file.author.clone();
        found.matched_version = versions_match.then(|| file.version.clone());
        found.latest_version = Some(file.version.clone());
        claims.push(Claim {
            found,
            wowinterface_id: None,
        });
    }

    for package in lookups.tukui.items() {
        let folders = package
            .directories
            .iter()
            .map(|folder| present(folder))
            .collect::<Option<Vec<_>>>();
        let Some(folders) = folders.filter(|folders| !folders.is_empty()) else {
            continue;
        };
        if !folders.iter().any(|folder| in_scope.contains(&key(folder))) {
            continue;
        }
        // Every folder of the package must carry the release's version: a current ElvUI with
        // an older ElvUI_Options is not the release, so it is only likely.
        let versions_match = folders.iter().all(|folder| {
            same_version(
                inputs
                    .get(folder)
                    .and_then(|local| local.version.as_deref()),
                Some(&package.version),
            )
        });
        let mut found = empty_found(
            FindSource::Tukui,
            &package.id,
            package.web_url.clone(),
            &package.name,
        );
        found.label = if versions_match {
            FindLabel::Exact
        } else {
            FindLabel::Likely
        };
        found.folders = folders;
        found.author = package.author.clone();
        found.matched_version = versions_match.then(|| package.version.clone());
        found.latest_version = Some(package.version.clone());
        claims.push(Claim {
            found,
            wowinterface_id: None,
        });
    }

    // TOC IDs: a folder whose TOC names a project no candidate links it to.
    let claims_folder = |claims: &[Claim], source: FindSource, id: &str, folder: &str| {
        claims.iter().any(|claim| {
            claim.found.source == source
                && claim.found.source_id == id
                && claim
                    .found
                    .folders
                    .iter()
                    .any(|known| key(known) == key(folder))
        })
    };
    for unit in &units {
        for folder in &unit.folders {
            let Some(local) = inputs
                .get(folder)
                .filter(|local| local.developer_copy.is_none())
            else {
                continue;
            };
            let current = groups
                .unit_containing(folder)
                .filter(|unit| key(&unit.folder) == key(folder))
                .map_or_else(|| vec![local.folder.clone()], |unit| unit.folders.clone());
            let name = local.title.clone().unwrap_or_else(|| local.folder.clone());
            if let Some(id) = &local.ids.wago
                && !claims_folder(&claims, FindSource::Wago, id, folder)
            {
                let mut found = empty_found(FindSource::Wago, id, wago_url(id), &name);
                found.folders = current.clone();
                found.matched_version = local.version.clone();
                claims.push(Claim {
                    found,
                    wowinterface_id: None,
                });
            }
            if let Some(id) = &local.ids.wowinterface
                && !claims_folder(&claims, FindSource::WowInterface, id, folder)
                && let Some(file) = lookups
                    .wowinterface
                    .items()
                    .iter()
                    .find(|file| file.id == *id)
            {
                let mut found = empty_found(
                    FindSource::WowInterface,
                    id,
                    wowinterface_url(id),
                    &file.title,
                );
                for package_folder in &file.folders {
                    match present(&package_folder.folder) {
                        Some(on_disk) => found.folders.push(on_disk),
                        None => found.missing.push(package_folder.folder.clone()),
                    }
                }
                if !found.folders.iter().any(|known| key(known) == key(folder)) {
                    found.folders.push(local.folder.clone());
                }
                found.author = file.author.clone();
                found.matched_version = local.version.clone();
                found.latest_version = Some(file.version.clone());
                claims.push(Claim {
                    found,
                    wowinterface_id: None,
                });
            }
        }
    }

    let claims = merge_same_projects(claims);
    let placed = claims
        .into_iter()
        .map(|claim| place(claim.found, inputs, &groups))
        .collect::<Vec<_>>();

    // Candidates that share a folder or a project are one "Choose one" result.
    let clusters = cluster(&placed);
    let mut placed = placed.into_iter().map(Some).collect::<Vec<_>>();
    let mut results = Vec::new();
    let mut covered = HashSet::new();
    for cluster in clusters {
        let mut candidates = cluster
            .into_iter()
            .filter_map(|index| placed[index].take())
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.source.cmp(&right.source))
                .then_with(|| {
                    left.parent_folder
                        .to_lowercase()
                        .cmp(&right.parent_folder.to_lowercase())
                })
        });
        for candidate in &candidates {
            covered.extend(candidate.folders.iter().map(|folder| key(folder)));
        }
        let first = &candidates[0];
        let label = if candidates.len() > 1 {
            FindLabel::ChooseOne
        } else {
            first.label
        };
        results.push(FindResult {
            label,
            name: first.name.clone(),
            folder: first.parent_folder.clone(),
            folders: first.folders.clone(),
            candidates,
            curseforge_id: None,
            not_checked_by: Vec::new(),
        });
    }

    let failed = lookups.failed();
    for unit in &units {
        if covered.contains(&key(&unit.folder)) {
            continue;
        }
        let local = inputs.get(&unit.folder);
        let folders = unit
            .folders
            .iter()
            .filter(|folder| !covered.contains(&key(folder)))
            .cloned()
            .collect::<Vec<_>>();
        let mut result = FindResult {
            label: FindLabel::NotFound,
            name: unit.name.clone(),
            folder: unit.folder.clone(),
            folders,
            candidates: Vec::new(),
            curseforge_id: None,
            not_checked_by: Vec::new(),
        };
        if local.is_some_and(|local| local.developer_copy.is_some()) {
            result.label = FindLabel::DeveloperCopy;
            results.push(result);
            continue;
        }
        let hashable = unit.folders.iter().any(|folder| {
            inputs
                .get(folder)
                .is_some_and(|local| local.hash().is_some())
        });
        let tukui_folder = unit
            .folders
            .iter()
            .any(|folder| TUKUI_FOLDERS.contains(&key(folder).as_str()));
        result.not_checked_by = failed
            .iter()
            .copied()
            .filter(|source| match source {
                FindSource::Wago => hashable,
                FindSource::WowInterface => true,
                FindSource::Tukui => tukui_folder,
            })
            .collect();
        let ids = unit
            .folders
            .iter()
            .filter_map(|folder| inputs.get(folder))
            .map(|local| &local.ids)
            .collect::<Vec<_>>();
        if !result.not_checked_by.is_empty() {
            result.label = FindLabel::NotChecked;
        } else if let Some(curseforge) = ids.iter().find_map(|ids| ids.curseforge.clone())
            && ids
                .iter()
                .all(|ids| ids.wago.is_none() && ids.wowinterface.is_none() && ids.tukui.is_none())
        {
            result.label = FindLabel::CurseForgeOnly;
            result.curseforge_id = Some(curseforge);
        }
        results.push(result);
    }

    results.sort_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.folder.to_lowercase().cmp(&right.folder.to_lowercase()))
    });
    FindReport {
        results,
        failed,
        skipped: if matches!(lookups.wago, SourceAnswer::Skipped) {
            vec![FindSource::Wago]
        } else {
            Vec::new()
        },
    }
}

/// One project found on two sources is one candidate: Wago.io's cross reference to
/// WoWInterface, or two sources that both match the same folders exactly. The stronger label
/// stays; on a tie, Wago.io, then TukUI, then WoWInterface. When Wago.io's hashes say some
/// files differ, its candidate stays whatever WoWInterface says, so it is never Exact.
fn merge_same_projects(claims: Vec<Claim>) -> Vec<Claim> {
    let folder_set = |found: &FoundAddon| {
        let mut folders = found
            .folders
            .iter()
            .map(|folder| key(folder))
            .collect::<Vec<_>>();
        folders.sort();
        folders
    };
    let priority = |source: FindSource| match source {
        FindSource::Wago => 0,
        FindSource::Tukui => 1,
        FindSource::WowInterface => 2,
    };
    let mut dropped = vec![false; claims.len()];
    for left in 0..claims.len() {
        for right in left + 1..claims.len() {
            if dropped[left] || dropped[right] {
                continue;
            }
            let (a, b) = (&claims[left], &claims[right]);
            if a.found.source == b.found.source {
                continue;
            }
            let shares_folder = a.found.folders.iter().any(|folder| {
                b.found
                    .folders
                    .iter()
                    .any(|other| key(other) == key(folder))
            });
            let cross_referenced = shares_folder
                && [(a, b), (b, a)].iter().any(|(wago, wowi)| {
                    wago.found.source == FindSource::Wago
                        && wowi.found.source == FindSource::WowInterface
                        && wago.wowinterface_id.as_deref() == Some(wowi.found.source_id.as_str())
                });
            let both_exact = a.found.label == FindLabel::Exact
                && b.found.label == FindLabel::Exact
                && folder_set(&a.found) == folder_set(&b.found);
            if !(cross_referenced || both_exact) {
                continue;
            }
            // Wago.io's hashes say the files differ: a version that matches on WoWInterface
            // doesn't make them the same, so the Wago.io candidate (and what differs) stays.
            let wago_differs = |found: &FoundAddon| {
                found.source == FindSource::Wago && !found.differing.is_empty()
            };
            let keep_left =
                if cross_referenced && (wago_differs(&a.found) || wago_differs(&b.found)) {
                    wago_differs(&a.found)
                } else {
                    (a.found.label, priority(a.found.source))
                        <= (b.found.label, priority(b.found.source))
                };
            if keep_left {
                dropped[right] = true;
            } else {
                dropped[left] = true;
            }
        }
    }
    claims
        .into_iter()
        .zip(dropped)
        .filter_map(|(claim, dropped)| (!dropped).then_some(claim))
        .collect()
}

/// Picks the parent folder and works out what moves and what is blocked.
fn place(mut found: FoundAddon, inputs: &FindInputs, groups: &Groups<'_>) -> FoundAddon {
    let is_unit_parent = |folder: &str| {
        groups
            .unit_containing(folder)
            .is_some_and(|unit| key(&unit.folder) == key(folder))
    };
    let mut ranked = found.folders.clone();
    ranked.sort_by_key(|folder| {
        (
            normalize_name(folder) != normalize_name(&found.name),
            !is_unit_parent(folder),
            folder.len(),
            folder.to_lowercase(),
        )
    });
    let parent = ranked.first().cloned().unwrap_or_default();
    found.folders.retain(|folder| key(folder) != key(&parent));
    found.folders.sort_by_key(|folder| folder.to_lowercase());
    found
        .folders
        .dedup_by(|left, right| key(left) == key(right));
    found.folders.insert(0, parent.clone());
    found.parent_folder = parent.clone();
    if found.matched_version.is_none() && found.label != FindLabel::Exact {
        found.matched_version = inputs.get(&parent).and_then(|local| local.version.clone());
    }

    let own_unit = groups
        .unit_containing(&parent)
        .filter(|unit| key(&unit.folder) == key(&parent));
    let own = own_unit
        .map(|unit| unit.folders.clone())
        .unwrap_or_default();
    let own_held = own_unit.map(|unit| unit.held.clone()).unwrap_or_default();
    let contains =
        |folders: &[String], folder: &str| folders.iter().any(|known| key(known) == key(folder));
    // Every folder it takes that isn't the addon's own joins, whether or not LemonUp has a row
    // for it yet (a folder without one is its own Not linked addon). Held folders are blocked
    // below instead.
    found.joining = found
        .folders
        .iter()
        .filter(|folder| !contains(&own, folder) && groups.holder(folder).is_none())
        .map(|folder| FolderMove {
            folder: folder.clone(),
            from: groups
                .unit_containing(folder)
                .map_or_else(|| folder.clone(), |unit| unit.folder.clone()),
        })
        .collect();
    found.leaving = own
        .iter()
        .filter(|folder| !contains(&found.folders, folder))
        .cloned()
        .collect();

    let mut blocked = Vec::new();
    let checked = found
        .folders
        .iter()
        .chain(found.missing.iter())
        .chain(found.leaving.iter())
        .chain(own_held.iter());
    for folder in checked {
        if let Some(owner) = groups.holder(folder) {
            let addon = if owner.name.trim().is_empty() {
                owner.folder.clone()
            } else {
                owner.name.clone()
            };
            blocked.push(BlockedFolder {
                folder: folder.clone(),
                reason: if owner.unrecognized.is_some() {
                    BlockReason::NewerVersion { addon }
                } else {
                    BlockReason::Linked { addon }
                },
            });
        } else if inputs
            .get(folder)
            .is_some_and(|local| local.developer_copy.is_some())
        {
            blocked.push(BlockedFolder {
                folder: folder.clone(),
                reason: BlockReason::DeveloperCopy,
            });
        }
    }
    found.blocked = blocked;
    found
}

/// Groups candidates that share a folder or a project (source and ID).
fn cluster(found: &[FoundAddon]) -> Vec<Vec<usize>> {
    let mut parent = (0..found.len()).collect::<Vec<_>>();
    fn root(parent: &mut [usize], index: usize) -> usize {
        let mut index = index;
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut owner_of = HashMap::<String, usize>::new();
    for (index, candidate) in found.iter().enumerate() {
        let keys = candidate
            .folders
            .iter()
            .map(|folder| format!("folder:{}", key(folder)))
            .chain(std::iter::once(format!(
                "project:{:?}:{}",
                candidate.source, candidate.source_id
            )));
        for item in keys {
            match owner_of.get(&item) {
                Some(&other) => {
                    let (left, right) = (root(&mut parent, index), root(&mut parent, other));
                    parent[left] = right;
                }
                None => {
                    owner_of.insert(item, index);
                }
            }
        }
    }
    let mut clusters = HashMap::<usize, Vec<usize>>::new();
    for index in 0..found.len() {
        let top = root(&mut parent, index);
        clusters.entry(top).or_default().push(index);
    }
    let mut clusters = clusters.into_values().collect::<Vec<_>>();
    clusters.sort();
    clusters
}

// ---------------------------------------------------------------------------------------------
// Linking.

/// The row linking `found` writes, built like an install's: the source, its URL, the version
/// the source names for the files on disk, the source's newest version (so the status is
/// right at once) and the source's folders as managed folders. The rest comes from the parent
/// folder's TOC, as a scan reads it.
///
/// When the source doesn't name the version on disk (a Wago.io match without a release label),
/// the TOC Version is kept but the source's newest version is not: the two may be written
/// differently, so comparing them could claim Update ready falsely. The addon is then "Not
/// checked yet" until the next check.
pub fn link_record(found: &FoundAddon, parent: &ScannedAddon, now: OffsetDateTime) -> AddonRecord {
    let mut record = AddonRecord::new(
        parent.name.clone(),
        parent.folder.clone(),
        found.source.source_kind(),
    );
    record.kind = parent.kind;
    record.flavor = parent.flavor;
    record.version = found
        .matched_version
        .clone()
        .or_else(|| parent.version.clone());
    record.author = found.author.clone().or_else(|| parent.author.clone());
    record.interface = parent.interface.clone();
    record.source_url = Some(found.source_url.clone());
    record.required_deps = parent.required_deps.clone();
    record.optional_deps = parent.optional_deps.clone();
    record.embedded_libs = parent.embedded_libs.clone();
    let latest = found
        .latest_version
        .clone()
        .filter(|_| found.matched_version.is_some());
    record.last_checked_at = latest.as_ref().map(|_| now);
    record.remote_version = latest;
    record.set_managed_owned_folders(
        found
            .folders
            .iter()
            .filter(|folder| key(folder) != key(&parent.folder))
            .map(|name| OwnedFolder { name: name.clone() })
            .collect(),
    );
    record
}

/// Links `picks` without downloading anything: writes one row per pick (absorbing the Not
/// linked rows of its folders, which keeps their .toc overrides), all in one transaction, so
/// any failure leaves the database as it was. No file on disk changes.
///
/// Refuses, before writing, a pick whose folders are not all on disk, picks that share a
/// folder, and (checked again inside the transaction) any pick that needs a folder of a linked
/// addon or of a row from a newer LemonUp, the folders the parent's Not linked row lists
/// included. Folders that leave a group have no row afterwards; the next scan lists them as Not
/// linked addons. Returns the linked rows as the database holds them after the write.
pub fn link_found_addons(
    database: &mut StateDatabase,
    addons_dir: &Path,
    flavor: GameFlavor,
    picks: &[FoundAddon],
) -> Result<Vec<AddonRecord>> {
    let mut seen = HashMap::<String, &str>::new();
    let now = OffsetDateTime::now_utc();
    let mut records = Vec::new();
    for pick in picks {
        if let Some(reason) = pick.not_linkable_reason() {
            return Err(LemonupError::InvalidArgument(format!(
                "{} can't be linked: {reason}",
                pick.name
            )));
        }
        for folder in &pick.folders {
            if !addons_dir.join(folder).is_dir() {
                return Err(LemonupError::InvalidArgument(format!(
                    "{} can't be linked: its folder {folder} is not in the AddOns folder",
                    pick.name
                )));
            }
            if let Some(other) = seen.insert(key(folder), &pick.name) {
                return Err(LemonupError::InvalidArgument(format!(
                    "{} and {other} both need the folder {folder}; link one of them",
                    pick.name
                )));
            }
        }
        let parent = scan_addon_folder(
            &addons_dir.join(&pick.parent_folder),
            &pick.parent_folder,
            flavor,
        )?
        .ok_or_else(|| {
            LemonupError::InvalidArgument(format!(
                "{} can't be linked: {} has no .toc file for this game version",
                pick.name, pick.parent_folder
            ))
        })?;
        records.push(link_record(pick, &parent, now));
    }

    database.record_managed_addons(&records, |rows| {
        for record in &records {
            if let Some(owner) = rows
                .iter()
                .find(|row| row.folder == record.folder && row.holds_its_folders())
            {
                return Err(LemonupError::InvalidArgument(format!(
                    "{} can't be linked: its folder {} belongs to {}",
                    record.name, record.folder, owner.folder
                )));
            }
            // The folders the parent's Not linked row lists are absorbed too, so they count.
            let listed = rows
                .iter()
                .filter(|row| row.folder == record.folder)
                .flat_map(|row| row.owned_folders.iter().map(|owned| &owned.name));
            let folders = std::iter::once(&record.folder)
                .chain(record.owned_folders.iter().map(|owned| &owned.name))
                .chain(listed);
            for folder in folders {
                if let Some(owner) = rows.iter().find(|row| {
                    row.folder != record.folder
                        && row.holds_its_folders()
                        && row.claims_folder(folder)
                }) {
                    return Err(LemonupError::InvalidArgument(format!(
                        "{} can't be linked: its folder {folder} belongs to {}",
                        record.name, owner.folder
                    )));
                }
            }
        }
        Ok(())
    })?;

    // What the database holds now, never what was planned.
    let rows = database.list_addons()?;
    Ok(records
        .iter()
        .filter_map(|record| {
            rows.iter()
                .find(|row| row.folder == record.folder && row.source == record.source)
                .cloned()
        })
        .collect())
}

#[cfg(test)]
mod tests;

pub mod wire;
