//! Switching between your games: the Switch game sheet, adding and
//! removing a game, the header badge for another game's updates, and the
//! checks of the other games that run in the background.
//!
//! Each game is one profile with its own config, addon list and backups.
//! Switching loads the app again for the other profile, the way starting
//! LemonUp with `--profile` does, so nothing of one game leaks into another.

use lemonup_core::ProfileRoots;
use lemonup_core::profiles::{
    ProfileError, create_profile, game_name, list_profiles, profile_name_for, remember_last_used,
    remove_profile,
};

use super::render_dashboard::fit_spans;
use super::sheet::{SheetBody, SheetRoom, text_height};
use super::*;

/// The "Game" column of the sheet, at most.
const NAME_WIDTH_MAX: usize = 28;
/// The "Addons" column: "1,234 addons" and room to spare.
const ADDONS_WIDTH: usize = 14;
/// The "Updates" column: "↑12 Update ready" and room to spare.
const UPDATES_WIDTH: usize = 19;

/// One game as the sheet and the header show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameEntry {
    /// The profile name, for `--profile`.
    pub(crate) profile: String,
    /// What users see: "Retail", "WoW Forever", or with the profile name
    /// when two games are of one version.
    pub(crate) display_name: String,
    pub(crate) flavor: Option<GameFlavor>,
    /// Addons in its list, libraries not counted; `None` when unreadable.
    pub(crate) addon_count: Option<usize>,
    /// Its addons with Update ready; `None` before any addon was checked.
    pub(crate) update_ready: Option<usize>,
    /// When an addon of it was last checked.
    pub(crate) last_checked: Option<OffsetDateTime>,
}

