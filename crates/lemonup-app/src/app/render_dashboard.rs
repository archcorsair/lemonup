use super::keymap::fit_key_line;
use super::render_inspect::docked_widths;
use super::sheet::{SheetScroll, wrap_lines};
use super::*;
use crate::theme::ColorRoles;

/// Below this width the header uses the word LEMONUP and the list drops the
/// Author column and shortens source names.
pub(super) const WIDE_FRAME_MIN_WIDTH: u16 = 110;
/// Cursor bar, selection dot and expand triangle.
const GUTTER_WIDTH: u16 = 5;
/// Width of the progress bar on the status line while a job runs.
const JOB_BAR_WIDTH: usize = 28;

/// The rows of the dashboard frame, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DashboardFrameLayout {
    pub(super) header: Rect,
    /// Blank under the header. While the filter is open or applied, the
    /// filter box covers the header and this row (see `filter_box`).
    pub(super) filter: Rect,
    pub(super) column_header: Rect,
    pub(super) top_rule: Rect,
    pub(super) list: Rect,
    pub(super) bottom_rule: Rect,
    pub(super) status: Rect,
    pub(super) keys: Rect,
}

impl DashboardFrameLayout {
    pub(super) fn new(area: Rect) -> Self {
        let [
            header,
            filter,
            column_header,
            top_rule,
            list,
            bottom_rule,
            status,
            keys,
        ] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(area);
        Self {
            header,
            filter,
            column_header,
            top_rule,
            list,
            bottom_rule,
            status,
            keys,
        }
    }

    /// The filter box: the header rows and the filter row together, so the
    /// column header and the list stay where they are when it opens.
    pub(super) fn filter_box(&self) -> Rect {
        Rect::new(
            self.header.x,
            self.header.y,
            self.header.width,
            self.filter.bottom().saturating_sub(self.header.y),
        )
    }

    /// Where sheets draw: everything above the status line and the key line.
    pub(super) fn sheet_region(&self) -> Rect {
        let top = self.header.y;
        Rect::new(
            self.header.x,
            top,
            self.header.width,
            self.status.y.saturating_sub(top),
        )
    }
}

/// Column widths of the addon list for one window width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ListColumns {
    pub(super) name: u16,
    pub(super) status: u16,
    pub(super) version: u16,
    /// Zero when the Author column is hidden.
    pub(super) author: u16,
    pub(super) source: u16,
    pub(super) wide: bool,
}

impl ListColumns {
    /// The widths from the design for a list `width` columns wide (the whole
    /// window today; a docked Details panel would take its share first and
    /// pass what is left). At 110 columns and wider: Addon 33, Status 21,
    /// Version 22, Author 26, Source 13. Below: Addon 27, Status 21,
    /// Version 19, Source 8 and no Author.
    ///
    /// Extra width grows Addon to 40, then Author to 36, then Version to 26.
    /// Anything left stays empty at the right edge: columns never stretch.
    pub(super) fn for_width(width: u16) -> Self {
        let wide = width >= WIDE_FRAME_MIN_WIDTH;
        let mut columns = if wide {
            Self {
                name: 33,
                status: 21,
                version: 22,
                author: 26,
                source: 13,
                wide,
            }
        } else {
            Self {
                name: 27,
                status: 21,
                version: 19,
                author: 0,
                source: 8,
                wide,
            }
        };
        let base = columns.total();
        if width < base {
            // Too narrow for the design: Addon gives way first.
            columns.name = columns.name.saturating_sub(base - width).max(8);
            return columns;
        }
        let mut extra = width - base;
        let mut grow = |column: &mut u16, cap: u16| {
            let step = cap.saturating_sub(*column).min(extra);
            *column += step;
            extra -= step;
        };
        grow(&mut columns.name, 40);
        if columns.author > 0 {
            grow(&mut columns.author, 36);
        }
        grow(&mut columns.version, 26);
        columns
    }

    /// The list beside docked Details: Addon, Status and Version only. At
    /// 78 columns (a 120 column window): Addon 30, Status 21, Version 22.
    /// Extra width grows Addon to 40, then Version to 26.
    pub(super) fn for_docked(width: u16) -> Self {
        let mut columns = Self {
            name: 30,
            status: 21,
            version: 22,
            author: 0,
            source: 0,
            wide: true,
        };
        let base = columns.total();
        if width < base {
            columns.name = columns.name.saturating_sub(base - width).max(8);
            return columns;
        }
        let mut extra = width - base;
        let mut grow = |column: &mut u16, cap: u16| {
            let step = cap.saturating_sub(*column).min(extra);
            *column += step;
            extra -= step;
        };
        grow(&mut columns.name, 40);
        grow(&mut columns.version, 26);
        columns
    }

    /// Columns used, gutter included.
    pub(super) fn total(self) -> u16 {
        GUTTER_WIDTH + self.name + self.status + self.version + self.author + self.source
    }

    fn constraints(self) -> Vec<Constraint> {
        let mut constraints = vec![
            Constraint::Length(GUTTER_WIDTH),
            Constraint::Length(self.name),
            Constraint::Length(self.status),
            Constraint::Length(self.version),
        ];
        if self.author > 0 {
            constraints.push(Constraint::Length(self.author));
        }
        if self.source > 0 {
            constraints.push(Constraint::Length(self.source));
        }
        // Empty space at the right edge, so the cursor row still spans it.
        constraints.push(Constraint::Fill(1));
        constraints
    }
}

/// What the Status column says about one addon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowStatus {
    UpdateReady,
    UpToDate,
    NewerThanSource,
    NotLinked,
    CheckFailed,
    /// Linked, but no check has answered yet.
    NotChecked,
    Checking,
    Updating,
    Waiting,
    /// The update has moved past this addon; its result is not known yet.
    Done,
    Updated,
    UpdateFailed,
}

