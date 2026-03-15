use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};

use lemonup_core::{
    AddonKind, AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE,
    GameFlavor, ScanSummary, SourceKind, StateDatabase, detect_known_addons_path, paths_match,
    scan_addons_dir, search_for_wow, validate_addons_path,
};
use tokio::sync::mpsc;

use crate::action::AppAction;
use crate::event::{EventHandler, TerminalEvent};
use crate::onboarding::{FoundAction, OnboardingPhase, OnboardingState, OnboardingTaskEvent};
use crate::tui::Backend;

const DASHBOARD_COMMANDS: &str = "q quit | j/k list | space select | a all | esc clear | x delete | y confirm | n cancel | enter tree | h collapse | o overview | i install | s search | u update | c config | b backup";

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppMessage {
    Tick,
    QuitRequested,
    TerminalResized { width: u16, height: u16 },
    DashboardSelectionNext,
    DashboardSelectionPrevious,
    DashboardRequestDelete,
    DashboardConfirmDelete,
    DashboardCancelPendingDelete,
    DashboardToggleSelected,
    DashboardSelectAll,
    DashboardClearSelection,
    DashboardToggleExpanded,
    DashboardCollapseExpanded,
    SetDetailMode(DetailMode),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AddonScanOutcome {
    path: PathBuf,
    summary: ScanSummary,
    addons: Vec<AddonRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardDeleteOutcome {
    deleted_parents: usize,
    deleted_folders: usize,
    addons: Vec<AddonRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardItem {
    name: String,
    folder: String,
    owned_folders: Vec<String>,
    source: SourceKind,
    kind: AddonKind,
    version: Option<String>,
    author: Option<String>,
    interface: Option<String>,
    git_commit: Option<String>,
    required_deps: Vec<String>,
    optional_deps: Vec<String>,
    embedded_libs: Vec<String>,
    owned_folder_count: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardRow {
    key: DashboardRowKey,
    name: String,
    folder: String,
    kind: DashboardRowKind,
    expandable: bool,
    expanded: bool,
}

struct DashboardState {
    items: Vec<DashboardItem>,
    rows: Vec<DashboardRow>,
    list_state: ListState,
    detail_mode: DetailMode,
    expanded_folders: HashSet<String>,
    selected_parents: HashSet<String>,
    pending_delete_folders: Option<Vec<String>>,
}

impl DashboardState {
    fn from_addons(addons: Vec<AddonRecord>) -> Self {
        let mut items = addons
            .into_iter()
            .map(|addon| {
                let owned_folder_count = addon.owned_folders.len();
                DashboardItem {
                    name: addon.name,
                    folder: addon.folder,
                    owned_folders: addon
                        .owned_folders
                        .into_iter()
                        .map(|owned_folder| owned_folder.name)
                        .collect(),
                    source: addon.source,
                    kind: addon.kind,
                    version: addon.version,
                    author: addon.author,
                    interface: addon.interface,
                    git_commit: addon.git_commit,
                    required_deps: addon.required_deps,
                    optional_deps: addon.optional_deps,
                    embedded_libs: addon.embedded_libs,
                    owned_folder_count,
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| {
                    left.folder
                        .to_ascii_lowercase()
                        .cmp(&right.folder.to_ascii_lowercase())
                })
        });

        let mut state = Self {
            items,
            rows: Vec::new(),
            list_state: ListState::default(),
            detail_mode: DetailMode::Overview,
            expanded_folders: HashSet::new(),
            selected_parents: HashSet::new(),
            pending_delete_folders: None,
        };
        state.rebuild_rows();
        if !state.rows.is_empty() {
            state.list_state.select(Some(0));
        }
        state
    }

    fn replace_addons(&mut self, addons: Vec<AddonRecord>) {
        let selected_key = self.selected_row().map(|row| row.key.clone());
        let previous_expanded = self.expanded_folders.clone();
        let previous_selected = self.selected_parents.clone();
        let next = DashboardState::from_addons(addons);

        self.items = next.items;
        self.rows = next.rows;
        self.list_state = next.list_state;
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
        self.rebuild_rows();

        let mut selected = selected_key
            .as_ref()
            .and_then(|key| self.rows.iter().position(|row| row.key == *key));
        if selected.is_none() && !self.rows.is_empty() {
            selected = Some(0);
        }
        self.list_state.select(selected);
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

        if !self.selected_parents.remove(&parent_folder) {
            self.selected_parents.insert(parent_folder);
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
            });

            if expanded {
                for child_folder in &item.owned_folders {
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
            .or_else(|| if self.rows.is_empty() { None } else { Some(0) });
        self.list_state.select(selection);
    }
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
    config_present: bool,
    config_store: ConfigStore,
    state_db_file: PathBuf,
    runtime: AppRuntime,
    effective_addon_dir: Option<PathBuf>,
    scan_state: ScanState,
    dashboard: DashboardState,
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
        let database = StateDatabase::open(state_db_file.clone())?;
        let addons = database.list_addons()?;
        let config_present = matches!(config_state, ConfigLoad::Loaded(_));
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
            .map(|path| OnboardingState::with_input(path.display().to_string()))
            .unwrap_or_else(OnboardingState::new);

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
            config_present,
            config_store,
            state_db_file,
            runtime,
            effective_addon_dir,
            scan_state,
            dashboard: DashboardState::from_addons(addons),
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
            terminal.draw(|frame| self.draw(frame))?;

            let Some(event) = events.next().await else {
                break;
            };

            let messages = self.messages_for_event(event);
            for message in messages {
                let actions = self.update(message);
                for action in actions {
                    self.apply(action);
                }
            }
        }

        Ok(())
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

    fn dashboard_messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        if self.dashboard.pending_delete_folders().is_some() {
            return match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                KeyCode::Char('y') => vec![AppMessage::DashboardConfirmDelete],
                KeyCode::Char('n') | KeyCode::Esc => vec![AppMessage::DashboardCancelPendingDelete],
                _ => vec![],
            };
        }

        match key.code {
            KeyCode::Char('q') => vec![AppMessage::QuitRequested],
            KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::DashboardSelectionNext],
            KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::DashboardSelectionPrevious],
            KeyCode::Char('x') => vec![AppMessage::DashboardRequestDelete],
            KeyCode::Char(' ') => vec![AppMessage::DashboardToggleSelected],
            KeyCode::Char('a') => vec![AppMessage::DashboardSelectAll],
            KeyCode::Esc => vec![AppMessage::DashboardClearSelection],
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                vec![AppMessage::DashboardToggleExpanded]
            }
            KeyCode::Left | KeyCode::Char('h') => vec![AppMessage::DashboardCollapseExpanded],
            KeyCode::Char('o') => vec![AppMessage::SetDetailMode(DetailMode::Overview)],
            KeyCode::Char('i') => vec![AppMessage::SetDetailMode(DetailMode::Install)],
            KeyCode::Char('s') | KeyCode::Char('/') => {
                vec![AppMessage::SetDetailMode(DetailMode::Search)]
            }
            KeyCode::Char('u') => vec![AppMessage::SetDetailMode(DetailMode::Update)],
            KeyCode::Char('c') => vec![AppMessage::SetDetailMode(DetailMode::Config)],
            KeyCode::Char('b') => vec![AppMessage::SetDetailMode(DetailMode::Backup)],
            _ => vec![],
        }
    }

    fn onboarding_messages_for_key(&self, key: KeyEvent) -> Vec<AppMessage> {
        match &self.onboarding.phase {
            OnboardingPhase::Bootstrapping | OnboardingPhase::QuickChecking => match key.code {
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingPhase::DeepScanning(_) => match key.code {
                KeyCode::Esc => vec![AppMessage::OnboardingCancel],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingPhase::Found(_) => match key.code {
                KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::OnboardingFoundActionNext],
                KeyCode::Up | KeyCode::Char('k') => {
                    vec![AppMessage::OnboardingFoundActionPrevious]
                }
                KeyCode::Enter => vec![AppMessage::OnboardingFoundConfirm],
                KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                _ => vec![],
            },
            OnboardingPhase::Ready | OnboardingPhase::Error(_) | OnboardingPhase::Cancelled => {
                if self.onboarding.is_editing {
                    return match key.code {
                        KeyCode::Enter => vec![AppMessage::OnboardingValidateInput],
                        KeyCode::Esc => vec![AppMessage::OnboardingStopEditing],
                        KeyCode::Backspace => vec![AppMessage::OnboardingBackspace],
                        KeyCode::Char('d') => vec![AppMessage::OnboardingDeepScan],
                        KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                        KeyCode::Char(character) => {
                            vec![AppMessage::OnboardingInputChar(character)]
                        }
                        _ => vec![],
                    };
                }

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
                    KeyCode::Esc => vec![AppMessage::OnboardingCancel],
                    KeyCode::Char('q') => vec![AppMessage::QuitRequested],
                    _ => vec![],
                }
            }
        }
    }

    fn update(&self, message: AppMessage) -> Vec<AppAction> {
        match message {
            AppMessage::Tick => {
                if self.shell_mode == ShellMode::Onboarding
                    && matches!(self.onboarding.phase, OnboardingPhase::Bootstrapping)
                {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.begin_quick_check()),
                        AppAction::SetStatus(
                            self.with_base_status("checking common install locations"),
                        ),
                        AppAction::StartOnboardingQuickCheck,
                    ]
                } else if self.shell_mode == ShellMode::Dashboard
                    && matches!(self.scan_state, ScanState::Pending)
                {
                    self.effective_addon_dir
                        .clone()
                        .map(AppAction::StartAddonScan)
                        .into_iter()
                        .collect()
                } else {
                    vec![AppAction::None]
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
                    AppAction::StartDashboardDelete { addon_dir, folders },
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
                    &format!("{} panel selected", detail_mode_label(detail_mode)),
                )),
            ],
            AppMessage::OnboardingBeginEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.begin_editing()),
                AppAction::SetStatus(self.with_base_status(
                    "editing path | enter validate | d deep scan | esc stop editing",
                )),
            ],
            AppMessage::OnboardingStopEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.stop_editing()),
                AppAction::SetStatus(self.location_finder_status()),
            ],
            AppMessage::OnboardingInputChar(character) => {
                vec![AppAction::SetOnboardingState(
                    self.onboarding.insert_char(character),
                )]
            }
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
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.clear_status_to_ready()),
                        AppAction::SetStatus(self.location_finder_status()),
                    ]
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
                FoundAction::UseThisPath => vec![AppAction::SaveAddonDir(PathBuf::from(
                    self.onboarding.input.trim(),
                ))],
                FoundAction::ScanAnotherLocation => vec![
                    AppAction::SetOnboardingState(self.onboarding.ready()),
                    AppAction::SetStatus(self.with_base_status(
                        "choose another root | enter validate | d deep scan | e edit path",
                    )),
                ],
                FoundAction::EditPathManually => vec![
                    AppAction::SetOnboardingState(self.onboarding.begin_editing()),
                    AppAction::SetStatus(self.with_base_status(
                        "editing found path | enter validate | d deep scan | esc stop editing",
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
                    AppAction::CompleteAddonScan {
                        path: outcome.path,
                        summary: outcome.summary,
                    },
                ],
                AppTaskEvent::AddonScanFinished(Err(error)) => {
                    vec![AppAction::FailAddonScan(error)]
                }
                AppTaskEvent::DashboardDeleteFinished(Ok(outcome)) => vec![
                    AppAction::ReplaceDashboardAddons(outcome.addons),
                    AppAction::SetPendingDelete(None),
                    AppAction::SetStatus(self.dashboard_status_for(
                        self.dashboard.detail_mode,
                        &format!(
                            "deleted {} addon{}, removed {} folder{}",
                            outcome.deleted_parents,
                            plural_suffix(outcome.deleted_parents),
                            outcome.deleted_folders,
                            plural_suffix(outcome.deleted_folders)
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
            },
        }
    }

    fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::None => {}
            AppAction::Quit => self.quit_requested = true,
            AppAction::SetStatus(status) => self.status_line = status,
            AppAction::SetDashboardSelection(selection) => {
                self.dashboard.list_state.select(selection)
            }
            AppAction::SetPendingDelete(folders) => {
                self.dashboard.set_pending_delete_folders(folders);
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
            AppAction::CollapseDashboardExpanded => {
                if self.dashboard.collapse_selected_expanded() {
                    self.status_line =
                        self.dashboard_status_for(self.dashboard.detail_mode, "tree state updated");
                }
            }
            AppAction::SetDetailMode(detail_mode) => self.dashboard.detail_mode = detail_mode,
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
            AppAction::SaveAddonDir(path) => {
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

                let mut config = match self.config_store.load() {
                    Ok(ConfigLoad::Loaded(config)) => config,
                    _ => AppConfig::new_unconfigured(),
                };
                config.addon_dir = Some(path.clone());

                match self.config_store.write_new_config(&config) {
                    Ok(()) => {
                        self.config_present = true;
                        self.effective_addon_dir = Some(path.clone());
                        self.shell_mode = ShellMode::Dashboard;
                        if tokio::runtime::Handle::try_current().is_ok() {
                            self.apply(AppAction::StartAddonScan(path));
                        } else {
                            self.scan_state = ScanState::Pending;
                            self.status_line = self.dashboard_status_for(
                                self.dashboard.detail_mode,
                                &format!("saved addon directory {} | scan queued", path.display()),
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
                        let scanned = scan_addons_dir(&path, GameFlavor::Retail)?;
                        let mut database = StateDatabase::open(state_db_file)?;
                        let summary = database.reconcile_scanned_addons(&scanned)?;
                        let addons = database.list_addons()?;
                        Ok::<AddonScanOutcome, lemonup_core::LemonupError>(AddonScanOutcome {
                            path,
                            summary,
                            addons,
                        })
                    })
                    .await
                    .map_err(|join_error| join_error.to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()));

                    let _ = sender.send(AppTaskEvent::AddonScanFinished(result));
                });
            }
            AppAction::StartDashboardDelete { addon_dir, folders } => {
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        delete_selected_addons(&state_db_file, &addon_dir, &folders)
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(AppTaskEvent::DashboardDeleteFinished(result));
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
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Min(12),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let header = Paragraph::new(self.header_lines())
            .block(Block::default().borders(Borders::ALL).title("Header"));

        let footer = Paragraph::new(self.status_line.clone())
            .block(Block::default().borders(Borders::ALL).title("Status"));

        frame.render_widget(header, layout[0]);
        match self.shell_mode {
            ShellMode::Onboarding => frame.render_widget(self.onboarding_body(), layout[1]),
            ShellMode::Dashboard => self.render_dashboard(frame, layout[1]),
        }
        frame.render_widget(footer, layout[2]);
    }

    fn header_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            Span::styled(
                "LemonUp v2",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  single-surface shell"),
        ])];
        lines.push(Line::from(format!(
            "Profile: {}",
            self.runtime.profile_name
        )));
        lines.push(Line::from(format!(
            "Target: {}",
            self.rendered_target_path()
        )));
        lines.push(Line::from(format!("Scan: {}", self.scan_status_label())));
        lines.push(Line::from(format!(
            "Surface: {}",
            match self.shell_mode {
                ShellMode::Onboarding => "setup takeover",
                ShellMode::Dashboard => detail_mode_label(self.dashboard.detail_mode),
            }
        )));
        if let Some(warning) = self.profile_warning() {
            lines.push(Line::from(Span::styled(
                warning,
                Style::default().fg(Color::Red),
            )));
        }
        lines
    }

    fn onboarding_body(&self) -> Paragraph<'static> {
        let mut lines = vec![
            Line::from("Locate your World of Warcraft AddOns folder"),
            Line::from("This is an inline takeover inside the single-screen shell."),
            Line::from(format!("Current path: {}", self.onboarding.input)),
            Line::from(format!("Config present: {}", self.config_present)),
            Line::from(""),
        ];

        match &self.onboarding.phase {
            OnboardingPhase::Bootstrapping => {
                lines.push(Line::from("Preparing location finder..."));
            }
            OnboardingPhase::QuickChecking => {
                lines.push(Line::from("Checking common install locations..."));
                lines.push(Line::from(
                    "This runs automatically before deeper scanning.",
                ));
            }
            OnboardingPhase::Ready => {
                lines.push(Line::from("No WoW install selected yet."));
                lines.push(Line::from(
                    "j/k choose suggestion | enter validate | d deep scan | e edit",
                ));
                if let Some(warning) = self.profile_warning() {
                    lines.push(Line::from(Span::styled(
                        warning,
                        Style::default().fg(Color::Red),
                    )));
                }
                for (index, suggestion) in self.onboarding.suggestions.iter().enumerate() {
                    let prefix = if self.onboarding.selected_suggestion == Some(index) {
                        "›"
                    } else {
                        " "
                    };
                    lines.push(Line::from(format!("{prefix} {suggestion}")));
                }
            }
            OnboardingPhase::DeepScanning(progress) => {
                lines.push(Line::from(format!(
                    "Deep scanning... ({} directories checked)",
                    progress.dirs_scanned
                )));
                lines.push(Line::from(progress.current_path.clone()));
                lines.push(Line::from("Esc cancels immediately."));
            }
            OnboardingPhase::Found(found) => {
                lines.push(Line::from("Found a retail WoW AddOns folder."));
                lines.push(Line::from(found.path.clone()));
                lines.push(Line::from("Verified using install artifacts."));
                if self.is_guarded_path(Path::new(&found.path)) {
                    lines.push(Line::from(Span::styled(
                        "Warning: this matches the default profile target; use a sandbox path instead.",
                        Style::default().fg(Color::Red),
                    )));
                }
                lines.push(Line::from(""));

                for (index, label) in [
                    "Use this path",
                    "Scan another location",
                    "Edit path manually",
                ]
                .iter()
                .enumerate()
                {
                    let prefix = if found.selected_action == index {
                        "›"
                    } else {
                        " "
                    };
                    lines.push(Line::from(format!("{prefix} {label}")));
                }
            }
            OnboardingPhase::Error(message) => {
                lines.push(Line::from(format!("Validation or scan failed: {message}")));
                lines.push(Line::from(
                    "Enter validate | d deep scan | e edit | esc clear",
                ));
            }
            OnboardingPhase::Cancelled => {
                lines.push(Line::from("Deep scan cancelled."));
                lines.push(Line::from(
                    "Choose a different root, edit the path, or scan again.",
                ));
            }
        }

        if self.onboarding.is_editing {
            lines.push(Line::from(""));
            lines.push(Line::from("Editing mode active."));
            lines.push(Line::from(
                "Type a path, Backspace to delete, Enter validate, d deep scan, Esc stop editing.",
            ));
        }

        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Setup"))
    }

    fn render_dashboard(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let [list_area, detail_area] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .areas(area);

        self.render_dashboard_list(frame, list_area);
        frame.render_widget(self.detail_panel(), detail_area);
    }

    fn render_dashboard_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if self.dashboard.rows.is_empty() {
            let body = Paragraph::new(vec![
                Line::from("No scanned addons yet."),
                Line::from(format!("Scan state: {}", self.scan_status_label())),
                Line::from(
                    "If this is your first launch on this profile, wait for the background scan.",
                ),
            ])
            .block(Block::default().borders(Borders::ALL).title("Addons"));
            frame.render_widget(body, area);
            return;
        }

        let items = self
            .dashboard
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let prefix = if self.dashboard.list_state.selected() == Some(index) {
                    "› "
                } else {
                    "  "
                };
                let selection_marker = match &row.kind {
                    DashboardRowKind::Parent => {
                        if self.dashboard.is_parent_selected(&row.folder) {
                            "[x] "
                        } else {
                            "[ ] "
                        }
                    }
                    DashboardRowKind::OwnedChild { .. } => "",
                };
                let marker = match &row.kind {
                    DashboardRowKind::Parent if row.expandable && row.expanded => "v ",
                    DashboardRowKind::Parent if row.expandable => "> ",
                    DashboardRowKind::Parent => "  ",
                    DashboardRowKind::OwnedChild { .. } => "|- ",
                };
                ListItem::new(vec![
                    Line::from(format!("{prefix}{selection_marker}{marker}{}", row.name)),
                    Line::from(match &row.kind {
                        DashboardRowKind::Parent => {
                            let item = self
                                .dashboard
                                .items
                                .iter()
                                .find(|item| item.folder == row.folder)
                                .expect("row parent item exists");
                            let version = item.version.as_deref().unwrap_or("unknown");
                            let kind = addon_kind_label(item.kind);
                            format!("    {} | {} | {}", item.folder, version, kind)
                        }
                        DashboardRowKind::OwnedChild { parent_folder } => {
                            format!("    child of {} | folder {}", parent_folder, row.folder)
                        }
                    }),
                ])
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(format!(
                "Addons ({} selected)",
                self.dashboard.selected_parent_count()
            )))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(list, area, &mut self.dashboard.list_state);
    }

    fn detail_panel(&self) -> Paragraph<'static> {
        let mut lines = Vec::new();
        let selected = self.dashboard.selected_item();
        let selected_row = self.dashboard.selected_row();
        let selected_owned_child = self.dashboard.selected_owned_child_folder();

        lines.push(Line::from(format!(
            "Mode: {}",
            detail_mode_label(self.dashboard.detail_mode)
        )));
        lines.push(Line::from(""));

        if let Some(pending_delete_folders) = self.dashboard.pending_delete_folders() {
            lines.push(Line::from("Pending delete confirmation"));
            lines.push(Line::from(format!(
                "{} parent addon{} selected for deletion",
                pending_delete_folders.len(),
                plural_suffix(pending_delete_folders.len())
            )));
            lines.push(Line::from(format!(
                "Targets: {}",
                pending_delete_folders.join(", ")
            )));
            lines.push(Line::from("Press y to confirm or n/esc to cancel."));
            lines.push(Line::from(""));
        }

        match self.dashboard.detail_mode {
            DetailMode::Overview => {
                if let (Some(item), Some(child_folder)) = (selected, selected_owned_child) {
                    lines.push(Line::from("Relationship row: owned child"));
                    lines.push(Line::from(format!("Child folder: {child_folder}")));
                    lines.push(Line::from(format!("Parent addon: {}", item.name)));
                    lines.push(Line::from(format!("Parent folder: {}", item.folder)));
                    lines.push(Line::from(format!(
                        "Parent source: {}",
                        source_label(item.source)
                    )));
                    lines.push(Line::from(format!(
                        "Parent version: {}",
                        item.version.as_deref().unwrap_or("unknown")
                    )));
                    lines.push(Line::from(
                        "Child rows are tree-visible from parent ownership data.",
                    ));
                } else if let Some(item) = selected {
                    lines.push(Line::from(format!("Name: {}", item.name)));
                    lines.push(Line::from(format!("Folder: {}", item.folder)));
                    lines.push(Line::from(format!("Kind: {}", addon_kind_label(item.kind))));
                    lines.push(Line::from(format!("Source: {}", source_label(item.source))));
                    lines.push(Line::from(format!(
                        "Version: {}",
                        item.version.as_deref().unwrap_or("unknown")
                    )));
                    lines.push(Line::from(format!(
                        "Author: {}",
                        item.author.as_deref().unwrap_or("unknown")
                    )));
                    lines.push(Line::from(format!(
                        "Interface: {}",
                        item.interface.as_deref().unwrap_or("unknown")
                    )));
                    lines.push(Line::from(format!(
                        "Git commit: {}",
                        item.git_commit.as_deref().unwrap_or("n/a")
                    )));
                    lines.push(Line::from(format!(
                        "Owned folders: {}",
                        item.owned_folder_count
                    )));
                    lines.push(Line::from(format!(
                        "Tree state: {}",
                        if self.dashboard.expanded_folders.contains(&item.folder) {
                            "expanded"
                        } else {
                            "collapsed"
                        }
                    )));
                    lines.push(Line::from(format!(
                        "Required deps: {}",
                        join_or_unknown(&item.required_deps)
                    )));
                    lines.push(Line::from(format!(
                        "Optional deps: {}",
                        join_or_unknown(&item.optional_deps)
                    )));
                    lines.push(Line::from(format!(
                        "Embedded libs: {}",
                        join_or_unknown(&item.embedded_libs)
                    )));
                    lines.push(Line::from(format!(
                        "Selected for bulk actions: {}",
                        if self.dashboard.is_parent_selected(&item.folder) {
                            "yes"
                        } else {
                            "no"
                        }
                    )));
                } else {
                    lines.push(Line::from("No addon selected."));
                    lines.push(Line::from("Waiting for scan results."));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "Selected parents: {}",
                    self.dashboard.selected_parent_count()
                )));
                if let Some(row) = selected_row {
                    lines.push(Line::from(format!(
                        "Selected row: {}",
                        match row.kind {
                            DashboardRowKind::Parent => "parent",
                            DashboardRowKind::OwnedChild { .. } => "owned child",
                        }
                    )));
                }
                lines.push(Line::from(format!(
                    "Scan status: {}",
                    self.scan_status_label()
                )));
            }
            DetailMode::Install => {
                lines.push(Line::from("Install area"));
                lines.push(Line::from(
                    "GitHub, TukUI, WoWInterface, and Wago entry points will live here.",
                ));
                lines.push(Line::from("Provider wiring is intentionally deferred."));
            }
            DetailMode::Search => {
                lines.push(Line::from("Search area"));
                lines.push(Line::from(
                    "Command/search surface placeholder for addon lookup and filtering.",
                ));
                lines.push(Line::from(
                    "Persistent single-screen routing is now in place.",
                ));
            }
            DetailMode::Update => {
                lines.push(Line::from("Update area"));
                lines.push(Line::from(
                    "Check/update actions will be integrated here without leaving the shell.",
                ));
                lines.push(Line::from(
                    "Delete confirmation groundwork is active while update actions stay pending.",
                ));
            }
            DetailMode::Config => {
                lines.push(Line::from("Config area"));
                lines.push(Line::from(format!(
                    "Target path: {}",
                    self.rendered_target_path()
                )));
                lines.push(Line::from(format!(
                    "Config present: {}",
                    self.config_present
                )));
                lines.push(Line::from("Config editing controls are not wired yet."));
            }
            DetailMode::Backup => {
                lines.push(Line::from("Backup area"));
                lines.push(Line::from(
                    "WTF backup and restore actions will be integrated here later.",
                ));
                lines.push(Line::from(
                    "This pane currently exists to validate the unified layout.",
                ));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from("Modes"));
        for (mode, key, label) in [
            (DetailMode::Overview, "o", "Overview"),
            (DetailMode::Install, "i", "Install"),
            (DetailMode::Search, "s", "Search"),
            (DetailMode::Update, "u", "Update"),
            (DetailMode::Config, "c", "Config"),
            (DetailMode::Backup, "b", "Backup"),
        ] {
            let marker = if self.dashboard.detail_mode == mode {
                "•"
            } else {
                " "
            };
            lines.push(Line::from(format!("{marker} {key} {label}")));
        }

        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Detail"))
    }

    fn base_status_line(&self) -> String {
        match self.shell_mode {
            ShellMode::Onboarding => format!("profile {} | q quit", self.runtime.profile_name),
            ShellMode::Dashboard => {
                format!(
                    "profile {} | {}",
                    self.runtime.profile_name, DASHBOARD_COMMANDS
                )
            }
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
            self.with_base_status("found WoW installation | select action with j/k, enter confirm")
        }
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
    folders: &[String],
) -> std::result::Result<DashboardDeleteOutcome, String> {
    let database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
    let mut deleted_parents = 0usize;
    let mut deleted_folders = 0usize;

    for folder in folders {
        let planned_folders = database
            .planned_removal_folders(folder)
            .map_err(|error| error.to_string())?;

        for planned_folder in planned_folders {
            let path = addon_dir.join(&planned_folder);
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|error| error.to_string())?;
                deleted_folders += 1;
            } else if path.is_file() {
                fs::remove_file(&path).map_err(|error| error.to_string())?;
                deleted_folders += 1;
            }
        }

        database
            .remove_addon(folder)
            .map_err(|error| error.to_string())?;
        deleted_parents += 1;
    }

    let addons = database.list_addons().map_err(|error| error.to_string())?;
    Ok(DashboardDeleteOutcome {
        deleted_parents,
        deleted_folders,
        addons,
    })
}

