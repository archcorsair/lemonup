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
            unrecognized: None,
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

    /// Whether a newer LemonUp stored how this addon's owned folders were assigned in a way this
    /// version does not know. Its owned folders are then kept as stored but never acted on.
    pub fn has_unrecognized_ownership(&self) -> bool {
        self.unrecognized
            .as_ref()
            .is_some_and(|values| values.ownership_source.is_some())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AddonRecord, GameFlavor, OwnershipSource, SourceKind};

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
