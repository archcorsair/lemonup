use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use super::direct_install::DirectInstallProvider;
use super::sheet::{SheetBody, SheetRoom, render_stateful, text_height, wrap_lines_hanging};
use super::*;
use crate::github::GithubTokenSource;
use crate::source_error::FolderClash;

/// The Addon column never gets narrower than this to make room for the
/// version.
const SEARCH_ADDON_MIN_WIDTH: u16 = 16;
/// The Version column grows from the first to the second to fit the longest
/// version shown.
const SEARCH_VERSION_MIN_WIDTH: u16 = 12;
const SEARCH_VERSION_MAX_WIDTH: u16 = 18;

/// The width of the Version column for versions up to `longest` characters,
/// in a table `table_width` wide. The Addon column keeps its minimum; when
/// there is no room the column stays at its minimum and versions are cut.
fn search_version_width(longest: usize, table_width: u16, show_author: bool) -> u16 {
    let columns: u16 = if show_author { 5 } else { 4 };
    let others = 9 + 11 + if show_author { 20 } else { 0 } + 2 * (columns - 1);
    let room = table_width.saturating_sub(others + SEARCH_ADDON_MIN_WIDTH);
    let wanted = u16::try_from(longest)
        .unwrap_or(u16::MAX)
        .clamp(SEARCH_VERSION_MIN_WIDTH, SEARCH_VERSION_MAX_WIDTH);
    wanted.min(room.max(SEARCH_VERSION_MIN_WIDTH))
}

/// How many terminal cells `text` takes.
fn cells(text: &str) -> usize {
    Span::raw(text).width()
}

/// The longest start of `characters` that fits in `width` cells.
fn fit_cells(characters: impl Iterator<Item = char>, width: usize) -> String {
    let mut used = 0;
    let mut fitted = String::new();
    for character in characters {
        used += text_input::char_width(character);
        if used > width {
            break;
        }
        fitted.push(character);
    }
    fitted
}

/// `text` cut to `width` cells, ending in `ellipsis` when it was cut.
fn cut_with_ellipsis(text: &str, width: usize, ellipsis: &str) -> String {
    if cells(text) <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(cells(ellipsis));
    format!("{}{ellipsis}", fit_cells(text.chars(), keep))
}

/// How many characters a marquee over `text` scrolls by until its end
/// fits in `width` cells.
fn marquee_steps(text: &str, width: usize) -> usize {
    let mut used = 0;
    let mut start = text.chars().count();
    for character in text.chars().rev() {
        used += text_input::char_width(character);
        if used > width {
            break;
        }
        start -= 1;
    }
    start
}

/// What the sheet says for each kind of source error: what happened and
/// what to do. The raw error is shown under it as a technical detail.
/// `github_token` is where the GitHub token in use comes from, if one is sent.
pub(crate) fn source_error_advice(
    kind: SourceErrorKind,
    provider: &str,
    github_token: Option<GithubTokenSource>,
) -> String {
    let github = provider == "GitHub";
    match kind {
        SourceErrorKind::NoConnection => format!(
            "LemonUp couldn't reach {provider}. Check your internet connection and try again."
        ),
        SourceErrorKind::KeyRejected if provider == "Wago.io" => {
            "Wago.io didn't accept your Wago.io key. Check it and paste it again.".to_string()
        }
        SourceErrorKind::KeyRejected if github => match github_token {
            Some(GithubTokenSource::Settings) => "GitHub didn't accept your GitHub token. Check \
                                                  it in Settings, then try again."
                .to_string(),
            // A new value in the variable counts only once LemonUp starts again.
            Some(GithubTokenSource::Environment(name)) => {
                format!("GitHub didn't accept the token in {name}. Replace or unset it.")
            }
            None => "GitHub turned down LemonUp's request. Try again later.".to_string(),
        },
        SourceErrorKind::KeyRejected => {
            format!("{provider} turned down LemonUp's request. Try again later.")
        }
        SourceErrorKind::NotFound => {
            format!("{provider} doesn't have that addon. Check the name or the link and try again.")
        }
        SourceErrorKind::SlowDown if github && github_token.is_none() => {
            "GitHub's limit of 60 requests an hour is used up. Add a GitHub token in Settings to \
             raise it, or try again later."
                .to_string()
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
        SourceErrorKind::WrongRepository => "The repository may have a new owner. LemonUp won't \
                                             install it until its list is updated."
            .to_string(),
        SourceErrorKind::Other => "Something went wrong on the way. Try again; if it keeps \
                                   happening, the technical detail helps with a bug report."
            .to_string(),
    }
}

/// `source_error_advice`, naming the folder and the linked addon it belongs
/// to when an install was refused for that.
pub(crate) fn install_error_advice(
    kind: SourceErrorKind,
    provider: &str,
    clash: Option<&FolderClash>,
    github_token: Option<GithubTokenSource>,
) -> String {
    let linked = clash.map_or(&[][..], |clash| clash.linked.as_slice());
    // An addon a newer LemonUp linked shows as "Not linked" here, so this version does not
    // claim to keep it updated.
    let newer = clash.is_some_and(|clash| clash.newer_version_linked);
    let keeps = if newer {
        "a newer LemonUp linked"
    } else {
        "LemonUp already keeps updated"
    };
    match (kind, linked) {
        (SourceErrorKind::FoldersOfLinkedAddon, [(folder, addon)]) => {
            format!("{folder} belongs to {addon}, which {keeps}, so LemonUp won't replace it.")
        }
        (SourceErrorKind::FoldersOfLinkedAddon, [(folder, addon), rest @ ..]) => {
            let others = if rest.iter().all(|(_, owner)| owner == addon) {
                format!("{addon}, which {keeps}")
            } else if newer {
                format!("{addon} and other linked addons")
            } else {
                format!("{addon} and other addons LemonUp already keeps updated")
            };
            format!(
                "{folder} and {} more belong to {others}, so LemonUp won't replace them.",
                rest.len()
            )
        }
        _ => source_error_advice(kind, provider, github_token),
    }
}

/// "matches" for one addon, "match" for any other count.
fn match_verb(count: usize) -> &'static str {
    if count == 1 { "matches" } else { "match" }
}

