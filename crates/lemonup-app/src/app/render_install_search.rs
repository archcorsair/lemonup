use super::*;
impl App {
    pub(super) fn render_search_overlay(&self, frame: &mut Frame<'_>, area: Rect) {
        let content_area = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        let summary_lines = self.search_summary_lines();
        let summary_height = summary_lines.len() as u16;
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(summary_height),
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(8),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);

        frame.render_widget(
            Paragraph::new(summary_lines).wrap(Wrap { trim: false }),
            sections[0],
        );

        self.render_search_input(frame, sections[1]);

        frame.render_widget(Paragraph::new(""), sections[2]);

        self.render_search_compose_first_body(frame, sections[3]);
        frame.render_widget(Paragraph::new(""), sections[4]);
        frame.render_widget(
            Paragraph::new(self.search_action_line()).alignment(Alignment::Center),
            sections[5],
        );
    }

    pub(super) fn render_search_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let active = self.search_pane.is_editing || self.search_pane.in_progress;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(if active {
                Style::default().fg(self.ui_theme.highlight)
            } else {
                Style::default().fg(self.ui_theme.border)
            })
            .title(" Addon ");

        let mut spans = Vec::new();
        if self.search_pane.query.trim().is_empty() {
            if self.search_pane.is_editing {
                spans.push(Span::styled(
                    "▌",
                    Style::default().fg(self.ui_theme.brand_gold),
                ));
            } else {
                spans.push(Span::styled(
                    "Type an addon name or paste a Wago URL",
                    Style::default().fg(self.ui_theme.muted),
                ));
            }
        } else {
            spans.push(Span::styled(
                self.search_pane.query.clone(),
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if self.search_pane.in_progress {
            spans.push(Span::raw("  "));
            spans.extend(shimmer_text_spans(
                "searching…",
                self.ui_theme.warning,
                self.ui_theme.shimmer_glow,
                ShimmerConfig::action(),
            ));
        } else if self.search_pane.is_editing && !self.search_pane.query.trim().is_empty() {
            spans.push(Span::styled(
                " ▌",
                Style::default().fg(self.ui_theme.brand_gold),
            ));
        }

        let query = Paragraph::new(Line::from(spans))
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(query, area);
    }

    pub(super) fn search_summary_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            Span::styled("Use: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                "Search Wago by addon name",
                Style::default().fg(self.ui_theme.panel_title),
            ),
            Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                "Paste a Wago URL to install directly",
                Style::default().fg(self.ui_theme.panel_title),
            ),
        ])];

        let status_line = if self.wago_api_key.is_none() {
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Wago API key required for this screen",
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else if self.search_pane.in_progress {
            let mut spans = vec![Span::styled(
                "Status: ",
                Style::default().fg(self.ui_theme.muted),
            )];
            spans.extend(shimmer_text_spans(
                "Searching Wago…",
                self.ui_theme.warning,
                self.ui_theme.shimmer_glow,
                ShimmerConfig::action(),
            ));
            Line::from(spans)
        } else if let Some(state) = self.active_search_install_state() {
            match state {
                SearchInstallState::Checking { .. } => {
                    let mut spans = vec![Span::styled(
                        "Status: ",
                        Style::default().fg(self.ui_theme.muted),
                    )];
                    spans.extend(shimmer_text_spans(
                        "Preparing install…",
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    ));
                    Line::from(spans)
                }
                SearchInstallState::Installing { .. } => {
                    let mut spans = vec![Span::styled(
                        "Status: ",
                        Style::default().fg(self.ui_theme.muted),
                    )];
                    spans.extend(shimmer_text_spans(
                        "Installing from Wago…",
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    ));
                    Line::from(spans)
                }
                SearchInstallState::Success { parent_folder, .. } => Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        format!("Installed into {parent_folder}"),
                        Style::default()
                            .fg(self.ui_theme.success)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                SearchInstallState::ConfirmReplace { tracked_parent, .. } => Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        format!("Already installed as {tracked_parent}"),
                        Style::default()
                            .fg(self.ui_theme.panel_title)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                SearchInstallState::Error { message, .. } => Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        message.clone(),
                        Style::default()
                            .fg(self.ui_theme.error)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                SearchInstallState::Idle => self.default_search_status_line(),
            }
        } else {
            self.default_search_status_line()
        };
        lines.push(status_line);
        lines
    }

    pub(super) fn default_search_status_line(&self) -> Line<'static> {
        if self.search_pane.results.is_empty() {
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    if self.search_pane.query.trim().is_empty() {
                        "Enter an addon name or Wago URL"
                    } else if self.search_direct_install_target().is_some() {
                        "Press Enter to install directly"
                    } else {
                        "Press Enter to search Wago"
                    },
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ])
        } else {
            let count = self.search_pane.results.len();
            let query = self.search_pane.last_query.as_deref().unwrap_or("query");
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    format!("{count} result{} for “{query}”", plural_suffix(count)),
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        }
    }

    pub(super) fn selected_search_install_state(&self) -> Option<&SearchInstallState> {
        let result = self.search_pane.selected_result()?;
        self.search_install_state_for_result(result)
    }

    pub(super) fn active_search_install_state(&self) -> Option<&SearchInstallState> {
        self.selected_search_install_state().or({
            if matches!(self.search_pane.install_state, SearchInstallState::Idle) {
                None
            } else {
                Some(&self.search_pane.install_state)
            }
        })
    }

    pub(super) fn search_direct_install_target(&self) -> Option<String> {
        let trimmed = self.search_pane.query.trim();
        let parsed = parse_wago_target(trimmed).ok()?;
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return Some(parsed);
        }
        None
    }

    pub(super) fn search_action_line(&self) -> Line<'static> {
        let submit_label = if self.search_direct_install_target().is_some() {
            "install"
        } else {
            "search"
        };
        let chips: Vec<SearchActionChip> = if self.pending_wago_install_confirmation.is_some() {
            vec![
                SearchActionChip {
                    key: "y",
                    label: "reinstall",
                    enabled: true,
                },
                SearchActionChip {
                    key: "n",
                    label: "cancel",
                    enabled: true,
                },
            ]
        } else if self.search_pane.is_editing || self.search_pane.in_progress {
            let mut chips = vec![
                SearchActionChip {
                    key: "enter",
                    label: submit_label,
                    enabled: !self.search_pane.in_progress,
                },
                SearchActionChip {
                    key: "esc",
                    label: "stop editing",
                    enabled: self.search_pane.is_editing,
                },
            ];
            if self.wago_api_key.is_none() && !self.search_pane.in_progress {
                chips.push(SearchActionChip {
                    key: ",",
                    label: "config",
                    enabled: true,
                });
            }
            chips
        } else if self.search_pane.results.is_empty() {
            vec![
                SearchActionChip {
                    key: "enter",
                    label: submit_label,
                    enabled: self.wago_api_key.is_some(),
                },
                SearchActionChip {
                    key: "/",
                    label: "edit input",
                    enabled: true,
                },
                SearchActionChip {
                    key: ",",
                    label: "config",
                    enabled: self.wago_api_key.is_none(),
                },
            ]
        } else {
            vec![
                SearchActionChip {
                    key: "↑/↓",
                    label: "move",
                    enabled: true,
                },
                SearchActionChip {
                    key: "enter",
                    label: "install",
                    enabled: true,
                },
                SearchActionChip {
                    key: "/",
                    label: "edit query",
                    enabled: true,
                },
            ]
        };

        let mut spans = Vec::new();
        for (index, chip) in chips.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            let key_bg = if chip.enabled {
                self.ui_theme.key_bg
            } else {
                self.ui_theme.key_bg_disabled
            };
            let key_fg = if chip.enabled {
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
                if chip.enabled {
                    Style::default().fg(self.ui_theme.highlight)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                },
            ));
        }

        Line::from(spans)
    }

    pub(super) fn render_search_compose_first_body(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.search_pane.results.is_empty() {
            self.render_search_empty_state(frame, area);
            return;
        }

        let [results, preview] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(5)])
            .areas(area);

        let results_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                " Results ",
                Style::default().fg(self.ui_theme.muted),
            ));
        let results_inner = results_block.inner(results);
        frame.render_widget(results_block, results);
        self.render_search_results_list(frame, results_inner);

        let preview_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.ui_theme.border))
            .title(Span::styled(
                " Selected ",
                Style::default().fg(self.ui_theme.muted),
            ));
        let preview_inner = preview_block.inner(preview);
        frame.render_widget(preview_block, preview);
        self.render_search_selected_preview(frame, preview_inner);
    }

    pub(super) fn render_search_empty_state(&self, frame: &mut Frame<'_>, area: Rect) {
        let message = if self.wago_api_key.is_none() {
            vec![
                Line::from(Span::styled(
                    "Wago install/search is unavailable",
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "This screen installs from Wago only.",
                    Style::default().fg(self.ui_theme.muted),
                )),
                Line::from(Span::styled(
                    "Press , to open Config and add your Wago API key.",
                    Style::default().fg(self.ui_theme.panel_title),
                )),
            ]
        } else if let Some(state) = self.active_search_install_state() {
            match state {
                SearchInstallState::Checking { addon_name, .. } => vec![
                    Line::from(shimmer_text_spans(
                        &format!("Preparing {addon_name} for install…"),
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "LemonUp is checking tracked state and preparing the package.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Installing { addon_name, .. } => vec![
                    Line::from(shimmer_text_spans(
                        &format!("Installing {addon_name} from Wago…"),
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "The addon will appear in Manage when install and sync complete.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Success {
                    addon_name,
                    parent_folder,
                    ..
                } => vec![
                    Line::from(vec![
                        Span::styled(
                            "✓ ",
                            Style::default()
                                .fg(self.ui_theme.success)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("Installed {addon_name}"),
                            Style::default()
                                .fg(self.ui_theme.success)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        format!("Installed into {parent_folder}."),
                        Style::default().fg(self.ui_theme.panel_title),
                    )),
                ],
                SearchInstallState::ConfirmReplace {
                    addon_name,
                    tracked_parent,
                    ..
                } => vec![
                    Line::from(Span::styled(
                        format!("{addon_name} is already installed."),
                        Style::default()
                            .fg(self.ui_theme.panel_title)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        format!(
                            "Press y to reinstall over tracked addon {tracked_parent}, or n to cancel."
                        ),
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Error { message, .. } => vec![
                    Line::from(Span::styled(
                        "Install failed",
                        Style::default()
                            .fg(self.ui_theme.error)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        message.clone(),
                        Style::default().fg(self.ui_theme.error),
                    )),
                ],
                SearchInstallState::Idle => unreachable!(),
            }
        } else if self.search_pane.in_progress {
            vec![
                Line::from(Span::styled(
                    "Searching Wago…",
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Results will appear here when the search completes.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]
        } else if let Some(last_query) = &self.search_pane.last_query {
            vec![
                Line::from(Span::styled(
                    format!("No results for “{last_query}”"),
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Try a broader name or a known addon title.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]
        } else {
            vec![
                Line::from(Span::styled(
                    "Start with an addon name or Wago URL",
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter to search Wago or install directly.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]
        };

        let paragraph = Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    pub(super) fn render_search_results_list(&self, frame: &mut Frame<'_>, area: Rect) {
        let result_count = self.search_pane.results.len();
        let mut show_above = false;
        let mut show_below = false;
        let mut window_start = 0;
        let mut window_end = 0;

        for _ in 0..2 {
            let reserved_rows = 1 + u16::from(show_above) + u16::from(show_below);
            let visible_rows = area.height.saturating_sub(reserved_rows).max(1) as usize;
            let (start, end) = visible_search_result_window(
                result_count,
                self.search_pane.selected_result,
                visible_rows,
            );
            window_start = start;
            window_end = end;
            show_above = window_start > 0;
            show_below = window_end < result_count;
        }

        let mut constraints = Vec::new();
        if show_above {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(2));
        if show_below {
            constraints.push(Constraint::Length(1));
        }
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        let mut area_index = 0;
        if show_above {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("↑ ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        format!(
                            "{} more result{} above",
                            window_start,
                            plural_suffix(window_start)
                        ),
                        Style::default().fg(self.ui_theme.muted),
                    ),
                ])),
                areas[area_index],
            );
            area_index += 1;
        }
        let table_area = areas[area_index];
        area_index += 1;

        let header = Row::new([
            Cell::from(Span::styled(
                "Name",
                Style::default().fg(self.ui_theme.muted),
            )),
            Cell::from(Span::styled(
                "Author",
                Style::default().fg(self.ui_theme.muted),
            )),
            Cell::from(Span::styled(
                "Version",
                Style::default().fg(self.ui_theme.muted),
            )),
            Cell::from(Span::styled("DL", Style::default().fg(self.ui_theme.muted))),
        ]);
        let mut rows = Vec::new();
        let mut selected_row = None;

        for (index, result) in self.search_pane.results[window_start..window_end]
            .iter()
            .enumerate()
        {
            let absolute_index = window_start + index;
            let selected = self.search_pane.selected_result == Some(absolute_index);
            let tracked_folder = self.tracked_wago_result_folder(result);
            let install_state = self.search_install_state_for_result(result);
            let author = result
                .owner
                .as_deref()
                .or_else(|| result.authors.first().map(String::as_str))
                .unwrap_or("unknown");
            let version = result.version.as_deref().unwrap_or("unknown");
            let row_style = if selected {
                selected_row = Some(index);
                Style::default().bg(self.ui_theme.row_highlight_bg)
            } else {
                Style::default()
            };

            rows.push(
                Row::new([
                    Cell::from(self.search_result_name_line(
                        result,
                        selected,
                        tracked_folder.as_deref(),
                        install_state,
                    )),
                    Cell::from(Span::styled(
                        truncate_text(author, 18),
                        Style::default().fg(self.ui_theme.muted),
                    )),
                    Cell::from(Span::styled(
                        truncate_text(version, 18),
                        if install_state.is_some() {
                            Style::default().fg(self.ui_theme.warning)
                        } else {
                            Style::default().fg(self.ui_theme.panel_title)
                        },
                    )),
                    Cell::from(Span::styled(
                        result
                            .download_count
                            .map(format_download_count)
                            .unwrap_or_else(|| "—".to_string()),
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ])
                .style(row_style),
            );
        }

        let table = Table::new(
            rows,
            [
                Constraint::Min(48),
                Constraint::Length(18),
                Constraint::Length(18),
                Constraint::Length(8),
            ],
        )
        .header(header)
        .column_spacing(2)
        .row_highlight_style(
            Style::default()
                .bg(self.ui_theme.row_highlight_bg)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = TableState::default().with_selected(selected_row);
        frame.render_stateful_widget(table, table_area, &mut state);

        if show_below {
            let remaining_below = result_count.saturating_sub(window_end);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("↓ ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        format!(
                            "{} more result{} below",
                            remaining_below,
                            plural_suffix(remaining_below)
                        ),
                        Style::default().fg(self.ui_theme.muted),
                    ),
                ])),
                areas[area_index],
            );
        }
    }

    pub(super) fn render_search_selected_preview(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = if let Some(result) = self.search_pane.selected_result() {
            let author = result
                .owner
                .as_deref()
                .or_else(|| result.authors.first().map(String::as_str))
                .unwrap_or("unknown");
            let tracked_folder = self.tracked_wago_result_folder(result);
            let install_state = self.search_install_state_for_result(result);
            let mut lines = vec![Line::from(Span::styled(
                result.display_name.clone(),
                Style::default()
                    .fg(self.ui_theme.highlight)
                    .add_modifier(Modifier::BOLD),
            ))];
            match install_state {
                Some(SearchInstallState::Checking { .. }) => {
                    lines.push(Line::from(shimmer_text_spans(
                        "Checking tracked state and preparing install…",
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )));
                }
                Some(SearchInstallState::Installing { .. }) => {
                    lines.push(Line::from(shimmer_text_spans(
                        "Installing from Wago…",
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )));
                }
                Some(SearchInstallState::Success { parent_folder, .. }) => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "✓ ",
                            Style::default()
                                .fg(self.ui_theme.success)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("Installed successfully into {parent_folder}"),
                            Style::default()
                                .fg(self.ui_theme.success)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
                Some(SearchInstallState::ConfirmReplace { tracked_parent, .. }) => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "📦 installed ",
                            Style::default()
                                .fg(self.ui_theme.panel_title)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("as {tracked_parent} · y reinstall · n cancel"),
                            Style::default().fg(self.ui_theme.panel_title),
                        ),
                    ]));
                }
                Some(SearchInstallState::Error { message, .. }) => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "Error ",
                            Style::default()
                                .fg(self.ui_theme.error)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(message.clone(), Style::default().fg(self.ui_theme.error)),
                    ]));
                }
                _ => {
                    if let Some(folder) = tracked_folder.as_deref() {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "📦 installed ",
                                Style::default()
                                    .fg(self.ui_theme.panel_title)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("already installed as {folder}"),
                                Style::default().fg(self.ui_theme.panel_title),
                            ),
                        ]));
                    }
                }
            }
            lines.push(Line::from(""));
            lines.extend([
                self.overlay_kv_line("Author", author),
                self.overlay_kv_line("Version", result.version.as_deref().unwrap_or("unknown")),
                self.overlay_kv_line("Slug", result.id.as_str()),
                self.overlay_kv_line(
                    "Summary",
                    result.summary.as_deref().unwrap_or("No summary provided."),
                ),
            ]);
            lines
        } else {
            vec![
                Line::from(Span::styled(
                    "No result selected",
                    Style::default().fg(self.ui_theme.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Run a search, then use ↑/↓ to choose a result.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]
        };

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    pub(super) fn search_result_name_line(
        &self,
        result: &WagoSearchResult,
        selected: bool,
        tracked_folder: Option<&str>,
        install_state: Option<&SearchInstallState>,
    ) -> Line<'static> {
        let mut spans = vec![Span::styled(
            if selected { "› " } else { "  " },
            Style::default().fg(self.ui_theme.highlight),
        )];

        let name_spans = if install_state.is_some() {
            shimmer_text_spans(
                &truncate_text(&result.display_name, 28),
                self.ui_theme.info,
                self.ui_theme.shimmer_glow,
                ShimmerConfig::action(),
            )
        } else {
            vec![Span::styled(
                truncate_text(&result.display_name, 28),
                if selected {
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.ui_theme.info)
                },
            )]
        };
        spans.extend(name_spans);

        if let Some(state) = install_state {
            spans.push(Span::raw("  "));
            match state {
                SearchInstallState::Checking { .. } => spans.extend(shimmer_text_spans(
                    "checking",
                    self.ui_theme.warning,
                    self.ui_theme.shimmer_glow,
                    ShimmerConfig::action(),
                )),
                SearchInstallState::Installing { .. } => spans.extend(shimmer_text_spans(
                    "installing",
                    self.ui_theme.warning,
                    self.ui_theme.shimmer_glow,
                    ShimmerConfig::action(),
                )),
                SearchInstallState::Success { .. } => spans.push(Span::styled(
                    "✓ installed",
                    Style::default()
                        .fg(self.ui_theme.success)
                        .add_modifier(Modifier::BOLD),
                )),
                SearchInstallState::ConfirmReplace { .. } => spans.push(Span::styled(
                    "📦 installed",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                )),
                SearchInstallState::Error { .. } => spans.push(Span::styled(
                    "error",
                    Style::default()
                        .fg(self.ui_theme.error)
                        .add_modifier(Modifier::BOLD),
                )),
                SearchInstallState::Idle => {}
            }
        } else if tracked_folder.is_some() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "📦 installed",
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        Line::from(spans)
    }

    pub(super) fn tracked_wago_result_folder(&self, result: &WagoSearchResult) -> Option<String> {
        let result_slug = wago_result_slug(result)?;
        self.dashboard
            .items
            .iter()
            .find(|item| {
                item.source == SourceKind::Wago
                    && item
                        .source_url
                        .as_deref()
                        .and_then(extract_wago_slug)
                        .is_some_and(|slug| slug.eq_ignore_ascii_case(&result_slug))
            })
            .map(|item| item.folder.clone())
    }

    pub(super) fn search_install_state_for_result(
        &self,
        result: &WagoSearchResult,
    ) -> Option<&SearchInstallState> {
        let (addon_id, _) = self.search_pane.install_identity();
        if addon_id.as_deref() == Some(result.id.as_str()) {
            match &self.search_pane.install_state {
                SearchInstallState::Idle => None,
                state => Some(state),
            }
        } else {
            None
        }
    }
}
