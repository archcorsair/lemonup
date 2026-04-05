use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration as StdDuration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};

use lemonup_core::{
    AddonKind, AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE,
    DefaultScreen, GameFlavor, ScanSummary, SourceKind, StateDatabase, ThemeMode, UpdateStatus,
    detect_known_addons_path, paths_match, scan_addons_dir, search_for_wow, validate_addons_path,
};
use tokio::sync::mpsc;

use crate::action::AppAction;
use crate::backup::{BackupEntry, BackupRunOutcome, backup_root, create_wtf_backup, list_backups};
use crate::drift::{DriftReport, compute_drift_report};
use crate::event::{EventHandler, TerminalEvent};
use crate::onboarding::{
    FoundAction, OnboardingPhase, OnboardingSettingsField, OnboardingState, OnboardingStep,
    OnboardingTaskEvent,
};
use crate::shimmer::{ShimmerConfig, shimmer_text_spans};
use crate::tui::Backend;
use crate::update::{
    CheckResult, LiveUpdateStatus, LiveUpdateSummary, apply_live_updates, build_update_checks,
    refresh_live_update_checks,
};
use crate::wago::{
    WagoInstallInspection, WagoInstallSummary, WagoSearchResult, WagoStability,
    inspect_wago_install_target, install_wago_addon_with_replace, parse_wago_target,
    resolve_wago_api_key, search_wago_addons,
};
use time::{Duration, OffsetDateTime};

mod render_dashboard;
mod render_inspect;
mod render_install_search;
mod render_shared;

#[cfg(test)]
use self::render_dashboard::{
    child_row_detail_prefix, child_row_prefix, dashboard_item_version_line,
};
use self::render_dashboard::{
    dashboard_item_author_sort_key, dashboard_item_management_summary,
    dashboard_item_sort_version_key, dashboard_item_source_label, dashboard_item_status_text,
    dashboard_item_version_label,
};
use self::render_shared::{
    format_download_count, summarize_owned_folders, truncate_middle_text, truncate_text,
    visible_search_result_window,
};

