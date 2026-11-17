use super::keymap::{HelpLabel, HelpRow, HelpSection, HelpWhen, help_page};
use super::sheet::SheetBody;
use super::*;

/// Help shows two columns from this body width; below it, one.
const HELP_TWO_COLUMN_MIN_WIDTH: u16 = 90;
/// Blank columns between the two columns.
const HELP_COLUMN_GAP: usize = 2;

impl App {
    pub(super) fn current_help_context(&self) -> HelpContext {
        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            if self.details_docked() {
                HelpContext::DetailsDocked
            } else {
                HelpContext::Inspect
            }
        } else {
            self.current_help_context_for_mode(self.dashboard.detail_mode)
        }
    }

    pub(super) fn current_help_context_for_mode(&self, mode: DetailMode) -> HelpContext {
        match mode {
            DetailMode::Overview | DetailMode::Update => HelpContext::Overview,
            DetailMode::Install | DetailMode::Search => HelpContext::InstallSearch,
            DetailMode::Config => HelpContext::Config,
            DetailMode::Backup => HelpContext::Backup,
            DetailMode::TocEdit => HelpContext::TocEdit,
        }
    }

    /// Whether a Help row's keys work right now.
    pub(super) fn help_row_applies(&self, when: HelpWhen) -> bool {
        match when {
            HelpWhen::Always => true,
            HelpWhen::NoWagoKey => self.wago_api_key.is_none(),
            HelpWhen::NotLinked => self.not_linked_current_item().is_some(),
            HelpWhen::FirstRunCard => self.first_run_card_visible(),
            HelpWhen::NoJob => self.dashboard_job_running().is_none(),
            // Help opened from docked Details keeps the panel's saved scroll
            // state, so the row shows there too.
            HelpWhen::DetailsOverflow => {
                self.docked_details_overflow()
                    || (matches!(self.shell_ui.help_context, HelpContext::DetailsDocked)
                        && self
                            .docked_details_scroll()
                            .overflows(Some(OverlayKind::Inspect)))
            }
        }
    }

    /// Every key of the context Help was opened from, in two columns when
    /// the sheet is wide enough. The body scrolls.
    pub(super) fn help_sheet_body(&self, width: u16) -> SheetBody<'static> {
        let page = help_page(self.shell_ui.help_context);
        let width = usize::from(width);
        let mut lines = Vec::new();
        if width >= usize::from(HELP_TWO_COLUMN_MIN_WIDTH) {
            // The right column gets the room its longest row needs, the left
            // column the rest.
            let right_keys = self.help_label_width(page.right);
            let right_width = (right_keys + self.help_text_width(page.right))
                .min((width - HELP_COLUMN_GAP) / 2 + 4);
            let left_width = width - HELP_COLUMN_GAP - right_width;
            let left_keys = self.help_label_width(page.left);
            let left = self.help_column(page.left, left_width, left_keys);
            let right = self.help_column(page.right, right_width, right_keys);
            for index in 0..left.len().max(right.len()) {
                let mut spans = Vec::new();
                let left_line = left.get(index).cloned().unwrap_or_default();
                let pad = left_width.saturating_sub(left_line.width()) + HELP_COLUMN_GAP;
                spans.extend(left_line.spans);
                spans.push(Span::raw(" ".repeat(pad)));
                if let Some(right_line) = right.get(index) {
                    spans.extend(right_line.spans.clone());
                }
                lines.push(Line::from(spans));
            }
        } else {
            let keys = self
                .help_label_width(page.left)
                .max(self.help_label_width(page.right));
            lines.extend(self.help_column(page.left, width, keys));
            lines.push(Line::default());
            lines.extend(self.help_column(page.right, width, keys));
        }
        lines.push(Line::default());
        let muted = Style::default().fg(self.ui_theme.roles.muted);
        // LemonUp takes the mouse, so the terminal's own selection needs Shift.
        lines.push(Line::from(Span::styled(
            "Hold Shift while dragging to copy text",
            muted,
        )));
        lines.push(Line::from(vec![
            Span::styled(format!("LemonUp {} · ", env!("CARGO_PKG_VERSION")), muted),
            Span::styled(
                "lemonup.org/keyboard-shortcuts",
                Style::default().fg(self.ui_theme.roles.blue),
            ),
        ]));
        let mut body = SheetBody::new();
        body.text(lines);
        body
    }

    fn help_rows(&self, section: &HelpSection) -> Vec<HelpRow> {
        section
            .rows
            .iter()
            .copied()
            .filter(|row| self.help_row_applies(row.when))
            .collect()
    }

    /// Columns the keys of `sections` take, with two spaces after them.
    /// Status words line up on their own, so they don't push the keys over.
    fn help_label_width(&self, sections: &[HelpSection]) -> usize {
        sections
            .iter()
            .flat_map(|section| self.help_rows(section))
            .filter(|row| !matches!(row.label, HelpLabel::Status(_)))
            .map(|row| {
                self.help_label_spans(row.label)
                    .iter()
                    .map(Span::width)
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
            + 2
    }

    /// The longest text in `sections`.
    fn help_text_width(&self, sections: &[HelpSection]) -> usize {
        sections
            .iter()
            .flat_map(|section| self.help_rows(section))
            .map(|row| row.text.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// One column of Help: each group's title, then its rows with the keys
    /// lined up and the words wrapped beside them.
    fn help_column(
        &self,
        sections: &[HelpSection],
        width: usize,
        key_width: usize,
    ) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let mut lines = Vec::new();
        for section in sections {
            let rows = self.help_rows(section);
            if rows.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(
                section.title,
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
            )));
            let status = matches!(rows[0].label, HelpLabel::Status(_));
            let label_width = if status {
                rows.iter()
                    .map(|row| {
                        self.help_label_spans(row.label)
                            .iter()
                            .map(Span::width)
                            .sum::<usize>()
                    })
                    .max()
                    .unwrap_or(0)
                    + 2
            } else {
                key_width
            };
            let text_style = if status {
                Style::default().fg(roles.muted)
            } else {
                Style::default().fg(roles.text)
            };
            let text_width = width.saturating_sub(label_width).max(10);
            for row in rows {
                let label = self.help_label_spans(row.label);
                let used: usize = label.iter().map(Span::width).sum();
                for (index, part) in wrap_words(row.text, text_width).into_iter().enumerate() {
                    let mut spans = if index == 0 {
                        let mut spans = label.clone();
                        spans.push(Span::raw(" ".repeat(label_width.saturating_sub(used))));
                        spans
                    } else {
                        vec![Span::raw(" ".repeat(label_width))]
                    };
                    spans.push(Span::styled(part, text_style));
                    lines.push(Line::from(spans));
                }
            }
        }
        lines
    }

    fn help_label_spans(&self, label: HelpLabel) -> Vec<Span<'static>> {
        let roles = self.ui_theme.roles;
        match label {
            HelpLabel::Keys(keys) => vec![Span::styled(
                keys,
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            )],
            HelpLabel::Status(status) => status.spans(" ", &self.ui_theme),
            HelpLabel::Typed(text) => vec![Span::styled(text, Style::default().fg(roles.text))],
        }
    }
}

/// Splits `text` into rows of at most `width` columns at spaces.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let needed = row.chars().count() + usize::from(!row.is_empty()) + word.chars().count();
        if !row.is_empty() && needed > width {
            rows.push(std::mem::take(&mut row));
        }
        if !row.is_empty() {
            row.push(' ');
        }
        row.push_str(word);
    }
    rows.push(row);
    rows
}
