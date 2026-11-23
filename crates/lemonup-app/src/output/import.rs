//! `lemonup import-addons`: what happened to each addon of the list.

use std::path::PathBuf;

use lemonup_core::{ImportAnalysis, TransferAddon, TransferSourceKind};
use serde::Serialize;

use super::{Entry, Paint, Report, Tone, context_line, count_headings, section, source_key, table};
use crate::find::FindTarget;
use crate::transfer::TransferInstallOutcome;

/// What `lemonup import-addons` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ImportReport {
    #[serde(skip)]
    target: Option<FindTarget>,
    #[serde(serialize_with = "super::lossy_path_buf")]
    file: PathBuf,
    dry_run: bool,
    summary: ImportSummary,
    addons: Vec<ImportRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ImportSummary {
    installed: usize,
    /// What a dry run would install. Always 0 for a real run.
    would_install: usize,
    already_installed: usize,
    manual: usize,
    duplicates: usize,
    unsupported: usize,
    errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ImportStatus {
    Installed,
    WouldInstall,
    AlreadyInstalled,
    SkippedManual,
    SkippedDuplicate,
    SkippedUnsupported,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ImportRow {
    name: String,
    /// `source_key`'s name, or the list's own word for a source this version doesn't know.
    source: String,
    /// The source as the TUI names it.
    #[serde(skip)]
    label: String,
    status: ImportStatus,
    message: Option<String>,
}

impl ImportRow {
    fn new(name: &str, source: &TransferSourceKind, status: ImportStatus) -> Self {
        let (key, label) = match source.as_source_kind() {
            Some(kind) => (source_key(kind), super::source_label(kind)),
            None => (source.as_str(), source.as_str()),
        };
        Self {
            name: name.to_string(),
            source: key.to_string(),
            label: label.to_string(),
            status,
            message: None,
        }
    }

    fn of(addon: &TransferAddon, status: ImportStatus) -> Self {
        Self::new(&addon.name, &addon.source, status)
    }
}

impl ImportReport {
    /// The report of an import from `file`. `outcomes` are the installs a real run made; a dry
    /// run has none and lists what it would install.
    pub(crate) fn new(
        target: Option<FindTarget>,
        file: PathBuf,
        dry_run: bool,
        analysis: &ImportAnalysis,
        outcomes: &[TransferInstallOutcome],
    ) -> Self {
        let mut rows = Vec::new();
        if dry_run {
            rows.extend(
                analysis
                    .to_install
                    .iter()
                    .map(|addon| ImportRow::of(addon, ImportStatus::WouldInstall)),
            );
        } else {
            rows.extend(outcomes.iter().map(|outcome| match &outcome.result {
                Ok(()) => ImportRow::new(&outcome.name, &outcome.source, ImportStatus::Installed),
                Err(error) => ImportRow {
                    message: Some(error.clone()),
                    ..ImportRow::new(&outcome.name, &outcome.source, ImportStatus::Error)
                },
            }));
        }
        for (list, status) in [
            (&analysis.already_installed, ImportStatus::AlreadyInstalled),
            (&analysis.manual_addons, ImportStatus::SkippedManual),
            (&analysis.duplicate_addons, ImportStatus::SkippedDuplicate),
            (
                &analysis.unsupported_addons,
                ImportStatus::SkippedUnsupported,
            ),
        ] {
            rows.extend(list.iter().map(|addon| ImportRow::of(addon, status)));
        }
        rows.sort_by_key(|row| row.name.to_lowercase());
        let count = |status: ImportStatus| rows.iter().filter(|row| row.status == status).count();
        let summary = ImportSummary {
            installed: count(ImportStatus::Installed),
            would_install: count(ImportStatus::WouldInstall),
            already_installed: analysis.already_installed.len(),
            manual: analysis.manual_addons.len(),
            duplicates: analysis.duplicate_addons.len(),
            unsupported: analysis.unsupported_addons.len(),
            errors: count(ImportStatus::Error),
        };
        Self {
            target,
            file,
            dry_run,
            summary,
            addons: rows,
        }
    }

    /// What happened, then the file in bold.
    fn closing(&self, paint: Paint) -> String {
        if self.dry_run {
            return paint.attention("Dry run - no changes");
        }
        let file = paint.path(&self.file);
        let parts = [
            (self.summary.installed, "installed"),
            (self.summary.errors, "failed"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect::<Vec<_>>();
        let words = if parts.is_empty() {
            "Nothing to install from ".to_string()
        } else {
            format!("{}, from ", parts.join(", "))
        };
        format!("{}{file}", paint.text(&words))
    }
}

impl Report for ImportReport {
    const COMMAND: &'static str = "import-addons";

    /// The context line, Installed (Would install in a dry run) and Failed with a row per
    /// addon, the headings that only count, then the closing line.
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
        let entries = |status: ImportStatus| {
            self.addons
                .iter()
                .filter(|row| row.status == status)
                .map(|row| Entry {
                    name: row.name.clone(),
                    source: row.label.clone(),
                    version: String::new(),
                    update: None,
                    notes: row.message.iter().cloned().collect(),
                    from: None,
                    done: status == ImportStatus::Installed,
                })
                .collect::<Vec<_>>()
        };
        let mut add = |tone, title: &str, entries: Vec<Entry>| {
            if !entries.is_empty() {
                let count = entries.len();
                section(&mut lines, paint, tone, title, count, table(entries, paint));
            }
        };
        if self.dry_run {
            add(
                Tone::Good,
                "Would install",
                entries(ImportStatus::WouldInstall),
            );
        } else {
            add(Tone::Good, "Installed", entries(ImportStatus::Installed));
        }
        add(Tone::Problem, "Failed", entries(ImportStatus::Error));
        count_headings(
            &mut lines,
            paint,
            Tone::Quiet,
            &[
                ("Already installed", self.summary.already_installed),
                ("Not linked, can't install", self.summary.manual),
                ("Listed twice", self.summary.duplicates),
                ("Can't install", self.summary.unsupported),
            ],
        );
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(self.closing(paint));
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{ImportAnalysis, TransferAddon, TransferSourceKind};

    use super::ImportReport;
    use crate::find::FindTarget;
    use crate::output::tests::{json_document, strip};
    use crate::output::{Paint, Report};
    use crate::theme::Symbols;
    use crate::transfer::TransferInstallOutcome;

    fn target() -> FindTarget {
        FindTarget {
            game: "Retail".to_string(),
            profile: "default".to_string(),
            addon_dir: PathBuf::from("/games/wow/_retail_/Interface/AddOns"),
        }
    }

    fn addon(name: &str, source: TransferSourceKind) -> TransferAddon {
        TransferAddon {
            name: name.to_string(),
            folder: name.to_string(),
            source,
            url: Some(format!("https://example.com/{name}")),
            owned_folders: None,
            reinstallable: true,
        }
    }

    /// A list with an addon of each kind: two to install, one installed already, one Not
    /// linked, one listed twice and one from a newer LemonUp.
    fn analysis() -> ImportAnalysis {
        use TransferSourceKind::{GitHub, Manual, Other, Wago, Wowinterface};
        ImportAnalysis {
            to_install: vec![
                addon("Details", GitHub),
                addon("Deadly Boss Mods", Wago),
                addon("Bagnon", Wowinterface),
            ],
            already_installed: vec![addon("Questie", GitHub)],
            manual_addons: vec![addon("HandyNotes", Manual)],
            duplicate_addons: vec![addon("Details Copy", GitHub)],
            unsupported_addons: vec![addon("Plater", Other("curseforge".to_string()))],
        }
    }

    fn outcomes() -> Vec<TransferInstallOutcome> {
        vec![
            TransferInstallOutcome {
                name: "Details".to_string(),
                source: TransferSourceKind::GitHub,
                result: Ok(()),
            },
            TransferInstallOutcome {
                name: "Deadly Boss Mods".to_string(),
                source: TransferSourceKind::Wago,
                result: Ok(()),
            },
            TransferInstallOutcome {
                name: "Bagnon".to_string(),
                source: TransferSourceKind::Wowinterface,
                result: Err("WoWInterface didn't answer. Try again later.".to_string()),
            },
        ]
    }

    fn file() -> PathBuf {
        PathBuf::from("/data/lemonup-addons.json")
    }

    fn report(dry_run: bool) -> ImportReport {
        let outcomes = if dry_run { Vec::new() } else { outcomes() };
        ImportReport::new(Some(target()), file(), dry_run, &analysis(), &outcomes)
    }

    #[test]
    fn installed_and_failed_get_rows_and_the_rest_only_count() {
        assert_eq!(
            report(false).human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Installed (2)",
                "  ✓ Deadly Boss Mods  Wago.io",
                "  ✓ Details           GitHub",
                "",
                "Failed (1)",
                "  Bagnon  WoWInterface",
                "      WoWInterface didn't answer. Try again later.",
                "",
                "Already installed (1)",
                "Not linked, can't install (1)",
                "Listed twice (1)",
                "Can't install (1)",
                "",
                "2 installed, 1 failed, from /data/lemonup-addons.json",
            ]
        );
    }

    #[test]
    fn a_dry_run_says_would_install_without_marks_and_that_nothing_changed() {
        let lines = report(true).human(Paint::PLAIN);
        assert_eq!(
            lines,
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Would install (3)",
                "  Bagnon            WoWInterface",
                "  Deadly Boss Mods  Wago.io",
                "  Details           GitHub",
                "",
                "Already installed (1)",
                "Not linked, can't install (1)",
                "Listed twice (1)",
                "Can't install (1)",
                "",
                "Dry run - no changes",
            ]
        );
    }

    #[test]
    fn the_closing_line_names_only_what_happened() {
        let closing = |outcomes: &[TransferInstallOutcome], analysis: &ImportAnalysis| {
            let lines =
                ImportReport::new(None, file(), false, analysis, outcomes).human(Paint::PLAIN);
            lines[lines.len() - 1].clone()
        };
        let all = outcomes();
        assert_eq!(
            closing(&all[..1], &analysis()),
            "1 installed, from /data/lemonup-addons.json"
        );
        assert_eq!(
            closing(&all[2..], &analysis()),
            "1 failed, from /data/lemonup-addons.json"
        );
        let empty = ImportAnalysis {
            to_install: Vec::new(),
            already_installed: Vec::new(),
            manual_addons: Vec::new(),
            duplicate_addons: Vec::new(),
            unsupported_addons: Vec::new(),
        };
        // An empty list without a folder is one line.
        assert_eq!(
            ImportReport::new(None, file(), false, &empty, &[]).human(Paint::PLAIN),
            ["Nothing to install from /data/lemonup-addons.json"]
        );
        assert_eq!(
            ImportReport::new(None, file(), true, &empty, &[]).human(Paint::PLAIN),
            ["Dry run - no changes"]
        );
        // Yellow, so a dry run is not taken for the real thing.
        assert_eq!(
            ImportReport::new(None, file(), true, &empty, &[]).human(Paint::terminal()),
            ["\x1b[38;5;3mDry run - no changes\x1b[0m"]
        );
    }

    #[test]
    fn installed_rows_get_a_green_mark_and_only_the_16_colors_are_used() {
        let colored = report(false).human(Paint::terminal());
        let joined = colored.join("\n");
        assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
        assert!(!joined.contains("48;"), "background: {joined:?}");
        assert!(
            colored[3].starts_with("  \x1b[38;5;2m✓\x1b[0m Deadly Boss Mods"),
            "{colored:?}"
        );
        assert_eq!(
            colored[6],
            "\x1b[1m\x1b[38;5;1mFailed\x1b[0m \x1b[2m(1)\x1b[0m"
        );
        // The file's path is bold in the closing line.
        assert!(
            colored
                .last()
                .expect("closing")
                .ends_with("from \x1b[1m/data/lemonup-addons.json\x1b[0m"),
            "{colored:?}"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, report(false).human(Paint::PLAIN));
        let simple = report(false).human(Paint::PLAIN.with_symbols(Symbols::SIMPLE));
        assert_eq!(simple[3], "  ok Deadly Boss Mods  Wago.io");
    }

    #[test]
    fn the_json_document_lists_every_addon_of_the_file_by_name() {
        let addon_dir = target().addon_dir;
        assert_eq!(
            json_document(&report(false), Some(&addon_dir), &[]),
            concat!(
                r#"{"schema":1,"command":"import-addons","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","#,
                r#""file":"/data/lemonup-addons.json","dry_run":false,"#,
                r#""summary":{"installed":2,"would_install":0,"already_installed":1,"manual":1,"duplicates":1,"unsupported":1,"errors":1},"#,
                r#""addons":["#,
                r#"{"name":"Bagnon","source":"wowinterface","status":"error","message":"WoWInterface didn't answer. Try again later."},"#,
                r#"{"name":"Deadly Boss Mods","source":"wago","status":"installed","message":null},"#,
                r#"{"name":"Details","source":"github","status":"installed","message":null},"#,
                r#"{"name":"Details Copy","source":"github","status":"skipped_duplicate","message":null},"#,
                r#"{"name":"HandyNotes","source":"manual","status":"skipped_manual","message":null},"#,
                r#"{"name":"Plater","source":"curseforge","status":"skipped_unsupported","message":null},"#,
                r#"{"name":"Questie","source":"github","status":"already_installed","message":null}"#,
                r#"],"warnings":[]}"#,
                "\n"
            )
        );
        let dry = json_document(&report(true), Some(&addon_dir), &[]);
        assert!(
            dry.contains(r#""dry_run":true,"summary":{"installed":0,"#),
            "{dry}"
        );
        assert!(
            dry.contains(r#"{"name":"Bagnon","source":"wowinterface","status":"would_install","message":null}"#),
            "{dry}"
        );
    }
}
