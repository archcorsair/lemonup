use std::path::PathBuf;

use lemonup_core::{AddonRecord, AppConfig, ScanSummary};

use crate::app::text_input::TextInput;
use crate::app::{
    BackupPaneState, ConfigPaneState, CurseForgeAction, DashboardJobStep, DashboardJobUiState,
    DashboardSortColumn, DashboardUndoDeleteState, DashboardUpdateRunSummary, DetailMode,
    DirectInstallRequest, FindAction, FooterKeyPulse, GamesAction, HeldUpdates, HelpContext,
    InstallPaneState, PendingWagoInstallRequest, SearchPaneState, StatusMessage, TocApplyRequest,
    TocEditPane, WagoInstallConfirmation,
};
use crate::drift::DriftReport;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Quit,
    /// What the Switch game sheet, adding a game and the background checks
    /// of other games do.
    Games(GamesAction),
    /// What the Find your addons online sheet does.
    Find(FindAction),
    /// The question to mark addons Ignored in CurseForge, and its answer.
    CurseForge(CurseForgeAction),
    /// Leaves setup that was adding a game, back to the Switch game sheet.
    CancelAddingGame,
    /// Opens or closes the "quit while updating?" question.
    SetQuitQuestion(bool),
    /// Asks about updates an update run held for "Not linked" folders (nothing when empty).
    OpenHeldQuestion(HeldUpdates),
    /// Yes to that question: applies the held updates from their kept packages.
    StartApplyHeldUpdates,
    /// No to it: skips them and removes their packages.
    SkipHeldUpdates,
    SetHeldQuestionOffset(u16),
    AdvanceMotionTick,
    SetFooterKeyPulse(Option<FooterKeyPulse>),
    SetInspectOverlay(bool),
    /// Scrolls the open sheet to this body row.
    SetSheetOffset(u16),
    /// Scrolls docked Details to this many rows down.
    SetDockedDetailsOffset(u16),
    SetHelpOverlay(bool),
    SetHelpContext(HelpContext),
    SetStatus(StatusMessage),
    SetDashboardSelection(Option<usize>),
    ToggleDashboardSort(DashboardSortColumn),
    SetDashboardFilter {
        query: TextInput,
        editing: bool,
    },
    SetPendingDelete(Option<Vec<String>>),
    SetDashboardUndoDelete(Option<DashboardUndoDeleteState>),
    SetDashboardDriftReport(Option<DriftReport>),
    SetDashboardUndoInProgress(bool),
    SetDashboardDeleteInProgress(bool),
    SetDashboardUpdateInProgress(bool),
    SetDashboardAutoCheckInProgress(bool),
    SetDashboardJobUi(Option<DashboardJobUiState>),
    /// What a running check or update reported for one addon.
    ApplyDashboardJobStep(DashboardJobStep),
    SetDashboardUpdateSummary(Option<DashboardUpdateRunSummary>),
    /// Folders the last update run updated, and the ones it failed on.
    SetDashboardUpdateResults {
        updated: Vec<String>,
        failed: Vec<String>,
    },
    SetSelectedDashboardParents(Vec<String>),
    SetInstallPaneState(InstallPaneState),
    SetSearchPaneState(SearchPaneState),
    SetConfigPaneState(ConfigPaneState),
    SetPersistedConfig(AppConfig),
    SetBackupPaneState(BackupPaneState),
    SetTocEditPane(TocEditPane),
    SetTocApplyInProgress(bool),
    SetImportRunning(bool),
    /// Writes a .toc change into the addon's files in the background.
    StartTocApply {
        addon_dir: PathBuf,
        request: TocApplyRequest,
    },
    /// Reads the backup folder again, for backups made outside the Backups sheet.
    ReloadBackups,
    SetPendingWagoInstallConfirmation(Option<WagoInstallConfirmation>),
    SetWagoInstallInProgress(bool),
    SetDirectInstallInProgress(bool),
    ToggleDashboardSelection,
    SelectAllDashboardParents,
    ClearDashboardSelection,
    ToggleDashboardExpanded,
    ExpandAllDashboardRelationships,
    CollapseAllDashboardRelationships,
    CollapseDashboardExpanded,
    SetDetailMode(DetailMode),
    ReenterOnboarding,
    /// Closes setup opened again from Settings, back to the list.
    LeaveOnboarding,
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
        check_interval_secs: u64,
    },
    StartDashboardCheckSelected {
        folders: Vec<String>,
        wago_api_key: Option<String>,
        check_interval_secs: u64,
    },
    StartBackupNow {
        addon_dir: PathBuf,
        backup_dir: PathBuf,
        retention: u16,
    },
    StartBackupRestore {
        addon_dir: PathBuf,
        backup_path: PathBuf,
    },
    StartBackupDelete {
        backup_root: PathBuf,
        backup_path: PathBuf,
    },
    StartTransferImport {
        addon_dir: PathBuf,
        input_path: PathBuf,
        wago_api_key: Option<String>,
    },
    StartWagoSearch {
        query: String,
        api_key: String,
    },
    StartWagoInstall {
        addon_dir: PathBuf,
        api_key: String,
        request: PendingWagoInstallRequest,
        /// `None` asks first when folders are in the way; `Some` is the list
        /// the person said yes to replacing.
        replace_folders: Option<Vec<String>>,
    },
    /// GitHub, WoWInterface or TukUI install from the install/search overlay.
    StartDirectInstall {
        addon_dir: PathBuf,
        request: DirectInstallRequest,
        /// The folders already in AddOns the person said yes to replacing.
        replace_folders: Vec<String>,
    },
    ReplaceDashboardAddons(Vec<AddonRecord>),
    CompleteAddonScan {
        path: PathBuf,
        summary: ScanSummary,
    },
    FailAddonScan(String),
}
