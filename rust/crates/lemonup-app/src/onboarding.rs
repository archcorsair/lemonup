use std::path::PathBuf;

use lemonup_core::suggested_scan_roots;

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
    ScanAnotherLocation,
    EditPathManually,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingTaskEvent {
    QuickCheckFinished(Option<PathBuf>),
    DeepScanProgress(ScanProgressState),
    DeepScanFinished(std::result::Result<Option<PathBuf>, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingState {
    pub phase: OnboardingPhase,
    pub input: String,
    pub suggestions: Vec<String>,
    pub selected_suggestion: Option<usize>,
    pub is_editing: bool,
}

impl OnboardingState {
    pub fn new() -> Self {
        let suggestions = suggested_scan_roots()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let input = suggestions.first().cloned().unwrap_or_default();

        Self {
            phase: OnboardingPhase::Bootstrapping,
            input,
            suggestions,
            selected_suggestion: Some(0),
            is_editing: false,
        }
    }

    pub fn begin_quick_check(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::QuickChecking;
        next
    }

    pub fn ready(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::Ready;
        next.is_editing = false;
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

    pub fn begin_editing(&self) -> Self {
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

    pub fn stop_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = false;
        if matches!(
            next.phase,
            OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
        ) {
            next.phase = OnboardingPhase::Ready;
        }
        next
    }

    pub fn insert_char(&self, value: char) -> Self {
        let mut next = self.clone();
        next.input.push(value);
        next
    }

    pub fn backspace(&self) -> Self {
        let mut next = self.clone();
        next.input.pop();
        next
    }

    pub fn clear_status_to_ready(&self) -> Self {
        let mut next = self.clone();
        if matches!(
            next.phase,
            OnboardingPhase::Error(_) | OnboardingPhase::Cancelled
        ) {
            next.phase = OnboardingPhase::Ready;
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
            found.selected_action = (found.selected_action + 1) % 3;
        }
        next
    }

    pub fn previous_found_action(&self) -> Self {
        let mut next = self.clone();
        if let OnboardingPhase::Found(found) = &mut next.phase {
            found.selected_action = if found.selected_action == 0 {
                2
            } else {
                found.selected_action - 1
            };
        }
        next
    }

    pub fn selected_found_action(&self) -> FoundAction {
        match &self.phase {
            OnboardingPhase::Found(found) => match found.selected_action {
                0 => FoundAction::UseThisPath,
                1 => FoundAction::ScanAnotherLocation,
                _ => FoundAction::EditPathManually,
            },
            _ => FoundAction::UseThisPath,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{FoundAction, OnboardingPhase, OnboardingState};

    #[test]
    fn suggestion_navigation_updates_input() {
        let state = OnboardingState {
            suggestions: vec!["C:\\".to_string(), "D:\\".to_string()],
            input: "C:\\".to_string(),
            selected_suggestion: Some(0),
            is_editing: false,
            phase: OnboardingPhase::Ready,
        };

        let next = state.next_suggestion();
        assert_eq!(next.selected_suggestion, Some(1));
        assert_eq!(next.input, "D:\\");
    }

    #[test]
    fn found_action_wraps() {
        let state = OnboardingState::new().found(PathBuf::from("C:\\Games\\WoW"));
        let rotated = state
            .next_found_action()
            .next_found_action()
            .next_found_action();
        assert_eq!(rotated.selected_found_action(), FoundAction::UseThisPath);
    }
}
