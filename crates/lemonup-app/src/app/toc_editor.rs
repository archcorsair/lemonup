//! The .toc editor: one addon's Title, Notes and Category, the value the
//! game uses for each and where it comes from, and a box to change one.
//! Saving writes the addon's .toc files right away, through the folder swap;
//! a "Not linked" addon asks first.

use std::collections::BTreeMap;

use lemonup_core::toc_edit::{self, TocField, TocFieldValues, TocValueSource};

use super::dialog::Question;
use super::sheet::{SheetBody, SheetRoom, text_height, wrap_lines};
use super::*;
use crate::update_targets::{TocApplyError, apply_toc_overrides_now};
use lemonup_core::paths::same_addons_folder;

/// The most Category suggestions shown under the box.
const SUGGESTION_LIMIT: usize = 5;
/// "Category  " and the cursor bar before it.
const LABEL_WIDTH: usize = 10;
const BAR_WIDTH: usize = 2;
/// The source column: the longest is "set by you, not in the files".
const SOURCE_WIDTH: usize = 28;
/// The most characters of your value the "not in the files" explanation
/// repeats, so its height has a limit the sheet can reserve.
const YOURS_WIDTH: usize = 40;

/// The editor's state: whose fields, the row under the cursor, the box
/// while a field is edited, and a save waiting for the "Not linked" question.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TocEditPane {
    /// The addon's parent folder.
    pub(crate) folder: String,
    pub(crate) selected: usize,
    pub(crate) edit: Option<TocFieldEdit>,
    pub(crate) pending: Option<TocApplyRequest>,
}

/// A field being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TocFieldEdit {
    pub(crate) field: TocField,
    pub(crate) input: TextInput,
    /// What the user typed, before Tab filled in a suggestion.
    pub(crate) typed: String,
    /// The suggestion Tab filled in last.
    pub(crate) suggestion: Option<usize>,
}

/// One change to write: the full overrides map the addon gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TocApplyRequest {
    pub(crate) folder: String,
    pub(crate) name: String,
    pub(crate) field: TocField,
    /// `None` gives back the addon's own value.
    pub(crate) value: Option<String>,
    pub(crate) overrides: BTreeMap<String, String>,
    /// The folders whose .toc files change: the parent and its own folders.
    pub(crate) folders: Vec<String>,
}

/// What the editor's keys ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TocEditMessage {
    Open,
    Next,
    Previous,
    BeginEdit,
    InputChar(char),
    Backspace,
    NextSuggestion,
    CancelEdit,
    Save,
    ConfirmApply,
    CancelApply,
}

impl TocEditPane {
    pub(crate) fn for_folder(folder: &str) -> Self {
        Self {
            folder: folder.to_string(),
            ..Self::default()
        }
    }

    pub(crate) fn selected_field(&self) -> TocField {
        TocField::ALL[self.selected.min(TocField::ALL.len() - 1)]
    }

    fn moved(&self, delta: isize) -> Self {
        let mut next = self.clone();
        let count = TocField::ALL.len() as isize;
        next.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
        next
    }

    /// Applies `change` to the box and forgets the suggestion Tab filled in.
    pub(crate) fn edit_text(&self, change: impl FnOnce(&mut TextInput)) -> Self {
        let mut next = self.clone();
        if let Some(edit) = next.edit.as_mut() {
            change(&mut edit.input);
            edit.typed = edit.input.value().to_string();
            edit.suggestion = None;
        }
        next
    }
}

/// The name a field goes by in messages.
fn field_label(field: TocField) -> &'static str {
    field.name()
}

/// What a field is, in one sentence.
fn field_meaning(field: TocField) -> &'static str {
    match field {
        TocField::Title => "The name the game's AddOns list shows for this addon.",
        TocField::Notes => {
            "The text the game shows when you point at this addon in its AddOns list."
        }
        TocField::Category => "The heading this addon sits under in the game's AddOns list.",
    }
}

fn source_label(source: TocValueSource) -> &'static str {
    match source {
        TocValueSource::Addon => "from the addon",
        TocValueSource::Override => "set by you",
        TocValueSource::OverrideNotApplied => "set by you, not in the files",
    }
}

