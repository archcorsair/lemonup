use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration as StdDuration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};

use lemonup_core::binding::{BindingCheck, check_addon_dir_binding, rebind_addon_dir};
use lemonup_core::{
    AddonKind, AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE,
    GameFlavor, ImportAnalysis, ScanSummary, SourceKind, StateDatabase, ThemeMode, UpdateStatus,
    analyze_import, default_transfer_path, detect_known_addons_paths, export_addons,
    parse_import_file, paths_match, scan_addons_dir_report, search_for_wow_installs,
    validate_addons_path,
};
use tokio::sync::mpsc;

use crate::action::AppAction;
use crate::app::text_input::{TextEdit, TextInput, sanitize_single_line_paste};
use crate::backup::{
    BackupDeleteOutcome, BackupEntry, BackupRestoreOutcome, BackupRunOutcome,
    PreUpdateBackupSettings, adopt_legacy_backups, backup_root, client_backup_dir,
    create_wtf_backup, delete_backup, list_backups, restore_wtf_backup, run_pre_update_backup,
};
use crate::drift::{DriftReport, compute_drift_report};
use crate::event::{EventHandler, TerminalEvent};
use crate::onboarding::{
    FoundAction, OnboardingPhase, OnboardingSettingsField, OnboardingState, OnboardingStep,
    OnboardingTaskEvent,
};
use crate::shimmer::{ShimmerConfig, shimmer_text_spans};
use crate::source_error::{FolderClash, SourceError, SourceErrorKind};
use crate::theme::UiTheme;
use crate::transfer::{
    TransferInstallSummary, collect_existing_import_folders, install_transfer_addons,
};
use crate::tui::Backend;
use crate::update::{
    CheckResult, LiveUpdateStatus, LiveUpdateSummary, apply_live_updates, build_update_checks,
    refresh_live_update_checks,
};
use crate::wago::{
    WagoInstallInspection, WagoInstallSummary, WagoSearchResult, WagoStability,
    inspect_wago_install_target, install_wago_addon_with_replace, resolve_wago_api_key,
    search_wago_addons,
};
use time::{Duration, OffsetDateTime};

mod dialog;
mod direct_install;
mod keymap;
mod render_backup;
mod render_config;
mod render_dashboard;
mod render_help;
mod render_inspect;
mod render_install_search;
mod render_shared;
mod sheet;
mod status;
pub(crate) mod text_input;

pub use self::direct_install::DirectInstallRequest;
use self::direct_install::{
    DirectInstallOutcome, SearchInputIntent, classify_search_input, run_direct_install_task,
};
use self::keymap::KeyHint;
#[cfg(test)]
use self::render_dashboard::{child_row_detail_prefix, child_row_prefix};
use self::render_dashboard::{
    dashboard_item_author_sort_key, dashboard_item_management_summary,
    dashboard_item_sort_version_key, dashboard_item_source_label, dashboard_item_status_text,
    dashboard_item_update_status, dashboard_item_version_label,
};
use self::render_shared::{
    format_download_count, summarize_owned_folders, truncate_middle_text, truncate_text,
    visible_search_result_window,
};
pub(crate) use self::sheet::{SheetMove, SheetScroll};
pub(crate) use self::status::{StatusKind, StatusMessage, StatusOffer};

const LOGO_FULL: [&str; 2] = [
    "█   █▀▀ █▀▄▀█ █▀█ █▄ █ █ █ █▀█",
    "█▄▄ ██▄ █ ▀ █ █▄█ █ ▀█ █▄█ █▀▀",
];
const MOTION_SPINNER_FRAMES: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];
const IDLE_TICK_RATE: StdDuration = StdDuration::from_millis(250);
const ANIMATED_TICK_RATE: StdDuration = StdDuration::from_millis(100);
const BACKUP_SPAM_GUARD_WINDOW: Duration = Duration::seconds(10);
/// How long a done or note message stays on the status line.
const MESSAGE_FADE: StdDuration = StdDuration::from_secs(4);

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    Inspect,
    Help,
    Install,
    Search,
    Update,
    Config,
    Backup,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct OverlayState {
    active: Option<OverlayKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HelpContext {
    #[default]
    Overview,
    Inspect,
    /// Details docked beside the list, where the list keys keep working.
    DetailsDocked,
    InstallSearch,
    Config,
    Backup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InspectResolvedTarget<'a> {
    item: &'a DashboardItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionPreset {
    Tasteful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MotionState {
    preset: MotionPreset,
    tick_count: usize,
}

impl Default for MotionState {
    fn default() -> Self {
        Self {
            preset: MotionPreset::Tasteful,
            tick_count: 0,
        }
    }
}

impl MotionState {
    fn advance(self) -> Self {
        Self {
            preset: self.preset,
            tick_count: self.tick_count.wrapping_add(1),
        }
    }

    fn spinner_frame(self) -> &'static str {
        MOTION_SPINNER_FRAMES[self.tick_count % MOTION_SPINNER_FRAMES.len()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ShellUiState {
    overlay: OverlayState,
    motion: MotionState,
    footer_pulse: Option<FooterKeyPulse>,
    status_flash: Option<StatusFlash>,
    help_context: HelpContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FooterKeyPulse {
    hint: FooterHintId,
    expires_at_tick: usize,
}

/// The message on the status line. Done and note messages fade after
/// `MESSAGE_FADE`; problems stay until the next key press; a message that
/// offers a key stays while that key still works.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusFlash {
    text: String,
    kind: StatusKind,
    offer: Option<StatusOffer>,
    /// `None` keeps the message until the next key press.
    expires_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterHintTier {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterHintId {
    Help,
    Nav,
    Inspect,
    Select,
    Clear,
    Check,
    Update,
    Delete,
    Install,
    Filter,
    Config,
    Backup,
    Confirm,
    Cancel,
    Close,
    Edit,
    Run,
    Toggle,
    SettingsNav,
    ThemeToggle,
    Next,
    Back,
    Validate,
    Fields,
    Results,
    Change,
    Save,
    UpdateAll,
    CheckAll,
    Quit,
    Scroll,
    HideCard,
    /// ← → Home End in a text box.
    Cursor,
    /// Ctrl+U in a text box.
    ClearText,
    /// `f` on a "Not linked" addon.
    FindOnline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShellFrame {
    header: Rect,
    body: Rect,
    footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    Onboarding,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailMode {
    Overview,
    Install,
    Search,
    /// The old Update screen. No key opens it any more; its render code goes
    /// with the overlay rebuild.
    #[cfg_attr(not(test), allow(dead_code))]
    Update,
    Config,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRuntime {
    pub profile_name: String,
    pub addon_dir_override: Option<PathBuf>,
    /// AddOns folders this profile must never target: the default profile's configured
    /// folder and the folder its state is bound to (they can differ or be set only one way).
    pub guarded_addon_dirs: Vec<PathBuf>,
}

impl AppRuntime {
    pub fn new(
        profile_name: String,
        addon_dir_override: Option<PathBuf>,
        guarded_addon_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            profile_name,
            addon_dir_override,
            guarded_addon_dirs,
        }
    }

    pub fn is_default_profile(&self) -> bool {
        self.profile_name == DEFAULT_PROFILE
    }

    pub fn is_guarded_path(&self, candidate: &Path) -> bool {
        if self.is_default_profile() {
            return false;
        }

        self.guarded_addon_dirs
            .iter()
            .any(|guarded| paths_match(candidate, guarded))
    }
}

/// The text box that has focus: typing, pasting and the editing keys go to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextBox {
    Onboarding,
    Search,
    /// The Wago.io key box inside the Install sheet.
    WagoKey,
    Install,
    Config,
    DashboardFilter,
}

/// Ctrl+C, which quits from anywhere.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// A Ctrl or Alt chord on a character key, which no screen acts on. On Windows
/// AltGr arrives as Ctrl+Alt and types ordinary characters such as `@` or `\`,
/// so a Ctrl+Alt chord only counts when the character is an ASCII letter or digit.
fn is_modified_char_chord(key: KeyEvent) -> bool {
    let KeyCode::Char(character) = key.code else {
        return false;
    };
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match (control, alt) {
        (false, false) => false,
        (true, true) => character.is_ascii_alphanumeric(),
        _ => true,
    }
}

/// The edit a key asks of a focused text box, other than typing or
/// backspace: the arrows, Home and End move the cursor, Delete deletes the
/// character under it and Ctrl+U empties the box. Ctrl+Alt+U is left out:
/// on Windows that is AltGr.
fn text_edit_for_key(key: KeyEvent) -> Option<TextEdit> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('u' | 'U') if control && !alt => Some(TextEdit::Clear),
        _ if control || alt => None,
        KeyCode::Left => Some(TextEdit::Left),
        KeyCode::Right => Some(TextEdit::Right),
        KeyCode::Home => Some(TextEdit::Home),
        KeyCode::End => Some(TextEdit::End),
        KeyCode::Delete => Some(TextEdit::Delete),
        _ => None,
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum AppMessage {
    Tick,
    QuitRequested,
    /// Yes to "quit while updating?".
    QuitConfirmed,
    /// No to "quit while updating?".
    QuitCancelled,
    /// Bracketed-paste text, routed to whichever text field is being edited.
    Paste(String),
    /// A cursor move, delete or clear in the focused text box.
    TextBoxEdit(TextEdit),
    TerminalResized {
        width: u16,
        height: u16,
    },
    DashboardOpenInspect,
    DashboardOpenHelp,
    /// `d` on the welcome card: hide it for good.
    DashboardHideFirstRunCard,
    DashboardCloseOverlay,
    /// Scrolls the open sheet's body.
    SheetScroll(SheetMove),
    InspectToggleSelected,
    InspectRequestDelete,
    InspectRunCheck,
    InspectRunUpdate,
    DashboardSelectionNext,
    DashboardSelectionPageDown,
    DashboardSelectionPageUp,
    DashboardSelectionFirst,
    DashboardSelectionLast,
    DashboardSelectionPrevious,
    DashboardPointerSelect {
        column: u16,
        row: u16,
    },
    DashboardRequestDelete,
    DashboardConfirmDelete,
    DashboardCancelPendingDelete,
    DashboardUndoDelete,
    DashboardRunCheckSelected,
    DashboardRunUpdateSelected,
    /// `C`: check every linked addon.
    DashboardRunCheckAll,
    /// `U`: update every addon that has an update ready.
    DashboardRunUpdateAll,
    DashboardToggleSort(DashboardSortColumn),
    DashboardToggleSelected,
    DashboardSelectAll,
    DashboardClearSelection,
    DashboardToggleExpanded,
    DashboardExpandAllRelationships,
    DashboardCollapseAllRelationships,
    DashboardCollapseExpanded,
    DashboardFilterBegin,
    DashboardFilterInputChar(char),
    DashboardFilterBackspace,
    DashboardFilterApply,
    DashboardFilterClear,
    SetDetailMode(DetailMode),
    OpenSearch(SearchPresentationMode),
    InstallBeginEditing,
    InstallStopEditing,
    InstallInputChar(char),
    InstallBackspace,
    InstallSubmit,
    /// `f` on a "Not linked" addon: Install, with its name searched.
    FindOnline,
    SearchFocusBox,
    SearchFocusResults,
    SearchSaveKey,
    SearchCloseKeyBox,
    SearchKeyChar(char),
    SearchKeyBackspace,
    /// Yes or no to replacing folders a direct install found in the way.
    SearchConfirmReplace,
    SearchCancelReplace,
    SearchInputChar(char),
    SearchBackspace,
    SearchSubmit,
    SearchResultNext,
    SearchResultPrevious,
    SearchInstallSelected,
    SearchConfirmDirectInstall,
    SearchCancelDirectInstall,
    WagoConfirmInstall,
    WagoCancelInstall,
    ConfigSelectionNext,
    ConfigSelectionPrevious,
    ConfigBeginEditing,
    ConfigInputChar(char),
    ConfigBackspace,
    ConfigCommitEdit,
    ConfigCancelEdit,
    ConfigToggleSelected,
    ConfigAdjustSelected(i16),
    ConfigRunOnboarding,
    ConfigExportAddons,
    ConfigImportAddons,
    BackupRunNow,
    BackupSelectionNext,
    BackupSelectionPrevious,
    BackupRequestRestore,
    BackupRequestDelete,
    BackupConfirmRestore,
    BackupConfirmDelete,
    BackupCancelRestore,
    OnboardingNextStep,
    OnboardingPreviousStep,
    OnboardingToggleTheme,
    OnboardingSettingsNext,
    OnboardingSettingsPrevious,
    OnboardingSettingIncrease,
    OnboardingSettingDecrease,
    OnboardingBeginEditing,
    OnboardingStopEditing,
    OnboardingInputChar(char),
    OnboardingBackspace,
    OnboardingValidateInput,
    OnboardingDeepScan,
    OnboardingCancel,
    OnboardingSuggestionNext,
    OnboardingSuggestionPrevious,
    OnboardingFoundActionNext,
    OnboardingFoundActionPrevious,
    OnboardingFoundConfirm,
    BackgroundTask(AppTaskEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppTaskEvent {
    Onboarding(OnboardingTaskEvent),
    AddonScanFinished(std::result::Result<AddonScanOutcome, String>),
    DashboardDeleteFinished(std::result::Result<DashboardDeleteOutcome, String>),
    DashboardUndoFinished(std::result::Result<DashboardUndoOutcome, String>),
    DashboardJobProgress(DashboardJobUiState),
    DashboardCheckFinished(std::result::Result<DashboardCheckOutcome, String>),
    DashboardUpdateFinished(std::result::Result<DashboardUpdateOutcome, String>),
    BackupFinished(std::result::Result<BackupRunOutcome, String>),
    BackupRestoreFinished(std::result::Result<BackupRestoreOutcome, String>),
    BackupDeleteFinished(std::result::Result<BackupDeleteOutcome, String>),
    TransferImportFinished(std::result::Result<TransferImportOutcome, String>),
    WagoSearchFinished(std::result::Result<WagoSearchOutcome, SourceError>),
    WagoInstallFinished(std::result::Result<WagoInstallTaskOutcome, SourceError>),
    DirectInstallFinished {
        request: DirectInstallRequest,
        result: std::result::Result<DirectInstallOutcome, SourceError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AddonScanOutcome {
    path: PathBuf,
    summary: ScanSummary,
    addons: Vec<AddonRecord>,
    drift_report: DriftReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardDeleteOutcome {
    deleted_parents: usize,
    deleted_folders: usize,
    undo_delete: Option<DashboardUndoDeleteState>,
    sync: AddonScanOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardCheckOutcome {
    addons: Vec<AddonRecord>,
    total: usize,
    targets: Vec<String>,
    live_checked: usize,
    cached: usize,
    errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardUpdateOutcome {
    summary: LiveUpdateSummary,
    /// Folders the run reports as updated, and as failed.
    updated_folders: Vec<String>,
    failed_folders: Vec<String>,
    updated_addon_name: Option<String>,
    warnings: Vec<String>,
    sync: AddonScanOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransferImportOutcome {
    analysis: ImportAnalysis,
    install_summary: TransferInstallSummary,
    sync: AddonScanOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WagoSearchOutcome {
    query: String,
    results: Vec<WagoSearchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WagoInstallOutcome {
    summary: WagoInstallSummary,
    sync: AddonScanOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardJobKind {
    #[allow(dead_code)]
    Check,
    Update,
}

impl DashboardJobKind {
    fn label(self) -> &'static str {
        match self {
            Self::Check => "Checking",
            Self::Update => "Updating",
        }
    }

    fn accent(self, theme: UiTheme) -> Color {
        match self {
            Self::Check => theme.info,
            Self::Update => theme.warning,
        }
    }
}

/// Where one row stands in a running update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateJobRowState {
    Waiting,
    Updating,
    /// The job has moved past this row; its result is not known yet.
    Done,
    /// The finished run reported this addon as updated.
    Updated,
    /// The finished run reported this addon as failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardJobUiState {
    kind: DashboardJobKind,
    current_index: usize,
    total: usize,
    current_addon: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigField {
    WagoApiKey,
    CheckInterval,
    AutoCheckEnabled,
    AutoCheckInterval,
    BackupWtf,
    BackupRetention,
    Theme,
    ShowLibs,
}

impl ConfigField {
    const ALL: [Self; 8] = [
        Self::WagoApiKey,
        Self::CheckInterval,
        Self::AutoCheckEnabled,
        Self::AutoCheckInterval,
        Self::BackupWtf,
        Self::BackupRetention,
        Self::Theme,
        Self::ShowLibs,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::WagoApiKey => "Wago.io key",
            Self::CheckInterval => "Don't re-check for",
            Self::AutoCheckEnabled => "Check automatically",
            Self::AutoCheckInterval => "Check every",
            Self::BackupWtf => "Back up game settings",
            Self::BackupRetention => "Backups to keep",
            Self::Theme => "Theme",
            Self::ShowLibs => "Show library folders",
        }
    }

    fn is_textual(self) -> bool {
        matches!(
            self,
            Self::WagoApiKey
                | Self::CheckInterval
                | Self::AutoCheckInterval
                | Self::BackupRetention
        )
    }

    fn supports_quick_adjust(self) -> bool {
        !matches!(self, Self::WagoApiKey)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigEditState {
    field: ConfigField,
    value: TextInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaneState {
    persisted: AppConfig,
    draft: AppConfig,
    selected_field: usize,
    edit: Option<ConfigEditState>,
}

impl ConfigPaneState {
    fn new(config: AppConfig) -> Self {
        Self {
            persisted: config.clone(),
            draft: config,
            selected_field: 0,
            edit: None,
        }
    }

    fn selected_field(&self) -> ConfigField {
        ConfigField::ALL[self.selected_field]
    }

    fn select_next(&self) -> Self {
        let mut next = self.clone();
        next.selected_field = (next.selected_field + 1) % ConfigField::ALL.len();
        next
    }

    fn select_previous(&self) -> Self {
        let mut next = self.clone();
        next.selected_field = if next.selected_field == 0 {
            ConfigField::ALL.len() - 1
        } else {
            next.selected_field - 1
        };
        next
    }

    fn begin_editing(&self) -> Self {
        let field = self.selected_field();
        let value = match field {
            ConfigField::WagoApiKey => self.draft.wago_api_key.clone().unwrap_or_default(),
            ConfigField::CheckInterval => self.draft.check_interval_secs.to_string(),
            ConfigField::AutoCheckInterval => self.draft.auto_check_interval_secs.to_string(),
            ConfigField::BackupRetention => self.draft.backup_retention.to_string(),
            _ => return self.clone(),
        };
        let mut next = self.clone();
        next.edit = Some(ConfigEditState {
            field,
            value: TextInput::new(value),
        });
        next
    }

    fn cancel_edit(&self) -> Self {
        let mut next = self.clone();
        next.edit = None;
        next
    }

    /// Applies `change` to the value being edited, if any.
    fn edit_text(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        if let Some(edit) = next.edit.as_mut() {
            change(&mut edit.value);
        }
        next
    }

    fn insert_char(&self, character: char) -> Self {
        self.edit_text(|value| value.insert_char(character))
    }

    fn backspace(&self) -> Self {
        self.edit_text(TextInput::backspace)
    }

    fn commit_edit(&self) -> Result<Self, String> {
        let Some(edit) = &self.edit else {
            return Ok(self.clone());
        };

        let mut next = self.clone();
        match edit.field {
            ConfigField::WagoApiKey => {
                let trimmed = edit.value.value().trim().to_string();
                next.draft.wago_api_key = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
            }
            ConfigField::CheckInterval => {
                let parsed =
                    edit.value.value().trim().parse::<u64>().map_err(|_| {
                        "check freshness must be a whole number of seconds".to_string()
                    })?;
                if parsed == 0 {
                    return Err("check freshness must be at least 1 second".to_string());
                }
                next.draft.check_interval_secs = parsed;
            }
            ConfigField::AutoCheckInterval => {
                let parsed = edit.value.value().trim().parse::<u64>().map_err(|_| {
                    "auto-check interval must be a whole number of seconds".to_string()
                })?;
                if parsed == 0 {
                    return Err("auto-check interval must be at least 1 second".to_string());
                }
                next.draft.auto_check_interval_secs = parsed;
            }
            ConfigField::BackupRetention => {
                let parsed = edit
                    .value
                    .value()
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| "backup retention must be a whole number".to_string())?;
                if parsed == 0 {
                    return Err("backup retention must be at least 1".to_string());
                }
                next.draft.backup_retention = parsed;
            }
            _ => {}
        }
        next.edit = None;
        Ok(next)
    }

    fn adjust_selected(&self, delta: i16) -> Self {
        let mut next = self.clone();
        match next.selected_field() {
            ConfigField::WagoApiKey => {}
            ConfigField::CheckInterval => {
                next.draft.check_interval_secs =
                    adjust_u64(next.draft.check_interval_secs, delta, 60, 1);
            }
            ConfigField::AutoCheckEnabled => {
                next.draft.auto_check_enabled = !next.draft.auto_check_enabled;
            }
            ConfigField::AutoCheckInterval => {
                next.draft.auto_check_interval_secs =
                    adjust_u64(next.draft.auto_check_interval_secs, delta, 300, 1);
            }
            ConfigField::BackupWtf => {
                next.draft.backup_wtf = !next.draft.backup_wtf;
            }
            ConfigField::BackupRetention => {
                next.draft.backup_retention = adjust_u16(next.draft.backup_retention, delta, 1, 1);
            }
            ConfigField::Theme => {
                next.draft.theme = match next.draft.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            ConfigField::ShowLibs => {
                next.draft.show_libs = !next.draft.show_libs;
            }
        }
        next
    }

    fn toggle_selected(&self) -> Self {
        self.adjust_selected(1)
    }

    fn mark_saved(&self, saved: AppConfig) -> Self {
        Self {
            persisted: saved.clone(),
            draft: saved,
            selected_field: self.selected_field,
            edit: None,
        }
    }

    fn is_dirty(&self) -> bool {
        self.persisted != self.draft
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackupPaneState {
    backups: Vec<BackupEntry>,
    in_progress: bool,
    selected_backup: usize,
    pending_restore: Option<BackupEntry>,
    pending_delete: Option<BackupEntry>,
    last_completed_backup_at: Option<OffsetDateTime>,
}

impl BackupPaneState {
    fn selected_backup(&self) -> Option<&BackupEntry> {
        self.backups.get(self.selected_backup)
    }

    fn select_next(&self) -> Self {
        if self.backups.is_empty() {
            return self.clone();
        }
        let mut next = self.clone();
        next.selected_backup = (next.selected_backup + 1) % next.backups.len();
        next
    }

    fn select_previous(&self) -> Self {
        if self.backups.is_empty() {
            return self.clone();
        }
        let mut next = self.clone();
        next.selected_backup = if next.selected_backup == 0 {
            next.backups.len() - 1
        } else {
            next.selected_backup - 1
        };
        next
    }

    fn with_backups(&self, backups: Vec<BackupEntry>) -> Self {
        let mut next = self.clone();
        next.backups = backups;
        if next.backups.is_empty() {
            next.selected_backup = 0;
        } else {
            next.selected_backup = next.selected_backup.min(next.backups.len() - 1);
        }
        next.pending_restore = None;
        next.pending_delete = None;
        next
    }

    fn with_last_completed_backup_at(&self, timestamp: Option<OffsetDateTime>) -> Self {
        let mut next = self.clone();
        next.last_completed_backup_at = timestamp;
        next
    }

    fn request_restore(&self) -> Self {
        let mut next = self.clone();
        next.pending_restore = self.selected_backup().cloned();
        next.pending_delete = None;
        next
    }

    fn request_delete(&self) -> Self {
        let mut next = self.clone();
        next.pending_delete = self.selected_backup().cloned();
        next.pending_restore = None;
        next
    }

    fn cancel_restore(&self) -> Self {
        let mut next = self.clone();
        next.pending_restore = None;
        next
    }

    fn cancel_delete(&self) -> Self {
        let mut next = self.clone();
        next.pending_delete = None;
        next
    }

    fn set_in_progress(&self, in_progress: bool) -> Self {
        let mut next = self.clone();
        next.in_progress = in_progress;
        if in_progress {
            next.pending_restore = None;
            next.pending_delete = None;
        }
        next
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WagoInstallTaskOutcome {
    NeedsConfirmation {
        request: PendingWagoInstallRequest,
        inspection: WagoInstallInspection,
    },
    Installed(WagoInstallOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardUndoOutcome {
    restored_parents: usize,
    restored_folders: usize,
    sync: AddonScanOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardTrashEntry {
    folder: String,
    original_path: PathBuf,
    trashed_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardUndoDeleteState {
    batch_dir: PathBuf,
    addon_dir: PathBuf,
    deleted_parent_folders: Vec<String>,
    tracked_records: Vec<AddonRecord>,
    moved_entries: Vec<DashboardTrashEntry>,
}

impl DashboardUndoDeleteState {
    fn parent_count(&self) -> usize {
        self.deleted_parent_folders.len()
    }

    fn moved_folder_count(&self) -> usize {
        self.moved_entries.len()
    }

    fn target_summary(&self) -> String {
        self.deleted_parent_folders.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardUpdateRunSummary {
    targets: usize,
    updated_addons: usize,
    up_to_date: usize,
    update_available: usize,
    unknown: usize,
    errors: usize,
    skipped_manual: usize,
    skipped_no_api_key: usize,
    remote_older: usize,
    scanned_addons: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardRefreshabilitySummary {
    total: usize,
    refreshable: usize,
    manual: usize,
    remote_older: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardItem {
    name: String,
    folder: String,
    owned_folders: Vec<String>,
    source: SourceKind,
    source_url: Option<String>,
    kind: AddonKind,
    version: Option<String>,
    author: Option<String>,
    interface: Option<String>,
    git_commit: Option<String>,
    remote_version: Option<String>,
    installed_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    last_checked_at: Option<OffsetDateTime>,
    required_deps: Vec<String>,
    optional_deps: Vec<String>,
    embedded_libs: Vec<String>,
    owned_folder_count: usize,
    has_authoritative_owned_folders: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DashboardRowKey {
    Parent(String),
    OwnedChild {
        parent_folder: String,
        child_folder: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DashboardRowKind {
    Parent,
    OwnedChild { parent_folder: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardChildConnector {
    Mid,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardRow {
    key: DashboardRowKey,
    name: String,
    folder: String,
    kind: DashboardRowKind,
    expandable: bool,
    expanded: bool,
    child_connector: Option<DashboardChildConnector>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardSortColumn {
    Name,
    Status,
    Author,
    Source,
}

impl DashboardSortColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Author => "Author",
            Self::Source => "Source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardSortDirection {
    Asc,
    Desc,
}

impl DashboardSortDirection {
    fn toggle(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    fn indicator(self) -> &'static str {
        match self {
            Self::Asc => "▲",
            Self::Desc => "▼",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardSortConfig {
    column: DashboardSortColumn,
    direction: DashboardSortDirection,
}

impl Default for DashboardSortConfig {
    fn default() -> Self {
        Self {
            column: DashboardSortColumn::Status,
            direction: DashboardSortDirection::Asc,
        }
    }
}

impl DashboardSortConfig {
    fn toggled(self, column: DashboardSortColumn) -> Self {
        if self.column == column {
            Self {
                column,
                direction: self.direction.toggle(),
            }
        } else {
            Self {
                column,
                direction: DashboardSortDirection::Asc,
            }
        }
    }
}

struct DashboardState {
    items: Vec<DashboardItem>,
    rows: Vec<DashboardRow>,
    list_state: TableState,
    sort_config: DashboardSortConfig,
    detail_mode: DetailMode,
    expanded_folders: HashSet<String>,
    selected_parents: HashSet<String>,
    pending_delete_folders: Option<Vec<String>>,
    drift_report: Option<DriftReport>,
    update_in_progress: bool,
    job_ui: Option<DashboardJobUiState>,
    /// Folders the running update was started for, and the ones it has
    /// reached so far, so each row can show Waiting, Updating or Updated.
    job_targets: Vec<String>,
    job_reached: Vec<String>,
    /// What the last update run reported per folder, kept until the next
    /// job starts, so a row only says Updated or Update failed when the run
    /// said so.
    updated_folders: Vec<String>,
    failed_folders: Vec<String>,
    last_update_summary: Option<DashboardUpdateRunSummary>,
    /// Live filter text applied to the visible rows; empty means no filter.
    filter_query: TextInput,
    /// True while `/` filter input is capturing keystrokes.
    filter_editing: bool,
}

impl DashboardState {
    fn from_addons(addons: Vec<AddonRecord>) -> Self {
        let items = addons
            .into_iter()
            .map(|addon| {
                let owned_folder_count = addon.owned_folders.len();
                let has_authoritative_owned_folders = addon.has_authoritative_owned_folders();
                DashboardItem {
                    name: addon.name,
                    folder: addon.folder,
                    owned_folders: addon
                        .owned_folders
                        .into_iter()
                        .map(|owned_folder| owned_folder.name)
                        .collect(),
                    source: addon.source,
                    source_url: addon.source_url,
                    kind: addon.kind,
                    version: addon.version,
                    author: addon.author,
                    interface: addon.interface,
                    git_commit: addon.git_commit,
                    remote_version: addon.remote_version,
                    installed_at: addon.installed_at,
                    updated_at: addon.updated_at,
                    last_checked_at: addon.last_checked_at,
                    required_deps: addon.required_deps,
                    optional_deps: addon.optional_deps,
                    embedded_libs: addon.embedded_libs,
                    owned_folder_count,
                    has_authoritative_owned_folders,
                }
            })
            .collect::<Vec<_>>();

        let mut state = Self {
            items,
            rows: Vec::new(),
            list_state: TableState::default(),
            sort_config: DashboardSortConfig::default(),
            detail_mode: DetailMode::Overview,
            expanded_folders: HashSet::new(),
            selected_parents: HashSet::new(),
            pending_delete_folders: None,
            drift_report: None,
            update_in_progress: false,
            job_ui: None,
            job_targets: Vec::new(),
            job_reached: Vec::new(),
            updated_folders: Vec::new(),
            failed_folders: Vec::new(),
            last_update_summary: None,
            filter_query: TextInput::default(),
            filter_editing: false,
        };
        state.sort_items();
        state.rebuild_rows();
        if !state.rows.is_empty() {
            state.list_state.select(Some(0));
        }
        state
    }

    fn replace_addons(&mut self, addons: Vec<AddonRecord>) {
        let selected_key = self.selected_row().map(|row| row.key.clone());
        let previous_offset = self.list_state.offset();
        let previous_expanded = self.expanded_folders.clone();
        let previous_selected = self.selected_parents.clone();
        let previous_sort = self.sort_config;
        let next = DashboardState::from_addons(addons);

        self.items = next.items;
        self.rows = next.rows;
        self.list_state = next.list_state;
        self.sort_config = previous_sort;
        self.expanded_folders = previous_expanded
            .into_iter()
            .filter(|folder| {
                self.items
                    .iter()
                    .any(|item| item.folder == *folder && !item.owned_folders.is_empty())
            })
            .collect();
        self.selected_parents = previous_selected
            .into_iter()
            .filter(|folder| self.items.iter().any(|item| item.folder == *folder))
            .collect();
        self.pending_delete_folders = self.pending_delete_folders.as_ref().and_then(|pending| {
            let filtered = pending
                .iter()
                .filter(|folder| self.items.iter().any(|item| item.folder == **folder))
                .cloned()
                .collect::<Vec<_>>();
            if filtered.is_empty() {
                None
            } else {
                Some(filtered)
            }
        });
        self.update_in_progress = false;
        // `next` starts unfiltered; the filter lives on `self` and survives rescans.
        self.sort_items();
        self.rebuild_rows();

        let mut selected = selected_key
            .as_ref()
            .and_then(|key| self.rows.iter().position(|row| row.key == *key));
        if selected.is_none() && !self.rows.is_empty() {
            selected = Some(0);
        }
        let max_offset = self.rows.len().saturating_sub(1);
        self.list_state = TableState::default()
            .with_offset(previous_offset.min(max_offset))
            .with_selected(selected);
    }

    fn next_selection(&self) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }

        Some(match self.list_state.selected() {
            Some(index) => (index + 1) % self.rows.len(),
            None => 0,
        })
    }

    /// Moves the cursor by `delta` rows, stopping at either end of the list.
    fn offset_selection(&self, delta: isize) -> Option<usize> {
        let last = self.rows.len().checked_sub(1)?;
        let current = self.list_state.selected().unwrap_or(0).min(last);
        Some(current.saturating_add_signed(delta).min(last))
    }

    fn first_selection(&self) -> Option<usize> {
        (!self.rows.is_empty()).then_some(0)
    }

    fn last_selection(&self) -> Option<usize> {
        self.rows.len().checked_sub(1)
    }

    fn previous_selection(&self) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }

        Some(match self.list_state.selected() {
            Some(0) | None => self.rows.len() - 1,
            Some(index) => index - 1,
        })
    }

    fn toggle_selected_expanded(&mut self) -> bool {
        let Some(row) = self.selected_row().cloned() else {
            return false;
        };

        match row.kind {
            DashboardRowKind::Parent if row.expandable => {
                if row.expanded {
                    self.expanded_folders.remove(&row.folder);
                } else {
                    self.expanded_folders.insert(row.folder.clone());
                }
                self.rebuild_rows();
                self.restore_selection(&row.key);
                true
            }
            DashboardRowKind::OwnedChild { parent_folder }
                if self.expanded_folders.remove(&parent_folder) =>
            {
                self.rebuild_rows();
                self.restore_selection(&DashboardRowKey::Parent(parent_folder));
                true
            }
            _ => false,
        }
    }

    fn collapse_selected_expanded(&mut self) -> bool {
        let Some(row) = self.selected_row().cloned() else {
            return false;
        };

        match row.kind {
            DashboardRowKind::Parent if row.expanded => {
                self.expanded_folders.remove(&row.folder);
                self.rebuild_rows();
                self.restore_selection(&row.key);
                true
            }
            DashboardRowKind::OwnedChild { parent_folder }
                if self.expanded_folders.remove(&parent_folder) =>
            {
                self.rebuild_rows();
                self.restore_selection(&DashboardRowKey::Parent(parent_folder));
                true
            }
            _ => false,
        }
    }

    fn expand_all_relationships(&mut self) -> bool {
        let selected_key = self.selected_row().map(|row| row.key.clone());
        let before = self.expanded_folders.len();
        self.expanded_folders = self
            .items
            .iter()
            .filter(|item| !item.owned_folders.is_empty())
            .map(|item| item.folder.clone())
            .collect();
        self.rebuild_rows();
        if let Some(key) = selected_key.as_ref() {
            self.restore_selection(key);
        }
        self.expanded_folders.len() != before
    }

    fn collapse_all_relationships(&mut self) -> bool {
        if self.expanded_folders.is_empty() {
            return false;
        }

        let selected_key = self.selected_row().map(|row| match &row.kind {
            DashboardRowKind::Parent => row.key.clone(),
            DashboardRowKind::OwnedChild { parent_folder } => {
                DashboardRowKey::Parent(parent_folder.clone())
            }
        });
        self.expanded_folders.clear();
        self.rebuild_rows();
        if let Some(key) = selected_key.as_ref() {
            self.restore_selection(key);
        }
        true
    }

    fn selected_row(&self) -> Option<&DashboardRow> {
        self.list_state
            .selected()
            .and_then(|index| self.rows.get(index))
    }

    fn selected_item(&self) -> Option<&DashboardItem> {
        let row = self.selected_row()?;
        let parent_folder = match &row.kind {
            DashboardRowKind::Parent => &row.folder,
            DashboardRowKind::OwnedChild { parent_folder } => parent_folder,
        };
        self.items.iter().find(|item| item.folder == *parent_folder)
    }

    fn selected_owned_child_folder(&self) -> Option<&str> {
        let row = self.selected_row()?;
        match &row.kind {
            DashboardRowKind::OwnedChild { .. } => Some(row.folder.as_str()),
            DashboardRowKind::Parent => None,
        }
    }

    fn selected_parent_folder(&self) -> Option<&str> {
        let row = self.selected_row()?;
        match &row.kind {
            DashboardRowKind::Parent => Some(row.folder.as_str()),
            DashboardRowKind::OwnedChild { parent_folder } => Some(parent_folder.as_str()),
        }
    }

    fn selected_parent_row_folder(&self) -> Option<&str> {
        let row = self.selected_row()?;
        match &row.kind {
            DashboardRowKind::Parent => Some(row.folder.as_str()),
            DashboardRowKind::OwnedChild { .. } => None,
        }
    }

    fn selected_parent_count(&self) -> usize {
        self.selected_parents.len()
    }

    fn is_parent_selected(&self, folder: &str) -> bool {
        self.selected_parents.contains(folder)
    }

    fn toggle_selected_parent(&mut self) -> bool {
        let Some(parent_folder) = self.selected_parent_row_folder().map(str::to_string) else {
            return false;
        };

        self.toggle_selected_parent_folder(&parent_folder)
    }

    fn toggle_selected_parent_folder(&mut self, folder: &str) -> bool {
        if !self.items.iter().any(|item| item.folder == folder) {
            return false;
        }

        if !self.selected_parents.remove(folder) {
            self.selected_parents.insert(folder.to_string());
        }

        true
    }

    /// Selects every parent row currently visible (all addons when unfiltered).
    /// Selections hidden by an active filter are kept.
    fn select_all_parents(&mut self) -> bool {
        let before = self.selected_parents.len();
        let visible = self
            .rows
            .iter()
            .filter(|row| matches!(row.kind, DashboardRowKind::Parent))
            .map(|row| row.folder.clone())
            .collect::<Vec<_>>();
        self.selected_parents.extend(visible);
        self.selected_parents.len() != before
    }

    fn visible_parent_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.kind, DashboardRowKind::Parent))
            .count()
    }

    fn filter_query(&self) -> &str {
        self.filter_query.value()
    }

    fn filter_input(&self) -> &TextInput {
        &self.filter_query
    }

    fn filter_editing(&self) -> bool {
        self.filter_editing
    }

    fn has_filter(&self) -> bool {
        !self.filter_query.is_empty()
    }

    /// The filter bar is shown while editing or while a filter is applied.
    fn filter_bar_visible(&self) -> bool {
        self.filter_editing || self.has_filter()
    }

    /// Updates the filter and rebuilds the visible rows, keeping the cursor on
    /// the same addon when it is still visible and clamping it otherwise.
    fn set_filter(&mut self, query: TextInput, editing: bool) {
        self.filter_editing = editing;
        if self.filter_query.value() == query.value() {
            // Only the cursor moved.
            self.filter_query = query;
            return;
        }

        let selected_key = self.selected_row().map(|row| row.key.clone());
        let selected_parent = self.selected_parent_folder().map(str::to_string);
        let previous_index = self.list_state.selected();
        self.filter_query = query;
        self.rebuild_rows();

        let selection = selected_key
            .and_then(|key| self.rows.iter().position(|row| row.key == key))
            .or_else(|| {
                selected_parent.and_then(|folder| {
                    self.rows
                        .iter()
                        .position(|row| row.key == DashboardRowKey::Parent(folder.clone()))
                })
            })
            .or_else(|| {
                (!self.rows.is_empty())
                    .then(|| previous_index.unwrap_or(0).min(self.rows.len() - 1))
            });
        self.list_state = TableState::default().with_selected(selection);
    }

    fn clear_selected_parents(&mut self) -> bool {
        if self.selected_parents.is_empty() {
            return false;
        }

        self.selected_parents.clear();
        true
    }

    fn selected_parent_folders(&self) -> Vec<String> {
        let mut folders = self.selected_parents.iter().cloned().collect::<Vec<_>>();
        folders.sort();
        folders
    }

    fn selected_parent_items(&self) -> Vec<&DashboardItem> {
        let mut items = self
            .items
            .iter()
            .filter(|item| self.selected_parents.contains(&item.folder))
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.folder.cmp(&right.folder));
        items
    }

    fn actionable_parent_targets(&self) -> Result<Vec<String>, &'static str> {
        let selected = self.selected_parent_folders();
        if !selected.is_empty() {
            return Ok(selected);
        }

        match self.selected_row() {
            Some(DashboardRow {
                kind: DashboardRowKind::Parent,
                folder,
                ..
            }) => Ok(vec![folder.clone()]),
            // A folder row acts for the addon it belongs to.
            Some(DashboardRow {
                kind: DashboardRowKind::OwnedChild { parent_folder },
                ..
            }) => Ok(vec![parent_folder.clone()]),
            None => Err("Move to an addon first"),
        }
    }

    fn set_selected_parents(&mut self, folders: Vec<String>) {
        self.selected_parents = folders
            .into_iter()
            .filter(|folder| self.items.iter().any(|item| item.folder == *folder))
            .collect();
    }

    fn set_pending_delete_folders(&mut self, folders: Option<Vec<String>>) {
        self.pending_delete_folders = folders.map(|mut folders| {
            folders.sort();
            folders.dedup();
            folders
        });
    }

    fn pending_delete_folders(&self) -> Option<&[String]> {
        self.pending_delete_folders.as_deref()
    }

    fn set_drift_report(&mut self, drift_report: Option<DriftReport>) {
        self.drift_report = drift_report;
    }

    fn drift_report(&self) -> Option<&DriftReport> {
        self.drift_report.as_ref()
    }

    fn set_update_in_progress(&mut self, in_progress: bool) {
        self.update_in_progress = in_progress;
    }

    fn update_in_progress(&self) -> bool {
        self.update_in_progress
    }

    fn set_job_ui(&mut self, job_ui: Option<DashboardJobUiState>) {
        match &job_ui {
            Some(job) => {
                if self.job_ui.is_none() {
                    // A new job: the last run's results no longer hold.
                    self.updated_folders.clear();
                    self.failed_folders.clear();
                }
                // Only the update phase counts; a preflight check reports
                // progress too.
                if job.kind == DashboardJobKind::Update
                    && !self.job_reached.contains(&job.current_addon)
                {
                    self.job_reached.push(job.current_addon.clone());
                }
            }
            None => {
                self.job_targets.clear();
                self.job_reached.clear();
            }
        }
        self.job_ui = job_ui;
    }

    fn begin_update_targets(&mut self, folders: &[String]) {
        self.job_targets = folders.to_vec();
        self.job_reached.clear();
        self.updated_folders.clear();
        self.failed_folders.clear();
    }

    fn set_update_results(&mut self, updated: Vec<String>, failed: Vec<String>) {
        self.updated_folders = updated;
        self.failed_folders = failed;
    }

    /// What the last update run reported for `folder`, if anything.
    fn update_result(&self, folder: &str) -> Option<UpdateJobRowState> {
        if self.failed_folders.iter().any(|failed| failed == folder) {
            Some(UpdateJobRowState::Failed)
        } else if self.updated_folders.iter().any(|updated| updated == folder) {
            Some(UpdateJobRowState::Updated)
        } else {
            None
        }
    }

    /// Where a row stands in the running update, if it is part of it.
    fn update_job_row_state(&self, folder: &str) -> Option<UpdateJobRowState> {
        let job = self.job_ui.as_ref()?;
        if job.kind != DashboardJobKind::Update {
            // An update's preflight check: its targets are still waiting,
            // except the one being checked.
            let waiting = job.current_addon != folder
                && self.job_targets.iter().any(|target| target == folder);
            return waiting.then_some(UpdateJobRowState::Waiting);
        }
        if job.current_addon == folder {
            return Some(UpdateJobRowState::Updating);
        }
        if self.job_reached.iter().any(|reached| reached == folder) {
            // The job has moved past it; whether it worked is not known yet.
            return Some(UpdateJobRowState::Done);
        }
        self.job_targets
            .iter()
            .any(|target| target == folder)
            .then_some(UpdateJobRowState::Waiting)
    }

    /// Addons with an update ready, in list order.
    fn update_ready_folders(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| {
                item.source != SourceKind::Manual
                    && dashboard_item_update_status(item) == UpdateStatus::UpdateAvailable
            })
            .map(|item| item.folder.clone())
            .collect()
    }

    /// Every addon LemonUp knows the source of.
    fn linked_folders(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| item.source != SourceKind::Manual)
            .map(|item| item.folder.clone())
            .collect()
    }

    fn job_ui(&self) -> Option<&DashboardJobUiState> {
        self.job_ui.as_ref()
    }

    fn set_last_update_summary(&mut self, summary: Option<DashboardUpdateRunSummary>) {
        self.last_update_summary = summary;
    }

    fn last_update_summary(&self) -> Option<&DashboardUpdateRunSummary> {
        self.last_update_summary.as_ref()
    }

    fn toggle_sort(&mut self, column: DashboardSortColumn) {
        let selected_key = self.selected_row().map(|row| row.key.clone());
        let previous_offset = self.list_state.offset();
        self.sort_config = self.sort_config.toggled(column);
        self.sort_items();
        self.rebuild_rows();
        if let Some(key) = selected_key {
            self.restore_selection(&key);
        } else if !self.rows.is_empty() {
            self.list_state.select(Some(0));
        }
        let max_offset = self.rows.len().saturating_sub(1);
        self.list_state
            .offset_mut()
            .clone_from(&previous_offset.min(max_offset));
    }

    fn sort_config(&self) -> DashboardSortConfig {
        self.sort_config
    }

    fn sort_items(&mut self) {
        let sort = self.sort_config;
        self.items.sort_by(|left, right| {
            let result = match sort.column {
                DashboardSortColumn::Name => compare_text(&left.name, &right.name)
                    .then_with(|| compare_text(&left.folder, &right.folder)),
                DashboardSortColumn::Status => dashboard_item_sort_version_key(left)
                    .cmp(&dashboard_item_sort_version_key(right))
                    .then_with(|| compare_text(&left.name, &right.name)),
                DashboardSortColumn::Author => compare_text(
                    &dashboard_item_author_sort_key(left),
                    &dashboard_item_author_sort_key(right),
                )
                .then_with(|| compare_text(&left.name, &right.name)),
                DashboardSortColumn::Source => compare_text(
                    dashboard_item_source_label(left),
                    dashboard_item_source_label(right),
                )
                .then_with(|| compare_text(&left.name, &right.name)),
            };

            match sort.direction {
                DashboardSortDirection::Asc => result,
                DashboardSortDirection::Desc => result.reverse(),
            }
        });
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        let terms = dashboard_filter_terms(self.filter_query.value());

        for item in &self.items {
            // A parent that only matches through its owned folders is shown expanded
            // so the matching child row is visible.
            let expanded_by_filter = match dashboard_item_filter_match(item, &terms) {
                DashboardFilterMatch::Hidden => continue,
                DashboardFilterMatch::Direct => false,
                DashboardFilterMatch::ViaOwnedFolders => true,
            };
            let expanded = expanded_by_filter || self.expanded_folders.contains(&item.folder);
            let expandable = !item.owned_folders.is_empty();
            rows.push(DashboardRow {
                key: DashboardRowKey::Parent(item.folder.clone()),
                name: item.name.clone(),
                folder: item.folder.clone(),
                kind: DashboardRowKind::Parent,
                expandable,
                expanded,
                child_connector: None,
            });

            if expanded {
                let child_count = item.owned_folders.len();
                for (index, child_folder) in item.owned_folders.iter().enumerate() {
                    rows.push(DashboardRow {
                        key: DashboardRowKey::OwnedChild {
                            parent_folder: item.folder.clone(),
                            child_folder: child_folder.clone(),
                        },
                        name: child_folder.clone(),
                        folder: child_folder.clone(),
                        kind: DashboardRowKind::OwnedChild {
                            parent_folder: item.folder.clone(),
                        },
                        expandable: false,
                        expanded: false,
                        child_connector: Some(if index + 1 == child_count {
                            DashboardChildConnector::Last
                        } else {
                            DashboardChildConnector::Mid
                        }),
                    });
                }
            }
        }

        self.rows = rows;
    }

    fn restore_selection(&mut self, key: &DashboardRowKey) {
        let selection = self
            .rows
            .iter()
            .position(|row| row.key == *key)
            .or(if self.rows.is_empty() { None } else { Some(0) });
        self.list_state.select(selection);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardFilterMatch {
    Hidden,
    /// Every term matches the addon's own name, folder, or author.
    Direct,
    /// Some term only matches one of the addon's owned folders.
    ViaOwnedFolders,
}

fn dashboard_filter_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Case-insensitive AND match: every term must be a substring of the title,
/// folder, author, or any owned folder name.
fn dashboard_item_filter_match(item: &DashboardItem, terms: &[String]) -> DashboardFilterMatch {
    if terms.is_empty() {
        return DashboardFilterMatch::Direct;
    }

    let own_fields = [
        Some(item.name.as_str()),
        Some(item.folder.as_str()),
        item.author.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::to_lowercase)
    .collect::<Vec<_>>();
    let owned_folders = item
        .owned_folders
        .iter()
        .map(|folder| folder.to_lowercase())
        .collect::<Vec<_>>();
    let in_own = |term: &String| own_fields.iter().any(|field| field.contains(term.as_str()));
    let in_owned = |term: &String| {
        owned_folders
            .iter()
            .any(|folder| folder.contains(term.as_str()))
    };

    if terms.iter().all(in_own) {
        DashboardFilterMatch::Direct
    } else if terms.iter().all(|term| in_own(term) || in_owned(term)) {
        DashboardFilterMatch::ViaOwnedFolders
    } else {
        DashboardFilterMatch::Hidden
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPaneState {
    input: TextInput,
    is_editing: bool,
}

impl Default for InstallPaneState {
    fn default() -> Self {
        Self {
            input: TextInput::default(),
            is_editing: true,
        }
    }
}

impl InstallPaneState {
    fn begin_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = true;
        next
    }

    fn stop_editing(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = false;
        next
    }

    fn edit_text(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        change(&mut next.input);
        next
    }

    fn insert_char(&self, character: char) -> Self {
        self.edit_text(|input| input.insert_char(character))
    }

    fn backspace(&self) -> Self {
        self.edit_text(TextInput::backspace)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchPresentationMode {
    ComposeFirst,
    TwoState,
}

/// Where keys go in the Install sheet: the box, or the results under it.
/// Typing a letter in the results goes back to the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchFocus {
    Box,
    Results,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPaneState {
    query: TextInput,
    presentation_mode: SearchPresentationMode,
    focus: SearchFocus,
    /// The Wago.io key box, open while a search waits for a key.
    key_box: Option<TextInput>,
    in_progress: bool,
    results: Vec<WagoSearchResult>,
    selected_result: Option<usize>,
    last_query: Option<String>,
    install_state: SearchInstallState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchInstallState {
    Idle,
    Checking {
        addon_id: String,
        addon_name: String,
    },
    ConfirmReplace {
        addon_id: String,
        addon_name: String,
        tracked_parent: String,
    },
    Installing {
        addon_id: String,
        addon_name: String,
    },
    Success {
        addon_id: String,
        addon_name: String,
        parent_folder: String,
        warning: Option<String>,
    },
    /// A GitHub, WoWInterface or TukUI target waiting for enter (install) or esc (cancel).
    ConfirmDirect {
        request: DirectInstallRequest,
    },
    InstallingDirect {
        request: DirectInstallRequest,
    },
    /// A direct install found its folders already in AddOns (say a "Not
    /// linked" copy): the replace question is open. `name` is the addon
    /// already there when known, `folders` what a yes replaces.
    ConfirmDirectReplace {
        request: DirectInstallRequest,
        name: String,
        folders: Vec<String>,
    },
    /// `addon_name` is `None` when the search itself failed.
    Error {
        addon_id: Option<String>,
        addon_name: Option<String>,
        provider: &'static str,
        kind: SourceErrorKind,
        detail: String,
        /// The folders in the way, for a refused install.
        clash: Option<FolderClash>,
    },
}

impl Default for SearchPaneState {
    fn default() -> Self {
        Self {
            query: TextInput::default(),
            presentation_mode: SearchPresentationMode::ComposeFirst,
            focus: SearchFocus::Box,
            key_box: None,
            in_progress: false,
            results: Vec::new(),
            selected_result: None,
            last_query: None,
            install_state: SearchInstallState::Idle,
        }
    }
}

impl SearchPaneState {
    /// Opening the sheet starts clean, so old text is never appended to.
    /// A search or install still running keeps its state.
    fn open_with_mode(&self, presentation_mode: SearchPresentationMode) -> Self {
        let busy = self.in_progress
            || matches!(
                self.install_state,
                SearchInstallState::Checking { .. }
                    | SearchInstallState::Installing { .. }
                    | SearchInstallState::InstallingDirect { .. }
            );
        let mut next = if busy { self.clone() } else { Self::default() };
        next.presentation_mode = presentation_mode;
        next.focus = SearchFocus::Box;
        next.key_box = None;
        next
    }

    /// Opens the sheet with `name` in the box and the cursor after it.
    fn open_prefilled(&self, name: &str) -> Self {
        let mut next = self.open_with_mode(SearchPresentationMode::ComposeFirst);
        next.query = TextInput::new(name).with_cursor_at_end();
        next
    }

    fn focus_box(&self) -> Self {
        let mut next = self.clone();
        next.focus = SearchFocus::Box;
        next
    }

    fn focus_results(&self) -> Self {
        let mut next = self.clone();
        if !next.results.is_empty() {
            next.focus = SearchFocus::Results;
            if next.selected_result.is_none() {
                next.selected_result = Some(0);
            }
        }
        next
    }

    fn results_focused(&self) -> bool {
        self.focus == SearchFocus::Results && !self.results.is_empty()
    }

    /// Editing the text leaves a finished install or a failure behind: the
    /// sheet shows what enter will do with the new text.
    fn edit_text(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        change(&mut next.query);
        next.focus = SearchFocus::Box;
        if matches!(
            next.install_state,
            SearchInstallState::Success { .. } | SearchInstallState::Error { .. }
        ) {
            next.install_state = SearchInstallState::Idle;
        }
        next
    }

    fn insert_char(&self, character: char) -> Self {
        self.edit_text(|query| query.insert_char(character))
    }

    fn backspace(&self) -> Self {
        self.edit_text(TextInput::backspace)
    }

    fn open_key_box(&self) -> Self {
        let mut next = self.clone();
        next.key_box = Some(TextInput::default());
        next.install_state = SearchInstallState::Idle;
        next
    }

    /// Wago.io refused the saved key: the key box comes back in place, the
    /// error kept so the sheet can say why.
    fn open_key_box_after_rejection(&self, _error: &SourceError) -> Self {
        let mut next = self.clone();
        next.key_box = Some(TextInput::default());
        next
    }

    fn close_key_box(&self) -> Self {
        let mut next = self.clone();
        next.key_box = None;
        next.focus = SearchFocus::Box;
        next
    }

    fn edit_key(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        if let Some(key) = next.key_box.as_mut() {
            change(key);
        }
        next
    }

    fn start_search(&self) -> Self {
        let mut next = self.clone();
        next.key_box = None;
        next.focus = SearchFocus::Box;
        next.in_progress = true;
        next.last_query = Some(next.query.value().trim().to_string());
        next.install_state = SearchInstallState::Idle;
        next
    }

    fn finish_search(&self, query: String, mut results: Vec<WagoSearchResult>) -> Self {
        let mut next = self.clone();
        next.in_progress = false;
        next.last_query = Some(query);
        sort_search_results(&mut results);
        next.results = results;
        next.install_state = SearchInstallState::Idle;
        next.selected_result = if next.results.is_empty() {
            None
        } else {
            Some(0)
        };
        if next.results.is_empty() {
            next.focus = SearchFocus::Box;
        }
        next
    }

    fn fail_search(&self, error: &SourceError) -> Self {
        let mut next = self.clone();
        next.in_progress = false;
        next.results.clear();
        next.selected_result = None;
        next.focus = SearchFocus::Box;
        next.install_state = SearchInstallState::Error {
            addon_id: None,
            addon_name: None,
            provider: "Wago.io",
            kind: error.kind,
            detail: error.detail.clone(),
            clash: None,
        };
        next
    }

    /// Down from the last result stays there; up from the first one goes
    /// back to the box.
    fn select_next(&self) -> Self {
        if self.results.is_empty() {
            return self.clone();
        }

        let mut next = self.clone();
        next.focus = SearchFocus::Results;
        next.selected_result = Some(match next.selected_result {
            Some(index) => (index + 1).min(next.results.len() - 1),
            None => 0,
        });
        next
    }

    fn select_previous(&self) -> Self {
        if self.results.is_empty() {
            return self.clone();
        }

        let mut next = self.clone();
        match next.selected_result {
            Some(0) | None => next.focus = SearchFocus::Box,
            Some(index) => next.selected_result = Some(index - 1),
        }
        next
    }

    fn selected_result(&self) -> Option<&WagoSearchResult> {
        self.selected_result
            .and_then(|index| self.results.get(index))
    }

    fn start_install_check(&self, result: &WagoSearchResult) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Checking {
            addon_id: result.id.clone(),
            addon_name: result.display_name.clone(),
        };
        next
    }

    fn start_direct_install_check(&self, addon_id: &str, addon_name: &str) -> Self {
        let mut next = self.clone();
        next.key_box = None;
        next.install_state = SearchInstallState::Checking {
            addon_id: addon_id.to_string(),
            addon_name: addon_name.to_string(),
        };
        next
    }

    fn require_install_confirmation(&self, inspection: &WagoInstallInspection) -> Self {
        let mut next = self.clone();
        next.install_state = if let Some(tracked_parent) = &inspection.tracked_parent {
            SearchInstallState::ConfirmReplace {
                addon_id: inspection.addon_id.clone(),
                addon_name: inspection.addon_name.clone(),
                tracked_parent: tracked_parent.clone(),
            }
        } else {
            SearchInstallState::Idle
        };
        next
    }

    fn start_installing(&self, inspection: &WagoInstallInspection) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Installing {
            addon_id: inspection.addon_id.clone(),
            addon_name: inspection.addon_name.clone(),
        };
        next
    }

    /// The addon is in the list now: the box empties for the next one.
    fn finished(&self, state: SearchInstallState) -> Self {
        let mut next = self.clone();
        next.query = TextInput::default();
        next.results.clear();
        next.selected_result = None;
        next.last_query = None;
        next.focus = SearchFocus::Box;
        next.install_state = state;
        next
    }

    fn finish_install(&self, summary: &WagoInstallSummary) -> Self {
        self.finished(SearchInstallState::Success {
            addon_id: summary.addon_id.clone(),
            addon_name: summary.addon_name.clone(),
            parent_folder: summary.parent_folder.clone(),
            warning: None,
        })
    }

    /// Shows the question for a direct (non-Wago) target; earlier Wago results are
    /// cleared so the overlay body shows the question and then the install progress.
    fn request_direct_install(&self, request: DirectInstallRequest) -> Self {
        let mut next = self.clone();
        next.key_box = None;
        next.focus = SearchFocus::Box;
        next.results.clear();
        next.selected_result = None;
        next.last_query = None;
        next.install_state = SearchInstallState::ConfirmDirect { request };
        next
    }

    fn cancel_direct_install(&self) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Idle;
        next
    }

    fn start_direct_install(&self, request: DirectInstallRequest) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::InstallingDirect { request };
        next
    }

    fn require_direct_replace(&self, request: DirectInstallRequest, clash: &FolderClash) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::ConfirmDirectReplace {
            name: clash
                .existing_name
                .clone()
                .unwrap_or_else(|| request.name()),
            request,
            folders: clash.replaceable.clone(),
        };
        next
    }

    /// The open replace question: the request, the addon's name and the
    /// folders a yes replaces.
    fn pending_direct_replace(&self) -> Option<(&DirectInstallRequest, &str, &[String])> {
        match &self.install_state {
            SearchInstallState::ConfirmDirectReplace {
                request,
                name,
                folders,
            } => Some((request, name.as_str(), folders.as_slice())),
            _ => None,
        }
    }

    fn finish_direct_install(
        &self,
        request: &DirectInstallRequest,
        outcome: &DirectInstallOutcome,
    ) -> Self {
        self.finished(SearchInstallState::Success {
            addon_id: request.target.clone(),
            addon_name: outcome.addon_name.clone(),
            parent_folder: outcome.parent_folder.clone(),
            warning: outcome.warning.clone(),
        })
    }

    fn pending_direct_install(&self) -> Option<&DirectInstallRequest> {
        match &self.install_state {
            SearchInstallState::ConfirmDirect { request } => Some(request),
            _ => None,
        }
    }

    fn fail_install(
        &self,
        addon_id: Option<String>,
        addon_name: Option<String>,
        provider: &'static str,
        error: &SourceError,
    ) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Error {
            addon_id,
            addon_name,
            provider,
            kind: error.kind,
            detail: error.detail.clone(),
            clash: error.clash.clone(),
        };
        next
    }

    fn clear_install_state(&self) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Idle;
        next
    }

    /// Whether a search or an install is running, so enter waits.
    fn busy(&self) -> bool {
        self.in_progress
            || matches!(
                self.install_state,
                SearchInstallState::Checking { .. }
                    | SearchInstallState::Installing { .. }
                    | SearchInstallState::InstallingDirect { .. }
            )
    }

    fn install_identity(&self) -> (Option<String>, Option<String>) {
        match &self.install_state {
            SearchInstallState::Idle => (None, None),
            SearchInstallState::Checking {
                addon_id,
                addon_name,
            }
            | SearchInstallState::ConfirmReplace {
                addon_id,
                addon_name,
                ..
            }
            | SearchInstallState::Installing {
                addon_id,
                addon_name,
            }
            | SearchInstallState::Success {
                addon_id,
                addon_name,
                ..
            } => (Some(addon_id.clone()), Some(addon_name.clone())),
            SearchInstallState::ConfirmDirect { request }
            | SearchInstallState::InstallingDirect { request }
            | SearchInstallState::ConfirmDirectReplace { request, .. } => {
                (Some(request.target.clone()), Some(request.name()))
            }
            SearchInstallState::Error {
                addon_id,
                addon_name,
                ..
            } => (addon_id.clone(), addon_name.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WagoInstallSource {
    DirectInput,
    SearchResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWagoInstallRequest {
    target: String,
    source: WagoInstallSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WagoInstallConfirmation {
    request: PendingWagoInstallRequest,
    inspection: WagoInstallInspection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScanState {
    Idle,
    Pending,
    Running(PathBuf),
    Succeeded { path: PathBuf, summary: ScanSummary },
    Failed(String),
}

pub struct App {
    shell_mode: ShellMode,
    quit_requested: bool,
    status_line: StatusMessage,
    ui_theme: UiTheme,
    shell_ui: ShellUiState,
    config_present: bool,
    config_pane: ConfigPaneState,
    backup_pane: BackupPaneState,
    wago_api_key: Option<String>,
    config_store: ConfigStore,
    state_db_file: PathBuf,
    trash_dir: PathBuf,
    /// Profile data folder: holds the backup store and pre-rebind snapshots.
    data_dir: PathBuf,
    /// WTF backups of the current client (`backup_root/<client>`).
    backup_dir: PathBuf,
    runtime: AppRuntime,
    effective_addon_dir: Option<PathBuf>,
    scan_state: ScanState,
    dashboard: DashboardState,
    install_pane: InstallPaneState,
    search_pane: SearchPaneState,
    pending_wago_install_confirmation: Option<WagoInstallConfirmation>,
    wago_install_in_progress: bool,
    direct_install_in_progress: bool,
    auto_check_in_progress: bool,
    last_auto_check_started_at: Option<Instant>,
    undo_delete: Option<DashboardUndoDeleteState>,
    undo_delete_in_progress: bool,
    /// A delete from the list is moving folders to the trash.
    delete_in_progress: bool,
    /// When the key being handled was read from the terminal; `None` in tests.
    key_read_at: Option<Instant>,
    /// The question last drawn and when it first was, so a key typed before
    /// a question appeared never answers it.
    question_shown: Option<(dialog::Question, Instant)>,
    last_dashboard_list_area: Option<Rect>,
    /// The window width at the last draw: Details docks beside the list at
    /// 120 columns and wider. `None` until the first draw.
    last_frame_width: Option<u16>,
    /// The open sheet as last drawn, so keys can scroll it.
    sheet_scroll: std::cell::Cell<SheetScroll>,
    onboarding: OnboardingState,
    task_events_tx: mpsc::UnboundedSender<AppTaskEvent>,
    task_events_rx: mpsc::UnboundedReceiver<AppTaskEvent>,
    active_onboarding_cancel: Option<Arc<AtomicBool>>,
    /// One-line startup notice (v1 migration result) and whether it is an error.
    startup_notice: Option<(String, bool)>,
    /// `q` or Ctrl+C during an update asked whether to quit.
    quit_question_open: bool,
}

impl App {
    pub fn bootstrap(paths: AppPaths, runtime: AppRuntime) -> lemonup_core::Result<Self> {
        let config_store = ConfigStore::new(paths.config_file);
        let config_state = config_store.load()?;
        let state_db_file = paths.state_db_file.clone();
        let trash_dir = paths.data_dir.join("trash");
        let data_dir = paths.data_dir.clone();
        let backup_store = backup_root(&data_dir);
        let database = StateDatabase::open(state_db_file.clone())?;
        let addons = database.list_addons()?;
        let config_present = matches!(config_state, ConfigLoad::Loaded(_));
        let wago_api_key = resolve_wago_api_key(&config_state);
        let loaded_config = match &config_state {
            ConfigLoad::Loaded(config) => config.clone(),
            ConfigLoad::Missing(_) => AppConfig::new_unconfigured(),
        };
        let configured_addon_dir = match &config_state {
            ConfigLoad::Loaded(config) => config.addon_dir.clone(),
            ConfigLoad::Missing(_) => None,
        };
        let validated_override = match runtime.addon_dir_override.as_ref() {
            Some(path) => {
                if runtime.is_guarded_path(path) {
                    return Err(lemonup_core::LemonupError::InvalidArgument(format!(
                        "invalid --addon-dir for profile '{}': {} | reason: this matches the default profile AddOns directory | hint: omit --addon-dir to use the Location Finder, or pass a valid sandbox _retail_ or _forever_ \\Interface\\AddOns path",
                        runtime.profile_name,
                        path.display()
                    )));
                }

                validate_addons_path(path).map_err(|error| {
                    lemonup_core::LemonupError::InvalidArgument(format!(
                        "invalid --addon-dir for profile '{}': {} | reason: {} | hint: omit --addon-dir to use the Location Finder, or pass a valid _retail_ or _forever_ \\Interface\\AddOns path",
                        runtime.profile_name,
                        path.display(),
                        error
                    ))
                })?;

                Some(path.clone())
            }
            None => None,
        };
        let mut effective_addon_dir = validated_override.or(configured_addon_dir.clone());

        // The profile's state belongs to one AddOns folder. An `--addon-dir` for another
        // folder is refused outright; a config pointing elsewhere (edited by hand) sends
        // the user through setup, where confirming a folder rebinds on purpose.
        let mut binding_notice = None;
        if let Some(path) = effective_addon_dir.as_ref()
            && validate_addons_path(path).is_ok()
            && !runtime.is_guarded_path(path)
        {
            match check_addon_dir_binding(
                &database,
                &runtime.profile_name,
                configured_addon_dir.as_deref(),
                path,
                true,
            )? {
                BindingCheck::Bound => {
                    if let Err(error) = adopt_legacy_backups(&backup_store, path) {
                        tracing::warn!(%error, "could not move legacy WTF backups into the client folder");
                    }
                }
                BindingCheck::Mismatch(mismatch) => {
                    if runtime.addon_dir_override.is_some() {
                        return Err(lemonup_core::LemonupError::InvalidArgument(format!(
                            "invalid --addon-dir for profile '{}': {mismatch}",
                            runtime.profile_name
                        )));
                    }
                    binding_notice = Some(format!(
                        "this profile tracks addons in {} but the config points at {} | confirm the AddOns folder below to move the profile there (tracked addons are kept and exported to a snapshot first), or use a separate --profile for another install",
                        mismatch.bound.display(),
                        mismatch.requested.display()
                    ));
                    effective_addon_dir = None;
                }
            }
        }
        let backup_dir = effective_addon_dir
            .as_deref()
            .map(|path| client_backup_dir(&backup_store, path))
            .unwrap_or_else(|| backup_store.clone());

        let onboarding = runtime
            .addon_dir_override
            .as_ref()
            .map(|path| {
                let mut config = loaded_config.clone();
                config.addon_dir = Some(path.clone());
                OnboardingState::from_config_with_input(config, path.display().to_string())
            })
            .unwrap_or_else(|| OnboardingState::from_config(loaded_config.clone()));

        let shell_mode = match effective_addon_dir.as_ref() {
            Some(path) if validate_addons_path(path).is_ok() && !runtime.is_guarded_path(path) => {
                ShellMode::Dashboard
            }
            _ => ShellMode::Onboarding,
        };

        let scan_state = if shell_mode == ShellMode::Dashboard {
            ScanState::Pending
        } else {
            ScanState::Idle
        };
        let (task_events_tx, task_events_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            shell_mode,
            quit_requested: false,
            status_line: StatusMessage::cleared(),
            ui_theme: UiTheme::from_mode(loaded_config.theme),
            shell_ui: ShellUiState::default(),
            config_present,
            config_pane: ConfigPaneState::new(loaded_config),
            backup_pane: BackupPaneState {
                backups: list_backups(&backup_dir).unwrap_or_default(),
                in_progress: false,
                selected_backup: 0,
                pending_restore: None,
                pending_delete: None,
                last_completed_backup_at: None,
            },
            wago_api_key,
            config_store,
            state_db_file,
            trash_dir,
            data_dir,
            backup_dir,
            runtime,
            effective_addon_dir,
            scan_state,
            dashboard: DashboardState::from_addons(addons),
            install_pane: InstallPaneState::default(),
            search_pane: SearchPaneState::default(),
            pending_wago_install_confirmation: None,
            wago_install_in_progress: false,
            direct_install_in_progress: false,
            auto_check_in_progress: false,
            last_auto_check_started_at: None,
            undo_delete: None,
            undo_delete_in_progress: false,
            delete_in_progress: false,
            key_read_at: None,
            question_shown: None,
            last_dashboard_list_area: None,
            last_frame_width: None,
            sheet_scroll: std::cell::Cell::new(SheetScroll::default()),
            onboarding,
            task_events_tx,
            task_events_rx,
            active_onboarding_cancel: None,
            startup_notice: None,
            quit_question_open: false,
        };
        app.status_line = app.initial_status_line();
        if let Some(notice) = binding_notice {
            app.show_startup_notice(&notice, true);
        }
        Ok(app)
    }

    /// Shows a startup notice: a dashboard toast, or a line on the onboarding
    /// screen (onboarding replaces the status line on its first tick).
    pub fn show_startup_notice(&mut self, text: &str, is_error: bool) {
        self.status_line = self.with_base_status(
            if is_error {
                StatusKind::Problem
            } else {
                StatusKind::Done
            },
            text,
        );
        if self.shell_mode == ShellMode::Dashboard {
            self.shell_ui.status_flash = Some(StatusFlash {
                text: truncate_text(&text.replace(" | ", " · "), 160),
                kind: if is_error {
                    StatusKind::Problem
                } else {
                    StatusKind::Done
                },
                offer: None,
                // A startup notice gets longer than the usual fade: the
                // window may only just have opened.
                expires_at: (!is_error).then(|| Instant::now() + StdDuration::from_secs(8)),
            });
        }
        self.startup_notice = Some((text.to_string(), is_error));
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<Backend>,
        mut events: EventHandler,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while !self.quit_requested {
            self.process_background_events();
            events.set_tick_rate(self.desired_tick_rate());
            terminal.draw(|frame| self.draw(frame))?;

            let Some(event) = events.next().await else {
                break;
            };
            self.key_read_at = events.last_read_at();

            self.clear_problem_on_key_press(&event);
            let messages = self.messages_for_event(&event);
            if let Some(pulse) = self.footer_pulse_for_event(&event, &messages) {
                self.apply(AppAction::SetFooterKeyPulse(Some(pulse)));
            }
            for message in messages {
                let actions = self.update(message);
                for action in actions {
                    self.apply(action);
                }
            }
        }

        Ok(())
    }

    fn desired_tick_rate(&self) -> StdDuration {
        if self.has_active_animation() {
            ANIMATED_TICK_RATE
        } else {
            IDLE_TICK_RATE
        }
    }

    fn has_active_animation(&self) -> bool {
        if self.dashboard.job_ui().is_some() {
            return true;
        }

        if matches!(self.scan_state, ScanState::Pending | ScanState::Running(_)) {
            return true;
        }

        if self.shell_mode == ShellMode::Onboarding
            && matches!(
                self.onboarding.phase,
                OnboardingPhase::Bootstrapping
                    | OnboardingPhase::QuickChecking
                    | OnboardingPhase::DeepScanning(_)
            )
        {
            return true;
        }

        false
    }

    fn initial_status_line(&self) -> StatusMessage {
        match self.shell_mode {
            ShellMode::Dashboard => {
                if self.runtime.addon_dir_override.is_some() {
                    self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Overview,
                        "Using the AddOns folder from --addon-dir",
                    )
                } else if matches!(self.scan_state, ScanState::Pending) {
                    self.dashboard_status_for(
                        StatusKind::Quiet,
                        DetailMode::Overview,
                        "dashboard ready | initial scan pending",
                    )
                } else {
                    self.dashboard_status_for(
                        StatusKind::Quiet,
                        DetailMode::Overview,
                        "dashboard ready",
                    )
                }
            }
            ShellMode::Onboarding => match &self.onboarding.phase {
                OnboardingPhase::Error(message) => self.with_base_status(
                    StatusKind::Problem,
                    &format!("location finder blocked: {message}"),
                ),
                OnboardingPhase::Ready => self.location_finder_status(),
                _ => self.with_base_status(StatusKind::Quiet, "location finder"),
            },
        }
    }

    /// Rows the addon list shows at once, for PgUp and PgDn. Before the list
    /// has been drawn this falls back to a fixed page.
    fn dashboard_page_size(&self) -> usize {
        const FALLBACK_PAGE: usize = 10;
        let Some(area) = self.last_dashboard_list_area else {
            return FALLBACK_PAGE;
        };
        // The list area holds rows only: the column header and the filter
        // line sit above it.
        usize::from(area.height).max(1)
    }

    /// Starts a check of `targets`, or says why it cannot start.
    fn dashboard_check_actions(
        &self,
        targets: Result<Vec<String>, &'static str>,
    ) -> Vec<AppAction> {
        match targets {
            Err(message) => vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                self.dashboard.detail_mode,
                message,
            ))],
            Ok(folders) => {
                let target_len = folders.len();
                vec![
                    AppAction::StartDashboardCheckSelected {
                        folders,
                        wago_api_key: self.wago_api_key.clone(),
                        check_interval_secs: self.config_pane.draft.check_interval_secs,
                    },
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Working,
                        self.dashboard.detail_mode,
                        &format!(
                            "Checking {} addon{} for updates",
                            target_len,
                            plural_suffix(target_len)
                        ),
                    )),
                ]
            }
        }
    }

    /// Starts an update of `targets` through the usual update flow, or says
    /// why it cannot start.
    fn dashboard_update_actions(
        &self,
        targets: Result<Vec<String>, &'static str>,
    ) -> Vec<AppAction> {
        if self.dashboard.update_in_progress() {
            vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                self.dashboard.detail_mode,
                "An update is already running",
            ))]
        } else if let Some(busy) = self.install_running_status() {
            vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                self.dashboard.detail_mode,
                busy,
            ))]
        } else if self.dashboard.pending_delete_folders().is_some() {
            vec![AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Note,
                self.dashboard.detail_mode,
                "Answer the delete question first",
            ))]
        } else {
            match (self.effective_addon_dir.clone(), targets) {
                (_, Err(message)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    self.dashboard.detail_mode,
                    message,
                ))],
                (None, Ok(_)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Problem,
                    self.dashboard.detail_mode,
                    "LemonUp doesn't know your AddOns folder yet",
                ))],
                (Some(addon_dir), Ok(folders)) => {
                    let selected_len = folders.len();
                    vec![
                        AppAction::SetDashboardUpdateInProgress(true),
                        AppAction::StartDashboardUpdateSelected {
                            addon_dir,
                            folders,
                            wago_api_key: self.wago_api_key.clone(),
                            check_interval_secs: self.config_pane.draft.check_interval_secs,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            self.dashboard.detail_mode,
                            &format!(
                                "Updating {} addon{}",
                                selected_len,
                                plural_suffix(selected_len)
                            ),
                        )),
                    ]
                }
            }
        }
    }

    fn dashboard_move_actions(&self, selection: Option<usize>) -> Vec<AppAction> {
        vec![
            AppAction::SetDashboardSelection(selection),
            AppAction::SetStatus(self.dashboard_status_for(
                StatusKind::Quiet,
                self.dashboard.detail_mode,
                "selection moved",
            )),
        ]
    }

    fn dashboard_selection_for_pointer(&self, column: u16, row: u16) -> Option<usize> {
        // The list area holds rows only, with no border or header inside it.
        let area = self.last_dashboard_list_area?;
        if self.dashboard.rows.is_empty() || area.width == 0 || area.height == 0 {
            return None;
        }
        let right = area.x.saturating_add(area.width);
        let bottom = area.y.saturating_add(area.height);
        if column < area.x || column >= right || row < area.y || row >= bottom {
            return None;
        }

        let row_in_view = usize::from(row - area.y);
        let absolute = self
            .dashboard
            .list_state
            .offset()
            .saturating_add(row_in_view);
        (absolute < self.dashboard.rows.len()).then_some(absolute)
    }

    /// The message the status line shows, with its kind.
    #[cfg(test)]
    fn status_line_message(&self) -> Option<(String, StatusKind)> {
        self.shell_ui
            .status_flash
            .as_ref()
            .map(|flash| (flash.text.clone(), flash.kind))
    }

    /// A problem stays on the status line until the next key press.
    fn clear_problem_on_key_press(&mut self, event: &TerminalEvent) {
        if let TerminalEvent::Key(key) = event
            && key.kind == KeyEventKind::Press
            && self
                .shell_ui
                .status_flash
                .as_ref()
                .is_some_and(|flash| flash.kind == StatusKind::Problem)
        {
            self.shell_ui.status_flash = None;
        }
    }

    /// Whether the key a message offers still does what the message says.
    fn status_offer_works(&self, offer: StatusOffer) -> bool {
        match offer {
            StatusOffer::UndoDelete => self.undo_delete.is_some(),
            StatusOffer::UpdateAll => {
                !self.dashboard.update_in_progress()
                    && !self.dashboard.update_ready_folders().is_empty()
            }
        }
    }

    /// Whether a status line message should still show at `now`.
    fn status_flash_is_live(&self, flash: &StatusFlash, now: Instant) -> bool {
        if flash
            .offer
            .is_some_and(|offer| self.status_offer_works(offer))
        {
            return true;
        }
        flash.expires_at.is_none_or(|expires_at| now < expires_at)
    }

    fn process_background_events(&mut self) {
        while let Ok(event) = self.task_events_rx.try_recv() {
            let actions = self.update(AppMessage::BackgroundTask(event));
            for action in actions {
                self.apply(action);
            }
        }
    }

    fn messages_for_event(&self, event: &TerminalEvent) -> Vec<AppMessage> {
        match event {
            TerminalEvent::Tick => vec![AppMessage::Tick],
            TerminalEvent::Resize(width, height) => {
                vec![AppMessage::TerminalResized {
                    width: *width,
                    height: *height,
                }]
            }
            TerminalEvent::Key(key) => self.messages_for_key(*key),
            TerminalEvent::Mouse(mouse) => self.messages_for_mouse(*mouse),
            TerminalEvent::Paste(text) => vec![AppMessage::Paste(text.clone())],
        }
    }

    fn footer_pulse_for_event(
        &self,
        event: &TerminalEvent,
        messages: &[AppMessage],
    ) -> Option<FooterKeyPulse> {
        if messages.is_empty() {
            return None;
        }

        let TerminalEvent::Key(key) = event else {
            return None;
        };

        self.footer_hint_for_key(*key).map(|hint| FooterKeyPulse {
            hint,
            expires_at_tick: self.shell_ui.motion.tick_count + 2,
        })
    }

    fn messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        if key.kind != KeyEventKind::Press {
            return vec![];
        }

        // Ctrl+C quits from every screen and state, text boxes included.
        if is_ctrl_c(key) {
            return vec![AppMessage::QuitRequested];
        }
        // In a focused text box the arrows, Home, End, Delete and Ctrl+U
        // edit the text, before any sheet or list takes them.
        if let Some(edit) = text_edit_for_key(key)
            && self.focused_text_box().is_some()
        {
            return vec![AppMessage::TextBoxEdit(edit)];
        }
        // Any other Ctrl or Alt chord never falls through to the plain letter.
        if is_modified_char_chord(key) {
            return vec![];
        }

        match self.shell_mode {
            ShellMode::Onboarding => self.onboarding_messages_for_key(key),
            ShellMode::Dashboard => self.dashboard_messages_for_key(key),
        }
    }

    fn messages_for_mouse(&self, mouse: MouseEvent) -> Vec<AppMessage> {
        if self.shell_mode != ShellMode::Dashboard {
            return vec![];
        }

        // The list behind docked Details still takes the mouse; clicks in
        // the panel fall outside the list area and do nothing.
        if self.shell_ui.overlay.active.is_some() && !self.details_docked() {
            return vec![];
        }

        if self.active_question().is_some() {
            return vec![];
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => vec![AppMessage::DashboardSelectionNext],
            MouseEventKind::ScrollUp => vec![AppMessage::DashboardSelectionPrevious],
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                vec![AppMessage::DashboardPointerSelect {
                    column: mouse.column,
                    row: mouse.row,
                }]
            }
            _ => vec![],
        }
    }

    fn dashboard_messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        // An open question takes every key: only its answers do anything.
        if let Some(messages) = self.question_messages_for_key(key) {
            return messages;
        }

        // Docked Details is not a mode: the list keys keep working, and only
        // enter, esc and q close the panel. While the filter has focus its
        // keys come first, so typing a q filters.
        let docked_details = self.details_docked();
        if docked_details
            && !self.dashboard.filter_editing()
            && let KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') = key.code
        {
            return vec![AppMessage::DashboardCloseOverlay];
        }

        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) && !docked_details {
            return match key.code {
                KeyCode::Char('?') => vec![AppMessage::DashboardOpenHelp],
                // The arrows move to the previous or next addon; the sheet
                // starts at the top for each one.
                KeyCode::Down | KeyCode::Char('j') => vec![
                    AppMessage::DashboardSelectionNext,
                    AppMessage::SheetScroll(SheetMove::Top),
                ],
                KeyCode::Up | KeyCode::Char('k') => vec![
                    AppMessage::DashboardSelectionPrevious,
                    AppMessage::SheetScroll(SheetMove::Top),
                ],
                code if sheet_move_for_key(code, false).is_some() => {
                    vec![AppMessage::SheetScroll(
                        sheet_move_for_key(code, false).expect("checked scroll key"),
                    )]
                }
                KeyCode::Char('c') => vec![AppMessage::InspectRunCheck],
                KeyCode::Char('u') => vec![AppMessage::InspectRunUpdate],
                KeyCode::Char('x') => vec![AppMessage::InspectRequestDelete],
                KeyCode::Char(' ') => vec![AppMessage::InspectToggleSelected],
                KeyCode::Char('f') if self.not_linked_current_item().is_some() => {
                    vec![AppMessage::FindOnline]
                }
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => {
                    vec![AppMessage::DashboardCloseOverlay]
                }
                _ => vec![],
            };
        }

        if self.shell_ui.overlay.active == Some(OverlayKind::Help) {
            return match key.code {
                KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => {
                    vec![AppMessage::DashboardCloseOverlay]
                }
                code => sheet_move_for_key(code, true)
                    .map(AppMessage::SheetScroll)
                    .into_iter()
                    .collect(),
            };
        }

        // PgUp, PgDn, Home and End scroll any sheet, even where the arrows
        // move a cursor. In a focused text box Home and End have gone to the
        // box already.
        let sheet_open = self.shell_ui.overlay.active.is_some() && !docked_details;
        if sheet_open && let Some(movement) = sheet_move_for_key(key.code, false) {
            return vec![AppMessage::SheetScroll(movement)];
        }

        if let Some(messages) = self.search_messages_for_key(key) {
            return messages;
        }
        if let Some(messages) = self.install_messages_for_key(key) {
            return messages;
        }
        if let Some(messages) = self.config_messages_for_key(key) {
            return messages;
        }
        if let Some(messages) = self.backup_messages_for_key(key) {
            return messages;
        }

        if sheet_open {
            return match key.code {
                KeyCode::Char('?') => vec![AppMessage::DashboardOpenHelp],
                KeyCode::Char('q') | KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
                // Where the arrows do not move a cursor, they scroll.
                code => sheet_move_for_key(code, true)
                    .map(AppMessage::SheetScroll)
                    .into_iter()
                    .collect(),
            };
        }

        if self.dashboard.filter_editing() {
            return match key.code {
                KeyCode::Enter => vec![AppMessage::DashboardFilterApply],
                KeyCode::Esc => vec![AppMessage::DashboardFilterClear],
                KeyCode::Backspace => vec![AppMessage::DashboardFilterBackspace],
                KeyCode::Down => vec![AppMessage::DashboardSelectionNext],
                KeyCode::Up => vec![AppMessage::DashboardSelectionPrevious],
                KeyCode::Char(character) => vec![AppMessage::DashboardFilterInputChar(character)],
                _ => vec![],
            };
        }

        match key.code {
            KeyCode::Char('q') => vec![AppMessage::QuitRequested],
            KeyCode::Char('?') => vec![AppMessage::DashboardOpenHelp],
            KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::DashboardSelectionNext],
            KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::DashboardSelectionPrevious],
            KeyCode::PageDown => vec![AppMessage::DashboardSelectionPageDown],
            KeyCode::PageUp => vec![AppMessage::DashboardSelectionPageUp],
            KeyCode::Home => vec![AppMessage::DashboardSelectionFirst],
            KeyCode::End => vec![AppMessage::DashboardSelectionLast],
            KeyCode::Char('x') => vec![AppMessage::DashboardRequestDelete],
            KeyCode::Char('z') => vec![AppMessage::DashboardUndoDelete],
            KeyCode::Char(' ') => vec![AppMessage::DashboardToggleSelected],
            KeyCode::Char('a') => {
                if self.dashboard.selected_parent_count() > 0 {
                    vec![AppMessage::DashboardClearSelection]
                } else {
                    vec![AppMessage::DashboardSelectAll]
                }
            }
            KeyCode::Esc if self.dashboard.has_filter() => vec![AppMessage::DashboardFilterClear],
            KeyCode::Esc => vec![AppMessage::DashboardClearSelection],
            KeyCode::Char('/') => vec![AppMessage::DashboardFilterBegin],
            KeyCode::Enter if self.dashboard.detail_mode == DetailMode::Overview => {
                vec![AppMessage::DashboardOpenInspect]
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
                vec![AppMessage::DashboardToggleExpanded]
            }
            KeyCode::Left | KeyCode::Char('h') => vec![AppMessage::DashboardCollapseExpanded],
            KeyCode::Char(']') => vec![AppMessage::DashboardExpandAllRelationships],
            KeyCode::Char('[') => vec![AppMessage::DashboardCollapseAllRelationships],
            KeyCode::Char('i') => {
                vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
            }
            KeyCode::Char('1') => vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Name)],
            KeyCode::Char('2') => {
                vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Status)]
            }
            KeyCode::Char('3') => {
                vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Author)]
            }
            KeyCode::Char('4') => {
                vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Source)]
            }
            KeyCode::Char('u') => vec![AppMessage::DashboardRunUpdateSelected],
            KeyCode::Char('c') => vec![AppMessage::DashboardRunCheckSelected],
            KeyCode::Char('U') => vec![AppMessage::DashboardRunUpdateAll],
            KeyCode::Char('C') => vec![AppMessage::DashboardRunCheckAll],
            KeyCode::Char(',') => vec![AppMessage::SetDetailMode(DetailMode::Config)],
            KeyCode::Char('b') => vec![AppMessage::SetDetailMode(DetailMode::Backup)],
            KeyCode::Char('d') if self.first_run_card_visible() => {
                vec![AppMessage::DashboardHideFirstRunCard]
            }
            KeyCode::Char('f') if self.not_linked_current_item().is_some() => {
                vec![AppMessage::FindOnline]
            }
            _ => vec![],
        }
    }

    /// Which text field (if any) currently has focus. Mirrors the key routing
    /// order so pasted text lands exactly where typed characters would.
    fn focused_text_box(&self) -> Option<TextBox> {
        if self.shell_mode == ShellMode::Onboarding {
            return self.onboarding.is_editing.then_some(TextBox::Onboarding);
        }

        // Docked Details is not a sheet: the filter beside it keeps focus.
        let docked_details = self.details_docked();
        if self.active_question().is_some()
            || self.shell_ui.overlay.active == Some(OverlayKind::Help)
            || (self.shell_ui.overlay.active == Some(OverlayKind::Inspect) && !docked_details)
        {
            return None;
        }

        match self.dashboard.detail_mode {
            DetailMode::Search if self.search_pane.key_box.is_some() => {
                return Some(TextBox::WagoKey);
            }
            // While an install runs the box takes no text, so a letter
            // typed then can never answer a question the install asks.
            DetailMode::Search
                if self.search_pane.pending_direct_install().is_none()
                    && !self.search_pane.results_focused()
                    && !self.install_running() =>
            {
                return Some(TextBox::Search);
            }
            DetailMode::Install if self.install_pane.is_editing => {
                return Some(TextBox::Install);
            }
            DetailMode::Config if self.config_pane.edit.is_some() => {
                return Some(TextBox::Config);
            }
            _ => {}
        }

        if (self.shell_ui.overlay.active.is_none() || docked_details)
            && self.dashboard.filter_editing()
        {
            return Some(TextBox::DashboardFilter);
        }

        None
    }

    fn paste_actions(&self, text: &str) -> Vec<AppAction> {
        let Some(target) = self.focused_text_box() else {
            return vec![];
        };
        let text = sanitize_single_line_paste(text);
        if text.is_empty() {
            return vec![];
        }
        self.text_box_actions(target, |input| input.insert_str(&text))
    }

    /// Applies `change` to the text in `target`, at its cursor.
    fn text_box_actions(
        &self,
        target: TextBox,
        change: impl FnOnce(&mut TextInput),
    ) -> Vec<AppAction> {
        match target {
            TextBox::Onboarding => vec![AppAction::SetOnboardingState(
                self.onboarding.edit_text(change),
            )],
            TextBox::Search => vec![AppAction::SetSearchPaneState(
                self.search_pane.edit_text(change),
            )],
            TextBox::WagoKey => vec![AppAction::SetSearchPaneState(
                self.search_pane.edit_key(change),
            )],
            TextBox::Install => vec![AppAction::SetInstallPaneState(
                self.install_pane.edit_text(change),
            )],
            TextBox::Config => vec![AppAction::SetConfigPaneState(
                self.config_pane.edit_text(change),
            )],
            TextBox::DashboardFilter => {
                let mut query = self.dashboard.filter_input().clone();
                change(&mut query);
                vec![AppAction::SetDashboardFilter {
                    query,
                    editing: true,
                }]
            }
        }
    }

    /// The Install sheet. Typing always goes to the box: there is no edit
    /// mode. `↓` moves into the results and `↑` from the first one comes
    /// back; a letter in the results goes back to the box. `esc` backs out
    /// of a question or the key box first, then closes. `q` is text.
    fn search_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Search {
            return None;
        }
        let pane = &self.search_pane;

        // While an install runs only esc does anything (ctrl+c quits earlier).
        if pane.key_box.is_none() && self.install_running() {
            return Some(match key.code {
                KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
                _ => vec![],
            });
        }

        if pane.pending_direct_install().is_some() {
            return Some(match key.code {
                KeyCode::Enter => vec![AppMessage::SearchConfirmDirectInstall],
                KeyCode::Esc => vec![AppMessage::SearchCancelDirectInstall],
                KeyCode::Backspace => vec![
                    AppMessage::SearchCancelDirectInstall,
                    AppMessage::SearchBackspace,
                ],
                KeyCode::Char(character) => vec![
                    AppMessage::SearchCancelDirectInstall,
                    AppMessage::SearchInputChar(character),
                ],
                _ => vec![],
            });
        }

        if pane.key_box.is_some() {
            return Some(match key.code {
                KeyCode::Enter => vec![AppMessage::SearchSaveKey],
                KeyCode::Esc => vec![AppMessage::SearchCloseKeyBox],
                KeyCode::Backspace => vec![AppMessage::SearchKeyBackspace],
                KeyCode::Char(character) => vec![AppMessage::SearchKeyChar(character)],
                _ => vec![],
            });
        }

        if pane.results_focused() {
            return Some(match key.code {
                KeyCode::Down => vec![AppMessage::SearchResultNext],
                KeyCode::Up => vec![AppMessage::SearchResultPrevious],
                KeyCode::Enter => vec![AppMessage::SearchInstallSelected],
                KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
                KeyCode::Char('?') => vec![AppMessage::DashboardOpenHelp],
                KeyCode::Backspace => {
                    vec![AppMessage::SearchFocusBox, AppMessage::SearchBackspace]
                }
                KeyCode::Char(character) => vec![
                    AppMessage::SearchFocusBox,
                    AppMessage::SearchInputChar(character),
                ],
                _ => vec![],
            });
        }

        Some(match key.code {
            KeyCode::Enter => vec![AppMessage::SearchSubmit],
            KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
            KeyCode::Down if !pane.results.is_empty() => vec![AppMessage::SearchFocusResults],
            // `?` is a letter in a box with text; Help opens from an empty one.
            KeyCode::Char('?') if pane.query.is_empty() => vec![AppMessage::DashboardOpenHelp],
            KeyCode::Backspace => vec![AppMessage::SearchBackspace],
            KeyCode::Char(character) => vec![AppMessage::SearchInputChar(character)],
            _ => vec![],
        })
    }

    fn install_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Install {
            return None;
        }

        if self.wago_api_key.is_none() && matches!(key.code, KeyCode::Char(',')) {
            return Some(vec![AppMessage::SetDetailMode(DetailMode::Config)]);
        }

        if self.install_pane.is_editing {
            return Some(match key.code {
                KeyCode::Enter => vec![AppMessage::InstallSubmit],
                KeyCode::Esc => vec![AppMessage::InstallStopEditing],
                KeyCode::Backspace => vec![AppMessage::InstallBackspace],
                KeyCode::Char(character) => vec![AppMessage::InstallInputChar(character)],
                _ => vec![],
            });
        }

        match key.code {
            KeyCode::Enter => Some(vec![AppMessage::InstallSubmit]),
            KeyCode::Char('e') => Some(vec![AppMessage::InstallBeginEditing]),
            _ => None,
        }
    }

    fn config_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Config {
            return None;
        }

        if self.config_pane.edit.is_some() {
            return Some(match key.code {
                KeyCode::Enter => vec![AppMessage::ConfigCommitEdit],
                KeyCode::Esc => vec![AppMessage::ConfigCancelEdit],
                KeyCode::Backspace => vec![AppMessage::ConfigBackspace],
                KeyCode::Char(character) => vec![AppMessage::ConfigInputChar(character)],
                _ => vec![],
            });
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(vec![AppMessage::ConfigSelectionNext]),
            KeyCode::Up | KeyCode::Char('k') => Some(vec![AppMessage::ConfigSelectionPrevious]),
            KeyCode::Left | KeyCode::Char('h') => Some(
                if self.config_pane.selected_field().supports_quick_adjust() {
                    vec![AppMessage::ConfigAdjustSelected(-1)]
                } else {
                    vec![]
                },
            ),
            KeyCode::Right | KeyCode::Char('l') => Some(
                if self.config_pane.selected_field().supports_quick_adjust() {
                    vec![AppMessage::ConfigAdjustSelected(1)]
                } else {
                    vec![]
                },
            ),
            KeyCode::Char('e') => Some(vec![AppMessage::ConfigBeginEditing]),
            KeyCode::Enter => Some(vec![AppMessage::ConfigToggleSelected]),
            KeyCode::Char('r') => Some(vec![AppMessage::ConfigRunOnboarding]),
            KeyCode::Char('x') => Some(vec![AppMessage::ConfigExportAddons]),
            KeyCode::Char('i') => Some(vec![AppMessage::ConfigImportAddons]),
            _ => None,
        }
    }

    fn backup_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Backup {
            return None;
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(vec![AppMessage::BackupSelectionNext]),
            KeyCode::Up | KeyCode::Char('k') => Some(vec![AppMessage::BackupSelectionPrevious]),
            KeyCode::Char('n') => Some(vec![AppMessage::BackupRunNow]),
            KeyCode::Char('r') | KeyCode::Enter => Some(vec![AppMessage::BackupRequestRestore]),
            KeyCode::Char('x') => Some(vec![AppMessage::BackupRequestDelete]),
            _ => None,
        }
    }

    fn onboarding_messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        if self.onboarding.is_editing {
            return match key.code {
                KeyCode::Enter => match self.onboarding.step {
                    OnboardingStep::Directory => vec![AppMessage::OnboardingValidateInput],
                    OnboardingStep::Wago => vec![AppMessage::OnboardingStopEditing],
                    _ => vec![],
                },
                KeyCode::Esc => vec![AppMessage::OnboardingStopEditing],
                KeyCode::Backspace => vec![AppMessage::OnboardingBackspace],
                KeyCode::Char(character) => vec![AppMessage::OnboardingInputChar(character)],
                _ => vec![],
            };
        }

        match self.onboarding.step {
            OnboardingStep::Theme => match key.code {
                KeyCode::Left | KeyCode::Right => vec![AppMessage::OnboardingToggleTheme],
                KeyCode::Enter => vec![AppMessage::OnboardingNextStep],
                KeyCode::Esc => vec![AppMessage::OnboardingCancel],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingStep::Directory => match &self.onboarding.phase {
                OnboardingPhase::Bootstrapping | OnboardingPhase::QuickChecking => match key.code {
                    KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                    KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                    _ => vec![],
                },
                OnboardingPhase::DeepScanning(_) => match key.code {
                    KeyCode::Esc => vec![AppMessage::OnboardingCancel],
                    KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                    _ => vec![],
                },
                OnboardingPhase::Found(_) => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        vec![AppMessage::OnboardingFoundActionNext]
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        vec![AppMessage::OnboardingFoundActionPrevious]
                    }
                    KeyCode::Enter => vec![AppMessage::OnboardingFoundConfirm],
                    KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                    KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                    _ => vec![],
                },
                OnboardingPhase::Ready | OnboardingPhase::Error(_) | OnboardingPhase::Cancelled => {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => {
                            vec![AppMessage::OnboardingSuggestionNext]
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            vec![AppMessage::OnboardingSuggestionPrevious]
                        }
                        KeyCode::Char('e') => vec![AppMessage::OnboardingBeginEditing],
                        KeyCode::Enter | KeyCode::Char('d') => vec![AppMessage::OnboardingDeepScan],
                        KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                        KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                        _ => vec![],
                    }
                }
            },
            OnboardingStep::Wago => match key.code {
                KeyCode::Char('e') => vec![AppMessage::OnboardingBeginEditing],
                KeyCode::Enter => vec![AppMessage::OnboardingNextStep],
                KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingStep::Settings => match key.code {
                KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::OnboardingSettingsNext],
                KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::OnboardingSettingsPrevious],
                KeyCode::Left | KeyCode::Char('h') => {
                    vec![AppMessage::OnboardingSettingDecrease]
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    vec![AppMessage::OnboardingSettingIncrease]
                }
                KeyCode::Enter => vec![AppMessage::OnboardingNextStep],
                KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingStep::Review => match key.code {
                KeyCode::Esc => vec![AppMessage::OnboardingPreviousStep],
                KeyCode::Enter => vec![AppMessage::OnboardingNextStep],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
        }
    }

    fn update(&self, message: AppMessage) -> Vec<AppAction> {
        match message {
            AppMessage::Tick => {
                let mut actions = vec![AppAction::AdvanceMotionTick];
                if self.shell_mode == ShellMode::Onboarding
                    && matches!(self.onboarding.phase, OnboardingPhase::Bootstrapping)
                {
                    actions.extend([
                        AppAction::SetOnboardingState(self.onboarding.begin_quick_check()),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Working,
                            "checking common install locations",
                        )),
                        AppAction::StartOnboardingQuickCheck,
                    ]);
                    actions
                } else if self.shell_mode == ShellMode::Dashboard
                    && matches!(self.scan_state, ScanState::Pending)
                {
                    actions.extend(
                        self.effective_addon_dir
                            .clone()
                            .map(AppAction::StartAddonScan),
                    );
                    actions
                } else if let Some(folders) = self.auto_check_due_targets() {
                    actions.extend([
                        AppAction::SetDashboardAutoCheckInProgress(true),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            DetailMode::Overview,
                            "Checking for updates in the background",
                        )),
                        AppAction::StartDashboardCheckSelected {
                            folders,
                            wago_api_key: self.wago_api_key.clone(),
                            check_interval_secs: self.config_pane.draft.check_interval_secs,
                        },
                    ]);
                    actions
                } else {
                    actions
                }
            }
            // During an update, q and Ctrl+C ask first; asked again (a
            // second Ctrl+C), they quit.
            AppMessage::QuitRequested
                if self.dashboard.update_in_progress() && !self.quit_question_open =>
            {
                vec![AppAction::SetQuitQuestion(true)]
            }
            AppMessage::QuitRequested | AppMessage::QuitConfirmed => vec![AppAction::Quit],
            AppMessage::QuitCancelled => vec![AppAction::SetQuitQuestion(false)],
            AppMessage::Paste(text) => self.paste_actions(&text),
            AppMessage::TextBoxEdit(edit) => match self.focused_text_box() {
                Some(target) => self.text_box_actions(target, |input| input.apply(edit)),
                None => vec![],
            },
            AppMessage::TerminalResized { width, height } => {
                vec![AppAction::SetStatus(self.with_base_status(
                    StatusKind::Quiet,
                    &format!("terminal resized to {width}x{height}"),
                ))]
            }
            AppMessage::DashboardSelectionNext => {
                let selection = self.dashboard.next_selection();
                vec![
                    AppAction::SetDashboardSelection(selection),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "selection moved",
                    )),
                ]
            }
            AppMessage::DashboardSelectionPageDown => {
                let page = self.dashboard_page_size();
                self.dashboard_move_actions(self.dashboard.offset_selection(page as isize))
            }
            AppMessage::DashboardSelectionPageUp => {
                let page = self.dashboard_page_size();
                self.dashboard_move_actions(self.dashboard.offset_selection(-(page as isize)))
            }
            AppMessage::DashboardSelectionFirst => {
                self.dashboard_move_actions(self.dashboard.first_selection())
            }
            AppMessage::DashboardSelectionLast => {
                self.dashboard_move_actions(self.dashboard.last_selection())
            }
            AppMessage::DashboardSelectionPrevious => {
                let selection = self.dashboard.previous_selection();
                vec![
                    AppAction::SetDashboardSelection(selection),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "selection moved",
                    )),
                ]
            }
            AppMessage::DashboardToggleSort(column) => vec![
                AppAction::ToggleDashboardSort(column),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Overview,
                    &sort_status_text(
                        column,
                        self.dashboard.sort_config().toggled(column).direction,
                    ),
                )),
            ],
            AppMessage::DashboardOpenInspect => {
                if self.dashboard.selected_row().is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Overview,
                        "Move to an addon first",
                    ))]
                } else {
                    vec![
                        AppAction::SetInspectOverlay(true),
                        AppAction::SetStatus(StatusMessage::cleared()),
                    ]
                }
            }
            AppMessage::DashboardHideFirstRunCard => {
                let mut saved = self.config_pane.persisted.clone();
                saved.hide_first_run_card = true;
                match self.config_store.write_config(&saved) {
                    Ok(()) => {
                        let mut pane = self.config_pane.clone();
                        pane.persisted = saved.clone();
                        pane.draft.hide_first_run_card = true;
                        vec![
                            AppAction::SetPersistedConfig(saved),
                            AppAction::SetConfigPaneState(pane),
                        ]
                    }
                    Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Overview,
                        &format!("Couldn't save your settings: {error}"),
                    ))],
                }
            }
            AppMessage::DashboardOpenHelp => vec![
                AppAction::SetHelpContext(self.current_help_context()),
                AppAction::SetHelpOverlay(true),
                AppAction::SetStatus(StatusMessage::cleared()),
            ],
            AppMessage::DashboardCloseOverlay => {
                if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
                    vec![
                        AppAction::SetInspectOverlay(false),
                        AppAction::SetStatus(StatusMessage::cleared()),
                    ]
                } else if self.shell_ui.overlay.active == Some(OverlayKind::Help) {
                    vec![
                        AppAction::SetHelpOverlay(false),
                        AppAction::SetStatus(StatusMessage::cleared()),
                    ]
                } else {
                    vec![
                        AppAction::SetStatus(StatusMessage::cleared()),
                        AppAction::SetDetailMode(DetailMode::Overview),
                    ]
                }
            }
            AppMessage::SheetScroll(movement) => vec![AppAction::SetSheetOffset(
                self.sheet_scroll
                    .get()
                    .moved(movement, self.shell_ui.overlay.active),
            )],
            AppMessage::InspectToggleSelected => match self.inspect_resolved_target() {
                Some(target) => {
                    let was_selected = self.dashboard.is_parent_selected(&target.item.folder);
                    let mut folders = self.dashboard.selected_parent_folders();
                    if was_selected {
                        folders.retain(|folder| folder != &target.item.folder);
                    } else {
                        folders.push(target.item.folder.clone());
                        folders.sort();
                        folders.dedup();
                    }
                    vec![
                        AppAction::SetSelectedDashboardParents(folders),
                        AppAction::SetStatus(if was_selected {
                            self.dashboard_status_for(
                                StatusKind::Quiet,
                                DetailMode::Overview,
                                &format!("cleared {}", target.item.name),
                            )
                        } else {
                            self.dashboard_status_for(
                                StatusKind::Quiet,
                                DetailMode::Overview,
                                &format!("selected {}", target.item.name),
                            )
                        }),
                    ]
                }
                None => vec![],
            },
            AppMessage::InspectRequestDelete => match self.inspect_resolved_target() {
                Some(target) => vec![
                    AppAction::SetPendingDelete(Some(vec![target.item.folder.clone()])),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Quiet,
                        DetailMode::Overview,
                        &format!("asked to delete {}", target.item.name),
                    )),
                ],
                None => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Overview,
                    "Move to an addon first",
                ))],
            },
            AppMessage::InspectRunCheck => match self.inspect_resolved_target() {
                Some(target) => vec![
                    AppAction::StartDashboardCheckSelected {
                        folders: vec![target.item.folder.clone()],
                        wago_api_key: self.wago_api_key.clone(),
                        check_interval_secs: self.config_pane.draft.check_interval_secs,
                    },
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Working,
                        DetailMode::Overview,
                        &format!("Checking {} for updates", target.item.name),
                    )),
                ],
                None => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Overview,
                    "Move to an addon first",
                ))],
            },
            AppMessage::InspectRunUpdate => {
                if self.dashboard.update_in_progress() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Overview,
                        "An update is already running",
                    ))]
                } else if let Some(busy) = self.install_running_status() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Overview,
                        busy,
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Overview,
                        "Answer the delete question first",
                    ))]
                } else {
                    match (
                        self.effective_addon_dir.clone(),
                        self.inspect_resolved_target(),
                    ) {
                        (_, None) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Note,
                            DetailMode::Overview,
                            "Move to an addon first",
                        ))],
                        (None, Some(_)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Overview,
                            "LemonUp doesn't know your AddOns folder yet",
                        ))],
                        (Some(_), Some(target)) if !is_refreshable_dashboard_item(target.item) => {
                            vec![AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Note,
                                DetailMode::Overview,
                                &format!(
                                    "LemonUp can't update {} yet: it isn't linked",
                                    target.item.name
                                ),
                            ))]
                        }
                        (Some(addon_dir), Some(target)) => vec![
                            AppAction::SetDashboardUpdateInProgress(true),
                            AppAction::StartDashboardUpdateSelected {
                                addon_dir,
                                folders: vec![target.item.folder.clone()],
                                wago_api_key: self.wago_api_key.clone(),
                                check_interval_secs: self.config_pane.draft.check_interval_secs,
                            },
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Working,
                                DetailMode::Overview,
                                &format!("Updating {}", target.item.name),
                            )),
                        ],
                    }
                }
            }
            AppMessage::DashboardPointerSelect { column, row } => {
                match self.dashboard_selection_for_pointer(column, row) {
                    Some(selection) if self.dashboard.list_state.selected() != Some(selection) => {
                        vec![
                            AppAction::SetDashboardSelection(Some(selection)),
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Quiet,
                                self.dashboard.detail_mode,
                                "selection moved",
                            )),
                        ]
                    }
                    Some(_) => vec![],
                    None => vec![],
                }
            }
            AppMessage::DashboardRequestDelete => {
                if let Some(busy) = self.install_running_status() {
                    return vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        busy,
                    ))];
                }
                let selected_count = self.dashboard.selected_parent_count();
                match self.dashboard.actionable_parent_targets() {
                    Err(message) => vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        message,
                    ))],
                    Ok(folders) => {
                        let question = if selected_count == 0 {
                            let name = self
                                .dashboard
                                .selected_row()
                                .map_or_else(|| folders[0].clone(), |row| row.name.clone());
                            format!("asked to delete {name}")
                        } else {
                            format!(
                                "asked to delete {} addon{}",
                                folders.len(),
                                plural_suffix(folders.len())
                            )
                        };
                        vec![
                            AppAction::SetPendingDelete(Some(folders)),
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Quiet,
                                self.dashboard.detail_mode,
                                &question,
                            )),
                        ]
                    }
                }
            }
            AppMessage::DashboardConfirmDelete => match (
                self.effective_addon_dir.clone(),
                self.dashboard
                    .pending_delete_folders()
                    .map(|value| value.to_vec()),
            ) {
                (Some(addon_dir), Some(folders)) if !folders.is_empty() => vec![
                    AppAction::SetDashboardDeleteInProgress(true),
                    AppAction::StartDashboardDelete {
                        addon_dir,
                        trash_dir: self.trash_dir.clone(),
                        folders,
                    },
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Working,
                        self.dashboard.detail_mode,
                        "Deleting…",
                    )),
                ],
                _ => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    self.dashboard.detail_mode,
                    "Nothing to delete",
                ))],
            },
            AppMessage::DashboardCancelPendingDelete => vec![
                AppAction::SetPendingDelete(None),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "delete question closed",
                )),
            ],
            AppMessage::DashboardUndoDelete => {
                if let Some(busy) = self.install_running_status() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        busy,
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "Answer the delete question first",
                    ))]
                } else if self.undo_delete_in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "Already undoing the delete",
                    ))]
                } else if let Some(undo) = self.undo_delete.clone() {
                    vec![
                        AppAction::SetDashboardUndoInProgress(true),
                        AppAction::StartDashboardUndoDelete { undo: undo.clone() },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            self.dashboard.detail_mode,
                            &format!(
                                "Undoing delete of {} addon{} ({} folder{})",
                                undo.parent_count(),
                                plural_suffix(undo.parent_count()),
                                undo.moved_folder_count(),
                                plural_suffix(undo.moved_folder_count())
                            ),
                        )),
                    ]
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "Nothing to undo",
                    ))]
                }
            }
            AppMessage::DashboardRunCheckAll => {
                let folders = self.dashboard.linked_folders();
                if folders.is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "Nothing to check: none of these addons are linked",
                    ))]
                } else {
                    self.dashboard_check_actions(Ok(folders))
                }
            }
            AppMessage::DashboardRunUpdateAll => {
                let folders = self.dashboard.update_ready_folders();
                if folders.is_empty() && !self.dashboard.update_in_progress() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "No updates are ready",
                    ))]
                } else {
                    self.dashboard_update_actions(Ok(folders))
                }
            }
            AppMessage::DashboardRunCheckSelected => {
                self.dashboard_check_actions(self.dashboard.actionable_parent_targets())
            }
            AppMessage::DashboardRunUpdateSelected => {
                self.dashboard_update_actions(self.dashboard.actionable_parent_targets())
            }
            AppMessage::DashboardToggleSelected => {
                vec![
                    AppAction::ToggleDashboardSelection,
                    AppAction::SetStatus(match self.dashboard.selected_row() {
                        Some(DashboardRow {
                            kind: DashboardRowKind::OwnedChild { .. },
                            ..
                        }) => self.dashboard_status_for(
                            StatusKind::Note,
                            self.dashboard.detail_mode,
                            "Move to the addon this folder belongs to first",
                        ),
                        _ => self.dashboard_status_for(
                            StatusKind::Quiet,
                            self.dashboard.detail_mode,
                            &format!(
                                "selected {} addon{}",
                                selection_count_after_toggle(&self.dashboard),
                                plural_suffix(selection_count_after_toggle(&self.dashboard))
                            ),
                        ),
                    }),
                ]
            }
            AppMessage::DashboardSelectAll => vec![
                AppAction::SelectAllDashboardParents,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    &format!(
                        "selected all {} addon{}",
                        self.dashboard.visible_parent_count(),
                        plural_suffix(self.dashboard.visible_parent_count())
                    ),
                )),
            ],
            AppMessage::DashboardFilterBegin => vec![AppAction::SetDashboardFilter {
                query: self.dashboard.filter_input().clone().with_cursor_at_end(),
                editing: true,
            }],
            AppMessage::DashboardFilterInputChar(character) => self
                .text_box_actions(TextBox::DashboardFilter, |query| {
                    query.insert_char(character)
                }),
            AppMessage::DashboardFilterBackspace => {
                self.text_box_actions(TextBox::DashboardFilter, TextInput::backspace)
            }
            AppMessage::DashboardFilterApply => vec![AppAction::SetDashboardFilter {
                query: self.dashboard.filter_input().clone(),
                editing: false,
            }],
            AppMessage::DashboardFilterClear => vec![AppAction::SetDashboardFilter {
                query: TextInput::default(),
                editing: false,
            }],
            AppMessage::DashboardClearSelection => vec![
                AppAction::ClearDashboardSelection,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    self.dashboard.detail_mode,
                    "selection cleared",
                )),
            ],
            AppMessage::DashboardToggleExpanded => vec![
                AppAction::ToggleDashboardExpanded,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "tree state updated",
                )),
            ],
            AppMessage::DashboardExpandAllRelationships => vec![
                AppAction::ExpandAllDashboardRelationships,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "expanded all relationship rows",
                )),
            ],
            AppMessage::DashboardCollapseAllRelationships => vec![
                AppAction::CollapseAllDashboardRelationships,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "collapsed all relationship rows",
                )),
            ],
            AppMessage::DashboardCollapseExpanded => vec![
                AppAction::CollapseDashboardExpanded,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "tree state updated",
                )),
            ],
            AppMessage::SetDetailMode(detail_mode) => vec![
                AppAction::SetDetailMode(detail_mode),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    detail_mode,
                    &format!("{} selected", detail_mode_label(detail_mode)),
                )),
            ],
            AppMessage::InstallBeginEditing => vec![
                AppAction::SetInstallPaneState(self.install_pane.begin_editing()),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Install,
                    "editing Wago.io link | enter install | esc stop editing",
                )),
            ],
            AppMessage::InstallStopEditing => vec![
                AppAction::SetInstallPaneState(self.install_pane.stop_editing()),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Install,
                    "Wago.io install ready | e edit link | enter install",
                )),
            ],
            AppMessage::InstallInputChar(character) => vec![AppAction::SetInstallPaneState(
                self.install_pane.insert_char(character),
            )],
            AppMessage::InstallBackspace => vec![AppAction::SetInstallPaneState(
                self.install_pane.backspace(),
            )],
            AppMessage::InstallSubmit => {
                if let Some(busy) = self.install_busy_status() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Install,
                        busy,
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Install,
                        "Answer the delete question first",
                    ))]
                } else if self.wago_api_key.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Install,
                        "Wago.io installs need your Wago.io key",
                    ))]
                } else if self.install_pane.input.value().trim().is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Install,
                        "Enter a Wago.io addon or link before installing",
                    ))]
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Install,
                        "LemonUp doesn't know your AddOns folder yet",
                    ))]
                } else {
                    let request = PendingWagoInstallRequest {
                        target: self.install_pane.input.value().trim().to_string(),
                        source: WagoInstallSource::DirectInput,
                    };
                    vec![
                        AppAction::SetWagoInstallInProgress(true),
                        AppAction::StartWagoInstall {
                            addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                            api_key: self.wago_api_key.clone().expect("checked above"),
                            request,
                            replace_folders: None,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            DetailMode::Install,
                            "Checking the Wago.io addon before installing",
                        )),
                    ]
                }
            }
            AppMessage::OpenSearch(mode) => vec![
                AppAction::SetSearchPaneState(self.search_pane.open_with_mode(mode)),
                AppAction::SetDetailMode(DetailMode::Search),
            ],
            AppMessage::FindOnline => match self.not_linked_current_item() {
                Some(item) => {
                    let pane = self.search_pane.open_prefilled(&item.name);
                    let mut actions = vec![
                        AppAction::SetDetailMode(DetailMode::Search),
                        AppAction::SetSearchPaneState(pane.clone()),
                    ];
                    actions.extend(self.search_submit_actions(pane, self.wago_api_key.clone()));
                    actions
                }
                None => vec![],
            },
            AppMessage::SearchFocusBox => {
                vec![AppAction::SetSearchPaneState(self.search_pane.focus_box())]
            }
            AppMessage::SearchFocusResults => {
                vec![AppAction::SetSearchPaneState(
                    self.search_pane.focus_results(),
                )]
            }
            AppMessage::SearchCloseKeyBox => {
                vec![AppAction::SetSearchPaneState(
                    self.search_pane.close_key_box(),
                )]
            }
            AppMessage::SearchKeyChar(character) => vec![AppAction::SetSearchPaneState(
                self.search_pane.edit_key(|key| key.insert_char(character)),
            )],
            AppMessage::SearchKeyBackspace => vec![AppAction::SetSearchPaneState(
                self.search_pane.edit_key(TextInput::backspace),
            )],
            AppMessage::SearchSaveKey => self.save_wago_key_actions(),
            AppMessage::SearchInputChar(character) => vec![AppAction::SetSearchPaneState(
                self.search_pane.insert_char(character),
            )],
            AppMessage::SearchBackspace => {
                vec![AppAction::SetSearchPaneState(self.search_pane.backspace())]
            }
            AppMessage::SearchSubmit => {
                self.search_submit_actions(self.search_pane.clone(), self.wago_api_key.clone())
            }
            AppMessage::SearchResultNext => {
                vec![AppAction::SetSearchPaneState(
                    self.search_pane.select_next(),
                )]
            }
            AppMessage::SearchResultPrevious => {
                vec![AppAction::SetSearchPaneState(
                    self.search_pane.select_previous(),
                )]
            }
            AppMessage::SearchInstallSelected => {
                if let Some(busy) = self.install_busy_status() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Search,
                        busy,
                    ))]
                } else if self.wago_api_key.is_none() {
                    vec![AppAction::SetSearchPaneState(
                        self.search_pane.open_key_box(),
                    )]
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Search,
                        "LemonUp doesn't know your AddOns folder yet",
                    ))]
                } else if let Some(result) = self.search_pane.selected_result() {
                    let request = PendingWagoInstallRequest {
                        target: result.id.clone(),
                        source: WagoInstallSource::SearchResult,
                    };
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.start_install_check(result)),
                        AppAction::SetWagoInstallInProgress(true),
                        AppAction::StartWagoInstall {
                            addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                            api_key: self.wago_api_key.clone().expect("checked above"),
                            request,
                            replace_folders: None,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            DetailMode::Search,
                            &format!("Installing {} from Wago.io", result.display_name),
                        )),
                    ]
                } else {
                    vec![]
                }
            }
            AppMessage::SearchConfirmDirectInstall => {
                if let Some(request) = self.search_pane.pending_direct_install().cloned() {
                    let name = request.name();
                    self.start_direct_install_actions(request, Vec::new(), name)
                } else {
                    vec![]
                }
            }
            AppMessage::SearchCancelDirectInstall => vec![AppAction::SetSearchPaneState(
                self.search_pane.cancel_direct_install(),
            )],
            AppMessage::SearchConfirmReplace => {
                if let Some((request, name, folders)) = self.search_pane.pending_direct_replace() {
                    self.start_direct_install_actions(
                        request.clone(),
                        folders.to_vec(),
                        name.to_string(),
                    )
                } else {
                    vec![]
                }
            }
            AppMessage::SearchCancelReplace => vec![
                AppAction::SetSearchPaneState(self.search_pane.clear_install_state()),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    DetailMode::Search,
                    "replace question closed",
                )),
            ],
            AppMessage::WagoConfirmInstall => {
                if let Some(confirmation) = self.pending_wago_install_confirmation.clone() {
                    if let Some(busy) = self.install_busy_status() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Note,
                            self.dashboard.detail_mode,
                            busy,
                        ))]
                    } else if self.wago_api_key.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            self.dashboard.detail_mode,
                            "Wago.io installs need your Wago.io key",
                        ))]
                    } else if let Some(addon_dir) = self.effective_addon_dir.clone() {
                        let mut actions = vec![
                            AppAction::SetPendingWagoInstallConfirmation(None),
                            AppAction::SetSearchPaneState(self.search_pane.clear_install_state()),
                            AppAction::SetWagoInstallInProgress(true),
                            AppAction::StartWagoInstall {
                                addon_dir,
                                api_key: self.wago_api_key.clone().expect("checked above"),
                                request: confirmation.request,
                                replace_folders: Some(
                                    confirmation.inspection.existing_folders.clone(),
                                ),
                            },
                        ];
                        if self.dashboard.detail_mode == DetailMode::Search {
                            actions[1] = AppAction::SetSearchPaneState(
                                self.search_pane.start_installing(&confirmation.inspection),
                            );
                        }
                        let status_message = if self.dashboard.detail_mode == DetailMode::Search {
                            format!(
                                "Reinstalling {} from Wago.io",
                                confirmation.inspection.addon_name
                            )
                        } else {
                            "Replacing your copy from Wago.io".to_string()
                        };
                        actions.push(AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            self.dashboard.detail_mode,
                            &status_message,
                        )));
                        actions
                    } else {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            self.dashboard.detail_mode,
                            "LemonUp doesn't know your AddOns folder yet",
                        ))]
                    }
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "Nothing to replace",
                    ))]
                }
            }
            AppMessage::WagoCancelInstall => vec![
                AppAction::SetPendingWagoInstallConfirmation(None),
                AppAction::SetSearchPaneState(self.search_pane.clear_install_state()),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    "replace question closed",
                )),
            ],
            AppMessage::ConfigSelectionNext => vec![AppAction::SetConfigPaneState(
                self.config_pane.select_next(),
            )],
            AppMessage::ConfigSelectionPrevious => vec![AppAction::SetConfigPaneState(
                self.config_pane.select_previous(),
            )],
            AppMessage::ConfigBeginEditing => {
                let field = self.config_pane.selected_field();
                if field.is_textual() {
                    vec![AppAction::SetConfigPaneState(
                        self.config_pane.begin_editing(),
                    )]
                } else {
                    vec![]
                }
            }
            AppMessage::ConfigInputChar(character) => vec![AppAction::SetConfigPaneState(
                self.config_pane.insert_char(character),
            )],
            AppMessage::ConfigBackspace => {
                vec![AppAction::SetConfigPaneState(self.config_pane.backspace())]
            }
            AppMessage::ConfigCommitEdit => match self.config_pane.commit_edit() {
                Ok(next) => match self.persist_config_pane(next, self.config_pane.selected_field())
                {
                    Ok(actions) => actions,
                    Err(error) => vec![
                        AppAction::SetConfigPaneState(self.config_pane.cancel_edit()),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Config,
                            &error,
                        )),
                    ],
                },
                Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Problem,
                    DetailMode::Config,
                    &format!("Couldn't save your settings: {error}"),
                ))],
            },
            AppMessage::ConfigCancelEdit => {
                vec![AppAction::SetConfigPaneState(
                    self.config_pane.cancel_edit(),
                )]
            }
            AppMessage::ConfigToggleSelected => {
                let field = self.config_pane.selected_field();
                if field.is_textual() {
                    vec![AppAction::SetConfigPaneState(
                        self.config_pane.begin_editing(),
                    )]
                } else {
                    let next = self.config_pane.toggle_selected();
                    match self.persist_config_pane(next, field) {
                        Ok(actions) => actions,
                        Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Config,
                            &error,
                        ))],
                    }
                }
            }
            AppMessage::ConfigAdjustSelected(delta) => {
                let field = self.config_pane.selected_field();
                let next = self.config_pane.adjust_selected(delta);
                match self.persist_config_pane(next, field) {
                    Ok(actions) => actions,
                    Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Config,
                        &error,
                    ))],
                }
            }
            AppMessage::ConfigRunOnboarding => vec![
                AppAction::ReenterOnboarding,
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Config,
                    "Opening setup again",
                )),
            ],
            AppMessage::ConfigExportAddons => match default_transfer_path() {
                Ok(output_path) => {
                    match StateDatabase::open(&self.state_db_file)
                        .and_then(|database| database.list_addons())
                        .and_then(|addons| export_addons(&addons, &output_path))
                    {
                        Ok(exported) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Done,
                            DetailMode::Config,
                            &format!(
                                "exported {} addon{} to {}",
                                exported.addons.len(),
                                plural_suffix(exported.addons.len()),
                                output_path.display()
                            ),
                        ))],
                        Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Config,
                            &format!("Couldn't export your addon list: {error}"),
                        ))],
                    }
                }
                Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Problem,
                    DetailMode::Config,
                    &format!("Couldn't export your addon list: {error}"),
                ))],
            },
            AppMessage::ConfigImportAddons => match default_transfer_path() {
                Ok(input_path) => {
                    if self.effective_addon_dir.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Config,
                            "Set your AddOns folder before importing",
                        ))]
                    } else {
                        vec![
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Working,
                                DetailMode::Config,
                                "importing addon list",
                            )),
                            AppAction::StartTransferImport {
                                addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                                input_path,
                                wago_api_key: self.wago_api_key.clone(),
                            },
                        ]
                    }
                }
                Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Problem,
                    DetailMode::Config,
                    &format!("Couldn't import the addon list: {error}"),
                ))],
            },
            AppMessage::BackupRunNow => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "A backup is already running",
                    ))]
                } else if let Some(last_completed) = self.backup_pane.last_completed_backup_at {
                    let elapsed = OffsetDateTime::now_utc() - last_completed;
                    if elapsed < BACKUP_SPAM_GUARD_WINDOW {
                        let remaining = (BACKUP_SPAM_GUARD_WINDOW - elapsed).whole_seconds().max(1);
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Note,
                            DetailMode::Backup,
                            &format!(
                                "You backed up a moment ago; wait {} more second{}",
                                remaining,
                                plural_suffix(remaining as usize)
                            ),
                        ))]
                    } else if self.effective_addon_dir.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Backup,
                            "LemonUp doesn't know your AddOns folder yet",
                        ))]
                    } else {
                        vec![
                            AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(true)),
                            AppAction::StartBackupNow {
                                addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                                backup_dir: self.backup_dir.clone(),
                                retention: self.config_pane.draft.backup_retention,
                            },
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Working,
                                DetailMode::Backup,
                                "Backing up your game settings…",
                            )),
                        ]
                    }
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Backup,
                        "LemonUp doesn't know your AddOns folder yet",
                    ))]
                } else {
                    vec![
                        AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(true)),
                        AppAction::StartBackupNow {
                            addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                            backup_dir: self.backup_dir.clone(),
                            retention: self.config_pane.draft.backup_retention,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            DetailMode::Backup,
                            "Backing up your game settings…",
                        )),
                    ]
                }
            }
            AppMessage::BackupSelectionNext => vec![AppAction::SetBackupPaneState(
                self.backup_pane.select_next(),
            )],
            AppMessage::BackupSelectionPrevious => vec![AppAction::SetBackupPaneState(
                self.backup_pane.select_previous(),
            )],
            AppMessage::BackupRequestRestore => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "A backup task is already running",
                    ))]
                } else if self.backup_pane.selected_backup().is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "Choose a backup first",
                    ))]
                } else {
                    vec![
                        AppAction::SetBackupPaneState(self.backup_pane.request_restore()),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Quiet,
                            DetailMode::Backup,
                            "asked to restore a backup",
                        )),
                    ]
                }
            }
            AppMessage::BackupRequestDelete => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "A backup task is already running",
                    ))]
                } else if self.backup_pane.selected_backup().is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "Choose a backup first",
                    ))]
                } else {
                    vec![
                        AppAction::SetBackupPaneState(self.backup_pane.request_delete()),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Quiet,
                            DetailMode::Backup,
                            "asked to delete a backup",
                        )),
                    ]
                }
            }
            AppMessage::BackupConfirmRestore => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "A backup task is already running",
                    ))]
                } else if let Some(backup) = self.backup_pane.pending_restore.clone() {
                    if self.effective_addon_dir.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Problem,
                            DetailMode::Backup,
                            "LemonUp doesn't know your AddOns folder yet",
                        ))]
                    } else {
                        vec![
                            AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(true)),
                            AppAction::StartBackupRestore {
                                addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                                backup_path: backup.path,
                            },
                            AppAction::SetStatus(self.dashboard_status_for(
                                StatusKind::Working,
                                DetailMode::Backup,
                                "Putting back your game settings…",
                            )),
                        ]
                    }
                } else {
                    vec![]
                }
            }
            AppMessage::BackupConfirmDelete => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Backup,
                        "A backup task is already running",
                    ))]
                } else if let Some(backup) = self.backup_pane.pending_delete.clone() {
                    vec![
                        AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(true)),
                        AppAction::StartBackupDelete {
                            backup_root: self.backup_dir.clone(),
                            backup_path: backup.path,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Working,
                            DetailMode::Backup,
                            "Deleting the backup…",
                        )),
                    ]
                } else {
                    vec![]
                }
            }
            AppMessage::BackupCancelRestore => vec![
                AppAction::SetBackupPaneState(self.backup_pane.cancel_restore().cancel_delete()),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Quiet,
                    DetailMode::Backup,
                    "backup question closed",
                )),
            ],
            AppMessage::OnboardingNextStep => match self.onboarding.step {
                OnboardingStep::Theme => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(self.with_base_status(
                        StatusKind::Note,
                        "step 2/5 | choose or check an AddOns folder",
                    )),
                ],
                OnboardingStep::Directory => vec![AppAction::SetStatus(self.with_base_status(
                    StatusKind::Note,
                    "Check or choose a folder before continuing",
                ))],
                OnboardingStep::Wago => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(
                        self.with_base_status(StatusKind::Note, "step 4/5 | adjust core settings"),
                    ),
                ],
                OnboardingStep::Settings => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(
                        self.with_base_status(StatusKind::Note, "step 5/5 | review setup"),
                    ),
                ],
                OnboardingStep::Review => {
                    vec![AppAction::SaveOnboardingConfig(
                        self.onboarding.apply_directory_to_draft().draft,
                    )]
                }
            },
            AppMessage::OnboardingPreviousStep => {
                if self.onboarding.step == OnboardingStep::Theme {
                    vec![AppAction::Quit]
                } else {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.previous_step()),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Note,
                            &format!("step {}/5", self.onboarding.step.previous().index() + 1),
                        )),
                    ]
                }
            }
            AppMessage::OnboardingToggleTheme => vec![
                AppAction::SetOnboardingState(self.onboarding.toggle_theme()),
                AppAction::SetStatus(
                    self.with_base_status(StatusKind::Quiet, "theme updated | enter next"),
                ),
            ],
            AppMessage::OnboardingSettingsNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_settings_field()),
                AppAction::SetStatus(
                    self.with_base_status(StatusKind::Quiet, "settings selection moved"),
                ),
            ],
            AppMessage::OnboardingSettingsPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_settings_field()),
                AppAction::SetStatus(
                    self.with_base_status(StatusKind::Quiet, "settings selection moved"),
                ),
            ],
            AppMessage::OnboardingSettingIncrease => vec![
                AppAction::SetOnboardingState(self.onboarding.adjust_settings_field(1)),
                AppAction::SetStatus(self.with_base_status(StatusKind::Quiet, "setting updated")),
            ],
            AppMessage::OnboardingSettingDecrease => vec![
                AppAction::SetOnboardingState(self.onboarding.adjust_settings_field(-1)),
                AppAction::SetStatus(self.with_base_status(StatusKind::Quiet, "setting updated")),
            ],
            AppMessage::OnboardingBeginEditing => {
                let next_state = match self.onboarding.step {
                    OnboardingStep::Directory => self.onboarding.begin_directory_editing(),
                    OnboardingStep::Wago => self.onboarding.begin_wago_editing(),
                    _ => self.onboarding.clone(),
                };
                let status = match self.onboarding.step {
                    OnboardingStep::Directory => "editing folder | enter check | esc stop editing",
                    OnboardingStep::Wago => "editing Wago API key | enter apply | esc cancel",
                    _ => "editing",
                };
                vec![
                    AppAction::SetOnboardingState(next_state),
                    AppAction::SetStatus(self.with_base_status(StatusKind::Note, status)),
                ]
            }
            AppMessage::OnboardingStopEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.stop_editing()),
                AppAction::SetStatus(match self.onboarding.step {
                    OnboardingStep::Directory => self.location_finder_status(),
                    OnboardingStep::Wago => self.with_base_status(
                        StatusKind::Note,
                        "Wago.io step | e edit key | enter next",
                    ),
                    _ => self.with_base_status(StatusKind::Note, "editing stopped"),
                }),
            ],
            AppMessage::OnboardingInputChar(character) => vec![AppAction::SetOnboardingState(
                self.onboarding.insert_char(character),
            )],
            AppMessage::OnboardingBackspace => {
                vec![AppAction::SetOnboardingState(self.onboarding.backspace())]
            }
            AppMessage::OnboardingValidateInput => {
                let current_path = PathBuf::from(self.onboarding.input.trim());
                if self.is_guarded_path(&current_path) {
                    return vec![
                        AppAction::SetOnboardingState(
                            self.onboarding
                                .error("this profile can't use the main AddOns folder"),
                        ),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Problem,
                            "main AddOns folder blocked | choose another folder",
                        )),
                    ];
                }

                match validate_addons_path(&current_path) {
                    Ok(()) => {
                        let next_state = self.onboarding_found_state(vec![current_path.clone()]);
                        vec![
                            AppAction::SetOnboardingState(next_state),
                            AppAction::SetStatus(self.found_status_for_path(&current_path)),
                        ]
                    }
                    Err(error) => vec![
                        AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Problem,
                            &format!("That folder doesn't work: {error}"),
                        )),
                    ],
                }
            }
            AppMessage::OnboardingDeepScan => self.onboarding_scan_actions(),
            AppMessage::OnboardingCancel => {
                if matches!(self.onboarding.phase, OnboardingPhase::DeepScanning(_)) {
                    vec![
                        AppAction::CancelOnboardingScan,
                        AppAction::SetOnboardingState(self.onboarding.cancelled()),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Note,
                            "scan stopped | j/k pick a folder | enter scan | e edit path",
                        )),
                    ]
                } else {
                    vec![AppAction::SetOnboardingState(
                        self.onboarding.stop_editing(),
                    )]
                }
            }
            AppMessage::OnboardingSuggestionNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_suggestion()),
                AppAction::SetStatus(self.location_finder_status()),
            ],
            AppMessage::OnboardingSuggestionPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_suggestion()),
                AppAction::SetStatus(self.location_finder_status()),
            ],
            AppMessage::OnboardingFoundActionNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_found_action()),
                AppAction::SetStatus(self.found_status_for_input()),
            ],
            AppMessage::OnboardingFoundActionPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_found_action()),
                AppAction::SetStatus(self.found_status_for_input()),
            ],
            AppMessage::OnboardingFoundConfirm => match self.onboarding.selected_found_action() {
                FoundAction::UseThisPath => vec![
                    AppAction::SetOnboardingState(
                        self.onboarding.apply_directory_to_draft().next_step(),
                    ),
                    AppAction::SetStatus(
                        self.with_base_status(StatusKind::Done, "folder chosen | step 3/5 Wago.io"),
                    ),
                ],
                FoundAction::EnterDifferentPath => vec![
                    AppAction::SetOnboardingState(self.onboarding.begin_directory_editing()),
                    AppAction::SetStatus(self.with_base_status(
                        StatusKind::Note,
                        "editing folder | enter check | esc stop editing",
                    )),
                ],
            },
            AppMessage::BackgroundTask(event) => match event {
                AppTaskEvent::Onboarding(OnboardingTaskEvent::QuickCheckFinished(paths))
                    if !paths.is_empty() =>
                {
                    let state = self.onboarding_found_state(paths);
                    let status = self.found_status_for_path(Path::new(state.input.trim()));
                    vec![
                        AppAction::SetOnboardingState(state),
                        AppAction::SetStatus(status),
                    ]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::QuickCheckFinished(_)) => {
                    self.onboarding_scan_actions()
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanProgress(progress)) => {
                    vec![AppAction::SetOnboardingState(
                        self.onboarding.set_scan_progress(progress),
                    )]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Ok(paths)))
                    if !paths.is_empty() =>
                {
                    let state = self.onboarding_found_state(paths);
                    let status = self.found_status_for_path(Path::new(state.input.trim()));
                    vec![
                        AppAction::SetOnboardingState(state),
                        AppAction::SetStatus(status),
                    ]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Ok(_))) => vec![
                    AppAction::SetOnboardingState(self.onboarding.error(format!(
                        "Couldn't find World of Warcraft under {}",
                        self.onboarding.input.trim()
                    ))),
                    AppAction::SetStatus(self.with_base_status(
                        StatusKind::Note,
                        "no install found | j/k pick a folder | enter scan | e edit path",
                    )),
                ],
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Err(error))) => {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                        AppAction::SetStatus(self.with_base_status(
                            StatusKind::Problem,
                            &format!("The search failed: {error}"),
                        )),
                    ]
                }
                AppTaskEvent::AddonScanFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.addons),
                    AppAction::SetDashboardDriftReport(Some(outcome.drift_report)),
                    AppAction::CompleteAddonScan {
                        path: outcome.path,
                        summary: outcome.summary,
                    },
                ],
                AppTaskEvent::AddonScanFinished(Err(error)) => {
                    vec![AppAction::FailAddonScan(error)]
                }
                AppTaskEvent::DashboardDeleteFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                    AppAction::SetPendingDelete(None),
                    AppAction::SetDashboardUndoDelete(outcome.undo_delete.clone()),
                    AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                    AppAction::CompleteAddonScan {
                        path: outcome.sync.path,
                        summary: outcome.sync.summary,
                    },
                    AppAction::SetStatus({
                        let status = self.dashboard_status_for(
                            StatusKind::Done,
                            self.dashboard.detail_mode,
                            &format!("Deleted {}", addon_count_label(outcome.deleted_parents)),
                        );
                        if outcome.undo_delete.is_some() {
                            status.offering(StatusOffer::UndoDelete)
                        } else {
                            status
                        }
                    }),
                    AppAction::SetDashboardDeleteInProgress(false),
                ],
                AppTaskEvent::DashboardDeleteFinished(Err(error)) => vec![
                    AppAction::SetDashboardDeleteInProgress(false),
                    AppAction::SetPendingDelete(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        self.dashboard.detail_mode,
                        &format!("Couldn't delete: {error}"),
                    )),
                ],
                AppTaskEvent::DashboardUndoFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                    AppAction::SetDashboardUndoInProgress(false),
                    AppAction::SetDashboardUndoDelete(None),
                    AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                    AppAction::CompleteAddonScan {
                        path: outcome.sync.path,
                        summary: outcome.sync.summary,
                    },
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Done,
                        self.dashboard.detail_mode,
                        &format!(
                            "Undid delete of {}",
                            addon_count_label(outcome.restored_parents)
                        ),
                    )),
                ],
                AppTaskEvent::DashboardUndoFinished(Err(error)) => vec![
                    AppAction::SetDashboardUndoInProgress(false),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        self.dashboard.detail_mode,
                        &format!("Couldn't undo delete: {error}"),
                    )),
                ],
                AppTaskEvent::DashboardJobProgress(progress) => vec![
                    AppAction::SetDashboardJobUi(Some(progress.clone())),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Working,
                        self.dashboard.detail_mode,
                        &format!(
                            "{} {} of {} · {}",
                            progress.kind.label().to_ascii_lowercase(),
                            progress.current_index,
                            progress.total,
                            progress.current_addon
                        ),
                    )),
                ],
                AppTaskEvent::DashboardCheckFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.addons),
                    AppAction::SetDashboardAutoCheckInProgress(false),
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(
                        self.dashboard_status_for(
                            if outcome.errors > 0 {
                                StatusKind::Problem
                            } else {
                                StatusKind::Done
                            },
                            self.dashboard.detail_mode,
                            &dashboard_check_status_message(
                                &self.dashboard_names(&outcome.targets),
                                outcome.total,
                                outcome.live_checked,
                                outcome.cached,
                                outcome.errors,
                            ),
                        )
                        // Shown only while some addon has an update ready.
                        .offering(StatusOffer::UpdateAll),
                    ),
                ],
                AppTaskEvent::DashboardCheckFinished(Err(error)) => vec![
                    AppAction::SetDashboardAutoCheckInProgress(false),
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        self.dashboard.detail_mode,
                        &format!("Couldn't check for updates: {error}"),
                    )),
                ],
                AppTaskEvent::DashboardUpdateFinished(Ok(outcome)) => {
                    let failed_names = self.dashboard_names(&outcome.failed_folders);
                    let status_message = dashboard_update_status_message(&outcome, &failed_names);
                    vec![
                        AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                        AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                        AppAction::CompleteAddonScan {
                            path: outcome.sync.path,
                            summary: outcome.sync.summary,
                        },
                        AppAction::SetDashboardUpdateInProgress(false),
                        AppAction::SetDashboardJobUi(None),
                        AppAction::SetDashboardUpdateSummary(Some(
                            dashboard_update_run_summary_from_live(outcome.summary),
                        )),
                        AppAction::SetDashboardUpdateResults {
                            updated: outcome.updated_folders.clone(),
                            failed: outcome.failed_folders.clone(),
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            if outcome.summary.errors > 0 {
                                StatusKind::Problem
                            } else {
                                StatusKind::Done
                            },
                            self.dashboard.detail_mode,
                            &status_message,
                        )),
                    ]
                }
                AppTaskEvent::DashboardUpdateFinished(Err(error)) => vec![
                    AppAction::SetDashboardUpdateInProgress(false),
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        self.dashboard.detail_mode,
                        &format!("Couldn't update: {error}"),
                    )),
                ],
                AppTaskEvent::BackupFinished(Ok(outcome)) => {
                    let mut message = "Backed up your game settings".to_string();
                    if !outcome.pruned_files.is_empty() {
                        message.push_str(&format!(
                            " · keeping the newest {}",
                            self.config_pane.draft.backup_retention
                        ));
                    }
                    vec![
                        AppAction::SetBackupPaneState(
                            self.backup_pane
                                .with_backups(outcome.backups)
                                .with_last_completed_backup_at(Some(OffsetDateTime::now_utc()))
                                .set_in_progress(false),
                        ),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Done,
                            DetailMode::Backup,
                            &message,
                        )),
                    ]
                }
                AppTaskEvent::BackupFinished(Err(error)) => vec![
                    AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(false)),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Backup,
                        &format!("Couldn't back up your game settings: {error}"),
                    )),
                ],
                AppTaskEvent::BackupRestoreFinished(Ok(outcome)) => vec![
                    AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(false)),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Done,
                        DetailMode::Backup,
                        &format!("Put back your game settings from {}", outcome.backup.label),
                    )),
                ],
                AppTaskEvent::BackupRestoreFinished(Err(error)) => vec![
                    AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(false)),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Backup,
                        &format!("Couldn't put back your game settings: {error}"),
                    )),
                ],
                AppTaskEvent::BackupDeleteFinished(Ok(outcome)) => vec![
                    AppAction::SetBackupPaneState(
                        self.backup_pane
                            .with_backups(outcome.backups)
                            .set_in_progress(false),
                    ),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Done,
                        DetailMode::Backup,
                        &format!("Deleted the backup from {}", outcome.deleted.label),
                    )),
                ],
                AppTaskEvent::BackupDeleteFinished(Err(error)) => vec![
                    AppAction::SetBackupPaneState(self.backup_pane.set_in_progress(false)),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Backup,
                        &format!("Couldn't delete the backup: {error}"),
                    )),
                ],
                AppTaskEvent::TransferImportFinished(Ok(outcome)) => {
                    let status =
                        transfer_import_status_message(&outcome.analysis, &outcome.install_summary);
                    vec![
                        AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                        AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                        AppAction::CompleteAddonScan {
                            path: outcome.sync.path,
                            summary: outcome.sync.summary,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            if outcome.install_summary.errors.is_empty() {
                                StatusKind::Done
                            } else {
                                StatusKind::Problem
                            },
                            DetailMode::Config,
                            &status,
                        )),
                    ]
                }
                AppTaskEvent::TransferImportFinished(Err(error)) => {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Problem,
                        DetailMode::Config,
                        &format!("Couldn't import the addon list: {error}"),
                    ))]
                }
                AppTaskEvent::WagoSearchFinished(Ok(outcome)) => vec![
                    AppAction::SetSearchPaneState(
                        self.search_pane
                            .finish_search(outcome.query.clone(), outcome.results.clone()),
                    ),
                    AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Done,
                        DetailMode::Search,
                        &format!(
                            "Found {} addon{} on Wago.io",
                            outcome.results.len(),
                            plural_suffix(outcome.results.len())
                        ),
                    )),
                ],
                AppTaskEvent::WagoSearchFinished(Err(error)) => {
                    let mut pane = self.search_pane.fail_search(&error);
                    if error.kind == SourceErrorKind::KeyRejected {
                        pane = pane.open_key_box_after_rejection(&error);
                    }
                    vec![AppAction::SetSearchPaneState(pane)]
                }
                AppTaskEvent::WagoInstallFinished(Ok(
                    WagoInstallTaskOutcome::NeedsConfirmation {
                        request,
                        inspection,
                    },
                )) => {
                    let confirmation_status = if inspection.tracked_parent.is_some() {
                        "asked to reinstall"
                    } else {
                        "asked to replace"
                    };
                    vec![
                        AppAction::SetSearchPaneState(
                            self.search_pane.require_install_confirmation(&inspection),
                        ),
                        AppAction::SetWagoInstallInProgress(false),
                        AppAction::SetPendingWagoInstallConfirmation(Some(
                            WagoInstallConfirmation {
                                request,
                                inspection,
                            },
                        )),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Quiet,
                            self.dashboard.detail_mode,
                            confirmation_status,
                        )),
                    ]
                }
                AppTaskEvent::WagoInstallFinished(Ok(WagoInstallTaskOutcome::Installed(
                    outcome,
                ))) => {
                    vec![
                        AppAction::SetSearchPaneState(
                            self.search_pane.finish_install(&outcome.summary),
                        ),
                        AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                        AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                        AppAction::CompleteAddonScan {
                            path: outcome.sync.path,
                            summary: outcome.sync.summary,
                        },
                        AppAction::SetPendingWagoInstallConfirmation(None),
                        AppAction::SetWagoInstallInProgress(false),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Done,
                            self.dashboard.detail_mode,
                            &format!("{} is installed", outcome.summary.addon_name),
                        )),
                    ]
                }
                AppTaskEvent::WagoInstallFinished(Err(error)) => {
                    let (addon_id, addon_name) = self.search_pane.install_identity();
                    let mut pane = self
                        .search_pane
                        .fail_install(addon_id, addon_name, "Wago.io", &error);
                    if error.kind == SourceErrorKind::KeyRejected {
                        pane = pane.open_key_box_after_rejection(&error);
                    }
                    vec![
                        AppAction::SetSearchPaneState(pane),
                        AppAction::SetWagoInstallInProgress(false),
                    ]
                }
                AppTaskEvent::DirectInstallFinished {
                    request,
                    result: Ok(outcome),
                    ..
                } => {
                    let mut status = format!(
                        "{} is installed from {}",
                        outcome.addon_name,
                        request.provider.label()
                    );
                    if let Some(warning) = &outcome.warning {
                        status.push_str(&format!(" | {warning}"));
                    }
                    vec![
                        AppAction::SetSearchPaneState(
                            self.search_pane.finish_direct_install(&request, &outcome),
                        ),
                        AppAction::ReplaceDashboardAddons(outcome.sync.addons),
                        AppAction::SetDashboardDriftReport(Some(outcome.sync.drift_report)),
                        AppAction::CompleteAddonScan {
                            path: outcome.sync.path,
                            summary: outcome.sync.summary,
                        },
                        AppAction::SetDirectInstallInProgress(false),
                        AppAction::SetStatus(self.dashboard_status_for(
                            StatusKind::Done,
                            self.dashboard.detail_mode,
                            &status,
                        )),
                    ]
                }
                // Its folders are already in AddOns (a "Not linked" copy):
                // ask before replacing them, naming them. A download that
                // brought a folder the last question did not name asks again.
                AppTaskEvent::DirectInstallFinished {
                    request,
                    result: Err(error),
                } if error.kind == SourceErrorKind::FoldersInTheWay => {
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.require_direct_replace(
                            request,
                            &error.clash.clone().unwrap_or_default(),
                        )),
                        AppAction::SetDirectInstallInProgress(false),
                    ]
                }
                AppTaskEvent::DirectInstallFinished {
                    request,
                    result: Err(error),
                } => vec![
                    AppAction::SetSearchPaneState(self.search_pane.fail_install(
                        Some(request.target.clone()),
                        Some(request.name()),
                        request.provider.label(),
                        &error,
                    )),
                    AppAction::SetDirectInstallInProgress(false),
                ],
            },
        }
    }

    fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::Quit => self.quit_requested = true,
            AppAction::SetQuitQuestion(open) => self.quit_question_open = open,
            AppAction::AdvanceMotionTick => {
                self.shell_ui.motion = self.shell_ui.motion.advance();
                if self
                    .shell_ui
                    .footer_pulse
                    .is_some_and(|pulse| self.shell_ui.motion.tick_count >= pulse.expires_at_tick)
                {
                    self.shell_ui.footer_pulse = None;
                }
                if self
                    .shell_ui
                    .status_flash
                    .as_ref()
                    .is_some_and(|flash| !self.status_flash_is_live(flash, Instant::now()))
                {
                    self.shell_ui.status_flash = None;
                }
            }
            AppAction::SetFooterKeyPulse(pulse) => self.shell_ui.footer_pulse = pulse,
            AppAction::SetInspectOverlay(active) => {
                self.shell_ui.overlay.active = active.then_some(OverlayKind::Inspect);
            }
            AppAction::SetSheetOffset(offset) => {
                let mut scroll = self.sheet_scroll.get();
                scroll.offset = offset;
                self.sheet_scroll.set(scroll);
            }
            AppAction::SetHelpOverlay(active) => {
                if active {
                    self.shell_ui.overlay.active = Some(OverlayKind::Help);
                } else if self.shell_ui.overlay.active == Some(OverlayKind::Help) {
                    self.shell_ui.overlay.active = match self.dashboard.detail_mode {
                        // Help opened from Details goes back to it.
                        DetailMode::Overview
                            if matches!(
                                self.shell_ui.help_context,
                                HelpContext::Inspect | HelpContext::DetailsDocked
                            ) =>
                        {
                            Some(OverlayKind::Inspect)
                        }
                        DetailMode::Overview => None,
                        DetailMode::Install => Some(OverlayKind::Install),
                        DetailMode::Search => Some(OverlayKind::Search),
                        DetailMode::Update => Some(OverlayKind::Update),
                        DetailMode::Config => Some(OverlayKind::Config),
                        DetailMode::Backup => Some(OverlayKind::Backup),
                    };
                }
            }
            AppAction::SetHelpContext(context) => {
                self.shell_ui.help_context = context;
            }
            AppAction::SetStatus(status) => {
                if let Some(flash) = self.status_flash_from_status(&status) {
                    self.shell_ui.status_flash = Some(flash);
                }
                self.status_line = status;
            }
            AppAction::SetDashboardSelection(selection) => {
                self.dashboard.list_state.select(selection)
            }
            AppAction::ToggleDashboardSort(column) => self.dashboard.toggle_sort(column),
            AppAction::SetDashboardFilter { query, editing } => {
                self.dashboard.set_filter(query, editing);
            }
            AppAction::SetPendingDelete(folders) => {
                self.dashboard.set_pending_delete_folders(folders);
            }
            AppAction::SetDashboardUndoDelete(undo) => {
                let previous = self.undo_delete.take();
                self.undo_delete = undo;
                if let Some(previous) = previous {
                    let keep_current = self
                        .undo_delete
                        .as_ref()
                        .is_some_and(|current| current.batch_dir == previous.batch_dir);
                    if !keep_current {
                        let _ = fs::remove_dir_all(previous.batch_dir);
                    }
                }
            }
            AppAction::SetDashboardDriftReport(drift_report) => {
                self.dashboard.set_drift_report(drift_report);
            }
            AppAction::SetDashboardUndoInProgress(in_progress) => {
                self.undo_delete_in_progress = in_progress;
            }
            AppAction::SetDashboardDeleteInProgress(in_progress) => {
                self.delete_in_progress = in_progress;
            }
            AppAction::SetDashboardUpdateInProgress(in_progress) => {
                self.dashboard.set_update_in_progress(in_progress);
                if !in_progress {
                    // Nothing is updating any more, so the question is moot.
                    self.quit_question_open = false;
                }
            }
            AppAction::SetDashboardAutoCheckInProgress(in_progress) => {
                self.auto_check_in_progress = in_progress;
                if in_progress {
                    self.last_auto_check_started_at = Some(Instant::now());
                }
            }
            AppAction::SetDashboardJobUi(job_ui) => {
                self.dashboard.set_job_ui(job_ui);
            }
            AppAction::SetDashboardUpdateSummary(summary) => {
                self.dashboard.set_last_update_summary(summary);
            }
            AppAction::SetDashboardUpdateResults { updated, failed } => {
                self.dashboard.set_update_results(updated, failed);
            }
            AppAction::SetSelectedDashboardParents(folders) => {
                self.dashboard.set_selected_parents(folders);
            }
            AppAction::SetInstallPaneState(state) => {
                self.install_pane = state;
            }
            AppAction::SetSearchPaneState(state) => {
                self.search_pane = state;
            }
            AppAction::SetConfigPaneState(state) => {
                self.config_pane = state;
            }
            AppAction::SetPersistedConfig(config) => {
                self.wago_api_key = config.wago_api_key.clone();
                self.config_present = true;
                self.ui_theme = UiTheme::from_mode(config.theme);
            }
            AppAction::SetBackupPaneState(state) => {
                self.backup_pane = state;
            }
            AppAction::SetPendingWagoInstallConfirmation(confirmation) => {
                self.pending_wago_install_confirmation = confirmation;
            }
            AppAction::SetWagoInstallInProgress(in_progress) => {
                self.wago_install_in_progress = in_progress;
            }
            AppAction::SetDirectInstallInProgress(in_progress) => {
                self.direct_install_in_progress = in_progress;
            }
            AppAction::ToggleDashboardSelection => {
                if self.dashboard.toggle_selected_parent() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        &format!(
                            "selected {} addon{}",
                            self.dashboard.selected_parent_count(),
                            plural_suffix(self.dashboard.selected_parent_count())
                        ),
                    );
                }
            }
            AppAction::SelectAllDashboardParents => {
                if self.dashboard.select_all_parents() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        &format!(
                            "selected all {} addon{}",
                            self.dashboard.selected_parent_count(),
                            plural_suffix(self.dashboard.selected_parent_count())
                        ),
                    );
                }
            }
            AppAction::ClearDashboardSelection => {
                if self.dashboard.clear_selected_parents() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        "selection cleared",
                    );
                }
            }
            AppAction::ToggleDashboardExpanded => {
                if self.dashboard.toggle_selected_expanded() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "tree state updated",
                    );
                }
            }
            AppAction::ExpandAllDashboardRelationships => {
                if self.dashboard.expand_all_relationships() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "expanded all relationship rows",
                    );
                }
            }
            AppAction::CollapseAllDashboardRelationships => {
                if self.dashboard.collapse_all_relationships() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "collapsed all relationship rows",
                    );
                }
            }
            AppAction::CollapseDashboardExpanded => {
                if self.dashboard.collapse_selected_expanded() {
                    self.status_line = self.dashboard_status_for(
                        StatusKind::Quiet,
                        self.dashboard.detail_mode,
                        "tree state updated",
                    );
                }
            }
            AppAction::SetDetailMode(detail_mode) => {
                self.dashboard.detail_mode = detail_mode;
                self.install_pane = self.install_pane.stop_editing();
                self.config_pane.edit = None;
                self.shell_ui.help_context = self.current_help_context_for_mode(detail_mode);
                self.shell_ui.overlay.active = match detail_mode {
                    DetailMode::Overview => None,
                    DetailMode::Install => Some(OverlayKind::Install),
                    DetailMode::Search => Some(OverlayKind::Search),
                    DetailMode::Update => Some(OverlayKind::Update),
                    DetailMode::Config => Some(OverlayKind::Config),
                    DetailMode::Backup => Some(OverlayKind::Backup),
                };
            }
            AppAction::ReenterOnboarding => {
                self.shell_mode = ShellMode::Onboarding;
                self.dashboard.detail_mode = DetailMode::Overview;
                self.shell_ui.overlay.active = None;
                self.install_pane = self.install_pane.stop_editing();
                self.config_pane.edit = None;
                let mut config = self.config_pane.draft.clone();
                if let Some(path) = &self.effective_addon_dir {
                    config.addon_dir = Some(path.clone());
                    self.onboarding =
                        OnboardingState::from_config_with_input(config, path.display().to_string())
                            .ready();
                } else {
                    self.onboarding = OnboardingState::from_config(config).ready();
                }
            }
            AppAction::SetOnboardingState(state) => {
                self.ui_theme = UiTheme::from_mode(state.draft.theme);
                self.onboarding = state;
            }
            AppAction::StartOnboardingQuickCheck => {
                let sender = self.task_events_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(detect_known_addons_paths)
                        .await
                        .unwrap_or_default();
                    let _ = sender.send(AppTaskEvent::Onboarding(
                        OnboardingTaskEvent::QuickCheckFinished(result),
                    ));
                });
            }
            AppAction::StartOnboardingDeepScan(root) => {
                let sender = self.task_events_tx.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                self.active_onboarding_cancel = Some(cancel.clone());
                tokio::spawn(async move {
                    let progress_sender = sender.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        search_for_wow_installs(
                            &root,
                            || cancel.load(Ordering::SeqCst),
                            |progress| {
                                let _ = progress_sender.send(AppTaskEvent::Onboarding(
                                    OnboardingTaskEvent::DeepScanProgress(
                                        crate::onboarding::ScanProgressState {
                                            dirs_scanned: progress.dirs_scanned,
                                            current_path: progress
                                                .current_path
                                                .display()
                                                .to_string(),
                                        },
                                    ),
                                ));
                            },
                        )
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::Onboarding(
                        OnboardingTaskEvent::DeepScanFinished(result),
                    ));
                });
            }
            AppAction::CancelOnboardingScan => {
                if let Some(cancel) = &self.active_onboarding_cancel {
                    cancel.store(true, Ordering::SeqCst);
                }
                self.active_onboarding_cancel = None;
            }
            AppAction::SaveOnboardingConfig(config) => {
                let Some(path) = config.addon_dir.clone() else {
                    self.onboarding = self
                        .onboarding
                        .error("wizard cannot finish without a valid AddOns directory");
                    self.status_line = self.with_base_status(
                        StatusKind::Problem,
                        "finish blocked | choose an AddOns folder that exists",
                    );
                    self.shell_mode = ShellMode::Onboarding;
                    return;
                };
                if self.is_guarded_path(&path) {
                    self.onboarding = self
                        .onboarding
                        .error("this profile can't use the main AddOns folder");
                    self.status_line = self.with_base_status(
                        StatusKind::Problem,
                        "refused to save the main AddOns folder into a non-default profile",
                    );
                    self.shell_mode = ShellMode::Onboarding;
                    return;
                }

                // Confirming a folder in setup is the deliberate way to (re)bind the
                // profile's tracked state to it.
                let rebind = match StateDatabase::open(&self.state_db_file).and_then(|database| {
                    rebind_addon_dir(
                        &database,
                        &path,
                        &self.data_dir.join("rebind"),
                        OffsetDateTime::now_utc(),
                    )
                }) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        self.onboarding = self.onboarding.error(error.to_string());
                        self.status_line = self.with_base_status(
                            StatusKind::Problem,
                            &format!("failed to bind profile to {}: {error}", path.display()),
                        );
                        self.shell_mode = ShellMode::Onboarding;
                        return;
                    }
                };

                match self.config_store.write_new_config(&config) {
                    Ok(()) => {
                        self.config_present = true;
                        self.config_pane = self.config_pane.mark_saved(config.clone());
                        self.wago_api_key = config.wago_api_key.clone();
                        self.effective_addon_dir = Some(path.clone());
                        let backup_store = backup_root(&self.data_dir);
                        if let Err(error) = adopt_legacy_backups(&backup_store, &path) {
                            tracing::warn!(%error, "could not move legacy WTF backups into the client folder");
                        }
                        self.backup_dir = client_backup_dir(&backup_store, &path);
                        self.backup_pane.backups =
                            list_backups(&self.backup_dir).unwrap_or_default();
                        self.backup_pane.selected_backup = 0;
                        if let (Some(previous), Some(snapshot)) =
                            (&rebind.previous, &rebind.snapshot)
                        {
                            self.startup_notice = None;
                            self.shell_ui.status_flash = Some(StatusFlash {
                                text: truncate_text(
                                    &format!(
                                        "Moved profile from {} · tracked addons saved to {}",
                                        previous.display(),
                                        snapshot.display()
                                    ),
                                    160,
                                ),
                                kind: StatusKind::Done,
                                offer: None,
                                expires_at: Some(Instant::now() + StdDuration::from_secs(12)),
                            });
                        }
                        self.shell_mode = ShellMode::Dashboard;
                        if tokio::runtime::Handle::try_current().is_ok() {
                            self.apply(AppAction::StartAddonScan(path));
                        } else {
                            self.scan_state = ScanState::Pending;
                            self.status_line = self.dashboard_status_for(
                                StatusKind::Done,
                                self.dashboard.detail_mode,
                                &format!(
                                    "saved onboarding config {} | scan queued",
                                    path.display()
                                ),
                            );
                        }
                    }
                    Err(error) => {
                        self.onboarding = self.onboarding.error(error.to_string());
                        self.status_line = self.with_base_status(
                            StatusKind::Problem,
                            &format!("failed to save config: {error}"),
                        );
                    }
                }
            }
            AppAction::StartAddonScan(path) => {
                self.scan_state = ScanState::Running(path.clone());
                self.status_line = self.dashboard_status_for(
                    StatusKind::Working,
                    self.dashboard.detail_mode,
                    "Looking through your AddOns folder…",
                );

                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        sync_dashboard_state(&state_db_file, &path)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::AddonScanFinished(result));
                });
            }
            AppAction::StartDashboardDelete {
                addon_dir,
                trash_dir,
                folders,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        delete_selected_addons(&state_db_file, &addon_dir, &trash_dir, &folders)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::DashboardDeleteFinished(result));
                });
            }
            AppAction::StartDashboardUndoDelete { undo } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        undo_deleted_addons(&state_db_file, &undo)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::DashboardUndoFinished(result));
                });
            }
            AppAction::StartDashboardUpdateSelected {
                addon_dir,
                folders,
                wago_api_key,
                check_interval_secs,
            } => {
                self.dashboard.begin_update_targets(&folders);
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                let backup_settings = PreUpdateBackupSettings {
                    enabled: self.config_pane.draft.backup_wtf,
                    retention: self.config_pane.draft.backup_retention,
                    backup_dir: self.backup_dir.clone(),
                };
                tokio::spawn(async move {
                    let result = run_dashboard_update_task(
                        &state_db_file,
                        &addon_dir,
                        &folders,
                        wago_api_key,
                        check_interval_secs,
                        backup_settings,
                        {
                            let sender = sender.clone();
                            move |kind, current_index, total, addon_name| {
                                let _ = sender.send(AppTaskEvent::DashboardJobProgress(
                                    DashboardJobUiState {
                                        kind,
                                        current_index,
                                        total,
                                        current_addon: addon_name,
                                    },
                                ));
                            }
                        },
                    )
                    .await;

                    let _ = sender.send(AppTaskEvent::DashboardUpdateFinished(result));
                });
            }
            AppAction::StartDashboardCheckSelected {
                folders,
                wago_api_key,
                check_interval_secs,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                let addon_dir = self.effective_addon_dir.clone();
                tokio::spawn(async move {
                    let result = run_dashboard_check_task(
                        &state_db_file,
                        addon_dir.as_deref(),
                        &folders,
                        wago_api_key,
                        check_interval_secs,
                        {
                            let sender = sender.clone();
                            move |kind, current_index, total, addon_name| {
                                let _ = sender.send(AppTaskEvent::DashboardJobProgress(
                                    DashboardJobUiState {
                                        kind,
                                        current_index,
                                        total,
                                        current_addon: addon_name,
                                    },
                                ));
                            }
                        },
                    )
                    .await;

                    let _ = sender.send(AppTaskEvent::DashboardCheckFinished(result));
                });
            }
            AppAction::StartBackupNow {
                addon_dir,
                backup_dir,
                retention,
            } => {
                let sender = self.task_events_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        create_wtf_backup(&addon_dir, &backup_dir, retention)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::BackupFinished(result));
                });
            }
            AppAction::StartBackupRestore {
                addon_dir,
                backup_path,
            } => {
                let sender = self.task_events_tx.clone();
                let backup_dir = self.backup_dir.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        restore_wtf_backup(&addon_dir, &backup_dir, &backup_path)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::BackupRestoreFinished(result));
                });
            }
            AppAction::StartBackupDelete {
                backup_root,
                backup_path,
            } => {
                let sender = self.task_events_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        delete_backup(&backup_root, &backup_path)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::BackupDeleteFinished(result));
                });
            }
            AppAction::StartTransferImport {
                addon_dir,
                input_path,
                wago_api_key,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|error| error.to_string())?;
                        runtime.block_on(run_transfer_import_task(
                            &state_db_file,
                            &addon_dir,
                            &input_path,
                            wago_api_key,
                        ))
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::TransferImportFinished(result));
                });
            }
            AppAction::StartWagoSearch { query, api_key } => {
                let sender = self.task_events_tx.clone();
                let flavor = self
                    .effective_addon_dir
                    .as_deref()
                    .map_or(GameFlavor::Retail, GameFlavor::for_addons_dir);
                tokio::spawn(async move {
                    let result = search_wago_addons(&query, &api_key, flavor)
                        .await
                        .map(|results| WagoSearchOutcome { query, results });
                    let _ = sender.send(AppTaskEvent::WagoSearchFinished(result));
                });
            }
            AppAction::StartWagoInstall {
                addon_dir,
                api_key,
                request,
                replace_folders,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|error| SourceError::from(error.to_string()))?;
                        runtime.block_on(run_wago_install_task(
                            &state_db_file,
                            &addon_dir,
                            &api_key,
                            request,
                            replace_folders,
                        ))
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string().into()));
                    let _ = sender.send(AppTaskEvent::WagoInstallFinished(result));
                });
            }
            AppAction::StartDirectInstall {
                addon_dir,
                request,
                replace_folders,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let task_request = request.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|error| SourceError::from(error.to_string()))?;
                        runtime.block_on(run_direct_install_task(
                            &state_db_file,
                            &addon_dir,
                            &task_request,
                            &replace_folders,
                        ))
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string().into()));
                    let _ = sender.send(AppTaskEvent::DirectInstallFinished { request, result });
                });
            }
            AppAction::ReplaceDashboardAddons(addons) => self.dashboard.replace_addons(addons),
            AppAction::CompleteAddonScan { path, summary } => {
                self.scan_state = ScanState::Succeeded {
                    path: path.clone(),
                    summary: summary.clone(),
                };
                // Looking through the folder ends with nothing to say.
                self.status_line = self.dashboard_status_for(
                    StatusKind::Quiet,
                    self.dashboard.detail_mode,
                    &format!(
                        "looked through the AddOns folder: {} addons, {} removed",
                        summary.upserted_addons, summary.removed_addons
                    ),
                );
                if let Some(text) = scan_warning_text(&summary) {
                    self.shell_ui.status_flash = Some(StatusFlash {
                        text: truncate_text(&text, 160),
                        kind: StatusKind::Problem,
                        offer: None,
                        expires_at: None,
                    });
                }
            }
            AppAction::FailAddonScan(error) => {
                self.scan_state = ScanState::Failed(error.clone());
                self.status_line = self.dashboard_status_for(
                    StatusKind::Problem,
                    self.dashboard.detail_mode,
                    &format!("LemonUp couldn't read your AddOns folder: {error}"),
                );
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        self.note_question_shown(Instant::now());
        if self.shell_mode == ShellMode::Dashboard {
            self.draw_dashboard_frame(frame);
            return;
        }
        frame.render_widget(
            Block::default().style(Style::default().bg(self.ui_theme.app_bg)),
            frame.area(),
        );
        let shell = self.shell_frame(frame.area());
        self.render_onboarding_header(frame, shell.header);
        self.render_onboarding(frame, shell.body);
        self.render_overlay_host(frame, shell.body, 0);
        self.render_footer(frame, shell.footer);
    }

    /// The setup frame: a boxed header, the body and a boxed footer. The
    /// dashboard draws its own frame (`draw_dashboard_frame`).
    fn shell_frame(&self, area: Rect) -> ShellFrame {
        let [header, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(4),
            ])
            .areas(area);
        ShellFrame {
            header,
            body,
            footer,
        }
    }

    fn footer_hint_styles(&self, id: Option<FooterHintId>) -> (Style, Style) {
        if self
            .shell_ui
            .footer_pulse
            .is_some_and(|pulse| Some(pulse.hint) == id)
        {
            return (
                Style::default()
                    .fg(self.ui_theme.overlay_bg)
                    .bg(self.ui_theme.brand_gold)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            );
        }

        (
            Style::default()
                .fg(self.ui_theme.panel_title)
                .bg(self.ui_theme.key_bg)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(self.ui_theme.panel_title),
        )
    }

    fn dashboard_job_footer_line(&self) -> Option<Line<'static>> {
        let job = self.dashboard.job_ui()?;
        let accent = job.kind.accent(self.ui_theme);
        let label = format!(
            "{} {}",
            self.shell_ui.motion.spinner_frame(),
            job.kind.label()
        );
        let mut spans = Vec::new();
        spans.extend(shimmer_text_spans(
            &label,
            accent,
            self.ui_theme.shimmer_glow,
            ShimmerConfig::action(),
        ));
        spans.extend([
            Span::styled(
                format!("  {}/{}", job.current_index, job.total),
                Style::default().fg(self.ui_theme.info),
            ),
            Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                truncate_text(&job.current_addon, 48),
                Style::default().fg(accent),
            ),
        ]);
        Some(Line::from(spans))
    }

    fn render_onboarding_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.ui_theme.border));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [left, right] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .areas(inner);

        let left_widget = Paragraph::new(Line::from(vec![
            Span::styled("🍋 ", Style::default().fg(self.ui_theme.brand_gold)),
            Span::styled(
                "LemonUp Setup",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        frame.render_widget(left_widget, left);

        let right_widget = Paragraph::new(Line::from(Span::styled(
            format!(
                "Step {} of {}",
                self.onboarding.step.index() + 1,
                OnboardingStep::ALL.len()
            ),
            Style::default()
                .fg(self.ui_theme.highlight)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Right);
        frame.render_widget(right_widget, right);
    }

    fn render_onboarding(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.ui_theme.border));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [stepper_area, content_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(12)])
            .areas(inner);

        let stepper = Paragraph::new(self.onboarding_stepper_lines()).wrap(Wrap { trim: false });
        frame.render_widget(stepper, stepper_area);

        let content = Paragraph::new(self.onboarding_content_lines(content_area.width))
            .wrap(Wrap { trim: false });
        frame.render_widget(content, content_area);
    }

    fn onboarding_stepper_lines(&self) -> Vec<Line<'static>> {
        let label_width = 16usize;
        let segment_width = 6usize;
        let marker_width = 4usize;
        let progress_spans = OnboardingStep::ALL
            .iter()
            .enumerate()
            .flat_map(|(index, step)| {
                let step_index = step.index();
                let is_completed = step_index < self.onboarding.step.index();
                let is_current = *step == self.onboarding.step;
                let marker = if is_completed {
                    "✓"
                } else if is_current {
                    "●"
                } else {
                    "○"
                };
                let marker_style = if is_completed {
                    Style::default()
                        .fg(self.ui_theme.success)
                        .add_modifier(Modifier::BOLD)
                } else if is_current {
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                };
                let left_connector_style = if index == 0 {
                    Style::default().fg(self.ui_theme.muted)
                } else if index - 1 < self.onboarding.step.index() {
                    Style::default().fg(self.ui_theme.success)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                };
                let right_connector_style = if index >= OnboardingStep::ALL.len() - 1 {
                    Style::default().fg(self.ui_theme.muted)
                } else if index < self.onboarding.step.index() {
                    Style::default().fg(self.ui_theme.success)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                };

                let leading = if index == 0 {
                    " ".repeat(segment_width)
                } else {
                    "━".repeat(segment_width)
                };
                let trailing = if index == OnboardingStep::ALL.len() - 1 {
                    " ".repeat(segment_width)
                } else {
                    "━".repeat(segment_width)
                };

                vec![
                    Span::styled(leading, left_connector_style),
                    Span::styled(
                        format!("{:^width$}", marker, width = marker_width),
                        marker_style,
                    ),
                    Span::styled(trailing, right_connector_style),
                ]
            })
            .collect::<Vec<_>>();

        let labels = OnboardingStep::ALL
            .iter()
            .map(|step| {
                let style = if step.index() < self.onboarding.step.index() {
                    Style::default().fg(self.ui_theme.success)
                } else if *step == self.onboarding.step {
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                };
                Span::styled(
                    format!("{:^width$}", step.label(), width = label_width),
                    style,
                )
            })
            .collect::<Vec<_>>();

        vec![Line::from(progress_spans), Line::from(labels)]
    }

    fn onboarding_content_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self.onboarding.step {
            OnboardingStep::Theme => self.onboarding_theme_lines(),
            OnboardingStep::Directory => self.onboarding_directory_lines(width),
            OnboardingStep::Wago => self.onboarding_wago_lines(),
            OnboardingStep::Settings => self.onboarding_settings_lines(),
            OnboardingStep::Review => self.onboarding_review_lines(),
        }
    }

    fn onboarding_theme_lines(&self) -> Vec<Line<'static>> {
        let label = match self.onboarding.draft.theme {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        };
        vec![
            Line::from(Span::styled(
                "Choose your theme:",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("◀ ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    format!("[ {label} ]"),
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ▶", Style::default().fg(self.ui_theme.muted)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Use ←/→ to toggle. Enter continues.",
                Style::default().fg(self.ui_theme.muted),
            )),
        ]
    }

    fn onboarding_directory_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(Span::styled(
                "Where is your WoW AddOns folder?",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        if let Some((notice, is_error)) = self.startup_notice.as_ref() {
            let color = if *is_error {
                self.ui_theme.warning
            } else {
                self.ui_theme.success
            };
            lines.push(Line::from(Span::styled(
                notice.replace(" | ", " · "),
                Style::default().fg(color),
            )));
            lines.push(Line::from(""));
        }

        match &self.onboarding.phase {
            OnboardingPhase::Bootstrapping => {
                lines.push(Line::from("Preparing location finder..."));
            }
            OnboardingPhase::QuickChecking => {
                lines.push(Line::from("Checking common install locations..."));
            }
            OnboardingPhase::Ready => {
                if let Some(warning) = self.profile_warning() {
                    lines.push(Line::from(Span::styled(
                        warning,
                        Style::default().fg(self.ui_theme.warning),
                    )));
                    lines.push(Line::from(""));
                }
                self.push_onboarding_folder_choices(&mut lines);
            }
            OnboardingPhase::DeepScanning(progress) => {
                lines.push(Line::from(Span::styled(
                    format!("Searching… {} folders checked", progress.dirs_scanned),
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(progress.current_path.clone()));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Esc stops the scan so you can pick a folder yourself.",
                    Style::default().fg(self.ui_theme.muted),
                )));
            }
            OnboardingPhase::Found(found) => {
                let labels = if let [path] = found.paths.as_slice() {
                    lines.push(Line::from(vec![
                        Span::styled("✓ Detected: ", Style::default().fg(self.ui_theme.success)),
                        Span::styled(path.clone(), Style::default().fg(self.ui_theme.info)),
                    ]));
                    vec!["Use this path".to_string()]
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("✓ Detected {} installs:", found.paths.len()),
                        Style::default().fg(self.ui_theme.success),
                    )));
                    found
                        .paths
                        .iter()
                        .map(|path| {
                            let client = match GameFlavor::detect_from_path(Path::new(path)) {
                                Some(GameFlavor::Forever) => "WoW Forever",
                                _ => "Retail",
                            };
                            format!("Use {client}: {path}")
                        })
                        .collect()
                };
                lines.push(Line::from(""));
                let labels = labels
                    .into_iter()
                    .chain(std::iter::once("Enter different path".to_string()))
                    .collect::<Vec<_>>();
                for (index, label) in labels.iter().enumerate() {
                    let selected = found.selected_action == index;
                    lines.push(Line::from(vec![
                        Span::styled(
                            if selected { "› " } else { "  " },
                            Style::default().fg(self.ui_theme.highlight),
                        ),
                        Span::styled(
                            label.clone(),
                            if selected {
                                Style::default()
                                    .fg(self.ui_theme.highlight)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(self.ui_theme.info)
                            },
                        ),
                    ]));
                }
                if found.paths.len() > 1 {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "LemonUp looks after one game version at a time. Use --profile <name> for the other one.",
                        Style::default().fg(self.ui_theme.muted),
                    )));
                }
            }
            OnboardingPhase::Error(message) => {
                lines.push(Line::from(Span::styled(
                    format!("✗ {message}"),
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(""));
                self.push_onboarding_folder_choices(&mut lines);
            }
            OnboardingPhase::Cancelled => {
                lines.push(Line::from(Span::styled(
                    "Scan stopped.",
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(""));
                self.push_onboarding_folder_choices(&mut lines);
            }
        }

        if self.onboarding.is_editing {
            // The shared text box, starting from the path as it is: the
            // cursor is at the end and the text before it can be edited.
            let label = "Editing: ";
            let style = Style::default().fg(self.ui_theme.highlight);
            let mut text_box = self.onboarding.text_box.clone();
            if text_box.value() != self.onboarding.input {
                text_box = TextInput::new(self.onboarding.input.clone());
            }
            let room = usize::from(width).saturating_sub(label.len());
            let mut spans = vec![Span::styled(label, style)];
            spans.extend(text_box.spans(room, style, style.add_modifier(Modifier::BOLD), true));
            lines.push(Line::from(""));
            lines.push(Line::from(spans));
        }

        lines
    }

    /// Folders to scan from (home first), shown when no install was found automatically.
    fn push_onboarding_folder_choices(&self, lines: &mut Vec<Line<'static>>) {
        lines.push(Line::from(
            "Pick a folder to search, or type the AddOns path:",
        ));
        lines.push(Line::from(""));
        for (index, suggestion) in self.onboarding.suggestions.iter().enumerate() {
            let selected = self.onboarding.selected_suggestion == Some(index);
            lines.push(Line::from(vec![
                Span::styled(
                    if selected { "› " } else { "  " },
                    Style::default().fg(self.ui_theme.highlight),
                ),
                Span::styled(
                    suggestion.clone(),
                    if selected {
                        Style::default().fg(self.ui_theme.highlight)
                    } else {
                        Style::default().fg(self.ui_theme.info)
                    },
                ),
            ]));
        }
        if self.onboarding.selected_suggestion.is_none() && !self.onboarding.is_editing {
            lines.push(Line::from(vec![
                Span::styled("› ", Style::default().fg(self.ui_theme.highlight)),
                Span::styled(
                    self.onboarding.input.clone(),
                    Style::default().fg(self.ui_theme.highlight),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Enter searches the selected folder. e types a path.",
            Style::default().fg(self.ui_theme.muted),
        )));
    }

    fn onboarding_wago_lines(&self) -> Vec<Line<'static>> {
        let value = self.onboarding.wago_api_key_value();
        vec![
            Line::from(Span::styled(
                "Wago.io key (you can skip this)",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Wago.io is one of the sites LemonUp gets addons from. Searching it, and updating addons that came from it, needs your own Wago.io key. Wago gives one to its Patreon supporters:",
                Style::default().fg(self.ui_theme.panel_title),
            )),
            Line::from(Span::styled(
                "addons.wago.io/patreon",
                Style::default().fg(self.ui_theme.info),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Addons from GitHub, WoWInterface and TukUI work without a key. You can add or change it later in Settings.",
                Style::default().fg(self.ui_theme.muted),
            )),
            Line::from(""),
            Line::from(Span::styled(
                if value.is_empty() {
                    "No Wago.io key saved yet."
                } else {
                    "Wago.io key entered."
                },
                Style::default().fg(self.ui_theme.info),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Key: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    if value.is_empty() {
                        "<empty>".to_string()
                    } else {
                        format!("set ({} chars)", value.len())
                    },
                    Style::default().fg(self.ui_theme.highlight),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                if self.onboarding.is_editing {
                    "Editing active. Type the key, Enter applies, Esc cancels."
                } else {
                    "Press e to edit, or Enter to continue without a key."
                },
                Style::default().fg(self.ui_theme.muted),
            )),
        ]
    }

    fn onboarding_settings_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(Span::styled(
                "Choose your core settings:",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        for field in OnboardingSettingsField::ALL {
            let selected = field == self.onboarding.settings_selection;
            lines.push(Line::from(vec![
                Span::styled(
                    if selected { "› " } else { "  " },
                    Style::default().fg(self.ui_theme.highlight),
                ),
                Span::styled(
                    format!("{:<18}", field.label()),
                    if selected {
                        Style::default()
                            .fg(self.ui_theme.highlight)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.ui_theme.info)
                    },
                ),
                Span::styled(
                    self.onboarding_setting_value(field),
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Use ↑/↓ to choose a setting. ←/→ changes the value. Enter continues.",
            Style::default().fg(self.ui_theme.muted),
        )));
        lines
    }

    fn onboarding_review_lines(&self) -> Vec<Line<'static>> {
        vec![
            Line::from(Span::styled(
                "Review your setup:",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!(
                "Theme: {}",
                match self.onboarding.draft.theme {
                    ThemeMode::Dark => "dark",
                    ThemeMode::Light => "light",
                }
            )),
            Line::from(format!("AddOns folder: {}", self.onboarding.input)),
            Line::from(format!(
                "Wago.io key: {}",
                if self.onboarding.wago_api_key_value().is_empty() {
                    "not configured".to_string()
                } else {
                    format!(
                        "saved ({} characters)",
                        self.onboarding.wago_api_key_value().len()
                    )
                }
            )),
            Line::from(format!(
                "Back up my game settings before updates: {}",
                on_off(self.onboarding.draft.backup_wtf)
            )),
            Line::from(format!(
                "Backups to keep: {}",
                self.onboarding.draft.backup_retention
            )),
            Line::from(format!(
                "Show library folders: {}",
                on_off(self.onboarding.draft.show_libs)
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Enter saves the config and starts the first scan. Esc goes back.",
                Style::default().fg(self.ui_theme.muted),
            )),
        ]
    }

    fn task_overlay_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let selected = self.dashboard.selected_item();
        let selected_row = self.dashboard.selected_row();
        let selected_owned_child = self.dashboard.selected_owned_child_folder();

        if let Some(undo_delete) = self.undo_delete.as_ref() {
            lines.push(self.overlay_section_title("Undo ready"));
            lines.push(self.overlay_kv_line(
                "Batch",
                format!(
                    "{} parent{}, {} folder{}",
                    undo_delete.parent_count(),
                    plural_suffix(undo_delete.parent_count()),
                    undo_delete.moved_folder_count(),
                    plural_suffix(undo_delete.moved_folder_count())
                ),
            ));
            lines.push(self.overlay_kv_line("Targets", undo_delete.target_summary()));
            lines.push(Line::from(""));
        }

        match self.dashboard.detail_mode {
            DetailMode::Overview => {
                if let (Some(item), Some(child_folder)) = (selected, selected_owned_child) {
                    lines.push(Line::from(format!("Child: {child_folder}")));
                    lines.push(Line::from(format!("Parent: {}", item.name)));
                    lines.push(Line::from(format!(
                        "Status: {}",
                        dashboard_item_status_text(item)
                    )));
                } else if let Some(item) = selected {
                    lines.push(Line::from(item.name.clone()));
                    lines.push(Line::from(format!(
                        "{} | {}",
                        dashboard_item_status_text(item),
                        dashboard_item_version_label(item)
                    )));
                    lines.push(Line::from(format!("Source: {}", source_label(item.source))));
                    lines.push(Line::from(format!(
                        "Management: {}",
                        dashboard_item_management_summary(
                            item,
                            self.parent_has_drift(&item.folder)
                        )
                    )));
                    let missing_owned_children =
                        self.dashboard_parent_missing_owned_children(&item.folder);
                    if !missing_owned_children.is_empty() {
                        lines.push(Line::from(format!(
                            "Missing: {}",
                            missing_owned_children.join(", ")
                        )));
                    }
                    if !item.owned_folders.is_empty() {
                        lines.push(Line::from(format!(
                            "Children: {}",
                            summarize_owned_folders(&item.owned_folders)
                        )));
                    }
                } else {
                    lines.push(Line::from("No addon selected."));
                    lines.push(Line::from("Waiting for scan results."));
                }
                lines.push(Line::from(""));
                if let Some(row) = selected_row {
                    lines.push(Line::from(format!(
                        "Row: {}",
                        match row.kind {
                            DashboardRowKind::Parent => "parent",
                            DashboardRowKind::OwnedChild { .. } => "owned child",
                        }
                    )));
                }
                lines.push(Line::from(format!("Scan: {}", self.scan_status_label())));
                for line in self.last_scan_drift_lines() {
                    lines.push(line);
                }
                if let Some(summary) = self.dashboard.last_update_summary() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(format!(
                        "Last update: updated {}, up-to-date {}, errors {}",
                        summary.updated_addons, summary.up_to_date, summary.errors
                    )));
                }
            }
            DetailMode::Install => {
                lines.push(self.overlay_section_title("Direct install"));
                lines.push(self.overlay_kv_line("Provider", "Wago"));
                lines.push(self.overlay_kv_line(
                    "Target",
                    if self.install_pane.input.value().trim().is_empty() {
                        "<empty>".to_string()
                    } else {
                        self.install_pane.input.value().to_string()
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "State",
                    if self.wago_install_in_progress {
                        "running"
                    } else if self.install_pane.is_editing {
                        "editing"
                    } else {
                        "ready"
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "Auth",
                    if self.wago_api_key.is_some() {
                        "ready"
                    } else {
                        "missing"
                    },
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                if self.wago_api_key.is_none() {
                    lines.push(self.overlay_hint_line("Set Wago API key before install can run."));
                    lines.push(self.overlay_hint_line(crate::wago::wago_api_key_sources_hint()));
                } else {
                    lines.push(
                        self.overlay_hint_line("Accepts a slug or addons.wago.io addon URL."),
                    );
                    lines.push(self.overlay_hint_line("Retail and stable only."));
                    lines.push(self.overlay_hint_line(if self.install_pane.is_editing {
                        "Type target, then Enter to install. Esc stops editing."
                    } else {
                        "Press e to edit target. Enter installs current target."
                    }));
                }
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Notes"));
                lines.push(self.overlay_hint_line(
                    "Other providers stay out of this direct-install surface for now.",
                ));
            }
            DetailMode::Search => {
                lines.push(self.overlay_section_title("Search"));
                lines.push(self.overlay_kv_line("Provider", "Wago"));
                lines.push(self.overlay_kv_line(
                    "Query",
                    if self.search_pane.query.value().trim().is_empty() {
                        "<empty>".to_string()
                    } else {
                        self.search_pane.query.value().to_string()
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "State",
                    if self.search_pane.in_progress {
                        "running"
                    } else {
                        "ready"
                    },
                ));
                lines.push(
                    self.overlay_kv_line("Results", self.search_pane.results.len().to_string()),
                );
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                if self.wago_api_key.is_none() {
                    lines.push(self.overlay_hint_line("Set Wago API key before search can run."));
                    lines.push(self.overlay_hint_line(crate::wago::wago_api_key_sources_hint()));
                } else {
                    lines.push(self.overlay_hint_line("Retail and stable only."));
                    lines.push(self.overlay_hint_line(
                        "Type a name, then Enter to search. Enter on a result installs it.",
                    ));
                    if self.search_pane.results.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(self.overlay_section_title("Results"));
                        lines.push(self.overlay_hint_line("No results loaded yet."));
                    } else {
                        let (window_start, window_end) = visible_search_result_window(
                            self.search_pane.results.len(),
                            self.search_pane.selected_result,
                            8,
                        );
                        lines.push(Line::from(""));
                        lines.push(self.overlay_section_title("Results"));
                        if window_start > 0 {
                            lines.push(self.overlay_hint_line(&format!(
                                "{} more result{} above",
                                window_start,
                                plural_suffix(window_start)
                            )));
                        }
                        for (index, result) in self.search_pane.results[window_start..window_end]
                            .iter()
                            .enumerate()
                        {
                            let absolute_index = window_start + index;
                            let marker = if self.search_pane.selected_result == Some(absolute_index)
                            {
                                "›"
                            } else {
                                " "
                            };
                            lines.push(Line::from(format!(
                                "{marker} {} | {} | dl {} | v {}",
                                result.display_name,
                                result
                                    .owner
                                    .as_deref()
                                    .or_else(|| result.authors.first().map(String::as_str))
                                    .unwrap_or("unknown"),
                                result
                                    .download_count
                                    .map(format_download_count)
                                    .unwrap_or_else(|| "<unknown>".to_string()),
                                result.version.as_deref().unwrap_or("<unknown>")
                            )));
                        }
                        let remaining_below =
                            self.search_pane.results.len().saturating_sub(window_end);
                        if remaining_below > 0 {
                            lines.push(self.overlay_hint_line(&format!(
                                "{} more result{} below",
                                remaining_below,
                                plural_suffix(remaining_below)
                            )));
                        }
                        if let Some(result) = self.search_pane.selected_result() {
                            lines.push(Line::from(""));
                            lines.push(self.overlay_section_title("Selected"));
                            lines.push(self.overlay_kv_line("Addon", result.display_name.clone()));
                            lines.push(self.overlay_kv_line("Slug", result.id.clone()));
                            lines.push(
                                self.overlay_kv_line(
                                    "Author",
                                    result
                                        .owner
                                        .as_deref()
                                        .or_else(|| result.authors.first().map(String::as_str))
                                        .unwrap_or("unknown"),
                                ),
                            );
                            lines.push(self.overlay_kv_line(
                                "Version",
                                result.version.as_deref().unwrap_or("<unknown>"),
                            ));
                            lines.push(self.overlay_kv_line(
                                "Summary",
                                result.summary.as_deref().unwrap_or("<none>"),
                            ));
                        }
                    }
                }
            }
            DetailMode::Update => {
                let all_items = self.dashboard.items.iter().collect::<Vec<_>>();
                let selected_items = self.dashboard.selected_parent_items();
                let selected_folders = self.dashboard.selected_parent_folders();
                let checks = build_update_checks_for_dashboard(&selected_items);
                let summary = summarize_checks(&checks);
                let inventory = summarize_refreshability(&all_items);
                let selection = summarize_refreshability(&selected_items);
                lines.push(self.overlay_section_title("Selected updates"));
                lines.push(self.overlay_kv_line(
                    "State",
                    if self.dashboard.update_in_progress() {
                        "running"
                    } else {
                        "ready"
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "Selected",
                    format!(
                        "{} parent{}",
                        selected_items.len(),
                        plural_suffix(selected_items.len())
                    ),
                ));
                lines.push(self.overlay_kv_line(
                    "Targets",
                    if selected_folders.is_empty() {
                        "<none>".to_string()
                    } else {
                        selected_folders.join(", ")
                    },
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Readiness"));
                lines.push(self.overlay_kv_line(
                    "Inventory",
                    format!(
                        "{} tracked | {} ready | {} manual | {} older",
                        inventory.total,
                        inventory.refreshable,
                        inventory.manual,
                        inventory.remote_older
                    ),
                ));
                lines.push(self.overlay_kv_line(
                    "Selection",
                    format!(
                        "{} ready | {} manual | {} older",
                        selection.refreshable, selection.manual, selection.remote_older
                    ),
                ));
                lines.push(self.overlay_kv_line(
                    "Remote",
                    format!(
                        "{} current | {} update | {} unknown | {} errors",
                        summary.up_to_date,
                        summary.update_available,
                        summary.unknown,
                        summary.errors
                    ),
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                lines.push(
                    self.overlay_hint_line("Press v to select update-ready tracked parents."),
                );
                lines.push(self.overlay_hint_line(
                    "Press r to apply provider-backed updates for the current selection.",
                ));
                if let Some(last_summary) = self.dashboard.last_update_summary() {
                    lines.push(Line::from(""));
                    lines.push(self.overlay_section_title("Last run"));
                    lines.push(self.overlay_kv_line(
                        "Results",
                        format!(
                            "{} targets | {} updated | {} current | {} older | {} manual | {} no key | {} errors",
                            last_summary.targets,
                            last_summary.updated_addons,
                            last_summary.up_to_date,
                            last_summary.remote_older,
                            last_summary.skipped_manual,
                            last_summary.skipped_no_api_key,
                            last_summary.errors
                        ),
                    ));
                }
            }
            DetailMode::Config => {
                lines.push(self.overlay_section_title("Config"));
                lines.push(self.overlay_kv_line("AddOns", self.rendered_target_path()));
                lines.push(self.overlay_kv_line(
                    "Config",
                    if self.config_present {
                        "present"
                    } else {
                        "missing"
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "Draft",
                    if self.config_pane.is_dirty() {
                        "unsaved changes"
                    } else {
                        "saved"
                    },
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Fields"));
                for (index, field) in ConfigField::ALL.iter().enumerate() {
                    let marker = if self.config_pane.selected_field == index {
                        "›"
                    } else {
                        " "
                    };
                    let mut value = self.render_config_field_value(*field);
                    if self
                        .config_pane
                        .edit
                        .as_ref()
                        .is_some_and(|edit| edit.field == *field)
                    {
                        value = format!(
                            "{} (editing)",
                            self.config_pane
                                .edit
                                .as_ref()
                                .map(|edit| edit.value.value().to_string())
                                .unwrap_or(value)
                        );
                    }
                    lines.push(Line::from(format!("{marker} {}: {}", field.label(), value)));
                }
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                lines.push(self.overlay_hint_line(
                    "j/k move | enter toggle/cycle | e edit text | s save | n reset | esc stop edit",
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Addon transfer"));
                lines.push(self.overlay_hint_line(&format!(
                        "x export tracked addons | i import addon list | {}",
                        default_transfer_path()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|error| format!("path unavailable: {error}"))
                    )));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Notes"));
                lines.push(self.overlay_hint_line(
                    "Only curated settings are editable here. Addon dir still stays read-only.",
                ));
                lines.push(self.overlay_hint_line(
                    "Use 'Run onboarding again' to reopen the location finder manually.",
                ));
            }
            DetailMode::Backup => {
                lines.push(self.overlay_section_title("Backup now"));
                lines.push(self.overlay_kv_line("AddOns", self.rendered_target_path()));
                lines.push(self.overlay_kv_line("Store", self.backup_dir.display().to_string()));
                lines.push(self.overlay_kv_line(
                    "Policy",
                    format!(
                        "enabled={} | retention={}",
                        self.config_pane.draft.backup_wtf, self.config_pane.draft.backup_retention
                    ),
                ));
                lines.push(self.overlay_kv_line(
                    "State",
                    if self.backup_pane.in_progress {
                        "running"
                    } else if self.backup_pane.pending_restore.is_some() {
                        "awaiting restore confirmation"
                    } else {
                        "ready"
                    },
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                if let Some(pending) = &self.backup_pane.pending_restore {
                    lines.push(self.overlay_hint_line(&format!(
                        "Restore {} and replace the current WTF folder? y confirm | n cancel",
                        pending.label
                    )));
                } else {
                    lines.push(self.overlay_hint_line(
                        "n create backup now | r/enter restore selected | j/k move",
                    ));
                }
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Recent backups"));
                if self.backup_pane.backups.is_empty() {
                    lines.push(self.overlay_hint_line("No backups created yet."));
                } else {
                    for (index, entry) in self.backup_pane.backups.iter().take(8).enumerate() {
                        lines.push(Line::from(format!(
                            "{} {} | {} bytes",
                            if index == self.backup_pane.selected_backup {
                                ">"
                            } else {
                                " "
                            },
                            entry.label,
                            entry.size_bytes
                        )));
                        lines.push(self.overlay_hint_line(&entry.file_name));
                    }
                    if self.backup_pane.backups.len() > 8 {
                        let remaining = self.backup_pane.backups.len() - 8;
                        lines.push(self.overlay_hint_line(&format!(
                            "{} more backup{}",
                            remaining,
                            plural_suffix(remaining)
                        )));
                    }
                }
            }
        }

        lines
    }

    /// The names the list shows for `folders`, in the same order.
    fn dashboard_names(&self, folders: &[String]) -> Vec<String> {
        folders
            .iter()
            .map(|folder| {
                self.dashboard
                    .items
                    .iter()
                    .find(|item| item.folder == *folder || item.name == *folder)
                    .map_or_else(|| folder.clone(), |item| item.name.clone())
            })
            .collect()
    }

    fn inspect_resolved_target(&self) -> Option<InspectResolvedTarget<'_>> {
        let item = self.dashboard.selected_item()?;
        Some(InspectResolvedTarget { item })
    }

    fn base_status_line(&self) -> String {
        match self.shell_mode {
            ShellMode::Onboarding => format!("profile {} | q quit", self.runtime.profile_name),
            ShellMode::Dashboard => format!("profile {}", self.runtime.profile_name),
        }
    }

    fn with_base_status(&self, kind: StatusKind, message: &str) -> StatusMessage {
        StatusMessage::new(kind, format!("{message} | {}", self.base_status_line()))
    }

    fn dashboard_status_for(
        &self,
        kind: StatusKind,
        detail_mode: DetailMode,
        message: &str,
    ) -> StatusMessage {
        self.with_base_status(
            kind,
            &format!("{} | {message}", detail_mode_label(detail_mode)),
        )
    }

    fn active_detail_status_message(&self, detail_mode: DetailMode) -> Option<String> {
        if self.status_line.is_empty() || !self.status_line.kind.is_shown() {
            return None;
        }

        let prefix = format!("{} | ", detail_mode_label(detail_mode));
        let suffix = format!(" | {}", self.base_status_line());
        let text = self.status_line.text.as_str();
        let without_suffix = text.strip_suffix(&suffix).unwrap_or(text);
        without_suffix
            .strip_prefix(&prefix)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(ToOwned::to_owned)
    }

    /// Starts a scan from the current onboarding path. The scan accepts an AddOns folder
    /// directly, so this both checks a typed path and searches a folder such as home.
    fn onboarding_scan_actions(&self) -> Vec<AppAction> {
        let root = PathBuf::from(self.onboarding.input.trim());
        vec![
            AppAction::SetOnboardingState(self.onboarding.start_deep_scan()),
            AppAction::SetStatus(
                self.with_base_status(StatusKind::Working, "scanning for WoW installs | esc stop"),
            ),
            AppAction::StartOnboardingDeepScan(root),
        ]
    }

    fn location_finder_status(&self) -> StatusMessage {
        self.with_base_status(
            StatusKind::Quiet,
            "location finder | enter scan | e edit path",
        )
    }

    /// The game client the AddOns folder belongs to, for the header.
    fn game_client_label(&self) -> String {
        match self.effective_addon_dir.as_deref() {
            None => "No AddOns folder set".to_string(),
            Some(path) => match GameFlavor::detect_from_path(path) {
                Some(GameFlavor::Forever) => "WoW Forever".to_string(),
                Some(_) => "Retail".to_string(),
                None => "AddOns".to_string(),
            },
        }
    }

    /// The AddOns folder with the home folder shortened to `~` (not on Windows).
    fn friendly_addon_dir(&self) -> String {
        let Some(path) = self.effective_addon_dir.as_deref() else {
            return String::new();
        };
        if !cfg!(windows)
            && let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
            && let Ok(rest) = path.strip_prefix(&home)
            && !home.as_os_str().is_empty()
        {
            return Path::new("~").join(rest).display().to_string();
        }
        path.display().to_string()
    }

    fn rendered_target_path(&self) -> String {
        self.effective_addon_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unconfigured>".to_string())
    }

    fn auto_check_due_targets(&self) -> Option<Vec<String>> {
        if self.shell_mode != ShellMode::Dashboard
            || self.dashboard.detail_mode != DetailMode::Overview
            || self.shell_ui.overlay.active.is_some()
            || !self.config_pane.draft.auto_check_enabled
            || self.auto_check_in_progress
            || self.dashboard.update_in_progress
            || self.dashboard.job_ui().is_some()
            || self.undo_delete_in_progress
            || self.backup_pane.in_progress
            || self.wago_install_in_progress
            || self.direct_install_in_progress
            || self.pending_wago_install_confirmation.is_some()
            || self.effective_addon_dir.is_none()
            || !matches!(self.scan_state, ScanState::Succeeded { .. })
        {
            return None;
        }

        let auto_interval = StdDuration::from_secs(self.config_pane.draft.auto_check_interval_secs);
        if let Some(last_started_at) = self.last_auto_check_started_at
            && last_started_at.elapsed() < auto_interval
        {
            return None;
        }

        // Every tracked row is checked, including ones whose remote is currently older, so a
        // fixed provider page is noticed.
        let folders = self
            .dashboard
            .items
            .iter()
            .filter(|item| item.source != SourceKind::Manual)
            .map(|item| item.folder.clone())
            .collect::<Vec<_>>();
        if folders.is_empty() {
            return None;
        }

        let stale = stale_target_selectors(
            &self
                .dashboard
                .items
                .iter()
                .map(dashboard_item_to_addon_record)
                .collect::<Vec<_>>(),
            &folders,
            self.config_pane.draft.check_interval_secs,
        );
        if stale.is_empty() {
            return None;
        }

        Some(folders)
    }

    fn render_config_field_value(&self, field: ConfigField) -> String {
        match field {
            ConfigField::WagoApiKey => self
                .config_pane
                .draft
                .wago_api_key
                .as_ref()
                .map(|value| {
                    if value.is_empty() {
                        "Saved".to_string()
                    } else {
                        format!("Saved ({} characters)", value.len())
                    }
                })
                .unwrap_or_else(|| "Not set".to_string()),
            ConfigField::CheckInterval => {
                format!("{}s", self.config_pane.draft.check_interval_secs)
            }
            ConfigField::AutoCheckEnabled => {
                if self.config_pane.draft.auto_check_enabled {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            }
            ConfigField::AutoCheckInterval => {
                format!("{}s", self.config_pane.draft.auto_check_interval_secs)
            }
            ConfigField::BackupWtf => {
                if self.config_pane.draft.backup_wtf {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            }
            ConfigField::BackupRetention => self.config_pane.draft.backup_retention.to_string(),
            ConfigField::Theme => match self.config_pane.draft.theme {
                ThemeMode::Dark => "Dark".to_string(),
                ThemeMode::Light => "Light".to_string(),
            },
            ConfigField::ShowLibs => {
                if self.config_pane.draft.show_libs {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            }
        }
    }

    fn onboarding_setting_value(&self, field: OnboardingSettingsField) -> String {
        match field {
            OnboardingSettingsField::BackupWtf => {
                on_off(self.onboarding.draft.backup_wtf).to_string()
            }
            OnboardingSettingsField::BackupRetention => {
                self.onboarding.draft.backup_retention.to_string()
            }
            OnboardingSettingsField::ShowLibs => {
                on_off(self.onboarding.draft.show_libs).to_string()
            }
        }
    }

    fn is_guarded_path(&self, candidate: &Path) -> bool {
        self.runtime.is_guarded_path(candidate)
    }

    fn profile_warning(&self) -> Option<String> {
        if self.runtime.is_default_profile() {
            return None;
        }

        if self.shell_mode == ShellMode::Onboarding
            && self.is_guarded_path(Path::new(self.onboarding.input.trim()))
        {
            return Some(
                "Warning: non-default profiles must use a sandbox AddOns directory.".to_string(),
            );
        }

        self.effective_addon_dir.as_ref().and_then(|path| {
            self.is_guarded_path(path).then(|| {
                "Warning: non-default profiles must use a sandbox AddOns directory.".to_string()
            })
        })
    }

    /// Found state for the discovered paths, moving the selection past any path this profile
    /// must not target (ending on "Enter different path" when every path is guarded).
    fn onboarding_found_state(&self, paths: Vec<PathBuf>) -> OnboardingState {
        let mut state = self.onboarding.found_many(paths);
        while state.selected_found_action() == FoundAction::UseThisPath
            && self.is_guarded_path(Path::new(state.input.trim()))
        {
            state = state.next_found_action();
        }
        state
    }

    fn found_status_for_input(&self) -> StatusMessage {
        self.found_status_for_path(Path::new(self.onboarding.input.trim()))
    }

    fn found_status_for_path(&self, path: &Path) -> StatusMessage {
        if self.is_guarded_path(path) {
            self.with_base_status(
                StatusKind::Note,
                "found default profile target | choose another location",
            )
        } else {
            self.with_base_status(
                StatusKind::Done,
                "found WoW installation | choose an action, then press enter",
            )
        }
    }

    fn parent_has_drift(&self, folder: &str) -> bool {
        !self
            .dashboard_parent_missing_owned_children(folder)
            .is_empty()
    }

    fn dashboard_parent_missing_owned_children(&self, folder: &str) -> &[String] {
        self.dashboard
            .drift_report()
            .map(|report| report.missing_owned_children_for(folder))
            .unwrap_or(&[])
    }

    fn last_scan_drift_lines(&self) -> Vec<Line<'static>> {
        let Some(report) = self.dashboard.drift_report() else {
            return vec![Line::from(
                "Last scan drift: unavailable until the next full scan.",
            )];
        };

        if report.is_empty() {
            return vec![Line::from("Last scan drift: no issues detected.")];
        }

        vec![Line::from(format!(
            "Last scan drift: imported {}, removed {}, orphaned children {}.",
            report.imported_disk_only_folders.len(),
            report.removed_missing_records.len(),
            report.orphaned_owned_children_on_disk.len()
        ))]
    }

    fn scan_status_label(&self) -> String {
        match &self.scan_state {
            ScanState::Idle => "not scanned yet".to_string(),
            ScanState::Pending | ScanState::Running(_) => "Scanning…".to_string(),
            ScanState::Succeeded { summary, .. } => {
                let count = summary.upserted_addons;
                let mut label = format!("{count} {}", if count == 1 { "addon" } else { "addons" });
                if summary.removed_addons > 0 {
                    label.push_str(&format!(" · {} removed", summary.removed_addons));
                }
                label
            }
            ScanState::Failed(error) => format!("scan failed: {error}"),
        }
    }

    fn config_field_saved_message(&self, field: ConfigField, config: &AppConfig) -> String {
        match field {
            ConfigField::WagoApiKey => {
                if config.wago_api_key.is_some() {
                    "Saved your Wago.io key".to_string()
                } else {
                    "Removed your Wago.io key".to_string()
                }
            }
            ConfigField::CheckInterval => {
                format!("Check freshness set to {}s", config.check_interval_secs)
            }
            ConfigField::AutoCheckEnabled => {
                if config.auto_check_enabled {
                    "Background auto-check enabled".to_string()
                } else {
                    "Background auto-check disabled".to_string()
                }
            }
            ConfigField::AutoCheckInterval => {
                format!(
                    "Auto-check interval set to {}s",
                    config.auto_check_interval_secs
                )
            }
            ConfigField::BackupWtf => {
                if config.backup_wtf {
                    "WTF backup enabled".to_string()
                } else {
                    "WTF backup disabled".to_string()
                }
            }
            ConfigField::BackupRetention => {
                format!("Backup retention set to {}", config.backup_retention)
            }
            ConfigField::Theme => match config.theme {
                ThemeMode::Dark => "Theme set to Dark".to_string(),
                ThemeMode::Light => "Theme set to Light".to_string(),
            },
            ConfigField::ShowLibs => {
                if config.show_libs {
                    "Show libs enabled".to_string()
                } else {
                    "Show libs disabled".to_string()
                }
            }
        }
    }

    fn persist_config_pane(
        &self,
        next: ConfigPaneState,
        field: ConfigField,
    ) -> std::result::Result<Vec<AppAction>, String> {
        match self.config_store.write_config(&next.draft) {
            Ok(()) => Ok(vec![
                AppAction::SetPersistedConfig(next.draft.clone()),
                AppAction::SetConfigPaneState(next.mark_saved(next.draft.clone())),
                AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Done,
                    DetailMode::Config,
                    &self.config_field_saved_message(field, &next.draft),
                )),
            ]),
            Err(error) => Err(format!("failed to save {}: {error}", field.label())),
        }
    }

    fn config_field_help_text(&self, field: ConfigField) -> &'static str {
        match field {
            ConfigField::WagoApiKey => {
                "Used only for Wago installs, search, and Wago update checks."
            }
            ConfigField::CheckInterval => {
                "How long a checked addon stays fresh before LemonUp fetches it again."
            }
            ConfigField::AutoCheckEnabled => {
                "Refresh tracked addon status in the background while the dashboard is open."
            }
            ConfigField::AutoCheckInterval => {
                "How often LemonUp runs background update checks for tracked addons."
            }
            ConfigField::BackupWtf => {
                "Include WTF data when creating backups before risky operations."
            }
            ConfigField::BackupRetention => {
                "How many recent backup archives LemonUp keeps before pruning older ones."
            }
            ConfigField::Theme => "Pick the shell color treatment used across the TUI.",
            ConfigField::ShowLibs => {
                "Show embedded library folders in addon scans and relationship views."
            }
        }
    }
}

fn delete_selected_addons(
    state_db_file: &Path,
    addon_dir: &Path,
    trash_dir: &Path,
    folders: &[String],
) -> std::result::Result<DashboardDeleteOutcome, String> {
    let database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let batch_dir = create_trash_batch_dir(trash_dir)?;
    let mut deleted_parents = 0usize;
    let mut deleted_folders = 0usize;
    let mut tracked_records = Vec::new();
    let mut tracked_folders = HashSet::new();
    let mut moved_entries = Vec::new();

    for folder in folders {
        let planned_folders = database
            .planned_removal_folders(folder)
            .map_err(|error| error.to_string())?;

        for planned_folder in &planned_folders {
            if let Some(record) = database
                .get_addon_by_folder(planned_folder)
                .map_err(|error| error.to_string())?
                && tracked_folders.insert(record.folder.clone())
            {
                tracked_records.push(record);
            }
        }

        for planned_folder in planned_folders {
            let path = addon_dir.join(&planned_folder);
            if path.exists() {
                let trashed_path = batch_dir.join(&planned_folder);
                move_path(&path, &trashed_path)?;
                moved_entries.push(DashboardTrashEntry {
                    folder: planned_folder,
                    original_path: path,
                    trashed_path,
                });
                deleted_folders += 1;
            }
        }

        database
            .remove_addon(folder)
            .map_err(|error| error.to_string())?;
        deleted_parents += 1;
    }

    let sync = sync_dashboard_state(state_db_file, addon_dir)?;
    let undo_delete = if moved_entries.is_empty() {
        let _ = fs::remove_dir_all(&batch_dir);
        None
    } else {
        Some(DashboardUndoDeleteState {
            batch_dir,
            addon_dir: addon_dir.to_path_buf(),
            deleted_parent_folders: folders.to_vec(),
            tracked_records,
            moved_entries,
        })
    };

    Ok(DashboardDeleteOutcome {
        deleted_parents,
        deleted_folders,
        undo_delete,
        sync,
    })
}

fn undo_deleted_addons(
    state_db_file: &Path,
    undo: &DashboardUndoDeleteState,
) -> std::result::Result<DashboardUndoOutcome, String> {
    for entry in &undo.moved_entries {
        if entry.original_path.exists() {
            return Err(format!(
                "cannot restore {} because the destination already exists",
                entry.original_path.display()
            ));
        }
    }

    for entry in &undo.moved_entries {
        move_path(&entry.trashed_path, &entry.original_path)?;
    }

    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    for record in &undo.tracked_records {
        if record.has_authoritative_owned_folders() {
            database
                .record_managed_addon(record)
                .map_err(|error| error.to_string())?;
        } else {
            database
                .upsert_addon(record)
                .map_err(|error| error.to_string())?;
        }
    }

    let _ = fs::remove_dir_all(&undo.batch_dir);

    let sync = sync_dashboard_state(state_db_file, &undo.addon_dir)?;
    let restored_parents = undo
        .deleted_parent_folders
        .iter()
        .filter(|folder| {
            sync.addons
                .iter()
                .any(|addon| addon.folder == folder.as_str())
        })
        .count();

    Ok(DashboardUndoOutcome {
        restored_parents,
        restored_folders: undo.moved_entries.len(),
        sync,
    })
}

fn create_trash_batch_dir(trash_dir: &Path) -> std::result::Result<PathBuf, String> {
    fs::create_dir_all(trash_dir).map_err(|error| error.to_string())?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();

    for attempt in 0..100u16 {
        let candidate = if attempt == 0 {
            trash_dir.join(format!("batch-{timestamp}"))
        } else {
            trash_dir.join(format!("batch-{timestamp}-{attempt}"))
        };
        if !candidate.exists() {
            fs::create_dir_all(&candidate).map_err(|error| error.to_string())?;
            return Ok(candidate);
        }
    }

    Err("failed to allocate a unique trash batch directory".to_string())
}

fn move_path(source: &Path, destination: &Path) -> std::result::Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) if source.is_dir() => {
            copy_dir_recursively(source, destination)?;
            fs::remove_dir_all(source).map_err(|error| error.to_string())
        }
        Err(_) if source.is_file() => {
            fs::copy(source, destination).map_err(|error| error.to_string())?;
            fs::remove_file(source).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn copy_dir_recursively(source: &Path, destination: &Path) -> std::result::Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_dir_recursively(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

async fn run_wago_install_task(
    state_db_file: &Path,
    addon_dir: &Path,
    api_key: &str,
    request: PendingWagoInstallRequest,
    replace_folders: Option<Vec<String>>,
) -> std::result::Result<WagoInstallTaskOutcome, SourceError> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    if replace_folders.is_none() {
        let inspection = inspect_wago_install_target(
            &database,
            addon_dir,
            &request.target,
            api_key,
            WagoStability::Stable,
        )
        .await?;
        if inspection.requires_confirmation() {
            return Ok(WagoInstallTaskOutcome::NeedsConfirmation {
                request,
                inspection,
            });
        }
    }

    // Only the folders the person said yes to are replaced. A fresh download
    // that brings another folder in the way asks again instead.
    let confirmed = replace_folders.unwrap_or_default();
    let summary = match install_wago_addon_with_replace(
        &mut database,
        addon_dir,
        &request.target,
        api_key,
        WagoStability::Stable,
        false,
        &confirmed,
    )
    .await
    {
        Err(error) if error.kind == SourceErrorKind::FoldersInTheWay => {
            let inspection = inspect_wago_install_target(
                &database,
                addon_dir,
                &request.target,
                api_key,
                WagoStability::Stable,
            )
            .await?;
            return Ok(WagoInstallTaskOutcome::NeedsConfirmation {
                request,
                inspection,
            });
        }
        result => result?,
    };
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;
    Ok(WagoInstallTaskOutcome::Installed(WagoInstallOutcome {
        summary,
        sync,
    }))
}

async fn run_transfer_import_task(
    state_db_file: &Path,
    addon_dir: &Path,
    input_path: &Path,
    wago_api_key: Option<String>,
) -> std::result::Result<TransferImportOutcome, String> {
    let transfer = parse_import_file(input_path).map_err(|error| error.to_string())?;
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let tracked = database.list_addons().map_err(|error| error.to_string())?;
    let existing_folders = collect_existing_import_folders(Some(addon_dir), &tracked)?;
    let analysis = analyze_import(&transfer, &existing_folders);
    let install_summary =
        install_transfer_addons(&mut database, addon_dir, &analysis, wago_api_key.as_deref()).await;
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;

    Ok(TransferImportOutcome {
        analysis,
        install_summary,
        sync,
    })
}

async fn run_dashboard_update_task(
    state_db_file: &Path,
    addon_dir: &Path,
    folders: &[String],
    wago_api_key: Option<String>,
    check_interval_secs: u64,
    backup_settings: PreUpdateBackupSettings,
    mut on_progress: impl FnMut(DashboardJobKind, usize, usize, String),
) -> std::result::Result<DashboardUpdateOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    preflight_stale_update_checks(
        &mut database,
        Some(addon_dir),
        folders,
        wago_api_key.as_deref(),
        check_interval_secs,
        &mut on_progress,
    )
    .await?;
    // Best-effort WTF backup before addons change; a failure only adds a status note.
    let backup_addon_dir = addon_dir.to_path_buf();
    let backup_note = tokio::task::spawn_blocking(move || {
        run_pre_update_backup(
            &backup_settings,
            &backup_addon_dir,
            OffsetDateTime::now_utc(),
        )
        .status_note()
    })
    .await
    .unwrap_or_else(|join_error| {
        Some(format!(
            "couldn't back up your game settings ({join_error}), the update went ahead"
        ))
    });
    let run = apply_live_updates(
        &mut database,
        addon_dir,
        folders,
        wago_api_key.as_deref(),
        false,
        false,
        |current_index, total, addon_name| {
            on_progress(DashboardJobKind::Update, current_index, total, addon_name)
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let updated_addon_name = if run.summary.updated_addons == 1 {
        run.results
            .iter()
            .find(|result| result.status == LiveUpdateStatus::Updated)
            .map(|result| result.addon_name.clone())
    } else {
        None
    };
    let mut warnings: Vec<String> = run
        .results
        .iter()
        .filter(|result| {
            result.status == LiveUpdateStatus::Updated && result.source == SourceKind::WowInterface
        })
        .filter_map(|result| result.message.clone())
        .collect();
    warnings.extend(backup_note);
    let summary = run.summary;
    let folders_with = |status: LiveUpdateStatus| {
        run.results
            .iter()
            .filter(|result| result.status == status)
            .map(|result| result.addon_name.clone())
            .collect::<Vec<_>>()
    };
    let updated_folders = folders_with(LiveUpdateStatus::Updated);
    let failed_folders = folders_with(LiveUpdateStatus::Error);
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;

    Ok(DashboardUpdateOutcome {
        summary,
        updated_folders,
        failed_folders,
        updated_addon_name,
        warnings,
        sync,
    })
}

fn transfer_import_status_message(
    analysis: &ImportAnalysis,
    install_summary: &TransferInstallSummary,
) -> String {
    let mut segments = Vec::new();
    if install_summary.installed > 0 {
        segments.push(format!(
            "imported {} addon{}",
            install_summary.installed,
            plural_suffix(install_summary.installed)
        ));
    } else {
        segments.push("no addons imported".to_string());
    }
    if !analysis.already_installed.is_empty() {
        segments.push(format!(
            "{} already installed",
            analysis.already_installed.len()
        ));
    }
    if !analysis.manual_addons.is_empty() {
        segments.push(format!(
            "{} not linked, skipped",
            analysis.manual_addons.len()
        ));
    }
    if !analysis.duplicate_addons.is_empty() {
        segments.push(format!(
            "{} duplicate skipped",
            analysis.duplicate_addons.len()
        ));
    }
    if !analysis.unsupported_addons.is_empty() {
        segments.push(format!(
            "{} unsupported skipped",
            analysis.unsupported_addons.len()
        ));
    }
    if !install_summary.errors.is_empty() {
        segments.push(format!("{} failed", install_summary.errors.len()));
    }
    segments.extend(install_summary.warnings.iter().cloned());
    segments.join(" · ")
}

async fn run_dashboard_check_task(
    state_db_file: &Path,
    addon_dir: Option<&Path>,
    folders: &[String],
    wago_api_key: Option<String>,
    check_interval_secs: u64,
    mut on_progress: impl FnMut(DashboardJobKind, usize, usize, String),
) -> std::result::Result<DashboardCheckOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let installed = database.list_addons().map_err(|error| error.to_string())?;
    let stale = stale_target_selectors(&installed, folders, check_interval_secs);

    for (index, folder) in stale.iter().enumerate() {
        on_progress(
            DashboardJobKind::Check,
            index + 1,
            stale.len(),
            folder.clone(),
        );
        refresh_live_update_checks(
            &mut database,
            std::slice::from_ref(folder),
            wago_api_key.as_deref(),
            addon_dir,
        )
        .await
        .map_err(|error| error.to_string())?;
    }

    let refreshed = database.list_addons().map_err(|error| error.to_string())?;
    let results = build_update_checks(&refreshed, folders).map_err(|error| error.to_string())?;
    let errors = results
        .iter()
        .filter(|result| result.status == UpdateStatus::Error)
        .count();

    Ok(DashboardCheckOutcome {
        addons: refreshed,
        total: folders.len(),
        targets: folders.to_vec(),
        live_checked: stale.len(),
        cached: folders.len().saturating_sub(stale.len()),
        errors,
    })
}

async fn preflight_stale_update_checks(
    database: &mut StateDatabase,
    addon_dir: Option<&Path>,
    folders: &[String],
    wago_api_key: Option<&str>,
    check_interval_secs: u64,
    mut on_progress: impl FnMut(DashboardJobKind, usize, usize, String),
) -> std::result::Result<(), String> {
    let installed = database.list_addons().map_err(|error| error.to_string())?;
    let stale = stale_target_selectors(&installed, folders, check_interval_secs);

    for (index, folder) in stale.iter().enumerate() {
        on_progress(
            DashboardJobKind::Check,
            index + 1,
            stale.len(),
            folder.clone(),
        );
        refresh_live_update_checks(
            database,
            std::slice::from_ref(folder),
            wago_api_key,
            addon_dir,
        )
        .await
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn stale_target_selectors(
    installed: &[AddonRecord],
    folders: &[String],
    check_interval_secs: u64,
) -> Vec<String> {
    folders
        .iter()
        .filter_map(|folder| {
            let addon = installed.iter().find(|addon| addon.folder == *folder)?;
            if !is_remote_check_refreshable(addon.source)
                || !is_check_stale(addon, check_interval_secs)
            {
                return None;
            }
            Some(folder.clone())
        })
        .collect()
}

fn is_remote_check_refreshable(source: SourceKind) -> bool {
    !matches!(source, SourceKind::Manual)
}

fn is_check_stale(addon: &AddonRecord, check_interval_secs: u64) -> bool {
    let Some(last_checked_at) = addon.last_checked_at else {
        return true;
    };
    let interval = Duration::seconds(check_interval_secs.min(i64::MAX as u64) as i64);
    OffsetDateTime::now_utc() - last_checked_at >= interval
}

/// Toast text for what a scan could not read or left behind, if anything.
fn scan_warning_text(summary: &ScanSummary) -> Option<String> {
    let mut parts = Vec::new();
    if !summary.skipped_folders.is_empty() {
        let count = summary.skipped_folders.len();
        parts.push(format!(
            "{count} addon folder{} could not be read; see log",
            plural_suffix(count)
        ));
    }
    if !summary.leftover_staging.is_empty() {
        parts.push(format!(
            "leftover update staging folder {} may contain previous addon versions; see Troubleshooting",
            summary.leftover_staging.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn sync_dashboard_state(
    state_db_file: &Path,
    addon_dir: &Path,
) -> std::result::Result<AddonScanOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let tracked = database.list_addons().map_err(|error| error.to_string())?;
    let report = scan_addons_dir_report(addon_dir, GameFlavor::for_addons_dir(addon_dir), &tracked)
        .map_err(|error| error.to_string())?;
    let drift_report = compute_drift_report(&tracked, &report.addons);
    let summary = database
        .reconcile_scan(&report)
        .map_err(|error| error.to_string())?;
    let addons = database.list_addons().map_err(|error| error.to_string())?;

    Ok(AddonScanOutcome {
        path: addon_dir.to_path_buf(),
        summary,
        addons,
        drift_report,
    })
}

fn build_update_checks_for_dashboard(items: &[&DashboardItem]) -> Vec<CheckResult> {
    items
        .iter()
        .map(|item| {
            let addon = dashboard_item_to_addon_record(item);
            let (status, message) = crate::update::determine_update_status(&addon);
            CheckResult {
                addon_name: addon.folder,
                status,
                remote_version: addon.remote_version,
                message,
            }
        })
        .collect()
}

fn dashboard_item_to_addon_record(item: &DashboardItem) -> AddonRecord {
    let mut addon = AddonRecord::new(&item.name, &item.folder, item.source);
    addon.version = item.version.clone();
    addon.remote_version = item.remote_version.clone();
    addon.git_commit = item.git_commit.clone();
    addon.last_checked_at = item.last_checked_at;
    addon
}

/// Tracked rows can be updated whatever their ownership (the provider takes the folder set from
/// the package); rows whose remote version is older than the installed one are held back.
fn is_refreshable_dashboard_item(item: &DashboardItem) -> bool {
    item.source != SourceKind::Manual
        && dashboard_item_update_status(item) != UpdateStatus::RemoteOlder
}

fn summarize_refreshability(items: &[&DashboardItem]) -> DashboardRefreshabilitySummary {
    let mut summary = DashboardRefreshabilitySummary {
        total: items.len(),
        refreshable: 0,
        manual: 0,
        remote_older: 0,
    };

    for item in items {
        if item.source == SourceKind::Manual {
            summary.manual += 1;
        } else if is_refreshable_dashboard_item(item) {
            summary.refreshable += 1;
        } else {
            summary.remote_older += 1;
        }
    }

    summary
}

fn summarize_checks(checks: &[CheckResult]) -> DashboardUpdateRunSummary {
    let up_to_date = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpToDate)
        .count();
    let update_available = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::UpdateAvailable)
        .count();
    let unknown = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Unknown)
        .count();
    let errors = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::Error)
        .count();
    let remote_older = checks
        .iter()
        .filter(|check| check.status == UpdateStatus::RemoteOlder)
        .count();

    DashboardUpdateRunSummary {
        targets: checks.len(),
        updated_addons: 0,
        up_to_date,
        update_available,
        unknown,
        errors,
        skipped_manual: 0,
        skipped_no_api_key: 0,
        remote_older,
        scanned_addons: 0,
    }
}

fn dashboard_update_run_summary_from_live(summary: LiveUpdateSummary) -> DashboardUpdateRunSummary {
    DashboardUpdateRunSummary {
        targets: summary.target_addons,
        updated_addons: summary.updated_addons,
        up_to_date: summary.up_to_date,
        update_available: 0,
        unknown: 0,
        errors: summary.errors,
        skipped_manual: summary.skipped_manual,
        skipped_no_api_key: summary.skipped_no_api_key,
        remote_older: summary.remote_older,
        scanned_addons: 0,
    }
}

/// The message an update run ends with. A failure names the addons that
/// failed, `failed_names` in list order.
fn dashboard_update_status_message(
    outcome: &DashboardUpdateOutcome,
    failed_names: &[String],
) -> String {
    let summary = outcome.summary;
    let mut parts = Vec::new();
    if summary.updated_addons == 1 {
        parts.push(match outcome.updated_addon_name.as_deref() {
            Some(addon_name) => format!("Updated {addon_name}"),
            None => "Updated 1 addon".to_string(),
        });
    } else if summary.updated_addons > 0 {
        parts.push(format!(
            "Updated {}",
            addon_count_label(summary.updated_addons)
        ));
    } else if summary.errors > 0 {
        parts.push(format!(
            "Couldn't update {}",
            name_list(failed_names, summary.errors)
        ));
    } else if summary.up_to_date > 0
        && summary.skipped_manual == 0
        && summary.skipped_no_api_key == 0
        && summary.remote_older == 0
    {
        parts.push("Everything is up to date".to_string());
    } else {
        parts.push("Nothing was updated".to_string());
    }

    if summary.updated_addons > 0 && summary.errors > 0 {
        parts.push(format!(
            "{} failed: {}",
            summary.errors,
            name_list(failed_names, summary.errors)
        ));
    }
    if summary.up_to_date > 0 && (summary.updated_addons > 0 || summary.errors > 0) {
        parts.push(format!("{} already up to date", summary.up_to_date));
    }
    if summary.skipped_manual > 0 {
        parts.push(format!("{} not linked", summary.skipped_manual));
    }
    if summary.remote_older > 0 {
        parts.push(format!("{} newer than source", summary.remote_older));
    }
    if summary.skipped_no_api_key > 0 {
        parts.push(format!(
            "skipped {} Wago.io addon{}: add your Wago.io key in Settings (,) to update {}",
            summary.skipped_no_api_key,
            plural_suffix(summary.skipped_no_api_key),
            if summary.skipped_no_api_key == 1 {
                "it"
            } else {
                "them"
            }
        ));
    }
    parts.extend(outcome.warnings.iter().cloned());
    parts.join(" · ")
}

/// "Pawn", "Pawn and Hekili", "Pawn, Hekili and 3 more". `count` is how
/// many there are, which can be more than the names known.
fn name_list(names: &[String], count: usize) -> String {
    const SHOWN: usize = 3;
    let count = count.max(names.len());
    if names.is_empty() {
        return addon_count_label(count);
    }
    let shown: Vec<&str> = names.iter().take(SHOWN).map(String::as_str).collect();
    let rest = count - shown.len();
    match (shown.as_slice(), rest) {
        ([only], 0) => (*only).to_string(),
        (_, 0) => {
            let (last, first) = shown.split_last().expect("two or more names");
            format!("{} and {last}", first.join(", "))
        }
        _ => format!("{} and {rest} more", shown.join(", ")),
    }
}

/// The message a check ends with. `names` are the checked addons' names.
fn dashboard_check_status_message(
    names: &[String],
    total: usize,
    live_checked: usize,
    cached: usize,
    errors: usize,
) -> String {
    let checked = live_checked + cached;
    if checked == 0 && errors == 0 {
        return "Nothing to check: none of these addons are linked".to_string();
    }
    let headline = if checked == 0 {
        format!("Couldn't check {}", name_list(names, errors.max(total)))
    } else if total == 1 && errors == 0 {
        format!("Checked {}", name_list(names, 1))
    } else {
        format!("Checked {}", addon_count_label(checked))
    };
    if errors > 0 && checked > 0 {
        format!("{headline} · couldn't check {errors}")
    } else {
        headline
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

/// "Sorted by name, A to Z".
fn sort_status_text(column: DashboardSortColumn, direction: DashboardSortDirection) -> String {
    let order = match (column, direction) {
        (DashboardSortColumn::Status, DashboardSortDirection::Asc) => "",
        (DashboardSortColumn::Status, DashboardSortDirection::Desc) => ", reversed",
        (_, DashboardSortDirection::Asc) => ", A to Z",
        (_, DashboardSortDirection::Desc) => ", Z to A",
    };
    format!("Sorted by {}{order}", column.label().to_ascii_lowercase())
}

fn addon_count_label(count: usize) -> String {
    if count == 1 {
        "1 addon".to_string()
    } else {
        format!("{count} addons")
    }
}

fn compare_text(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn shorten_commit(value: &str) -> String {
    value.chars().take(7).collect()
}

fn looks_like_commit_hash(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 7
        && trimmed.len() <= 40
        && trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn is_placeholder_version(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('@') && trimmed.ends_with('@')
}

fn sort_search_results(results: &mut [WagoSearchResult]) {
    results.sort_by(|left, right| {
        right
            .download_count
            .unwrap_or(0)
            .cmp(&left.download_count.unwrap_or(0))
            .then_with(|| {
                left.display_name
                    .to_ascii_lowercase()
                    .cmp(&right.display_name.to_ascii_lowercase())
            })
            .then_with(|| {
                left.owner
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .cmp(&right.owner.as_deref().unwrap_or("").to_ascii_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn format_compact_count(count: u64, divisor: u64, suffix: &str) -> String {
    let scaled_tenths = count.saturating_mul(10) / divisor;
    let whole = scaled_tenths / 10;
    let tenth = scaled_tenths % 10;

    if tenth == 0 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{tenth}{suffix}")
    }
}

fn wago_result_slug(result: &WagoSearchResult) -> Option<String> {
    result
        .website_url
        .as_deref()
        .and_then(extract_wago_slug)
        .map(str::to_string)
        .or_else(|| {
            if result.id.trim().is_empty() {
                None
            } else {
                Some(result.id.clone())
            }
        })
}

fn extract_wago_slug(url: &str) -> Option<&str> {
    let path = url.split_once("/addons/")?.1;
    let slug = path.split(['/', '?', '#']).next()?.trim();
    if slug.is_empty() { None } else { Some(slug) }
}

fn interpolate_palette(colors: &[Color], t: f32) -> Color {
    if colors.is_empty() {
        return Color::Reset;
    }
    if colors.len() == 1 {
        return colors[0];
    }

    let clamped = t.clamp(0.0, 1.0);
    let scaled = clamped * (colors.len() - 1) as f32;
    let left_index = scaled.floor() as usize;
    let right_index = scaled.ceil() as usize;

    if left_index == right_index {
        return colors[left_index];
    }

    let local_t = scaled - left_index as f32;
    lerp_color(colors[left_index], colors[right_index], local_t)
}

fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => Color::Rgb(
            lerp_channel(r1, r2, t),
            lerp_channel(g1, g2, t),
            lerp_channel(b1, b2, t),
        ),
        _ => to,
    }
}

fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    let start = from as f32;
    let end = to as f32;
    (start + ((end - start) * t)).round().clamp(0.0, 255.0) as u8
}

fn detail_mode_label(detail_mode: DetailMode) -> &'static str {
    match detail_mode {
        DetailMode::Overview => "Overview",
        DetailMode::Install => "Install",
        DetailMode::Search => "Install",
        DetailMode::Update => "Update",
        DetailMode::Config => "Settings",
        DetailMode::Backup => "Backups",
    }
}

fn source_label(source: SourceKind) -> &'static str {
    match source {
        SourceKind::GitHub => "GitHub",
        SourceKind::Tukui => "TukUI",
        SourceKind::WowInterface => "WoWInterface",
        SourceKind::Wago => "Wago",
        SourceKind::Manual => "Manual",
    }
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn selection_count_after_toggle(dashboard: &DashboardState) -> usize {
    let Some(parent_folder) = dashboard.selected_parent_folder() else {
        return dashboard.selected_parent_count();
    };

    if dashboard.is_parent_selected(parent_folder) {
        dashboard.selected_parent_count().saturating_sub(1)
    } else {
        dashboard.selected_parent_count() + 1
    }
}

fn adjust_u64(current: u64, delta: i16, step: u64, minimum: u64) -> u64 {
    if delta >= 0 {
        current
            .saturating_add(step.saturating_mul(delta as u64))
            .max(minimum)
    } else {
        current
            .saturating_sub(step.saturating_mul((-delta) as u64))
            .max(minimum)
    }
}

fn adjust_u16(current: u16, delta: i16, step: u16, minimum: u16) -> u16 {
    if delta >= 0 {
        current
            .saturating_add(step.saturating_mul(delta as u16))
            .max(minimum)
    } else {
        current
            .saturating_sub(step.saturating_mul((-delta) as u16))
            .max(minimum)
    }
}

/// The sheet scroll a key asks for. `arrows` includes the arrow keys and
/// `j` `k`, for sheets where they do not move a cursor.
fn sheet_move_for_key(code: KeyCode, arrows: bool) -> Option<SheetMove> {
    match code {
        KeyCode::PageUp => Some(SheetMove::PageUp),
        KeyCode::PageDown => Some(SheetMove::PageDown),
        KeyCode::Home => Some(SheetMove::Top),
        KeyCode::End => Some(SheetMove::Bottom),
        KeyCode::Up | KeyCode::Char('k') if arrows => Some(SheetMove::LineUp),
        KeyCode::Down | KeyCode::Char('j') if arrows => Some(SheetMove::LineDown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use lemonup_core::ThemeMode;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::widgets::TableState;
    use tempfile::tempdir;
    use time::{Duration, OffsetDateTime};
    use tokio::sync::mpsc;

    use super::{
        AddonScanOutcome, App, AppMessage, AppRuntime, AppTaskEvent, BackupPaneState,
        ConfigEditState, ConfigField, ConfigPaneState, DashboardChildConnector,
        DashboardDeleteOutcome, DashboardJobKind, DashboardJobUiState, DashboardRow,
        DashboardSortColumn, DashboardState, DashboardUpdateOutcome, DetailMode, FooterHintId,
        FooterKeyPulse, HelpContext, InstallPaneState, OverlayKind, PendingWagoInstallRequest,
        ScanState, SearchFocus, SearchInstallState, SearchPaneState, SearchPresentationMode,
        ShellMode, ShellUiState, TextBox, UiTheme, WagoInstallConfirmation, WagoInstallOutcome,
        WagoInstallSource, WagoInstallTaskOutcome, WagoSearchOutcome, child_row_detail_prefix,
        child_row_prefix, dashboard_item_version_label, format_download_count,
        summarize_owned_folders, truncate_middle_text, visible_search_result_window,
    };
    use super::{
        DashboardItem, DashboardUndoDeleteState, SheetMove, SheetScroll, StatusKind, StatusMessage,
        StatusOffer,
    };
    use crate::action::AppAction;
    use crate::app::render_install_search::install_error_advice;
    use crate::app::text_input::{TextEdit, TextInput};
    use crate::backup::{BackupEntry, BackupRunOutcome, backup_root};
    use crate::drift::{DriftReport, OwnedChildDrift};
    use crate::event::TerminalEvent;
    use crate::onboarding::{
        FoundAction, FoundState, OnboardingPhase, OnboardingState, OnboardingStep,
    };
    use crate::source_error::{FolderClash, SourceError, SourceErrorKind};
    use crate::update::LiveUpdateSummary;
    use crate::wago::{WagoInstallInspection, WagoInstallSummary, WagoSearchResult};
    use lemonup_core::{
        AddonKind, AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE,
        OwnedFolder, ScanSummary, SourceKind, StateDatabase,
    };
    use ratatui::text::Line;
    use std::time::{Duration as StdDuration, Instant};

    fn apply_all(app: &mut App, message: AppMessage) {
        for action in app.update(message) {
            app.apply(action);
        }
    }

    fn app_for_tests(shell_mode: ShellMode) -> App {
        let (task_events_tx, task_events_rx) = mpsc::unbounded_channel();
        let config = AppConfig::new_unconfigured();
        let base = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("base timestamp");
        let mut first = AddonRecord::new("First", "First", SourceKind::Manual);
        first.kind = AddonKind::Addon;
        first.version = Some("1.0.0".to_string());
        first.installed_at = base;
        first.updated_at = base;

        let mut second = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        second.kind = AddonKind::Addon;
        second.version = Some("2.0.0".to_string());
        second.remote_version = Some("2.1.0".to_string());
        second.author = Some("Author".to_string());
        second.git_commit = Some("abcdef".to_string());
        second.required_deps = vec!["Ace3".to_string()];
        second.embedded_libs = vec!["LibStub".to_string()];
        second.installed_at = base + Duration::days(1);
        second.updated_at = base + Duration::days(1);
        second.set_managed_owned_folders(vec![lemonup_core::OwnedFolder {
            name: "Second_Config".to_string(),
        }]);

        let mut third = AddonRecord::new("Third", "Third", SourceKind::Wago);
        third.kind = AddonKind::Library;
        third.version = Some("3.0.0".to_string());
        third.remote_version = Some("3.0.0".to_string());
        third.interface = Some("110005".to_string());
        third.optional_deps = vec!["Optional".to_string()];
        third.installed_at = base + Duration::days(2);
        third.updated_at = base + Duration::days(2);

        App {
            shell_mode,
            quit_requested: false,
            status_line: StatusMessage::new(
                StatusKind::Quiet,
                format!("profile {} | q quit", DEFAULT_PROFILE),
            ),
            ui_theme: UiTheme::from_mode(config.theme),
            shell_ui: ShellUiState::default(),
            config_present: true,
            config_pane: ConfigPaneState::new({
                let mut config = config.clone();
                config.wago_api_key = Some("test-wago-key".to_string());
                config
            }),
            backup_pane: BackupPaneState::default(),
            wago_api_key: Some("test-wago-key".to_string()),
            config_store: ConfigStore::new(std::env::temp_dir().join("lemonup-test-config.toml")),
            state_db_file: std::env::temp_dir().join("lemonup-test-state.sqlite"),
            trash_dir: std::env::temp_dir().join("lemonup-test-trash"),
            data_dir: std::env::temp_dir().join("lemonup-test-data"),
            backup_dir: std::env::temp_dir().join("lemonup-test-backups"),
            runtime: AppRuntime::new(DEFAULT_PROFILE.to_string(), None, Vec::new()),
            effective_addon_dir: None,
            scan_state: ScanState::Idle,
            dashboard: DashboardState::from_addons(vec![first, second, third]),
            install_pane: InstallPaneState::default(),
            search_pane: SearchPaneState::default(),
            pending_wago_install_confirmation: None,
            wago_install_in_progress: false,
            direct_install_in_progress: false,
            auto_check_in_progress: false,
            last_auto_check_started_at: None,
            undo_delete: None,
            undo_delete_in_progress: false,
            delete_in_progress: false,
            key_read_at: None,
            question_shown: None,
            last_dashboard_list_area: None,
            last_frame_width: None,
            sheet_scroll: std::cell::Cell::new(SheetScroll::default()),
            onboarding: OnboardingState::new(),
            task_events_tx,
            task_events_rx,
            active_onboarding_cancel: None,
            startup_notice: None,
            quit_question_open: false,
        }
    }

    #[test]
    fn dashboard_navigation_keys_emit_messages_on_dashboard() {
        let app = app_for_tests(ShellMode::Dashboard);

        let next = app.messages_for_key(KeyEvent::from(KeyCode::Char('j')));
        let previous = app.messages_for_key(KeyEvent::from(KeyCode::Up));

        assert_eq!(next, vec![AppMessage::DashboardSelectionNext]);
        assert_eq!(previous, vec![AppMessage::DashboardSelectionPrevious]);
    }

    #[test]
    fn dashboard_sort_keys_emit_sort_messages() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('1'))),
            vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Name)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('2'))),
            vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Status)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('3'))),
            vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Author)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('4'))),
            vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Source)]
        );
    }

    #[test]
    fn dashboard_sort_toggle_reverses_and_preserves_selection() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let third_index = app
            .dashboard
            .rows
            .iter()
            .position(|row| row.folder == "Third")
            .expect("third row");
        app.dashboard.list_state.select(Some(third_index));
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Third")
        );

        for action in app.update(AppMessage::DashboardToggleSort(DashboardSortColumn::Name)) {
            app.apply(action);
        }
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Third")
        );
        assert_eq!(
            app.dashboard.rows.first().map(|row| row.folder.as_str()),
            Some("First")
        );

        for action in app.update(AppMessage::DashboardToggleSort(DashboardSortColumn::Name)) {
            app.apply(action);
        }
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Third")
        );
        assert_eq!(
            app.dashboard.rows.first().map(|row| row.folder.as_str()),
            Some("Third")
        );
    }

    #[test]
    fn dashboard_defaults_to_version_sort_with_update_available_first() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.dashboard.sort_config().column,
            DashboardSortColumn::Status
        );
        assert_eq!(
            app.dashboard.rows.first().map(|row| row.folder.as_str()),
            Some("Third")
        );
        assert_eq!(
            app.dashboard.rows.get(1).map(|row| row.folder.as_str()),
            Some("Second")
        );
    }

    #[test]
    fn mouse_wheel_moves_dashboard_selection_one_row() {
        let app = app_for_tests(ShellMode::Dashboard);

        let down = app.messages_for_event(&TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        let up = app.messages_for_event(&TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));

        assert_eq!(down, vec![AppMessage::DashboardSelectionNext]);
        assert_eq!(up, vec![AppMessage::DashboardSelectionPrevious]);
    }

    #[test]
    fn left_click_emits_pointer_select_message() {
        let app = app_for_tests(ShellMode::Dashboard);

        let messages = app.messages_for_event(&TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 4,
            modifiers: KeyModifiers::NONE,
        }));

        assert_eq!(
            messages,
            vec![AppMessage::DashboardPointerSelect { column: 5, row: 4 }]
        );
    }

    #[test]
    fn left_drag_emits_pointer_select_message() {
        let app = app_for_tests(ShellMode::Dashboard);

        let messages = app.messages_for_event(&TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }));

        assert_eq!(
            messages,
            vec![AppMessage::DashboardPointerSelect { column: 5, row: 5 }]
        );
    }

    #[test]
    fn pointer_selection_maps_click_inside_list_to_visible_row() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.last_dashboard_list_area = Some(Rect::new(0, 0, 20, 10));
        app.dashboard.list_state = TableState::default().with_offset(1).with_selected(Some(1));

        let actions = app.update(AppMessage::DashboardPointerSelect { column: 2, row: 1 });

        assert_eq!(
            actions,
            vec![
                AppAction::SetDashboardSelection(Some(2)),
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Quiet,
                    app.dashboard.detail_mode,
                    "selection moved",
                )),
            ]
        );
    }

    #[test]
    fn pointer_selection_ignores_clicks_outside_list_inner_area() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.last_dashboard_list_area = Some(Rect::new(0, 0, 20, 10));

        let actions = app.update(AppMessage::DashboardPointerSelect { column: 25, row: 2 });

        assert!(actions.is_empty());
    }

    #[test]
    fn pointer_selection_is_noop_when_clicking_already_selected_row() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.last_dashboard_list_area = Some(Rect::new(0, 0, 20, 10));
        app.dashboard.list_state = TableState::default().with_selected(Some(2));

        let actions = app.update(AppMessage::DashboardPointerSelect { column: 2, row: 2 });

        assert!(actions.is_empty());
    }

    #[test]
    fn detail_mode_keys_emit_messages_on_dashboard() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('b'))),
            vec![AppMessage::SetDetailMode(DetailMode::Backup)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('c'))),
            vec![AppMessage::DashboardRunCheckSelected]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('u'))),
            vec![AppMessage::DashboardRunUpdateSelected]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(','))),
            vec![AppMessage::SetDetailMode(DetailMode::Config)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
            vec![AppMessage::DashboardOpenInspect]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Right)),
            vec![AppMessage::DashboardToggleExpanded]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(' '))),
            vec![AppMessage::DashboardToggleSelected]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('a'))),
            vec![AppMessage::DashboardSelectAll]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardClearSelection]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('x'))),
            vec![AppMessage::DashboardRequestDelete]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('z'))),
            vec![AppMessage::DashboardUndoDelete]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(']'))),
            vec![AppMessage::DashboardExpandAllRelationships]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('['))),
            vec![AppMessage::DashboardCollapseAllRelationships]
        );
        // The old mode switches are gone, including in the retired Update mode.
        let mut update_mode = app_for_tests(ShellMode::Dashboard);
        update_mode.dashboard.detail_mode = DetailMode::Update;
        for retired in ['o', 'r', 'v'] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char(retired))),
                vec![],
                "{retired}"
            );
            assert_eq!(
                update_mode.messages_for_key(KeyEvent::from(KeyCode::Char(retired))),
                vec![],
                "{retired} in Update mode"
            );
        }
    }

    #[test]
    fn dashboard_a_key_clears_selection_when_any_parent_is_selected() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.selected_parents.insert("BigWigs".to_string());

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('a'))),
            vec![AppMessage::DashboardClearSelection]
        );
    }

    #[test]
    fn footer_status_text_hides_redundant_resize_noise() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.status_line = StatusMessage::new(
            StatusKind::Quiet,
            "terminal resized to 132x45 | profile manual-smoke",
        );

        assert_eq!(app.footer_status_text(), "");
    }

    #[test]
    fn dashboard_event_line_stays_quiet_for_selection_only() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::ToggleDashboardSelection);

        assert!(app.status_line_message().is_none());
    }

    #[test]
    fn dashboard_event_line_uses_humanized_status_summary() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(StatusMessage::new(
            StatusKind::Done,
            "scan complete | 6 addons synced, 0 removed",
        )));

        let (text, kind) = app.status_line_message().expect("event line");

        assert_eq!(text, "scan complete · 6 addons synced, 0 removed");
        assert_eq!(kind, StatusKind::Done);
    }

    #[test]
    fn dashboard_toast_persists_across_non_toast_status_updates() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Done,
            DetailMode::Overview,
            "Updated BigWigs",
        )));
        let initial = app.status_line_message().expect("toast");

        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Quiet,
            DetailMode::Overview,
            "selection moved",
        )));

        assert_eq!(app.status_line_message(), Some(initial));
    }

    #[test]
    fn scan_with_unreadable_or_leftover_folders_shows_a_warning_toast() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let path = PathBuf::from("C:\\Sandbox\\AddOns");

        app.apply(AppAction::CompleteAddonScan {
            path: path.clone(),
            summary: ScanSummary::default(),
        });
        assert!(app.status_line_message().is_none(), "clean scan: no toast");

        app.apply(AppAction::CompleteAddonScan {
            path: path.clone(),
            summary: ScanSummary {
                skipped_folders: vec!["Locked".to_string(), "Pawn".to_string()],
                ..Default::default()
            },
        });
        let (message, _) = app.status_line_message().expect("skipped toast");
        assert!(
            message.contains("2 addon folders could not be read; see log"),
            "{message}"
        );

        app.apply(AppAction::CompleteAddonScan {
            path,
            summary: ScanSummary {
                leftover_staging: vec![".lemonup-staging-1-2-0".to_string()],
                ..Default::default()
            },
        });
        let (message, _) = app.status_line_message().expect("staging toast");
        assert!(message.contains(".lemonup-staging-1-2-0"), "{message}");
        assert!(message.contains("Troubleshooting"), "{message}");
    }

    #[test]
    fn dashboard_toast_expires_after_duration() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Done,
            DetailMode::Overview,
            "Updated BigWigs",
        )));
        assert!(app.status_line_message().is_some());

        if let Some(flash) = app.shell_ui.status_flash.as_mut() {
            flash.expires_at = Some(super::Instant::now() - super::StdDuration::from_millis(1));
        }
        app.apply(AppAction::AdvanceMotionTick);

        assert!(app.status_line_message().is_none());
    }

    #[test]
    fn key_line_keeps_to_one_row_on_narrow_windows() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "1.0", Some("1.1"))]);

        let wide = line_text(&app.dashboard_key_line(120));
        let narrow = line_text(&app.dashboard_key_line(80));
        assert_eq!(
            wide,
            " U update all (1)   u update   c check   enter details   space select   / filter   i install   ? help   C check all"
        );
        assert_eq!(
            narrow,
            " U update all (1)  u update  enter details  i install  / filter  ? help"
        );
        // Hints that do not fit are dropped from the end, never wrapped.
        let tiny = line_text(&app.dashboard_key_line(30));
        assert!(tiny.chars().count() <= 30, "{tiny}");
        assert!(tiny.starts_with(" U update all (1)"), "{tiny}");

        // Nothing to update: C comes first.
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "1.0", Some("1.0"))]);
        assert!(line_text(&app.dashboard_key_line(80)).starts_with(" C check all"));
    }

    #[test]
    fn recognized_key_sets_matching_footer_pulse() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "1.0", Some("1.1"))]);
        for (key, hint) in [
            (KeyEvent::from(KeyCode::Char('c')), FooterHintId::Check),
            (KeyEvent::from(KeyCode::Char('u')), FooterHintId::Update),
            (
                KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT),
                FooterHintId::UpdateAll,
            ),
            (
                KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT),
                FooterHintId::CheckAll,
            ),
            (KeyEvent::from(KeyCode::Char('/')), FooterHintId::Filter),
        ] {
            let event = TerminalEvent::Key(key);
            let messages = app.messages_for_event(&event);

            let pulse = app.footer_pulse_for_event(&event, &messages);

            assert_eq!(
                pulse,
                Some(FooterKeyPulse {
                    hint,
                    expires_at_tick: app.shell_ui.motion.tick_count + 2,
                })
            );
            // The pulsed key is one the key line shows.
            assert!(
                app.key_table(120)
                    .iter()
                    .any(|shown| shown.id == Some(hint)),
                "{hint:?} is not on the key line"
            );
        }

        app.shell_ui.footer_pulse = Some(FooterKeyPulse {
            hint: FooterHintId::UpdateAll,
            expires_at_tick: app.shell_ui.motion.tick_count + 2,
        });
        let line = app.dashboard_key_line(120);
        let pulsed = line
            .spans
            .iter()
            .find(|span| span.content == "U")
            .expect("U key");
        assert_eq!(pulsed.style.bg, Some(app.ui_theme.roles.lemon));
    }

    #[test]
    fn footer_pulse_expires_after_motion_ticks() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetFooterKeyPulse(Some(FooterKeyPulse {
            hint: FooterHintId::Check,
            expires_at_tick: 2,
        })));

        app.apply(AppAction::AdvanceMotionTick);
        assert!(app.shell_ui.footer_pulse.is_some());

        app.apply(AppAction::AdvanceMotionTick);
        assert!(app.shell_ui.footer_pulse.is_none());
    }

    #[test]
    fn inspect_overlay_closes_on_escape_and_q() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCloseOverlay]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::DashboardCloseOverlay]
        );
    }

    #[test]
    fn inspect_overlay_keys_include_check_action() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('c'))),
            vec![AppMessage::InspectRunCheck]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            vec![AppMessage::QuitRequested]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            vec![]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::DashboardCloseOverlay]
        );
    }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    fn alt(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::ALT)
    }

    fn pending_wago_confirmation() -> WagoInstallConfirmation {
        WagoInstallConfirmation {
            request: PendingWagoInstallRequest {
                target: "VBNBxKx5".to_string(),
                source: WagoInstallSource::SearchResult,
            },
            inspection: WagoInstallInspection {
                addon_id: "VBNBxKx5".to_string(),
                addon_name: "WeakAuras".to_string(),
                parent_folder: "WeakAuras".to_string(),
                installed_folders: vec!["WeakAuras".to_string()],
                stability: crate::wago::WagoStability::Stable,
                version: Some("5.21.1".to_string()),
                existing_folders: vec!["WeakAuras".to_string()],
                tracked_parent: Some("WeakAuras".to_string()),
            },
        }
    }

    fn sample_backup() -> BackupEntry {
        BackupEntry {
            file_name: "WTF-20260405T120000.000Z.zip".to_string(),
            path: PathBuf::from("C:\\Temp\\WTF-20260405T120000.000Z.zip"),
            label: "2026-04-05 12:00:00.000Z".to_string(),
            size_bytes: 128,
        }
    }

    fn open_overlay(app: &mut App, mode: DetailMode, overlay: OverlayKind) {
        app.dashboard.detail_mode = mode;
        app.shell_ui.overlay.active = Some(overlay);
    }

    #[test]
    fn ctrl_c_quits_from_every_screen_and_state() {
        let quit = vec![AppMessage::QuitRequested];

        let app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "addon list");

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Help);
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "help overlay");

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Config, OverlayKind::Config);
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "config overlay");

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Search, OverlayKind::Search);
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "install text box");

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Config, OverlayKind::Config);
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: TextInput::default(),
        });
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "config text box");

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDashboardFilter {
            query: "sec".into(),
            editing: true,
        });
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "filter box");

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard
            .set_pending_delete_folders(Some(vec!["Second".to_string()]));
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "delete question");

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Backup, OverlayKind::Backup);
        app.backup_pane.pending_restore = Some(sample_backup());
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "restore question");

        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Theme;
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "setup");

        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.is_editing = true;
        assert_eq!(app.messages_for_key(ctrl('c')), quit, "setup text box");
    }

    #[test]
    fn ctrl_and_alt_letters_do_nothing_on_the_list() {
        let app = app_for_tests(ShellMode::Dashboard);

        for key in [
            ctrl('x'),
            ctrl('u'),
            alt('x'),
            alt('u'),
            alt('q'),
            ctrl('q'),
        ] {
            assert_eq!(app.messages_for_key(key), vec![], "{key:?}");
            assert_eq!(app.footer_hint_for_key(key), None, "{key:?}");
        }
        assert_eq!(
            app.messages_for_key(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            vec![]
        );
    }

    #[test]
    fn ctrl_u_clears_the_install_box_instead_of_typing() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Search, OverlayKind::Search);

        assert_eq!(
            app.messages_for_key(ctrl('u')),
            vec![AppMessage::TextBoxEdit(TextEdit::Clear)]
        );
        assert_eq!(app.messages_for_key(alt('u')), vec![]);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('u'))),
            vec![AppMessage::SearchInputChar('u')]
        );
        // AltGr arrives as Ctrl+Alt on Windows and still types its character.
        assert_eq!(
            app.messages_for_key(KeyEvent::new(
                KeyCode::Char('@'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            vec![AppMessage::SearchInputChar('@')]
        );

        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.is_editing = true;
        assert_eq!(
            app.messages_for_key(ctrl('u')),
            vec![AppMessage::TextBoxEdit(TextEdit::Clear)]
        );
        // Ctrl+Alt+U is AltGr+U on Windows, not Ctrl+U.
        assert_eq!(
            app.messages_for_key(KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            vec![]
        );
    }

    /// Sends the messages a key asks for through update and apply.
    fn press_text_key(app: &mut App, key: KeyEvent) {
        for message in app.messages_for_key(key) {
            for action in app.update(message) {
                app.apply(action);
            }
        }
    }

    /// Every text box: ← → Home End move the cursor, typing and paste go
    /// in at the cursor, Delete and backspace edit there and Ctrl+U clears.
    fn exercise_text_box(app: &mut App, value: fn(&App) -> String) {
        type_text(app, "aé中d");
        press_text_key(app, KeyEvent::from(KeyCode::Left));
        press_text_key(app, KeyEvent::from(KeyCode::Left));
        type_text(app, "q");
        assert_eq!(value(app), "aéq中d");
        press_text_key(app, KeyEvent::from(KeyCode::Delete));
        assert_eq!(value(app), "aéqd");
        press_text_key(app, KeyEvent::from(KeyCode::Home));
        press_text_key(app, KeyEvent::from(KeyCode::Right));
        press_text_key(app, KeyEvent::from(KeyCode::Backspace));
        assert_eq!(value(app), "éqd");
        for action in app.update(AppMessage::Paste("C:\\Wow\"\r\n".to_string())) {
            app.apply(action);
        }
        assert_eq!(value(app), "C:\\Wow\"éqd");
        press_text_key(app, KeyEvent::from(KeyCode::End));
        type_text(app, "!");
        assert_eq!(value(app), "C:\\Wow\"éqd!");
        press_text_key(app, ctrl('u'));
        assert_eq!(value(app), "");
        type_text(app, "x");
        assert_eq!(value(app), "x");
    }

    #[test]
    fn every_text_box_edits_at_the_cursor() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_text_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
        exercise_text_box(&mut app, |app| app.dashboard.filter_query().to_string());
        assert!(app.dashboard.filter_editing());

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Search, OverlayKind::Search);
        exercise_text_box(&mut app, |app| app.search_pane.query.value().to_string());

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Install, OverlayKind::Install);
        app.install_pane = app.install_pane.begin_editing();
        exercise_text_box(&mut app, |app| app.install_pane.input.value().to_string());

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Config, OverlayKind::Config);
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: TextInput::default(),
        });
        exercise_text_box(&mut app, config_edit_value);

        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.input = String::new();
        app.onboarding = app.onboarding.begin_directory_editing();
        exercise_text_box(&mut app, |app| app.onboarding.input.clone());

        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Wago;
        app.onboarding = app.onboarding.begin_wago_editing();
        exercise_text_box(&mut app, |app| {
            app.onboarding.wago_api_key_value().to_string()
        });
        assert_eq!(app.onboarding.draft.wago_api_key.as_deref(), Some("x"));
        press_text_key(&mut app, ctrl('u'));
        assert_eq!(app.onboarding.draft.wago_api_key, None);
    }

    #[test]
    fn setup_path_box_starts_from_the_path_with_the_cursor_at_the_end() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.phase = OnboardingPhase::Ready;
        app.onboarding.input = "/games/Wow/AddOns".to_string();
        press_text_key(&mut app, KeyEvent::from(KeyCode::Char('e')));
        assert!(app.onboarding.is_editing);
        for _ in 0.."AddOns".len() {
            press_text_key(&mut app, KeyEvent::from(KeyCode::Left));
        }
        press_text_key(&mut app, KeyEvent::from(KeyCode::Backspace));
        type_text(&mut app, "_retail_/Interface/");
        assert_eq!(app.onboarding.input, "/games/Wow_retail_/Interface/AddOns");
        let rows = render_rows(&mut app, 80, 24);
        let editing = rows
            .iter()
            .find(|row| row.contains("Editing:"))
            .expect("editing line");
        assert!(
            editing.contains("/games/Wow_retail_/Interface/AddOns"),
            "{editing}"
        );
    }

    #[test]
    fn home_and_end_go_to_the_focused_box_not_the_sheet_or_list() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Search, OverlayKind::Search);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Home)),
            vec![AppMessage::TextBoxEdit(TextEdit::Home)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::PageDown)),
            vec![AppMessage::SheetScroll(SheetMove::PageDown)]
        );

        let mut app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::End)),
            vec![AppMessage::DashboardSelectionLast]
        );
        press_text_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::End)),
            vec![AppMessage::TextBoxEdit(TextEdit::End)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Down)),
            vec![AppMessage::DashboardSelectionNext]
        );

        // Beside docked Details the filter keeps its box, paste included.
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.last_frame_width = Some(160);
        app.apply(AppAction::SetInspectOverlay(true));
        assert!(app.details_docked());
        press_text_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
        type_text(&mut app, "sd");
        press_text_key(&mut app, KeyEvent::from(KeyCode::Left));
        for action in app.update(AppMessage::Paste("econ".to_string())) {
            app.apply(action);
        }
        assert_eq!(app.dashboard.filter_query(), "second");
    }

    #[test]
    fn q_closes_each_overlay_instead_of_quitting() {
        let close = vec![AppMessage::DashboardCloseOverlay];
        let q = KeyEvent::from(KeyCode::Char('q'));

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Help);
        assert_eq!(app.messages_for_key(q), close, "help");

        // In Install `q` is always text; see `install_sheet_q_is_text`.
        for (mode, overlay) in [
            (DetailMode::Install, OverlayKind::Install),
            (DetailMode::Config, OverlayKind::Config),
            (DetailMode::Backup, OverlayKind::Backup),
        ] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_overlay(&mut app, mode, overlay);
            app.install_pane.is_editing = false;
            assert_eq!(app.messages_for_key(q), close, "{mode:?}");
        }

        // Questions: q answers no.
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard
            .set_pending_delete_folders(Some(vec!["Second".to_string()]));
        assert_eq!(
            app.messages_for_key(q),
            vec![AppMessage::DashboardCancelPendingDelete]
        );

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.pending_wago_install_confirmation = Some(pending_wago_confirmation());
        assert_eq!(app.messages_for_key(q), vec![AppMessage::WagoCancelInstall]);

        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Backup, OverlayKind::Backup);
        app.backup_pane.pending_delete = Some(sample_backup());
        assert_eq!(
            app.messages_for_key(q),
            vec![AppMessage::BackupCancelRestore]
        );

        // In a text box q is a letter.
        let mut app = app_for_tests(ShellMode::Dashboard);
        open_overlay(&mut app, DetailMode::Config, OverlayKind::Config);
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: TextInput::default(),
        });
        assert_eq!(
            app.messages_for_key(q),
            vec![AppMessage::ConfigInputChar('q')]
        );

        // Only the bare list quits.
        let app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(app.messages_for_key(q), vec![AppMessage::QuitRequested]);
    }

    #[test]
    fn dashboard_page_keys_move_by_the_visible_list_height() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::PageDown)),
            vec![AppMessage::DashboardSelectionPageDown]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::PageUp)),
            vec![AppMessage::DashboardSelectionPageUp]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Home)),
            vec![AppMessage::DashboardSelectionFirst]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::End)),
            vec![AppMessage::DashboardSelectionLast]
        );

        let last = app.dashboard.rows.len() - 1;
        assert!(last >= 2, "fixture needs at least three rows");
        let selection_of = |actions: Vec<AppAction>| match actions.first() {
            Some(AppAction::SetDashboardSelection(selection)) => *selection,
            other => panic!("expected a selection, got {other:?}"),
        };

        app.dashboard.list_state.select(Some(0));
        assert_eq!(
            selection_of(app.update(AppMessage::DashboardSelectionLast)),
            Some(last)
        );
        app.dashboard.list_state.select(Some(last));
        assert_eq!(
            selection_of(app.update(AppMessage::DashboardSelectionFirst)),
            Some(0)
        );

        // Ten rows tall: the list area holds rows only, so a page is 10 rows,
        // more than the fixture has, and paging stops at either end.
        app.last_dashboard_list_area = Some(Rect::new(0, 0, 40, 10));
        assert_eq!(app.dashboard_page_size(), 10);
        app.dashboard.list_state.select(Some(0));
        assert_eq!(
            selection_of(app.update(AppMessage::DashboardSelectionPageDown)),
            Some(last)
        );
        assert_eq!(
            selection_of(app.update(AppMessage::DashboardSelectionPageUp)),
            Some(0)
        );

        // A one-row page moves one row at a time.
        app.last_dashboard_list_area = Some(Rect::new(0, 0, 40, 1));
        assert_eq!(app.dashboard_page_size(), 1);
        assert_eq!(
            selection_of(app.update(AppMessage::DashboardSelectionPageDown)),
            Some(1)
        );
        let actions = app.update(AppMessage::DashboardSelectionPageDown);
        assert!(matches!(
            actions.last(),
            Some(AppAction::SetStatus(StatusMessage {
                kind: StatusKind::Quiet,
                ..
            }))
        ));
    }

    #[test]
    fn inspect_page_keys_scroll_within_the_details_body() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::PageDown)),
            vec![AppMessage::SheetScroll(SheetMove::PageDown)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::End)),
            vec![AppMessage::SheetScroll(SheetMove::Bottom)]
        );

        app.sheet_scroll.set(SheetScroll {
            kind: Some(OverlayKind::Inspect),
            offset: 0,
            focus: None,
            max_offset: 12,
            viewport: 5,
        });
        let scroll = |app: &mut App, message: AppMessage| {
            let actions = app.update(message);
            let offset = match actions.as_slice() {
                [AppAction::SetSheetOffset(offset)] => *offset,
                other => panic!("expected a sheet offset, got {other:?}"),
            };
            for action in actions {
                app.apply(action);
            }
            offset
        };

        // A page is the body height less the two arrow lines.
        assert_eq!(
            scroll(&mut app, AppMessage::SheetScroll(SheetMove::PageDown)),
            3
        );
        assert_eq!(
            scroll(&mut app, AppMessage::SheetScroll(SheetMove::Bottom)),
            12
        );
        assert_eq!(
            scroll(&mut app, AppMessage::SheetScroll(SheetMove::LineDown)),
            12
        );
        assert_eq!(
            scroll(&mut app, AppMessage::SheetScroll(SheetMove::PageUp)),
            9
        );
        assert_eq!(scroll(&mut app, AppMessage::SheetScroll(SheetMove::Top)), 0);
    }

    #[test]
    fn status_kind_sets_footer_colour() {
        let app = app_for_tests(ShellMode::Dashboard);
        let theme = &app.ui_theme;
        assert_eq!(app.status_kind_color(StatusKind::Done), theme.success);
        assert_eq!(app.status_kind_color(StatusKind::Problem), theme.error);
        assert_eq!(app.status_kind_color(StatusKind::Note), theme.info);
        assert_eq!(app.status_kind_color(StatusKind::Working), theme.warning);
    }

    #[test]
    fn keeping_the_addons_shows_no_message() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard
            .set_pending_delete_folders(Some(vec!["Second".to_string()]));

        for action in app.update(AppMessage::DashboardCancelPendingDelete) {
            app.apply(action);
        }

        assert!(app.dashboard.pending_delete_folders().is_none());
        assert_eq!(app.status_line.kind, StatusKind::Quiet);
        assert_eq!(app.footer_status_text(), "");
        assert!(app.status_line_message().is_none());
    }

    #[test]
    fn quiet_status_is_never_shown_whatever_its_text() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Quiet,
            DetailMode::Overview,
            "delete failed: this text would once have been red",
        )));

        assert_eq!(app.footer_status_text(), "");
        assert!(app.status_line_message().is_none());

        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Problem,
            DetailMode::Overview,
            "selection moved",
        )));
        assert_eq!(app.footer_status_text(), "selection moved");
        assert_eq!(
            app.status_line_message().map(|(_, kind)| kind),
            Some(StatusKind::Problem)
        );
    }

    #[test]
    fn details_arrows_and_jk_move_to_the_previous_or_next_addon() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);
        let next = vec![
            AppMessage::DashboardSelectionNext,
            AppMessage::SheetScroll(SheetMove::Top),
        ];
        let previous = vec![
            AppMessage::DashboardSelectionPrevious,
            AppMessage::SheetScroll(SheetMove::Top),
        ];
        for code in [KeyCode::Down, KeyCode::Char('j')] {
            assert_eq!(app.messages_for_key(KeyEvent::from(code)), next);
        }
        for code in [KeyCode::Up, KeyCode::Char('k')] {
            assert_eq!(app.messages_for_key(KeyEvent::from(code)), previous);
        }

        // Details stays open on the next addon.
        app.dashboard.list_state.select(Some(0));
        let first = app
            .inspect_resolved_target()
            .expect("target")
            .item
            .folder
            .clone();
        for message in app.messages_for_key(KeyEvent::from(KeyCode::Down)) {
            for action in app.update(message) {
                app.apply(action);
            }
        }
        assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Inspect));
        let second = app
            .inspect_resolved_target()
            .expect("target")
            .item
            .folder
            .clone();
        assert_ne!(first, second);
    }

    #[test]
    fn inspect_on_child_row_resolves_to_parent_with_child_context() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));

        let resolved = app.inspect_resolved_target().expect("inspect target");

        assert_eq!(resolved.item.folder, "Second");
    }

    #[test]
    fn details_no_longer_binds_section_toggles_or_find_online() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);
        for code in ['r', 'd', 't', 'f'] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char(code))),
                vec![],
                "{code}"
            );
        }
        let keys: Vec<&str> = app.key_table(80).iter().map(|hint| hint.key).collect();
        for key in ["r", "d", "t", "f"] {
            assert!(!keys.contains(&key), "{key} on the key line: {keys:?}");
        }
    }

    #[test]
    fn switching_detail_modes_closes_inspect_overlay() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);

        let actions = app.update(AppMessage::SetDetailMode(DetailMode::Install));

        assert_eq!(actions[0], AppAction::SetDetailMode(DetailMode::Install));
    }

    #[test]
    fn install_overlay_idle_escape_returns_to_overview() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Install));

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCloseOverlay]
        );
    }

    #[test]
    fn install_overlay_edit_escape_stops_edit_before_closing_overlay() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Install));
        app.install_pane = app.install_pane.begin_editing();

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::InstallStopEditing]
        );
    }

    #[test]
    fn install_edit_mode_treats_quit_letter_as_input() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Install;
        app.install_pane = app.install_pane.begin_editing();

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::InstallInputChar('q')]
        );
    }

    #[test]
    fn search_mode_submit_starts_background_search() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.query = "WeakAuras".into();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_search()),
                AppAction::StartWagoSearch {
                    query: "WeakAuras".to_string(),
                    api_key: "test-wago-key".to_string(),
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Search,
                    "Searching Wago.io for \"WeakAuras\""
                )),
            ]
        );
    }

    #[test]
    fn search_mode_submit_direct_target_starts_install_flow() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.effective_addon_dir = Some(PathBuf::from(
            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.search_pane.query = "https://addons.wago.io/addons/VBNBxKx5".into();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_direct_install_check(
                    "VBNBxKx5",
                    "https://addons.wago.io/addons/VBNBxKx5",
                )),
                AppAction::SetWagoInstallInProgress(true),
                AppAction::StartWagoInstall {
                    addon_dir: PathBuf::from(
                        "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
                    ),
                    api_key: "test-wago-key".to_string(),
                    request: PendingWagoInstallRequest {
                        target: "VBNBxKx5".to_string(),
                        source: WagoInstallSource::DirectInput,
                    },
                    replace_folders: None,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Search,
                    "Installing https://addons.wago.io/addons/VBNBxKx5 from Wago.io",
                )),
            ]
        );
    }

    #[test]
    fn search_mode_submit_plain_lowercase_name_stays_search() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.query = "details".into();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_search()),
                AppAction::StartWagoSearch {
                    query: "details".to_string(),
                    api_key: "test-wago-key".to_string(),
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Search,
                    "Searching Wago.io for \"details\""
                )),
            ]
        );
    }

    #[test]
    fn install_sheet_q_and_comma_are_text() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.wago_api_key = None;

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(','))),
            vec![AppMessage::SearchInputChar(',')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::SearchInputChar('q')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCloseOverlay]
        );
    }

    #[test]
    fn down_and_up_move_between_the_box_and_the_results() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::WagoSearchFinished(Ok(WagoSearchOutcome {
                query: "big".to_string(),
                results: vec![sample_result("a", "Alpha"), sample_result("b", "Beta")],
            }))),
        );
        assert_eq!(app.search_pane.focus, SearchFocus::Box);
        assert_eq!(app.focused_text_box(), Some(TextBox::Search));

        let press = |app: &mut App, code: KeyCode| {
            for message in app.messages_for_key(KeyEvent::from(code)) {
                apply_all(app, message);
            }
        };
        press(&mut app, KeyCode::Down);
        assert_eq!(app.search_pane.focus, SearchFocus::Results);
        assert_eq!(app.search_pane.selected_result, Some(0));
        assert_eq!(app.focused_text_box(), None);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.search_pane.selected_result, Some(1));
        // Down on the last one stays there.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.search_pane.selected_result, Some(1));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.search_pane.focus, SearchFocus::Box);
        assert_eq!(app.focused_text_box(), Some(TextBox::Search));
    }

    #[test]
    fn a_letter_in_the_results_goes_back_to_the_box_and_types() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.search_pane.query = "big".into();
        app.search_pane.results = vec![sample_result("a", "Alpha")];
        app.search_pane = app.search_pane.focus_results();

        let messages = app.messages_for_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(
            messages,
            vec![AppMessage::SearchFocusBox, AppMessage::SearchInputChar('j')]
        );
        for message in messages {
            apply_all(&mut app, message);
        }
        assert_eq!(app.search_pane.focus, SearchFocus::Box);
        assert_eq!(app.search_pane.query, "bigj");
        // enter on a result installs it.
        app.search_pane = app.search_pane.focus_results();
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
            vec![AppMessage::SearchInstallSelected]
        );
    }

    fn sample_result(id: &str, name: &str) -> WagoSearchResult {
        WagoSearchResult {
            id: id.to_string(),
            display_name: name.to_string(),
            summary: Some(format!("{name} summary")),
            owner: Some("Owner".to_string()),
            authors: vec![],
            website_url: Some(format!("https://addons.wago.io/addons/{id}")),
            download_count: Some(1_200),
            version: Some("1.0".to_string()),
        }
    }

    #[test]
    fn error_kinds_lead_with_what_happened_and_keep_the_detail() {
        use crate::app::render_install_search::source_error_advice;
        assert_eq!(
            source_error_advice(SourceErrorKind::NoConnection, "TukUI"),
            "LemonUp couldn't reach TukUI. Check your internet connection and try again."
        );
        assert!(
            source_error_advice(SourceErrorKind::KeyRejected, "Wago.io").contains("Wago.io key")
        );
        assert!(
            source_error_advice(SourceErrorKind::KeyRejected, "GitHub").contains("GITHUB_TOKEN")
        );
        assert!(
            source_error_advice(SourceErrorKind::NotFound, "WoWInterface")
                .starts_with("WoWInterface doesn't have that addon")
        );
        assert!(source_error_advice(SourceErrorKind::SlowDown, "GitHub").contains("slow down"));
        assert!(source_error_advice(SourceErrorKind::Other, "GitHub").contains("technical detail"));

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.search_pane.query = "elvui".into();
        let request = crate::app::direct_install::DirectInstallRequest {
            provider: crate::app::direct_install::DirectInstallProvider::Tukui,
            target: "elvui".to_string(),
            display: "ElvUI".to_string(),
        };
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request,
                result: Err(SourceError::new(
                    SourceErrorKind::NoConnection,
                    "error sending request for url (https://api.tukui.org/v1/addons)",
                )),
            }),
        );
        let lines = app
            .search_state_lines(100)
            .expect("error lines")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(lines[0], "! Couldn't install ElvUI.");
        assert!(lines[2].contains("couldn't reach TukUI"), "{lines:?}");
        assert!(
            lines.last().is_some_and(
                |line: &String| line.contains("Technical detail: error sending request")
            ),
            "{lines:?}"
        );
    }

    #[test]
    fn f_finds_only_a_not_linked_addon_online() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard = DashboardState::from_addons(vec![
            AddonRecord::new("BugSack", "BugSack", SourceKind::Manual),
            AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago),
        ]);
        let select = |app: &mut App, folder: &str| {
            let index = app
                .dashboard
                .rows
                .iter()
                .position(|row| row.folder == folder)
                .expect("row");
            app.dashboard.list_state.select(Some(index));
        };
        let f = KeyEvent::from(KeyCode::Char('f'));

        select(&mut app, "WeakAuras");
        assert_eq!(app.messages_for_key(f), vec![]);
        assert!(!app.key_table(160).iter().any(|hint| hint.key == "f"));
        apply_all(&mut app, AppMessage::DashboardOpenInspect);
        assert_eq!(app.messages_for_key(f), vec![]);
        assert!(!app.key_table(100).iter().any(|hint| hint.key == "f"));
        app.apply(AppAction::SetDetailMode(DetailMode::Overview));
        app.shell_ui.overlay.active = None;

        select(&mut app, "BugSack");
        assert_eq!(app.messages_for_key(f), vec![AppMessage::FindOnline]);
        assert!(app.key_table(160).iter().any(|hint| hint.key == "f"));
        // In the Details sheet and docked beside the list.
        apply_all(&mut app, AppMessage::DashboardOpenInspect);
        app.last_frame_width = Some(100);
        assert_eq!(app.messages_for_key(f), vec![AppMessage::FindOnline]);
        assert!(app.key_table(100).iter().any(|hint| hint.key == "f"));
        app.last_frame_width = Some(160);
        assert!(app.details_docked());
        assert_eq!(app.messages_for_key(f), vec![AppMessage::FindOnline]);
        assert!(app.key_table(160).iter().any(|hint| hint.key == "f"));
    }

    #[test]
    fn f_opens_install_with_the_name_and_searches_right_away() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard = DashboardState::from_addons(vec![AddonRecord::new(
            "Mythic Dungeon Tools",
            "MythicDungeonTools",
            SourceKind::Manual,
        )]);
        app.dashboard.list_state.select(Some(0));
        app.search_pane.query = "old text".into();

        let actions = app.update(AppMessage::FindOnline);
        assert!(
            actions.contains(&AppAction::SetDetailMode(DetailMode::Search)),
            "{actions:?}"
        );
        assert!(
            actions.contains(&AppAction::StartWagoSearch {
                query: "Mythic Dungeon Tools".to_string(),
                api_key: "test-wago-key".to_string(),
            }),
            "{actions:?}"
        );
        for action in actions {
            if !matches!(action, AppAction::StartWagoSearch { .. }) {
                app.apply(action);
            }
        }
        assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
        assert_eq!(app.search_pane.query, "Mythic Dungeon Tools");
        assert_eq!(
            app.search_pane.query.cursor(),
            "Mythic Dungeon Tools".chars().count()
        );
        assert!(app.search_pane.in_progress);

        // Without a key the key box asks for it, the name kept in the box.
        app.wago_api_key = None;
        app.search_pane = SearchPaneState::default();
        apply_all(&mut app, AppMessage::DashboardCloseOverlay);
        apply_all(&mut app, AppMessage::FindOnline);
        assert!(app.search_pane.key_box.is_some());
        assert_eq!(app.search_pane.query, "Mythic Dungeon Tools");
    }

    fn elvui_request() -> crate::app::direct_install::DirectInstallRequest {
        crate::app::direct_install::DirectInstallRequest {
            provider: crate::app::direct_install::DirectInstallProvider::Tukui,
            target: "elvui".to_string(),
            display: "elvui".to_string(),
        }
    }

    fn folders_in_the_way(folders: &[&str], existing_name: Option<&str>) -> SourceError {
        SourceError::folders(
            SourceErrorKind::FoldersInTheWay,
            format!(
                "install refused because these addon folders already exist: {}",
                folders.join(", ")
            ),
            FolderClash {
                replaceable: folders.iter().map(|folder| (*folder).to_string()).collect(),
                linked: Vec::new(),
                existing_name: existing_name.map(str::to_string),
            },
        )
    }

    /// An app with a direct install of ElvUI running in the Install sheet.
    fn installing_elvui() -> App {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.effective_addon_dir = Some(PathBuf::from("/sandbox/AddOns"));
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.search_pane = app.search_pane.start_direct_install(elvui_request());
        app.direct_install_in_progress = true;
        app
    }

    #[test]
    fn a_link_over_a_not_linked_folder_asks_naming_the_folders_then_replaces_only_those() {
        let mut app = installing_elvui();
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request: elvui_request(),
                result: Err(folders_in_the_way(
                    &["ElvUI", "ElvUI_Options"],
                    Some("ElvUI"),
                )),
            }),
        );
        // The addon already there names the question, not the link's slug.
        assert_eq!(
            app.active_question(),
            Some(crate::app::dialog::Question::ReplaceAddon {
                name: "ElvUI".to_string(),
                source: "TukUI",
                direct: true,
                folders: vec!["ElvUI".to_string(), "ElvUI_Options".to_string()],
            })
        );
        app.note_question_shown(Instant::now());
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::SearchConfirmReplace]
        );
        let actions = app.update(AppMessage::SearchConfirmReplace);
        assert!(actions.contains(&AppAction::StartDirectInstall {
            addon_dir: PathBuf::from("/sandbox/AddOns"),
            request: elvui_request(),
            replace_folders: vec!["ElvUI".to_string(), "ElvUI_Options".to_string()],
        }));

        // No leaves the folders alone.
        app.search_pane = app.search_pane.require_direct_replace(
            elvui_request(),
            &folders_in_the_way(&["ElvUI"], None).clash.expect("clash"),
        );
        app.note_question_shown(Instant::now());
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('n'))),
            vec![AppMessage::SearchCancelReplace]
        );
        apply_all(&mut app, AppMessage::SearchCancelReplace);
        assert_eq!(app.active_question(), None);
    }

    #[test]
    fn a_download_that_changed_since_the_question_asks_again() {
        let mut app = installing_elvui();
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request: elvui_request(),
                result: Err(folders_in_the_way(&["ElvUI"], Some("ElvUI"))),
            }),
        );
        app.note_question_shown(Instant::now());
        for action in app.update(AppMessage::SearchConfirmReplace) {
            if !matches!(action, AppAction::StartDirectInstall { .. }) {
                app.apply(action);
            }
        }
        assert!(app.direct_install_in_progress);

        // The fresh download also brings ElvUI_Options: a new question names both.
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request: elvui_request(),
                result: Err(folders_in_the_way(
                    &["ElvUI", "ElvUI_Options"],
                    Some("ElvUI"),
                )),
            }),
        );
        assert!(!app.direct_install_in_progress);
        assert!(matches!(
            app.active_question(),
            Some(crate::app::dialog::Question::ReplaceAddon { ref folders, .. })
                if folders == &["ElvUI".to_string(), "ElvUI_Options".to_string()]
        ));
        // It is a new question: a y needs it drawn first.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            Vec::<AppMessage>::new()
        );
        app.note_question_shown(Instant::now());
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::SearchConfirmReplace]
        );
    }

    #[test]
    fn a_folder_of_a_linked_addon_is_refused_without_a_question() {
        let mut app = installing_elvui();
        let error = SourceError::folders(
            SourceErrorKind::FoldersOfLinkedAddon,
            "install refused because these addon folders belong to linked addons: ElvUI (ElvUI)",
            FolderClash {
                replaceable: Vec::new(),
                linked: vec![("ElvUI".to_string(), "ElvUI".to_string())],
                existing_name: None,
            },
        );
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request: elvui_request(),
                result: Err(error),
            }),
        );
        assert_eq!(app.active_question(), None);
        assert!(!app.direct_install_in_progress);
        let lines = app
            .search_state_lines(100)
            .expect("error lines")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            lines[2].contains(
                "ElvUI belongs to ElvUI, which LemonUp already keeps updated, so LemonUp \
                 won't replace it."
            ),
            "{lines:?}"
        );

        let many = FolderClash {
            replaceable: Vec::new(),
            linked: vec![
                ("DBM-Core".to_string(), "Deadly Boss Mods".to_string()),
                ("DBM-Raids".to_string(), "Deadly Boss Mods".to_string()),
                ("BigWigs".to_string(), "BigWigs".to_string()),
            ],
            existing_name: None,
        };
        assert_eq!(
            install_error_advice(SourceErrorKind::FoldersOfLinkedAddon, "GitHub", Some(&many)),
            "DBM-Core and 2 more belong to Deadly Boss Mods and other addons LemonUp already \
             keeps updated, so LemonUp won't replace them."
        );
    }

    #[test]
    fn a_y_typed_while_installing_never_answers_the_replace_question() {
        let mut app = installing_elvui();
        app.search_pane.query = "elvui".into();
        // While the install runs the box takes no letters, paste or edits.
        assert_eq!(app.focused_text_box(), None);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            Vec::<AppMessage>::new()
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Backspace)),
            Vec::<AppMessage>::new()
        );
        assert!(app.update(AppMessage::Paste("more".to_string())).is_empty());
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCloseOverlay]
        );

        // A y read while installing waits in the queue; the question arrives
        // and is drawn before the app gets to that y.
        let typed_at = Instant::now();
        app.key_read_at = Some(typed_at);
        apply_all(
            &mut app,
            AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                request: elvui_request(),
                result: Err(folders_in_the_way(&["ElvUI"], Some("ElvUI"))),
            }),
        );
        let shown_at = typed_at + StdDuration::from_millis(5);
        app.note_question_shown(shown_at);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            Vec::<AppMessage>::new()
        );
        assert!(app.active_question().is_some());

        // A y pressed once the question is on screen answers it.
        app.key_read_at = Some(shown_at + StdDuration::from_millis(5));
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::SearchConfirmReplace]
        );
    }

    #[test]
    fn installs_and_list_changes_wait_for_each_other() {
        // An install can't start while an update, delete or undo runs.
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.effective_addon_dir = Some(PathBuf::from("/sandbox/AddOns"));
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        let note = |actions: &[AppAction]| {
            actions.iter().find_map(|action| match action {
                AppAction::SetStatus(status) => Some(status.text.clone()),
                _ => None,
            })
        };
        for (setup, expected) in [
            (
                AppAction::SetDashboardUpdateInProgress(true),
                "Wait for the update to finish first.",
            ),
            (
                AppAction::SetDashboardDeleteInProgress(true),
                "Wait for the delete to finish first.",
            ),
            (
                AppAction::SetDashboardUndoInProgress(true),
                "Wait for the undo to finish first.",
            ),
        ] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.effective_addon_dir = Some(PathBuf::from("/sandbox/AddOns"));
            app.apply(AppAction::SetDetailMode(DetailMode::Search));
            app.apply(setup);
            let actions =
                app.start_direct_install_actions(elvui_request(), Vec::new(), "ElvUI".to_string());
            assert!(
                !actions
                    .iter()
                    .any(|action| matches!(action, AppAction::StartDirectInstall { .. })),
                "{actions:?}"
            );
            assert!(
                note(&actions).is_some_and(|text| text.contains(expected)),
                "{actions:?}"
            );
        }

        // U, u, x and z wait while an install runs.
        app.apply(AppAction::SetDetailMode(DetailMode::Overview));
        app.apply(AppAction::SetDirectInstallInProgress(true));
        // U with updates ready goes through the same check as u.
        let actions = app.dashboard_update_actions(Ok(vec!["Second".to_string()]));
        assert!(
            note(&actions)
                .is_some_and(|text| text.contains("Wait for the install to finish first.")),
            "U: {actions:?}"
        );
        for message in [
            AppMessage::DashboardRunUpdateSelected,
            AppMessage::DashboardRequestDelete,
            AppMessage::DashboardUndoDelete,
        ] {
            let actions = app.update(message.clone());
            assert!(
                note(&actions)
                    .is_some_and(|text| text.contains("Wait for the install to finish first.")),
                "{message:?}: {actions:?}"
            );
            assert!(
                !actions.iter().any(|action| matches!(
                    action,
                    AppAction::SetPendingDelete(Some(_))
                        | AppAction::StartDashboardUndoDelete { .. }
                        | AppAction::SetDashboardUpdateInProgress(true)
                )),
                "{message:?}: {actions:?}"
            );
        }
        app.apply(AppAction::SetDirectInstallInProgress(false));
        app.apply(AppAction::SetWagoInstallInProgress(true));
        let actions = app.update(AppMessage::DashboardRequestDelete);
        assert!(
            note(&actions)
                .is_some_and(|text| text.contains("Wait for the install to finish first.")),
            "{actions:?}"
        );
    }

    #[test]
    fn a_long_technical_detail_wraps_under_its_own_start() {
        for width in [80u16, 120] {
            let mut app = installing_elvui();
            apply_all(
                &mut app,
                AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                    request: elvui_request(),
                    result: Err(SourceError::new(
                        SourceErrorKind::Other,
                        "error decoding response body: expected value at line 1 column 1 \
                         while reading https://api.tukui.org/v1/addons for the ElvUI \
                         package, then the connection closed before the message completed",
                    )),
                }),
            );
            let rows = render_rows(&mut app, width, 40);
            let text = rows.join("\n");
            let first = rows
                .iter()
                .position(|row| row.contains("Technical detail: "))
                .unwrap_or_else(|| panic!("no detail at {width}:\n{text}"));
            let column = rows[first]
                .chars()
                .collect::<Vec<_>>()
                .windows(18)
                .position(|window| window.iter().collect::<String>() == "Technical detail: ")
                .expect("label column")
                + "Technical detail: ".len();
            let next: Vec<char> = rows[first + 1].chars().collect();
            let label = column - "Technical detail: ".len();
            assert!(
                next[label..column].iter().all(|cell| *cell == ' ') && next[column] != ' ',
                "continuation not aligned at {width}:\n{text}"
            );
        }
    }

    #[test]
    fn dashboard_search_keys_open_expected_ab_variants() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('/'))),
            vec![AppMessage::DashboardFilterBegin]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('?'))),
            vec![AppMessage::DashboardOpenHelp]
        );
    }

    #[test]
    fn help_overlay_opens_and_closes_with_question_mark() {
        let mut app = app_for_tests(ShellMode::Dashboard);

        let actions = app.update(AppMessage::DashboardOpenHelp);
        assert_eq!(actions[0], AppAction::SetHelpContext(HelpContext::Overview));
        assert_eq!(actions[1], AppAction::SetHelpOverlay(true));

        app.apply(AppAction::SetHelpContext(HelpContext::Overview));
        app.apply(AppAction::SetHelpOverlay(true));
        assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Help));

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('?'))),
            vec![AppMessage::DashboardCloseOverlay]
        );
    }

    #[test]
    fn opening_install_clears_the_box_and_old_results() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.search_pane.query = "WeakAuras".into();
        app.search_pane.results = vec![WagoSearchResult {
            id: "VBNBxKx5".to_string(),
            display_name: "WeakAuras".to_string(),
            summary: Some("Aura framework".to_string()),
            owner: Some("WeakAuras Team".to_string()),
            authors: vec!["WeakAuras Team".to_string()],
            website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
            download_count: Some(500_000),
            version: Some("5.21.1".to_string()),
        }];
        app.search_pane.selected_result = Some(0);
        app.search_pane.last_query = Some("WeakAuras".to_string());

        let actions = app.update(AppMessage::OpenSearch(SearchPresentationMode::TwoState));
        for action in actions {
            app.apply(action);
        }

        assert_eq!(app.dashboard.detail_mode, DetailMode::Search);
        assert_eq!(
            app.search_pane.presentation_mode,
            SearchPresentationMode::TwoState
        );
        assert_eq!(app.search_pane.query, "");
        assert!(app.search_pane.results.is_empty());
        assert_eq!(app.search_pane.last_query, None);
        assert_eq!(app.search_pane.focus, SearchFocus::Box);
        // Typing goes straight to the box: there is no edit mode.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('w'))),
            vec![AppMessage::SearchInputChar('w')]
        );
    }

    #[test]
    fn search_edit_mode_treats_quit_letter_as_input() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::SearchInputChar('q')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('e'))),
            vec![AppMessage::SearchInputChar('e')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
            vec![AppMessage::SearchSubmit]
        );
    }

    #[test]
    fn a_name_search_without_a_key_asks_for_it_in_place() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.wago_api_key = None;
        app.search_pane.query = "WeakAuras".into();

        apply_all(&mut app, AppMessage::SearchSubmit);

        assert!(app.search_pane.key_box.is_some());
        assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
        // Typing and pasting go to the key box, not the search box.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('k'))),
            vec![AppMessage::SearchKeyChar('k')]
        );
        apply_all(&mut app, AppMessage::Paste("secret-key".to_string()));
        assert_eq!(
            app.search_pane.key_box.as_ref().map(TextInput::value),
            Some("secret-key")
        );
        assert_eq!(app.search_pane.query, "WeakAuras");
        // The key is drawn masked.
        assert_eq!(
            app.search_pane
                .key_box
                .as_ref()
                .map(|key| key.masked('•').value().to_string()),
            Some("••••••••••".to_string())
        );
        // esc goes back to the search box first.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::SearchCloseKeyBox]
        );
        apply_all(&mut app, AppMessage::SearchCloseKeyBox);
        assert!(app.search_pane.key_box.is_none());
        assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
    }

    #[test]
    fn saving_the_key_in_place_stores_it_and_runs_the_search() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_store = ConfigStore::new(temp.path().join("config.toml"));
        app.apply(AppAction::SetDetailMode(DetailMode::Search));
        app.wago_api_key = None;
        app.search_pane.query = "bigwigs".into();
        apply_all(&mut app, AppMessage::SearchSubmit);
        apply_all(&mut app, AppMessage::Paste("  test-key-123 ".to_string()));

        let actions = app.update(AppMessage::SearchSaveKey);

        assert!(actions.contains(&AppAction::StartWagoSearch {
            query: "bigwigs".to_string(),
            api_key: "test-key-123".to_string(),
        }));
        // The search itself needs a runtime; everything else applies here.
        for action in actions {
            if !matches!(action, AppAction::StartWagoSearch { .. }) {
                app.apply(action);
            }
        }
        assert_eq!(app.wago_api_key.as_deref(), Some("test-key-123"));
        assert_eq!(
            app.config_pane.draft.wago_api_key.as_deref(),
            Some("test-key-123")
        );
        let saved = std::fs::read_to_string(temp.path().join("config.toml")).expect("config");
        assert!(saved.contains("test-key-123"));
        assert!(app.search_pane.key_box.is_none());
        assert!(app.search_pane.in_progress);
        // The status line never shows the key.
        assert!(!app.status_line.text.contains("test-key-123"));
    }

    #[test]
    fn search_install_selected_starts_install_flow_for_selected_result() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.effective_addon_dir = Some(PathBuf::from(
            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.search_pane = SearchPaneState {
            query: "WeakAuras".into(),
            presentation_mode: SearchPresentationMode::ComposeFirst,
            focus: SearchFocus::Results,
            key_box: None,
            in_progress: false,
            results: vec![WagoSearchResult {
                id: "VBNBxKx5".to_string(),
                display_name: "WeakAuras".to_string(),
                summary: Some("Aura framework".to_string()),
                owner: Some("WeakAuras Team".to_string()),
                authors: vec!["WeakAuras Team".to_string()],
                website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
                download_count: Some(500_000),
                version: Some("5.21.1".to_string()),
            }],
            selected_result: Some(0),
            last_query: Some("WeakAuras".to_string()),
            install_state: SearchInstallState::Idle,
        };

        let actions = app.update(AppMessage::SearchInstallSelected);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_install_check(
                    app.search_pane.selected_result().expect("selected result"),
                )),
                AppAction::SetWagoInstallInProgress(true),
                AppAction::StartWagoInstall {
                    addon_dir: PathBuf::from(
                        "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
                    ),
                    api_key: "test-wago-key".to_string(),
                    request: PendingWagoInstallRequest {
                        target: "VBNBxKx5".to_string(),
                        source: WagoInstallSource::SearchResult,
                    },
                    replace_folders: None,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Search,
                    "Installing WeakAuras from Wago.io",
                )),
            ]
        );
    }

    #[test]
    fn install_submit_starts_direct_install_flow() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Install;
        app.effective_addon_dir = Some(PathBuf::from(
            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.install_pane.input = "https://addons.wago.io/addons/VBNBxKx5".into();

        let actions = app.update(AppMessage::InstallSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetWagoInstallInProgress(true),
                AppAction::StartWagoInstall {
                    addon_dir: PathBuf::from(
                        "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
                    ),
                    api_key: "test-wago-key".to_string(),
                    request: PendingWagoInstallRequest {
                        target: "https://addons.wago.io/addons/VBNBxKx5".to_string(),
                        source: WagoInstallSource::DirectInput,
                    },
                    replace_folders: None,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Install,
                    "Checking the Wago.io addon before installing",
                )),
            ]
        );
    }

    #[test]
    fn config_mode_navigation_and_edit_keys_are_routed_to_config_pane() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.selected_field = ConfigField::ALL
            .iter()
            .position(|field| matches!(field, ConfigField::ShowLibs))
            .expect("show libs field present");

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('j'))),
            vec![AppMessage::ConfigSelectionNext]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('e'))),
            vec![AppMessage::ConfigBeginEditing]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('r'))),
            vec![AppMessage::ConfigRunOnboarding]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('l'))),
            vec![AppMessage::ConfigAdjustSelected(1)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Left)),
            vec![AppMessage::ConfigAdjustSelected(-1)]
        );

        app.config_pane.selected_field = 0;
        app.apply(AppAction::SetConfigPaneState(
            app.config_pane.begin_editing(),
        ));
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Backspace)),
            vec![AppMessage::ConfigBackspace]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
            vec![AppMessage::ConfigCommitEdit]
        );
    }

    #[test]
    fn config_mode_allows_global_quit_and_escape_when_not_editing() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;
        app.apply(AppAction::SelectAllDashboardParents);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::QuitRequested]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardClearSelection]
        );
    }

    #[test]
    fn backup_mode_allows_global_quit_and_escape_when_idle() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.apply(AppAction::SelectAllDashboardParents);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::QuitRequested]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardClearSelection]
        );
    }

    #[test]
    fn config_commit_edit_updates_runtime_wago_api_key_and_marks_config_present() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_present = false;
        app.wago_api_key = None;
        app.config_store = ConfigStore::new(config_path.clone());
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: "new-wago-key".into(),
        });

        let actions = app.update(AppMessage::ConfigCommitEdit);
        assert!(matches!(actions[0], AppAction::SetPersistedConfig(_)));
        for action in actions {
            app.apply(action);
        }

        assert_eq!(app.wago_api_key.as_deref(), Some("new-wago-key"));
        assert!(app.config_present);

        let saved = app.config_store.load().expect("load config");
        match saved {
            ConfigLoad::Loaded(config) => {
                assert_eq!(config.wago_api_key.as_deref(), Some("new-wago-key"));
            }
            ConfigLoad::Missing(_) => panic!("config should have been written"),
        }
    }

    #[test]
    fn set_persisted_config_updates_ui_theme_from_saved_theme() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let mut config = app.config_pane.draft.clone();
        config.theme = ThemeMode::Light;

        app.apply(AppAction::SetPersistedConfig(config));

        assert_eq!(app.ui_theme.panel_title, Color::Rgb(46, 125, 233));
        assert_eq!(app.ui_theme.overlay_bg, Color::Rgb(232, 233, 238));
    }

    #[test]
    fn config_run_onboarding_reenters_location_finder_with_current_path() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;
        app.shell_ui.overlay.active = Some(OverlayKind::Config);
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));

        let actions = app.update(AppMessage::ConfigRunOnboarding);
        assert!(matches!(actions[0], AppAction::ReenterOnboarding));
        assert!(matches!(actions[1], AppAction::SetStatus(_)));

        for action in actions {
            app.apply(action);
        }

        assert_eq!(app.shell_mode, ShellMode::Onboarding);
        assert_eq!(app.dashboard.detail_mode, DetailMode::Overview);
        assert_eq!(app.shell_ui.overlay.active, None);
        assert_eq!(
            app.onboarding.input,
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
        );
        assert!(matches!(app.onboarding.phase, OnboardingPhase::Ready));
    }

    #[test]
    fn config_toggle_selected_persists_toggle_without_explicit_save() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_store = ConfigStore::new(config_path.clone());
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.selected_field = ConfigField::ALL
            .iter()
            .position(|field| *field == ConfigField::ShowLibs)
            .expect("show libs field index");
        app.config_pane.draft.show_libs = false;
        app.config_pane.persisted.show_libs = false;

        let actions = app.update(AppMessage::ConfigToggleSelected);
        assert!(matches!(actions[0], AppAction::SetPersistedConfig(_)));
        for action in actions {
            app.apply(action);
        }

        assert!(app.config_pane.persisted.show_libs);
        let saved = app.config_store.load().expect("load config");
        match saved {
            ConfigLoad::Loaded(config) => assert!(config.show_libs),
            ConfigLoad::Missing(_) => panic!("config should have been written"),
        }
    }

    #[test]
    fn config_adjust_selected_persists_numeric_change_without_explicit_save() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_store = ConfigStore::new(config_path.clone());
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.selected_field = ConfigField::ALL
            .iter()
            .position(|field| *field == ConfigField::CheckInterval)
            .expect("check interval field index");
        app.config_pane.draft.check_interval_secs = 300;
        app.config_pane.persisted.check_interval_secs = 300;

        let actions = app.update(AppMessage::ConfigAdjustSelected(1));
        assert!(matches!(actions[0], AppAction::SetPersistedConfig(_)));
        for action in actions {
            app.apply(action);
        }

        assert_eq!(app.config_pane.persisted.check_interval_secs, 360);
        let saved = app.config_store.load().expect("load config");
        match saved {
            ConfigLoad::Loaded(config) => assert_eq!(config.check_interval_secs, 360),
            ConfigLoad::Missing(_) => panic!("config should have been written"),
        }
    }

    #[test]
    fn config_commit_edit_reverts_to_persisted_value_on_write_failure() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config-dir");
        std::fs::create_dir_all(&config_path).expect("create config dir");

        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_store = ConfigStore::new(config_path);
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.persisted.wago_api_key = Some("old-key".to_string());
        app.config_pane.draft.wago_api_key = Some("old-key".to_string());
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: "new-key".into(),
        });

        let actions = app.update(AppMessage::ConfigCommitEdit);
        for action in actions {
            app.apply(action);
        }

        assert_eq!(app.config_pane.edit, None);
        assert_eq!(
            app.config_pane.draft.wago_api_key.as_deref(),
            Some("old-key")
        );
        assert_eq!(
            app.config_pane.persisted.wago_api_key.as_deref(),
            Some("old-key")
        );
    }

    #[test]
    fn backup_run_now_starts_background_backup_task() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));

        let actions = app.update(AppMessage::BackupRunNow);

        assert_eq!(actions.len(), 3);
        assert!(matches!(actions[0], AppAction::SetBackupPaneState(_)));
        assert!(matches!(actions[1], AppAction::StartBackupNow { .. }));
        assert!(matches!(actions[2], AppAction::SetStatus(_)));
    }

    #[test]
    fn backup_run_now_is_throttled_when_recent_backup_completed() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.backup_pane.last_completed_backup_at =
            Some(OffsetDateTime::now_utc() - Duration::seconds(2));

        let actions = app.update(AppMessage::BackupRunNow);

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            AppAction::SetStatus(status) => {
                assert!(status.text.contains("You backed up a moment ago"));
                assert!(status.text.contains("wait"));
            }
            other => panic!("expected throttled backup status, got {other:?}"),
        }
    }

    #[test]
    fn backup_finished_updates_backup_history_and_status() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.backup_pane.in_progress = true;

        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::BackupFinished(
            Ok(BackupRunOutcome {
                backup: BackupEntry {
                    file_name: "WTF-20260324T120000Z.zip".to_string(),
                    path: PathBuf::from("C:\\Temp\\WTF-20260324T120000Z.zip"),
                    label: "2026-03-24 12:00:00Z".to_string(),
                    size_bytes: 1024,
                },
                pruned_files: vec!["WTF-20260323T120000Z.zip".to_string()],
                backups: vec![BackupEntry {
                    file_name: "WTF-20260324T120000Z.zip".to_string(),
                    path: PathBuf::from("C:\\Temp\\WTF-20260324T120000Z.zip"),
                    label: "2026-03-24 12:00:00Z".to_string(),
                    size_bytes: 1024,
                }],
            }),
        )));

        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], AppAction::SetBackupPaneState(_)));
        assert!(matches!(actions[1], AppAction::SetStatus(_)));
    }

    #[test]
    fn pending_wago_install_confirmation_keys_route_to_confirm_or_cancel() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.pending_wago_install_confirmation = Some(WagoInstallConfirmation {
            request: PendingWagoInstallRequest {
                target: "VBNBxKx5".to_string(),
                source: WagoInstallSource::SearchResult,
            },
            inspection: WagoInstallInspection {
                addon_id: "VBNBxKx5".to_string(),
                addon_name: "WeakAuras".to_string(),
                parent_folder: "WeakAuras".to_string(),
                installed_folders: vec!["WeakAuras".to_string()],
                stability: crate::wago::WagoStability::Stable,
                version: Some("5.21.1".to_string()),
                existing_folders: vec!["WeakAuras".to_string()],
                tracked_parent: Some("WeakAuras".to_string()),
            },
        });
        app.note_question_shown(Instant::now());

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::WagoConfirmInstall]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::WagoCancelInstall]
        );
    }

    #[test]
    fn wago_search_finished_updates_results_and_status() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.in_progress = true;

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::WagoSearchFinished(Ok(WagoSearchOutcome {
                query: "WeakAuras".to_string(),
                results: vec![WagoSearchResult {
                    id: "VBNBxKx5".to_string(),
                    display_name: "WeakAuras".to_string(),
                    summary: Some("Aura framework".to_string()),
                    owner: Some("WeakAuras Team".to_string()),
                    authors: vec!["WeakAuras Team".to_string()],
                    website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
                    download_count: Some(500_000),
                    version: Some("5.21.1".to_string()),
                }],
            })),
        ));

        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], AppAction::SetSearchPaneState(_)));
        assert_eq!(
            actions[1],
            AppAction::SetStatus(app.dashboard_status_for(
                StatusKind::Done,
                DetailMode::Search,
                "Found 1 addon on Wago.io"
            ))
        );
    }

    #[test]
    fn wago_search_finished_sorts_results_by_downloads_descending() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.in_progress = true;

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::WagoSearchFinished(Ok(WagoSearchOutcome {
                query: "details".to_string(),
                results: vec![
                    WagoSearchResult {
                        id: "low".to_string(),
                        display_name: "Low DL".to_string(),
                        summary: None,
                        owner: Some("Owner B".to_string()),
                        authors: vec!["Owner B".to_string()],
                        website_url: None,
                        download_count: Some(10),
                        version: Some("1.0.0".to_string()),
                    },
                    WagoSearchResult {
                        id: "high".to_string(),
                        display_name: "High DL".to_string(),
                        summary: None,
                        owner: Some("Owner A".to_string()),
                        authors: vec!["Owner A".to_string()],
                        website_url: None,
                        download_count: Some(1_000),
                        version: Some("2.0.0".to_string()),
                    },
                    WagoSearchResult {
                        id: "none".to_string(),
                        display_name: "No DL".to_string(),
                        summary: None,
                        owner: Some("Owner C".to_string()),
                        authors: vec!["Owner C".to_string()],
                        website_url: None,
                        download_count: None,
                        version: Some("3.0.0".to_string()),
                    },
                ],
            })),
        ));

        for action in actions {
            app.apply(action);
        }

        let ordered = app
            .search_pane
            .results
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["high", "low", "none"]);
        assert_eq!(app.search_pane.selected_result, Some(0));
    }

    #[test]
    fn format_download_count_uses_compact_suffixes() {
        assert_eq!(format_download_count(999), "999");
        assert_eq!(format_download_count(1_000), "1K");
        assert_eq!(format_download_count(1_500), "1.5K");
        assert_eq!(format_download_count(15_999), "15.9K");
        assert_eq!(format_download_count(1_000_000), "1M");
        assert_eq!(format_download_count(1_290_000), "1.2M");
        assert_eq!(format_download_count(2_000_000_000), "2B");
    }

    #[test]
    fn wago_install_confirmation_event_sets_pending_confirmation() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::WagoInstallFinished(Ok(WagoInstallTaskOutcome::NeedsConfirmation {
                request: PendingWagoInstallRequest {
                    target: "VBNBxKx5".to_string(),
                    source: WagoInstallSource::SearchResult,
                },
                inspection: WagoInstallInspection {
                    addon_id: "VBNBxKx5".to_string(),
                    addon_name: "WeakAuras".to_string(),
                    parent_folder: "WeakAuras".to_string(),
                    installed_folders: vec!["WeakAuras".to_string()],
                    stability: crate::wago::WagoStability::Stable,
                    version: Some("5.21.1".to_string()),
                    existing_folders: vec!["WeakAuras".to_string()],
                    tracked_parent: Some("WeakAuras".to_string()),
                },
            })),
        ));

        assert_eq!(actions.len(), 4);
        assert!(matches!(actions[0], AppAction::SetSearchPaneState(_)));
        assert_eq!(actions[1], AppAction::SetWagoInstallInProgress(false));
        assert!(matches!(
            actions[2],
            AppAction::SetPendingWagoInstallConfirmation(Some(_))
        ));
        assert!(matches!(actions[3], AppAction::SetStatus(_)));
    }

    #[test]
    fn tracked_wago_result_folder_matches_exact_source_url() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.items[0].source = SourceKind::Wago;
        app.dashboard.items[0].folder = "WeakAuras".to_string();
        app.dashboard.items[0].source_url =
            Some("https://addons.wago.io/addons/VBNBxKx5".to_string());

        let result = WagoSearchResult {
            id: "VBNBxKx5".to_string(),
            display_name: "WeakAuras".to_string(),
            summary: None,
            owner: Some("WeakAuras Team".to_string()),
            authors: vec!["WeakAuras Team".to_string()],
            website_url: Some("https://addons.wago.io/addons/VBNBxKx5".to_string()),
            download_count: Some(500_000),
            version: Some("5.21.1".to_string()),
        };

        assert_eq!(
            app.tracked_wago_result_folder(&result).as_deref(),
            Some("WeakAuras")
        );
    }

    #[test]
    fn wago_install_success_sets_search_success_state() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.install_state = SearchInstallState::Installing {
            addon_id: "VBNBxKx5".to_string(),
            addon_name: "WeakAuras".to_string(),
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::WagoInstallFinished(Ok(WagoInstallTaskOutcome::Installed(
                WagoInstallOutcome {
                    summary: WagoInstallSummary {
                        addon_id: "VBNBxKx5".to_string(),
                        addon_name: "WeakAuras".to_string(),
                        parent_folder: "WeakAuras".to_string(),
                        installed_folders: vec!["WeakAuras".to_string()],
                        stability: crate::wago::WagoStability::Stable,
                        version: Some("5.21.1".to_string()),
                        dry_run: false,
                    },
                    sync: AddonScanOutcome {
                        path: PathBuf::from(
                            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
                        ),
                        summary: ScanSummary {
                            scanned_addons: 1,
                            upserted_addons: 1,
                            removed_addons: 0,
                            ..Default::default()
                        },
                        addons: Vec::new(),
                        drift_report: DriftReport::empty(),
                    },
                },
            ))),
        ));

        assert!(matches!(actions[0], AppAction::SetSearchPaneState(_)));
    }

    #[test]
    fn visible_search_result_window_keeps_selected_result_in_view() {
        assert_eq!(visible_search_result_window(12, Some(0), 8), (0, 8));
        assert_eq!(visible_search_result_window(12, Some(7), 8), (0, 8));
        assert_eq!(visible_search_result_window(12, Some(8), 8), (1, 9));
        assert_eq!(visible_search_result_window(12, Some(11), 8), (4, 12));
    }

    #[test]
    fn pending_delete_keys_emit_confirm_or_cancel_messages() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard
            .set_pending_delete_folders(Some(vec!["Second".to_string()]));

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::DashboardConfirmDelete]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('n'))),
            vec![AppMessage::DashboardCancelPendingDelete]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCancelPendingDelete]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::DashboardCancelPendingDelete]
        );
        assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Enter)), vec![]);
        assert_eq!(
            app.messages_for_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            vec![]
        );
    }

    #[test]
    fn dashboard_selection_wraps_forward_and_backward() {
        let app = app_for_tests(ShellMode::Dashboard);

        let forward = app.update(AppMessage::DashboardSelectionNext);
        assert_eq!(
            forward,
            vec![
                AppAction::SetDashboardSelection(Some(1)),
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Quiet,
                    app.dashboard.detail_mode,
                    "selection moved"
                )),
            ]
        );

        let mut wrapped = app_for_tests(ShellMode::Dashboard);
        wrapped.dashboard.list_state.select(Some(0));
        let backward = wrapped.update(AppMessage::DashboardSelectionPrevious);
        assert_eq!(
            backward,
            vec![
                AppAction::SetDashboardSelection(Some(2)),
                AppAction::SetStatus(wrapped.dashboard_status_for(
                    StatusKind::Quiet,
                    wrapped.dashboard.detail_mode,
                    "selection moved"
                )),
            ]
        );
    }

    #[test]
    fn dashboard_toggle_expand_reveals_owned_child_rows() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));

        app.apply(AppAction::ToggleDashboardExpanded);

        assert!(app.dashboard.expanded_folders.contains("Second"));
        assert_eq!(app.dashboard.rows.len(), 4);
        assert_eq!(app.dashboard.rows[2].folder, "Second_Config");
    }

    #[test]
    fn expand_and_collapse_all_relationships_updates_tree_rows() {
        let mut app = app_for_tests(ShellMode::Dashboard);

        app.apply(AppAction::ExpandAllDashboardRelationships);
        assert!(app.dashboard.expanded_folders.contains("Second"));
        assert_eq!(app.dashboard.rows.len(), 4);
        assert_eq!(
            app.dashboard.rows[2].child_connector,
            Some(DashboardChildConnector::Last)
        );

        app.apply(AppAction::CollapseAllDashboardRelationships);
        assert!(app.dashboard.expanded_folders.is_empty());
        assert_eq!(app.dashboard.rows.len(), 3);
    }

    #[test]
    fn expand_all_preserves_logical_selection_when_rows_are_inserted_above() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let third_index = app
            .dashboard
            .rows
            .iter()
            .position(|row| row.folder == "Third")
            .expect("third row");
        app.dashboard.list_state.select(Some(third_index));

        app.apply(AppAction::ExpandAllDashboardRelationships);

        assert_eq!(
            app.dashboard
                .selected_item()
                .map(|item| item.folder.as_str()),
            Some("Third")
        );
        assert_eq!(app.dashboard.list_state.selected(), Some(0));
    }

    #[test]
    fn child_row_prefixes_match_sibling_position() {
        let mid = DashboardRow {
            key: super::DashboardRowKey::OwnedChild {
                parent_folder: "Parent".to_string(),
                child_folder: "One".to_string(),
            },
            name: "One".to_string(),
            folder: "One".to_string(),
            kind: super::DashboardRowKind::OwnedChild {
                parent_folder: "Parent".to_string(),
            },
            expandable: false,
            expanded: false,
            child_connector: Some(DashboardChildConnector::Mid),
        };
        let last = DashboardRow {
            key: super::DashboardRowKey::OwnedChild {
                parent_folder: "Parent".to_string(),
                child_folder: "Two".to_string(),
            },
            name: "Two".to_string(),
            folder: "Two".to_string(),
            kind: super::DashboardRowKind::OwnedChild {
                parent_folder: "Parent".to_string(),
            },
            expandable: false,
            expanded: false,
            child_connector: Some(DashboardChildConnector::Last),
        };

        assert_eq!(child_row_prefix(&mid), "├ ");
        assert_eq!(child_row_prefix(&last), "└ ");
        assert_eq!(child_row_detail_prefix(&mid), "  │  ");
        assert_eq!(child_row_detail_prefix(&last), "     ");
    }

    #[test]
    fn collapsing_selected_child_returns_selection_to_parent() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));

        app.apply(AppAction::CollapseDashboardExpanded);

        assert!(!app.dashboard.expanded_folders.contains("Second"));
        assert_eq!(app.dashboard.list_state.selected(), Some(1));
        assert_eq!(
            app.dashboard
                .selected_item()
                .map(|item| item.folder.as_str()),
            Some("Second")
        );
    }

    #[test]
    fn toggle_selection_marks_parent_rows_for_bulk_actions() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));

        app.apply(AppAction::ToggleDashboardSelection);
        assert!(app.dashboard.is_parent_selected("Second"));

        app.apply(AppAction::ToggleDashboardSelection);
        assert!(!app.dashboard.is_parent_selected("Second"));
    }

    #[test]
    fn toggling_selected_child_is_a_noop_for_bulk_selection() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));

        app.apply(AppAction::ToggleDashboardSelection);

        assert!(!app.dashboard.is_parent_selected("Second"));
        assert_eq!(app.dashboard.selected_parent_count(), 0);
    }

    #[test]
    fn select_all_and_clear_selection_update_parent_set() {
        let mut app = app_for_tests(ShellMode::Dashboard);

        app.apply(AppAction::SelectAllDashboardParents);
        assert_eq!(app.dashboard.selected_parent_count(), 3);
        assert!(app.dashboard.is_parent_selected("First"));
        assert!(app.dashboard.is_parent_selected("Second"));
        assert!(app.dashboard.is_parent_selected("Third"));

        app.apply(AppAction::ClearDashboardSelection);
        assert_eq!(app.dashboard.selected_parent_count(), 0);
    }

    #[test]
    fn requesting_delete_without_selection_targets_the_current_addon() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(0));
        let current = app.dashboard.selected_row().cloned().expect("current row");
        assert_eq!(app.dashboard.selected_parent_count(), 0);

        let actions = app.update(AppMessage::DashboardRequestDelete);

        assert_eq!(
            actions,
            vec![
                AppAction::SetPendingDelete(Some(vec![current.folder.clone()])),
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Quiet,
                    app.dashboard.detail_mode,
                    &format!("asked to delete {}", current.name),
                )),
            ]
        );
    }

    #[test]
    fn requesting_delete_with_no_rows_sets_status_only() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(None);

        let actions = app.update(AppMessage::DashboardRequestDelete);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                StatusKind::Note,
                app.dashboard.detail_mode,
                "Move to an addon first",
            ))]
        );
    }

    #[test]
    fn requesting_delete_with_selection_sets_pending_delete() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardSelection);

        let actions = app.update(AppMessage::DashboardRequestDelete);

        assert_eq!(
            actions,
            vec![
                AppAction::SetPendingDelete(Some(vec!["Second".to_string()])),
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Quiet,
                    app.dashboard.detail_mode,
                    "asked to delete 1 addon",
                )),
            ]
        );
    }

    #[test]
    fn cancelling_pending_delete_clears_pending_state() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard
            .set_pending_delete_folders(Some(vec!["Second".to_string()]));

        app.apply(AppAction::SetPendingDelete(None));

        assert!(app.dashboard.pending_delete_folders().is_none());
    }

    #[test]
    fn update_on_a_folder_row_updates_the_addon_it_belongs_to() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Overview;
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));
        assert_eq!(
            app.dashboard.actionable_parent_targets(),
            Ok(vec![app.dashboard.items[1].folder.clone()])
        );

        // With no addon row at all there is nothing to act on.
        app.dashboard.list_state.select(None);
        let actions = app.update(AppMessage::DashboardRunUpdateSelected);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                StatusKind::Note,
                DetailMode::Overview,
                "Move to an addon first",
            ))]
        );
    }

    #[test]
    fn update_selected_starts_background_refresh_for_selected_parents() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Overview;
        app.effective_addon_dir = Some(PathBuf::from(
            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardSelection);

        let actions = app.update(AppMessage::DashboardRunUpdateSelected);

        assert_eq!(
            actions,
            vec![
                AppAction::SetDashboardUpdateInProgress(true),
                AppAction::StartDashboardUpdateSelected {
                    addon_dir: PathBuf::from(
                        "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
                    ),
                    folders: vec!["Second".to_string()],
                    wago_api_key: Some("test-wago-key".to_string()),
                    check_interval_secs: app.config_pane.draft.check_interval_secs,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Overview,
                    "Updating 1 addon",
                )),
            ]
        );
    }

    #[test]
    fn check_selected_starts_background_check_for_focused_parent() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Overview;
        app.dashboard.list_state.select(Some(1));

        let actions = app.update(AppMessage::DashboardRunCheckSelected);

        assert_eq!(
            actions,
            vec![
                AppAction::StartDashboardCheckSelected {
                    folders: vec!["Second".to_string()],
                    wago_api_key: Some("test-wago-key".to_string()),
                    check_interval_secs: app.config_pane.draft.check_interval_secs,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    StatusKind::Working,
                    DetailMode::Overview,
                    "Checking 1 addon for updates",
                )),
            ]
        );
    }

    #[test]
    fn delete_selected_addons_removes_parent_owned_folders_and_state_rows() {
        let temp = tempdir().expect("temp dir");
        let addon_dir = temp.path().join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(addon_dir.join("Second")).expect("create parent dir");
        fs::create_dir_all(addon_dir.join("Second_Config")).expect("create child dir");

        let state_db_file = temp.path().join("state.sqlite");
        let trash_dir = temp.path().join("trash");
        let mut database = StateDatabase::open(&state_db_file).expect("open state db");

        let mut managed = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        managed.version = Some("2.0.0".to_string());
        managed.set_managed_owned_folders(vec![OwnedFolder {
            name: "Second_Config".to_string(),
        }]);
        database
            .record_managed_addon(&managed)
            .expect("record managed addon");

        let outcome = super::delete_selected_addons(
            &state_db_file,
            &addon_dir,
            &trash_dir,
            &["Second".to_string()],
        )
        .expect("delete selected addons");

        assert_eq!(outcome.deleted_parents, 1);
        assert_eq!(outcome.deleted_folders, 2);
        assert!(outcome.undo_delete.is_some());
        assert!(outcome.sync.addons.is_empty());
        assert!(!addon_dir.join("Second").exists());
        assert!(!addon_dir.join("Second_Config").exists());
    }

    #[test]
    fn undo_deleted_addons_restores_parent_owned_folders_and_state_rows() {
        let temp = tempdir().expect("temp dir");
        let addon_dir = temp.path().join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        let parent_dir = addon_dir.join("Second");
        let child_dir = addon_dir.join("Second_Config");
        fs::create_dir_all(&parent_dir).expect("create parent dir");
        fs::create_dir_all(&child_dir).expect("create child dir");
        fs::write(parent_dir.join("Second.toc"), "## Title: Second\n").expect("write parent toc");
        fs::write(
            child_dir.join("Second_Config.toc"),
            "## Title: Second Config\n",
        )
        .expect("write child toc");

        let state_db_file = temp.path().join("state.sqlite");
        let trash_dir = temp.path().join("trash");
        let mut database = StateDatabase::open(&state_db_file).expect("open state db");

        let mut managed = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        managed.version = Some("2.0.0".to_string());
        managed.set_managed_owned_folders(vec![OwnedFolder {
            name: "Second_Config".to_string(),
        }]);
        database
            .record_managed_addon(&managed)
            .expect("record managed addon");

        let delete_outcome = super::delete_selected_addons(
            &state_db_file,
            &addon_dir,
            &trash_dir,
            &["Second".to_string()],
        )
        .expect("delete selected addons");
        let undo = delete_outcome.undo_delete.expect("undo state");

        let restore_outcome =
            super::undo_deleted_addons(&state_db_file, &undo).expect("undo deleted addons");

        assert_eq!(restore_outcome.restored_parents, 1);
        assert_eq!(restore_outcome.restored_folders, 2);
        assert!(addon_dir.join("Second").exists());
        assert!(addon_dir.join("Second_Config").exists());

        let restored = StateDatabase::open(&state_db_file)
            .expect("open restored db")
            .list_addons()
            .expect("list addons");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].folder, "Second");
        assert!(restored[0].has_authoritative_owned_folders());
        assert_eq!(restored[0].owned_folders.len(), 1);
    }

    #[test]
    fn dashboard_delete_finished_triggers_fresh_sync_actions() {
        let app = app_for_tests(ShellMode::Dashboard);
        let sync = AddonScanOutcome {
            path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
            summary: ScanSummary {
                scanned_addons: 1,
                upserted_addons: 1,
                removed_addons: 0,
                ..Default::default()
            },
            addons: vec![AddonRecord::new("First", "First", SourceKind::Manual)],
            drift_report: DriftReport::empty(),
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardDeleteFinished(Ok(DashboardDeleteOutcome {
                deleted_parents: 1,
                deleted_folders: 2,
                undo_delete: None,
                sync: sync.clone(),
            })),
        ));

        assert_eq!(actions.len(), 7);
        assert_eq!(actions[6], AppAction::SetDashboardDeleteInProgress(false));
        assert!(matches!(actions[0], AppAction::ReplaceDashboardAddons(_)));
        assert_eq!(actions[1], AppAction::SetPendingDelete(None));
        assert!(matches!(
            actions[2],
            AppAction::SetDashboardUndoDelete(None)
        ));
        assert!(matches!(
            actions[3],
            AppAction::SetDashboardDriftReport(Some(_))
        ));
        assert_eq!(
            actions[4],
            AppAction::CompleteAddonScan {
                path: sync.path,
                summary: sync.summary,
            }
        );
    }

    #[test]
    fn dashboard_undo_finished_clears_undo_state_and_triggers_fresh_sync_actions() {
        let app = app_for_tests(ShellMode::Dashboard);
        let sync = AddonScanOutcome {
            path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
            summary: ScanSummary {
                scanned_addons: 1,
                upserted_addons: 1,
                removed_addons: 0,
                ..Default::default()
            },
            addons: vec![AddonRecord::new("Second", "Second", SourceKind::GitHub)],
            drift_report: DriftReport::empty(),
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardUndoFinished(Ok(super::DashboardUndoOutcome {
                restored_parents: 1,
                restored_folders: 2,
                sync: sync.clone(),
            })),
        ));

        assert_eq!(actions.len(), 6);
        assert!(matches!(actions[0], AppAction::ReplaceDashboardAddons(_)));
        assert_eq!(actions[1], AppAction::SetDashboardUndoInProgress(false));
        assert_eq!(actions[2], AppAction::SetDashboardUndoDelete(None));
        assert!(matches!(
            actions[3],
            AppAction::SetDashboardDriftReport(Some(_))
        ));
        assert_eq!(
            actions[4],
            AppAction::CompleteAddonScan {
                path: sync.path,
                summary: sync.summary,
            }
        );
        assert!(matches!(actions[5], AppAction::SetStatus(_)));
    }

    #[test]
    fn dashboard_update_finished_triggers_fresh_sync_actions() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Update;
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardSelection);

        let sync = AddonScanOutcome {
            path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
            summary: ScanSummary {
                scanned_addons: 1,
                upserted_addons: 1,
                removed_addons: 0,
                ..Default::default()
            },
            addons: vec![AddonRecord::new("Second", "Second", SourceKind::GitHub)],
            drift_report: DriftReport::empty(),
        };

        let refresh = LiveUpdateSummary {
            target_addons: 1,
            updated_addons: 1,
            up_to_date: 0,
            skipped_manual: 0,
            skipped_no_api_key: 0,
            remote_older: 0,
            errors: 0,
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardUpdateFinished(Ok(DashboardUpdateOutcome {
                summary: refresh,
                updated_folders: vec!["Second".to_string()],
                failed_folders: Vec::new(),
                updated_addon_name: Some("Second".to_string()),
                warnings: Vec::new(),
                sync: sync.clone(),
            })),
        ));

        assert_eq!(actions.len(), 8);
        assert!(matches!(actions[0], AppAction::ReplaceDashboardAddons(_)));
        assert!(matches!(
            actions[1],
            AppAction::SetDashboardDriftReport(Some(_))
        ));
        assert_eq!(
            actions[2],
            AppAction::CompleteAddonScan {
                path: sync.path,
                summary: sync.summary,
            }
        );
        assert_eq!(actions[3], AppAction::SetDashboardUpdateInProgress(false));
        assert_eq!(actions[4], AppAction::SetDashboardJobUi(None));
        assert!(matches!(
            actions[5],
            AppAction::SetDashboardUpdateSummary(Some(_))
        ));
        assert_eq!(
            actions[6],
            AppAction::SetDashboardUpdateResults {
                updated: vec!["Second".to_string()],
                failed: Vec::new(),
            }
        );
        assert!(matches!(actions[7], AppAction::SetStatus(_)));
    }

    #[test]
    fn dashboard_update_finished_uses_human_status_when_nothing_is_updateable() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Update;
        app.dashboard.list_state.select(Some(0));
        app.apply(AppAction::ToggleDashboardSelection);

        let sync = AddonScanOutcome {
            path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
            summary: ScanSummary {
                scanned_addons: 3,
                upserted_addons: 3,
                removed_addons: 0,
                ..Default::default()
            },
            addons: vec![
                AddonRecord::new("First", "First", SourceKind::Manual),
                AddonRecord::new("Second", "Second", SourceKind::GitHub),
            ],
            drift_report: DriftReport::empty(),
        };

        let refresh = LiveUpdateSummary {
            target_addons: 1,
            updated_addons: 0,
            up_to_date: 0,
            skipped_manual: 1,
            skipped_no_api_key: 0,
            remote_older: 0,
            errors: 0,
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardUpdateFinished(Ok(DashboardUpdateOutcome {
                summary: refresh,
                updated_folders: Vec::new(),
                failed_folders: Vec::new(),
                updated_addon_name: None,
                warnings: Vec::new(),
                sync,
            })),
        ));

        assert_eq!(
            actions[7],
            AppAction::SetStatus(app.dashboard_status_for(
                StatusKind::Done,
                DetailMode::Update,
                "Nothing was updated · 1 not linked",
            ))
        );
    }

    #[test]
    fn dashboard_update_status_uses_single_addon_name_when_available() {
        let outcome = DashboardUpdateOutcome {
            summary: LiveUpdateSummary {
                target_addons: 1,
                updated_addons: 1,
                up_to_date: 0,
                skipped_manual: 0,
                skipped_no_api_key: 0,
                remote_older: 0,
                errors: 0,
            },
            updated_folders: vec!["WeakAuras".to_string()],
            failed_folders: Vec::new(),
            updated_addon_name: Some("WeakAuras".to_string()),
            warnings: Vec::new(),
            sync: AddonScanOutcome {
                path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
                summary: ScanSummary {
                    scanned_addons: 1,
                    upserted_addons: 1,
                    removed_addons: 0,
                    ..Default::default()
                },
                addons: vec![],
                drift_report: DriftReport::empty(),
            },
        };

        assert_eq!(
            super::dashboard_update_status_message(&outcome, &[]),
            "Updated WeakAuras"
        );
    }

    #[test]
    fn dashboard_update_status_names_the_addons_that_failed() {
        let summary = |updated, errors| LiveUpdateSummary {
            target_addons: updated + errors,
            updated_addons: updated,
            up_to_date: 0,
            skipped_manual: 0,
            skipped_no_api_key: 0,
            remote_older: 0,
            errors,
        };
        let outcome = |updated, errors| DashboardUpdateOutcome {
            summary: summary(updated, errors),
            updated_folders: Vec::new(),
            failed_folders: Vec::new(),
            updated_addon_name: None,
            warnings: Vec::new(),
            sync: AddonScanOutcome {
                path: PathBuf::from("AddOns"),
                summary: ScanSummary::default(),
                addons: vec![],
                drift_report: DriftReport::empty(),
            },
        };
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();

        assert_eq!(
            super::dashboard_update_status_message(&outcome(3, 1), &names(&["Pawn"])),
            "Updated 3 addons · 1 failed: Pawn"
        );
        assert_eq!(
            super::dashboard_update_status_message(
                &outcome(2, 5),
                &names(&["Pawn", "Hekili", "TomTom", "BugSack", "OmniCC"])
            ),
            "Updated 2 addons · 5 failed: Pawn, Hekili, TomTom and 2 more"
        );
        assert_eq!(
            super::dashboard_update_status_message(&outcome(0, 2), &names(&["Pawn", "Hekili"])),
            "Couldn't update Pawn and Hekili"
        );
    }

    #[test]
    fn dashboard_check_status_humanizes_recently_checked_single_target() {
        assert_eq!(
            super::dashboard_check_status_message(&["BigWigs".to_string()], 1, 0, 1, 0),
            "Checked BigWigs"
        );
    }

    #[test]
    fn dashboard_job_progress_event_sets_job_ui_state() {
        let app = app_for_tests(ShellMode::Dashboard);
        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardJobProgress(DashboardJobUiState {
                kind: DashboardJobKind::Update,
                current_index: 2,
                total: 3,
                current_addon: "Second".to_string(),
            }),
        ));

        assert!(matches!(actions[0], AppAction::SetDashboardJobUi(Some(_))));
        assert!(matches!(actions[1], AppAction::SetStatus(_)));
    }

    #[test]
    fn active_update_job_marks_its_row_updating() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.set_job_ui(Some(DashboardJobUiState {
            kind: DashboardJobKind::Update,
            current_index: 1,
            total: 2,
            current_addon: "Second".to_string(),
        }));
        let row = app
            .dashboard
            .rows
            .iter()
            .find(|row| row.folder == "Second")
            .expect("second row");
        let item = app
            .dashboard
            .items
            .iter()
            .find(|item| item.folder == row.folder)
            .expect("second item");
        assert_eq!(app.dashboard_row_status(item).label("⠋"), "⠋ Updating…");
    }

    #[test]
    fn replacing_addons_preserves_selected_row_and_scroll_offset() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let third_index = app
            .dashboard
            .rows
            .iter()
            .position(|row| row.folder == "Third")
            .expect("third row");
        app.dashboard.list_state = TableState::default()
            .with_offset(third_index)
            .with_selected(Some(third_index));

        let updated = vec![
            AddonRecord::new("First", "First", SourceKind::Manual),
            AddonRecord::new("Second", "Second", SourceKind::GitHub),
            AddonRecord::new("Third", "Third", SourceKind::Wago),
        ];

        app.apply(AppAction::ReplaceDashboardAddons(updated));

        assert_eq!(app.dashboard.list_state.selected(), Some(0));
        assert_eq!(app.dashboard.list_state.offset(), 0);
        assert_eq!(
            app.dashboard
                .selected_item()
                .map(|item| item.folder.as_str()),
            Some("Third")
        );
    }

    #[test]
    fn replacing_addons_drops_stale_selected_parents() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SelectAllDashboardParents);

        let mut only_second = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        only_second.version = Some("2.1.0".to_string());
        app.apply(AppAction::ReplaceDashboardAddons(vec![only_second]));

        assert_eq!(app.dashboard.selected_parent_count(), 1);
        assert!(app.dashboard.is_parent_selected("Second"));
        assert!(!app.dashboard.is_parent_selected("First"));
        assert!(!app.dashboard.is_parent_selected("Third"));
    }

    #[test]
    fn last_scan_drift_lines_stay_single_line_when_report_has_counts() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.set_drift_report(Some(DriftReport {
            imported_disk_only_folders: vec!["WeakAuras".to_string()],
            removed_missing_records: vec!["DeadAddon".to_string()],
            orphaned_owned_children_on_disk: vec![OwnedChildDrift {
                parent_folder: "DBM-Core".to_string(),
                child_folder: "DBM-Naxx".to_string(),
            }],
            parent_missing_owned_children: HashMap::new(),
        }));

        let lines = app.last_scan_drift_lines();

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].to_string(),
            "Last scan drift: imported 1, removed 1, orphaned children 1."
        );
    }

    #[test]
    fn summarize_owned_folders_limits_long_relationship_lists() {
        let folders = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
        ];

        assert_eq!(summarize_owned_folders(&folders), "A, B, C, D +1 more");
    }

    #[test]
    fn github_version_label_uses_short_commit() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        addon.git_commit = Some("1234567890abcdef".to_string());
        let dashboard = DashboardState::from_addons(vec![addon]);

        let item = dashboard.items.first().expect("dashboard item");
        assert_eq!(dashboard_item_version_label(item), "1234567");
    }

    #[test]
    fn github_version_label_uses_unknown_when_only_placeholder_metadata_is_available() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::GitHub);
        addon.version = Some("@project-version@".to_string());
        addon.remote_version = Some("abcdef1234567890".to_string());
        let dashboard = DashboardState::from_addons(vec![addon]);

        let item = dashboard.items.first().expect("dashboard item");
        assert_eq!(dashboard_item_version_label(item), "unknown");
    }

    fn item_for(addon: AddonRecord) -> DashboardItem {
        DashboardState::from_addons(vec![addon])
            .items
            .into_iter()
            .next()
            .expect("dashboard item")
    }

    fn versioned(source: SourceKind, version: &str, remote: Option<&str>) -> AddonRecord {
        let mut addon = AddonRecord::new("Addon", "Addon", source);
        addon.version = Some(version.to_string());
        addon.remote_version = remote.map(ToString::to_string);
        addon
    }

    #[test]
    fn status_words_name_each_state() {
        use super::render_dashboard::{RowStatus, dashboard_item_row_status};
        let label = |addon: AddonRecord| dashboard_item_row_status(&item_for(addon)).label("⠋");

        assert_eq!(
            label(AddonRecord::new("Bug", "Bug", SourceKind::Manual)),
            "· Not linked"
        );
        assert_eq!(
            label(versioned(SourceKind::Wago, "12.0.8", Some("12.0.10"))),
            "↑ Update ready"
        );
        assert_eq!(
            label(versioned(SourceKind::Wago, "5.21.1", Some("5.21.1"))),
            "✓ Up to date"
        );
        assert_eq!(
            label(versioned(
                SourceKind::WowInterface,
                "1.6.15",
                Some("1.6.14")
            )),
            "✓ Newer than source"
        );
        assert_eq!(
            label(versioned(SourceKind::Wago, "5.21.1", None)),
            "· Not checked yet"
        );
        let words = [
            (RowStatus::CheckFailed, "! Check failed"),
            (RowStatus::Checking, "⠋ Checking…"),
            (RowStatus::Updating, "⠋ Updating…"),
            (RowStatus::Waiting, "· Waiting"),
            (RowStatus::Updated, "✓ Updated"),
        ];
        for (status, word) in words {
            assert_eq!(status.label("⠋"), word);
        }
    }

    #[test]
    fn version_cell_shows_old_and_new_only_when_an_update_is_ready() {
        use super::render_dashboard::VersionCell;
        let text =
            |addon: AddonRecord, wide: bool| VersionCell::for_item(&item_for(addon), wide).text();

        assert_eq!(
            text(AddonRecord::new("Bug", "Bug", SourceKind::Manual), true),
            "unknown"
        );
        assert_eq!(
            text(versioned(SourceKind::Wago, "5.21.1", Some("5.21.1")), true),
            "5.21.1"
        );
        assert_eq!(
            text(versioned(SourceKind::Wago, "12.0.8", Some("12.0.10")), true),
            "12.0.8 → 12.0.10"
        );
        assert_eq!(
            text(
                versioned(SourceKind::WowInterface, "1.6.15", Some("1.6.14")),
                true
            ),
            "1.6.15"
        );

        // GitHub: the version from the addon's own files, and "new build".
        let mut github = versioned(SourceKind::GitHub, "5.20.4", Some("b83e51d0aa"));
        github.git_commit = Some("4f1c9a7123".to_string());
        assert_eq!(text(github.clone(), true), "5.20.4 → new build");
        assert_eq!(text(github.clone(), false), "5.20.4 → newer");
        github.remote_version = Some("4f1c9a7123".to_string());
        assert_eq!(text(github.clone(), true), "5.20.4");
        // Without a usable version of its own, the short commit stands in.
        github.version = Some("@project-version@".to_string());
        assert_eq!(text(github, true), "4f1c9a7");
    }

    #[test]
    fn source_names_are_full_when_wide_and_empty_when_not_linked() {
        use super::render_dashboard::dashboard_source_name;
        let wide: Vec<_> = [
            SourceKind::Wago,
            SourceKind::GitHub,
            SourceKind::Tukui,
            SourceKind::WowInterface,
            SourceKind::Manual,
        ]
        .into_iter()
        .map(|source| dashboard_source_name(source, true))
        .collect();
        assert_eq!(wide, ["Wago.io", "GitHub", "TukUI", "WoWInterface", ""]);
        assert_eq!(dashboard_source_name(SourceKind::Wago, false), "Wago");
        assert_eq!(
            dashboard_source_name(SourceKind::WowInterface, false),
            "WoWI"
        );
    }

    #[test]
    fn list_columns_match_the_design_widths() {
        use super::render_dashboard::ListColumns;
        let wide = ListColumns::for_width(120);
        assert_eq!(
            (
                wide.name,
                wide.status,
                wide.version,
                wide.author,
                wide.source
            ),
            (33, 21, 22, 26, 13)
        );
        let narrow = ListColumns::for_width(80);
        assert_eq!(
            (
                narrow.name,
                narrow.status,
                narrow.version,
                narrow.author,
                narrow.source
            ),
            (27, 21, 19, 0, 8)
        );
        assert!(!ListColumns::for_width(109).wide);
        assert!(ListColumns::for_width(110).wide);

        // Extra width: Addon to 40, then Author to 36, then Version to 26,
        // then empty space. Nothing stretches to fill.
        let widths = |width: u16| {
            let c = ListColumns::for_width(width);
            (c.name, c.status, c.version, c.author, c.source, c.total())
        };
        assert_eq!(widths(127), (40, 21, 22, 26, 13, 127));
        assert_eq!(widths(137), (40, 21, 22, 36, 13, 137));
        assert_eq!(widths(138), (40, 21, 23, 36, 13, 138));
        assert_eq!(widths(141), (40, 21, 26, 36, 13, 141));
        assert_eq!(widths(200), (40, 21, 26, 36, 13, 141));
        assert_eq!(widths(100), (40, 21, 26, 0, 8, 100));
        assert_eq!(widths(160).5, 141);
    }

    #[test]
    fn path_is_shortened_in_the_middle_by_whole_folders() {
        use super::render_dashboard::shorten_path_middle;
        let path = "~/Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns";
        assert_eq!(shorten_path_middle(path, 200), path);
        assert_eq!(
            shorten_path_middle(path, 76),
            "~/Games/world-of-warcraft/…/World of Warcraft/_retail_/Interface/AddOns"
        );
        assert_eq!(
            shorten_path_middle(path, 40),
            "~/Games/…/_retail_/Interface/AddOns"
        );
        assert!(shorten_path_middle(path, 12).chars().count() <= 12);
    }

    fn answer_text(app: &App, now: OffsetDateTime) -> String {
        app.dashboard_answer_spans(now)
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn header_answer_line_counts_updates_and_says_when_it_last_checked() {
        let now = OffsetDateTime::now_utc();
        let checked = |minutes: i64, addon: AddonRecord| {
            let mut addon = addon;
            addon.last_checked_at = Some(now - Duration::minutes(minutes));
            addon
        };
        let mut app = app_for_tests(ShellMode::Dashboard);

        app.dashboard = DashboardState::from_addons(vec![
            versioned(SourceKind::Wago, "1.0", Some("1.0")),
            versioned(SourceKind::Wago, "2.0", None),
        ]);
        assert_eq!(answer_text(&app, now), "2 addons · not checked yet");

        app.dashboard = DashboardState::from_addons(vec![
            checked(5, versioned(SourceKind::Wago, "1.0", Some("1.0"))),
            checked(0, versioned(SourceKind::Wago, "2.0", Some("2.0"))),
        ]);
        assert_eq!(
            answer_text(&app, now),
            "2 addons · all up to date · checked just now"
        );

        app.dashboard = DashboardState::from_addons(vec![
            checked(5, versioned(SourceKind::Wago, "1.0", Some("1.1"))),
            checked(9, versioned(SourceKind::Wago, "2.0", Some("2.0"))),
        ]);
        assert_eq!(
            answer_text(&app, now),
            "2 addons · 1 update ready · checked 5 min ago"
        );

        let mut many = (0..18)
            .map(|index| {
                let mut addon = checked(
                    180,
                    versioned(
                        SourceKind::Wago,
                        "1.0",
                        Some(if index < 4 { "1.1" } else { "1.0" }),
                    ),
                );
                addon.name = format!("Addon {index}");
                addon.folder = format!("Addon{index}");
                addon
            })
            .collect::<Vec<_>>();
        many.push(AddonRecord::new("Bug", "Bug", SourceKind::Manual));
        app.dashboard = DashboardState::from_addons(many);
        assert_eq!(
            answer_text(&app, now),
            "19 addons · 4 updates ready · checked 3 hours ago"
        );

        app.dashboard =
            DashboardState::from_addons(vec![AddonRecord::new("Bug", "Bug", SourceKind::Manual)]);
        assert_eq!(answer_text(&app, now), "1 addon · none linked yet");
    }

    #[test]
    fn done_and_note_messages_fade_after_about_four_seconds() {
        for kind in [StatusKind::Done, StatusKind::Note] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let before = Instant::now();
            app.apply(AppAction::SetStatus(app.dashboard_status_for(
                kind,
                DetailMode::Overview,
                "sorted by name",
            )));
            let flash = app.shell_ui.status_flash.clone().expect("message");
            let expires = flash.expires_at.expect("fades");
            assert!(expires >= before + StdDuration::from_millis(3900));
            assert!(expires <= Instant::now() + StdDuration::from_millis(4100));
            assert!(app.status_flash_is_live(&flash, Instant::now()));
            assert!(!app.status_flash_is_live(&flash, expires));
        }
    }

    #[test]
    fn problems_stay_until_the_next_key_press() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Problem,
            DetailMode::Overview,
            "check failed: offline",
        )));
        let flash = app.shell_ui.status_flash.clone().expect("problem");
        assert_eq!(flash.expires_at, None);
        assert!(app.status_flash_is_live(&flash, Instant::now() + StdDuration::from_secs(600)));

        app.clear_problem_on_key_press(&TerminalEvent::Tick);
        assert!(app.shell_ui.status_flash.is_some());
        app.clear_problem_on_key_press(&TerminalEvent::Key(KeyEvent::from(KeyCode::Down)));
        assert!(app.shell_ui.status_flash.is_none());

        // A key press leaves a done message alone; it fades on its own.
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Done,
            DetailMode::Overview,
            "updated 2 addons",
        )));
        app.clear_problem_on_key_press(&TerminalEvent::Key(KeyEvent::from(KeyCode::Down)));
        assert!(app.shell_ui.status_flash.is_some());
    }

    #[test]
    fn quiet_messages_never_show() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Quiet,
            DetailMode::Overview,
            "selection moved",
        )));
        assert!(app.shell_ui.status_flash.is_none());
        let status = line_text(&app.dashboard_status_line(120));
        assert!(!status.contains("selection moved"), "{status}");
    }

    #[test]
    fn a_message_that_offers_a_key_stays_while_the_key_works() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(
            app.dashboard_status_for(StatusKind::Done, DetailMode::Overview, "deleted 1 addon")
                .offering(StatusOffer::UndoDelete),
        ));
        let late = Instant::now() + StdDuration::from_secs(60);
        let flash = app.shell_ui.status_flash.clone().expect("message");
        // No undo available: it fades like any done message, without the key.
        assert!(!app.status_flash_is_live(&flash, late));
        assert!(!line_text(&app.dashboard_status_line(120)).contains("undo delete"));

        app.undo_delete = Some(DashboardUndoDeleteState {
            batch_dir: PathBuf::from("trash"),
            addon_dir: PathBuf::from("AddOns"),
            deleted_parent_folders: vec!["Addon".to_string()],
            tracked_records: Vec::new(),
            moved_entries: Vec::new(),
        });
        assert!(app.status_flash_is_live(&flash, late));
        let status = line_text(&app.dashboard_status_line(120));
        assert!(
            status.contains("✓ Deleted 1 addon · z undo delete"),
            "{status}"
        );

        // U after a check stays while an update is ready.
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "1.0", Some("1.1"))]);
        app.apply(AppAction::SetStatus(
            app.dashboard_status_for(StatusKind::Done, DetailMode::Overview, "check complete")
                .offering(StatusOffer::UpdateAll),
        ));
        let flash = app.shell_ui.status_flash.clone().expect("message");
        assert!(app.status_flash_is_live(&flash, late));
        let status = line_text(&app.dashboard_status_line(120));
        assert!(
            status.contains("✓ Check complete · 1 update ready · U updates them all"),
            "{status}"
        );
        app.dashboard.set_update_in_progress(true);
        assert!(!app.status_flash_is_live(&flash, late));
    }

    #[test]
    fn capital_u_and_c_route_to_update_all_and_check_all() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let shift = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::SHIFT);
        assert_eq!(
            app.messages_for_key(shift('U')),
            vec![AppMessage::DashboardRunUpdateAll]
        );
        assert_eq!(
            app.messages_for_key(shift('C')),
            vec![AppMessage::DashboardRunCheckAll]
        );
        // The pulse follows the key table: with nothing to update the key
        // line has no U, so there is nothing to pulse.
        assert_eq!(app.footer_hint_for_key(shift('U')), None);
        assert_eq!(
            app.footer_hint_for_key(shift('C')),
            Some(FooterHintId::CheckAll)
        );

        // While typing a filter, U and C are letters.
        press_key(&mut app, KeyCode::Char('/'));
        assert_eq!(
            app.messages_for_key(shift('U')),
            vec![AppMessage::DashboardFilterInputChar('U')]
        );
    }

    #[test]
    fn update_all_targets_only_addons_with_an_update_ready() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.effective_addon_dir = Some(PathBuf::from("C:\\Sandbox\\AddOns"));
        let mut ready = versioned(SourceKind::Wago, "1.0", Some("1.1"));
        ready.name = "Ready".to_string();
        ready.folder = "Ready".to_string();
        app.dashboard = DashboardState::from_addons(vec![
            ready,
            versioned(SourceKind::Wago, "2.0", Some("2.0")),
            AddonRecord::new("Bug", "Bug", SourceKind::Manual),
        ]);

        let actions = app.update(AppMessage::DashboardRunUpdateAll);
        let folders = actions.iter().find_map(|action| match action {
            AppAction::StartDashboardUpdateSelected { folders, .. } => Some(folders.clone()),
            _ => None,
        });
        assert_eq!(folders, Some(vec!["Ready".to_string()]));

        let actions = app.update(AppMessage::DashboardRunCheckAll);
        let folders = actions.iter().find_map(|action| match action {
            AppAction::StartDashboardCheckSelected { folders, .. } => Some(folders.clone()),
            _ => None,
        });
        let mut folders = folders.expect("check started");
        folders.sort();
        assert_eq!(folders, vec!["Addon".to_string(), "Ready".to_string()]);

        // Nothing ready: U says so instead of starting an empty run.
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "2.0", Some("2.0"))]);
        let actions = app.update(AppMessage::DashboardRunUpdateAll);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, AppAction::StartDashboardUpdateSelected { .. }))
        );
    }

    #[test]
    fn running_update_claims_success_only_when_the_run_reports_it() {
        use super::render_dashboard::RowStatus;
        let mut app = app_for_tests(ShellMode::Dashboard);
        let folders = [
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ];
        app.dashboard.begin_update_targets(&folders);
        let job = |kind: DashboardJobKind, index: usize, folder: &str| DashboardJobUiState {
            kind,
            current_index: index,
            total: 3,
            current_addon: folder.to_string(),
        };
        let status = |app: &App, folder: &str| {
            let item = app
                .dashboard
                .items
                .iter()
                .find(|item| item.folder == folder)
                .expect("item");
            app.dashboard_row_status(item)
        };
        // The preflight check reports progress too; it does not count as done.
        app.dashboard
            .set_job_ui(Some(job(DashboardJobKind::Check, 1, "First")));
        assert_eq!(status(&app, "First"), RowStatus::Checking);
        app.dashboard
            .set_job_ui(Some(job(DashboardJobKind::Check, 2, "Second")));
        assert_eq!(status(&app, "First"), RowStatus::Waiting);
        app.dashboard
            .set_job_ui(Some(job(DashboardJobKind::Update, 1, "First")));
        app.dashboard
            .set_job_ui(Some(job(DashboardJobKind::Update, 2, "Second")));
        // Moved past, result unknown: a neutral "Done", never "Updated".
        assert_eq!(status(&app, "First"), RowStatus::Done);
        assert_eq!(RowStatus::Done.label("⠋"), "· Done");
        assert_eq!(status(&app, "Second"), RowStatus::Updating);
        assert_eq!(status(&app, "Third"), RowStatus::Waiting);

        let line = line_text(&app.dashboard_status_line(120));
        assert!(line.contains("Updating 2 of 3 · Second"), "{line}");
        assert!(line.contains("█"), "{line}");
        let keys = line_text(&app.dashboard_key_line(120));
        assert!(
            keys.contains("LemonUp is updating. You can keep browsing."),
            "{keys}"
        );

        // The run reports one updated and one failed.
        app.apply(AppAction::SetDashboardJobUi(None));
        app.apply(AppAction::SetDashboardUpdateResults {
            updated: vec!["First".to_string()],
            failed: vec!["Second".to_string()],
        });
        assert_eq!(status(&app, "First"), RowStatus::Updated);
        assert_eq!(RowStatus::Updated.label("⠋"), "✓ Updated");
        assert_eq!(status(&app, "Second"), RowStatus::UpdateFailed);
        assert_eq!(RowStatus::UpdateFailed.label("⠋"), "! Update failed");
        assert_ne!(status(&app, "Third"), RowStatus::Done);

        // The next job clears the last run's results.
        app.dashboard
            .set_job_ui(Some(job(DashboardJobKind::Check, 1, "Third")));
        assert_ne!(status(&app, "First"), RowStatus::Updated);
        assert_ne!(status(&app, "Second"), RowStatus::UpdateFailed);
    }

    #[test]
    fn not_checked_yet_sorts_with_up_to_date_above_not_linked() {
        let mut ready = versioned(SourceKind::Wago, "1.0", Some("1.1"));
        ready.name = "Ready".to_string();
        ready.folder = "Ready".to_string();
        let mut current = versioned(SourceKind::Wago, "1.0", Some("1.0"));
        current.name = "Current".to_string();
        current.folder = "Current".to_string();
        let mut unchecked = versioned(SourceKind::Wago, "1.0", None);
        unchecked.name = "Unchecked".to_string();
        unchecked.folder = "Unchecked".to_string();
        let mut newer = versioned(SourceKind::Wago, "1.6.15", Some("1.6.14"));
        newer.name = "Newer".to_string();
        newer.folder = "Newer".to_string();
        let manual = AddonRecord::new("Manual", "Manual", SourceKind::Manual);

        let dashboard = DashboardState::from_addons(vec![manual, newer, unchecked, current, ready]);
        let order: Vec<&str> = dashboard
            .items
            .iter()
            .map(|item| item.folder.as_str())
            .collect();
        assert_eq!(order[0], "Ready");
        let position = |folder: &str| order.iter().position(|item| *item == folder).unwrap();
        assert!(position("Unchecked") < position("Newer"), "{order:?}");
        assert!(position("Current") < position("Newer"), "{order:?}");
        assert_eq!(order.last(), Some(&"Manual"), "{order:?}");
    }

    #[test]
    fn selection_key_line_only_offers_keys_that_work() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SelectAllDashboardParents);
        let keys = line_text(&app.dashboard_key_line(120));
        // `a` clears a selection, so "select all" would be untrue.
        assert!(!keys.contains("select all"), "{keys}");
        assert!(keys.contains("space deselect"), "{keys}");
        assert!(keys.contains("esc clear selection"), "{keys}");
        assert!(!keys.contains(" s "), "{keys}");

        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "f");
        press_key(&mut app, KeyCode::Enter);
        let keys = line_text(&app.dashboard_key_line(120));
        assert!(keys.contains("esc clear filter"), "{keys}");
        press_key(&mut app, KeyCode::Esc);
        assert!(!app.dashboard.has_filter());

        // With nothing to update, settings is `,` until the `s` alias exists.
        app.dashboard =
            DashboardState::from_addons(vec![versioned(SourceKind::Wago, "1.0", Some("1.0"))]);
        let keys = line_text(&app.dashboard_key_line(120));
        assert!(keys.contains(", settings"), "{keys}");
        assert!(!keys.contains("s settings"), "{keys}");
    }

    #[test]
    fn dashboard_frame_renders_at_every_size() {
        let now = OffsetDateTime::now_utc();
        for (width, height) in [(80u16, 24u16), (120, 30), (138, 36)] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let mut addons = vec![
                versioned(SourceKind::Wago, "12.0.8", Some("12.0.10")),
                versioned(SourceKind::GitHub, "5.20.4", Some("b83e51d0aa")),
                AddonRecord::new("Bug Sack", "BugSack", SourceKind::Manual),
            ];
            addons[0].name = "Deadly Boss Mods".to_string();
            addons[0].folder = "DBM-Core".to_string();
            addons[0].author = Some("MysticalOS".to_string());
            addons[0].last_checked_at = Some(now - Duration::minutes(5));
            addons[1].name = "WeakAuras".to_string();
            addons[1].folder = "WeakAuras".to_string();
            addons[1].git_commit = Some("4f1c9a7123".to_string());
            app.dashboard = DashboardState::from_addons(addons);
            app.effective_addon_dir = Some(PathBuf::from(
                "/home/player/Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
            ));

            let rows = render_rows(&mut app, width, height);
            let text = rows.join("\n");
            let at = |index: usize| rows[index].as_str();
            let wide = width >= 110;

            // Header: logo, client and folder; then the answer line.
            if wide {
                assert!(at(0).contains("█   █▀▀ █▀▄▀█"), "{text}");
            } else {
                assert!(at(0).starts_with(" LEMONUP  Retail  "), "{text}");
            }
            assert!(at(0).contains("Interface/AddOns"), "{text}");
            assert!(
                at(1).contains("3 addons · 2 updates ready · checked 5 min ago"),
                "{text}"
            );
            assert_eq!(at(2).trim(), "", "{text}");
            // Column headers: the narrow set drops Author.
            for label in ["Addon", "Status ▲", "Version", "Source"] {
                assert!(
                    at(3).contains(label),
                    "{label} missing at {width}x{height}\n{text}"
                );
            }
            assert_eq!(at(3).contains("Author"), wide, "{text}");
            assert!(at(4).chars().all(|ch| ch == '─'), "{text}");
            // Rows.
            assert!(text.contains("↑ Update ready"), "{text}");
            assert!(text.contains("12.0.8 → 12.0.10"), "{text}");
            assert!(
                text.contains(if wide {
                    "5.20.4 → new build"
                } else {
                    "5.20.4 → newer"
                }),
                "{text}"
            );
            assert!(text.contains("· Not linked"), "{text}");
            assert!(
                text.contains(if wide { "Wago.io" } else { "Wago " }),
                "{text}"
            );
            // Bottom: rule, status line with the position, key line.
            let last = usize::from(height) - 1;
            assert!(at(last - 2).chars().all(|ch| ch == '─'), "{text}");
            assert!(at(last - 1).trim_end().ends_with("1 of 3"), "{text}");
            assert!(at(last).starts_with(" U update all (2)"), "{text}");
            for key in [
                "u update",
                "enter details",
                "/ filter",
                "i install",
                "? help",
            ] {
                assert!(
                    at(last).contains(key),
                    "{key} missing at {width}x{height}\n{text}"
                );
            }
            if wide {
                assert!(at(last).contains("C check all"), "{text}");
            }
        }
    }

    #[test]
    fn overlay_keeps_the_frame_and_shows_its_own_keys() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('?'));
        let rows = render_rows(&mut app, 120, 30);
        let text = rows.join("\n");
        assert!(rows[0].contains("█"), "{text}");
        assert!(rows[29].contains("esc or ? close"), "{text}");
        // Help is a sheet: it scrolls to the rest.
        assert!(text.contains("Getting things done"), "{text}");
        assert!(rows[29].contains("↑/↓ scroll"), "{text}");
    }

    #[test]
    fn truncate_middle_text_preserves_suffix_for_long_versions() {
        assert_eq!(
            truncate_middle_text("Details.20260327.14812.171", 16),
            "Details…4812.171"
        );
        assert_eq!(
            truncate_middle_text("Details.20260327.14812.999", 16),
            "Details…4812.999"
        );
    }

    #[test]
    fn onboarding_tick_bootstraps_quick_check() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.phase = OnboardingPhase::Bootstrapping;

        let actions = app.update(AppMessage::Tick);

        assert_eq!(
            actions,
            vec![
                AppAction::AdvanceMotionTick,
                AppAction::SetOnboardingState(app.onboarding.begin_quick_check()),
                AppAction::SetStatus(
                    app.with_base_status(StatusKind::Working, "checking common install locations")
                ),
                AppAction::StartOnboardingQuickCheck,
            ]
        );
    }

    #[test]
    fn empty_quick_check_starts_a_scan_from_the_current_path() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.phase = OnboardingPhase::QuickChecking;
        app.onboarding.input = "/home/player".to_string();

        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::Onboarding(
            crate::onboarding::OnboardingTaskEvent::QuickCheckFinished(Vec::new()),
        )));

        assert_eq!(
            actions,
            vec![
                AppAction::SetOnboardingState(app.onboarding.start_deep_scan()),
                AppAction::SetStatus(
                    app.with_base_status(
                        StatusKind::Working,
                        "scanning for WoW installs | esc stop"
                    )
                ),
                AppAction::StartOnboardingDeepScan(PathBuf::from("/home/player")),
            ]
        );
    }

    #[test]
    fn enter_on_folder_choice_searches_it() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        for phase in [
            OnboardingPhase::Ready,
            OnboardingPhase::Cancelled,
            OnboardingPhase::Error("nothing".to_string()),
        ] {
            app.onboarding.phase = phase;
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
                vec![AppMessage::OnboardingDeepScan]
            );
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('d'))),
                vec![AppMessage::OnboardingDeepScan]
            );
        }
    }

    #[test]
    fn empty_scan_result_names_the_folder_it_searched() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.input = "/home/player".to_string();

        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::Onboarding(
            crate::onboarding::OnboardingTaskEvent::DeepScanFinished(Ok(Vec::new())),
        )));

        assert_eq!(
            actions.first(),
            Some(&AppAction::SetOnboardingState(
                app.onboarding
                    .error("Couldn't find World of Warcraft under /home/player")
            ))
        );
    }

    #[test]
    fn found_confirmation_uses_selected_action() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.input =
            "C:\\Games\\World of Warcraft\\_retail_\\Interface\\AddOns".to_string();
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.phase = OnboardingPhase::Found(FoundState {
            paths: vec![app.onboarding.input.clone()],
            selected_action: 0,
        });

        let actions = app.update(AppMessage::OnboardingFoundConfirm);
        assert_eq!(
            actions,
            vec![
                AppAction::SetOnboardingState(
                    app.onboarding.apply_directory_to_draft().next_step()
                ),
                AppAction::SetStatus(
                    app.with_base_status(StatusKind::Done, "folder chosen | step 3/5 Wago.io")
                )
            ]
        );
    }

    #[test]
    fn onboarding_edit_mode_treats_command_letters_as_input() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding = app.onboarding.begin_directory_editing();

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('d'))),
            vec![AppMessage::OnboardingInputChar('d')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::OnboardingInputChar('q')]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
            vec![AppMessage::OnboardingValidateInput]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::OnboardingStopEditing]
        );
    }

    #[test]
    fn deep_scan_progress_event_updates_onboarding_state() {
        let app = app_for_tests(ShellMode::Onboarding);

        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::Onboarding(
            crate::onboarding::OnboardingTaskEvent::DeepScanProgress(
                crate::onboarding::ScanProgressState {
                    dirs_scanned: 42,
                    current_path: "C:\\".to_string(),
                },
            ),
        )));

        assert_eq!(
            actions,
            vec![AppAction::SetOnboardingState(
                app.onboarding
                    .set_scan_progress(crate::onboarding::ScanProgressState {
                        dirs_scanned: 42,
                        current_path: "C:\\".to_string(),
                    },)
            )]
        );
    }

    #[test]
    fn release_events_are_ignored_for_navigation() {
        let app = app_for_tests(ShellMode::Onboarding);
        let release = KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };

        assert!(app.messages_for_key(release).is_empty());
    }

    #[test]
    fn guarded_found_path_defaults_to_scan_another_location() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.runtime = AppRuntime::new(
            "dev".to_string(),
            None,
            vec![PathBuf::from(
                "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
            )],
        );

        let path = PathBuf::from("D:\\World of Warcraft\\_retail_\\Interface\\AddOns");
        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::Onboarding(
            crate::onboarding::OnboardingTaskEvent::QuickCheckFinished(vec![path.clone()]),
        )));

        assert_eq!(
            actions,
            vec![
                AppAction::SetOnboardingState(app.onboarding_found_state(vec![path.clone()])),
                AppAction::SetStatus(app.found_status_for_path(&path)),
            ]
        );
        assert_eq!(
            app.onboarding_found_state(vec![path])
                .selected_found_action(),
            FoundAction::EnterDifferentPath
        );
    }

    #[test]
    fn save_refuses_guarded_path_for_non_default_profile() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.runtime = AppRuntime::new(
            "dev".to_string(),
            None,
            vec![PathBuf::from(
                "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
            )],
        );

        let mut config = app.config_pane.draft.clone();
        config.addon_dir = Some(PathBuf::from(
            "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.apply(AppAction::SaveOnboardingConfig(config));

        assert_eq!(app.shell_mode, ShellMode::Onboarding);
        assert!(matches!(app.onboarding.phase, OnboardingPhase::Error(_)));
        assert!(
            app.status_line
                .text
                .contains("refused to save the main AddOns folder")
        );
    }

    #[test]
    fn bootstrap_rejects_invalid_addon_dir_override() {
        let temp = tempdir().expect("tempdir");
        let paths = AppPaths {
            profile: "dev".to_string(),
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            cache_dir: temp.path().join("cache"),
            log_dir: temp.path().join("data").join("logs"),
            config_file: temp.path().join("config").join("config.toml"),
            state_db_file: temp.path().join("data").join("state.sqlite"),
        };

        let error = match App::bootstrap(
            paths,
            AppRuntime::new(
                "dev".to_string(),
                Some(PathBuf::from("D:\\bad-path")),
                Vec::new(),
            ),
        ) {
            Ok(_) => panic!("invalid override should fail"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("invalid --addon-dir for profile 'dev'")
        );
    }

    #[test]
    fn bootstrap_rejects_guarded_addon_dir_override() {
        let temp = tempdir().expect("tempdir");
        let paths = AppPaths {
            profile: "dev".to_string(),
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            cache_dir: temp.path().join("cache"),
            log_dir: temp.path().join("data").join("logs"),
            config_file: temp.path().join("config").join("config.toml"),
            state_db_file: temp.path().join("data").join("state.sqlite"),
        };

        let guarded = PathBuf::from("D:\\World of Warcraft\\_retail_\\Interface\\AddOns");
        let error = match App::bootstrap(
            paths,
            AppRuntime::new("dev".to_string(), Some(guarded.clone()), vec![guarded]),
        ) {
            Ok(_) => panic!("guarded override should fail"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("matches the default profile AddOns directory")
        );
    }

    /// A WoW root with retail and Forever clients and a default-profile config for retail.
    fn two_client_profile(root: &Path) -> (AppPaths, PathBuf, PathBuf) {
        let wow = root.join("World of Warcraft");
        std::fs::create_dir_all(wow.join("Data")).expect("data dir");
        let mut clients = Vec::new();
        for client in ["_retail_", "_forever_"] {
            let addons = wow.join(client).join("Interface").join("AddOns");
            std::fs::create_dir_all(&addons).expect("addons dir");
            std::fs::create_dir_all(wow.join(client).join("WTF")).expect("wtf dir");
            std::fs::write(wow.join(client).join("Wow.exe"), "").expect("wow exe");
            clients.push(addons);
        }
        let paths = AppPaths {
            profile: DEFAULT_PROFILE.to_string(),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            log_dir: root.join("data").join("logs"),
            config_file: root.join("config").join("config.toml"),
            state_db_file: root.join("data").join("state.sqlite"),
        };
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some(clients[0].clone());
        ConfigStore::new(paths.config_file.clone())
            .write_new_config(&config)
            .expect("write config");
        let forever = clients.pop().expect("forever");
        let retail = clients.pop().expect("retail");
        (paths, retail, forever)
    }

    #[test]
    fn bootstrap_refuses_addon_dir_override_for_another_client() {
        let temp = tempdir().expect("tempdir");
        let (paths, retail, forever) = two_client_profile(temp.path());
        let state_db_file = paths.state_db_file.clone();

        let error = match App::bootstrap(
            paths,
            AppRuntime::new(DEFAULT_PROFILE.to_string(), Some(forever), Vec::new()),
        ) {
            Ok(_) => panic!("forever override must be refused for a retail profile"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("is bound to the AddOns folder"));
        let database = StateDatabase::open(&state_db_file).expect("open");
        assert_eq!(database.bound_addon_dir().expect("binding"), Some(retail));
    }

    #[test]
    fn bootstrap_with_config_moved_off_the_bound_folder_reopens_setup() {
        let temp = tempdir().expect("tempdir");
        let (paths, retail, forever) = two_client_profile(temp.path());
        StateDatabase::open(&paths.state_db_file)
            .expect("open")
            .bind_addon_dir(&forever)
            .expect("bind forever");

        let app = App::bootstrap(
            paths,
            AppRuntime::new(DEFAULT_PROFILE.to_string(), None, Vec::new()),
        )
        .expect("bootstrap");

        assert_eq!(app.shell_mode, ShellMode::Onboarding);
        assert_eq!(app.effective_addon_dir, None);
        let (notice, is_error) = app.startup_notice.clone().expect("binding notice");
        assert!(is_error);
        assert!(notice.contains(&forever.display().to_string()), "{notice}");
        assert!(notice.contains(&retail.display().to_string()), "{notice}");
    }

    #[test]
    fn bootstrap_uses_the_bound_clients_backup_folder() {
        let temp = tempdir().expect("tempdir");
        let (paths, _retail, _forever) = two_client_profile(temp.path());
        let data_dir = paths.data_dir.clone();
        let legacy = backup_root(&data_dir).join("WTF-20260101T000000000Z.zip");
        std::fs::create_dir_all(legacy.parent().expect("root")).expect("store");
        std::fs::write(&legacy, b"legacy").expect("legacy archive");

        let app = App::bootstrap(
            paths,
            AppRuntime::new(DEFAULT_PROFILE.to_string(), None, Vec::new()),
        )
        .expect("bootstrap");

        assert_eq!(app.backup_dir, backup_root(&data_dir).join("retail"));
        assert!(!legacy.exists());
        assert_eq!(app.backup_pane.backups.len(), 1);
    }

    #[test]
    fn confirming_a_new_folder_in_setup_rebinds_and_snapshots_tracking() {
        let temp = tempdir().expect("tempdir");
        let (paths, retail, forever) = two_client_profile(temp.path());
        let state_db_file = paths.state_db_file.clone();
        let data_dir = paths.data_dir.clone();
        {
            let database = StateDatabase::open(&state_db_file).expect("open");
            database.bind_addon_dir(&retail).expect("bind retail");
            database
                .upsert_addon(&AddonRecord::new(
                    "GitAddon",
                    "GitAddon",
                    SourceKind::GitHub,
                ))
                .expect("seed");
        }
        let mut app = App::bootstrap(
            paths,
            AppRuntime::new(DEFAULT_PROFILE.to_string(), None, Vec::new()),
        )
        .expect("bootstrap");

        let mut config = app.config_pane.draft.clone();
        config.addon_dir = Some(forever.clone());
        app.apply(AppAction::SaveOnboardingConfig(config));

        let database = StateDatabase::open(&state_db_file).expect("open");
        assert_eq!(
            database.bound_addon_dir().expect("binding"),
            Some(forever.clone())
        );
        assert_eq!(database.list_addons().expect("rows").len(), 1);
        let snapshots = std::fs::read_dir(data_dir.join("rebind"))
            .expect("snapshot dir")
            .count();
        assert_eq!(snapshots, 1);
        assert_eq!(app.backup_dir, backup_root(&data_dir).join("forever"));
        assert_eq!(app.effective_addon_dir, Some(forever));
    }

    #[test]
    fn dashboard_tick_starts_pending_scan() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.scan_state = ScanState::Pending;
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));

        let actions = app.update(AppMessage::Tick);

        assert_eq!(
            actions,
            vec![
                AppAction::AdvanceMotionTick,
                AppAction::StartAddonScan(PathBuf::from(
                    "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
                )),
            ]
        );
    }

    #[test]
    fn dashboard_tick_starts_auto_check_when_due() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.scan_state = ScanState::Succeeded {
            path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
            summary: ScanSummary {
                scanned_addons: 3,
                upserted_addons: 3,
                removed_addons: 0,
                ..Default::default()
            },
        };
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.dashboard.detail_mode = DetailMode::Overview;
        app.config_pane.draft.auto_check_enabled = true;
        app.config_pane.draft.auto_check_interval_secs = 60;

        let actions = app.update(AppMessage::Tick);

        assert!(actions.contains(&AppAction::SetDashboardAutoCheckInProgress(true)));
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, AppAction::StartDashboardCheckSelected { .. }))
        );
    }

    #[test]
    fn config_mode_routes_import_export_keys() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::ConfigImportAddons]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('x'))),
            vec![AppMessage::ConfigExportAddons]
        );
    }

    #[test]
    fn active_detail_status_message_extracts_config_message() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(app.dashboard_status_for(
            StatusKind::Done,
            DetailMode::Config,
            "exported 3 addons",
        )));

        assert_eq!(
            app.active_detail_status_message(DetailMode::Config),
            Some("exported 3 addons".to_string())
        );
        assert_eq!(app.active_detail_status_message(DetailMode::Search), None);
    }

    #[test]
    fn backup_mode_routes_restore_confirmation_keys() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.backup_pane.pending_restore = Some(BackupEntry {
            file_name: "WTF-20260405T120000.000Z.zip".to_string(),
            path: PathBuf::from("C:\\Temp\\WTF-20260405T120000.000Z.zip"),
            label: "2026-04-05 12:00:00.000Z".to_string(),
            size_bytes: 128,
        });

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::BackupConfirmRestore]
        );
        for cancel in [KeyCode::Char('n'), KeyCode::Char('q'), KeyCode::Esc] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(cancel)),
                vec![AppMessage::BackupCancelRestore],
                "{cancel:?}"
            );
        }
        // enter never confirms a backup restore.
        assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Enter)), vec![]);
    }

    #[test]
    fn backup_mode_routes_delete_confirmation_keys() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Backup;
        app.backup_pane.pending_delete = Some(BackupEntry {
            file_name: "WTF-20260405T120000.000Z.zip".to_string(),
            path: PathBuf::from("C:\\Temp\\WTF-20260405T120000.000Z.zip"),
            label: "2026-04-05 12:00:00.000Z".to_string(),
            size_bytes: 128,
        });

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
            vec![AppMessage::BackupConfirmDelete]
        );
        for cancel in [KeyCode::Char('n'), KeyCode::Char('q'), KeyCode::Esc] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(cancel)),
                vec![AppMessage::BackupCancelRestore],
                "{cancel:?}"
            );
        }
        // enter never confirms a backup delete.
        assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Enter)), vec![]);
    }

    #[test]
    fn addon_scan_success_hydrates_dashboard_and_status() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let mut stored = AddonRecord::new("Details", "Details", SourceKind::Manual);
        stored.kind = AddonKind::Addon;
        stored.version = Some("1.2.3".to_string());
        stored.author = Some("Author".to_string());
        stored.interface = Some("110005".to_string());
        stored.required_deps = vec!["Ace3".to_string()];
        stored.embedded_libs = vec!["LibStub".to_string()];

        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::AddonScanFinished(
            Ok(AddonScanOutcome {
                path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
                summary: ScanSummary {
                    scanned_addons: 1,
                    upserted_addons: 1,
                    removed_addons: 0,
                    ..Default::default()
                },
                addons: vec![stored.clone()],
                drift_report: DriftReport::empty(),
            }),
        )));

        assert_eq!(actions.len(), 3);
        app.apply(actions[0].clone());
        app.apply(actions[1].clone());
        app.apply(actions[2].clone());

        assert_eq!(app.dashboard.items.len(), 1);
        assert_eq!(app.dashboard.items[0].name, "Details");
        assert!(
            app.status_line
                .text
                .contains("looked through the AddOns folder")
        );
        assert!(matches!(app.scan_state, ScanState::Succeeded { .. }));
    }

    #[test]
    fn bootstrap_with_valid_config_enters_dashboard_with_pending_scan() {
        let temp = tempdir().expect("tempdir");
        let addons_dir = temp
            .path()
            .join("World of Warcraft")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addons_dir).expect("create addons dir");
        std::fs::create_dir_all(temp.path().join("World of Warcraft").join("Data"))
            .expect("create data dir");
        std::fs::write(
            temp.path()
                .join("World of Warcraft")
                .join("_retail_")
                .join("Wow.exe"),
            "",
        )
        .expect("write wow exe");

        let paths = AppPaths {
            profile: "dev".to_string(),
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            cache_dir: temp.path().join("cache"),
            log_dir: temp.path().join("data").join("logs"),
            config_file: temp.path().join("config").join("config.toml"),
            state_db_file: temp.path().join("data").join("state.sqlite"),
        };

        let config_store = ConfigStore::new(paths.config_file.clone());
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some(addons_dir.clone());
        config_store
            .write_new_config(&config)
            .expect("write config");

        let app = App::bootstrap(
            paths,
            AppRuntime::new("dev".to_string(), None, vec![PathBuf::from("D:\\guarded")]),
        )
        .expect("bootstrap app");

        assert_eq!(app.shell_mode, ShellMode::Dashboard);
        assert_eq!(app.effective_addon_dir, Some(addons_dir));
        assert_eq!(app.scan_state, ScanState::Pending);
    }

    #[test]
    fn bootstrap_with_light_theme_config_applies_day_palette() {
        let temp = tempdir().expect("tempdir");
        let addons_dir = temp
            .path()
            .join("World of Warcraft")
            .join("_retail_")
            .join("Interface")
            .join("AddOns");
        std::fs::create_dir_all(&addons_dir).expect("create addons dir");
        std::fs::create_dir_all(temp.path().join("World of Warcraft").join("Data"))
            .expect("create data dir");
        std::fs::write(
            temp.path()
                .join("World of Warcraft")
                .join("_retail_")
                .join("Wow.exe"),
            "",
        )
        .expect("write wow exe");

        let paths = AppPaths {
            profile: "dev".to_string(),
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            cache_dir: temp.path().join("cache"),
            log_dir: temp.path().join("data").join("logs"),
            config_file: temp.path().join("config").join("config.toml"),
            state_db_file: temp.path().join("data").join("state.sqlite"),
        };

        let config_store = ConfigStore::new(paths.config_file.clone());
        let mut config = AppConfig::new_unconfigured();
        config.addon_dir = Some(addons_dir);
        config.theme = ThemeMode::Light;
        config_store
            .write_new_config(&config)
            .expect("write config");

        let app = App::bootstrap(
            paths,
            AppRuntime::new("dev".to_string(), None, vec![PathBuf::from("D:\\guarded")]),
        )
        .expect("bootstrap app");

        assert_eq!(app.ui_theme.panel_title, Color::Rgb(46, 125, 233));
        assert_eq!(app.ui_theme.overlay_bg, Color::Rgb(232, 233, 238));
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// Draws the app into a test buffer and returns its rows as text.
    fn render_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal.draw(|frame| app.draw(frame)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    /// The three rows of the filter box at `width`, as text.
    fn filter_box_text(app: &App, width: u16) -> String {
        app.dashboard_filter_box_lines(width)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press_key(app: &mut App, code: KeyCode) {
        for message in app.messages_for_key(KeyEvent::from(code)) {
            for action in app.update(message) {
                app.apply(action);
            }
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press_key(app, KeyCode::Char(character));
        }
    }

    fn paste_text(app: &mut App, text: &str) {
        for message in app.messages_for_event(&TerminalEvent::Paste(text.to_string())) {
            for action in app.update(message) {
                app.apply(action);
            }
        }
    }

    fn start_config_edit(app: &mut App, value: &str) {
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.edit = Some(ConfigEditState {
            field: ConfigField::WagoApiKey,
            value: value.to_string().into(),
        });
    }

    fn config_edit_value(app: &App) -> String {
        app.config_pane
            .edit
            .as_ref()
            .expect("config edit active")
            .value
            .value()
            .to_string()
    }

    #[test]
    fn config_edit_captures_quit_and_action_letters_as_text() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        start_config_edit(&mut app, "");

        for character in ['q', 'e', 'r', 'x', 'i', 'j', 'k', 'h', 'l', '?', ','] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char(character))),
                vec![AppMessage::ConfigInputChar(character)]
            );
        }
        type_text(&mut app, "qWAGOqrxij");
        assert_eq!(config_edit_value(&app), "qWAGOqrxij");
        assert!(!app.quit_requested);
        assert_eq!(app.dashboard.detail_mode, DetailMode::Config);
    }

    #[test]
    fn paste_into_config_edit_appends_and_strips_newlines() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        start_config_edit(&mut app, "pre-");

        paste_text(&mut app, "abc-quit-q\r\nxyz\n");

        assert_eq!(config_edit_value(&app), "pre-abc-quit-qxyz");
        assert!(!app.quit_requested);
    }

    #[test]
    fn paste_into_dashboard_filter_appends_to_query() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "a");

        paste_text(&mut app, "q\nu");

        assert_eq!(app.dashboard.filter_query(), "aqu");
        assert!(app.dashboard.filter_editing());
        assert!(!app.quit_requested);
    }

    #[test]
    fn paste_into_search_query_appends_to_query() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;

        paste_text(&mut app, "quest log\r\n");

        assert_eq!(app.search_pane.query, "quest log");
        assert!(!app.quit_requested);
    }

    #[test]
    fn paste_into_install_target_appends_to_input() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Install;
        app.install_pane = app.install_pane.begin_editing();

        paste_text(&mut app, "12345\n");

        assert_eq!(app.install_pane.input, "12345");
    }

    #[test]
    fn paste_into_onboarding_inputs_appends_text() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.input = String::new();
        app.onboarding.is_editing = true;

        paste_text(&mut app, "/games/q\n/AddOns");
        assert_eq!(app.onboarding.input, "/games/q/AddOns");

        app.onboarding.step = OnboardingStep::Wago;
        app.onboarding.draft.wago_api_key = None;
        paste_text(&mut app, "key-with-q\n");
        assert_eq!(
            app.onboarding.draft.wago_api_key.as_deref(),
            Some("key-with-q")
        );
        assert!(!app.quit_requested);
    }

    #[test]
    fn paste_is_ignored_when_no_text_field_is_editing() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let config_before = app.config_pane.clone();

        paste_text(&mut app, "quit\n");
        assert_eq!(app.dashboard.filter_query(), "");
        assert!(!app.dashboard.filter_editing());
        assert!(!app.quit_requested);
        assert!(app.config_pane.edit.is_none());

        app.dashboard.detail_mode = DetailMode::Config;
        paste_text(&mut app, "quit\n");
        assert_eq!(app.config_pane.draft, config_before.draft);
        assert!(app.config_pane.edit.is_none());

        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.install_state = SearchInstallState::ConfirmDirect {
            request: crate::app::direct_install::DirectInstallRequest {
                provider: crate::app::direct_install::DirectInstallProvider::Tukui,
                target: "elvui".to_string(),
                display: "ElvUI".to_string(),
            },
        };
        paste_text(&mut app, "quit\n");
        assert_eq!(app.search_pane.query, "");

        app.dashboard.detail_mode = DetailMode::Overview;
        app.shell_ui.overlay.active = Some(OverlayKind::Help);
        paste_text(&mut app, "quit\n");
        assert!(!app.quit_requested);
    }

    fn visible_folders(app: &App) -> Vec<String> {
        app.dashboard
            .rows
            .iter()
            .map(|row| row.folder.clone())
            .collect()
    }

    #[test]
    fn dashboard_slash_starts_filter_and_does_not_open_search() {
        let mut app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('/'))),
            vec![AppMessage::DashboardFilterBegin]
        );
        press_key(&mut app, KeyCode::Char('/'));

        assert!(app.dashboard.filter_editing());
        assert!(app.dashboard.filter_bar_visible());
        assert_eq!(app.dashboard.detail_mode, DetailMode::Overview);
        assert!(app.shell_ui.overlay.active.is_none());
        assert_eq!(app.dashboard.rows.len(), 3);
    }

    #[test]
    fn dashboard_i_still_opens_search_and_search_slash_still_edits_query() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
        );

        app.dashboard.detail_mode = DetailMode::Search;
        app.shell_ui.overlay.active = Some(OverlayKind::Search);
        // No edit mode: `/` is a letter in the Install box.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('/'))),
            vec![AppMessage::SearchInputChar('/')]
        );
    }

    #[test]
    fn footer_hint_for_slash_is_filter_on_dashboard_and_edit_in_search() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        assert_eq!(
            app.footer_hint_for_key(KeyEvent::from(KeyCode::Char('/'))),
            Some(FooterHintId::Filter)
        );
        assert!(
            app.key_table(120)
                .iter()
                .any(|hint| hint.id == Some(FooterHintId::Filter) && hint.key == "/")
        );

        app.dashboard.detail_mode = DetailMode::Search;
        app.shell_ui.overlay.active = Some(OverlayKind::Search);
        assert_eq!(
            app.footer_hint_for_key(KeyEvent::from(KeyCode::Char('/'))),
            None
        );
    }

    #[test]
    fn dashboard_filter_matches_name_folder_and_author_case_insensitively() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.items[0].name = "Deadly Boss Mods".to_string();
        app.dashboard.rebuild_rows();
        let dbm_folder = app.dashboard.items[0].folder.clone();

        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "BOSS");
        assert_eq!(visible_folders(&app), vec![dbm_folder.clone()]);

        app.dashboard.set_filter(TextInput::default(), false);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "thi");
        assert_eq!(visible_folders(&app), vec!["Third".to_string()]);

        app.dashboard.set_filter(TextInput::default(), false);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "author");
        assert_eq!(visible_folders(&app), vec!["Second".to_string()]);
    }

    #[test]
    fn dashboard_filter_child_folder_match_shows_parent_expanded() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        assert!(!app.dashboard.expanded_folders.contains("Second"));

        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "config");

        assert_eq!(
            visible_folders(&app),
            vec!["Second".to_string(), "Second_Config".to_string()]
        );
        assert!(app.dashboard.rows[0].expanded);
        // The forced expansion is a view of the filter, not a persisted tree change.
        assert!(!app.dashboard.expanded_folders.contains("Second"));

        press_key(&mut app, KeyCode::Esc);
        assert_eq!(app.dashboard.rows.len(), 3);
    }

    #[test]
    fn dashboard_filter_requires_every_term_to_match() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));

        type_text(&mut app, "second config");
        assert_eq!(
            visible_folders(&app),
            vec!["Second".to_string(), "Second_Config".to_string()]
        );

        type_text(&mut app, " third");
        assert!(app.dashboard.rows.is_empty());
        assert_eq!(app.dashboard.list_state.selected(), None);
    }

    #[test]
    fn dashboard_filter_backspace_widens_the_list() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "thirdx");
        assert!(app.dashboard.rows.is_empty());

        press_key(&mut app, KeyCode::Backspace);
        assert_eq!(app.dashboard.filter_query(), "third");
        assert_eq!(visible_folders(&app), vec!["Third".to_string()]);
        assert_eq!(app.dashboard.list_state.selected(), Some(0));
    }

    #[test]
    fn dashboard_filter_enter_keeps_filter_and_stops_editing() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "third");
        press_key(&mut app, KeyCode::Enter);

        assert!(!app.dashboard.filter_editing());
        assert_eq!(app.dashboard.filter_query(), "third");
        assert_eq!(visible_folders(&app), vec!["Third".to_string()]);
        assert!(app.dashboard.filter_bar_visible());

        // Dashboard keys work again on the filtered rows.
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('u'))),
            vec![AppMessage::DashboardRunUpdateSelected]
        );
        assert_eq!(
            app.dashboard.actionable_parent_targets(),
            Ok(vec!["Third".to_string()])
        );

        // `/` re-enters editing with the current text.
        press_key(&mut app, KeyCode::Char('/'));
        assert!(app.dashboard.filter_editing());
        assert_eq!(app.dashboard.filter_query(), "third");
    }

    #[test]
    fn dashboard_filter_escape_while_editing_clears_and_closes() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "third");

        press_key(&mut app, KeyCode::Esc);

        assert!(!app.dashboard.filter_editing());
        assert!(!app.dashboard.has_filter());
        assert!(!app.dashboard.filter_bar_visible());
        assert_eq!(app.dashboard.rows.len(), 3);
    }

    #[test]
    fn dashboard_escape_clears_applied_filter_before_selection() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.selected_parents.insert("Third".to_string());
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "third");
        press_key(&mut app, KeyCode::Enter);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardFilterClear]
        );
        press_key(&mut app, KeyCode::Esc);
        assert!(!app.dashboard.has_filter());
        assert_eq!(app.dashboard.selected_parent_count(), 1);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardClearSelection]
        );
    }

    #[test]
    fn dashboard_filter_editing_captures_action_letters_as_text() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));

        for character in ['u', 'x', 'c', 'a', 'q', 'i', 'j', 'k', '1', ' ', '/', '?'] {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char(character))),
                vec![AppMessage::DashboardFilterInputChar(character)]
            );
        }
        type_text(&mut app, "uxca");
        assert_eq!(app.dashboard.filter_query(), "uxca");
        assert!(app.dashboard.pending_delete_folders().is_none());
        assert_eq!(app.dashboard.selected_parent_count(), 0);
        assert!(app.shell_ui.overlay.active.is_none());
        assert_eq!(
            app.footer_hint_for_key(KeyEvent::from(KeyCode::Char('u'))),
            None
        );
    }

    #[test]
    fn dashboard_select_all_with_filter_selects_only_visible_rows() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "second");
        press_key(&mut app, KeyCode::Enter);

        press_key(&mut app, KeyCode::Char('a'));

        assert_eq!(
            app.dashboard.selected_parent_folders(),
            vec!["Second".to_string()]
        );
        assert!(filter_box_text(&app, 120).contains("1 of 3 shown"));
    }

    #[test]
    fn dashboard_selections_survive_filter_changes() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.selected_parents.insert("First".to_string());
        app.dashboard.selected_parents.insert("Third".to_string());

        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "second");
        assert_eq!(app.dashboard.selected_parent_count(), 2);
        assert!(filter_box_text(&app, 120).contains("1 of 3 shown"));
        press_key(&mut app, KeyCode::Enter);

        // Bulk actions keep targeting the full selection, including hidden rows.
        assert_eq!(
            app.dashboard.actionable_parent_targets(),
            Ok(vec!["First".to_string(), "Third".to_string()])
        );

        press_key(&mut app, KeyCode::Esc);
        assert_eq!(app.dashboard.selected_parent_count(), 2);
        assert!(line_text(&app.dashboard_key_line(120)).contains("2 selected:"));
    }

    #[test]
    fn dashboard_filter_keeps_cursor_on_same_addon_or_clamps() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let third_index = app
            .dashboard
            .rows
            .iter()
            .position(|row| row.folder == "Third")
            .expect("third row");
        app.dashboard.list_state.select(Some(third_index));

        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "t");
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Third")
        );

        // Cursor row filtered out: clamp into the remaining rows.
        app.dashboard.set_filter("second".into(), true);
        assert_eq!(app.dashboard.rows.len(), 1);
        assert_eq!(app.dashboard.list_state.selected(), Some(0));

        // No rows: no cursor; growing again restores a valid one.
        app.dashboard.set_filter("nomatch".into(), true);
        assert_eq!(app.dashboard.list_state.selected(), None);
        assert!(app.dashboard.selected_row().is_none());
        app.dashboard.set_filter(TextInput::default(), true);
        assert_eq!(app.dashboard.list_state.selected(), Some(0));
        assert_eq!(app.dashboard.rows.len(), 3);
    }

    #[test]
    fn dashboard_filter_keeps_current_sort_order() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ir");
        let before = visible_folders(&app);
        assert_eq!(before.len(), 2);

        app.apply(AppAction::ToggleDashboardSort(DashboardSortColumn::Name));
        let ascending = visible_folders(&app);
        app.apply(AppAction::ToggleDashboardSort(DashboardSortColumn::Name));
        let descending = visible_folders(&app);

        assert_eq!(ascending, vec!["First".to_string(), "Third".to_string()]);
        assert_eq!(descending, vec!["Third".to_string(), "First".to_string()]);
    }

    #[test]
    fn dashboard_filter_zero_match_renders_empty_state() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "zzz");
        assert!(filter_box_text(&app, 120).contains("0 of 3 shown"));

        let rows = render_rows(&mut app, 100, 30);
        let text = rows.join("\n");
        assert!(
            text.contains("Nothing you have installed matches \"zzz\"."),
            "{text}"
        );
        // The filter box covers rows 1 to 3, so the table stays put.
        assert!(rows[1].starts_with("│  / zzz▌"), "{text}");
        assert!(rows[0].ends_with(" 0 of 3 shown ─╮"), "{text}");
        assert!(rows[3].contains("Addon"), "{text}");
        assert!(
            !text.contains("Looking through your AddOns folder"),
            "{text}"
        );
    }

    #[test]
    fn dashboard_filter_box_keeps_the_list_rows_in_place() {
        for (width, height) in [(120u16, 30u16), (80, 24), (60, 20)] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let before = render_rows(&mut app, width, height);
            press_key(&mut app, KeyCode::Char('/'));
            let empty = render_rows(&mut app, width, height);
            type_text(&mut app, "ir");
            let typing = render_rows(&mut app, width, height);
            press_key(&mut app, KeyCode::Enter);
            let applied = render_rows(&mut app, width, height);
            press_key(&mut app, KeyCode::Esc);
            let cleared = render_rows(&mut app, width, height);

            let bottom_rule = usize::from(height) - 3;
            // The row of the first addon in the list.
            let first_row = |rows: &[String]| {
                rows.iter().position(|row| {
                    ["First", "Second", "Third"]
                        .iter()
                        .any(|name| row.contains(name))
                })
            };
            for (label, rows) in [
                ("empty", &empty),
                ("typing", &typing),
                ("applied", &applied),
            ] {
                let text = rows.join("\n");
                assert!(rows[0].starts_with("╭─ Filter "), "{label} {width}\n{text}");
                assert!(rows[0].ends_with("─╮"), "{label} {width}\n{text}");
                assert!(rows[1].starts_with("│  / "), "{label} {width}\n{text}");
                assert!(rows[1].ends_with('│'), "{label} {width}\n{text}");
                assert_eq!(rows[2], format!("╰{}╯", "─".repeat(usize::from(width) - 2)));
                // The column header, both rules and the first list row stay put.
                assert_eq!(rows[3], before[3], "{label} {width}\n{text}");
                assert_eq!(rows[4], before[4], "{label} {width}\n{text}");
                assert_eq!(rows[bottom_rule], before[bottom_rule], "{label} {width}");
                assert_eq!(
                    first_row(rows),
                    first_row(&before),
                    "{label} {width}\n{text}"
                );
            }
            assert_eq!(first_row(&before), Some(5));
            // esc brings the header and the blank row back.
            assert!(!cleared[0].starts_with('╭'), "{}", cleared.join("\n"));
            assert!(cleared[2].trim().is_empty(), "{}", cleared.join("\n"));
            assert_eq!(cleared[3], before[3]);
        }
    }

    #[test]
    fn dashboard_filter_query_lines_up_with_the_addon_names() {
        let column = |row: &str, needle: &str| {
            row.find(needle)
                .map(|byte| row[..byte].chars().count())
                .expect("needle")
        };
        for (width, docked) in [(120u16, false), (80, false), (120, true)] {
            let mut app = app_for_tests(ShellMode::Dashboard);
            if docked {
                app.apply(AppAction::SetDashboardFilter {
                    query: "First".into(),
                    editing: false,
                });
                app.apply(AppAction::SetInspectOverlay(true));
            } else {
                press_key(&mut app, KeyCode::Char('/'));
                type_text(&mut app, "First");
            }
            let rows = render_rows(&mut app, width, 30);
            let text = rows.join("\n");
            let list_row = rows[5..]
                .iter()
                .find(|row| row.contains("First"))
                .expect("list row");
            assert_eq!(
                column(&rows[1], "First"),
                column(list_row, "First"),
                "{text}"
            );
            assert_eq!(column(&rows[1], "First"), 5, "{text}");
            if docked {
                // The box spans the window over docked Details, and esc
                // closes Details first, so the box does not offer it.
                assert!(app.details_docked());
                assert_eq!(rows[1].chars().count(), usize::from(width));
                assert!(rows[1].ends_with("/ edit  │"), "{text}");
                assert!(!rows[1].contains("esc"), "{text}");
            }
        }
    }

    #[test]
    fn dashboard_filter_box_shows_the_count_and_the_keys_for_its_state() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        let empty = filter_box_text(&app, 120);
        assert!(
            empty.contains("│  / ▌part of a name, folder or author "),
            "{empty}"
        );
        assert!(empty.contains(" 3 of 3 shown ─╮"), "{empty}");
        assert!(empty.contains("enter keep · esc clear  │"), "{empty}");

        type_text(&mut app, "ir");
        let typing = app.dashboard_filter_box_lines(120);
        let rows: Vec<String> = typing.iter().map(line_text).collect();
        for row in &rows {
            assert_eq!(row.chars().count(), 120, "{row}");
        }
        assert!(rows[0].starts_with("╭─ Filter ──"), "{}", rows[0]);
        assert!(rows[0].ends_with("── 2 of 3 shown ─╮"), "{}", rows[0]);
        assert!(rows[1].starts_with("│  / ir▌ "), "{}", rows[1]);
        assert!(
            rows[1].ends_with("enter keep · esc clear  │"),
            "{}",
            rows[1]
        );
        // Typing: the border is the accent and the query is strong.
        let roles = app.ui_theme.roles;
        assert_eq!(typing[0].spans[0].style.fg, Some(roles.lemon));
        let query = typing[1]
            .spans
            .iter()
            .find(|span| span.content == "ir")
            .expect("query span");
        assert!(
            query
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );

        press_key(&mut app, KeyCode::Enter);
        let applied = app.dashboard_filter_box_lines(120);
        let rows: Vec<String> = applied.iter().map(line_text).collect();
        assert!(rows[1].starts_with("│  / ir "), "{}", rows[1]);
        assert!(!rows[1].contains('▌'), "{}", rows[1]);
        assert!(rows[1].ends_with("/ edit · esc clear  │"), "{}", rows[1]);
        assert!(rows[0].ends_with(" 2 of 3 shown ─╮"), "{}", rows[0]);
        // Applied: the border steps back and the query is plain text.
        assert_eq!(applied[0].spans[0].style.fg, Some(roles.border));
        let query = applied[1]
            .spans
            .iter()
            .find(|span| span.content == "ir")
            .expect("query span");
        assert!(
            !query
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        // The old filter line's "esc clears the filter" is gone.
        assert!(!filter_box_text(&app, 120).contains("clears the filter"));
    }

    #[test]
    fn dashboard_filter_box_gives_way_on_narrow_windows() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        press_key(&mut app, KeyCode::Char('/'));
        let rows = |app: &App, width: u16| -> Vec<String> {
            let rows: Vec<String> = app
                .dashboard_filter_box_lines(width)
                .iter()
                .map(line_text)
                .collect();
            for row in &rows {
                assert_eq!(row.chars().count(), usize::from(width), "{row}");
            }
            rows
        };
        // An empty query: the shorter placeholder keeps the hint at 60.
        assert!(rows(&app, 80)[1].contains("▌part of a name, folder or author"));
        let at_60 = rows(&app, 60);
        assert!(
            at_60[1].contains("▌name, folder or author "),
            "{}",
            at_60[1]
        );
        assert!(
            at_60[1].ends_with("enter keep · esc clear  │"),
            "{}",
            at_60[1]
        );

        // A short query fits with its hint and the full count at 80 and 60.
        type_text(&mut app, "weak");
        for width in [80, 60] {
            let rows = rows(&app, width);
            assert!(rows[0].ends_with(" 0 of 3 shown ─╮"), "{}", rows[0]);
            assert!(rows[1].starts_with("│  / weak▌ "), "{}", rows[1]);
            assert!(
                rows[1].ends_with("enter keep · esc clear  │"),
                "{}",
                rows[1]
            );
        }

        // A longer query drops the hint first.
        let long = "abcdefghij".repeat(4);
        app.dashboard.set_filter(long.clone().into(), true);
        let at_60 = rows(&app, 60);
        assert!(
            at_60[1].starts_with(&format!("│  / {long}▌")),
            "{}",
            at_60[1]
        );
        assert!(!at_60[1].contains("enter"), "{}", at_60[1]);
        assert!(at_60[0].ends_with(" 0 of 3 shown ─╮"), "{}", at_60[0]);

        // Then, on a very narrow window, the count loses "shown".
        let at_26 = rows(&app, 26);
        assert!(at_26[0].ends_with("─ 0 of 3 ─╮"), "{}", at_26[0]);

        // A query wider than the box is cut from the left: the end stays.
        let longer = format!("{}END", "0123456789".repeat(7));
        app.dashboard.set_filter(longer.clone().into(), true);
        for width in [80u16, 60] {
            let rows = rows(&app, width);
            assert!(rows[1].starts_with("│  / …"), "{}", rows[1]);
            assert!(rows[1].ends_with("END▌ │"), "{}", rows[1]);
        }
        // Applied, the same: no cursor, the end still shows.
        app.dashboard.set_filter(longer.into(), false);
        let at_60 = rows(&app, 60);
        assert!(at_60[1].starts_with("│  / …"), "{}", at_60[1]);
        assert!(at_60[1].ends_with("END │"), "{}", at_60[1]);
    }

    #[test]
    fn dashboard_pointer_selection_maps_rows_of_the_list_area() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        // The list area holds rows only: the first row is the first addon.
        app.last_dashboard_list_area = Some(Rect::new(0, 5, 40, 10));
        assert_eq!(app.dashboard_selection_for_pointer(2, 4), None);
        assert_eq!(app.dashboard_selection_for_pointer(2, 5), Some(0));
        assert_eq!(app.dashboard_selection_for_pointer(2, 6), Some(1));
        assert_eq!(app.dashboard_selection_for_pointer(40, 6), None);

        // A filter does not move the rows: it has its own line above them.
        press_key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ir");
        assert_eq!(app.dashboard_selection_for_pointer(2, 5), Some(0));
        assert_eq!(app.dashboard_selection_for_pointer(2, 6), Some(1));
        assert_eq!(app.dashboard_selection_for_pointer(2, 7), None);
    }

    #[test]
    fn drawn_list_area_lines_up_with_clicks() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        let rows = render_rows(&mut app, 120, 30);
        let area = app.last_dashboard_list_area.expect("list area");
        // Two header rows, the filter row, the column header and a rule.
        assert_eq!(area.y, 5, "{}", rows.join("\n"));
        assert_eq!(area.height, 30 - 8);
        let second_name = app.dashboard.rows[1].name.clone();
        assert!(rows[6].contains(&second_name), "{}", rows.join("\n"));
        assert_eq!(app.dashboard_selection_for_pointer(10, 6), Some(1));
    }

    mod sheets {
        use super::*;
        use crate::app::keymap::{HelpLabel, HelpWhen, KeyHint, help_page};
        use crate::app::sheet::{SheetFrame, offset_for_focus, wrap_lines};
        use ratatui::style::Style;
        use ratatui::text::Span;

        /// The sizes the design was checked at: the smallest supported, the
        /// Windows Terminal default, and the website screenshots.
        const SIZES: [(u16, u16); 3] = [(80, 24), (120, 30), (138, 36)];

        fn apply_all(app: &mut App, message: AppMessage) {
            for action in app.update(message) {
                app.apply(action);
            }
        }

        fn wago_results(count: usize) -> Vec<WagoSearchResult> {
            (0..count)
                .map(|index| WagoSearchResult {
                    id: format!("slug{index}"),
                    display_name: format!("Result Addon {index}"),
                    summary: Some(format!("Summary for result {index}")),
                    owner: Some("Owner".to_string()),
                    authors: vec!["Owner".to_string()],
                    website_url: None,
                    download_count: Some(1_000 + index as u64),
                    version: Some("1.0.0".to_string()),
                })
                .collect()
        }

        fn backups(count: usize) -> Vec<BackupEntry> {
            (0..count)
                .map(|index| BackupEntry {
                    file_name: format!("WTF-2026040{index}T120000.000Z.zip"),
                    path: PathBuf::from(format!("/sandbox/WTF-{index}.zip")),
                    label: format!("2026-04-0{index} 12:00:00.000Z"),
                    size_bytes: 2048,
                })
                .collect()
        }

        fn open_details_on(app: &mut App, folder: &str) {
            let index = app
                .dashboard
                .rows
                .iter()
                .position(|row| {
                    row.folder == folder && row.kind == crate::app::DashboardRowKind::Parent
                })
                .expect("addon row");
            app.dashboard.list_state.select(Some(index));
            app.apply(AppAction::SetInspectOverlay(true));
        }

        /// Details on Second: linked, with an author, a dependency and a
        /// folder of its own. The longest Details the fixture has.
        fn open_inspect(app: &mut App) {
            open_details_on(app, "Second");
        }

        fn open_inspect_not_linked(app: &mut App) {
            open_details_on(app, "First");
        }

        fn open_inspect_up_to_date(app: &mut App) {
            open_details_on(app, "Third");
        }

        fn open_install_empty(app: &mut App) {
            app.apply(AppAction::SetDetailMode(DetailMode::Search));
            app.wago_api_key = None;
        }

        fn open_install_results(app: &mut App) {
            app.apply(AppAction::SetDetailMode(DetailMode::Search));
            app.search_pane.focus = SearchFocus::Results;
            app.search_pane.results = wago_results(30);
            app.search_pane.selected_result = Some(0);
            app.search_pane.last_query = Some("result".to_string());
        }

        fn open_config(app: &mut App) {
            app.apply(AppAction::SetDetailMode(DetailMode::Config));
        }

        fn open_backups(app: &mut App) {
            app.apply(AppAction::SetDetailMode(DetailMode::Backup));
            app.backup_pane.backups = backups(9);
        }

        fn open_backups_empty(app: &mut App) {
            app.apply(AppAction::SetDetailMode(DetailMode::Backup));
            app.backup_pane.backups = Vec::new();
        }

        fn open_help(app: &mut App) {
            app.apply(AppAction::SetHelpContext(HelpContext::Overview));
            app.apply(AppAction::SetHelpOverlay(true));
        }

        struct Case {
            name: &'static str,
            open: fn(&mut App),
            /// Text the sheet must show, on screen or after scrolling.
            labels: Vec<String>,
            /// Keys the key line must offer, as "key label".
            keys: &'static [&'static str],
        }

        fn cases() -> Vec<Case> {
            let strings = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
            let version = format!("LemonUp {}", env!("CARGO_PKG_VERSION"));
            let mut config_labels: Vec<String> = ConfigField::ALL
                .iter()
                .map(|field| field.label().to_string())
                .collect();
            config_labels.extend(strings(&[
                "Credentials",
                "Updates",
                "Backups",
                "Display",
                "Selected:",
                "Changes save instantly",
            ]));
            // Titles and the short rows, which never wrap.
            let mut help_labels: Vec<String> = help_page(HelpContext::Overview)
                .sections()
                .flat_map(|section| {
                    std::iter::once(section.title.to_string()).chain(
                        section
                            .rows
                            .iter()
                            .filter(|row| row.when == HelpWhen::Always && row.text.len() <= 20)
                            .map(|row| row.text.to_string()),
                    )
                })
                .collect();
            help_labels.push(version.clone());
            vec![
                Case {
                    name: "inspect",
                    open: open_inspect,
                    labels: strings(&[
                        "Second",
                        "Not checked yet",
                        "From",
                        "GitHub",
                        "Author",
                        "Installed",
                        "Folders",
                        "Second_Config",
                        "Needs",
                        "Ace3",
                    ]),
                    keys: &["c check", "x delete", "space select"],
                },
                Case {
                    name: "inspect not linked",
                    open: open_inspect_not_linked,
                    labels: strings(&[
                        "Not linked",
                        "doesn't know where it came",
                        "Folders",
                        "Needs",
                        "nothing else",
                    ]),
                    keys: &["x delete", "space select"],
                },
                Case {
                    name: "inspect up to date",
                    open: open_inspect_up_to_date,
                    labels: strings(&["Up to date", "From", "Wago.io", "Made for", "WoW 11.0.5"]),
                    keys: &["c check", "x delete", "space select"],
                },
                Case {
                    name: "install empty",
                    open: open_install_empty,
                    labels: strings(&[
                        "Addon name or link",
                        "You can type or paste any of these:",
                        "an addon name",
                        "a GitHub link",
                        "a WoWInterface link",
                        "a Wago.io link",
                        "installs ElvUI or Tukui",
                    ]),
                    keys: &["type or paste", "esc close"],
                },
                Case {
                    name: "install with results",
                    open: open_install_results,
                    labels: strings(&[
                        "Addon name or link",
                        "Addon",
                        "Downloads",
                        "Result Addon 0",
                        "more results below",
                        "Summary for result 0",
                    ]),
                    keys: &["↑/↓ choose", "enter install", "esc close"],
                },
                Case {
                    name: "config",
                    open: open_config,
                    labels: config_labels,
                    keys: &["↑/↓ fields", "enter edit", "r setup"],
                },
                Case {
                    name: "backups",
                    open: open_backups,
                    labels: strings(&[
                        "Status: 9 backups ready",
                        "Store:",
                        "Recent backups",
                        "2026-04-00 12:00:00.000Z",
                        "Selected",
                        "File: WTF-20260400T120000.000Z.zip",
                        "Size: 2.0 KB",
                    ]),
                    keys: &["↑/↓ backups", "enter/r restore", "n backup", "x delete"],
                },
                Case {
                    name: "backups empty",
                    open: open_backups_empty,
                    labels: strings(&[
                        "Status: No backups yet",
                        "No backups of your game settings yet.",
                        "Press n to back up your game settings now.",
                    ]),
                    keys: &["n backup", "esc close"],
                },
                Case {
                    name: "help",
                    open: open_help,
                    labels: help_labels,
                    keys: &["esc or ? close"],
                },
            ]
        }

        /// Draws the sheet, then pages its body down to the end, keeping
        /// every screen on the way.
        fn screens_while_scrolling(app: &mut App, width: u16, height: u16) -> Vec<Vec<String>> {
            let mut screens = vec![render_rows(app, width, height)];
            for _ in 0..40 {
                let scroll = app.sheet_scroll.get();
                if scroll.offset >= scroll.max_offset {
                    break;
                }
                apply_all(app, AppMessage::SheetScroll(SheetMove::PageDown));
                screens.push(render_rows(app, width, height));
            }
            screens
        }

        fn has_more_below(screen: &[String]) -> bool {
            screen
                .iter()
                .any(|row| row.contains("↓ ") && row.contains(" more"))
        }

        #[test]
        fn every_sheet_shows_all_its_content_at_every_size() {
            for (width, height) in SIZES {
                for case in cases() {
                    let mut app = app_for_tests(ShellMode::Dashboard);
                    (case.open)(&mut app);
                    let screens = screens_while_scrolling(&mut app, width, height);
                    let first = screens[0].join("\n");
                    let all = screens
                        .iter()
                        .map(|screen| screen.join("\n"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let context = format!("{} at {width}x{height}:\n{first}", case.name);

                    for label in &case.labels {
                        assert!(all.contains(label.as_str()), "{label:?} missing, {context}");
                        if !first.contains(label.as_str()) {
                            assert!(
                                has_more_below(&screens[0]),
                                "{label:?} is below the fold with no ↓ more, {context}"
                            );
                        }
                    }
                    for (index, screen) in screens.iter().enumerate() {
                        let key_line = screen.last().expect("key line");
                        assert!(!key_line.trim().is_empty(), "blank key line, {context}");
                        if index + 1 < screens.len() {
                            assert!(has_more_below(screen), "no ↓ more, {context}");
                        }
                    }
                    let key_line = screens[0].last().expect("key line");
                    for key in case.keys {
                        assert!(
                            key_line.contains(key),
                            "{key:?} not on the key line, {context}"
                        );
                    }
                    // The title row always says how to close the sheet.
                    assert!(first.contains("esc close"), "{context}");
                }
            }
        }

        #[test]
        fn sheets_float_over_a_dimmed_list_at_100x30_and_up_and_fill_the_body_below() {
            for (width, height, full) in [
                (80, 24, true),
                (99, 40, true),
                (140, 29, true),
                (100, 30, false),
                (120, 30, false),
                (138, 36, false),
            ] {
                let window = Rect::new(0, 0, width, height);
                let region = Rect::new(0, 0, width, height - 2);
                let frame = SheetFrame::new(window, region, 2, 108);
                assert_eq!(frame.full, full, "{width}x{height}");
                let placed = frame.place(500);
                // Never taller than the space above the status and key lines.
                assert!(placed.outer.bottom() <= height - 2, "{width}x{height}");
                assert!(placed.outer.right() <= width, "{width}x{height}");
                if full {
                    assert_eq!(placed.outer.width, width);
                    assert_eq!(placed.outer.y, 0);
                } else {
                    assert_eq!(placed.outer.width, 108.min(width));
                }
            }

            // Content shorter than the window: the sheet is as tall as it.
            let window = Rect::new(0, 0, 138, 36);
            let frame = SheetFrame::new(window, Rect::new(0, 0, 138, 34), 2, 84);
            let placed = frame.place(10);
            assert_eq!(placed.body.height, 10);
            assert_eq!(placed.outer.height, 14);
            assert_eq!(placed.outer.width, 84);

            // A narrow window clamps a wide sheet.
            let window = Rect::new(0, 0, 104, 40);
            let frame = SheetFrame::new(window, Rect::new(0, 0, 104, 38), 2, 108);
            assert_eq!(frame.place(5).outer.width, 104);

            // The list behind a floating sheet is dimmed; the key line is not.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups_empty(&mut app);
            use ratatui::{Terminal, backend::TestBackend};
            let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
            terminal.draw(|frame| app.draw(frame)).expect("draw");
            let buffer = terminal.backend().buffer();
            let header_cell = buffer[(1, 0)].clone();
            assert_ne!(header_cell.fg, app.ui_theme.roles.lemon);
            let key_line_key = buffer[(1, 29)].clone();
            assert_eq!(key_line_key.fg, app.ui_theme.roles.blue);
        }

        #[test]
        fn sheet_text_wraps_at_words_and_measures_what_it_draws() {
            let line = Line::from(vec![
                Span::styled("Store: ", Style::default().fg(Color::Red)),
                Span::raw("one two three four"),
            ]);
            let rows: Vec<String> = wrap_lines(&[line], 12).iter().map(line_text).collect();
            assert_eq!(rows, vec!["Store: one", "two three", "four"]);
            // A word wider than the row is broken, never lost.
            let rows: Vec<String> = wrap_lines(&[Line::from("abcdefghij")], 4)
                .iter()
                .map(line_text)
                .collect();
            assert_eq!(rows, vec!["abcd", "efgh", "ij"]);
            // Blank lines keep their row; wide characters count as two.
            assert_eq!(wrap_lines(&[Line::from("")], 10).len(), 1);
            assert_eq!(wrap_lines(&[Line::from("📦📦📦")], 4).len(), 2);
        }

        #[test]
        fn floating_sheets_centre_below_the_header_and_grow_over_it_only_when_needed() {
            let window = Rect::new(0, 0, 120, 30);
            // Everything above the status line and the key line.
            let region = Rect::new(0, 0, 120, 28);
            let frame = SheetFrame::new(window, region, 2, 100);
            // Short: centred in rows 2 to 27, clear of the header.
            let placed = frame.place(10);
            assert_eq!((placed.outer.y, placed.outer.height), (8, 14));
            // Exactly the room below the header: still clear of it.
            let placed = frame.place(22);
            assert_eq!((placed.outer.y, placed.outer.bottom()), (2, 28));
            // Taller: grows upward over the header, one row at a time.
            let placed = frame.place(23);
            assert_eq!((placed.outer.y, placed.outer.bottom()), (1, 28));
            // Taller still: the whole region, then the body scrolls.
            let placed = frame.place(500);
            assert_eq!((placed.outer.y, placed.outer.bottom()), (0, 28));
            assert_eq!(placed.body.height, 24);

            // Drawn: the header and the status and key lines stay visible.
            for (width, height) in [(120, 30), (138, 36)] {
                for open in [
                    open_config as fn(&mut App),
                    open_backups_empty,
                    open_inspect,
                ] {
                    let mut app = app_for_tests(ShellMode::Dashboard);
                    open(&mut app);
                    let rows = render_rows(&mut app, width, height);
                    let text = rows.join("\n");
                    assert!(rows[0].contains("█   █▀▀"), "{text}");
                    assert!(rows[1].contains("3 addons"), "{text}");
                    let key_line = &rows[usize::from(height) - 1];
                    assert!(key_line.contains("? help"), "{text}");
                    assert!(!key_line.contains('│'), "{text}");
                    assert!(!rows[usize::from(height) - 2].contains('│'), "{text}");
                }
            }
            // A full screen sheet (below 100 x 30) still covers the header.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_config(&mut app);
            let rows = render_rows(&mut app, 80, 24);
            assert!(rows[0].starts_with(" Settings"), "{}", rows.join("\n"));
        }

        fn key_line_at(app: &mut App, width: u16) -> String {
            render_rows(app, width, 24);
            let line = line_text(&app.dashboard_key_line(width));
            assert!(line.chars().count() <= usize::from(width), "{line}");
            line
        }

        /// Entries in `line`, split on the two space separator.
        fn entries(line: &str) -> Vec<String> {
            line.trim()
                .split("  ")
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect()
        }

        #[test]
        fn the_key_line_fits_80_columns_by_tightening_then_dropping_by_priority() {
            // Settings on a setting that steps: too long for 80 columns.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_config(&mut app);
            press_key(&mut app, KeyCode::Down);
            let line = key_line_at(&mut app, 80);
            assert_eq!(
                entries(&line),
                [
                    "↑/↓ fields",
                    "enter edit",
                    "←/→ change",
                    "r setup",
                    "i import",
                    "esc close",
                    "? help"
                ],
                "{line}"
            );
            // Tighter still: the rest goes from the end; enter, esc and ? stay.
            let line = key_line_at(&mut app, 50);
            assert_eq!(
                entries(&line),
                ["↑/↓ fields", "enter edit", "esc close", "? help"],
                "{line}"
            );
            let line = key_line_at(&mut app, 30);
            assert_eq!(
                entries(&line),
                ["enter edit", "esc close", "? help"],
                "{line}"
            );

            // Backups fits at 80; at 60 it loses x delete first.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups(&mut app);
            let line = key_line_at(&mut app, 80);
            assert_eq!(
                entries(&line),
                [
                    "↑/↓ backups",
                    "enter/r restore",
                    "n backup",
                    "x delete",
                    "esc close",
                    "? help"
                ],
                "{line}"
            );
            let line = key_line_at(&mut app, 60);
            assert_eq!(
                entries(&line),
                [
                    "↑/↓ backups",
                    "enter/r restore",
                    "n backup",
                    "esc close",
                    "? help"
                ],
                "{line}"
            );

            // Questions: y and n never drop.
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.dashboard
                .set_pending_delete_folders(Some(vec!["Second".to_string()]));
            let line = key_line_at(&mut app, 80);
            assert_eq!(entries(&line), ["y yes", "n no", "esc no"], "{line}");
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups(&mut app);
            app.backup_pane.pending_restore = Some(backups(1).remove(0));
            let line = key_line_at(&mut app, 80);
            assert_eq!(entries(&line), ["y yes", "n no", "esc no"], "{line}");
            let line = key_line_at(&mut app, 22);
            assert_eq!(entries(&line), ["y yes", "n no", "esc no"], "{line}");
        }

        #[test]
        fn sheet_scroll_clamps_and_keeps_the_cursor_in_view() {
            // Everything fits: never scrolls.
            assert_eq!(offset_for_focus(4, (2, 3), 10, 10), 0);
            // Below the window: scroll just enough, leaving room for "↓ more".
            assert_eq!(offset_for_focus(0, (20, 21), 40, 10), 12);
            // The last row needs no "↓ more" line.
            assert_eq!(offset_for_focus(0, (39, 40), 40, 10), 30);
            // Above the window: the row above it carries "↑ more".
            assert_eq!(offset_for_focus(20, (5, 6), 40, 10), 4);
            assert_eq!(offset_for_focus(20, (0, 1), 40, 10), 0);
            // Already visible: stays put.
            assert_eq!(offset_for_focus(5, (8, 9), 40, 10), 5);

            let scroll = SheetScroll {
                kind: Some(OverlayKind::Help),
                offset: 3,
                focus: None,
                max_offset: 6,
                viewport: 10,
            };
            let kind = Some(OverlayKind::Help);
            assert_eq!(scroll.moved(SheetMove::LineUp, kind), 2);
            assert_eq!(scroll.moved(SheetMove::PageDown, kind), 6);
            assert_eq!(scroll.moved(SheetMove::PageUp, kind), 0);
            assert_eq!(scroll.moved(SheetMove::Bottom, kind), 6);
            assert_eq!(scroll.moved(SheetMove::Top, kind), 0);
            // A sheet not drawn yet starts from the top.
            assert_eq!(
                scroll.moved(SheetMove::LineDown, Some(OverlayKind::Config)),
                1
            );
        }

        #[test]
        fn help_scrolls_with_arrows_and_page_keys() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_help(&mut app);
            for (code, movement) in [
                (KeyCode::Down, SheetMove::LineDown),
                (KeyCode::Char('j'), SheetMove::LineDown),
                (KeyCode::Up, SheetMove::LineUp),
                (KeyCode::PageDown, SheetMove::PageDown),
                (KeyCode::PageUp, SheetMove::PageUp),
                (KeyCode::Home, SheetMove::Top),
                (KeyCode::End, SheetMove::Bottom),
            ] {
                assert_eq!(
                    app.messages_for_key(KeyEvent::from(code)),
                    vec![AppMessage::SheetScroll(movement)],
                    "{code:?}"
                );
            }

            // 80 x 20 is too short for all of Help.
            let rows = render_rows(&mut app, 80, 20);
            let text = rows.join("\n");
            assert!(has_more_below(&rows), "{text}");
            assert!(
                rows.last().expect("key line").contains("↑/↓ scroll"),
                "{text}"
            );
            press_key(&mut app, KeyCode::End);
            let rows = render_rows(&mut app, 80, 20);
            let text = rows.join("\n");
            assert!(text.contains("↑ "), "{text}");
            assert!(text.contains("quit from anywhere"), "{text}");
            assert!(!has_more_below(&rows), "{text}");

            // Reopened, Help starts at the top again.
            apply_all(&mut app, AppMessage::DashboardCloseOverlay);
            render_rows(&mut app, 80, 24);
            open_help(&mut app);
            let rows = render_rows(&mut app, 80, 24).join("\n");
            assert!(rows.contains("Your addon list"), "{rows}");
        }

        #[test]
        fn page_keys_scroll_every_sheet_and_arrows_still_move_cursors() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_config(&mut app);
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::PageDown)),
                vec![AppMessage::SheetScroll(SheetMove::PageDown)]
            );
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Down)),
                vec![AppMessage::ConfigSelectionNext]
            );

            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups(&mut app);
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Home)),
                vec![AppMessage::SheetScroll(SheetMove::Top)]
            );
            // With a question open, only its answers do anything.
            app.backup_pane.pending_restore = Some(backups(1).remove(0));
            assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Home)), vec![]);

            let mut app = app_for_tests(ShellMode::Dashboard);
            open_install_empty(&mut app);
            // The arrows never scroll Install: ↓ goes to results when there are some.
            assert_eq!(app.messages_for_key(KeyEvent::from(KeyCode::Down)), vec![]);
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::PageDown)),
                vec![AppMessage::SheetScroll(SheetMove::PageDown)]
            );
            open_install_results(&mut app);
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Down)),
                vec![AppMessage::SearchResultNext]
            );
        }

        #[test]
        fn config_keeps_the_cursor_in_view_at_80x24() {
            for (width, height) in [(80, 24), (80, 18), (120, 20)] {
                let mut app = app_for_tests(ShellMode::Dashboard);
                open_config(&mut app);
                for (index, field) in ConfigField::ALL.iter().enumerate() {
                    let rows = render_rows(&mut app, width, height);
                    let text = rows.join("\n");
                    let row = rows
                        .iter()
                        .find(|row| row.contains(field.label()) && row.contains('›'))
                        .unwrap_or_else(|| {
                            panic!(
                                "{index} {:?} not in view at {width}x{height}:\n{text}",
                                field
                            )
                        });
                    assert!(!row.contains("more"), "{text}");
                    press_key(&mut app, KeyCode::Down);
                }
                // And back up to the top.
                for _ in 0..ConfigField::ALL.len() {
                    press_key(&mut app, KeyCode::Up);
                    render_rows(&mut app, width, height);
                }
                let rows = render_rows(&mut app, width, height).join("\n");
                assert!(rows.contains("› Wago"), "{rows}");
            }
        }

        #[test]
        fn backups_always_show_the_selected_card_and_ask_in_the_dialog() {
            for (width, height) in SIZES {
                let mut app = app_for_tests(ShellMode::Dashboard);
                open_backups(&mut app);
                app.backup_pane.selected_backup = 8;
                let text = render_rows(&mut app, width, height).join("\n");
                assert!(text.contains("› 2026-04-08"), "{text}");
                assert!(text.contains("File: WTF-20260408"), "{text}");

                app.backup_pane.pending_delete = Some(backups(9).remove(8));
                let text = render_rows(&mut app, width, height).join("\n");
                assert!(text.contains("Delete this backup?"), "{text}");
                assert!(text.contains("Backup from 2026-04-08"), "{text}");
                assert!(text.contains("y  delete it"), "{text}");
                assert!(text.contains("n  keep it"), "{text}");
            }
        }

        fn key_line_states() -> Vec<(&'static str, App)> {
            let mut states = Vec::new();
            let mut app = app_for_tests(ShellMode::Dashboard);
            states.push(("list", app_for_tests(ShellMode::Dashboard)));
            open_inspect(&mut app);
            states.push(("inspect", app));
            for (name, open) in [
                ("install", open_install_empty as fn(&mut App)),
                ("results", open_install_results),
                ("config", open_config),
                ("backups", open_backups),
                ("backups empty", open_backups_empty),
                ("help", open_help),
            ] {
                let mut app = app_for_tests(ShellMode::Dashboard);
                open(&mut app);
                states.push((name, app));
            }
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_config(&mut app);
            app.config_pane.edit = Some(ConfigEditState {
                field: ConfigField::WagoApiKey,
                value: TextInput::default(),
            });
            states.push(("config editing", app));
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups(&mut app);
            app.backup_pane.pending_restore = Some(backups(1).remove(0));
            states.push(("restore question", app));
            states
        }

        #[test]
        fn the_key_table_drives_the_key_line() {
            for (name, mut app) in key_line_states() {
                render_rows(&mut app, 160, 40);
                let table = app.key_table(160);
                assert!(!table.is_empty(), "{name}");
                let expected = std::iter::once(" ".to_string())
                    .chain(table.iter().enumerate().map(|(index, hint)| {
                        let separator = if index > 0 { "   " } else { "" };
                        if hint.key.is_empty() {
                            format!("{separator}{}", hint.label)
                        } else {
                            format!("{separator}{} {}", hint.key, hint.label)
                        }
                    }))
                    .collect::<String>();
                assert_eq!(line_text(&app.dashboard_key_line(160)), expected, "{name}");
            }
        }

        #[test]
        fn the_key_line_only_offers_keys_that_work() {
            for (name, mut app) in key_line_states() {
                render_rows(&mut app, 120, 30);
                for hint in app.key_table(120) {
                    let Some(code) = hint.codes.first() else {
                        continue;
                    };
                    assert!(
                        !app.messages_for_key(KeyEvent::from(*code)).is_empty(),
                        "{name}: {} {} does nothing",
                        hint.key,
                        hint.label
                    );
                }
            }
        }

        #[test]
        fn the_key_table_drives_the_pulse() {
            for (name, mut app) in key_line_states() {
                render_rows(&mut app, 120, 30);
                let table: Vec<KeyHint> = app.key_table(120);
                for hint in &table {
                    for code in hint.codes {
                        let pulsed = app
                            .footer_hint_for_key(KeyEvent::from(*code))
                            .unwrap_or_else(|| panic!("{name}: {code:?} pulses nothing"));
                        assert!(
                            table.iter().any(|shown| shown.id == Some(pulsed)),
                            "{name}: {code:?} pulses {pulsed:?}, which is not on the line"
                        );
                    }
                }
            }

            // In Backups, enter pulses "enter/r restore", not "n backup".
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_backups(&mut app);
            assert_eq!(
                app.footer_hint_for_key(KeyEvent::from(KeyCode::Enter)),
                Some(FooterHintId::Confirm)
            );
            app.shell_ui.footer_pulse = Some(FooterKeyPulse {
                hint: FooterHintId::Confirm,
                expires_at_tick: app.shell_ui.motion.tick_count + 2,
            });
            let line = app.dashboard_key_line(120);
            let pulsed = line
                .spans
                .iter()
                .find(|span| span.content == "enter/r")
                .expect("enter/r key");
            assert_eq!(pulsed.style.bg, Some(app.ui_theme.roles.lemon));
        }

        fn press(app: &mut App, code: KeyCode) {
            for message in app.messages_for_key(KeyEvent::from(code)) {
                apply_all(app, message);
            }
        }

        /// The column of a character in a rendered row.
        fn column_of(row: &str, needle: char) -> Option<usize> {
            row.chars().position(|character| character == needle)
        }

        #[test]
        fn details_docks_at_120_columns_and_is_a_sheet_below() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            let rows = render_rows(&mut app, 119, 36);
            let text = rows.join("\n");
            assert!(!app.details_docked(), "{text}");
            assert!(text.contains("╭─ Second"), "{text}");
            assert!(!text.contains('┼'), "{text}");

            let rows = render_rows(&mut app, 120, 36);
            let text = rows.join("\n");
            assert!(app.details_docked(), "{text}");
            assert!(!text.contains("╭─"), "{text}");
            // The divider joins the rule under the column header.
            assert_eq!(column_of(&rows[4], '┼'), Some(78), "{text}");
            assert_eq!(column_of(&rows[3], '│'), Some(78), "{text}");
            assert_eq!(column_of(&rows[6], '│'), Some(78), "{text}");
            assert!(
                rows[3][..].ends_with("│ Second") || rows[3].contains("│ Second"),
                "{text}"
            );
            // The list keeps Addon, Status and Version only.
            assert!(rows[3].contains("Version"), "{text}");
            assert!(!rows[3].contains("Author"), "{text}");
            assert!(!rows[3].contains("Source"), "{text}");
            // Header and the bottom lines still span the window.
            assert!(rows[0].contains("█   █▀▀"), "{text}");
            assert!(!rows[34].contains('│'), "{text}");
            assert!(rows[35].contains("enter or esc close details"), "{text}");
            assert!(rows[35].contains("↑/↓ move"), "{text}");
        }

        #[test]
        fn docked_columns_are_78_1_41_at_120_and_the_panel_stops_near_52() {
            let columns = crate::app::render_dashboard::ListColumns::for_docked(78);
            assert_eq!(
                (
                    columns.name,
                    columns.status,
                    columns.version,
                    columns.author,
                    columns.source
                ),
                (30, 21, 22, 0, 0)
            );
            assert_eq!(columns.total(), 78);
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            for (width, list) in [(120u16, 78usize), (130, 83), (160, 107)] {
                let rows = render_rows(&mut app, width, 40);
                assert_eq!(column_of(&rows[4], '┼'), Some(list), "{width}");
                let panel = usize::from(width) - list - 1;
                assert!(panel <= 52, "{width}");
                assert!(rows[4].chars().count() == usize::from(width), "{width}");
            }
            assert_eq!(render_rows(&mut app, 120, 40)[4].chars().count(), 120);
        }

        #[test]
        fn docked_details_that_does_not_fit_ends_in_more() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            let rows = render_rows(&mut app, 120, 14);
            let text = rows.join("\n");
            // Six panel rows: five of content, then the count of the rest.
            let panel: Vec<String> = rows[5..11]
                .iter()
                .map(|row| row.chars().skip(79).collect())
                .collect();
            assert!(panel[5].trim_start().starts_with("↓ "), "{text}");
            assert!(panel[5].trim_end().ends_with(" more"), "{text}");
            assert!(panel[1].contains("Not checked yet"), "{text}");
        }

        #[test]
        fn resizing_switches_between_dock_and_sheet_without_closing() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            render_rows(&mut app, 120, 36);
            assert!(app.details_docked());
            let text = render_rows(&mut app, 110, 36).join("\n");
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Inspect));
            assert!(text.contains("╭─ Second"), "{text}");
            render_rows(&mut app, 120, 36);
            assert!(app.details_docked());
        }

        #[test]
        fn docked_esc_closes_details_then_clears_the_filter_then_the_selection() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDashboardFilter {
                query: "e".into(),
                editing: false,
            });
            app.apply(AppAction::SetSelectedDashboardParents(vec![
                "Second".to_string(),
            ]));
            open_inspect(&mut app);
            render_rows(&mut app, 120, 36);
            assert!(app.details_docked());

            press(&mut app, KeyCode::Esc);
            assert_eq!(app.shell_ui.overlay.active, None);
            assert!(app.dashboard.has_filter());
            press(&mut app, KeyCode::Esc);
            assert!(!app.dashboard.has_filter());
            assert_eq!(app.dashboard.selected_parent_count(), 1);
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.dashboard.selected_parent_count(), 0);
        }

        #[test]
        fn q_and_enter_close_details_and_q_quits_only_from_the_list() {
            for width in [80, 120] {
                for code in [KeyCode::Char('q'), KeyCode::Enter] {
                    let mut app = app_for_tests(ShellMode::Dashboard);
                    open_inspect(&mut app);
                    render_rows(&mut app, width, 36);
                    assert_eq!(
                        app.messages_for_key(KeyEvent::from(code)),
                        vec![AppMessage::DashboardCloseOverlay],
                        "{width} {code:?}"
                    );
                }
            }
            let app = app_for_tests(ShellMode::Dashboard);
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
                vec![AppMessage::QuitRequested]
            );
        }

        #[test]
        fn docked_details_keeps_the_list_keys_and_the_mouse() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            render_rows(&mut app, 120, 36);
            for (code, message) in [
                (KeyCode::Down, AppMessage::DashboardSelectionNext),
                (KeyCode::Char('/'), AppMessage::DashboardFilterBegin),
                (KeyCode::Char('U'), AppMessage::DashboardRunUpdateAll),
                (KeyCode::Char('C'), AppMessage::DashboardRunCheckAll),
                (KeyCode::Char('u'), AppMessage::DashboardRunUpdateSelected),
                (
                    KeyCode::Char('1'),
                    AppMessage::DashboardToggleSort(DashboardSortColumn::Name),
                ),
            ] {
                assert_eq!(
                    app.messages_for_key(KeyEvent::from(code)),
                    vec![message],
                    "{code:?}"
                );
            }
            // ↓ moves the list cursor and the panel follows it.
            let before = app
                .inspect_resolved_target()
                .expect("target")
                .item
                .folder
                .clone();
            press(&mut app, KeyCode::Down);
            let after = app
                .inspect_resolved_target()
                .expect("target")
                .item
                .folder
                .clone();
            assert_ne!(before, after);
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Inspect));
            let text = render_rows(&mut app, 120, 36).join("\n");
            let name = app
                .inspect_resolved_target()
                .expect("target")
                .item
                .name
                .clone();
            assert!(text.contains(&format!("│ {name}")), "{text}");

            // The list area stops at the divider: clicks in the panel do nothing.
            let area = app.last_dashboard_list_area.expect("list area");
            assert_eq!(area.width, 78);
            let click = |column: u16| MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            };
            assert_eq!(
                app.messages_for_mouse(click(10)),
                vec![AppMessage::DashboardPointerSelect {
                    column: 10,
                    row: area.y
                }]
            );
            assert_eq!(app.dashboard_selection_for_pointer(100, area.y), None);

            // Typing into the filter wins over closing Details.
            press(&mut app, KeyCode::Char('/'));
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
                vec![AppMessage::DashboardFilterInputChar('q')]
            );
        }

        #[test]
        fn details_offers_update_only_when_an_update_is_ready() {
            let keys = |app: &App| -> Vec<String> {
                app.key_table(80)
                    .iter()
                    .map(|hint| format!("{} {}", hint.key, hint.label))
                    .collect()
            };
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect_up_to_date(&mut app);
            assert!(
                !keys(&app).contains(&"u update".to_string()),
                "{:?}",
                keys(&app)
            );
            assert!(keys(&app).contains(&"c check".to_string()));
            let third = app
                .dashboard
                .items
                .iter_mut()
                .find(|item| item.folder == "Third")
                .expect("third");
            third.remote_version = Some("3.1.0".to_string());
            assert!(
                keys(&app).contains(&"u update".to_string()),
                "{:?}",
                keys(&app)
            );
            let text = render_rows(&mut app, 80, 24).join("\n");
            assert!(text.contains("↑ Update ready      3.0.0 → 3.1.0"), "{text}");

            // Not linked: nothing to check or update.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect_not_linked(&mut app);
            let shown = keys(&app);
            assert!(
                !shown
                    .iter()
                    .any(|key| key.starts_with("u ") || key.starts_with("c ")),
                "{shown:?}"
            );
        }

        #[test]
        fn help_lists_the_details_keys_without_section_toggles() {
            let labels: Vec<&str> = help_page(HelpContext::Inspect)
                .sections()
                .flat_map(|section| section.rows.iter())
                .filter_map(|row| match row.label {
                    HelpLabel::Keys(keys) => Some(keys),
                    _ => None,
                })
                .collect();
            assert!(labels.contains(&"↑ ↓  or  j k"), "{labels:?}");
            assert!(labels.contains(&"enter  esc  q"), "{labels:?}");
            for gone in ["r", "d", "t"] {
                assert!(!labels.contains(&gone), "{labels:?}");
            }

            // Help opened from Details goes back to Details.
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_inspect(&mut app);
            press(&mut app, KeyCode::Char('?'));
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Help));
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Inspect));
        }

        /// The state each Help group's keys work in. A new group needs a
        /// state here, or the test below fails.
        fn help_state(context: HelpContext, title: &str) -> fn(&mut App) {
            fn list(_: &mut App) {}
            fn filter_typing(app: &mut App) {
                apply_all(app, AppMessage::DashboardFilterBegin);
            }
            fn question(app: &mut App) {
                app.pending_wago_install_confirmation = Some(pending_wago_confirmation());
            }
            fn docked(app: &mut App) {
                app.last_frame_width = Some(160);
                open_inspect(app);
            }
            fn install_typing(app: &mut App) {
                apply_all(
                    app,
                    AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst),
                );
            }
            fn install_key_box(app: &mut App) {
                install_typing(app);
                app.wago_api_key = None;
                app.search_pane.query = "bigwigs".into();
                apply_all(app, AppMessage::SearchSubmit);
            }
            fn install_link(app: &mut App) {
                install_typing(app);
                app.search_pane.query = "https://github.com/WeakAuras/WeakAuras2".into();
                apply_all(app, AppMessage::SearchSubmit);
            }
            fn settings(app: &mut App) {
                open_config(app);
                // A setting with a value to change, not the key box.
                app.config_pane.selected_field = 1;
            }
            fn settings_editing(app: &mut App) {
                open_config(app);
                apply_all(app, AppMessage::ConfigBeginEditing);
            }
            fn questions_in(open: fn(&mut App), app: &mut App) {
                open(app);
                question(app);
            }
            match (context, title) {
                (HelpContext::Overview, "While you type a filter") => filter_typing,
                (HelpContext::Overview, "Questions") => question,
                (HelpContext::Overview, "What the status means") => list,
                (HelpContext::Overview, _) => list,
                (HelpContext::Inspect, "Questions") => |app| questions_in(open_inspect, app),
                (HelpContext::Inspect, _) => open_inspect,
                (HelpContext::DetailsDocked, "Questions") => |app| questions_in(docked, app),
                (HelpContext::DetailsDocked, _) => docked,
                (HelpContext::InstallSearch, "Typing in the box" | "What you can type") => {
                    install_typing
                }
                (HelpContext::InstallSearch, "Install from a link?") => install_link,
                (HelpContext::InstallSearch, "Your Wago.io key") => install_key_box,
                (HelpContext::InstallSearch, "Questions") => {
                    |app| questions_in(open_install_results, app)
                }
                (HelpContext::InstallSearch, _) => open_install_results,
                (HelpContext::Config, "While you edit a value") => settings_editing,
                (HelpContext::Config, _) => settings,
                (HelpContext::Backup, "Questions") => |app| questions_in(open_backups, app),
                (HelpContext::Backup, _) => open_backups,
            }
        }

        const HELP_CONTEXTS: [HelpContext; 6] = [
            HelpContext::Overview,
            HelpContext::Inspect,
            HelpContext::DetailsDocked,
            HelpContext::InstallSearch,
            HelpContext::Config,
            HelpContext::Backup,
        ];

        fn help_test_app(context: HelpContext, title: &str, when: HelpWhen) -> App {
            let mut app = app_for_tests(ShellMode::Dashboard);
            match when {
                HelpWhen::Always => {}
                HelpWhen::NoWagoKey => app.wago_api_key = None,
                HelpWhen::NotLinked => {}
                HelpWhen::FirstRunCard => {
                    app.dashboard = DashboardState::from_addons(vec![
                        AddonRecord::new("First", "First", SourceKind::Manual),
                        AddonRecord::new("Other", "Other", SourceKind::Manual),
                    ]);
                }
            }
            help_state(context, title)(&mut app);
            // The cursor (and Details) on the one addon that is not linked.
            if when == HelpWhen::NotLinked {
                let index = app
                    .dashboard
                    .rows
                    .iter()
                    .position(|row| row.folder == "First")
                    .expect("First row");
                app.dashboard.list_state.select(Some(index));
            }
            assert!(app.help_row_applies(when), "{context:?} {title}");
            // As drawn: a question answers keys once it is on screen.
            app.note_question_shown(Instant::now());
            app
        }

        #[test]
        fn every_key_help_lists_works_where_it_lists_it() {
            for context in HELP_CONTEXTS {
                for section in help_page(context).sections() {
                    for row in section.rows {
                        match row.label {
                            HelpLabel::Keys(label) => {
                                assert!(!row.keys.is_empty(), "{context:?} {label}");
                                for key in row.keys {
                                    let app = help_test_app(context, section.title, row.when);
                                    assert!(
                                        !app.messages_for_key(*key).is_empty(),
                                        "{context:?} / {}: {label} ({key:?}) does nothing",
                                        section.title
                                    );
                                }
                            }
                            // Status words and things to type are not keys.
                            HelpLabel::Status(_) | HelpLabel::Typed(_) => {
                                assert!(row.keys.is_empty(), "{context:?} {}", row.text);
                            }
                        }
                    }
                }
            }
        }

        #[test]
        fn every_key_line_key_is_in_help_and_opens_it_there() {
            for context in HELP_CONTEXTS {
                let page = help_page(context);
                let listed: Vec<KeyEvent> = page
                    .sections()
                    .flat_map(|section| section.rows.iter())
                    .flat_map(|row| row.keys.iter().copied())
                    .collect();
                for section in page.sections() {
                    let mut whens = vec![HelpWhen::Always];
                    whens.extend(section.rows.iter().map(|row| row.when));
                    for when in whens {
                        let app = help_test_app(context, section.title, when);
                        for hint in app.key_table(u16::MAX) {
                            for code in hint.codes {
                                assert!(
                                    listed.contains(&KeyEvent::from(*code)),
                                    "{context:?} / {}: key line {} {} ({code:?}) is not in Help",
                                    section.title,
                                    hint.key,
                                    hint.label
                                );
                            }
                        }
                    }
                }
                // `?` opens this page from the context's own state.
                let mut app = help_test_app(context, "", HelpWhen::Always);
                press(&mut app, KeyCode::Char('?'));
                assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Help));
                assert_eq!(app.shell_ui.help_context, context);
            }
        }

        fn nothing_linked_app(config_file: PathBuf) -> App {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.config_store = ConfigStore::new(config_file);
            app.dashboard = DashboardState::from_addons(vec![
                AddonRecord::new("Alpha", "Alpha", SourceKind::Manual),
                AddonRecord::new("Beta", "Beta", SourceKind::Manual),
            ]);
            app
        }

        #[test]
        fn nothing_linked_shows_the_welcome_card_until_d_hides_it_for_good() {
            let temp = tempdir().expect("tempdir");
            let config_file = temp.path().join("config.toml");
            let mut app = nothing_linked_app(config_file.clone());
            for (width, height) in SIZES {
                let rows = render_rows(&mut app, width, height);
                let text = rows.join("\n");
                assert!(text.contains("Welcome. One thing before LemonUp"), "{text}");
                assert!(text.contains("LemonUp found 2 addons"), "{text}");
                assert!(text.contains("d hide this"), "{text}");
                assert!(text.contains("Alpha"), "{text}");
                assert!(text.contains("f find online"), "{text}");
                let keys = rows.last().expect("key line");
                assert!(keys.contains("f find online"), "{text}");
                assert!(!keys.contains("C check"), "{text}");
                if width >= 120 {
                    assert!(keys.contains("d hide card"), "{text}");
                }
            }
            // Help lists `d` while the card is there.
            press_key(&mut app, KeyCode::Char('?'));
            let text = render_rows(&mut app, 120, 60).join("\n");
            assert!(text.contains("hide the welcome card"), "{text}");
            press_key(&mut app, KeyCode::Esc);

            press_key(&mut app, KeyCode::Char('d'));
            let text = render_rows(&mut app, 120, 30).join("\n");
            assert!(!text.contains("Welcome. One thing"), "{text}");
            assert!(!text.contains("hide card"), "{text}");
            assert!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('d')))
                    .is_empty()
            );

            // Saved, so it stays hidden after a restart.
            let ConfigLoad::Loaded(saved) = app.config_store.load().expect("load") else {
                panic!("config written");
            };
            assert!(saved.hide_first_run_card);
            let mut restarted = nothing_linked_app(config_file);
            restarted.config_pane = ConfigPaneState::new(saved);
            let text = render_rows(&mut restarted, 120, 30).join("\n");
            assert!(!text.contains("Welcome. One thing"), "{text}");

            // One linked addon and the card has nothing to say.
            let mut app = app_for_tests(ShellMode::Dashboard);
            let text = render_rows(&mut app, 120, 30).join("\n");
            assert!(!text.contains("Welcome. One thing"), "{text}");
        }

        #[test]
        fn an_empty_list_says_why_and_offers_only_keys_that_work() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.dashboard = DashboardState::from_addons(Vec::new());

            app.scan_state = ScanState::Running(PathBuf::from("/sandbox/AddOns"));
            let rows = render_rows(&mut app, 80, 24);
            let text = rows.join("\n");
            assert!(
                text.contains("Looking through your AddOns folder…"),
                "{text}"
            );
            assert_eq!(rows[23].trim(), "? help  q quit", "{text}");

            app.scan_state = ScanState::Succeeded {
                path: PathBuf::from("/sandbox/AddOns"),
                summary: ScanSummary::default(),
            };
            for (width, height) in SIZES {
                let rows = render_rows(&mut app, width, height);
                let text = rows.join("\n");
                assert!(text.contains("Your AddOns folder is empty."), "{text}");
                assert!(text.contains("i  install your first addon"), "{text}");
                assert!(text.contains(",  settings, where r chooses"), "{text}");
                assert!(!text.contains("wait"), "{text}");
                let keys = rows.last().expect("key line");
                assert!(
                    keys.contains("i install") && keys.contains(", settings"),
                    "{text}"
                );
                assert!(!keys.contains("check"), "{text}");
            }

            app.scan_state =
                ScanState::Failed("No such file or directory (os error 2)".to_string());
            let rows = render_rows(&mut app, 120, 30);
            let text = rows.join("\n");
            assert!(
                text.contains("LemonUp couldn't read your AddOns folder."),
                "{text}"
            );
            assert!(text.contains("may have moved"), "{text}");
            assert!(text.contains("Technical detail: No such file"), "{text}");
            assert!(rows[29].contains(", settings"), "{text}");
            assert!(!rows[29].contains("i install"), "{text}");

            // A kept filter that matches nothing.
            let mut app = app_for_tests(ShellMode::Dashboard);
            press_key(&mut app, KeyCode::Char('/'));
            type_text(&mut app, "zzzz");
            press_key(&mut app, KeyCode::Enter);
            let rows = render_rows(&mut app, 80, 24);
            let text = rows.join("\n");
            assert!(
                text.contains("Nothing you have installed matches \"zzzz\"."),
                "{text}"
            );
            assert!(rows[23].contains("esc clear filter"), "{text}");
            assert!(!rows[23].contains("enter"), "{text}");
        }

        #[test]
        fn help_shows_two_columns_when_wide_and_one_when_narrow() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            open_help(&mut app);
            let rows = render_rows(&mut app, 120, 40);
            let text = rows.join("\n");
            assert!(text.contains("Help: your addon list"), "{text}");
            assert!(
                rows.iter()
                    .any(|row| row.contains("Your addon list")
                        && row.contains("Getting things done")),
                "{text}"
            );
            assert!(!text.contains("hide the welcome card"), "{text}");
            let rows = render_rows(&mut app, 80, 60);
            let text = rows.join("\n");
            assert!(
                !rows
                    .iter()
                    .any(|row| row.contains("Your addon list")
                        && row.contains("Getting things done")),
                "{text}"
            );
        }
    }

    mod direct_install_overlay {
        use super::*;
        use crate::app::direct_install::DirectInstallProvider;
        use crate::app::{DirectInstallOutcome, DirectInstallRequest};
        use lemonup_core::ScanSummary;

        const ADDON_DIR: &str = "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns";

        fn search_app(query: &str) -> App {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDetailMode(DetailMode::Search));
            app.effective_addon_dir = Some(PathBuf::from(ADDON_DIR));
            app.search_pane.query = query.to_string().into();
            app
        }

        /// Applies the resulting actions except the background task spawn, so no provider
        /// network call runs; tests assert on the `StartDirectInstall` action instead.
        fn run(app: &mut App, message: AppMessage) -> Vec<AppAction> {
            let actions = app.update(message);
            for action in actions.clone() {
                if !matches!(action, AppAction::StartDirectInstall { .. }) {
                    app.apply(action);
                }
            }
            actions
        }

        fn request(
            provider: DirectInstallProvider,
            target: &str,
            display: &str,
        ) -> DirectInstallRequest {
            DirectInstallRequest {
                provider,
                target: target.to_string(),
                display: display.to_string(),
            }
        }

        fn start_install_request(actions: &[AppAction]) -> Option<&DirectInstallRequest> {
            actions.iter().find_map(|action| match action {
                AppAction::StartDirectInstall { request, .. } => Some(request),
                _ => None,
            })
        }

        fn sync_outcome() -> AddonScanOutcome {
            AddonScanOutcome {
                path: PathBuf::from(ADDON_DIR),
                summary: ScanSummary {
                    scanned_addons: 1,
                    upserted_addons: 1,
                    removed_addons: 0,
                    ..Default::default()
                },
                addons: Vec::new(),
                drift_report: DriftReport::empty(),
            }
        }

        #[test]
        fn the_success_line_names_the_installed_addon_not_the_repo_slug() {
            let mut sync = sync_outcome();
            let mut hekili = AddonRecord::new("Hekili", "Hekili", SourceKind::GitHub);
            hekili.name = "Hekili".to_string();
            sync.addons = vec![hekili];
            assert_eq!(
                crate::app::direct_install::installed_addon_name(&sync, "Hekili").as_deref(),
                Some("Hekili")
            );
            // Nothing known: the caller keeps the repo name.
            assert_eq!(
                crate::app::direct_install::installed_addon_name(&sync, "Other"),
                None
            );
        }

        #[test]
        fn github_url_asks_for_confirmation_then_installs_without_wago_key() {
            let mut app = search_app("https://github.com/WeakAuras/WeakAuras2");
            app.wago_api_key = None;
            let expected = request(
                DirectInstallProvider::Github,
                "https://github.com/WeakAuras/WeakAuras2",
                "WeakAuras/WeakAuras2",
            );

            let actions = app.update(AppMessage::SearchSubmit);
            assert_eq!(
                actions,
                vec![AppAction::SetSearchPaneState(
                    app.search_pane.request_direct_install(expected.clone())
                )]
            );
            // The question asks once, by the addon's name.
            assert_eq!(expected.name(), "WeakAuras2");
            for action in actions {
                app.apply(action);
            }
            assert_eq!(app.search_pane.pending_direct_install(), Some(&expected));

            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
                vec![AppMessage::SearchConfirmDirectInstall]
            );
            let actions = run(&mut app, AppMessage::SearchConfirmDirectInstall);
            assert!(actions.contains(&AppAction::SetDirectInstallInProgress(true)));
            assert!(actions.contains(&AppAction::StartDirectInstall {
                addon_dir: PathBuf::from(ADDON_DIR),
                request: expected.clone(),
                replace_folders: Vec::new(),
            }));
            assert_eq!(
                app.search_pane.install_state,
                SearchInstallState::InstallingDirect { request: expected }
            );
            assert!(app.direct_install_in_progress);
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));

            // A second submit while the install runs is refused.
            app.search_pane.query = "details".into();
            assert_eq!(
                app.update(AppMessage::SearchSubmit),
                vec![
                    AppAction::SetSearchPaneState(app.search_pane.clone()),
                    AppAction::SetStatus(app.dashboard_status_for(
                        StatusKind::Note,
                        DetailMode::Search,
                        "an addon install is already running",
                    ))
                ]
            );
        }

        #[test]
        fn esc_cancels_pending_direct_install_and_resumes_editing() {
            let mut app = search_app("elvui");
            run(&mut app, AppMessage::SearchSubmit);
            assert!(app.search_pane.pending_direct_install().is_some());
            // A letter answers "not now" and types it into the box.
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('x'))),
                vec![
                    AppMessage::SearchCancelDirectInstall,
                    AppMessage::SearchInputChar('x')
                ]
            );

            let messages = app.messages_for_key(KeyEvent::from(KeyCode::Esc));
            assert_eq!(messages, vec![AppMessage::SearchCancelDirectInstall]);
            let actions = run(&mut app, AppMessage::SearchCancelDirectInstall);

            assert!(start_install_request(&actions).is_none());
            assert_eq!(app.search_pane.install_state, SearchInstallState::Idle);
            assert_eq!(app.focused_text_box(), Some(TextBox::Search));
            assert_eq!(app.search_pane.query, "elvui");
            assert!(!app.direct_install_in_progress);
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
        }

        #[test]
        fn install_task_carries_provider_and_cli_target_for_each_source() {
            let cases = [
                (
                    "https://www.wowinterface.com/downloads/info25687-ElvUI_WindTools.html",
                    request(DirectInstallProvider::Wowinterface, "25687", "addon 25687"),
                ),
                (
                    "elvui",
                    request(DirectInstallProvider::Tukui, "elvui", "ElvUI"),
                ),
                (
                    "tukui",
                    request(DirectInstallProvider::Tukui, "tukui", "Tukui"),
                ),
                (
                    "https://github.com/AdiAddons/AdiBags",
                    request(
                        DirectInstallProvider::Github,
                        "https://github.com/AdiAddons/AdiBags",
                        "AdiAddons/AdiBags",
                    ),
                ),
            ];
            for (query, expected) in cases {
                let mut app = search_app(query);
                run(&mut app, AppMessage::SearchSubmit);
                let actions = run(&mut app, AppMessage::SearchConfirmDirectInstall);
                assert_eq!(start_install_request(&actions), Some(&expected), "{query}");
            }
        }

        #[test]
        fn wago_inputs_without_key_keep_todays_blocked_status() {
            for query in ["WeakAuras", "https://addons.wago.io/addons/VBNBxKx5"] {
                let mut app = search_app(query);
                app.wago_api_key = None;
                // The key box opens in place; nothing leaves the sheet.
                assert_eq!(
                    app.update(AppMessage::SearchSubmit),
                    vec![AppAction::SetSearchPaneState(
                        app.search_pane.open_key_box()
                    )],
                    "{query}"
                );
            }
        }

        #[test]
        fn unparsable_url_shows_error_without_closing_overlay() {
            let mut app = search_app("https://github.com/WeakAuras");
            app.wago_api_key = None;
            let actions = run(&mut app, AppMessage::SearchSubmit);

            assert!(start_install_request(&actions).is_none());
            assert!(
                !actions
                    .iter()
                    .any(|action| matches!(action, AppAction::StartWagoSearch { .. }))
            );
            // The line under the box says why enter does nothing.
            assert_eq!(app.search_pane.install_state, SearchInstallState::Idle);
            let hint = app.search_hint_line().to_string();
            assert!(hint.contains("github.com/<owner>/<repo>"), "{hint}");
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
        }

        #[test]
        fn direct_install_success_and_failure_events_update_overlay_state() {
            let wowi = request(DirectInstallProvider::Wowinterface, "25687", "addon 25687");
            let mut app = search_app("");
            app.direct_install_in_progress = true;
            app.search_pane = app.search_pane.start_direct_install(wowi.clone());

            let actions = run(
                &mut app,
                AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                    request: wowi.clone(),
                    result: Ok(DirectInstallOutcome {
                        addon_name: "ElvUI WindTools".to_string(),
                        parent_folder: "ElvUI_WindTools".to_string(),
                        warning: Some("WoWInterface does not tag WoW Forever builds".to_string()),
                        sync: sync_outcome(),
                    }),
                }),
            );
            assert!(actions.contains(&AppAction::SetDirectInstallInProgress(false)));
            assert!(
                actions
                    .iter()
                    .any(|action| matches!(action, AppAction::ReplaceDashboardAddons(_)))
            );
            assert!(!app.direct_install_in_progress);
            assert_eq!(
                app.search_pane.install_state,
                SearchInstallState::Success {
                    addon_id: "25687".to_string(),
                    addon_name: "ElvUI WindTools".to_string(),
                    parent_folder: "ElvUI_WindTools".to_string(),
                    warning: Some("WoWInterface does not tag WoW Forever builds".to_string()),
                }
            );
            assert!(
                app.status_line
                    .text
                    .contains("ElvUI WindTools is installed from WoWInterface")
            );
            assert!(
                app.status_line
                    .text
                    .contains("| WoWInterface does not tag WoW Forever builds")
            );

            let tukui = request(DirectInstallProvider::Tukui, "elvui", "ElvUI");
            app.direct_install_in_progress = true;
            app.search_pane = app.search_pane.start_direct_install(tukui.clone());
            let refusal = "TukUI addon 'ElvUI' does not list WoW Forever support".to_string();
            run(
                &mut app,
                AppMessage::BackgroundTask(AppTaskEvent::DirectInstallFinished {
                    request: tukui,
                    result: Err(SourceError::from(refusal.clone())),
                }),
            );
            assert!(!app.direct_install_in_progress);
            assert_eq!(
                app.search_pane.install_state,
                SearchInstallState::Error {
                    addon_id: Some("elvui".to_string()),
                    addon_name: Some("ElvUI".to_string()),
                    provider: "TukUI",
                    kind: SourceErrorKind::Other,
                    detail: refusal,
                    clash: None,
                }
            );
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Search));
        }

        #[test]
        fn empty_overlay_renders_source_hints_and_wago_key_hint() {
            use ratatui::{Terminal, backend::TestBackend};

            let mut app = search_app("");
            app.wago_api_key = None;
            let mut terminal = Terminal::new(TestBackend::new(160, 45)).expect("terminal");
            terminal.draw(|frame| app.draw(frame)).expect("draw");
            let buffer = terminal.backend().buffer().clone();
            let width = buffer.area.width as usize;
            let rows = buffer
                .content()
                .chunks(width)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");

            assert!(rows.contains("Install an addon"), "{rows}");
            assert!(rows.contains("Addon name or link"), "{rows}");
            assert!(
                rows.contains("You can type or paste any of these:"),
                "{rows}"
            );
            assert!(rows.contains("github.com/BigWigsMods/LittleWigs"), "{rows}");
            assert!(rows.contains("installs ElvUI or Tukui"), "{rows}");
            // No detour to Settings, no label lines.
            assert!(!rows.contains("press , to open"), "{rows}");
            assert!(!rows.contains("Use:"), "{rows}");
            assert!(!rows.contains("Status:"), "{rows}");
        }

        #[test]
        fn confirm_overlay_states_the_prompt_once() {
            use ratatui::{Terminal, backend::TestBackend};

            let mut app = search_app("https://github.com/WeakAuras/WeakAuras2");
            app.wago_api_key = None;
            for action in app.update(AppMessage::SearchSubmit) {
                app.apply(action);
            }
            let mut terminal = Terminal::new(TestBackend::new(160, 45)).expect("terminal");
            terminal.draw(|frame| app.draw(frame)).expect("draw");
            let buffer = terminal.backend().buffer().clone();
            let width = buffer.area.width as usize;
            let rows = buffer
                .content()
                .chunks(width)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");

            assert!(rows.contains("That's a GitHub link."), "{rows}");
            assert_eq!(
                rows.matches("Install WeakAuras2 from GitHub?").count(),
                1,
                "{rows}"
            );
            assert!(rows.contains("github.com/WeakAuras/WeakAuras2"), "{rows}");
            let key_line = rows.lines().last().expect("key line");
            assert!(key_line.contains("enter install"), "{rows}");
            assert!(key_line.contains("esc not now"), "{rows}");
        }

        #[test]
        fn header_drops_the_tagline_and_version() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let rows = render_rows(&mut app, 160, 45).join("\n");

            assert!(!rows.contains("World of Warcraft addon manager"), "{rows}");
            assert!(!rows.contains("v2"), "{rows}");
            assert!(rows.contains("█   █▀▀ █▀▄▀█"), "{rows}");
        }
    }

    mod questions {
        use super::*;
        use crate::app::dialog::Question;

        const SIZES: [(u16, u16); 2] = [(80, 24), (120, 36)];

        fn apply_all(app: &mut App, message: AppMessage) {
            for action in app.update(message) {
                app.apply(action);
            }
        }

        fn backup(label: &str) -> BackupEntry {
            BackupEntry {
                file_name: "WTF-20260401T120000.000Z.zip".to_string(),
                path: PathBuf::from("/sandbox/WTF-1.zip"),
                label: label.to_string(),
                size_bytes: 2048,
            }
        }

        /// The keys every question shares: y yes, n esc q no, enter and
        /// anything else nothing.
        fn assert_answers(app: &App, yes: AppMessage, no: AppMessage) {
            assert_eq!(
                app.messages_for_key(KeyEvent::from(KeyCode::Char('y'))),
                vec![yes]
            );
            for code in [KeyCode::Char('n'), KeyCode::Esc, KeyCode::Char('q')] {
                assert_eq!(
                    app.messages_for_key(KeyEvent::from(code)),
                    vec![no.clone()],
                    "{code:?}"
                );
            }
            for code in [
                KeyCode::Enter,
                KeyCode::Char('x'),
                KeyCode::Down,
                KeyCode::Home,
            ] {
                assert_eq!(
                    app.messages_for_key(KeyEvent::from(code)),
                    vec![],
                    "{code:?}"
                );
            }
            let keys: Vec<String> = app
                .key_table(80)
                .iter()
                .map(|hint| format!("{} {}", hint.key, hint.label))
                .collect();
            assert_eq!(keys, ["y yes", "n no", "esc no"]);
        }

        /// The dialog is drawn whole: its title row and its bottom border,
        /// with every expected line in between.
        fn assert_dialog(app: &mut App, title: &str, lines: &[&str]) {
            for (width, height) in SIZES {
                let rows = render_rows(app, width, height);
                let text = rows.join("\n");
                let top = rows
                    .iter()
                    .position(|row| row.contains(&format!("╭─ {title} ")))
                    .unwrap_or_else(|| panic!("no {title:?} at {width}x{height}:\n{text}"));
                let bottom = rows
                    .iter()
                    .skip(top + 1)
                    .position(|row| row.contains('╰'))
                    .map(|offset| top + 1 + offset)
                    .unwrap_or_else(|| panic!("clipped at {width}x{height}:\n{text}"));
                // Above the status line and the key line.
                assert!(bottom < usize::from(height) - 2, "{text}");
                for line in lines {
                    assert!(
                        rows[top..=bottom].iter().any(|row| row.contains(line)),
                        "{line:?} missing at {width}x{height}:\n{text}"
                    );
                }
            }
        }

        #[test]
        fn deleting_one_addon_asks_and_only_y_deletes() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.effective_addon_dir = Some(PathBuf::from("/sandbox/AddOns"));
            app.dashboard.list_state.select(Some(1));
            press_key(&mut app, KeyCode::Char('x'));
            let name = app.dashboard.items[1].name.clone();
            assert!(matches!(
                app.active_question(),
                Some(Question::DeleteAddons(ref targets)) if targets.len() == 1
            ));
            assert_answers(
                &app,
                AppMessage::DashboardConfirmDelete,
                AppMessage::DashboardCancelPendingDelete,
            );
            assert_dialog(
                &mut app,
                &format!("Delete {name}?"),
                &[
                    &name,
                    "Your settings for this addon are not touched.",
                    "You can undo the delete with z.",
                    "y  delete it",
                    "n  keep it",
                ],
            );

            // Enter does nothing; n keeps the addon and closes the question.
            press_key(&mut app, KeyCode::Enter);
            assert!(app.dashboard.pending_delete_folders().is_some());
            press_key(&mut app, KeyCode::Char('n'));
            assert!(app.active_question().is_none());
        }

        #[test]
        fn deleting_many_addons_lists_them_and_never_clips() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let folders: Vec<String> = (0..20).map(|index| format!("Addon{index:02}")).collect();
            app.dashboard.set_pending_delete_folders(Some(folders));
            assert_dialog(
                &mut app,
                "Delete 20 addons?",
                &[
                    "Addon00",
                    "more",
                    "You can undo the delete with z.",
                    "y  delete them",
                    "n  keep them",
                ],
            );
            let rows = render_rows(&mut app, 80, 24).join("\n");
            assert!(!rows.contains("Addon19"), "{rows}");
        }

        #[test]
        fn restoring_a_backup_asks_and_enter_does_not_restore() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDetailMode(DetailMode::Backup));
            app.backup_pane.backups = vec![backup("2026-04-01 12:00:00.000Z")];
            press_key(&mut app, KeyCode::Enter);
            assert!(app.backup_pane.pending_restore.is_some());
            assert_answers(
                &app,
                AppMessage::BackupConfirmRestore,
                AppMessage::BackupCancelRestore,
            );
            assert_dialog(
                &mut app,
                "Restore this backup?",
                &[
                    "Backup from 2026-04-01 12:00:00.000Z",
                    "Close the game first",
                    "y  restore",
                    "n  keep what I have now",
                ],
            );
            press_key(&mut app, KeyCode::Esc);
            assert!(app.backup_pane.pending_restore.is_none());
            // The Backups sheet is still open behind it.
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Backup));
        }

        #[test]
        fn deleting_a_backup_asks_and_says_it_cannot_be_undone() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDetailMode(DetailMode::Backup));
            app.backup_pane.backups = vec![backup("2026-04-01 12:00:00.000Z")];
            press_key(&mut app, KeyCode::Char('x'));
            assert!(app.backup_pane.pending_delete.is_some());
            assert_answers(
                &app,
                AppMessage::BackupConfirmDelete,
                AppMessage::BackupCancelRestore,
            );
            assert_dialog(
                &mut app,
                "Delete this backup?",
                &["This can't be undone.", "y  delete it", "n  keep it"],
            );
            press_key(&mut app, KeyCode::Char('q'));
            assert!(app.backup_pane.pending_delete.is_none());
            assert_eq!(app.shell_ui.overlay.active, Some(OverlayKind::Backup));
        }

        #[test]
        fn replacing_an_addon_asks_with_the_normal_border() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.pending_wago_install_confirmation = Some(pending_wago_confirmation());
            let question = app.active_question().expect("question");
            assert!(!question.danger());
            app.note_question_shown(Instant::now());
            assert_answers(
                &app,
                AppMessage::WagoConfirmInstall,
                AppMessage::WagoCancelInstall,
            );
            assert_dialog(
                &mut app,
                "Replace your copy of WeakAuras?",
                &[
                    "WeakAuras is already installed.",
                    "This folder is replaced:",
                    "Your WeakAuras settings are kept.",
                    "y  replace it",
                    "n  leave it alone",
                ],
            );
        }

        #[test]
        fn the_replace_question_lists_its_folders_and_fits_at_80x24() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            let mut confirmation = pending_wago_confirmation();
            confirmation.inspection.existing_folders = (1..=14)
                .map(|index| format!("WeakAuras_Part{index:02}"))
                .collect();
            app.pending_wago_install_confirmation = Some(confirmation);
            assert_dialog(
                &mut app,
                "Replace your copy of WeakAuras?",
                &[
                    "These folders are replaced:",
                    "WeakAuras_Part01",
                    "more",
                    "y  replace it",
                    "n  leave it alone",
                ],
            );
        }

        #[test]
        fn quitting_during_an_update_asks_first() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDashboardUpdateInProgress(true));
            app.apply(AppAction::SetDashboardJobUi(Some(DashboardJobUiState {
                kind: DashboardJobKind::Update,
                current_index: 3,
                total: 4,
                current_addon: app.dashboard.items[0].folder.clone(),
            })));

            press_key(&mut app, KeyCode::Char('q'));
            assert!(!app.quit_requested);
            assert_eq!(
                app.active_question(),
                Some(Question::QuitWhileUpdating {
                    progress: Some((2, 4))
                })
            );
            assert_answers(&app, AppMessage::QuitConfirmed, AppMessage::QuitCancelled);
            assert_dialog(
                &mut app,
                "Quit while LemonUp is still updating?",
                &["2 of 4 addons are done.", "y  quit", "n  keep going"],
            );

            // n keeps going.
            press_key(&mut app, KeyCode::Char('n'));
            assert!(app.active_question().is_none());
            assert!(!app.quit_requested);

            // Ctrl+C asks too, and a second Ctrl+C quits.
            let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
            for message in app.messages_for_key(ctrl_c) {
                apply_all(&mut app, message);
            }
            assert!(app.active_question().is_some());
            assert!(!app.quit_requested);
            for message in app.messages_for_key(ctrl_c) {
                apply_all(&mut app, message);
            }
            assert!(app.quit_requested);
        }

        #[test]
        fn y_quits_and_a_finished_update_closes_the_question() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            app.apply(AppAction::SetDashboardUpdateInProgress(true));
            press_key(&mut app, KeyCode::Char('q'));
            assert_eq!(
                app.active_question(),
                Some(Question::QuitWhileUpdating { progress: None })
            );
            // The update ends: the question no longer holds.
            app.apply(AppAction::SetDashboardUpdateInProgress(false));
            assert!(app.active_question().is_none());

            app.apply(AppAction::SetDashboardUpdateInProgress(true));
            press_key(&mut app, KeyCode::Char('q'));
            press_key(&mut app, KeyCode::Char('y'));
            assert!(app.quit_requested);
        }

        #[test]
        fn quitting_with_nothing_running_does_not_ask() {
            let mut app = app_for_tests(ShellMode::Dashboard);
            press_key(&mut app, KeyCode::Char('q'));
            assert!(app.quit_requested);
            let mut app = app_for_tests(ShellMode::Dashboard);
            let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
            for message in app.messages_for_key(ctrl_c) {
                apply_all(&mut app, message);
            }
            assert!(app.quit_requested);
        }

        #[test]
        fn delete_and_restore_are_red_replace_and_quit_are_not() {
            let label = "today".to_string();
            assert!(Question::DeleteAddons(Vec::new()).danger());
            assert!(
                Question::RestoreBackup {
                    label: label.clone()
                }
                .danger()
            );
            assert!(Question::DeleteBackup { label }.danger());
            assert!(
                !Question::ReplaceAddon {
                    name: "Pawn".to_string(),
                    source: "Wago.io",
                    direct: false,
                    folders: Vec::new(),
                }
                .danger()
            );
            assert!(!Question::QuitWhileUpdating { progress: None }.danger());
        }
    }
}