/// The games, the sheet's cursor and the background checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GamesState {
    pub(crate) games: Vec<GameEntry>,
    /// The sheet's cursor; `games.len()` is "Add another game…".
    pub(crate) selected: usize,
    /// Game rows the sheet keeps room for while it is open, so removing a
    /// game does not move it.
    pub(crate) rows_reserved: usize,
    /// The game the remove question asks about.
    pub(crate) pending_remove: Option<String>,
    /// When each other game's background check last started.
    pub(crate) attempted: HashMap<String, OffsetDateTime>,
    /// The game a background check is running for.
    pub(crate) checking: Option<String>,
    /// Setup is open to add another game.
    pub(crate) adding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GamesMessage {
    Open,
    Next,
    Previous,
    Choose,
    RequestRemove,
    ConfirmRemove,
    CancelRemove,
    /// A background check of another game ended.
    Checked {
        profile: String,
        result: std::result::Result<(), String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GamesAction {
    /// Reads the games again and puts the cursor on the one in use.
    Reload,
    Select(usize),
    SetPendingRemove(Option<String>),
    Switch(String),
    Remove(String),
    /// Opens setup's game step to add another game.
    StartAdd,
    /// Sets up the install at this AddOns folder as a new game, then
    /// switches to it.
    Create(PathBuf),
    StartCheck(String),
    CheckFinished {
        profile: String,
        result: std::result::Result<(), String>,
    },
}

/// A background check's end, sent on the channel that outlives a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OtherGameChecked {
    pub(crate) profile: String,
    pub(crate) result: std::result::Result<(), String>,
}

/// The games that are set up, and `current` even when it is not yet.
/// `None` when the list of games can't be read; the caller keeps what it
/// had.
pub(crate) fn read_games(roots: &ProfileRoots, current: &str) -> Option<Vec<GameEntry>> {
    let summaries = list_profiles(roots).ok()?;
    Some(
        summaries
            .into_iter()
            .filter(|summary| summary.addon_dir.is_some() || summary.name == current)
            .map(|summary| {
                let addons = roots
                    .paths(&summary.name)
                    .ok()
                    .and_then(|paths| StateDatabase::read_addons(&paths.state_db_file).ok());
                let (update_ready, last_checked) =
                    addons.as_deref().map(update_counts).unwrap_or((None, None));
                GameEntry {
                    profile: summary.name,
                    display_name: summary.display_name,
                    flavor: summary.flavor,
                    addon_count: summary.addon_count,
                    update_ready,
                    last_checked,
                }
            })
            .collect(),
    )
}

/// How many addons have Update ready (`None` while none was checked), and
/// when one was last checked.
fn update_counts(addons: &[AddonRecord]) -> (Option<usize>, Option<OffsetDateTime>) {
    let last_checked = addons
        .iter()
        .filter_map(|addon| addon.last_checked_at)
        .max();
    let ready = addons
        .iter()
        .filter(|addon| {
            addon.source != SourceKind::Manual
                && crate::update::determine_update_status(addon).0 == UpdateStatus::UpdateAvailable
        })
        .count();
    (last_checked.map(|_| ready), last_checked)
}

/// Checks every addon of another game, the way `lemonup check` does, and
/// writes the results to that game's own addon list. It never updates,
/// links or touches a file of the game, and writes nothing unless the list
/// is tied to the game's AddOns folder.
pub(crate) async fn check_other_game(
    paths: AppPaths,
    wago_env_key: Option<String>,
) -> std::result::Result<(), String> {
    let store = ConfigStore::for_profile(&paths);
    // Never writes the settings all games share: the game in use may be saving them.
    let config = match store
        .load_without_writing()
        .map_err(|error| error.to_string())?
    {
        ConfigLoad::Loaded(config) => config,
        ConfigLoad::Missing(_) => return Err("it isn't set up".to_string()),
    };
    let addon_dir = config
        .addon_dir
        .clone()
        .ok_or_else(|| "it has no AddOns folder".to_string())?;
    validate_addons_path(&addon_dir)?;
    if StateDatabase::read_bound_addon_dir(&paths.state_db_file)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("its addon list isn't tied to an AddOns folder yet".to_string());
    }
    let mut database = StateDatabase::open(&paths.state_db_file).map_err(|e| e.to_string())?;
    if let BindingCheck::Mismatch(mismatch) = check_addon_dir_binding(
        &database,
        &paths.profile,
        Some(&addon_dir),
        &addon_dir,
        false,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(mismatch.to_string());
    }
    let key = wago_api_key_in_use(config.wago_api_key.clone(), wago_env_key);
    refresh_live_update_checks(&mut database, &[], key.as_deref(), Some(&addon_dir))
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

impl App {
    /// The game in use, as the list of games names it.
    pub(super) fn current_game(&self) -> Option<&GameEntry> {
        self.games
            .games
            .iter()
            .find(|game| game.profile == self.runtime.profile_name)
    }

    /// The name the header gives the game in use: its name in the list of
    /// games once its version is known, else what its folder says.
    pub(super) fn current_game_name(&self) -> String {
        match self.current_game() {
            Some(game) if game.flavor.is_some() => game.display_name.clone(),
            _ => self.game_client_label(),
        }
    }

    /// Why switching games has to wait: a job that changes this game's
    /// addons or files is running.
    pub(super) fn switch_busy_note(&self) -> Option<&'static str> {
        if self.dashboard.update_in_progress() || self.dashboard.job_is_update {
            Some("Wait for the update to finish first.")
        } else if self.dashboard_job_running().is_some() {
            Some("Wait for the check to finish first.")
        } else if self.install_running() {
            Some("Wait for the install to finish first.")
        } else if self.delete_in_progress {
            Some("Wait for the delete to finish first.")
        } else if self.undo_delete_in_progress {
            Some("Wait for the undo to finish first.")
        } else if self.toc_apply_running {
            Some("Wait for the .toc change to finish first.")
        } else if self.import_running {
            Some("Wait for the import to finish first.")
        } else if self.backup_pane.in_progress {
            Some("Wait for the backup to finish first.")
        } else if self.find.linking || self.find.looking() || !self.find.replacements.is_empty() {
            Some("Wait for the find to finish first.")
        } else {
            None
        }
    }

    /// Why `profile` can't be removed now: a check of it is running, or an import is
    /// running in the game in use.
    fn remove_busy_note(&self, profile: &str, name: &str) -> Option<String> {
        if self.games.checking.as_deref() == Some(profile) {
            Some(format!("Wait for the check of {name} to finish first."))
        } else if self.import_running {
            Some("Wait for the import to finish first.".to_string())
        } else {
            None
        }
    }

    fn games_note(&self, kind: StatusKind, text: &str) -> AppAction {
        AppAction::SetStatus(self.dashboard_status_for(kind, DetailMode::Games, text))
    }

    pub(super) fn games_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Games {
            return None;
        }
        let message = match key.code {
            KeyCode::Down | KeyCode::Char('j') => GamesMessage::Next,
            KeyCode::Up | KeyCode::Char('k') => GamesMessage::Previous,
            KeyCode::Enter => GamesMessage::Choose,
            KeyCode::Char('x') => GamesMessage::RequestRemove,
            _ => return None,
        };
        Some(vec![AppMessage::Games(message)])
    }

    pub(super) fn games_actions(&self, message: GamesMessage) -> Vec<AppAction> {
        let games = &self.games.games;
        let choices = games.len() + 1;
        let selected = self.games.selected.min(games.len());
        match message {
            GamesMessage::Open => match self.switch_busy_note() {
                Some(note) => vec![self.games_note(StatusKind::Note, note)],
                None => vec![
                    AppAction::Games(GamesAction::Reload),
                    AppAction::SetDetailMode(DetailMode::Games),
                ],
            },
            GamesMessage::Next => vec![AppAction::Games(GamesAction::Select(
                (selected + 1) % choices,
            ))],
            GamesMessage::Previous => vec![AppAction::Games(GamesAction::Select(
                (selected + choices - 1) % choices,
            ))],
            GamesMessage::Choose => {
                if let Some(note) = self.switch_busy_note() {
                    return vec![self.games_note(StatusKind::Note, note)];
                }
                let Some(game) = games.get(selected) else {
                    return vec![AppAction::Games(GamesAction::StartAdd)];
                };
                if game.profile == self.runtime.profile_name {
                    return vec![AppAction::SetDetailMode(DetailMode::Overview)];
                }
                if self.games.checking.as_deref() == Some(game.profile.as_str()) {
                    return vec![self.games_note(
                        StatusKind::Note,
                        &format!(
                            "Wait for the check of {} to finish first.",
                            game.display_name
                        ),
                    )];
                }
                vec![AppAction::Games(GamesAction::Switch(game.profile.clone()))]
            }
            GamesMessage::RequestRemove => {
                let Some(game) = games.get(selected) else {
                    return vec![];
                };
                let name = &game.display_name;
                if game.profile == self.runtime.profile_name {
                    vec![self.games_note(
                        StatusKind::Note,
                        &format!("{name} is the game in use. Switch to another game to remove it."),
                    )]
                } else if game.profile == DEFAULT_PROFILE {
                    vec![self.games_note(
                        StatusKind::Note,
                        &format!(
                            "{name} can't be removed: LemonUp commands without --profile use it."
                        ),
                    )]
                } else if let Some(note) = self.remove_busy_note(&game.profile, name) {
                    vec![self.games_note(StatusKind::Note, &note)]
                } else {
                    vec![AppAction::Games(GamesAction::SetPendingRemove(Some(
                        game.profile.clone(),
                    )))]
                }
            }
            GamesMessage::ConfirmRemove => match self.games.pending_remove.clone() {
                // A check may have started while the question was open.
                Some(profile) => {
                    match self.remove_busy_note(&profile, &self.game_display_name(&profile)) {
                        Some(note) => vec![
                            AppAction::Games(GamesAction::SetPendingRemove(None)),
                            self.games_note(StatusKind::Note, &note),
                        ],
                        None => vec![
                            AppAction::Games(GamesAction::SetPendingRemove(None)),
                            AppAction::Games(GamesAction::Remove(profile)),
                        ],
                    }
                }
                None => vec![],
            },
            GamesMessage::CancelRemove => {
                vec![AppAction::Games(GamesAction::SetPendingRemove(None))]
            }
            GamesMessage::Checked { profile, result } => {
                vec![AppAction::Games(GamesAction::CheckFinished {
                    profile,
                    result,
                })]
            }
        }
    }

    pub(super) fn apply_games(&mut self, action: GamesAction) {
        match action {
            GamesAction::Reload => {
                self.reload_games();
                self.games.selected = self
                    .games
                    .games
                    .iter()
                    .position(|game| game.profile == self.runtime.profile_name)
                    .unwrap_or(0);
                self.games.rows_reserved = self.games.games.len();
            }
            GamesAction::Select(index) => self.games.selected = index,
            GamesAction::SetPendingRemove(profile) => self.games.pending_remove = profile,
            GamesAction::Switch(profile) => {
                let name = self.game_display_name(&profile);
                if let Err(error) = self.switch_game(&profile) {
                    let status = self.games_note(
                        StatusKind::Problem,
                        &format!("Couldn't open {name}: {error}"),
                    );
                    self.apply(status);
                }
            }
            GamesAction::Remove(profile) => {
                let name = self.game_display_name(&profile);
                let status = match remove_profile(&self.roots, &profile, &self.runtime.profile_name)
                {
                    Ok(removed) => {
                        self.reload_games();
                        self.games.attempted.remove(&profile);
                        self.games.selected = self.games.selected.min(self.games.games.len());
                        self.games_note(
                            StatusKind::Done,
                            &format!(
                                "Removed {name} from LemonUp. Its backups stay in {}.",
                                friendly_path(&removed.backups_dir)
                            ),
                        )
                    }
                    Err(error) => self.games_note(
                        StatusKind::Problem,
                        &format!("Couldn't remove {name}: {error}"),
                    ),
                };
                self.apply(status);
            }
            GamesAction::StartAdd => self.start_adding_game(),
            GamesAction::Create(addon_dir) => self.create_game(&addon_dir),
            GamesAction::StartCheck(profile) => {
                let Ok(paths) = self.roots.paths(&profile) else {
                    return;
                };
                self.games
                    .attempted
                    .insert(profile.clone(), OffsetDateTime::now_utc());
                self.games.checking = Some(profile.clone());
                let sender = self.games_tx.clone();
                let env_key = self.wago_env_key.clone();
                tokio::spawn(async move {
                    let result = check_other_game(paths, env_key).await;
                    let _ = sender.send(OtherGameChecked { profile, result });
                });
            }
            GamesAction::CheckFinished { profile, result } => {
                if self.games.checking.as_deref() == Some(profile.as_str()) {
                    self.games.checking = None;
                }
                match result {
                    // A check that wrote results: read that game's counts again.
                    Ok(()) => self.reload_games(),
                    // A failed check keeps the counts that game had.
                    Err(error) => {
                        tracing::warn!(%error, %profile, "background check of another game failed");
                    }
                }
            }
        }
    }

    /// Reads the games again; a list that can't be read keeps the old one.
    pub(super) fn reload_games(&mut self) {
        if let Some(games) = read_games(&self.roots, &self.runtime.profile_name) {
            self.games.games = games;
        }
    }

    fn game_display_name(&self, profile: &str) -> String {
        self.games
            .games
            .iter()
            .find(|game| game.profile == profile)
            .map(|game| game.display_name.clone())
            .unwrap_or_else(|| profile.to_string())
    }

    /// Loads the app again for `profile`: its settings, addon list,
    /// backups and folder, the way `lemonup --profile` starts. The settings
    /// all games share come along by themselves; the background checks
    /// carry on. Only called while nothing runs that changes this game.
    pub(super) fn switch_game(&mut self, profile: &str) -> std::result::Result<(), String> {
        let paths = self
            .roots
            .paths(profile)
            .map_err(|error| error.to_string())?;
        paths.ensure().map_err(|error| error.to_string())?;
        let guarded = if profile == DEFAULT_PROFILE {
            Vec::new()
        } else {
            let default_paths = self
                .roots
                .paths(DEFAULT_PROFILE)
                .map_err(|error| error.to_string())?;
            crate::guarded_dirs_of(&default_paths).map_err(|error| error.to_string())?
        };
        let runtime =
            AppRuntime::new(profile.to_string(), None, guarded).with_display(self.runtime.display);
        let mut next = App::bootstrap(paths, runtime).map_err(|error| error.to_string())?;
        next.games.attempted = std::mem::take(&mut self.games.attempted);
        next.games.checking = self.games.checking.take();
        std::mem::swap(&mut next.games_tx, &mut self.games_tx);
        std::mem::swap(&mut next.games_rx, &mut self.games_rx);
        next.last_frame_width = self.last_frame_width;
        if let Err(error) = remember_last_used(&self.roots, profile) {
            tracing::warn!(%error, "couldn't remember the last used game");
        }
        *self = next;
        let name = self.current_game_name();
        let status = self.games_note(StatusKind::Done, &format!("Switched to {name}."));
        self.apply(status);
        Ok(())
    }

    /// AddOns folders of the games already set up, which adding a game
    /// does not offer again.
    pub(super) fn set_up_addon_dirs(&self) -> Vec<PathBuf> {
        list_profiles(&self.roots)
            .map(|summaries| {
                summaries
                    .into_iter()
                    .filter_map(|summary| summary.addon_dir)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Setup's game step, without the welcome, to pick another install.
    fn start_adding_game(&mut self) {
        self.shell_mode = ShellMode::Onboarding;
        self.dashboard.detail_mode = DetailMode::Overview;
        self.shell_ui.overlay.active = None;
        self.config_pane.edit = None;
        let mut config = self.config_pane.draft.clone();
        config.addon_dir = None;
        self.onboarding = OnboardingState::from_config(config);
        self.games.adding = true;
    }

    /// Leaves setup that was adding a game, back to the sheet, with nothing
    /// created.
    pub(super) fn cancel_adding_game(&mut self) {
        self.games.adding = false;
        self.onboarding = self.onboarding.set_quit_question(false);
        self.ui_theme = self.look_theme(&self.config_pane.draft);
        self.shell_mode = ShellMode::Dashboard;
        self.apply(AppAction::Games(GamesAction::Reload));
        self.apply(AppAction::SetDetailMode(DetailMode::Games));
    }

    /// Sets up the install at `addon_dir` as a new game and opens it. A
    /// second install of one version gets a numbered name.
    fn create_game(&mut self, addon_dir: &Path) {
        match create_numbered_profile(&self.roots, addon_dir) {
            Ok(profile) => {
                self.games.adding = false;
                if let Err(error) = self.switch_game(&profile) {
                    self.shell_mode = ShellMode::Dashboard;
                    let status = self.games_note(
                        StatusKind::Problem,
                        &format!("The game was added, but LemonUp couldn't open it: {error}"),
                    );
                    self.apply(status);
                }
            }
            Err(error) => {
                self.onboarding = self
                    .onboarding
                    .with_problem(format!("LemonUp couldn't add this game: {error}"));
            }
        }
    }

    /// The other game Setup's last step offers to set up with the one at
    /// `chosen`: the Retail or WoW Forever install the search found next to
    /// it, when no game uses it yet and this game may use it. Never when
    /// Setup is only adding a game, which picks one game already.
    pub(super) fn other_game_to_offer(&self, chosen: &Path) -> Option<PathBuf> {
        if self.games.adding {
            return None;
        }
        let OnboardingPhase::Found(found) = &self.onboarding.phase else {
            return None;
        };
        let wanted = match GameFlavor::detect_from_path(chosen)? {
            GameFlavor::Retail => GameFlavor::Forever,
            GameFlavor::Forever => GameFlavor::Retail,
            GameFlavor::Classic | GameFlavor::Cata => return None,
        };
        let set_up = self.set_up_addon_dirs();
        found
            .installs
            .iter()
            .map(|install| &install.addons_dir)
            .find(|dir| {
                GameFlavor::detect_from_path(dir) == Some(wanted)
                    && !lemonup_core::paths::same_addons_folder(dir, chosen)
                    && !self.is_guarded_path(dir)
                    && !set_up
                        .iter()
                        .any(|used| lemonup_core::paths::same_addons_folder(used, dir))
            })
            .cloned()
    }

    /// After the first game's setup was saved: sets up the other game too
    /// when step 3's "Also set up" was on, so it takes the shared settings
    /// chosen in Setup. The first game stays as it is if this fails; the
    /// screen says what happened either way.
    pub(super) fn set_up_other_game_after_save(&mut self) {
        let Some(addon_dir) = self
            .onboarding
            .other_game
            .clone()
            .filter(|_| self.onboarding.set_up_other_game && !self.games.adding)
        else {
            return;
        };
        // Once: a later save from this screen must not set it up again.
        self.onboarding = self.onboarding.with_other_game(None);
        let name = GameFlavor::detect_from_path(&addon_dir)
            .map(game_name)
            .unwrap_or("your other game");
        let status: (StatusKind, String) = match create_numbered_profile(&self.roots, &addon_dir) {
            Ok(_) => {
                self.reload_games();
                (
                    StatusKind::Done,
                    format!("Also set up {name}. Press p to switch between your games."),
                )
            }
            // The error comes last: a long one is cut, the advice is not.
            Err(error) => (
                StatusKind::Problem,
                format!(
                    "Your game is ready. LemonUp couldn't set up {name} (add it later with p): {error}"
                ),
            ),
        };
        let status = self.dashboard_status_for(status.0, self.dashboard.detail_mode, &status.1);
        self.apply(AppAction::SetStatus(status));
    }

    /// The question `x` asks, while it is open.
    pub(super) fn remove_game_question(&self) -> Option<dialog::Question> {
        let profile = self.games.pending_remove.as_deref()?;
        let backups = self
            .roots
            .paths(profile)
            .map(|paths| friendly_path(&paths.backup_root()))
            .unwrap_or_default();
        Some(dialog::Question::RemoveGame {
            game: self.game_display_name(profile),
            backups,
        })
    }

    /// The other game a background check is due for at `now`, if any: one
    /// at a time, at the "Check other games" interval, and never while this
    /// game checks, updates or changes its files.
    pub(super) fn other_game_check_due(&self, now: OffsetDateTime) -> Option<String> {
        let interval = self.config_pane.draft.check_other_games.interval()?;
        if self.shell_mode != ShellMode::Dashboard
            || self.games.checking.is_some()
            || self.switch_busy_note().is_some()
            || self.auto_check_in_progress
        {
            return None;
        }
        let interval = Duration::try_from(interval).ok()?;
        self.games
            .games
            .iter()
            .filter(|game| game.profile != self.runtime.profile_name && game.flavor.is_some())
            // A game being removed is never checked: the removal waits for a check.
            .filter(|game| self.games.pending_remove.as_deref() != Some(game.profile.as_str()))
            .find(|game| {
                let last = game
                    .last_checked
                    .into_iter()
                    .chain(self.games.attempted.get(&game.profile).copied())
                    .max();
                last.is_none_or(|last| now - last >= interval)
            })
            .map(|game| game.profile.clone())
    }

    /// The header's word on other games with updates: "Retail: ↑3 Update
    /// ready", or "Retail ↑3" when narrow.
    pub(super) fn other_games_badge(&self, wide: bool) -> Vec<Span<'static>> {
        let roles = self.ui_theme.roles;
        let mut spans = Vec::new();
        for game in &self.games.games {
            let ready = game.update_ready.unwrap_or(0);
            if game.profile == self.runtime.profile_name || ready == 0 {
                continue;
            }
            let amber = Style::default()
                .fg(roles.amber)
                .add_modifier(Modifier::BOLD);
            spans.push(Span::raw("   "));
            if wide {
                spans.push(Span::styled(
                    format!("{}: ", game.display_name),
                    roles.muted,
                ));
                spans.push(Span::styled(format!("↑{ready} Update ready"), amber));
            } else {
                spans.push(Span::styled(format!("{} ", game.display_name), roles.muted));
                spans.push(Span::styled(format!("↑{ready}"), amber));
            }
        }
        spans
    }

    /// The Switch game sheet: one row per game with its addons and updates,
    /// then "Add another game…", and what enter does on the row under the
    /// cursor. It keeps room for every game it opened with, so removing one
    /// does not move it.
    pub(super) fn games_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let width = room.width;
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let games = &self.games.games;
        let selected = self.games.selected.min(games.len());
        let intro = vec![Line::from(Span::styled(
            "Each game has its own addon list, backups and game folder.",
            muted,
        ))];
        let footer = vec![Line::from(""), self.games_hint_line(selected, width)];
        let rows = u16::try_from(self.games.rows_reserved.max(games.len())).unwrap_or(u16::MAX);
        let mut body = SheetBody::new();
        body.reserve(
            text_height(&intro, width)
                .saturating_add(rows)
                .saturating_add(4)
                .saturating_add(text_height(&footer, width)),
        );
        let name_width = games
            .iter()
            .map(|game| game.display_name.chars().count() + 2)
            .max()
            .unwrap_or(0)
            .clamp(12, NAME_WIDTH_MAX);
        body.text(intro).gap(1);
        body.text(vec![Line::from(Span::styled(
            format!(
                "  {:<name_width$}{:<ADDONS_WIDTH$}Updates",
                "Game", "Addons"
            ),
            muted.add_modifier(Modifier::BOLD),
        ))]);
        for (index, game) in games.iter().enumerate() {
            body.text(vec![self.games_row_line(
                game,
                index == selected,
                name_width,
            )]);
            if index == selected {
                body.focus_last();
            }
        }
        let add_selected = selected == games.len();
        let mut add_style = Style::default().fg(roles.text);
        if add_selected {
            add_style = add_style.add_modifier(Modifier::BOLD);
        }
        body.text(vec![Line::from(vec![
            Span::styled(
                if add_selected { "▌ " } else { "  " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled("Add another game…", add_style),
        ])]);
        if add_selected {
            body.focus_last();
        }
        body.footer(footer);
        body
    }

    fn games_row_line(&self, game: &GameEntry, selected: bool, name_width: usize) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let in_use = game.profile == self.runtime.profile_name;
        let mut name_style = Style::default().fg(roles.text);
        if selected {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let name = truncate_text(&game.display_name, name_width.saturating_sub(2));
        let addons = match game.addon_count {
            Some(count) => addon_count_label(count),
            None => "couldn't read".to_string(),
        };
        // The game in use counts from the list on screen.
        let update_ready = if in_use {
            let checked = self
                .dashboard
                .items
                .iter()
                .any(|item| item.last_checked_at.is_some());
            checked.then(|| self.dashboard.update_ready_folders().len())
        } else {
            game.update_ready
        };
        let updates = match update_ready {
            Some(0) => Span::styled(
                format!("{:<UPDATES_WIDTH$}", "Up to date"),
                Style::default().fg(roles.green),
            ),
            Some(count) => Span::styled(
                format!("{:<UPDATES_WIDTH$}", format!("↑{count} Update ready")),
                Style::default()
                    .fg(roles.amber)
                    .add_modifier(Modifier::BOLD),
            ),
            None => Span::styled(
                format!("{:<UPDATES_WIDTH$}", "Not checked yet"),
                roles.muted,
            ),
        };
        let mut spans = vec![
            Span::styled(
                if selected { "▌ " } else { "  " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(format!("{name:<name_width$}"), name_style),
            Span::styled(
                format!("{addons:<ADDONS_WIDTH$}"),
                Style::default().fg(roles.text),
            ),
            updates,
        ];
        if in_use {
            spans.push(Span::styled("current", roles.muted));
        }
        Line::from(spans)
    }

    /// What the keys do on the row under the cursor.
    fn games_hint_line(&self, selected: usize, width: u16) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let key = |label: &'static str| {
            Span::styled(
                label,
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            )
        };
        let spans = match self.games.games.get(selected) {
            None => vec![key("enter"), Span::styled(" add another game", muted)],
            Some(game) if game.profile == self.runtime.profile_name => {
                vec![Span::styled("This is your current game", muted)]
            }
            Some(game) if game.profile == DEFAULT_PROFILE => {
                vec![key("enter"), Span::styled(" select game", muted)]
            }
            Some(_) => vec![
                key("enter"),
                Span::styled(" select game, ", muted),
                key("x"),
                Span::styled(" remove from LemonUp", muted),
            ],
        };
        Line::from(fit_spans(spans, usize::from(width)))
    }
}

/// Creates a game for `addon_dir` named after its version (`retail`,
/// `forever`), or `forever-2` and on when that name is taken.
fn create_numbered_profile(
    roots: &ProfileRoots,
    addon_dir: &Path,
) -> std::result::Result<String, ProfileError> {
    let flavor = GameFlavor::detect_from_path(addon_dir);
    let base = flavor.map(profile_name_for).unwrap_or("game");
    let mut last_error = None;
    for number in 1..=9 {
        let name = if number == 1 {
            base.to_string()
        } else {
            format!("{base}-{number}")
        };
        match create_profile(roots, addon_dir, Some(&name)) {
            Ok(created) => return Ok(created.paths.profile),
            Err(ProfileError::NameTaken(taken)) => {
                last_error = Some(ProfileError::NameTaken(taken));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or(ProfileError::NameTaken(base.to_string())))
}