const LOGO_FULL: [&str; 2] = [
    "█   █▀▀ █▀▄▀█ █▀█ █▄ █ █ █ █▀█",
    "█▄▄ ██▄ █ ▀ █ █▄█ █ ▀█ █▄█ █▀▀",
];
const LOGO_COMPACT: &str = "LEMONUP";
const MOTION_SPINNER_FRAMES: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];
const IDLE_TICK_RATE: StdDuration = StdDuration::from_millis(250);
const ANIMATED_TICK_RATE: StdDuration = StdDuration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellLayoutMode {
    Standard,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderVariant {
    FullLogo,
    CompactLogo,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogoStyle {
    FruitGradient,
    StripedLegacy,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    Inspect,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum InspectSection {
    IncludedAddons,
    Dependencies,
    AddonInfo,
}

impl InspectSection {
    fn key(self) -> &'static str {
        match self {
            Self::IncludedAddons => "r",
            Self::Dependencies => "d",
            Self::AddonInfo => "t",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::IncludedAddons => "Included addons",
            Self::Dependencies => "Dependencies",
            Self::AddonInfo => "Addon info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct InspectOverlayState {
    open_sections: HashSet<InspectSection>,
    scroll_offset: u16,
}

impl InspectOverlayState {
    fn toggle(mut self, section: InspectSection) -> Self {
        if !self.open_sections.insert(section) {
            self.open_sections.remove(&section);
        }
        self
    }

    fn is_open(&self, section: InspectSection) -> bool {
        self.open_sections.contains(&section)
    }

    fn scroll_up(mut self) -> Self {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
        self
    }

    fn scroll_down(mut self) -> Self {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InspectResolvedTarget<'a> {
    item: &'a DashboardItem,
    opened_from_child: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InspectActionChip {
    key: &'static str,
    label: &'static str,
    enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchActionChip {
    key: &'static str,
    label: &'static str,
    enabled: bool,
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
    dashboard_toast: Option<DashboardToast>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FooterKeyPulse {
    hint: FooterHintId,
    expires_at_tick: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardToast {
    text: String,
    kind: DashboardEventKind,
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterHintTier {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterHintId {
    Nav,
    Inspect,
    Select,
    Clear,
    Sort,
    Tree,
    Check,
    Update,
    Delete,
    Undo,
    Install,
    Search,
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
    DeepScan,
    Validate,
    Fields,
    Results,
    Change,
    Save,
    Reset,
    Relations,
    Dependencies,
    Technical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FooterCommandHint {
    id: FooterHintId,
    key: &'static str,
    label: &'static str,
    tier: FooterHintTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShellFrame {
    header: Rect,
    body: Rect,
    footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiTheme {
    brand_hot: Color,
    brand_warm: Color,
    brand_gold: Color,
    warning: Color,
    error: Color,
    success: Color,
    info: Color,
    highlight: Color,
    muted: Color,
    border: Color,
    panel_title: Color,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            brand_hot: Color::Rgb(255, 95, 95),
            brand_warm: Color::Rgb(255, 158, 100),
            brand_gold: Color::Rgb(224, 175, 104),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            success: Color::Rgb(158, 206, 106),
            info: Color::Rgb(125, 207, 255),
            highlight: Color::Rgb(255, 158, 100),
            muted: Color::Rgb(115, 122, 162),
            border: Color::Rgb(84, 92, 126),
            panel_title: Color::Rgb(122, 162, 247),
        }
    }
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
    Update,
    Config,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRuntime {
    pub profile_name: String,
    pub addon_dir_override: Option<PathBuf>,
    pub guarded_addon_dir: Option<PathBuf>,
}

impl AppRuntime {
    pub fn new(
        profile_name: String,
        addon_dir_override: Option<PathBuf>,
        guarded_addon_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            profile_name,
            addon_dir_override,
            guarded_addon_dir,
        }
    }

    pub fn is_default_profile(&self) -> bool {
        self.profile_name == DEFAULT_PROFILE
    }

    pub fn is_guarded_path(&self, candidate: &Path) -> bool {
        if self.is_default_profile() {
            return false;
        }

        self.guarded_addon_dir
            .as_ref()
            .is_some_and(|guarded| paths_match(candidate, guarded))
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum AppMessage {
    Tick,
    QuitRequested,
    TerminalResized { width: u16, height: u16 },
    DashboardOpenInspect,
    DashboardCloseOverlay,
    InspectScrollUp,
    InspectScrollDown,
    InspectToggleSection(InspectSection),
    InspectToggleSelected,
    InspectRequestDelete,
    InspectRunCheck,
    InspectRunUpdate,
    DashboardSelectionNext,
    DashboardSelectionPrevious,
    DashboardPointerSelect { column: u16, row: u16 },
    DashboardRequestDelete,
    DashboardConfirmDelete,
    DashboardCancelPendingDelete,
    DashboardUndoDelete,
    DashboardRunCheckSelected,
    DashboardRunUpdateSelected,
    DashboardSelectRefreshableUpdates,
    DashboardToggleSort(DashboardSortColumn),
    DashboardToggleSelected,
    DashboardSelectAll,
    DashboardClearSelection,
    DashboardToggleExpanded,
    DashboardExpandAllRelationships,
    DashboardCollapseAllRelationships,
    DashboardCollapseExpanded,
    SetDetailMode(DetailMode),
    OpenSearch(SearchPresentationMode),
    InstallBeginEditing,
    InstallStopEditing,
    InstallInputChar(char),
    InstallBackspace,
    InstallSubmit,
    SearchBeginEditing,
    SearchStopEditing,
    SearchInputChar(char),
    SearchBackspace,
    SearchSubmit,
    SearchResultNext,
    SearchResultPrevious,
    SearchInstallSelected,
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
    ConfigSave,
    ConfigResetDraft,
    BackupRunNow,
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
    WagoSearchFinished(std::result::Result<WagoSearchOutcome, String>),
    WagoInstallFinished(std::result::Result<WagoInstallTaskOutcome, String>),
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
    updated_addon_name: Option<String>,
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
    BackupWtf,
    BackupRetention,
    Theme,
    ShowLibs,
    DefaultScreen,
    RunOnboardingAgain,
}

impl ConfigField {
    const ALL: [Self; 7] = [
        Self::WagoApiKey,
        Self::BackupWtf,
        Self::BackupRetention,
        Self::Theme,
        Self::ShowLibs,
        Self::DefaultScreen,
        Self::RunOnboardingAgain,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::WagoApiKey => "Wago API key",
            Self::BackupWtf => "Backup WTF",
            Self::BackupRetention => "Backup retention",
            Self::Theme => "Theme",
            Self::ShowLibs => "Show libs",
            Self::DefaultScreen => "Default screen",
            Self::RunOnboardingAgain => "Run onboarding again",
        }
    }

    fn is_textual(self) -> bool {
        matches!(self, Self::WagoApiKey | Self::BackupRetention)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigEditState {
    field: ConfigField,
    value: String,
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
            ConfigField::BackupRetention => self.draft.backup_retention.to_string(),
            _ => return self.clone(),
        };
        let mut next = self.clone();
        next.edit = Some(ConfigEditState { field, value });
        next
    }

    fn cancel_edit(&self) -> Self {
        let mut next = self.clone();
        next.edit = None;
        next
    }

    fn insert_char(&self, character: char) -> Self {
        let mut next = self.clone();
        if let Some(edit) = next.edit.as_mut() {
            edit.value.push(character);
        }
        next
    }

    fn backspace(&self) -> Self {
        let mut next = self.clone();
        if let Some(edit) = next.edit.as_mut() {
            edit.value.pop();
        }
        next
    }

    fn commit_edit(&self) -> Result<Self, String> {
        let Some(edit) = &self.edit else {
            return Ok(self.clone());
        };

        let mut next = self.clone();
        match edit.field {
            ConfigField::WagoApiKey => {
                let trimmed = edit.value.trim().to_string();
                next.draft.wago_api_key = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
            }
            ConfigField::BackupRetention => {
                let parsed = edit
                    .value
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

    fn toggle_selected(&self) -> Self {
        let mut next = self.clone();
        match next.selected_field() {
            ConfigField::BackupWtf => next.draft.backup_wtf = !next.draft.backup_wtf,
            ConfigField::Theme => {
                next.draft.theme = match next.draft.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            ConfigField::ShowLibs => next.draft.show_libs = !next.draft.show_libs,
            ConfigField::DefaultScreen => {
                next.draft.default_screen = match next.draft.default_screen {
                    DefaultScreen::Manage => DefaultScreen::Install,
                    DefaultScreen::Install => DefaultScreen::Config,
                    DefaultScreen::Config => DefaultScreen::WagoSearch,
                    DefaultScreen::WagoSearch => DefaultScreen::Manage,
                };
            }
            ConfigField::RunOnboardingAgain => {}
            _ => {}
        }
        next
    }

    fn reset_draft(&self) -> Self {
        Self {
            persisted: self.persisted.clone(),
            draft: self.persisted.clone(),
            selected_field: self.selected_field,
            edit: None,
        }
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
    skipped_unmanaged: usize,
    skipped_unsupported: usize,
    scanned_addons: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardRefreshabilitySummary {
    total: usize,
    refreshable: usize,
    manual: usize,
    unmanaged: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardEventKind {
    Info,
    Success,
    Error,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AddonTableRowViewModel {
    name: Line<'static>,
    version: Line<'static>,
    author: String,
    source: Line<'static>,
    row_style: Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardSortColumn {
    Name,
    Version,
    Author,
    Source,
}

impl DashboardSortColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Version => "Version",
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
            column: DashboardSortColumn::Name,
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
    last_update_summary: Option<DashboardUpdateRunSummary>,
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
            last_update_summary: None,
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
            DashboardRowKind::OwnedChild { parent_folder } => {
                if self.expanded_folders.remove(&parent_folder) {
                    self.rebuild_rows();
                    self.restore_selection(&DashboardRowKey::Parent(parent_folder));
                    true
                } else {
                    false
                }
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
            DashboardRowKind::OwnedChild { parent_folder } => {
                if self.expanded_folders.remove(&parent_folder) {
                    self.rebuild_rows();
                    self.restore_selection(&DashboardRowKey::Parent(parent_folder));
                    true
                } else {
                    false
                }
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

    fn select_all_parents(&mut self) -> bool {
        let before = self.selected_parents.len();
        self.selected_parents = self
            .items
            .iter()
            .map(|item| item.folder.clone())
            .collect::<HashSet<_>>();
        self.selected_parents.len() != before
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

    fn refreshable_parent_folders(&self) -> Vec<String> {
        let mut folders = self
            .items
            .iter()
            .filter(|item| is_refreshable_dashboard_item(item))
            .map(|item| item.folder.clone())
            .collect::<Vec<_>>();
        folders.sort();
        folders
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
            Some(DashboardRow {
                kind: DashboardRowKind::OwnedChild { .. },
                ..
            }) => Err("focus a parent row or select one or more parent addons first"),
            None => Err("focus a parent row or select one or more parent addons first"),
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
        self.job_ui = job_ui;
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
                DashboardSortColumn::Version => compare_text(
                    &dashboard_item_sort_version_key(left),
                    &dashboard_item_sort_version_key(right),
                )
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

        for item in &self.items {
            let expanded = self.expanded_folders.contains(&item.folder);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPaneState {
    input: String,
    is_editing: bool,
}

impl Default for InstallPaneState {
    fn default() -> Self {
        Self {
            input: String::new(),
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

    fn insert_char(&self, character: char) -> Self {
        let mut next = self.clone();
        next.input.push(character);
        next
    }

    fn backspace(&self) -> Self {
        let mut next = self.clone();
        next.input.pop();
        next
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchPresentationMode {
    ComposeFirst,
    TwoState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPaneState {
    query: String,
    presentation_mode: SearchPresentationMode,
    is_editing: bool,
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
    },
    Error {
        addon_id: Option<String>,
        addon_name: Option<String>,
        message: String,
    },
}

impl Default for SearchPaneState {
    fn default() -> Self {
        Self {
            query: String::new(),
            presentation_mode: SearchPresentationMode::ComposeFirst,
            is_editing: true,
            in_progress: false,
            results: Vec::new(),
            selected_result: None,
            last_query: None,
            install_state: SearchInstallState::Idle,
        }
    }
}

impl SearchPaneState {
    fn open_with_mode(&self, presentation_mode: SearchPresentationMode) -> Self {
        let mut next = self.clone();
        next.presentation_mode = presentation_mode;
        next.is_editing = true;
        next
    }

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

    fn insert_char(&self, character: char) -> Self {
        let mut next = self.clone();
        next.query.push(character);
        next
    }

    fn backspace(&self) -> Self {
        let mut next = self.clone();
        next.query.pop();
        next
    }

    fn start_search(&self) -> Self {
        let mut next = self.clone();
        next.is_editing = false;
        next.in_progress = true;
        next.last_query = Some(next.query.trim().to_string());
        next.install_state = SearchInstallState::Idle;
        next
    }

    fn finish_search(&self, query: String, mut results: Vec<WagoSearchResult>) -> Self {
        let mut next = self.clone();
        next.in_progress = false;
        next.is_editing = false;
        next.last_query = Some(query);
        sort_search_results(&mut results);
        next.results = results;
        next.install_state = SearchInstallState::Idle;
        next.selected_result = if next.results.is_empty() {
            None
        } else {
            Some(0)
        };
        next
    }

    fn fail_search(&self) -> Self {
        let mut next = self.clone();
        next.in_progress = false;
        next.install_state = SearchInstallState::Idle;
        next
    }

    fn select_next(&self) -> Self {
        if self.results.is_empty() {
            return self.clone();
        }

        let mut next = self.clone();
        next.selected_result = Some(match next.selected_result {
            Some(index) => (index + 1) % next.results.len(),
            None => 0,
        });
        next
    }

    fn select_previous(&self) -> Self {
        if self.results.is_empty() {
            return self.clone();
        }

        let mut next = self.clone();
        next.selected_result = Some(match next.selected_result {
            Some(0) | None => next.results.len() - 1,
            Some(index) => index - 1,
        });
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
        next.is_editing = false;
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

    fn finish_install(&self, summary: &WagoInstallSummary) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Success {
            addon_id: summary.addon_id.clone(),
            addon_name: summary.addon_name.clone(),
            parent_folder: summary.parent_folder.clone(),
        };
        next
    }

    fn fail_install(
        &self,
        addon_id: Option<String>,
        addon_name: Option<String>,
        message: String,
    ) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Error {
            addon_id,
            addon_name,
            message,
        };
        next
    }

    fn clear_install_state(&self) -> Self {
        let mut next = self.clone();
        next.install_state = SearchInstallState::Idle;
        next
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
    status_line: String,
    ui_theme: UiTheme,
    shell_ui: ShellUiState,
    config_present: bool,
    config_pane: ConfigPaneState,
    backup_pane: BackupPaneState,
    wago_api_key: Option<String>,
    config_store: ConfigStore,
    state_db_file: PathBuf,
    trash_dir: PathBuf,
    backup_dir: PathBuf,
    runtime: AppRuntime,
    effective_addon_dir: Option<PathBuf>,
    scan_state: ScanState,
    dashboard: DashboardState,
    inspect_overlay: InspectOverlayState,
    install_pane: InstallPaneState,
    search_pane: SearchPaneState,
    pending_wago_install_confirmation: Option<WagoInstallConfirmation>,
    wago_install_in_progress: bool,
    undo_delete: Option<DashboardUndoDeleteState>,
    undo_delete_in_progress: bool,
    last_dashboard_list_area: Option<Rect>,
    onboarding: OnboardingState,
    task_events_tx: mpsc::UnboundedSender<AppTaskEvent>,
    task_events_rx: mpsc::UnboundedReceiver<AppTaskEvent>,
    active_onboarding_cancel: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn bootstrap(paths: AppPaths, runtime: AppRuntime) -> lemonup_core::Result<Self> {
        let config_store = ConfigStore::new(paths.config_file);
        let config_state = config_store.load()?;
        let state_db_file = paths.state_db_file.clone();
        let trash_dir = paths.data_dir.join("trash");
        let backup_dir = backup_root(&paths.data_dir);
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
                        "invalid --addon-dir for profile '{}': {} | reason: this matches the default profile AddOns directory | hint: omit --addon-dir to use the Location Finder, or pass a valid sandbox _retail_\\Interface\\AddOns path",
                        runtime.profile_name,
                        path.display()
                    )));
                }

                validate_addons_path(path).map_err(|error| {
                    lemonup_core::LemonupError::InvalidArgument(format!(
                        "invalid --addon-dir for profile '{}': {} | reason: {} | hint: omit --addon-dir to use the Location Finder, or pass a valid _retail_\\Interface\\AddOns path",
                        runtime.profile_name,
                        path.display(),
                        error
                    ))
                })?;

                Some(path.clone())
            }
            None => None,
        };
        let effective_addon_dir = validated_override.or(configured_addon_dir.clone());

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
            status_line: String::new(),
            ui_theme: UiTheme::default(),
            shell_ui: ShellUiState::default(),
            config_present,
            config_pane: ConfigPaneState::new(loaded_config),
            backup_pane: BackupPaneState {
                backups: list_backups(&backup_dir).unwrap_or_default(),
                in_progress: false,
            },
            wago_api_key,
            config_store,
            state_db_file,
            trash_dir,
            backup_dir,
            runtime,
            effective_addon_dir,
            scan_state,
            dashboard: DashboardState::from_addons(addons),
            inspect_overlay: InspectOverlayState::default(),
            install_pane: InstallPaneState::default(),
            search_pane: SearchPaneState::default(),
            pending_wago_install_confirmation: None,
            wago_install_in_progress: false,
            undo_delete: None,
            undo_delete_in_progress: false,
            last_dashboard_list_area: None,
            onboarding,
            task_events_tx,
            task_events_rx,
            active_onboarding_cancel: None,
        };
        app.status_line = app.initial_status_line();
        Ok(app)
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

            let messages = self.messages_for_event(event);
            if let Some(pulse) = self.footer_pulse_for_event(event, &messages) {
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

    fn initial_status_line(&self) -> String {
        match self.shell_mode {
            ShellMode::Dashboard => {
                if self.runtime.addon_dir_override.is_some() {
                    self.dashboard_status_for(DetailMode::Overview, "using addon-dir override")
                } else if matches!(self.scan_state, ScanState::Pending) {
                    self.dashboard_status_for(
                        DetailMode::Overview,
                        "dashboard ready | initial scan pending",
                    )
                } else {
                    self.dashboard_status_for(DetailMode::Overview, "dashboard ready")
                }
            }
            ShellMode::Onboarding => match &self.onboarding.phase {
                OnboardingPhase::Error(message) => {
                    self.with_base_status(&format!("location finder blocked: {message}"))
                }
                OnboardingPhase::Ready => self.location_finder_status(),
                _ => self.with_base_status("location finder"),
            },
        }
    }

    fn dashboard_selection_for_pointer(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.last_dashboard_list_area?;
        if self.dashboard.rows.is_empty() || area.width < 3 || area.height < 3 {
            return None;
        }

        let inner_left = area.x.saturating_add(1);
        let inner_top = area.y.saturating_add(1);
        let inner_right = area.x.saturating_add(area.width.saturating_sub(2));
        let inner_bottom = area.y.saturating_add(area.height.saturating_sub(2));

        if column < inner_left || column > inner_right || row < inner_top || row > inner_bottom {
            return None;
        }

        let table_body_top = inner_top.saturating_add(1);
        if row < table_body_top || row > inner_bottom {
            return None;
        }

        let row_in_view = usize::from(row.saturating_sub(table_body_top));
        let absolute = self
            .dashboard
            .list_state
            .offset()
            .saturating_add(row_in_view);
        (absolute < self.dashboard.rows.len()).then_some(absolute)
    }

    fn process_background_events(&mut self) {
        while let Ok(event) = self.task_events_rx.try_recv() {
            let actions = self.update(AppMessage::BackgroundTask(event));
            for action in actions {
                self.apply(action);
            }
        }
    }

    fn messages_for_event(&self, event: TerminalEvent) -> Vec<AppMessage> {
        match event {
            TerminalEvent::Tick => vec![AppMessage::Tick],
            TerminalEvent::Resize(width, height) => {
                vec![AppMessage::TerminalResized { width, height }]
            }
            TerminalEvent::Key(key) => self.messages_for_key(key),
            TerminalEvent::Mouse(mouse) => self.messages_for_mouse(mouse),
        }
    }

    fn footer_pulse_for_event(
        &self,
        event: TerminalEvent,
        messages: &[AppMessage],
    ) -> Option<FooterKeyPulse> {
        if messages.is_empty() {
            return None;
        }

        let TerminalEvent::Key(key) = event else {
            return None;
        };

        self.footer_hint_for_key(key).map(|hint| FooterKeyPulse {
            hint,
            expires_at_tick: self.shell_ui.motion.tick_count + 2,
        })
    }

    fn footer_hint_for_key(&self, key: KeyEvent) -> Option<FooterHintId> {
        if self.shell_mode == ShellMode::Onboarding {
            if self.onboarding.is_editing {
                return match key.code {
                    KeyCode::Enter => Some(FooterHintId::Save),
                    KeyCode::Esc => Some(FooterHintId::Cancel),
                    KeyCode::Backspace => Some(FooterHintId::Back),
                    _ => None,
                };
            }

            return match self.onboarding.step {
                OnboardingStep::Theme => match key.code {
                    KeyCode::Left | KeyCode::Right => Some(FooterHintId::ThemeToggle),
                    KeyCode::Enter => Some(FooterHintId::Next),
                    KeyCode::Esc => Some(FooterHintId::Cancel),
                    _ => None,
                },
                OnboardingStep::Directory => match self.onboarding.phase {
                    OnboardingPhase::Found(_) => match key.code {
                        KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                            Some(FooterHintId::Nav)
                        }
                        KeyCode::Enter => Some(FooterHintId::Next),
                        KeyCode::Esc => Some(FooterHintId::Back),
                        _ => None,
                    },
                    OnboardingPhase::DeepScanning(_) => match key.code {
                        KeyCode::Esc => Some(FooterHintId::Cancel),
                        _ => None,
                    },
                    _ => match key.code {
                        KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                            Some(FooterHintId::Nav)
                        }
                        KeyCode::Enter => Some(FooterHintId::Validate),
                        KeyCode::Char('d') => Some(FooterHintId::DeepScan),
                        KeyCode::Char('e') => Some(FooterHintId::Edit),
                        KeyCode::Esc => Some(FooterHintId::Back),
                        _ => None,
                    },
                },
                OnboardingStep::Wago => match key.code {
                    KeyCode::Char('e') => Some(FooterHintId::Edit),
                    KeyCode::Enter => Some(FooterHintId::Next),
                    KeyCode::Esc => Some(FooterHintId::Back),
                    _ => None,
                },
                OnboardingStep::Settings => match key.code {
                    KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                        Some(FooterHintId::SettingsNav)
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                        Some(FooterHintId::Change)
                    }
                    KeyCode::Enter => Some(FooterHintId::Next),
                    KeyCode::Esc => Some(FooterHintId::Back),
                    _ => None,
                },
                OnboardingStep::Review => match key.code {
                    KeyCode::Enter => Some(FooterHintId::Next),
                    KeyCode::Esc => Some(FooterHintId::Back),
                    _ => None,
                },
            };
        }

        if self.dashboard.pending_delete_folders().is_some()
            || self.pending_wago_install_confirmation.is_some()
        {
            return match key.code {
                KeyCode::Char('y') => Some(FooterHintId::Confirm),
                KeyCode::Char('n') | KeyCode::Esc => Some(FooterHintId::Cancel),
                _ => None,
            };
        }

        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            return match key.code {
                KeyCode::Char('c') => Some(FooterHintId::Check),
                KeyCode::Char('u') => Some(FooterHintId::Update),
                KeyCode::Char('x') => Some(FooterHintId::Delete),
                KeyCode::Char(' ') => Some(FooterHintId::Select),
                KeyCode::Char('r') => Some(FooterHintId::Relations),
                KeyCode::Char('d') => Some(FooterHintId::Dependencies),
                KeyCode::Char('t') => Some(FooterHintId::Technical),
                _ => None,
            };
        }

        if self.shell_ui.overlay.active.is_some() {
            return match self.dashboard.detail_mode {
                DetailMode::Install => {
                    if self.install_pane.is_editing {
                        match key.code {
                            KeyCode::Enter => Some(FooterHintId::Save),
                            KeyCode::Esc => Some(FooterHintId::Cancel),
                            _ => None,
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('e') => Some(FooterHintId::Edit),
                            KeyCode::Enter => Some(FooterHintId::Run),
                            KeyCode::Esc => Some(FooterHintId::Close),
                            _ => None,
                        }
                    }
                }
                DetailMode::Search => {
                    if self.search_pane.is_editing {
                        match key.code {
                            KeyCode::Enter => Some(FooterHintId::Save),
                            KeyCode::Esc => Some(FooterHintId::Cancel),
                            KeyCode::Backspace => Some(FooterHintId::Back),
                            _ => None,
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('e') | KeyCode::Char('/') => Some(FooterHintId::Edit),
                            KeyCode::Down
                            | KeyCode::Up
                            | KeyCode::Char('j')
                            | KeyCode::Char('k') => Some(FooterHintId::Results),
                            KeyCode::Enter => Some(FooterHintId::Install),
                            KeyCode::Esc => Some(FooterHintId::Close),
                            _ => None,
                        }
                    }
                }
                DetailMode::Update => match key.code {
                    KeyCode::Char('u') | KeyCode::Char('r') => Some(FooterHintId::Update),
                    KeyCode::Char('c') | KeyCode::Char('v') => Some(FooterHintId::Check),
                    KeyCode::Esc => Some(FooterHintId::Close),
                    _ => None,
                },
                DetailMode::Config => {
                    if self.config_pane.edit.is_some() {
                        match key.code {
                            KeyCode::Enter => Some(FooterHintId::Save),
                            KeyCode::Esc => Some(FooterHintId::Cancel),
                            KeyCode::Backspace => Some(FooterHintId::Back),
                            _ => None,
                        }
                    } else {
                        match key.code {
                            KeyCode::Down
                            | KeyCode::Up
                            | KeyCode::Char('j')
                            | KeyCode::Char('k') => Some(FooterHintId::Fields),
                            KeyCode::Enter => Some(FooterHintId::Toggle),
                            KeyCode::Char('e') => Some(FooterHintId::Edit),
                            KeyCode::Char('s') => Some(FooterHintId::Save),
                            KeyCode::Char('n') => Some(FooterHintId::Reset),
                            KeyCode::Esc => Some(FooterHintId::Close),
                            _ => None,
                        }
                    }
                }
                DetailMode::Backup => match key.code {
                    KeyCode::Char('r') | KeyCode::Enter => Some(FooterHintId::Run),
                    KeyCode::Esc => Some(FooterHintId::Close),
                    _ => None,
                },
                DetailMode::Overview => match key.code {
                    KeyCode::Esc => Some(FooterHintId::Close),
                    _ => None,
                },
            };
        }

        match key.code {
            KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                Some(FooterHintId::Nav)
            }
            KeyCode::Enter => Some(FooterHintId::Inspect),
            KeyCode::Char(' ') | KeyCode::Char('a') => Some(FooterHintId::Select),
            KeyCode::Esc => Some(FooterHintId::Clear),
            KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3') | KeyCode::Char('4') => {
                Some(FooterHintId::Sort)
            }
            KeyCode::Right
            | KeyCode::Left
            | KeyCode::Char('l')
            | KeyCode::Char('h')
            | KeyCode::Char(']')
            | KeyCode::Char('[') => Some(FooterHintId::Tree),
            KeyCode::Char('c') => Some(FooterHintId::Check),
            KeyCode::Char('u') => Some(FooterHintId::Update),
            KeyCode::Char('x') => Some(FooterHintId::Delete),
            KeyCode::Char('z') => Some(FooterHintId::Undo),
            KeyCode::Char('i') => Some(FooterHintId::Install),
            KeyCode::Char('s') | KeyCode::Char('/') => Some(FooterHintId::Search),
            KeyCode::Char(',') => Some(FooterHintId::Config),
            KeyCode::Char('b') => Some(FooterHintId::Backup),
            _ => None,
        }
    }

    fn messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        if key.kind != KeyEventKind::Press {
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

        if self.shell_ui.overlay.active.is_some() {
            return vec![];
        }

        if self.dashboard.pending_delete_folders().is_some() {
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
        if self.dashboard.pending_delete_folders().is_some() {
            return match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Char('y') => vec![AppMessage::DashboardConfirmDelete],
                KeyCode::Char('n') | KeyCode::Esc => vec![AppMessage::DashboardCancelPendingDelete],
                _ => vec![],
            };
        }

        if self.pending_wago_install_confirmation.is_some() {
            return match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Char('y') => vec![AppMessage::WagoConfirmInstall],
                KeyCode::Char('n') | KeyCode::Esc => vec![AppMessage::WagoCancelInstall],
                _ => vec![],
            };
        }

        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            return match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::InspectScrollDown],
                KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::InspectScrollUp],
                KeyCode::Char('c') => vec![AppMessage::InspectRunCheck],
                KeyCode::Char('u') => vec![AppMessage::InspectRunUpdate],
                KeyCode::Char('x') => vec![AppMessage::InspectRequestDelete],
                KeyCode::Char(' ') => vec![AppMessage::InspectToggleSelected],
                KeyCode::Char('r') => {
                    vec![AppMessage::InspectToggleSection(
                        InspectSection::IncludedAddons,
                    )]
                }
                KeyCode::Char('d') => {
                    vec![AppMessage::InspectToggleSection(
                        InspectSection::Dependencies,
                    )]
                }
                KeyCode::Char('t') => {
                    vec![AppMessage::InspectToggleSection(InspectSection::AddonInfo)]
                }
                KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
                _ => vec![],
            };
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

        if self.shell_ui.overlay.active.is_some() {
            return match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Char('r') if self.dashboard.detail_mode == DetailMode::Update => {
                    vec![AppMessage::DashboardRunUpdateSelected]
                }
                KeyCode::Char('v') if self.dashboard.detail_mode == DetailMode::Update => {
                    vec![AppMessage::DashboardSelectRefreshableUpdates]
                }
                KeyCode::Esc => vec![AppMessage::DashboardCloseOverlay],
                _ => vec![],
            };
        }

        match key.code {
            KeyCode::Char('q') => vec![AppMessage::QuitRequested],
            KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::DashboardSelectionNext],
            KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::DashboardSelectionPrevious],
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
            KeyCode::Esc => vec![AppMessage::DashboardClearSelection],
            KeyCode::Enter if self.dashboard.detail_mode == DetailMode::Overview => {
                vec![AppMessage::DashboardOpenInspect]
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
                vec![AppMessage::DashboardToggleExpanded]
            }
            KeyCode::Left | KeyCode::Char('h') => vec![AppMessage::DashboardCollapseExpanded],
            KeyCode::Char(']') => vec![AppMessage::DashboardExpandAllRelationships],
            KeyCode::Char('[') => vec![AppMessage::DashboardCollapseAllRelationships],
            KeyCode::Char('o') => vec![AppMessage::SetDetailMode(DetailMode::Overview)],
            KeyCode::Char('i') | KeyCode::Char('/') => {
                vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
            }
            KeyCode::Char('1') => vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Name)],
            KeyCode::Char('2') => {
                vec![AppMessage::DashboardToggleSort(
                    DashboardSortColumn::Version,
                )]
            }
            KeyCode::Char('3') => {
                vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Author)]
            }
            KeyCode::Char('4') => {
                vec![AppMessage::DashboardToggleSort(DashboardSortColumn::Source)]
            }
            KeyCode::Char('r') if self.dashboard.detail_mode == DetailMode::Update => {
                vec![AppMessage::DashboardRunUpdateSelected]
            }
            KeyCode::Char('v') if self.dashboard.detail_mode == DetailMode::Update => {
                vec![AppMessage::DashboardSelectRefreshableUpdates]
            }
            KeyCode::Char('u') => vec![AppMessage::DashboardRunUpdateSelected],
            KeyCode::Char('c') => vec![AppMessage::DashboardRunCheckSelected],
            KeyCode::Char(',') => vec![AppMessage::SetDetailMode(DetailMode::Config)],
            KeyCode::Char('b') => vec![AppMessage::SetDetailMode(DetailMode::Backup)],
            _ => vec![],
        }
    }

    fn search_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Search {
            return None;
        }

        if self.search_pane.is_editing {
            return Some(match key.code {
                KeyCode::Enter => vec![AppMessage::SearchSubmit],
                KeyCode::Esc => vec![AppMessage::SearchStopEditing],
                KeyCode::Backspace => vec![AppMessage::SearchBackspace],
                KeyCode::Char(character) => vec![AppMessage::SearchInputChar(character)],
                _ => vec![],
            });
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') if !self.search_pane.results.is_empty() => {
                Some(vec![AppMessage::SearchResultNext])
            }
            KeyCode::Up | KeyCode::Char('k') if !self.search_pane.results.is_empty() => {
                Some(vec![AppMessage::SearchResultPrevious])
            }
            KeyCode::Enter if self.search_pane.results.is_empty() => {
                Some(vec![AppMessage::SearchSubmit])
            }
            KeyCode::Enter => Some(vec![AppMessage::SearchInstallSelected]),
            KeyCode::Char('/') => Some(vec![AppMessage::SearchBeginEditing]),
            _ => None,
        }
    }

    fn install_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Install {
            return None;
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
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Char(character) => vec![AppMessage::ConfigInputChar(character)],
                _ => vec![],
            });
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(vec![AppMessage::ConfigSelectionNext]),
            KeyCode::Up | KeyCode::Char('k') => Some(vec![AppMessage::ConfigSelectionPrevious]),
            KeyCode::Char('e') => Some(vec![AppMessage::ConfigBeginEditing]),
            KeyCode::Enter => Some(vec![AppMessage::ConfigToggleSelected]),
            KeyCode::Char('s') => Some(vec![AppMessage::ConfigSave]),
            KeyCode::Char('n') => Some(vec![AppMessage::ConfigResetDraft]),
            _ => None,
        }
    }

    fn backup_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Backup {
            return None;
        }

        match key.code {
            KeyCode::Char('r') | KeyCode::Enter => Some(vec![AppMessage::BackupRunNow]),
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
                        KeyCode::Enter => vec![AppMessage::OnboardingValidateInput],
                        KeyCode::Char('d') => vec![AppMessage::OnboardingDeepScan],
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
                        AppAction::SetStatus(
                            self.with_base_status("checking common install locations"),
                        ),
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
                } else {
                    actions
                }
            }
            AppMessage::QuitRequested => vec![AppAction::Quit],
            AppMessage::TerminalResized { width, height } => vec![AppAction::SetStatus(
                self.with_base_status(&format!("terminal resized to {width}x{height}")),
            )],
            AppMessage::DashboardSelectionNext => {
                let selection = self.dashboard.next_selection();
                vec![
                    AppAction::SetDashboardSelection(selection),
                    AppAction::SetStatus(
                        self.dashboard_status_for(self.dashboard.detail_mode, "selection moved"),
                    ),
                ]
            }
            AppMessage::DashboardSelectionPrevious => {
                let selection = self.dashboard.previous_selection();
                vec![
                    AppAction::SetDashboardSelection(selection),
                    AppAction::SetStatus(
                        self.dashboard_status_for(self.dashboard.detail_mode, "selection moved"),
                    ),
                ]
            }
            AppMessage::DashboardToggleSort(column) => vec![
                AppAction::ToggleDashboardSort(column),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Overview,
                    &format!(
                        "Sorted by {} {}",
                        column.label(),
                        self.dashboard.sort_config().toggled(column).direction.indicator()
                    ),
                )),
            ],
            AppMessage::DashboardOpenInspect => {
                if self.dashboard.selected_row().is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Overview,
                        "select an addon row before opening inspect",
                    ))]
                } else {
                    vec![
                        AppAction::SetInspectOverlay(true),
                        AppAction::SetInspectOverlayState(InspectOverlayState::default()),
                        AppAction::SetStatus(String::new()),
                    ]
                }
            }
            AppMessage::DashboardCloseOverlay => {
                if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
                    vec![
                        AppAction::SetInspectOverlay(false),
                        AppAction::SetInspectOverlayState(InspectOverlayState::default()),
                        AppAction::SetStatus(String::new()),
                    ]
                } else {
                    vec![
                        AppAction::SetStatus(String::new()),
                        AppAction::SetDetailMode(DetailMode::Overview),
                    ]
                }
            }
            AppMessage::InspectToggleSection(section) => vec![AppAction::SetInspectOverlayState(
                self.inspect_overlay.clone().toggle(section),
            )],
            AppMessage::InspectScrollUp => vec![AppAction::SetInspectOverlayState(
                self.inspect_overlay.clone().scroll_up(),
            )],
            AppMessage::InspectScrollDown => vec![AppAction::SetInspectOverlayState(
                self.inspect_overlay.clone().scroll_down(),
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
                                DetailMode::Overview,
                                &format!("cleared {}", target.item.name),
                            )
                        } else {
                            self.dashboard_status_for(
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
                        DetailMode::Overview,
                        &format!(
                            "delete {}? press y to confirm, n to cancel",
                            target.item.name
                        ),
                    )),
                ],
                None => vec![AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Overview,
                    "inspect has no addon target",
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
                        DetailMode::Overview,
                        &format!("checking {} for updates", target.item.name),
                    )),
                ],
                None => vec![AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Overview,
                    "inspect has no addon target",
                ))],
            },
            AppMessage::InspectRunUpdate => {
                if self.dashboard.update_in_progress() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Overview,
                        "selected addon update already running",
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Overview,
                        "confirm or cancel the pending delete before applying updates",
                    ))]
                } else {
                    match (
                        self.effective_addon_dir.clone(),
                        self.inspect_resolved_target(),
                    ) {
                        (_, None) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Overview,
                            "inspect has no addon target",
                        ))],
                        (None, Some(_)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Overview,
                            "addon directory is not configured",
                        ))],
                        (Some(_), Some(target)) if !is_refreshable_dashboard_item(target.item) => {
                            vec![AppAction::SetStatus(self.dashboard_status_for(
                                DetailMode::Overview,
                                &format!("{} is not updateable", target.item.name),
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
                                DetailMode::Overview,
                                &format!("updating {}", target.item.name),
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
                let selected = self.dashboard.selected_parent_folders();
                if selected.is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "select one or more parent addons before deleting",
                    ))]
                } else {
                    vec![
                        AppAction::SetPendingDelete(Some(selected.clone())),
                        AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!(
                                "delete {} selected addon{}? press y to confirm, n to cancel",
                                selected.len(),
                                plural_suffix(selected.len())
                            ),
                        )),
                    ]
                }
            }
            AppMessage::DashboardConfirmDelete => match (
                self.effective_addon_dir.clone(),
                self.dashboard
                    .pending_delete_folders()
                    .map(|value| value.to_vec()),
            ) {
                (Some(addon_dir), Some(folders)) if !folders.is_empty() => vec![
                    AppAction::StartDashboardDelete {
                        addon_dir,
                        trash_dir: self.trash_dir.clone(),
                        folders,
                    },
                    AppAction::SetStatus(
                        self.dashboard_status_for(self.dashboard.detail_mode, "delete running"),
                    ),
                ],
                _ => vec![AppAction::SetStatus(self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    "no pending delete to confirm",
                ))],
            },
            AppMessage::DashboardCancelPendingDelete => vec![
                AppAction::SetPendingDelete(None),
                AppAction::SetStatus(
                    self.dashboard_status_for(self.dashboard.detail_mode, "delete cancelled"),
                ),
            ],
            AppMessage::DashboardUndoDelete => {
                if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "confirm or cancel the pending delete before undoing the last delete",
                    ))]
                } else if self.undo_delete_in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "undo delete already running",
                    ))]
                } else if let Some(undo) = self.undo_delete.clone() {
                    vec![
                        AppAction::SetDashboardUndoInProgress(true),
                        AppAction::StartDashboardUndoDelete { undo: undo.clone() },
                        AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!(
                                "restoring last delete: {} parent addon{}, {} folder{}",
                                undo.parent_count(),
                                plural_suffix(undo.parent_count()),
                                undo.moved_folder_count(),
                                plural_suffix(undo.moved_folder_count())
                            ),
                        )),
                    ]
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "no delete undo is available",
                    ))]
                }
            }
            AppMessage::DashboardRunCheckSelected => {
                match self.dashboard.actionable_parent_targets() {
                    Err(message) => vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!("{message} before checking for updates"),
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
                                self.dashboard.detail_mode,
                                &format!(
                                    "checking {} addon{} for updates",
                                    target_len,
                                    plural_suffix(target_len)
                                ),
                            )),
                        ]
                    }
                }
            }
            AppMessage::DashboardRunUpdateSelected => {
                if self.dashboard.update_in_progress() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "selected addon update already running",
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "confirm or cancel the pending delete before applying selected updates",
                    ))]
                } else {
                    match (
                        self.effective_addon_dir.clone(),
                        self.dashboard.actionable_parent_targets(),
                    ) {
                        (_, Err(message)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!("{message} before applying updates"),
                        ))],
                        (None, Ok(_)) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            "addon directory is not configured",
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
                                    self.dashboard.detail_mode,
                                    &format!(
                                        "updating {} addon{}",
                                        selected_len,
                                        plural_suffix(selected_len)
                                    ),
                                )),
                            ]
                        }
                    }
                }
            }
            AppMessage::DashboardSelectRefreshableUpdates => {
                let refreshable = self.dashboard.refreshable_parent_folders();
                if refreshable.is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "no updateable tracked parent addons are available in the current list",
                    ))]
                } else {
                    let count = refreshable.len();
                    vec![
                        AppAction::SetSelectedDashboardParents(refreshable),
                        AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!(
                                "selected {} updateable tracked addon{}",
                                count,
                                plural_suffix(count)
                            ),
                        )),
                    ]
                }
            }
            AppMessage::DashboardToggleSelected => vec![
                AppAction::ToggleDashboardSelection,
                AppAction::SetStatus(match self.dashboard.selected_row() {
                    Some(DashboardRow {
                        kind: DashboardRowKind::OwnedChild { .. },
                        ..
                    }) => self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "owned child rows inherit parent actions | select the parent row instead",
                    ),
                    _ => self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!(
                            "selected {} addon{}",
                            selection_count_after_toggle(&self.dashboard),
                            plural_suffix(selection_count_after_toggle(&self.dashboard))
                        ),
                    ),
                }),
            ],
            AppMessage::DashboardSelectAll => vec![
                AppAction::SelectAllDashboardParents,
                AppAction::SetStatus(self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    &format!(
                        "selected all {} addon{}",
                        self.dashboard.items.len(),
                        plural_suffix(self.dashboard.items.len())
                    ),
                )),
            ],
            AppMessage::DashboardClearSelection => vec![
                AppAction::ClearDashboardSelection,
                AppAction::SetStatus(
                    self.dashboard_status_for(self.dashboard.detail_mode, "selection cleared"),
                ),
            ],
            AppMessage::DashboardToggleExpanded => vec![
                AppAction::ToggleDashboardExpanded,
                AppAction::SetStatus(
                    self.dashboard_status_for(self.dashboard.detail_mode, "tree state updated"),
                ),
            ],
            AppMessage::DashboardExpandAllRelationships => vec![
                AppAction::ExpandAllDashboardRelationships,
                AppAction::SetStatus(self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    "expanded all relationship rows",
                )),
            ],
            AppMessage::DashboardCollapseAllRelationships => vec![
                AppAction::CollapseAllDashboardRelationships,
                AppAction::SetStatus(self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    "collapsed all relationship rows",
                )),
            ],
            AppMessage::DashboardCollapseExpanded => vec![
                AppAction::CollapseDashboardExpanded,
                AppAction::SetStatus(
                    self.dashboard_status_for(self.dashboard.detail_mode, "tree state updated"),
                ),
            ],
            AppMessage::SetDetailMode(detail_mode) => vec![
                AppAction::SetDetailMode(detail_mode),
                AppAction::SetStatus(self.dashboard_status_for(
                    detail_mode,
                    &format!("{} selected", detail_mode_label(detail_mode)),
                )),
            ],
            AppMessage::InstallBeginEditing => vec![
                AppAction::SetInstallPaneState(self.install_pane.begin_editing()),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Install,
                    "editing Wago target | enter install | esc stop editing",
                )),
            ],
            AppMessage::InstallStopEditing => vec![
                AppAction::SetInstallPaneState(self.install_pane.stop_editing()),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Install,
                    "Wago direct install ready | e edit target | enter install",
                )),
            ],
            AppMessage::InstallInputChar(character) => vec![AppAction::SetInstallPaneState(
                self.install_pane.insert_char(character),
            )],
            AppMessage::InstallBackspace => vec![AppAction::SetInstallPaneState(
                self.install_pane.backspace(),
            )],
            AppMessage::InstallSubmit => {
                if self.wago_install_in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Install,
                        "Wago install already running",
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Install,
                        "confirm or cancel the pending delete before installing addons",
                    ))]
                } else if self.wago_api_key.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Install,
                        "Wago install unavailable: no API key configured",
                    ))]
                } else if self.install_pane.input.trim().is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Install,
                        "enter a Wago slug or addon URL before installing",
                    ))]
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Install,
                        "addon directory is not configured",
                    ))]
                } else {
                    let request = PendingWagoInstallRequest {
                        target: self.install_pane.input.trim().to_string(),
                        source: WagoInstallSource::DirectInput,
                    };
                    vec![
                        AppAction::SetWagoInstallInProgress(true),
                        AppAction::StartWagoInstall {
                            addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                            api_key: self.wago_api_key.clone().expect("checked above"),
                            request,
                            allow_replace: false,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Install,
                            "checking Wago package and preparing install",
                        )),
                    ]
                }
            }
            AppMessage::SearchBeginEditing => vec![
                AppAction::SetSearchPaneState(self.search_pane.begin_editing()),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Search,
                    "editing install input | enter search or install | esc stop editing",
                )),
            ],
            AppMessage::OpenSearch(mode) => vec![
                AppAction::SetSearchPaneState(self.search_pane.open_with_mode(mode)),
                AppAction::SetDetailMode(DetailMode::Search),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Search,
                    "install from Wago | type a name or Wago URL and press enter",
                )),
            ],
            AppMessage::SearchStopEditing => vec![
                AppAction::SetSearchPaneState(self.search_pane.stop_editing()),
                AppAction::SetStatus(
                    self.dashboard_status_for(DetailMode::Search, "install/search ready"),
                ),
            ],
            AppMessage::SearchInputChar(character) => vec![AppAction::SetSearchPaneState(
                self.search_pane.insert_char(character),
            )],
            AppMessage::SearchBackspace => {
                vec![AppAction::SetSearchPaneState(self.search_pane.backspace())]
            }
            AppMessage::SearchSubmit => {
                if self.search_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "Wago search already running",
                    ))]
                } else if self.wago_install_in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "Wago install already running",
                    ))]
                } else if self.wago_api_key.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "Wago search unavailable: no API key configured",
                    ))]
                } else if self.search_pane.query.trim().is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "enter an addon name or Wago URL before submitting",
                    ))]
                } else if self.dashboard.pending_delete_folders().is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "confirm or cancel the pending delete before installing addons",
                    ))]
                } else if let Some(target) = self.search_direct_install_target() {
                    if self.effective_addon_dir.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Search,
                            "addon directory is not configured",
                        ))]
                    } else {
                        let request = PendingWagoInstallRequest {
                            target: target.clone(),
                            source: WagoInstallSource::DirectInput,
                        };
                        vec![
                            AppAction::SetSearchPaneState(
                                self.search_pane.start_direct_install_check(
                                    &target,
                                    self.search_pane.query.trim(),
                                ),
                            ),
                            AppAction::SetWagoInstallInProgress(true),
                            AppAction::StartWagoInstall {
                                addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                                api_key: self.wago_api_key.clone().expect("checked above"),
                                request,
                                allow_replace: false,
                            },
                            AppAction::SetStatus(self.dashboard_status_for(
                                DetailMode::Search,
                                &format!(
                                    "checking Wago addon '{}' for install",
                                    self.search_pane.query.trim()
                                ),
                            )),
                        ]
                    }
                } else {
                    let query = self.search_pane.query.trim().to_string();
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.start_search()),
                        AppAction::StartWagoSearch {
                            query: query.clone(),
                            api_key: self.wago_api_key.clone().expect("checked above"),
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Search,
                            &format!("searching Wago for '{query}'"),
                        )),
                    ]
                }
            }
            AppMessage::SearchResultNext => {
                if self.search_pane.results.is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "no Wago search results to select",
                    ))]
                } else {
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.select_next()),
                        AppAction::SetStatus(
                            self.dashboard_status_for(DetailMode::Search, "result selection moved"),
                        ),
                    ]
                }
            }
            AppMessage::SearchResultPrevious => {
                if self.search_pane.results.is_empty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "no Wago search results to select",
                    ))]
                } else {
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.select_previous()),
                        AppAction::SetStatus(
                            self.dashboard_status_for(DetailMode::Search, "result selection moved"),
                        ),
                    ]
                }
            }
            AppMessage::SearchInstallSelected => {
                if self.wago_install_in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "Wago install already running",
                    ))]
                } else if self.wago_api_key.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "Wago install unavailable: no API key configured",
                    ))]
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "addon directory is not configured",
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
                            allow_replace: false,
                        },
                        AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Search,
                            &format!("checking Wago result '{}' for install", result.display_name),
                        )),
                    ]
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        "select a Wago search result before installing",
                    ))]
                }
            }
            AppMessage::WagoConfirmInstall => {
                if let Some(confirmation) = self.pending_wago_install_confirmation.clone() {
                    if self.wago_install_in_progress {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            "Wago install already running",
                        ))]
                    } else if self.wago_api_key.is_none() {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            "Wago install unavailable: no API key configured",
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
                                allow_replace: true,
                            },
                        ];
                        if self.dashboard.detail_mode == DetailMode::Search {
                            actions[1] = AppAction::SetSearchPaneState(
                                self.search_pane.start_installing(&confirmation.inspection),
                            );
                        }
                        let status_message = if self.dashboard.detail_mode == DetailMode::Search {
                            format!(
                                "reinstalling tracked addon '{}' from Wago",
                                confirmation.inspection.addon_name
                            )
                        } else {
                            "replacing existing addon folders from confirmed Wago install"
                                .to_string()
                        };
                        actions.push(AppAction::SetStatus(
                            self.dashboard_status_for(self.dashboard.detail_mode, &status_message),
                        ));
                        actions
                    } else {
                        vec![AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            "addon directory is not configured",
                        ))]
                    }
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "no Wago install confirmation is pending",
                    ))]
                }
            }
            AppMessage::WagoCancelInstall => vec![
                AppAction::SetPendingWagoInstallConfirmation(None),
                AppAction::SetSearchPaneState(self.search_pane.clear_install_state()),
                AppAction::SetStatus(
                    self.dashboard_status_for(self.dashboard.detail_mode, "Wago install cancelled"),
                ),
            ],
            AppMessage::ConfigSelectionNext => vec![
                AppAction::SetConfigPaneState(self.config_pane.select_next()),
                AppAction::SetStatus(
                    self.dashboard_status_for(DetailMode::Config, "config field selection moved"),
                ),
            ],
            AppMessage::ConfigSelectionPrevious => vec![
                AppAction::SetConfigPaneState(self.config_pane.select_previous()),
                AppAction::SetStatus(
                    self.dashboard_status_for(DetailMode::Config, "config field selection moved"),
                ),
            ],
            AppMessage::ConfigBeginEditing => {
                let field = self.config_pane.selected_field();
                if field.is_textual() {
                    vec![
                        AppAction::SetConfigPaneState(self.config_pane.begin_editing()),
                        AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Config,
                            &format!("editing {} | enter apply | esc cancel", field.label()),
                        )),
                    ]
                } else {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Config,
                        &format!("press enter to change {}", field.label()),
                    ))]
                }
            }
            AppMessage::ConfigInputChar(character) => vec![AppAction::SetConfigPaneState(
                self.config_pane.insert_char(character),
            )],
            AppMessage::ConfigBackspace => {
                vec![AppAction::SetConfigPaneState(self.config_pane.backspace())]
            }
            AppMessage::ConfigCommitEdit => match self.config_pane.commit_edit() {
                Ok(next) => vec![
                    AppAction::SetConfigPaneState(next),
                    AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Config,
                        "config field updated locally | s save | n reset",
                    )),
                ],
                Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Config,
                    &format!("config edit failed: {error}"),
                ))],
            },
            AppMessage::ConfigCancelEdit => vec![
                AppAction::SetConfigPaneState(self.config_pane.cancel_edit()),
                AppAction::SetStatus(
                    self.dashboard_status_for(DetailMode::Config, "config edit cancelled"),
                ),
            ],
            AppMessage::ConfigToggleSelected => vec![
                if self.config_pane.selected_field() == ConfigField::RunOnboardingAgain {
                    AppAction::ReenterOnboarding
                } else if self.config_pane.selected_field().is_textual() {
                    AppAction::SetConfigPaneState(self.config_pane.begin_editing())
                } else {
                    AppAction::SetConfigPaneState(self.config_pane.toggle_selected())
                },
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Config,
                    if self.config_pane.selected_field() == ConfigField::RunOnboardingAgain {
                        "re-entering onboarding"
                    } else if self.config_pane.selected_field().is_textual() {
                        "editing config field | enter apply | esc cancel"
                    } else {
                        "config field updated locally | s save | n reset"
                    },
                )),
            ],
            AppMessage::ConfigSave => {
                if self.config_pane.edit.is_some() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Config,
                        "finish or cancel the active config edit before saving",
                    ))]
                } else if !self.config_pane.is_dirty() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Config,
                        "no config changes to save",
                    ))]
                } else {
                    match self.config_store.write_config(&self.config_pane.draft) {
                        Ok(()) => vec![
                            AppAction::SetPersistedConfig(self.config_pane.draft.clone()),
                            AppAction::SetConfigPaneState(
                                self.config_pane.mark_saved(self.config_pane.draft.clone()),
                            ),
                            AppAction::SetStatus(
                                self.dashboard_status_for(DetailMode::Config, "config saved"),
                            ),
                        ],
                        Err(error) => vec![AppAction::SetStatus(self.dashboard_status_for(
                            DetailMode::Config,
                            &format!("config save failed: {error}"),
                        ))],
                    }
                }
            }
            AppMessage::ConfigResetDraft => vec![
                AppAction::SetConfigPaneState(self.config_pane.reset_draft()),
                AppAction::SetStatus(self.dashboard_status_for(
                    DetailMode::Config,
                    "config draft reset to saved values",
                )),
            ],
            AppMessage::BackupRunNow => {
                if self.backup_pane.in_progress {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Backup,
                        "WTF backup already running",
                    ))]
                } else if self.effective_addon_dir.is_none() {
                    vec![AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Backup,
                        "addon directory is not configured",
                    ))]
                } else {
                    vec![
                        AppAction::SetBackupPaneState(BackupPaneState {
                            backups: self.backup_pane.backups.clone(),
                            in_progress: true,
                        }),
                        AppAction::StartBackupNow {
                            addon_dir: self.effective_addon_dir.clone().expect("checked above"),
                            backup_dir: self.backup_dir.clone(),
                            retention: self.config_pane.draft.backup_retention,
                        },
                        AppAction::SetStatus(
                            self.dashboard_status_for(DetailMode::Backup, "creating WTF backup"),
                        ),
                    ]
                }
            }
            AppMessage::OnboardingNextStep => match self.onboarding.step {
                OnboardingStep::Theme => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(
                        self.with_base_status("step 2/5 | choose or validate an AddOns path"),
                    ),
                ],
                OnboardingStep::Directory => vec![AppAction::SetStatus(
                    self.with_base_status("validate or confirm a directory before continuing"),
                )],
                OnboardingStep::Wago => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(self.with_base_status("step 4/5 | adjust core settings")),
                ],
                OnboardingStep::Settings => vec![
                    AppAction::SetOnboardingState(self.onboarding.next_step()),
                    AppAction::SetStatus(self.with_base_status("step 5/5 | review setup")),
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
                        AppAction::SetStatus(self.with_base_status(&format!(
                            "step {}/5",
                            self.onboarding.step.previous().index() + 1
                        ))),
                    ]
                }
            }
            AppMessage::OnboardingToggleTheme => vec![
                AppAction::SetOnboardingState(self.onboarding.toggle_theme()),
                AppAction::SetStatus(self.with_base_status("theme updated | enter next")),
            ],
            AppMessage::OnboardingSettingsNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_settings_field()),
                AppAction::SetStatus(self.with_base_status("settings selection moved")),
            ],
            AppMessage::OnboardingSettingsPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_settings_field()),
                AppAction::SetStatus(self.with_base_status("settings selection moved")),
            ],
            AppMessage::OnboardingSettingIncrease => vec![
                AppAction::SetOnboardingState(self.onboarding.adjust_settings_field(1)),
                AppAction::SetStatus(self.with_base_status("setting updated")),
            ],
            AppMessage::OnboardingSettingDecrease => vec![
                AppAction::SetOnboardingState(self.onboarding.adjust_settings_field(-1)),
                AppAction::SetStatus(self.with_base_status("setting updated")),
            ],
            AppMessage::OnboardingBeginEditing => {
                let next_state = match self.onboarding.step {
                    OnboardingStep::Directory => self.onboarding.begin_directory_editing(),
                    OnboardingStep::Wago => self.onboarding.begin_wago_editing(),
                    _ => self.onboarding.clone(),
                };
                let status = match self.onboarding.step {
                    OnboardingStep::Directory => {
                        "editing path | enter validate | d deep scan | esc stop editing"
                    }
                    OnboardingStep::Wago => "editing Wago API key | enter apply | esc cancel",
                    _ => "editing",
                };
                vec![
                    AppAction::SetOnboardingState(next_state),
                    AppAction::SetStatus(self.with_base_status(status)),
                ]
            }
            AppMessage::OnboardingStopEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.stop_editing()),
                AppAction::SetStatus(match self.onboarding.step {
                    OnboardingStep::Directory => self.location_finder_status(),
                    OnboardingStep::Wago => {
                        self.with_base_status("wago step ready | e edit key | enter next")
                    }
                    _ => self.with_base_status("editing stopped"),
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
                        AppAction::SetOnboardingState(self.onboarding.error(
                            "this profile cannot target the default profile AddOns directory",
                        )),
                        AppAction::SetStatus(
                            self.with_base_status(
                                "blocked default target | choose another location",
                            ),
                        ),
                    ];
                }

                match validate_addons_path(&current_path) {
                    Ok(()) => {
                        let next_state = self.onboarding_found_state(current_path.clone());
                        vec![
                            AppAction::SetOnboardingState(next_state),
                            AppAction::SetStatus(self.found_status_for_path(&current_path)),
                        ]
                    }
                    Err(error) => vec![
                        AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                        AppAction::SetStatus(
                            self.with_base_status(&format!("validation failed: {error}")),
                        ),
                    ],
                }
            }
            AppMessage::OnboardingDeepScan => {
                let root = PathBuf::from(self.onboarding.input.trim());
                vec![
                    AppAction::SetOnboardingState(self.onboarding.start_deep_scan()),
                    AppAction::SetStatus(self.with_base_status("deep scan running | esc cancel")),
                    AppAction::StartOnboardingDeepScan(root),
                ]
            }
            AppMessage::OnboardingCancel => {
                if matches!(self.onboarding.phase, OnboardingPhase::DeepScanning(_)) {
                    vec![
                        AppAction::CancelOnboardingScan,
                        AppAction::SetOnboardingState(self.onboarding.cancelled()),
                        AppAction::SetStatus(self.with_base_status(
                            "deep scan cancelled | enter validate | d deep scan | e edit path",
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
                        self.with_base_status("directory confirmed | step 3/5 Wago setup"),
                    ),
                ],
                FoundAction::EnterDifferentPath => vec![
                    AppAction::SetOnboardingState(self.onboarding.begin_directory_editing()),
                    AppAction::SetStatus(self.with_base_status(
                        "editing path | enter validate | d deep scan | esc stop editing",
                    )),
                ],
            },
            AppMessage::BackgroundTask(event) => match event {
                AppTaskEvent::Onboarding(OnboardingTaskEvent::QuickCheckFinished(Some(path))) => {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding_found_state(path.clone())),
                        AppAction::SetStatus(self.found_status_for_path(&path)),
                    ]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::QuickCheckFinished(None)) => vec![
                    AppAction::SetOnboardingState(self.onboarding.ready()),
                    AppAction::SetStatus(self.with_base_status(
                        "no install found yet | enter validate | d deep scan | e edit path",
                    )),
                ],
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanProgress(progress)) => {
                    vec![AppAction::SetOnboardingState(
                        self.onboarding.set_scan_progress(progress),
                    )]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Ok(Some(path)))) => {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding_found_state(path.clone())),
                        AppAction::SetStatus(self.found_status_for_path(&path)),
                    ]
                }
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Ok(None))) => vec![
                    AppAction::SetOnboardingState(
                        self.onboarding
                            .error("no WoW installation found from this root".to_string()),
                    ),
                    AppAction::SetStatus(self.with_base_status(
                        "deep scan finished with no result | e edit path | d scan again",
                    )),
                ],
                AppTaskEvent::Onboarding(OnboardingTaskEvent::DeepScanFinished(Err(error))) => {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                        AppAction::SetStatus(
                            self.with_base_status(&format!("deep scan failed: {error}")),
                        ),
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
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!(
                            "deleted {} addon{}, moved {} folder{} to trash{}, sync complete",
                            outcome.deleted_parents,
                            plural_suffix(outcome.deleted_parents),
                            outcome.deleted_folders,
                            plural_suffix(outcome.deleted_folders),
                            if outcome.undo_delete.is_some() {
                                " | press z to undo"
                            } else {
                                ""
                            }
                        ),
                    )),
                ],
                AppTaskEvent::DashboardDeleteFinished(Err(error)) => vec![
                    AppAction::SetPendingDelete(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!("delete failed: {error}"),
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
                        self.dashboard.detail_mode,
                        &format!(
                            "undo complete: restored {} parent addon{} and {} folder{}",
                            outcome.restored_parents,
                            plural_suffix(outcome.restored_parents),
                            outcome.restored_folders,
                            plural_suffix(outcome.restored_folders)
                        ),
                    )),
                ],
                AppTaskEvent::DashboardUndoFinished(Err(error)) => vec![
                    AppAction::SetDashboardUndoInProgress(false),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!("undo delete failed: {error}"),
                    )),
                ],
                AppTaskEvent::DashboardJobProgress(progress) => vec![
                    AppAction::SetDashboardJobUi(Some(progress.clone())),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!(
                            "{} {} of {} | {}",
                            progress.kind.label().to_ascii_lowercase(),
                            progress.current_index,
                            progress.total,
                            progress.current_addon
                        ),
                    )),
                ],
                AppTaskEvent::DashboardCheckFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.addons),
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &dashboard_check_status_message(
                            &outcome.targets,
                            outcome.total,
                            outcome.live_checked,
                            outcome.cached,
                            outcome.errors,
                        ),
                    )),
                ],
                AppTaskEvent::DashboardCheckFinished(Err(error)) => vec![
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!("check failed: {error}"),
                    )),
                ],
                AppTaskEvent::DashboardUpdateFinished(Ok(outcome)) => {
                    let status_message = dashboard_update_status_message(&outcome);
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
                        AppAction::SetStatus(
                            self.dashboard_status_for(self.dashboard.detail_mode, &status_message),
                        ),
                    ]
                }
                AppTaskEvent::DashboardUpdateFinished(Err(error)) => vec![
                    AppAction::SetDashboardUpdateInProgress(false),
                    AppAction::SetDashboardJobUi(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!("selected addon update failed: {error}"),
                    )),
                ],
                AppTaskEvent::BackupFinished(Ok(outcome)) => {
                    let mut message = format!(
                        "backup complete: {} ({} bytes)",
                        outcome.backup.label, outcome.backup.size_bytes
                    );
                    if !outcome.pruned_files.is_empty() {
                        message.push_str(&format!(
                            " | pruned {} older backup{}",
                            outcome.pruned_files.len(),
                            plural_suffix(outcome.pruned_files.len())
                        ));
                    }
                    vec![
                        AppAction::SetBackupPaneState(BackupPaneState {
                            backups: outcome.backups,
                            in_progress: false,
                        }),
                        AppAction::SetStatus(
                            self.dashboard_status_for(DetailMode::Backup, &message),
                        ),
                    ]
                }
                AppTaskEvent::BackupFinished(Err(error)) => vec![
                    AppAction::SetBackupPaneState(BackupPaneState {
                        backups: self.backup_pane.backups.clone(),
                        in_progress: false,
                    }),
                    AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Backup,
                        &format!("WTF backup failed: {error}"),
                    )),
                ],
                AppTaskEvent::WagoSearchFinished(Ok(outcome)) => vec![
                    AppAction::SetSearchPaneState(
                        self.search_pane
                            .finish_search(outcome.query.clone(), outcome.results.clone()),
                    ),
                    AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        &format!(
                            "Wago search complete: {} result{}",
                            outcome.results.len(),
                            plural_suffix(outcome.results.len())
                        ),
                    )),
                ],
                AppTaskEvent::WagoSearchFinished(Err(error)) => vec![
                    AppAction::SetSearchPaneState(self.search_pane.fail_search()),
                    AppAction::SetStatus(self.dashboard_status_for(
                        DetailMode::Search,
                        &format!("Wago search failed: {error}"),
                    )),
                ],
                AppTaskEvent::WagoInstallFinished(Ok(
                    WagoInstallTaskOutcome::NeedsConfirmation {
                        request,
                        inspection,
                    },
                )) => {
                    let confirmation_status = if inspection.tracked_parent.is_some() {
                        "tracked addon already installed: press y to reinstall, n or esc to cancel"
                    } else {
                        "Wago install needs confirmation: press y to replace, n or esc to cancel"
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
                        AppAction::SetStatus(
                            self.dashboard_status_for(
                                self.dashboard.detail_mode,
                                confirmation_status,
                            ),
                        ),
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
                            self.dashboard.detail_mode,
                            &format!(
                                "installed Wago addon '{}' into {} | sync complete",
                                outcome.summary.addon_name, outcome.summary.parent_folder
                            ),
                        )),
                    ]
                }
                AppTaskEvent::WagoInstallFinished(Err(error)) => {
                    let (addon_id, addon_name) = self.search_pane.install_identity();
                    vec![
                        AppAction::SetSearchPaneState(self.search_pane.fail_install(
                            addon_id,
                            addon_name,
                            error.clone(),
                        )),
                        AppAction::SetWagoInstallInProgress(false),
                        AppAction::SetStatus(self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!("Wago install failed: {error}"),
                        )),
                    ]
                }
            },
        }
    }

    fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::Quit => self.quit_requested = true,
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
                    .dashboard_toast
                    .as_ref()
                    .is_some_and(|toast| Instant::now() >= toast.expires_at)
                {
                    self.shell_ui.dashboard_toast = None;
                }
            }
            AppAction::SetFooterKeyPulse(pulse) => self.shell_ui.footer_pulse = pulse,
            AppAction::SetInspectOverlay(active) => {
                self.shell_ui.overlay.active = active.then_some(OverlayKind::Inspect);
            }
            AppAction::SetInspectOverlayState(state) => {
                self.inspect_overlay = state;
            }
            AppAction::SetStatus(status) => {
                if let Some(toast) = self.dashboard_toast_from_status(&status) {
                    self.shell_ui.dashboard_toast = Some(toast);
                }
                self.status_line = status;
            }
            AppAction::SetDashboardSelection(selection) => {
                self.dashboard.list_state.select(selection)
            }
            AppAction::ToggleDashboardSort(column) => self.dashboard.toggle_sort(column),
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
            AppAction::SetDashboardUpdateInProgress(in_progress) => {
                self.dashboard.set_update_in_progress(in_progress);
            }
            AppAction::SetDashboardJobUi(job_ui) => {
                self.dashboard.set_job_ui(job_ui);
            }
            AppAction::SetDashboardUpdateSummary(summary) => {
                self.dashboard.set_last_update_summary(summary);
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
            AppAction::ToggleDashboardSelection => {
                if self.dashboard.toggle_selected_parent() {
                    self.status_line = self.dashboard_status_for(
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
                    self.status_line =
                        self.dashboard_status_for(self.dashboard.detail_mode, "selection cleared");
                }
            }
            AppAction::ToggleDashboardExpanded => {
                if self.dashboard.toggle_selected_expanded() {
                    self.status_line =
                        self.dashboard_status_for(self.dashboard.detail_mode, "tree state updated");
                }
            }
            AppAction::ExpandAllDashboardRelationships => {
                if self.dashboard.expand_all_relationships() {
                    self.status_line = self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "expanded all relationship rows",
                    );
                }
            }
            AppAction::CollapseAllDashboardRelationships => {
                if self.dashboard.collapse_all_relationships() {
                    self.status_line = self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        "collapsed all relationship rows",
                    );
                }
            }
            AppAction::CollapseDashboardExpanded => {
                if self.dashboard.collapse_selected_expanded() {
                    self.status_line =
                        self.dashboard_status_for(self.dashboard.detail_mode, "tree state updated");
                }
            }
            AppAction::SetDetailMode(detail_mode) => {
                self.dashboard.detail_mode = detail_mode;
                self.install_pane = self.install_pane.stop_editing();
                if detail_mode != DetailMode::Search {
                    self.search_pane = self.search_pane.stop_editing();
                }
                self.config_pane.edit = None;
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
                self.search_pane = self.search_pane.stop_editing();
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
            AppAction::SetOnboardingState(state) => self.onboarding = state,
            AppAction::StartOnboardingQuickCheck => {
                let sender = self.task_events_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(detect_known_addons_path)
                        .await
                        .ok()
                        .flatten();
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
                        search_for_wow(
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
                    self.status_line =
                        self.with_base_status("finish blocked | select a valid AddOns directory");
                    self.shell_mode = ShellMode::Onboarding;
                    return;
                };
                if self.is_guarded_path(&path) {
                    self.onboarding = self
                        .onboarding
                        .error("this profile cannot target the default profile AddOns directory");
                    self.status_line = self.with_base_status(
                        "refused to save default target into non-default profile",
                    );
                    self.shell_mode = ShellMode::Onboarding;
                    return;
                }

                match self.config_store.write_new_config(&config) {
                    Ok(()) => {
                        self.config_present = true;
                        self.config_pane = self.config_pane.mark_saved(config.clone());
                        self.wago_api_key = config.wago_api_key.clone();
                        self.effective_addon_dir = Some(path.clone());
                        self.shell_mode = ShellMode::Dashboard;
                        if tokio::runtime::Handle::try_current().is_ok() {
                            self.apply(AppAction::StartAddonScan(path));
                        } else {
                            self.scan_state = ScanState::Pending;
                            self.status_line = self.dashboard_status_for(
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
                        self.status_line =
                            self.with_base_status(&format!("failed to save config: {error}"));
                    }
                }
            }
            AppAction::StartAddonScan(path) => {
                self.scan_state = ScanState::Running(path.clone());
                self.status_line = self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    &format!("scanning {} and syncing state", path.display()),
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
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = run_dashboard_update_task(
                        &state_db_file,
                        &addon_dir,
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
                tokio::spawn(async move {
                    let result = run_dashboard_check_task(
                        &state_db_file,
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
            AppAction::StartWagoSearch { query, api_key } => {
                let sender = self.task_events_tx.clone();
                tokio::spawn(async move {
                    let result = search_wago_addons(&query, &api_key)
                        .await
                        .map(|results| WagoSearchOutcome { query, results });
                    let _ = sender.send(AppTaskEvent::WagoSearchFinished(result));
                });
            }
            AppAction::StartWagoInstall {
                addon_dir,
                api_key,
                request,
                allow_replace,
            } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|error| error.to_string())?;
                        runtime.block_on(run_wago_install_task(
                            &state_db_file,
                            &addon_dir,
                            &api_key,
                            request,
                            allow_replace,
                        ))
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));
                    let _ = sender.send(AppTaskEvent::WagoInstallFinished(result));
                });
            }
            AppAction::ReplaceDashboardAddons(addons) => self.dashboard.replace_addons(addons),
            AppAction::CompleteAddonScan { path, summary } => {
                self.scan_state = ScanState::Succeeded {
                    path: path.clone(),
                    summary: summary.clone(),
                };
                self.status_line = self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    &format!(
                        "scan complete | {} addons synced, {} removed",
                        summary.upserted_addons, summary.removed_addons
                    ),
                );
            }
            AppAction::FailAddonScan(error) => {
                self.scan_state = ScanState::Failed(error.clone());
                self.status_line = self.dashboard_status_for(
                    self.dashboard.detail_mode,
                    &format!("scan failed: {error}"),
                );
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let layout_mode = self.shell_layout_mode(frame.area());
        let shell = self.shell_frame(frame.area(), layout_mode);
        self.render_header(frame, shell.header, layout_mode);
        match self.shell_mode {
            ShellMode::Onboarding => self.render_onboarding(frame, shell.body),
            ShellMode::Dashboard => self.render_dashboard(frame, shell.body),
        }
        self.render_overlay_host(frame, shell.body);
        self.render_footer(frame, shell.footer);
    }

    fn shell_layout_mode(&self, area: Rect) -> ShellLayoutMode {
        if area.width < 110 || area.height < 28 {
            ShellLayoutMode::Compact
        } else {
            ShellLayoutMode::Standard
        }
    }

    fn header_variant(&self, mode: ShellLayoutMode) -> HeaderVariant {
        match mode {
            ShellLayoutMode::Standard => HeaderVariant::FullLogo,
            ShellLayoutMode::Compact => HeaderVariant::CompactLogo,
        }
    }

    fn shell_frame(&self, area: Rect, mode: ShellLayoutMode) -> ShellFrame {
        let header_height = if self.shell_mode == ShellMode::Onboarding {
            4
        } else {
            match self.header_variant(mode) {
                HeaderVariant::FullLogo => 7,
                HeaderVariant::CompactLogo => {
                    if self.dashboard.job_ui().is_some() {
                        7
                    } else {
                        5
                    }
                }
            }
        };
        let footer_height = 4;
        let [header, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(12),
                Constraint::Length(footer_height),
            ])
            .areas(area);
        ShellFrame {
            header,
            body,
            footer,
        }
    }

    fn logo_line(&self, text: &str, style: LogoStyle) -> Line<'static> {
        match style {
            LogoStyle::FruitGradient => self.fruit_gradient_logo_line(text),
            LogoStyle::StripedLegacy => self.striped_legacy_logo_line(text),
        }
    }

    fn footer_hint_styles(&self, id: FooterHintId) -> (Style, Style) {
        if self
            .shell_ui
            .footer_pulse
            .is_some_and(|pulse| pulse.hint == id)
        {
            return (
                Style::default()
                    .fg(Color::Rgb(24, 24, 34))
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
                .bg(Color::Rgb(31, 37, 58))
                .add_modifier(Modifier::BOLD),
            Style::default().fg(self.ui_theme.panel_title),
        )
    }

    fn scan_status_with_motion(&self) -> String {
        match self.scan_state {
            ScanState::Pending | ScanState::Running(_) => {
                format!(
                    "{} {}",
                    self.shell_ui.motion.spinner_frame(),
                    self.scan_status_label()
                )
            }
            _ => self.scan_status_label(),
        }
    }

    fn dashboard_job_header_line(&self) -> Option<Line<'static>> {
        let job = self.dashboard.job_ui()?;
        let accent = job.kind.accent(self.ui_theme);
        let label = format!(
            "{} {}",
            self.shell_ui.motion.spinner_frame(),
            job.kind.label()
        );
        let mut spans = vec![
            Span::styled(job.kind.label(), Style::default().fg(self.ui_theme.muted)),
            Span::raw(" "),
        ];
        spans.extend(shimmer_text_spans(
            &label,
            accent,
            Color::Rgb(255, 244, 214),
            ShimmerConfig::action(),
        ));
        spans.extend([
            Span::styled(
                format!("  {}/{}", job.current_index, job.total),
                Style::default().fg(self.ui_theme.info),
            ),
            Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(job.current_addon.clone(), Style::default().fg(accent)),
        ]);
        Some(Line::from(spans))
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
            Color::Rgb(255, 244, 214),
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

        let content = Paragraph::new(self.onboarding_content_lines()).wrap(Wrap { trim: false });
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

    fn onboarding_content_lines(&self) -> Vec<Line<'static>> {
        match self.onboarding.step {
            OnboardingStep::Theme => self.onboarding_theme_lines(),
            OnboardingStep::Directory => self.onboarding_directory_lines(),
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

    fn onboarding_directory_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(Span::styled(
                "Where is your WoW AddOns folder?",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

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
                lines.push(Line::from(format!(
                    "Current path: {}",
                    self.onboarding.input
                )));
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
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Enter validates. d runs a deep scan. e edits the path manually.",
                    Style::default().fg(self.ui_theme.muted),
                )));
            }
            OnboardingPhase::DeepScanning(progress) => {
                lines.push(Line::from(Span::styled(
                    format!("Scanning... {} directories checked", progress.dirs_scanned),
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(progress.current_path.clone()));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Esc cancels the deep scan.",
                    Style::default().fg(self.ui_theme.muted),
                )));
            }
            OnboardingPhase::Found(found) => {
                lines.push(Line::from(vec![
                    Span::styled("✓ Detected: ", Style::default().fg(self.ui_theme.success)),
                    Span::styled(found.path.clone(), Style::default().fg(self.ui_theme.info)),
                ]));
                lines.push(Line::from(""));
                for (index, label) in ["Use this path", "Enter different path"].iter().enumerate() {
                    let selected = found.selected_action == index;
                    lines.push(Line::from(vec![
                        Span::styled(
                            if selected { "› " } else { "  " },
                            Style::default().fg(self.ui_theme.highlight),
                        ),
                        Span::styled(
                            (*label).to_string(),
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
            }
            OnboardingPhase::Error(message) => {
                lines.push(Line::from(Span::styled(
                    format!("Validation failed: {message}"),
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Edit the path, pick another suggestion, or scan again.",
                    Style::default().fg(self.ui_theme.muted),
                )));
            }
            OnboardingPhase::Cancelled => {
                lines.push(Line::from(Span::styled(
                    "Deep scan cancelled.",
                    Style::default().fg(self.ui_theme.warning),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Pick another root or edit the path manually.",
                    Style::default().fg(self.ui_theme.muted),
                )));
            }
        }

        if self.onboarding.is_editing {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Editing: {}", self.onboarding.input),
                Style::default().fg(self.ui_theme.highlight),
            )));
        }

        lines
    }

    fn onboarding_wago_lines(&self) -> Vec<Line<'static>> {
        let value = self.onboarding.wago_api_key_value();
        vec![
            Line::from(Span::styled(
                "Connect Wago (optional)",
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                if value.is_empty() {
                    "No API key configured yet."
                } else {
                    "API key saved in draft."
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
            Line::from(format!("AddOns path: {}", self.onboarding.input)),
            Line::from(format!(
                "Wago: {}",
                if self.onboarding.wago_api_key_value().is_empty() {
                    "not configured".to_string()
                } else {
                    format!(
                        "configured ({} chars)",
                        self.onboarding.wago_api_key_value().len()
                    )
                }
            )),
            Line::from(format!("Back up WTF: {}", self.onboarding.draft.backup_wtf)),
            Line::from(format!(
                "Backup retention: {}",
                self.onboarding.draft.backup_retention
            )),
            Line::from(format!(
                "Show libraries: {}",
                self.onboarding.draft.show_libs
            )),
            Line::from(format!(
                "Default screen: {}",
                self.onboarding_setting_value(OnboardingSettingsField::DefaultScreen)
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

        if let Some(pending_delete_folders) = self.dashboard.pending_delete_folders() {
            lines.push(self.overlay_section_title("Delete pending"));
            lines.push(self.overlay_kv_line(
                "Batch",
                format!(
                    "{} parent{}",
                    pending_delete_folders.len(),
                    plural_suffix(pending_delete_folders.len())
                ),
            ));
            lines.push(self.overlay_kv_line("Targets", pending_delete_folders.join(", ")));
            lines.push(Line::from(""));
        }

        if let Some(confirmation) = self.pending_wago_install_confirmation.as_ref() {
            lines.push(self.overlay_section_title("Replace pending"));
            lines.push(self.overlay_kv_line("Addon", confirmation.inspection.addon_name.clone()));
            lines.push(
                self.overlay_kv_line("Parent", confirmation.inspection.parent_folder.clone()),
            );
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
                    if self.install_pane.input.trim().is_empty() {
                        "<empty>".to_string()
                    } else {
                        self.install_pane.input.clone()
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
                    lines.push(self.overlay_hint_line(
                        "Accepted sources: profile config, WAGO_API_KEY, repo-root .env.",
                    ));
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
                    if self.search_pane.query.trim().is_empty() {
                        "<empty>".to_string()
                    } else {
                        self.search_pane.query.clone()
                    },
                ));
                lines.push(self.overlay_kv_line(
                    "State",
                    if self.search_pane.in_progress {
                        "running"
                    } else if self.search_pane.is_editing {
                        "editing"
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
                    lines.push(self.overlay_hint_line(
                        "Accepted sources: profile config, WAGO_API_KEY, repo-root .env.",
                    ));
                } else {
                    lines.push(self.overlay_hint_line("Retail and stable only."));
                    lines.push(self.overlay_hint_line(if self.search_pane.is_editing {
                        "Type query, then Enter to search. Esc stops editing."
                    } else {
                        "Press e to edit query. j/k changes result. Enter installs selected result."
                    }));
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
                        "{} tracked | {} ready | {} manual | {} unmgd",
                        inventory.total,
                        inventory.refreshable,
                        inventory.manual,
                        inventory.unmanaged
                    ),
                ));
                lines.push(self.overlay_kv_line(
                    "Selection",
                    format!(
                        "{} ready | {} manual | {} unmgd",
                        selection.refreshable, selection.manual, selection.unmanaged
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
                            "{} targets | {} updated | {} current | {} manual | {} unmgd | {} unsupported | {} errors",
                            last_summary.targets,
                            last_summary.updated_addons,
                            last_summary.up_to_date,
                            last_summary.skipped_manual,
                            last_summary.skipped_unmanaged,
                            last_summary.skipped_unsupported,
                            last_summary.errors
                        ),
                    ));
                }
            }
            DetailMode::Config => {
                lines.push(self.overlay_section_title("Config"));
                lines.push(self.overlay_kv_line("Target", self.rendered_target_path()));
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
                                .map(|edit| edit.value.clone())
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
                lines.push(self.overlay_section_title("Notes"));
                lines.push(self.overlay_hint_line(
                    "Only curated settings are editable here. Addon dir and runtime intervals stay read-only.",
                ));
                lines.push(self.overlay_hint_line(
                    "Use 'Run onboarding again' to reopen the location finder manually.",
                ));
            }
            DetailMode::Backup => {
                lines.push(self.overlay_section_title("Backup now"));
                lines.push(self.overlay_kv_line("Target", self.rendered_target_path()));
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
                    } else {
                        "ready"
                    },
                ));
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Actions"));
                lines.push(self.overlay_hint_line("Press r or Enter to create a WTF backup now."));
                lines.push(
                    self.overlay_hint_line(
                        "Restore stays intentionally deferred to a later slice.",
                    ),
                );
                lines.push(Line::from(""));
                lines.push(self.overlay_section_title("Recent backups"));
                if self.backup_pane.backups.is_empty() {
                    lines.push(self.overlay_hint_line("No backups created yet."));
                } else {
                    for entry in self.backup_pane.backups.iter().take(8) {
                        lines.push(Line::from(format!(
                            "{} | {} bytes",
                            entry.label, entry.size_bytes
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

    fn inspect_resolved_target(&self) -> Option<InspectResolvedTarget<'_>> {
        let item = self.dashboard.selected_item()?;
        Some(InspectResolvedTarget {
            item,
            opened_from_child: self.dashboard.selected_owned_child_folder(),
        })
    }

    fn base_status_line(&self) -> String {
        match self.shell_mode {
            ShellMode::Onboarding => format!("profile {} | q quit", self.runtime.profile_name),
            ShellMode::Dashboard => format!("profile {}", self.runtime.profile_name),
        }
    }

    fn with_base_status(&self, message: &str) -> String {
        format!("{message} | {}", self.base_status_line())
    }

    fn dashboard_status_for(&self, detail_mode: DetailMode, message: &str) -> String {
        self.with_base_status(&format!("{} | {message}", detail_mode_label(detail_mode)))
    }

    fn location_finder_status(&self) -> String {
        self.with_base_status("location finder | enter validate | d deep scan | e edit path")
    }

    fn rendered_target_path(&self) -> String {
        self.effective_addon_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unconfigured>".to_string())
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
                        "<empty>".to_string()
                    } else {
                        format!("set ({} chars)", value.len())
                    }
                })
                .unwrap_or_else(|| "<unset>".to_string()),
            ConfigField::BackupWtf => self.config_pane.draft.backup_wtf.to_string(),
            ConfigField::BackupRetention => self.config_pane.draft.backup_retention.to_string(),
            ConfigField::Theme => match self.config_pane.draft.theme {
                ThemeMode::Dark => "dark".to_string(),
                ThemeMode::Light => "light".to_string(),
            },
            ConfigField::ShowLibs => self.config_pane.draft.show_libs.to_string(),
            ConfigField::DefaultScreen => match self.config_pane.draft.default_screen {
                DefaultScreen::Manage => "manage".to_string(),
                DefaultScreen::Install => "install".to_string(),
                DefaultScreen::Config => "config".to_string(),
                DefaultScreen::WagoSearch => "wago_search".to_string(),
            },
            ConfigField::RunOnboardingAgain => "press Enter".to_string(),
        }
    }

    fn onboarding_setting_value(&self, field: OnboardingSettingsField) -> String {
        match field {
            OnboardingSettingsField::BackupWtf => self.onboarding.draft.backup_wtf.to_string(),
            OnboardingSettingsField::BackupRetention => {
                self.onboarding.draft.backup_retention.to_string()
            }
            OnboardingSettingsField::ShowLibs => self.onboarding.draft.show_libs.to_string(),
            OnboardingSettingsField::DefaultScreen => match self.onboarding.draft.default_screen {
                DefaultScreen::Manage => "manage".to_string(),
                DefaultScreen::Install => "install".to_string(),
                DefaultScreen::Config => "config".to_string(),
                DefaultScreen::WagoSearch => "wago_search".to_string(),
            },
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

    fn onboarding_found_state(&self, path: PathBuf) -> OnboardingState {
        let state = self.onboarding.found(path.clone());
        if self.is_guarded_path(&path) {
            state.next_found_action()
        } else {
            state
        }
    }

    fn found_status_for_input(&self) -> String {
        self.found_status_for_path(Path::new(self.onboarding.input.trim()))
    }

    fn found_status_for_path(&self, path: &Path) -> String {
        if self.is_guarded_path(path) {
            self.with_base_status("found default profile target | choose another location")
        } else {
            self.with_base_status("found WoW installation | choose an action, then press enter")
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
            ScanState::Idle => "idle".to_string(),
            ScanState::Pending => "pending".to_string(),
            ScanState::Running(path) => format!("running ({})", path.display()),
            ScanState::Succeeded { summary, .. } => format!(
                "synced {} addons, removed {}",
                summary.upserted_addons, summary.removed_addons
            ),
            ScanState::Failed(error) => format!("failed ({error})"),
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
    allow_replace: bool,
) -> std::result::Result<WagoInstallTaskOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    if !allow_replace {
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

    let summary = install_wago_addon_with_replace(
        &mut database,
        addon_dir,
        &request.target,
        api_key,
        WagoStability::Stable,
        false,
        allow_replace,
    )
    .await?;
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;
    Ok(WagoInstallTaskOutcome::Installed(WagoInstallOutcome {
        summary,
        sync,
    }))
}

async fn run_dashboard_update_task(
    state_db_file: &Path,
    addon_dir: &Path,
    folders: &[String],
    wago_api_key: Option<String>,
    check_interval_secs: u64,
    mut on_progress: impl FnMut(DashboardJobKind, usize, usize, String),
) -> std::result::Result<DashboardUpdateOutcome, String> {
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    preflight_stale_update_checks(
        &mut database,
        folders,
        wago_api_key.as_deref(),
        check_interval_secs,
        &mut on_progress,
    )
    .await?;
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
    let summary = run.summary;
    let sync = sync_dashboard_state(state_db_file, addon_dir)?;

    Ok(DashboardUpdateOutcome {
        summary,
        updated_addon_name,
        sync,
    })
}

async fn run_dashboard_check_task(
    state_db_file: &Path,
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
        refresh_live_update_checks(database, std::slice::from_ref(folder), wago_api_key)
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

fn sync_dashboard_state(
    state_db_file: &Path,
    addon_dir: &Path,
) -> std::result::Result<AddonScanOutcome, String> {
    let scanned =
        scan_addons_dir(addon_dir, GameFlavor::Retail).map_err(|error| error.to_string())?;
    let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let tracked = database.list_addons().map_err(|error| error.to_string())?;
    let drift_report = compute_drift_report(&tracked, &scanned);
    let summary = database
        .reconcile_scanned_addons(&scanned)
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
            let mut addon = AddonRecord::new(&item.name, &item.folder, item.source);
            addon.version = item.version.clone();
            addon.remote_version = item.remote_version.clone();
            addon.git_commit = item.git_commit.clone();
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

fn is_refreshable_dashboard_item(item: &DashboardItem) -> bool {
    item.source != SourceKind::Manual && item.has_authoritative_owned_folders
}

fn summarize_refreshability(items: &[&DashboardItem]) -> DashboardRefreshabilitySummary {
    let mut summary = DashboardRefreshabilitySummary {
        total: items.len(),
        refreshable: 0,
        manual: 0,
        unmanaged: 0,
    };

    for item in items {
        if item.source == SourceKind::Manual {
            summary.manual += 1;
        } else if item.has_authoritative_owned_folders {
            summary.refreshable += 1;
        } else {
            summary.unmanaged += 1;
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

    DashboardUpdateRunSummary {
        targets: checks.len(),
        updated_addons: 0,
        up_to_date,
        update_available,
        unknown,
        errors,
        skipped_manual: 0,
        skipped_unmanaged: 0,
        skipped_unsupported: 0,
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
        skipped_unmanaged: summary.skipped_unmanaged,
        skipped_unsupported: summary.skipped_unsupported,
        scanned_addons: 0,
    }
}

fn dashboard_update_status_message(outcome: &DashboardUpdateOutcome) -> String {
    let summary = outcome.summary;
    let headline = if summary.updated_addons == 1 {
        if let Some(addon_name) = outcome.updated_addon_name.as_deref() {
            format!("Updated {addon_name}")
        } else {
            "Updated 1 addon".to_string()
        }
    } else if summary.updated_addons > 0 {
        format!("Updated {}", addon_count_label(summary.updated_addons))
    } else if summary.up_to_date > 0
        && summary.skipped_manual == 0
        && summary.skipped_unmanaged == 0
        && summary.skipped_unsupported == 0
        && summary.errors == 0
    {
        "Already up to date".to_string()
    } else {
        "No changes".to_string()
    };

    let mut details = Vec::new();
    if summary.up_to_date > 0 {
        details.push(format!("{} current", summary.up_to_date));
    }
    if summary.skipped_manual > 0 {
        details.push(format!("{} manual skipped", summary.skipped_manual));
    }
    if summary.skipped_unmanaged > 0 {
        details.push(format!("{} unmanaged skipped", summary.skipped_unmanaged));
    }
    if summary.skipped_unsupported > 0 {
        details.push(format!(
            "{} unsupported skipped",
            summary.skipped_unsupported
        ));
    }
    if summary.errors > 0 {
        details.push(format!(
            "{} error{}",
            summary.errors,
            if summary.errors == 1 { "" } else { "s" }
        ));
    }

    if details.is_empty() {
        headline
    } else {
        format!("{headline} · {}", details.join(" · "))
    }
}

fn dashboard_check_status_message(
    targets: &[String],
    total: usize,
    live_checked: usize,
    cached: usize,
    errors: usize,
) -> String {
    if live_checked == 0 && errors == 0 && cached > 0 {
        return if total == 1 {
            format!(
                "{} was checked recently",
                targets.first().map(String::as_str).unwrap_or("This addon")
            )
        } else {
            format!("{} addons were checked recently", cached)
        };
    }

    let mut details = Vec::new();
    if live_checked > 0 {
        details.push(format!("{live_checked} checked now"));
    }
    if cached > 0 {
        details.push(format!("{cached} recently checked"));
    }
    if errors > 0 {
        details.push(format!(
            "{errors} error{}",
            if errors == 1 { "" } else { "s" }
        ));
    }

    if details.is_empty() {
        "Nothing to check".to_string()
    } else {
        format!("Check complete · {}", details.join(" · "))
    }
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

fn source_compact_label(source: SourceKind) -> &'static str {
    match source {
        SourceKind::GitHub => "Git",
        SourceKind::Tukui => "TukUI",
        SourceKind::WowInterface => "WoWI",
        SourceKind::Wago => "Wago",
        SourceKind::Manual => "Manual",
    }
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
        DetailMode::Config => "Config",
        DetailMode::Backup => "Backup",
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

fn source_badge_color(source: SourceKind) -> Color {
    match source {
        SourceKind::GitHub => Color::Cyan,
        SourceKind::Tukui => Color::Yellow,
        SourceKind::WowInterface => Color::Magenta,
        SourceKind::Wago => Color::Blue,
        SourceKind::Manual => Color::DarkGray,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use ratatui::layout::Rect;
    use ratatui::widgets::TableState;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{
        AddonScanOutcome, App, AppMessage, AppRuntime, AppTaskEvent, BackupPaneState, ConfigField,
        ConfigPaneState, DashboardChildConnector, DashboardDeleteOutcome, DashboardJobKind,
        DashboardJobUiState, DashboardRow, DashboardSortColumn, DashboardState,
        DashboardUpdateOutcome, DetailMode, FooterHintId, FooterKeyPulse, InspectOverlayState,
        InspectResolvedTarget, InspectSection, InstallPaneState, MotionState, OverlayKind,
        PendingWagoInstallRequest, ScanState, SearchInstallState, SearchPaneState,
        SearchPresentationMode, ShellMode, ShellUiState, UiTheme, WagoInstallConfirmation,
        WagoInstallOutcome, WagoInstallSource, WagoInstallTaskOutcome, WagoSearchOutcome,
        child_row_detail_prefix, child_row_prefix, dashboard_item_version_label,
        dashboard_item_version_line, format_download_count, summarize_owned_folders,
        truncate_middle_text, visible_search_result_window,
    };
    use crate::action::AppAction;
    use crate::backup::{BackupEntry, BackupRunOutcome};
    use crate::drift::{DriftReport, OwnedChildDrift};
    use crate::event::TerminalEvent;
    use crate::onboarding::{
        FoundAction, FoundState, OnboardingPhase, OnboardingState, OnboardingStep,
    };
    use crate::update::LiveUpdateSummary;
    use crate::wago::{WagoInstallInspection, WagoInstallSummary, WagoSearchResult};
    use lemonup_core::{
        AddonKind, AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE,
        OwnedFolder, ScanSummary, SourceKind, StateDatabase,
    };

    fn app_for_tests(shell_mode: ShellMode) -> App {
        let (task_events_tx, task_events_rx) = mpsc::unbounded_channel();
        let config = AppConfig::new_unconfigured();
        let mut first = AddonRecord::new("First", "First", SourceKind::Manual);
        first.kind = AddonKind::Addon;
        first.version = Some("1.0.0".to_string());

        let mut second = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        second.kind = AddonKind::Addon;
        second.version = Some("2.0.0".to_string());
        second.author = Some("Author".to_string());
        second.git_commit = Some("abcdef".to_string());
        second.required_deps = vec!["Ace3".to_string()];
        second.embedded_libs = vec!["LibStub".to_string()];
        second.set_managed_owned_folders(vec![lemonup_core::OwnedFolder {
            name: "Second_Config".to_string(),
        }]);

        let mut third = AddonRecord::new("Third", "Third", SourceKind::Wago);
        third.kind = AddonKind::Library;
        third.interface = Some("110005".to_string());
        third.optional_deps = vec!["Optional".to_string()];

        App {
            shell_mode,
            quit_requested: false,
            status_line: format!("profile {} | q quit", DEFAULT_PROFILE),
            ui_theme: UiTheme::default(),
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
            backup_dir: std::env::temp_dir().join("lemonup-test-backups"),
            runtime: AppRuntime::new(DEFAULT_PROFILE.to_string(), None, None),
            effective_addon_dir: None,
            scan_state: ScanState::Idle,
            dashboard: DashboardState::from_addons(vec![first, second, third]),
            install_pane: InstallPaneState::default(),
            search_pane: SearchPaneState::default(),
            pending_wago_install_confirmation: None,
            wago_install_in_progress: false,
            undo_delete: None,
            undo_delete_in_progress: false,
            last_dashboard_list_area: None,
            onboarding: OnboardingState::new(),
            task_events_tx,
            task_events_rx,
            active_onboarding_cancel: None,
            inspect_overlay: InspectOverlayState::default(),
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
            vec![AppMessage::DashboardToggleSort(
                DashboardSortColumn::Version
            )]
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
        app.dashboard.list_state.select(Some(1));
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Second")
        );

        for action in app.update(AppMessage::DashboardToggleSort(DashboardSortColumn::Name)) {
            app.apply(action);
        }
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Second")
        );
        assert_eq!(
            app.dashboard.rows.first().map(|row| row.folder.as_str()),
            Some("Third")
        );

        for action in app.update(AppMessage::DashboardToggleSort(DashboardSortColumn::Name)) {
            app.apply(action);
        }
        assert_eq!(
            app.dashboard.selected_row().map(|row| row.folder.as_str()),
            Some("Second")
        );
        assert_eq!(
            app.dashboard.rows.first().map(|row| row.folder.as_str()),
            Some("First")
        );
    }

    #[test]
    fn mouse_wheel_moves_dashboard_selection_one_row() {
        let app = app_for_tests(ShellMode::Dashboard);

        let down = app.messages_for_event(TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        let up = app.messages_for_event(TerminalEvent::Mouse(MouseEvent {
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

        let messages = app.messages_for_event(TerminalEvent::Mouse(MouseEvent {
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

        let messages = app.messages_for_event(TerminalEvent::Mouse(MouseEvent {
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

        let actions = app.update(AppMessage::DashboardPointerSelect { column: 2, row: 3 });

        assert_eq!(
            actions,
            vec![
                AppAction::SetDashboardSelection(Some(2)),
                AppAction::SetStatus(
                    app.dashboard_status_for(app.dashboard.detail_mode, "selection moved",)
                ),
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

        let actions = app.update(AppMessage::DashboardPointerSelect { column: 2, row: 4 });

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
        let mut update_mode = app_for_tests(ShellMode::Dashboard);
        update_mode.dashboard.detail_mode = DetailMode::Update;
        assert_eq!(
            update_mode.messages_for_key(KeyEvent::from(KeyCode::Char('r'))),
            vec![AppMessage::DashboardRunUpdateSelected]
        );
        assert_eq!(
            update_mode.messages_for_key(KeyEvent::from(KeyCode::Char('v'))),
            vec![AppMessage::DashboardSelectRefreshableUpdates]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char(']'))),
            vec![AppMessage::DashboardExpandAllRelationships]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('['))),
            vec![AppMessage::DashboardCollapseAllRelationships]
        );
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
        app.status_line = "terminal resized to 132x45 | profile manual-smoke".to_string();

        assert_eq!(app.footer_status_text(), "");
    }

    #[test]
    fn dashboard_event_line_stays_quiet_for_selection_only() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::ToggleDashboardSelection);

        assert!(app.dashboard_event_message().is_none());
    }

    #[test]
    fn dashboard_event_line_uses_humanized_status_summary() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(
            "scan complete | 6 addons synced, 0 removed".to_string(),
        ));

        let (text, kind) = app.dashboard_event_message().expect("event line");

        assert_eq!(text, "scan complete · 6 addons synced, 0 removed");
        assert_eq!(kind, super::DashboardEventKind::Info);
    }

    #[test]
    fn dashboard_toast_persists_across_non_toast_status_updates() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(
            app.dashboard_status_for(DetailMode::Overview, "Updated BigWigs"),
        ));
        let initial = app.dashboard_event_message().expect("toast");

        app.apply(AppAction::SetStatus(
            app.dashboard_status_for(DetailMode::Overview, "selection moved"),
        ));

        assert_eq!(app.dashboard_event_message(), Some(initial));
    }

    #[test]
    fn dashboard_toast_expires_after_duration() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.apply(AppAction::SetStatus(
            app.dashboard_status_for(DetailMode::Overview, "Updated BigWigs"),
        ));
        assert!(app.dashboard_event_message().is_some());

        if let Some(toast) = app.shell_ui.dashboard_toast.as_mut() {
            toast.expires_at = super::Instant::now() - super::StdDuration::from_millis(1);
        }
        app.apply(AppAction::AdvanceMotionTick);

        assert!(app.dashboard_event_message().is_none());
    }

    #[test]
    fn footer_command_lines_split_on_narrow_width() {
        let app = app_for_tests(ShellMode::Dashboard);

        let wide = app.footer_command_lines();
        let narrow = app.footer_command_lines();

        assert_eq!(wide.len(), 2);
        assert_eq!(narrow.len(), 2);
        assert!(wide[0].iter().any(|hint| hint.id == FooterHintId::Check));
        assert!(wide[1].iter().any(|hint| hint.id == FooterHintId::Sort));
        assert!(narrow[0].iter().any(|hint| hint.id == FooterHintId::Check));
        assert!(narrow[1].iter().any(|hint| hint.id == FooterHintId::Sort));
    }

    #[test]
    fn recognized_key_sets_matching_footer_pulse() {
        let app = app_for_tests(ShellMode::Dashboard);
        let event = TerminalEvent::Key(KeyEvent::from(KeyCode::Char('c')));
        let messages = app.messages_for_event(event);

        let pulse = app.footer_pulse_for_event(event, &messages);

        assert_eq!(
            pulse,
            Some(FooterKeyPulse {
                hint: FooterHintId::Check,
                expires_at_tick: app.shell_ui.motion.tick_count + 2,
            })
        );
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
    fn inspect_overlay_closes_on_escape_and_does_not_trap_quit() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Esc)),
            vec![AppMessage::DashboardCloseOverlay]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('q'))),
            vec![AppMessage::QuitRequested]
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
    }

    #[test]
    fn inspect_overlay_keys_route_jk_to_scroll() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.shell_ui.overlay.active = Some(OverlayKind::Inspect);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('j'))),
            vec![AppMessage::InspectScrollDown]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('k'))),
            vec![AppMessage::InspectScrollUp]
        );
    }

    #[test]
    fn inspect_on_child_row_resolves_to_parent_with_child_context() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));

        let resolved = app.inspect_resolved_target().expect("inspect target");

        assert_eq!(resolved.item.folder, "Second");
        assert_eq!(resolved.opened_from_child, Some("Second_Config"));
    }

    #[test]
    fn inspect_summary_hides_empty_author_placeholder_and_internal_noise() {
        let app = app_for_tests(ShellMode::Dashboard);
        let target = InspectResolvedTarget {
            item: &app.dashboard.items[2],
            opened_from_child: None,
        };

        let summary = app
            .inspect_summary_lines(target)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(summary.contains("Version: unknown"));
        assert!(summary.contains("Folder: Third"));
        assert!(!summary.contains("Author:"));
        assert!(!summary.contains("selection"));
        assert!(!summary.contains("scan"));
        assert!(!summary.contains("drift"));
    }

    #[test]
    fn inspect_section_toggle_message_opens_and_closes_requested_section() {
        let mut app = app_for_tests(ShellMode::Dashboard);

        for action in app.update(AppMessage::InspectToggleSection(
            InspectSection::IncludedAddons,
        )) {
            app.apply(action);
        }
        assert!(app.inspect_overlay.is_open(InspectSection::IncludedAddons));

        for action in app.update(AppMessage::InspectToggleSection(
            InspectSection::IncludedAddons,
        )) {
            app.apply(action);
        }
        assert!(!app.inspect_overlay.is_open(InspectSection::IncludedAddons));
    }

    #[test]
    fn inspect_detail_sections_start_collapsed() {
        let app = app_for_tests(ShellMode::Dashboard);
        let target = app.inspect_resolved_target().expect("inspect target");

        let details = app
            .inspect_detail_lines(target)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            details,
            vec!["▸ r Included addons", "▸ d Dependencies", "▸ t Addon info"]
        );
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
        app.search_pane.query = "WeakAuras".to_string();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_search()),
                AppAction::StartWagoSearch {
                    query: "WeakAuras".to_string(),
                    api_key: "test-wago-key".to_string(),
                },
                AppAction::SetStatus(
                    app.dashboard_status_for(DetailMode::Search, "searching Wago for 'WeakAuras'")
                ),
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
        app.search_pane.query = "https://addons.wago.io/addons/VBNBxKx5".to_string();

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
                    allow_replace: false,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    DetailMode::Search,
                    "checking Wago addon 'https://addons.wago.io/addons/VBNBxKx5' for install",
                )),
            ]
        );
    }

    #[test]
    fn search_mode_submit_plain_lowercase_name_stays_search() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane.query = "details".to_string();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSearchPaneState(app.search_pane.start_search()),
                AppAction::StartWagoSearch {
                    query: "details".to_string(),
                    api_key: "test-wago-key".to_string(),
                },
                AppAction::SetStatus(
                    app.dashboard_status_for(DetailMode::Search, "searching Wago for 'details'")
                ),
            ]
        );
    }

    #[test]
    fn dashboard_search_keys_open_expected_ab_variants() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('/'))),
            vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::OpenSearch(SearchPresentationMode::ComposeFirst)]
        );
        assert!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('?')))
                .is_empty()
        );
    }

    #[test]
    fn open_search_preserves_shared_state_and_enters_editing() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.search_pane.query = "WeakAuras".to_string();
        app.search_pane.is_editing = false;
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
        assert!(app.search_pane.is_editing);
        assert_eq!(app.search_pane.query, "WeakAuras");
        assert_eq!(app.search_pane.results.len(), 1);
        assert_eq!(app.search_pane.selected_result, Some(0));
    }

    #[test]
    fn search_edit_mode_treats_quit_letter_as_input() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.search_pane = app.search_pane.begin_editing();

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
    fn search_mode_without_api_key_reports_blocked_status() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.wago_api_key = None;
        app.search_pane.query = "WeakAuras".to_string();

        let actions = app.update(AppMessage::SearchSubmit);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                DetailMode::Search,
                "Wago search unavailable: no API key configured",
            ))]
        );
    }

    #[test]
    fn search_install_selected_starts_install_flow_for_selected_result() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Search;
        app.effective_addon_dir = Some(PathBuf::from(
            "C:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.search_pane = SearchPaneState {
            query: "WeakAuras".to_string(),
            presentation_mode: SearchPresentationMode::ComposeFirst,
            is_editing: false,
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
                    allow_replace: false,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    DetailMode::Search,
                    "checking Wago result 'WeakAuras' for install",
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
        app.install_pane.input = "https://addons.wago.io/addons/VBNBxKx5".to_string();

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
                    allow_replace: false,
                },
                AppAction::SetStatus(app.dashboard_status_for(
                    DetailMode::Install,
                    "checking Wago package and preparing install",
                )),
            ]
        );
    }

    #[test]
    fn config_mode_navigation_and_edit_keys_are_routed_to_config_pane() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('j'))),
            vec![AppMessage::ConfigSelectionNext]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('e'))),
            vec![AppMessage::ConfigBeginEditing]
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
    fn config_save_updates_runtime_wago_api_key_and_marks_config_present() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.config_present = false;
        app.wago_api_key = None;
        app.config_store = ConfigStore::new(config_path.clone());
        app.dashboard.detail_mode = DetailMode::Config;
        app.config_pane.draft.wago_api_key = Some("new-wago-key".to_string());

        let actions = app.update(AppMessage::ConfigSave);
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
    fn config_onboarding_field_reenters_location_finder_with_current_path() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Config;
        app.shell_ui.overlay.active = Some(OverlayKind::Config);
        app.effective_addon_dir = Some(PathBuf::from(
            "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.config_pane.selected_field = ConfigField::ALL
            .iter()
            .position(|field| *field == ConfigField::RunOnboardingAgain)
            .expect("onboarding field index");

        let actions = app.update(AppMessage::ConfigToggleSelected);
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
            AppAction::SetStatus(
                app.dashboard_status_for(DetailMode::Search, "Wago search complete: 1 result")
            )
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
    }

    #[test]
    fn dashboard_selection_wraps_forward_and_backward() {
        let app = app_for_tests(ShellMode::Dashboard);

        let forward = app.update(AppMessage::DashboardSelectionNext);
        assert_eq!(
            forward,
            vec![
                AppAction::SetDashboardSelection(Some(1)),
                AppAction::SetStatus(
                    app.dashboard_status_for(app.dashboard.detail_mode, "selection moved")
                ),
            ]
        );

        let mut wrapped = app_for_tests(ShellMode::Dashboard);
        wrapped.dashboard.list_state.select(Some(0));
        let backward = wrapped.update(AppMessage::DashboardSelectionPrevious);
        assert_eq!(
            backward,
            vec![
                AppAction::SetDashboardSelection(Some(2)),
                AppAction::SetStatus(
                    wrapped.dashboard_status_for(wrapped.dashboard.detail_mode, "selection moved")
                ),
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
        app.dashboard.list_state.select(Some(2));

        app.apply(AppAction::ExpandAllDashboardRelationships);

        assert_eq!(
            app.dashboard
                .selected_item()
                .map(|item| item.folder.as_str()),
            Some("Third")
        );
        assert_eq!(app.dashboard.list_state.selected(), Some(3));
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

        assert_eq!(child_row_prefix(&mid), "  ├─ ");
        assert_eq!(child_row_prefix(&last), "  └─ ");
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
    fn requesting_delete_without_selection_sets_status_only() {
        let app = app_for_tests(ShellMode::Dashboard);

        let actions = app.update(AppMessage::DashboardRequestDelete);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                app.dashboard.detail_mode,
                "select one or more parent addons before deleting",
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
                    app.dashboard.detail_mode,
                    "delete 1 selected addon? press y to confirm, n to cancel",
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
    fn update_selected_requires_parent_selection() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Overview;
        app.dashboard.list_state.select(Some(1));
        app.apply(AppAction::ToggleDashboardExpanded);
        app.dashboard.list_state.select(Some(2));

        let actions = app.update(AppMessage::DashboardRunUpdateSelected);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                DetailMode::Overview,
                "focus a parent row or select one or more parent addons first before applying updates",
            ))]
        );
    }

    #[test]
    fn selecting_refreshable_updates_targets_managed_tracked_parents_only() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Update;

        let actions = app.update(AppMessage::DashboardSelectRefreshableUpdates);

        assert_eq!(
            actions,
            vec![
                AppAction::SetSelectedDashboardParents(vec!["Second".to_string()]),
                AppAction::SetStatus(app.dashboard_status_for(
                    DetailMode::Update,
                    "selected 1 updateable tracked addon",
                )),
            ]
        );
    }

    #[test]
    fn selecting_refreshable_updates_reports_when_none_are_available() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.detail_mode = DetailMode::Update;
        app.apply(AppAction::ReplaceDashboardAddons(vec![
            AddonRecord::new("First", "First", SourceKind::Manual),
            AddonRecord::new("Third", "Third", SourceKind::Wago),
        ]));

        let actions = app.update(AppMessage::DashboardSelectRefreshableUpdates);

        assert_eq!(
            actions,
            vec![AppAction::SetStatus(app.dashboard_status_for(
                DetailMode::Update,
                "no updateable tracked parent addons are available in the current list",
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
                AppAction::SetStatus(
                    app.dashboard_status_for(DetailMode::Overview, "updating 1 addon",)
                ),
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
                AppAction::SetStatus(
                    app.dashboard_status_for(DetailMode::Overview, "checking 1 addon for updates",)
                ),
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

        assert_eq!(actions.len(), 6);
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
            },
            addons: vec![AddonRecord::new("Second", "Second", SourceKind::GitHub)],
            drift_report: DriftReport::empty(),
        };

        let refresh = LiveUpdateSummary {
            target_addons: 1,
            updated_addons: 1,
            up_to_date: 0,
            skipped_manual: 0,
            skipped_unmanaged: 0,
            skipped_unsupported: 0,
            errors: 0,
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardUpdateFinished(Ok(DashboardUpdateOutcome {
                summary: refresh,
                updated_addon_name: Some("Second".to_string()),
                sync: sync.clone(),
            })),
        ));

        assert_eq!(actions.len(), 7);
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
        assert!(matches!(actions[6], AppAction::SetStatus(_)));
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
            skipped_unmanaged: 0,
            skipped_unsupported: 0,
            errors: 0,
        };

        let actions = app.update(AppMessage::BackgroundTask(
            AppTaskEvent::DashboardUpdateFinished(Ok(DashboardUpdateOutcome {
                summary: refresh,
                updated_addon_name: None,
                sync,
            })),
        ));

        assert_eq!(
            actions[6],
            AppAction::SetStatus(
                app.dashboard_status_for(DetailMode::Update, "No changes · 1 manual skipped",)
            )
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
                skipped_unmanaged: 0,
                skipped_unsupported: 0,
                errors: 0,
            },
            updated_addon_name: Some("WeakAuras".to_string()),
            sync: AddonScanOutcome {
                path: PathBuf::from("D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"),
                summary: ScanSummary {
                    scanned_addons: 1,
                    upserted_addons: 1,
                    removed_addons: 0,
                },
                addons: vec![],
                drift_report: DriftReport::empty(),
            },
        };

        assert_eq!(
            super::dashboard_update_status_message(&outcome),
            "Updated WeakAuras"
        );
    }

    #[test]
    fn dashboard_check_status_humanizes_recently_checked_single_target() {
        assert_eq!(
            super::dashboard_check_status_message(&["BigWigs".to_string()], 1, 0, 1, 0),
            "BigWigs was checked recently"
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
    fn active_update_job_prefixes_spinner_into_version_cell() {
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
        let view = app.dashboard_table_row(row);
        let rendered = view.version.to_string();
        assert!(rendered.contains("⠋"));
        assert!(rendered.contains("abcdef"));
    }

    #[test]
    fn replacing_addons_preserves_selected_row_and_scroll_offset() {
        let mut app = app_for_tests(ShellMode::Dashboard);
        app.dashboard.list_state = TableState::default().with_offset(2).with_selected(Some(2));

        let updated = vec![
            AddonRecord::new("First", "First", SourceKind::Manual),
            AddonRecord::new("Second", "Second", SourceKind::GitHub),
            AddonRecord::new("Third", "Third", SourceKind::Wago),
        ];

        app.apply(AppAction::ReplaceDashboardAddons(updated));

        assert_eq!(app.dashboard.list_state.selected(), Some(2));
        assert_eq!(app.dashboard.list_state.offset(), 2);
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

    #[test]
    fn version_line_renders_plain_version_for_manual_addons() {
        let addon = AddonRecord::new("ManualSmokeAddon", "ManualSmokeAddon", SourceKind::Manual);
        let dashboard = DashboardState::from_addons(vec![addon]);

        let item = dashboard.items.first().expect("dashboard item");
        let rendered =
            dashboard_item_version_line(item, None, MotionState::default(), UiTheme::default());
        let text = rendered
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
            .trim()
            .to_string();

        assert_eq!(text, "unknown");
    }

    #[test]
    fn version_line_does_not_mark_single_folder_managed_wago_as_unmanaged() {
        let mut addon = AddonRecord::new("WeakAuras", "WeakAuras", SourceKind::Wago);
        addon.version = Some("5.21.1".to_string());
        addon.remote_version = Some("5.21.1".to_string());
        addon.set_managed_owned_folders(Vec::new());
        let dashboard = DashboardState::from_addons(vec![addon]);

        let item = dashboard.items.first().expect("dashboard item");
        let rendered =
            dashboard_item_version_line(item, None, MotionState::default(), UiTheme::default());
        let text = rendered
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(text.trim(), "5.21.1");
        assert!(!text.contains("unmanaged"));
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
                AppAction::SetStatus(app.with_base_status("checking common install locations")),
                AppAction::StartOnboardingQuickCheck,
            ]
        );
    }

    #[test]
    fn found_confirmation_uses_selected_action() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.input =
            "C:\\Games\\World of Warcraft\\_retail_\\Interface\\AddOns".to_string();
        app.onboarding.step = OnboardingStep::Directory;
        app.onboarding.phase = OnboardingPhase::Found(FoundState {
            path: app.onboarding.input.clone(),
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
                    app.with_base_status("directory confirmed | step 3/5 Wago setup")
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
            Some(PathBuf::from(
                "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
            )),
        );

        let path = PathBuf::from("D:\\World of Warcraft\\_retail_\\Interface\\AddOns");
        let actions = app.update(AppMessage::BackgroundTask(AppTaskEvent::Onboarding(
            crate::onboarding::OnboardingTaskEvent::QuickCheckFinished(Some(path.clone())),
        )));

        assert_eq!(
            actions,
            vec![
                AppAction::SetOnboardingState(app.onboarding_found_state(path.clone())),
                AppAction::SetStatus(app.found_status_for_path(&path)),
            ]
        );
        assert_eq!(
            app.onboarding_found_state(path).selected_found_action(),
            FoundAction::EnterDifferentPath
        );
    }

    #[test]
    fn save_refuses_guarded_path_for_non_default_profile() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.runtime = AppRuntime::new(
            "dev".to_string(),
            None,
            Some(PathBuf::from(
                "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
            )),
        );

        let mut config = app.config_pane.draft.clone();
        config.addon_dir = Some(PathBuf::from(
            "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ));
        app.apply(AppAction::SaveOnboardingConfig(config));

        assert_eq!(app.shell_mode, ShellMode::Onboarding);
        assert!(matches!(app.onboarding.phase, OnboardingPhase::Error(_)));
        assert!(app.status_line.contains("refused to save default target"));
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
            AppRuntime::new("dev".to_string(), Some(PathBuf::from("D:\\bad-path")), None),
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
            AppRuntime::new("dev".to_string(), Some(guarded.clone()), Some(guarded)),
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
        assert!(app.status_line.contains("scan complete"));
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
            AppRuntime::new("dev".to_string(), None, Some(PathBuf::from("D:\\guarded"))),
        )
        .expect("bootstrap app");

        assert_eq!(app.shell_mode, ShellMode::Dashboard);
        assert_eq!(app.effective_addon_dir, Some(addons_dir));
        assert_eq!(app.scan_state, ScanState::Pending);
    }
}
