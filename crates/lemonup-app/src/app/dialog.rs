//! The question dialog: one centered box for every yes or no question.
//!
//! The title is the question, the body names exactly what is affected, and
//! the two answers are written as outcomes ("keep"). `y` says yes; `n`,
//! `esc` and `q` say no; `enter` does nothing, because it is what people hit
//! by reflex. The border is red for delete and restore only.

use super::keymap::KeyHint;
use super::sheet::{dim_area, wrap_lines};
use super::*;
use crate::theme::ColorRoles;

/// The widest a dialog gets, borders included.
const DIALOG_WIDTH: u16 = 64;
/// Columns between the border and the text, on each side.
const DIALOG_PADDING: u16 = 2;
/// The most addon names a delete question (or folders a replace question)
/// lists before "and N more".
const DIALOG_MAX_NAMES: usize = 8;

/// An addon a delete question names, with the folders that go with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeleteTarget {
    pub(super) name: String,
    pub(super) folders: usize,
}

/// An addon whose update waits for a yes, with every "Not linked" folder it
/// replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldEntry {
    /// The linked addon's folder.
    pub(crate) folder: String,
    pub(crate) name: String,
    pub(crate) folders: Vec<String>,
    /// Those of `folders` with a `.git` entry inside.
    pub(crate) dev_copies: Vec<String>,
}

/// The question that is open, if any. Built fresh from the app's state on
/// every draw and key press, so it never says anything that stopped being true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Question {
    DeleteAddons(Vec<DeleteTarget>),
    /// `when` is "today at 09:15", `ago` is "2 hours ago".
    RestoreBackup {
        when: String,
        ago: String,
    },
    DeleteBackup {
        when: String,
        ago: String,
    },
    /// `direct` is a GitHub, WoWInterface or TukUI install, `source` its
    /// label; otherwise Wago.io. `folders` are exactly what a yes replaces.
    /// `curseforge`: CurseForge also manages one of `folders`.
    ReplaceAddon {
        name: String,
        source: &'static str,
        direct: bool,
        folders: Vec<String>,
        curseforge: bool,
    },
    /// Saving a .toc field of a "Not linked" addon. `folders` are exactly
    /// the folders whose .toc files change; `restore` when the field goes
    /// back to the addon's own.
    ChangeTocFiles {
        name: String,
        field: &'static str,
        restore: bool,
        folders: Vec<String>,
    },
    /// `progress` is (done, total) once the update phase has started.
    QuitWhileUpdating {
        progress: Option<(usize, usize)>,
    },
    /// Esc on setup's first step. `rerun`: setup was opened from Settings,
    /// so a yes goes back to the list.
    QuitSetup {
        rerun: bool,
    },
    /// `x` on a game in the Switch game sheet. `backups` is where its
    /// backups of game settings stay.
    RemoveGame {
        game: String,
        backups: String,
    },
    /// Enter on "Link" in the Find sheet. `taken` are every folder the
    /// linked addons take over, `replaced` every folder on disk replaced now
    /// with the source's copy, and `added` the source's folders that aren't
    /// on disk yet. `curseforge` names the picked addons CurseForge also
    /// manages. The list scrolls from `offset`.
    LinkFound {
        links: usize,
        replaces: usize,
        taken: Vec<String>,
        replaced: Vec<String>,
        added: Vec<String>,
        curseforge: Vec<String>,
        offset: u16,
    },
    /// After a link or install, or `m` in Details: mark `names` Ignored in
    /// CurseForge, which still manages them.
    MarkIgnored {
        names: Vec<String>,
    },
    /// After an update run: these addons' new versions replace folders
    /// LemonUp didn't install. The list scrolls from `offset`.
    ReplaceNotLinked {
        addons: Vec<HeldEntry>,
        offset: u16,
    },
}

