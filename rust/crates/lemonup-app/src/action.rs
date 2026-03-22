use std::path::PathBuf;

use lemonup_core::{AddonRecord, ScanSummary};

use crate::app::{
    DashboardUndoDeleteState, DashboardUpdateRunSummary, DetailMode, InstallPaneState,
    PendingWagoInstallRequest, SearchPaneState, WagoInstallConfirmation,
};
use crate::drift::DriftReport;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    SetStatus(String),
    SetDashboardSelection(Option<usize>),
    SetPendingDelete(Option<Vec<String>>),
    SetDashboardUndoDelete(Option<DashboardUndoDeleteState>),
    SetDashboardDriftReport(Option<DriftReport>),
    SetDashboardUndoInProgress(bool),
    SetDashboardUpdateInProgress(bool),
    SetDashboardUpdateSummary(Option<DashboardUpdateRunSummary>),
    SetSelectedDashboardParents(Vec<String>),
    SetInstallPaneState(InstallPaneState),
    SetSearchPaneState(SearchPaneState),
    SetPendingWagoInstallConfirmation(Option<WagoInstallConfirmation>),
    SetWagoInstallInProgress(bool),
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
        trash_dir: PathBuf,
        folders: Vec<String>,
    },
    StartDashboardUndoDelete {
        undo: DashboardUndoDeleteState,
    },
    StartDashboardUpdateSelected {
        addon_dir: PathBuf,
        folders: Vec<String>,
    },
    StartWagoSearch {
        query: String,
        api_key: String,
    },
    StartWagoInstall {
        addon_dir: PathBuf,
        api_key: String,
        request: PendingWagoInstallRequest,
        allow_replace: bool,
    },
    ReplaceDashboardAddons(Vec<AddonRecord>),
    CompleteAddonScan {
        path: PathBuf,
        summary: ScanSummary,
    },
    FailAddonScan(String),
}
