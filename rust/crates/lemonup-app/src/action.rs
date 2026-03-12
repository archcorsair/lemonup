use std::path::PathBuf;

use crate::app::Screen;
use crate::onboarding::OnboardingState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    SetScreen(Screen),
    SetStatus(String),
    SetManageSelection(Option<usize>),
    SetOnboardingState(OnboardingState),
    StartOnboardingQuickCheck,
    StartOnboardingDeepScan(PathBuf),
    CancelOnboardingScan,
    SaveAddonDir(PathBuf),
}
