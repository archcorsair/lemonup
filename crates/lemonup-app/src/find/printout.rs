//! What `lemonup find` prints: the readable report, or its JSON document.

use lemonup_core::AddonRecord;
use lemonup_core::find::{
    BlockReason, BlockedFolder, FindLabel, FindReport, FindResult, FindSource, FoundAddon,
};
use serde::Serialize;

use super::human::{Closing, Rerun, closing_text, draw_closing};
use super::{FindTarget, LinkOutcome, LocalNames, SourceProblem, human_report, target_line};
use crate::output::{Paint, Report, error_kind_key, source_key};

/// What `lemonup find` prints.
#[derive(Serialize)]
pub(crate) struct FindPrintout<'a> {
    #[serde(skip)]
    target: &'a FindTarget,
    #[serde(skip)]
    report: &'a FindReport,
    #[serde(skip)]
    source_problems: &'a [SourceProblem],
    #[serde(skip)]
    names: &'a LocalNames,
    /// How many folder names went to Wago.io, as the readable report counts them.
    #[serde(skip)]
    wago_folders: usize,
    /// What ends the readable report. Empty while the run can still end another way.
    #[serde(skip)]
    closing: Vec<Closing>,
    /// The readable report is on screen already, so only what ends it is left to print.
    #[serde(skip)]
    report_shown: bool,
    /// The sources the lookup covered: those that were named (all, without `--source`) and
    /// could be asked. Wago.io is one of them whenever its key is set, also when there was
    /// nothing to send it.
    sources_asked: Vec<&'static str>,
    /// Sources that were not asked because they need a key that is not set.
    sources_without_key: Vec<&'static str>,
    /// The sources that failed. A run with one exits 1 and still prints its document.
    problems: Vec<ProblemRow>,
    summary: Summary,
    /// The addons that were looked up, as they were before anything was linked.
    addons: Vec<AddonRow>,
    /// What `--link` linked; `null` without `--link`.
    link: Option<LinkRow>,
}

#[derive(Serialize)]
struct ProblemRow {
    source: &'static str,
    error_kind: &'static str,
    /// The source's or the system's own words, when there are any.
    message: String,
}

#[derive(Serialize)]
struct Summary {
    /// Folders read in the AddOns folder.
    folders: usize,
    addons: usize,
    exact: usize,
    likely: usize,
    partial: usize,
    choose_one: usize,
    not_checked: usize,
    curseforge_only: usize,
    not_found: usize,
    developer_copy: usize,
}

#[derive(Serialize)]
struct AddonRow {
    /// The name LemonUp shows for the addon, as the readable report has it. For
    /// `choose_one`, `name` and `folder` are those of the first candidate.
    name: String,
    folder: String,
    label: &'static str,
    /// The addon's folders as LemonUp groups them now. `null` for a result with a match or
    /// candidates: each of those lists the folders it would take.
    folders: Option<Vec<String>>,
    /// The match of an exact, likely or partial result.
    #[serde(rename = "match")]
    found: Option<CandidateRow>,
    /// The matches to choose from, for `choose_one`.
    candidates: Vec<CandidateRow>,
    /// The sources that failed and could have found it, for `not_checked`.
    not_checked_by: Vec<&'static str>,
    curseforge_id: Option<String>,
}

#[derive(Serialize)]
struct CandidateRow {
    label: &'static str,
    /// The addon's name on the source.
    name: String,
    source: &'static str,
    id: String,
    url: Option<String>,
    author: Option<String>,
    /// The version installed now, as the source names it, when the source could tell.
    installed_version: Option<String>,
    /// The source's newest version.
    remote_version: Option<String>,
    /// The folder the linked addon is listed under.
    folder: String,
    /// Every folder it takes over, that folder first.
    folders: Vec<String>,
    /// Folders it takes from other Not linked addons.
    joining: Vec<JoiningRow>,
    /// Folders grouped with it now that the source doesn't list.
    leaving: Vec<String>,
    /// The source's folders that are not in the AddOns folder.
    missing: Vec<String>,
    /// Folders whose files differ from the source's.
    differing: Vec<String>,
    linkable: bool,
    /// Why it can't be linked: the folders LemonUp won't take.
    blocked: Vec<BlockedRow>,
}

#[derive(Serialize)]
struct JoiningRow {
    folder: String,
    /// The folder of the addon it is grouped under now; `null` when it is an addon of its own.
    from: Option<String>,
}

