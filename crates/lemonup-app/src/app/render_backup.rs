use std::sync::OnceLock;

use time::UtcOffset;

use super::render_dashboard::shorten_path_middle;
use super::render_inspect::time_ago;
use super::sheet::{SheetBody, SheetRoom, text_height};
use super::*;

/// The "Made" column: the longest date ("Wed 11 Nov at 20:33", or
/// "28 Nov 2025 at 17:42") and room to spare.
const MADE_WIDTH: usize = 24;
/// The "2 hours ago" column.
const AGO_WIDTH: usize = 18;

impl App {
    /// What a backup holds and when LemonUp makes one, the backups newest
    /// first with the cursor on one, and the folder they are kept in.
    /// Restoring and deleting ask in the question dialog, over this sheet.
    ///
    /// The sheet keeps room for one more backup and for the result line, so
    /// making, deleting or restoring a backup does not move it.
    pub(super) fn backup_sheet_body(&self, room: &SheetRoom) -> SheetBody<'static> {
        let width = room.width;
        let now = OffsetDateTime::now_utc();
        let offset = local_offset();
        let backups = &self.backup_pane.backups;

        let mut body = SheetBody::new();
        let intro = self.backup_intro_lines();
        // The intro, a blank row, the heading, the backups and one more, a
        // blank row, the folder, then a blank row and the result.
        let count = u16::try_from(backups.len()).unwrap_or(u16::MAX);
        body.reserve(
            text_height(&intro, width)
                .saturating_add(count)
                .saturating_add(7),
        );
        body.text(intro).gap(1);
        if backups.is_empty() {
            body.text(vec![self.backup_empty_line()]);
        } else {
            let selected = self.backup_pane.selected_backup.min(backups.len() - 1);
            body.text(vec![self.backup_heading_line(offset.is_none())]);
            for (index, entry) in backups.iter().enumerate() {
                let is_selected = index == selected;
                body.text(vec![self.backup_row_line(entry, is_selected, now, offset)]);
                if is_selected {
                    body.focus_last();
                }
            }
            body.gap(1).text(vec![self.backup_folder_line(width)]);
        }
        if let Some(status) = self.backup_status_line(width) {
            body.footer(vec![Line::from(""), status]);
        }
        body
    }

    /// "Your keybinds, macros and addon setups for Retail (the game's WTF
    /// folder)." and what LemonUp does with them, from Settings.
    fn backup_intro_lines(&self) -> Vec<Line<'static>> {
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let client = self
            .effective_addon_dir
            .as_deref()
            .and_then(GameFlavor::detect_from_path)
            .map(|flavor| match flavor {
                GameFlavor::Forever => " for WoW Forever",
                _ => " for Retail",
            })
            .unwrap_or("");
        let keep = self.config_pane.draft.backup_retention;
        let policy = if self.config_pane.draft.backup_wtf {
            format!("LemonUp makes a backup before it updates addons, and keeps the newest {keep}.")
        } else {
            format!(
                "Backing up before updates is off in Settings. LemonUp keeps the newest {keep}."
            )
        };
        vec![
            Line::from(vec![
                Span::styled(
                    format!("Your keybinds, macros and addon setups{client}"),
                    Style::default().fg(roles.text),
                ),
                Span::styled(" (the game's WTF folder).", muted),
            ]),
            Line::from(Span::styled(policy, muted)),
        ]
    }

    fn backup_empty_line(&self) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let muted = roles.muted;
        let mut spans = vec![Span::styled(
            "No backups of your game settings yet.",
            Style::default().fg(roles.text),
        )];
        // `b` is not offered while a backup is running.
        if !self.backup_pane.in_progress {
            spans.extend([
                Span::styled(" Press ", muted),
                Span::styled(
                    "b",
                    Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to make one now.", muted),
            ]);
        }
        Line::from(spans)
    }

    /// "Made" and "Size" over the columns. Times are local, or say UTC when
    /// the local time zone is unknown.
    fn backup_heading_line(&self, utc: bool) -> Line<'static> {
        let made = if utc { "Made (UTC)" } else { "Made" };
        Line::from(Span::styled(
            format!("  {made:<width$}Size", width = MADE_WIDTH + AGO_WIDTH),
            self.ui_theme.roles.muted.add_modifier(Modifier::BOLD),
        ))
    }

    /// "▌ Today at 09:15          2 hours ago       1.9 KB".
    fn backup_row_line(
        &self,
        entry: &BackupEntry,
        is_selected: bool,
        now: OffsetDateTime,
        offset: Option<UtcOffset>,
    ) -> Line<'static> {
        let roles = self.ui_theme.roles;
        let made = backup_when(entry.made_at, now, offset.unwrap_or(UtcOffset::UTC));
        let mut made_style = Style::default().fg(roles.text);
        if is_selected {
            made_style = made_style.add_modifier(Modifier::BOLD);
        }
        Line::from(vec![
            Span::styled(
                if is_selected { "▌ " } else { "  " },
                Style::default().fg(roles.lemon),
            ),
            Span::styled(format!("{made:<MADE_WIDTH$}"), made_style),
            Span::styled(
                format!("{:<AGO_WIDTH$}", time_ago(now, entry.made_at)),
                roles.muted,
            ),
            Span::styled(
                format_backup_size(entry.size_bytes),
                Style::default().fg(roles.text),
            ),
        ])
    }

    /// "3 backups, kept in ~/.local/share/lemonup/backups/wtf/retail".
    fn backup_folder_line(&self, width: u16) -> Line<'static> {
        let count = self.backup_pane.backups.len();
        let lead = format!("{count} backup{}, kept in ", plural_suffix(count));
        let room = usize::from(width).saturating_sub(lead.chars().count());
        let folder = shorten_path_middle(&friendly_path(&self.backup_dir), room);
        Line::from(Span::styled(
            truncate_text(&format!("{lead}{folder}"), usize::from(width)),
            self.ui_theme.roles.muted,
        ))
    }

    /// What the last backup, restore or delete said, pinned under the list
    /// (the status line shows nothing under a sheet).
    fn backup_status_line(&self, width: u16) -> Option<Line<'static>> {
        let roles = self.ui_theme.roles;
        let message = self.active_detail_status_message(DetailMode::Backup)?;
        let (text, color) = match self.status_line.kind {
            StatusKind::Problem => (message, roles.red),
            StatusKind::Done => (
                format!("{} {message}", self.ui_theme.symbols.done),
                roles.green,
            ),
            _ => (message, roles.text),
        };
        Some(Line::from(Span::styled(
            truncate_text(&text, usize::from(width)),
            Style::default().fg(color),
        )))
    }

    /// When `entry` was made, for a sentence: "today at 09:15", "Wed 11 Nov
    /// at 20:33". Says UTC when the local time zone is unknown.
    pub(super) fn backup_when_phrase(&self, entry: &BackupEntry) -> String {
        let offset = local_offset();
        let when = backup_when(
            entry.made_at,
            OffsetDateTime::now_utc(),
            offset.unwrap_or(UtcOffset::UTC),
        );
        let when = mid_sentence(when);
        if offset.is_none() {
            format!("{when} UTC")
        } else {
            when
        }
    }

    /// "2 hours ago", for the questions.
    pub(super) fn backup_age_phrase(&self, entry: &BackupEntry) -> String {
        time_ago(OffsetDateTime::now_utc(), entry.made_at)
    }
}

