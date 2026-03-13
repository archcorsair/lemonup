use std::path::PathBuf;

use crate::app::DetailMode;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    SetStatus(String),
    SetDashboardSelection(Option<usize>),
    SetDetailMode(DetailMode),
    SetOnboardingState(OnboardingState),
    StartOnboardingQuickCheck,
    StartOnboardingDeepScan(PathBuf),
    CancelOnboardingScan,
    SaveAddonDir(PathBuf),
}
