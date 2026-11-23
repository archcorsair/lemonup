//! The four `install-*` commands: the addon that was installed, or that a dry run would install.

use lemonup_core::SourceKind;
use serde::Serialize;

use super::{INDENT, Paint, Report, columns, context_line, display_version, wrap_units};
use crate::find::FindTarget;
use crate::source_error::{SourceError, SourceErrorKind};

/// What an install command was asked for, for the words of a failure.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstallRequest<'a> {
    pub(crate) source: SourceKind,
    /// The addon as the command was given it: a name or a link.
    pub(crate) target: &'a str,
    pub(crate) profile: &'a str,
    /// The run named its AddOns folder with `--addon-dir`.
    pub(crate) addon_dir_given: bool,
}

/// Why an install failed, to stand under "Install failed", and the command to run next when
/// there is one. The kind decides, never the message's text; the words are the ones the TUI's
/// install sheet uses. Where those don't say what was found, the source's own words are the
/// reason. The reason can hold a source's or an addon's own words; the hint is LemonUp's.
pub(crate) fn failure(error: &SourceError, request: InstallRequest) -> (String, Option<String>) {
    let InstallRequest { source, target, .. } = request;
    let advice = || {
        crate::app::render_install_search::install_error_advice(
            error.kind,
            super::source_label(source),
            error.clash.as_ref(),
        )
    };
    // The addon itself is there already, and linked: installing it again is an update. This
    // comes as folders of a linked addon, or from Wago.io as folders in the way.
    let installed = error
        .clash
        .as_ref()
        .and_then(|clash| clash.installed.as_deref())
        .filter(|_| {
            matches!(
                error.kind,
                SourceErrorKind::FoldersOfLinkedAddon | SourceErrorKind::FoldersInTheWay
            )
        });
    if let Some(installed) = installed {
        // Linked to another source than the one asked for: say which, since that is where
        // the update installs it from.
        let reason = if installed.source == source {
            format!("{} is already installed.", installed.name)
        } else {
            format!(
                "{} is already installed from {}.",
                installed.name,
                super::source_label(installed.source)
            )
        };
        return (reason, Some(reinstall_hint(&installed.folder, request)));
    }
    let reason = match (error.kind, &error.clash) {
        (SourceErrorKind::Other, _) => error.detail.clone(),
        // No period after it: one would end up in a link that is copied or clicked.
        (SourceErrorKind::NotFound, _) if target.contains("://") => {
            format!("Couldn't find an addon at {target}")
        }
        (SourceErrorKind::NotFound, _) => {
            format!("Couldn't find {target} on {}", super::source_label(source))
        }
        // The TUI asks about these folders next; a command can only name them.
        (SourceErrorKind::FoldersInTheWay, Some(clash)) if !clash.replaceable.is_empty() => {
            format!(
                "{}: {}",
                advice().trim_end_matches('.'),
                clash.replaceable.join(", ")
            )
        }
        (SourceErrorKind::FoldersInTheWay, _) => error.detail.clone(),
        _ => advice(),
    };
    (reason, None)
}

/// The command that installs the linked addon in `folder` again from its source. The folder
/// goes into the command only when it is a plain word, or plain words to put in quotes, that
/// every shell reads as one argument and `lemonup update` takes as a selector. A folder can
/// have any name a zip gave it, so anything else is left for the reader to type.
fn reinstall_hint(folder: &str, request: InstallRequest) -> String {
    let lemonup = if request.profile == lemonup_core::DEFAULT_PROFILE {
        "lemonup".to_string()
    } else {
        format!("lemonup --profile {}", request.profile)
    };
    let with = if request.addon_dir_given {
        " with the same `--addon-dir`"
    } else {
        ""
    };
    match update_selector(folder) {
        Some(selector) => {
            format!("Run `{lemonup} update {selector} --force`{with} to install it again.")
        }
        None => format!(
            "Run `{lemonup} update` with its folder name and `--force`{with} to install it \
             again."
        ),
    }
}