/// The local time zone, read once. `None` where it can't be read (on Linux
/// once other threads are running); times are then shown in UTC and say so.
fn local_offset() -> Option<UtcOffset> {
    static OFFSET: OnceLock<Option<UtcOffset>> = OnceLock::new();
    *OFFSET.get_or_init(|| UtcOffset::current_local_offset().ok())
}

/// A backup's time in plain words, in `offset`: "Today at 09:15",
/// "Yesterday at 20:33", "Wed 11 Nov at 20:33" this year, and
/// "5 Nov 2025 at 17:42" before it.
pub(super) fn backup_when(at: OffsetDateTime, now: OffsetDateTime, offset: UtcOffset) -> String {
    let at = at.to_offset(offset);
    let today = now.to_offset(offset).date();
    let date = at.date();
    let clock = format!("{:02}:{:02}", at.hour(), at.minute());
    let month = at.month().to_string();
    let month = &month[..3];
    if date == today {
        format!("Today at {clock}")
    } else if today.previous_day() == Some(date) {
        format!("Yesterday at {clock}")
    } else if date.year() == today.year() {
        let weekday = at.weekday().to_string();
        format!("{} {} {month} at {clock}", &weekday[..3], date.day())
    } else {
        format!("{} {month} {} at {clock}", date.day(), date.year())
    }
}

