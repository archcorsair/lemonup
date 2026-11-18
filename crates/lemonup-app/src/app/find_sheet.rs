//! "Find your addons online": the `F` sheet (and `f` on one addon).
//!
//! The sheet opens looking: it reads the folders, then asks each source that is on for this
//! game, one request each. A rail on the right shows every source with its number; `1`, `2`
//! and `3` turn one on or off. Every answer is kept for the life of the sheet and the report is
//! worked out again from the answers of the sources that are on, so a source is asked only the
//! first time it is on. Results list what was found the way `lemonup find` does. Linking asks one
//! question first; partial matches are replaced through the normal Wago.io install, one at a
//! time. Matching and linking live in `lemonup_core::find`; the lookups in `crate::find`.

use std::collections::VecDeque;

use lemonup_core::find::{
    FindChoice, FindInputs, FindLabel, FindReport, FindResult, FindSource, FoundAddon,
    SourceAnswer, SourceLookups,
};

use super::sheet::{SheetBody, SheetMove, SheetRoom};
use super::*;
use crate::find::{LocalNames, Lookups, SourceProblem};

/// Rows a page key moves in the results.
const PAGE_ROWS: usize = 8;
/// The name column of the results gets at least this wide when a name needs it, even when
/// long versions leave less room.
const NAME_WIDTH_MIN: usize = 28;
/// In rows without the version column, the name column gets at least this wide when a name
/// needs it, even when long notes leave less room: at 80 columns that still leaves room for a
/// note such as "Update ready (1.2.3)" on the row.
const NAME_WIDTH_NARROW_MIN: usize = 14;
/// The width of the source column of the results.
const SOURCE_WIDTH: usize = 14;
/// The results show the version column from this width on. Narrower, a row is the name, the
/// source and the note, so every source and note fits beside the rail.
const VERSION_COLUMN_FROM: usize = 70;
/// The columns the source rail takes on the right of the sheet besides its text: a gap, its
/// left border and a space.
const RAIL_EDGE: usize = 3;
/// In a rail line, the number, the box and the name, each with the space after it.
const RAIL_LEAD: usize = 6 + 12;
/// The words that say a source can't answer. A wide rail puts them where the count goes; the
/// narrow one under the source.
const RAIL_WORDS: [&str; 2] = ["Needs a key", "Check failed"];

/// How wide the source rail is: narrow, with the status in 4 columns, or wide enough for the
/// longest of [`RAIL_WORDS`] in its place. A sheet picks it from its width alone, so the rail
/// keeps its width while sources fail or answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RailLayout {
    wide: bool,
}

impl RailLayout {
    /// The wide rail when the results keep the version column beside it.
    fn for_width(width: u16) -> Self {
        let wide = Self { wide: true };
        Self {
            wide: usize::from(width) >= usize::from(wide.width()) + VERSION_COLUMN_FROM,
        }
    }

    /// The columns of the status at the end of a source's line.
    fn status(self) -> usize {
        if self.wide {
            2 + RAIL_WORDS
                .iter()
                .map(|word| word.chars().count())
                .max()
                .unwrap_or(0)
        } else {
            4
        }
    }

    /// The text width.
    fn text(self) -> u16 {
        u16::try_from(RAIL_LEAD + self.status()).unwrap_or(u16::MAX)
    }

    /// The columns the rail takes, its edge included.
    fn width(self) -> u16 {
        self.text()
            .saturating_add(u16::try_from(RAIL_EDGE).unwrap_or(0))
    }
}
/// The index of "Not checked" in [`GROUPS`].
const NOT_CHECKED_GROUP: usize = 5;
/// In the rail, the first time Wago.io is asked on this machine.
pub(crate) const WAGO_HASH_NOTICE: &str = "Sends folder names and file hashes to Wago.io.";

/// `lines` word wrapped at `width`, each row after a line's first indented
/// by `indent`, so a long row continues under its own text and never at the
/// sheet's left edge.
fn hanging(lines: Vec<Line<'static>>, width: u16, indent: usize) -> Vec<Line<'static>> {
    super::sheet::wrap_lines_hanging(&lines, width, u16::try_from(indent).unwrap_or(0))
}

/// What the sheet shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindStep {
    /// Reading the folders, then asking the sources, until one that is on answers.
    Looking {
        folders: usize,
    },
    /// The lookup couldn't run; nothing changed.
    Failed(String),
    Results(Box<FindResults>),
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
    /// Whether the person set that pick (or cleared it) themselves, rather than the report
    /// picking it. Only their own picks outlive a report worked out again.
    pub(crate) chosen: Vec<bool>,
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
    /// The linked addons CurseForge also manages.
    pub(crate) curseforge: Vec<String>,
    /// The list after the rescan; `None` when that read failed.
    pub(super) sync: Option<AddonScanOutcome>,
}

