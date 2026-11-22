use super::render_dashboard::shorten_path_middle;
use super::sheet::{SheetBody, SheetRoom, text_height};
use super::*;

/// The two groups of Settings: what all games share, then what belongs to
/// the game in use. The second also shows the game folder, read only,
/// above its rows.
const CONFIG_SECTIONS: [(ConfigGroup, &[ConfigField]); 2] = [
    (
        ConfigGroup::AllGames,
        &[
            ConfigField::WagoApiKey,
            ConfigField::Theme,
            ConfigField::SimpleSymbols,
            ConfigField::ReduceMotion,
            ConfigField::AutoCheckEnabled,
            ConfigField::AutoCheckInterval,
            ConfigField::CheckInterval,
            ConfigField::CheckOtherGames,
            ConfigField::CheckLemonupUpdates,
        ],
    ),
    (
        ConfigGroup::ThisGame,
        &[
            ConfigField::BackupWtf,
            ConfigField::BackupRetention,
            ConfigField::ShowLibs,
            ConfigField::ChangeGameFolder,
            ConfigField::SaveAddonList,
            ConfigField::LoadAddonList,
        ],
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigGroup {
    AllGames,
    ThisGame,
}

/// The label column on a wide sheet, and on a narrow one.
const LABEL_WIDTH_WIDE: usize = 49;
const LABEL_WIDTH_NARROW: usize = 44;

impl App {
    /// Every setting in one list that scrolls and keeps the cursor in view,
    /// with the highlighted row explained at the bottom. With room the
    /// sections have a blank row between them and the explanation is in
    /// full; with less the blank rows go first, then the explanation is cut
    /// to one line.
    ///
    /// The sheet reserves room for the longest explanation, so moving the
    /// cursor never resizes or moves it.
    pub(super) fn config_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let width = room.width;
        let long_rows = self.config_reserved_rows(width);
        let spaced_rows = self.config_list_rows(true);
        let tight_rows = self.config_list_rows(false);
        let short = self.config_description(width, false);
        let short_rows = text_height(&short, width);
        let layouts = [
            spaced_rows + long_rows,
            tight_rows + long_rows,
            tight_rows + short_rows,
        ];
        let layout = room.pick(&layouts);
        let (spaced, footer) = match layout {
            0 => (true, self.config_description(width, true)),
            1 => (false, self.config_description(width, true)),
            _ => (false, short),
        };

        let mut body = SheetBody::new();
        body.reserve(layouts[layout]);
        if spaced {
            body.gap(1);
        }
        for (index, (group, fields)) in CONFIG_SECTIONS.iter().enumerate() {
            if spaced && index > 0 {
                body.gap(1);
            }
            let title = match group {
                ConfigGroup::AllGames => "For all games".to_string(),
                // Before a folder is chosen the game has no name yet.
                ConfigGroup::ThisGame if self.effective_addon_dir.is_none() => {
                    "For this game".to_string()
                }
                ConfigGroup::ThisGame => format!("For {}", self.current_game_name()),
            };
            body.text(vec![Line::from(Span::styled(
                title,
                self.ui_theme.roles.muted.add_modifier(Modifier::BOLD),
            ))]);
            if *group == ConfigGroup::ThisGame {
                body.text(vec![self.config_game_folder_line(width)]);
            }
            for field in *fields {
                body.text(vec![self.config_field_line(*field, width)]);
                if self.config_pane.selected_field() == *field {
                    body.focus_last();
                }
            }
        }
        body.footer(footer);
        body
    }

    /// Rows the list takes, with or without a blank row between sections.
    fn config_list_rows(&self, spaced: bool) -> u16 {
        let rows = CONFIG_SECTIONS
            .iter()
            .map(|(_, fields)| fields.len() + 1)
            .sum::<usize>()
            + 1;
        let gaps = if spaced { CONFIG_SECTIONS.len() } else { 0 };
        u16::try_from(rows + gaps).unwrap_or(u16::MAX)
    }

    fn config_label_width(width: u16) -> usize {
        if width >= 90 {
            LABEL_WIDTH_WIDE
        } else {
            LABEL_WIDTH_NARROW.min(usize::from(width).saturating_sub(20))
        }
    }

    /// "Game folder    Retail · ~/Games/…/_retail_/Interface/AddOns": shown
    /// here, next to the row that changes it. The cursor does not stop on it.
    fn config_game_folder_line(&self, width: u16) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let label_width = Self::config_label_width(width);
        let room = usize::from(width).saturating_sub(2 + label_width);
        let path = self.friendly_addon_dir();
        let value = if path.is_empty() {
            "Not chosen yet".to_string()
        } else {
            let client = self.game_client_label();
            let path = shorten_path_middle(&path, room.saturating_sub(client.len() + 3));
            format!("{client} · {path}")
        };
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<label_width$}", "Game folder"),
                Style::default().fg(roles.text),
            ),
            Span::styled(truncate_text(&value, room), roles.muted),
        ])
    }

    fn config_field_line(&self, field: ConfigField, width: u16) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let is_selected = self.config_pane.selected_field() == field;
        let edit = self
            .config_pane
            .edit
            .as_ref()
            .filter(|edit| edit.field == field);
        let bold = |style: Style| {
            if is_selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            }
        };
        let label_width = Self::config_label_width(width);
        let room = usize::from(width).saturating_sub(2 + label_width).max(1);

        let mut spans = vec![Span::styled(
            if is_selected { "▌ " } else { "  " },
            Style::default().fg(roles.lemon),
        )];
        let label_style = bold(Style::default().fg(roles.text));
        if field.kind() == ConfigRowKind::Action {
            spans.push(Span::styled(
                truncate_text(field.label(), usize::from(width).saturating_sub(2)),
                label_style,
            ));
            return Line::from(spans);
        }
        spans.push(Span::styled(
            format!(
                "{:<label_width$}",
                truncate_text(field.label(), label_width.saturating_sub(1))
            ),
            label_style,
        ));

        if let Some(edit) = edit {
            // The key is a secret: every character shows as a dot.
            spans.extend(
                edit.value.masked('•').spans(
                    room,
                    Style::default()
                        .fg(roles.lemon)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(roles.lemon),
                    true,
                ),
            );
            if edit.value.is_empty() {
                spans.push(Span::styled(" paste your key here", roles.muted));
            }
            return Line::from(spans);
        }

        let value = self.render_config_field_value(field);
        if field.supports_quick_adjust() {
            let arrows = Style::default().fg(roles.dim);
            spans.extend([
                Span::styled("‹ ", arrows),
                Span::styled(
                    truncate_text(&value, room.saturating_sub(4).max(1)),
                    bold(Style::default().fg(roles.text)),
                ),
                Span::styled(" ›", arrows),
            ]);
        } else {
            spans.push(Span::styled(truncate_text(&value, room), roles.muted));
        }
        Line::from(spans)
    }

    /// What the last action said, while the sheet is open (the status line
    /// shows nothing under a sheet): a problem, work in progress, or the
    /// result of an action row while the cursor is on it.
    fn config_status_line(&self, width: u16) -> Option<Line<'static>> {
        let roles = self.ui_theme.roles;
        let message = self.active_detail_status_message(DetailMode::Config)?;
        let kind = self.status_line.kind;
        let on_action = self.config_pane.selected_field().kind() == ConfigRowKind::Action;
        let color = match kind {
            StatusKind::Problem => roles.red,
            StatusKind::Working => roles.text,
            StatusKind::Done if on_action => roles.green,
            StatusKind::Note if on_action => roles.text,
            _ => return None,
        };
        Some(Line::from(Span::styled(
            truncate_text(&message, usize::from(width)),
            Style::default().fg(color),
        )))
    }

    /// The highlighted row explained, pinned under the list: a rule, its
    /// name and what it does (`long`), or one line "Name: what it does".
    /// A message from the last action goes under it, or in place of the one
    /// line.
    fn config_description(&self, width: u16, long: bool) -> Vec<Line<'static>> {
        let status = self.config_status_line(width);
        if !long && let Some(status) = status {
            return vec![status];
        }
        let field = self.config_pane.selected_field();
        let editing = self.config_pane.edit.is_some();
        let mut lines = self.config_description_text(field, editing, width, long);
        lines.extend(status);
        lines
    }

    /// Rows the long explanation reserves: the tallest one among all the
    /// rows at `width`, with one more row for the result of an action row.
    /// The explanation on screen counts too, so a message that does not fit
    /// the others still has room.
    fn config_reserved_rows(&self, width: u16) -> u16 {
        let mut rows = text_height(&self.config_description(width, true), width);
        for field in ConfigField::ALL {
            // Only the key row reads differently while it is being edited.
            let edits: &[bool] = if field == ConfigField::WagoApiKey {
                &[false, true]
            } else {
                &[false]
            };
            for editing in edits {
                let text = self.config_description_text(field, *editing, width, true);
                let result = u16::from(field.kind() == ConfigRowKind::Action);
                rows = rows.max(text_height(&text, width).saturating_add(result));
            }
        }
        rows
    }

    fn config_description_text(
        &self,
        field: ConfigField,
        editing: bool,
        width: u16,
        long: bool,
    ) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let link = Style::default().fg(roles.blue);
        let key = Style::default().fg(roles.blue).add_modifier(Modifier::BOLD);
        let display = self.runtime.display;
        let following = self.follow_terminal.get();
        let transfer_path = || {
            default_transfer_path()
                .map(|path| friendly_path(&path))
                .unwrap_or_else(|_| "your home folder".to_string())
        };
        if !long {
            let text = match field {
                ConfigField::WagoApiKey if editing => "enter save, esc cancel".to_string(),
                ConfigField::WagoApiKey
                    if self.wago_key_from_environment(&self.config_pane.draft) =>
                {
                    "from WAGO_API_KEY, and a key you save here is used instead".to_string()
                }
                ConfigField::WagoApiKey => {
                    "needed to search Wago.io and to update addons from it".to_string()
                }
                ConfigField::AutoCheckEnabled => {
                    "checks for updates in the background while LemonUp is open".to_string()
                }
                ConfigField::AutoCheckInterval => "how often the background check runs".to_string(),
                ConfigField::CheckInterval => {
                    "an addon checked this recently is not checked again yet".to_string()
                }
                ConfigField::CheckOtherGames => {
                    "how often your other games are checked while LemonUp is open".to_string()
                }
                ConfigField::CheckLemonupUpdates => "looks on GitHub once a day for a newer \
                     LemonUp and shows it at the top of the addon list"
                    .to_string(),
                ConfigField::BackupWtf => {
                    "copies your keybinds, macros and addon setups before updates".to_string()
                }
                ConfigField::BackupRetention => "older backups past this number go".to_string(),
                ConfigField::Theme if display.no_color => {
                    "NO_COLOR is set, so colors are off".to_string()
                }
                ConfigField::Theme if following => {
                    "your terminal does not report full color, so Terminal for now".to_string()
                }
                ConfigField::Theme => "dark, light, or your terminal's own colors".to_string(),
                ConfigField::SimpleSymbols if !display.utf8 => {
                    "on, because your locale is not UTF-8".to_string()
                }
                ConfigField::SimpleSymbols => {
                    "plain ASCII symbols, and LEMONUP in place of the logo".to_string()
                }
                ConfigField::ReduceMotion => {
                    "a still … in place of the spinner, and no shimmer".to_string()
                }
                ConfigField::ShowLibs => "each addon's folders open in the list".to_string(),
                ConfigField::ChangeGameFolder => {
                    "opens setup to choose a different game folder".to_string()
                }
                ConfigField::SaveAddonList => {
                    format!("exports your addon list to {}", transfer_path())
                }
                ConfigField::LoadAddonList => {
                    format!("installs the addons listed in {}", transfer_path())
                }
            };
            let name = field.label().trim_end_matches('…');
            let line = format!("{name}: {text}.");
            return vec![Line::from(Span::styled(
                truncate_text(&line, usize::from(width)),
                muted,
            ))];
        }

        let plain = |text: &str| vec![Span::styled(text.to_string(), muted)];
        let text: Vec<Span<'static>> = match field {
            ConfigField::WagoApiKey if editing => vec![
                Span::styled("Paste your key, then press ", muted),
                Span::styled("enter", key),
                Span::styled(" to save. An empty box removes the key. ", muted),
                Span::styled("esc", key),
                Span::styled(" cancel.", muted),
            ],
            ConfigField::WagoApiKey if self.wago_key_from_environment(&self.config_pane.draft) => {
                vec![
                    Span::styled(
                        "This key comes from the WAGO_API_KEY environment variable. A key you \
                         save here is used instead. Press ",
                        muted,
                    ),
                    Span::styled("enter", key),
                    Span::styled(" to type or paste one.", muted),
                ]
            }
            ConfigField::WagoApiKey => vec![
                Span::styled(
                    "Needed to search Wago.io and to update addons that came from it. Wago \
                     gives a key to its Patreon supporters: ",
                    muted,
                ),
                Span::styled("addons.wago.io/patreon", link),
                Span::styled(". Press ", muted),
                Span::styled("enter", key),
                Span::styled(
                    if self.config_pane.draft.wago_api_key.is_some() {
                        " to change it."
                    } else {
                        " to type or paste it."
                    },
                    muted,
                ),
            ],
            ConfigField::AutoCheckEnabled => {
                plain("While LemonUp is open, it checks your addons for updates in the background.")
            }
            ConfigField::AutoCheckInterval => plain(if self.config_pane.draft.auto_check_enabled {
                "How often the background check runs."
            } else {
                "How often the background check runs. Automatic checks are off now."
            }),
            ConfigField::CheckInterval => plain(
                "An addon checked less than this long ago is not checked again yet, which \
                 keeps checks quick.",
            ),
            ConfigField::CheckOtherGames => plain(
                "While LemonUp is open, it also checks your other games for updates this \
                 often, one at a time. It only checks: it never updates or changes them.",
            ),
            ConfigField::CheckLemonupUpdates => plain(
                "Looks on GitHub once a day for a newer LemonUp and shows it at the top of the \
                 addon list.",
            ),
            ConfigField::BackupWtf => plain(
                "Before updates, LemonUp saves a copy of your game settings: your keybinds, \
                 macros and addon setups (the game's WTF folder).",
            ),
            ConfigField::BackupRetention => plain(
                "LemonUp keeps this many backups of your game settings and removes older ones.",
            ),
            ConfigField::Theme => {
                let mut text = String::from(
                    "Dark or Light colors, or Terminal: your terminal's own background and its \
                     16 colors. When NO_COLOR is set, LemonUp uses Terminal with no colors at all.",
                );
                if display.no_color {
                    text.push_str(" NO_COLOR is set now.");
                } else if following {
                    text.push_str(
                        " Your terminal does not report full color, so LemonUp starts in \
                         Terminal. You can still choose Dark or Light here.",
                    );
                }
                plain(&text)
            }
            ConfigField::SimpleSymbols => {
                let mut text = String::from(
                    "Plain ASCII symbols, such as ^ for an update and ok for up to date, and the \
                     word LEMONUP in place of the block logo. For terminals and fonts that show \
                     boxes or question marks.",
                );
                if !display.utf8 {
                    text.push_str(
                        " It is on now because your locale (LC_ALL, LC_CTYPE or LANG) is not \
                         UTF-8.",
                    );
                }
                plain(&text)
            }
            ConfigField::ReduceMotion => plain(
                "Shows a still … in place of the spinner on the line that is working, and \
                 turns off the shimmer.",
            ),
            ConfigField::ShowLibs => plain(
                "When this is on, the addon list opens with every addon's folders shown under \
                 it. ] and [ still show or hide them all while LemonUp runs.",
            ),
            ConfigField::ChangeGameFolder => {
                plain("Opens setup, where you can choose a different game folder.")
            }
            ConfigField::SaveAddonList => plain(&format!(
                "Saves the list of your addons to {}, so you can load it on another computer.",
                transfer_path()
            )),
            ConfigField::LoadAddonList => {
                let saved = default_transfer_path().is_ok_and(|path| path.exists());
                plain(&format!(
                    "Installs the addons listed in {} into this game folder.{}",
                    transfer_path(),
                    if saved {
                        ""
                    } else {
                        " No list has been saved there yet."
                    }
                ))
            }
        };
        vec![
            Line::default(),
            Line::from(Span::styled(
                "─".repeat(usize::from(width)),
                Style::default().fg(roles.dim),
            )),
            Line::from(Span::styled(
                field.label().trim_end_matches('…').to_string(),
                Style::default().fg(roles.text).add_modifier(Modifier::BOLD),
            )),
            Line::from(text),
        ]
    }
}
