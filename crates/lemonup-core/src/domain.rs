use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    GitHub,
    Tukui,
    WowInterface,
    Wago,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameFlavor {
    Retail,
    Classic,
    Cata,
    Forever,
}

impl GameFlavor {
    /// Detects the client flavor from an AddOns path. The client folder is the AddOns
    /// directory's grandparent (`<client>/Interface/AddOns`); only that component is considered
    /// (`_retail_` or `_forever_`, case-insensitive), never other parts of the path.
    pub fn detect_from_path(path: &Path) -> Option<Self> {
        let rendered = path.to_string_lossy();
        let client = rendered
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .rev()
            .nth(2)?;
        if client.eq_ignore_ascii_case("_retail_") {
            Some(Self::Retail)
        } else if client.eq_ignore_ascii_case("_forever_") {
            Some(Self::Forever)
        } else {
            None
        }
    }

    /// Flavor for a configured AddOns directory; paths without a known client folder
    /// (for example test fixtures) are treated as retail.
    pub fn for_addons_dir(path: &Path) -> Self {
        Self::detect_from_path(path).unwrap_or(Self::Retail)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddonKind {
    Addon,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipSource {
    None,
    ScanInferred,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFolder {
    pub name: String,
}

/// Values a newer LemonUp stored for an addon that this version does not know (a new source,
/// flavor, kind or ownership value).
///
/// Such an addon loads as not linked (`source` is [`SourceKind::Manual`]), so it is never
/// checked or updated, and the known fields stand in with safe fallbacks. The stored values are
/// written back unchanged on every save until this version links the addon to a source itself,
/// so upgrading again restores the link.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnrecognizedValues {
    /// The stored source, even when it is known: an unknown flavor, kind or ownership value also
    /// leaves the addon not linked here.
    pub source: String,
    pub flavor: Option<String>,
    pub kind: Option<String>,
    pub ownership_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonRecord {
    pub id: Option<i64>,
    pub name: String,
    pub folder: String,
    pub owned_folders: Vec<OwnedFolder>,
    pub ownership_source: OwnershipSource,
    pub kind: AddonKind,
    pub kind_override: bool,
    pub flavor: GameFlavor,
    pub version: Option<String>,
    pub git_commit: Option<String>,
    pub author: Option<String>,
    pub interface: Option<String>,
    pub source: SourceKind,
    pub source_url: Option<String>,
    pub required_deps: Vec<String>,
    pub optional_deps: Vec<String>,
    pub embedded_libs: Vec<String>,
    pub installed_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub last_checked_at: Option<OffsetDateTime>,
    pub remote_version: Option<String>,
    /// The user's overrides of TOC display fields, by field name (`Title`, `Notes`, `Category`;
    /// see `toc_edit::TocField`). LemonUp writes them into the addon's TOC files on every install
    /// and update. Keys this version does not know are kept as they are and never acted on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub toc_overrides: BTreeMap<String, String>,
    /// The addon's name at its source (Wago.io, WoWInterface, TukUI), cleaned with
    /// [`source_name`]. Shown instead of the TOC Title; see [`AddonRecord::display_name`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// Set only for a row written by a newer LemonUp with values this version does not know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unrecognized: Option<UnrecognizedValues>,
}

impl AddonRecord {
    pub fn new(name: impl Into<String>, folder: impl Into<String>, source: SourceKind) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            id: None,
            name: name.into(),
            folder: folder.into(),
            owned_folders: Vec::new(),
            ownership_source: OwnershipSource::None,
            kind: AddonKind::Addon,
            kind_override: false,
            flavor: GameFlavor::Retail,
            version: None,
            git_commit: None,
            author: None,
            interface: None,
            source,
            source_url: None,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
            installed_at: now,
            updated_at: now,
            last_checked_at: None,
            remote_version: None,
            toc_overrides: BTreeMap::new(),
            source_name: None,
            unrecognized: None,
        }
    }

    /// The name LemonUp shows for this addon: a Title the user set in the TOC editor, else the
    /// source's name for an addon linked to Wago.io, WoWInterface or TukUI, else the TOC Title.
    /// GitHub only gives a repository name, so GitHub addons keep their TOC Title.
    pub fn display_name(&self) -> &str {
        let has_title_override =
            crate::toc_edit::title_override_name(&self.toc_overrides).is_some();
        let named_by_source = matches!(
            self.source,
            SourceKind::Wago | SourceKind::WowInterface | SourceKind::Tukui
        );
        match self.source_name.as_deref() {
            Some(source_name) if named_by_source && !has_title_override => source_name,
            // `name` is the Title override when there is one, else the TOC Title.
            _ => &self.name,
        }
    }

    pub fn effective_ownership_source(&self) -> OwnershipSource {
        if self.ownership_source == OwnershipSource::Managed {
            OwnershipSource::Managed
        } else if self.owned_folders.is_empty() {
            OwnershipSource::None
        } else if self.ownership_source == OwnershipSource::None {
            OwnershipSource::ScanInferred
        } else {
            self.ownership_source
        }
    }

    pub fn set_scan_owned_folders(&mut self, owned_folders: Vec<OwnedFolder>) {
        self.owned_folders = owned_folders;
        self.ownership_source = if self.owned_folders.is_empty() {
            OwnershipSource::None
        } else {
            OwnershipSource::ScanInferred
        };
    }

    pub fn set_managed_owned_folders(&mut self, owned_folders: Vec<OwnedFolder>) {
        self.owned_folders = owned_folders;
        self.ownership_source = OwnershipSource::Managed;
    }

    pub fn clear_owned_folders(&mut self) {
        self.owned_folders.clear();
        self.ownership_source = OwnershipSource::None;
    }

    pub fn has_authoritative_owned_folders(&self) -> bool {
        self.effective_ownership_source() == OwnershipSource::Managed
    }

    /// Whether another addon must leave this row's folders alone: it is linked here, or a newer
    /// LemonUp linked it (it shows as "Not linked" in this version, but its folders still belong
    /// to it).
    pub fn holds_its_folders(&self) -> bool {
        self.source != SourceKind::Manual || self.unrecognized.is_some()
    }

    /// Whether this row is `folder` itself or lists it among its owned folders, without case.
    pub fn claims_folder(&self, folder: &str) -> bool {
        self.folder.to_lowercase() == folder.to_lowercase()
            || self
                .owned_folders
                .iter()
                .any(|owned| owned.name.to_lowercase() == folder.to_lowercase())
    }

    /// Whether a newer LemonUp stored how this addon's owned folders were assigned in a way this
    /// version does not know. Its owned folders are then kept as stored but never acted on.
    pub fn has_unrecognized_ownership(&self) -> bool {
        self.unrecognized
            .as_ref()
            .is_some_and(|values| values.ownership_source.is_some())
    }
}

/// `raw`, a name an addon source gives, as LemonUp stores it: HTML entities decoded, WoW color
/// codes removed and spaces trimmed, the same as a TOC Title. `None` when nothing is left.
pub fn source_name(raw: &str) -> Option<String> {
    let name = crate::scan::strip_wow_color_codes(decode_html_entities(raw));
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Decodes the HTML entities addon sites put in names (`&amp;`, `&lt;`, `&gt;`, `&quot;`,
/// `&#39;`, `&apos;` and numeric `&#NN;` or `&#xNN;`). Anything else is kept as it is.
fn decode_html_entities(raw: &str) -> String {
    let mut decoded = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let entity = rest[1..]
            .find(';')
            .filter(|end| *end <= 10)
            .map(|end| &rest[1..=end]);
        let character = entity.and_then(|entity| match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let number = entity.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse::<u32>().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (entity, character) {
            (Some(entity), Some(character)) => {
                decoded.push(character);
                rest = &rest[entity.len() + 2..];
            }
            _ => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{AddonRecord, GameFlavor, OwnershipSource, SourceKind, source_name};

    #[test]
    fn source_names_lose_color_codes_and_html_entities() {
        assert_eq!(
            source_name(" |cff1784d1Deadly|r Boss Mods &amp; Friends ").as_deref(),
            Some("Deadly Boss Mods & Friends")
        );
        assert_eq!(
            source_name("&lt;DBM&gt; &quot;Core&quot; &#39;x&#39; &#65;&#x42; &apos;").as_deref(),
            Some("<DBM> \"Core\" 'x' AB '")
        );
        // Not entities: kept as they are.
        assert_eq!(
            source_name("Tom & Jerry &bogus; &#xZZ; &").as_deref(),
            Some("Tom & Jerry &bogus; &#xZZ; &")
        );
        assert_eq!(source_name("|cffff0000|r  "), None);
        assert_eq!(source_name(""), None);
    }

    #[test]
    fn display_name_prefers_a_title_override_then_the_source_name_then_the_toc_title() {
        let mut addon = AddonRecord::new("<DBM Core> Main Core", "DBM-Core", SourceKind::Wago);
        addon.source_name = Some("Deadly Boss Mods (DBM)".to_string());
        assert_eq!(addon.display_name(), "Deadly Boss Mods (DBM)");

        for source in [SourceKind::WowInterface, SourceKind::Tukui] {
            addon.source = source;
            assert_eq!(addon.display_name(), "Deadly Boss Mods (DBM)", "{source:?}");
        }
        for source in [SourceKind::GitHub, SourceKind::Manual] {
            addon.source = source;
            assert_eq!(addon.display_name(), "<DBM Core> Main Core", "{source:?}");
        }

        addon.source = SourceKind::Wago;
        addon.source_name = None;
        assert_eq!(addon.display_name(), "<DBM Core> Main Core");

        // With a Title override, `name` is that override (storage keeps it so).
        addon.source_name = Some("Deadly Boss Mods (DBM)".to_string());
        addon.toc_overrides = BTreeMap::from([("Title".to_string(), "My DBM".to_string())]);
        addon.name = "My DBM".to_string();
        assert_eq!(addon.display_name(), "My DBM");
    }

    #[test]
    fn detects_flavor_from_client_folder() {
        assert_eq!(
            GameFlavor::detect_from_path(Path::new(
                "/Applications/World of Warcraft/_retail_/Interface/AddOns"
            )),
            Some(GameFlavor::Retail)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new(
                "/Applications/World of Warcraft/_forever_/Interface/AddOns"
            )),
            Some(GameFlavor::Forever)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new(
                r"D:\World of Warcraft\_FOREVER_\Interface\AddOns"
            )),
            Some(GameFlavor::Forever)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new(
                r"D:\World of Warcraft\_Retail_\Interface\AddOns"
            )),
            Some(GameFlavor::Retail)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new(
                "/Applications/World of Warcraft/_forever_/Interface/AddOns/"
            )),
            Some(GameFlavor::Forever)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new("/x/_retail_link/_forever_/Interface/AddOns")),
            Some(GameFlavor::Forever)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new("/x/_retail_/stuff/_forever_/Interface/AddOns")),
            Some(GameFlavor::Forever)
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new("/x/_forever_/stuff/Interface/AddOns")),
            None
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new("/x/_retail_/Interface/AddOns/Sub")),
            None
        );
        assert_eq!(
            GameFlavor::detect_from_path(Path::new("/tmp/sandbox/Interface/AddOns")),
            None
        );
        assert_eq!(GameFlavor::detect_from_path(Path::new("AddOns")), None);
        assert_eq!(
            GameFlavor::for_addons_dir(Path::new("/tmp/sandbox/Interface/AddOns")),
            GameFlavor::Retail
        );
    }

    #[test]
    fn managed_empty_owned_folders_remain_authoritative() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.set_managed_owned_folders(Vec::new());

        assert_eq!(addon.effective_ownership_source(), OwnershipSource::Managed);
        assert!(addon.has_authoritative_owned_folders());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallSource {
    Url {
        url: String,
    },
    Tukui {
        channel: String,
        folder: String,
    },
    Wago {
        slug: String,
        stability: Option<String>,
    },
    ExistingFolder {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub source: InstallSource,
    pub destination: PathBuf,
    pub target_folders: Vec<String>,
    pub overwrite_existing: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    UpToDate,
    UpdateAvailable,
    /// The remote version parses as strictly lower than the installed one (a stale provider
    /// page, for example); it is not offered as an update unless forced.
    RemoteOlder,
    Unknown,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheck {
    pub addon_name: String,
    pub status: UpdateStatus,
    pub remote_version: Option<String>,
    pub checked_at: OffsetDateTime,
    pub message: Option<String>,
}
