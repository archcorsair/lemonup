//! Colors, symbols and motion for the whole TUI. Every frame is drawn with
//! one `UiTheme`, and `UiTheme::finish_frame` makes the last pass over the
//! drawn buffer: no colors at all under `NO_COLOR`, only the terminal's own
//! 16 colors in the Terminal theme, and plain ASCII with simple symbols.

use lemonup_core::{AppConfig, ThemeMode};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

/// What the terminal and environment allow, read once at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayEnv {
    /// `NO_COLOR` is set to a non-empty value: no colors at all.
    pub no_color: bool,
    /// The terminal reports 24-bit color (`COLORTERM`).
    pub truecolor: bool,
    /// The locale can show symbols beyond ASCII.
    pub utf8: bool,
    /// The terminal is one known to turn OSC 8 links into something to click.
    pub hyperlinks: bool,
}

impl DisplayEnv {
    /// Everything available. Tests use this so they never depend on the
    /// terminal they run in.
    pub const FULL: Self = Self {
        no_color: false,
        truecolor: true,
        utf8: true,
        hyperlinks: true,
    };

    pub fn detect() -> Self {
        Self::from_vars(
            |name| std::env::var_os(name).map(|value| value.to_string_lossy().into_owned()),
            cfg!(windows),
        )
    }

    /// `get` reads one environment variable. An empty value counts as not
    /// set.
    pub(crate) fn from_vars(get: impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        let set = |name: &str| get(name).filter(|value| !value.is_empty());
        let no_color = set("NO_COLOR").is_some();
        // Windows Terminal draws 24-bit color but does not always say so in
        // COLORTERM; WT_SESSION is how it identifies itself.
        let truecolor = set("COLORTERM").is_some_and(|value| {
            matches!(value.to_ascii_lowercase().as_str(), "truecolor" | "24bit")
        }) || set("WT_SESSION").is_some();
        // On Windows the console is written in UTF-16 whatever its code
        // page, so the locale says nothing about which symbols show.
        let utf8 = windows
            || ["LC_ALL", "LC_CTYPE", "LANG"]
                .iter()
                .find_map(|name| set(name))
                .is_some_and(|locale| {
                    let locale = locale.to_ascii_lowercase();
                    locale.contains("utf-8") || locale.contains("utf8")
                });
        Self {
            no_color,
            truecolor,
            utf8,
            hyperlinks: hyperlinks(set),
        }
    }
}

/// Whether the terminal turns OSC 8 links into something to click. No terminal can be asked,
/// so this goes by how the ones that do identify themselves. `FORCE_HYPERLINK` overrides it,
/// as in other tools: `0` turns links off, anything else turns them on.
fn hyperlinks(set: impl Fn(&str) -> Option<String>) -> bool {
    if let Some(forced) = set("FORCE_HYPERLINK") {
        return forced != "0";
    }
    let at_least = |name: &str, version: u32| {
        set(name).is_some_and(|value| value.parse::<u32>().is_ok_and(|found| found >= version))
    };
    let program = set("TERM_PROGRAM").unwrap_or_default();
    let term = set("TERM").unwrap_or_default();
    // Over SSH a `file://` link would open a file of the machine the terminal runs on. And
    // tmux and screen sit between LemonUp and the terminal named in the environment: some
    // versions drop a link, some print it as text.
    let remote = set("SSH_CONNECTION").is_some() || set("SSH_TTY").is_some();
    let multiplexed = set("TMUX").is_some()
        || set("STY").is_some()
        || term.starts_with("screen")
        || term.starts_with("tmux");
    if remote || multiplexed || term == "dumb" {
        return false;
    }
    matches!(
        program.as_str(),
        "ghostty" | "iTerm.app" | "WezTerm" | "vscode" | "Hyper" | "Tabby"
    ) || ["xterm-ghostty", "xterm-kitty", "alacritty", "foot", "wezterm"]
        .iter()
        .any(|known| term.starts_with(known))
        || set("WT_SESSION").is_some()
        // GNOME Terminal and the others built on VTE, from 0.50.
        || at_least("VTE_VERSION", 5000)
        // Konsole from 21.04.
        || at_least("KONSOLE_VERSION", 210_400)
}

