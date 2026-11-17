//! "Find your addons online": the `F` sheet (and `f` on one addon).
//!
//! Three steps in one sheet that keeps its size: Start says what is sent
//! where, Looking reads the folders and asks the sources in the background,
//! and Results lists what was found the way `lemonup find` does. Linking
//! asks one question first; partial matches are replaced through the normal
//! Wago.io install, one at a time. Matching and linking live in
//! `lemonup_core::find`; the lookups in `crate::find`.

use std::collections::VecDeque;

use lemonup_core::find::{FindInputs, FindLabel, FindReport, FindResult, FindSource, FoundAddon};

use super::render_dashboard::fit_spans;
use super::sheet::{SheetBody, SheetRoom};
use super::*;
use crate::find::{LocalNames, SourceProblem};

/// Rows a page key moves in the results.
const PAGE_ROWS: usize = 8;
/// The widest the name column of the results gets.
const NAME_WIDTH_MAX: usize = 28;
/// The width of the source column of the results.
const SOURCE_WIDTH: usize = 14;
/// The index of "Not checked" in [`GROUPS`].
const NOT_CHECKED_GROUP: usize = 5;

/// `lines` word wrapped at `width`, each row after a line's first indented
/// by `indent`, so a long row continues under its own text and never at the
/// sheet's left edge.
fn hanging(lines: Vec<Line<'static>>, width: u16, indent: usize) -> Vec<Line<'static>> {
    super::sheet::wrap_lines_hanging(&lines, width, u16::try_from(indent).unwrap_or(0))
}

/// What the sheet shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindStep {
    /// What happens, and what is sent; enter starts.
    Start,
    /// Reading the folders (`asking` empty), then asking the sources.
    Looking {
        folders: usize,
        asking: Vec<FindSource>,
    },
    /// The lookup couldn't run; nothing changed.
    Failed(String),
    Results(FindResults),
}

/// A line of the results the cursor can be on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindFocus {
    /// A result, by its index in the report.
    Result(usize),
    /// The action under the results: link, or search by name.
    Action,
}

/// What a picked candidate does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickKind {
    /// Link it as it is on disk.
    Link,
    /// Install the source's copy over the differing folders.
    Replace,
}

/// What the lookup found and what is picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindResults {
    pub(crate) report: FindReport,
    pub(crate) problems: Vec<SourceProblem>,
    pub(crate) names: LocalNames,
    /// The picked candidate of each result, by report index.
    pub(crate) picks: Vec<Option<usize>>,
    pub(crate) cursor: FindFocus,
}

/// What the background lookup sends back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindOutcome {
    pub(crate) report: FindReport,
    pub(crate) problems: Vec<SourceProblem>,
    pub(crate) names: LocalNames,
}

/// What linking did, read back from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindLinkOutcome {
    /// The linked rows as the database holds them after the rescan.
    pub(crate) rows: Vec<AddonRecord>,
    /// Linked addons the database doesn't hold after all.
    pub(crate) unconfirmed: Vec<String>,
    /// The list after the rescan; `None` when that read failed.
    pub(super) sync: Option<AddonScanOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindTaskEvent {
    /// The folders are read; these sources are asked now.
    Read {
        generation: u64,
        folders: usize,
        asking: Vec<FindSource>,
    },
    Looked {
        generation: u64,
        result: std::result::Result<Box<FindOutcome>, String>,
    },
    Linked(std::result::Result<Box<FindLinkOutcome>, String>),
}

/// The sheet, the background jobs and the replacements still to run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FindState {
    /// `None` while the sheet is closed.
    pub(crate) step: Option<FindStep>,
    /// The parent folders looked up; `None` for every Not linked addon.
    pub(crate) scope: Option<Vec<String>>,
    /// `f` on one addon: the name to search Wago.io for when nothing is found.
    pub(crate) single: Option<String>,
    /// Bumped whenever a lookup starts or is cancelled, so a late answer is dropped.
    pub(crate) generation: u64,
    /// The link question is open.
    pub(crate) question: bool,
    pub(crate) question_offset: u16,
    /// How far the open question's folder list scrolls, as last drawn.
    pub(crate) question_max: std::cell::Cell<u16>,
    /// Linking runs in the background.
    pub(crate) linking: bool,
    /// Partial matches to replace from Wago.io after linking, one at a time.
    pub(crate) replacements: VecDeque<FoundAddon>,
    /// The name of the partial match the Wago.io install is replacing now.
    pub(crate) replacing: Option<String>,
}

impl FindState {
    pub(crate) fn looking(&self) -> bool {
        matches!(self.step, Some(FindStep::Looking { .. }))
    }

