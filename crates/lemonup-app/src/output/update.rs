//! `lemonup update` and `lemonup update-all`: what the run did to each addon.

use std::path::PathBuf;

use lemonup_core::{AddonRecord, SourceKind};
use serde::{Serialize, Serializer};

use super::{
    Entry, Paint, Report, Tone, context_line, count_headings, display_version, rows_by_folder,
    section, source_label, table,
};
use crate::backup::PreUpdateBackup;
use crate::find::FindTarget;
use crate::update::{
    LiveUpdateResult, LiveUpdateStatus, LiveUpdateSummary, serialize_live_update_status,
};

/// What `lemonup update` and `lemonup update-all` print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UpdateReport {
    #[serde(skip)]
    target: Option<FindTarget>,
    /// Whether addons were named on the command line: then every one gets a row.
    #[serde(skip)]
    named: bool,
    force: bool,
    dry_run: bool,
    backup: BackupLine,
    summary: UpdateSummary,
    addons: Vec<UpdateRow>,
}

/// What the backup of the game settings before the run did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum BackupLine {
    Created {
        #[serde(serialize_with = "super::lossy_path_buf")]
        path: PathBuf,
    },
    Skipped {
        minutes_ago: i64,
    },
    Failed {
        error: String,
    },
    /// The setting is off.
    Disabled,
    /// A dry run: no backup is made, whatever the setting.
    NotRun,
}

impl BackupLine {
    /// What `backup` did. `None` is a dry run, which makes no backup.
    pub(crate) fn of(backup: Option<&PreUpdateBackup>) -> Self {
        match backup {
            None => Self::NotRun,
            Some(PreUpdateBackup::Disabled) => Self::Disabled,
            Some(PreUpdateBackup::Created { path }) => Self::Created { path: path.clone() },
            Some(PreUpdateBackup::Skipped { minutes_ago }) => Self::Skipped {
                minutes_ago: *minutes_ago,
            },
            Some(PreUpdateBackup::Warning(error)) => Self::Failed {
                error: error.clone(),
            },
        }
    }

    /// The warning a failed backup gives. The update goes ahead without one.
    pub(crate) fn warning(&self) -> Option<String> {
        match self {
            Self::Failed { error } => Some(format!(
                "Couldn't make a backup of your game settings: {error}"
            )),
            _ => None,
        }
    }

