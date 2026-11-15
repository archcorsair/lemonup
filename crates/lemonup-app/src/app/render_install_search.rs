use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use super::direct_install::DirectInstallProvider;
use super::sheet::{SheetBody, render_stateful, text_height, wrap_lines_hanging};
use super::*;
use crate::source_error::FolderClash;

/// What the sheet says for each kind of source error: what happened and
/// what to do. The raw error is shown under it as a technical detail.
pub(super) fn source_error_advice(kind: SourceErrorKind, provider: &str) -> String {
    match kind {
        SourceErrorKind::NoConnection => format!(
            "LemonUp couldn't reach {provider}. Check your internet connection and try again."
        ),
        SourceErrorKind::KeyRejected if provider == "Wago.io" => {
            "Wago.io didn't accept your Wago.io key. Check it and paste it again.".to_string()
        }
        SourceErrorKind::KeyRejected => format!(
            "{provider} didn't accept the token LemonUp sent (GITHUB_TOKEN or GH_TOKEN). \
             Fix it or unset it, then try again."
        ),
        SourceErrorKind::NotFound => {
            format!("{provider} doesn't have that addon. Check the name or the link and try again.")
        }
        SourceErrorKind::SlowDown => {
            format!("{provider} asked LemonUp to slow down. Wait a few minutes, then try again.")
        }
        SourceErrorKind::FoldersInTheWay => {
            "Its folders are already in your AddOns folder.".to_string()
        }
        SourceErrorKind::FoldersOfLinkedAddon => "Its folders belong to an addon LemonUp \
                                                  already keeps updated, so LemonUp won't \
                                                  replace them."
            .to_string(),
        SourceErrorKind::Other => "Something went wrong on the way. Try again; if it keeps \
                                   happening, the technical detail helps with a bug report."
            .to_string(),
    }
}

/// `source_error_advice`, naming the folder and the linked addon it belongs
/// to when an install was refused for that.
pub(super) fn install_error_advice(
    kind: SourceErrorKind,
    provider: &str,
    clash: Option<&FolderClash>,
) -> String {
    let linked = clash.map_or(&[][..], |clash| clash.linked.as_slice());
    match (kind, linked) {
        (SourceErrorKind::FoldersOfLinkedAddon, [(folder, addon)]) => format!(
            "{folder} belongs to {addon}, which LemonUp already keeps updated, so LemonUp \
             won't replace it."
        ),
        (SourceErrorKind::FoldersOfLinkedAddon, [(folder, addon), rest @ ..]) => {
            let others = if rest.iter().all(|(_, owner)| owner == addon) {
                format!("{addon}, which LemonUp already keeps updated")
            } else {
                format!("{addon} and other addons LemonUp already keeps updated")
            };
            format!(
                "{folder} and {} more belong to {others}, so LemonUp won't replace them.",
                rest.len()
            )
        }
        _ => source_error_advice(kind, provider),
    }
}

/// Columns before the text of "  Technical detail: ".
const TECHNICAL_DETAIL_INDENT: u16 = 20;

/// An addon already in the list that a result matches.
pub(super) struct InstalledCopy {
    pub(super) linked: bool,
}

impl App {
    /// The box (always drawn), one line saying what enter will do with its
    /// text, then the body for what the sheet is doing. The results get the
    /// rows the rest leaves.
    pub(super) fn search_sheet_body(&self, width: u16, max_height: u16) -> SheetBody<'_> {
        let mut body = SheetBody::new();
        body.widget(3, |area, buffer| self.render_search_input(area, buffer))
            .text(vec![self.search_hint_line()])
            .gap(1);