    pub(crate) fn results(&self) -> Option<&FindResults> {
        match &self.step {
            Some(FindStep::Results(results)) => Some(results),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindMessage {
    /// `F`: every Not linked addon, or the selected ones.
    Open,
    /// `f`: the Not linked addon under the cursor.
    OpenOne,
    /// Enter on Start.
    Start,
    /// Esc or q: closes, and cancels a lookup.
    Close,
    Move(isize),
    First,
    Last,
    /// Space: select or unselect the result under the cursor.
    Toggle,
    /// Enter in the results: pick in a "Choose one" row, or run the action.
    Enter,
    ConfirmLink,
    CancelLink,
    QuestionScroll(i16),
    Task(FindTaskEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindAction {
    Open {
        scope: Option<Vec<String>>,
        single: Option<String>,
    },
    Close,
    StartLookup,
    SetStep(FindStep),
    SetQuestion(bool),
    SetQuestionOffset(u16),
    /// Links `links` in the background, then replaces `replacements` one at a time.
    StartLink {
        links: Vec<FoundAddon>,
        replacements: Vec<FoundAddon>,
    },
    LinkFinished,
    /// Takes the next replacement off the queue.
    PopReplacement,
    /// Takes the next replacement off the queue: its install starts now.
    BeginReplacement,
    /// The running replacement's install is over.
    ReplacementFinished,
    /// Drops the replacements still to run.
    StopReplacements,
}

/// The name to search Wago.io for: WoW color codes removed, and `<...>` tag brackets dropped
/// while their words stay ("<DBM Core> Main Core" searches "DBM Core Main Core").
pub(crate) fn search_name(name: &str) -> String {
    let plain = lemonup_core::toc_edit::display_value(name);
    let spaced = plain.replace(['<', '>'], " ");
    let cleaned = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() { plain } else { cleaned }
}

/// What picking `candidate` does, when it can be picked.
pub(crate) fn pick_kind(candidate: &FoundAddon) -> Option<PickKind> {
    if !candidate.is_linkable() {
        return None;
    }
    match candidate.label {
        // Only Wago.io installs from a project ID.
        FindLabel::Partial if candidate.source == FindSource::Wago => Some(PickKind::Replace),
        FindLabel::Partial => None,
        _ => Some(PickKind::Link),
    }
}

/// The order the report lists results in, with the title of each group.
const GROUPS: [(FindLabel, &str); 9] = [
    (FindLabel::Exact, "Ready to link"),
    (FindLabel::Exact, "Can't link yet"),
    (FindLabel::Likely, "Likely matches"),
    (FindLabel::Partial, "Partial matches"),
    (FindLabel::ChooseOne, "Choose one"),
    (FindLabel::NotChecked, "Not checked"),
    (
        FindLabel::CurseForgeOnly,
        "On CurseForge only, LemonUp can't update these yet",
    ),
    (FindLabel::NotFound, "Not found"),
    (FindLabel::DeveloperCopy, "Skipped, developer copy"),
];

impl FindResults {
    pub(crate) fn new(outcome: FindOutcome) -> Self {
        let picks = outcome
            .report
            .results
            .iter()
            .map(|result| result.preselected().then_some(0))
            .collect();
        let mut results = Self {
            report: outcome.report,
            problems: outcome.problems,
            names: outcome.names,
            picks,
            cursor: FindFocus::Action,
        };
        results.cursor = results
            .focusables(false)
            .first()
            .copied()
            .unwrap_or(FindFocus::Action);
        results
    }

    /// The results of group `index` of [`GROUPS`], in the report's order.
    pub(crate) fn group(&self, index: usize) -> Vec<usize> {
        let (label, _) = GROUPS[index];
        let mut members = self
            .report
            .results
            .iter()
            .enumerate()
            .filter(|(_, result)| result.label == label)
            .filter(|(_, result)| match index {
                0 => result.preselected(),
                1 => !result.preselected(),
                _ => true,
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if label == FindLabel::ChooseOne {
            members.sort_by_key(|index| self.name_of(*index).to_lowercase());
        }
        members
    }

    pub(crate) fn name_of(&self, index: usize) -> String {
        let result = &self.report.results[index];
        self.names.name(&result.folder, &result.name)
    }

    /// Whether a result can be picked at all.
    pub(crate) fn selectable(&self, index: usize) -> bool {
        let result = &self.report.results[index];
        match result.label {
            FindLabel::Exact | FindLabel::Likely | FindLabel::Partial => {
                result.found().and_then(pick_kind).is_some()
            }
            FindLabel::ChooseOne => result
                .candidates
                .iter()
                .any(|candidate| pick_kind(candidate).is_some()),
            _ => false,
        }
    }

    /// The candidates picked, with what each does.
    pub(crate) fn picked(&self) -> Vec<(&FoundAddon, PickKind)> {
        self.picks
            .iter()
            .enumerate()
            .filter_map(|(index, pick)| {
                let candidate = self.report.results[index].candidates.get((*pick)?)?;
                Some((candidate, pick_kind(candidate)?))
            })
            .collect()
    }

    pub(crate) fn counts(&self) -> (usize, usize) {
        let picked = self.picked();
        let links = picked
            .iter()
            .filter(|(_, kind)| *kind == PickKind::Link)
            .count();
        (links, picked.len() - links)
    }

    /// Whether any result has a candidate to pick.
    pub(crate) fn any_selectable(&self) -> bool {
        (0..self.report.results.len()).any(|index| self.selectable(index))
    }

    /// The lines the cursor stops on, top to bottom. `search` is whether the
    /// action row offers a name search.
    pub(crate) fn focusables(&self, search: bool) -> Vec<FindFocus> {
        let mut focus = (0..GROUPS.len())
            .flat_map(|group| self.group(group))
            .filter(|index| self.selectable(*index))
            .map(FindFocus::Result)
            .collect::<Vec<_>>();
        if search || self.any_selectable() {
            focus.push(FindFocus::Action);
        }
        focus
    }
}

impl App {
    /// Whether `f` should offer a name search: it looked up one addon and
    /// found nothing to pick.
    pub(super) fn find_offers_search(&self) -> bool {
        self.find.single.is_some()
            && self
                .find
                .results()
                .is_some_and(|results| !results.any_selectable())
    }

    /// The Not linked addons `F` looks up: the selected ones, or all.
    /// `Err` with a note when there are none.
    fn find_scope(&self) -> std::result::Result<Option<Vec<String>>, &'static str> {
        let not_linked = |item: &&DashboardItem| item.source == SourceKind::Manual;
        if self.dashboard.selected_parent_count() > 0 {
            let selected = self
                .dashboard
                .items
                .iter()
                .filter(not_linked)
                .filter(|item| self.dashboard.is_parent_selected(&item.folder))
                .map(|item| item.folder.clone())
                .collect::<Vec<_>>();
            return if selected.is_empty() {
                Err("None of the selected addons is Not linked.")
            } else {
                Ok(Some(selected))
            };
        }
        if self.dashboard.items.iter().any(|item| not_linked(&item)) {
            Ok(None)
        } else if matches!(self.scan_state, ScanState::Failed(_)) {
            Err("LemonUp couldn't read your AddOns folder, so there is nothing to look up.")
        } else if matches!(self.scan_state, ScanState::Pending | ScanState::Running(_)) {
            Err("Wait for LemonUp to read your AddOns folder first.")
        } else if self.dashboard.items.is_empty() {
            Err("Your AddOns folder has no addons to look up.")
        } else {
            Err("Every addon is linked already.")
        }
    }

    /// The Not linked addons in the scope, as the list shows them.
    pub(super) fn find_scope_items(&self) -> Vec<&DashboardItem> {
        self.dashboard
            .items
            .iter()
            .filter(|item| item.source == SourceKind::Manual)
            .filter(|item| {
                self.find.scope.as_ref().is_none_or(|scope| {
                    scope
                        .iter()
                        .any(|folder| folder.eq_ignore_ascii_case(&item.folder))
                })
            })
            .collect()
    }

    fn find_note(&self, kind: StatusKind, text: &str) -> AppAction {
        AppAction::SetStatus(self.dashboard_status_for(kind, self.dashboard.detail_mode, text))
    }

    fn set_find_step(&self, step: FindStep) -> AppAction {
        AppAction::Find(FindAction::SetStep(step))
    }

    pub(super) fn find_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::Find {
            return None;
        }
        let message = |message| Some(vec![AppMessage::Find(message)]);
        let step = self.find.step.as_ref()?;
        match (step, key.code) {
            (_, KeyCode::Char('?')) => Some(vec![AppMessage::DashboardOpenHelp]),
            (_, KeyCode::Esc | KeyCode::Char('q')) => message(FindMessage::Close),
            (FindStep::Start, KeyCode::Enter) => message(FindMessage::Start),
            (FindStep::Results(results), code) => {
                let has_rows = !results.focusables(self.find_offers_search()).is_empty();
                match code {
                    KeyCode::Down | KeyCode::Char('j') if has_rows => message(FindMessage::Move(1)),
                    KeyCode::Up | KeyCode::Char('k') if has_rows => message(FindMessage::Move(-1)),
                    KeyCode::PageDown if has_rows => message(FindMessage::Move(PAGE_ROWS as isize)),
                    KeyCode::PageUp if has_rows => {
                        message(FindMessage::Move(-(PAGE_ROWS as isize)))
                    }
                    KeyCode::Home if has_rows => message(FindMessage::First),
                    KeyCode::End if has_rows => message(FindMessage::Last),
                    KeyCode::Char(' ') if self.find_space_works() => message(FindMessage::Toggle),
                    KeyCode::Enter if self.find_enter_works() => message(FindMessage::Enter),
                    // The keys of the list never reach it behind the sheet.
                    _ => Some(vec![]),
                }
            }
            _ => Some(vec![]),
        }
    }

    /// Space selects the row under the cursor: a match, not a choice.
    pub(super) fn find_space_works(&self) -> bool {
        let Some(results) = self.find.results() else {
            return false;
        };
        match results.cursor {
            FindFocus::Result(index) => {
                results.report.results[index].label != FindLabel::ChooseOne
                    && results.selectable(index)
            }
            FindFocus::Action => false,
        }
    }

    /// Enter picks in a "Choose one" row, and runs the action when it has
    /// something to do.
    pub(super) fn find_enter_works(&self) -> bool {
        let Some(results) = self.find.results() else {
            return false;
        };
        match results.cursor {
            FindFocus::Result(index) => results.report.results[index].label == FindLabel::ChooseOne,
            FindFocus::Action => self.find_offers_search() || !results.picked().is_empty(),
        }
    }

    pub(super) fn find_actions(&self, message: FindMessage) -> Vec<AppAction> {
        match message {
            FindMessage::Open => match self.find_scope() {
                Ok(scope) => vec![AppAction::Find(FindAction::Open {
                    scope,
                    single: None,
                })],
                Err(note) => vec![self.find_note(StatusKind::Note, note)],
            },
            FindMessage::OpenOne => match self.not_linked_current_item() {
                Some(item) => vec![AppAction::Find(FindAction::Open {
                    scope: Some(vec![item.folder.clone()]),
                    single: Some(item.name.clone()),
                })],
                None => vec![],
            },
            FindMessage::Start => {
                if self.effective_addon_dir.is_none() {
                    return vec![
                        self.set_find_step(FindStep::Failed(
                            "LemonUp has no AddOns folder for this game yet. Pick one in Settings."
                                .to_string(),
                        )),
                    ];
                }
                // An update, install or other job that changes the addons runs.
                if let Some(note) = self.switch_busy_note() {
                    return vec![self.find_note(StatusKind::Note, note)];
                }
                vec![AppAction::Find(FindAction::StartLookup)]
            }
            FindMessage::Close => vec![AppAction::Find(FindAction::Close)],
            FindMessage::Move(delta) => {
                self.find_move(|position, last| position.saturating_add_signed(delta).min(last))
            }
            FindMessage::First => self.find_move(|_, _| 0),
            FindMessage::Last => self.find_move(|_, last| last),
            FindMessage::Toggle => {
                let Some(results) = self.find.results() else {
                    return vec![];
                };
                let FindFocus::Result(index) = results.cursor else {
                    return vec![];
                };
                let mut next = results.clone();
                next.picks[index] = match next.picks[index] {
                    Some(_) => None,
                    None => Some(0),
                };
                vec![self.set_find_step(FindStep::Results(next))]
            }
            FindMessage::Enter => self.find_enter_actions(),
            FindMessage::ConfirmLink => self.find_confirm_actions(),
            FindMessage::CancelLink => {
                let replaces = self
                    .find
                    .results()
                    .is_some_and(|results| results.counts().1 > 0);
                vec![
                    AppAction::Find(FindAction::SetQuestion(false)),
                    self.find_note(
                        StatusKind::Quiet,
                        if replaces {
                            "Nothing was changed."
                        } else {
                            "Nothing was linked."
                        },
                    ),
                ]
            }
            FindMessage::QuestionScroll(delta) => {
                let max = self.find.question_max.get();
                let offset = self
                    .find
                    .question_offset
                    .saturating_add_signed(delta)
                    .min(max);
                vec![AppAction::Find(FindAction::SetQuestionOffset(offset))]
            }
            FindMessage::Task(event) => self.find_task_actions(event),
        }
    }

    fn find_move(&self, to: impl Fn(usize, usize) -> usize) -> Vec<AppAction> {
        let Some(results) = self.find.results() else {
            return vec![];
        };
        let focus = results.focusables(self.find_offers_search());
        if focus.is_empty() {
            return vec![];
        }
        let position = focus
            .iter()
            .position(|focus| *focus == results.cursor)
            .unwrap_or(0);
        let mut next = results.clone();
        next.cursor = focus[to(position, focus.len() - 1)];
        vec![self.set_find_step(FindStep::Results(next))]
    }

    fn find_enter_actions(&self) -> Vec<AppAction> {
        let Some(results) = self.find.results() else {
            return vec![];
        };
        match results.cursor {
            // The next candidate that can be picked, then none.
            FindFocus::Result(index) => {
                let result = &results.report.results[index];
                let start = results.picks[index].map_or(0, |pick| pick + 1);
                let next_pick = (start..result.candidates.len())
                    .find(|candidate| pick_kind(&result.candidates[*candidate]).is_some());
                let mut next = results.clone();
                next.picks[index] = next_pick;
                vec![self.set_find_step(FindStep::Results(next))]
            }
            FindFocus::Action if self.find_offers_search() => self.find_search_by_name_actions(),
            FindFocus::Action if results.picked().is_empty() => vec![],
            FindFocus::Action => vec![
                AppAction::Find(FindAction::SetQuestionOffset(0)),
                AppAction::Find(FindAction::SetQuestion(true)),
            ],
        }
    }

    /// The old `f`: Install, with the addon's name searched on Wago.io.
    fn find_search_by_name_actions(&self) -> Vec<AppAction> {
        let Some(name) = self.find.single.as_deref() else {
            return vec![];
        };
        let pane = self.search_pane.open_prefilled(&search_name(name));
        let mut actions = vec![
            AppAction::Find(FindAction::Close),
            AppAction::SetDetailMode(DetailMode::Search),
            AppAction::SetSearchPaneState(pane.clone()),
        ];
        actions.extend(self.search_submit_actions(pane, self.wago_api_key.clone()));
        actions
    }

    fn find_confirm_actions(&self) -> Vec<AppAction> {
        let Some(results) = self.find.results() else {
            return vec![AppAction::Find(FindAction::SetQuestion(false))];
        };
        // A job that changes the addons may have started meanwhile.
        if let Some(note) = self.switch_busy_note() {
            return vec![
                AppAction::Find(FindAction::SetQuestion(false)),
                self.find_note(StatusKind::Note, note),
            ];
        }
        let mut links = Vec::new();
        let mut replacements = Vec::new();
        for (candidate, kind) in results.picked() {
            match kind {
                PickKind::Link => links.push(candidate.clone()),
                PickKind::Replace => replacements.push(candidate.clone()),
            }
        }
        if !replacements.is_empty() && self.wago_api_key.is_none() {
            return vec![
                AppAction::Find(FindAction::SetQuestion(false)),
                self.find_note(
                    StatusKind::Problem,
                    "Replacing with the copy from Wago.io needs your Wago.io key. Add one in Settings.",
                ),
            ];
        }
        vec![
            AppAction::Find(FindAction::SetQuestion(false)),
            AppAction::Find(FindAction::Close),
            AppAction::Find(FindAction::StartLink {
                links,
                replacements,
            }),
        ]
    }

    fn find_task_actions(&self, event: FindTaskEvent) -> Vec<AppAction> {
        match event {
            FindTaskEvent::Read {
                generation,
                folders,
                asking,
            } if generation == self.find.generation && self.find.looking() => {
                vec![self.set_find_step(FindStep::Looking { folders, asking })]
            }
            FindTaskEvent::Looked { generation, result }
                if generation == self.find.generation && self.find.looking() =>
            {
                vec![self.set_find_step(match result {
                    Ok(outcome) => FindStep::Results(FindResults::new(*outcome)),
                    Err(error) => FindStep::Failed(error),
                })]
            }
            // A lookup that was cancelled: its answer changes nothing.
            FindTaskEvent::Read { .. } | FindTaskEvent::Looked { .. } => vec![],
            FindTaskEvent::Linked(result) => {
                let mut actions = vec![AppAction::Find(FindAction::LinkFinished)];
                match result {
                    Ok(outcome) => {
                        if let Some(sync) = outcome.sync.clone() {
                            actions.extend([
                                AppAction::ReplaceDashboardAddons(sync.addons),
                                AppAction::SetDashboardDriftReport(Some(sync.drift_report)),
                                AppAction::CompleteAddonScan {
                                    path: sync.path,
                                    summary: sync.summary,
                                },
                            ]);
                        }
                        let (kind, text) = linked_status(&outcome);
                        actions.push(self.find_note(kind, &text));
                    }
                    // Replacements wait for the links; none runs after a failure.
                    Err(error) => {
                        let what = if self.find.replacements.is_empty() {
                            "Nothing was linked"
                        } else {
                            "Nothing was linked or replaced"
                        };
                        actions.extend([
                            AppAction::Find(FindAction::StopReplacements),
                            self.find_note(
                                StatusKind::Problem,
                                &format!("{what}: {}.", error.trim_end_matches('.')),
                            ),
                        ]);
                    }
                }
                actions
            }
        }
    }

    /// Starts the next partial replacement once nothing else installs or
    /// asks: through the normal Wago.io install, which asks again when the
    /// download brings another folder in the way.
    pub(super) fn find_replacement_actions(&self) -> Vec<AppAction> {
        let Some(next) = self.find.replacements.front() else {
            return vec![];
        };
        if self.find.linking
            || self.pending_wago_install_confirmation.is_some()
            || self.install_busy_status().is_some()
        {
            return vec![];
        }
        let Some(addon_dir) = self.effective_addon_dir.clone() else {
            return vec![AppAction::Find(FindAction::PopReplacement)];
        };
        let Some(api_key) = self.wago_api_key.clone() else {
            return vec![
                AppAction::Find(FindAction::PopReplacement),
                self.find_note(
                    StatusKind::Problem,
                    &format!("{} wasn't replaced: it needs your Wago.io key.", next.name),
                ),
            ];
        };
        let target = if next.source_url.is_empty() {
            next.source_id.clone()
        } else {
            next.source_url.clone()
        };
        vec![
            AppAction::Find(FindAction::BeginReplacement),
            AppAction::SetWagoInstallInProgress(true),
            AppAction::StartWagoInstall {
                addon_dir,
                api_key,
                request: PendingWagoInstallRequest {
                    target,
                    source: WagoInstallSource::SearchResult,
                },
                replace_folders: Some(next.folders.clone()),
            },
            self.find_note(
                StatusKind::Working,
                &format!("Replacing {} with the copy from Wago.io", next.name),
            ),
        ]
    }

    /// The Wago.io install of a replacement failed: says which addon and
    /// why, and stops, so the next one doesn't fail the same way unseen.
    pub(super) fn find_replacement_failed_actions(&self, error: &SourceError) -> Vec<AppAction> {
        let name = self.find.replacing.clone().unwrap_or_default();
        let advice = super::render_install_search::install_error_advice(
            error.kind,
            "Wago.io",
            error.clash.as_ref(),
        );
        let mut text = format!("{name} wasn't replaced: {advice} Its folders weren't changed.");
        if let Some(rest) = self.replacements_left() {
            text.push_str(&format!(
                " LemonUp stopped there, so {rest} replaced either."
            ));
        }
        vec![
            AppAction::SetWagoInstallInProgress(false),
            AppAction::Find(FindAction::ReplacementFinished),
            AppAction::Find(FindAction::StopReplacements),
            self.find_note(StatusKind::Problem, &text),
        ]
    }

    /// The replacement installed and is recorded, but LemonUp couldn't read
    /// the list again. Says it was replaced, and stops: the next one would
    /// run against a folder LemonUp can't read.
    pub(super) fn find_replacement_unrefreshed_actions(&self, reason: &str) -> Vec<AppAction> {
        let name = self.find.replacing.clone().unwrap_or_default();
        let mut text = format!("{name} was replaced. {}", super::unrefreshed_text(reason));
        if let Some(rest) = self.replacements_left() {
            text.push_str(&format!(" LemonUp stopped there, so {rest} replaced yet."));
        }
        vec![
            AppAction::SetWagoInstallInProgress(false),
            AppAction::Find(FindAction::ReplacementFinished),
            AppAction::Find(FindAction::StopReplacements),
            self.find_note(StatusKind::Problem, &text),
        ]
    }

    /// "Details wasn't" or "A, B weren't": the replacements still waiting.
    fn replacements_left(&self) -> Option<String> {
        let rest = self
            .find
            .replacements
            .iter()
            .map(|replacement| replacement.name.as_str())
            .collect::<Vec<_>>();
        (!rest.is_empty()).then(|| {
            format!(
                "{} {}",
                rest.join(", "),
                if rest.len() == 1 { "wasn't" } else { "weren't" }
            )
        })
    }

    pub(super) fn apply_find(&mut self, action: FindAction) {
        match action {
            FindAction::Open { scope, single } => {
                self.find.step = Some(FindStep::Start);
                self.find.scope = scope;
                self.find.single = single;
                self.find.question = false;
                self.apply(AppAction::SetDetailMode(DetailMode::Find));
            }
            FindAction::Close => {
                // A lookup still running answers into nothing.
                if self.find.looking() {
                    self.find.generation += 1;
                }
                self.find.step = None;
                self.find.question = false;
                if self.dashboard.detail_mode == DetailMode::Find {
                    self.apply(AppAction::SetDetailMode(DetailMode::Overview));
                }
            }
            FindAction::StartLookup => self.start_find_lookup(),
            FindAction::SetStep(step) => {
                if self.find.step.is_some() {
                    self.find.step = Some(step);
                }
            }
            FindAction::SetQuestion(open) => self.find.question = open,
            FindAction::SetQuestionOffset(offset) => self.find.question_offset = offset,
            FindAction::StartLink {
                links,
                replacements,
            } => {
                self.find.replacements = replacements.into();
                if links.is_empty() {
                    return;
                }
                let Some(addon_dir) = self.effective_addon_dir.clone() else {
                    return;
                };
                self.find.linking = true;
                let status = self.find_note(
                    StatusKind::Working,
                    &format!("Linking {}", addon_count_label(links.len())),
                );
                self.apply(status);
                let sender = self.task_events_tx.clone();
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        link_found(&state_db_file, &addon_dir, &links).map(Box::new)
                    })
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()));
                    let _ = sender.send(AppTaskEvent::Find(FindTaskEvent::Linked(result)));
                });
            }
            FindAction::LinkFinished => self.find.linking = false,
            FindAction::PopReplacement => {
                self.find.replacements.pop_front();
            }
            FindAction::BeginReplacement => {
                self.find.replacing = self
                    .find
                    .replacements
                    .pop_front()
                    .map(|replacement| replacement.name);
            }
            FindAction::ReplacementFinished => self.find.replacing = None,
            FindAction::StopReplacements => self.find.replacements.clear(),
        }
    }

    fn start_find_lookup(&mut self) {
        let Some(addon_dir) = self.effective_addon_dir.clone() else {
            return;
        };
        self.find.generation += 1;
        let generation = self.find.generation;
        let folders = self
            .find_scope_items()
            .iter()
            .map(|item| 1 + item.owned_folders.len())
            .sum();
        self.find.step = Some(FindStep::Looking {
            folders,
            asking: Vec::new(),
        });
        let sender = self.task_events_tx.clone();
        let state_db_file = self.state_db_file.clone();
        let scope = self.find.scope.clone();
        let api_key = self.wago_api_key.clone();
        let cache_dir = self
            .roots
            .paths(&self.runtime.profile_name)
            .map(|paths| paths.cache_dir)
            .unwrap_or_else(|_| self.data_dir.join("cache"));
        tokio::spawn(async move {
            let read_sender = sender.clone();
            let read_scope = scope.clone();
            let read_dir = addon_dir.clone();
            let read =
                tokio::task::spawn_blocking(move || read_find_inputs(&state_db_file, &read_dir))
                    .await
                    .unwrap_or_else(|error| {
                        Err(format!("The lookup stopped unexpectedly: {error}"))
                    });
            let (rows, inputs) = match read {
                Ok(read) => read,
                Err(error) => {
                    let _ = sender.send(AppTaskEvent::Find(FindTaskEvent::Looked {
                        generation,
                        result: Err(error),
                    }));
                    return;
                }
            };
            let wago_lookups = inputs.wago_lookups(&rows, read_scope.as_deref());
            let mut asking = Vec::new();
            if api_key.is_some() && !wago_lookups.is_empty() {
                asking.push(FindSource::Wago);
            }
            asking.extend([FindSource::WowInterface, FindSource::Tukui]);
            let _ = read_sender.send(AppTaskEvent::Find(FindTaskEvent::Read {
                generation,
                folders: inputs.folders.len(),
                asking,
            }));
            let lookups = crate::find::lookup_sources(
                &wago_lookups,
                inputs.flavor,
                api_key.as_deref(),
                &cache_dir,
            )
            .await;
            let outcome = find_outcome(&inputs, &rows, scope.as_deref(), lookups);
            let _ = sender.send(AppTaskEvent::Find(FindTaskEvent::Looked {
                generation,
                result: Ok(Box::new(outcome)),
            }));
        });
    }

    /// The question enter on "Link" asks, while it is open.
    pub(super) fn find_question(&self) -> Option<dialog::Question> {
        if !self.find.question {
            return None;
        }
        let results = self.find.results()?;
        let mut links = 0;
        let mut replaces = 0;
        let mut taken = Vec::new();
        let mut replaced = Vec::new();
        let mut added = Vec::new();
        for (candidate, kind) in results.picked() {
            match kind {
                PickKind::Link => {
                    links += 1;
                    taken.extend(candidate.folders.iter().cloned());
                }
                PickKind::Replace => {
                    replaces += 1;
                    replaced.extend(candidate.folders.iter().cloned());
                    // The source's folders that aren't on disk are new.
                    added.extend(candidate.missing.iter().cloned());
                }
            }
        }
        Some(dialog::Question::LinkFound {
            links,
            replaces,
            taken,
            replaced,
            added,
            offset: self.find.question_offset,
        })
    }

    /// The sheet's body for the step it is at. Every step keeps room for the
    /// tallest one, so the sheet holds still.
    pub(super) fn find_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let roles = self.ui_theme.roles;
        let width = room.width;
        let text = Style::default().fg(roles.text);
        let muted = Style::default().fg(roles.muted);
        let mut body = SheetBody::new();
        body.reserve(room.height);
        let ellipsis = self.ui_theme.symbols.ellipsis;
        match self.find.step.as_ref() {
            None => {}
            Some(FindStep::Start) => {
                let what = match &self.find.single {
                    Some(name) => format!("LemonUp looks up {name}"),
                    None => {
                        let count = self.find_scope_items().len();
                        format!(
                            "LemonUp looks up {count} Not linked {}",
                            if count == 1 { "addon" } else { "addons" }
                        )
                    }
                };
                // Wago.io is asked only with a key.
                let sources = if self.wago_api_key.is_some() {
                    "Wago.io, WoWInterface and TukUI"
                } else {
                    "WoWInterface and TukUI"
                };
                body.text(vec![Line::from(Span::styled(
                    format!(
                        "{what} on {sources}, and shows what it finds. \
                         Nothing changes until you say yes."
                    ),
                    text,
                ))]);
                body.gap(1);
                let note = if self.wago_api_key.is_some() {
                    "Sends folder names and file hashes to Wago.io."
                } else {
                    "A Wago.io key finds more of your addons. Add one in Settings."
                };
                body.text(vec![Line::from(Span::styled(note, muted))]);
                body.footer(
                    self.find_hint_footer(vec![("enter", " look up, "), ("esc", " close")], width),
                );
            }
            Some(FindStep::Looking { folders, asking }) => {
                let line = if asking.is_empty() {
                    format!("Reading {folders} folders{ellipsis}")
                } else {
                    format!(
                        "Asking {}{ellipsis}",
                        asking
                            .iter()
                            .map(|source| source.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                body.text(vec![Line::from(Span::styled(line, text))]);
                body.footer(
                    self.find_hint_footer(vec![("esc", " cancel, nothing changes")], width),
                );
            }
            Some(FindStep::Failed(error)) => {
                body.text(vec![Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(roles.red),
                ))]);
                body.gap(1);
                body.text(vec![Line::from(Span::styled(
                    "Nothing was changed.",
                    muted,
                ))]);
                body.footer(self.find_hint_footer(vec![("esc", " close")], width));
            }
            Some(FindStep::Results(results)) => self.find_results_body(&mut body, results, width),
        }
        body
    }

    fn find_hint_footer(
        &self,
        keys: Vec<(&'static str, &'static str)>,
        width: u16,
    ) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let mut spans = Vec::new();
        for (key, text) in keys {
            spans.push(Span::styled(
                key,
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(text, Style::default().fg(roles.muted)));
        }
        vec![
            Line::from(""),
            Line::from(fit_spans(spans, usize::from(width))),
        ]
    }

    fn find_results_body(&self, body: &mut SheetBody<'static>, results: &FindResults, width: u16) {
        let roles = self.ui_theme.roles;
        let text = Style::default().fg(roles.text);
        let muted = Style::default().fg(roles.muted);
        let names = &results.names;
        let name_width = results
            .report
            .results
            .iter()
            .enumerate()
            .filter(|(_, result)| result.found().is_some())
            .map(|(index, _)| results.name_of(index).chars().count())
            .max()
            .unwrap_or(0)
            .min(NAME_WIDTH_MAX);
        let report = &results.report;
        if report.skipped.contains(&FindSource::Wago) {
            body.text(vec![Line::from(Span::styled(
                "A Wago.io key finds more of your addons. Add one in Settings.",
                muted,
            ))]);
            body.gap(1);
        }
        if report.results.is_empty() {
            body.text(vec![Line::from(Span::styled(
                "No addons to look up.",
                text,
            ))]);
        }
        let not_checked_by = report
            .results
            .iter()
            .filter(|result| result.label == FindLabel::NotChecked)
            .flat_map(|result| result.not_checked_by.iter().copied())
            .collect::<Vec<_>>();
        for (group, (label, title)) in GROUPS.iter().enumerate() {
            // A source that failed outright comes right after "Not checked", as in the CLI.
            if group == NOT_CHECKED_GROUP + 1 {
                self.find_check_failed_lines(body, results, &not_checked_by, width);
            }
            let members = results.group(group);
            if members.is_empty() {
                continue;
            }
            let tone = match group {
                0 => roles.green,
                NOT_CHECKED_GROUP => roles.red,
                6..=8 => roles.muted,
                _ => roles.amber,
            };
            body.text(vec![Line::from(vec![
                Span::styled(
                    title.to_string(),
                    Style::default().fg(tone).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({})", members.len()), muted),
            ])]);
            match label {
                FindLabel::Exact | FindLabel::Likely | FindLabel::Partial => {
                    for index in members {
                        self.find_entry_lines(body, results, index, name_width, width);
                    }
                }
                FindLabel::ChooseOne => {
                    for index in members {
                        self.find_choice_lines(body, results, index, width);
                    }
                }
                _ => {
                    let listed = members
                        .iter()
                        .map(|index| &report.results[*index])
                        .collect::<Vec<&FindResult>>();
                    body.text(hanging(
                        vec![Line::from(Span::styled(
                            format!("  {}", crate::find::listed_names(&listed, names).join(", ")),
                            text,
                        ))],
                        width,
                        2,
                    ));
                    if *label == FindLabel::NotChecked {
                        for problem in results
                            .problems
                            .iter()
                            .filter(|problem| not_checked_by.contains(&problem.source))
                        {
                            body.text(hanging(
                                vec![Line::from(Span::styled(
                                    format!("  {}", crate::find::problem_text(problem)),
                                    muted,
                                ))],
                                width,
                                2,
                            ));
                        }
                    }
                }
            }
            body.gap(1);
        }
        body.footer(self.find_results_footer(results, width));
    }

    /// "Check failed": the sources that failed without leaving an addon
    /// "Not checked".
    fn find_check_failed_lines(
        &self,
        body: &mut SheetBody<'static>,
        results: &FindResults,
        not_checked_by: &[FindSource],
        width: u16,
    ) {
        let roles = self.ui_theme.roles;
        let unexplained = results
            .problems
            .iter()
            .filter(|problem| !not_checked_by.contains(&problem.source))
            .collect::<Vec<_>>();
        if unexplained.is_empty() {
            return;
        }
        body.text(vec![Line::from(Span::styled(
            format!("Check failed ({})", unexplained.len()),
            Style::default().fg(roles.red).add_modifier(Modifier::BOLD),
        ))]);
        for problem in unexplained {
            body.text(hanging(
                vec![Line::from(Span::styled(
                    format!("  {}", crate::find::problem_text(problem)),
                    Style::default().fg(roles.muted),
                ))],
                width,
                2,
            ));
        }
        body.gap(1);
    }

    fn find_mark(&self, picked: bool, selectable: bool) -> &'static str {
        match (selectable, picked) {
            (false, _) => "  ",
            (true, true) => "● ",
            (true, false) => "○ ",
        }
    }

    fn find_entry_lines(
        &self,
        body: &mut SheetBody<'static>,
        results: &FindResults,
        index: usize,
        name_width: usize,
        width: u16,
    ) {
        let roles = self.ui_theme.roles;
        let muted = Style::default().fg(roles.muted);
        let result = &results.report.results[index];
        let Some(entry) = crate::find::result_entry(result, &results.names) else {
            return;
        };
        let cursor = results.cursor == FindFocus::Result(index);
        let selectable = results.selectable(index);
        let picked = results.picks[index].is_some();
        let mut name_style = Style::default().fg(roles.text);
        if cursor {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let name = truncate_text(&entry.name, name_width);
        let mut spans = vec![
            Span::styled(
                if cursor { "▌" } else { " " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(
                self.find_mark(picked, selectable),
                Style::default().fg(roles.lemon),
            ),
            Span::styled(format!("{name:<name_width$}  "), name_style),
            Span::styled(format!("{:<SOURCE_WIDTH$}", entry.source), muted),
            Span::styled(entry.version.clone(), Style::default().fg(roles.text)),
        ];
        if let Some(update) = &entry.update {
            spans.push(Span::styled(
                format!("  {update}"),
                Style::default().fg(roles.amber),
            ));
        }
        // A long version or reason continues under itself, or under the name.
        let version_column = 3 + name_width + 2 + SOURCE_WIDTH;
        let indent = if version_column < usize::from(width) / 2 {
            version_column
        } else {
            3
        };
        body.text(hanging(vec![Line::from(spans)], width, indent));
        if cursor {
            body.focus_last();
        }
        let mut notes = entry.notes.clone();
        if result.label == FindLabel::Partial && selectable {
            notes.push(if picked {
                "Replace with the source's copy: yes".to_string()
            } else {
                "Replace with the source's copy: press space".to_string()
            });
        }
        if !notes.is_empty() {
            body.text(hanging(
                notes
                    .into_iter()
                    .map(|note| Line::from(Span::styled(format!("      {note}"), muted)))
                    .collect(),
                width,
                6,
            ));
        }
    }

    fn find_choice_lines(
        &self,
        body: &mut SheetBody<'static>,
        results: &FindResults,
        index: usize,
        width: u16,
    ) {
        let roles = self.ui_theme.roles;
        let muted = Style::default().fg(roles.muted);
        let result = &results.report.results[index];
        let cursor = results.cursor == FindFocus::Result(index);
        let mut name_style = Style::default().fg(roles.text);
        if cursor {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let pick = results.picks[index];
        let picked_text = match pick.and_then(|pick| result.candidates.get(pick)) {
            Some(candidate) => format!("  {}", candidate.parent_folder),
            None => "  none picked".to_string(),
        };
        body.text(hanging(
            vec![Line::from(vec![
                Span::styled(
                    if cursor { "▌ " } else { "  " },
                    Style::default().fg(roles.lemon),
                ),
                Span::styled(results.name_of(index), name_style),
                Span::styled(picked_text, muted),
            ])],
            width,
            2,
        ));
        if cursor {
            body.focus_last();
        }
        let lines = result
            .candidates
            .iter()
            .enumerate()
            .map(|(choice, candidate)| {
                let mark = if pick_kind(candidate).is_none() {
                    "  "
                } else if pick == Some(choice) {
                    "● "
                } else {
                    "○ "
                };
                let mut text = crate::find::choice_text(result, candidate);
                // Picking a partial match installs the source's copy.
                if pick_kind(candidate) == Some(PickKind::Replace) {
                    const REPLACES: &str =
                        "picking it replaces your copy with the one from Wago.io";
                    text = match text.strip_suffix(')') {
                        Some(open) => format!("{open}; {REPLACES})"),
                        None => format!("{text} ({REPLACES})"),
                    };
                }
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled(mark, Style::default().fg(roles.lemon)),
                    Span::styled(text, muted),
                ])
            })
            .collect();
        body.text(hanging(lines, width, 6));
    }

    /// The action row and what the keys do on the row under the cursor.
    fn find_results_footer(&self, results: &FindResults, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let on_action = results.cursor == FindFocus::Action;
        let (links, replaces) = results.counts();
        let (label, active) = if self.find_offers_search() {
            ("Search Wago.io by name".to_string(), true)
        } else if links + replaces == 0 {
            ("Nothing selected".to_string(), false)
        } else {
            let label = match (links, replaces) {
                (links, 0) => format!("Link {}", addon_count_label(links)),
                (0, replaces) => format!("Replace {}", addon_count_label(replaces)),
                (links, replaces) => {
                    format!("Link {}, replace {replaces}", addon_count_label(links))
                }
            };
            (label, true)
        };
        let mut style = Style::default().fg(if active { roles.lemon } else { roles.muted });
        if on_action {
            style = style.add_modifier(Modifier::BOLD);
        }
        let action = Line::from(vec![
            Span::styled(
                if on_action && active { "▌ " } else { "  " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(label, style),
        ]);
        let hint: Vec<(&'static str, &'static str)> = match results.cursor {
            FindFocus::Action if self.find_offers_search() => {
                vec![("enter", " search Wago.io for this addon's name")]
            }
            FindFocus::Action if active => vec![("enter", " asks before anything changes")],
            FindFocus::Action => vec![("space", " on a match selects it")],
            FindFocus::Result(index)
                if results.report.results[index].label == FindLabel::ChooseOne =>
            {
                vec![("enter", " picks the next one, or none")]
            }
            FindFocus::Result(_) => vec![("space", " selects or unselects it")],
        };
        let mut lines = vec![Line::from(""), action];
        lines.extend(self.find_hint_footer(hint, width).into_iter().skip(1));
        lines
    }
}

/// The report for what the sources answered, ready for the sheet.
pub(crate) fn find_outcome(
    inputs: &FindInputs,
    rows: &[AddonRecord],
    scope: Option<&[String]>,
    lookups: crate::find::Lookups,
) -> FindOutcome {
    let report = lemonup_core::find::match_addons(inputs, rows, scope, &lookups.lookups);
    FindOutcome {
        report,
        problems: lookups.problems,
        names: LocalNames::new(rows, inputs),
    }
}

/// What the lookup reads first: LemonUp's rows and the AddOns folder. `Err`
/// says which of the two couldn't be read.
pub(crate) fn read_find_inputs(
    state_db_file: &Path,
    addon_dir: &Path,
) -> std::result::Result<(Vec<AddonRecord>, FindInputs), String> {
    let rows = StateDatabase::read_addons(state_db_file)
        .map_err(|error| format!("LemonUp couldn't read its list of your addons: {error}"))?;
    let flavor = GameFlavor::for_addons_dir(addon_dir);
    let inputs = FindInputs::read(addon_dir, flavor)
        .map_err(|error| format!("LemonUp couldn't read your AddOns folder: {error}"))?;
    Ok((rows, inputs))
}

/// Links `links` the way `lemonup find --link` does: reads the AddOns folder
/// first, links in one transaction, then reads it again. Returns the linked
/// rows as the database holds them afterwards.
pub(crate) fn link_found(
    state_db_file: &Path,
    addon_dir: &Path,
    links: &[FoundAddon],
) -> std::result::Result<FindLinkOutcome, String> {
    let flavor = GameFlavor::for_addons_dir(addon_dir);
    let linked = {
        let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
        crate::find::rescan_addons(&mut database, addon_dir, flavor)
            .map_err(|error| format!("LemonUp couldn't read your AddOns folder ({error})"))?;
        lemonup_core::find::link_found_addons(&mut database, addon_dir, flavor, links)
            .map_err(|error| error.to_string())?
    };
    // The list catches up on the next scan if this read fails.
    let sync = match sync_dashboard_state(state_db_file, addon_dir) {
        Ok(sync) => Some(sync),
        Err(error) => {
            tracing::warn!(%error, "could not read the AddOns folder after linking");
            None
        }
    };
    let rows = StateDatabase::read_addons(state_db_file).map_err(|error| error.to_string())?;
    let mut stored = Vec::new();
    let mut unconfirmed = Vec::new();
    for linked in &linked {
        match rows
            .iter()
            .find(|row| row.folder == linked.folder && row.source == linked.source)
        {
            Some(row) => stored.push(row.clone()),
            None => unconfirmed.push(linked.name.clone()),
        }
    }
    Ok(FindLinkOutcome {
        rows: stored,
        unconfirmed,
        sync,
    })
}

/// The status line after linking, from what the database holds.
pub(crate) fn linked_status(outcome: &FindLinkOutcome) -> (StatusKind, String) {
    let mut text = String::new();
    if !outcome.rows.is_empty() {
        let (parts, _) = crate::find::linked_parts(&outcome.rows);
        text = format!(
            "Linked {}: {}.",
            addon_count_label(outcome.rows.len()),
            parts.join(", ")
        );
    }
    if !outcome.unconfirmed.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&format!(
            "LemonUp couldn't confirm that {} {} linked. Press F to look again.",
            outcome.unconfirmed.join(", "),
            if outcome.unconfirmed.len() == 1 {
                "was"
            } else {
                "were"
            }
        ));
        return (StatusKind::Problem, text);
    }
    if text.is_empty() {
        text = "Nothing was linked.".to_string();
    }
    (StatusKind::Done, text)
}