impl Question {
    pub(super) fn title(&self) -> String {
        match self {
            Self::DeleteAddons(targets) if targets.len() == 1 => {
                format!("Delete {}?", targets[0].name)
            }
            Self::DeleteAddons(targets) => format!("Delete {} addons?", targets.len()),
            Self::RestoreBackup { .. } => "Restore this backup?".to_string(),
            Self::DeleteBackup { .. } => "Delete this backup?".to_string(),
            Self::ReplaceAddon { name, .. } => format!("Replace your copy of {name}?"),
            Self::ChangeTocFiles { name, .. } => format!("Change the files of {name}?"),
            Self::QuitWhileUpdating { .. } => "Quit while LemonUp is still updating?".to_string(),
            Self::QuitSetup { .. } => "Quit setup?".to_string(),
            Self::RemoveGame { game, .. } => format!("Remove {game} from LemonUp?"),
            Self::LinkFound {
                links, replaces, ..
            } => match (*links, *replaces) {
                (links, 0) => format!("Link {}?", addon_count_label(links)),
                (0, replaces) => format!("Replace {}?", addon_count_label(replaces)),
                (links, replaces) => {
                    format!("Link {} and replace {replaces}?", addon_count_label(links))
                }
            },
            Self::MarkIgnored { names } => match names.as_slice() {
                [one] => format!("Mark {one} Ignored in CurseForge?"),
                _ => format!("Mark these {} addons Ignored in CurseForge?", names.len()),
            },
            Self::ReplaceNotLinked { addons, .. } => match addons.as_slice() {
                [one] => format!("Replace Not linked folders for {}?", one.name),
                _ => format!("Replace Not linked folders for {} addons?", addons.len()),
            },
        }
    }

    /// Delete and restore get the red border; replace and quit do not.
    pub(super) fn danger(&self) -> bool {
        matches!(
            self,
            Self::DeleteAddons(_)
                | Self::RestoreBackup { .. }
                | Self::DeleteBackup { .. }
                | Self::RemoveGame { .. }
        )
    }

    /// What `y` and `n` do, as outcomes.
    pub(super) fn answers(&self) -> (&'static str, &'static str) {
        match self {
            Self::DeleteAddons(targets) if targets.len() == 1 => ("delete", "keep"),
            Self::DeleteAddons(_) => ("delete", "keep"),
            Self::RestoreBackup { .. } => ("restore", "keep"),
            Self::DeleteBackup { .. } => ("delete", "keep"),
            Self::ReplaceAddon { .. } => ("replace", "keep"),
            Self::ChangeTocFiles { .. } => ("change", "keep"),
            Self::QuitWhileUpdating { .. } => ("quit", "continue"),
            Self::QuitSetup { rerun: false } => ("quit", "continue"),
            Self::QuitSetup { rerun: true } => ("back", "continue"),
            Self::RemoveGame { .. } => ("remove", "keep"),
            Self::LinkFound { replaces: 0, .. } => ("link", "skip"),
            Self::LinkFound { links: 0, .. } => ("replace", "skip"),
            Self::LinkFound { .. } => ("link and replace", "skip"),
            Self::MarkIgnored { .. } => ("mark", "skip"),
            Self::ReplaceNotLinked { .. } => ("replace", "skip"),
        }
    }

