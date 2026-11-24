//! `lemonup check`: what each linked addon's source has, by what it means.

use lemonup_core::{AddonRecord, DEFAULT_PROFILE, SourceKind, UpdateStatus};
use serde::{Serialize, Serializer};

use super::{
    Entry, Paint, Report, Tone, context_line, count_headings, display_version, rows_by_folder,
    section, source_label, table,
};
use crate::find::FindTarget;
use crate::github::GithubTokenSource;
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
    /// Whether, and how, the closing line names the command that installs the updates.
    #[serde(skip)]
    hint: UpdateHint,
    /// Where the GitHub token the check sent comes from, if it sent one.
    #[serde(skip)]
    github_token: Option<GithubTokenSource>,
    summary: CheckSummary,
    addons: Vec<CheckRow>,
}

/// Whether, and how, the closing line names the command that installs the updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateHint {
    /// `lemonup update-all` acts on the folder that was checked.
    Plain,
    /// The run was given `--addon-dir`, and the update needs it again.
    WithAddonDir,
    /// The update would be refused: no AddOns folder is set, or the profile is bound to
    /// another one. No command is named.
    None,
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
    /// `None` when LemonUp's list had no row for the folder. Such a result is never taken
    /// for Not linked, so it is not left out of the report, and JSON says `null`.
    #[serde(serialize_with = "serialize_known_source")]
    source: Option<SourceKind>,
    #[serde(serialize_with = "serialize_status")]
    status: UpdateStatus,
    installed_version: Option<String>,
    remote_version: Option<String>,
    message: Option<String>,
    #[serde(serialize_with = "super::serialize_error_kind")]
    error_kind: Option<SourceErrorKind>,
}

impl CheckRow {
    /// An addon LemonUp lists as Not linked. It is shown only when it was named.
    fn not_linked(&self) -> bool {
        self.source == Some(SourceKind::Manual)
    }

    /// The source column: nothing for a result LemonUp's list has no row for.
    fn source_text(&self) -> &'static str {
        self.source.map_or("", source_label)
    }

    /// The source inside a sentence, and at the start of one.
    fn source_words(&self) -> (&'static str, &'static str) {
        match self.source {
            Some(source) => (source_label(source), source_label(source)),
            None => ("the source", "The source"),
        }
    }
}

fn serialize_known_source<S: Serializer>(
    source: &Option<SourceKind>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match source {
        Some(source) => serializer.serialize_some(super::source_key(*source)),
        None => serializer.serialize_none(),
    }
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
                    source: addon.map(|addon| addon.source),
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
            hint: UpdateHint::Plain,
            github_token: None,
            summary,
            addons: rows,
        }
    }

    /// The same report for a check that sent the GitHub token from `source`.
    pub(crate) fn with_github_token(mut self, source: Option<GithubTokenSource>) -> Self {
        self.github_token = source;
        self
    }

    /// The same report with another hint: `update-all` alone would act on the profile's saved
    /// folder, which may not be the one checked, or be refused.
    pub(crate) fn with_hint(mut self, hint: UpdateHint) -> Self {
        self.hint = hint;
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
        let with = match self.hint {
            UpdateHint::Plain => "",
            UpdateHint::WithAddonDir => " with the same `--addon-dir`",
            // A command that would be refused is not suggested.
            UpdateHint::None => return closing,
        };
        if updates > 0 {
            closing.push_str(&format!(
                " Run `{}`{with} to update{}.",
                self.update_all_command(),
                if updates == 1 { "" } else { " all" }
            ));
        }
        closing
    }
}

fn entry(row: &CheckRow, version: String, notes: Vec<String>) -> Entry {
    Entry {
        name: row.name.clone(),
        source: row.source_text().to_string(),
        version,
        update: None,
        notes,
        from: None,
        done: false,
    }
}

