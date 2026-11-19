//! Marking linked addons Ignored in CurseForge, so its updates leave them to
//! LemonUp. Only on Linux, only while CurseForge is closed (it writes its
//! record again while it runs), and only after a yes to the shared question.
//! LemonUp never sets an addon back to not Ignored.

use super::*;

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

/// Addons to mark Ignored in CurseForge: their names as LemonUp shows them,
/// and every folder they own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CurseForgeMark {
    pub(crate) names: Vec<String>,
    pub(crate) folders: Vec<String>,
}

impl CurseForgeMark {
    /// Adds `other`'s addons and folders that aren't in this one yet.
    fn merge(&mut self, other: Self) {
        for name in other.names {
            if !self.names.contains(&name) {
                self.names.push(name);
            }
        }
        for folder in other.folders {
            if !self
                .folders
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&folder))
            {
                self.folders.push(folder);
            }
        }
    }
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
    /// Opens the question, or adds to the open one.
    Ask(CurseForgeMark),
    /// Closes the question; nothing changes.
    Close,
    /// Marks the question's addons, says how that went, and closes it.
    Mark,
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

impl App {
    /// What follows a confirmed install or link of addons CurseForge still
    /// manages: nothing where LemonUp can't mark them; the sentence to add
    /// to the hint while CurseForge runs; otherwise the question.
    pub(super) fn curseforge_follow_up(
        &self,
        mark: CurseForgeMark,
    ) -> (Option<&'static str>, Vec<AppAction>) {
        if mark.names.is_empty() || !self.curseforge_can_mark {
            return (None, Vec::new());
        }
        if (self.curseforge_running)() {
            return (Some(QUIT_TO_MARK), Vec::new());
        }
        (
            None,
            vec![AppAction::CurseForge(CurseForgeAction::Ask(mark))],
        )
    }

    /// The addon `m` marks from Details: the one shown, when it is linked,
    /// CurseForge manages it, and LemonUp can change CurseForge's list here.
    pub(super) fn details_curseforge_mark(&self) -> Option<CurseForgeMark> {
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
            .then(|| CurseForgeMark {
                names: vec![item.name.clone()],
                folders,
            })
    }

    pub(super) fn curseforge_actions(&self, message: CurseForgeMessage) -> Vec<AppAction> {
        match message {
            CurseForgeMessage::MarkFromDetails => {
                let Some(mark) = self.details_curseforge_mark() else {
                    return Vec::new();
                };
                if (self.curseforge_running)() {
                    return vec![AppAction::SetStatus(self.dashboard_status_for(
                        StatusKind::Note,
                        self.dashboard.detail_mode,
                        RUNNING_PRESS_AGAIN,
                    ))];
                }
                vec![AppAction::CurseForge(CurseForgeAction::Ask(mark))]
            }
            CurseForgeMessage::Confirm => vec![AppAction::CurseForge(CurseForgeAction::Mark)],
            CurseForgeMessage::Skip => vec![AppAction::CurseForge(CurseForgeAction::Close)],
        }
    }

    pub(super) fn apply_curseforge(&mut self, action: CurseForgeAction) {
        match action {
            CurseForgeAction::Ask(mark) => match &mut self.curseforge_mark {
                Some(open) => open.merge(mark),
                None => self.curseforge_mark = Some(mark),
            },
            CurseForgeAction::Close => self.curseforge_mark = None,
            CurseForgeAction::Mark => {
                let Some(mark) = self.curseforge_mark.take() else {
                    return;
                };
                let (kind, text) = if (self.curseforge_running)() {
                    (StatusKind::Problem, RUNNING_NOTHING_CHANGED.to_string())
                } else {
                    match self.write_curseforge_mark(&mark) {
                        Ok(()) => (StatusKind::Done, marked_text(&mark.names)),
                        Err(error) => {
                            tracing::warn!(%error, "LemonUp left CurseForge's list as it was");
                            (StatusKind::Problem, MARK_FAILED.to_string())
                        }
                    }
                };
                // Read again, so the hints for the marked addons go. Details
                // has no status line: it says how it went in their place.
                self.curseforge = self.curseforge_now();
                self.curseforge_details_note = Some((mark.folders.clone(), kind, text.clone()));
                let status = self.dashboard_status_for(kind, self.dashboard.detail_mode, &text);
                self.apply(AppAction::SetStatus(status));
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

    fn write_curseforge_mark(&self, mark: &CurseForgeMark) -> Result<(), String> {
        let addon_dir = self
            .effective_addon_dir
            .as_deref()
            .ok_or("no AddOns folder")?;
        let record = lemonup_core::curseforge::record_in_use(&self.curseforge_files)
            .ok_or("CurseForge's record can't be read")?;
        lemonup_core::curseforge::mark_ignored(&record, addon_dir, &mark.folders)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
