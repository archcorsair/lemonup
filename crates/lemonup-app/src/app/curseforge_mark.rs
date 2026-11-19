//! Marking linked addons Ignored in CurseForge, so its updates leave them to
//! LemonUp. Only on Linux, only while CurseForge is closed (it writes its
//! record again while it runs), and only after a yes to the shared question.
//! LemonUp never sets an addon back to not Ignored.

use super::*;
use crate::theme::ColorRoles;
use lemonup_core::curseforge::MarkError;
use ratatui::style::Color;

/// Said after the CurseForge hint when LemonUp could mark the addons itself
/// but CurseForge is running.
pub(super) const QUIT_TO_MARK: &str = "Quit CurseForge to have LemonUp do it.";
/// `m` in Details while CurseForge is running.
pub(super) const RUNNING_PRESS_AGAIN: &str = "CurseForge is running. Quit it, then press m again.";
/// A yes to the question while CurseForge has started since it was asked.
pub(super) const RUNNING_NOTHING_CHANGED: &str =
    "CurseForge is running. Nothing changed. Quit it, then press m in Details.";
/// The change was refused or failed; CurseForge's record is as it was.
pub(super) const MARK_FAILED: &str = "Couldn't change CurseForge's list. Nothing changed.";
/// A yes to a question whose addons are no longer what a mark would change.
pub(super) const ASK_AGAIN: &str =
    "CurseForge's list changed. Nothing was marked. Press m in Details.";

/// Shown when LemonUp replaced CurseForge's list but couldn't read the
/// change back.
pub(super) const MARK_UNCONFIRMED: &str =
    "LemonUp changed CurseForge's list but couldn't confirm it. Check these addons in CurseForge.";

/// How LemonUp changes CurseForge's list: [`lemonup_core::curseforge::mark_ignored`].
pub(super) type CurseForgeWriter =
    fn(&Path, &Path, &[String], &[String]) -> Result<Vec<String>, MarkError>;

