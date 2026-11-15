use super::sheet::SheetBody;
use super::*;

const CONFIG_CREDENTIALS_FIELDS: [ConfigField; 1] = [ConfigField::WagoApiKey];
const CONFIG_UPDATES_FIELDS: [ConfigField; 3] = [
    ConfigField::CheckInterval,
    ConfigField::AutoCheckEnabled,
    ConfigField::AutoCheckInterval,
];
const CONFIG_BACKUPS_FIELDS: [ConfigField; 2] =
    [ConfigField::BackupWtf, ConfigField::BackupRetention];
const CONFIG_DISPLAY_FIELDS: [ConfigField; 2] = [ConfigField::Theme, ConfigField::ShowLibs];

impl App {
    /// Every setting in one list that scrolls and keeps the cursor in view.
    pub(super) fn config_sheet_body(&self, width: u16) -> SheetBody<'static> {
        let mut body = SheetBody::new();
        body.text(self.config_summary_lines()).gap(1);
        for (title, fields) in [
            (" Credentials ", &CONFIG_CREDENTIALS_FIELDS[..]),
            (" Updates ", &CONFIG_UPDATES_FIELDS[..]),
            (" Backups ", &CONFIG_BACKUPS_FIELDS[..]),
            (" Display ", &CONFIG_DISPLAY_FIELDS[..]),
        ] {
            body.heading(title);
            for field in fields {
                body.text(vec![self.config_field_line(*field, width)]);
                if self.config_pane.selected_field() == *field {
                    body.focus_last();
                }
            }
        }
        body
    }

    fn config_summary_lines(&self) -> Vec<Line<'static>> {
        let field = self.config_pane.selected_field();
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Use: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Changes save instantly",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    "Text fields apply on Enter",
                    Style::default().fg(self.ui_theme.panel_title),
                ),
            ]),
            Line::from(vec![
                Span::styled("Selected: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    field.label(),
                    Style::default()
                        .fg(self.ui_theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        if let Some(status) = self.active_detail_status_message(DetailMode::Config) {
            lines.push(Line::from(vec![
                Span::styled("Status: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(status, Style::default().fg(self.ui_theme.panel_title)),
            ]));
        }
        lines.push(Line::from(vec![Span::styled(
            self.config_field_help_text(field),
            Style::default().fg(self.ui_theme.muted),
        )]));
        lines
    }

    fn config_field_line(&self, field: ConfigField, width: u16) -> Line<'static> {
        let is_selected = self.config_pane.selected_field() == field;
        let is_editing = self
            .config_pane
            .edit
            .as_ref()
            .is_some_and(|edit| edit.field == field);
        let marker_style = Style::default()
            .fg(if is_selected {
                self.ui_theme.highlight
            } else {
                self.ui_theme.muted
            })
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default()
            .fg(if is_selected {
                self.ui_theme.panel_title
            } else {
                self.ui_theme.muted
            })
            .add_modifier(if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let value_style = Style::default()
            .fg(if is_editing {
                self.ui_theme.highlight
            } else {
                self.ui_theme.panel_title
            })
            .add_modifier(if is_editing {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });

        let label_width = width.saturating_div(2).clamp(16, 24) as usize;
        let label = truncate_text(field.label(), label_width.saturating_sub(1));
        let mut spans = vec![
            Span::styled(if is_selected { "› " } else { "  " }, marker_style),
            Span::styled(format!("{label:<width$}", width = label_width), label_style),
        ];

        if is_editing {
            let edit = self
                .config_pane
                .edit
                .as_ref()
                .expect("checked editing state");
            let value = if edit.value.is_empty() {
                "▌".to_string()
            } else {
                format!("{} ▌", edit.value)
            };
            spans.push(Span::styled(value, value_style));
        } else {
            spans.push(Span::styled(
                truncate_text(
                    &self.render_config_field_value(field),
                    (width as usize).saturating_sub(2 + label_width).max(1),
                ),
                value_style,
            ));
        }

        Line::from(spans)
    }
}