        let pane = &self.search_pane;
        if pane.key_box.is_some() {
            body.text(self.search_key_box_intro_lines())
                .widget(3, |area, buffer| self.render_wago_key_box(area, buffer))
                .text(vec![self.body_keys_line(&[
                    ("enter", "save the key and search", true),
                    ("esc", "back to the search box", false),
                ])]);
            return body;
        }
        if let Some(request) = pane.pending_direct_install() {
            let card = self.direct_install_card_lines(request);
            let height = u16::try_from(card.len()).unwrap_or(u16::MAX) + 2;
            body.widget(height, move |area, buffer| {
                let card_width = area.width.min(68);
                let card_area = Rect::new(area.x, area.y, card_width, area.height);
                Paragraph::new(card)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(self.ui_theme.border)),
                    )
                    .wrap(Wrap { trim: false })
                    .render(card_area, buffer);
            });
            return body;
        }
        if let Some(lines) = self.search_state_lines(width) {
            body.text(lines);
            return body;
        }
        if pane.results.is_empty() {
            body.text(self.search_empty_state_lines());
            return body;
        }

        let preview = self.search_selected_preview_lines(width);
        // Box, hint, gap and the preview.
        let fixed = 5u16.saturating_add(text_height(&preview, width));
        let needed = u16::try_from(pane.results.len())
            .unwrap_or(u16::MAX)
            .saturating_add(1);
        let room = max_height.saturating_sub(fixed);
        let results_height = if needed <= room {
            needed
        } else {
            room.max(needed.min(4))
        };
        body.widget(results_height, |area, buffer| {
            self.render_search_results_list(area, buffer)
        })
        .focus_last();
        if !preview.is_empty() {
            body.text(preview);
        }
        body
    }

    /// Whether typed keys go to the install box right now.
    fn search_box_focused(&self) -> bool {
        let pane = &self.search_pane;
        pane.key_box.is_none()
            && pane.pending_direct_install().is_none()
            && pane.pending_direct_replace().is_none()
            && !pane.results_focused()
            && !self.install_running()
    }

    pub(super) fn render_search_input(&self, area: Rect, buffer: &mut Buffer) {
        let focused = self.search_box_focused();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(if focused {
                Style::default().fg(self.ui_theme.highlight)
            } else {
                Style::default().fg(self.ui_theme.border)
            })
            .title(" Addon name or link ");
        let room = usize::from(area.width.saturating_sub(2));
        // Dimmed while an install runs: the box takes no text then.
        let text_style = if self.install_running() {
            Style::default().fg(self.ui_theme.muted)
        } else {
            Style::default()
                .fg(self.ui_theme.highlight)
                .add_modifier(Modifier::BOLD)
        };
        let spans = self.search_pane.query.spans(
            room,
            text_style,
            Style::default().fg(self.ui_theme.brand_gold),
            focused,
        );
        Paragraph::new(Line::from(spans))
            .block(block)
            .render(area, buffer);
    }

    fn render_wago_key_box(&self, area: Rect, buffer: &mut Buffer) {
        let width = area.width.min(82);
        let area = Rect::new(area.x, area.y, width, area.height);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.ui_theme.highlight))
            .title(" Wago.io key ");
        let key = self.search_pane.key_box.clone().unwrap_or_default();
        let room = usize::from(area.width.saturating_sub(2));
        let mut spans = key.masked('•').spans(
            room,
            Style::default().fg(self.ui_theme.highlight),
            Style::default().fg(self.ui_theme.brand_gold),
            true,
        );
        if key.is_empty() {
            spans.push(Span::styled(
                " paste your key here",
                Style::default().fg(self.ui_theme.muted),
            ));
        }
        Paragraph::new(Line::from(spans))
            .block(block)
            .render(area, buffer);
    }

    /// The line under the box: what enter will do with the text.
    pub(super) fn search_hint_line(&self) -> Line<'static> {
        let pane = &self.search_pane;
        let muted = Style::default().fg(self.ui_theme.muted);
        let quiet = matches!(
            pane.install_state,
            SearchInstallState::Success { .. }
                | SearchInstallState::Error { .. }
                | SearchInstallState::Checking { .. }
                | SearchInstallState::Installing { .. }
                | SearchInstallState::InstallingDirect { .. }
        ) || pane.in_progress;
        if self.install_running() && pane.key_box.is_none() {
            return Line::from(Span::styled(
                "Installing. You can type again when it's done.",
                muted,
            ));
        }
        if quiet && pane.key_box.is_none() {
            return Line::from("");
        }
        match classify_search_input(pane.query.value()) {
            SearchInputIntent::Empty => Line::from(""),
            SearchInputIntent::Direct(request) => Line::from(Span::styled(
                match request.provider {
                    DirectInstallProvider::Tukui => {
                        format!("Installs {} from TukUI.", request.display)
                    }
                    provider => format!("That's a {} link.", provider.label()),
                },
                muted,
            )),
            SearchInputIntent::WagoUrl(_) => {
                Line::from(Span::styled("That's a Wago.io link.", muted))
            }
            SearchInputIntent::WagoSearch(query) => {
                let count = pane.results.len();
                let text = if count > 0 && pane.last_query.as_deref() == Some(query.as_str()) {
                    format!(
                        "{count} addon{} on Wago.io match \"{query}\"",
                        plural_suffix(count)
                    )
                } else {
                    format!("Search Wago.io for \"{query}\"")
                };
                Line::from(Span::styled(text, muted))
            }
            SearchInputIntent::InvalidUrl(message) => Line::from(Span::styled(
                message,
                Style::default().fg(self.ui_theme.warning),
            )),
        }
    }

    fn search_key_box_intro_lines(&self) -> Vec<Line<'static>> {
        let rejected = matches!(
            self.search_pane.install_state,
            SearchInstallState::Error {
                kind: SourceErrorKind::KeyRejected,
                ..
            }
        );
        let headline = if rejected {
            "Wago.io didn't accept that key. Check it and paste it again."
        } else {
            "Searching Wago.io needs your Wago.io key."
        };
        vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("Wago gives a key to its Patreon supporters: "),
                Span::styled(
                    "addons.wago.io/patreon",
                    Style::default().fg(self.ui_theme.info),
                ),
            ]),
            Line::from(Span::styled(
                "GitHub, WoWInterface and TukUI links work without one.",
                Style::default().fg(self.ui_theme.muted),
            )),
            Line::from(""),
        ]
    }

    /// Keys written into the body, as the mockups show them.
    fn body_keys_line(&self, keys: &[(&str, &str, bool)]) -> Line<'static> {
        let mut spans = Vec::new();
        for (index, (key, label, primary)) in keys.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("          "));
            }
            spans.push(Span::styled(
                key.to_string(),
                Style::default()
                    .fg(if *primary {
                        self.ui_theme.brand_gold
                    } else {
                        self.ui_theme.info
                    })
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                label.to_string(),
                if *primary {
                    Style::default().fg(self.ui_theme.panel_title)
                } else {
                    Style::default().fg(self.ui_theme.muted)
                },
            ));
        }
        Line::from(spans)
    }

    fn direct_install_card_lines(&self, request: &DirectInstallRequest) -> Vec<Line<'static>> {
        let link = match request.provider {
            DirectInstallProvider::Github => format!("github.com/{}", request.display),
            DirectInstallProvider::Wowinterface => {
                format!("wowinterface.com, {}", request.display)
            }
            DirectInstallProvider::Tukui => "tukui.org".to_string(),
        };
        vec![
            Line::from(Span::styled(
                format!(
                    " Install {} from {}?",
                    request.name(),
                    request.provider.label()
                ),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(" {link}"),
                Style::default().fg(self.ui_theme.muted),
            )),
            Line::from(""),
            {
                let mut line =
                    self.body_keys_line(&[("enter", "install", true), ("esc", "not now", false)]);
                line.spans.insert(0, Span::raw("     "));
                line
            },
        ]
    }

    /// The body while something runs, is done, failed or waits for the
    /// replace question, `width` columns wide. `None` when the sheet shows
    /// results or examples.
    pub(super) fn search_state_lines(&self, width: u16) -> Option<Vec<Line<'static>>> {
        let pane = &self.search_pane;
        let title = Style::default()
            .fg(self.ui_theme.panel_title)
            .add_modifier(Modifier::BOLD);
        let working = |text: String| {
            let mut spans = vec![Span::styled(
                format!("{} ", self.shell_ui.motion.spinner_frame()),
                Style::default().fg(self.ui_theme.brand_gold),
            )];
            spans.extend(shimmer_text_spans(
                &text,
                self.ui_theme.panel_title,
                self.ui_theme.shimmer_glow,
                ShimmerConfig::action(),
            ));
            vec![Line::from(spans)]
        };
        match &pane.install_state {
            SearchInstallState::Checking { addon_name, .. }
            | SearchInstallState::Installing { addon_name, .. } => {
                return Some(working(format!("Installing {addon_name} from Wago.io…")));
            }
            SearchInstallState::InstallingDirect { request } => {
                return Some(working(format!(
                    "Installing {} from {}…",
                    request.name(),
                    request.provider.label()
                )));
            }
            SearchInstallState::ConfirmDirectReplace { name, .. } => {
                return Some(vec![Line::from(Span::styled(
                    format!("{name}'s folders are already in your AddOns folder."),
                    Style::default().fg(self.ui_theme.muted),
                ))]);
            }
            SearchInstallState::Success {
                addon_name,
                warning,
                ..
            } => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        format!("✓ {addon_name} is installed."),
                        Style::default()
                            .fg(self.ui_theme.success)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  It's in your addon list now. Reload the game interface ("),
                        Span::styled("/reload", title),
                        Span::raw(") to use it."),
                    ]),
                ];
                if let Some(warning) = warning {
                    lines.push(Line::from(Span::styled(
                        format!("  {warning}"),
                        Style::default().fg(self.ui_theme.warning),
                    )));
                }
                lines.push(Line::from(""));
                let mut keys = self.body_keys_line(&[
                    ("enter", "install another", true),
                    ("esc", "back to my addons", false),
                ]);
                keys.spans.insert(0, Span::raw("  "));
                lines.push(keys);
                return Some(lines);
            }
            SearchInstallState::Error {
                addon_name,
                provider,
                kind,
                detail,
                clash,
                ..
            } => {
                let headline = match addon_name {
                    Some(name) => format!("! Couldn't install {name}."),
                    None => format!("! Couldn't search {provider}."),
                };
                let mut keys =
                    self.body_keys_line(&[("enter", "try again", true), ("esc", "close", false)]);
                keys.spans.insert(0, Span::raw("  "));
                let mut lines = vec![
                    Line::from(Span::styled(
                        headline,
                        Style::default()
                            .fg(self.ui_theme.error)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(format!(
                        "  {}",
                        install_error_advice(*kind, provider, clash.as_ref())
                    )),
                    Line::from(""),
                    keys,
                    Line::from(""),
                ];
                // A long detail wraps under its own start, not under the label.
                lines.extend(wrap_lines_hanging(
                    &[Line::from(Span::styled(
                        format!("  Technical detail: {detail}"),
                        Style::default().fg(self.ui_theme.muted),
                    ))],
                    width,
                    TECHNICAL_DETAIL_INDENT,
                ));
                return Some(lines);
            }
            SearchInstallState::Idle
            | SearchInstallState::ConfirmReplace { .. }
            | SearchInstallState::ConfirmDirect { .. } => {}
        }
        if pane.in_progress {
            let query = pane.last_query.clone().unwrap_or_default();
            return Some(working(format!("Searching Wago.io for \"{query}\"…")));
        }
        None
    }

    /// No results yet: the things you can type, or why a search found none.
    pub(super) fn search_empty_state_lines(&self) -> Vec<Line<'static>> {
        let muted = Style::default().fg(self.ui_theme.muted);
        if let Some(query) = &self.search_pane.last_query {
            return vec![
                Line::from(Span::styled(
                    format!("No addons on Wago.io match \"{query}\"."),
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Check the spelling, or try fewer words. You can also paste a link to \
                     the addon's page.",
                    muted,
                )),
            ];
        }
        let example = |label: &str, text: &str| {
            Line::from(vec![
                Span::raw(format!("  {label:<22}")),
                Span::styled(text.to_string(), muted),
            ])
        };
        vec![
            Line::from(Span::styled("You can type or paste any of these:", muted)),
            Line::from(""),
            example("an addon name", "searches Wago.io"),
            example("a GitHub link", "github.com/BigWigsMods/LittleWigs"),
            example(
                "a WoWInterface link",
                "wowinterface.com/downloads/info4802-Pawn.html",
            ),
            example("a Wago.io link", "addons.wago.io/addons/details"),
            Line::from(vec![
                Span::styled(
                    "  elvui",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  or  "),
                Span::styled(
                    "tukui",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(22 - 16)),
                Span::styled("installs ElvUI or Tukui", muted),
            ]),
        ]
    }

    pub(super) fn render_search_results_list(&self, area: Rect, buffer: &mut Buffer) {
        let pane = &self.search_pane;
        let result_count = pane.results.len();
        let focused = pane.results_focused();
        let mut show_above = false;
        let mut show_below = false;
        let mut window_start = 0;
        let mut window_end = 0;

        for _ in 0..2 {
            let reserved_rows = 1 + u16::from(show_above) + u16::from(show_below);
            let visible_rows = area.height.saturating_sub(reserved_rows).max(1) as usize;
            let (start, end) = visible_search_result_window(
                result_count,
                pane.selected_result.filter(|_| focused),
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
        let muted = Style::default().fg(self.ui_theme.muted);
        let mut area_index = 0;
        if show_above {
            Widget::render(
                Paragraph::new(Line::from(Span::styled(
                    format!(
                        "↑ {} more result{} above",
                        window_start,
                        plural_suffix(window_start)
                    ),
                    muted,
                ))),
                areas[area_index],
                buffer,
            );
            area_index += 1;
        }
        let table_area = areas[area_index];
        area_index += 1;

        // Narrow sheets drop the author before anything else.
        let show_author = table_area.width >= 72;
        let heading = muted.add_modifier(Modifier::BOLD);
        let mut header = vec![Cell::from(Span::styled("  Addon", heading))];
        if show_author {
            header.push(Cell::from(Span::styled("Author", heading)));
        }
        header.push(Cell::from(Span::styled("Downloads", heading)));
        header.push(Cell::from(Span::styled("Version", heading)));
        header.push(Cell::from(""));
        let mut rows = Vec::new();
        for (index, result) in pane.results[window_start..window_end].iter().enumerate() {
            let selected = focused && pane.selected_result == Some(window_start + index);
            let author = result
                .owner
                .as_deref()
                .or_else(|| result.authors.first().map(String::as_str))
                .unwrap_or("");
            let mut cells = vec![Cell::from(Line::from(vec![
                Span::styled(
                    if selected { "▌ " } else { "  " },
                    Style::default().fg(self.ui_theme.brand_gold),
                ),
                Span::styled(
                    result.display_name.clone(),
                    if selected {
                        Style::default()
                            .fg(self.ui_theme.highlight)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.ui_theme.panel_title)
                    },
                ),
            ]))];
            if show_author {
                cells.push(Cell::from(Span::styled(author.to_string(), muted)));
            }
            cells.push(Cell::from(Span::styled(
                format!(
                    "{:>9}",
                    result
                        .download_count
                        .map(format_download_count)
                        .unwrap_or_default()
                ),
                muted,
            )));
            cells.push(Cell::from(Span::styled(
                result.version.clone().unwrap_or_default(),
                muted,
            )));
            cells.push(Cell::from(if self.installed_copy(result).is_some() {
                Line::from(vec![
                    Span::styled("✓ ", Style::default().fg(self.ui_theme.success)),
                    Span::styled("Installed", muted),
                ])
            } else {
                Line::from("")
            }));
            rows.push(Row::new(cells));
        }

        let mut widths = vec![Constraint::Min(16)];
        if show_author {
            widths.push(Constraint::Length(20));
        }
        widths.extend([
            Constraint::Length(9),
            Constraint::Length(12),
            Constraint::Length(11),
        ]);
        let table = Table::new(rows, widths)
            .header(Row::new(header))
            .column_spacing(2);
        let mut state = TableState::default();
        render_stateful(table, table_area, buffer, &mut state);

        if show_below {
            let remaining_below = result_count.saturating_sub(window_end);
            Widget::render(
                Paragraph::new(Line::from(Span::styled(
                    format!(
                        "↓ {} more result{} below",
                        remaining_below,
                        plural_suffix(remaining_below)
                    ),
                    muted,
                ))),
                areas[area_index],
                buffer,
            );
        }
    }

    /// Under the results, for the one the cursor is on: its name, summary
    /// and what enter does if you already have it.
    pub(super) fn search_selected_preview_lines(&self, width: u16) -> Vec<Line<'static>> {
        if !self.search_pane.results_focused() {
            return Vec::new();
        }
        let Some(result) = self.search_pane.selected_result() else {
            return Vec::new();
        };
        let muted = Style::default().fg(self.ui_theme.muted);
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "─".repeat(usize::from(width.saturating_sub(2))),
                Style::default().fg(self.ui_theme.border),
            )),
            Line::from(Span::styled(
                result.display_name.clone(),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(summary) = result.summary.as_deref().filter(|text| !text.is_empty()) {
            lines.push(Line::from(summary.to_string()));
        }
        match self.installed_copy(result) {
            Some(InstalledCopy { linked: true }) => lines.push(Line::from(Span::styled(
                "You already have this one. Enter reinstalls it over your copy.",
                muted,
            ))),
            Some(InstalledCopy { linked: false }) => lines.push(Line::from(Span::styled(
                "You have a copy that isn't linked. Enter replaces it with this one and \
                 links it. Your addon settings stay as they are.",
                muted,
            ))),
            None => {}
        }
        lines
    }

    /// Whether a result is already in the addon list: linked to it on
    /// Wago.io, or a not linked addon with the same name.
    pub(super) fn installed_copy(&self, result: &WagoSearchResult) -> Option<InstalledCopy> {
        if self.tracked_wago_result_folder(result).is_some() {
            return Some(InstalledCopy { linked: true });
        }
        self.dashboard
            .items
            .iter()
            .any(|item| {
                item.source == SourceKind::Manual
                    && (item.name.eq_ignore_ascii_case(&result.display_name)
                        || item.folder.eq_ignore_ascii_case(&result.display_name))
            })
            .then_some(InstalledCopy { linked: false })
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
}
