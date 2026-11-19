//! Whether the CurseForge app also manages an addon folder.
//!
//! CurseForge writes a `.curseclient` file (a GUID) into each game flavor
//! folder it manages (`<WoW>/_retail_/`, the parent of `Interface/AddOns`),
//! and keeps its record of every game instance and the addons it installed in
//! `AddonGameInstance.json`. A folder counts as managed only when all of these
//! hold: the flavor folder has `.curseclient`; the record has an instance with
//! that GUID (or, without a GUID match, whose install path is that folder);
//! one of that instance's addons lists the folder among its modules; and the
//! user hasn't set that addon to Ignored in CurseForge.
//!
//! Reading never fails: a file that is missing, unreadable or not in the
//! expected shape means "not managed". The one write, [`mark_ignored`], sets
//! `preferenceIsIgnored` to true and nothing else, and refuses whenever the
//! record is not exactly as expected. The `.toc` fields
//! that name CurseForge (`X-Curse-Project-ID`) are written by addon authors
//! and are in every copy, so they say nothing and are never used.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

use crate::domain::AddonRecord;

/// The file CurseForge keeps its game instances in, under its own folder.
const RECORD_PATH: [&str; 4] = [
    "CurseForge",
    "agent",
    "GameInstances",
    "AddonGameInstance.json",
];

/// The file in a flavor folder that holds the GUID of its CurseForge instance.
const CLIENT_FILE: &str = ".curseclient";

/// The places CurseForge's record may be, most likely first.
///
/// Linux (`~/.config/CurseForge/...`) is confirmed. The Windows (`%APPDATA%`
/// and `%LOCALAPPDATA%`) and macOS (`~/Library/Application Support`)
/// locations follow the same layout but are untested.
pub fn record_files() -> Vec<PathBuf> {
    let Some(base) = BaseDirs::new() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for dir in [base.config_dir(), base.data_local_dir()] {
        let file = RECORD_PATH
            .iter()
            .fold(dir.to_path_buf(), |path, part| path.join(part));
        if !files.contains(&file) {
            files.push(file);
        }
    }
    files
}

/// The folders CurseForge manages in one AddOns folder, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurseForgeManaged {
    /// Folder names, lowercased: Windows and macOS ignore case.
    folders: HashSet<String>,
}

impl CurseForgeManaged {
    /// What CurseForge manages in `addons_dir`, from the first of
    /// `record_files` that can be read. Nothing when any part is missing.
    pub fn read(addons_dir: &Path, record_files: &[PathBuf]) -> Self {
        let Some(flavor_dir) = flavor_dir(addons_dir) else {
            return Self::default();
        };
        let Some(guid) = read_client_guid(&flavor_dir) else {
            return Self::default();
        };
        let Some(instances) = record_files.iter().find_map(|file| read_record(file)) else {
            return Self::default();
        };
        Self::from_instances(&instances, &guid, &flavor_dir)
    }

    /// The instance for this flavor folder: one whose GUID and install
    /// path both match it, then one whose GUID does (a copied game folder
    /// keeps its `.curseclient`), then one whose install path does.
    fn from_instances(instances: &[Option<Instance>], guid: &str, flavor_dir: &Path) -> Self {
        let instance = instance_index(instances, guid, flavor_dir)
            .and_then(|index| instances.get(index)?.as_ref());
        let Some(instance) = instance else {
            return Self::default();
        };
        let folders = instance
            .installed_addons
            .iter()
            .flatten()
            .filter_map(|addon| addon.as_ref())
            // Only an explicit "not ignored" counts.
            .filter(|addon| addon.preference_is_ignored == Some(false))
            .filter_map(|addon| addon.installed_file.as_ref()?.modules.as_ref())
            .flatten()
            .filter_map(|module| module.as_ref()?.foldername.as_deref())
            .map(str::to_lowercase)
            .collect();
        Self { folders }
    }

    /// Whether CurseForge manages `folder`, in any case.
    pub fn manages(&self, folder: &str) -> bool {
        self.folders.contains(&folder.to_lowercase())
    }

    /// Whether CurseForge manages any of `folders`.
    pub fn manages_any<'a>(&self, folders: impl IntoIterator<Item = &'a str>) -> bool {
        folders.into_iter().any(|folder| self.manages(folder))
    }

    /// The names of the addons CurseForge manages, in order. Each addon is
    /// a name and every folder it owns; one managed folder is enough.
    pub fn managed_names<N, F>(&self, addons: impl IntoIterator<Item = (N, F)>) -> Vec<N>
    where
        F: IntoIterator,
        F::Item: AsRef<str>,
    {
        addons
            .into_iter()
            .filter_map(|(name, folders)| {
                folders
                    .into_iter()
                    .any(|folder| self.manages(folder.as_ref()))
                    .then_some(name)
            })
            .collect()
    }

    /// The names of the linked `rows` CurseForge manages, counting each
    /// row's own folder and the folders it owns.
    pub fn managed_row_names(&self, rows: &[AddonRecord]) -> Vec<String> {
        self.managed_names(rows.iter().map(|row| {
            let folders = std::iter::once(row.folder.as_str())
                .chain(row.owned_folders.iter().map(|owned| owned.name.as_str()));
            (row.name.clone(), folders)
        }))
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }
}