/// Whether `url` is the GitHub link of `owner/repo`, read the way updates read
/// it, so a link kept from 1.x (`www.`, `.git`) still counts.
fn github_url_names_repo(url: &str, owner: &str, repo: &str) -> bool {
    crate::github::github_url_is_repo(url, owner, repo)
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
    ///
    /// The sheet is as tall as it can be under the header in every state,
    /// the room the results get, so typing, searching, results and messages
    /// never move or resize it. Results past that scroll.
    pub(super) fn search_sheet_body(&self, room: &SheetRoom) -> SheetBody<'_> {
        let (width, max_height) = (room.width, room.clear_of_header());
        let mut body = SheetBody::new();
        body.reserve(max_height);
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
                            .border_style(Style::default().fg(self.ui_theme.roles.border)),
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
                Style::default().fg(self.ui_theme.roles.lemon)
            } else {
                Style::default().fg(self.ui_theme.roles.border)
            })
            .title(" Addon name or link ");
        let room = usize::from(area.width.saturating_sub(2));
        // Dimmed while an install runs: the box takes no text then.
        let text_style = if self.install_running() {
            self.ui_theme.roles.muted
        } else {
            Style::default()
                .fg(self.ui_theme.roles.lemon)
                .add_modifier(Modifier::BOLD)
        };
        let spans = self.search_pane.query.spans(
            room,
            text_style,
            Style::default().fg(self.ui_theme.roles.lemon),
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
            .border_style(Style::default().fg(self.ui_theme.roles.lemon))
            .title(" Wago.io key ");
        let key = self.search_pane.key_box.clone().unwrap_or_default();
        let room = usize::from(area.width.saturating_sub(2));
        let mut spans = key.masked('•').spans(
            room,
            Style::default().fg(self.ui_theme.roles.lemon),
            Style::default().fg(self.ui_theme.roles.lemon),
            true,
        );
        if key.is_empty() {
            spans.push(Span::styled(
                " paste your key here",
                self.ui_theme.roles.muted,
            ));
        }
        Paragraph::new(Line::from(spans))
            .block(block)
            .render(area, buffer);
    }

    /// The line under the box: what enter will do with the text.
    pub(super) fn search_hint_line(&self) -> Line<'static> {
        let pane = &self.search_pane;
        let muted = self.ui_theme.roles.muted;
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
                let shown = count > 0 && pane.last_query.as_deref() == Some(query.as_str());
                let text = if shown && pane.known_only {
                    known_rows_need_key_message(count)
                } else if shown && !pane.some_result_has_every_word() {
                    NO_ADDON_HAS_EVERY_WORD.to_string()
                } else if shown && pane.has_known_rows() {
                    // Known addons are not on Wago.io.
                    format!(
                        "{count} addon{} {} \"{query}\"",
                        plural_suffix(count),
                        match_verb(count)
                    )
                } else if shown {
                    format!(
                        "{count} addon{} on Wago.io {} \"{query}\"",
                        plural_suffix(count),
                        match_verb(count)
                    )
                } else {
                    format!("Search Wago.io for \"{query}\"")
                };
                Line::from(Span::styled(text, muted))
            }
            SearchInputIntent::InvalidUrl(message) => Line::from(Span::styled(
                message,
                Style::default().fg(self.ui_theme.roles.amber),
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
                    .fg(self.ui_theme.roles.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("Wago gives a key to its Patreon supporters: "),
                Span::styled(
                    "addons.wago.io/patreon",
                    Style::default().fg(self.ui_theme.roles.blue),
                ),
            ]),
            Line::from(Span::styled(
                "GitHub, WoWInterface and TukUI links work without one.",
                self.ui_theme.roles.muted,
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
                        self.ui_theme.roles.lemon
                    } else {
                        self.ui_theme.roles.blue
                    })
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                label.to_string(),
                if *primary {
                    Style::default().fg(self.ui_theme.roles.text)
                } else {
                    self.ui_theme.roles.muted
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
                    .fg(self.ui_theme.roles.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(format!(" {link}"), self.ui_theme.roles.muted)),
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
            .fg(self.ui_theme.roles.text)
            .add_modifier(Modifier::BOLD);
        let working = |text: String| {
            let mut spans = vec![Span::styled(
                format!("{} ", self.spinner_frame()),
                Style::default().fg(self.ui_theme.roles.lemon),
            )];
            spans.extend(working_text_spans(
                &text,
                self.ui_theme.roles.text,
                self.ui_theme.roles.lemon,
                self.ui_theme.reduce_motion,
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
                    self.ui_theme.roles.muted,
                ))]);
            }
            SearchInstallState::Success {
                addon_name,
                parent_folder,
                warning,
                ..
            } => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        format!("{} {addon_name} is installed.", self.ui_theme.symbols.done),
                        Style::default()
                            .fg(self.ui_theme.roles.green)
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
                        Style::default().fg(self.ui_theme.roles.amber),
                    )));
                }
                // The sheet has no status line: what marking it Ignored in
                // CurseForge did shows here.
                if let Some((kind, note)) = self.curseforge_details_note(parent_folder) {
                    let color = super::curseforge_mark::note_color(kind, self.ui_theme.roles);
                    lines.push(Line::from(Span::styled(
                        format!("  {note}"),
                        Style::default().fg(color),
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
                let mut keys = if pane.retry_refused() {
                    self.body_keys_line(&[("esc", "close", false)])
                } else {
                    self.body_keys_line(&[("enter", "try again", true), ("esc", "close", false)])
                };
                keys.spans.insert(0, Span::raw("  "));
                let mut lines = vec![
                    Line::from(Span::styled(
                        headline,
                        Style::default()
                            .fg(self.ui_theme.roles.red)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(format!(
                        "  {}",
                        install_error_advice(
                            *kind,
                            provider,
                            clash.as_ref(),
                            self.github_token_source()
                        )
                    )),
                    Line::from(""),
                    keys,
                    Line::from(""),
                ];
                // A long detail wraps under its own start, not under the label.
                lines.extend(wrap_lines_hanging(
                    &[Line::from(Span::styled(
                        format!("  Technical detail: {detail}"),
                        self.ui_theme.roles.muted,
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
        let muted = self.ui_theme.roles.muted;
        if let Some(query) = &self.search_pane.last_query {
            return vec![
                Line::from(Span::styled(
                    format!("No addons on Wago.io match \"{query}\"."),
                    Style::default()
                        .fg(self.ui_theme.roles.text)
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
                        .fg(self.ui_theme.roles.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  or  "),
                Span::styled(
                    "tukui",
                    Style::default()
                        .fg(self.ui_theme.roles.text)
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
        let muted = self.ui_theme.roles.muted;
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
        // Every result, not just the visible window, so the column keeps
        // its width while the list scrolls.
        let longest_version = pane
            .results
            .iter()
            .map(|result| result.version.as_deref().map_or(0, cells))
            .max()
            .unwrap_or(0);
        let version_width = search_version_width(longest_version, table_area.width, show_author);
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
                    Style::default().fg(self.ui_theme.roles.lemon),
                ),
                Span::styled(
                    result.display_name.clone(),
                    if selected {
                        Style::default()
                            .fg(self.ui_theme.roles.lemon)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.ui_theme.roles.text)
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
                self.search_version_text(
                    result.version.as_deref().unwrap_or_default(),
                    usize::from(version_width),
                    selected,
                ),
                muted,
            )));
            let installed = match pane.known_entry(result) {
                Some(addon) => self.linked_known_addon(addon),
                None => self.installed_copy(result).is_some(),
            };
            cells.push(Cell::from(if installed {
                Line::from(vec![
                    Span::styled(
                        format!("{} ", self.ui_theme.symbols.done),
                        Style::default().fg(self.ui_theme.roles.green),
                    ),
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
            Constraint::Length(version_width),
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

    /// A version as it shows in the Version column. One that does not fit
    /// is cut with an ellipsis. On the selected row it scrolls to show the
    /// whole of it, except under reduce motion, where it stays cut.
    fn search_version_text(&self, version: &str, width: usize, selected: bool) -> String {
        let cut = cut_with_ellipsis(version, width, self.ui_theme.symbols.ellipsis);
        if !selected || cells(version) <= width || self.ui_theme.reduce_motion {
            return cut;
        }
        self.search_marquee_live.set(true);
        let ticks = self
            .shell_ui
            .motion
            .tick_count
            .wrapping_sub(self.shell_ui.search_selection_tick);
        // The scroll moves by characters. Each frame is fitted by cells, so
        // a double-width character is never cut in half.
        let steps = marquee_steps(version, width);
        let offset =
            lemonup_core::marquee::marquee_offset(steps + width, width, ticks, MARQUEE_HOLD_TICKS);
        if offset == 0 {
            cut
        } else {
            fit_cells(version.chars().skip(offset), width)
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
        let muted = self.ui_theme.roles.muted;
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "─".repeat(usize::from(width.saturating_sub(2))),
                Style::default().fg(self.ui_theme.roles.border),
            )),
            Line::from(Span::styled(
                result.display_name.clone(),
                Style::default()
                    .fg(self.ui_theme.roles.text)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(addon) = self.search_pane.known_entry(result) {
            lines.extend(self.known_preview_lines(addon));
            return lines;
        }
        if let Some(summary) = result.summary.as_deref().filter(|text| !text.is_empty()) {
            lines.push(Line::from(summary.to_string()));
        }
        match self.installed_copy(result) {
            Some(InstalledCopy { linked: true }) => lines.push(Line::from(Span::styled(
                "You already have this one. Enter reinstalls it over your copy.",
                muted,
            ))),
            Some(InstalledCopy { linked: false }) => lines.push(Line::from(Span::styled(
                "You have a copy that isn't linked. Enter replaces it with this one.",
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
        self.not_linked_named(&result.display_name)
            .then_some(InstalledCopy { linked: false })
    }

    /// Under a known addon's name: its repository, and the line about a
    /// copy that isn't linked when one has its name. A linked copy gets no
    /// line, because a GitHub install won't replace a linked addon's folders.
    fn known_preview_lines(&self, addon: &KnownAddon) -> Vec<Line<'static>> {
        let muted = self.ui_theme.roles.muted;
        let KnownSource::GitHub { owner, repo, .. } = &addon.source else {
            return vec![Line::from(Span::styled(
                "Only on CurseForge. LemonUp can't install it.",
                muted,
            ))];
        };
        let mut lines = vec![Line::from(Span::styled(
            format!("github.com/{owner}/{repo}"),
            muted,
        ))];
        if !self.linked_known_addon(addon) && self.not_linked_named(&addon.name) {
            lines.push(Line::from(Span::styled(
                "You have a copy that isn't linked. Enter replaces it with this one.",
                muted,
            )));
        }
        lines
    }

    /// Whether the cursor is on a known addon LemonUp can't install: a
    /// CurseForge-only one, or one whose repository is already linked, since
    /// a GitHub install won't replace a linked addon's folders. Enter does
    /// nothing there.
    pub(super) fn known_row_cannot_install(&self) -> bool {
        self.search_pane
            .selected_known_entry()
            .is_some_and(|addon| !self.known_addon_installable(addon))
    }

    pub(super) fn known_addon_installable(&self, addon: &KnownAddon) -> bool {
        addon.source != KnownSource::CurseForgeOnly && !self.linked_known_addon(addon)
    }

    /// Whether an addon in the list is linked to the known addon's GitHub
    /// repository.
    pub(super) fn linked_known_addon(&self, addon: &KnownAddon) -> bool {
        let KnownSource::GitHub { owner, repo, .. } = &addon.source else {
            return false;
        };
        self.dashboard.items.iter().any(|item| {
            item.source == SourceKind::GitHub
                && item
                    .source_url
                    .as_deref()
                    .is_some_and(|url| github_url_names_repo(url, owner, repo))
        })
    }

    /// Whether a "Not linked" addon has `name` as its name or folder.
    fn not_linked_named(&self, name: &str) -> bool {
        self.dashboard.items.iter().any(|item| {
            item.source == SourceKind::Manual
                && (item.name.eq_ignore_ascii_case(name) || item.folder.eq_ignore_ascii_case(name))
        })
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
