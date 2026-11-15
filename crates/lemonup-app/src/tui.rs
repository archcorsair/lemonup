use std::io::{self, Stdout, Write};
use std::panic;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(windows))]
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub type Backend = CrosstermBackend<Stdout>;

static PANIC_HOOK: Once = Once::new();
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

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

fn restore_terminal() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }

    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
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
