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
    pub(super) fn render_config_overlay(&self, frame: &mut Frame<'_>, area: Rect) {
        let content_area = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        let summary_lines = self.config_summary_lines();
        let summary_height = summary_lines.len() as u16;
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(summary_height),
                Constraint::Length(2),
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);

        frame.render_widget(
            Paragraph::new(summary_lines).wrap(Wrap { trim: false }),
            sections[0],
        );
        self.render_config_group(
            frame,
            sections[1],
            " Credentials ",
            &CONFIG_CREDENTIALS_FIELDS,
        );
        self.render_config_group(frame, sections[2], " Updates ", &CONFIG_UPDATES_FIELDS);
        self.render_config_group(frame, sections[3], " Backups ", &CONFIG_BACKUPS_FIELDS);
        self.render_config_group(frame, sections[4], " Display ", &CONFIG_DISPLAY_FIELDS);
        frame.render_widget(Paragraph::new(""), sections[5]);
        frame.render_widget(
            Paragraph::new(self.config_action_line()).alignment(Alignment::Center),
            sections[6],
        );
    }

    fn config_summary_lines(&self) -> Vec<Line<'static>> {
        let field = self.config_pane.selected_field();
        vec![
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
            Line::from(vec![Span::styled(
                self.config_field_help_text(field),
                Style::default().fg(self.ui_theme.muted),
            )]),
        ]
    }

    fn render_config_group(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &'static str,
        fields: &[ConfigField],
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
        let rows = fields
            .iter()
            .map(|field| self.config_field_line(*field, inner.width))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(rows).wrap(Wrap { trim: false }), inner);
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
                truncate_text(&self.render_config_field_value(field), width as usize),
                value_style,
            ));
        }

        Line::from(spans)
    }

    fn config_action_line(&self) -> Line<'static> {
        let chips = if self.config_pane.edit.is_some() {
            vec![
                SearchActionChip {
                    key: "enter",
                    label: "apply",
                    enabled: true,
                },
                SearchActionChip {
                    key: "esc",
                    label: "cancel",
                    enabled: true,
                },
                SearchActionChip {
                    key: "backspace",
                    label: "delete",
                    enabled: true,
                },
            ]
        } else {
            vec![
                SearchActionChip {
                    key: "↑/↓",
                    label: "fields",
                    enabled: true,
                },
                SearchActionChip {
                    key: "enter",
                    label: if self.config_pane.selected_field().is_textual() {
                        "edit"
                    } else {
                        "toggle"
                    },
                    enabled: true,
                },
                SearchActionChip {
                    key: "h/l",
                    label: "change",
                    enabled: self.config_pane.selected_field().supports_quick_adjust(),
                },
                SearchActionChip {
                    key: "r",
                    label: "onboarding",
                    enabled: true,
                },
                SearchActionChip {
                    key: "i",
                    label: "import",
                    enabled: true,
                },
                SearchActionChip {
                    key: "x",
                    label: "export",
                    enabled: true,
                },
            ]
        };

        let mut spans = Vec::new();
        for (index, chip) in chips.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(
                format!(" {} ", chip.key),
                Style::default()
                    .fg(if chip.enabled {
                        self.ui_theme.panel_title
                    } else {
                        self.ui_theme.muted
                    })
                    .bg(if chip.enabled {
                        Color::Rgb(31, 37, 58)
                    } else {
                        Color::Rgb(27, 30, 45)
                    })
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                chip.label,
                Style::default().fg(if chip.enabled {
                    self.ui_theme.highlight
                } else {
                    self.ui_theme.muted
                }),
            ));
        }

        Line::from(spans)
    }
}