#[derive(Serialize)]
struct BlockedRow {
    folder: String,
    /// `linked`: a linked addon owns it. `newer_version`: an addon a newer LemonUp linked
    /// owns it. `developer_copy`: it is a symlink or has `.git`.
    reason: &'static str,
    /// The addon that owns the folder; `null` for a developer copy.
    addon: Option<String>,
}

#[derive(Serialize)]
struct LinkRow {
    /// The linked addons LemonUp's list holds now.
    linked: Vec<LinkedRow>,
    /// Linked addons LemonUp could not find again after linking.
    unconfirmed: Vec<UnconfirmedRow>,
}

#[derive(Serialize)]
struct LinkedRow {
    /// The name LemonUp shows for the addon, as `check` and `update` have it.
    name: String,
    folder: String,
    source: &'static str,
    /// The CurseForge app manages it too.
    also_on_curseforge: bool,
}

#[derive(Serialize)]
struct UnconfirmedRow {
    name: String,
    folder: String,
    source: &'static str,
}

fn find_source_key(source: FindSource) -> &'static str {
    source_key(source.source_kind())
}

fn blocked_row(blocked: &BlockedFolder) -> BlockedRow {
    let (reason, addon) = match &blocked.reason {
        BlockReason::Linked { addon } => ("linked", Some(addon.clone())),
        BlockReason::NewerVersion { addon } => ("newer_version", Some(addon.clone())),
        BlockReason::DeveloperCopy => ("developer_copy", None),
    };
    BlockedRow {
        folder: blocked.folder.clone(),
        reason,
        addon,
    }
}

fn candidate_row(found: &FoundAddon) -> CandidateRow {
    CandidateRow {
        label: found.label.cli_name(),
        name: found.name.clone(),
        source: find_source_key(found.source),
        id: found.source_id.clone(),
        url: Some(found.source_url.clone()).filter(|url| !url.is_empty()),
        author: found.author.clone(),
        installed_version: found.matched_version.clone(),
        remote_version: found.latest_version.clone(),
        folder: found.parent_folder.clone(),
        folders: found.folders.clone(),
        joining: found
            .joining
            .iter()
            .map(|moved| JoiningRow {
                folder: moved.folder.clone(),
                from: (!moved.is_own_addon()).then(|| moved.from.clone()),
            })
            .collect(),
        leaving: found.leaving.clone(),
        missing: found.missing.clone(),
        differing: found.differing.clone(),
        linkable: found.is_linkable(),
        blocked: found.blocked.iter().map(blocked_row).collect(),
    }
}

fn addon_row(result: &FindResult, names: &LocalNames) -> AddonRow {
    let found = result.found().map(candidate_row);
    let candidates = if result.label == FindLabel::ChooseOne {
        result.candidates.iter().map(candidate_row).collect()
    } else {
        Vec::new()
    };
    AddonRow {
        name: names.name(&result.folder, &result.name),
        folder: result.folder.clone(),
        label: result.label.cli_name(),
        folders: (found.is_none() && candidates.is_empty()).then(|| result.folders.clone()),
        found,
        candidates,
        not_checked_by: result
            .not_checked_by
            .iter()
            .map(|source| find_source_key(*source))
            .collect(),
        curseforge_id: result.curseforge_id.clone(),
    }
}

fn unconfirmed_row(row: &AddonRecord) -> UnconfirmedRow {
    UnconfirmedRow {
        name: row.display_name().to_string(),
        folder: row.folder.clone(),
        source: source_key(row.source),
    }
}

fn link_row(outcome: &LinkOutcome) -> Option<LinkRow> {
    match outcome {
        LinkOutcome::NothingToLink => Some(LinkRow {
            linked: Vec::new(),
            unconfirmed: Vec::new(),
        }),
        // Nothing was linked, and the run ends in a refusal: there is no document.
        LinkOutcome::NeedsYes(_) => None,
        LinkOutcome::Linked {
            rows,
            unconfirmed,
            curseforge,
        } => Some(LinkRow {
            linked: rows
                .iter()
                .map(|row| LinkedRow {
                    name: row.display_name().to_string(),
                    folder: row.folder.clone(),
                    source: source_key(row.source),
                    also_on_curseforge: curseforge.contains(&row.folder),
                })
                .collect(),
            unconfirmed: unconfirmed.iter().map(unconfirmed_row).collect(),
        }),
    }
}

