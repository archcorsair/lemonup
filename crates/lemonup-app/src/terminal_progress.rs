//! The terminal's own progress bar: the ConEmu "progress state" sequence (OSC 9;4) that some
//! terminals draw in a tab, the title bar or the taskbar.
//!
//! Other terminals read OSC 9 as "show a notification", so nothing here is ever written,
//! not even the sequence that removes the bar, unless the terminal is known to draw one.

/// What the terminal's progress bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Bar {
    /// No bar.
    #[default]
    Off,
    /// This much is done. `trouble` once an addon failed: the bar turns yellow and stays so.
    Percent { percent: u8, trouble: bool },
    /// Work with no count to show.
    Pulse,
    /// The whole run failed: a full red bar.
    Failed,
}

impl Bar {
    /// `done` of `total`, in percent. Nothing to do counts as all done.
    pub(crate) fn of(done: usize, total: usize, trouble: bool) -> Self {
        let percent = if total == 0 {
            100
        } else {
            // Widened first, so no count is too large to multiply.
            (done.min(total) as u128 * 100 / total as u128) as u8
        };
        Self::Percent { percent, trouble }
    }

    /// The sequence that makes the terminal show this.
    pub(crate) fn sequence(self) -> String {
        // States: 0 remove, 1 running, 2 error, 3 no count, 4 warning.
        let (state, percent) = match self {
            Self::Off => (0, 0),
            Self::Percent {
                percent,
                trouble: false,
            } => (1, percent.min(100)),
            Self::Percent {
                percent,
                trouble: true,
            } => (4, percent.min(100)),
            Self::Pulse => (3, 0),
            Self::Failed => (2, 100),
        };
        format!("\x1b]9;4;{state};{percent}\x1b\\")
    }
}

/// Whether the terminal draws a progress bar for OSC 9;4. No terminal can be asked, so this
/// goes by how the ones that do identify themselves, each from the version its own notes
/// name. `get` reads one environment variable that is set to something.
///
/// A terminal started from inside another one inherits that one's variables. So a terminal
/// is taken at its word only where nothing says another terminal is in between, and when in
/// doubt the answer is no: a missing bar costs nothing, a notification for every addon does.
pub(crate) fn drawn_by_terminal(get: impl Fn(&str) -> Option<String>) -> bool {
    let term = get("TERM").unwrap_or_default();
    // tmux and screen sit between LemonUp and the terminal named in the environment, and
    // pass on what they know.
    let multiplexed = get("TMUX").is_some()
        || get("STY").is_some()
        || term.starts_with("screen")
        || term.starts_with("tmux");
    if multiplexed || term == "dumb" {
        return false;
    }
    // Terminals that draw no bar and say who they are without `TERM_PROGRAM`: kitty,
    // Alacritty, foot, rxvt, st, xterm, Konsole, GNOME Terminal and JetBrains. Each could
    // be running inside one that does.
    let draws_none = ["xterm-kitty", "alacritty", "foot", "rxvt", "st-"]
        .iter()
        .any(|known| term.starts_with(known))
        || term == "st"
        || [
            "KITTY_WINDOW_ID",
            "ALACRITTY_WINDOW_ID",
            "XTERM_VERSION",
            "KONSOLE_VERSION",
            "GNOME_TERMINAL_SCREEN",
            "TERMINAL_EMULATOR",
        ]
        .iter()
        .any(|name| get(name).is_some());
    if draws_none {
        return false;
    }
    // GNOME Terminal and the others built on VTE draw none. Ptyxis is built on it too, and
    // is the one that does, from 48.
    let ptyxis = get("PTYXIS_VERSION").is_some_and(|version| numbers(&version) >= vec![48]);
    let vte = get("VTE_VERSION").is_some();
    let version = get("TERM_PROGRAM_VERSION").unwrap_or_default();
    match get("TERM_PROGRAM").as_deref() {
        // iTerm2 lists what it can do, with `P` for this, from 3.6.6.
        Some("iTerm.app") => {
            !vte && get("TERM_FEATURES").is_some_and(|features| has_feature(&features, "P"))
        }
        // Ghostty from 1.2.0, and builds from its main branch since then.
        Some("ghostty") => !vte && ghostty_draws_it(&version),
        // WezTerm from the builds of 2025-02-10: the sequence came in the evening before.
        // Its last release, 20240203, and the builds up to then show OSC 9 as a
        // notification.
        Some("WezTerm") => {
            !vte && numbers(&version)
                .first()
                .is_some_and(|date| *date >= 20_250_210)
        }
        // Another terminal that names itself, such as VS Code or Apple's Terminal.
        Some(_) => false,
        // Windows Terminal and ConEmu name themselves in other ways.
        None => {
            ptyxis
                || (!vte
                    && (get("WT_SESSION").is_some() || get("ConEmuANSI").as_deref() == Some("ON")))
        }
    }
}

