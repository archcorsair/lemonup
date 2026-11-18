//! Details of one addon: the status first, a sentence when the status needs
//! explaining, then one list of facts. Below 120 columns it is a sheet over
//! the list; at 120 columns and wider it docks on the right of the list.

use super::render_dashboard::{
    RowStatus, dashboard_item_installed_version, dashboard_item_remote_label, dashboard_source_name,
};
use super::sheet::{SheetBody, SheetRoom, SheetScroll, text_height, wrap_lines};
use super::*;

/// At this width and wider, Details docks beside the list.
pub(super) const DETAILS_DOCK_MIN_WIDTH: u16 = 120;
/// The docked panel at 120 columns; it grows with the window up to the max.
const PANEL_MIN_WIDTH: u16 = 41;
const PANEL_MAX_WIDTH: u16 = 52;
/// Label column of the sheet ("Last checked    ") and of the docked panel.
const SHEET_LABEL_WIDTH: usize = 16;
const PANEL_LABEL_WIDTH: usize = 14;
/// The status word column of the sheet's first row.
const SHEET_STATUS_WIDTH: usize = 20;

/// The list and panel widths for a docked window `width` columns wide. The
/// divider takes the one column between them. At 120 columns: 78 and 41.
/// Wider windows share the extra columns until the panel reaches 52.
pub(super) fn docked_widths(width: u16) -> (u16, u16) {
    let panel = (PANEL_MIN_WIDTH + width.saturating_sub(DETAILS_DOCK_MIN_WIDTH) / 2)
        .min(PANEL_MAX_WIDTH)
        .min(width.saturating_sub(1));
    (width.saturating_sub(panel + 1), panel)
}

/// "WoW 12.0.5" for interface 120005: major n/10000, minor (n/100)%100,
/// patch n%100. Several interfaces are joined in the TOC's order, as
/// "WoW 11.2.7, 1.15.7"; `None` when none parses.
pub(super) fn wow_versions_for_interface(interface: &str) -> Option<String> {
    let mut versions: Vec<String> = Vec::new();
    for part in interface.split(',') {
        let Ok(number) = part.trim().parse::<u32>() else {
            continue;
        };
        if number == 0 {
            continue;
        }
        let version = format!(
            "{}.{}.{}",
            number / 10000,
            (number / 100) % 100,
            number % 100
        );
        if !versions.contains(&version) {
            versions.push(version);
        }
    }
    (!versions.is_empty()).then(|| format!("WoW {}", versions.join(", ")))
}

/// "5 minutes ago", for Last checked.
pub(super) fn time_ago(now: OffsetDateTime, at: OffsetDateTime) -> String {
    let seconds = (now - at).whole_seconds().max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let unit =
        |count: i64, word: &str| format!("{count} {word}{} ago", if count == 1 { "" } else { "s" });
    if minutes < 1 {
        "just now".to_string()
    } else if hours < 1 {
        unit(minutes, "minute")
    } else if days < 1 {
        unit(hours, "hour")
    } else {
        unit(days, "day")
    }
}