impl RowStatus {
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::UpdateReady => "Update ready",
            Self::UpToDate => "Up to date",
            Self::NewerThanSource => "Newer than source",
            Self::NotLinked => "Not linked",
            Self::CheckFailed => "Check failed",
            Self::NotChecked => "Not checked yet",
            Self::Checking => "Checking…",
            Self::Updating => "Updating…",
            Self::Waiting => "Waiting",
            Self::Done => "Done",
            Self::Updated => "Updated",
            Self::UpdateFailed => "Update failed",
        }
    }

    /// The symbol in front of the word. Working states use the spinner frame.
    pub(super) fn symbol(self, spinner: &'static str) -> &'static str {
        match self {
            Self::UpdateReady => "↑",
            Self::UpToDate | Self::NewerThanSource | Self::Updated => "✓",
            Self::CheckFailed | Self::UpdateFailed => "!",
            Self::Checking | Self::Updating => spinner,
            Self::NotLinked | Self::NotChecked | Self::Waiting | Self::Done => "·",
        }
    }

    /// The status word as plain text, for example `↑ Update ready`.
    #[cfg(test)]
    pub(super) fn label(self, spinner: &'static str) -> String {
        format!("{} {}", self.symbol(spinner), self.word())
    }

    pub(super) fn spans(self, spinner: &'static str, roles: ColorRoles) -> Vec<Span<'static>> {
        let symbol = self.symbol(spinner);
        let word = self.word();
        let bold = Modifier::BOLD;
        match self {
            Self::UpdateReady => vec![Span::styled(
                format!("{symbol} {word}"),
                Style::default().fg(roles.amber).add_modifier(bold),
            )],
            Self::UpToDate | Self::NewerThanSource => vec![
                Span::styled(symbol, Style::default().fg(roles.green)),
                Span::styled(format!(" {word}"), Style::default().fg(roles.muted)),
            ],
            Self::Updated => vec![Span::styled(
                format!("{symbol} {word}"),
                Style::default().fg(roles.green).add_modifier(bold),
            )],
            Self::CheckFailed | Self::UpdateFailed => vec![Span::styled(
                format!("{symbol} {word}"),
                Style::default().fg(roles.red).add_modifier(bold),
            )],
            Self::Checking | Self::Updating => vec![
                Span::styled(symbol, Style::default().fg(roles.lemon)),
                Span::styled(
                    format!(" {word}"),
                    Style::default().fg(roles.text).add_modifier(bold),
                ),
            ],
            Self::NotLinked | Self::NotChecked | Self::Waiting | Self::Done => vec![Span::styled(
                format!("{symbol} {word}"),
                Style::default().fg(roles.muted),
            )],
        }
    }
}

/// The status of an addon from its own data, ignoring any running job.
pub(super) fn dashboard_item_row_status(item: &DashboardItem) -> RowStatus {
    if item.source == SourceKind::Manual {
        return RowStatus::NotLinked;
    }
    match dashboard_item_update_status(item) {
        UpdateStatus::UpdateAvailable => RowStatus::UpdateReady,
        UpdateStatus::UpToDate => RowStatus::UpToDate,
        UpdateStatus::RemoteOlder => RowStatus::NewerThanSource,
        UpdateStatus::Error => RowStatus::CheckFailed,
        UpdateStatus::Unknown => RowStatus::NotChecked,
    }
}

/// The Version column: the installed version, and the new one only when an
/// update is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VersionCell {
    pub(super) installed: String,
    pub(super) new: Option<String>,
}

impl VersionCell {
    pub(super) fn for_item(item: &DashboardItem, wide: bool) -> Self {
        let new = (dashboard_item_row_status(item) == RowStatus::UpdateReady).then(|| {
            if item.source == SourceKind::GitHub {
                // A commit hash means nothing to a player; the hash is in Details.
                if wide { "new build" } else { "newer" }.to_string()
            } else {
                dashboard_item_remote_label(item).unwrap_or_else(|| "update".to_string())
            }
        });
        Self {
            installed: dashboard_item_installed_version(item),
            new,
        }
    }

    pub(super) fn text(&self) -> String {
        match &self.new {
            Some(new) => format!("{} → {new}", self.installed),
            None => self.installed.clone(),
        }
    }

    /// The cell fitted to `max` characters, cutting the installed version
    /// first so the new version stays readable.
    fn spans(&self, max: usize, roles: ColorRoles) -> Vec<Span<'static>> {
        let muted = Style::default().fg(roles.muted);
        match &self.new {
            None => vec![Span::styled(truncate_text(&self.installed, max), muted)],
            Some(new) => {
                let suffix = new.chars().count() + 3;
                if max <= suffix + 1 {
                    return vec![Span::styled(truncate_text(&self.text(), max), muted)];
                }
                let installed = truncate_text(&self.installed, max - suffix);
                vec![
                    Span::styled(format!("{installed} →"), muted),
                    Span::raw(" "),
                    Span::styled(
                        new.clone(),
                        Style::default()
                            .fg(roles.amber)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]
            }
        }
    }
}

/// The installed version as the addon's own files give it. For GitHub addons
/// without a usable version this falls back to the short commit.
pub(super) fn dashboard_item_installed_version(item: &DashboardItem) -> String {
    let own = item
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !is_placeholder_version(value));
    if item.source == SourceKind::GitHub {
        if let Some(version) = own.filter(|value| !looks_like_commit_hash(value)) {
            return version.to_string();
        }
        if let Some(commit) = item.git_commit.as_deref().or(own) {
            return shorten_commit(commit);
        }
        return "unknown".to_string();
    }
    own.map(ToString::to_string)
        .unwrap_or_else(|| "unknown".to_string())
}

/// Source names in full at 110 columns and wider, short below. Empty for an
/// addon that is not linked.
pub(super) fn dashboard_source_name(source: SourceKind, wide: bool) -> &'static str {
    match (source, wide) {
        (SourceKind::Wago, true) => "Wago.io",
        (SourceKind::Wago, false) => "Wago",
        (SourceKind::GitHub, _) => "GitHub",
        (SourceKind::Tukui, _) => "TukUI",
        (SourceKind::WowInterface, true) => "WoWInterface",
        (SourceKind::WowInterface, false) => "WoWI",
        (SourceKind::Manual, _) => "",
    }
}