/// Whether a Ghostty of this version draws the bar: 1.2.0 or newer. Some packages build
/// Ghostty from its main branch and number it `0.0.0-` with the day of the build, such as
/// `0.0.0-20261123.r18897.gdf28358`. Those count from the day 1.2.0 came out, 2025-09-15.
fn ghostty_draws_it(version: &str) -> bool {
    if numbers(version) >= vec![1, 2] {
        return true;
    }
    version
        .strip_prefix("0.0.0-")
        .and_then(|rest| numbers(rest).first().copied())
        .is_some_and(|day| day >= 20_250_915)
}

/// The numbers a version starts with: `1.2.0-main+abc` is 1, 2, 0 and
/// `20240203-110809-5046fc22` is 20240203.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map_while(|part| {
            let digits = part
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            digits
                .parse()
                .ok()
                .map(|number| (number, digits.len() == part.len()))
        })
        .scan(true, |more, (number, whole)| {
            // A part with something after its digits is the last one that counts.
            let take = *more;
            *more = whole;
            take.then_some(number)
        })
        .collect()
}

/// Whether iTerm2's `TERM_FEATURES` names `feature`. Each feature is an uppercase letter
/// followed by lowercase letters or digits, one after another.
fn has_feature(features: &str, feature: &str) -> bool {
    let mut current = String::new();
    for character in features.chars() {
        if !character.is_ascii_alphanumeric() {
            break;
        }
        if character.is_ascii_uppercase() {
            if current == feature {
                return true;
            }
            current.clear();
        }
        current.push(character);
    }
    current == feature
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(pairs: &[(&str, &str)]) -> bool {
        drawn_by_terminal(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        })
    }

    #[test]
    fn each_state_has_its_sequence() {
        assert_eq!(Bar::Off.sequence(), "\x1b]9;4;0;0\x1b\\");
        assert_eq!(Bar::of(1, 4, false).sequence(), "\x1b]9;4;1;25\x1b\\");
        // Yellow once an addon failed.
        assert_eq!(Bar::of(3, 4, true).sequence(), "\x1b]9;4;4;75\x1b\\");
        assert_eq!(Bar::Pulse.sequence(), "\x1b]9;4;3;0\x1b\\");
        assert_eq!(Bar::Failed.sequence(), "\x1b]9;4;2;100\x1b\\");
    }

    #[test]
    fn percent_stays_between_0_and_100() {
        assert_eq!(
            Bar::of(0, 17, false),
            Bar::Percent {
                percent: 0,
                trouble: false
            }
        );
        assert_eq!(
            Bar::of(17, 17, false),
            Bar::Percent {
                percent: 100,
                trouble: false
            }
        );
        // More done than there was to do, and nothing to do at all.
        assert_eq!(
            Bar::of(20, 17, false),
            Bar::Percent {
                percent: 100,
                trouble: false
            }
        );
        assert_eq!(
            Bar::of(0, 0, false),
            Bar::Percent {
                percent: 100,
                trouble: false
            }
        );
        assert_eq!(
            Bar::of(1, 3, false),
            Bar::Percent {
                percent: 33,
                trouble: false
            }
        );
        // A count too large to multiply by 100 as it is.
        assert_eq!(
            Bar::of(usize::MAX / 2, usize::MAX, false),
            Bar::Percent {
                percent: 49,
                trouble: false
            }
        );
    }

    #[test]
    fn only_terminals_known_to_draw_the_bar_get_it() {
        for (pairs, expected) in [
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                true,
            ),
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.3.0-main+abc12"),
                ][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "ghostty"), ("TERM_PROGRAM_VERSION", "2.0")][..],
                true,
            ),
            // A minor version of two digits is not read as text, and short versions count.
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.10.0"),
                ][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "ghostty"), ("TERM_PROGRAM_VERSION", "1.2")][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "ghostty"), ("TERM_PROGRAM_VERSION", "2")][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "ghostty"), ("TERM_PROGRAM_VERSION", "1")][..],
                false,
            ),
            // iTerm2: `P` only as a feature of its own, wherever it stands.
            (
                &[("TERM_PROGRAM", "iTerm.app"), ("TERM_FEATURES", "MPB")][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "iTerm.app"), ("TERM_FEATURES", "PMB")][..],
                true,
            ),
            (&[("TERM_FEATURES", "MBT2Ps")][..], false),
            (&[("TERM_FEATURES", "P2")][..], false),
            (&[("TERM_FEATURES", "ScSy")][..], false),
            // A build from the main branch that a package numbers 0.0.0 with its day.
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    (
                        "TERM_PROGRAM_VERSION",
                        "0.0.0-20261123.r18897.gdf28358.aur-ghostty-nightly-bin",
                    ),
                ][..],
                true,
            ),
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "0.0.0-20250915"),
                ][..],
                true,
            ),
            // One from before 1.2.0, and one that names no day.
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "0.0.0-20250601.r100.gabc1234"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "0.0.0"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "0.0.0-dev"),
                ][..],
                false,
            ),
            // Before 1.2.0, and a version that can't be read.
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.1.3"),
                ][..],
                false,
            ),
            (&[("TERM_PROGRAM", "ghostty")][..], false),
            (
                &[("TERM_PROGRAM", "ghostty"), ("TERM_PROGRAM_VERSION", "tip")][..],
                false,
            ),
            // iTerm2 says so itself.
            (
                &[
                    ("TERM_PROGRAM", "iTerm.app"),
                    ("TERM_FEATURES", "T3LrMSc7UUw9Ts3BFGsSyHNoSxFP"),
                ][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "iTerm.app"), ("TERM_FEATURES", "MBT2ScP")][..],
                true,
            ),
            (
                &[("TERM_PROGRAM", "iTerm.app"), ("TERM_FEATURES", "MBT2Sc")][..],
                false,
            ),
            (&[("TERM_PROGRAM", "iTerm.app")][..], false),
            // WezTerm: only builds that know the sequence. Older ones, the last release
            // among them, show a notification.
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20250210-064424-488dd372"),
                ][..],
                true,
            ),
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20260101-000000-0000abcd"),
                ][..],
                true,
            ),
            // The day it came in can't be told apart from the hours before it.
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20250209-182623-44866cc1"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20241115-101010-0000abcd"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20240203-110809-5046fc22"),
                ][..],
                false,
            ),
            (&[("TERM_PROGRAM", "WezTerm")][..], false),
            (&[("WT_SESSION", "1b2c")][..], true),
            (&[("ConEmuANSI", "ON")][..], true),
            (&[("ConEmuANSI", "OFF")][..], false),
            (&[("PTYXIS_VERSION", "48.0")][..], true),
            (&[("PTYXIS_VERSION", "49.beta")][..], true),
            (&[("PTYXIS_VERSION", "47.9")][..], false),
            // A terminal started from inside another inherits its variables: the one in between
            // is believed, and what it inherited is not.
            (&[("TERM_FEATURES", "MBT2ScP")][..], false),
            (
                &[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", "20240203-110809-5046fc22"),
                    ("TERM_FEATURES", "MBT2ScP"),
                ][..],
                false,
            ),
            (
                &[("TERM_PROGRAM", "vscode"), ("WT_SESSION", "1b2c")][..],
                false,
            ),
            (
                &[("TERM_PROGRAM", "vscode"), ("PTYXIS_VERSION", "48.0")][..],
                false,
            ),
            (
                &[("TERM", "xterm-kitty"), ("WT_SESSION", "1b2c")][..],
                false,
            ),
            (
                &[
                    ("KITTY_WINDOW_ID", "1"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (&[("TERM", "alacritty"), ("ConEmuANSI", "ON")][..], false),
            (&[("TERM", "foot"), ("PTYXIS_VERSION", "48.0")][..], false),
            (
                &[
                    ("KONSOLE_VERSION", "250800"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERMINAL_EMULATOR", "JetBrains-JediTerm"),
                    ("WT_SESSION", "1b2c"),
                ][..],
                false,
            ),
            // xterm, rxvt and st started from Ghostty, and GNOME Terminal started from Ptyxis.
            (
                &[
                    ("XTERM_VERSION", "XTerm(390)"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM", "rxvt-unicode-256color"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM", "st-256color"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (
                &[
                    ("GNOME_TERMINAL_SCREEN", "/org/gnome/Terminal/screen/1"),
                    ("VTE_VERSION", "8000"),
                    ("PTYXIS_VERSION", "48.0"),
                ][..],
                false,
            ),
            // GNOME Terminal inside Ghostty or Windows Terminal. Ptyxis is VTE too, and draws it.
            (
                &[
                    ("VTE_VERSION", "8000"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                ][..],
                false,
            ),
            (
                &[("VTE_VERSION", "8000"), ("WT_SESSION", "1b2c")][..],
                false,
            ),
            (
                &[("VTE_VERSION", "8000"), ("PTYXIS_VERSION", "48.0")][..],
                true,
            ),
            // Terminals that show OSC 9 as a notification, or draw nothing.
            (&[("TERM", "xterm-kitty")][..], false),
            (&[("TERM_PROGRAM", "Apple_Terminal")][..], false),
            (&[("VTE_VERSION", "8000")][..], false),
            (&[("KONSOLE_VERSION", "250800")][..], false),
            (&[][..], false),
            // tmux and screen sit in between, and a dumb terminal knows no escape codes.
            (
                &[
                    ("WT_SESSION", "1b2c"),
                    ("TMUX", "/tmp/tmux-1000/default,1,0"),
                ][..],
                false,
            ),
            (
                &[
                    ("TERM_PROGRAM", "ghostty"),
                    ("TERM_PROGRAM_VERSION", "1.2.0"),
                    ("STY", "1.pts"),
                ][..],
                false,
            ),
            (
                &[("WT_SESSION", "1b2c"), ("TERM", "screen-256color")][..],
                false,
            ),
            (
                &[("WT_SESSION", "1b2c"), ("TERM", "tmux-256color")][..],
                false,
            ),
            (&[("WT_SESSION", "1b2c"), ("TERM", "dumb")][..], false),
        ] {
            assert_eq!(drawn(pairs), expected, "{pairs:?}");
        }
    }

    #[test]
    fn a_version_is_read_up_to_what_is_not_a_number() {
        assert_eq!(numbers("1.2.0"), [1, 2, 0]);
        assert_eq!(numbers("1.10.0"), [1, 10, 0]);
        assert!(numbers("1.10.0") >= vec![1, 2]);
        assert!(numbers("1.1.9") < vec![1, 2]);
        assert_eq!(numbers("1.3.0-main+abc"), [1, 3, 0]);
        assert_eq!(numbers("20240203-110809-5046fc22"), [20_240_203]);
        assert_eq!(numbers("48"), [48]);
        assert_eq!(numbers("49.beta"), [49]);
        assert_eq!(numbers("tip"), Vec::<u64>::new());
        assert_eq!(numbers(""), Vec::<u64>::new());
    }
}
