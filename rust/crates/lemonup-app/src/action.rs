use std::path::PathBuf;

use lemonup_core::{AddonRecord, ScanSummary};

use crate::app::{DashboardUpdateRunSummary, DetailMode};
use crate::drift::DriftReport;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    SetStatus(String),
    SetDashboardSelection(Option<usize>),
    SetPendingDelete(Option<Vec<String>>),
    SetDashboardDriftReport(Option<DriftReport>),
    SetDashboardUpdateInProgress(bool),
    SetDashboardUpdateSummary(Option<DashboardUpdateRunSummary>),
    SetSelectedDashboardParents(Vec<String>),
    ToggleDashboardSelection,
    SelectAllDashboardParents,
    ClearDashboardSelection,
    ToggleDashboardExpanded,
    ExpandAllDashboardRelationships,
    CollapseAllDashboardRelationships,
    CollapseDashboardExpanded,
    SetDetailMode(DetailMode),
    SetOnboardingState(OnboardingState),
    StartOnboardingQuickCheck,
    StartOnboardingDeepScan(PathBuf),
    CancelOnboardingScan,
    SaveAddonDir(PathBuf),
    StartAddonScan(PathBuf),
    StartDashboardDelete {
        addon_dir: PathBuf,
        folders: Vec<String>,
    },
    StartDashboardUpdateSelected {
        addon_dir: PathBuf,
        folders: Vec<String>,
    },
    ReplaceDashboardAddons(Vec<AddonRecord>),
    CompleteAddonScan {
        path: PathBuf,
        summary: ScanSummary,
    },
    FailAddonScan(String),
}
