use std::path::PathBuf;

use lemonup_core::{AddonRecord, ScanSummary};

use crate::app::DetailMode;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    SetStatus(String),
    SetDashboardSelection(Option<usize>),
    SetPendingDelete(Option<Vec<String>>),
    ToggleDashboardSelection,
    SelectAllDashboardParents,
    ClearDashboardSelection,
    ToggleDashboardExpanded,
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
    ReplaceDashboardAddons(Vec<AddonRecord>),
    CompleteAddonScan {
        path: PathBuf,
        summary: ScanSummary,
    },
    FailAddonScan(String),
}