/// What the lookup reads first: LemonUp's rows and the AddOns folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindRead {
    pub(crate) rows: Vec<AddonRecord>,
    pub(crate) inputs: FindInputs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindTaskEvent {
    /// The folders are read, or couldn't be.
    Read {
        generation: u64,
        read: std::result::Result<Box<FindRead>, String>,
    },
    /// One source answered.
    Answered {
        generation: u64,
        source: FindSource,
        lookups: Box<Lookups>,
    },
    Linked(std::result::Result<Box<FindLinkOutcome>, String>),
}

/// Work the sheet does in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindJob {
    /// Reads LemonUp's rows and the AddOns folder.
    Read,
    /// Asks one source: one request.
    Ask(FindSource),
}

/// The sheet, the background jobs and the replacements still to run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FindState {
    /// `None` while the sheet is closed.
    pub(crate) step: Option<FindStep>,
    /// The sources wanted: this game's saved choice when the sheet opened, changed with `1`,
    /// `2` and `3`. Wago.io stays wanted without a key; it is on only with one.
    pub(crate) sources: FindChoice,
    /// What the lookup read, once it is read.
    pub(crate) read: Option<Box<FindRead>>,
    /// Every answer, kept for the life of the sheet: a source turned off and on again is not
    /// asked again.
    pub(crate) answers: SourceLookups,
    /// The sources that answered, and the problems their answers came with.
    pub(crate) answered: Vec<FindSource>,
    pub(crate) problems: Vec<SourceProblem>,
    /// The sources asked that haven't answered yet.
    pub(crate) asking: Vec<FindSource>,
    /// Answers that came while the link question was open. They wait until it closes, so the
    /// question and the results behind it stay as they were when it opened.
    pub(crate) held: Vec<(FindSource, Box<Lookups>)>,
    /// The Wago.io notice shows in this sheet.
    pub(crate) hash_notice: bool,
    /// The Wago.io notice was shown on this machine, so it never shows again.
    pub(crate) hash_notice_shown: bool,
    /// The jobs the sheet started, in place of starting them.
    #[cfg(test)]
    pub(crate) jobs: Vec<FindJob>,
    /// The parent folders looked up; `None` for every Not linked addon.
    pub(crate) scope: Option<Vec<String>>,
    /// `f` on one addon: the name to search Wago.io for when nothing is found.
    pub(crate) single: Option<String>,
    /// Bumped whenever a lookup starts or is cancelled, so a late answer is dropped.
    pub(crate) generation: u64,
    /// The link question is open.
    pub(crate) question: bool,
    /// The picked addons CurseForge also manages, read when the question opened.
    pub(crate) question_curseforge: Vec<String>,
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
    /// `1`, `2` or `3`: turns a source on or off.
    ToggleSource(FindSource),
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
    /// Starts work in the background.
    Spawn(FindJob),
    /// Turns a source on or off.
    ToggleSource(FindSource),
    /// The folders are read.
    StoreRead(Box<FindRead>),
    /// These sources are asked now.
    Asking(Vec<FindSource>),
    /// A source answered.
    StoreAnswer(FindSource, Box<Lookups>),
    /// A source answered while the link question is open: kept for when it closes.
    HoldAnswer(FindSource, Box<Lookups>),
    /// Works the report out again from the answers of the sources that are on.
    Refresh,
    /// Shows the Wago.io notice in this sheet, and remembers it was shown.
    ShowHashNotice,
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
            .collect::<Vec<_>>();
        let mut results = Self {
            chosen: vec![false; picks.len()],
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

    /// The results of a report worked out again: a pick the person made stays while its match
    /// is still there and is dropped when it is not, and one they cleared stays clear while the
    /// matches are the same. Every other result starts as a new one would, so a pick the report
    /// made never depends on which source answered first. The cursor stays on its
    /// row while it can be picked. `search` is whether the sheet looked up one addon.
    pub(crate) fn carried(outcome: FindOutcome, old: &FindResults, search: bool) -> Self {
        let mut next = Self::new(outcome);
        let same = |a: &FoundAddon, b: &FoundAddon| {
            a.source == b.source
                && a.source_id == b.source_id
                && a.parent_folder.eq_ignore_ascii_case(&b.parent_folder)
        };
        for index in 0..next.report.results.len() {
            let result = &next.report.results[index];
            let Some(before) = old
                .report
                .results
                .iter()
                .position(|was| was.folder.eq_ignore_ascii_case(&result.folder))
            else {
                continue;
            };
            let was = &old.report.results[before];
            if !old.chosen[before] {
                continue;
            }
            let (pick, chosen) = match old.picks[before] {
                Some(pick) => {
                    let kept = was.candidates.get(pick).and_then(|picked| {
                        result
                            .candidates
                            .iter()
                            .position(|candidate| same(candidate, picked))
                    });
                    (kept, kept.is_some())
                }
                None if was.candidates.len() == result.candidates.len()
                    && was
                        .candidates
                        .iter()
                        .zip(&result.candidates)
                        .all(|(a, b)| same(a, b)) =>
                {
                    (None, true)
                }
                None => (next.picks[index], false),
            };
            next.picks[index] = pick;
            next.chosen[index] = chosen;
        }
        let search = search && !next.any_selectable();
        let focus = next.focusables(search);
        let kept = match old.cursor {
            FindFocus::Result(index) => {
                let folder = &old.report.results[index].folder;
                next.report
                    .results
                    .iter()
                    .position(|result| result.folder.eq_ignore_ascii_case(folder))
                    .map(FindFocus::Result)
            }
            FindFocus::Action => Some(FindFocus::Action),
        };
        next.cursor = kept
            .filter(|cursor| focus.contains(cursor))
            .or_else(|| focus.first().copied())
            .unwrap_or(FindFocus::Action);
        next
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
            // Each source has a number that turns it on or off.
            (
                FindStep::Looking { .. } | FindStep::Results(_),
                KeyCode::Char(number @ '1'..='3'),
            ) => {
                let source = FindSource::ALL[usize::from(number as u8 - b'1')];
                if self.find_toggle_works(source) {
                    message(FindMessage::ToggleSource(source))
                } else {
                    Some(vec![])
                }
            }
            (FindStep::Results(results), code) => {
                let focus = results.focusables(self.find_offers_search());
                let position = focus
                    .iter()
                    .position(|focus| *focus == results.cursor)
                    .unwrap_or(0);
                // The cursor moves while it can; at the first or last row (or
                // with no rows at all) the keys scroll the report instead, so
                // every line of it, listed names and source errors too, can
                // be read.
                let can_down = position + 1 < focus.len();
                let can_up = position > 0;
                let scroll = |movement| Some(vec![AppMessage::SheetScroll(movement)]);
                let with_scroll = |first, movement| {
                    let mut messages = if focus.is_empty() {
                        vec![]
                    } else {
                        vec![AppMessage::Find(first)]
                    };
                    messages.push(AppMessage::SheetScroll(movement));
                    Some(messages)
                };
                match code {
                    KeyCode::Down | KeyCode::Char('j') if can_down => message(FindMessage::Move(1)),
                    KeyCode::Down | KeyCode::Char('j') => scroll(SheetMove::LineDown),
                    KeyCode::Up | KeyCode::Char('k') if can_up => message(FindMessage::Move(-1)),
                    KeyCode::Up | KeyCode::Char('k') => scroll(SheetMove::LineUp),
                    KeyCode::PageDown if can_down => message(FindMessage::Move(PAGE_ROWS as isize)),
                    KeyCode::PageDown => scroll(SheetMove::PageDown),
                    KeyCode::PageUp if can_up => message(FindMessage::Move(-(PAGE_ROWS as isize))),
                    KeyCode::PageUp => scroll(SheetMove::PageUp),
                    // The cursor goes to the first or last row, and the view
                    // to the very top or bottom of the report.
                    KeyCode::Home => with_scroll(FindMessage::First, SheetMove::Top),
                    KeyCode::End => with_scroll(FindMessage::Last, SheetMove::Bottom),
                    KeyCode::Char(' ') if self.find_space_works() => message(FindMessage::Toggle),
                    KeyCode::Enter if self.find_enter_works() => message(FindMessage::Enter),
                    // The keys of the list never reach it behind the sheet.
                    _ => Some(vec![]),
                }
            }
            _ => Some(vec![]),
        }
    }

    /// Whether a source can be on: Wago.io needs a key.
    pub(super) fn find_source_available(&self, source: FindSource) -> bool {
        source != FindSource::Wago || self.wago_api_key.is_some()
    }

    /// Whether a source is on. Wago.io without a key never is, though the saved choice keeps
    /// it for when there is one.
    pub(super) fn find_on(&self, source: FindSource) -> bool {
        self.find.sources.wants(source) && self.find_source_available(source)
    }

    /// The sources that are on, in their usual order.
    pub(super) fn find_on_sources(&self) -> Vec<FindSource> {
        self.find.sources.asked(self.wago_api_key.is_some())
    }

    /// Whether the number of `source` does something: while looking or on the results, for a
    /// source that can be on, and never for the last one that is on.
    pub(super) fn find_toggle_works(&self, source: FindSource) -> bool {
        matches!(
            self.find.step,
            Some(FindStep::Looking { .. } | FindStep::Results(_))
        ) && self.find_source_available(source)
            && !(self.find_on(source) && self.find_on_sources().len() == 1)
    }

    /// The keys that turn a source on or off, as the key line shows them: only the numbers
    /// that do something now, and `None` when none does.
    pub(super) fn find_source_keys(&self) -> Option<(&'static str, super::keymap::KeyCodes)> {
        const ONE: KeyCode = KeyCode::Char('1');
        const TWO: KeyCode = KeyCode::Char('2');
        const THREE: KeyCode = KeyCode::Char('3');
        let works = FindSource::ALL.map(|source| self.find_toggle_works(source));
        Some(match works {
            [true, true, true] => ("1 2 3", &[ONE, TWO, THREE]),
            [true, true, false] => ("1 2", &[ONE, TWO]),
            [true, false, true] => ("1 3", &[ONE, THREE]),
            [false, true, true] => ("2 3", &[TWO, THREE]),
            [true, false, false] => ("1", &[ONE]),
            [false, true, false] => ("2", &[TWO]),
            [false, false, true] => ("3", &[THREE]),
            [false, false, false] => return None,
        })
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
                Ok(scope) => self.find_open_actions(scope, None),
                Err(note) => vec![self.find_note(StatusKind::Note, note)],
            },
            FindMessage::OpenOne => match self.not_linked_current_item() {
                Some(item) => {
                    self.find_open_actions(Some(vec![item.folder.clone()]), Some(item.name.clone()))
                }
                None => vec![],
            },
            FindMessage::ToggleSource(source) if self.find_toggle_works(source) => {
                self.find_toggle_actions(source)
            }
            FindMessage::ToggleSource(_) => vec![],
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
                next.chosen[index] = true;
                vec![self.set_find_step(FindStep::Results(Box::new(next)))]
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

    /// Opens the sheet and starts looking at once.
    fn find_open_actions(
        &self,
        scope: Option<Vec<String>>,
        single: Option<String>,
    ) -> Vec<AppAction> {
        // An update, install or other job that changes the addons runs.
        if let Some(note) = self.switch_busy_note() {
            return vec![self.find_note(StatusKind::Note, note)];
        }
        vec![
            AppAction::Find(FindAction::Open { scope, single }),
            if self.effective_addon_dir.is_none() {
                self.set_find_step(FindStep::Failed(
                    "LemonUp has no AddOns folder for this game yet. Pick one in Settings."
                        .to_string(),
                ))
            } else {
                AppAction::Find(FindAction::Spawn(FindJob::Read))
            },
        ]
    }

    /// Turns `source` on or off, saves the choice for this game, asks a source turned on that
    /// was never asked, and works the report out again.
    fn find_toggle_actions(&self, source: FindSource) -> Vec<AppAction> {
        let mut next = self.find.sources;
        next.toggle(source);
        let mut actions = vec![AppAction::Find(FindAction::ToggleSource(source))];
        actions.extend(self.find_remember_actions(next));
        // While the folders are read, the sources that are on get asked once they are.
        if next.wants(source)
            && let Some(read) = self.find.read.as_deref()
        {
            actions.extend(self.find_ask_actions(read, &[source]));
        }
        actions.push(AppAction::Find(FindAction::Refresh));
        actions
    }

    /// Asks each of `sources` that was never asked, one request each. The first time folder
    /// names and file hashes go to Wago.io on this machine, the sheet says so.
    fn find_ask_actions(&self, read: &FindRead, sources: &[FindSource]) -> Vec<AppAction> {
        let ask = sources
            .iter()
            .copied()
            .filter(|source| {
                !self.find.answered.contains(source) && !self.find.asking.contains(source)
            })
            .collect::<Vec<_>>();
        if ask.is_empty() {
            return vec![];
        }
        let mut actions = vec![AppAction::Find(FindAction::Asking(ask.clone()))];
        actions.extend(
            ask.iter()
                .map(|source| AppAction::Find(FindAction::Spawn(FindJob::Ask(*source)))),
        );
        if ask.contains(&FindSource::Wago)
            && !self.find.hash_notice_shown
            && self.wago_api_key.is_some()
            && !read
                .inputs
                .wago_lookups(&read.rows, self.find.scope.as_deref())
                .is_empty()
        {
            actions.push(AppAction::Find(FindAction::ShowHashNotice));
        }
        actions
    }

    /// Saves `choice` in this game's config, when it differs from what it holds. A save that
    /// fails says so; the sheet goes on.
    fn find_remember_actions(&self, choice: FindChoice) -> Vec<AppAction> {
        let persisted = &self.config_pane.persisted;
        if FindChoice::from_config(persisted.find_sources.as_deref()) == choice {
            return vec![];
        }
        let mut saved = persisted.clone();
        saved.find_sources = Some(choice.to_config());
        match self.config_store.write_config(&saved) {
            Ok(saved) => {
                let pane = self.config_pane.mark_saved(saved.clone());
                vec![
                    AppAction::SetPersistedConfig(saved),
                    AppAction::SetConfigPaneState(pane),
                ]
            }
            Err(error) => vec![self.find_note(
                StatusKind::Problem,
                &format!("Couldn't remember which sources to look at: {error}"),
            )],
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
        vec![self.set_find_step(FindStep::Results(Box::new(next)))]
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
                next.chosen[index] = true;
                vec![self.set_find_step(FindStep::Results(Box::new(next)))]
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
            FindTaskEvent::Read { generation, read }
                if generation == self.find.generation && self.find.looking() =>
            {
                match read {
                    Ok(read) => {
                        let mut actions = self.find_ask_actions(&read, &self.find_on_sources());
                        actions.insert(0, AppAction::Find(FindAction::StoreRead(read)));
                        actions.push(AppAction::Find(FindAction::Refresh));
                        actions
                    }
                    Err(error) => vec![self.set_find_step(FindStep::Failed(error))],
                }
            }
            FindTaskEvent::Answered {
                generation,
                source,
                lookups,
            } if generation == self.find.generation && self.find.step.is_some() => {
                if self.find.question {
                    vec![AppAction::Find(FindAction::HoldAnswer(source, lookups))]
                } else {
                    vec![
                        AppAction::Find(FindAction::StoreAnswer(source, lookups)),
                        AppAction::Find(FindAction::Refresh),
                    ]
                }
            }
            // A lookup that was cancelled: its answer changes nothing.
            FindTaskEvent::Read { .. } | FindTaskEvent::Answered { .. } => vec![],
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
                // A lookup still running answers into nothing.
                self.find.generation += 1;
                let mut sources =
                    FindChoice::from_config(self.config_pane.persisted.find_sources.as_deref());
                // Only Wago.io is wanted and there is no key: the others are on until changed.
                if sources.asked(self.wago_api_key.is_some()).is_empty() {
                    for source in [FindSource::WowInterface, FindSource::Tukui] {
                        sources.toggle(source);
                    }
                }
                self.find.sources = sources;
                self.find.scope = scope;
                self.find.single = single;
                self.find.question = false;
                self.find.held.clear();
                self.find.read = None;
                self.find.answers = SourceLookups::not_asked();
                self.find.answered.clear();
                self.find.problems.clear();
                self.find.asking.clear();
                self.find.hash_notice = false;
                if !self.find.hash_notice_shown {
                    self.find.hash_notice_shown = self
                        .config_store
                        .shared_store()
                        .is_some_and(|store| store.wago_hash_notice_shown());
                }
                let folders = self
                    .find_scope_items()
                    .iter()
                    .map(|item| 1 + item.owned_folders.len())
                    .sum();
                self.find.step = Some(FindStep::Looking { folders });
                self.apply(AppAction::SetDetailMode(DetailMode::Find));
            }
            FindAction::Close => {
                // A lookup still running answers into nothing.
                self.find.generation += 1;
                self.find.step = None;
                self.find.question = false;
                self.find.held.clear();
                self.find.asking.clear();
                if self.dashboard.detail_mode == DetailMode::Find {
                    self.apply(AppAction::SetDetailMode(DetailMode::Overview));
                }
            }
            FindAction::Spawn(job) => self.spawn_find_job(job),
            FindAction::ToggleSource(source) => self.find.sources.toggle(source),
            FindAction::StoreRead(read) => self.find.read = Some(read),
            FindAction::Asking(sources) => self.find.asking.extend(sources),
            FindAction::StoreAnswer(source, lookups) => {
                self.find.asking.retain(|asking| *asking != source);
                if !self.find.answered.contains(&source) {
                    self.find.answered.push(source);
                }
                let Lookups { lookups, problems } = *lookups;
                self.find
                    .problems
                    .retain(|problem| problem.source != source);
                self.find.problems.extend(
                    problems
                        .into_iter()
                        .filter(|problem| problem.source == source),
                );
                self.find.answers.take_answer(source, lookups);
            }
            FindAction::HoldAnswer(source, lookups) => self.find.held.push((source, lookups)),
            FindAction::Refresh => self.refresh_find(),
            FindAction::ShowHashNotice => {
                self.find.hash_notice = true;
                self.find.hash_notice_shown = true;
                // A flag that can't be saved shows the notice again next time, which is fine.
                if let Some(store) = self.config_store.shared_store()
                    && let Err(error) = store.remember_wago_hash_notice()
                {
                    tracing::warn!(%error, "couldn't remember that the Wago.io notice was shown");
                }
            }
            FindAction::SetStep(step) => {
                if self.find.step.is_some() {
                    self.find.step = Some(step);
                }
            }
            FindAction::SetQuestion(open) => {
                self.find.question = open;
                self.find.question_curseforge = if open {
                    self.find_picks_curseforge_manages()
                } else {
                    Vec::new()
                };
                // Answers held while the question was open count now.
                if !open && !self.find.held.is_empty() {
                    for (source, lookups) in std::mem::take(&mut self.find.held) {
                        self.apply_find(FindAction::StoreAnswer(source, lookups));
                    }
                    self.refresh_find();
                }
            }
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
                let curseforge_files = self.curseforge_files.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        link_found(&state_db_file, &addon_dir, &links, &curseforge_files)
                            .map(Box::new)
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

    /// Works the report out again from the answers of the sources that are on. The results show
    /// once the folders are read and a source that is on answered.
    fn refresh_find(&mut self) {
        let Some(read) = self.find.read.as_deref() else {
            return;
        };
        if !matches!(
            self.find.step,
            Some(FindStep::Looking { .. } | FindStep::Results(_))
        ) {
            return;
        }
        let answered = self
            .find_on_sources()
            .into_iter()
            .filter(|source| self.find.answered.contains(source))
            .collect::<Vec<_>>();
        if answered.is_empty() {
            // Nothing that is on has answered yet.
            let folders = read.inputs.folders.len();
            self.find.step = Some(FindStep::Looking { folders });
            return;
        }
        let mut lookups = self.find.answers.only(&answered);
        // Wago.io is wanted but there is no key: the report says a key finds more.
        if self.find.sources.wants(FindSource::Wago) && self.wago_api_key.is_none() {
            lookups.wago = SourceAnswer::Skipped;
        }
        let problems = self
            .find
            .problems
            .iter()
            .filter(|problem| answered.contains(&problem.source))
            .cloned()
            .collect();
        let outcome = find_outcome(
            &read.inputs,
            &read.rows,
            self.find.scope.as_deref(),
            Lookups { lookups, problems },
        );
        let next = match self.find.results() {
            Some(old) => FindResults::carried(outcome, old, self.find.single.is_some()),
            None => FindResults::new(outcome),
        };
        self.find.step = Some(FindStep::Results(Box::new(next)));
    }

    /// Records the job instead of starting it, so a test never reads a folder or asks a source.
    #[cfg(test)]
    fn spawn_find_job(&mut self, job: FindJob) {
        self.find.jobs.push(job);
    }

    /// Reads the folders, or asks one source, in the background. The answer carries the
    /// sheet's generation, so one that comes after the sheet closed is dropped.
    #[cfg(not(test))]
    fn spawn_find_job(&mut self, job: FindJob) {
        let generation = self.find.generation;
        let sender = self.task_events_tx.clone();
        match job {
            FindJob::Read => {
                let Some(addon_dir) = self.effective_addon_dir.clone() else {
                    return;
                };
                let state_db_file = self.state_db_file.clone();
                tokio::spawn(async move {
                    let read = tokio::task::spawn_blocking(move || {
                        read_find_inputs(&state_db_file, &addon_dir)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(format!("The lookup stopped unexpectedly: {error}"))
                    })
                    .map(|(rows, inputs)| Box::new(FindRead { rows, inputs }));
                    let _ =
                        sender.send(AppTaskEvent::Find(FindTaskEvent::Read { generation, read }));
                });
            }
            FindJob::Ask(source) => {
                let Some(read) = self.find.read.clone() else {
                    return;
                };
                let scope = self.find.scope.clone();
                let api_key = self.wago_api_key.clone();
                let cache_dir = self
                    .roots
                    .paths(&self.runtime.profile_name)
                    .map(|paths| paths.cache_dir)
                    .unwrap_or_else(|_| self.data_dir.join("cache"));
                tokio::spawn(async move {
                    let wago_lookups = read.inputs.wago_lookups(&read.rows, scope.as_deref());
                    let lookups = crate::find::lookup_sources(
                        &wago_lookups,
                        read.inputs.flavor,
                        api_key.as_deref(),
                        &cache_dir,
                        FindChoice::only(&[source]),
                    )
                    .await;
                    let _ = sender.send(AppTaskEvent::Find(FindTaskEvent::Answered {
                        generation,
                        source,
                        lookups: Box::new(lookups),
                    }));
                });
            }
        }
    }

    /// The names of the picked addons CurseForge also manages: reads its
    /// record once.
    fn find_picks_curseforge_manages(&self) -> Vec<String> {
        let Some(results) = self.find.results() else {
            return Vec::new();
        };
        let managed = self.curseforge_now();
        managed.managed_names(
            results
                .picked()
                .into_iter()
                .map(|(candidate, _)| (candidate.name.clone(), candidate.folders.iter())),
        )
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
            curseforge: self.find.question_curseforge.clone(),
            offset: self.find.question_offset,
        })
    }

    /// The sheet's body for the step it is at. Every step keeps room for the
    /// tallest one, so the sheet holds still.
    pub(super) fn find_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let roles = self.ui_theme.roles;
        let width = room.width;
        let text = Style::default().fg(roles.text);
        let muted = roles.muted;
        let mut body = SheetBody::new();
        body.reserve(room.height);
        let ellipsis = self.ui_theme.symbols.ellipsis;
        let rail = RailLayout::for_width(width);
        match self.find.step.as_ref() {
            None => {}
            Some(FindStep::Looking { folders }) => {
                body.rail(rail.width(), self.find_rail(rail));
                if self.find.read.is_none() {
                    body.text(vec![Line::from(Span::styled(
                        format!(
                            "Reading {folders} folder{}{ellipsis}",
                            plural_suffix(*folders)
                        ),
                        text,
                    ))]);
                }
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
            }
            Some(FindStep::Results(results)) => {
                body.rail(rail.width(), self.find_rail(rail));
                let width = width.saturating_sub(rail.width()).max(1);
                self.find_results_body(&mut body, results, width);
            }
        }
        body
    }

    /// The rail of sources: under "Sources", one line per source with its number, a box that
    /// says whether it is on, its name, then how many addons it found, a spinner while it is
    /// asked, or `off`. "Needs a key" or "Check failed" takes the count's place in a wide rail
    /// and goes on the line under the source in a narrow one. The Wago.io notice goes under
    /// them all while it shows and Wago.io is on.
    fn find_rail(&self, rail: RailLayout) -> Vec<Line<'static>> {
        enum Shown {
            NeedsKey,
            Off,
            Asking,
            Failed,
            Found(usize),
        }
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let results = self.find.results();
        let mut lines = vec![Line::from(Span::styled(
            "Sources",
            muted.add_modifier(Modifier::BOLD),
        ))];
        for (index, source) in FindSource::ALL.into_iter().enumerate() {
            let shown = if !self.find_source_available(source) {
                Shown::NeedsKey
            } else if !self.find.sources.wants(source) {
                Shown::Off
            } else if self.find.asking.contains(&source) {
                Shown::Asking
            } else {
                match results {
                    Some(results)
                        if results
                            .problems
                            .iter()
                            .any(|problem| problem.source == source) =>
                    {
                        Shown::Failed
                    }
                    Some(results) => Shown::Found(results.report.found_on(source)),
                    // The folders are still read.
                    None => Shown::Asking,
                }
            };
            let digit = match shown {
                Shown::NeedsKey => muted,
                _ => Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            };
            let (mark, mark_style) = match shown {
                Shown::NeedsKey => ("[-]", Style::default().fg(roles.dim)),
                _ if self.find_on(source) => ("[x]", Style::default().fg(roles.lemon)),
                _ => ("[ ]", muted),
            };
            let name = match shown {
                Shown::NeedsKey | Shown::Off => muted,
                _ => Style::default().fg(roles.text),
            };
            let (status, status_style) = match shown {
                Shown::Found(count) => (count.to_string(), muted),
                Shown::Asking => (
                    self.spinner_frame().to_string(),
                    Style::default().fg(roles.lemon),
                ),
                Shown::Off => ("off".to_string(), muted),
                Shown::NeedsKey if rail.wide => (RAIL_WORDS[0].to_string(), muted),
                Shown::Failed if rail.wide => {
                    (RAIL_WORDS[1].to_string(), Style::default().fg(roles.red))
                }
                Shown::NeedsKey | Shown::Failed => (String::new(), muted),
            };
            let status_width = rail.status();
            lines.push(Line::from(vec![
                Span::styled((index + 1).to_string(), digit),
                Span::raw(" "),
                Span::styled(mark, mark_style),
                Span::raw(" "),
                Span::styled(format!("{:<12}", source.name()), name),
                Span::styled(format!("{status:>status_width$}"), status_style),
            ]));
            // Under the name, in a narrow rail.
            let under = match shown {
                _ if rail.wide => None,
                Shown::NeedsKey => Some((RAIL_WORDS[0], muted)),
                Shown::Failed => Some((RAIL_WORDS[1], Style::default().fg(roles.red))),
                _ => None,
            };
            if let Some((text, style)) = under {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(text, style),
                ]));
            }
        }
        if self.find.hash_notice && self.find_on(FindSource::Wago) {
            lines.push(Line::from(""));
            lines.extend(super::sheet::wrap_lines(
                &[Line::from(Span::styled(WAGO_HASH_NOTICE, muted))],
                rail.text(),
            ));
        }
        lines
    }

    fn find_results_body(&self, body: &mut SheetBody<'static>, results: &FindResults, width: u16) {
        let roles = self.ui_theme.roles;
        let text = Style::default().fg(roles.text);
        let muted = roles.muted;
        let names = &results.names;
        let report = &results.report;
        let entries = report
            .results
            .iter()
            .filter_map(|result| crate::find::result_entry(result, names))
            .collect::<Vec<_>>();
        let longest = entries
            .iter()
            .map(|entry| entry.name.chars().count())
            .max()
            .unwrap_or(0);
        let versions = usize::from(width) >= VERSION_COLUMN_FROM;
        let name_width = if versions {
            let version_width = entries
                .iter()
                .map(|entry| entry.version.chars().count())
                .max()
                .unwrap_or(0);
            // The names get the room the cursor, the mark, the source and the versions leave.
            let room = usize::from(width).saturating_sub(3 + 2 + SOURCE_WIDTH + version_width);
            longest.min(room.max(NAME_WIDTH_MIN))
        } else {
            let note_width = entries
                .iter()
                .filter_map(|entry| entry.update.as_ref())
                .map(|update| update.chars().count())
                .max()
                .unwrap_or(0);
            // The names get the room the cursor, the mark, the source and the notes leave.
            let room = usize::from(width).saturating_sub(3 + 2 + SOURCE_WIDTH + note_width);
            longest.min(room.max(NAME_WIDTH_NARROW_MIN))
        };
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
                0 => Style::default().fg(roles.green),
                NOT_CHECKED_GROUP => Style::default().fg(roles.red),
                6..=8 => roles.muted,
                _ => Style::default().fg(roles.amber),
            };
            // "Not found" says where the lookup looked when it didn't ask every source.
            let title = if *label == FindLabel::NotFound {
                report.not_found_title()
            } else {
                title.to_string()
            };
            body.text(vec![Line::from(vec![
                Span::styled(title.to_string(), tone.add_modifier(Modifier::BOLD)),
                Span::styled(format!(" ({})", members.len()), muted),
            ])]);
            match label {
                FindLabel::Exact | FindLabel::Likely | FindLabel::Partial => {
                    for index in members {
                        self.find_entry_lines(body, results, index, name_width, versions, width);
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
        // One blank line above the action, from the footer.
        body.trim_trailing_gap();
        body.footer(self.find_results_footer(results))
            .footer_at_bottom();
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
                    roles.muted,
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
        versions: bool,
        width: u16,
    ) {
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
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
        ];
        // Without the version column, a Likely or Partial match keeps the reason its version
        // carries on the line under it.
        let mut notes = Vec::new();
        if versions {
            spans.push(Span::styled(
                entry.version.clone(),
                Style::default().fg(roles.text),
            ));
        } else if result.label != FindLabel::Exact {
            notes.push(entry.version.clone());
        }
        if let Some(update) = &entry.update {
            spans.push(Span::styled(
                format!(
                    "{}{}",
                    if versions { "  " } else { "" },
                    crate::find::capitalize(update)
                ),
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
        notes.extend(entry.notes.iter().cloned());
        if result.label == FindLabel::Partial && selectable {
            notes.push(if picked {
                "Replace with the source's copy: yes".to_string()
            } else {
                "Replace with the source's copy: no".to_string()
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
        let muted = roles.muted;
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

    /// The action under the results, one blank line above it, its text in line with the names.
    fn find_results_footer(&self, results: &FindResults) -> Vec<Line<'static>> {
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
        let mut style = if active {
            Style::default().fg(roles.lemon)
        } else {
            roles.muted
        };
        if on_action {
            style = style.add_modifier(Modifier::BOLD);
        }
        // The cursor's column, then the mark's two, as on a result row.
        let action = Line::from(vec![
            Span::styled(
                if on_action && active { "▌  " } else { "   " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(label, style),
        ]);
        vec![Line::from(""), action]
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
    curseforge_files: &[PathBuf],
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
    let managed = CurseForgeManaged::read(addon_dir, curseforge_files);
    let curseforge = managed.managed_row_names(&stored);
    Ok(FindLinkOutcome {
        rows: stored,
        unconfirmed,
        curseforge,
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
    if !outcome.curseforge.is_empty() {
        text.push_str(&format!(
            " {}.",
            lemonup_core::curseforge::short_hint_naming(&outcome.curseforge)
        ));
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
