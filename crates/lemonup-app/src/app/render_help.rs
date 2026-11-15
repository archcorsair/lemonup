use super::keymap::help_sections;
use super::sheet::SheetBody;
use super::*;

impl App {
    pub(super) fn current_help_context(&self) -> HelpContext {
        if self.shell_ui.overlay.active == Some(OverlayKind::Inspect) {
            HelpContext::Inspect
        } else {
            self.current_help_context_for_mode(self.dashboard.detail_mode)
        }
    }

    pub(super) fn current_help_context_for_mode(&self, mode: DetailMode) -> HelpContext {
        match mode {
            DetailMode::Overview | DetailMode::Update => HelpContext::Overview,
            DetailMode::Install | DetailMode::Search => HelpContext::InstallSearch,
            DetailMode::Config => HelpContext::Config,
            DetailMode::Backup => HelpContext::Backup,
        }
    }

    /// The keys for the context Help was opened from. The body scrolls.
    pub(super) fn help_sheet_body(&self) -> SheetBody<'static> {
        let mut body = SheetBody::new();
        body.text(self.help_summary_lines());
        for (index, section) in help_sections(self.shell_ui.help_context).iter().enumerate() {
            if index > 0 {
                body.gap(1);
            }
            body.heading(section.title);
            body.text(
                section
                    .rows
                    .iter()
                    .map(|(key, text)| self.help_entry(key, text))
                    .collect(),
            );
        }
        body
    }

    fn help_summary_lines(&self) -> Vec<Line<'static>> {
        let (title, subtitle) = match self.shell_ui.help_context {
            HelpContext::Overview => (
                "Overview shortcuts",
                "Navigate, open details, select, and run bulk actions.",
            ),
            HelpContext::Inspect => ("Details shortcuts", "Review an addon and act on it."),
            HelpContext::InstallSearch => (
                "Install shortcuts",
                "Search Wago or install from a GitHub, WoWInterface, TukUI or Wago URL.",
            ),
            HelpContext::Config => (
                "Config shortcuts",
                "Move through settings and apply changes instantly.",
            ),
            HelpContext::Backup => (
                "Backup shortcuts",
                "Create backups and restore a selected archive.",
            ),
        };

        vec![
            Line::from(vec![
                Span::styled("Context: ", Style::default().fg(self.ui_theme.muted)),
                Span::styled(
                    title,
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                subtitle,
                Style::default().fg(self.ui_theme.muted),
            )),
            // The header no longer shows the version, so Help does.
            Line::from(Span::styled(
                format!("LemonUp {}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(self.ui_theme.muted),
            )),
            Line::default(),
        ]
    }

    fn help_entry(&self, key: &'static str, text: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!(" {key} "),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .bg(self.ui_theme.key_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(text, Style::default().fg(self.ui_theme.panel_title)),
        ])
    }
}
