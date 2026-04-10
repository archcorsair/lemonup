use super::*;
impl App {
    pub(super) fn render_header(&self, frame: &mut Frame<'_>, area: Rect, mode: ShellLayoutMode) {
        if self.shell_mode == ShellMode::Onboarding {
            self.render_onboarding_header(frame, area);
            return;
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.ui_theme.border));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [left, right] = match mode {
            ShellLayoutMode::Standard => Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(inner),
            ShellLayoutMode::Compact => [inner, Rect::new(inner.x, inner.y, 0, 0)],
        };

        let logo = Paragraph::new(self.header_logo_lines(mode));
        frame.render_widget(logo, left);

        if mode == ShellLayoutMode::Standard {
            let meta = Paragraph::new(self.header_meta_lines(mode)).wrap(Wrap { trim: false });
            frame.render_widget(meta, right);
        }

        if self.shell_mode == ShellMode::Dashboard
            && self.shell_ui.overlay.active.is_none()
            && let Some((event_text, event_kind)) = self.dashboard_event_message()
        {
            let anchor = if mode == ShellLayoutMode::Standard && right.width > 0 {
                right
            } else {
                inner
            };
            self.render_dashboard_toast(frame, anchor, &event_text, event_kind);
        }
    }

    pub(super) fn render_dashboard(&mut self, frame: &mut Frame<'_>, area: Rect) {
        self.render_dashboard_list(frame, area);
    }

    pub(super) fn render_dashboard_toast(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        text: &str,
        kind: DashboardEventKind,
    ) {
        let padded = if area.width > 2 {
            area.inner(Margin {
                horizontal: 1,
                vertical: 0,
            })
        } else {
            area
        };
        let (icon_fg, badge_bg, text_fg, bg, prefix) = match kind {
            DashboardEventKind::Info => (
                self.ui_theme.toast_info_icon,
                self.ui_theme.toast_info_badge_bg,
                self.ui_theme.info,
                self.ui_theme.toast_info_bg,
                "ℹ",
            ),
            DashboardEventKind::Success => (
                self.ui_theme.toast_success_icon,
                self.ui_theme.toast_success_badge_bg,
                self.ui_theme.success,
                self.ui_theme.toast_success_bg,
                "✓",
            ),
            DashboardEventKind::Error => (
                self.ui_theme.toast_error_icon,
                self.ui_theme.toast_error_badge_bg,
                self.ui_theme.error,
                self.ui_theme.toast_error_bg,
                "×",
            ),
        };
        let toast_y = padded.y.saturating_add(padded.height.saturating_sub(1));
        let line = Line::from(vec![
            Span::styled(
                format!(" {prefix} "),
                Style::default()
                    .fg(icon_fg)
                    .bg(badge_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {text} "),
                Style::default()
                    .fg(text_fg)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let pill_width = line.width().min(padded.width as usize) as u16;
        let pill_area = Rect::new(padded.x, toast_y, pill_width, 1);
        frame.render_widget(Clear, pill_area);
        let pill = Paragraph::new(line)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false });
        frame.render_widget(pill, pill_area);
    }

    pub(super) fn render_dashboard_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        self.last_dashboard_list_area = Some(area);
        if self.dashboard.rows.is_empty() {
            let body = Paragraph::new(vec![
                Line::from("No scanned addons yet."),
                Line::from(format!("Scan state: {}", self.scan_status_label())),
                Line::from(
                    "If this is your first launch on this profile, wait for the background scan.",
                ),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.ui_theme.border))
                    .title(Span::styled(
                        "Addons",
                        Style::default()
                            .fg(self.ui_theme.panel_title)
                            .add_modifier(Modifier::BOLD),
                    )),
            );
            frame.render_widget(body, area);
            return;
        }

        let header = Row::new(vec![
            Cell::from(self.dashboard_sort_header(DashboardSortColumn::Name)),
            Cell::from(self.dashboard_sort_header(DashboardSortColumn::Version)),
            Cell::from(self.dashboard_sort_header(DashboardSortColumn::Author)),
            Cell::from(self.dashboard_sort_header(DashboardSortColumn::Source)),
        ])
        .style(
            Style::default()
                .fg(self.ui_theme.panel_title)
                .add_modifier(Modifier::BOLD),
        );

        let rows = self
            .dashboard
            .rows
            .iter()
            .map(|row| self.dashboard_table_row(row))
            .map(|view| {
                Row::new(vec![
                    Cell::from(view.name),
                    Cell::from(view.version),
                    Cell::from(view.author),
                    Cell::from(view.source),
                ])
                .style(view.row_style)
            })
            .collect::<Vec<_>>();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(43),
                Constraint::Percentage(23),
                Constraint::Percentage(22),
                Constraint::Percentage(12),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.ui_theme.border))
                .title(Span::styled(
                    format!(
                        "Addons ({} selected)",
                        self.dashboard.selected_parent_count()
                    ),
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(self.ui_theme.row_highlight_bg)
                .add_modifier(Modifier::BOLD),
        );

        frame.render_stateful_widget(table, area, &mut self.dashboard.list_state);
    }

    pub(super) fn dashboard_sort_header(&self, column: DashboardSortColumn) -> Line<'static> {
        let sort = self.dashboard.sort_config();
        let mut spans = vec![Span::styled(
            column.label(),
            Style::default()
                .fg(self.ui_theme.panel_title)
                .add_modifier(Modifier::BOLD),
        )];
        if sort.column == column {
            spans.extend([
                Span::raw(" "),
                Span::styled(
                    sort.direction.indicator(),
                    Style::default()
                        .fg(self.ui_theme.brand_gold)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
        }
        Line::from(spans)
    }

    pub(super) fn dashboard_table_row(&self, row: &DashboardRow) -> AddonTableRowViewModel {
        let selected = self.dashboard.is_parent_selected(&row.folder);
        let item = self.dashboard.items.iter().find(|item| match &row.kind {
            DashboardRowKind::Parent => item.folder == row.folder,
            DashboardRowKind::OwnedChild { parent_folder } => item.folder == *parent_folder,
        });
        let is_child = matches!(row.kind, DashboardRowKind::OwnedChild { .. });
        let active_job = self
            .dashboard
            .job_ui()
            .filter(|job| job.current_addon == row.folder);
        let name = dashboard_row_name_line(row, selected);
        let version = if is_child {
            Line::from("")
        } else {
            item.map(|item| {
                dashboard_item_version_line(item, active_job, self.shell_ui.motion, self.ui_theme)
            })
            .unwrap_or_else(|| Line::from(""))
        };
        let author = if is_child {
            String::new()
        } else {
            item.map(dashboard_item_author_label)
                .unwrap_or_else(String::new)
        };
        let source = if is_child {
            Line::from("")
        } else {
            item.map(dashboard_item_source_line)
                .unwrap_or_else(|| Line::from(""))
        };
        let row_style = if is_child {
            Style::default().fg(self.ui_theme.muted)
        } else if let Some(job) = active_job {
            let tint = match job.kind {
                DashboardJobKind::Check => self.ui_theme.job_check_bg,
                DashboardJobKind::Update => self.ui_theme.job_update_bg,
            };
            Style::default().bg(tint)
        } else if selected {
            Style::default().bg(self.ui_theme.key_bg_selected)
        } else {
            Style::default()
        };

        AddonTableRowViewModel {
            name,
            version,
            author,
            source,
            row_style,
        }
    }
}

pub(super) fn dashboard_row_name_line(row: &DashboardRow, selected: bool) -> Line<'static> {
    let gutter = match &row.kind {
        DashboardRowKind::Parent if selected => Span::styled("▎", Style::default().fg(Color::Cyan)),
        DashboardRowKind::Parent => Span::raw(" "),
        DashboardRowKind::OwnedChild { .. } => Span::raw(" "),
    };
    let marker = match &row.kind {
        DashboardRowKind::Parent if row.expandable && row.expanded => "▾ ",
        DashboardRowKind::Parent if row.expandable => "▸ ",
        DashboardRowKind::Parent => "  ",
        DashboardRowKind::OwnedChild { .. } => child_row_prefix(row),
    };

    Line::from(vec![
        gutter,
        Span::raw(marker),
        Span::raw(truncate_text(&row.name, 40)),
    ])
}