    /// The messages `y` and a "no" key send.
    fn messages(&self) -> (AppMessage, AppMessage) {
        match self {
            Self::DeleteAddons(_) => (
                AppMessage::DashboardConfirmDelete,
                AppMessage::DashboardCancelPendingDelete,
            ),
            Self::RestoreBackup { .. } => (
                AppMessage::BackupConfirmRestore,
                AppMessage::BackupCancelRestore,
            ),
            Self::DeleteBackup { .. } => (
                AppMessage::BackupConfirmDelete,
                AppMessage::BackupCancelRestore,
            ),
            Self::ReplaceAddon { direct: true, .. } => (
                AppMessage::SearchConfirmReplace,
                AppMessage::SearchCancelReplace,
            ),
            Self::ReplaceAddon { .. } => (
                AppMessage::WagoConfirmInstall,
                AppMessage::WagoCancelInstall,
            ),
            Self::ChangeTocFiles { .. } => (
                AppMessage::TocEdit(TocEditMessage::ConfirmApply),
                AppMessage::TocEdit(TocEditMessage::CancelApply),
            ),
            Self::QuitWhileUpdating { .. } => {
                (AppMessage::QuitConfirmed, AppMessage::QuitCancelled)
            }
            Self::QuitSetup { .. } => (
                AppMessage::OnboardingQuitConfirmed,
                AppMessage::OnboardingQuitCancelled,
            ),
            Self::RemoveGame { .. } => (
                AppMessage::Games(GamesMessage::ConfirmRemove),
                AppMessage::Games(GamesMessage::CancelRemove),
            ),
            Self::LinkFound { .. } => (
                AppMessage::Find(FindMessage::ConfirmLink),
                AppMessage::Find(FindMessage::CancelLink),
            ),
            Self::MarkIgnored { .. } => (
                AppMessage::CurseForge(CurseForgeMessage::Confirm),
                AppMessage::CurseForge(CurseForgeMessage::Skip),
            ),
            Self::ReplaceNotLinked { .. } => {
                (AppMessage::HeldUpdatesConfirm, AppMessage::HeldUpdatesSkip)
            }
        }
    }

