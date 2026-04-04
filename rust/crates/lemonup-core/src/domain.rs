use std::path::PathBuf;

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
}

#[cfg(test)]
mod tests {
    use super::{AddonRecord, OwnershipSource, SourceKind};

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