impl App {
    /// The addon the editor is open for.
    fn toc_edit_item(&self) -> Option<&DashboardItem> {
        self.dashboard
            .items
            .iter()
            .find(|item| item.folder == self.toc_pane.folder)
    }

    /// What the last scan read from `folder`'s .toc, if it has been read.
    fn toc_scanned(&self, folder: &str) -> Option<&TocFieldValues> {
        self.dashboard
            .drift_report()
            .and_then(|report| report.toc_fields.get(folder))
    }

    /// The value the game uses for `field` of `item`, and where it comes
    /// from. Before a scan has read the addon, a stored value counts as in
    /// the files and the addon's own is not known.
    pub(super) fn toc_field_state(
        &self,
        item: &DashboardItem,
        field: TocField,
    ) -> (Option<String>, TocValueSource) {
        match self.toc_scanned(&item.folder) {
            Some(scanned) => {
                let effective = scanned.get(field);
                (
                    effective.map(str::to_string),
                    toc_edit::value_source(field, &item.toc_overrides, effective),
                )
            }
            None => match item.toc_overrides.get(field.name()) {
                Some(value) => (Some(value.clone()), TocValueSource::Override),
                None => (None, TocValueSource::Addon),
            },
        }
    }

    /// Details' Category row: the heading the game uses and, when it is
    /// yours, that it is. `None` when there is neither.
    pub(super) fn details_category(&self, item: &DashboardItem) -> Option<String> {
        let (value, source) = self.toc_field_state(item, TocField::Category);
        let value = value.map(|value| toc_edit::display_value(&value));
        match source {
            TocValueSource::Addon => value,
            TocValueSource::Override => value.map(|value| format!("{value} (set by you)")),
            TocValueSource::OverrideNotApplied => Some(format!(
                "{} (set by you, not in the files)",
                value.unwrap_or_else(|| "none".to_string())
            )),
        }
    }

    /// The Category suggestions for what is in the box: headings other
    /// addons use, and the game's own. Empty unless Category is edited.
    pub(super) fn toc_suggestions(&self) -> Vec<String> {
        let Some(edit) = self
            .toc_pane
            .edit
            .as_ref()
            .filter(|edit| edit.field == TocField::Category)
        else {
            return Vec::new();
        };
        let mut used: Vec<&str> = self
            .dashboard
            .drift_report()
            .map(|report| {
                report
                    .toc_fields
                    .iter()
                    .filter(|(folder, _)| **folder != self.toc_pane.folder)
                    .filter_map(|(_, fields)| fields.get(TocField::Category))
                    .collect()
            })
            .unwrap_or_default();
        used.sort_unstable();
        toc_edit::category_suggestions(&edit.typed, used, SUGGESTION_LIMIT)
    }