    /// The body above the answers, before wrapping. `names` is how many
    /// addon names a delete question may list.
    fn body_lines(&self, roles: ColorRoles, names: usize) -> Vec<Line<'static>> {
        let text = Style::default().fg(roles.text);
        let strong = text.add_modifier(Modifier::BOLD);
        let muted = roles.muted;
        let key = Style::default().fg(roles.blue).add_modifier(Modifier::BOLD);
        match self {
            Self::DeleteAddons(targets) => {
                let mut lines = Vec::new();
                let shown = if targets.len() > names {
                    names.saturating_sub(1)
                } else {
                    targets.len()
                };
                let column = targets
                    .iter()
                    .take(shown)
                    .map(|target| target.name.chars().count())
                    .max()
                    .unwrap_or(0);
                for target in targets.iter().take(shown) {
                    let mut spans = vec![Span::styled(target.name.clone(), strong)];
                    if target.folders > 0 {
                        let pad = column - target.name.chars().count() + 2;
                        spans.push(Span::raw(" ".repeat(pad)));
                        spans.push(Span::styled(
                            format!(
                                "and the {} folder{} that belong{} to it",
                                target.folders,
                                plural_suffix(target.folders),
                                if target.folders == 1 { "s" } else { "" }
                            ),
                            muted,
                        ));
                    }
                    lines.push(Line::from(spans));
                }
                if shown < targets.len() {
                    lines.push(Line::from(Span::styled(
                        format!("and {} more", targets.len() - shown),
                        muted,
                    )));
                }
                let these = if targets.len() == 1 {
                    "this addon"
                } else {
                    "these addons"
                };
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("Your settings for {these} are not touched."),
                    muted,
                )));
                lines.push(Line::from(vec![
                    Span::styled("You can undo the delete with ", muted),
                    Span::styled("z", key),
                    Span::styled(".", muted),
                ]));
                lines
            }
            Self::RestoreBackup { when, ago } => vec![
                Line::from(vec![
                    Span::styled(format!("Backup from {when}"), strong),
                    Span::styled(format!(" ({ago})"), muted),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "This puts back the keybinds, macros and addon setups from that moment \
                     and replaces the ones you have now.",
                    text,
                )),
                Line::from(Span::styled(
                    "Close the game first, or it will overwrite them again.",
                    Style::default().fg(roles.amber),
                )),
            ],
            Self::DeleteBackup { when, ago } => vec![
                Line::from(vec![
                    Span::styled(format!("Backup from {when}"), strong),
                    Span::styled(format!(" ({ago})"), muted),
                ]),
                Line::from(""),
                Line::from(Span::styled("This can't be undone.", text)),
            ],
            Self::ReplaceAddon {
                name,
                source,
                folders,
                ..
            } => {
                let mut lines = vec![Line::from(Span::styled(
                    format!(
                        "{name} is already installed. Installing it from {source} replaces it."
                    ),
                    text,
                ))];
                if !folders.is_empty() {
                    let these = if folders.len() == 1 {
                        "This folder is replaced:"
                    } else {
                        "These folders are replaced:"
                    };
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(these, muted)));
                    let shown = if folders.len() > names {
                        names.saturating_sub(1)
                    } else {
                        folders.len()
                    };
                    for folder in folders.iter().take(shown) {
                        lines.push(Line::from(Span::styled(format!("  {folder}"), strong)));
                    }
                    if shown < folders.len() {
                        lines.push(Line::from(Span::styled(
                            format!("  and {} more", folders.len() - shown),
                            muted,
                        )));
                    }
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(Span::styled(
                    format!("Your {name} settings are kept."),
                    muted,
                )));
                lines
            }
            Self::ChangeTocFiles {
                name,
                field,
                restore,
                folders,
            } => {
                let what = if *restore {
                    format!("puts back the addon's own {field}")
                } else {
                    format!("writes your {field}")
                };
                let mut lines = vec![Line::from(Span::styled(
                    format!(
                        "LemonUp didn't install {name}. This {what} into the .toc files of \
                         these folders:"
                    ),
                    text,
                ))];
                let shown = if folders.len() > names {
                    names.saturating_sub(1)
                } else {
                    folders.len()
                };
                for folder in folders.iter().take(shown) {
                    lines.push(Line::from(Span::styled(format!("  {folder}"), strong)));
                }
                if shown < folders.len() {
                    lines.push(Line::from(Span::styled(
                        format!("  and {} more", folders.len() - shown),
                        muted,
                    )));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "The addon's own lines are kept, so you can restore them by clearing the field.",
                    muted,
                )));
                lines
            }
            Self::QuitWhileUpdating { progress } => {
                let first = match progress {
                    Some((done, total)) => format!(
                        "{done} of {total} addon{} {} done.",
                        plural_suffix(*total),
                        if *done == 1 { "is" } else { "are" }
                    ),
                    None => "LemonUp is still working out which addons to update.".to_string(),
                };
                vec![Line::from(Span::styled(
                    format!("{first} If you quit now, the update stops."),
                    text,
                ))]
            }
            Self::QuitSetup { rerun: false } => vec![Line::from(Span::styled(
                "Nothing is saved yet. LemonUp opens setup again the next time you start it.",
                text,
            ))],
            Self::QuitSetup { rerun: true } => vec![Line::from(Span::styled(
                "Nothing you chose here is saved. Your game folder and settings stay as they were.",
                text,
            ))],
            Self::RemoveGame { game, backups } => vec![
                Line::from(Span::styled(
                    format!(
                        "LemonUp forgets the addon list and settings it keeps for {game}. It doesn't delete any addons or game settings."
                    ),
                    text,
                )),
                Line::from(vec![
                    Span::styled("Its backups of your game settings stay in ", muted),
                    Span::styled(backups.clone(), text),
                    Span::styled(".", muted),
                ]),
            ],
            Self::LinkFound {
                links,
                taken,
                replaced,
                added,
                ..
            } => {
                let mut lines = Vec::new();
                if *links > 0 {
                    lines.push(Line::from(Span::styled(
                        if *links == 1 {
                            "Link 1 addon to its source? Its updates replace these folders:"
                                .to_string()
                        } else {
                            format!(
                                "Link {links} addons to their source? Their updates replace \
                                 these folders:"
                            )
                        },
                        text,
                    )));
                    for folder in taken {
                        lines.push(Line::from(Span::styled(format!("  {folder}"), strong)));
                    }
                }
                if !replaced.is_empty() {
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(Span::styled(
                        "These folders are replaced now with the copy from Wago.io:",
                        text,
                    )));
                    for folder in replaced {
                        lines.push(Line::from(Span::styled(format!("  {folder}"), strong)));
                    }
                }
                if !added.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "These folders are added from Wago.io:",
                        text,
                    )));
                    for folder in added {
                        lines.push(Line::from(Span::styled(format!("  {folder}"), strong)));
                    }
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Your addon settings are kept.",
                    muted,
                )));
                lines
            }
            Self::MarkIgnored { names: all } => {
                let mut lines = Vec::new();
                let (these, them) = if all.len() == 1 {
                    ("this addon", "it")
                } else {
                    let shown = if all.len() > names {
                        names.saturating_sub(1).max(1)
                    } else {
                        all.len()
                    };
                    let mut list = all[..shown].join(", ");
                    if shown < all.len() {
                        list.push_str(&format!(" and {} more", all.len() - shown));
                    }
                    lines.push(Line::from(Span::styled(list, strong)));
                    lines.push(Line::from(""));
                    ("these addons", "them")
                };
                lines.push(Line::from(Span::styled(
                    format!(
                        "CurseForge also manages {these}. Once Ignored there, CurseForge \
                         leaves {them} to LemonUp."
                    ),
                    text,
                )));
                lines.push(Line::from(Span::styled(
                    "LemonUp backs up CurseForge's list first.",
                    muted,
                )));
                lines
            }
            Self::ReplaceNotLinked { addons, .. } => {
                let mut lines = vec![Line::from(Span::styled(
                    if addons.len() == 1 {
                        "The update replaces folders LemonUp didn't install:"
                    } else {
                        "These updates replace folders LemonUp didn't install:"
                    },
                    text,
                ))];
                for addon in addons {
                    let mut spans = vec![Span::styled(format!("{}: ", addon.name), strong)];
                    for (index, folder) in addon.folders.iter().enumerate() {
                        if index > 0 {
                            spans.push(Span::styled(", ", text));
                        }
                        spans.push(Span::styled(folder.clone(), text));
                        if addon.dev_copies.contains(folder) {
                            spans.push(Span::styled(
                                " (has .git, likely a dev copy)",
                                Style::default().fg(roles.amber),
                            ));
                        }
                    }
                    lines.push(Line::from(spans));
                }
                lines
            }
        }
    }

    /// The note under the body that the folders are also CurseForge's, so
    /// its updates would fight LemonUp's copy.
    fn curseforge_note(&self, roles: ColorRoles) -> Option<Line<'static>> {
        let text = match self {
            Self::ReplaceAddon {
                curseforge: true, ..
            } => lemonup_core::curseforge::HINT.to_string(),
            Self::LinkFound { curseforge, .. } if !curseforge.is_empty() => {
                lemonup_core::curseforge::hint_naming(curseforge)
            }
            _ => return None,
        };
        Some(Line::from(Span::styled(
            text,
            Style::default().fg(roles.amber),
        )))
    }

    /// The answers row: `    y  delete          n  keep`.
    fn answer_line(&self, roles: ColorRoles) -> Line<'static> {
        let (yes, no) = self.answers();
        let yes_key = if self.danger() {
            roles.red
        } else {
            roles.lemon
        };
        Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "y",
                Style::default().fg(yes_key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {yes}"), Style::default().fg(roles.text)),
            Span::raw("          "),
            Span::styled(
                "n",
                Style::default().fg(roles.blue).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {no}"), Style::default().fg(roles.text)),
        ])
    }

    /// Every row inside the border at `width` columns, at most `max_rows`
    /// of them: a long list of addons gives way to "and N more" first.
    pub(super) fn lines(&self, roles: ColorRoles, width: u16, max_rows: u16) -> Vec<Line<'static>> {
        if let Self::LinkFound { offset, .. } | Self::ReplaceNotLinked { offset, .. } = self {
            return self.scrolled_lines(roles, width, max_rows, *offset).0;
        }
        let note = self.curseforge_note(roles);
        let build = |names: usize| {
            let mut lines = vec![Line::from("")];
            lines.extend(wrap_lines(&self.body_lines(roles, names), width));
            if let Some(note) = &note {
                lines.push(Line::from(""));
                lines.extend(wrap_lines(std::slice::from_ref(note), width));
            }
            lines.push(Line::from(""));
            lines.extend(wrap_lines(&[self.answer_line(roles)], width));
            lines.push(Line::from(""));
            lines
        };
        let mut names = match self {
            Self::DeleteAddons(targets) => targets.len().min(DIALOG_MAX_NAMES),
            Self::ReplaceAddon { folders, .. } | Self::ChangeTocFiles { folders, .. } => {
                folders.len().min(DIALOG_MAX_NAMES)
            }
            Self::MarkIgnored { names } => names.len().min(DIALOG_MAX_NAMES),
            _ => 0,
        };
        let mut lines = build(names);
        while lines.len() > usize::from(max_rows) && names > 1 {
            names -= 1;
            lines = build(names);
        }
        lines
    }
}