/// "checked 5 min ago", from the latest check of any addon.
pub(super) fn checked_ago_label(now: OffsetDateTime, checked_at: OffsetDateTime) -> String {
    let seconds = (now - checked_at).whole_seconds().max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 1 {
        "checked just now".to_string()
    } else if hours < 1 {
        format!("checked {minutes} min ago")
    } else if days < 1 {
        format!("checked {hours} hour{} ago", plural_suffix(hours as usize))
    } else {
        format!("checked {days} day{} ago", plural_suffix(days as usize))
    }
}

/// The path with whole folders removed from the middle until it fits:
/// `~/Games/…/_retail_/Interface/AddOns`.
pub(super) fn shorten_path_middle(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let separator = if path.contains('/') { '/' } else { '\\' };
    let parts: Vec<&str> = path.split(separator).collect();
    if parts.len() < 3 {
        return truncate_middle_text(path, max);
    }
    let render = |head: usize, tail: usize| {
        let mut kept: Vec<&str> = parts[..head].to_vec();
        kept.push("…");
        kept.extend_from_slice(&parts[parts.len() - tail..]);
        kept.join(&separator.to_string())
    };
    let (mut head, mut tail) = (1, 1);
    if render(head, tail).chars().count() > max {
        return truncate_middle_text(path, max);
    }
    loop {
        let mut grew = false;
        if head + tail + 1 < parts.len() && render(head, tail + 1).chars().count() <= max {
            tail += 1;
            grew = true;
        }
        if head + tail + 1 < parts.len() && render(head + 1, tail).chars().count() <= max {
            head += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    render(head, tail)
}

/// Cuts a styled line to `max` characters, ending with `…` when cut.
pub(super) fn fit_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if total <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut left = max.saturating_sub(1);
    for span in spans {
        if left == 0 {
            break;
        }
        let count = span.content.chars().count();
        if count <= left {
            left -= count;
            out.push(span);
        } else {
            let cut: String = span.content.chars().take(left).collect();
            out.push(Span::styled(cut, span.style));
            left = 0;
        }
    }
    if max > 0 {
        out.push(Span::raw("…"));
    }
    out
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

impl App {
    /// The dashboard frame: two header rows and a blank row (or the filter
    /// box over all three), the column header and a rule, the list, then a
    /// rule, the status line and the key line.
    pub(super) fn draw_dashboard_frame(&mut self, frame: &mut Frame<'_>) {
        let roles = self.ui_theme.roles;
        let area = frame.area();
        self.last_frame_width = Some(area.width);
        frame.render_widget(
            Block::default().style(Style::default().bg(roles.background).fg(roles.text)),
            area,
        );
        let layout = DashboardFrameLayout::new(area);
        let now = OffsetDateTime::now_utc();
        // While the filter is open or applied its box takes the header rows
        // and the blank row under them; the rows below never move.
        let filter_box = self.dashboard.filter_bar_visible();
        let top_rows = if filter_box {
            frame.render_widget(
                Paragraph::new(self.dashboard_filter_box_lines(area.width)),
                layout.filter_box(),
            );
            layout.filter_box().height
        } else {
            frame.render_widget(
                Paragraph::new(self.dashboard_header_lines(area.width, now)),
                layout.header,
            );
            layout.header.height
        };
        // Docked Details takes the right of the list rows; everything else
        // spans the window as usual.
        let docked = self.details_docked();
        let (list_width, panel_width) = if docked {
            docked_widths(area.width)
        } else {
            (area.width, 0)
        };
        let columns = if docked {
            ListColumns::for_docked(list_width)
        } else {
            ListColumns::for_width(area.width)
        };
        let narrow = |row: Rect| Rect {
            width: list_width,
            ..row
        };
        frame.render_widget(
            Paragraph::new(self.dashboard_column_header_line(columns)),
            narrow(layout.column_header),
        );
        let dim = Style::default().fg(roles.dim);
        let rule = Paragraph::new(Line::from(Span::styled(
            "─".repeat(usize::from(area.width)),
            dim,
        )));
        if docked {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(
                        "{}┼{}",
                        "─".repeat(usize::from(list_width)),
                        "─".repeat(usize::from(panel_width))
                    ),
                    dim,
                ))),
                layout.top_rule,
            );
        } else {
            frame.render_widget(rule.clone(), layout.top_rule);
        }
        self.render_dashboard_list(frame, narrow(layout.list), columns);
        frame.render_widget(rule, layout.bottom_rule);
        if docked {
            // No sheet is open, so the next one starts at the top.
            self.sheet_scroll.set(SheetScroll::default());
            let divider_x = area.x + list_width;
            for row in [layout.column_header, layout.list] {
                for y in row.top()..row.bottom() {
                    frame.render_widget(
                        Paragraph::new(Span::styled("│", dim)),
                        Rect::new(divider_x, y, 1, 1),
                    );
                }
            }
            let panel = |row: Rect| Rect::new(divider_x + 1, row.y, panel_width, row.height);
            self.render_docked_details(frame, panel(layout.column_header), panel(layout.list));
        } else {
            self.render_overlay_host(frame, layout.sheet_region(), top_rows);
        }
        // A question sits on top of everything, sheets and docked Details too.
        self.render_question(frame, layout.sheet_region());
        frame.render_widget(
            Paragraph::new(self.dashboard_status_line(area.width)),
            layout.status,
        );
        frame.render_widget(
            Paragraph::new(self.dashboard_key_line(area.width)),
            layout.keys,
        );
    }

    /// "LEMONUP" in the brand gradient, the logo below 110 columns.
    pub(super) fn short_logo_spans(&self) -> Vec<Span<'static>> {
        let anchors = [
            self.ui_theme.brand_hot,
            self.ui_theme.brand_warm,
            self.ui_theme.brand_gold,
        ];
        let part = |text: &'static str, t: f32| {
            Span::styled(
                text,
                Style::default()
                    .fg(interpolate_palette(&anchors, t))
                    .add_modifier(Modifier::BOLD),
            )
        };
        vec![part("LEM", 0.2), part("ON", 0.5), part("UP", 0.8)]
    }

    /// Header row 1 (logo, client, folder) and row 2 (the answer line).
    pub(super) fn dashboard_header_lines(
        &self,
        width: u16,
        now: OffsetDateTime,
    ) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let wide = width >= WIDE_FRAME_MIN_WIDTH;
        let client = Span::styled(
            self.game_client_label(),
            Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
        );
        let (mut first, mut second) = if wide {
            let mut first = vec![Span::raw(" ")];
            first.extend(self.fruit_gradient_logo_line(LOGO_FULL[0]).spans);
            first.push(Span::raw("    "));
            let mut second = vec![Span::raw(" ")];
            second.extend(self.fruit_gradient_logo_line(LOGO_FULL[1]).spans);
            second.push(Span::raw("    "));
            (first, second)
        } else {
            let mut first = vec![Span::raw(" ")];
            first.extend(self.short_logo_spans());
            first.push(Span::raw("  "));
            (first, vec![Span::raw(" ")])
        };
        first.push(client);
        let mut profile = Vec::new();
        if !self.runtime.is_default_profile() {
            profile.push(Span::styled(
                "   Profile: ",
                Style::default().fg(roles.muted),
            ));
            profile.push(Span::styled(
                self.runtime.profile_name.clone(),
                Style::default()
                    .fg(roles.amber)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let used = spans_width(&first) + spans_width(&profile) + 3;
        let path = self.friendly_addon_dir();
        if !path.is_empty() {
            let room = usize::from(width).saturating_sub(used);
            first.push(Span::raw("  "));
            first.push(Span::styled(
                shorten_path_middle(&path, room),
                Style::default().fg(roles.muted),
            ));
        }
        first.extend(profile);
        second.extend(self.dashboard_answer_spans(now));
        if let Some(warning) = self.profile_warning() {
            second.push(Span::styled(" · ", Style::default().fg(roles.dim)));
            second.push(Span::styled(
                warning,
                Style::default()
                    .fg(roles.amber)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let width = usize::from(width);
        vec![
            Line::from(fit_spans(first, width)),
            Line::from(fit_spans(second, width)),
        ]
    }

    /// Header row 2: `18 addons · 4 updates ready · checked 5 min ago`.
    pub(super) fn dashboard_answer_spans(&self, now: OffsetDateTime) -> Vec<Span<'static>> {
        let roles = self.ui_theme.roles;
        let bold = Modifier::BOLD;
        let dot = || Span::styled(" · ", Style::default().fg(roles.dim));
        let items = &self.dashboard.items;
        if items.is_empty() {
            return match &self.scan_state {
                ScanState::Pending | ScanState::Running(_) => vec![
                    Span::styled(
                        self.shell_ui.motion.spinner_frame(),
                        Style::default().fg(roles.lemon),
                    ),
                    Span::styled(
                        " Looking through your AddOns folder…",
                        Style::default().fg(roles.text),
                    ),
                ],
                ScanState::Failed(_) => vec![
                    Span::styled("!", Style::default().fg(roles.red).add_modifier(bold)),
                    Span::styled(
                        " Couldn't read your AddOns folder",
                        Style::default().fg(roles.red),
                    ),
                ],
                _ => vec![Span::styled(
                    "No addons yet",
                    Style::default().fg(roles.text).add_modifier(bold),
                )],
            };
        }

        let mut spans = vec![Span::styled(
            addon_count_label(items.len()),
            Style::default().fg(roles.text).add_modifier(bold),
        )];
        if let Some(job) = self
            .dashboard
            .job_ui()
            .filter(|job| job.kind == DashboardJobKind::Update)
        {
            spans.push(dot());
            spans.push(Span::styled(
                format!("updating {}", job.total),
                Style::default().fg(roles.lemon).add_modifier(bold),
            ));
            return spans;
        }
        if items.iter().all(|item| item.source == SourceKind::Manual) {
            spans.push(dot());
            spans.push(Span::styled(
                "none linked yet",
                Style::default().fg(roles.muted),
            ));
            return spans;
        }
        let Some(checked_at) = items.iter().filter_map(|item| item.last_checked_at).max() else {
            spans.push(dot());
            spans.push(Span::styled(
                "not checked yet",
                Style::default().fg(roles.muted),
            ));
            return spans;
        };
        let updates = self.dashboard.update_ready_folders().len();
        spans.push(dot());
        spans.push(if updates > 0 {
            Span::styled(
                format!("{updates} update{} ready", plural_suffix(updates)),
                Style::default().fg(roles.amber).add_modifier(bold),
            )
        } else {
            Span::styled(
                "all up to date",
                Style::default().fg(roles.green).add_modifier(bold),
            )
        });
        spans.push(dot());
        spans.push(Span::styled(
            checked_ago_label(now, checked_at),
            Style::default().fg(roles.muted),
        ));
        spans
    }

    /// Rows 1 to 3 while the filter is open or applied: a box with the
    /// query, the match count and the keys that act on it.
    ///
    /// ```text
    /// ╭─ Filter ─────────────────────────────────── 3 of 33 shown ─╮
    /// │  / weak▌                            enter keep · esc clear  │
    /// ╰─────────────────────────────────────────────────────────────╯
    /// ```
    ///
    /// The query starts in the column of the addon names below. When room
    /// runs out the hint goes first, then the count loses "shown", then the
    /// query is cut from the left so its end (and the cursor) stays visible.
    pub(super) fn dashboard_filter_box_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let width = usize::from(width);
        if width < 2 {
            return vec![Line::default(); 3];
        }
        let editing = self.dashboard.filter_editing();
        let border = Style::default().fg(if editing { roles.lemon } else { roles.border });
        let muted = Style::default().fg(roles.muted);
        let key = Style::default().fg(roles.blue).add_modifier(Modifier::BOLD);
        // Between the corners.
        let inner = width - 2;

        // ╭─ Filter ───── 3 of 33 shown ─╮
        let title = "─ Filter ";
        let title_style = if editing {
            Style::default().fg(roles.text).add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        let shown = self.dashboard.visible_parent_count();
        let total = self.dashboard.items.len();
        let count = [
            format!(" {shown} of {total} shown "),
            format!(" {shown} of {total} "),
        ]
        .into_iter()
        // A dash on each side of the count.
        .find(|count| title.chars().count() + 1 + count.chars().count() < inner);
        let mut top = vec![Span::styled("╭", border)];
        if title.chars().count() + 1 > inner {
            top.push(Span::styled("─".repeat(inner), border));
        } else {
            top.push(Span::styled("─ ", border));
            top.push(Span::styled("Filter", title_style));
            top.push(Span::styled(" ", border));
            let count_width = count.as_ref().map_or(0, |count| count.chars().count() + 1);
            top.push(Span::styled(
                "─".repeat(inner - title.chars().count() - count_width),
                border,
            ));
            if let Some(count) = count {
                top.push(Span::styled(count, muted));
                top.push(Span::styled("─", border));
            }
        }
        top.push(Span::styled("╮", border));

        // │  / query▌                 enter keep · esc clear  │
        let gutter = usize::from(GUTTER_WIDTH);
        let mut row = vec![
            Span::styled("│", border),
            Span::raw(" ".repeat(gutter.saturating_sub(3))),
            Span::styled(
                "/",
                if editing {
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD)
                } else {
                    muted
                },
            ),
            Span::raw(" "),
        ];
        let hint = |keys: &[(&'static str, &'static str)]| {
            let mut spans = Vec::new();
            for (index, (name, label)) in keys.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::styled(" · ", Style::default().fg(roles.dim)));
                }
                spans.push(Span::styled(*name, key));
                spans.push(Span::styled(*label, muted));
            }
            spans
        };
        let hint = if editing {
            hint(&[("enter", " keep"), ("esc", " clear")])
        } else if self.details_docked() {
            // esc closes docked Details first, so it does not offer to clear.
            hint(&[("/", " edit")])
        } else {
            hint(&[("/", " edit"), ("esc", " clear")])
        };
        // The field runs from the query column to one space before the
        // right border; the hint needs a gap before it and ends two columns
        // in, under the count's last character.
        let field_room = inner.saturating_sub(gutter.saturating_sub(1) + 1);
        let hint_room = spans_width(&hint) + 3;
        let query = self.dashboard.filter_input();
        let caret = Style::default().fg(roles.lemon);
        let cursor = || Span::styled("▌", caret);
        let (field, show_hint) = if editing && query.is_empty() {
            // Says what the filter looks at; see `dashboard_item_filter_match`.
            let (placeholder, show_hint) = [
                ("part of a name, folder or author", true),
                ("name, folder or author", true),
                ("part of a name, folder or author", false),
                ("name, folder or author", false),
            ]
            .into_iter()
            .find(|(text, with_hint)| {
                let room = if *with_hint {
                    field_room.saturating_sub(hint_room)
                } else {
                    field_room
                };
                // The cursor sits before the placeholder.
                text.chars().count() < room
            })
            .unwrap_or(("name, folder or author", false));
            let room = field_room.saturating_sub(1);
            (
                vec![
                    cursor(),
                    Span::styled(truncate_text(placeholder, room), muted),
                ],
                show_hint,
            )
        } else {
            let needed = query.width() + usize::from(editing);
            let show_hint = needed + hint_room <= field_room;
            let style = if editing {
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(roles.text)
            };
            // The shared text box: cut with "…" so the cursor stays in view,
            // from the left when the cursor is at the end.
            (query.spans(field_room, style, caret, editing), show_hint)
        };
        row.extend(field);
        let used = spans_width(&row);
        let hint = if show_hint { hint } else { Vec::new() };
        let tail = if hint.is_empty() { 1 } else { 2 };
        let end = width.saturating_sub(1 + tail + spans_width(&hint));
        if end > used {
            row.push(Span::raw(" ".repeat(end - used)));
        }
        row.extend(hint);
        let mut row = fit_spans(row, width.saturating_sub(1 + tail));
        let used = spans_width(&row);
        row.push(Span::raw(" ".repeat(width.saturating_sub(1 + used))));
        row.push(Span::styled("│", border));

        let bottom = vec![Span::styled(format!("╰{}╯", "─".repeat(inner)), border)];
        vec![Line::from(top), Line::from(row), Line::from(bottom)]
    }

    pub(super) fn dashboard_column_header_line(&self, columns: ListColumns) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let sort = self.dashboard.sort_config();
        let cell = |label: &'static str, sort_column: Option<DashboardSortColumn>, width: u16| {
            let mut spans = if sort_column == Some(sort.column) {
                vec![
                    Span::styled(
                        label,
                        Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(sort.direction.indicator(), Style::default().fg(roles.lemon)),
                ]
            } else {
                vec![Span::styled(
                    label,
                    Style::default()
                        .fg(roles.muted)
                        .add_modifier(Modifier::BOLD),
                )]
            };
            let used = spans_width(&spans);
            if width > 0 {
                spans.push(Span::raw(
                    " ".repeat(usize::from(width).saturating_sub(used)),
                ));
            }
            spans
        };
        let mut spans = vec![Span::raw(" ".repeat(usize::from(GUTTER_WIDTH)))];
        spans.extend(cell("Addon", Some(DashboardSortColumn::Name), columns.name));
        spans.extend(cell(
            "Status",
            Some(DashboardSortColumn::Status),
            columns.status,
        ));
        spans.extend(cell("Version", None, columns.version));
        if columns.author > 0 {
            spans.extend(cell(
                "Author",
                Some(DashboardSortColumn::Author),
                columns.author,
            ));
        }
        if columns.source > 0 {
            spans.extend(cell("Source", Some(DashboardSortColumn::Source), 0));
        }
        Line::from(spans)
    }

    pub(super) fn render_dashboard_list(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        columns: ListColumns,
    ) {
        let roles = self.ui_theme.roles;
        if self.dashboard.items.is_empty() || self.dashboard.rows.is_empty() {
            self.last_dashboard_list_area = Some(area);
            frame.render_widget(Paragraph::new(self.dashboard_empty_lines(area.width)), area);
            return;
        }

        let mut area = area;
        if self.first_run_card_visible() {
            let card = self.first_run_card_lines(area.width.saturating_sub(4 + 2 + 4));
            let height = u16::try_from(card.len()).unwrap_or(u16::MAX) + 2;
            // The card only shows when it leaves room for a few addons.
            if area.width >= 30 && area.height >= height + 3 {
                let card_area = Rect::new(area.x + 2, area.y, area.width - 4, height);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(roles.lemon))
                    .title(Line::from(vec![
                        Span::styled("─ ", Style::default().fg(roles.lemon)),
                        Span::styled(
                            "Welcome. One thing before LemonUp can update your addons",
                            Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                    ]));
                let inner = block.inner(card_area).inner(Margin::new(2, 0));
                frame.render_widget(block, card_area);
                frame.render_widget(Paragraph::new(card), inner);
                area = Rect::new(area.x, area.y + height, area.width, area.height - height);
            }
        }
        self.last_dashboard_list_area = Some(area);

        let cursor = self.dashboard.list_state.selected();
        let rows = self
            .dashboard
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.dashboard_list_row(row, cursor == Some(index), columns))
            .collect::<Vec<_>>();
        let table = Table::new(rows, columns.constraints())
            .column_spacing(0)
            .row_highlight_style(Style::default().bg(roles.cursor_row));
        frame.render_stateful_widget(table, area, &mut self.dashboard.list_state);
    }

    /// The one time card while nothing is linked: every addon is "Not
    /// linked" and the card has not been hidden with `d`. Docked Details
    /// and a filter hide it for the moment.
    pub(super) fn first_run_card_visible(&self) -> bool {
        self.shell_mode == ShellMode::Dashboard
            && !self.config_pane.persisted.hide_first_run_card
            && !self.dashboard.items.is_empty()
            && self
                .dashboard
                .items
                .iter()
                .all(|item| item.source == SourceKind::Manual)
            && !self.dashboard.has_filter()
            && !self.dashboard.filter_editing()
            && !self.details_docked()
    }

    /// What the welcome card says, wrapped at `width`, with its keys at the
    /// end of the last row when they fit.
    fn first_run_card_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let text = Style::default().fg(roles.text);
        let muted = Style::default().fg(roles.muted);
        let key = |label: &'static str| {
            Span::styled(
                label,
                Style::default()
                    .fg(roles.lemon)
                    .add_modifier(Modifier::BOLD),
            )
        };
        let mut lines = wrap_lines(
            &[
                Line::from(Span::styled(
                    format!(
                        "LemonUp found {} in your AddOns folder, but it only updates addons it knows the source of.",
                        addon_count_label(self.dashboard.items.len())
                    ),
                    text,
                )),
                Line::from(vec![
                    Span::styled("Pick an addon and press ", text),
                    key("f"),
                    Span::styled(
                        " to find it online. Do that once per addon and LemonUp keeps it updated.",
                        text,
                    ),
                ]),
                Line::from(Span::styled(
                    "Your addon settings are never touched.",
                    muted,
                )),
            ],
            width,
        );
        let keys = vec![
            key("f"),
            Span::styled(" find online", muted),
            Span::raw("    "),
            Span::styled(
                "d",
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" hide this", muted),
        ];
        let keys_width: usize = keys.iter().map(Span::width).sum();
        let last = lines.last().map_or(0, Line::width);
        if last + 4 + keys_width <= usize::from(width) {
            let pad = usize::from(width) - last - keys_width;
            if let Some(line) = lines.last_mut() {
                line.spans.push(Span::raw(" ".repeat(pad)));
                line.spans.extend(keys);
            }
        } else {
            lines.push(Line::from(keys));
        }
        lines
    }

    /// The list body when there is nothing to list: still looking, the
    /// folder can't be read, it is empty, or the filter matches nothing.
    fn dashboard_empty_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let indent = || Span::raw("     ");
        let key = |label: &'static str, primary: bool| {
            Span::styled(
                label,
                Style::default()
                    .fg(if primary { roles.lemon } else { roles.blue })
                    .add_modifier(Modifier::BOLD),
            )
        };
        let text = Style::default().fg(roles.text);
        let bold = text.add_modifier(Modifier::BOLD);
        let mut lines = vec![Line::default()];
        if !self.dashboard.items.is_empty() {
            lines.push(Line::from(vec![
                indent(),
                Span::styled(
                    format!(
                        "Nothing you have installed matches \"{}\".",
                        truncate_text(self.dashboard.filter_query(), 40)
                    ),
                    text,
                ),
            ]));
            lines.push(Line::default());
            lines.push(Line::from(vec![
                indent(),
                key("esc", false),
                Span::styled("  clear the filter", Style::default().fg(roles.muted)),
            ]));
            return lines;
        }
        match &self.scan_state {
            ScanState::Failed(error) => {
                lines.push(Line::from(vec![
                    indent(),
                    Span::styled(
                        "! ",
                        Style::default().fg(roles.red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("LemonUp couldn't read your AddOns folder.", bold),
                ]));
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    indent(),
                    Span::styled(
                        "The folder may have moved, or the drive it is on isn't connected.",
                        text,
                    ),
                ]));
                let path = self.friendly_addon_dir();
                if !path.is_empty() {
                    lines.push(Line::from(vec![
                        indent(),
                        Span::styled(
                            shorten_path_middle(&path, usize::from(width).saturating_sub(6)),
                            Style::default().fg(roles.muted),
                        ),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    indent(),
                    key("s", true),
                    Span::styled("  choose the game folder again", text),
                ]));
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    indent(),
                    Span::styled(
                        format!("Technical detail: {error}"),
                        Style::default().fg(roles.dim),
                    ),
                ]));
            }
            ScanState::Pending | ScanState::Running(_) => {
                lines.push(Line::from(vec![
                    indent(),
                    Span::styled(
                        "Looking through your AddOns folder…",
                        Style::default().fg(roles.muted),
                    ),
                ]));
            }
            _ => {
                lines.push(Line::from(vec![
                    indent(),
                    Span::styled("Your AddOns folder is empty.", bold),
                ]));
                lines.push(Line::default());
                let install = vec![
                    key("i", true),
                    Span::styled("  install your first addon", text),
                ];
                let settings = vec![
                    key("s", false),
                    Span::styled(
                        "  check that this is the right game folder",
                        Style::default().fg(roles.muted),
                    ),
                ];
                let one_row: usize = 5
                    + install.iter().map(Span::width).sum::<usize>()
                    + 10
                    + settings.iter().map(Span::width).sum::<usize>();
                if one_row <= usize::from(width) {
                    let mut spans = vec![indent()];
                    spans.extend(install);
                    spans.push(Span::raw(" ".repeat(10)));
                    spans.extend(settings);
                    lines.push(Line::from(spans));
                } else {
                    let mut first = vec![indent()];
                    first.extend(install);
                    lines.push(Line::from(first));
                    let mut second = vec![indent()];
                    second.extend(settings);
                    lines.push(Line::from(second));
                }
            }
        }
        lines
    }

    /// Where a row stands right now, running jobs included.
    pub(super) fn dashboard_row_status(&self, item: &DashboardItem) -> RowStatus {
        let state = self
            .dashboard
            .update_job_row_state(&item.folder)
            .or_else(|| self.dashboard.update_result(&item.folder));
        if let Some(state) = state {
            return match state {
                UpdateJobRowState::Waiting => RowStatus::Waiting,
                UpdateJobRowState::Updating => RowStatus::Updating,
                UpdateJobRowState::Done => RowStatus::Done,
                UpdateJobRowState::Updated => RowStatus::Updated,
                UpdateJobRowState::Failed => RowStatus::UpdateFailed,
            };
        }
        if self.dashboard.job_ui().is_some_and(|job| {
            job.kind == DashboardJobKind::Check && job.current_addon == item.folder
        }) {
            return RowStatus::Checking;
        }
        dashboard_item_row_status(item)
    }

    fn dashboard_list_row(
        &self,
        row: &DashboardRow,
        is_cursor: bool,
        columns: ListColumns,
    ) -> Row<'static> {
        let roles = self.ui_theme.roles;
        let bar = if is_cursor {
            Span::styled("▌", Style::default().fg(roles.lemon))
        } else {
            Span::raw(" ")
        };
        let name_width = usize::from(columns.name.saturating_sub(1));

        if matches!(row.kind, DashboardRowKind::OwnedChild { .. }) {
            let gutter = Line::from(vec![bar, Span::raw("    ")]);
            let name = Line::from(vec![
                Span::raw("  "),
                Span::styled(child_row_prefix(row), Style::default().fg(roles.dim)),
                Span::styled(
                    truncate_text(&row.name, name_width.saturating_sub(4)),
                    Style::default().fg(roles.muted),
                ),
            ]);
            return Row::new(vec![Cell::from(gutter), Cell::from(name)]);
        }

        let selected = self.dashboard.is_parent_selected(&row.folder);
        let item = self
            .dashboard
            .items
            .iter()
            .find(|item| item.folder == row.folder);
        let dot = if selected {
            Span::styled("●", Style::default().fg(roles.lemon))
        } else {
            Span::raw(" ")
        };
        let triangle = match (row.expandable, row.expanded) {
            (true, true) => Span::styled("▾", Style::default().fg(roles.muted)),
            (true, false) => Span::styled("▸", Style::default().fg(roles.muted)),
            (false, _) => Span::raw(" "),
        };
        let gutter = Line::from(vec![bar, dot, Span::raw(" "), triangle, Span::raw(" ")]);
        let name_style = if is_cursor {
            Style::default().fg(roles.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(roles.text)
        };
        let mut cells = vec![
            Cell::from(gutter),
            Cell::from(Line::from(Span::styled(
                truncate_text(&row.name, name_width),
                name_style,
            ))),
        ];
        if let Some(item) = item {
            let spinner = self.shell_ui.motion.spinner_frame();
            cells.push(Cell::from(Line::from(
                self.dashboard_row_status(item).spans(spinner, roles),
            )));
            cells.push(Cell::from(Line::from(
                VersionCell::for_item(item, columns.wide)
                    .spans(usize::from(columns.version.saturating_sub(1)), roles),
            )));
            if columns.author > 0 {
                let author = item.author.as_deref().map(str::trim).unwrap_or_default();
                cells.push(Cell::from(Span::styled(
                    truncate_text(author, usize::from(columns.author.saturating_sub(1))),
                    Style::default().fg(roles.muted),
                )));
            }
            if columns.source > 0 {
                cells.push(Cell::from(Span::styled(
                    dashboard_source_name(item.source, columns.wide),
                    Style::default().fg(roles.muted),
                )));
            }
        }
        let style = if selected {
            Style::default().bg(roles.selected_row)
        } else {
            Style::default()
        };
        Row::new(cells).style(style)
    }

    /// "2 of 18": the cursor's addon among the addons shown.
    pub(super) fn dashboard_position_label(&self) -> Option<String> {
        let rows = &self.dashboard.rows;
        let last = rows.len().checked_sub(1)?;
        let selected = self.dashboard.list_state.selected()?.min(last);
        let position = rows[..=selected]
            .iter()
            .filter(|row| matches!(row.kind, DashboardRowKind::Parent))
            .count();
        let total = self.dashboard.visible_parent_count();
        (total > 0).then(|| format!("{} of {total}", position.max(1)))
    }

    /// The status line: the running job or the last message on the left,
    /// the cursor position on the right.
    pub(super) fn dashboard_status_line(&self, width: u16) -> Line<'static> {
        let width = usize::from(width);
        let left = self
            .dashboard_job_status_spans(width)
            .or_else(|| self.status_flash_spans())
            .unwrap_or_default();
        // Details moves through the addons too, so it keeps the position.
        let show_position = matches!(
            self.shell_ui.overlay.active,
            None | Some(OverlayKind::Inspect)
        ) && !self.dashboard.filter_editing()
            && self.dashboard.job_ui().is_none();
        let right = show_position
            .then(|| self.dashboard_position_label())
            .flatten()
            .unwrap_or_default();
        let right_width = right.chars().count();
        let room = width.saturating_sub(right_width + 3);
        let mut spans = vec![Span::raw(" ")];
        spans.extend(fit_spans(left, room));
        if !right.is_empty() {
            let used = spans_width(&spans);
            spans.push(Span::raw(
                " ".repeat(width.saturating_sub(used + right_width + 1)),
            ));
            spans.push(Span::styled(
                right,
                Style::default().fg(self.ui_theme.roles.muted),
            ));
        }
        Line::from(spans)
    }

    /// `⠋ Updating 2 of 4 · Deadly Boss Mods   ██████░░░░`.
    fn dashboard_job_status_spans(&self, width: usize) -> Option<Vec<Span<'static>>> {
        let roles = self.ui_theme.roles;
        let job = self.dashboard.job_ui()?;
        let name = self
            .dashboard
            .items
            .iter()
            .find(|item| item.folder == job.current_addon)
            .map_or_else(|| job.current_addon.clone(), |item| item.name.clone());
        let mut spans = vec![
            Span::styled(
                self.shell_ui.motion.spinner_frame(),
                Style::default().fg(roles.lemon),
            ),
            Span::styled(
                format!(
                    " {} {} of {}",
                    job.kind.label(),
                    job.current_index,
                    job.total
                ),
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(roles.dim)),
            Span::styled(truncate_text(&name, 40), Style::default().fg(roles.text)),
        ];
        if width >= usize::from(WIDE_FRAME_MIN_WIDTH) && job.total > 0 {
            let filled = (job.current_index.min(job.total) * JOB_BAR_WIDTH) / job.total;
            spans.push(Span::raw("   "));
            spans.push(Span::styled(
                "█".repeat(filled),
                Style::default().fg(roles.lemon),
            ));
            spans.push(Span::styled(
                "░".repeat(JOB_BAR_WIDTH - filled),
                Style::default().fg(roles.dim),
            ));
        }
        Some(spans)
    }

    /// The current message with its symbol, and the key it offers while
    /// that key still works.
    fn status_flash_spans(&self) -> Option<Vec<Span<'static>>> {
        let roles = self.ui_theme.roles;
        let flash = self.shell_ui.status_flash.as_ref()?;
        let text = sentence_case(&flash.text);
        let mut spans = match flash.kind {
            StatusKind::Done => vec![
                Span::styled("✓ ", Style::default().fg(roles.green)),
                Span::styled(text, Style::default().fg(roles.text)),
            ],
            StatusKind::Problem => vec![
                Span::styled(
                    "! ",
                    Style::default().fg(roles.red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(text, Style::default().fg(roles.text)),
            ],
            StatusKind::Working => vec![
                Span::styled(
                    format!("{} ", self.shell_ui.motion.spinner_frame()),
                    Style::default().fg(roles.lemon),
                ),
                Span::styled(text, Style::default().fg(roles.text)),
            ],
            StatusKind::Note | StatusKind::Quiet => {
                vec![Span::styled(
                    format!("· {text}"),
                    Style::default().fg(roles.muted),
                )]
            }
        };
        if let Some(offer) = flash.offer.filter(|offer| self.status_offer_works(*offer)) {
            let dot = || Span::styled(" · ", Style::default().fg(roles.dim));
            let key = |key: &'static str, color: Color| {
                Span::styled(key, Style::default().fg(color).add_modifier(Modifier::BOLD))
            };
            match offer {
                StatusOffer::UndoDelete => spans.extend([
                    dot(),
                    key("z", roles.blue),
                    Span::styled(" undo delete", Style::default().fg(roles.muted)),
                ]),
                StatusOffer::UpdateAll => {
                    let updates = self.dashboard.update_ready_folders().len();
                    spans.extend([
                        dot(),
                        Span::styled(
                            format!("{updates} update{} ready", plural_suffix(updates)),
                            Style::default()
                                .fg(roles.amber)
                                .add_modifier(Modifier::BOLD),
                        ),
                        dot(),
                        key("U", roles.lemon),
                        Span::styled(" updates them all", Style::default().fg(roles.muted)),
                    ]);
                }
            }
        }
        Some(spans)
    }

    /// The key line, dropping hints from the end rather than wrapping.
    pub(super) fn dashboard_key_line(&self, width: u16) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let (hints, separator) =
            fit_key_line(self.key_table(width), width, width >= WIDE_FRAME_MIN_WIDTH);
        let mut spans = vec![Span::raw(" ")];
        for (index, hint) in hints.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(separator));
            }
            if hint.key.is_empty() {
                spans.push(Span::styled(
                    hint.label.clone(),
                    Style::default().fg(roles.muted),
                ));
                continue;
            }
            let pulsed = hint.id.is_some_and(|id| {
                self.shell_ui
                    .footer_pulse
                    .is_some_and(|pulse| pulse.hint == id)
            });
            let (key_style, label_style) = if pulsed {
                (
                    Style::default()
                        .fg(roles.background)
                        .bg(roles.lemon)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                )
            } else if hint.primary {
                (
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
                    Style::default().fg(roles.muted),
                )
            };
            spans.push(Span::styled(hint.key, key_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(hint.label.clone(), label_style));
        }
        Line::from(fit_spans(spans, usize::from(width)))
    }
}

