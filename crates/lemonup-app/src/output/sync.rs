//! `lemonup sync`: what reading the AddOns folder found.

use std::path::Path;

use lemonup_core::ScanSummary;
use serde::Serialize;

use super::{INDENT, Paint, Report, Tone, context_line, section, wrap_units};
use crate::find::FindTarget;

/// What `lemonup sync` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SyncReport {
    #[serde(skip)]
    target: FindTarget,
    summary: SyncSummary,
    /// The folders that couldn't be read. They were left as they were.
    skipped_folders: Vec<String>,
    /// Each `.lemonup-staging-*` folder an earlier update left behind, as a full path.
    leftover_staging: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct SyncSummary {
    scanned_addons: usize,
    upserted_addons: usize,
    removed_addons: usize,
    skipped: usize,
}

impl SyncReport {
    pub(crate) fn new(target: FindTarget, summary: &ScanSummary) -> Self {
        let leftover_staging = summary
            .leftover_staging
            .iter()
            .map(|name| target.addon_dir.join(name).display().to_string())
            .collect();
        Self {
            summary: SyncSummary {
                scanned_addons: summary.scanned_addons,
                upserted_addons: summary.upserted_addons,
                removed_addons: summary.removed_addons,
                skipped: summary.skipped_folders.len(),
            },
            skipped_folders: summary.skipped_folders.clone(),
            leftover_staging,
            target,
        }
    }

    /// The AddOns folder that was read.
    pub(crate) fn addon_dir(&self) -> &Path {
        &self.target.addon_dir
    }

    /// One warning per leftover update staging folder.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.leftover_staging
            .iter()
            .map(|path| {
                format!(
                    "Leftover update staging folder {path} may contain previous addon versions. \
                     See Troubleshooting."
                )
            })
            .collect()
    }

    /// How many addons the folder has, then what changed in LemonUp's list. The count is of
    /// addons as LemonUp lists them, not of folders: an addon can have several. Every addon
    /// read is written again, so the list is only called up to date, never "changed": it is
    /// when every folder could be read.
    fn closing(&self) -> String {
        let summary = &self.summary;
        let mut line = match summary.upserted_addons {
            1 => "1 addon found".to_string(),
            count => format!("{count} addons found"),
        };
        if summary.removed_addons > 0 {
            line.push_str(&format!(", {} removed", summary.removed_addons));
        }
        line.push_str(" - sync complete");
        line
    }
}

impl Report for SyncReport {
    const COMMAND: &'static str = "sync";

    /// The context line, the folders that couldn't be read, then the closing line.
    fn human(&self, paint: Paint) -> Vec<String> {
        let mut lines = vec![context_line(
            paint,
            &self.target.game,
            &self.target.addon_dir,
            &self.target.profile,
        )];
        if !self.skipped_folders.is_empty() {
            let last = self.skipped_folders.len() - 1;
            let names = self
                .skipped_folders
                .iter()
                .enumerate()
                .map(|(at, name)| {
                    if at == last {
                        name.clone()
                    } else {
                        format!("{name},")
                    }
                })
                .collect::<Vec<_>>();
            let body = wrap_units(INDENT, &names, paint.width())
                .iter()
                .map(|line| paint.text(line))
                .collect();
            section(
                &mut lines,
                paint,
                Tone::Problem,
                "Couldn't read",
                self.skipped_folders.len(),
                body,
            );
        }
        lines.push(String::new());
        lines.push(format!("{} {}", paint.done(), paint.text(&self.closing())));
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::ScanSummary;

    use super::SyncReport;
    use crate::find::FindTarget;
    use crate::output::tests::{json_document, strip};
    use crate::output::{Paint, Report};

    fn target(profile: &str) -> FindTarget {
        FindTarget {
            game: "Retail".to_string(),
            profile: profile.to_string(),
            addon_dir: PathBuf::from("/games/wow/_retail_/Interface/AddOns"),
        }
    }

    fn summary(scanned: usize, upserted: usize, removed: usize) -> ScanSummary {
        ScanSummary {
            scanned_addons: scanned,
            upserted_addons: upserted,
            removed_addons: removed,
            ..ScanSummary::default()
        }
    }

    fn lines(summary: &ScanSummary) -> Vec<String> {
        SyncReport::new(target("default"), summary).human(Paint::PLAIN)
    }

    #[test]
    fn a_folder_with_unreadable_addons_lists_them_and_what_was_removed() {
        let mut scan = summary(52, 52, 1);
        scan.skipped_folders = vec!["BrokenAddon".to_string()];
        assert_eq!(
            lines(&scan),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Couldn't read (1)",
                "  BrokenAddon",
                "",
                "✓ 52 addons found, 1 removed - sync complete",
            ]
        );
    }