fn detail_mode_label(detail_mode: DetailMode) -> &'static str {
    match detail_mode {
        DetailMode::Overview => "Overview",
        DetailMode::Install => "Install",
        DetailMode::Search => "Search",
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

fn addon_kind_label(kind: AddonKind) -> &'static str {
    match kind {
        AddonKind::Addon => "Addon",
        AddonKind::Library => "Library",
    }
}

fn join_or_unknown(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
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
    use std::fs;
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{
        AddonScanOutcome, App, AppMessage, AppRuntime, AppTaskEvent, DashboardState, DetailMode,
        ScanState, ShellMode,
    };
    use crate::action::AppAction;
    use crate::onboarding::{FoundAction, FoundState, OnboardingPhase, OnboardingState};
    use lemonup_core::{
        AddonKind, AddonRecord, AppConfig, AppPaths, ConfigStore, DEFAULT_PROFILE, OwnedFolder,
        ScanSummary, SourceKind, StateDatabase,
    };

    fn app_for_tests(shell_mode: ShellMode) -> App {
        let (task_events_tx, task_events_rx) = mpsc::unbounded_channel();
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
        second.owned_folders = vec![lemonup_core::OwnedFolder {
            name: "Second_Config".to_string(),
        }];

        let mut third = AddonRecord::new("Third", "Third", SourceKind::Wago);
        third.kind = AddonKind::Library;
        third.interface = Some("110005".to_string());
        third.optional_deps = vec!["Optional".to_string()];

        App {
            shell_mode,
            quit_requested: false,
            status_line: format!("profile {} | q quit", DEFAULT_PROFILE),
            config_present: true,
            config_store: ConfigStore::new(std::env::temp_dir().join("lemonup-test-config.toml")),
            state_db_file: std::env::temp_dir().join("lemonup-test-state.sqlite"),
            runtime: AppRuntime::new(DEFAULT_PROFILE.to_string(), None, None),
            effective_addon_dir: None,
            scan_state: ScanState::Idle,
            dashboard: DashboardState::from_addons(vec![first, second, third]),
            onboarding: OnboardingState::new(),
            task_events_tx,
            task_events_rx,
            active_onboarding_cancel: None,
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
    fn detail_mode_keys_emit_messages_on_dashboard() {
        let app = app_for_tests(ShellMode::Dashboard);

        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('i'))),
            vec![AppMessage::SetDetailMode(DetailMode::Install)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('b'))),
            vec![AppMessage::SetDetailMode(DetailMode::Backup)]
        );
        assert_eq!(
            app.messages_for_key(KeyEvent::from(KeyCode::Enter)),
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
    fn delete_selected_addons_removes_parent_owned_folders_and_state_rows() {
        let temp = tempdir().expect("temp dir");
        let addon_dir = temp.path().join("AddOns");
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::create_dir_all(addon_dir.join("Second")).expect("create parent dir");
        fs::create_dir_all(addon_dir.join("Second_Config")).expect("create child dir");

        let state_db_file = temp.path().join("state.sqlite");
        let mut database = StateDatabase::open(&state_db_file).expect("open state db");

        let mut managed = AddonRecord::new("Second", "Second", SourceKind::GitHub);
        managed.version = Some("2.0.0".to_string());
        managed.set_managed_owned_folders(vec![OwnedFolder {
            name: "Second_Config".to_string(),
        }]);
        database
            .record_managed_addon(&managed)
            .expect("record managed addon");

        let outcome =
            super::delete_selected_addons(&state_db_file, &addon_dir, &["Second".to_string()])
                .expect("delete selected addons");

        assert_eq!(outcome.deleted_parents, 1);
        assert_eq!(outcome.deleted_folders, 2);
        assert!(outcome.addons.is_empty());
        assert!(!addon_dir.join("Second").exists());
        assert!(!addon_dir.join("Second_Config").exists());
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
    fn onboarding_tick_bootstraps_quick_check() {
        let mut app = app_for_tests(ShellMode::Onboarding);
        app.onboarding.phase = OnboardingPhase::Bootstrapping;

        let actions = app.update(AppMessage::Tick);

        assert_eq!(
            actions,
            vec![
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
        app.onboarding.phase = OnboardingPhase::Found(FoundState {
            path: app.onboarding.input.clone(),
            selected_action: 0,
        });

        let actions = app.update(AppMessage::OnboardingFoundConfirm);
        assert_eq!(
            actions,
            vec![AppAction::SaveAddonDir(PathBuf::from(
                "C:\\Games\\World of Warcraft\\_retail_\\Interface\\AddOns"
            ))]
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
            FoundAction::ScanAnotherLocation
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

        app.apply(AppAction::SaveAddonDir(PathBuf::from(
            "D:\\World of Warcraft\\_retail_\\Interface\\AddOns",
        )));

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
            vec![AppAction::StartAddonScan(PathBuf::from(
                "D:\\Sandbox\\World of Warcraft\\_retail_\\Interface\\AddOns"
            ))]
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
            }),
        )));

        assert_eq!(actions.len(), 2);
        app.apply(actions[0].clone());
        app.apply(actions[1].clone());

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
