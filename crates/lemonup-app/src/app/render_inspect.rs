use super::render_dashboard::{dashboard_item_remote_label, dashboard_item_update_status};
use super::sheet::SheetBody;
use super::*;
impl App {
    /// Details of one addon: the summary first, then the sections that
    /// `r` `d` `t` open. The whole body scrolls, so nothing is dropped.
    pub(super) fn inspect_sheet_body(&self, _width: u16) -> SheetBody<'static> {
        let mut body = SheetBody::new();
        let Some(target) = self.inspect_resolved_target() else {
            body.text(vec![
                Line::from(Span::styled(
                    "No addon selected",
                    Style::default()
                        .fg(self.ui_theme.panel_title)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Select a row, then press Enter to inspect.",
                    Style::default().fg(self.ui_theme.muted),
                )),
            ]);
            return body;
        };

        let hero_lines = self.inspect_hero_lines(target);
        if !hero_lines.is_empty() {
            body.text(hero_lines);
        }
        body.heading(" Summary ")
            .text(self.inspect_summary_lines(target))
            .gap(1)
            .heading(" Details ")
            .text(self.inspect_detail_lines(target));
        body
    }

    pub(super) fn inspect_hero_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(child_folder) = target.opened_from_child {
            lines.push(Line::from(Span::styled(
                format!("Opened from child: {child_folder}"),
                Style::default().fg(self.ui_theme.muted),
            )));
        }
        lines
    }

    pub(super) fn inspect_summary_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![
            self.inspect_source_line(target.item),
            self.inspect_status_line(target.item),
            self.inspect_version_line(target.item),
            self.overlay_kv_line("Folder", target.item.folder.clone()),
        ];

        if let Some(author) = target.item.author.as_deref().map(str::trim)
            && !author.is_empty()
            && !author.eq_ignore_ascii_case("unknown")
        {
            lines.push(self.overlay_kv_line("Author", author.to_string()));
        }

        if target.item.has_authoritative_owned_folders && target.item.source != SourceKind::Manual {
            lines.push(self.overlay_kv_line("Tracking", "Managed by LemonUp"));
        }

        lines
    }

    pub(super) fn inspect_source_line(&self, item: &DashboardItem) -> Line<'static> {
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                source_label(item.source).to_string(),
                Style::default()
                    .fg(source_badge_color(item.source))
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    }

    pub(super) fn inspect_version_line(&self, item: &DashboardItem) -> Line<'static> {
        Line::from(vec![
            Span::styled("Version: ", Style::default().fg(self.ui_theme.muted)),
            Span::styled(
                dashboard_item_version_label(item),
                Style::default().fg(self.ui_theme.installed_text),
            ),
        ])
    }

    pub(super) fn inspect_detail_lines(
        &self,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for section in [
            InspectSection::IncludedAddons,
            InspectSection::Dependencies,
            InspectSection::AddonInfo,
        ] {
            let is_open = self.inspect_overlay.is_open(section);
            lines.push(self.inspect_section_header(section, is_open));
            if is_open {
                lines.extend(self.inspect_section_body(section, target));
            }
        }
        lines
    }

    pub(super) fn inspect_status_line(&self, item: &DashboardItem) -> Line<'static> {
        let (status_label, status_color) = self.inspect_status_chip(item);
        let mut spans = vec![Span::styled(
            "Status: ",
            Style::default().fg(self.ui_theme.muted),
        )];
        let item_status = dashboard_item_update_status(item);
        let is_update_available =
            item_status == UpdateStatus::UpdateAvailable && item.source != SourceKind::Manual;
        let display_color = if is_update_available {
            self.inspect_pulse_color(status_color)
        } else {
            status_color
        };
        spans.push(Span::styled(
            status_label.to_string(),
            Style::default()
                .fg(display_color)
                .add_modifier(Modifier::BOLD),
        ));
        if is_update_available {
            let remote = dashboard_item_remote_label(item).unwrap_or_else(|| "update".to_string());
            spans.extend([
                Span::raw(" "),
                Span::styled("→", Style::default().fg(self.ui_theme.warning)),
                Span::raw(" "),
                Span::styled(
                    remote,
                    Style::default()
                        .fg(display_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled("📦", Style::default().fg(self.ui_theme.package_icon)),
            ]);
        } else if item_status == UpdateStatus::RemoteOlder && item.source != SourceKind::Manual {
            let remote = dashboard_item_remote_label(item).unwrap_or_else(|| "?".to_string());
            spans.push(Span::styled(
                format!(" (remote {remote}; not offered as an update)"),
                Style::default().fg(self.ui_theme.muted),
            ));
        }
        Line::from(spans)
    }

    pub(super) fn inspect_pulse_color(&self, base: Color) -> Color {
        match (base, self.shell_ui.motion.tick_count % 6) {
            (Color::Rgb(r, g, b), 0..=2) => Color::Rgb(r, g, b),
            (Color::Rgb(r, g, b), _) => Color::Rgb(
                r.saturating_add(22),
                g.saturating_add(18),
                b.saturating_add(8),
            ),
            (other, _) => other,
        }
    }

    pub(super) fn inspect_status_chip(&self, item: &DashboardItem) -> (&'static str, Color) {
        if item.source == SourceKind::Manual {
            return ("Manual", Color::Magenta);
        }
        match dashboard_item_update_status(item) {
            UpdateStatus::UpToDate => ("Up to date", self.ui_theme.success),
            UpdateStatus::UpdateAvailable => ("Update available", self.ui_theme.warning),
            UpdateStatus::RemoteOlder => ("Remote older", self.ui_theme.muted),
            UpdateStatus::Unknown => ("Unknown", Color::Magenta),
            UpdateStatus::Error => ("Error", self.ui_theme.error),
        }
    }

    pub(super) fn inspect_section_header(
        &self,
        section: InspectSection,
        is_open: bool,
    ) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                if is_open { "▾ " } else { "▸ " },
                Style::default().fg(self.ui_theme.muted),
            ),
            Span::styled(
                format!("{} ", section.key()),
                Style::default()
                    .fg(self.ui_theme.panel_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                section.label(),
                Style::default().fg(self.ui_theme.panel_title),
            ),
        ])
    }

    pub(super) fn inspect_section_body(
        &self,
        section: InspectSection,
        target: InspectResolvedTarget<'_>,
    ) -> Vec<Line<'static>> {
        let indent = "  ";
        match section {
            InspectSection::IncludedAddons => {
                let mut lines = Vec::new();
                if let Some(child_folder) = target.opened_from_child {
                    lines.push(Line::from(format!(
                        "{indent}Opened from child: {child_folder}"
                    )));
                }
                if !target.item.owned_folders.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Children: {}",
                        summarize_owned_folders(&target.item.owned_folders)
                    )));
                }
                let missing = self.dashboard_parent_missing_owned_children(&target.item.folder);
                if !missing.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Missing: {}",
                        summarize_owned_folders(missing)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No relationship details")));
                }
                lines
            }
            InspectSection::Dependencies => {
                let mut lines = Vec::new();
                if !target.item.required_deps.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Required: {}",
                        summarize_owned_folders(&target.item.required_deps)
                    )));
                }
                if !target.item.optional_deps.is_empty() {
                    lines.push(Line::from(format!(
                        "{indent}Optional: {}",
                        summarize_owned_folders(&target.item.optional_deps)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No dependency metadata")));
                }
                lines
            }
            InspectSection::AddonInfo => {
                let mut lines = Vec::new();
                if let Some(interface) = target.item.interface.as_deref().map(str::trim)
                    && !interface.is_empty()
                {
                    lines.push(Line::from(format!("{indent}Interface: {interface}")));
                }
                if let Some(source_url) = target.item.source_url.as_deref().map(str::trim)
                    && !source_url.is_empty()
                {
                    lines.push(Line::from(format!(
                        "{indent}Source URL: {}",
                        truncate_text(source_url, 64)
                    )));
                }
                if lines.is_empty() {
                    lines.push(Line::from(format!("{indent}No technical details")));
                }
                lines
            }
        }
    }
}
