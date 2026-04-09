use super::*;

impl App {
    pub(super) fn current_help_context(&self) -> HelpContext {
        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            HelpContext::Inspect
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
        }
    }

    pub(super) fn render_help_overlay(&self, frame: &mut Frame<'_>, area: Rect) {
        let content_area = area.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });
        let summary = self.help_summary_lines();
        let groups = self.help_groups();
        let group_heights: Vec<u16> = groups
            .iter()
            .map(|(_, lines)| lines.len() as u16 + 3)
            .collect();
        let groups_height = group_heights.iter().copied().sum::<u16>();
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(summary.len() as u16 + 1),
                Constraint::Min(groups_height),
                Constraint::Length(1),
            ])
            .split(content_area);

        frame.render_widget(
            Paragraph::new(summary).wrap(Wrap { trim: false }),
            sections[0],
        );

        let group_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                group_heights
                    .iter()
                    .copied()
                    .map(Constraint::Length)
                    .collect::<Vec<_>>(),
            )
            .split(sections[1]);
        for (index, (title, lines)) in groups.iter().enumerate() {
            self.render_help_group(frame, group_rows[index], title, lines);
        }

        frame.render_widget(
            Paragraph::new(self.help_action_line()).alignment(Alignment::Center),
            sections[2],
        );
    }

    fn help_summary_lines(&self) -> Vec<Line<'static>> {
        let (title, subtitle) = match self.shell_ui.help_context {
            HelpContext::Overview => (
                "Overview shortcuts",
                "Navigate, inspect, select, and run bulk actions.",
            ),
            HelpContext::Inspect => (
                "Inspect shortcuts",
                "Review addon details and take direct actions.",
            ),
            HelpContext::InstallSearch => (
                "Install shortcuts",
                "Search Wago, edit the query, and install results.",
            ),
            HelpContext::Config => (
                "Config shortcuts",
                "Move through settings and apply changes instantly.",
            ),
            HelpContext::Backup => (
                "Backup shortcuts",
                "Create backups and restore a selected archive.",
            ),
        };

        vec![
            Line::from(vec![
                Span::styled("Context: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    title,
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                subtitle,
                Style::default().fg(self.ui_theme.muted),
            )),
            Line::default(),
        ]
    }

    fn help_groups(&self) -> Vec<(&'static str, Vec<Line<'static>>)> {
        match self.shell_ui.help_context {
            HelpContext::Overview => vec![
                (
                    " Movement ",
                    vec![
                        self.help_entry("↑/↓", "Move through addons"),
                        self.help_entry("enter", "Open inspect"),
                    ],
                ),
                (
                    " Actions ",
                    vec![
                        self.help_entry("space / a", "Select or deselect addons"),
                        self.help_entry("c / u / x", "Check, update, or delete"),
                    ],
                ),
                (
                    " More ",
                    vec![
                        self.help_entry("i", "Open install and search"),
                        self.help_entry("1-4", "Sort the table"),
                    ],
                ),
            ],
            HelpContext::Inspect => vec![
                (
                    " Actions ",
                    vec![
                        self.help_entry("space", "Select or deselect this addon"),
                        self.help_entry("c / u / x", "Check, update, or delete"),
                    ],
                ),
                (
                    " Sections ",
                    vec![
                        self.help_entry(
                            "r / d / t",
                            "Toggle included addons, deps, and addon info",
                        ),
                        self.help_entry("↑/↓", "Scroll details when content overflows"),
                    ],
                ),
            ],
            HelpContext::InstallSearch => vec![
                (
                    " Query ",
                    vec![
                        self.help_entry("enter", "Search by name or install a Wago URL"),
                        self.help_entry("/", "Edit the query again"),
                    ],
                ),
                (
                    " Results ",
                    vec![
                        self.help_entry("↑/↓", "Move through search results"),
                        self.help_entry("enter", "Install the selected result"),
                    ],
                ),
                (
                    " Setup ",
                    vec![self.help_entry(",", "Open Config for your Wago API key")],
                ),
            ],
            HelpContext::Config => vec![
                (
                    " Fields ",
                    vec![
                        self.help_entry("↑/↓", "Move between settings"),
                        self.help_entry("enter", "Edit text or toggle the current field"),
                    ],
                ),
                (
                    " Adjust ",
                    vec![
                        self.help_entry("←/→", "Change booleans and numeric values"),
                        self.help_entry("r", "Run onboarding again"),
                    ],
                ),
                (
                    " Transfer ",
                    vec![
                        self.help_entry("i", "Import addon list"),
                        self.help_entry("x", "Export addon list"),
                    ],
                ),
            ],
            HelpContext::Backup => vec![
                (
                    " Backups ",
                    vec![
                        self.help_entry("↑/↓", "Choose a backup"),
                        self.help_entry("n", "Create a new backup now"),
                    ],
                ),
                (
                    " Restore ",
                    vec![self.help_entry("enter / r", "Restore the selected backup")],
                ),
            ],
        }
    }

    fn render_help_group(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &'static str,
        lines: &[Line<'static>],
    ) {
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
            Paragraph::new({
                let mut group_lines = lines.to_vec();
                group_lines.push(Line::default());
                group_lines
            })
            .wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn help_entry(&self, key: &'static str, text: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!(" {key} "),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .bg(self.ui_theme.key_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(text, Style::default().fg(self.ui_theme.panel_title)),
        ])
    }

    fn help_action_line(&self) -> Line<'static> {
        let chips = [
            SearchActionChip {
                key: "?",
                label: "close",
                enabled: true,
            },
            SearchActionChip {
                key: "esc",
                label: "close",
                enabled: true,
            },
        ];

        let mut spans = Vec::new();
        for (index, chip) in chips.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(
                format!(" {} ", chip.key),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .bg(self.ui_theme.key_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                chip.label,
                Style::default().fg(self.ui_theme.panel_title),
            ));
        }
        Line::from(spans)
    }
}