    #[test]
    fn nothing_removed_and_everything_read_is_up_to_date() {
        assert_eq!(
            lines(&summary(52, 52, 0)),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "✓ 52 addons found - sync complete",
            ]
        );
        assert_eq!(
            lines(&summary(1, 1, 0))[2],
            "✓ 1 addon found - sync complete"
        );
        assert_eq!(
            lines(&summary(0, 0, 0))[2],
            "✓ 0 addons found - sync complete"
        );
        // 41 folders that make 38 addons: the count matches the addon list.
        assert_eq!(
            lines(&summary(41, 38, 0))[2],
            "✓ 38 addons found - sync complete"
        );
        assert_eq!(
            lines(&summary(3, 3, 2))[2],
            "✓ 3 addons found, 2 removed - sync complete"
        );
        // A folder that couldn't be read keeps its old row, so the list isn't called up to date.
        let mut scan = summary(3, 3, 0);
        scan.skipped_folders = vec!["Locked".to_string()];
        assert_eq!(lines(&scan)[5], "✓ 3 addons found - sync complete");
    }

    #[test]
    fn many_unreadable_folders_wrap_with_commas_and_another_profile_is_named() {
        let mut scan = summary(30, 30, 0);
        scan.skipped_folders = (1..=12).map(|at| format!("BrokenAddon{at:02}")).collect();
        let lines = SyncReport::new(target("forever"), &scan).human(Paint::PLAIN);
        assert_eq!(
            lines[..6],
            [
                "Retail  /games/wow/_retail_/Interface/AddOns  (profile forever)",
                "",
                "Couldn't read (12)",
                "  BrokenAddon01, BrokenAddon02, BrokenAddon03, BrokenAddon04, BrokenAddon05,",
                "  BrokenAddon06, BrokenAddon07, BrokenAddon08, BrokenAddon09, BrokenAddon10,",
                "  BrokenAddon11, BrokenAddon12",
            ]
        );
    }

    #[test]
    fn each_leftover_staging_folder_is_a_warning_with_its_path() {
        let mut scan = summary(2, 2, 0);
        scan.leftover_staging = vec![
            ".lemonup-staging-1-2-0".to_string(),
            ".lemonup-staging-3-4-0".to_string(),
        ];
        let report = SyncReport::new(target("default"), &scan);
        assert_eq!(
            report.warnings(),
            [
                "Leftover update staging folder \
                 /games/wow/_retail_/Interface/AddOns/.lemonup-staging-1-2-0 may contain \
                 previous addon versions. See Troubleshooting.",
                "Leftover update staging folder \
                 /games/wow/_retail_/Interface/AddOns/.lemonup-staging-3-4-0 may contain \
                 previous addon versions. See Troubleshooting.",
            ]
        );
        // The report itself doesn't mention them.
        assert_eq!(report.human(Paint::PLAIN).len(), 3);
        assert!(
            SyncReport::new(target("default"), &summary(1, 1, 0))
                .warnings()
                .is_empty()
        );
    }

    #[test]
    fn colored_lines_use_only_the_16_colors_and_keep_the_text() {
        let mut scan = summary(52, 52, 1);
        scan.skipped_folders = vec!["BrokenAddon".to_string()];
        let report = SyncReport::new(target("default"), &scan);
        let colored = report.human(Paint::terminal());
        let joined = colored.join("\n");
        assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
        assert!(!joined.contains("48;"), "background: {joined:?}");
        assert_eq!(
            colored[2],
            "\x1b[1m\x1b[38;5;1mCouldn't read\x1b[0m \x1b[2m(1)\x1b[0m"
        );
        assert_eq!(
            colored[5],
            "\x1b[38;5;2m✓\x1b[0m 52 addons found, 1 removed - sync complete"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, report.human(Paint::PLAIN));
    }

    #[test]
    fn the_json_document_has_the_summary_and_every_folder() {
        let mut scan = summary(52, 52, 1);
        scan.skipped_folders = vec!["BrokenAddon".to_string()];
        scan.leftover_staging = vec![".lemonup-staging-1-2-0".to_string()];
        let report = SyncReport::new(target("default"), &scan);
        assert_eq!(
            json_document(&report, Some(report.addon_dir()), &[]),
            concat!(
                r#"{"schema":1,"command":"sync","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","#,
                r#""summary":{"scanned_addons":52,"upserted_addons":52,"removed_addons":1,"skipped":1},"#,
                r#""skipped_folders":["BrokenAddon"],"#,
                r#""leftover_staging":["/games/wow/_retail_/Interface/AddOns/.lemonup-staging-1-2-0"],"#,
                r#""warnings":[]}"#,
                "\n"
            )
        );
    }
}