    /// Keys of the editor sheet. While a field is edited the box takes
    /// every letter, `q` `g` `j` `k` included. `None` leaves the key to the
    /// keys every sheet has.
    pub(super) fn toc_edit_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        if self.dashboard.detail_mode != DetailMode::TocEdit {
            return None;
        }
        let message = |message| Some(vec![AppMessage::TocEdit(message)]);
        if let Some(edit) = &self.toc_pane.edit {
            return match key.code {
                KeyCode::Enter => message(TocEditMessage::Save),
                KeyCode::Esc => message(TocEditMessage::CancelEdit),
                KeyCode::Backspace => message(TocEditMessage::Backspace),
                KeyCode::Tab
                    if edit.field == TocField::Category && !self.toc_suggestions().is_empty() =>
                {
                    message(TocEditMessage::NextSuggestion)
                }
                KeyCode::Char(character) => message(TocEditMessage::InputChar(character)),
                _ => Some(vec![]),
            };
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => message(TocEditMessage::Next),
            KeyCode::Up | KeyCode::Char('k') => message(TocEditMessage::Previous),
            KeyCode::Enter => message(TocEditMessage::BeginEdit),
            _ => None,
        }
    }

    /// Why a .toc change can't start now, if it can't.
    fn toc_apply_busy(&self) -> Option<&'static str> {
        if self.toc_apply_running {
            Some("Wait for the .toc change to finish first.")
        } else if self.dashboard_job_running() == Some(DashboardJobKind::Update) {
            Some("Wait for the update to finish first.")
        } else {
            self.install_busy_status()
        }
    }

    fn toc_status(&self, kind: StatusKind, message: &str) -> AppAction {
        AppAction::SetStatus(self.dashboard_status_for(kind, DetailMode::TocEdit, message))
    }

    pub(super) fn toc_edit_actions(&self, message: TocEditMessage) -> Vec<AppAction> {
        let pane = &self.toc_pane;
        match message {
            TocEditMessage::Open => match self.inspect_resolved_target() {
                Some(target) => vec![
                    AppAction::SetTocEditPane(TocEditPane::for_folder(&target.item.folder)),
                    AppAction::SetDetailMode(DetailMode::TocEdit),
                    AppAction::SetStatus(StatusMessage::cleared()),
                ],
                None => vec![AppAction::SetStatus(self.dashboard_status_for(
                    StatusKind::Note,
                    DetailMode::Overview,
                    "Move to an addon first",
                ))],
            },
            TocEditMessage::Next => vec![AppAction::SetTocEditPane(pane.moved(1))],
            TocEditMessage::Previous => vec![AppAction::SetTocEditPane(pane.moved(-1))],
            TocEditMessage::BeginEdit => {
                let Some(item) = self.toc_edit_item() else {
                    return vec![];
                };
                let field = pane.selected_field();
                // Your value when you have one, else the addon's own.
                let start = item
                    .toc_overrides
                    .get(field.name())
                    .cloned()
                    .or_else(|| self.toc_field_state(item, field).0)
                    .unwrap_or_default();
                let mut next = pane.clone();
                next.edit = Some(TocFieldEdit {
                    field,
                    input: TextInput::new(start.clone()),
                    typed: start,
                    suggestion: None,
                });
                vec![
                    AppAction::SetTocEditPane(next),
                    AppAction::SetStatus(StatusMessage::cleared()),
                ]
            }
            TocEditMessage::InputChar(character) => vec![AppAction::SetTocEditPane(
                pane.edit_text(|input| input.insert_char(character)),
            )],
            TocEditMessage::Backspace => vec![AppAction::SetTocEditPane(
                pane.edit_text(TextInput::backspace),
            )],
            TocEditMessage::NextSuggestion => {
                let suggestions = self.toc_suggestions();
                let mut next = pane.clone();
                if let Some(edit) = next.edit.as_mut()
                    && !suggestions.is_empty()
                {
                    let index = edit
                        .suggestion
                        .map_or(0, |index| (index + 1) % suggestions.len());
                    edit.input = TextInput::new(suggestions[index].clone());
                    edit.suggestion = Some(index);
                }
                vec![AppAction::SetTocEditPane(next)]
            }
            TocEditMessage::CancelEdit => {
                let mut next = pane.clone();
                next.edit = None;
                vec![
                    AppAction::SetTocEditPane(next),
                    AppAction::SetStatus(StatusMessage::cleared()),
                ]
            }
            TocEditMessage::Save => self.toc_save_actions(),
            TocEditMessage::ConfirmApply => {
                let mut next = pane.clone();
                let Some(request) = next.pending.take() else {
                    return vec![];
                };
                let mut actions = vec![AppAction::SetTocEditPane(next)];
                actions.extend(self.toc_start_actions(request));
                actions
            }
            TocEditMessage::CancelApply => {
                let mut next = pane.clone();
                let name = next
                    .pending
                    .take()
                    .map(|request| request.name)
                    .unwrap_or_default();
                vec![
                    AppAction::SetTocEditPane(next),
                    self.toc_status(
                        StatusKind::Note,
                        &format!("Left {name}'s files as they are"),
                    ),
                ]
            }
        }
    }

    /// Enter in the box: work out the new overrides and write them, after
    /// the question for a "Not linked" addon. An empty box gives back the
    /// addon's own value; a value that changes nothing writes nothing.
    fn toc_save_actions(&self) -> Vec<AppAction> {
        let pane = &self.toc_pane;
        let (Some(edit), Some(item)) = (pane.edit.as_ref(), self.toc_edit_item()) else {
            return vec![];
        };
        let field = edit.field;
        let typed = edit.input.value().trim();
        let value = (!typed.is_empty()).then(|| typed.to_string());
        let mut overrides = item.toc_overrides.clone();
        if let Err(error) = toc_edit::set_override(&mut overrides, field, value.as_deref()) {
            return vec![self.toc_status(
                StatusKind::Problem,
                &format!("Couldn't save {}: {error}", field_label(field)),
            )];
        }
        let mut closed = pane.clone();
        closed.edit = None;

        let (effective, source) = self.toc_field_state(item, field);
        let unchanged =
            overrides == item.toc_overrides && source != TocValueSource::OverrideNotApplied;
        let same_as_own = source == TocValueSource::Addon
            && value.is_some()
            && value.as_deref() == effective.as_deref().map(str::trim);
        if unchanged || same_as_own {
            return vec![
                AppAction::SetTocEditPane(closed),
                self.toc_status(StatusKind::Note, "Nothing changed"),
            ];
        }
        if let Some(busy) = self.toc_apply_busy() {
            return vec![self.toc_status(StatusKind::Note, busy)];
        }

        let mut folders = vec![item.folder.clone()];
        for owned in &item.owned_folders {
            if !folders
                .iter()
                .any(|known| known.eq_ignore_ascii_case(owned))
            {
                folders.push(owned.clone());
            }
        }
        if let Some(addon_dir) = &self.effective_addon_dir {
            folders.retain(|folder| addon_dir.join(folder).exists());
        }
        let request = TocApplyRequest {
            folder: item.folder.clone(),
            name: item.name.clone(),
            field,
            value,
            overrides,
            folders,
        };
        if item.source == SourceKind::Manual {
            closed.pending = Some(request);
            return vec![
                AppAction::SetTocEditPane(closed),
                AppAction::SetStatus(StatusMessage::cleared()),
            ];
        }
        let mut actions = vec![AppAction::SetTocEditPane(closed)];
        actions.extend(self.toc_start_actions(request));
        actions
    }

    fn toc_start_actions(&self, request: TocApplyRequest) -> Vec<AppAction> {
        if let Some(busy) = self.toc_apply_busy() {
            return vec![self.toc_status(StatusKind::Note, busy)];
        }
        let Some(addon_dir) = self.effective_addon_dir.clone() else {
            return vec![self.toc_status(StatusKind::Problem, "Choose your game folder first")];
        };
        let working = format!("Writing {}'s .toc files", request.name);
        vec![
            AppAction::SetTocApplyInProgress(true),
            AppAction::StartTocApply { addon_dir, request },
            self.toc_status(StatusKind::Working, &working),
        ]
    }

    /// The end of a .toc change: the list, Details and the editor show what
    /// the files say now. When the game folder changed while it ran, the
    /// result belongs to the old folder, so the screen keeps showing the new
    /// one and says nothing about it.
    pub(super) fn toc_apply_finished_actions(
        &self,
        addon_dir: &Path,
        request: TocApplyRequest,
        result: std::result::Result<Option<AddonScanOutcome>, String>,
    ) -> Vec<AppAction> {
        if !self
            .effective_addon_dir
            .as_deref()
            .is_some_and(|current| same_addons_folder(current, addon_dir))
        {
            tracing::info!(
                addon_dir = %addon_dir.display(),
                "a .toc change finished after the game folder changed"
            );
            return vec![AppAction::SetTocApplyInProgress(false)];
        }
        match result {
            Ok(sync) => {
                let mut actions = Vec::new();
                if let Some(sync) = sync {
                    actions.extend([
                        AppAction::ReplaceDashboardAddons(sync.addons),
                        AppAction::SetDashboardDriftReport(Some(sync.drift_report)),
                        AppAction::CompleteAddonScan {
                            path: sync.path,
                            summary: sync.summary,
                        },
                    ]);
                }
                let field = field_label(request.field);
                let message = match request.value {
                    Some(_) => format!(
                        "Saved {field} for {} · /reload in the game to see it",
                        request.name
                    ),
                    None => format!("Restored {}'s own {field}", request.name),
                };
                actions.push(AppAction::SetTocApplyInProgress(false));
                actions.push(self.toc_status(StatusKind::Done, &message));
                actions
            }
            Err(reason) => vec![
                AppAction::SetTocApplyInProgress(false),
                self.toc_status(StatusKind::Problem, &reason),
            ],
        }
    }

    /// The question before a "Not linked" addon's files change.
    pub(super) fn toc_apply_question(&self) -> Option<Question> {
        let request = self.toc_pane.pending.as_ref()?;
        (self.dashboard.detail_mode == DetailMode::TocEdit).then(|| Question::ChangeTocFiles {
            name: request.name.clone(),
            field: field_label(request.field),
            restore: request.value.is_none(),
            folders: request.folders.clone(),
        })
    }

    /// The editor: the addon, its three fields with where each value comes
    /// from, suggestions under the Category box, and the focused row
    /// explained at the bottom with the last result under it.
    ///
    /// The sheet keeps its size and place while it is open: it reserves the
    /// rows of the longest explanation, a full list of suggestions and the
    /// result line, and pads shorter states with blank rows at the bottom of
    /// each area, so the result line stays put too. With little room the
    /// blank rows go first, then the explanation is cut to one line.
    pub(super) fn toc_edit_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let roles = self.ui_theme.roles;
        let width = room.width;
        let mut body = SheetBody::new();
        let Some(item) = self.toc_edit_item() else {
            body.text(vec![Line::from(Span::styled(
                "This addon is not in the list any more.",
                Style::default().fg(roles.muted),
            ))]);
            return body;
        };
        let long_rows = self.toc_reserved_rows(item, width);
        // The name, the three fields and the suggestions, then the
        // explanation and the result line; spaced adds two blank rows.
        let list = 1 + TocField::ALL.len() as u16 + SUGGESTION_LIMIT as u16;
        let rows =
            |spaced: bool, explanation: u16| list + explanation + 1 + if spaced { 2 } else { 0 };
        let layouts = [
            rows(true, long_rows),
            rows(false, long_rows),
            rows(false, 1),
        ];
        let layout = room.pick(&layouts);
        body.reserve(layouts[layout]);
        let (spaced, long) = (layout == 0, layout < 2);
        let linked = if item.source == SourceKind::Manual {
            " · Not linked"
        } else {
            ""
        };
        body.text(vec![Line::from(vec![
            Span::styled(
                item.name.clone(),
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(linked, Style::default().fg(roles.muted)),
        ])]);
        if spaced {
            body.gap(1);
        }
        for (index, field) in TocField::ALL.into_iter().enumerate() {
            body.text(vec![self.toc_field_line(item, field, index, width)]);
            if index == self.toc_pane.selected.min(TocField::ALL.len() - 1) {
                body.focus_last();
            }
            if field == TocField::Category {
                // Room for a full list, whether or not one is showing.
                let mut lines = self.toc_suggestion_lines(width);
                lines.resize_with(SUGGESTION_LIMIT, Line::default);
                body.text(lines);
            }
        }
        let mut footer = Vec::new();
        if spaced {
            footer.push(Line::default());
        }
        let line = self.toc_explanation_line(item);
        let mut explanation = wrap_lines(std::slice::from_ref(&line), width);
        if long {
            explanation.resize_with(usize::from(long_rows).max(explanation.len()), Line::default);
        } else if explanation.len() > 1 {
            let mut first = wrap_lines(std::slice::from_ref(&line), width.saturating_sub(1))
                .into_iter()
                .next()
                .unwrap_or_default();
            first.spans.push(Span::raw("…"));
            explanation = vec![first];
        }
        footer.extend(explanation);
        footer.push(self.toc_status_line(width).unwrap_or_default());
        body.footer(footer);
        body
    }

    /// Rows the long explanation reserves at `width`: the tallest one for
    /// any row in any state. The one on screen counts too.
    fn toc_reserved_rows(&self, item: &DashboardItem, width: u16) -> u16 {
        let rows = |line: Line<'static>| text_height(&[line], width);
        let mut most = rows(self.toc_explanation_line(item));
        for tab_hint in [false, true] {
            most = most.max(rows(self.toc_explanation(
                TocField::Category,
                TocValueSource::Addon,
                Some(tab_hint),
                "",
            )));
        }
        let yours = "x".repeat(YOURS_WIDTH);
        for field in TocField::ALL {
            for source in [
                TocValueSource::Addon,
                TocValueSource::Override,
                TocValueSource::OverrideNotApplied,
            ] {
                most = most.max(rows(self.toc_explanation(field, source, None, &yours)));
            }
        }
        most
    }

    /// "▌ Category  Bags                       set by you", or the box.
    fn toc_field_line(
        &self,
        item: &DashboardItem,
        field: TocField,
        index: usize,
        width: u16,
    ) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let width = usize::from(width);
        let selected = index == self.toc_pane.selected.min(TocField::ALL.len() - 1);
        let bold = |style: Style| {
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            }
        };
        let mut spans = vec![
            Span::styled(
                if selected { "▌ " } else { "  " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(
                format!("{:<LABEL_WIDTH$}", field_label(field)),
                bold(Style::default().fg(roles.text)),
            ),
        ];
        let rest = width.saturating_sub(BAR_WIDTH + LABEL_WIDTH).max(1);
        if let Some(edit) = self
            .toc_pane
            .edit
            .as_ref()
            .filter(|edit| edit.field == field)
        {
            spans.extend(
                edit.input.spans(
                    rest,
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(roles.lemon),
                    true,
                ),
            );
            if edit.input.is_empty() {
                spans.push(Span::styled(
                    " empty gives back the addon's own",
                    Style::default().fg(roles.muted),
                ));
            }
            return Line::from(spans);
        }

        let (value, source) = self.toc_field_state(item, field);
        let source_width = SOURCE_WIDTH.min(rest / 2);
        let value_width = rest.saturating_sub(source_width + 2).max(1);
        let (value_text, value_style) = match value {
            Some(value) => (
                toc_edit::display_value(&value),
                bold(Style::default().fg(roles.text)),
            ),
            None if self.toc_scanned(&item.folder).is_none() => {
                ("not read yet".to_string(), Style::default().fg(roles.muted))
            }
            None => ("none".to_string(), Style::default().fg(roles.muted)),
        };
        let source_color = match source {
            TocValueSource::Addon => roles.muted,
            TocValueSource::Override => roles.green,
            TocValueSource::OverrideNotApplied => roles.amber,
        };
        spans.push(Span::styled(
            format!(
                "{:<value_width$}  ",
                truncate_text(&value_text, value_width)
            ),
            value_style,
        ));
        spans.push(Span::styled(
            truncate_text(source_label(source), source_width),
            Style::default().fg(source_color),
        ));
        Line::from(spans)
    }

    /// Up to five headings under the Category box, the one Tab filled in
    /// highlighted.
    fn toc_suggestion_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let suggestions = self.toc_suggestions();
        let indent = " ".repeat(BAR_WIDTH + LABEL_WIDTH);
        let room = usize::from(width)
            .saturating_sub(BAR_WIDTH + LABEL_WIDTH)
            .max(1);
        let chosen = self.toc_pane.edit.as_ref().and_then(|edit| edit.suggestion);
        suggestions
            .into_iter()
            .enumerate()
            .map(|(index, suggestion)| {
                let style = if chosen == Some(index) {
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(roles.muted)
                };
                Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::styled(truncate_text(&suggestion, room), style),
                ])
            })
            .collect()
    }

    /// What the focused row is, and what its keys do there.
    fn toc_explanation_line(&self, item: &DashboardItem) -> Line<'static> {
        let field = self.toc_pane.selected_field();
        let editing = self
            .toc_pane
            .edit
            .as_ref()
            .map(|edit| edit.field == TocField::Category && !self.toc_suggestions().is_empty());
        let (_, source) = self.toc_field_state(item, field);
        let yours = item
            .toc_overrides
            .get(field.name())
            .map(|value| toc_edit::display_value(value))
            .unwrap_or_default();
        self.toc_explanation(field, source, editing, &yours)
    }

    /// The explanation of `field` with its value `source`, or of the box
    /// while it is edited (`editing` says whether Tab has a suggestion to
    /// fill in). `yours` is what the "not in the files" one quotes.
    fn toc_explanation(
        &self,
        field: TocField,
        source: TocValueSource,
        editing: Option<bool>,
        yours: &str,
    ) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let muted = Style::default().fg(roles.muted);
        if let Some(tab_hint) = editing {
            let mut text = "enter saves it and writes it to the addon's files, esc keeps what \
                            you had. An empty box gives back the addon's own."
                .to_string();
            if tab_hint {
                text.push_str(" tab fills in the next suggestion.");
            }
            return Line::from(Span::styled(text, muted));
        }
        let mut spans = vec![Span::styled(
            field_meaning(field),
            Style::default().fg(roles.text),
        )];
        match source {
            TocValueSource::Addon => {}
            TocValueSource::Override => spans.push(Span::styled(
                " LemonUp keeps your value through updates.",
                muted,
            )),
            TocValueSource::OverrideNotApplied => {
                let yours = truncate_text(yours, YOURS_WIDTH);
                spans.push(Span::styled(
                    format!(
                        " Your value \"{yours}\" is not in the files (another program may have \
                         changed them). enter, then enter again, writes it back."
                    ),
                    Style::default().fg(roles.amber),
                ));
            }
        }
        Line::from(spans)
    }

    /// What the last save said, pinned under the explanation (the status
    /// line shows nothing under a sheet).
    fn toc_status_line(&self, width: u16) -> Option<Line<'static>> {
        let roles = self.ui_theme.roles;
        let message = self.active_detail_status_message(DetailMode::TocEdit)?;
        let (text, color) = match self.status_line.kind {
            StatusKind::Problem => (message, roles.red),
            StatusKind::Done => (
                format!("{} {message}", self.ui_theme.symbols.done),
                roles.green,
            ),
            _ => (message, roles.text),
        };
        Some(Line::from(Span::styled(
            truncate_text(&text, usize::from(width)),
            Style::default().fg(color),
        )))
    }
}

