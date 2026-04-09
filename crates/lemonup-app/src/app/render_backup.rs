use super::*;

impl App {
    pub(super) fn render_backup_overlay(&self, frame: &mut Frame<'_>, area: Rect) {
        let content_area = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        let summary_lines = self.backup_summary_lines();
        let selected_lines = self.backup_selected_lines();
        let summary_height = summary_lines.len() as u16;
        let selected_height = selected_lines.len() as u16 + 1;

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(summary_height),
                Constraint::Length(1),
                Constraint::Min(6),
                Constraint::Length(1),
                Constraint::Length(selected_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);

        frame.render_widget(
            Paragraph::new(summary_lines).wrap(Wrap { trim: false }),
            sections[0],
        );
        frame.render_widget(Paragraph::new(""), sections[1]);
        let visible_rows = sections[2].height.saturating_sub(1) as usize;
        self.render_backup_list(frame, sections[2]);
        frame.render_widget(Paragraph::new(""), sections[3]);
        self.render_backup_selected(frame, sections[4], selected_lines);
        self.render_backup_overflow_hint(frame, sections[5], visible_rows);
        frame.render_widget(
            Paragraph::new(self.backup_action_line()).alignment(Alignment::Center),
            sections[6],
        );
    }

    fn backup_summary_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Use: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Create backups before risky changes",
                    Style::default().fg(self.ui_theme.panel_title),
                ),
                Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Restore a selected archive into WTF",
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
            self.backup_status_line(),
            self.overlay_kv_line("Target", self.rendered_target_path()),
            self.overlay_kv_line("Store", self.backup_dir.display().to_string()),
            self.overlay_kv_line(
                "Policy",
                format!(
                    "{} · keep {} archive{}",
                    if self.config_pane.draft.backup_wtf {
                        "WTF backup enabled"
                    } else {
                        "WTF backup disabled"
                    },
                    self.config_pane.draft.backup_retention,
                    plural_suffix(self.config_pane.draft.backup_retention as usize)
                ),
            ),
        ];

        if self.backup_pane.pending_restore.is_some() {
            lines.push(Line::from(Span::styled(
                "Restore will replace the current WTF folder after confirmation.",
                Style::default().fg(self.ui_theme.warning),
            )));
        } else if self.backup_pane.pending_delete.is_some() {
            lines.push(Line::from(Span::styled(
                "Delete removes the selected backup archive after confirmation.",
                Style::default().fg(self.ui_theme.error),
            )));
        }

        lines
    }

    fn backup_status_line(&self) -> Line<'static> {
        let status = self
            .active_detail_status_message(DetailMode::Backup)
            .unwrap_or_else(|| {
                if self.backup_pane.in_progress {
                    "Backup work running".to_string()
                } else if self.backup_pane.pending_restore.is_some() {
                    "Awaiting restore confirmation".to_string()
                } else if self.backup_pane.backups.is_empty() {
                    "No backups created yet".to_string()
                } else {
                    format!(
                        "{} backup{} ready",
                        self.backup_pane.backups.len(),
                        plural_suffix(self.backup_pane.backups.len())
                    )
                }
            });

        let color = if self.backup_pane.pending_delete.is_some() {
            self.ui_theme.error
        } else if self.backup_pane.pending_restore.is_some() {
            self.ui_theme.warning
        } else {
            self.ui_theme.highlight
        };

        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                status,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
    }

    fn render_backup_list(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                " Recent backups ",
                Style::default().fg(self.ui_theme.muted),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.backup_pane.backups.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No backups created yet.",
                    Style::default().fg(self.ui_theme.muted),
                ))),
                inner,
            );
            return;
        }

        let (start, end) = self.backup_visible_range(inner.height as usize);
        let selected = self
            .backup_pane
            .selected_backup
            .min(self.backup_pane.backups.len().saturating_sub(1));

        let rows = self.backup_pane.backups[start..end]
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
            .collect::<Vec<_>>();

        frame.render_widget(Paragraph::new(rows).wrap(Wrap { trim: false }), inner);
    }

    fn render_backup_selected(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        selected_lines: Vec<Line<'static>>,
    ) {
        let title = if self.backup_pane.pending_delete.is_some() {
            " Delete "
        } else if self.backup_pane.pending_restore.is_some() {
            " Restore "
        } else {
            " Selected "
        };
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                title,
                Style::default().fg(self.ui_theme.muted),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(selected_lines).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn backup_selected_lines(&self) -> Vec<Line<'static>> {
        if let Some(pending) = &self.backup_pane.pending_delete {
            return vec![
                Line::from(Span::styled(
                    pending.label.clone(),
                    Style::default()
                        .fg(self.ui_theme.error)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "{}  {}",
                        pending.file_name,
                        format_backup_size(pending.size_bytes)
                    ),
                    Style::default().fg(self.ui_theme.panel_title),
                )),
                Line::from(Span::styled(
                    "Press y to delete or n to cancel.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ];
        }

        if let Some(pending) = &self.backup_pane.pending_restore {
            return vec![
                Line::from(Span::styled(
                    pending.label.clone(),
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "{}  {}",
                        pending.file_name,
                        format_backup_size(pending.size_bytes)
                    ),
                    Style::default().fg(self.ui_theme.panel_title),
                )),
                Line::from(Span::styled(
                    "Press y to restore or n to cancel.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ];
        }

        let Some(selected) = self.backup_pane.selected_backup() else {
            return vec![Line::from(Span::styled(
                "Create a backup with n, then restore it here later.",
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
                Span::styled("Archive: ", Style::default().fg(self.ui_theme.muted)),
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

    fn render_backup_overflow_hint(&self, frame: &mut Frame<'_>, area: Rect, visible_rows: usize) {
        if self.backup_pane.backups.is_empty() {
            frame.render_widget(Paragraph::new(""), area);
            return;
        }

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

        if let Some(hint) = hint {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default().fg(self.ui_theme.muted),
                ))),
                area,
            );
        } else {
            frame.render_widget(Paragraph::new(""), area);
        }
    }

    fn backup_action_line(&self) -> Line<'static> {
        let chips = if self.backup_pane.pending_restore.is_some() {
            vec![
                SearchActionChip {
                    key: "y",
                    label: "restore",
                    enabled: true,
                },
                SearchActionChip {
                    key: "n",
                    label: "cancel",
                    enabled: true,
                },
            ]
        } else if self.backup_pane.pending_delete.is_some() {
            vec![
                SearchActionChip {
                    key: "y",
                    label: "delete",
                    enabled: true,
                },
                SearchActionChip {
                    key: "n",
                    label: "cancel",
                    enabled: true,
                },
            ]
        } else {
            vec![
                SearchActionChip {
                    key: "↑/↓",
                    label: "backups",
                    enabled: !self.backup_pane.backups.is_empty(),
                },
                SearchActionChip {
                    key: "enter/r",
                    label: "restore",
                    enabled: self.backup_pane.selected_backup().is_some(),
                },
                SearchActionChip {
                    key: "n",
                    label: "backup",
                    enabled: !self.backup_pane.in_progress,
                },
                SearchActionChip {
                    key: "x",
                    label: "delete",
                    enabled: self.backup_pane.selected_backup().is_some()
                        && !self.backup_pane.in_progress,
                },
            ]
        };

        let mut spans = Vec::new();
        for (index, chip) in chips.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(
                format!(" {} ", chip.key),
                Style::default()
                    .fg(if chip.enabled {
                        self.ui_theme.panel_title
                    } else {
                        self.ui_theme.muted
                    })
                    .bg(if chip.enabled {
                        self.ui_theme.key_bg
                    } else {
                        self.ui_theme.key_bg_disabled
                    })
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                chip.label,
                Style::default().fg(if chip.enabled {
                    self.ui_theme.highlight
                } else {
                    self.ui_theme.muted
                }),
            ));
        }

        Line::from(spans)
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
        format!("{:.1} GiB", size_bytes as f64 / GIB as f64)
    } else if size_bytes >= MIB {
        format!("{:.1} MiB", size_bytes as f64 / MIB as f64)
    } else if size_bytes >= KIB {
        format!("{:.1} KiB", size_bytes as f64 / KIB as f64)
    } else {
        format!("{size_bytes} B")
    }
}
