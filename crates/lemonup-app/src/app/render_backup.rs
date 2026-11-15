use super::sheet::{SheetBody, text_height};
use super::*;

impl App {
    /// The summary, the list of backups, and the card for the selected one.
    /// The list gets the rows the rest leaves, so the card always shows.
    /// Restoring and deleting ask in the question dialog, over this sheet.
    pub(super) fn backup_sheet_body(&self, width: u16, max_height: u16) -> SheetBody<'static> {
        let summary = self.backup_summary_lines();
        let selected = self.backup_selected_lines();
        let total = self.backup_pane.backups.len();
        // Summary, gap, heading, the list, gap, heading, the card.
        let fixed = text_height(&summary, width)
            .saturating_add(4)
            .saturating_add(text_height(&selected, width));
        let room = usize::from(max_height.saturating_sub(fixed));
        let rows = if total <= room {
            total
        } else {
            // One row goes to the "↓ N more" line under the list.
            room.saturating_sub(1).clamp(1.min(total), total)
        };

        let mut body = SheetBody::new();
        body.text(summary).gap(1).heading(" Recent backups ");
        if total == 0 {
            body.text(vec![Line::from(Span::styled(
                "No backups of your game settings yet.",
                Style::default().fg(self.ui_theme.muted),
            ))]);
        } else {
            let mut lines = self.backup_list_lines(rows);
            lines.extend(self.backup_overflow_line(rows));
            body.text(lines);
            body.focus_last();
        }
        body.gap(1).heading(" Selected ").text(selected);
        body
    }

    fn backup_summary_lines(&self) -> Vec<Line<'static>> {
        vec![
            Line::from(vec![
                Span::styled("Use: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Create backups before risky changes",
                    Style::default().fg(self.ui_theme.panel_title),
                ),
                Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Restore one to put your game settings back",
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
            self.backup_status_line(),
            self.overlay_kv_line("AddOns", self.rendered_target_path()),
            self.overlay_kv_line("Store", self.backup_dir.display().to_string()),
            self.overlay_kv_line(
                "Policy",
                format!(
                    "Back up before updates: {} · keeping the newest {}",
                    if self.config_pane.draft.backup_wtf {
                        "On"
                    } else {
                        "Off"
                    },
                    self.config_pane.draft.backup_retention,
                ),
            ),
        ]
    }

    fn backup_status_line(&self) -> Line<'static> {
        let status = self
            .active_detail_status_message(DetailMode::Backup)
            .unwrap_or_else(|| {
                if self.backup_pane.in_progress {
                    "Working on a backup…".to_string()
                } else if self.backup_pane.backups.is_empty() {
                    "No backups yet".to_string()
                } else {
                    format!(
                        "{} backup{} ready",
                        self.backup_pane.backups.len(),
                        plural_suffix(self.backup_pane.backups.len())
                    )
                }
            });

        let color = self.ui_theme.highlight;

        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                status,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
    }

    fn backup_list_lines(&self, visible_rows: usize) -> Vec<Line<'static>> {
        let (start, end) = self.backup_visible_range(visible_rows);
        let selected = self
            .backup_pane
            .selected_backup
            .min(self.backup_pane.backups.len().saturating_sub(1));

        self.backup_pane.backups[start..end]
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let absolute = start + index;
                let is_selected = absolute == selected;
                let marker_style = Style::default()
                    .fg(if is_selected {
                        self.ui_theme.highlight
                    } else {
                        self.ui_theme.muted
                    })
                    .add_modifier(Modifier::BOLD);
                let label_style = Style::default()
                    .fg(if is_selected {
                        self.ui_theme.highlight
                    } else {
                        self.ui_theme.panel_title
                    })
                    .add_modifier(if is_selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    });
                let size_style = Style::default().fg(self.ui_theme.muted);
                Line::from(vec![
                    Span::styled(if is_selected { "› " } else { "  " }, marker_style),
                    Span::styled(entry.label.clone(), label_style),
                    Span::styled(
                        format!("  {}", format_backup_size(entry.size_bytes)),
                        size_style,
                    ),
                ])
            })
            .collect::<Vec<_>>()
    }

    fn backup_selected_lines(&self) -> Vec<Line<'static>> {
        let Some(selected) = self.backup_pane.selected_backup() else {
            return vec![Line::from(Span::styled(
                "Press n to back up your game settings now. You can put them back from here later.",
                Style::default().fg(self.ui_theme.muted),
            ))];
        };

        vec![
            Line::from(Span::styled(
                selected.label.clone(),
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("File: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    selected.file_name.clone(),
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
            Line::from(vec![
                Span::styled("Size: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    format_backup_size(selected.size_bytes),
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
        ]
    }

    fn backup_overflow_line(&self, visible_rows: usize) -> Option<Line<'static>> {
        let (start, end) = self.backup_visible_range(visible_rows);
        let total = self.backup_pane.backups.len();
        let above = start;
        let below = total.saturating_sub(end);

        let hint = match (above, below) {
            (0, 0) => None,
            (a, 0) => Some(format!("↑ {a} more backup{}", plural_suffix(a))),
            (0, b) => Some(format!("↓ {b} more backup{}", plural_suffix(b))),
            (a, b) => Some(format!("↑ {a} more  ·  ↓ {b} more")),
        };

        hint.map(|hint| Line::from(Span::styled(hint, Style::default().fg(self.ui_theme.muted))))
    }

    fn backup_visible_range(&self, max_rows: usize) -> (usize, usize) {
        let total = self.backup_pane.backups.len();
        if total == 0 || max_rows == 0 {
            return (0, 0);
        }

        let visible = total.min(max_rows);
        let selected = self
            .backup_pane
            .selected_backup
            .min(total.saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let end = (start + visible).min(total);
        (start, end)
    }
}

fn format_backup_size(size_bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if size_bytes >= GIB {
        format!("{:.1} GB", size_bytes as f64 / GIB as f64)
    } else if size_bytes >= MIB {
        format!("{:.1} MB", size_bytes as f64 / MIB as f64)
    } else if size_bytes >= KIB {
        format!("{:.1} KB", size_bytes as f64 / KIB as f64)
    } else {
        format!("{size_bytes} B")
    }
}