/// A link without its scheme or `www.`: `addons.wago.io/addons/dbm`.
fn web_page_label(url: &str) -> String {
    let url = url.trim();
    let url = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    url.strip_prefix("www.")
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

/// Rows of at most `width` characters, broken after a `/` where possible.
fn wrap_link(link: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut rest: Vec<char> = link.chars().collect();
    while rest.len() > width {
        let cut = rest[..width]
            .iter()
            .rposition(|character| *character == '/')
            .map_or(width, |index| index + 1);
        rows.push(rest[..cut].iter().collect());
        rest.drain(..cut);
    }
    if !rest.is_empty() || rows.is_empty() {
        rows.push(rest.into_iter().collect());
    }
    rows
}

/// One fact in Details.
enum DetailValue {
    Text(String),
    /// One name per row when docked, a comma list in the sheet.
    Folders(Vec<String>),
    Link(String),
}

/// What Details says about one addon, before it is laid out.
struct DetailsContent {
    /// Symbol and status word.
    status: Vec<Span<'static>>,
    /// `12.0.8 → 12.0.10`, or the installed version alone.
    versions: Vec<Span<'static>>,
    /// Empty unless the status needs explaining.
    sentence: Vec<Span<'static>>,
    /// Lines after the sentence, each its own row: what to do about it.
    follow_up: Vec<Vec<Span<'static>>>,
    rows: Vec<(&'static str, DetailValue)>,
}

impl App {
    /// True while Details is open in a window wide enough to dock it.
    pub(super) fn details_docked(&self) -> bool {
        self.shell_ui.overlay.active == Some(OverlayKind::Inspect)
            && self
                .last_frame_width
                .is_some_and(|width| width >= DETAILS_DOCK_MIN_WIDTH)
    }

    /// Details as a sheet: the status row, the sentence, then the facts with
    /// their labels in one column. The body scrolls, so nothing is dropped.
    ///
    /// The sheet is as tall as the tallest addon's Details, so moving to
    /// another addon never moves or resizes it.
    pub(super) fn inspect_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let width = room.width;
        let mut body = SheetBody::new();
        match self.details_content() {
            Some(content) => body.text(self.details_sheet_lines(&content, width)),
            None => body.text(self.details_empty_lines()),
        };
        body.reserve(
            room.reserved()
                .unwrap_or_else(|| self.details_sheet_rows(width)),
        );
        body
    }

    /// Rows the tallest Details of all the addons in the list takes at
    /// `width`.
    fn details_sheet_rows(&self, width: u16) -> u16 {
        self.dashboard
            .items
            .iter()
            .map(|item| {
                let lines = self.details_sheet_lines(&self.details_content_for(item), width);
                text_height(&lines, width)
            })
            .max()
            .unwrap_or(1)
    }

    fn details_empty_lines(&self) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        vec![Line::from(Span::styled(
            "No addon selected",
            Style::default().fg(roles.muted),
        ))]
    }

    fn details_content(&self) -> Option<DetailsContent> {
        let target = self.inspect_resolved_target()?;
        Some(self.details_content_for(target.item))
    }

    fn details_content_for(&self, item: &DashboardItem) -> DetailsContent {
        let roles = self.ui_theme.roles;
        let text = Style::default().fg(roles.text);
        let muted = Style::default().fg(roles.muted);
        let status = self.dashboard_row_status(item);
        let spinner = self.spinner_frame();
        let status_spans = match status {
            RowStatus::UpToDate | RowStatus::NewerThanSource => vec![
                Span::styled(
                    status.symbol(spinner, self.ui_theme.symbols.done),
                    Style::default().fg(roles.green),
                ),
                Span::styled(
                    format!(" {}", status.word()),
                    text.add_modifier(Modifier::BOLD),
                ),
            ],
            other => other.spans(spinner, &self.ui_theme),
        };

        let installed = dashboard_item_installed_version(item);
        let remote = dashboard_item_remote_label(item);
        let linked = item.source != SourceKind::Manual;
        let source_name = dashboard_source_name(item.source, true);
        let versions = if status == RowStatus::UpdateReady {
            vec![
                Span::styled(
                    format!("{installed} {}", self.ui_theme.symbols.arrow),
                    muted,
                ),
                Span::raw(" "),
                Span::styled(
                    remote.clone().unwrap_or_else(|| "newer".to_string()),
                    Style::default()
                        .fg(roles.amber)
                        .add_modifier(Modifier::BOLD),
                ),
            ]
        } else {
            vec![Span::styled(installed.clone(), muted)]
        };

        let key = |label: &'static str| {
            Span::styled(
                label,
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            )
        };
        let sentence = match status {
            RowStatus::NewerThanSource => vec![Span::styled(
                format!(
                    "You have {installed}. {source_name} lists {}, which is older.",
                    remote.as_deref().unwrap_or("an older version")
                ),
                text,
            )],
            RowStatus::NotLinked => vec![Span::styled(
                "LemonUp found this addon in your AddOns folder but doesn't know where it came from, so it can't check it for updates.",
                text,
            )],
            RowStatus::CheckFailed => check_failed_sentence(
                self.dashboard
                    .check_failure(&item.folder)
                    .map(|failure| failure.kind),
                item.source,
            )
            .into_iter()
            .map(|(part, is_key)| {
                if is_key {
                    key("c")
                } else {
                    Span::styled(part, text)
                }
            })
            .collect(),
            _ => Vec::new(),
        };

        let follow_up = if status == RowStatus::NotLinked {
            vec![vec![
                Span::styled("Press ", text),
                key("f"),
                Span::styled(" to find it online.", text),
            ]]
        } else {
            Vec::new()
        };

        let mut rows = Vec::new();
        if linked {
            rows.push(("From", DetailValue::Text(source_name.to_string())));
        }
        if let Some(author) = item.author.as_deref().map(str::trim)
            && !author.is_empty()
            && !author.eq_ignore_ascii_case("unknown")
        {
            rows.push(("Author", DetailValue::Text(author.to_string())));
        }
        if linked {
            rows.push(("Installed", DetailValue::Text(installed.clone())));
            if let Some(checked_at) = item.last_checked_at {
                rows.push((
                    "Last checked",
                    DetailValue::Text(time_ago(OffsetDateTime::now_utc(), checked_at)),
                ));
            }
        }
        if let Some(made_for) = item
            .interface
            .as_deref()
            .and_then(wow_versions_for_interface)
        {
            rows.push(("Made for", DetailValue::Text(made_for)));
        }
        if let Some(category) = self.details_category(item) {
            rows.push(("Category", DetailValue::Text(category)));
        }
        let missing = self.dashboard_parent_missing_owned_children(&item.folder);
        let mut folders = vec![item.folder.clone()];
        for folder in &item.owned_folders {
            if !folders.contains(folder) {
                folders.push(folder.clone());
            }
        }
        let folders = folders
            .into_iter()
            .map(|folder| {
                if missing.contains(&folder) {
                    format!("{folder} (missing)")
                } else {
                    folder
                }
            })
            .collect();
        rows.push(("Folders", DetailValue::Folders(folders)));
        let mut needs: Vec<String> = Vec::new();
        for dependency in &item.required_deps {
            if !needs.contains(dependency) {
                needs.push(dependency.clone());
            }
        }
        rows.push((
            "Needs",
            DetailValue::Text(if needs.is_empty() {
                "nothing else".to_string()
            } else {
                needs.join(", ")
            }),
        ));
        if let Some(url) = item.source_url.as_deref().map(str::trim)
            && !url.is_empty()
        {
            rows.push(("Web page", DetailValue::Link(web_page_label(url))));
        }

        DetailsContent {
            status: status_spans,
            versions,
            sentence,
            follow_up,
            rows,
        }
    }

    fn details_sheet_lines(&self, content: &DetailsContent, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let width = usize::from(width);
        let mut first = content.status.clone();
        let status_width = spans_text_width(&content.status);
        first.push(Span::raw(
            " ".repeat(SHEET_STATUS_WIDTH.saturating_sub(status_width).max(2)),
        ));
        first.extend(content.versions.clone());
        let mut lines = vec![Line::from(first), Line::default()];
        if !content.sentence.is_empty() {
            lines.push(Line::from(content.sentence.clone()));
            lines.push(Line::default());
        }
        if !content.follow_up.is_empty() {
            lines.extend(content.follow_up.iter().cloned().map(Line::from));
            lines.push(Line::default());
        }
        let value_width = width.saturating_sub(SHEET_LABEL_WIDTH).max(8);
        for (label, value) in &content.rows {
            let rows = match value {
                DetailValue::Text(text) => self.detail_value_rows(text, value_width),
                DetailValue::Folders(folders) => {
                    self.detail_value_rows(&folders.join(", "), value_width)
                }
                DetailValue::Link(link) => wrap_link(link, value_width)
                    .into_iter()
                    .map(|row| Line::from(Span::styled(row, Style::default().fg(roles.blue))))
                    .collect(),
            };
            for (index, row) in rows.into_iter().enumerate() {
                let prefix = if index == 0 {
                    Span::styled(
                        format!("{label:<SHEET_LABEL_WIDTH$}"),
                        Style::default().fg(roles.muted),
                    )
                } else {
                    Span::raw(" ".repeat(SHEET_LABEL_WIDTH))
                };
                let mut spans = vec![prefix];
                spans.extend(row.spans);
                lines.push(Line::from(spans));
            }
        }
        lines
    }

    /// A value word wrapped at `width`.
    fn detail_value_rows(&self, value: &str, width: usize) -> Vec<Line<'static>> {
        let style = Style::default().fg(self.ui_theme.roles.text);
        wrap_lines(
            &[Line::from(Span::styled(value.to_string(), style))],
            u16::try_from(width).unwrap_or(u16::MAX),
        )
    }

    /// Details laid out for the docked panel, `width` columns of text after
    /// the one column of padding: versions under the status, and folders
    /// and the web page on their own indented rows.
    fn details_docked_lines(&self, width: u16) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let pad = || Span::raw(" ");
        let Some(content) = self.details_content() else {
            let mut lines = vec![Line::default()];
            for line in self.details_empty_lines() {
                let mut spans = vec![pad()];
                spans.extend(line.spans);
                lines.push(Line::from(spans));
            }
            return lines;
        };
        let width = usize::from(width);
        let with_pad = |spans: Vec<Span<'static>>| {
            let mut row = vec![pad()];
            row.extend(spans);
            Line::from(row)
        };
        let mut lines = vec![Line::default(), with_pad(content.status.clone())];
        lines.push(with_pad(content.versions.clone()));
        lines.push(Line::default());
        if !content.sentence.is_empty() {
            let sentence = wrap_lines(
                &[Line::from(content.sentence.clone())],
                u16::try_from(width).unwrap_or(u16::MAX),
            );
            lines.extend(sentence.into_iter().map(|line| with_pad(line.spans)));
            lines.push(Line::default());
        }
        if !content.follow_up.is_empty() {
            let follow_up: Vec<Line<'static>> =
                content.follow_up.iter().cloned().map(Line::from).collect();
            let follow_up = wrap_lines(&follow_up, u16::try_from(width).unwrap_or(u16::MAX));
            lines.extend(follow_up.into_iter().map(|line| with_pad(line.spans)));
            lines.push(Line::default());
        }
        let blank_before = |lines: &mut Vec<Line<'static>>| {
            if lines.last().is_some_and(|line| line.width() > 0) {
                lines.push(Line::default());
            }
        };
        let label = |text: &'static str| Span::styled(text, Style::default().fg(roles.muted));
        let indent = 3usize;
        let item_width = width.saturating_sub(indent - 1);
        let mut after_block = false;
        for (name, value) in &content.rows {
            match value {
                DetailValue::Text(text) => {
                    if after_block {
                        blank_before(&mut lines);
                        after_block = false;
                    }
                    let value_width = width.saturating_sub(PANEL_LABEL_WIDTH).max(8);
                    for (index, row) in self
                        .detail_value_rows(text, value_width)
                        .into_iter()
                        .enumerate()
                    {
                        let prefix = if index == 0 {
                            label(name)
                        } else {
                            Span::raw("")
                        };
                        let used = prefix.width();
                        let mut spans = vec![
                            pad(),
                            prefix,
                            Span::raw(" ".repeat(PANEL_LABEL_WIDTH.saturating_sub(used))),
                        ];
                        spans.extend(row.spans);
                        lines.push(Line::from(spans));
                    }
                }
                DetailValue::Folders(folders) => {
                    blank_before(&mut lines);
                    lines.push(with_pad(vec![label(name)]));
                    for folder in folders {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(indent)),
                            Span::styled(
                                truncate_text(folder, item_width),
                                Style::default().fg(roles.text),
                            ),
                        ]));
                    }
                    after_block = true;
                }
                DetailValue::Link(link) => {
                    blank_before(&mut lines);
                    lines.push(with_pad(vec![label(name)]));
                    for row in wrap_link(link, item_width) {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(indent)),
                            Span::styled(row, Style::default().fg(roles.blue)),
                        ]));
                    }
                    after_block = true;
                }
            }
        }
        while lines.last().is_some_and(|line| line.width() == 0) {
            lines.pop();
        }
        lines
    }

    fn docked_details_folder(&self) -> Option<String> {
        self.inspect_resolved_target()
            .map(|target| target.item.folder.clone())
    }

    /// How far the docked panel is scrolled, for the addon it shows now:
    /// another addon starts at the top.
    pub(super) fn docked_details_scroll(&self) -> SheetScroll {
        let docked = self.docked_scroll.borrow();
        if docked.folder.is_some() && docked.folder == self.docked_details_folder() {
            docked.scroll
        } else {
            SheetScroll::default()
        }
    }

    /// True while Details is docked and does not fit beside the list.
    pub(super) fn docked_details_overflow(&self) -> bool {
        self.details_docked()
            && self
                .docked_details_scroll()
                .overflows(Some(OverlayKind::Inspect))
    }

    /// Where the docked panel's body was last drawn.
    pub(super) fn docked_details_area(&self) -> Option<Rect> {
        if self.details_docked() {
            self.docked_scroll.borrow().area
        } else {
            None
        }
    }

    /// Scrolls the docked panel to `offset` rows; the next draw clamps it.
    pub(super) fn set_docked_details_offset(&self, offset: u16) {
        let folder = self.docked_details_folder();
        let mut docked = self.docked_scroll.borrow_mut();
        if docked.folder != folder {
            docked.folder = folder;
            docked.scroll = SheetScroll::default();
        }
        docked.scroll.offset = offset;
    }

    /// The docked panel: the addon name on the column header row, then the
    /// details from the scroll offset. What does not fit ends in "↓ N more".
    pub(super) fn render_docked_details(&self, frame: &mut Frame<'_>, title: Rect, body: Rect) {
        let roles = self.ui_theme.roles;
        let text_width = body.width.saturating_sub(2);
        let name = self
            .inspect_resolved_target()
            .map_or_else(|| "Details".to_string(), |target| target.item.name.clone());
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    truncate_text(&name, usize::from(text_width)),
                    Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
                ),
            ])),
            title,
        );
        let mut lines = self.details_docked_lines(text_width);
        let folder = self.docked_details_folder();
        {
            let mut docked = self.docked_scroll.borrow_mut();
            if docked.folder != folder {
                docked.folder = folder;
                docked.scroll = SheetScroll::default();
            }
            let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
            let max_offset = total.saturating_sub(body.height);
            let offset = docked.scroll.offset.min(max_offset);
            docked.scroll = SheetScroll {
                kind: Some(OverlayKind::Inspect),
                offset,
                focus: None,
                max_offset,
                viewport: body.height,
                kept: None,
            };
            docked.area = Some(body);
            lines.drain(..usize::from(offset));
        }
        let height = usize::from(body.height);
        if height > 0 && lines.len() > height {
            let shown = height - 1;
            let hidden = lines.len() - shown;
            lines.truncate(shown);
            lines.push(Line::from(Span::styled(
                format!(" ↓ {hidden} more"),
                Style::default().fg(roles.muted),
            )));
        }
        frame.render_widget(Paragraph::new(lines), body);
    }
}

