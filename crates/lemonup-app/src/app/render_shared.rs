use super::sheet::{SheetBody, SheetChrome};
use super::*;
impl App {
    pub(super) fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.ui_theme.border));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let padded = if inner.width > 8 {
            inner.inner(Margin {
                horizontal: 2,
                vertical: 0,
            })
        } else {
            inner
        };

        let command_lines = self.footer_command_lines();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(padded);

        let status_lines = self.footer_status_lines();

        let right_width = command_lines
            .iter()
            .map(|line| self.footer_hint_line_width(line) as u16)
            .max()
            .unwrap_or(0)
            .min(padded.width.saturating_sub(20));

        for (index, hints) in command_lines.iter().enumerate() {
            let row = rows[index];
            let status_line = status_lines[index].clone();
            let status_is_empty = self.line_is_empty(&status_line);

            if status_is_empty {
                if !hints.is_empty() {
                    let commands = Paragraph::new(self.footer_hint_line(hints))
                        .alignment(Alignment::Center)
                        .wrap(Wrap { trim: false });
                    frame.render_widget(commands, row);
                }
            } else {
                let [left, _, right] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Fill(1),
                        Constraint::Length(if right_width > 0 { 3 } else { 0 }),
                        Constraint::Length(right_width),
                    ])
                    .areas(row);

                let status = Paragraph::new(status_line)
                    .alignment(Alignment::Left)
                    .wrap(Wrap { trim: false });
                frame.render_widget(status, left);

                if !hints.is_empty() {
                    let commands = Paragraph::new(self.footer_hint_line(hints))
                        .alignment(Alignment::Right)
                        .wrap(Wrap { trim: false });
                    frame.render_widget(commands, right);
                }
            }
        }
    }

    pub(super) fn line_is_empty(&self, line: &Line<'_>) -> bool {
        line.spans.iter().all(|span| span.content.is_empty())
    }

    /// Draws the open overlay as a sheet. `region` is the space above the
    /// status line and the key line.
    pub(super) fn render_overlay_host(&self, frame: &mut Frame<'_>, region: Rect, header: u16) {
        let Some(kind) = self.shell_ui.overlay.active else {
            // A closed sheet opens at the top next time.
            if self.sheet_scroll.get().kind.is_some() {
                self.sheet_scroll.set(SheetScroll::default());
            }
            return;
        };
        let title = match kind {
            OverlayKind::Inspect => self
                .inspect_resolved_target()
                .map(|target| target.item.name.clone())
                .unwrap_or_else(|| "Details".to_string()),
            OverlayKind::Help => super::keymap::help_page(self.shell_ui.help_context)
                .title
                .to_string(),
            OverlayKind::Install | OverlayKind::Search => "Install".to_string(),
            OverlayKind::Update => "Update".to_string(),
            OverlayKind::Config => "Settings".to_string(),
            OverlayKind::Backup => "Backups of your game settings".to_string(),
            OverlayKind::Confirm => "Confirm".to_string(),
        };
        let chrome = SheetChrome {
            title,
            danger: kind == OverlayKind::Confirm
                && self.dashboard.pending_delete_folders().is_some(),
        };
        match kind {
            OverlayKind::Inspect => {
                self.render_sheet(frame, region, header, kind, chrome, |width, _| {
                    self.inspect_sheet_body(width)
                })
            }
            OverlayKind::Help => {
                self.render_sheet(frame, region, header, kind, chrome, |width, _| {
                    self.help_sheet_body(width)
                })
            }
            OverlayKind::Install | OverlayKind::Search => {
                self.render_sheet(frame, region, header, kind, chrome, |width, height| {
                    self.search_sheet_body(width, height)
                })
            }
            OverlayKind::Config => {
                self.render_sheet(frame, region, header, kind, chrome, |width, _| {
                    self.config_sheet_body(width)
                })
            }
            OverlayKind::Backup => {
                self.render_sheet(frame, region, header, kind, chrome, |width, height| {
                    self.backup_sheet_body(width, height)
                })
            }
            OverlayKind::Update | OverlayKind::Confirm => {
                self.render_sheet(frame, region, header, kind, chrome, |_, _| {
                    let mut body = SheetBody::new();
                    body.text(self.task_overlay_lines());
                    body
                })
            }
        }
    }

    pub(super) fn fruit_gradient_logo_line(&self, text: &str) -> Line<'static> {
        let anchors = [
            self.ui_theme.brand_hot,
            self.ui_theme.brand_warm,
            self.ui_theme.brand_gold,
        ];
        let visible_count = text.chars().filter(|ch| !ch.is_whitespace()).count();
        if visible_count == 0 {
            return Line::from(text.to_string());
        }

        let mut spans = Vec::with_capacity(text.len());
        let mut visible_index = 0usize;
        for ch in text.chars() {
            if ch.is_whitespace() {
                spans.push(Span::raw(ch.to_string()));
                continue;
            }

            let t = if visible_count <= 1 {
                0.0
            } else {
                visible_index as f32 / (visible_count - 1) as f32
            };
            let color = interpolate_palette(&anchors, t);
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            visible_index = visible_index.wrapping_add(1);
        }

        Line::from(spans)
    }

    pub(super) fn dashboard_surface_label(&self) -> &'static str {
        match self.shell_ui.overlay.active {
            Some(OverlayKind::Inspect) => "Details",
            _ => detail_mode_label(self.dashboard.detail_mode),
        }
    }

    pub(super) fn footer_status_lines(&self) -> Vec<Line<'static>> {
        vec![
            self.footer_primary_status_line(),
            self.footer_secondary_status_line(),
        ]
    }

    pub(super) fn footer_primary_status_line(&self) -> Line<'static> {
        if self.shell_mode == ShellMode::Dashboard
            && let Some(job_line) = self.dashboard_job_footer_line()
        {
            return job_line;
        }

        Line::from(vec![Span::styled(
            truncate_text(&self.footer_status_text(), 120),
            Style::default().fg(self.status_kind_color(self.status_line.kind)),
        )])
    }

    pub(super) fn footer_secondary_status_line(&self) -> Line<'static> {
        Line::from(vec![Span::styled(
            truncate_text(&self.footer_secondary_status_text(), 120),
            Style::default().fg(self.ui_theme.muted),
        )])
    }

    pub(super) fn footer_status_text(&self) -> String {
        match self.shell_mode {
            ShellMode::Dashboard => self.dashboard_footer_status_text(),
            ShellMode::Onboarding => self.onboarding_footer_status_text(),
        }
    }

    pub(super) fn footer_secondary_status_text(&self) -> String {
        match self.shell_mode {
            ShellMode::Dashboard => self.dashboard_footer_secondary_text(),
            ShellMode::Onboarding => String::new(),
        }
    }

    pub(super) fn dashboard_footer_status_text(&self) -> String {
        if let Some(summary) = self.sanitized_footer_status_text() {
            return summary;
        }

        String::new()
    }

    pub(super) fn dashboard_footer_secondary_text(&self) -> String {
        String::new()
    }

    /// The status line message for `status`, or `None` when it is not
    /// shown: a quiet kind, or a message from inside an overlay (overlays
    /// show their own messages).
    pub(super) fn status_flash_from_status(&self, status: &StatusMessage) -> Option<StatusFlash> {
        if self.shell_mode != ShellMode::Dashboard || self.shell_ui.overlay.active.is_some() {
            return None;
        }

        let summary = self.sanitized_status_text(status)?;
        let expires_at = match status.kind {
            StatusKind::Problem => None,
            StatusKind::Done | StatusKind::Note | StatusKind::Working | StatusKind::Quiet => {
                Some(Instant::now() + MESSAGE_FADE)
            }
        };

        Some(StatusFlash {
            text: truncate_text(&summary, 160),
            kind: status.kind,
            offer: status.offer,
            expires_at,
        })
    }

    pub(super) fn onboarding_footer_status_text(&self) -> String {
        if let Some(summary) = self.sanitized_footer_status_text() {
            return summary;
        }

        String::new()
    }

    pub(super) fn sanitized_footer_status_text(&self) -> Option<String> {
        self.sanitized_status_text(&self.status_line)
    }

    /// The message part of a status, without the surface label and profile
    /// that every status carries. `None` when the kind is not shown.
    pub(super) fn sanitized_status_text(&self, status: &StatusMessage) -> Option<String> {
        if !status.kind.is_shown() {
            return None;
        }

        let mut parts: Vec<String> = status
            .text
            .split('|')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .filter(|part| !part.starts_with("profile "))
            .map(ToString::to_string)
            .collect();

        if self.shell_mode == ShellMode::Dashboard
            && parts
                .first()
                .is_some_and(|first| *first == self.dashboard_surface_label())
        {
            parts.remove(0);
        }

        let summary = parts.join(" · ");
        (!summary.is_empty()).then_some(summary)
    }

    /// Footer colour for a status message, from its kind alone.
    pub(super) fn status_kind_color(&self, kind: StatusKind) -> Color {
        match kind {
            StatusKind::Done => self.ui_theme.success,
            StatusKind::Problem => self.ui_theme.error,
            StatusKind::Working => self.ui_theme.warning,
            StatusKind::Note | StatusKind::Quiet => self.ui_theme.info,
        }
    }

    /// Setup's footer: the key table split over two rows.
    pub(super) fn footer_command_lines(&self) -> Vec<Vec<KeyHint>> {
        let mut primary = Vec::new();
        let mut secondary = Vec::new();
        for hint in self.key_table(u16::MAX) {
            match hint.tier {
                FooterHintTier::Primary => primary.push(hint),
                FooterHintTier::Secondary => secondary.push(hint),
            }
        }
        vec![primary, secondary]
    }

    pub(super) fn footer_hint_line(&self, hints: &[KeyHint]) -> Line<'static> {
        let mut spans = Vec::new();
        for (index, hint) in hints.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            let (key_style, label_style) = self.footer_hint_styles(hint.id);
            spans.push(Span::styled(format!(" {} ", hint.key), key_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(hint.label.clone(), label_style));
        }
        Line::from(spans)
    }

    pub(super) fn footer_hint_line_width(&self, hints: &[KeyHint]) -> usize {
        hints
            .iter()
            .enumerate()
            .map(|(index, hint)| {
                let base = hint.key.chars().count() + 2 + 1 + hint.label.chars().count();
                if index == 0 { base } else { base + 3 }
            })
            .sum()
    }

    pub(super) fn overlay_section_title(&self, title: &str) -> Line<'static> {
        Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(self.ui_theme.highlight)
                .add_modifier(Modifier::BOLD),
        ))
    }

    pub(super) fn overlay_kv_line(
        &self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!("{}: ", label.into()),
                Style::default().fg(self.ui_theme.muted),
            ),
            Span::raw(value.into()),
        ])
    }

    pub(super) fn overlay_hint_line(&self, text: &str) -> Line<'static> {
        Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(self.ui_theme.muted),
        ))
    }
}

