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
    AddonRecord, AppConfig, AppPaths, ConfigLoad, ConfigStore, DEFAULT_PROFILE, SourceKind,
    StateDatabase, detect_known_addons_path, paths_match, search_for_wow, validate_addons_path,
};
use tokio::sync::mpsc;

use crate::action::AppAction;
use crate::event::{EventHandler, TerminalEvent};
use crate::onboarding::{FoundAction, OnboardingPhase, OnboardingState, OnboardingTaskEvent};
use crate::tui::Backend;

const DASHBOARD_COMMANDS: &str =
    "q quit | j/k list | o overview | i install | s search | u update | c config | b backup";

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
    OnboardingTaskEvent(OnboardingTaskEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardItem {
    name: String,
    folder: String,
    source: SourceKind,
    version: Option<String>,
    author: Option<String>,
    owned_folder_count: usize,
}

struct DashboardState {
    items: Vec<DashboardItem>,
    list_state: ListState,
    detail_mode: DetailMode,
}

impl DashboardState {
    fn from_addons(addons: Vec<AddonRecord>) -> Self {
        let items = addons
            .into_iter()
            .map(|addon| DashboardItem {
                name: addon.name,
                folder: addon.folder,
                source: addon.source,
                version: addon.version,
                author: addon.author,
                owned_folder_count: addon.owned_folders.len(),
            })
            .collect::<Vec<_>>();

        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            items,
            list_state,
            detail_mode: DetailMode::Overview,
        }
    }

    fn next_selection(&self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }

        Some(match self.list_state.selected() {
            Some(index) => (index + 1) % self.items.len(),
            None => 0,
        })
    }

    fn previous_selection(&self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }

        Some(match self.list_state.selected() {
            Some(0) | None => self.items.len() - 1,
            Some(index) => index - 1,
        })
    }

    fn selected_item(&self) -> Option<&DashboardItem> {
        self.list_state
            .selected()
            .and_then(|index| self.items.get(index))
    }
}

pub struct App {
    shell_mode: ShellMode,
    quit_requested: bool,
    status_line: String,
    config_present: bool,
    config_store: ConfigStore,
    runtime: AppRuntime,
    effective_addon_dir: Option<PathBuf>,
    dashboard: DashboardState,
    onboarding: OnboardingState,
    onboarding_events_tx: mpsc::UnboundedSender<OnboardingTaskEvent>,
    onboarding_events_rx: mpsc::UnboundedReceiver<OnboardingTaskEvent>,
    active_onboarding_cancel: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn bootstrap(paths: AppPaths, runtime: AppRuntime) -> lemonup_core::Result<Self> {
        let config_store = ConfigStore::new(paths.config_file);
        let config_state = config_store.load()?;
        let database = StateDatabase::open(paths.state_db_file)?;
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

        let (onboarding_events_tx, onboarding_events_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            shell_mode,
            quit_requested: false,
            status_line: String::new(),
            config_present,
            config_store,
            runtime,
            effective_addon_dir,
            dashboard: DashboardState::from_addons(addons),
            onboarding,
            onboarding_events_tx,
            onboarding_events_rx,
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
                } else {
                    self.dashboard_status_for(
                        DetailMode::Overview,
                        "single-surface shell ready | scan sync wiring lands next",
                    )
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
        while let Ok(event) = self.onboarding_events_rx.try_recv() {
            let actions = self.update(AppMessage::OnboardingTaskEvent(event));
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
        match key.code {
            KeyCode::Char('q') => vec![AppMessage::QuitRequested],
            KeyCode::Down | KeyCode::Char('j') => vec![AppMessage::DashboardSelectionNext],
            KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::DashboardSelectionPrevious],
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
            AppMessage::OnboardingTaskEvent(event) => match event {
                OnboardingTaskEvent::QuickCheckFinished(Some(path)) => vec![
                    AppAction::SetOnboardingState(self.onboarding_found_state(path.clone())),
                    AppAction::SetStatus(self.found_status_for_path(&path)),
                ],
                OnboardingTaskEvent::QuickCheckFinished(None) => vec![
                    AppAction::SetOnboardingState(self.onboarding.ready()),
                    AppAction::SetStatus(self.with_base_status(
                        "no install found yet | enter validate | d deep scan | e edit path",
                    )),
                ],
                OnboardingTaskEvent::DeepScanProgress(progress) => {
                    vec![AppAction::SetOnboardingState(
                        self.onboarding.set_scan_progress(progress),
                    )]
                }
                OnboardingTaskEvent::DeepScanFinished(Ok(Some(path))) => vec![
                    AppAction::SetOnboardingState(self.onboarding_found_state(path.clone())),
                    AppAction::SetStatus(self.found_status_for_path(&path)),
                ],
                OnboardingTaskEvent::DeepScanFinished(Ok(None)) => vec![
                    AppAction::SetOnboardingState(
                        self.onboarding
                            .error("no WoW installation found from this root".to_string()),
                    ),
                    AppAction::SetStatus(self.with_base_status(
                        "deep scan finished with no result | e edit path | d scan again",
                    )),
                ],
                OnboardingTaskEvent::DeepScanFinished(Err(error)) => vec![
                    AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                    AppAction::SetStatus(
                        self.with_base_status(&format!("deep scan failed: {error}")),
                    ),
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
            AppAction::SetDetailMode(detail_mode) => self.dashboard.detail_mode = detail_mode,
            AppAction::SetOnboardingState(state) => self.onboarding = state,
            AppAction::StartOnboardingQuickCheck => {
                let sender = self.onboarding_events_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(detect_known_addons_path)
                        .await
                        .ok()
                        .flatten();
                    let _ = sender.send(OnboardingTaskEvent::QuickCheckFinished(result));
                });
            }
            AppAction::StartOnboardingDeepScan(root) => {
                let sender = self.onboarding_events_tx.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                self.active_onboarding_cancel = Some(cancel.clone());
                tokio::spawn(async move {
                    let progress_sender = sender.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        search_for_wow(
                            &root,
                            || cancel.load(Ordering::SeqCst),
                            |progress| {
                                let _ =
                                    progress_sender.send(OnboardingTaskEvent::DeepScanProgress(
                                        crate::onboarding::ScanProgressState {
                                            dirs_scanned: progress.dirs_scanned,
                                            current_path: progress
                                                .current_path
                                                .display()
                                                .to_string(),
                                        },
                                    ));
                            },
                        )
                    })
                    .await
                    .unwrap_or_else(|join_error| Err(join_error.to_string()));

                    let _ = sender.send(OnboardingTaskEvent::DeepScanFinished(result));
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
                        self.status_line = self.dashboard_status_for(
                            self.dashboard.detail_mode,
                            &format!(
                                "saved addon directory {} | scan sync wiring lands next",
                                path.display()
                            ),
                        );
                    }
                    Err(error) => {
                        self.onboarding = self.onboarding.error(error.to_string());
                        self.status_line =
                            self.with_base_status(&format!("failed to save config: {error}"));
                    }
                }
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
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
        if self.dashboard.items.is_empty() {
            let body = Paragraph::new(vec![
                Line::from("No scanned addons yet."),
                Line::from("This shell foundation is ready; scan sync wiring lands next chunk."),
                Line::from("Use the detail pane modes to inspect the integrated surface layout."),
            ])
            .block(Block::default().borders(Borders::ALL).title("Addons"));
            frame.render_widget(body, area);
            return;
        }

        let items = self
            .dashboard
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if self.dashboard.list_state.selected() == Some(index) {
                    "› "
                } else {
                    "  "
                };
                let version = item.version.as_deref().unwrap_or("unknown");
                ListItem::new(vec![
                    Line::from(format!("{prefix}{}", item.name)),
                    Line::from(format!("    {} | {}", item.folder, version)),
                ])
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Addons"))
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

        lines.push(Line::from(format!(
            "Mode: {}",
            detail_mode_label(self.dashboard.detail_mode)
        )));
        lines.push(Line::from(""));

        match self.dashboard.detail_mode {
            DetailMode::Overview => {
                if let Some(item) = selected {
                    lines.push(Line::from(format!("Name: {}", item.name)));
                    lines.push(Line::from(format!("Folder: {}", item.folder)));
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
                        "Owned folders: {}",
                        item.owned_folder_count
                    )));
                } else {
                    lines.push(Line::from("No addon selected."));
                    lines.push(Line::from(
                        "Once scan sync is wired, real addon rows will populate here.",
                    ));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "Single-surface foundation is active. Scan sync and richer interactions land next.",
                ));
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
                lines.push(Line::from("Only shell boundaries are wired in this chunk."));
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::widgets::ListState;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{
        App, AppMessage, AppRuntime, DashboardItem, DashboardState, DetailMode, ShellMode,
    };
    use crate::action::AppAction;
    use crate::onboarding::{FoundAction, FoundState, OnboardingPhase, OnboardingState};
    use lemonup_core::{AppPaths, ConfigStore, DEFAULT_PROFILE, SourceKind};

