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
//! Everything here only reads. A file that is missing, unreadable or not in
//! the expected shape means "not managed", never an error. The `.toc` fields
//! that name CurseForge (`X-Curse-Project-ID`) are written by addon authors
//! and are in every copy, so they say nothing and are never used.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::Deserialize;
use serde::de::IgnoredAny;

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

    fn from_instances(instances: &[Instance], guid: &str, flavor_dir: &Path) -> Self {
        let instance = instances
            .iter()
            .find(|instance| {
                instance
                    .guid
                    .as_deref()
                    .is_some_and(|candidate| candidate.trim().eq_ignore_ascii_case(guid))
            })
            .or_else(|| {
                let wanted = normalized_path(flavor_dir);
                instances.iter().find(|instance| {
                    instance
                        .install_path
                        .as_deref()
                        .is_some_and(|path| normalized_path(Path::new(path)) == wanted)
                })
            });
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

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }
}

/// What to tell the user about one addon CurseForge also manages.
pub const HINT: &str =
    "CurseForge also manages this addon. Mark it Ignored there (Install Settings).";

/// The hint naming the addons CurseForge also manages: "CurseForge also
/// manages Pawn. Mark it Ignored there (Install Settings)."
pub fn hint_naming(names: &[String]) -> String {
    let (list, them) = match names {
        [] => return String::new(),
        [one] => (one.clone(), "it"),
        [rest @ .., last] => (format!("{} and {last}", rest.join(", ")), "them"),
    };
    format!("CurseForge also manages {list}. Mark {them} Ignored there (Install Settings).")
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

/// The game instances in CurseForge's record; `None` when it can't be read.
fn read_record(file: &Path) -> Option<Vec<Instance>> {
    let bytes = fs::read(file).ok()?;
    parse_record(&bytes)
}

fn parse_record(bytes: &[u8]) -> Option<Vec<Instance>> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let instances = serde_json::from_slice::<Vec<Lenient<Instance>>>(bytes).ok()?;
    Some(instances.into_iter().filter_map(Lenient::item).collect())
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
    }
}
