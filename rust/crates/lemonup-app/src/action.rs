use std::path::PathBuf;

use lemonup_core::{AddonRecord, AppConfig, ScanSummary};

use crate::app::{
    BackupPaneState, ConfigPaneState, DashboardUndoDeleteState, DashboardUpdateRunSummary,
    DetailMode, InstallPaneState, PendingWagoInstallRequest, SearchPaneState,
    WagoInstallConfirmation,
};
use crate::drift::DriftReport;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Quit,
    AdvanceMotionTick,
    SetInspectOverlay(bool),
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
    SetConfigPaneState(ConfigPaneState),
    SetPersistedConfig(AppConfig),
    SetBackupPaneState(BackupPaneState),
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
    ReenterOnboarding,
    SetOnboardingState(OnboardingState),
    StartOnboardingQuickCheck,
    StartOnboardingDeepScan(PathBuf),
    CancelOnboardingScan,
    SaveOnboardingConfig(AppConfig),
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
        wago_api_key: Option<String>,
    },
    StartBackupNow {
        addon_dir: PathBuf,
        backup_dir: PathBuf,
        retention: u16,
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