/// `folder` as an argument of `lemonup update`, or `None` when it is not safe to paste: only
/// ASCII letters, digits, spaces, `_`, `-` and `.`, not starting with `-` (a flag), without
/// `..`, and no longer than a selector may be. No quote of any kind and nothing a shell
/// expands gets through, in bash, zsh, nushell or PowerShell.
fn update_selector(folder: &str) -> Option<String> {
    let plain = |character: char| {
        character.is_ascii_alphanumeric() || matches!(character, ' ' | '_' | '-' | '.')
    };
    let safe = !folder.is_empty()
        && folder.len() <= 120
        && folder.trim() == folder
        && !folder.starts_with('-')
        && !folder.contains("..")
        && folder.chars().all(plain);
    safe.then(|| {
        if folder.contains(' ') {
            format!("\"{folder}\"")
        } else {
            folder.to_string()
        }
    })
}

/// What an `install-*` command prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallReport {
    #[serde(skip)]
    target: FindTarget,
    dry_run: bool,
    addon: InstalledAddon,
}

/// The addon an install command installed, as its source names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstalledAddon {
    pub(crate) name: String,
    #[serde(serialize_with = "super::serialize_source")]
    pub(crate) source: SourceKind,
    /// What the source knows the addon by: a Wago.io or WoWInterface id, a TukUI name, or a
    /// GitHub address.
    pub(crate) id: String,
    pub(crate) version: Option<String>,
    /// Wago.io only: `stable`, `beta` or `alpha`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) channel: Option<&'static str>,
    /// GitHub only: the release whose zip was installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) release: Option<String>,
    /// GitHub only: the commit of the source that was installed, when there is no release zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) commit: Option<String>,
    pub(crate) parent_folder: String,
    pub(crate) folders: Vec<String>,
}

impl InstallReport {
    pub(crate) fn new(target: FindTarget, dry_run: bool, addon: InstalledAddon) -> Self {
        Self {
            target,
            dry_run,
            addon,
        }
    }

    /// The AddOns folder the report is about, for the JSON document.
    pub(crate) fn addon_dir(&self) -> &std::path::Path {
        &self.target.addon_dir
    }

    /// "WeakAuras 5.19.0 from GitHub", with the channel when Wago.io's is not the stable one:
    /// the name bold, the version green like the new version of an update, the source muted,
    /// and a channel that is not stable in yellow.
    fn what(&self, paint: Paint) -> String {
        let addon = &self.addon;
        let mut what = paint.bold(&addon.name);
        // An addon without a version is named alone, not as "unknown".
        if addon
            .version
            .as_deref()
            .is_some_and(|version| !version.trim().is_empty())
        {
            what.push(' ');
            what.push_str(&paint.added(&display_version(
                addon.version.as_deref(),
                &[&addon.name, &addon.parent_folder],
            )));
        }
        what.push(' ');
        match addon.channel.filter(|channel| *channel != "stable") {
            Some(channel) => {
                what.push_str(
                    &paint.muted(&format!("from {},", super::source_label(addon.source))),
                );
                what.push(' ');
                what.push_str(&paint.attention(channel));
            }
            None => {
                what.push_str(&paint.muted(&format!("from {}", super::source_label(addon.source))))
            }
        }
        what
    }
}

