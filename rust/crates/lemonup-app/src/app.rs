use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use lemonup_core::{
    AppConfig, AppPaths, ConfigLoad, ConfigStore, StateDatabase, detect_known_addons_path,
    search_for_wow, validate_addons_path,
};
use tokio::sync::mpsc;

use crate::action::AppAction;
use crate::event::{EventHandler, TerminalEvent};
use crate::onboarding::{FoundAction, OnboardingPhase, OnboardingState, OnboardingTaskEvent};
use crate::tui::Backend;

const BASE_STATUS: &str = "q quit | 1 onboarding | 2 manage | 3 install | 4 config | 5 wago";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Onboarding,
    Manage,
    Install,
    Config,
    WagoSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppMessage {
    Tick,
    QuitRequested,
    Navigate(Screen),
    TerminalResized { width: u16, height: u16 },
    ManageSelectionNext,
    ManageSelectionPrevious,
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

struct ManageState {
    items: Vec<String>,
    list_state: ListState,
}

impl ManageState {
    fn new(addon_count: usize) -> Self {
        let items = vec![
            format!("Tracked addons in state DB: {addon_count}"),
            "Planned: update/remove/check actions against typed core events".to_string(),
            "Planned: filter/search, ownership, dependencies, and backup actions".to_string(),
        ];
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }

        Self { items, list_state }
    }

    fn next_selection(&self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }

        let next = match self.list_state.selected() {
            Some(index) => (index + 1) % self.items.len(),
            None => 0,
        };
        Some(next)
    }

    fn previous_selection(&self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }

        let previous = match self.list_state.selected() {
            Some(0) | None => self.items.len() - 1,
            Some(index) => index - 1,
        };
        Some(previous)
    }
}

