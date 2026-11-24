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
    /// What happened to each addon of the list, in the order they were installed.
    pub outcomes: Vec<TransferInstallOutcome>,
}

/// One addon of an import: installed, or the error that stopped it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransferInstallOutcome {
    pub name: String,
    pub source: TransferSourceKind,
    pub result: Result<(), String>,
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

/// Installs the addons of `analysis` one after another. `on_start` hears which one is next,
/// by its place in the list, before its request goes out.
pub(crate) async fn install_transfer_addons(
    database: &mut StateDatabase,
    addon_dir: &Path,
    analysis: &ImportAnalysis,
    wago_api_key: Option<&str>,
    mut on_start: impl FnMut(usize, &TransferAddon),
) -> TransferInstallSummary {
    let mut summary = TransferInstallSummary {
        installed: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
        outcomes: Vec::new(),
    };

    for (index, addon) in analysis.to_install.iter().enumerate() {
        on_start(index, addon);
        let result = match install_transfer_addon(database, addon_dir, addon, wago_api_key).await {
            Ok(warning) => {
                summary.installed += 1;
                summary.warnings.extend(warning);
                Ok(())
            }
            Err(error) => {
                summary.errors.push(format!("{}: {error}", addon.name));
                Err(error)
            }
        };
        summary.outcomes.push(TransferInstallOutcome {
            name: addon.name.clone(),
            source: addon.source.clone(),
            result,
        });
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

#[cfg(test)]
mod tests {
    use lemonup_core::{ImportAnalysis, StateDatabase, TransferAddon, TransferSourceKind};
    use tempfile::tempdir;

    use super::{TransferInstallOutcome, install_transfer_addons};

    fn addon(name: &str, source: TransferSourceKind, url: Option<&str>) -> TransferAddon {
        TransferAddon {
            name: name.to_string(),
            folder: name.to_string(),
            source,
            url: url.map(str::to_string),
            owned_folders: None,
            reinstallable: true,
        }
    }

    #[tokio::test]
    async fn each_addon_has_its_outcome_and_the_errors_keep_their_text() {
        let temp = tempdir().expect("tempdir");
        let addon_dir = temp.path().join("AddOns");
        std::fs::create_dir_all(&addon_dir).expect("addons");
        let mut database = StateDatabase::open(temp.path().join("state.sqlite")).expect("db");
        // None of these reaches a source: each fails before any download.
        let analysis = ImportAnalysis {
            to_install: vec![
                addon("NoLink", TransferSourceKind::GitHub, None),
                addon(
                    "WagoOne",
                    TransferSourceKind::Wago,
                    Some("https://addons.wago.io/x"),
                ),
                addon(
                    "Mine",
                    TransferSourceKind::Manual,
                    Some("https://example.com"),
                ),
            ],
            already_installed: Vec::new(),
            manual_addons: Vec::new(),
            duplicate_addons: Vec::new(),
            unsupported_addons: Vec::new(),
        };
        let mut started = Vec::new();
        let summary = install_transfer_addons(
            &mut database,
            &addon_dir,
            &analysis,
            None,
            |index, addon| {
                started.push((index, addon.name.clone()));
            },
        )
        .await;
        // Each addon is announced before its turn, in the order of the list.
        assert_eq!(
            started,
            [
                (0, "NoLink".to_string()),
                (1, "WagoOne".to_string()),
                (2, "Mine".to_string()),
            ]
        );
        assert_eq!(summary.installed, 0);
        assert_eq!(
            summary.errors,
            [
                "NoLink: missing install URL",
                "WagoOne: missing Wago API key",
                "Mine: exports of addons that are not linked cannot be reinstalled",
            ]
        );
        assert_eq!(
            summary.outcomes,
            [
                TransferInstallOutcome {
                    name: "NoLink".to_string(),
                    source: TransferSourceKind::GitHub,
                    result: Err("missing install URL".to_string()),
                },
                TransferInstallOutcome {
                    name: "WagoOne".to_string(),
                    source: TransferSourceKind::Wago,
                    result: Err("missing Wago API key".to_string()),
                },
                TransferInstallOutcome {
                    name: "Mine".to_string(),
                    source: TransferSourceKind::Manual,
                    result: Err(
                        "exports of addons that are not linked cannot be reinstalled".to_string()
                    ),
                },
            ]
        );
    }
}
