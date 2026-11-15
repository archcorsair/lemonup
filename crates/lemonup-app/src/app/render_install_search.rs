use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use super::sheet::{SheetBody, render_stateful, text_height};
use super::*;

const SEARCH_EMPTY_HINT_SOURCES: &str =
    "Search Wago.io by name, or paste a GitHub, WoWInterface or Wago.io link.";
const SEARCH_EMPTY_HINT_TUKUI: &str = "Type elvui or tukui for TukUI.";

impl App {
    /// The summary, the install box (always drawn), then the empty state or
    /// the results and the selected result. The results get the rows the
    /// rest leaves.
    pub(super) fn search_sheet_body(&self, width: u16, max_height: u16) -> SheetBody<'_> {
        let summary = self.search_summary_lines();
        let summary_height = text_height(&summary, width);
        let mut body = SheetBody::new();
        body.text(summary)
            .widget(3, |area, buffer| self.render_search_input(area, buffer))
            .gap(1);
        if self.search_pane.results.is_empty() {
            body.centered_text(self.search_empty_state_lines());
            return body;
        }

        let preview = self.search_selected_preview_lines();
        // Summary, box, gap, two headings and the preview.
        let fixed = summary_height
            .saturating_add(6)
            .saturating_add(text_height(&preview, width));
        let needed = u16::try_from(self.search_pane.results.len())
            .unwrap_or(u16::MAX)
            .saturating_add(1);
        let room = max_height.saturating_sub(fixed);
        let results_height = if needed <= room {
            needed
        } else {
            room.max(needed.min(4))
        };
        body.heading(" Results ")
            .widget(results_height, |area, buffer| {
                self.render_search_results_list(area, buffer)
            })
            .focus_last()
            .heading(" Selected ")
            .text(preview);
        body
    }

    pub(super) fn render_search_input(&self, area: Rect, buffer: &mut Buffer) {
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
                    "Type an addon name or paste an addon URL",
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

        Paragraph::new(Line::from(spans))
            .block(block)
            .wrap(Wrap { trim: false })
            .render(area, buffer);
    }

    pub(super) fn search_summary_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            Span::styled("Use: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                "Search Wago.io by addon name",
                Style::default().fg(self.ui_theme.panel_title),
            ),
            Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                "Paste an addon URL to install directly",
                Style::default().fg(self.ui_theme.panel_title),
            ),
        ])];

        let status_line = if self.search_pane.in_progress {
            let mut spans = vec![Span::styled(
                "Status: ",
                Style::default().fg(self.ui_theme.muted),
            )];
            spans.extend(shimmer_text_spans(
                "Searching Wago.io…",
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
                        "Installing from Wago.io…",
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
                SearchInstallState::ConfirmDirect { .. } => Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                    Span::styled(
                        "Waiting for confirmation",
                        Style::default()
                            .fg(self.ui_theme.highlight)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                SearchInstallState::InstallingDirect { request } => {
                    let mut spans = vec![Span::styled(
                        "Status: ",
                        Style::default().fg(self.ui_theme.muted),
                    )];
                    spans.extend(shimmer_text_spans(
                        &format!("Installing from {}…", request.provider.label()),
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    ));
                    Line::from(spans)
                }
                SearchInstallState::Idle => self.default_search_status_line(),
            }
        } else if self.wago_api_key.is_none() {
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Wago.io search and installs need your Wago.io key",
                    Style::default()
                        .fg(self.ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
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
                    match classify_search_input(&self.search_pane.query) {
                        SearchInputIntent::Empty => "Enter an addon name or URL".to_string(),
                        SearchInputIntent::Direct(request) => {
                            format!("Press Enter to install from {}", request.provider.label())
                        }
                        SearchInputIntent::WagoUrl(_) => {
                            "Press Enter to install directly".to_string()
                        }
                        SearchInputIntent::InvalidUrl(_) => {
                            "Press Enter to check this URL".to_string()
                        }
                        SearchInputIntent::WagoSearch(_) => {
                            "Press Enter to search Wago.io".to_string()
                        }
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

    /// The Wago slug when the query is a Wago addon URL (after the GitHub, WoWInterface and
    /// TukUI checks, which take precedence).
    pub(super) fn search_direct_install_target(&self) -> Option<String> {
        match classify_search_input(&self.search_pane.query) {
            SearchInputIntent::WagoUrl(slug) => Some(slug),
            _ => None,
        }
    }

    pub(super) fn search_empty_state_lines(&self) -> Vec<Line<'static>> {
        let mut message = if let Some(state) = self.active_search_install_state() {
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
                        "LemonUp is checking your addon list and preparing the package.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Installing { addon_name, .. } => vec![
                    Line::from(shimmer_text_spans(
                        &format!("Installing {addon_name} from Wago.io…"),
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "It will show up in your addon list when it is done.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Success {
                    addon_name,
                    parent_folder,
                    warning,
                    ..
                } => {
                    let mut lines = vec![
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
                    ];
                    if let Some(warning) = warning {
                        lines.push(Line::from(Span::styled(
                            warning.clone(),
                            Style::default().fg(self.ui_theme.warning),
                        )));
                    }
                    lines
                }
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
                        format!("You have it as {tracked_parent}."),
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
                SearchInstallState::ConfirmDirect { request } => vec![Line::from(Span::styled(
                    format!(
                        "Install from {}: {}?",
                        request.provider.label(),
                        request.display
                    ),
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ))],
                SearchInstallState::InstallingDirect { request } => vec![
                    Line::from(shimmer_text_spans(
                        &format!(
                            "Installing {} from {}…",
                            request.display,
                            request.provider.label()
                        ),
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "It will show up in your addon list when it is done.",
                        Style::default().fg(self.ui_theme.muted),
                    )),
                ],
                SearchInstallState::Idle => unreachable!(),
            }
        } else if self.search_pane.in_progress {
            vec![
                Line::from(Span::styled(
                    "Searching Wago.io…",
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
                    "Start with an addon name or URL",
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    SEARCH_EMPTY_HINT_SOURCES,
                    Style::default().fg(self.ui_theme.muted),
                )),
                Line::from(Span::styled(
                    SEARCH_EMPTY_HINT_TUKUI,
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]
        };
        if self.wago_api_key.is_none() && self.active_search_install_state().is_none() {
            message.push(Line::from(""));
            message.push(Line::from(Span::styled(
                "Wago.io search needs your Wago.io key: press , to open Settings.",
                Style::default().fg(self.ui_theme.warning),
            )));
        }

        message
    }

    pub(super) fn render_search_results_list(&self, area: Rect, buffer: &mut Buffer) {
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
            Widget::render(
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
                buffer,
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
                            .unwrap_or_else(|| "-".to_string()),
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
        render_stateful(table, table_area, buffer, &mut state);

        if show_below {
            let remaining_below = result_count.saturating_sub(window_end);
            Widget::render(
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
                buffer,
            );
        }
    }

    pub(super) fn search_selected_preview_lines(&self) -> Vec<Line<'static>> {
        if let Some(result) = self.search_pane.selected_result() {
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
                        "Checking your addon list and preparing install…",
                        self.ui_theme.warning,
                        self.ui_theme.shimmer_glow,
                        ShimmerConfig::action(),
                    )));
                }
                Some(SearchInstallState::Installing { .. }) => {
                    lines.push(Line::from(shimmer_text_spans(
                        "Installing from Wago.io…",
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
                            format!("as {tracked_parent}"),
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
        }
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
                SearchInstallState::ConfirmDirect { .. }
                | SearchInstallState::InstallingDirect { .. } => {}
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