/// Docked Details as last drawn, so PgUp, PgDn and the wheel can scroll it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DockedScroll {
    /// The addon the panel showed; another one starts at the top.
    folder: Option<String>,
    scroll: SheetScroll,
    /// The panel body on screen.
    area: Option<Rect>,
}

fn spans_text_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// The Details sentence for "Check failed", as text parts and the key `c`
/// (`true`): the cause in plain words when LemonUp knows it, otherwise the
/// plain fact.
pub(super) fn check_failed_sentence(
    kind: Option<SourceErrorKind>,
    source: SourceKind,
) -> Vec<(String, bool)> {
    let name = dashboard_source_name(source, true);
    let try_again = |before: String, after: &str| {
        vec![
            (before, false),
            ("c".to_string(), true),
            (after.to_string(), false),
        ]
    };
    match kind {
        Some(SourceErrorKind::NoConnection) => try_again(
            format!("Couldn't reach {name}. Check your internet connection, then press "),
            " to try again.",
        ),
        Some(SourceErrorKind::KeyRejected) if source == SourceKind::Wago => try_again(
            "Wago.io turned down your Wago.io key. Check the key in Settings, then press "
                .to_string(),
            " to try again.",
        ),
        Some(SourceErrorKind::KeyRejected) if source == SourceKind::GitHub => try_again(
            "GitHub turned down LemonUp's request. If you set a GitHub token, check it, then press "
                .to_string(),
            " to try again.",
        ),
        Some(SourceErrorKind::KeyRejected) => try_again(
            format!("{name} turned down LemonUp's request. Press "),
            " to try again later.",
        ),
        Some(SourceErrorKind::NotFound) => try_again(
            format!(
                "{name} doesn't list this addon anymore. It may have moved or been taken down. Press "
            ),
            " to check again.",
        ),
        Some(SourceErrorKind::SlowDown) => try_again(
            format!("{name} asked LemonUp to slow down. Wait a few minutes, then press "),
            " to try again.",
        ),
        Some(
            SourceErrorKind::Other
            | SourceErrorKind::FoldersInTheWay
            | SourceErrorKind::FoldersOfLinkedAddon,
        )
        | None => try_again(
            format!("LemonUp couldn't check {name} for a newer version. Press "),
            " to try again.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_numbers_read_as_wow_versions() {
        assert_eq!(
            wow_versions_for_interface("120005").as_deref(),
            Some("WoW 12.0.5")
        );
        assert_eq!(
            wow_versions_for_interface("11507").as_deref(),
            Some("WoW 1.15.7")
        );
        assert_eq!(
            wow_versions_for_interface("110207, 11507").as_deref(),
            Some("WoW 11.2.7, 1.15.7")
        );
        assert_eq!(
            wow_versions_for_interface("120005,120005").as_deref(),
            Some("WoW 12.0.5")
        );
        assert_eq!(wow_versions_for_interface("").as_deref(), None);
        assert_eq!(wow_versions_for_interface("abc").as_deref(), None);
    }

    #[test]
    fn docked_panel_is_41_at_120_and_stops_near_52() {
        assert_eq!(docked_widths(120), (78, 41));
        assert_eq!(docked_widths(130), (83, 46));
        assert_eq!(docked_widths(142), (89, 52));
        assert_eq!(docked_widths(160), (107, 52));
        assert_eq!(docked_widths(250), (197, 52));
    }

    #[test]
    fn links_break_after_a_slash() {
        assert_eq!(
            wrap_link("addons.wago.io/addons/deadly-boss-mods", 36),
            vec!["addons.wago.io/addons/", "deadly-boss-mods"]
        );
        assert_eq!(wrap_link("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(
            web_page_label("https://www.wowinterface.com/downloads/info5633/"),
            "wowinterface.com/downloads/info5633"
        );
    }

    #[test]
    fn time_ago_uses_whole_words() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("now");
        let ago = |seconds: i64| time_ago(now, now - time::Duration::seconds(seconds));
        assert_eq!(ago(20), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(300), "5 minutes ago");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(3 * 86_400), "3 days ago");
    }
}
