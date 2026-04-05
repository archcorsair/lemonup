use super::render_dashboard::{dashboard_item_remote_label, dashboard_item_update_status};
use super::*;
impl App {
    pub(super) fn render_inspect_overlay(&self, frame: &mut Frame<'_>, area: Rect) {
        let content_area = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        let Some(target) = self.inspect_resolved_target() else {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "No addon selected",
                        Style::default()
                            .fg(self.ui_theme.panel_title)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Select a row, then press Enter to inspect.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ])
                .wrap(Wrap { trim: false }),
                content_area,
            );
            return;
        };

        let hero_lines = self.inspect_hero_lines(target);
        let summary_lines = self.inspect_summary_lines(target);
        let detail_lines = self.inspect_detail_lines(target);
        let hero_height = if hero_lines.is_empty() {
            0
        } else {
            hero_lines.len() as u16
        };
        let summary_height = (summary_lines.len() as u16).saturating_add(1).max(4);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(hero_height),
                Constraint::Length(summary_height),
                Constraint::Length(1),
                Constraint::Min((detail_lines.len() as u16).clamp(4, 8)),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);

        if hero_height > 0 {
            let hero = Paragraph::new(hero_lines).wrap(Wrap { trim: false });
            frame.render_widget(hero, sections[0]);
        }

        let summary_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                " Summary ",
                Style::default().fg(self.ui_theme.muted),
            ));
        let summary_inner = summary_block.inner(sections[1]);
        frame.render_widget(summary_block, sections[1]);
        frame.render_widget(
            Paragraph::new(summary_lines).wrap(Wrap { trim: false }),
            summary_inner,
        );

        frame.render_widget(Paragraph::new(""), sections[2]);

        let details_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                " Details ",
                Style::default().fg(self.ui_theme.muted),
            ));
        let details_inner = details_block.inner(sections[3]);
        frame.render_widget(details_block, sections[3]);
        let max_scroll = detail_lines
            .len()
            .saturating_sub(details_inner.height as usize) as u16;
        let scroll_offset = self.inspect_overlay.scroll_offset.min(max_scroll);
        let can_scroll_up = scroll_offset > 0;
        let can_scroll_down = scroll_offset < max_scroll;
        frame.render_widget(
            Paragraph::new(detail_lines)
                .scroll((scroll_offset, 0))
                .wrap(Wrap { trim: false }),
            details_inner,
        );

        let overflow_hint = match (can_scroll_up, can_scroll_down) {
            (true, true) => Some("↑/↓ more"),
            (true, false) => Some("↑ more"),
            (false, true) => Some("↓ more"),
            (false, false) => None,
        };

        if let Some(hint) = overflow_hint {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default()
                        .fg(self.ui_theme.muted)
                        .add_modifier(Modifier::BOLD),
                ))),
                sections[4],
            );
        } else {
            frame.render_widget(Paragraph::new(""), sections[4]);
        }
        frame.render_widget(
            Paragraph::new(self.inspect_action_line(target)).alignment(Alignment::Center),
            sections[5],
        );
    }

    pub(super) fn inspect_hero_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(child_folder) = target.opened_from_child {
            lines.push(Line::from(Span::styled(
                format!("Opened from child: {child_folder}"),
                Style::default().fg(self.ui_theme.muted),
            )));
        }
        lines
    }

    pub(super) fn inspect_summary_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![
            self.inspect_source_line(target.item),
            self.inspect_status_line(target.item),
            self.inspect_version_line(target.item),
            self.overlay_kv_line("Folder", target.item.folder.clone()),
        ];

        if let Some(author) = target.item.author.as_deref().map(str::trim)
            && !author.is_empty()
            && !author.eq_ignore_ascii_case("unknown")
        {
            lines.push(self.overlay_kv_line("Author", author.to_string()));
        }

        if target.item.has_authoritative_owned_folders && target.item.source != SourceKind::Manual {
            lines.push(self.overlay_kv_line("Tracking", "Managed by LemonUp"));
        }

        lines
    }

    pub(super) fn inspect_source_line(&self, item: &DashboardItem) -> Line<'static> {
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                source_label(item.source).to_string(),
                Style::default()
                    .fg(source_badge_color(item.source))
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    }

    pub(super) fn inspect_version_line(&self, item: &DashboardItem) -> Line<'static> {
        Line::from(vec![
            Span::styled("Version: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                dashboard_item_version_label(item),
                Style::default().fg(Color::Rgb(172, 182, 220)),
            ),
        ])
    }

    pub(super) fn inspect_action_line(&self, target: InspectResolvedTarget<'_>) -> Line<'static> {
        let is_selected = self.dashboard.is_parent_selected(&target.item.folder);
        let select_label = if is_selected { "deselect" } else { "select" };
        let chips = [
            InspectActionChip {
                key: "space",
                label: select_label,
                enabled: true,
            },
            InspectActionChip {
                key: "c",
                label: "check",
                enabled: true,
            },
            InspectActionChip {
                key: "u",
                label: "update",
                enabled: is_refreshable_dashboard_item(target.item),
            },
            InspectActionChip {
                key: "x",
                label: "delete",
                enabled: true,
            },
        ];

        let mut spans = Vec::new();
        for (index, chip) in chips.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            let key_bg = if chip.key == "space" && is_selected {
                Color::Rgb(28, 34, 48)
            } else if chip.enabled {
                Color::Rgb(31, 37, 58)
            } else {
                Color::Rgb(27, 30, 45)
            };
            let key_fg = if chip.key == "space" && is_selected {
                Color::Rgb(172, 220, 255)
            } else if chip.enabled {
                self.ui_theme.panel_title
            } else {
                self.ui_theme.muted
            };
            spans.push(Span::styled(
                format!(" {} ", chip.key),
                Style::default()
                    .fg(key_fg)
                    .bg(key_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                chip.label,
                Style::default().fg(if chip.enabled {
                    self.ui_theme.panel_title
                } else {
                    self.ui_theme.muted
                }),
            ));
        }

        Line::from(spans)
    }

    pub(super) fn inspect_detail_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for section in [
            InspectSection::IncludedAddons,
            InspectSection::Dependencies,
            InspectSection::AddonInfo,
        ] {
            let is_open = self.inspect_overlay.is_open(section);
            lines.push(self.inspect_section_header(section, is_open));
            if is_open {
                lines.extend(self.inspect_section_body(section, target));
            }
        }
        lines
    }

    pub(super) fn inspect_status_line(&self, item: &DashboardItem) -> Line<'static> {
        let (status_label, status_color) = self.inspect_status_chip(item);
        let mut spans = vec![Span::styled(
            "Status: ",
            Style::default().fg(self.ui_theme.muted),
        )];
        let is_update_available = matches!(
            dashboard_item_update_status(item),
            UpdateStatus::UpdateAvailable
        ) && item.source != SourceKind::Manual
            && item.has_authoritative_owned_folders;
        let display_color = if is_update_available {
            self.inspect_pulse_color(status_color)
        } else {
            status_color
        };
        spans.push(Span::styled(
            status_label.to_string(),
            Style::default()
                .fg(display_color)
                .add_modifier(Modifier::BOLD),
        ));
        if is_update_available {
            let remote = dashboard_item_remote_label(item).unwrap_or_else(|| "update".to_string());
            spans.extend([
                Span::raw(" "),
                Span::styled("→", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(
                    remote,
                    Style::default()
                        .fg(display_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled("📦", Style::default().fg(Color::Rgb(212, 175, 55))),
            ]);
        }
        Line::from(spans)
    }

    pub(super) fn inspect_pulse_color(&self, base: Color) -> Color {
        match (base, self.shell_ui.motion.tick_count % 6) {
            (Color::Rgb(r, g, b), 0..=2) => Color::Rgb(r, g, b),
            (Color::Rgb(r, g, b), _) => Color::Rgb(
                r.saturating_add(22),
                g.saturating_add(18),
                b.saturating_add(8),
            ),
            (Color::Yellow, 0..=2) => Color::Yellow,
            (Color::Yellow, _) => Color::Rgb(255, 232, 168),
            (other, _) => other,
        }
    }

    pub(super) fn inspect_status_chip(&self, item: &DashboardItem) -> (&'static str, Color) {
        if item.source == SourceKind::Manual {
            return ("Manual", Color::Magenta);
        }
        if !item.has_authoritative_owned_folders {
            return ("Unmanaged", Color::Magenta);
        }

        match dashboard_item_update_status(item) {
            UpdateStatus::UpToDate => ("Up to date", self.ui_theme.success),
            UpdateStatus::UpdateAvailable => ("Update available", self.ui_theme.warning),
            UpdateStatus::Unknown => ("Unknown", Color::Magenta),
            UpdateStatus::Error => ("Error", self.ui_theme.error),
        }
    }

    pub(super) fn inspect_section_header(
        &self,
        section: InspectSection,
        is_open: bool,
    ) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                if is_open { "▾ " } else { "▸ " },
                Style::default().fg(self.ui_theme.muted),
            ),
            Span::styled(
                format!("{} ", section.key()),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                section.label(),
                Style::default().fg(self.ui_theme.panel_title),
            ),
        ])
    }

    pub(super) fn inspect_section_body(
        &self,
        section: InspectSection,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let indent = "  ";
        match section {
            InspectSection::IncludedAddons => {
                let mut lines = Vec::new();
                if let Some(child_folder) = target.opened_from_child {
                    lines.push(Line::from(format!(
                        "{indent}Opened from child: {child_folder}"
                    )));
                }
                if !target.item.owned_folders.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Children: {}",
                        summarize_owned_folders(&target.item.owned_folders)
                    )));
                }
                let missing = self.dashboard_parent_missing_owned_children(&target.item.folder);
                if !missing.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Missing: {}",
                        summarize_owned_folders(missing)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No relationship details")));
                }
                lines
            }
            InspectSection::Dependencies => {
                let mut lines = Vec::new();
                if !target.item.required_deps.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Required: {}",
                        summarize_owned_folders(&target.item.required_deps)
                    )));
                }
                if !target.item.optional_deps.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Optional: {}",
                        summarize_owned_folders(&target.item.optional_deps)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No dependency metadata")));
                }
                lines
            }
            InspectSection::AddonInfo => {
                let mut lines = Vec::new();
                if let Some(interface) = target.item.interface.as_deref().map(str::trim)
                    && !interface.is_empty()
                {
                    lines.push(Line::from(format!("{indent}Interface: {interface}")));
                }
                if let Some(source_url) = target.item.source_url.as_deref().map(str::trim)
                    && !source_url.is_empty()
                {
                    lines.push(Line::from(format!(
                        "{indent}Source URL: {}",
                        truncate_text(source_url, 64)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No technical details")));
                }
                lines
            }
        }
    }
}
