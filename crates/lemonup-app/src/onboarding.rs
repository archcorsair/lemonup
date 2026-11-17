//! Setup: three steps, Find your game, Wago.io and Preferences.
//!
//! Step 1 searches on its own first. What it finds is listed by game
//! version with an addon count; when it finds nothing it offers plain
//! choices, one of them a folder box that takes any folder of the game.

use std::path::{Path, PathBuf};

use lemonup_core::{AppConfig, suggested_scan_roots};

use crate::app::text_input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    FindGame,
    Wago,
    Preferences,
}

impl OnboardingStep {
    pub const ALL: [Self; 3] = [Self::FindGame, Self::Wago, Self::Preferences];

    pub fn index(self) -> usize {
        match self {
            Self::FindGame => 0,
            Self::Wago => 1,
            Self::Preferences => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FindGame => "Find your game",
            Self::Wago => "Wago.io",
            Self::Preferences => "Preferences",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL
            .get(self.index() + 1)
            .copied()
            .unwrap_or(Self::Preferences)
    }

    pub fn previous(self) -> Self {
        self.index()
            .checked_sub(1)
            .and_then(|index| Self::ALL.get(index).copied())
            .unwrap_or(Self::FindGame)
    }
}

/// The rows step 3 shows: three settings worded as the Settings sheet
/// words them, and, when two games were found, whether to set up the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceField {
    Theme,
    BackupWtf,
    BackupRetention,
    /// "Also set up <other game>". Shown only when the other game was found
    /// and no game uses it yet.
    AlsoSetUp,
}

impl PreferenceField {
    /// The settings rows, which are always there.
    pub const SETTINGS: [Self; 3] = [Self::Theme, Self::BackupWtf, Self::BackupRetention];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::BackupWtf => "Back up my game settings before updates",
            Self::BackupRetention => "Backups to keep",
            Self::AlsoSetUp => "Also set up my other game",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgressState {
    pub dirs_scanned: usize,
    pub current_path: String,
}

/// One game version found: its AddOns folder and the addons in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundInstall {
    pub addons_dir: PathBuf,
    pub addons: usize,
}

/// The game versions found, then "Somewhere else…".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundState {
    pub installs: Vec<FoundInstall>,
    pub selected: usize,
}

impl FoundState {
    pub fn choice_count(&self) -> usize {
        self.installs.len() + 1
    }

    /// The selected install; `None` on "Somewhere else…".
    pub fn selected_install(&self) -> Option<&FoundInstall> {
        self.installs.get(self.selected)
    }
}

