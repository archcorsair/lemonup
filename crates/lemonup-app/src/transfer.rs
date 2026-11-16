use std::collections::HashSet;
use std::fs;
use std::path::Path;

use lemonup_core::{AddonRecord, ImportAnalysis, StateDatabase, TransferAddon, TransferSourceKind};

use crate::github::install_github_addon;
use crate::tukui::install_tukui_addon;
use crate::wago::{WagoStability, install_wago_addon};
use crate::wowinterface::install_wowinterface_addon;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransferInstallSummary {
    pub installed: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) fn collect_existing_import_folders(
    addon_dir: Option<&Path>,
    tracked: &[AddonRecord],
) -> Result<HashSet<String>, String> {
    let mut folders = tracked
        .iter()
        .flat_map(|addon| {
            std::iter::once(addon.folder.clone())
                .chain(addon.owned_folders.iter().map(|owned| owned.name.clone()))
        })
        .map(|folder| folder.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    if let Some(addon_dir) = addon_dir
        && addon_dir.exists()
    {
        for entry in fs::read_dir(addon_dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                folders.insert(entry.file_name().to_string_lossy().to_ascii_lowercase());
            }
        }
    }

    Ok(folders)
}

pub(crate) async fn install_transfer_addons(
    database: &mut StateDatabase,
    addon_dir: &Path,
    analysis: &ImportAnalysis,
    wago_api_key: Option<&str>,
) -> TransferInstallSummary {
    let mut summary = TransferInstallSummary {
        installed: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    for addon in &analysis.to_install {
        match install_transfer_addon(database, addon_dir, addon, wago_api_key).await {
            Ok(warning) => {
                summary.installed += 1;
                summary.warnings.extend(warning);
            }
            Err(error) => summary.errors.push(format!("{}: {error}", addon.name)),
        }
    }

    summary
}

async fn install_transfer_addon(
    database: &mut StateDatabase,
    addon_dir: &Path,
    addon: &TransferAddon,
    wago_api_key: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(url) = addon.url.as_deref() else {
        return Err("missing install URL".to_string());
    };

    match addon.source {
        TransferSourceKind::GitHub => install_github_addon(database, addon_dir, url, false)
            .await
            .map(|_| None),
        TransferSourceKind::Tukui => install_tukui_addon(database, addon_dir, url, false)
            .await
            .map(|_| None),
        TransferSourceKind::Wowinterface => {
            install_wowinterface_addon(database, addon_dir, url, false)
                .await
                .map(|summary| summary.warning)
        }
        TransferSourceKind::Wago => {
            let Some(api_key) = wago_api_key else {
                return Err("missing Wago API key".to_string());
            };
            install_wago_addon(
                database,
                addon_dir,
                url,
                api_key,
                WagoStability::Stable,
                false,
            )
            .await
            .map(|_| None)
        }
        TransferSourceKind::Manual => {
            Err("exports of addons that are not linked cannot be reinstalled".to_string())
        }
        TransferSourceKind::Other(_) => Err(format!(
            "this version of LemonUp can't install addons from '{}'",
            addon.source.as_str()
        )),
    }
}