/// The theme a saved choice shows as right now: Terminal while `NO_COLOR`
/// is set, or while LemonUp follows a terminal without 24-bit color.
pub(crate) fn effective_theme(
    saved: ThemeMode,
    display: DisplayEnv,
    follow_terminal: bool,
) -> ThemeMode {
    if display.no_color || follow_terminal {
        ThemeMode::Terminal
    } else {
        saved
    }
}

/// The next Theme value: Dark, Light, Terminal and round again.
pub(crate) fn cycle_theme(current: ThemeMode, delta: i16) -> ThemeMode {
    const ORDER: [ThemeMode; 3] = [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Terminal];
    let index = ORDER.iter().position(|mode| *mode == current).unwrap_or(0);
    let step = if delta < 0 { ORDER.len() - 1 } else { 1 };
    ORDER[(index + step) % ORDER.len()]
}

pub(crate) fn theme_name(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Dark => "Dark",
        ThemeMode::Light => "Light",
        ThemeMode::Terminal => "Terminal",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UiTheme {
    /// The color roles. All drawing code uses only these.
    pub(crate) roles: ColorRoles,
    /// The theme on screen, which can differ from the saved one.
    pub(crate) mode: ThemeMode,
    /// `NO_COLOR` is set: the last pass removes every color.
    pub(crate) no_color: bool,
    pub(crate) symbols: Symbols,
    /// A still symbol in place of the spinner, and no shimmer.
    pub(crate) reduce_motion: bool,
}

/// Color roles from the approved design. Each role has one meaning, so a
/// color never has to be guessed at from context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ColorRoles {
    /// Where you are and the main action: cursor bar, primary key, focus,
    /// logo.
    pub(crate) lemon: Color,
    /// An update is ready, and the new version.
    pub(crate) amber: Color,
    /// Done, or nothing to do.
    pub(crate) green: Color,
    /// A problem, or the delete and restore questions. Nothing else.
    pub(crate) red: Color,
    /// Keys you can press, and links.
    pub(crate) blue: Color,
    /// Everything that does not need you.
    pub(crate) muted: Style,
    /// Rules and separators.
    pub(crate) dim: Color,
    pub(crate) text: Color,
    pub(crate) background: Color,
    pub(crate) cursor_row: Color,
    pub(crate) selected_row: Color,
    /// Background of a sheet (an overlay such as Help or Settings).
    pub(crate) sheet: Color,
    /// The outline of a sheet.
    pub(crate) border: Color,
    /// The cursor row is reverse video and bold, not a background color:
    /// the Terminal theme has no background of its own.
    pub(crate) reverse_cursor: bool,
}

impl ColorRoles {
    pub(crate) fn dark() -> Self {
        Self {
            lemon: Color::Rgb(240, 200, 90),
            amber: Color::Rgb(255, 158, 100),
            green: Color::Rgb(158, 206, 106),
            red: Color::Rgb(247, 118, 142),
            blue: Color::Rgb(122, 162, 247),
            muted: Style::new().fg(Color::Rgb(140, 150, 192)),
            dim: Color::Rgb(0x45, 0x4b, 0x6c),
            text: Color::Rgb(0xc5, 0xcd, 0xf2),
            background: Color::Rgb(0x24, 0x28, 0x3b),
            cursor_row: Color::Rgb(0x32, 0x38, 0x56),
            selected_row: Color::Rgb(0x2c, 0x31, 0x48),
            sheet: Color::Rgb(0x29, 0x2e, 0x45),
            border: Color::Rgb(0x5d, 0x66, 0x94),
            reverse_cursor: false,
        }
    }