/// A choice when nothing was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotFoundChoice {
    TypeFolder,
    Search(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFoundState {
    /// Where LemonUp looked, or why it stopped, as a sentence.
    pub note: String,
    pub choices: Vec<NotFoundChoice>,
    pub selected: usize,
}

impl NotFoundState {
    pub fn selected_choice(&self) -> Option<&NotFoundChoice> {
        self.choices.get(self.selected)
    }
}

/// The folder box. `back` is the screen esc returns to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypingState {
    pub error: Option<String>,
    pub back: Box<OnboardingPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingPhase {
    Bootstrapping,
    QuickChecking,
    DeepScanning(ScanProgressState),
    Found(FoundState),
    NotFound(NotFoundState),
    Typing(TypingState),
}

impl OnboardingPhase {
    /// Looking for the game: the usual places, then a folder search.
    pub fn is_searching(&self) -> bool {
        matches!(
            self,
            Self::Bootstrapping | Self::QuickChecking | Self::DeepScanning(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingTaskEvent {
    QuickCheckFinished(Vec<PathBuf>),
    DeepScanProgress(ScanProgressState),
    DeepScanFinished(std::result::Result<Vec<PathBuf>, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub phase: OnboardingPhase,
    pub draft: AppConfig,
    /// The folder the current (or last) search started from. The search
    /// that starts on its own uses the home folder.
    pub scan_root: PathBuf,
    /// The game folder box on step 1. It starts empty.
    pub folder_box: TextInput,
    /// The Wago.io key box on step 2; its text is the draft's key.
    pub key_box: TextInput,
    pub preference: PreferenceField,
    /// The AddOns folder of the other game, when step 3 offers to set it up
    /// too: Setup found both Retail and WoW Forever and no game uses the
    /// other one yet.
    pub other_game: Option<PathBuf>,
    /// "Also set up" is on. It starts off: it creates a second game, which
    /// someone who only plays one should not have to undo.
    pub set_up_other_game: bool,
    /// "Quit setup?" is open.
    pub quit_question: bool,
    /// Why the last save failed, shown on step 3.
    pub problem: Option<String>,
    /// The usual install places were checked this time.
    pub checked_usual_places: bool,
}

impl OnboardingState {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::from_config(AppConfig::new_unconfigured())
    }

    pub fn from_config(draft: AppConfig) -> Self {
        let scan_root = suggested_scan_roots()
            .into_iter()
            .next()
            .unwrap_or_default();
        let key_box = TextInput::new(draft.wago_api_key.clone().unwrap_or_default());
        Self {
            step: OnboardingStep::FindGame,
            phase: OnboardingPhase::Bootstrapping,
            draft,
            scan_root,
            folder_box: TextInput::default(),
            key_box,
            preference: PreferenceField::Theme,
            other_game: None,
            set_up_other_game: false,
            quit_question: false,
            problem: None,
            checked_usual_places: false,
        }
    }

    /// Whether a text box has focus: the folder box, or the key box.
    pub fn text_box_focused(&self) -> bool {
        match self.step {
            OnboardingStep::FindGame => matches!(self.phase, OnboardingPhase::Typing(_)),
            OnboardingStep::Wago => true,
            OnboardingStep::Preferences => false,
        }
    }

    pub fn begin_quick_check(&self) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::QuickChecking;
        next.checked_usual_places = true;
        next
    }

    pub fn start_deep_scan(&self, root: PathBuf) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::DeepScanning(ScanProgressState {
            dirs_scanned: 0,
            current_path: root.display().to_string(),
        });
        next.scan_root = root;
        next
    }

    pub fn set_scan_progress(&self, progress: ScanProgressState) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::DeepScanning(progress);
        next
    }

    /// The installs to choose from; `selected` is preselected.
    pub fn found(&self, installs: Vec<FoundInstall>, selected: usize) -> Self {
        let mut next = self.clone();
        let selected = selected.min(installs.len());
        next.phase = OnboardingPhase::Found(FoundState { installs, selected });
        next
    }

    pub fn not_found(&self, note: impl Into<String>, choices: Vec<NotFoundChoice>) -> Self {
        let mut next = self.clone();
        next.phase = OnboardingPhase::NotFound(NotFoundState {
            note: note.into(),
            choices,
            selected: 0,
        });
        next
    }

    /// Opens the folder box, empty; esc goes back to the screen it came from.
    pub fn begin_typing(&self) -> Self {
        let mut next = self.clone();
        let back = match &next.phase {
            OnboardingPhase::Typing(typing) => typing.back.clone(),
            phase => Box::new(phase.clone()),
        };
        next.phase = OnboardingPhase::Typing(TypingState { error: None, back });
        next.folder_box = TextInput::default();
        next
    }

    /// Leaves the folder box for the screen it was opened from.
    pub fn stop_typing(&self) -> Self {
        let mut next = self.clone();
        if let OnboardingPhase::Typing(typing) = &next.phase {
            next.phase = (*typing.back).clone();
        }
        next
    }

    pub fn typing_error(&self, message: impl Into<String>) -> Self {
        let mut next = self.clone();
        if let OnboardingPhase::Typing(typing) = &mut next.phase {
            typing.error = Some(message.into());
        }
        next
    }

    /// Step 1's choice moves down (`forward`) or up, wrapping.
    pub fn move_choice(&self, forward: bool) -> Self {
        let mut next = self.clone();
        let (selected, count) = match &mut next.phase {
            OnboardingPhase::Found(found) => {
                let count = found.choice_count();
                (&mut found.selected, count)
            }
            OnboardingPhase::NotFound(not_found) => {
                let count = not_found.choices.len();
                (&mut not_found.selected, count)
            }
            _ => return next,
        };
        if count > 0 {
            *selected = if forward {
                (*selected + 1) % count
            } else {
                selected.checked_sub(1).unwrap_or(count - 1)
            };
        }
        next
    }

    /// Uses `addons_dir` for the game and moves on to step 2. Step 3 offers
    /// no other game until `with_other_game` says so.
    pub fn choose_install(&self, addons_dir: &Path) -> Self {
        let mut next = self.with_other_game(None);
        next.draft.addon_dir = Some(addons_dir.to_path_buf());
        next.step = OnboardingStep::Wago;
        next.key_box = TextInput::new(next.draft.wago_api_key.clone().unwrap_or_default());
        next
    }

    /// Step 3 offers to set up the game at `other_game` too (off until the
    /// person turns it on), or offers nothing when it is `None`.
    pub fn with_other_game(&self, other_game: Option<PathBuf>) -> Self {
        let mut next = self.clone();
        next.other_game = other_game;
        next.set_up_other_game = false;
        if next.other_game.is_none() && next.preference == PreferenceField::AlsoSetUp {
            next.preference = PreferenceField::Theme;
        }
        next
    }

    pub fn next_step(&self) -> Self {
        let mut next = self.clone();
        next.step = next.step.next();
        next.problem = None;
        next
    }

    pub fn previous_step(&self) -> Self {
        let mut next = self.clone();
        next.step = next.step.previous();
        next.problem = None;
        next
    }

    pub fn set_quit_question(&self, open: bool) -> Self {
        let mut next = self.clone();
        next.quit_question = open;
        next
    }

    pub fn with_problem(&self, message: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.problem = Some(message.into());
        next
    }

    /// Applies `change` to the focused box at its cursor: the folder box on
    /// step 1, the key box on step 2 (whose text becomes the draft's key).
    pub fn edit_text(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        if !next.text_box_focused() {
            return next;
        }
        match next.step {
            OnboardingStep::Wago => {
                change(&mut next.key_box);
                let key = next.key_box.value().trim().to_string();
                next.draft.wago_api_key = (!key.is_empty()).then_some(key);
            }
            _ => {
                change(&mut next.folder_box);
                if let OnboardingPhase::Typing(typing) = &mut next.phase {
                    typing.error = None;
                }
            }
        }
        next
    }

    pub fn insert_char(&self, value: char) -> Self {
        self.edit_text(|text| text.insert_char(value))
    }

    pub fn backspace(&self) -> Self {
        self.edit_text(TextInput::backspace)
    }

    /// The rows step 3 shows, top to bottom.
    pub fn preference_fields(&self) -> Vec<PreferenceField> {
        let mut fields = PreferenceField::SETTINGS.to_vec();
        if self.other_game.is_some() {
            fields.push(PreferenceField::AlsoSetUp);
        }
        fields
    }

    /// The row moves down (`forward`) or up and stops at the ends.
    pub fn move_preference(&self, forward: bool) -> Self {
        let mut next = self.clone();
        let fields = next.preference_fields();
        let index = fields
            .iter()
            .position(|field| *field == next.preference)
            .unwrap_or(0);
        let index = if forward {
            (index + 1).min(fields.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        next.preference = fields[index];
        next
    }

    /// ←/→ on step 3. Backups to keep moves by one and never goes below
    /// one, as in Settings.
    pub fn change_preference(&self, delta: i16) -> Self {
        let mut next = self.clone();
        match next.preference {
            PreferenceField::Theme => {
                next.draft.theme = crate::theme::cycle_theme(next.draft.theme, delta);
            }
            PreferenceField::BackupWtf => next.draft.backup_wtf = !next.draft.backup_wtf,
            PreferenceField::AlsoSetUp => next.set_up_other_game = !next.set_up_other_game,
            PreferenceField::BackupRetention => {
                let current = i32::from(next.draft.backup_retention);
                next.draft.backup_retention =
                    u16::try_from((current + i32::from(delta)).max(1)).unwrap_or(u16::MAX);
            }
        }
        next
    }
}

/// The choices when nothing was found: the folder box first, because
/// someone who gets here usually knows where the game is, then wider
/// searches.
pub fn not_found_choices() -> Vec<NotFoundChoice> {
    let mut choices = vec![NotFoundChoice::TypeFolder];
    if cfg!(windows) {
        choices.extend(
            suggested_scan_roots()
                .into_iter()
                .skip(1)
                .filter(|root| root.exists())
                .map(NotFoundChoice::Search),
        );
    } else {
        let drives = if cfg!(target_os = "macos") {
            "/Volumes"
        } else {
            "/mnt"
        };
        if Path::new(drives).is_dir() {
            choices.push(NotFoundChoice::Search(PathBuf::from(drives)));
        }
        choices.push(NotFoundChoice::Search(PathBuf::from("/")));
    }
    choices
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::ThemeMode;

    use super::{
        FoundInstall, NotFoundChoice, OnboardingPhase, OnboardingState, OnboardingStep,
        PreferenceField, not_found_choices,
    };

    fn install(path: &str, addons: usize) -> FoundInstall {
        FoundInstall {
            addons_dir: PathBuf::from(path),
            addons,
        }
    }

    #[test]
    fn three_steps_and_no_review() {
        assert_eq!(
            OnboardingStep::ALL.map(OnboardingStep::label),
            ["Find your game", "Wago.io", "Preferences"]
        );
        assert_eq!(
            OnboardingStep::Preferences.next(),
            OnboardingStep::Preferences
        );
        assert_eq!(
            OnboardingStep::FindGame.previous(),
            OnboardingStep::FindGame
        );
    }

    #[test]
    fn found_choices_wrap_through_somewhere_else() {
        let state = OnboardingState::new().found(
            vec![
                install("/r/_retail_/Interface/AddOns", 25),
                install("/r/_forever_/Interface/AddOns", 0),
            ],
            0,
        );
        let OnboardingPhase::Found(found) = &state.phase else {
            panic!("found");
        };
        assert_eq!(found.selected_install().map(|i| i.addons), Some(25));

        let other = state.move_choice(true).move_choice(true);
        let OnboardingPhase::Found(found) = &other.phase else {
            panic!("found");
        };
        assert_eq!(found.selected_install(), None);
        let OnboardingPhase::Found(found) = &state.move_choice(false).phase else {
            panic!("found");
        };
        assert_eq!(found.selected, 2);
    }

    #[test]
    fn the_folder_box_starts_empty_and_esc_returns_to_where_it_opened() {
        let state = OnboardingState::new().not_found("nothing", not_found_choices());
        let typing = state.begin_typing();
        assert!(typing.text_box_focused());
        assert_eq!(typing.folder_box.value(), "");
        let typed = typing.insert_char('~').typing_error("no").insert_char('/');
        assert_eq!(typed.folder_box.value(), "~/");
        // Typing clears the last error.
        let OnboardingPhase::Typing(t) = &typed.phase else {
            panic!("typing");
        };
        assert_eq!(t.error, None);
        assert_eq!(typed.stop_typing().phase, state.phase);
        assert_eq!(
            not_found_choices().first(),
            Some(&NotFoundChoice::TypeFolder)
        );
    }

    #[test]
    fn the_key_box_is_the_drafts_key() {
        let mut state = OnboardingState::new();
        state.step = OnboardingStep::Wago;
        let typed = state.insert_char('a').insert_char('b');
        assert_eq!(typed.draft.wago_api_key.as_deref(), Some("ab"));
        assert_eq!(typed.backspace().backspace().draft.wago_api_key, None);
    }

    #[test]
    fn preferences_change_with_the_settings_rules() {
        let mut state = OnboardingState::new();
        state.step = OnboardingStep::Preferences;
        assert_eq!(state.change_preference(1).draft.theme, ThemeMode::Light);
        let backups = state.move_preference(true).move_preference(true);
        assert_eq!(backups.preference, PreferenceField::BackupRetention);
        let kept = backups.draft.backup_retention;
        assert_eq!(
            backups.change_preference(1).draft.backup_retention,
            kept + 1
        );
        let mut one = backups.clone();
        one.draft.backup_retention = 1;
        assert_eq!(one.change_preference(-1).draft.backup_retention, 1);
        let wtf = state.move_preference(true);
        assert_eq!(
            wtf.change_preference(-1).draft.backup_wtf,
            !state.draft.backup_wtf
        );
    }

    #[test]
    fn also_set_up_is_a_row_only_with_another_game_and_starts_off() {
        let state = OnboardingState::new();
        assert_eq!(state.preference_fields(), PreferenceField::SETTINGS);
        let last = state.move_preference(true).move_preference(true);
        assert_eq!(last.move_preference(true).preference, last.preference);

        let offered = state.with_other_game(Some(PathBuf::from("/r/_forever_/Interface/AddOns")));
        assert!(!offered.set_up_other_game);
        assert_eq!(
            offered.preference_fields().last(),
            Some(&PreferenceField::AlsoSetUp)
        );
        let mut on = offered;
        on.preference = PreferenceField::AlsoSetUp;
        let on = on.change_preference(1);
        assert!(on.set_up_other_game);
        assert!(!on.change_preference(-1).set_up_other_game);
        // Offering nothing again takes the row, the cursor and the choice away.
        let gone = on.with_other_game(None);
        assert_eq!(gone.preference, PreferenceField::Theme);
        assert!(!gone.set_up_other_game);
        // Choosing another install forgets the old offer.
        let chosen = on.choose_install(std::path::Path::new("/r/_retail_/Interface/AddOns"));
        assert_eq!(chosen.other_game, None);
    }
}
