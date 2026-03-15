use std::collections::{HashMap, HashSet};

use lemonup_core::{AddonRecord, ScannedAddon};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriftReport {
    pub(crate) imported_disk_only_folders: Vec<String>,
    pub(crate) removed_missing_records: Vec<String>,
    pub(crate) orphaned_owned_children_on_disk: Vec<OwnedChildDrift>,
    pub(crate) parent_missing_owned_children: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedChildDrift {
    pub(crate) parent_folder: String,
    pub(crate) child_folder: String,
}

impl DriftReport {
    pub(crate) fn empty() -> Self {
        Self {
            imported_disk_only_folders: Vec::new(),
            removed_missing_records: Vec::new(),
            orphaned_owned_children_on_disk: Vec::new(),
            parent_missing_owned_children: HashMap::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.imported_disk_only_folders.is_empty()
            && self.removed_missing_records.is_empty()
            && self.orphaned_owned_children_on_disk.is_empty()
            && self.parent_missing_owned_children.is_empty()
    }

    pub(crate) fn missing_owned_children_for(&self, parent_folder: &str) -> &[String] {
        self.parent_missing_owned_children
            .get(parent_folder)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub(crate) fn compute_drift_report(
    tracked: &[AddonRecord],
    scanned: &[ScannedAddon],
) -> DriftReport {
    let tracked_by_folder = tracked
        .iter()
        .map(|addon| (addon.folder.as_str(), addon))
        .collect::<HashMap<_, _>>();
    let scanned_folders = scanned
        .iter()
        .map(|addon| addon.folder.as_str())
        .collect::<HashSet<_>>();

    let mut report = DriftReport::empty();

    report.imported_disk_only_folders = scanned
        .iter()
        .filter(|addon| !tracked_by_folder.contains_key(addon.folder.as_str()))
        .map(|addon| addon.folder.clone())
        .collect();
    report.imported_disk_only_folders.sort();

    report.removed_missing_records = tracked
        .iter()
        .filter(|addon| !scanned_folders.contains(addon.folder.as_str()))
        .map(|addon| addon.folder.clone())
        .collect();
    report.removed_missing_records.sort();

    for addon in tracked {
        if !addon.has_authoritative_owned_folders() {
            continue;
        }

        let parent_on_disk = scanned_folders.contains(addon.folder.as_str());
        let mut missing_owned_children = addon
            .owned_folders
            .iter()
            .filter(|owned_folder| !scanned_folders.contains(owned_folder.name.as_str()))
            .map(|owned_folder| owned_folder.name.clone())
            .collect::<Vec<_>>();
        missing_owned_children.sort();

        if parent_on_disk && !missing_owned_children.is_empty() {
            report
                .parent_missing_owned_children
                .insert(addon.folder.clone(), missing_owned_children);
        }

        if !parent_on_disk {
            for owned_folder in &addon.owned_folders {
                if scanned_folders.contains(owned_folder.name.as_str()) {
                    report
                        .orphaned_owned_children_on_disk
                        .push(OwnedChildDrift {
                            parent_folder: addon.folder.clone(),
                            child_folder: owned_folder.name.clone(),
                        });
                }
            }
        }
    }

    report
        .orphaned_owned_children_on_disk
        .sort_by(|left, right| {
            left.parent_folder
                .cmp(&right.parent_folder)
                .then_with(|| left.child_folder.cmp(&right.child_folder))
        });

    report
}

#[cfg(test)]
mod tests {
    use super::compute_drift_report;
    use lemonup_core::{AddonKind, AddonRecord, GameFlavor, OwnedFolder, ScannedAddon, SourceKind};

    #[test]
    fn drift_report_captures_disk_only_missing_record_and_missing_owned_child() {
        let mut tracked = AddonRecord::new("ElvUI", "ElvUI", SourceKind::Wago);
        tracked.set_managed_owned_folders(vec![OwnedFolder {
            name: "ElvUI_OptionsUI".to_string(),
        }]);

        let scanned = vec![
            ScannedAddon {
                name: "ElvUI".to_string(),
                folder: "ElvUI".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: None,
                git_commit: None,
                author: None,
                interface: None,
                source: SourceKind::Manual,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
            },
            ScannedAddon {
                name: "WeakAuras".to_string(),
                folder: "WeakAuras".to_string(),
                owned_folders: Vec::new(),
                kind: AddonKind::Addon,
                flavor: GameFlavor::Retail,
                version: None,
                git_commit: None,
                author: None,
                interface: None,
                source: SourceKind::Manual,
                required_deps: Vec::new(),
                optional_deps: Vec::new(),
                embedded_libs: Vec::new(),
            },
        ];

        let report = compute_drift_report(&[tracked], &scanned);
        assert_eq!(
            report.imported_disk_only_folders,
            vec!["WeakAuras".to_string()]
        );
        assert!(report.removed_missing_records.is_empty());
        assert_eq!(
            report.missing_owned_children_for("ElvUI"),
            ["ElvUI_OptionsUI".to_string()]
        );
    }

    #[test]
    fn drift_report_flags_orphaned_owned_child_when_parent_is_missing() {
        let mut tracked = AddonRecord::new("DBM", "DBM-Core", SourceKind::Wago);
        tracked.set_managed_owned_folders(vec![OwnedFolder {
            name: "DBM-Naxx".to_string(),
        }]);

        let scanned = vec![ScannedAddon {
            name: "DBM Naxx".to_string(),
            folder: "DBM-Naxx".to_string(),
            owned_folders: Vec::new(),
            kind: AddonKind::Addon,
            flavor: GameFlavor::Retail,
            version: None,
            git_commit: None,
            author: None,
            interface: None,
            source: SourceKind::Manual,
            required_deps: Vec::new(),
            optional_deps: Vec::new(),
            embedded_libs: Vec::new(),
        }];

        let report = compute_drift_report(&[tracked], &scanned);
        assert_eq!(report.removed_missing_records, vec!["DBM-Core".to_string()]);
        assert_eq!(report.orphaned_owned_children_on_disk.len(), 1);
        assert_eq!(
            report.orphaned_owned_children_on_disk[0].child_folder,
            "DBM-Naxx"
        );
    }
}