pub struct App {
    active_screen: Screen,
    quit_requested: bool,
    status_line: String,
    config_present: bool,
    config_store: ConfigStore,
    manage: ManageState,
    onboarding: OnboardingState,
    onboarding_events_tx: mpsc::UnboundedSender<OnboardingTaskEvent>,
    onboarding_events_rx: mpsc::UnboundedReceiver<OnboardingTaskEvent>,
    active_onboarding_cancel: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn bootstrap(paths: AppPaths) -> lemonup_core::Result<Self> {
        let config_store = ConfigStore::new(paths.config_file);
        let config_state = config_store.load()?;
        let database = StateDatabase::open(paths.state_db_file)?;
        let addon_count = database.list_addons()?.len();
        let config_present = matches!(config_state, ConfigLoad::Loaded(_));
        let active_screen = if config_present {
            Screen::Manage
        } else {
            Screen::Onboarding
        };
        let (onboarding_events_tx, onboarding_events_rx) = mpsc::unbounded_channel();

        Ok(Self {
            active_screen,
            quit_requested: false,
            status_line: BASE_STATUS.to_string(),
            config_present,
            config_store,
            manage: ManageState::new(addon_count),
            onboarding: OnboardingState::new(),
            onboarding_events_tx,
            onboarding_events_rx,
            active_onboarding_cancel: None,
        })
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

        if self.active_screen == Screen::Onboarding {
            return self.onboarding_messages_for_key(key);
        }

        match key.code {
            KeyCode::Char('q') => vec![AppMessage::QuitRequested],
            KeyCode::Char('1') => vec![AppMessage::Navigate(Screen::Onboarding)],
            KeyCode::Char('2') => vec![AppMessage::Navigate(Screen::Manage)],
            KeyCode::Char('3') => vec![AppMessage::Navigate(Screen::Install)],
            KeyCode::Char('4') => vec![AppMessage::Navigate(Screen::Config)],
            KeyCode::Char('5') => vec![AppMessage::Navigate(Screen::WagoSearch)],
            KeyCode::Down | KeyCode::Char('j') if self.active_screen == Screen::Manage => {
                vec![AppMessage::ManageSelectionNext]
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_screen == Screen::Manage => {
                vec![AppMessage::ManageSelectionPrevious]
            }
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
                KeyCode::Up | KeyCode::Char('k') => vec![AppMessage::OnboardingFoundActionPrevious],
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
                if self.active_screen == Screen::Onboarding
                    && matches!(self.onboarding.phase, OnboardingPhase::Bootstrapping)
                {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.begin_quick_check()),
                        AppAction::SetStatus(
                            "checking common install locations | q quit".to_string(),
                        ),
                        AppAction::StartOnboardingQuickCheck,
                    ]
                } else {
                    vec![AppAction::None]
                }
            }
            AppMessage::QuitRequested => vec![AppAction::Quit],
            AppMessage::Navigate(screen) => {
                vec![
                    AppAction::SetScreen(screen),
                    AppAction::SetStatus(BASE_STATUS.to_string()),
                ]
            }
            AppMessage::TerminalResized { width, height } => vec![AppAction::SetStatus(format!(
                "terminal resized to {width}x{height} | {BASE_STATUS}"
            ))],
            AppMessage::ManageSelectionNext => {
                let selection = self.manage.next_selection();
                vec![
                    AppAction::SetManageSelection(selection),
                    AppAction::SetStatus(format!("manage selection moved | {BASE_STATUS}")),
                ]
            }
            AppMessage::ManageSelectionPrevious => {
                let selection = self.manage.previous_selection();
                vec![
                    AppAction::SetManageSelection(selection),
                    AppAction::SetStatus(format!("manage selection moved | {BASE_STATUS}")),
                ]
            }
            AppMessage::OnboardingBeginEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.begin_editing()),
                AppAction::SetStatus(
                    "editing path | enter validate | d deep scan | esc stop editing".to_string(),
                ),
            ],
            AppMessage::OnboardingStopEditing => vec![
                AppAction::SetOnboardingState(self.onboarding.stop_editing()),
                AppAction::SetStatus(
                    "location finder | enter validate | d deep scan | e edit path".to_string(),
                ),
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
                match validate_addons_path(&current_path) {
                    Ok(()) => {
                        let next_state = self.onboarding.found(current_path.clone());
                        vec![
                            AppAction::SetOnboardingState(next_state),
                            AppAction::SetStatus(
                                "found WoW installation | enter confirm selected action"
                                    .to_string(),
                            ),
                        ]
                    }
                    Err(error) => vec![
                        AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                        AppAction::SetStatus(format!("validation failed: {error}")),
                    ],
                }
            }
            AppMessage::OnboardingDeepScan => {
                let root = PathBuf::from(self.onboarding.input.trim());
                vec![
                    AppAction::SetOnboardingState(self.onboarding.start_deep_scan()),
                    AppAction::SetStatus("deep scan running | esc cancel".to_string()),
                    AppAction::StartOnboardingDeepScan(root),
                ]
            }
            AppMessage::OnboardingCancel => {
                if matches!(self.onboarding.phase, OnboardingPhase::DeepScanning(_)) {
                    vec![
                        AppAction::CancelOnboardingScan,
                        AppAction::SetOnboardingState(self.onboarding.cancelled()),
                        AppAction::SetStatus(
                            "deep scan cancelled | enter validate | d deep scan | e edit path"
                                .to_string(),
                        ),
                    ]
                } else {
                    vec![
                        AppAction::SetOnboardingState(self.onboarding.clear_status_to_ready()),
                        AppAction::SetStatus(
                            "location finder | enter validate | d deep scan | e edit path"
                                .to_string(),
                        ),
                    ]
                }
            }
            AppMessage::OnboardingSuggestionNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_suggestion()),
                AppAction::SetStatus(
                    "location finder | enter validate | d deep scan | e edit path".to_string(),
                ),
            ],
            AppMessage::OnboardingSuggestionPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_suggestion()),
                AppAction::SetStatus(
                    "location finder | enter validate | d deep scan | e edit path".to_string(),
                ),
            ],
            AppMessage::OnboardingFoundActionNext => vec![
                AppAction::SetOnboardingState(self.onboarding.next_found_action()),
                AppAction::SetStatus(
                    "found WoW installation | select action with j/k, enter confirm".to_string(),
                ),
            ],
            AppMessage::OnboardingFoundActionPrevious => vec![
                AppAction::SetOnboardingState(self.onboarding.previous_found_action()),
                AppAction::SetStatus(
                    "found WoW installation | select action with j/k, enter confirm".to_string(),
                ),
            ],
            AppMessage::OnboardingFoundConfirm => match self.onboarding.selected_found_action() {
                FoundAction::UseThisPath => vec![AppAction::SaveAddonDir(PathBuf::from(
                    self.onboarding.input.trim(),
                ))],
                FoundAction::ScanAnotherLocation => vec![
                    AppAction::SetOnboardingState(self.onboarding.ready()),
                    AppAction::SetStatus(
                        "choose another root | enter validate | d deep scan | e edit path"
                            .to_string(),
                    ),
                ],
                FoundAction::EditPathManually => vec![
                    AppAction::SetOnboardingState(self.onboarding.begin_editing()),
                    AppAction::SetStatus(
                        "editing found path | enter validate | d deep scan | esc stop editing"
                            .to_string(),
                    ),
                ],
            },
            AppMessage::OnboardingTaskEvent(event) => match event {
                OnboardingTaskEvent::QuickCheckFinished(Some(path)) => vec![
                    AppAction::SetOnboardingState(self.onboarding.found(path)),
                    AppAction::SetStatus(
                        "found WoW installation | select action with j/k, enter confirm"
                            .to_string(),
                    ),
                ],
                OnboardingTaskEvent::QuickCheckFinished(None) => vec![
                    AppAction::SetOnboardingState(self.onboarding.ready()),
                    AppAction::SetStatus(
                        "no install found yet | enter validate | d deep scan | e edit path"
                            .to_string(),
                    ),
                ],
                OnboardingTaskEvent::DeepScanProgress(progress) => {
                    vec![AppAction::SetOnboardingState(
                        self.onboarding.set_scan_progress(progress),
                    )]
                }
                OnboardingTaskEvent::DeepScanFinished(Ok(Some(path))) => vec![
                    AppAction::SetOnboardingState(self.onboarding.found(path)),
                    AppAction::SetStatus(
                        "found WoW installation | select action with j/k, enter confirm"
                            .to_string(),
                    ),
                ],
                OnboardingTaskEvent::DeepScanFinished(Ok(None)) => vec![
                    AppAction::SetOnboardingState(
                        self.onboarding
                            .error("no WoW installation found from this root".to_string()),
                    ),
                    AppAction::SetStatus(
                        "deep scan finished with no result | e edit path | d scan again"
                            .to_string(),
                    ),
                ],
                OnboardingTaskEvent::DeepScanFinished(Err(error)) => vec![
                    AppAction::SetOnboardingState(self.onboarding.error(error.clone())),
                    AppAction::SetStatus(format!("deep scan failed: {error}")),
                ],
            },
        }
    }

    fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::None => {}
            AppAction::Quit => self.quit_requested = true,
            AppAction::SetScreen(screen) => self.active_screen = screen,
            AppAction::SetStatus(status) => self.status_line = status,
            AppAction::SetManageSelection(selection) => self.manage.list_state.select(selection),
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
                let mut config = match self.config_store.load() {
                    Ok(ConfigLoad::Loaded(config)) => config,
                    _ => AppConfig::new_unconfigured(),
                };
                config.addon_dir = Some(path.clone());

                match self.config_store.write_new_config(&config) {
                    Ok(()) => {
                        self.config_present = true;
                        self.active_screen = Screen::Manage;
                        self.status_line =
                            format!("saved addon directory {} | {BASE_STATUS}", path.display());
                    }
                    Err(error) => {
                        self.onboarding = self.onboarding.error(error.to_string());
                        self.status_line = format!("failed to save config: {error}");
                    }
                }
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                "LemonUp v2",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Rust + Ratatui foundation"),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Header"));

        let footer = Paragraph::new(self.status_line.clone())
            .block(Block::default().borders(Borders::ALL).title("Status"));

        frame.render_widget(header, layout[0]);
        match self.active_screen {
            Screen::Onboarding => frame.render_widget(self.onboarding_body(), layout[1]),
            Screen::Manage => self.render_manage(frame, layout[1]),
            Screen::Install => frame.render_widget(self.install_body(), layout[1]),
            Screen::Config => frame.render_widget(self.config_body(), layout[1]),
            Screen::WagoSearch => frame.render_widget(self.wago_body(), layout[1]),
        }
        frame.render_widget(footer, layout[2]);
    }

    fn onboarding_body(&self) -> Paragraph<'static> {
        let mut lines = vec![
            Line::from("Locate your World of Warcraft AddOns folder"),
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

        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Onboarding"))
    }

    fn render_manage(&mut self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let items = self
            .manage
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if self.manage.list_state.selected() == Some(index) {
                    "› "
                } else {
                    "  "
                };
                ListItem::new(format!("{prefix}{item}"))
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Manage"))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(list, area, &mut self.manage.list_state);
    }

    fn install_body(&self) -> Paragraph<'static> {
        Paragraph::new(vec![
            Line::from("Current screen: install"),
            Line::from("Planned sources: GitHub, TukUI, WoWInterface, Wago."),
            Line::from("Typed InstallPlan exists in core; provider wiring is next."),
        ])
        .block(Block::default().borders(Borders::ALL).title("Install"))
    }

    fn config_body(&self) -> Paragraph<'static> {
        Paragraph::new(vec![
            Line::from("Current screen: config"),
            Line::from("Config store uses versioned TOML under OS-native config dirs."),
            Line::from("State database lives separately under the OS-native data dir."),
        ])
        .block(Block::default().borders(Borders::ALL).title("Config"))
    }

    fn wago_body(&self) -> Paragraph<'static> {
        Paragraph::new(vec![
            Line::from("Current screen: Wago search"),
            Line::from("Planned: search/install flow over a shared provider trait."),
            Line::from("This screen exists now to lock routing and public UI boundaries."),
        ])
        .block(Block::default().borders(Borders::ALL).title("Wago"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::widgets::ListState;

    use super::{App, AppMessage, ManageState, Screen};
    use crate::action::AppAction;
    use crate::onboarding::{FoundState, OnboardingPhase, OnboardingState};
    use lemonup_core::ConfigStore;
    use tokio::sync::mpsc;

    fn app_for_tests(screen: Screen) -> App {
        let (onboarding_events_tx, onboarding_events_rx) = mpsc::unbounded_channel();
        App {
            active_screen: screen,
            quit_requested: false,
            status_line: super::BASE_STATUS.to_string(),
            config_present: true,
            config_store: ConfigStore::new(std::env::temp_dir().join("lemonup-test-config.toml")),
            manage: ManageState {
                items: vec![
                    "first".to_string(),
                    "second".to_string(),
                    "third".to_string(),
                ],
                list_state: {
                    let mut state = ListState::default();
                    state.select(Some(0));
                    state
                },
            },
            onboarding: OnboardingState::new(),
            onboarding_events_tx,
            onboarding_events_rx,
            active_onboarding_cancel: None,
        }
    }

    #[test]
    fn manage_navigation_keys_emit_messages_on_manage_screen() {
        let app = app_for_tests(Screen::Manage);

        let next = app.messages_for_key(KeyEvent::from(KeyCode::Char('j')));
        let previous = app.messages_for_key(KeyEvent::from(KeyCode::Up));

        assert_eq!(next, vec![AppMessage::ManageSelectionNext]);
        assert_eq!(previous, vec![AppMessage::ManageSelectionPrevious]);
    }

    #[test]
    fn manage_navigation_keys_are_ignored_outside_manage_screen() {
        let app = app_for_tests(Screen::Install);

        assert!(
            app.messages_for_key(KeyEvent::from(KeyCode::Char('j')))
                .is_empty()
        );
        assert!(
            app.messages_for_key(KeyEvent::from(KeyCode::Down))
                .is_empty()
        );
    }

    #[test]
    fn manage_selection_wraps_forward_and_backward() {
        let app = app_for_tests(Screen::Manage);

        let forward = app.update(AppMessage::ManageSelectionNext);
        assert_eq!(
            forward,
            vec![
                AppAction::SetManageSelection(Some(1)),
                AppAction::SetStatus(format!("manage selection moved | {}", super::BASE_STATUS)),
            ]
        );

        let mut wrapped = app_for_tests(Screen::Manage);
        wrapped.manage.list_state.select(Some(0));
        let backward = wrapped.update(AppMessage::ManageSelectionPrevious);
        assert_eq!(
            backward,
            vec![
                AppAction::SetManageSelection(Some(2)),
                AppAction::SetStatus(format!("manage selection moved | {}", super::BASE_STATUS)),
            ]
        );
    }

    #[test]
    fn onboarding_tick_bootstraps_quick_check() {
        let mut app = app_for_tests(Screen::Onboarding);
        app.onboarding.phase = OnboardingPhase::Bootstrapping;

        let actions = app.update(AppMessage::Tick);

        assert_eq!(
            actions,
            vec![
                AppAction::SetOnboardingState(app.onboarding.begin_quick_check()),
                AppAction::SetStatus("checking common install locations | q quit".to_string()),
                AppAction::StartOnboardingQuickCheck,
            ]
        );
    }

    #[test]
    fn found_confirmation_uses_selected_action() {
        let mut app = app_for_tests(Screen::Onboarding);
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
        let app = app_for_tests(Screen::Onboarding);

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
        let app = app_for_tests(Screen::Onboarding);
        let release = KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };

        assert!(app.messages_for_key(release).is_empty());
    }
}