/// What a lookup found, apart from how the run ended.
#[derive(Clone, Copy)]
pub(crate) struct Found<'a> {
    pub(crate) target: &'a FindTarget,
    pub(crate) report: &'a FindReport,
    pub(crate) problems: &'a [SourceProblem],
    pub(crate) names: &'a LocalNames,
    /// Folders read in the AddOns folder.
    pub(crate) folders_read: usize,
    /// Folder names sent to Wago.io, `0` when it was not asked.
    pub(crate) wago_folders: usize,
    /// The run was given `--addon-dir`, so a command it suggests needs it again.
    pub(crate) addon_dir_given: bool,
}

impl<'a> Found<'a> {
    /// How to run LemonUp again on the game the lookup was about.
    pub(crate) fn rerun(&self) -> Rerun<'a> {
        Rerun {
            profile: &self.target.profile,
            addon_dir_given: self.addon_dir_given,
        }
    }
}

impl<'a> FindPrintout<'a> {
    /// The printout of a run that got to its end: `link` is what `--link` did, `None` without
    /// it. The readable report closes with what happened. A link that needs `--yes` is a
    /// refusal and never a run that got to its end: see [`super::needs_yes_refusal`].
    pub(crate) fn finished(found: Found<'a>, link: Option<LinkOutcome>) -> Self {
        debug_assert!(
            !matches!(link, Some(LinkOutcome::NeedsYes(_))),
            "a refusal is not a finished run"
        );
        let link_row = link.as_ref().and_then(link_row);
        let closing = closing_text(found.report, found.names, link, found.rerun());
        Self {
            closing,
            link: link_row,
            ..Self::report(found)
        }
    }

    /// The same printout for a run whose report is on screen already (see
    /// [`FindPrintout::report`]): readable, only what ends it; with `--json`, the document.
    pub(crate) fn after_report(self) -> Self {
        Self {
            report_shown: true,
            ..self
        }
    }

    /// The readable report alone, without what ends it, for
    /// [`crate::output::Output::print_readable`]: `--link` shows it before it links, since
    /// the run can still end in a refusal or a failure.
    pub(crate) fn report(found: Found<'a>) -> Self {
        let Found {
            target,
            report,
            problems,
            names,
            folders_read,
            wago_folders,
            addon_dir_given: _,
        } = found;
        let count = |label: FindLabel| {
            report
                .results
                .iter()
                .filter(|result| result.label == label)
                .count()
        };
        Self {
            target,
            report,
            source_problems: problems,
            names,
            wago_folders,
            closing: Vec::new(),
            report_shown: false,
            sources_asked: report.asked().into_iter().map(find_source_key).collect(),
            sources_without_key: report
                .skipped
                .iter()
                .map(|source| find_source_key(*source))
                .collect(),
            problems: problems
                .iter()
                .map(|problem| ProblemRow {
                    source: find_source_key(problem.source),
                    error_kind: error_kind_key(problem.error.kind),
                    message: problem.error.detail.clone(),
                })
                .collect(),
            summary: Summary {
                folders: folders_read,
                addons: report.results.len(),
                exact: count(FindLabel::Exact),
                likely: count(FindLabel::Likely),
                partial: count(FindLabel::Partial),
                choose_one: count(FindLabel::ChooseOne),
                not_checked: count(FindLabel::NotChecked),
                curseforge_only: count(FindLabel::CurseForgeOnly),
                not_found: count(FindLabel::NotFound),
                developer_copy: count(FindLabel::DeveloperCopy),
            },
            addons: report
                .results
                .iter()
                .map(|result| addon_row(result, names))
                .collect(),
            link: None,
        }
    }
}

impl Report for FindPrintout<'_> {
    fn command(&self) -> &'static str {
        "find"
    }

    /// The game, the report, then a blank line and what happened. Once the report is on
    /// screen, only what happened.
    fn human(&self, paint: Paint) -> Vec<String> {
        let closing = draw_closing(&self.closing, paint);
        if self.report_shown {
            return closing;
        }
        let mut lines = vec![target_line(self.target, paint)];
        lines.extend(human_report(
            self.report,
            self.source_problems,
            self.names,
            self.wago_folders,
            paint,
        ));
        lines.push(String::new());
        lines.extend(closing);
        lines
    }
}
