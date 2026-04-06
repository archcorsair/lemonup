use std::path::PathBuf;

use lemonup_core::{AppConfig, ThemeMode, suggested_scan_roots};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    Theme,
    Directory,
    Wago,
    Settings,
    Review,
}

impl OnboardingStep {
    pub const ALL: [Self; 5] = [
        Self::Theme,
        Self::Directory,
        Self::Wago,
        Self::Settings,
        Self::Review,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Theme => 0,
            Self::Directory => 1,
            Self::Wago => 2,
            Self::Settings => 3,
            Self::Review => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Directory => "Directory",
            Self::Wago => "Wago",
            Self::Settings => "Settings",
            Self::Review => "Review",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL
            .get(self.index() + 1)
            .copied()
            .unwrap_or(Self::Review)
    }

    pub fn previous(self) -> Self {
        self.index()
            .checked_sub(1)
            .and_then(|index| Self::ALL.get(index).copied())
            .unwrap_or(Self::Theme)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingSettingsField {
    BackupWtf,
    BackupRetention,
    ShowLibs,
}

impl OnboardingSettingsField {
    pub const ALL: [Self; 3] = [Self::BackupWtf, Self::BackupRetention, Self::ShowLibs];

    pub fn index(self) -> usize {
        match self {
            Self::BackupWtf => 0,
            Self::BackupRetention => 1,
            Self::ShowLibs => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BackupWtf => "Back up WTF",
            Self::BackupRetention => "Backup retention",
            Self::ShowLibs => "Show libraries",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL
            .get(self.index() + 1)
            .copied()
            .unwrap_or(Self::ShowLibs)
    }

    pub fn previous(self) -> Self {
        self.index()
            .checked_sub(1)
            .and_then(|index| Self::ALL.get(index).copied())
            .unwrap_or(Self::BackupWtf)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgressState {
    pub dirs_scanned: usize,
    pub current_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundState {
    pub path: String,
    pub selected_action: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingPhase {
    Bootstrapping,
    QuickChecking,
    Ready,
    DeepScanning(ScanProgressState),
    Found(FoundState),
    Error(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundAction {
    UseThisPath,
    EnterDifferentPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingTaskEvent {
    QuickCheckFinished(Option<PathBuf>),
    DeepScanProgress(ScanProgressState),
    DeepScanFinished(std::result::Result<Option<PathBuf>, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub phase: OnboardingPhase,
    pub draft: AppConfig,
    pub input: String,
    pub suggestions: Vec<String>,
    pub selected_suggestion: Option<usize>,
    pub settings_selection: OnboardingSettingsField,
    pub is_editing: bool,
}

impl OnboardingState {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::from_config(AppConfig::new_unconfigured())
    }

    #[cfg(test)]
    pub fn with_input(input: String) -> Self {
        let mut draft = AppConfig::new_unconfigured();
        draft.addon_dir = Some(PathBuf::from(input.clone()));
        Self::from_config_with_input(draft, input)
    }

    pub fn from_config(mut draft: AppConfig) -> Self {
        let suggestions = suggested_scan_roots()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let input = draft
            .addon_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .or_else(|| suggestions.first().cloned())
            .unwrap_or_default();
        draft.addon_dir = if input.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(input.clone()))
        };

        Self {
            step: OnboardingStep::Theme,
            phase: OnboardingPhase::Bootstrapping,
            draft,
            input,
            suggestions,
            selected_suggestion: Some(0),
            settings_selection: OnboardingSettingsField::BackupWtf,
            is_editing: false,
        }
    }

    pub fn from_config_with_input(mut draft: AppConfig, input: String) -> Self {
        let mut next = Self::from_config(draft.clone());
        next.phase = OnboardingPhase::Ready;
        next.input = input.clone();
        next.selected_suggestion = None;
        draft.addon_dir = Some(PathBuf::from(input));
        next.draft = draft;
        next
    }

    pub fn ready(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::Ready;
        next.is_editing = false;
        next
    }

    pub fn begin_quick_check(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::QuickChecking;
        next
    }

    pub fn start_deep_scan(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::DeepScanning(ScanProgressState {
            dirs_scanned: 0,
            current_path: next.input.clone(),
        });
        next.is_editing = false;
        next
    }

    pub fn set_scan_progress(&self, progress: ScanProgressState) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::DeepScanning(progress);
        next
    }

    pub fn found(&self, path: PathBuf) -> Self {
        let mut next = self.clone();
        let rendered = path.display().to_string();
        next.input = rendered.clone();
        next.phase = OnboardingPhase::Found(FoundState {
            path: rendered,
            selected_action: 0,
        });
        next.is_editing = false;
        next
    }

    pub fn error(&self, message: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::Error(message.into());
        next.is_editing = false;
        next
    }

    pub fn cancelled(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::Cancelled;
        next.is_editing = false;
        next
    }

    pub fn next_step(&self) -> Self {
        let mut next = self.clone();
        next.step = next.step.next();
        next.is_editing = false;
        next
    }

    pub fn previous_step(&self) -> Self {
        let mut next = self.clone();
        next.step = next.step.previous();
        next.is_editing = false;
        next
    }

    pub fn toggle_theme(&self) -> Self {
        let mut next = self.clone();
        next.draft.theme = match next.draft.theme {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        next
    }

    pub fn begin_directory_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = true;
        if !matches!(
            next.phase,
            OnboardingPhase::Ready | OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
        ) {
            next.phase = OnboardingPhase::Ready;
        }
        next
    }

    pub fn begin_wago_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = true;
        next
    }

    pub fn stop_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = false;
        if next.step == OnboardingStep::Directory
            && matches!(
                next.phase,
                OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
            )
        {
            next.phase = OnboardingPhase::Ready;
        }
        next
    }

    pub fn insert_char(&self, value: char) -> Self {
        let mut next = self.clone();
        match next.step {
            OnboardingStep::Directory => next.input.push(value),
            OnboardingStep::Wago => {
                let value_ref = next.draft.wago_api_key.get_or_insert_with(String::new);
                value_ref.push(value);
            }
            _ => {}
        }
        next
    }

    pub fn backspace(&self) -> Self {
        let mut next = self.clone();
        match next.step {
            OnboardingStep::Directory => {
                next.input.pop();
            }
            OnboardingStep::Wago => {
                if let Some(value) = &mut next.draft.wago_api_key {
                    value.pop();
                    if value.is_empty() {
                        next.draft.wago_api_key = None;
                    }
                }
            }
            _ => {}
        }
        next
    }

    pub fn next_suggestion(&self) -> Self {
        let mut next = self.clone();
        if next.suggestions.is_empty() {
            return next;
        }

        let selected = match next.selected_suggestion {
            Some(index) => (index + 1) % next.suggestions.len(),
            None => 0,
        };
        next.selected_suggestion = Some(selected);
        if let Some(suggestion) = next.suggestions.get(selected) {
            next.input = suggestion.clone();
        }
        if matches!(
            next.phase,
            OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
        ) {
            next.phase = OnboardingPhase::Ready;
        }
        next
    }

    pub fn previous_suggestion(&self) -> Self {
        let mut next = self.clone();
        if next.suggestions.is_empty() {
            return next;
        }

        let selected = match next.selected_suggestion {
            Some(0) | None => next.suggestions.len() - 1,
            Some(index) => index - 1,
        };
        next.selected_suggestion = Some(selected);
        if let Some(suggestion) = next.suggestions.get(selected) {
            next.input = suggestion.clone();
        }
        if matches!(
            next.phase,
            OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
        ) {
            next.phase = OnboardingPhase::Ready;
        }
        next
    }

    pub fn next_found_action(&self) -> Self {
        let mut next = self.clone();
        if let OnboardingPhase::Found(found) = &mut next.phase {
            found.selected_action = (found.selected_action + 1) % 2;
        }
        next
    }

    pub fn previous_found_action(&self) -> Self {
        let mut next = self.clone();
        if let OnboardingPhase::Found(found) = &mut next.phase {
            found.selected_action = if found.selected_action == 0 { 1 } else { 0 };
        }
        next
    }

    pub fn selected_found_action(&self) -> FoundAction {
        match &self.phase {
            OnboardingPhase::Found(found) => match found.selected_action {
                0 => FoundAction::UseThisPath,
                _ => FoundAction::EnterDifferentPath,
            },
            _ => FoundAction::UseThisPath,
        }
    }

    pub fn apply_directory_to_draft(&self) -> Self {
        let mut next = self.clone();
        next.draft.addon_dir = Some(PathBuf::from(next.input.trim()));
        next
    }

    pub fn next_settings_field(&self) -> Self {
        let mut next = self.clone();
        next.settings_selection = next.settings_selection.next();
        next
    }

    pub fn previous_settings_field(&self) -> Self {
        let mut next = self.clone();
        next.settings_selection = next.settings_selection.previous();
        next
    }

    pub fn adjust_settings_field(&self, delta: i16) -> Self {
        let mut next = self.clone();
        match next.settings_selection {
            OnboardingSettingsField::BackupWtf => next.draft.backup_wtf = !next.draft.backup_wtf,
            OnboardingSettingsField::BackupRetention => {
                let current = i32::from(next.draft.backup_retention);
                let updated = (current + i32::from(delta)).clamp(1, 30) as u16;
                next.draft.backup_retention = updated;
            }
            OnboardingSettingsField::ShowLibs => next.draft.show_libs = !next.draft.show_libs,
        }
        next
    }

    pub fn wago_api_key_value(&self) -> &str {
        self.draft.wago_api_key.as_deref().unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{AppConfig, ThemeMode};

    use super::{
        FoundAction, OnboardingPhase, OnboardingSettingsField, OnboardingState, OnboardingStep,
    };

    #[test]
    fn with_input_prefills_without_suggestion_selection() {
        let state = OnboardingState::with_input("D:\\Sandbox\\Interface\\AddOns".to_string());
        assert_eq!(state.phase, OnboardingPhase::Ready);
        assert_eq!(state.input, "D:\\Sandbox\\Interface\\AddOns");
        assert_eq!(state.selected_suggestion, None);
    }

    #[test]
    fn suggestion_navigation_updates_input() {
        let state = OnboardingState {
            suggestions: vec!["C:\\".to_string(), "D:\\".to_string()],
            input: "C:\\".to_string(),
            selected_suggestion: Some(0),
            is_editing: false,
            phase: OnboardingPhase::Ready,
            step: OnboardingStep::Directory,
            draft: AppConfig::new_unconfigured(),
            settings_selection: OnboardingSettingsField::BackupWtf,
        };

        let next = state.next_suggestion();
        assert_eq!(next.selected_suggestion, Some(1));
        assert_eq!(next.input, "D:\\");
    }

    #[test]
    fn found_action_wraps() {
        let state = OnboardingState::new().found(PathBuf::from("C:\\Games\\WoW"));
        let rotated = state.next_found_action().next_found_action();
        assert_eq!(rotated.selected_found_action(), FoundAction::UseThisPath);
    }

    #[test]
    fn theme_toggle_updates_draft() {
        let state = OnboardingState::new();
        assert_eq!(state.toggle_theme().draft.theme, ThemeMode::Light);
    }
}
