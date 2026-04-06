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

        if self.shell_mode == ShellMode::Dashboard {
            if self.shell_ui.overlay.active.is_some() {
                return;
            }
            self.render_dashboard_footer_grid(frame, &rows, &command_lines);
            return;
        }

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

    pub(super) fn render_dashboard_footer_grid(
        &self,
        frame: &mut Frame<'_>,
        rows: &[Rect],
        command_lines: &[Vec<FooterCommandHint>],
    ) {
        let columns = command_lines.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }

        let gap = 3u16;
        let max_hint_width = command_lines
            .iter()
            .flat_map(|line| line.iter())
            .map(|hint| self.footer_hint_line_width(std::slice::from_ref(hint)))
            .max()
            .unwrap_or(0) as u16;
        let available_width = rows.iter().map(|row| row.width as usize).max().unwrap_or(0);
        let grid_width = max_hint_width
            .saturating_mul(columns as u16)
            .saturating_add(gap.saturating_mul(columns.saturating_sub(1) as u16));

        if available_width < grid_width as usize {
            for (index, hints) in command_lines.iter().enumerate() {
                if hints.is_empty() {
                    continue;
                }
                let commands = Paragraph::new(self.footer_hint_line(hints))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false });
                frame.render_widget(commands, rows[index]);
            }
            return;
        }

        let constraints = vec![Constraint::Fill(1); columns];

        for (row_index, hints) in command_lines.iter().enumerate() {
            if hints.is_empty() {
                continue;
            }
            let row = rows[row_index];
            let cells = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints.clone())
                .split(row);
            for (column_index, hint) in hints.iter().enumerate() {
                let cell = cells[column_index];
                let command = Paragraph::new(self.footer_hint_line(std::slice::from_ref(hint)))
                    .alignment(Alignment::Left);
                frame.render_widget(command, cell);
            }
        }
    }

    pub(super) fn render_overlay_host(&self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(kind) = self.shell_ui.overlay.active {
            frame.render_widget(
                Block::default().style(Style::default().bg(self.ui_theme.scrim_bg)),
                area,
            );
            let title = match kind {
                OverlayKind::Inspect => self
                    .inspect_resolved_target()
                    .map(|target| target.item.name.clone())
                    .unwrap_or_else(|| "Inspect".to_string()),
                OverlayKind::Install => "Install".to_string(),
                OverlayKind::Search => "Install".to_string(),
                OverlayKind::Update => "Update".to_string(),
                OverlayKind::Config => "Config".to_string(),
                OverlayKind::Backup => "Backup".to_string(),
                OverlayKind::Confirm => "Confirm".to_string(),
            }
            .to_string();
            let overlay_border_style = match kind {
                OverlayKind::Inspect => Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
                OverlayKind::Confirm if self.dashboard.pending_delete_folders().is_some() => {
                    Style::default()
                        .fg(self.ui_theme.error)
                        .add_modifier(Modifier::BOLD)
                }
                OverlayKind::Confirm if self.pending_wago_install_confirmation.is_some() => {
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD)
                }
                _ => Style::default().fg(self.ui_theme.border),
            };
            let overlay = match kind {
                OverlayKind::Inspect => {
                    let overlay = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Percentage(18),
                            Constraint::Percentage(58),
                            Constraint::Percentage(24),
                        ])
                        .split(area)[1];
                    Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(18),
                            Constraint::Percentage(64),
                            Constraint::Percentage(18),
                        ])
                        .split(overlay)[1]
                }
                OverlayKind::Install | OverlayKind::Search => {
                    let overlay = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Percentage(8),
                            Constraint::Percentage(84),
                            Constraint::Percentage(8),
                        ])
                        .split(area)[1];
                    Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(4),
                            Constraint::Percentage(92),
                            Constraint::Percentage(4),
                        ])
                        .split(overlay)[1]
                }
                _ => {
                    let overlay = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Percentage(15),
                            Constraint::Percentage(70),
                            Constraint::Percentage(15),
                        ])
                        .split(area)[1];
                    Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(12),
                            Constraint::Percentage(76),
                            Constraint::Percentage(12),
                        ])
                        .split(overlay)[1]
                }
            };
            frame.render_widget(Clear, overlay);
            let title_style = match kind {
                OverlayKind::Confirm if self.dashboard.pending_delete_folders().is_some() => {
                    Style::default()
                        .fg(self.ui_theme.error)
                        .add_modifier(Modifier::BOLD)
                }
                OverlayKind::Confirm if self.pending_wago_install_confirmation.is_some() => {
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD)
                }
                _ => Style::default()
                    .fg(self.ui_theme.modal_title_color())
                    .add_modifier(Modifier::BOLD),
            };
            let close_title = Line::from(vec![
                Span::styled(
                    " esc ",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .bg(self.ui_theme.key_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " close ",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
            .right_aligned();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_set(symbols::border::ROUNDED)
                .title(Line::from(Span::styled(format!(" {title} "), title_style)).left_aligned())
                .title(close_title)
                .border_style(overlay_border_style)
                .style(Style::default().bg(self.ui_theme.overlay_bg));
            let inner = block.inner(overlay);
            frame.render_widget(block, overlay);
            if kind == OverlayKind::Inspect {
                self.render_inspect_overlay(frame, inner);
            } else if kind == OverlayKind::Install || kind == OverlayKind::Search {
                self.render_search_overlay(frame, inner);
            } else if kind == OverlayKind::Config {
                self.render_config_overlay(frame, inner);
            } else {
                let content = Paragraph::new(self.task_overlay_lines()).wrap(Wrap { trim: false });
                frame.render_widget(content, inner);
            }
        }
    }

    pub(super) fn header_logo_lines(&self, mode: ShellLayoutMode) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        match self.header_variant(mode) {
            HeaderVariant::FullLogo => {
                lines.push(self.logo_line(LOGO_FULL[0], LogoStyle::FruitGradient));
                lines.push(self.logo_line(LOGO_FULL[1], LogoStyle::FruitGradient));
                lines.push(Line::from(vec![
                    Span::styled(
                        "v2",
                        Style::default()
                            .fg(self.ui_theme.brand_gold)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "  single-surface shell",
                        Style::default().fg(self.ui_theme.info),
                    ),
                ]));
            }
            HeaderVariant::CompactLogo => {
                lines.push(Line::from(vec![
                    Span::styled(
                        LOGO_COMPACT,
                        Style::default()
                            .fg(self.ui_theme.brand_warm)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  v2", Style::default().fg(self.ui_theme.brand_gold)),
                    Span::styled(
                        "  single-surface shell",
                        Style::default().fg(self.ui_theme.info),
                    ),
                ]));
            }
        }
        if mode == ShellLayoutMode::Compact {
            lines.extend(self.header_meta_lines(mode));
        }
        lines
    }

    pub(super) fn header_meta_lines(&self, mode: ShellLayoutMode) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Profile: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    self.runtime.profile_name.clone(),
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled("Surface: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    match self.shell_mode {
                        ShellMode::Onboarding => "setup".to_string(),
                        ShellMode::Dashboard => self.dashboard_surface_label().to_string(),
                    },
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
            Line::from(vec![
                Span::styled("Target: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    self.rendered_target_path(),
                    Style::default().fg(self.ui_theme.highlight),
                ),
            ]),
            Line::from(vec![
                Span::styled("Scan: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    self.scan_status_with_motion(),
                    Style::default().fg(self.scan_status_color()),
                ),
            ]),
        ];
        if self.shell_mode != ShellMode::Dashboard
            && let Some(job_line) = self.dashboard_job_header_line()
        {
            lines.push(job_line);
        }
        if mode == ShellLayoutMode::Compact {
            return lines;
        }
        if let Some(warning) = self.profile_warning() {
            lines.push(Line::from(Span::styled(
                warning,
                Style::default()
                    .fg(self.ui_theme.warning)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        lines
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

    pub(super) fn striped_legacy_logo_line(&self, text: &str) -> Line<'static> {
        let palette = [
            self.ui_theme.brand_hot,
            self.ui_theme.brand_hot,
            self.ui_theme.brand_warm,
            self.ui_theme.brand_warm,
            self.ui_theme.brand_gold,
            self.ui_theme.brand_gold,
        ];
        let mut spans = Vec::with_capacity(text.len());
        let mut visible_index = 0usize;
        for ch in text.chars() {
            if ch.is_whitespace() {
                spans.push(Span::raw(ch.to_string()));
            } else {
                let color = palette[visible_index % palette.len()];
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
                visible_index = visible_index.wrapping_add(1);
            }
        }
        Line::from(spans)
    }

    pub(super) fn scan_status_color(&self) -> Color {
        if matches!(self.scan_state, ScanState::Pending | ScanState::Running(_)) {
            self.ui_theme.warning
        } else if self.scan_status_label().starts_with("synced") {
            self.ui_theme.success
        } else {
            self.ui_theme.info
        }
    }

    pub(super) fn dashboard_surface_label(&self) -> &'static str {
        match self.shell_ui.overlay.active {
            Some(OverlayKind::Inspect) => "Inspect",
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
            Style::default().fg(self.ui_theme.info),
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

    pub(super) fn dashboard_event_message(&self) -> Option<(String, DashboardEventKind)> {
        self.shell_ui
            .dashboard_toast
            .as_ref()
            .map(|toast| (toast.text.clone(), toast.kind))
    }

    pub(super) fn dashboard_toast_from_status(&self, status: &str) -> Option<DashboardToast> {
        if self.shell_mode != ShellMode::Dashboard || self.shell_ui.overlay.active.is_some() {
            return None;
        }

        let summary = self.sanitized_status_text(status)?;
        let kind = if summary.contains("failed")
            || summary.contains("error")
            || summary.contains("delete")
        {
            DashboardEventKind::Error
        } else if summary.starts_with("Updated")
            || summary.starts_with("Saved")
            || summary.starts_with("Restored")
            || summary.starts_with("Installed")
            || summary.starts_with("Backup complete")
        {
            DashboardEventKind::Success
        } else {
            DashboardEventKind::Info
        };
        let duration = match kind {
            DashboardEventKind::Info => StdDuration::from_millis(2500),
            DashboardEventKind::Success => StdDuration::from_millis(3000),
            DashboardEventKind::Error => StdDuration::from_millis(5000),
        };

        Some(DashboardToast {
            text: truncate_text(&summary, 160),
            kind,
            expires_at: Instant::now() + duration,
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

    pub(super) fn sanitized_status_text(&self, status: &str) -> Option<String> {
        let mut parts: Vec<String> = status
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
        self.humanized_footer_status(&summary)
    }

    pub(super) fn humanized_footer_status(&self, summary: &str) -> Option<String> {
        if summary.is_empty()
            || summary.starts_with("terminal resized")
            || summary.starts_with("selected ")
            || summary.contains("selection moved")
            || summary.contains("overlay closed")
            || summary.contains("inspect overlay open")
            || summary.contains("tree state updated")
            || summary.contains("dashboard ready")
            || summary.contains("location finder")
            || summary.contains("theme updated")
            || summary.contains("setting updated")
            || summary.contains("settings selection moved")
            || summary.contains("config field selection moved")
            || summary.contains("result selection moved")
        {
            None
        } else {
            Some(summary.to_string())
        }
    }

    pub(super) fn footer_command_lines(&self) -> Vec<Vec<FooterCommandHint>> {
        let hints = self.footer_command_hints();
        if hints.is_empty() {
            return vec![Vec::new(), Vec::new()];
        }

        let mut primary = Vec::new();
        let mut secondary = Vec::new();
        for hint in hints {
            match hint.tier {
                FooterHintTier::Primary => primary.push(hint),
                FooterHintTier::Secondary => secondary.push(hint),
            }
        }

        if secondary.is_empty() {
            return vec![primary, Vec::new()];
        }

        vec![primary, secondary]
    }

    pub(super) fn footer_command_hints(&self) -> Vec<FooterCommandHint> {
        let primary = FooterHintTier::Primary;
        let secondary = FooterHintTier::Secondary;

        if self.shell_mode == ShellMode::Onboarding {
            if self.onboarding.is_editing {
                return vec![
                    FooterCommandHint {
                        id: FooterHintId::Save,
                        key: "enter",
                        label: "apply",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Cancel,
                        key: "esc",
                        label: "cancel",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Back,
                        key: "backspace",
                        label: "delete",
                        tier: secondary,
                    },
                ];
            }

            return match self.onboarding.step {
                OnboardingStep::Theme => vec![
                    FooterCommandHint {
                        id: FooterHintId::ThemeToggle,
                        key: "←/→",
                        label: "theme",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Next,
                        key: "enter",
                        label: "next",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Cancel,
                        key: "esc",
                        label: "exit",
                        tier: secondary,
                    },
                ],
                OnboardingStep::Directory => match self.onboarding.phase {
                    OnboardingPhase::Found(_) => vec![
                        FooterCommandHint {
                            id: FooterHintId::Nav,
                            key: "j/k",
                            label: "choice",
                            tier: primary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::Next,
                            key: "enter",
                            label: "next",
                            tier: primary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::Back,
                            key: "esc",
                            label: "back",
                            tier: secondary,
                        },
                    ],
                    OnboardingPhase::DeepScanning(_) => vec![FooterCommandHint {
                        id: FooterHintId::Cancel,
                        key: "esc",
                        label: "cancel scan",
                        tier: primary,
                    }],
                    _ => vec![
                        FooterCommandHint {
                            id: FooterHintId::Nav,
                            key: "j/k",
                            label: "choice",
                            tier: primary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::Validate,
                            key: "enter",
                            label: "validate",
                            tier: primary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::DeepScan,
                            key: "d",
                            label: "deep scan",
                            tier: primary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::Edit,
                            key: "e",
                            label: "edit",
                            tier: secondary,
                        },
                        FooterCommandHint {
                            id: FooterHintId::Back,
                            key: "esc",
                            label: "back",
                            tier: secondary,
                        },
                    ],
                },
                OnboardingStep::Wago => vec![
                    FooterCommandHint {
                        id: FooterHintId::Edit,
                        key: "e",
                        label: "edit key",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Next,
                        key: "enter",
                        label: "next",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Back,
                        key: "esc",
                        label: "back",
                        tier: secondary,
                    },
                ],
                OnboardingStep::Settings => vec![
                    FooterCommandHint {
                        id: FooterHintId::SettingsNav,
                        key: "j/k",
                        label: "settings",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Change,
                        key: "←/→",
                        label: "change",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Next,
                        key: "enter",
                        label: "next",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Back,
                        key: "esc",
                        label: "back",
                        tier: secondary,
                    },
                ],
                OnboardingStep::Review => vec![
                    FooterCommandHint {
                        id: FooterHintId::Next,
                        key: "enter",
                        label: "finish",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Back,
                        key: "esc",
                        label: "back",
                        tier: primary,
                    },
                ],
            };
        }

        if self.dashboard.pending_delete_folders().is_some()
            || self.pending_wago_install_confirmation.is_some()
        {
            return vec![
                FooterCommandHint {
                    id: FooterHintId::Confirm,
                    key: "y",
                    label: "confirm",
                    tier: primary,
                },
                FooterCommandHint {
                    id: FooterHintId::Cancel,
                    key: "n/esc",
                    label: "cancel",
                    tier: primary,
                },
            ];
        }

        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            let toggle_label = self
                .inspect_resolved_target()
                .map(|target| {
                    if self.dashboard.is_parent_selected(&target.item.folder) {
                        "deselect"
                    } else {
                        "select"
                    }
                })
                .unwrap_or("select");
            return vec![
                FooterCommandHint {
                    id: FooterHintId::Select,
                    key: "space",
                    label: toggle_label,
                    tier: primary,
                },
                FooterCommandHint {
                    id: FooterHintId::Check,
                    key: "c",
                    label: "check",
                    tier: primary,
                },
                FooterCommandHint {
                    id: FooterHintId::Update,
                    key: "u",
                    label: "update",
                    tier: primary,
                },
                FooterCommandHint {
                    id: FooterHintId::Delete,
                    key: "x",
                    label: "delete",
                    tier: primary,
                },
                FooterCommandHint {
                    id: FooterHintId::Relations,
                    key: "r",
                    label: "relations",
                    tier: secondary,
                },
                FooterCommandHint {
                    id: FooterHintId::Dependencies,
                    key: "d",
                    label: "deps",
                    tier: secondary,
                },
                FooterCommandHint {
                    id: FooterHintId::Technical,
                    key: "t",
                    label: "tech",
                    tier: secondary,
                },
            ];
        }

        if self.shell_ui.overlay.active.is_some() {
            return match self.dashboard.detail_mode {
                DetailMode::Install => {
                    if self.install_pane.is_editing {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Save,
                                key: "enter",
                                label: "apply",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Cancel,
                                key: "esc",
                                label: "cancel",
                                tier: primary,
                            },
                        ]
                    } else {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Edit,
                                key: "e",
                                label: "edit",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Run,
                                key: "enter",
                                label: "install",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Close,
                                key: "esc",
                                label: "close",
                                tier: secondary,
                            },
                        ]
                    }
                }
                DetailMode::Search => {
                    if self.search_pane.is_editing {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Save,
                                key: "enter",
                                label: "search",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Cancel,
                                key: "esc",
                                label: "cancel",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Back,
                                key: "backspace",
                                label: "delete",
                                tier: secondary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Close,
                                key: "esc",
                                label: "stop",
                                tier: secondary,
                            },
                        ]
                    } else {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Edit,
                                key: "/ ?",
                                label: "query",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Results,
                                key: "j/k",
                                label: "results",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Install,
                                key: "enter",
                                label: "install",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Close,
                                key: "esc",
                                label: "close",
                                tier: secondary,
                            },
                        ]
                    }
                }
                DetailMode::Update => vec![
                    FooterCommandHint {
                        id: FooterHintId::Update,
                        key: "u",
                        label: "update",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Check,
                        key: "c",
                        label: "check",
                        tier: primary,
                    },
                    FooterCommandHint {
                        id: FooterHintId::Close,
                        key: "esc",
                        label: "close",
                        tier: secondary,
                    },
                ],
                DetailMode::Config => {
                    if self.config_pane.edit.is_some() {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Save,
                                key: "enter",
                                label: "apply",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Cancel,
                                key: "esc",
                                label: "cancel",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Back,
                                key: "backspace",
                                label: "delete",
                                tier: secondary,
                            },
                        ]
                    } else {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Fields,
                                key: "↑/↓",
                                label: "fields",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Toggle,
                                key: "enter",
                                label: if self.config_pane.selected_field().is_textual() {
                                    "edit"
                                } else {
                                    "toggle"
                                },
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Change,
                                key: "←/→",
                                label: "change",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Run,
                                key: "r",
                                label: "onboarding",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Install,
                                key: "i",
                                label: "import",
                                tier: secondary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Delete,
                                key: "x",
                                label: "export",
                                tier: secondary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Close,
                                key: "esc",
                                label: "close",
                                tier: secondary,
                            },
                        ]
                    }
                }
                DetailMode::Backup => {
                    if self.backup_pane.pending_restore.is_some() {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Confirm,
                                key: "y",
                                label: "restore",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Cancel,
                                key: "n",
                                label: "cancel",
                                tier: secondary,
                            },
                        ]
                    } else {
                        vec![
                            FooterCommandHint {
                                id: FooterHintId::Nav,
                                key: "↑/↓",
                                label: "backups",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Confirm,
                                key: "enter/r",
                                label: "restore",
                                tier: primary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Run,
                                key: "n",
                                label: "backup",
                                tier: secondary,
                            },
                            FooterCommandHint {
                                id: FooterHintId::Close,
                                key: "esc",
                                label: "close",
                                tier: secondary,
                            },
                        ]
                    }
                }
                DetailMode::Overview => vec![FooterCommandHint {
                    id: FooterHintId::Close,
                    key: "esc",
                    label: "close",
                    tier: primary,
                }],
            };
        }

        vec![
            FooterCommandHint {
                id: FooterHintId::Nav,
                key: "↑/↓",
                label: "move",
                tier: primary,
            },
            FooterCommandHint {
                id: FooterHintId::Inspect,
                key: "enter",
                label: "inspect",
                tier: primary,
            },
            FooterCommandHint {
                id: FooterHintId::Select,
                key: "space/a",
                label: if self.dashboard.selected_parent_count() > 0 {
                    "deselect"
                } else {
                    "select"
                },
                tier: primary,
            },
            FooterCommandHint {
                id: FooterHintId::Check,
                key: "c",
                label: "check",
                tier: primary,
            },
            FooterCommandHint {
                id: FooterHintId::Update,
                key: "u",
                label: if self.dashboard.selected_parent_count() > 0 {
                    "update sel"
                } else {
                    "update"
                },
                tier: secondary,
            },
            FooterCommandHint {
                id: FooterHintId::Install,
                key: "i",
                label: "install",
                tier: secondary,
            },
            FooterCommandHint {
                id: FooterHintId::Delete,
                key: "x",
                label: if self.dashboard.selected_parent_count() > 0 {
                    "delete sel"
                } else {
                    "delete"
                },
                tier: secondary,
            },
            FooterCommandHint {
                id: FooterHintId::Sort,
                key: "1-4",
                label: "sort",
                tier: secondary,
            },
        ]
    }

    pub(super) fn footer_hint_line(&self, hints: &[FooterCommandHint]) -> Line<'static> {
        let mut spans = Vec::new();
        for (index, hint) in hints.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            let (key_style, label_style) = self.footer_hint_styles(hint.id);
            spans.push(Span::styled(format!(" {} ", hint.key), key_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(hint.label, label_style));
        }
        Line::from(spans)
    }

    pub(super) fn footer_hint_line_width(&self, hints: &[FooterCommandHint]) -> usize {
        hints
            .iter()
            .enumerate()
            .map(|(index, hint)| {
                let base = hint.key.len() + 2 + 1 + hint.label.len();
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