/// What a failed check says: the cause in plain words when LemonUp knows it, as Details says
/// it in the TUI, else its message. The kind decides, never the message's text.
/// `github_token` is where the GitHub token the check sent comes from, if it sent one.
fn failure_note(row: &CheckRow, github_token: Option<GithubTokenSource>) -> Option<String> {
    let Some(kind) = row.error_kind else {
        return row.message.clone();
    };
    // `name` inside a sentence, `first` at the start of one.
    let (name, first) = row.source_words();
    let mut note = match kind {
        SourceErrorKind::NoConnection => {
            format!("Couldn't reach {name}. Check your internet connection and try again.")
        }
        SourceErrorKind::KeyRejected if row.source == Some(SourceKind::Wago) => {
            "Wago.io turned down your Wago.io key. Check the key in Settings and try again."
                .to_string()
        }
        SourceErrorKind::KeyRejected if row.source == Some(SourceKind::GitHub) => {
            match github_token {
                Some(GithubTokenSource::Settings) => {
                    "GitHub didn't accept your GitHub token. Check it in Settings and try again."
                        .to_string()
                }
                Some(GithubTokenSource::Environment(name)) => {
                    format!("GitHub didn't accept the token in {name}. Replace or unset it.")
                }
                None => "GitHub turned down LemonUp's request. Try again later.".to_string(),
            }
        }
        SourceErrorKind::KeyRejected => {
            format!("{first} turned down LemonUp's request. Try again later.")
        }
        SourceErrorKind::NotFound => {
            format!(
                "{first} doesn't list this addon anymore. It may have moved or been taken down."
            )
        }
        SourceErrorKind::SlowDown
            if row.source == Some(SourceKind::GitHub) && github_token.is_none() =>
        {
            "GitHub's limit of 60 requests an hour is used up. Add a GitHub token in Settings or \
             set GITHUB_TOKEN to raise it, or try again later."
                .to_string()
        }
        SourceErrorKind::SlowDown => {
            format!("{first} asked LemonUp to slow down. Wait a few minutes and try again.")
        }
        SourceErrorKind::WrongRepository => "The repository may have a new owner. LemonUp won't \
                                             update it until its list is updated."
            .to_string(),
        SourceErrorKind::Other
        | SourceErrorKind::FoldersInTheWay
        | SourceErrorKind::FoldersOfLinkedAddon => {
            format!("LemonUp couldn't check {name} for a newer version.")
        }
    };
    // Where the plain words don't say what was found, the source's own words follow.
    let detailed = matches!(
        kind,
        SourceErrorKind::Other
            | SourceErrorKind::FoldersInTheWay
            | SourceErrorKind::FoldersOfLinkedAddon
            | SourceErrorKind::WrongRepository
    );
    if detailed && let Some(detail) = row.message.as_deref().filter(|detail| !detail.is_empty()) {
        note.push_str(&format!(" Details: {detail}"));
    }
    Some(note)
}