    pub(crate) fn light() -> Self {
        Self {
            lemon: Color::Rgb(122, 90, 0),
            amber: Color::Rgb(150, 78, 0),
            green: Color::Rgb(70, 100, 40),
            // The design's 198,67,67 is 3.9:1 on this background; this is
            // the nearest red that reaches 4.5:1.
            red: Color::Rgb(180, 48, 48),
            blue: Color::Rgb(28, 92, 190),
            muted: Style::new().fg(Color::Rgb(86, 94, 134)),
            dim: Color::Rgb(0xa9, 0xaf, 0xc8),
            text: Color::Rgb(0x2b, 0x31, 0x50),
            background: Color::Rgb(0xe4, 0xe5, 0xeb),
            cursor_row: Color::Rgb(0xcf, 0xd4, 0xe3),
            selected_row: Color::Rgb(0xd8, 0xdb, 0xe6),
            sheet: Color::Rgb(0xee, 0xef, 0xf4),
            border: Color::Rgb(0x7f, 0x88, 0xad),
            reverse_cursor: false,
        }
    }

    /// The terminal's own 16 colors on its own background. The palette has
    /// one yellow that reads on both light and dark backgrounds, so lemon
    /// and amber share it; an update is bold as well.
    pub(crate) fn terminal() -> Self {
        Self {
            lemon: Color::Yellow,
            amber: Color::Yellow,
            green: Color::Green,
            red: Color::Red,
            blue: Color::Cyan,
            muted: Style::new().fg(Color::Reset).add_modifier(Modifier::DIM),
            dim: Color::DarkGray,
            text: Color::Reset,
            background: Color::Reset,
            cursor_row: Color::Reset,
            selected_row: Color::Reset,
            sheet: Color::Reset,
            border: Color::DarkGray,
            reverse_cursor: true,
        }
    }

    /// The row the cursor is on.
    pub(crate) fn cursor_row_style(self) -> Style {
        if self.reverse_cursor {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default().bg(self.cursor_row)
        }
    }

    /// A row marked with space.
    pub(crate) fn selected_row_style(self) -> Style {
        if self.reverse_cursor {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(self.selected_row)
        }
    }

    /// The colors the logo runs through, left to right. One color when the
    /// theme cannot blend.
    pub(crate) fn logo_colors(self) -> Vec<Color> {
        if matches!(self.lemon, Color::Rgb(..)) {
            vec![self.amber, self.lemon, self.lemon]
        } else {
            vec![self.lemon]
        }
    }
}

/// The symbols that are wider than one cell in plain ASCII. Every other
/// symbol is one cell either way and is swapped by `finish_frame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Symbols {
    /// Plain ASCII and the word LEMONUP in place of the block logo.
    pub(crate) simple: bool,
    /// Up to date, done.
    pub(crate) done: &'static str,
    /// A command that failed.
    pub(crate) failed: &'static str,
    /// From the installed version to the new one.
    pub(crate) arrow: &'static str,
    /// Work in progress, and the spinner under reduce motion.
    pub(crate) ellipsis: &'static str,
    pub(crate) spinner: &'static [&'static str],
}

impl Symbols {
    pub(crate) const FANCY: Self = Self {
        simple: false,
        done: "✓",
        failed: "✗",
        arrow: "→",
        ellipsis: "…",
        spinner: &["⠋", "⠙", "⠸", "⠴"],
    };

    pub(crate) const SIMPLE: Self = Self {
        simple: true,
        done: "ok",
        failed: "x",
        arrow: "->",
        ellipsis: "...",
        spinner: &["|", "/", "-", "\\"],
    };
}

/// The ASCII cell for a symbol drawn one cell wide, or `None` for anything
/// that is not one of LemonUp's symbols (such as letters in an addon name).
pub(crate) fn ascii_symbol(symbol: &str) -> Option<&'static str> {
    let mut chars = symbol.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || ch.is_ascii() {
        return None;
    }
    let ascii = match ch {
        '↑' | '▲' => "^",
        '↓' | '▼' | '▾' => "v",
        '←' | '‹' => "<",
        '→' | '›' | '▸' => ">",
        '·' | '─' | '━' | '⠸' => "-",
        '•' | '●' | '✓' => "*",
        '○' => "o",
        '✗' => "x",
        '▌' | '│' | '┃' | '⠋' => "|",
        '⠙' => "/",
        '⠴' => "\\",
        '…' => ".",
        '┼' | '├' | '┤' | '┬' | '┴' | '└' | '┘' | '┌' | '┐' | '╭' | '╮' | '╯' | '╰' => {
            "+"
        }
        '█' | '▀' | '▄' => "#",
        '░' => ".",
        // Anything else from the arrow, box, block, shape or symbol blocks.
        '\u{2190}'..='\u{2bff}' => "*",
        _ => return None,
    };
    Some(ascii)
}