/// "Today at 09:15" as it reads inside a sentence: "today at 09:15".
fn mid_sentence(when: String) -> String {
    if when.starts_with("Today") || when.starts_with("Yesterday") {
        let mut chars = when.chars();
        chars
            .next()
            .map(|first| first.to_lowercase().chain(chars).collect())
            .unwrap_or(when)
    } else {
        when
    }
}

fn format_backup_size(size_bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if size_bytes >= GIB {
        format!("{:.1} GB", size_bytes as f64 / GIB as f64)
    } else if size_bytes >= MIB {
        format!("{:.1} MB", size_bytes as f64 / MIB as f64)
    } else if size_bytes >= KIB {
        format!("{:.1} KB", size_bytes as f64 / KIB as f64)
    } else {
        format!("{size_bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use time::macros::{datetime, offset};

    use super::*;

    const NOW: OffsetDateTime = datetime!(2026-11-15 14:30 UTC);

    #[test]
    fn today_and_yesterday_in_words() {
        let when = |at| backup_when(at, NOW, UtcOffset::UTC);
        assert_eq!(when(datetime!(2026-11-15 14:02 UTC)), "Today at 14:02");
        assert_eq!(when(datetime!(2026-11-15 00:00 UTC)), "Today at 00:00");
        assert_eq!(when(datetime!(2026-11-14 23:59 UTC)), "Yesterday at 23:59");
        assert_eq!(when(datetime!(2026-11-14 09:15 UTC)), "Yesterday at 09:15");
    }

    #[test]
    fn older_backups_show_the_date_and_the_year_once_it_changes() {
        let when = |at| backup_when(at, NOW, UtcOffset::UTC);
        assert_eq!(when(datetime!(2026-11-11 20:33 UTC)), "Wed 11 Nov at 20:33");
        assert_eq!(when(datetime!(2026-11-05 17:42 UTC)), "Thu 5 Nov at 17:42");
        assert_eq!(when(datetime!(2026-01-01 08:00 UTC)), "Thu 1 Jan at 08:00");
        assert_eq!(
            when(datetime!(2025-11-12 09:15 UTC)),
            "12 Nov 2025 at 09:15"
        );
    }

    #[test]
    fn days_follow_the_local_time_zone() {
        // 23:30 UTC on the 14th is already the 15th, 01:30, two hours east.
        let east = offset!(+2);
        assert_eq!(
            backup_when(datetime!(2026-11-14 23:30 UTC), NOW, east),
            "Today at 01:30"
        );
        // And 01:00 UTC on the 15th is still the 14th five hours west.
        let west = offset!(-5);
        assert_eq!(
            backup_when(datetime!(2026-11-15 01:00 UTC), NOW, west),
            "Yesterday at 20:00"
        );
    }

    #[test]
    fn sentences_lower_case_today_and_yesterday_only() {
        assert_eq!(mid_sentence("Today at 09:15".to_string()), "today at 09:15");
        assert_eq!(
            mid_sentence("Yesterday at 09:15".to_string()),
            "yesterday at 09:15"
        );
        assert_eq!(
            mid_sentence("Wed 11 Nov at 20:33".to_string()),
            "Wed 11 Nov at 20:33"
        );
    }

    #[test]
    fn sizes_in_kb_and_mb() {
        assert_eq!(format_backup_size(512), "512 B");
        assert_eq!(format_backup_size(1946), "1.9 KB");
        assert_eq!(format_backup_size(5 * 1024 * 1024 + 300_000), "5.3 MB");
    }
}
