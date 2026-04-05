use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::domain::{AddonRecord, SourceKind};
use crate::error::{LemonupError, Result};

pub const DEFAULT_TRANSFER_FILE_NAME: &str = "lemonup-addons.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferSourceKind {
    GitHub,
    Tukui,
    Wowinterface,
    Manual,
    Wago,
}

impl TransferSourceKind {
    fn from_source_kind(source: SourceKind) -> Self {
        match source {
            SourceKind::GitHub => Self::GitHub,
            SourceKind::Tukui => Self::Tukui,
            SourceKind::WowInterface => Self::Wowinterface,
            SourceKind::Manual => Self::Manual,
            SourceKind::Wago => Self::Wago,
        }
    }

    pub fn as_source_kind(self) -> SourceKind {
        match self {
            Self::GitHub => SourceKind::GitHub,
            Self::Tukui => SourceKind::Tukui,
            Self::Wowinterface => SourceKind::WowInterface,
            Self::Manual => SourceKind::Manual,
            Self::Wago => SourceKind::Wago,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferAddon {
    pub name: String,
    pub folder: String,
    #[serde(rename = "type")]
    pub source: TransferSourceKind,
    pub url: Option<String>,
    #[serde(rename = "ownedFolders", skip_serializing_if = "Option::is_none")]
    pub owned_folders: Option<Vec<String>>,
    pub reinstallable: bool,
}

impl TransferAddon {
    pub fn from_addon_record(addon: &AddonRecord) -> Self {
        let owned_folders = (!addon.owned_folders.is_empty()).then(|| {
            addon
                .owned_folders
                .iter()
                .map(|owned| owned.name.clone())
                .collect::<Vec<_>>()
        });

        Self {
            name: addon.name.clone(),
            folder: addon.folder.clone(),
            source: TransferSourceKind::from_source_kind(addon.source),
            url: addon.source_url.clone(),
            owned_folders,
            reinstallable: addon.source != SourceKind::Manual && addon.source_url.is_some(),
        }
    }

    pub fn represented_folders(&self) -> Vec<String> {
        std::iter::once(self.folder.clone())
            .chain(
                self.owned_folders
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|folder| folder != &self.folder),
            )
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferFile {
    pub version: u8,
    #[serde(rename = "exportedAt")]
    pub exported_at: String,
    pub addons: Vec<TransferAddon>,
}

impl TransferFile {
    pub fn from_addons(addons: &[AddonRecord]) -> Self {
        let mut exported = addons
            .iter()
            .map(TransferAddon::from_addon_record)
            .collect::<Vec<_>>();
        exported.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.folder.cmp(&right.folder))
        });

        Self {
            version: 1,
            exported_at: OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
            addons: exported,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAnalysis {
    pub to_install: Vec<TransferAddon>,
    pub already_installed: Vec<TransferAddon>,
    pub manual_addons: Vec<TransferAddon>,
    pub duplicate_addons: Vec<TransferAddon>,
    pub unsupported_addons: Vec<TransferAddon>,
}

pub fn default_transfer_path() -> Result<PathBuf> {
    let Some(base_dirs) = BaseDirs::new() else {
        return Err(LemonupError::PathsUnavailable);
    };

    Ok(base_dirs.home_dir().join(DEFAULT_TRANSFER_FILE_NAME))
}

pub fn export_addons(addons: &[AddonRecord], output_path: &Path) -> Result<TransferFile> {
    let export_file = TransferFile::from_addons(addons);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_vec_pretty(&export_file)?)?;
    Ok(export_file)
}

pub fn parse_import_file(input_path: &Path) -> Result<TransferFile> {
    let bytes = fs::read(input_path)?;
    let transfer: TransferFile = serde_json::from_slice(&bytes)?;
    if transfer.version != 1 {
        return Err(LemonupError::InvalidArgument(format!(
            "unsupported import file version {}",
            transfer.version
        )));
    }
    Ok(transfer)
}

pub fn analyze_import(
    transfer: &TransferFile,
    existing_folders: &HashSet<String>,
) -> ImportAnalysis {
    let mut analysis = ImportAnalysis {
        to_install: Vec::new(),
        already_installed: Vec::new(),
        manual_addons: Vec::new(),
        duplicate_addons: Vec::new(),
        unsupported_addons: Vec::new(),
    };
    let mut scheduled_folders = HashSet::new();

    for addon in &transfer.addons {
        let represented = addon
            .represented_folders()
            .into_iter()
            .map(|folder| folder.to_ascii_lowercase())
            .collect::<Vec<_>>();

        if !addon.reinstallable {
            analysis.manual_addons.push(addon.clone());
            continue;
        }

        if addon.url.as_deref().is_none_or(str::is_empty) {
            analysis.unsupported_addons.push(addon.clone());
            continue;
        }

        if represented
            .iter()
            .any(|folder| existing_folders.contains(folder))
        {
            analysis.already_installed.push(addon.clone());
            continue;
        }

        if represented
            .iter()
            .any(|folder| scheduled_folders.contains(folder))
        {
            analysis.duplicate_addons.push(addon.clone());
            continue;
        }

        scheduled_folders.extend(represented);
        analysis.to_install.push(addon.clone());
    }

    analysis
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use super::{
        ImportAnalysis, TransferFile, TransferSourceKind, analyze_import, export_addons,
        parse_import_file,
    };
    use crate::{AddonRecord, OwnedFolder, SourceKind};

    #[test]
    fn export_round_trip_matches_v1_shape() {
        let temp = tempdir().expect("tempdir");
        let output = temp.path().join("lemonup-addons.json");
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.source_url = Some("https://addons.wago.io/addons/weakauras".to_string());
        addon.owned_folders = vec![OwnedFolder {
            name: "WeakAurasArchive".to_string(),
        }];

        let exported = export_addons(&[addon], &output).expect("export");
        assert_eq!(exported.version, 1);
        assert_eq!(exported.addons.len(), 1);
        assert_eq!(exported.addons[0].source, TransferSourceKind::Wago);

        let reparsed = parse_import_file(&output).expect("parse import");
        assert_eq!(reparsed, exported);
    }

    #[test]
    fn analyze_import_classifies_duplicates_and_existing_by_owned_folders() {
        let transfer = TransferFile {
            version: 1,
            exported_at: "2026-04-05T00:00:00Z".to_string(),
            addons: vec![
                super::TransferAddon {
                    name: "Deadly Boss Mods".to_string(),
                    folder: "DBM-Core".to_string(),
                    source: TransferSourceKind::Wago,
                    url: Some("https://addons.wago.io/addons/dbm-core".to_string()),
                    owned_folders: Some(vec!["DBM-Party".to_string()]),
                    reinstallable: true,
                },
                super::TransferAddon {
                    name: "Deadly Boss Mods Alt".to_string(),
                    folder: "DBM-Core".to_string(),
                    source: TransferSourceKind::Wago,
                    url: Some("https://addons.wago.io/addons/dbm-core".to_string()),
                    owned_folders: None,
                    reinstallable: true,
                },
                super::TransferAddon {
                    name: "Manual".to_string(),
                    folder: "ManualAddon".to_string(),
                    source: TransferSourceKind::Manual,
                    url: None,
                    owned_folders: None,
                    reinstallable: false,
                },
            ],
        };

        let existing = HashSet::from(["weakaurasarchive".to_string()]);
        let analysis: ImportAnalysis = analyze_import(&transfer, &existing);
        assert_eq!(analysis.to_install.len(), 1);
        assert_eq!(analysis.already_installed.len(), 0);
        assert_eq!(analysis.duplicate_addons.len(), 1);
        assert_eq!(analysis.manual_addons.len(), 1);
    }
}
