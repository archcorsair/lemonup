//! `lemonup check`: what each linked addon's source has, by what it means.

use lemonup_core::{AddonRecord, DEFAULT_PROFILE, SourceKind, UpdateStatus};
use serde::{Serialize, Serializer};

use super::{
    Entry, Paint, Report, Tone, context_line, count_headings, rows_by_folder, section,
    short_version, source_label, table, version_text,
};
use crate::find::FindTarget;
use crate::source_error::SourceErrorKind;
use crate::update::{CheckResult, serialize_update_status};

/// What `lemonup check` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckReport {
    /// The game and AddOns folder checked, when one is known.
    #[serde(skip)]
    target: Option<FindTarget>,
    /// For the `update-all` hint.
    #[serde(skip)]
    profile_name: String,
    /// Whether addons were named on the command line: then every one gets a row.
    #[serde(skip)]
    named: bool,
    /// Whether the run was given `--addon-dir`. The update needs it again, so the hint says so.
    #[serde(skip)]
    with_addon_dir: bool,
    summary: CheckSummary,
    addons: Vec<CheckRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct CheckSummary {
    checked: usize,
    up_to_date: usize,
    update_available: usize,
    remote_older: usize,
    unknown: usize,
    errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CheckRow {
    name: String,
    folder: String,
    #[serde(serialize_with = "super::serialize_source")]
    source: SourceKind,
    #[serde(serialize_with = "serialize_status")]
    status: UpdateStatus,
    installed_version: Option<String>,
    remote_version: Option<String>,
    message: Option<String>,
    #[serde(serialize_with = "super::serialize_error_kind")]
    error_kind: Option<SourceErrorKind>,
}

fn serialize_status<S: Serializer>(
    status: &UpdateStatus,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(serialize_update_status(*status))
}

impl CheckReport {
    /// The report of `checks`, with names, sources and installed versions from `addons` (the
    /// rows of the same profile, joined by folder).
    pub(crate) fn new(
        target: Option<FindTarget>,
        profile: &str,
        named: bool,
        checks: &[CheckResult],
        addons: &[AddonRecord],
    ) -> Self {
        let by_folder = rows_by_folder(addons);
        let mut rows = checks
            .iter()
            .map(|check| {
                let addon = by_folder.get(&check.addon_name.to_lowercase());
                CheckRow {
                    name: addon.map_or_else(
                        || check.addon_name.clone(),
                        |addon| addon.display_name().to_string(),
                    ),
                    folder: check.addon_name.clone(),
                    source: addon.map_or(SourceKind::Manual, |addon| addon.source),
                    status: check.status,
                    installed_version: addon.and_then(|addon| addon.version.clone()),
                    remote_version: check.remote_version.clone(),
                    message: check.message.clone(),
                    error_kind: check.error_kind,
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.name.to_lowercase());
        let count = |status: UpdateStatus| rows.iter().filter(|row| row.status == status).count();
        let summary = CheckSummary {
            checked: rows.len(),
            up_to_date: count(UpdateStatus::UpToDate),
            update_available: count(UpdateStatus::UpdateAvailable),
            remote_older: count(UpdateStatus::RemoteOlder),
            unknown: count(UpdateStatus::Unknown),
            errors: count(UpdateStatus::Error),
        };
        Self {
            target,
            profile_name: profile.to_string(),
            named,
            with_addon_dir: false,
            summary,
            addons: rows,
        }
    }

    /// The same report for a run that was given `--addon-dir`: `update-all` alone would act
    /// on the profile's saved folder, or refuse when there is none.
    pub(crate) fn with_addon_dir(mut self, given: bool) -> Self {
        self.with_addon_dir = given;
        self
    }

    /// The command that installs the updates, for this profile.
    fn update_all_command(&self) -> String {
        if self.profile_name == DEFAULT_PROFILE {
            "lemonup update-all".to_string()
        } else {
            format!("lemonup --profile {} update-all", self.profile_name)
        }
    }

    /// What the check found, and the command that installs the updates. "Everything is up to
    /// date." only when nothing is ready, failed or still unchecked.
    fn closing(&self, updates: usize, newer: usize, failed: usize, not_checked: usize) -> String {
        let mut parts = Vec::new();
        match updates {
            0 => {}
            1 => parts.push("1 update ready".to_string()),
            count => parts.push(format!("{count} updates ready")),
        }
        if newer > 0 {
            parts.push(format!("{newer} newer than source"));
        }
        match failed {
            0 => {}
            1 => parts.push("1 check failed".to_string()),
            count => parts.push(format!("{count} checks failed")),
        }
        if not_checked > 0 {
            parts.push(format!("{not_checked} not checked yet"));
        }
        if parts.is_empty() {
            return "Everything is up to date.".to_string();
        }
        let mut closing = format!("{}.", parts.join(", "));
        if updates > 0 {
            closing.push_str(&format!(
                " Run `{}`{} to update{}.",
                self.update_all_command(),
                if self.with_addon_dir {
                    " with the same `--addon-dir`"
                } else {
                    ""
                },
                if updates == 1 { "" } else { " all" }
            ));
        }
        closing
    }
}

fn entry(row: &CheckRow, version: String, notes: Vec<String>) -> Entry {
    Entry {
        name: row.name.clone(),
        source: source_label(row.source).to_string(),
        version,
        update: None,
        notes,
        from: None,
        done: false,
    }
}

/// What a failed check says: the advice for its kind of failure, else its message.
fn failure_note(row: &CheckRow) -> Option<String> {
    let Some(kind) = row.error_kind else {
        return row.message.clone();
    };
    let mut note =
        crate::app::render_install_search::source_error_advice(kind, source_label(row.source));
    if kind == SourceErrorKind::Other
        && let Some(detail) = row.message.as_deref().filter(|detail| !detail.is_empty())
    {
        note.push_str(&format!(" Details: {detail}"));
    }
    Some(note)
}

impl Report for CheckReport {
    const COMMAND: &'static str = "check";

    /// The context line, then a section for each state that has addons: Update ready, Newer
    /// than source, Check failed, Not checked yet and Up to date, which only counts unless
    /// addons were named. Addons that are not linked are shown only when named.
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
        if !self.named
            && self
                .addons
                .iter()
                .all(|row| row.source == SourceKind::Manual)
        {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(paint.text("No linked addons to check."));
            return lines;
        }
        let shown = |status: UpdateStatus| {
            self.addons
                .iter()
                .filter(move |row| row.status == status)
                .filter(|row| self.named || row.source != SourceKind::Manual)
                .collect::<Vec<_>>()
        };
        // Without the addon's own name in front, which the row already shows.
        let short = |row: &CheckRow, version: Option<&str>| {
            short_version(version_text(version), &[&row.name, &row.folder]).to_string()
        };
        let installed = |row: &CheckRow| short(row, row.installed_version.as_deref());
        let remote = |row: &CheckRow| short(row, row.remote_version.as_deref());

        let add = |lines: &mut Vec<String>, tone, title: &str, entries: Vec<Entry>| {
            if !entries.is_empty() {
                let count = entries.len();
                section(lines, paint, tone, title, count, table(entries, paint));
            }
        };
        let ready = shown(UpdateStatus::UpdateAvailable);
        add(
            &mut lines,
            Tone::Warn,
            "Update ready",
            ready
                .iter()
                .map(|row| Entry {
                    from: Some(installed(row)),
                    ..entry(row, remote(row), Vec::new())
                })
                .collect(),
        );
        let newer = shown(UpdateStatus::RemoteOlder);
        add(
            &mut lines,
            Tone::Warn,
            "Newer than source",
            newer
                .iter()
                .map(|row| {
                    let version = format!(
                        "{}, {} has {}",
                        installed(row),
                        source_label(row.source),
                        remote(row)
                    );
                    entry(row, version, Vec::new())
                })
                .collect(),
        );
        let failed = shown(UpdateStatus::Error);
        add(
            &mut lines,
            Tone::Problem,
            "Check failed",
            failed
                .iter()
                .map(|row| entry(row, installed(row), failure_note(row).into_iter().collect()))
                .collect(),
        );
        let not_checked = shown(UpdateStatus::Unknown);
        add(
            &mut lines,
            Tone::Quiet,
            "Not checked yet",
            not_checked
                .iter()
                .map(|row| entry(row, installed(row), row.message.iter().cloned().collect()))
                .collect(),
        );
        let up_to_date = shown(UpdateStatus::UpToDate);
        if self.named {
            add(
                &mut lines,
                Tone::Quiet,
                "Up to date",
                up_to_date
                    .iter()
                    .map(|row| entry(row, installed(row), Vec::new()))
                    .collect(),
            );
        } else {
            count_headings(
                &mut lines,
                paint,
                Tone::Quiet,
                &[("Up to date", up_to_date.len())],
            );
        }
        lines.push(String::new());
        lines.push(paint.text(&self.closing(
            ready.len(),
            newer.len(),
            failed.len(),
            not_checked.len(),
        )));
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{AddonRecord, SourceKind, UpdateStatus};

    use super::CheckReport;
    use crate::find::FindTarget;
    use crate::output::{Output, OutputMode, Paint, Report};
    use crate::source_error::SourceErrorKind;
    use crate::theme::Symbols;
    use crate::update::CheckResult;

    fn addon(name: &str, folder: &str, source: SourceKind, version: Option<&str>) -> AddonRecord {
        let mut addon = AddonRecord::new(name, folder, source);
        addon.version = version.map(str::to_string);
        addon
    }

    fn check(folder: &str, status: UpdateStatus, remote: Option<&str>) -> CheckResult {
        CheckResult {
            addon_name: folder.to_string(),
            status,
            remote_version: remote.map(str::to_string),
            message: None,
            error_kind: None,
        }
    }

    fn target(profile: &str) -> FindTarget {
        FindTarget {
            game: "Retail".to_string(),
            profile: profile.to_string(),
            addon_dir: PathBuf::from("/games/wow/_retail_/Interface/AddOns"),
        }
    }

    /// One addon in each of the five states, two of them ready, and one not linked.
    fn mixed() -> (Vec<CheckResult>, Vec<AddonRecord>) {
        let addons = vec![
            addon(
                "Deadly Boss Mods",
                "DBM-Core",
                SourceKind::Wago,
                Some("12.0.8"),
            ),
            addon("Details", "Details", SourceKind::GitHub, Some("v1.2")),
            addon("Pawn", "Pawn", SourceKind::WowInterface, Some("2.11.4")),
            addon("Bagnon", "Bagnon", SourceKind::WowInterface, Some("11.0.1")),
            addon("WeakAuras", "WeakAuras", SourceKind::Wago, None),
            addon("Questie", "Questie", SourceKind::GitHub, Some("10.1")),
            addon("Postal", "Postal", SourceKind::Tukui, Some("3.6")),
            addon("HandyNotes", "HandyNotes", SourceKind::Manual, Some("1.0")),
        ];
        let mut failed = check("Bagnon", UpdateStatus::Error, None);
        failed.message = Some("connection reset".to_string());
        failed.error_kind = Some(SourceErrorKind::NoConnection);
        let mut no_key = check("WeakAuras", UpdateStatus::Unknown, None);
        no_key.message = Some("Wago API key not configured".to_string());
        let mut not_linked = check("HandyNotes", UpdateStatus::Unknown, None);
        not_linked.message = Some("addons that are not linked cannot be checked yet".to_string());
        let checks = vec![
            check("DBM-Core", UpdateStatus::UpdateAvailable, Some("12.0.10")),
            check("Details", UpdateStatus::UpdateAvailable, Some("v1.3")),
            check("Pawn", UpdateStatus::RemoteOlder, Some("2.11.2")),
            failed,
            no_key,
            check("Questie", UpdateStatus::UpToDate, Some("10.1")),
            check("Postal", UpdateStatus::UpToDate, Some("3.6")),
            not_linked,
        ];
        (checks, addons)
    }

    /// `line` with its escape codes taken out.
    fn strip(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn every_state_has_its_section_and_up_to_date_only_counts() {
        let (checks, addons) = mixed();
        let report = CheckReport::new(Some(target("default")), "default", false, &checks, &addons);
        assert_eq!(
            report.human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Update ready (2)",
                "  Deadly Boss Mods  Wago.io  12.0.8 → 12.0.10",
                "  Details           GitHub   v1.2 → v1.3",
                "",
                "Newer than source (1)",
                "  Pawn  WoWInterface  2.11.4, WoWInterface has 2.11.2",
                "",
                "Check failed (1)",
                "  Bagnon  WoWInterface  11.0.1",
                "      LemonUp couldn't reach WoWInterface. Check your internet connection and",
                "      try again.",
                "",
                "Not checked yet (1)",
                "  WeakAuras  Wago.io  unknown",
                "      Wago API key not configured",
                "",
                "Up to date (2)",
                "",
                "2 updates ready, 1 newer than source, 1 check failed, 1 not checked yet. Run `lemonup update-all` to update all.",
            ]
        );
    }

    #[test]
    fn named_addons_each_get_a_row_up_to_date_and_not_linked_included() {
        let (checks, addons) = mixed();
        let report = CheckReport::new(Some(target("default")), "default", true, &checks, &addons);
        let lines = report.human(Paint::PLAIN);
        let start = lines
            .iter()
            .position(|line| line == "Not checked yet (2)")
            .expect("not checked yet");
        assert_eq!(
            lines[start..],
            [
                "Not checked yet (2)",
                "  HandyNotes  Not linked  1.0",
                "      addons that are not linked cannot be checked yet",
                "  WeakAuras   Wago.io     unknown",
                "      Wago API key not configured",
                "",
                "Up to date (2)",
                "  Postal   TukUI   3.6",
                "  Questie  GitHub  10.1",
                "",
                "2 updates ready, 1 newer than source, 1 check failed, 2 not checked yet. Run `lemonup update-all` to update all.",
            ]
        );
    }

    #[test]
    fn a_named_not_linked_addon_gets_its_row_not_the_nothing_linked_line() {
        let addons = vec![addon(
            "HandyNotes",
            "HandyNotes",
            SourceKind::Manual,
            Some("1.0"),
        )];
        let checks = vec![check("HandyNotes", UpdateStatus::Unknown, None)];
        let lines = CheckReport::new(None, "default", true, &checks, &addons).human(Paint::PLAIN);
        assert_eq!(
            lines,
            [
                "Not checked yet (1)",
                "  HandyNotes  Not linked  1.0",
                "",
                "1 not checked yet.",
            ]
        );
    }

    #[test]
    fn nothing_linked_says_so_under_the_context_line() {
        let addons = vec![addon("HandyNotes", "HandyNotes", SourceKind::Manual, None)];
        let checks = vec![check("HandyNotes", UpdateStatus::Unknown, None)];
        let report = CheckReport::new(Some(target("default")), "default", false, &checks, &addons);
        assert_eq!(
            report.human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "No linked addons to check.",
            ]
        );
        // Without an AddOns folder there is no context line.
        let report = CheckReport::new(None, "default", false, &[], &[]);
        assert_eq!(report.human(Paint::PLAIN), ["No linked addons to check."]);
    }

    fn closing(profile: &str, statuses: &[UpdateStatus]) -> String {
        let addons = statuses
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let name = format!("Addon{index}");
                addon(&name, &name, SourceKind::Tukui, Some("1.0"))
            })
            .collect::<Vec<_>>();
        let checks = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| check(&format!("Addon{index}"), *status, Some("1.1")))
            .collect::<Vec<_>>();
        let lines = CheckReport::new(None, profile, false, &checks, &addons).human(Paint::PLAIN);
        assert_eq!(lines[lines.len() - 2], "", "{lines:?}");
        lines.last().expect("closing").clone()
    }

    #[test]
    fn a_run_given_an_addons_folder_says_the_update_needs_it_too() {
        let addons = vec![addon("Pawn", "Pawn", SourceKind::Wago, Some("2.14.4"))];
        let checks = vec![check("Pawn", UpdateStatus::UpdateAvailable, Some("2.14.8"))];
        let report = |profile: &str| {
            CheckReport::new(None, profile, false, &checks, &addons).with_addon_dir(true)
        };
        assert_eq!(
            report("default")
                .human(Paint::PLAIN)
                .last()
                .expect("closing"),
            "1 update ready. Run `lemonup update-all` with the same `--addon-dir` to update."
        );
        assert_eq!(
            report("forever")
                .human(Paint::PLAIN)
                .last()
                .expect("closing"),
            "1 update ready. Run `lemonup --profile forever update-all` with the same \
             `--addon-dir` to update."
        );
        // Without the option the hint stays as it was.
        assert_eq!(
            CheckReport::new(None, "default", false, &checks, &addons)
                .with_addon_dir(false)
                .human(Paint::PLAIN)
                .last()
                .expect("closing"),
            "1 update ready. Run `lemonup update-all` to update."
        );
    }

    #[test]
    fn the_closing_line_counts_updates_and_failures() {
        use UpdateStatus::{Error, Unknown, UpToDate, UpdateAvailable};
        assert_eq!(closing("default", &[UpToDate]), "Everything is up to date.");
        // Neither is one that is newer than its source.
        assert_eq!(
            closing("default", &[UpToDate, UpdateStatus::RemoteOlder]),
            "1 newer than source."
        );
        // An addon that couldn't be checked is not up to date.
        assert_eq!(
            closing("default", &[UpToDate, Unknown]),
            "1 not checked yet."
        );
        assert_eq!(
            closing("default", &[UpdateAvailable, Error, Unknown, Unknown]),
            "1 update ready, 1 check failed, 2 not checked yet. Run `lemonup update-all` to update."
        );
        assert_eq!(
            closing("default", &[UpdateAvailable]),
            "1 update ready. Run `lemonup update-all` to update."
        );
        assert_eq!(
            closing("default", &[UpdateAvailable, UpdateAvailable]),
            "2 updates ready. Run `lemonup update-all` to update all."
        );
        assert_eq!(closing("default", &[Error]), "1 check failed.");
        assert_eq!(closing("default", &[Error, Error]), "2 checks failed.");
        assert_eq!(
            closing("default", &[UpdateAvailable, Error]),
            "1 update ready, 1 check failed. Run `lemonup update-all` to update."
        );
        assert_eq!(
            closing("forever", &[UpdateAvailable, UpdateAvailable, Error, Error]),
            "2 updates ready, 2 checks failed. Run `lemonup --profile forever update-all` to update all."
        );
        assert_eq!(
            closing("forever", &[UpdateAvailable]),
            "1 update ready. Run `lemonup --profile forever update-all` to update."
        );
    }

    #[test]
    fn a_failed_check_shows_the_advice_for_its_kind_else_its_message() {
        let addons = vec![
            addon("Bagnon", "Bagnon", SourceKind::Wago, Some("1.0")),
            addon("Pawn", "Pawn", SourceKind::Tukui, Some("2.0")),
            addon("Postal", "Postal", SourceKind::GitHub, Some("3.0")),
        ];
        let mut rejected = check("Bagnon", UpdateStatus::Error, None);
        rejected.error_kind = Some(SourceErrorKind::KeyRejected);
        rejected.message = Some("HTTP 401".to_string());
        let mut plain = check("Pawn", UpdateStatus::Error, None);
        plain.message = Some("the zip has no TOC".to_string());
        let mut other = check("Postal", UpdateStatus::Error, None);
        other.error_kind = Some(SourceErrorKind::Other);
        other.message = Some("bad gateway".to_string());
        let lines = CheckReport::new(None, "default", false, &[rejected, plain, other], &addons)
            .human(Paint::PLAIN.with_width(200));
        assert_eq!(
            lines[..7],
            [
                "Check failed (3)",
                "  Bagnon  Wago.io  1.0",
                "      Wago.io didn't accept your Wago.io key. Check it and paste it again.",
                "  Pawn    TukUI    2.0",
                "      the zip has no TOC",
                "  Postal  GitHub   3.0",
                "      Something went wrong on the way. Try again; if it keeps happening, the \
                 technical detail helps with a bug report. Details: bad gateway",
            ]
        );
    }

    #[test]
    fn an_update_ready_row_shows_the_old_version_in_red_and_the_new_one_in_green() {
        let addons = vec![addon("Pawn", "Pawn", SourceKind::Wago, Some("Pawn 2.14.4"))];
        let checks = vec![check(
            "Pawn",
            UpdateStatus::UpdateAvailable,
            Some("Pawn 2.14.8"),
        )];
        let report = CheckReport::new(None, "default", false, &checks, &addons);
        // The addon's own name is not repeated in the versions.
        assert_eq!(
            report.human(Paint::PLAIN)[1],
            "  Pawn  Wago.io  2.14.4 → 2.14.8"
        );
        let colored = report.human(Paint::terminal());
        assert!(
            colored[1].ends_with("\x1b[38;5;1m2.14.4\x1b[0m → \x1b[38;5;2m2.14.8\x1b[0m"),
            "{:?}",
            colored[1]
        );
    }

    #[test]
    fn simple_symbols_draw_the_arrow_as_ascii() {
        let (checks, addons) = mixed();
        let lines = CheckReport::new(None, "default", false, &checks, &addons)
            .human(Paint::PLAIN.with_symbols(Symbols::SIMPLE));
        assert_eq!(lines[1], "  Deadly Boss Mods  Wago.io  12.0.8 -> 12.0.10");
    }

    #[test]
    fn colored_lines_use_only_the_16_colors_and_keep_the_text() {
        let (checks, addons) = mixed();
        let report = CheckReport::new(Some(target("forever")), "forever", true, &checks, &addons);
        let colored = report.human(Paint::terminal());
        let joined = colored.join("\n");
        assert!(joined.contains('\x1b'), "{joined:?}");
        assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
        assert!(!joined.contains("48;"), "background: {joined:?}");
        assert!(
            colored
                .contains(&"\x1b[1m\x1b[38;5;3mUpdate ready\x1b[0m \x1b[2m(2)\x1b[0m".to_string()),
            "{colored:?}"
        );
        assert!(
            colored
                .contains(&"\x1b[1m\x1b[38;5;1mCheck failed\x1b[0m \x1b[2m(1)\x1b[0m".to_string()),
            "{colored:?}"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, report.human(Paint::PLAIN));
    }

    #[test]
    fn the_json_document_lists_every_checked_addon_by_name() {
        let (checks, addons) = mixed();
        let mut output = Output::new(
            OutputMode::Json,
            Paint::terminal(),
            Paint::terminal(),
            "default",
            Vec::new(),
            Vec::new(),
        );
        let report = CheckReport::new(Some(target("default")), "default", false, &checks, &addons);
        output
            .print(
                &report,
                Some(std::path::Path::new("/games/wow/_retail_/Interface/AddOns")),
            )
            .expect("print");
        let (stdout, stderr) = output.into_writers();
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).expect("utf-8"),
            concat!(
                r#"{"schema":1,"command":"check","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","#,
                r#""summary":{"checked":8,"up_to_date":2,"update_available":2,"remote_older":1,"unknown":2,"errors":1},"#,
                r#""addons":["#,
                r#"{"name":"Bagnon","folder":"Bagnon","source":"wowinterface","status":"error","installed_version":"11.0.1","remote_version":null,"message":"connection reset","error_kind":"no_connection"},"#,
                r#"{"name":"Deadly Boss Mods","folder":"DBM-Core","source":"wago","status":"update_available","installed_version":"12.0.8","remote_version":"12.0.10","message":null,"error_kind":null},"#,
                r#"{"name":"Details","folder":"Details","source":"github","status":"update_available","installed_version":"v1.2","remote_version":"v1.3","message":null,"error_kind":null},"#,
                r#"{"name":"HandyNotes","folder":"HandyNotes","source":"manual","status":"unknown","installed_version":"1.0","remote_version":null,"message":"addons that are not linked cannot be checked yet","error_kind":null},"#,
                r#"{"name":"Pawn","folder":"Pawn","source":"wowinterface","status":"remote_older","installed_version":"2.11.4","remote_version":"2.11.2","message":null,"error_kind":null},"#,
                r#"{"name":"Postal","folder":"Postal","source":"tukui","status":"up_to_date","installed_version":"3.6","remote_version":"3.6","message":null,"error_kind":null},"#,
                r#"{"name":"Questie","folder":"Questie","source":"github","status":"up_to_date","installed_version":"10.1","remote_version":"10.1","message":null,"error_kind":null},"#,
                r#"{"name":"WeakAuras","folder":"WeakAuras","source":"wago","status":"unknown","installed_version":null,"remote_version":null,"message":"Wago API key not configured","error_kind":null}"#,
                r#"],"warnings":[]}"#,
                "\n"
            )
        );
    }
}
