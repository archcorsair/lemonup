use std::collections::{HashMap, HashSet};
use std::path::Path;

use lemonup_core::{
    AddonRecord, GameFlavor, LemonupError, ScannedAddon, StateDatabase, UpdateStatus,
    scan_addons_dir,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckResult {
    pub(crate) addon_name: String,
    pub(crate) status: UpdateStatus,
    pub(crate) remote_version: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateRefreshSummary {
    pub(crate) target_addons: usize,
    pub(crate) scanned_addons: usize,
    pub(crate) refreshed_addons: usize,
    pub(crate) skipped_unmanaged: usize,
    pub(crate) missing_on_disk: usize,
}

pub(crate) fn build_update_checks(
    installed: &[AddonRecord],
    selectors: &[String],
) -> Result<Vec<CheckResult>, LemonupError> {
    let selected = resolve_selected_addons(installed, selectors)?;

    Ok(selected
        .into_iter()
        .map(|addon| {
            let (status, message) = determine_update_status(addon);
            CheckResult {
                addon_name: addon.folder.clone(),
                status,
                remote_version: addon.remote_version.clone(),
                message,
            }
        })
        .collect())
}

pub(crate) fn determine_update_status(addon: &AddonRecord) -> (UpdateStatus, Option<String>) {
    if addon.source == lemonup_core::SourceKind::Manual {
        return (
            UpdateStatus::Unknown,
            Some("manual addons cannot be checked yet".to_string()),
        );
    }

    match (addon.version.as_deref(), addon.remote_version.as_deref()) {
        (Some(version), Some(remote_version))
            if normalize_version(version) == normalize_version(remote_version) =>
        {
            (UpdateStatus::UpToDate, None)
        }
        (Some(_), Some(_)) => (
            UpdateStatus::UpdateAvailable,
            Some("tracked remote version differs from installed version".to_string()),
        ),
        (_, None) => (
            UpdateStatus::Unknown,
            Some("no tracked remote version yet".to_string()),
        ),
        (None, Some(_)) => (
            UpdateStatus::Unknown,
            Some("no installed version metadata is tracked yet".to_string()),
        ),
    }
}

pub(crate) fn serialize_update_status(status: UpdateStatus) -> &'static str {
    match status {
        UpdateStatus::UpToDate => "up_to_date",
        UpdateStatus::UpdateAvailable => "update_available",
        UpdateStatus::Unknown => "unknown",
        UpdateStatus::Error => "error",
    }
}

pub(crate) fn refresh_managed_update_state(
    database: &mut StateDatabase,
    addon_dir: &Path,
    dry_run: bool,
) -> lemonup_core::Result<UpdateRefreshSummary> {
    refresh_managed_update_state_for_selectors(database, addon_dir, &[], dry_run)
}

pub(crate) fn refresh_managed_update_state_for_selectors(
    database: &mut StateDatabase,
    addon_dir: &Path,
    selectors: &[String],
    dry_run: bool,
) -> lemonup_core::Result<UpdateRefreshSummary> {
    let scanned = scan_addons_dir(addon_dir, GameFlavor::Retail)?;
    let scanned_by_folder = scanned
        .iter()
        .map(|addon| (addon.folder.as_str(), addon))
        .collect::<HashMap<_, _>>();

    let installed = database.list_addons()?;
    let selected = resolve_selected_addons(&installed, selectors)?;

    let mut refreshed_addons = 0;
    let mut skipped_unmanaged = 0;
    let mut missing_on_disk = 0;

    for addon in &selected {
        if addon.source == lemonup_core::SourceKind::Manual {
            continue;
        }

        if !addon.has_authoritative_owned_folders() {
            skipped_unmanaged += 1;
            continue;
        }

        let Some(scanned_addon) = scanned_by_folder.get(addon.folder.as_str()) else {
            missing_on_disk += 1;
            continue;
        };

        let refreshed = build_managed_update_record(addon, scanned_addon);
        if !dry_run {
            database.record_managed_addon(&refreshed)?;
        }
        refreshed_addons += 1;
    }

    Ok(UpdateRefreshSummary {
        target_addons: selected.len(),
        scanned_addons: scanned.len(),
        refreshed_addons,
        skipped_unmanaged,
        missing_on_disk,
    })
}

pub(crate) fn build_managed_update_record(
    existing: &AddonRecord,
    scanned: &ScannedAddon,
) -> AddonRecord {
    let mut refreshed = AddonRecord {
        id: existing.id,
        name: scanned.name.clone(),
        folder: existing.folder.clone(),
        owned_folders: existing.owned_folders.clone(),
        ownership_source: existing.effective_ownership_source(),
        kind: if existing.kind_override {
            existing.kind
        } else {
            scanned.kind
        },
        kind_override: existing.kind_override,
        flavor: scanned.flavor,
        version: scanned.version.clone(),
        git_commit: scanned.git_commit.clone(),
        author: scanned.author.clone(),
        interface: scanned.interface.clone(),
        source: existing.source,
        source_url: existing.source_url.clone(),
        required_deps: scanned.required_deps.clone(),
        optional_deps: scanned.optional_deps.clone(),
        embedded_libs: scanned.embedded_libs.clone(),
        installed_at: existing.installed_at,
        updated_at: existing.updated_at,
        last_checked_at: existing.last_checked_at,
        remote_version: existing.remote_version.clone(),
    };
    refreshed.set_managed_owned_folders(existing.owned_folders.clone());
    refreshed
}

fn resolve_selected_addons<'a>(
    installed: &'a [AddonRecord],
    selectors: &[String],
) -> Result<Vec<&'a AddonRecord>, LemonupError> {
    if selectors.is_empty() {
        return Ok(installed.iter().collect());
    }

    let by_name_or_folder = installed
        .iter()
        .flat_map(|addon| {
            [
                (normalize_selector(&addon.folder), addon),
                (normalize_selector(&addon.name), addon),
            ]
        })
        .collect::<HashMap<_, _>>();

    let mut selected = Vec::new();
    let mut seen_folders = HashSet::new();
    let mut missing = Vec::new();

    for selector in selectors {
        let normalized = normalize_selector(selector);
        let Some(addon) = by_name_or_folder.get(&normalized).copied() else {
            missing.push(selector.clone());
            continue;
        };

        if seen_folders.insert(addon.folder.clone()) {
            selected.push(addon);
        }
    }

    if !missing.is_empty() {
        let noun = if missing.len() == 1 {
            "No tracked addon matches"
        } else {
            "No tracked addons match"
        };
        return Err(LemonupError::InvalidArgument(format!(
            "{noun}: {}. Use exact addon names or folder names, or run `lemonup check` to see the overall summary.",
            missing.join(", ")
        )));
    }

    Ok(selected)
}

fn normalize_selector(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_version(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