impl Question {
    /// A question whose folder list scrolls instead of saying "and N more":
    /// the rows from `offset` that fit, then where the rest is. Also returns
    /// how far it scrolls.
    pub(super) fn scrolled_lines(
        &self,
        roles: ColorRoles,
        width: u16,
        max_rows: u16,
        offset: u16,
    ) -> (Vec<Line<'static>>, u16) {
        let body = wrap_lines(&self.body_lines(roles, usize::MAX), width);
        let answers = wrap_lines(&[self.answer_line(roles)], width);
        // The note stays in view under the list, a blank row above it.
        let note = self
            .curseforge_note(roles)
            .map(|note| wrap_lines(&[note], width))
            .unwrap_or_default();
        let note_rows = if note.is_empty() { 0 } else { note.len() + 1 };
        // A blank row above the body, and above and below the answers.
        let room = usize::from(max_rows)
            .saturating_sub(answers.len() + 3 + note_rows)
            .max(1);
        let mut lines = vec![Line::from("")];
        let max = if body.len() <= room {
            lines.extend(body);
            0
        } else {
            let shown = room.saturating_sub(1).max(1);
            let max = body.len() - shown;
            let start = usize::from(offset).min(max);
            lines.extend(body.iter().skip(start).take(shown).cloned());
            let below = body.len() - start - shown;
            let where_ = match (start > 0, below > 0) {
                (_, true) => format!("↓ {below} more, ↑/↓ scroll"),
                (true, false) => "↑/↓ scroll".to_string(),
                (false, false) => String::new(),
            };
            lines.push(Line::from(Span::styled(where_, roles.muted)));
            u16::try_from(max).unwrap_or(u16::MAX)
        };
        if !note.is_empty() {
            lines.push(Line::from(""));
            lines.extend(note);
        }
        lines.push(Line::from(""));
        lines.extend(answers);
        lines.push(Line::from(""));
        (lines, max)
    }
}