pub(super) fn truncate_middle_text(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }

    let visible = max_chars - 1;
    let head = visible / 2;
    let tail = visible - head;

    let prefix = chars.iter().take(head).collect::<String>();
    let suffix = chars[chars.len().saturating_sub(tail)..]
        .iter()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

pub(super) fn truncate_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    let visible = value.chars().take(max_chars - 1).collect::<String>();
    format!("{visible}…")
}

pub(super) fn summarize_owned_folders(folders: &[String]) -> String {
    const LIMIT: usize = 4;
    if folders.is_empty() {
        return "none".to_string();
    }

    let visible = folders.iter().take(LIMIT).cloned().collect::<Vec<_>>();
    if folders.len() <= LIMIT {
        visible.join(", ")
    } else {
        format!(
            "{} +{} more",
            visible.join(", "),
            folders.len().saturating_sub(LIMIT)
        )
    }
}

pub(super) fn visible_search_result_window(
    result_count: usize,
    selected_result: Option<usize>,
    max_visible: usize,
) -> (usize, usize) {
    if result_count == 0 || max_visible == 0 {
        return (0, 0);
    }

    let visible = result_count.min(max_visible);
    let selected = selected_result
        .unwrap_or(0)
        .min(result_count.saturating_sub(1));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let end = (start + visible).min(result_count);
    (start, end)
}

pub(super) fn format_download_count(count: u64) -> String {
    const THOUSAND: u64 = 1_000;
    const MILLION: u64 = 1_000_000;
    const BILLION: u64 = 1_000_000_000;

    match count {
        0..=999 => count.to_string(),
        THOUSAND..=999_999 => format_compact_count(count, THOUSAND, "K"),
        MILLION..=999_999_999 => format_compact_count(count, MILLION, "M"),
        _ => format_compact_count(count, BILLION, "B"),
    }
}