/// What to tell the user about one addon CurseForge also manages.
pub const HINT: &str =
    "CurseForge also manages this addon. Mark it Ignored there (Install Settings).";

/// The most addon names a hint lists before "and N more".
const HINT_MAX_NAMES: usize = 3;

/// The hint naming the addons CurseForge also manages: "CurseForge also
/// manages Pawn. Mark it Ignored there (Install Settings)." Empty for none.
pub fn hint_naming(names: &[String]) -> String {
    let short = short_hint_naming(names);
    if short.is_empty() {
        short
    } else {
        format!("{short} (Install Settings).")
    }
}

/// The hint short enough for a result line: "CurseForge also manages Pawn.
/// Mark it Ignored there". Empty for none.
pub fn short_hint_naming(names: &[String]) -> String {
    let them = if names.len() == 1 { "it" } else { "them" };
    let list = match names {
        [] => return String::new(),
        [one] => one.clone(),
        _ if names.len() > HINT_MAX_NAMES => format!(
            "{} and {} more",
            names[..HINT_MAX_NAMES].join(", "),
            names.len() - HINT_MAX_NAMES
        ),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("CurseForge also manages {list}. Mark {them} Ignored there")
}

/// Where in the record the instance for this flavor folder is: one whose GUID
/// and install path both match it, then one whose GUID does (a copied game
/// folder keeps its `.curseclient`), then one whose install path does.
fn instance_index(instances: &[Option<Instance>], guid: &str, flavor_dir: &Path) -> Option<usize> {
    let wanted = normalized_path(flavor_dir);
    let guid_matches = |instance: &Instance| {
        instance
            .guid
            .as_deref()
            .is_some_and(|candidate| candidate.trim().eq_ignore_ascii_case(guid))
    };
    let path_matches = |instance: &Instance| {
        instance
            .install_path
            .as_deref()
            .is_some_and(|path| normalized_path(Path::new(path)) == wanted)
    };
    let find = |test: &dyn Fn(&Instance) -> bool| {
        instances
            .iter()
            .position(|instance| instance.as_ref().is_some_and(test))
    };
    find(&|instance| guid_matches(instance) && path_matches(instance))
        .or_else(|| find(&guid_matches))
        .or_else(|| find(&path_matches))
}

/// The flavor folder of `<flavor>/Interface/AddOns`; `None` for any other shape.
fn flavor_dir(addons_dir: &Path) -> Option<PathBuf> {
    let named = |path: &Path, name: &str| {
        path.file_name()
            .and_then(|found| found.to_str())
            .is_some_and(|found| found.eq_ignore_ascii_case(name))
    };
    if !named(addons_dir, "AddOns") {
        return None;
    }
    let interface = addons_dir.parent()?;
    if !named(interface, "Interface") {
        return None;
    }
    interface.parent().map(Path::to_path_buf)
}

/// The GUID in `<flavor>/.curseclient`, trimmed. Only the flavor folder's
/// file counts: an old Curse client left one at the WoW root too.
fn read_client_guid(flavor_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(flavor_dir.join(CLIENT_FILE)).ok()?;
    let guid = text.trim_start_matches('\u{feff}').trim();
    (!guid.is_empty()).then(|| guid.to_string())
}

/// The game instances in CurseForge's record, each where it is in the file
/// (`None` for an entry in another shape); `None` when it can't be read.
fn read_record(file: &Path) -> Option<Vec<Option<Instance>>> {
    let bytes = fs::read(file).ok()?;
    parse_record(&bytes)
}

fn parse_record(bytes: &[u8]) -> Option<Vec<Option<Instance>>> {
    let instances = serde_json::from_slice::<Vec<Lenient<Instance>>>(without_bom(bytes)).ok()?;
    Some(instances.into_iter().map(Lenient::item).collect())
}

fn without_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

/// The first of `record_files` that LemonUp can read: the one
/// [`CurseForgeManaged::read`] reads, so the one to change.
pub fn record_in_use(record_files: &[PathBuf]) -> Option<PathBuf> {
    record_files
        .iter()
        .find(|file| read_record(file).is_some())
        .cloned()
}

/// What is added to the record's file name for the copy LemonUp keeps
/// before it changes the record: `AddonGameInstance.json.lemonup-backup`.
pub const BACKUP_SUFFIX: &str = ".lemonup-backup";

/// Why LemonUp left CurseForge's record as it was.
#[derive(Debug, thiserror::Error)]
pub enum MarkError {
    #[error("couldn't read CurseForge's record: {0}")]
    Read(#[source] io::Error),
    #[error("CurseForge's record is not in the shape LemonUp knows")]
    Shape,
    #[error("CurseForge's record has no game for this AddOns folder")]
    NoGame,
    #[error("CurseForge already ignores these addons, or doesn't manage them")]
    NothingToChange,
    #[error("CurseForge's record changed while LemonUp was reading it")]
    Changed,
    #[error("couldn't write CurseForge's record: {0}")]
    Write(#[source] io::Error),
    #[error("CurseForge's record didn't read back as written")]
    NotConfirmed,
}

/// Sets `preferenceIsIgnored` to true on every addon of the game instance
/// for `addons_dir` that lists one of `folders` (in any case) and isn't
/// Ignored yet. Returns the names of the addons it marked.
///
/// Only the `false` after each of those keys becomes `true`; every other
/// byte of the record stays as it was. Before writing, the record is copied
/// to its [`BACKUP_SUFFIX`] file next to it, replacing an older copy. The
/// new record is written beside it and moved over it, then read back. When
/// anything is not as expected, the record is left as it was.
pub fn mark_ignored(
    record_file: &Path,
    addons_dir: &Path,
    folders: &[String],
) -> Result<Vec<String>, MarkError> {
    let original = fs::read(record_file).map_err(MarkError::Read)?;
    let (edited, names) = plan_mark(&original, addons_dir, folders)?;
    fs::write(sibling(record_file, BACKUP_SUFFIX), &original).map_err(MarkError::Write)?;
    // CurseForge may have written the record since it was read.
    if fs::read(record_file).map_err(MarkError::Read)? != original {
        return Err(MarkError::Changed);
    }
    replace_file(record_file, &edited).map_err(MarkError::Write)?;
    let read_back = fs::read(record_file).map_err(MarkError::Read)?;
    if read_back == edited
        || matches!(
            plan_mark(&read_back, addons_dir, folders),
            Err(MarkError::NothingToChange)
        )
    {
        Ok(names)
    } else {
        Err(MarkError::NotConfirmed)
    }
}

/// The record with the flags of the addons to mark set to true, and their
/// names. Each `"preferenceIsIgnored":` in the text belongs to the addon
/// entry in the same place in document order, so their counts must agree.
/// The edited text must parse to the original with exactly those flags set.
fn plan_mark(
    original: &[u8],
    addons_dir: &Path,
    folders: &[String],
) -> Result<(Vec<u8>, Vec<String>), MarkError> {
    let flavor = flavor_dir(addons_dir).ok_or(MarkError::NoGame)?;
    let guid = read_client_guid(&flavor).ok_or(MarkError::NoGame)?;
    let text = without_bom(original);
    let bom = original.len() - text.len();
    let instances = parse_record(text).ok_or(MarkError::Shape)?;
    let wanted_instance = instance_index(&instances, &guid, &flavor).ok_or(MarkError::NoGame)?;
    let record: Value = serde_json::from_slice(text).map_err(|_| MarkError::Shape)?;
    let root = record.as_array().ok_or(MarkError::Shape)?;

    // Every addon entry that has the key, in document order.
    let mut entries = Vec::new();
    for (instance, value) in root.iter().enumerate() {
        let Some(addons) = value.get("installedAddons").and_then(Value::as_array) else {
            continue;
        };
        for (addon, entry) in addons.iter().enumerate() {
            if entry.get("preferenceIsIgnored").is_some() {
                entries.push((instance, addon, entry));
            }
        }
    }
    let flags = flag_values(text);
    if flags.len() != entries.len() {
        return Err(MarkError::Shape);
    }

    let wanted = folders
        .iter()
        .map(|folder| folder.to_lowercase())
        .collect::<HashSet<_>>();
    let chosen = entries
        .iter()
        .enumerate()
        .filter(|(_, (instance, _, entry))| {
            *instance == wanted_instance
                && entry.get("preferenceIsIgnored") == Some(&Value::Bool(false))
                && entry_folders(entry).any(|folder| wanted.contains(&folder.to_lowercase()))
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if chosen.is_empty() {
        return Err(MarkError::NothingToChange);
    }

    let mut edited = Vec::with_capacity(original.len());
    let mut copied = 0;
    for &index in &chosen {
        let at = bom + flags[index];
        if !original[at..].starts_with(b"false") {
            return Err(MarkError::Shape);
        }
        edited.extend_from_slice(&original[copied..at]);
        edited.extend_from_slice(b"true");
        copied = at + b"false".len();
    }
    edited.extend_from_slice(&original[copied..]);

    let mut expected = record.clone();
    let mut names = Vec::new();
    for &index in &chosen {
        let (instance, addon, entry) = entries[index];
        expected[instance]["installedAddons"][addon]["preferenceIsIgnored"] = Value::Bool(true);
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| entry_folders(entry).next())
            .unwrap_or_default();
        names.push(name.to_string());
    }
    let parsed: Value =
        serde_json::from_slice(without_bom(&edited)).map_err(|_| MarkError::Shape)?;
    if parsed != expected {
        return Err(MarkError::Shape);
    }
    Ok((edited, names))
}

/// The folder names an addon entry lists among its modules.
fn entry_folders(entry: &Value) -> impl Iterator<Item = &str> {
    entry
        .pointer("/installedFile/modules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|module| module.get("foldername")?.as_str())
}

/// Where the value of each `"preferenceIsIgnored":` key in `text` starts.
/// Whitespace may follow the key and the colon.
fn flag_values(text: &[u8]) -> Vec<usize> {
    const KEY: &[u8] = b"\"preferenceIsIgnored\"";
    let skip_space = |mut at: usize| {
        while text.get(at).is_some_and(|byte| byte.is_ascii_whitespace()) {
            at += 1;
        }
        at
    };
    let mut values = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..]
        .windows(KEY.len())
        .position(|window| window == KEY)
    {
        let after = skip_space(from + found + KEY.len());
        if text.get(after) == Some(&b':') {
            values.push(skip_space(after + 1));
        }
        from += found + KEY.len();
    }
    values
}

/// `path` with `suffix` added to its file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Writes `bytes` to a new file next to `path`, then moves it over `path`,
/// so `path` is never half written.
fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = sibling(path, ".lemonup-new");
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temp, metadata.permissions())?;
        }
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Whether the CurseForge app may be running, so a change to its record
/// would be lost when it writes the record again. Only Linux can tell; any
/// other system says it may be.
pub fn is_running() -> bool {
    if cfg!(target_os = "linux") {
        running_in(Path::new("/proc"))
    } else {
        true
    }
}

/// Whether a process in `proc_dir` (laid out like `/proc`) is named
/// `curseforge`, in any case, by its `comm` or the file name of its `exe`.
/// A folder that can't be read, or lists no process, may hide one: true.
pub fn running_in(proc_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(proc_dir) else {
        return true;
    };
    let named = |name: &str| name.trim().eq_ignore_ascii_case("curseforge");
    let mut processes = 0;
    for entry in entries.flatten() {
        let is_process = entry
            .file_name()
            .to_str()
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()));
        if !is_process {
            continue;
        }
        processes += 1;
        let process = entry.path();
        if fs::read_to_string(process.join("comm")).is_ok_and(|comm| named(&comm)) {
            return true;
        }
        if fs::read_link(process.join("exe")).is_ok_and(|exe| {
            exe.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(named)
        }) {
            return true;
        }
    }
    processes == 0
}