/// Writes `request` into the addon's files and stores it, then reads the
/// AddOns folder again so the list shows the new name. The error is the
/// reason in plain words. A failed rescan after a good write is no failure:
/// the list catches up on the next scan.
pub(super) fn run_toc_apply(
    state_db_file: &Path,
    addon_dir: &Path,
    request: &TocApplyRequest,
) -> std::result::Result<Option<AddonScanOutcome>, String> {
    let name = &request.name;
    {
        let mut database = StateDatabase::open(state_db_file).map_err(|error| error.to_string())?;
        apply_toc_overrides_now(
            &mut database,
            addon_dir,
            &request.folder,
            &request.overrides,
        )
        .map_err(|error| match error {
            TocApplyError::Symlink { .. } => {
                format!("{name} has a symlink inside, so LemonUp won't edit it")
            }
            TocApplyError::HeldByOther { folder, owner } => {
                format!("{folder} belongs to {owner}, so LemonUp won't edit {name}'s files")
            }
            TocApplyError::Missing { .. } => {
                format!("LemonUp can't find {name} in your AddOns folder")
            }
            TocApplyError::Other(reason) => {
                format!("Couldn't change {name}'s files: {reason}")
            }
        })?;
    }
    // The game folder may have changed while the files were written: the
    // state then belongs to the new folder, and reading the old one into it
    // would drop the new folder's rows.
    if !still_bound_to(state_db_file, addon_dir) {
        tracing::info!(
            addon_dir = %addon_dir.display(),
            "the game folder changed during a .toc change; not reading the old folder again"
        );
        return Ok(None);
    }
    match sync_dashboard_state(state_db_file, addon_dir) {
        Ok(sync) => Ok(Some(sync)),
        Err(error) => {
            tracing::warn!(%error, "could not read the AddOns folder after a .toc change");
            Ok(None)
        }
    }
}

/// Whether the profile's state still belongs to `addon_dir`. A database
/// without a binding yet counts as bound; one that can't be read does not,
/// so nothing is reconciled into it.
fn still_bound_to(state_db_file: &Path, addon_dir: &Path) -> bool {
    match StateDatabase::open(state_db_file).and_then(|database| database.bound_addon_dir()) {
        Ok(Some(bound)) => same_addons_folder(&bound, addon_dir),
        Ok(None) => true,
        Err(error) => {
            tracing::warn!(%error, "could not read which AddOns folder the profile is bound to");
            false
        }
    }
}