    /// The line of the readable report, when there is one to show.
    pub(crate) fn text(&self, paint: Paint) -> Option<String> {
        match self {
            Self::Created { path } => Some(format!(
                "{} {}",
                paint.text("Made a backup of your game settings:"),
                paint.path(path)
            )),
            Self::Skipped { minutes_ago } => Some(paint.text(&format!(
                "Backup of your game settings skipped, the last one is {minutes_ago} min old"
            ))),
            Self::Failed { .. } | Self::Disabled | Self::NotRun => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct UpdateSummary {
    tracked_addons: usize,
    updated: usize,
    up_to_date: usize,
    remote_older: usize,
    held: usize,
    skipped_manual: usize,
    skipped_no_api_key: usize,
    errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct UpdateRow {
    name: String,
    folder: String,
    #[serde(serialize_with = "super::serialize_source")]
    source: SourceKind,
    #[serde(serialize_with = "serialize_status")]
    status: LiveUpdateStatus,
    previous_version: Option<String>,
    remote_version: Option<String>,
    message: Option<String>,
    /// The "Not linked" folders a held update would replace.
    replaces: Vec<String>,
    /// Those of `replaces` that hold a `.git` entry. The readable note marks them.
    #[serde(skip)]
    dev_copies: Vec<String>,
}

fn serialize_status<S: Serializer>(
    status: &LiveUpdateStatus,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(serialize_live_update_status(*status))
}

/// How a run was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateFlags {
    /// Addons were named on the command line.
    pub(crate) named: bool,
    pub(crate) force: bool,
    pub(crate) dry_run: bool,
}

impl UpdateReport {
    /// The report of a run. `addons` are the profile's rows before it, joined to `results` by
    /// folder. A held result carries the "Not linked" folders it would replace.
    pub(crate) fn new(
        target: Option<FindTarget>,
        flags: UpdateFlags,
        backup: BackupLine,
        summary: &LiveUpdateSummary,
        results: &[LiveUpdateResult],
        addons: &[AddonRecord],
    ) -> Self {
        let by_folder = rows_by_folder(addons);
        let mut rows = results
            .iter()
            .map(|result| {
                let addon = by_folder.get(&result.addon_name.to_lowercase());
                UpdateRow {
                    name: addon.map_or_else(
                        || result.addon_name.clone(),
                        |addon| addon.display_name().to_string(),
                    ),
                    folder: result.addon_name.clone(),
                    source: result.source,
                    status: result.status,
                    previous_version: result.previous_version.clone(),
                    remote_version: result.remote_version.clone(),
                    message: result.message.clone(),
                    replaces: result.not_linked.clone(),
                    dev_copies: result.dev_copies.clone(),
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.name.to_lowercase());
        Self {
            target,
            named: flags.named,
            force: flags.force,
            dry_run: flags.dry_run,
            backup,
            summary: UpdateSummary {
                tracked_addons: addons.len(),
                updated: summary.updated_addons,
                up_to_date: summary.up_to_date,
                remote_older: summary.remote_older,
                held: summary.held,
                skipped_manual: summary.skipped_manual,
                skipped_no_api_key: summary.skipped_no_api_key,
                errors: summary.errors,
            },
            addons: rows,
        }
    }

    fn closing(&self) -> String {
        if self.dry_run {
            return "Dry run - no changes".to_string();
        }
        let parts = [
            (self.summary.updated, "updated"),
            (self.summary.held, "held"),
            (self.summary.errors, "failed"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect::<Vec<_>>();
        if !parts.is_empty() {
            return format!("{}.", parts.join(", "));
        }
        let summary = &self.summary;
        let linked = summary.up_to_date + summary.remote_older + summary.skipped_no_api_key;
        if linked == 0 && !self.named {
            return "No linked addons to update.".to_string();
        }
        // "Up to date" is only true of addons the run could compare with their source.
        let all_up_to_date = summary.up_to_date > 0
            && summary.remote_older == 0
            && summary.skipped_no_api_key == 0
            && (!self.named || summary.skipped_manual == 0);
        if all_up_to_date {
            "Everything is up to date.".to_string()
        } else {
            "Nothing was updated.".to_string()
        }
    }
}

fn entry(row: &UpdateRow, version: String, notes: Vec<String>) -> Entry {
    Entry {
        name: row.name.clone(),
        source: source_label(row.source).to_string(),
        version,
        update: None,
        notes,
        hint: None,
        from: None,
        done: false,
    }
}

/// What a held row says: every "Not linked" folder the update would replace, with developer
/// copies marked, and what to run to replace them.
fn held_note(row: &UpdateRow) -> (Option<String>, Option<String>) {
    if row.replaces.is_empty() {
        return (row.message.clone(), None);
    }
    (
        Some(format!(
            "Would replace Not linked {}: {}.",
            if row.replaces.len() == 1 {
                "folder"
            } else {
                "folders"
            },
            crate::update::describe_not_linked_folders(&row.replaces, &row.dev_copies)
        )),
        Some("Run again with `--replace`.".to_string()),
    )
}

impl Report for UpdateReport {
    fn command(&self) -> &'static str {
        "update"
    }

    /// The context line, the backup line, then a section for each outcome that has addons:
    /// Updated (Would update in a dry run), Held, Newer than source and Failed, then Up to
    /// date, Not linked and the addons skipped for a missing Wago.io key, which only count
    /// unless addons were named.
    fn human(&self, paint: Paint) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(target) = &self.target {
            lines.push(context_line(
                paint,
                &target.game,
                &target.addon_dir,
                &target.profile,
            ));
        }
        if let Some(text) = self.backup.text(paint) {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(text);
        }
        let shown = |status: LiveUpdateStatus| {
            self.addons
                .iter()
                .filter(move |row| row.status == status)
                .collect::<Vec<_>>()
        };
        // Without the addon's own name in front, which the row already shows.
        let short = |row: &UpdateRow, version: Option<&str>| {
            display_version(version, &[&row.name, &row.folder])
        };
        let previous = |row: &UpdateRow| short(row, row.previous_version.as_deref());
        let remote = |row: &UpdateRow| short(row, row.remote_version.as_deref());
        // `previous → remote`, the old version in red and the new one in green.
        let change = |row: &UpdateRow, notes: Vec<String>| Entry {
            from: Some(previous(row)),
            ..entry(row, remote(row), notes)
        };
        let add = |lines: &mut Vec<String>, tone, title: &str, entries: Vec<Entry>| {
            if !entries.is_empty() {
                let count = entries.len();
                section(lines, paint, tone, title, count, table(entries, paint));
            }
        };
        add(
            &mut lines,
            Tone::Good,
            if self.dry_run {
                "Would update"
            } else {
                "Updated"
            },
            shown(LiveUpdateStatus::Updated)
                .iter()
                .map(|row| Entry {
                    // A real update is done; a dry run only says what would be.
                    done: !self.dry_run,
                    ..change(row, row.message.iter().cloned().collect())
                })
                .collect(),
        );
        add(
            &mut lines,
            Tone::Warn,
            "Held",
            shown(LiveUpdateStatus::Held)
                .iter()
                .map(|row| {
                    let (note, hint) = held_note(row);
                    Entry {
                        hint,
                        ..change(row, note.into_iter().collect())
                    }
                })
                .collect(),
        );
        add(
            &mut lines,
            Tone::Warn,
            "Newer than source",
            shown(LiveUpdateStatus::RemoteOlder)
                .iter()
                .map(|row| {
                    let version = format!(
                        "{}, {} has {}",
                        previous(row),
                        source_label(row.source),
                        remote(row)
                    );
                    Entry {
                        hint: Some("Run again with `--force` to install it.".to_string()),
                        ..entry(row, version, Vec::new())
                    }
                })
                .collect(),
        );
        add(
            &mut lines,
            Tone::Problem,
            "Failed",
            shown(LiveUpdateStatus::Error)
                .iter()
                .map(|row| entry(row, previous(row), row.message.iter().cloned().collect()))
                .collect(),
        );
        let quiet = [
            ("Up to date", LiveUpdateStatus::UpToDate),
            ("Not linked", LiveUpdateStatus::SkippedManual),
            ("Skipped, no Wago.io key", LiveUpdateStatus::SkippedNoApiKey),
        ];
        if self.named {
            for (title, status) in quiet {
                add(
                    &mut lines,
                    Tone::Quiet,
                    title,
                    shown(status)
                        .iter()
                        .map(|row| entry(row, previous(row), Vec::new()))
                        .collect(),
                );
            }
        } else {
            let counts = quiet.map(|(title, status)| (title, shown(status).len()));
            count_headings(&mut lines, paint, Tone::Quiet, &counts);
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        let closing = self.closing();
        lines.push(if self.dry_run {
            paint.attention(&closing)
        } else {
            paint.text(&closing)
        });
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{AddonRecord, SourceKind};

    use super::{BackupLine, UpdateFlags, UpdateReport};
    use crate::backup::PreUpdateBackup;
    use crate::find::FindTarget;
    use crate::output::{Output, OutputMode, Paint, Report};
    use crate::update::{LiveUpdateResult, LiveUpdateStatus, LiveUpdateSummary};

    fn addon(name: &str, folder: &str, source: SourceKind) -> AddonRecord {
        AddonRecord::new(name, folder, source)
    }

    fn result(
        folder: &str,
        source: SourceKind,
        status: LiveUpdateStatus,
        versions: (Option<&str>, Option<&str>),
    ) -> LiveUpdateResult {
        LiveUpdateResult {
            addon_name: folder.to_string(),
            source,
            status,
            previous_version: versions.0.map(str::to_string),
            remote_version: versions.1.map(str::to_string),
            message: None,
            not_linked: Vec::new(),
            dev_copies: Vec::new(),
        }
    }

    fn target() -> FindTarget {
        FindTarget {
            game: "Retail".to_string(),
            profile: "default".to_string(),
            addon_dir: PathBuf::from("/games/wow/_retail_/Interface/AddOns"),
        }
    }

    const ALL: UpdateFlags = UpdateFlags {
        named: false,
        force: false,
        dry_run: false,
    };

    /// A run with each outcome: updated, held, newer than source, failed and the quiet ones.
    fn mixed() -> (LiveUpdateSummary, Vec<LiveUpdateResult>, Vec<AddonRecord>) {
        use LiveUpdateStatus::{
            Error, Held, RemoteOlder, SkippedManual, SkippedNoApiKey, UpToDate, Updated,
        };
        let addons = vec![
            addon("Deadly Boss Mods", "DBM-Core", SourceKind::Wago),
            addon("Details", "Details", SourceKind::GitHub),
            addon("ElvUI", "ElvUI", SourceKind::Tukui),
            addon("Pawn", "Pawn", SourceKind::WowInterface),
            addon("Bagnon", "Bagnon", SourceKind::WowInterface),
            addon("Questie", "Questie", SourceKind::GitHub),
            addon("HandyNotes", "HandyNotes", SourceKind::Manual),
            addon("WeakAuras", "WeakAuras", SourceKind::Wago),
        ];
        let mut dbm = result(
            "DBM-Core",
            SourceKind::Wago,
            Updated,
            (Some("12.0.8"), Some("12.0.10")),
        );
        dbm.message = Some("WoWInterface may not support WoW Forever".to_string());
        let mut held = result(
            "ElvUI",
            SourceKind::Tukui,
            Held,
            (Some("14.04"), Some("14.05")),
        );
        held.message =
            Some("would replace Not linked folders: ElvUI_Options, ElvUI_Libraries".to_string());
        held.not_linked = vec!["ElvUI_Options".to_string(), "ElvUI_Libraries".to_string()];
        let mut older = result(
            "Pawn",
            SourceKind::WowInterface,
            RemoteOlder,
            (Some("2.11.4"), Some("2.11.2")),
        );
        older.message = Some("remote version is older".to_string());
        let mut failed = result(
            "Bagnon",
            SourceKind::WowInterface,
            Error,
            (Some("11.0.1"), None),
        );
        failed.message = Some("WoWInterface didn't answer".to_string());
        let results = vec![
            dbm,
            result(
                "Details",
                SourceKind::GitHub,
                Updated,
                (Some("v1.2"), Some("v1.3")),
            ),
            held,
            older,
            failed,
            result(
                "Questie",
                SourceKind::GitHub,
                UpToDate,
                (Some("10.1"), Some("10.1")),
            ),
            result(
                "HandyNotes",
                SourceKind::Manual,
                SkippedManual,
                (Some("1.0"), None),
            ),
            result(
                "WeakAuras",
                SourceKind::Wago,
                SkippedNoApiKey,
                (Some("5.0"), None),
            ),
        ];
        let summary = LiveUpdateSummary {
            target_addons: 8,
            updated_addons: 2,
            up_to_date: 1,
            skipped_manual: 1,
            skipped_no_api_key: 1,
            remote_older: 1,
            held: 1,
            errors: 1,
        };
        (summary, results, addons)
    }

    fn report(flags: UpdateFlags, backup: BackupLine) -> UpdateReport {
        let (summary, results, addons) = mixed();
        UpdateReport::new(Some(target()), flags, backup, &summary, &results, &addons)
    }

    #[cfg(unix)]
    #[test]
    fn the_backup_is_bold_and_a_link_where_the_terminal_has_links() {
        let backup = BackupLine::Created {
            path: PathBuf::from("/data/backups/retail/WTF-2026-11-22-2140.zip"),
        };
        assert_eq!(
            backup.text(Paint::terminal()).as_deref(),
            Some(
                "Made a backup of your game settings: \
                 \x1b[1m/data/backups/retail/WTF-2026-11-22-2140.zip\x1b[0m"
            )
        );
        assert_eq!(
            backup.text(Paint::terminal().with_links(true)).as_deref(),
            Some(
                "Made a backup of your game settings: \x1b]8;;file:///data/backups/retail/WTF-2026-11-22-2140.zip\x1b\\\x1b[1m/data/backups/retail/WTF-2026-11-22-2140.zip\x1b[0m\x1b]8;;\x1b\\"
            )
        );
        let lines = report(ALL, backup).human(Paint::terminal().with_links(true));
        assert!(
            lines[2].contains("\x1b]8;;file:///data/backups/"),
            "{lines:?}"
        );
    }

    #[test]
    fn every_outcome_has_its_section_and_the_quiet_ones_only_count() {
        let backup = BackupLine::Created {
            path: PathBuf::from("/data/backups/retail/WTF-2026-11-22-2140.zip"),
        };
        assert_eq!(
            report(ALL, backup).human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Made a backup of your game settings: /data/backups/retail/WTF-2026-11-22-2140.zip",
                "",
                "Updated (2)",
                "  ✓ Deadly Boss Mods  Wago.io  12.0.8 → 12.0.10",
                "        WoWInterface may not support WoW Forever",
                "  ✓ Details           GitHub   v1.2 → v1.3",
                "",
                "Held (1)",
                "  ElvUI  TukUI  14.04 → 14.05",
                "      Would replace Not linked folders: ElvUI_Options, ElvUI_Libraries.",
                "      Run again with `--replace`.",
                "",
                "Newer than source (1)",
                "  Pawn  WoWInterface  2.11.4, WoWInterface has 2.11.2",
                "      Run again with `--force` to install it.",
                "",
                "Failed (1)",
                "  Bagnon  WoWInterface  11.0.1",
                "      WoWInterface didn't answer",
                "",
                "Up to date (1)",
                "Not linked (1)",
                "Skipped, no Wago.io key (1)",
                "",
                "2 updated, 1 held, 1 failed.",
            ]
        );
    }

    #[test]
    fn a_dry_run_says_would_update_and_that_nothing_changed() {
        let flags = UpdateFlags {
            dry_run: true,
            ..ALL
        };
        let lines = report(flags, BackupLine::of(None)).human(Paint::PLAIN);
        assert_eq!(lines[0], "Retail  /games/wow/_retail_/Interface/AddOns");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "Would update (2)");
        assert!(
            !lines.iter().any(|line| line.contains("backup")),
            "{lines:?}"
        );
        assert_eq!(lines[lines.len() - 2..], ["", "Dry run - no changes"]);
        // Yellow, so a dry run is not taken for the real thing.
        let colored = report(flags, BackupLine::of(None)).human(Paint::terminal());
        assert_eq!(
            colored.last().map(String::as_str),
            Some("\x1b[38;5;3mDry run - no changes\x1b[0m")
        );
        // Nothing is done yet, so no row has the done mark.
        assert_eq!(
            lines[3..6],
            [
                "  Deadly Boss Mods  Wago.io  12.0.8 → 12.0.10",
                "      WoWInterface may not support WoW Forever",
                "  Details           GitHub   v1.2 → v1.3",
            ]
        );
        // Nothing else changes.
        let real = report(ALL, BackupLine::Disabled).human(Paint::PLAIN);
        assert_eq!(lines[6..lines.len() - 1], real[6..real.len() - 1]);
    }

    #[test]
    fn versions_change_from_red_to_green_and_updated_rows_get_a_green_mark() {
        let lines = report(ALL, BackupLine::Disabled).human(Paint::terminal());
        let red = "\x1b[38;5;1m";
        let green = "\x1b[38;5;2m";
        let reset = "\x1b[0m";
        let details = lines
            .iter()
            .find(|line| line.contains("Details"))
            .expect("details");
        assert!(
            details.starts_with(&format!("  {green}✓{reset} Details")),
            "{details:?}"
        );
        assert!(
            details.ends_with(&format!("{red}v1.2{reset} → {green}v1.3{reset}")),
            "{details:?}"
        );
        // A held update changes versions too, but is not done.
        let held = lines
            .iter()
            .find(|line| line.contains("ElvUI"))
            .expect("held");
        assert!(held.starts_with("  ElvUI"), "{held:?}");
        assert!(
            held.ends_with(&format!("{red}14.04{reset} → {green}14.05{reset}")),
            "{held:?}"
        );
        // Simple symbols draw the mark and the arrow in ASCII.
        let simple = report(ALL, BackupLine::Disabled)
            .human(Paint::PLAIN.with_symbols(crate::theme::Symbols::SIMPLE));
        assert!(
            simple.contains(&"  ok Details           GitHub   v1.2 -> v1.3".to_string()),
            "{simple:?}"
        );
    }

    #[test]
    fn a_reinstall_of_the_same_version_shows_no_change() {
        let addons = vec![addon("Details", "Details", SourceKind::GitHub)];
        let results = vec![result(
            "Details",
            SourceKind::GitHub,
            LiveUpdateStatus::Updated,
            (Some("v1.2"), Some("v1.2")),
        )];
        let summary = LiveUpdateSummary {
            updated_addons: 1,
            ..LiveUpdateSummary::default()
        };
        let lines = UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &results, &addons)
            .human(Paint::terminal());
        // Done, so it has the mark, but one version with no arrow and no red.
        assert_eq!(
            lines[1],
            "  \x1b[38;5;2m✓\x1b[0m Details  \x1b[2mGitHub\x1b[0m  v1.2"
        );
    }

    #[test]
    fn a_version_drops_the_addons_own_name() {
        let addons = vec![addon("Pawn", "Pawn", SourceKind::Wago)];
        let results = vec![result(
            "Pawn",
            SourceKind::Wago,
            LiveUpdateStatus::Updated,
            (Some("Pawn 2.14.4"), Some("Pawn 2.14.8")),
        )];
        let summary = LiveUpdateSummary {
            updated_addons: 1,
            ..LiveUpdateSummary::default()
        };
        let report =
            UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &results, &addons);
        assert_eq!(
            report.human(Paint::PLAIN)[1],
            "  ✓ Pawn  Wago.io  2.14.4 → 2.14.8"
        );
        // The JSON keeps the versions as the source wrote them.
        let document = json(&report);
        assert!(
            document.contains(r#""previous_version":"Pawn 2.14.4","remote_version":"Pawn 2.14.8""#),
            "{document}"
        );
    }

    #[test]
    fn each_backup_outcome_has_its_line_or_none() {
        let line = |backup: BackupLine| report(ALL, backup).human(Paint::PLAIN)[2].clone();
        assert_eq!(
            line(BackupLine::of(Some(&PreUpdateBackup::Created {
                path: PathBuf::from("/data/WTF.zip")
            }))),
            "Made a backup of your game settings: /data/WTF.zip"
        );
        assert_eq!(
            line(BackupLine::of(Some(&PreUpdateBackup::Skipped {
                minutes_ago: 12
            }))),
            "Backup of your game settings skipped, the last one is 12 min old"
        );
        // No line: the next one is the first heading.
        assert_eq!(
            line(BackupLine::of(Some(&PreUpdateBackup::Disabled))),
            "Updated (2)"
        );
        let failed = BackupLine::of(Some(&PreUpdateBackup::Warning(
            "WTF folder missing".to_string(),
        )));
        assert_eq!(
            failed.warning().as_deref(),
            Some("Couldn't make a backup of your game settings: WTF folder missing")
        );
        assert_eq!(line(failed), "Updated (2)");
        assert_eq!(BackupLine::Disabled.warning(), None);
    }

    #[test]
    fn named_addons_each_get_a_row_quiet_ones_included() {
        let flags = UpdateFlags { named: true, ..ALL };
        let lines = report(flags, BackupLine::Disabled).human(Paint::PLAIN);
        let start = lines
            .iter()
            .position(|line| line == "Up to date (1)")
            .expect("up to date");
        assert_eq!(
            lines[start..],
            [
                "Up to date (1)",
                "  Questie  GitHub  10.1",
                "",
                "Not linked (1)",
                "  HandyNotes  Not linked  1.0",
                "",
                "Skipped, no Wago.io key (1)",
                "  WeakAuras  Wago.io  5.0",
                "",
                "2 updated, 1 held, 1 failed.",
            ]
        );
    }

    #[test]
    fn a_held_row_names_its_folders_in_a_dry_run_and_with_names_given() {
        for flags in [
            UpdateFlags {
                dry_run: true,
                ..ALL
            },
            UpdateFlags { named: true, ..ALL },
        ] {
            let lines = report(flags, BackupLine::Disabled).human(Paint::PLAIN.with_width(200));
            let at = lines
                .iter()
                .position(|line| line == "Held (1)")
                .expect("held");
            assert_eq!(
                lines[at + 1..at + 4],
                [
                    "  ElvUI  TukUI  14.04 → 14.05",
                    "      Would replace Not linked folders: ElvUI_Options, ElvUI_Libraries.",
                    "      Run again with `--replace`.",
                ]
            );
        }
    }

    #[test]
    fn a_held_note_says_folder_for_one_folder() {
        let (summary, mut results, addons) = mixed();
        for result in &mut results {
            result.not_linked.truncate(1);
        }
        let lines = UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &results, &addons)
            .human(Paint::PLAIN.with_width(200));
        let at = lines
            .iter()
            .position(|line| line == "Held (1)")
            .expect("held");
        assert_eq!(
            lines[at + 2..at + 4],
            [
                "      Would replace Not linked folder: ElvUI_Options.",
                "      Run again with `--replace`.",
            ]
        );
    }

    fn closing(summary: LiveUpdateSummary) -> String {
        let lines = UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &[], &[])
            .human(Paint::PLAIN);
        assert_eq!(lines.len(), 1, "{lines:?}");
        lines[0].clone()
    }

    #[test]
    fn the_closing_line_names_only_what_happened() {
        let summary = |updated_addons, held, errors| LiveUpdateSummary {
            updated_addons,
            held,
            errors,
            up_to_date: 4,
            ..LiveUpdateSummary::default()
        };
        assert_eq!(closing(summary(0, 0, 0)), "Everything is up to date.");
        // Up to date is not claimed for an addon the run couldn't compare or left alone.
        assert_eq!(
            closing(LiveUpdateSummary {
                up_to_date: 4,
                remote_older: 1,
                ..LiveUpdateSummary::default()
            }),
            "Nothing was updated."
        );
        assert_eq!(
            closing(LiveUpdateSummary {
                up_to_date: 4,
                skipped_no_api_key: 1,
                ..LiveUpdateSummary::default()
            }),
            "Nothing was updated."
        );
        assert_eq!(
            closing(LiveUpdateSummary {
                skipped_manual: 3,
                ..LiveUpdateSummary::default()
            }),
            "No linked addons to update."
        );
        assert_eq!(
            closing(LiveUpdateSummary::default()),
            "No linked addons to update."
        );
        // With names given, a Not linked addon was asked for and left alone.
        let named = |summary: LiveUpdateSummary| {
            let flags = UpdateFlags { named: true, ..ALL };
            let lines = UpdateReport::new(None, flags, BackupLine::Disabled, &summary, &[], &[])
                .human(Paint::PLAIN);
            lines.last().expect("closing").clone()
        };
        assert_eq!(
            named(LiveUpdateSummary {
                skipped_manual: 1,
                ..LiveUpdateSummary::default()
            }),
            "Nothing was updated."
        );
        assert_eq!(
            named(LiveUpdateSummary {
                up_to_date: 1,
                skipped_manual: 1,
                ..LiveUpdateSummary::default()
            }),
            "Nothing was updated."
        );
        assert_eq!(
            named(LiveUpdateSummary {
                up_to_date: 2,
                ..LiveUpdateSummary::default()
            }),
            "Everything is up to date."
        );
        assert_eq!(closing(summary(3, 0, 0)), "3 updated.");
        assert_eq!(closing(summary(0, 1, 0)), "1 held.");
        assert_eq!(closing(summary(0, 0, 2)), "2 failed.");
        assert_eq!(closing(summary(1, 0, 2)), "1 updated, 2 failed.");
        assert_eq!(closing(summary(2, 1, 1)), "2 updated, 1 held, 1 failed.");
    }

    #[test]
    fn a_held_update_without_known_folders_shows_its_message() {
        let (summary, mut results, addons) = mixed();
        for result in &mut results {
            result.not_linked.clear();
        }
        let lines = UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &results, &addons)
            .human(Paint::PLAIN.with_width(200));
        let held = lines
            .iter()
            .position(|line| line == "Held (1)")
            .expect("held");
        assert_eq!(
            lines[held + 2],
            "      would replace Not linked folders: ElvUI_Options, ElvUI_Libraries"
        );
    }

    #[test]
    fn a_held_note_marks_developer_copies() {
        let (summary, mut results, addons) = mixed();
        for result in &mut results {
            if result.addon_name == "ElvUI" {
                result.dev_copies = vec!["ElvUI_Libraries".to_string()];
            }
        }
        let lines = UpdateReport::new(None, ALL, BackupLine::Disabled, &summary, &results, &addons)
            .human(Paint::PLAIN.with_width(200));
        let at = lines
            .iter()
            .position(|line| line == "Held (1)")
            .expect("held");
        assert_eq!(
            lines[at + 2..at + 4],
            [
                "      Would replace Not linked folders: ElvUI_Options, ElvUI_Libraries (has .git, \
                 likely a dev copy).",
                "      Run again with `--replace`.",
            ]
        );
    }

    fn json(report: &UpdateReport) -> String {
        let mut output = Output::new(
            OutputMode::Json,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            Vec::new(),
            Vec::new(),
        );
        output
            .print(
                report,
                Some(std::path::Path::new("/games/wow/_retail_/Interface/AddOns")),
            )
            .expect("print");
        let (stdout, stderr) = output.into_writers();
        assert!(stderr.is_empty());
        String::from_utf8(stdout).expect("utf-8")
    }

    #[test]
    fn the_json_document_lists_every_addon_with_what_held_ones_replace() {
        let backup = BackupLine::Created {
            path: PathBuf::from("/data/backups/retail/WTF-2026-11-22-2140.zip"),
        };
        assert_eq!(
            json(&report(ALL, backup)),
            concat!(
                r#"{"schema":1,"command":"update","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","force":false,"dry_run":false,"#,
                r#""backup":{"status":"created","path":"/data/backups/retail/WTF-2026-11-22-2140.zip"},"#,
                r#""summary":{"tracked_addons":8,"updated":2,"up_to_date":1,"remote_older":1,"held":1,"skipped_manual":1,"skipped_no_api_key":1,"errors":1},"#,
                r#""addons":["#,
                r#"{"name":"Bagnon","folder":"Bagnon","source":"wowinterface","status":"error","previous_version":"11.0.1","remote_version":null,"message":"WoWInterface didn't answer","replaces":[]},"#,
                r#"{"name":"Deadly Boss Mods","folder":"DBM-Core","source":"wago","status":"updated","previous_version":"12.0.8","remote_version":"12.0.10","message":"WoWInterface may not support WoW Forever","replaces":[]},"#,
                r#"{"name":"Details","folder":"Details","source":"github","status":"updated","previous_version":"v1.2","remote_version":"v1.3","message":null,"replaces":[]},"#,
                r#"{"name":"ElvUI","folder":"ElvUI","source":"tukui","status":"held","previous_version":"14.04","remote_version":"14.05","message":"would replace Not linked folders: ElvUI_Options, ElvUI_Libraries","replaces":["ElvUI_Options","ElvUI_Libraries"]},"#,
                r#"{"name":"HandyNotes","folder":"HandyNotes","source":"manual","status":"skipped_manual","previous_version":"1.0","remote_version":null,"message":null,"replaces":[]},"#,
                r#"{"name":"Pawn","folder":"Pawn","source":"wowinterface","status":"remote_older","previous_version":"2.11.4","remote_version":"2.11.2","message":"remote version is older","replaces":[]},"#,
                r#"{"name":"Questie","folder":"Questie","source":"github","status":"up_to_date","previous_version":"10.1","remote_version":"10.1","message":null,"replaces":[]},"#,
                r#"{"name":"WeakAuras","folder":"WeakAuras","source":"wago","status":"skipped_no_api_key","previous_version":"5.0","remote_version":null,"message":null,"replaces":[]}"#,
                r#"],"warnings":[]}"#,
                "\n"
            )
        );
    }

    #[test]
    fn the_json_backup_names_each_outcome() {
        let backup = |line: BackupLine| {
            let document: serde_json::Value =
                serde_json::from_str(&json(&report(ALL, line))).expect("json");
            document["backup"].clone()
        };
        assert_eq!(
            backup(BackupLine::of(Some(&PreUpdateBackup::Skipped {
                minutes_ago: 12
            }))),
            serde_json::json!({"status": "skipped", "minutes_ago": 12})
        );
        assert_eq!(
            backup(BackupLine::of(Some(&PreUpdateBackup::Warning(
                "WTF folder missing".to_string()
            )))),
            serde_json::json!({"status": "failed", "error": "WTF folder missing"})
        );
        assert_eq!(
            backup(BackupLine::of(Some(&PreUpdateBackup::Disabled))),
            serde_json::json!({"status": "disabled"})
        );
        // A dry run makes no backup, whatever the setting, and says so.
        assert_eq!(
            backup(BackupLine::of(None)),
            serde_json::json!({"status": "not_run"})
        );
        let dry = UpdateFlags {
            dry_run: true,
            ..ALL
        };
        let document = json(&report(dry, BackupLine::of(None)));
        assert!(
            document.contains(r#""force":false,"dry_run":true,"backup":{"status":"not_run"},"#),
            "{document}"
        );
    }
}