/// The question to mark addons Ignored in CurseForge: the names of the
/// CurseForge addons a yes changes, as CurseForge calls them (a pack that
/// shares a folder is one of them), and the folders that pick them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CurseForgeMark {
    pub(crate) names: Vec<String>,
    pub(crate) folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CurseForgeMessage {
    /// `m` in Details.
    MarkFromDetails,
    /// `y` to the question.
    Confirm,
    /// A no to the question.
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CurseForgeAction {
    /// Opens the question, or adds its folders to the open one.
    Ask(CurseForgeMark),
    /// Closes the question; nothing changes.
    Close,
    /// Marks the question's addons, says how that went, and closes it.
    Mark,
    /// Says something about the addons in `folders`, in the status line and
    /// in Details.
    Note {
        folders: Vec<String>,
        kind: StatusKind,
        text: String,
    },
}

/// "Marked Pawn Ignored in CurseForge." or "Marked 3 addons Ignored in CurseForge."
pub(super) fn marked_text(names: &[String]) -> String {
    match names {
        [one] => format!("Marked {one} Ignored in CurseForge."),
        _ => format!(
            "Marked {} Ignored in CurseForge.",
            addon_count_label(names.len())
        ),
    }
}

/// The color of a note about marking: green when done, red for a problem.
pub(super) fn note_color(kind: StatusKind, roles: ColorRoles) -> Color {
    match kind {
        StatusKind::Done => roles.green,
        StatusKind::Problem => roles.red,
        _ => roles.amber,
    }
}

impl App {
    /// What follows a confirmed install or link of addons that own
    /// `folders`: nothing where LemonUp can't mark them (or CurseForge's
    /// list has nothing to change); the sentence to add to the hint while
    /// CurseForge runs; otherwise the question.
    pub(super) fn curseforge_follow_up(
        &self,
        folders: Vec<String>,
    ) -> (Option<&'static str>, Vec<AppAction>) {
        if folders.is_empty() || !self.curseforge_can_mark {
            return (None, Vec::new());
        }
        let Some(names) = self.curseforge_names_to_mark(&folders) else {
            return (None, Vec::new());
        };
        if (self.curseforge_running)() {
            return (Some(QUIT_TO_MARK), Vec::new());
        }
        (
            None,
            vec![AppAction::CurseForge(CurseForgeAction::Ask(
                CurseForgeMark { names, folders },
            ))],
        )
    }

    /// The CurseForge addons a mark of `folders` would change, read without
    /// changing anything. `None` when it would change nothing or can't.
    fn curseforge_names_to_mark(&self, folders: &[String]) -> Option<Vec<String>> {
        let addon_dir = self.effective_addon_dir.as_deref()?;
        let record = lemonup_core::curseforge::record_in_use(&self.curseforge_files)?;
        lemonup_core::curseforge::names_to_mark(&record, addon_dir, folders)
            .inspect_err(|error| tracing::info!(%error, "nothing to mark in CurseForge"))
            .ok()
            .filter(|names| !names.is_empty())
    }

    /// The folders `m` marks from Details: those of the addon shown, when it
    /// is linked, CurseForge manages it, and LemonUp can change CurseForge's
    /// list here.
    pub(super) fn details_curseforge_mark(&self) -> Option<Vec<String>> {
        if !self.curseforge_can_mark {
            return None;
        }
        let item = self
            .dashboard
            .selected_item()
            .filter(|item| item.source != SourceKind::Manual)?;
        let folders = std::iter::once(item.folder.clone())
            .chain(item.owned_folders.iter().cloned())
            .collect::<Vec<_>>();
        self.curseforge
            .manages_any(folders.iter().map(String::as_str))
            .then_some(folders)
    }

    pub(super) fn curseforge_actions(&self, message: CurseForgeMessage) -> Vec<AppAction> {
        match message {
            CurseForgeMessage::MarkFromDetails => {
                let Some(folders) = self.details_curseforge_mark() else {
                    return Vec::new();
                };
                let note = |kind, text: &str| {
                    vec![AppAction::CurseForge(CurseForgeAction::Note {
                        folders: folders.clone(),
                        kind,
                        text: text.to_string(),
                    })]
                };
                if (self.curseforge_running)() {
                    return note(StatusKind::Note, RUNNING_PRESS_AGAIN);
                }
                match self.curseforge_names_to_mark(&folders) {
                    Some(names) => vec![AppAction::CurseForge(CurseForgeAction::Ask(
                        CurseForgeMark { names, folders },
                    ))],
                    None => note(StatusKind::Problem, MARK_FAILED),
                }
            }
            CurseForgeMessage::Confirm => vec![AppAction::CurseForge(CurseForgeAction::Mark)],
            CurseForgeMessage::Skip => vec![AppAction::CurseForge(CurseForgeAction::Close)],
        }
    }

    pub(super) fn apply_curseforge(&mut self, action: CurseForgeAction) {
        match action {
            CurseForgeAction::Ask(mark) => {
                let Some(open) = &self.curseforge_mark else {
                    self.curseforge_mark = Some(mark);
                    return;
                };
                // One question for both: it names what a yes changes now.
                let mut folders = open.folders.clone();
                for folder in mark.folders {
                    if !folders
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(&folder))
                    {
                        folders.push(folder);
                    }
                }
                if let Some(names) = self.curseforge_names_to_mark(&folders) {
                    self.curseforge_mark = Some(CurseForgeMark { names, folders });
                }
            }
            CurseForgeAction::Close => self.curseforge_mark = None,
            CurseForgeAction::Note {
                folders,
                kind,
                text,
            } => {
                // Details has no status line: it says it in place.
                self.curseforge_details_note = Some((folders, kind, text.clone()));
                let status = self.dashboard_status_for(kind, self.dashboard.detail_mode, &text);
                self.apply(AppAction::SetStatus(status));
            }
            CurseForgeAction::Mark => {
                let Some(mark) = self.curseforge_mark.take() else {
                    return;
                };
                let (kind, text) = if (self.curseforge_running)() {
                    (StatusKind::Problem, RUNNING_NOTHING_CHANGED.to_string())
                } else {
                    match self.write_curseforge_mark(&mark) {
                        Ok(names) => (StatusKind::Done, marked_text(&names)),
                        Err(Some(error)) if error.changed_record() => {
                            tracing::warn!(%error, "LemonUp couldn't confirm CurseForge's list");
                            (StatusKind::Problem, MARK_UNCONFIRMED.to_string())
                        }
                        Err(Some(MarkError::NotAsAsked)) => {
                            (StatusKind::Problem, ASK_AGAIN.to_string())
                        }
                        Err(error) => {
                            if let Some(error) = error {
                                tracing::warn!(%error, "LemonUp left CurseForge's list as it was");
                            }
                            (StatusKind::Problem, MARK_FAILED.to_string())
                        }
                    }
                };
                // Read again, so the hints for the marked addons go.
                self.curseforge = self.curseforge_now();
                self.apply(AppAction::CurseForge(CurseForgeAction::Note {
                    folders: mark.folders,
                    kind,
                    text,
                }));
            }
        }
    }

    /// What the last mark did, when it was about the addon in `folder`.
    pub(super) fn curseforge_details_note(&self, folder: &str) -> Option<(StatusKind, String)> {
        let (folders, kind, text) = self.curseforge_details_note.as_ref()?;
        folders
            .iter()
            .any(|known| known.eq_ignore_ascii_case(folder))
            .then(|| (*kind, text.clone()))
    }

    /// The names the write marked, only when they are the ones the question
    /// named. `Err(None)` when it couldn't start.
    fn write_curseforge_mark(
        &self,
        mark: &CurseForgeMark,
    ) -> Result<Vec<String>, Option<MarkError>> {
        let addon_dir = self.effective_addon_dir.as_deref().ok_or(None)?;
        let record = lemonup_core::curseforge::record_in_use(&self.curseforge_files).ok_or(None)?;
        (self.curseforge_writer)(&record, addon_dir, &mark.folders, &mark.names).map_err(Some)
    }
}