pub(super) fn dashboard_item_version_label(item: &DashboardItem) -> String {
    match item.source {
        SourceKind::GitHub => {
            if let Some(commit) = item.git_commit.as_deref() {
                return shorten_commit(commit);
            }
            item.version
                .as_deref()
                .filter(|value| !is_placeholder_version(value))
                .map(|value| {
                    if looks_like_commit_hash(value) {
                        shorten_commit(value)
                    } else {
                        value.to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".to_string())
        }
        _ => item
            .version
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

pub(super) fn dashboard_item_remote_label(item: &DashboardItem) -> Option<String> {
    match item.source {
        SourceKind::GitHub => item
            .remote_version
            .as_deref()
            .filter(|value| looks_like_commit_hash(value))
            .map(shorten_commit),
        _ => item
            .remote_version
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(ToString::to_string),
    }
}

pub(super) fn dashboard_item_sort_version_key(
    item: &DashboardItem,
) -> (
    u8,
    std::cmp::Reverse<OffsetDateTime>,
    std::cmp::Reverse<OffsetDateTime>,
    String,
    String,
) {
    let priority = match dashboard_item_update_status(item) {
        UpdateStatus::UpdateAvailable => 0,
        UpdateStatus::UpToDate => 1,
        UpdateStatus::Unknown => 2,
        UpdateStatus::Error => 3,
    };

    (
        priority,
        std::cmp::Reverse(item.updated_at),
        std::cmp::Reverse(item.installed_at),
        dashboard_item_version_label(item),
        dashboard_item_remote_label(item).unwrap_or_default(),
    )
}

pub(super) fn dashboard_item_version_line(
    item: &DashboardItem,
    active_job: Option<&DashboardJobUiState>,
    motion: MotionState,
    ui_theme: UiTheme,
) -> Line<'static> {
    let installed = truncate_middle_text(&dashboard_item_version_label(item), 16);
    let installed_style = Style::default().fg(ui_theme.installed_text);
    let active_prefix = active_job.map(|job| {
        vec![
            Span::styled(
                motion.spinner_frame(),
                Style::default()
                    .fg(job.kind.accent(ui_theme))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]
    });

    if item.source == SourceKind::Manual {
        let mut spans = active_prefix.unwrap_or_default();
        spans.push(Span::styled(installed, installed_style));
        return Line::from(spans);
    }

    if !item.has_authoritative_owned_folders {
        let mut spans = active_prefix.unwrap_or_default();
        spans.extend([
            Span::styled(installed, installed_style),
            Span::raw(" "),
            Span::styled(
                "unmanaged",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        return Line::from(spans);
    }

    match dashboard_item_update_status(item) {
        UpdateStatus::UpToDate => {
            let mut spans = active_prefix.unwrap_or_default();
            spans.push(Span::styled(installed, installed_style));
            Line::from(spans)
        }
        UpdateStatus::UpdateAvailable => {
            let remote = truncate_middle_text(
                &dashboard_item_remote_label(item).unwrap_or_else(|| "update".to_string()),
                12,
            );
            let mut spans = active_prefix.unwrap_or_default();
            spans.extend([
                Span::styled(installed, installed_style),
                Span::raw(" "),
                Span::styled("→", Style::default().fg(ui_theme.warning)),
                Span::raw(" "),
                Span::styled(
                    remote,
                    Style::default()
                        .fg(ui_theme.warning)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled("📦", Style::default().fg(ui_theme.package_icon)),
            ]);
            Line::from(spans)
        }
        UpdateStatus::Unknown => {
            let mut spans = active_prefix.unwrap_or_default();
            spans.extend([
                Span::styled(installed, installed_style),
                Span::raw(" "),
                Span::styled(
                    "unknown",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            Line::from(spans)
        }
        UpdateStatus::Error => {
            let mut spans = active_prefix.unwrap_or_default();
            spans.extend([
                Span::styled(installed, installed_style),
                Span::raw(" "),
                Span::styled(
                    "error",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ]);
            Line::from(spans)
        }
    }
}

pub(super) fn dashboard_item_author_label(item: &DashboardItem) -> String {
    item.author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .map(|author| truncate_text(author, 24))
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn dashboard_item_author_sort_key(item: &DashboardItem) -> String {
    item.author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .unwrap_or("~")
        .to_ascii_lowercase()
}

pub(super) fn dashboard_item_source_label(item: &DashboardItem) -> &'static str {
    match item.source {
        SourceKind::Wago => "wago",
        SourceKind::Tukui => "tukui",
        SourceKind::GitHub => "github",
        SourceKind::WowInterface => "wowi",
        SourceKind::Manual => "manual",
    }
}

pub(super) fn dashboard_item_source_line(item: &DashboardItem) -> Line<'static> {
    let label = source_compact_label(item.source);
    let color = source_badge_color(item.source);

    Line::from(vec![
        Span::styled("[", Style::default().fg(self::Color::DarkGray)),
        Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(self::Color::DarkGray)),
    ])
}

pub(super) fn dashboard_item_management_summary(item: &DashboardItem, has_drift: bool) -> String {
    let mut parts = Vec::new();
    if has_drift {
        parts.push("drift");
    }
    if item.has_authoritative_owned_folders {
        parts.push("managed");
    } else if item.owned_folder_count > 0 {
        parts.push("inferred");
    }
    if item.kind == AddonKind::Library {
        parts.push("library");
    }
    if item.owned_folder_count > 0 {
        parts.push("tree");
    }

    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

pub(super) fn dashboard_item_status_text(item: &DashboardItem) -> String {
    if item.source == SourceKind::Manual {
        return "Manual".to_string();
    }
    if !item.has_authoritative_owned_folders {
        return "Unmanaged".to_string();
    }
    match dashboard_item_update_status(item) {
        UpdateStatus::UpToDate => "Up to date".to_string(),
        UpdateStatus::UpdateAvailable => dashboard_item_remote_label(item)
            .map(|version| format!("Update: {version}"))
            .unwrap_or_else(|| "Update".to_string()),
        UpdateStatus::Unknown => "Unknown".to_string(),
        UpdateStatus::Error => "Error".to_string(),
    }
}

pub(super) fn dashboard_item_update_status(item: &DashboardItem) -> UpdateStatus {
    let mut addon = AddonRecord::new(item.name.clone(), item.folder.clone(), item.source);
    addon.version = item.version.clone();
    addon.remote_version = item.remote_version.clone();
    addon.git_commit = item.git_commit.clone();
    let (status, _) = crate::update::determine_update_status(&addon);
    status
}

pub(super) fn child_row_prefix(row: &DashboardRow) -> &'static str {
    match row.child_connector {
        Some(DashboardChildConnector::Mid) => "  ├─ ",
        Some(DashboardChildConnector::Last) => "  └─ ",
        None => "  └─ ",
    }
}

#[cfg(test)]
pub(super) fn child_row_detail_prefix(row: &DashboardRow) -> &'static str {
    match row.child_connector {
        Some(DashboardChildConnector::Mid) => "  │  ",
        Some(DashboardChildConnector::Last) => "     ",
        None => "     ",
    }
}