impl App {
    /// Remembers the question a draw shows and when it first showed it.
    pub(super) fn note_question_shown(&mut self, now: Instant) {
        match self.active_question() {
            Some(question) => {
                if self
                    .question_shown
                    .as_ref()
                    .is_none_or(|(shown, _)| *shown != question)
                {
                    self.question_shown = Some((question, now));
                }
            }
            None => self.question_shown = None,
        }
    }

    /// Whether `question` was on screen before the key being handled was read.
    fn question_seen_before_key(&self, question: &Question) -> bool {
        match &self.question_shown {
            Some((shown, shown_at)) if shown == question => {
                self.key_read_at.is_none_or(|read_at| read_at >= *shown_at)
            }
            _ => false,
        }
    }

    /// The question waiting for an answer, if any. Quitting comes first: it
    /// can be asked on top of another question.
    pub(super) fn active_question(&self) -> Option<Question> {
        if self.shell_mode == ShellMode::Onboarding {
            return self.onboarding.quit_question.then(|| Question::QuitSetup {
                rerun: self.setup_is_rerun(),
            });
        }
        if self.quit_question_open && self.dashboard.update_in_progress() {
            let progress = self
                .dashboard
                .job_ui()
                .filter(|job| job.kind == DashboardJobKind::Update)
                .map(|job| (job.current_index.saturating_sub(1), job.total));
            return Some(Question::QuitWhileUpdating { progress });
        }
        if let Some(folders) = self.dashboard.pending_delete_folders() {
            let targets = folders
                .iter()
                .map(|folder| {
                    let item = self
                        .dashboard
                        .items
                        .iter()
                        .find(|item| item.folder == *folder);
                    DeleteTarget {
                        name: item.map_or_else(|| folder.clone(), |item| item.name.clone()),
                        folders: item.map_or(0, |item| item.owned_folder_count),
                    }
                })
                .collect();
            return Some(Question::DeleteAddons(targets));
        }
        if let Some(held) = &self.held_question {
            return Some(Question::ReplaceNotLinked {
                addons: held.entries.clone(),
                offset: held.offset,
            });
        }
        if let Some(confirmation) = self.pending_wago_install_confirmation.as_ref() {
            return Some(Question::ReplaceAddon {
                name: confirmation.inspection.addon_name.clone(),
                source: "Wago.io",
                direct: false,
                folders: confirmation.inspection.existing_folders.clone(),
                curseforge: confirmation.curseforge,
            });
        }
        if self.dashboard.detail_mode == DetailMode::Search
            && let Some((request, name, folders, curseforge)) =
                self.search_pane.pending_direct_replace()
        {
            return Some(Question::ReplaceAddon {
                name: name.to_string(),
                source: request.provider.label(),
                direct: true,
                folders: folders.to_vec(),
                curseforge,
            });
        }
        if let Some(question) = self.toc_apply_question() {
            return Some(question);
        }
        if let Some(backup) = &self.backup_pane.pending_restore {
            return Some(Question::RestoreBackup {
                when: self.backup_when_phrase(backup),
                ago: self.backup_age_phrase(backup),
            });
        }
        if let Some(backup) = &self.backup_pane.pending_delete {
            return Some(Question::DeleteBackup {
                when: self.backup_when_phrase(backup),
                ago: self.backup_age_phrase(backup),
            });
        }
        if let Some(question) = self.remove_game_question() {
            return Some(question);
        }
        if let Some(question) = self.find_question() {
            return Some(question);
        }
        if let Some(mark) = &self.curseforge_mark {
            return Some(Question::MarkIgnored {
                names: mark.names.clone(),
            });
        }
        None
    }