impl UiTheme {
    fn build(mode: ThemeMode) -> Self {
        Self {
            roles: match mode {
                ThemeMode::Dark => ColorRoles::dark(),
                ThemeMode::Light => ColorRoles::light(),
                ThemeMode::Terminal => ColorRoles::terminal(),
            },
            mode,
            no_color: false,
            symbols: Symbols::FANCY,
            reduce_motion: false,
        }
    }

    /// A theme on its own, with fancy symbols and motion.
    #[cfg(test)]
    pub(crate) fn from_mode(mode: ThemeMode) -> Self {
        Self::build(mode)
    }

    /// The theme for `config` in this terminal. `follow_terminal` puts the
    /// Terminal theme on screen without saving it.
    pub(crate) fn for_config(
        config: &AppConfig,
        display: DisplayEnv,
        follow_terminal: bool,
    ) -> Self {
        let mut theme = Self::build(effective_theme(config.theme, display, follow_terminal));
        theme.no_color = display.no_color;
        if config.simple_symbols || !display.utf8 {
            theme.symbols = Symbols::SIMPLE;
        }
        theme.reduce_motion = config.reduce_motion;
        theme
    }

    /// The last pass over a drawn frame. Under `NO_COLOR` every color goes;
    /// in the Terminal theme any 24-bit color left goes; with simple symbols
    /// every symbol becomes ASCII. Reverse video and bold stay.
    pub(crate) fn finish_frame(&self, buffer: &mut Buffer) {
        let terminal = self.mode == ThemeMode::Terminal;
        if !self.no_color && !terminal && !self.symbols.simple {
            return;
        }
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &mut buffer[(x, y)];
                if self.no_color {
                    cell.fg = Color::Reset;
                    cell.bg = Color::Reset;
                } else if terminal {
                    if matches!(cell.fg, Color::Rgb(..) | Color::Indexed(_)) {
                        cell.fg = Color::Reset;
                    }
                    if matches!(cell.bg, Color::Rgb(..) | Color::Indexed(_)) {
                        cell.bg = Color::Reset;
                    }
                }
                if !self.symbols.simple {
                    continue;
                }
                let symbol = cell.symbol();
                if symbol == "…" && x + 3 < area.right() {
                    // "..." when the two cells after it are free.
                    let blank = (1..=3).all(|step| buffer[(x + step, y)].symbol() == " ");
                    if blank {
                        let fg = buffer[(x, y)].fg;
                        for step in 0..3 {
                            let cell = &mut buffer[(x + step, y)];
                            cell.set_symbol(".");
                            if step > 0 {
                                cell.fg = fg;
                            }
                        }
                        continue;
                    }
                }
                if let Some(ascii) = ascii_symbol(buffer[(x, y)].symbol()) {
                    buffer[(x, y)].set_symbol(ascii);
                }
            }
        }
    }
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::build(ThemeMode::Dark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("{color:?} is not a 24-bit color");
        };
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// The WCAG contrast ratio between two colors.
    fn contrast(a: Color, b: Color) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn every_text_role_reaches_4_5_to_1_on_its_background() {
        for (name, roles) in [("dark", ColorRoles::dark()), ("light", ColorRoles::light())] {
            for (role, color) in [
                ("lemon", roles.lemon),
                ("amber", roles.amber),
                ("green", roles.green),
                ("red", roles.red),
                ("blue", roles.blue),
                ("muted", roles.muted.fg.expect("muted has a color")),
                ("text", roles.text),
            ] {
                for (surface, background) in
                    [("background", roles.background), ("sheet", roles.sheet)]
                {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 4.5,
                        "{name} {role} on {surface} is {ratio:.2}:1, under 4.5:1"
                    );
                }
            }
        }
    }

    #[test]
    fn terminal_roles_use_only_the_16_colors_and_no_background() {
        let roles = ColorRoles::terminal();
        for color in [
            roles.lemon,
            roles.amber,
            roles.green,
            roles.red,
            roles.blue,
            roles.muted.fg.expect("muted has a color"),
            roles.dim,
            roles.text,
            roles.border,
        ] {
            assert!(!matches!(color, Color::Rgb(..) | Color::Indexed(_)));
        }
        assert_eq!(roles.background, Color::Reset);
        assert_eq!(roles.sheet, Color::Reset);
        assert_eq!(roles.muted.bg, None);
        assert!(
            roles
                .cursor_row_style()
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn terminal_muted_is_the_normal_text_color_dimmed() {
        let muted = ColorRoles::terminal().muted;
        assert_eq!(muted.fg, Some(Color::Reset));
        assert!(muted.add_modifier.contains(Modifier::DIM));
        for roles in [ColorRoles::dark(), ColorRoles::light()] {
            assert!(matches!(roles.muted.fg, Some(Color::Rgb(..))));
            assert!(roles.muted.add_modifier.is_empty());
        }
    }

    #[test]
    fn no_color_keeps_dim_text_dim() {
        let mut theme = UiTheme::from_mode(ThemeMode::Terminal);
        theme.no_color = true;
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "mute", theme.roles.muted);
        theme.finish_frame(&mut buffer);
        for cell in buffer.content() {
            assert_eq!(cell.fg, Color::Reset);
            assert!(cell.modifier.contains(Modifier::DIM));
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn no_color_with_any_value_turns_colors_off() {
        let env = DisplayEnv::from_vars(
            vars(&[
                ("NO_COLOR", "1"),
                ("COLORTERM", "truecolor"),
                ("LANG", "en_US.UTF-8"),
            ]),
            false,
        );
        assert!(env.no_color);
        assert_eq!(
            effective_theme(ThemeMode::Dark, env, false),
            ThemeMode::Terminal
        );
        let empty = DisplayEnv::from_vars(vars(&[("NO_COLOR", ""), ("COLORTERM", "24bit")]), false);
        assert!(!empty.no_color);
        assert!(empty.truecolor);
    }

    #[test]
    fn colorterm_decides_truecolor() {
        for (value, truecolor) in [
            ("truecolor", true),
            ("24bit", true),
            ("TrueColor", true),
            ("256color", false),
            ("", false),
        ] {
            let env = DisplayEnv::from_vars(vars(&[("COLORTERM", value)]), false);
            assert_eq!(env.truecolor, truecolor, "COLORTERM={value}");
        }
        assert!(!DisplayEnv::from_vars(vars(&[]), false).truecolor);
        assert!(DisplayEnv::from_vars(vars(&[("WT_SESSION", "abc")]), true).truecolor);
    }

    #[test]
    fn links_are_for_terminals_known_to_have_them() {
        for (pairs, hyperlinks) in [
            (vec![("TERM_PROGRAM", "ghostty")], true),
            (vec![("TERM_PROGRAM", "iTerm.app")], true),
            (vec![("TERM_PROGRAM", "WezTerm")], true),
            (vec![("TERM_PROGRAM", "vscode")], true),
            (vec![("TERM", "xterm-ghostty")], true),
            (vec![("TERM", "xterm-kitty")], true),
            (vec![("TERM", "alacritty")], true),
            (vec![("TERM", "foot-extra")], true),
            (vec![("WT_SESSION", "abc")], true),
            (vec![("VTE_VERSION", "7600")], true),
            (vec![("KONSOLE_VERSION", "240800")], true),
            // Too old, unknown, or nothing said.
            (vec![("VTE_VERSION", "4804")], false),
            (vec![("KONSOLE_VERSION", "200400")], false),
            (vec![("VTE_VERSION", "new")], false),
            (vec![("TERM_PROGRAM", "Apple_Terminal")], false),
            (vec![("TERM", "xterm-256color")], false),
            (vec![("TERM", "dumb")], false),
            (vec![], false),
            // The override wins both ways; empty counts as not set.
            (vec![("FORCE_HYPERLINK", "1"), ("TERM", "dumb")], true),
            (
                vec![("FORCE_HYPERLINK", "0"), ("TERM_PROGRAM", "ghostty")],
                false,
            ),
            (vec![("FORCE_HYPERLINK", ""), ("TERM", "dumb")], false),
            (
                vec![("FORCE_HYPERLINK", ""), ("TERM_PROGRAM", "ghostty")],
                true,
            ),
            // A link would point at the wrong machine over SSH, and tmux or screen may not
            // pass it on. The override still wins.
            (
                vec![("TERM_PROGRAM", "ghostty"), ("SSH_CONNECTION", "1 2 3 4")],
                false,
            ),
            (
                vec![("TERM_PROGRAM", "ghostty"), ("SSH_TTY", "/dev/pts/1")],
                false,
            ),
            (
                vec![
                    ("VTE_VERSION", "7600"),
                    ("TMUX", "/tmp/tmux-1000/default,1,0"),
                ],
                false,
            ),
            (
                vec![("VTE_VERSION", "7600"), ("TERM", "screen-256color")],
                false,
            ),
            (
                vec![("WT_SESSION", "abc"), ("TERM", "tmux-256color")],
                false,
            ),
            (
                vec![("TERM_PROGRAM", "vscode"), ("STY", "1234.pts-0.host")],
                false,
            ),
            (vec![("TERM_PROGRAM", "vscode"), ("TERM", "dumb")], false),
            (
                vec![("FORCE_HYPERLINK", "1"), ("TMUX", "x"), ("SSH_TTY", "y")],
                true,
            ),
        ] {
            let env = DisplayEnv::from_vars(vars(&pairs), false);
            assert_eq!(env.hyperlinks, hyperlinks, "{pairs:?}");
        }
    }

    #[test]
    fn locale_decides_utf8() {
        for (pairs, utf8) in [
            (vec![("LANG", "en_US.UTF-8")], true),
            (vec![("LANG", "de_DE.utf8")], true),
            (vec![("LANG", "C")], false),
            (vec![("LANG", "POSIX")], false),
            (vec![], false),
            // LC_ALL wins over LANG, then LC_CTYPE.
            (vec![("LC_ALL", "C"), ("LANG", "en_US.UTF-8")], false),
            (vec![("LC_CTYPE", "en_US.UTF-8"), ("LANG", "C")], true),
            (vec![("LC_ALL", ""), ("LANG", "en_US.UTF-8")], true),
        ] {
            let env = DisplayEnv::from_vars(vars(&pairs), false);
            assert_eq!(env.utf8, utf8, "{pairs:?}");
        }
        assert!(DisplayEnv::from_vars(vars(&[("LANG", "C")]), true).utf8);
    }

    #[test]
    fn colors_cycle_through_three_themes() {
        assert_eq!(cycle_theme(ThemeMode::Dark, 1), ThemeMode::Light);
        assert_eq!(cycle_theme(ThemeMode::Light, 1), ThemeMode::Terminal);
        assert_eq!(cycle_theme(ThemeMode::Terminal, 1), ThemeMode::Dark);
        assert_eq!(cycle_theme(ThemeMode::Dark, -1), ThemeMode::Terminal);
    }

    #[test]
    fn simple_symbols_map_every_symbol_to_one_ascii_cell() {
        for symbol in [
            "↑", "✓", "·", "▸", "▾", "●", "▌", "→", "…", "‹", "›", "─", "│", "╭", "⠋",
        ] {
            let ascii = ascii_symbol(symbol).expect(symbol);
            assert_eq!(ascii.len(), 1);
            assert!(ascii.is_ascii());
        }
        assert_eq!(ascii_symbol("a"), None);
        assert_eq!(ascii_symbol("é"), None);
    }
}