/// Messages read as sentences: the first letter is a capital.
fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
    // Updates first; "Not checked yet" sorts with "Up to date", above
    // "Not linked".
    let priority = match dashboard_item_row_status(item) {
        RowStatus::UpdateReady => 0,
        RowStatus::UpToDate | RowStatus::NotChecked => 1,
        RowStatus::NewerThanSource => 2,
        RowStatus::NotLinked => 3,
        _ => 4,
    };

    (
        priority,
        std::cmp::Reverse(item.updated_at),
        std::cmp::Reverse(item.installed_at),
        dashboard_item_version_label(item),
        dashboard_item_remote_label(item).unwrap_or_default(),
    )
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
        return "Not linked".to_string();
    }
    match dashboard_item_update_status(item) {
        UpdateStatus::UpToDate => "Up to date".to_string(),
        UpdateStatus::UpdateAvailable => dashboard_item_remote_label(item)
            .map(|version| format!("Update ready: {version}"))
            .unwrap_or_else(|| "Update ready".to_string()),
        UpdateStatus::RemoteOlder => "Newer than source".to_string(),
        UpdateStatus::Unknown => "Not checked yet".to_string(),
        UpdateStatus::Error => "Check failed".to_string(),
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
        Some(DashboardChildConnector::Mid) => "├ ",
        Some(DashboardChildConnector::Last) => "└ ",
        None => "└ ",
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