    /// Keys while a question is open: `y` yes, `n` `esc` `q` no, and every
    /// other key does nothing. `None` when no question is open.
    ///
    /// A replace question, and the CurseForge one after an install or link,
    /// arrive from background work, so their `y` counts only when read after
    /// the question was first drawn: a `y` typed while the work ran never
    /// answers it.
    pub(super) fn question_messages_for_key(&self, key: KeyEvent) -> Option<Vec<AppMessage>> {
        let question = self.active_question()?;
        if key.code == KeyCode::Char('y')
            && matches!(
                question,
                Question::ReplaceAddon { .. }
                    | Question::MarkIgnored { .. }
                    | Question::ReplaceNotLinked { .. }
            )
            && !self.question_seen_before_key(&question)
        {
            return Some(vec![]);
        }
        let (yes, no) = question.messages();
        // The Find question's folder list scrolls, when it doesn't fit.
        if matches!(question, Question::ReplaceNotLinked { .. })
            && let Some(delta) = question_scroll_delta(key.code)
        {
            let scrolls = self
                .held_question
                .as_ref()
                .is_some_and(|held| held.max.get() > 0);
            return Some(if scrolls {
                vec![AppMessage::HeldUpdatesScroll(delta)]
            } else {
                vec![]
            });
        }
        if matches!(question, Question::LinkFound { .. })
            && let Some(delta) = question_scroll_delta(key.code)
        {
            return Some(if self.find.question_max.get() > 0 {
                vec![AppMessage::Find(FindMessage::QuestionScroll(delta))]
            } else {
                vec![]
            });
        }
        Some(match key.code {
            KeyCode::Char('y') => vec![yes],
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => vec![no],
            _ => vec![],
        })
    }