impl Report for InstallReport {
    fn command(&self) -> &'static str {
        match self.addon.source {
            SourceKind::Wago => "install-wago",
            SourceKind::Tukui => "install-tukui",
            SourceKind::WowInterface => "install-wowinterface",
            // No install command installs a Not linked addon; GitHub is the one left.
            SourceKind::GitHub | SourceKind::Manual => "install-github",
        }
    }

    /// The game, then what was installed and the folders it brought. A dry run says what it
    /// would install, with no done mark, and that nothing changed.
    fn human(&self, paint: Paint) -> Vec<String> {
        let target = &self.target;
        let mut lines = vec![
            context_line(paint, &target.game, &target.addon_dir, &target.profile),
            String::new(),
        ];
        lines.push(if self.dry_run {
            format!("{} {}", paint.text("Would install"), self.what(paint))
        } else {
            format!(
                "{} {} {}",
                paint.done(),
                paint.text("Installed"),
                self.what(paint)
            )
        });

        let key = "Folders";
        let folders = self
            .addon
            .folders
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                if index + 1 < self.addon.folders.len() {
                    format!("{folder},")
                } else {
                    folder.clone()
                }
            })
            .collect::<Vec<_>>();
        // The folders wrap under the first one, past the key.
        let hang = " ".repeat(columns(INDENT) + columns(key) + 2);
        for (index, line) in wrap_units(&hang, &folders, paint.width())
            .iter()
            .enumerate()
        {
            let names = paint.text(line.strip_prefix(hang.as_str()).unwrap_or(line));
            lines.push(if index == 0 {
                format!("{INDENT}{}  {names}", paint.bold(key))
            } else {
                format!("{hang}{names}")
            });
        }

        if self.dry_run {
            lines.push(String::new());
            lines.push(paint.attention("Dry run - no changes"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lemonup_core::{GameFlavor, SourceKind};

    use super::{InstallReport, InstalledAddon};
    use crate::find::FindTarget;
    use crate::output::tests::{json_document, strip};
    use crate::output::{Paint, Report};
    use crate::theme::Symbols;

    fn target(profile: &str) -> FindTarget {
        FindTarget::new(
            profile,
            Path::new("/games/wow/_retail_/Interface/AddOns"),
            GameFlavor::Retail,
            &[],
        )
    }

    fn weakauras() -> InstalledAddon {
        InstalledAddon {
            name: "WeakAuras".to_string(),
            source: SourceKind::GitHub,
            id: "https://github.com/WeakAuras/WeakAuras2".to_string(),
            version: Some("5.19.0".to_string()),
            channel: None,
            release: Some("5.19.0".to_string()),
            commit: None,
            parent_folder: "WeakAuras".to_string(),
            folders: vec![
                "WeakAuras".to_string(),
                "WeakAurasArchive".to_string(),
                "WeakAurasOptions".to_string(),
            ],
        }
    }

    fn wago(channel: &'static str) -> InstalledAddon {
        InstalledAddon {
            name: "Deadly Boss Mods".to_string(),
            source: SourceKind::Wago,
            id: "qv63A7Gb".to_string(),
            version: Some("12.0.10".to_string()),
            channel: Some(channel),
            release: None,
            commit: None,
            parent_folder: "DBM-Core".to_string(),
            folders: vec!["DBM-Core".to_string()],
        }
    }

    fn report(dry_run: bool, addon: InstalledAddon) -> InstallReport {
        InstallReport::new(target("default"), dry_run, addon)
    }

    #[test]
    fn an_install_names_the_addon_its_version_its_source_and_its_folders() {
        assert_eq!(
            report(false, weakauras()).human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "✓ Installed WeakAuras 5.19.0 from GitHub",
                "  Folders  WeakAuras, WeakAurasArchive, WeakAurasOptions",
            ]
        );
        assert_eq!(
            report(false, weakauras()).human(Paint::PLAIN.with_symbols(Symbols::SIMPLE))[2],
            "ok Installed WeakAuras 5.19.0 from GitHub"
        );
        assert_eq!(
            InstallReport::new(target("forever"), false, wago("stable")).human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns  (profile forever)",
                "",
                "✓ Installed Deadly Boss Mods 12.0.10 from Wago.io",
                "  Folders  DBM-Core",
            ]
        );
    }

    #[test]
    fn a_dry_run_says_would_install_with_no_done_mark_and_that_nothing_changed() {
        assert_eq!(
            report(true, weakauras()).human(Paint::PLAIN),
            [
                "Retail  /games/wow/_retail_/Interface/AddOns",
                "",
                "Would install WeakAuras 5.19.0 from GitHub",
                "  Folders  WeakAuras, WeakAurasArchive, WeakAurasOptions",
                "",
                "Dry run - no changes",
            ]
        );
    }

    #[test]
    fn the_channel_shows_only_when_it_is_not_stable() {
        assert_eq!(
            report(false, wago("beta")).human(Paint::PLAIN)[2],
            "✓ Installed Deadly Boss Mods 12.0.10 from Wago.io, beta"
        );
        assert_eq!(
            report(false, wago("alpha")).human(Paint::PLAIN)[2],
            "✓ Installed Deadly Boss Mods 12.0.10 from Wago.io, alpha"
        );
        assert_eq!(
            report(false, wago("stable")).human(Paint::PLAIN)[2],
            "✓ Installed Deadly Boss Mods 12.0.10 from Wago.io"
        );
    }

    #[test]
    fn a_version_is_shown_short_and_left_out_when_there_is_none() {
        // The addon's own name in front of a version is not repeated.
        let mut named = weakauras();
        named.version = Some("WeakAuras-5.19.0".to_string());
        assert_eq!(
            report(false, named).human(Paint::PLAIN)[2],
            "✓ Installed WeakAuras 5.19.0 from GitHub"
        );
        // A commit is shown by its first seven characters.
        let mut commit = weakauras();
        commit.version = Some("0123456789abcdef0123456789abcdef01234567".to_string());
        assert_eq!(
            report(false, commit).human(Paint::PLAIN)[2],
            "✓ Installed WeakAuras 0123456 from GitHub"
        );
        for version in [None, Some("  ".to_string())] {
            let mut none = weakauras();
            none.version = version;
            assert_eq!(
                report(false, none).human(Paint::PLAIN)[2],
                "✓ Installed WeakAuras from GitHub"
            );
        }
    }

    #[test]
    fn many_folders_wrap_under_the_first_one() {
        let mut addon = weakauras();
        addon.folders = (1..=9)
            .map(|index| format!("ElvUI_Plugin{index}"))
            .collect();
        assert_eq!(
            report(false, addon).human(Paint::PLAIN.with_width(60))[3..],
            [
                "  Folders  ElvUI_Plugin1, ElvUI_Plugin2, ElvUI_Plugin3,",
                "           ElvUI_Plugin4, ElvUI_Plugin5, ElvUI_Plugin6,",
                "           ElvUI_Plugin7, ElvUI_Plugin8, ElvUI_Plugin9",
            ]
        );
    }

    #[test]
    fn colored_lines_use_only_the_16_colors_and_keep_the_text() {
        let colored = report(false, weakauras()).human(Paint::terminal());
        let joined = colored.join("\n");
        assert!(!joined.contains("38;2;"), "24-bit color: {joined:?}");
        assert!(!joined.contains("48;"), "background: {joined:?}");
        // The name is bold, the version green, the source muted.
        assert_eq!(
            colored[2],
            "\x1b[38;5;2m✓\x1b[0m Installed \x1b[1mWeakAuras\x1b[0m \
             \x1b[38;5;2m5.19.0\x1b[0m \x1b[2mfrom GitHub\x1b[0m"
        );
        assert_eq!(
            colored[3],
            "  \x1b[1mFolders\x1b[0m  WeakAuras, WeakAurasArchive, WeakAurasOptions"
        );
        let stripped = colored.iter().map(|line| strip(line)).collect::<Vec<_>>();
        assert_eq!(stripped, report(false, weakauras()).human(Paint::PLAIN));

        // A dry run has no done mark, and its last line is yellow.
        let dry = report(true, weakauras()).human(Paint::terminal());
        assert_eq!(
            dry[2],
            "Would install \x1b[1mWeakAuras\x1b[0m \x1b[38;5;2m5.19.0\x1b[0m \
             \x1b[2mfrom GitHub\x1b[0m"
        );
        // A channel that is not the stable one is yellow.
        assert_eq!(
            report(false, wago("beta")).human(Paint::terminal())[2],
            "\x1b[38;5;2m✓\x1b[0m Installed \x1b[1mDeadly Boss Mods\x1b[0m \
             \x1b[38;5;2m12.0.10\x1b[0m \x1b[2mfrom Wago.io,\x1b[0m \x1b[38;5;3mbeta\x1b[0m"
        );
        assert_eq!(
            dry.last().map(String::as_str),
            Some("\x1b[38;5;3mDry run - no changes\x1b[0m")
        );
    }

    #[test]
    fn names_and_folders_lose_their_control_characters() {
        let mut addon = weakauras();
        addon.name = "Evil\x1b]0;x\x07".to_string();
        addon.folders = vec!["Evil\x1b[31m".to_string()];
        for paint in [Paint::PLAIN, Paint::terminal()] {
            let lines = report(false, addon.clone()).human(paint);
            // The escape codes of the name are gone before the line is drawn.
            assert!(!lines[2].contains("\x1b]") && !lines[2].contains('\x07'));
            assert!(
                strip(&lines[2]).ends_with("Installed Evil ]0;x  5.19.0 from GitHub"),
                "{lines:?}"
            );
            assert!(lines[3].ends_with("  Evil [31m"), "{lines:?}");
        }
    }

    fn request(source: SourceKind, target: &str) -> super::InstallRequest<'_> {
        super::InstallRequest {
            source,
            target,
            profile: "default",
            addon_dir_given: false,
        }
    }

    #[test]
    fn a_failure_says_its_cause_in_the_words_of_the_install_sheet() {
        use crate::source_error::{FolderClash, SourceError, SourceErrorKind};
        let reason = |kind, source| {
            super::failure(
                &SourceError::new(kind, "raw words of the source"),
                request(source, "elvui"),
            )
        };
        let said = |words: &str| (words.to_string(), None);
        assert_eq!(
            reason(SourceErrorKind::NoConnection, SourceKind::GitHub),
            said("LemonUp couldn't reach GitHub. Check your internet connection and try again.")
        );
        assert_eq!(
            reason(SourceErrorKind::KeyRejected, SourceKind::Wago),
            said("Wago.io didn't accept your Wago.io key. Check it and paste it again.")
        );
        assert_eq!(
            reason(SourceErrorKind::KeyRejected, SourceKind::GitHub),
            said(
                "GitHub didn't accept the token LemonUp sent (GITHUB_TOKEN or GH_TOKEN). Fix it \
                 or unset it, then try again."
            )
        );
        assert_eq!(
            reason(SourceErrorKind::SlowDown, SourceKind::WowInterface),
            said("WoWInterface asked LemonUp to slow down. Wait a few minutes, then try again.")
        );
        assert_eq!(
            reason(SourceErrorKind::WrongRepository, SourceKind::GitHub),
            said(
                "The repository may have a new owner. LemonUp won't install it until its list \
                 is updated."
            )
        );
        // An addon that isn't there is named as the command was given it: a name, or a link.
        assert_eq!(
            reason(SourceErrorKind::NotFound, SourceKind::Tukui),
            said("Couldn't find elvui on TukUI")
        );
        assert_eq!(
            super::failure(
                &SourceError::new(SourceErrorKind::NotFound, "raw"),
                request(SourceKind::GitHub, "https://github.com/archcorsair/nope"),
            ),
            said("Couldn't find an addon at https://github.com/archcorsair/nope")
        );
        // Anything else is said in the source's own words: no advice would say what happened.
        assert_eq!(
            reason(SourceErrorKind::Other, SourceKind::GitHub),
            said("raw words of the source")
        );

        // Folders of another linked addon are named with the addon they belong to.
        let mut linked = SourceError::new(SourceErrorKind::FoldersOfLinkedAddon, "raw");
        linked.clash = Some(FolderClash {
            linked: vec![
                ("Kui_Media".to_string(), "Kui Nameplates".to_string()),
                ("Kui_Nameplates".to_string(), "Kui Nameplates".to_string()),
            ],
            ..FolderClash::default()
        });
        assert_eq!(
            super::failure(&linked, request(SourceKind::GitHub, "x")),
            said(
                "Kui_Media and 1 more belong to Kui Nameplates, which LemonUp already keeps \
                 updated, so LemonUp won't replace them."
            )
        );
        // Folders that are Not linked are named: a command can't ask about them.
        let mut in_the_way = SourceError::new(SourceErrorKind::FoldersInTheWay, "raw words");
        assert_eq!(
            super::failure(&in_the_way, request(SourceKind::GitHub, "x")),
            said("raw words")
        );
        in_the_way.clash = Some(FolderClash {
            replaceable: vec!["Pawn".to_string(), "PawnExtras".to_string()],
            ..FolderClash::default()
        });
        assert_eq!(
            super::failure(&in_the_way, request(SourceKind::GitHub, "x")),
            said("Its folders are already in your AddOns folder: Pawn, PawnExtras")
        );
    }

    /// The failure of installing an addon that is already there as `folder`, linked to `linked`.
    fn already_installed(
        kind: crate::source_error::SourceErrorKind,
        name: &str,
        folder: &str,
        linked: SourceKind,
    ) -> crate::source_error::SourceError {
        use crate::source_error::{FolderClash, InstalledAddon, SourceError};
        let mut error = SourceError::new(kind, "raw");
        error.clash = Some(FolderClash {
            linked: vec![(folder.to_string(), name.to_string())],
            replaceable: vec![folder.to_string()],
            installed: Some(Box::new(InstalledAddon {
                name: name.to_string(),
                folder: folder.to_string(),
                source: linked,
            })),
            ..FolderClash::default()
        });
        error
    }

    #[test]
    fn an_addon_that_is_already_installed_says_so_and_names_the_update_that_installs_it_again() {
        use crate::source_error::SourceErrorKind;
        let github = request(SourceKind::GitHub, "x");
        // As folders of a linked addon, or from Wago.io as folders in the way.
        for kind in [
            SourceErrorKind::FoldersOfLinkedAddon,
            SourceErrorKind::FoldersInTheWay,
        ] {
            assert_eq!(
                super::failure(
                    &already_installed(
                        kind,
                        "Kui Nameplates",
                        "Kui_Nameplates",
                        SourceKind::GitHub
                    ),
                    github,
                ),
                (
                    "Kui Nameplates is already installed.".to_string(),
                    Some(
                        "Run `lemonup update Kui_Nameplates --force` to install it again."
                            .to_string()
                    )
                ),
                "{kind:?}"
            );
        }
        // Linked to another source: the reason says which, since the update installs from there.
        let elsewhere = already_installed(
            SourceErrorKind::FoldersOfLinkedAddon,
            "ElvUI",
            "ElvUI",
            SourceKind::Tukui,
        );
        assert_eq!(
            super::failure(&elsewhere, github).0,
            "ElvUI is already installed from TukUI."
        );
        // The hint names the profile and the `--addon-dir` of the run.
        let run = super::InstallRequest {
            profile: "forever",
            addon_dir_given: true,
            ..github
        };
        assert_eq!(
            super::failure(&elsewhere, run).1.as_deref(),
            Some(
                "Run `lemonup --profile forever update ElvUI --force` with the same \
                 `--addon-dir` to install it again."
            )
        );
        // Another kind of failure never says "already installed", whatever the clash holds.
        let other = already_installed(
            SourceErrorKind::NoConnection,
            "ElvUI",
            "ElvUI",
            SourceKind::Tukui,
        );
        assert_eq!(super::failure(&other, github).1, None);
    }

    #[test]
    fn the_reinstall_command_holds_only_a_folder_that_is_safe_to_paste_and_that_update_takes() {
        use crate::cli::{Cli, Commands};
        use clap::Parser;
        // Plain folders go in as they are, and one with a space in quotes.
        for (folder, selector) in [
            ("Kui_Nameplates", "Kui_Nameplates"),
            ("DBM-Core", "DBM-Core"),
            ("Raider.IO", "Raider.IO"),
            ("My Addon", "\"My Addon\""),
        ] {
            assert_eq!(
                super::update_selector(folder).as_deref(),
                Some(selector),
                "{folder}"
            );
            // `lemonup update` takes it and selects exactly that folder.
            let cli = Cli::try_parse_from(["lemonup", "update", folder, "--force"]).expect(folder);
            match cli.command {
                Some(Commands::Update { addons, force, .. }) => {
                    assert_eq!(addons, [folder.to_string()]);
                    assert!(force);
                }
                other => panic!("expected update, got {other:?}"),
            }
        }
        // A flag, a quote of any kind, something a shell expands, a path, other alphabets, a
        // name too long for a selector: none of these is put into a command.
        let long = "a".repeat(121);
        for folder in [
            "",
            "--replace",
            "-x",
            " Padded",
            "Padded ",
            "up..dir",
            "Evil`rm`",
            "Pay$HOME",
            "Quo\"te",
            "It's",
            "Bang!",
            "Back\\slash",
            "Per%cent",
            "Semi;colon",
            "Amp&ersand",
            "Pipe|d",
            "<DBM Core> Main Core",
            "Paren(s)",
            "Curly\u{201d}; calc; \u{201c}",
            "\u{80cc}\u{5305}",
            "Tab\tbed",
            "New\nline",
            long.as_str(),
        ] {
            assert_eq!(super::update_selector(folder), None, "{folder:?}");
        }
        let hint = super::reinstall_hint("--replace", request(SourceKind::GitHub, "x"));
        assert_eq!(
            hint,
            "Run `lemonup update` with its folder name and `--force` to install it again."
        );
    }

    #[test]
    fn each_source_names_its_own_command() {
        for (source, command) in [
            (SourceKind::Wago, "install-wago"),
            (SourceKind::Tukui, "install-tukui"),
            (SourceKind::WowInterface, "install-wowinterface"),
            (SourceKind::GitHub, "install-github"),
        ] {
            let mut addon = weakauras();
            addon.source = source;
            assert_eq!(report(false, addon).command(), command);
        }
    }

    #[test]
    fn the_json_document_has_the_addon_and_only_the_fields_its_source_has() {
        let github = report(false, weakauras());
        assert_eq!(
            json_document(&github, Some(github.addon_dir()), &[]),
            concat!(
                r#"{"schema":1,"command":"install-github","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","dry_run":false,"#,
                r#""addon":{"name":"WeakAuras","source":"github","#,
                r#""id":"https://github.com/WeakAuras/WeakAuras2","version":"5.19.0","#,
                r#""release":"5.19.0","parent_folder":"WeakAuras","#,
                r#""folders":["WeakAuras","WeakAurasArchive","WeakAurasOptions"]},"#,
                r#""warnings":[]}"#,
                "\n"
            )
        );
        let wago = report(true, wago("beta"));
        let document = json_document(&wago, Some(wago.addon_dir()), &["careful"]);
        assert!(
            document.contains(concat!(
                r#""command":"install-wago","profile":"default","#,
                r#""addon_dir":"/games/wow/_retail_/Interface/AddOns","dry_run":true,"#,
                r#""addon":{"name":"Deadly Boss Mods","source":"wago","id":"qv63A7Gb","#,
                r#""version":"12.0.10","channel":"beta","parent_folder":"DBM-Core","#,
                r#""folders":["DBM-Core"]},"warnings":["careful"]}"#
            )),
            "{document}"
        );
    }
}