impl Report for CheckReport {
    fn command(&self) -> &'static str {
        "check"
    }

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
        // Nothing to show: no addon came back, or only Not linked ones that weren't named.
        let nothing = if self.addons.is_empty() && self.named {
            Some("Nothing was checked.")
        } else if !self.named && self.addons.iter().all(CheckRow::not_linked) {
            Some("No linked addons to check.")
        } else {
            None
        };
        if let Some(text) = nothing {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(paint.text(text));
            return lines;
        }
        let shown = |status: UpdateStatus| {
            self.addons
                .iter()
                .filter(move |row| row.status == status)
                .filter(|row| self.named || !row.not_linked())
                .collect::<Vec<_>>()
        };
        // Without the addon's own name in front, which the row already shows.
        let short = |row: &CheckRow, version: Option<&str>| {
            display_version(version, &[&row.name, &row.folder])
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
                .map(|row| {
                    let (from, to) = (installed(row), remote(row));
                    // Every update carries a message. It is worth a line only when the two
                    // versions read the same, where it is the one thing that says why.
                    let why = if from == to {
                        row.message.iter().cloned().collect()
                    } else {
                        Vec::new()
                    };
                    Entry {
                        from: Some(from),
                        ..entry(row, to, why)
                    }
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
                        row.source_words().0,
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
                .map(|row| {
                    entry(
                        row,
                        installed(row),
                        failure_note(row, self.github_token).into_iter().collect(),
                    )
                })
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
        lines.push(paint.commands(
            &self.closing(ready.len(), newer.len(), failed.len(), not_checked.len()),
            Paint::text,
        ));
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lemonup_core::{AddonRecord, SourceKind, UpdateStatus};

    use super::{CheckReport, UpdateHint};
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
                "      Couldn't reach WoWInterface. Check your internet connection and try",
                "      again.",
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
            CheckReport::new(None, profile, false, &checks, &addons)
                .with_hint(UpdateHint::WithAddonDir)
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
                .with_hint(UpdateHint::Plain)
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
                "      Wago.io turned down your Wago.io key. Check the key in Settings and try again.",
                "  Pawn    TukUI    2.0",
                "      the zip has no TOC",
                "  Postal  GitHub   3.0",
                "      LemonUp couldn't check GitHub for a newer version. Details: bad gateway",
            ]
        );
    }

    #[test]
    fn a_github_failure_says_what_the_token_has_to_do_with_it() {
        use crate::github::GithubTokenSource;
        let addons = vec![addon("Postal", "Postal", SourceKind::GitHub, Some("3.0"))];
        let note = |kind, token| {
            let mut failed = check("Postal", UpdateStatus::Error, None);
            failed.error_kind = Some(kind);
            failed.message = Some("raw words".to_string());
            let report = CheckReport::new(None, "default", false, &[failed], &addons)
                .with_github_token(token);
            report.human(Paint::PLAIN.with_width(200))[2]
                .trim()
                .to_string()
        };
        // No token sent: one raises the limit.
        assert_eq!(
            note(SourceErrorKind::SlowDown, None),
            "GitHub's limit of 60 requests an hour is used up. Add a GitHub token in Settings or \
             set GITHUB_TOKEN to raise it, or try again later."
        );
        // A token sent: never ask for one.
        assert_eq!(
            note(SourceErrorKind::SlowDown, Some(GithubTokenSource::Settings)),
            "GitHub asked LemonUp to slow down. Wait a few minutes and try again."
        );
        assert_eq!(
            note(
                SourceErrorKind::KeyRejected,
                Some(GithubTokenSource::Settings)
            ),
            "GitHub didn't accept your GitHub token. Check it in Settings and try again."
        );
        assert_eq!(
            note(
                SourceErrorKind::KeyRejected,
                Some(GithubTokenSource::Environment("GITHUB_TOKEN"))
            ),
            "GitHub didn't accept the token in GITHUB_TOKEN. Replace or unset it."
        );
        assert_eq!(
            note(
                SourceErrorKind::KeyRejected,
                Some(GithubTokenSource::Environment("GH_TOKEN"))
            ),
            "GitHub didn't accept the token in GH_TOKEN. Replace or unset it."
        );
        // JSON never says where the token came from.
        let mut failed = check("Postal", UpdateStatus::Error, None);
        failed.error_kind = Some(SourceErrorKind::KeyRejected);
        let report = CheckReport::new(None, "default", false, &[failed], &addons);
        let plain = serde_json::to_string(&report).expect("json");
        let with_token =
            serde_json::to_string(&report.with_github_token(Some(GithubTokenSource::Settings)))
                .expect("json");
        assert_eq!(plain, with_token);
    }

    #[test]
    fn a_repository_with_a_new_owner_keeps_its_own_kind_and_detail() {
        let addons = vec![addon("Postal", "Postal", SourceKind::GitHub, Some("3.0"))];
        let mut moved = check("Postal", UpdateStatus::Error, None);
        moved.error_kind = Some(SourceErrorKind::WrongRepository);
        moved.message = Some("github.com/someone-else/Postal".to_string());
        let report = CheckReport::new(None, "default", false, &[moved], &addons);
        assert_eq!(
            report.human(Paint::PLAIN.with_width(200))[2],
            "      The repository may have a new owner. LemonUp won't update it until its list is \
             updated. Details: github.com/someone-else/Postal"
        );
        let mut output = Output::new(
            OutputMode::Json,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            Vec::new(),
            Vec::new(),
        );
        output.print(&report, None).expect("print");
        let document = String::from_utf8(output.into_writers().0).expect("utf-8");
        assert!(
            document.contains(r#""error_kind":"wrong_repository""#),
            "{document}"
        );
    }

    #[test]
    fn a_result_with_no_row_is_shown_not_taken_for_not_linked() {
        // The list had no row for it when the report was built, so nothing says it is linked.
        let mut ready = check("Stranger", UpdateStatus::UpdateAvailable, Some("2.0"));
        ready.message = None;
        let report = CheckReport::new(None, "default", false, &[ready], &[]);
        assert_eq!(
            report.human(Paint::PLAIN),
            [
                "Update ready (1)",
                "  Stranger    unknown → 2.0",
                "",
                "1 update ready. Run `lemonup update-all` to update.",
            ]
        );
    }

    #[test]
    fn a_result_with_no_row_reads_as_a_sentence_and_has_no_source_in_json() {
        let older = check("Stranger", UpdateStatus::RemoteOlder, Some("1.0"));
        let mut gone = check("Wanderer", UpdateStatus::Error, None);
        gone.error_kind = Some(SourceErrorKind::NotFound);
        let mut slow = check("Yonder", UpdateStatus::Error, None);
        slow.error_kind = Some(SourceErrorKind::SlowDown);
        let report = CheckReport::new(None, "default", false, &[older, gone, slow], &[]);
        assert_eq!(
            report.human(Paint::PLAIN.with_width(200)),
            [
                "Newer than source (1)",
                "  Stranger    unknown, the source has 1.0",
                "",
                "Check failed (2)",
                "  Wanderer    unknown",
                "      The source doesn't list this addon anymore. It may have moved or been taken \
                 down.",
                "  Yonder      unknown",
                "      The source asked LemonUp to slow down. Wait a few minutes and try again.",
                "",
                "1 newer than source, 2 checks failed.",
            ]
        );
        let mut output = Output::new(
            OutputMode::Json,
            Paint::PLAIN,
            Paint::PLAIN,
            "default",
            Vec::new(),
            Vec::new(),
        );
        output.print(&report, None).expect("print");
        let document = String::from_utf8(output.into_writers().0).expect("utf-8");
        // Nothing says these are Not linked, so JSON doesn't call them `manual`.
        assert!(
            document.contains(r#""name":"Stranger","folder":"Stranger","source":null"#),
            "{document}"
        );
    }

    #[test]
    fn an_update_ready_row_has_no_note_when_its_versions_differ() {
        let addons = vec![addon("Pawn", "Pawn", SourceKind::Wago, Some("2.14.4"))];
        // Every real update carries this message.
        let mut ready = check("Pawn", UpdateStatus::UpdateAvailable, Some("2.14.8"));
        ready.message =
            Some("the source has a different version than the installed one".to_string());
        assert_eq!(
            CheckReport::new(None, "default", false, &[ready], &addons).human(Paint::PLAIN),
            [
                "Update ready (1)",
                "  Pawn  Wago.io  2.14.4 → 2.14.8",
                "",
                "1 update ready. Run `lemonup update-all` to update.",
            ]
        );
    }

    #[test]
    fn a_named_check_that_brought_nothing_back_says_so() {
        assert_eq!(
            CheckReport::new(None, "default", true, &[], &[]).human(Paint::PLAIN),
            ["Nothing was checked."]
        );
        assert_eq!(
            CheckReport::new(None, "default", false, &[], &[]).human(Paint::PLAIN),
            ["No linked addons to check."]
        );
    }

    #[test]
    fn no_command_is_named_when_the_update_would_be_refused() {
        let addons = vec![addon("Pawn", "Pawn", SourceKind::Wago, Some("2.14.4"))];
        let checks = vec![check("Pawn", UpdateStatus::UpdateAvailable, Some("2.14.8"))];
        let report =
            CheckReport::new(None, "default", false, &checks, &addons).with_hint(UpdateHint::None);
        assert_eq!(
            report.human(Paint::PLAIN).last().expect("closing"),
            "1 update ready."
        );
    }

    #[test]
    fn the_rows_are_joined_by_folder_whatever_its_case() {
        let addons = vec![addon("Pawn", "pawn", SourceKind::Wago, Some("2.14.4"))];
        let checks = vec![check("PAWN", UpdateStatus::UpToDate, Some("2.14.4"))];
        let lines = CheckReport::new(None, "default", true, &checks, &addons).human(Paint::PLAIN);
        assert_eq!(lines[1], "  Pawn  Wago.io  2.14.4");
    }

    #[test]
    fn an_update_ready_row_shows_why_when_the_check_says() {
        let addons = vec![addon(
            "Details",
            "Details",
            SourceKind::GitHub,
            Some("v1.2"),
        )];
        let mut ready = check("Details", UpdateStatus::UpdateAvailable, Some("v1.2"));
        ready.message = Some("the source has a release for this game".to_string());
        let lines =
            CheckReport::new(None, "default", false, &[ready], &addons).human(Paint::terminal());
        // The same version on both sides is no change: one version, no arrow, no red or green.
        assert_eq!(lines[1], "  Details  \x1b[2mGitHub\x1b[0m  v1.2");
        assert_eq!(
            lines[2],
            "\x1b[2m      the source has a release for this game\x1b[0m"
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
        // The command is bold cyan, and the backticks that mark it in plain text are gone.
        assert!(
            colored.last().expect("closing").ends_with(
                "Run \x1b[1m\x1b[38;5;6mlemonup --profile forever update-all\x1b[0m to update all."
            ),
            "{colored:?}"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        let plain = report
            .human(Paint::PLAIN)
            .iter()
            .map(|line| line.replace('`', ""))
            .collect::<Vec<_>>();
        assert_eq!(stripped, plain);
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