    /// The key line while a question is open.
    pub(super) fn question_key_table(&self) -> Vec<KeyHint> {
        let mut hints = Vec::new();
        let scrolls = match self.active_question() {
            Some(Question::LinkFound { .. }) => self.find.question_max.get() > 0,
            Some(Question::ReplaceNotLinked { .. }) => self
                .held_question
                .as_ref()
                .is_some_and(|held| held.max.get() > 0),
            _ => false,
        };
        if scrolls {
            hints.push(KeyHint::key(
                FooterHintId::Scroll,
                "↑/↓",
                "scroll",
                &[
                    KeyCode::Up,
                    KeyCode::Down,
                    KeyCode::Char('j'),
                    KeyCode::Char('k'),
                    KeyCode::PageUp,
                    KeyCode::PageDown,
                ],
            ));
        }
        hints.extend([
            KeyHint::key(FooterHintId::Confirm, "y", "yes", &[KeyCode::Char('y')]).kept(),
            KeyHint::key(
                FooterHintId::Cancel,
                "n",
                "no",
                &[KeyCode::Char('n'), KeyCode::Char('q')],
            )
            .kept(),
            KeyHint::key(FooterHintId::Close, "esc", "no", &[KeyCode::Esc]).kept(),
        ]);
        hints
    }

    /// Draws the open question centered in `region`, the space above the
    /// status line and the key line, over a dimmed screen.
    pub(super) fn render_question(&self, frame: &mut Frame<'_>, region: Rect) {
        let Some(question) = self.active_question() else {
            return;
        };
        if region.width < 4 || region.height < 3 {
            return;
        }
        let roles = self.ui_theme.roles;
        let width = DIALOG_WIDTH.min(region.width.saturating_sub(2).max(4));
        let text_width = width.saturating_sub(2 + DIALOG_PADDING * 2).max(1);
        let max_rows = region.height.saturating_sub(2);
        let lines = match &question {
            Question::LinkFound { offset, .. } => {
                let (lines, max) = question.scrolled_lines(roles, text_width, max_rows, *offset);
                self.find.question_max.set(max);
                lines
            }
            Question::ReplaceNotLinked { offset, .. } => {
                let (lines, max) = question.scrolled_lines(roles, text_width, max_rows, *offset);
                if let Some(held) = &self.held_question {
                    held.max.set(max);
                }
                lines
            }
            _ => question.lines(roles, text_width, max_rows),
        };
        let height = (u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2).min(region.height);
        let outer = Rect::new(
            region.x + (region.width - width) / 2,
            region.y + (region.height - height) / 2,
            width,
            height,
        );

        dim_area(frame.buffer_mut(), region, roles);
        frame.render_widget(Clear, outer);
        let outline = Style::default().fg(if question.danger() {
            roles.red
        } else {
            roles.border
        });
        let title_style = Style::default()
            .fg(if question.danger() {
                roles.red
            } else {
                roles.text
            })
            .add_modifier(Modifier::BOLD);
        let title = truncate_text(&question.title(), usize::from(width.saturating_sub(6)));
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_set(symbols::border::ROUNDED)
                .border_style(outline)
                .style(Style::default().bg(roles.sheet).fg(roles.text))
                .title(Line::from(vec![
                    Span::styled("─ ", outline),
                    Span::styled(title, title_style),
                    Span::styled(" ", outline),
                ])),
            outer,
        );
        let inner = Rect::new(
            outer.x + 1 + DIALOG_PADDING,
            outer.y + 1,
            text_width.min(outer.width.saturating_sub(2)),
            outer.height.saturating_sub(2),
        );
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// How far a key scrolls a question's list, if it does.
fn question_scroll_delta(code: KeyCode) -> Option<i16> {
    match code {
        KeyCode::Down | KeyCode::Char('j') => Some(1),
        KeyCode::Up | KeyCode::Char('k') => Some(-1),
        KeyCode::PageDown => Some(8),
        KeyCode::PageUp => Some(-8),
        _ => None,
    }
}