/// `/a/B/` and `\a\b` compare equal: one separator, no trailing one, no case.
fn normalized_path(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// An entry in the expected shape, or anything else (skipped).
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Item(T),
    Other(IgnoredAny),
}

impl<T> Lenient<T> {
    fn item(self) -> Option<T> {
        match self {
            Self::Item(item) => Some(item),
            Self::Other(_) => None,
        }
    }

    fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Item(item) => Some(item),
            Self::Other(_) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Instance {
    guid: Option<String>,
    install_path: Option<String>,
    installed_addons: Option<Vec<Lenient<InstalledAddon>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledAddon {
    installed_file: Option<InstalledFile>,
    preference_is_ignored: Option<bool>,
}

#[derive(Deserialize)]
struct InstalledFile {
    modules: Option<Vec<Lenient<Module>>>,
}

#[derive(Deserialize)]
struct Module {
    foldername: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GUID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_GUID: &str = "00000000-0000-4000-8000-000000000002";

    struct Game {
        _temp: tempfile::TempDir,
        root: PathBuf,
        flavor: PathBuf,
        addons: PathBuf,
        record: PathBuf,
    }

    /// A WoW folder with `_retail_/Interface/AddOns`, and a record file path
    /// next to it (not written yet).
    fn game() -> Game {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("World of Warcraft");
        let flavor = root.join("_retail_");
        let addons = flavor.join("Interface").join("AddOns");
        fs::create_dir_all(&addons).expect("addons");
        let record = temp.path().join("AddonGameInstance.json");
        Game {
            _temp: temp,
            root,
            flavor,
            addons,
            record,
        }
    }

    fn addon(folders: &[&str], ignored: bool) -> serde_json::Value {
        json!({
            "name": "Synthetic",
            "status": 3,
            "installSource": 0,
            "filePaths": [],
            "preferenceIsIgnored": ignored,
            "installedFile": {
                "id": 1,
                "modules": folders
                    .iter()
                    .map(|folder| json!({ "foldername": folder, "fingerprint": 1 }))
                    .collect::<Vec<_>>(),
            },
        })
    }

    fn instance(guid: &str, path: &Path, addons: Vec<serde_json::Value>) -> serde_json::Value {
        json!({
            "guid": guid,
            "installPath": format!("{}/", path.display()),
            "name": "World of Warcraft",
            "installedAddons": addons,
        })
    }

    fn write(path: &Path, value: &serde_json::Value) {
        fs::write(path, serde_json::to_vec(value).expect("json")).expect("write");
    }

    fn managed(game: &Game) -> CurseForgeManaged {
        CurseForgeManaged::read(&game.addons, std::slice::from_ref(&game.record))
    }

    #[test]
    fn a_folder_of_the_instance_with_the_flavor_guid_is_managed() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let elsewhere = game.root.join("_classic_");
        write(
            &game.record,
            &json!([
                instance(OTHER_GUID, &elsewhere, vec![addon(&["Other"], false)]),
                // The install path is stale; the GUID decides.
                instance(
                    GUID,
                    &elsewhere,
                    vec![addon(&["Pawn", "Pawn_Extra"], false)]
                ),
            ]),
        );

        let managed = managed(&game);
        assert!(managed.manages("Pawn"));
        assert!(managed.manages("Pawn_Extra"));
        assert!(!managed.manages("Other"));
        assert!(!managed.manages("Clique"));
    }

    #[test]
    fn without_a_guid_match_the_install_path_decides() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), format!("{GUID}\r\n")).expect("client");
        write(
            &game.record,
            &json!([instance(
                OTHER_GUID,
                &game.flavor,
                vec![addon(&["Pawn"], false)]
            )]),
        );
        assert!(managed(&game).manages("Pawn"));

        // Neither the GUID nor the path matches.
        write(
            &game.record,
            &json!([instance(
                OTHER_GUID,
                &game.root.join("_classic_"),
                vec![addon(&["Pawn"], false)]
            )]),
        );
        assert!(managed(&game).is_empty());
    }

    #[test]
    fn folders_compare_without_case() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID.to_uppercase()).expect("client");
        write(
            &game.record,
            &json!([instance(
                GUID,
                &game.flavor,
                vec![addon(&["DBM-Core"], false)]
            )]),
        );
        let managed = managed(&game);
        assert!(managed.manages("dbm-core"));
        assert!(managed.manages_any(["Clique", "DBM-CORE"]));
        assert!(!managed.manages_any(["Clique"]));
    }

    #[test]
    fn an_addon_ignored_in_curseforge_is_not_managed() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let mut unknown = addon(&["Unknown"], false);
        unknown
            .as_object_mut()
            .expect("object")
            .remove("preferenceIsIgnored");
        write(
            &game.record,
            &json!([instance(
                GUID,
                &game.flavor,
                vec![addon(&["Pawn"], true), unknown, addon(&["Clique"], false)]
            )]),
        );
        let managed = managed(&game);
        assert!(!managed.manages("Pawn"));
        assert!(!managed.manages("Unknown"));
        assert!(managed.manages("Clique"));
    }

    #[test]
    fn without_curseclient_in_the_flavor_folder_nothing_is_managed() {
        let game = game();
        write(
            &game.record,
            &json!([instance(GUID, &game.flavor, vec![addon(&["Pawn"], false)])]),
        );
        assert!(managed(&game).is_empty());

        // An old client's file at the WoW root never counts.
        fs::write(game.root.join(CLIENT_FILE), GUID).expect("root client");
        assert!(managed(&game).is_empty());
    }

    #[test]
    fn a_missing_or_garbled_record_manages_nothing() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        assert!(managed(&game).is_empty(), "no record file");

        for garbled in [
            &b"{ not json"[..],
            b"",
            b"{\"guid\": 1}",
            b"[1, \"two\", null]",
            b"\xff\xfe\x00",
        ] {
            fs::write(&game.record, garbled).expect("record");
            assert!(managed(&game).is_empty(), "{garbled:?}");
        }

        // An AddOns folder in another shape has no flavor folder.
        assert!(
            CurseForgeManaged::read(&game.flavor, std::slice::from_ref(&game.record)).is_empty()
        );
        // The first readable record is used.
        write(
            &game.record,
            &json!([instance(GUID, &game.flavor, vec![addon(&["Pawn"], false)])]),
        );
        let missing = game.root.join("missing.json");
        assert!(
            CurseForgeManaged::read(&game.addons, &[missing, game.record.clone()]).manages("Pawn")
        );
    }

    #[test]
    fn unknown_fields_and_odd_entries_are_tolerated() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let mut pawn = addon(&["Pawn"], false);
        pawn["somethingNew"] = json!({ "nested": [1, 2, 3] });
        pawn["installedFile"]["modules"]
            .as_array_mut()
            .expect("modules")
            .push(json!("not a module"));
        let mut record = json!([
            "not an instance",
            instance(
                GUID,
                &game.flavor,
                vec![
                    pawn,
                    json!(42),
                    json!({ "preferenceIsIgnored": false, "installedFile": null }),
                ]
            )
        ]);
        record[1]["cachedScans"] = json!([{ "anything": true }]);
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(serde_json::to_vec(&record).expect("json"));
        fs::write(&game.record, bytes).expect("record");

        assert!(managed(&game).manages("Pawn"));
    }

    #[test]
    fn the_hint_names_the_addons() {
        assert_eq!(hint_naming(&[]), "");
        assert_eq!(
            hint_naming(&["Pawn".to_string()]),
            "CurseForge also manages Pawn. Mark it Ignored there (Install Settings)."
        );
        assert_eq!(
            hint_naming(&["Pawn".to_string(), "Clique".to_string(), "Spy".to_string()]),
            "CurseForge also manages Pawn, Clique and Spy. Mark them Ignored there \
             (Install Settings)."
        );
        assert_eq!(
            short_hint_naming(&["Pawn".to_string()]),
            "CurseForge also manages Pawn. Mark it Ignored there"
        );
        assert_eq!(short_hint_naming(&[]), "");
    }

    #[test]
    fn a_long_list_of_names_gives_way_to_and_n_more() {
        let names = (0..40)
            .map(|index| format!("Addon{index:02}"))
            .collect::<Vec<_>>();
        assert_eq!(
            hint_naming(&names),
            "CurseForge also manages Addon00, Addon01, Addon02 and 37 more. Mark them \
             Ignored there (Install Settings)."
        );
        assert_eq!(
            short_hint_naming(&names[..4]),
            "CurseForge also manages Addon00, Addon01, Addon02 and 1 more. Mark them \
             Ignored there"
        );
    }

    #[test]
    fn the_instance_matching_both_guid_and_path_wins() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let copy = game.root.join("_retail_copy_");
        write(
            &game.record,
            &json!([
                // The same GUID at another path: a copied game folder.
                instance(GUID, &copy, vec![addon(&["FromTheCopy"], false)]),
                // Another GUID at this path.
                instance(OTHER_GUID, &game.flavor, vec![addon(&["PathOnly"], false)]),
                instance(GUID, &game.flavor, vec![addon(&["Both"], false)]),
            ]),
        );
        let managed = managed(&game);
        assert!(managed.manages("Both"));
        assert!(!managed.manages("FromTheCopy"));
        assert!(!managed.manages("PathOnly"));

        // Without that instance, the GUID wins over the path.
        write(
            &game.record,
            &json!([
                instance(OTHER_GUID, &game.flavor, vec![addon(&["PathOnly"], false)]),
                instance(GUID, &copy, vec![addon(&["FromTheCopy"], false)]),
            ]),
        );
        let managed =
            super::CurseForgeManaged::read(&game.addons, std::slice::from_ref(&game.record));
        assert!(managed.manages("FromTheCopy"));
        assert!(!managed.manages("PathOnly"));
    }

    /// A game whose record lists these addons for the flavor folder.
    fn managed_with(addons: Vec<serde_json::Value>) -> CurseForgeManaged {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        write(&game.record, &json!([instance(GUID, &game.flavor, addons)]));
        managed(&game)
    }

    #[test]
    fn managed_names_match_folders_without_case() {
        let managed = managed_with(vec![addon(&["DBM-Core"], false)]);
        let names = managed.managed_names([
            ("Deadly Boss Mods", vec!["dbm-core"]),
            ("Clique", vec!["Clique"]),
        ]);
        assert_eq!(names, ["Deadly Boss Mods"]);
    }

    #[test]
    fn managed_names_skip_addons_ignored_in_curseforge() {
        let managed = managed_with(vec![addon(&["Pawn"], true), addon(&["Spy"], false)]);
        let names = managed.managed_names([("Pawn", ["Pawn"]), ("Spy", ["Spy"])]);
        assert_eq!(names, ["Spy"]);
    }

    #[test]
    fn managed_names_count_an_addon_once_when_any_folder_is_managed() {
        let managed = managed_with(vec![addon(&["Pawn_Extra"], false)]);
        let names = managed.managed_names([
            ("Pawn", vec!["Pawn", "Pawn_Extra", "Pawn_Other"]),
            ("Clique", vec!["Clique", "Clique_Extra"]),
        ]);
        assert_eq!(names, ["Pawn"]);
        assert!(
            managed
                .managed_names([("Nothing", Vec::<&str>::new())])
                .is_empty()
        );
    }

    #[test]
    fn managed_row_names_check_the_own_folder_and_the_owned_ones() {
        use crate::domain::{OwnedFolder, SourceKind};

        let managed = managed_with(vec![addon(&["Pawn_Extra", "SPY"], false)]);
        let mut pawn = AddonRecord::new("Pawn", "Pawn", SourceKind::GitHub);
        pawn.owned_folders = vec![OwnedFolder {
            name: "pawn_extra".to_string(),
        }];
        let spy = AddonRecord::new("Spy", "spy", SourceKind::GitHub);
        let clique = AddonRecord::new("Clique", "Clique", SourceKind::GitHub);
        assert_eq!(
            managed.managed_row_names(&[pawn, clique, spy]),
            ["Pawn", "Spy"]
        );
    }

    /// An addon entry with a name, as CurseForge writes one.
    fn named(name: &str, folders: &[&str], ignored: bool) -> serde_json::Value {
        let mut entry = addon(folders, ignored);
        entry["name"] = json!(name);
        entry
    }

    /// Two instances, minified: the flavor folder's (GUID) and another
    /// game's that lists Pawn too.
    fn two_instances(game: &Game) -> Vec<u8> {
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let record = json!([
            instance(
                OTHER_GUID,
                &game.root.join("_classic_"),
                vec![named("Pawn Classic", &["Pawn"], false)]
            ),
            instance(
                GUID,
                &game.flavor,
                vec![
                    named("Pawn", &["Pawn"], false),
                    named(
                        "Deadly Boss Mods",
                        &["DBM-Core", "DBM-StatusBarTimers"],
                        false
                    ),
                    named("Clique", &["Clique"], false),
                    named("Spy", &["Spy"], true),
                ]
            ),
        ]);
        let bytes = serde_json::to_vec(&record).expect("json");
        fs::write(&game.record, &bytes).expect("record");
        bytes
    }

    /// How many `false` literals of `original` became `true` in `edited`,
    /// failing on any other difference.
    fn flips(original: &[u8], edited: &[u8]) -> usize {
        let (mut at, mut to, mut flips) = (0, 0, 0);
        while at < original.len() {
            if edited.get(to) == Some(&original[at]) {
                at += 1;
                to += 1;
                continue;
            }
            assert!(original[at..].starts_with(b"false"), "differs at {at}");
            assert!(edited[to..].starts_with(b"true"), "differs at {to}");
            at += b"false".len();
            to += b"true".len();
            flips += 1;
        }
        assert_eq!(to, edited.len(), "edited is longer");
        flips
    }

    fn folders(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn backup(game: &Game) -> PathBuf {
        game.record
            .with_file_name(format!("AddonGameInstance.json{BACKUP_SUFFIX}"))
    }

    #[test]
    fn marking_flips_only_the_chosen_addons_of_the_matching_instance() {
        let game = game();
        let original = two_instances(&game);
        assert!(managed(&game).manages("DBM-Core"));

        // The folder in another case still matches; Clique is not asked for.
        let marked = mark_ignored(&game.record, &game.addons, &folders(&["Pawn", "dbm-core"]))
            .expect("marked");
        assert_eq!(marked, ["Pawn", "Deadly Boss Mods"]);

        let edited = fs::read(&game.record).expect("record");
        assert_eq!(edited.len(), original.len() - 2);
        assert_eq!(flips(&original, &edited), 2);

        let mut expected: serde_json::Value = serde_json::from_slice(&original).expect("json");
        expected[1]["installedAddons"][0]["preferenceIsIgnored"] = json!(true);
        expected[1]["installedAddons"][1]["preferenceIsIgnored"] = json!(true);
        let parsed: serde_json::Value = serde_json::from_slice(&edited).expect("json");
        assert_eq!(parsed, expected);
        // The other game's Pawn is untouched.
        assert_eq!(
            parsed[0]["installedAddons"][0]["preferenceIsIgnored"],
            json!(false)
        );

        let managed = managed(&game);
        assert!(!managed.manages("Pawn"));
        assert!(!managed.manages("DBM-StatusBarTimers"));
        assert!(managed.manages("Clique"));
        assert_eq!(fs::read(backup(&game)).expect("backup"), original);
    }

    #[test]
    fn an_already_ignored_addon_is_nothing_to_change() {
        let game = game();
        let original = two_instances(&game);
        for asked in [&["Spy"][..], &["NotThere"], &[]] {
            let result = mark_ignored(&game.record, &game.addons, &folders(asked));
            assert!(
                matches!(result, Err(MarkError::NothingToChange)),
                "{asked:?}: {result:?}"
            );
        }
        assert_eq!(fs::read(&game.record).expect("record"), original);
        assert!(!backup(&game).exists());
    }

    #[test]
    fn a_flag_key_outside_the_addon_entries_is_refused() {
        let game = game();
        two_instances(&game);
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&game.record).expect("record")).expect("json");
        record[1]["preferenceIsIgnored"] = json!(false);
        let original = serde_json::to_vec(&record).expect("json");
        fs::write(&game.record, &original).expect("record");

        let result = mark_ignored(&game.record, &game.addons, &folders(&["Pawn"]));
        assert!(matches!(result, Err(MarkError::Shape)), "{result:?}");
        assert_eq!(fs::read(&game.record).expect("record"), original);
        assert!(!backup(&game).exists());

        // A key written another way counts once in the record and not in the text.
        let clean = two_instances(&game);
        let escaped = String::from_utf8(clean).expect("utf8").replacen(
            "\"preferenceIsIgnored\"",
            "\"preferenceIsIgnor\\u0065d\"",
            1,
        );
        assert!(escaped.contains("Ignor\\u0065d"));
        fs::write(&game.record, &escaped).expect("record");
        let result = mark_ignored(&game.record, &game.addons, &folders(&["Pawn"]));
        assert!(matches!(result, Err(MarkError::Shape)), "{result:?}");
        assert_eq!(fs::read_to_string(&game.record).expect("record"), escaped);
    }

    #[test]
    fn the_backup_is_replaced_on_the_next_mark() {
        let game = game();
        let original = two_instances(&game);
        mark_ignored(&game.record, &game.addons, &folders(&["Pawn"])).expect("first");
        assert_eq!(fs::read(backup(&game)).expect("backup"), original);
        let after_first = fs::read(&game.record).expect("record");

        mark_ignored(&game.record, &game.addons, &folders(&["Clique"])).expect("second");
        assert_eq!(fs::read(backup(&game)).expect("backup"), after_first);
        let after_second = fs::read(&game.record).expect("record");
        assert_eq!(flips(&original, &after_second), 2);
        // No file is left beside it.
        let names = fs::read_dir(game.record.parent().expect("parent"))
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<HashSet<_>>();
        assert!(!names.contains(std::ffi::OsStr::new("AddonGameInstance.json.lemonup-new")));
    }

    #[test]
    fn an_unknown_shape_or_no_matching_game_is_refused() {
        let game = game();
        assert!(matches!(
            mark_ignored(&game.record, &game.addons, &folders(&["Pawn"])),
            Err(MarkError::Read(_))
        ));
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        for shape in [&b"{\"a\": 1}"[..], b"not json", b""] {
            fs::write(&game.record, shape).expect("record");
            let result = mark_ignored(&game.record, &game.addons, &folders(&["Pawn"]));
            assert!(matches!(result, Err(MarkError::Shape)), "{result:?}");
            assert_eq!(fs::read(&game.record).expect("record"), shape);
        }

        let elsewhere = json!([instance(
            OTHER_GUID,
            &game.root.join("_classic_"),
            vec![named("Pawn", &["Pawn"], false)]
        )]);
        write(&game.record, &elsewhere);
        let before = fs::read(&game.record).expect("record");
        let result = mark_ignored(&game.record, &game.addons, &folders(&["Pawn"]));
        assert!(matches!(result, Err(MarkError::NoGame)), "{result:?}");
        assert_eq!(fs::read(&game.record).expect("record"), before);
        assert!(!backup(&game).exists());
    }

    #[test]
    fn whitespace_after_the_key_and_a_bom_are_kept() {
        let game = game();
        fs::write(game.flavor.join(CLIENT_FILE), GUID).expect("client");
        let path = game.flavor.display().to_string().replace('\\', "\\\\");
        let text = format!(
            "\u{feff}[\n  {{\n    \"guid\": \"{GUID}\",\n    \"installPath\": \"{path}\",\n    \
             \"installedAddons\": [\n      {{ \"name\": \"Pawn\", \"preferenceIsIgnored\" :\n  \
             false, \"installedFile\": {{ \"modules\": [ {{ \"foldername\": \"Pawn\" }} ] }} }},\n      \
             {{ \"name\": \"Spy\", \"preferenceIsIgnored\":false, \"installedFile\": \
             {{ \"modules\": [ {{ \"foldername\": \"Spy\" }} ] }} }}\n    ]\n  }}\n]\n"
        );
        fs::write(&game.record, &text).expect("record");
        assert_eq!(
            mark_ignored(&game.record, &game.addons, &folders(&["Pawn"])).expect("marked"),
            ["Pawn"]
        );
        let edited = fs::read_to_string(&game.record).expect("record");
        assert_eq!(edited, text.replacen("false", "true", 1));
        assert!(managed(&game).manages("Spy"));
        assert!(!managed(&game).manages("Pawn"));
    }

    #[test]
    fn the_record_in_use_is_the_first_readable_one() {
        let game = game();
        two_instances(&game);
        let missing = game.root.join("missing.json");
        let garbled = game.root.join("garbled.json");
        fs::write(&garbled, "nope").expect("garbled");
        assert_eq!(
            record_in_use(&[missing.clone(), garbled, game.record.clone()]),
            Some(game.record.clone())
        );
        assert_eq!(record_in_use(&[missing]), None);
    }

    /// A folder laid out like `/proc`, one process per `(pid, comm)`.
    fn proc_dir(processes: &[(&str, &str)]) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("temp");
        for (pid, comm) in processes {
            let process = temp.path().join(pid);
            fs::create_dir_all(&process).expect("process");
            fs::write(process.join("comm"), format!("{comm}\n")).expect("comm");
        }
        fs::write(temp.path().join("cpuinfo"), "").expect("cpuinfo");
        fs::create_dir_all(temp.path().join("self")).expect("self");
        temp
    }

    #[test]
    fn curseforge_is_running_when_a_process_is_named_so() {
        let proc = proc_dir(&[("1", "systemd"), ("42", "bash")]);
        assert!(!running_in(proc.path()));

        let proc = proc_dir(&[("1", "systemd"), ("4242", "CurseForge")]);
        assert!(running_in(proc.path()));

        // A name that only starts the same is something else.
        let proc = proc_dir(&[("1", "curseforge-helper")]);
        assert!(!running_in(proc.path()));
    }

    #[cfg(unix)]
    #[test]
    fn curseforge_is_running_when_a_process_runs_its_program() {
        let proc = proc_dir(&[("1", "systemd"), ("77", "electron")]);
        std::os::unix::fs::symlink(
            "/opt/CurseForge/curseforge",
            proc.path().join("77").join("exe"),
        )
        .expect("exe");
        assert!(running_in(proc.path()));
    }

    #[test]
    fn a_proc_folder_that_cant_be_read_may_hide_curseforge() {
        let proc = proc_dir(&[]);
        assert!(running_in(&proc.path().join("missing")));
        // No process at all is not a real /proc.
        assert!(running_in(proc.path()));
    }
}