    fn app_for_tests(shell_mode: ShellMode) -> App {
        let (onboarding_events_tx, onboarding_events_rx) = mpsc::unbounded_channel();
        App {
            shell_mode,
            quit_requested: false,
            status_line: format!("profile {} | q quit", DEFAULT_PROFILE),
            config_present: true,
            config_store: ConfigStore::new(std::env::temp_dir().join("lemonup-test-config.toml")),
            runtime: AppRuntime::new(DEFAULT_PROFILE.to_string(), None, None),
            effective_addon_dir: None,
            dashboard: DashboardState {
                items: vec![
                    DashboardItem {
                        name: "First".to_string(),
                        folder: "First".to_string(),
                        source: SourceKind::Manual,
                        version: Some("1.0.0".to_string()),
                        author: None,
                        owned_folder_count: 0,
                    },
                    DashboardItem {
                        name: "Second".to_string(),
                        folder: "Second".to_string(),
                        source: SourceKind::GitHub,
                        version: Some("2.0.0".to_string()),
                        author: Some("Author".to_string()),
                        owned_folder_count: 1,
                    },
                    DashboardItem {
                        name: "Third".to_string(),
                        folder: "Third".to_string(),
                        source: SourceKind::Wago,
                        version: None,
                        author: None,
                        owned_folder_count: 0,
                    },
                ],
                list_state: {
                    let mut state = ListState::default();
                    state.select(Some(0));
                    state
                },
                detail_mode: DetailMode::Overview,
            },
            onboarding: OnboardingState::new(),
            onboarding_events_tx,
            onboarding_events_rx,
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

        let actions = app.update(AppMessage::OnboardingTaskEvent(
            crate::onboarding::OnboardingTaskEvent::DeepScanProgress(
                crate::onboarding::ScanProgressState {
                    dirs_scanned: 42,
                    current_path: "C:\\".to_string(),
                },
            ),
        ));

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
        let actions = app.update(AppMessage::OnboardingTaskEvent(
            crate::onboarding::OnboardingTaskEvent::QuickCheckFinished(Some(path.clone())),
        ));

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
}
