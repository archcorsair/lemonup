use super::sheet::{SheetBody, SheetChrome};
use super::*;
impl App {
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
            OverlayKind::Install | OverlayKind::Search => "Install an addon".to_string(),
            OverlayKind::Update => "Update".to_string(),
            OverlayKind::Config => "Settings".to_string(),
            OverlayKind::Backup => "Backups of your game settings".to_string(),
            OverlayKind::TocEdit => "Edit .toc fields".to_string(),
            OverlayKind::Confirm => "Confirm".to_string(),
            OverlayKind::Games => "Switch game".to_string(),
            OverlayKind::Find => "Find your addons online".to_string(),
        };
        let chrome = SheetChrome {
            title,
            note: (kind == OverlayKind::Config).then_some("changes save right away"),
            danger: kind == OverlayKind::Confirm
                && self.dashboard.pending_delete_folders().is_some(),
        };
        match kind {
            OverlayKind::Inspect => {
                self.render_sheet(frame, region, header, kind, chrome, |room| {
                    self.inspect_sheet_body(room)
                })
            }
            OverlayKind::Help => self.render_sheet(frame, region, header, kind, chrome, |room| {
                self.help_sheet_body(room.width)
            }),
            OverlayKind::Install | OverlayKind::Search => {
                self.render_sheet(frame, region, header, kind, chrome, |room| {
                    self.search_sheet_body(room)
                })
            }
            OverlayKind::Config => self.render_sheet(frame, region, header, kind, chrome, |room| {
                self.config_sheet_body(room)
            }),
            OverlayKind::Backup => self.render_sheet(frame, region, header, kind, chrome, |room| {
                self.backup_sheet_body(room)
            }),
            OverlayKind::Games => self.render_sheet(frame, region, header, kind, chrome, |room| {
                self.games_sheet_body(room)
            }),
            OverlayKind::Find => self.render_sheet(frame, region, header, kind, chrome, |room| {
                self.find_sheet_body(room)
            }),
            OverlayKind::TocEdit => {
                self.render_sheet(frame, region, header, kind, chrome, |room| {
                    self.toc_edit_sheet_body(room)
                })
            }
            OverlayKind::Update | OverlayKind::Confirm => {
                self.render_sheet(frame, region, header, kind, chrome, |_| {
                    let mut body = SheetBody::new();
                    body.text(self.task_overlay_lines());
                    body
                })
            }
        }
    }

    pub(super) fn fruit_gradient_logo_line(&self, text: &str) -> Line<'static> {
        let anchors = self.ui_theme.roles.logo_colors();
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

    /// The status message as the status line words it, empty when none
    /// shows.
    #[cfg(test)]
    pub(super) fn footer_status_text(&self) -> String {
        self.sanitized_status_text(&self.status_line)
            .unwrap_or_default()
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

    pub(super) fn overlay_section_title(&self, title: &str) -> Line<'static> {
        Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(self.ui_theme.roles.lemon)
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
                Style::default().fg(self.ui_theme.roles.muted),
            ),
            Span::raw(value.into()),
        ])
    }

    pub(super) fn overlay_hint_line(&self, text: &str) -> Line<'static> {
        Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(self.ui_theme.roles.muted),
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
