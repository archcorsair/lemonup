use std::io::{self, Stdout, Write};
use std::panic;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

#[cfg(not(windows))]
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::terminal_progress::Bar;

pub type Backend = CrosstermBackend<Stdout>;

static PANIC_HOOK: Once = Once::new();
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
/// What the terminal's own progress bar was last told to show (see [`bar_state`]).
static TERMINAL_BAR: AtomicU16 = AtomicU16::new(BAR_OFF);

pub struct Tui {
    terminal: Terminal<Backend>,
}

impl Tui {
    pub fn enter() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        enable_bracketed_paste(&mut stdout);
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<Backend> {
        &mut self.terminal
    }

    pub fn exit(&mut self) -> io::Result<()> {
        if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        clear_terminal_bar(self.terminal.backend_mut());
        self.terminal.show_cursor()?;
        disable_raw_mode()?;
        disable_bracketed_paste(self.terminal.backend_mut());
        execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        )?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.exit();
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            restore_terminal();
            previous_hook(panic_info);
        }));
    });
}

/// Tells the terminal's own progress bar what to show, when that differs from what it shows
/// now. The caller has made sure the terminal draws one. Whatever is showing is taken away
/// when the TUI leaves, also on an error or a panic.
pub(crate) fn show_terminal_bar(writer: &mut impl Write, bar: Bar) -> io::Result<()> {
    let state = bar_state(bar);
    if TERMINAL_BAR.load(Ordering::SeqCst) == state {
        return Ok(());
    }
    writer.write_all(bar.sequence().as_bytes())?;
    writer.flush()?;
    // Remembered only once it is written: a write that failed is tried again with the next
    // frame, and a bar that may still be showing is still taken away on the way out.
    TERMINAL_BAR.store(state, Ordering::SeqCst);
    Ok(())
}

/// Takes the terminal's progress bar away if one is showing.
fn clear_terminal_bar(writer: &mut impl Write) {
    if TERMINAL_BAR.swap(BAR_OFF, Ordering::SeqCst) != BAR_OFF {
        let _ = writer.write_all(Bar::Off.sequence().as_bytes());
        let _ = writer.flush();
    }
}

/// What the terminal's progress bar shows, as one number to remember it by.
const BAR_OFF: u16 = 0;

fn bar_state(bar: Bar) -> u16 {
    match bar {
        Bar::Off => BAR_OFF,
        Bar::Pulse => 1,
        Bar::Failed => 2,
        Bar::Percent { percent, trouble } => {
            256 + u16::from(percent.min(100)) * 2 + u16::from(trouble)
        }
    }
}

fn restore_terminal() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }

    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    clear_terminal_bar(&mut stdout);
    disable_bracketed_paste(&mut stdout);
    let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
    let _ = stdout.flush();
}

/// Asks the terminal to wrap pasted text in markers so it arrives as one
/// `Event::Paste` instead of a stream of key presses (which would trigger
/// single-letter actions). Best effort: failure only means pastes arrive as
/// key events. Skipped on Windows, where crossterm's console reader cannot
/// parse the markers and would surface them as literal keystrokes.
#[cfg(not(windows))]
fn enable_bracketed_paste(writer: &mut impl Write) {
    let _ = execute!(writer, EnableBracketedPaste);
}

#[cfg(windows)]
fn enable_bracketed_paste(_writer: &mut impl Write) {}

#[cfg(not(windows))]
fn disable_bracketed_paste(writer: &mut impl Write) {
    let _ = execute!(writer, DisableBracketedPaste);
}

#[cfg(windows)]
fn disable_bracketed_paste(_writer: &mut impl Write) {}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: what the terminal's bar shows is remembered for the whole process.
    #[test]
    fn the_terminals_bar_is_sent_when_it_changes_and_taken_away_on_the_way_out() {
        let mut screen = Vec::new();
        let mut sent = |bar: Bar| {
            screen.clear();
            show_terminal_bar(&mut screen, bar).expect("write");
            String::from_utf8(screen.clone()).expect("utf-8")
        };
        // Nothing showing and nothing to show: nothing is written, not even "remove".
        assert_eq!(sent(Bar::Off), "");
        assert_eq!(sent(Bar::of(1, 4, false)), "\x1b]9;4;1;25\x1b\\");
        // The same again is not sent with every frame.
        assert_eq!(sent(Bar::of(1, 4, false)), "");
        assert_eq!(sent(Bar::of(1, 4, true)), "\x1b]9;4;4;25\x1b\\");
        assert_eq!(sent(Bar::of(2, 4, true)), "\x1b]9;4;4;50\x1b\\");
        assert_eq!(sent(Bar::Pulse), "\x1b]9;4;3;0\x1b\\");
        assert_eq!(sent(Bar::Failed), "\x1b]9;4;2;100\x1b\\");
        assert_eq!(sent(Bar::Off), "\x1b]9;4;0;0\x1b\\");
        assert_eq!(sent(Bar::Off), "");

        // Leaving the TUI takes away a bar that is showing, once.
        assert_eq!(sent(Bar::of(3, 4, false)), "\x1b]9;4;1;75\x1b\\");
        let mut left = Vec::new();
        clear_terminal_bar(&mut left);
        clear_terminal_bar(&mut left);
        assert_eq!(
            String::from_utf8(left).expect("utf-8"),
            "\x1b]9;4;0;0\x1b\\"
        );
        // A write that fails is not remembered as done: the same bar is sent again, and a
        // bar that could not be taken away is taken away on the way out.
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("no terminal"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        show_terminal_bar(&mut Broken, Bar::of(1, 2, false)).expect_err("not written");
        assert_eq!(sent(Bar::of(1, 2, false)), "\x1b]9;4;1;50\x1b\\");
        show_terminal_bar(&mut Broken, Bar::Off).expect_err("not written");
        let mut left = Vec::new();
        clear_terminal_bar(&mut left);
        assert_eq!(
            String::from_utf8(left).expect("utf-8"),
            "\x1b]9;4;0;0\x1b\\"
        );

        // Every state is told apart from every other.
        let states = [
            Bar::Off,
            Bar::Pulse,
            Bar::Failed,
            Bar::of(0, 1, false),
            Bar::of(0, 1, true),
            Bar::of(1, 1, false),
            Bar::of(1, 1, true),
        ]
        .map(bar_state);
        for (index, state) in states.iter().enumerate() {
            assert!(!states[..index].contains(state), "{states:?}");
        }
    }
}
