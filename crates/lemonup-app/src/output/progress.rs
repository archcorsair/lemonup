//! The status line a command shows on stderr while it waits on the network: a spinner and
//! what is going on, erased before anything else is printed.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::Duration;

use lemonup_core::{AddonRecord, SourceKind};

use super::{OutputMode, Paint, columns, printable};
use crate::terminal_progress::Bar;

/// How long one spinner frame stays.
const FRAME: Duration = Duration::from_millis(100);

/// The width taken for a terminal that does not say how wide it is.
const UNKNOWN_WIDTH: usize = 80;

/// Erases the line the cursor is on and puts the cursor at its start.
const ERASE_LINE: &str = "\r\x1b[K";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

/// The status line on this process's stderr, once a command has said something on it. Log
/// lines and a panic's message go to the same terminal without passing through
/// [`super::Output`], so they find the line here to take it off the screen first.
static ON_STDERR: OnceLock<Arc<Shared>> = OnceLock::new();

/// Whether a run shows the status line: a readable run whose stderr is a terminal that
/// understands escape codes. Never in a pipe, and never with `--json`.
pub(crate) fn wanted(mode: OutputMode, stderr_is_terminal: bool, term: Option<&str>) -> bool {
    mode == OutputMode::Human && stderr_is_terminal && term != Some("dumb")
}

/// Whether a run uses the terminal's own progress bar: the terminal draws one, and the
/// "Terminal progress bar" setting is on.
pub(crate) fn bar_in_use(drawn_by_terminal: bool, setting: bool) -> bool {
    drawn_by_terminal && setting
}

/// What is shown: the text while the spinner turns, and the text of the one unchanging line
/// under Reduce motion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    moving: String,
    still: String,
}

/// Where the line is written: stderr, or a buffer in tests.
type Sink = Box<dyn Write + Send>;

/// How wide the line may be.
#[derive(Debug, Clone, Copy)]
enum Width {
    /// As wide as the terminal is right now. It is asked with every frame, since a window
    /// can be made narrower while a command runs.
    Terminal,
    #[cfg(test)]
    Columns(usize),
}

struct State {
    writer: Sink,
    paint: Paint,
    /// Reduce motion: one unchanging line with the ellipsis, no animation.
    still: bool,
    width: Width,
    shown: Option<Shown>,
    frame: usize,
    /// The line is on screen as it should be. Under Reduce motion it is then left alone.
    drawn: bool,
    /// The terminal draws a progress bar of its own, and the setting for it is on.
    bar_enabled: bool,
    /// What the terminal's progress bar should show, and what it was last told to show.
    bar_wanted: Bar,
    bar_shown: Bar,
    /// The cursor is hidden only when something stands ready to show it again on Ctrl+C.
    may_hide_cursor: bool,
    cursor_hidden: bool,
    stopped: bool,
    /// The line draws itself: a thread is started with the first thing a command says.
    /// Without it only [`Progress::tick`] draws, which is what tests use.
    live: bool,
    started: bool,
    /// This is the line on the process's own stderr, which log lines and a panic's message
    /// share with it.
    on_stderr: bool,
    /// Every text a command showed, for a test to read.
    #[cfg(test)]
    said: Vec<String>,
}

/// `text` without control characters, cut to `room` columns. A name is what a source or a
/// file says it is, and a line that wraps could not be erased again.
fn cut_to(text: &str, room: usize) -> (String, bool) {
    let text = printable(text);
    let mut cut = String::new();
    let mut taken = 0;
    let mut buffer = [0; 4];
    for character in text.chars() {
        let wide = columns(character.encode_utf8(&mut buffer));
        if taken + wide > room {
            return (cut.trim_end().to_string(), true);
        }
        taken += wide;
        cut.push(character);
    }
    (cut, false)
}

impl State {
    /// The columns a line may take: one less than the terminal has, so the cursor never
    /// moves to the next row.
    fn room(&self) -> usize {
        let columns = match self.width {
            #[cfg(test)]
            Width::Columns(columns) => columns,
            Width::Terminal => crossterm::terminal::size()
                .map(|(columns, _)| usize::from(columns))
                .unwrap_or(0),
        };
        // A terminal that gives no size, or says 0, is taken to be 80 columns wide.
        let columns = if columns == 0 { UNKNOWN_WIDTH } else { columns };
        columns.saturating_sub(1)
    }

    /// `text` with the ellipsis after it when it was cut to fit `room`. `None` when not
    /// even the ellipsis fits.
    fn fitted(&self, text: &str, room: usize) -> Option<String> {
        let (whole, was_cut) = cut_to(text, room);
        if !was_cut {
            return Some(whole);
        }
        let ellipsis = self.paint.symbols.ellipsis;
        let room = room.checked_sub(columns(ellipsis))?;
        Some(format!("{}{ellipsis}", cut_to(text, room).0))
    }

    /// The line as it is now, or `None` when there is nothing to draw or no room for it.
    fn line(&mut self) -> Option<String> {
        let shown = self.shown.clone()?;
        let room = self.room();
        if self.still {
            if self.drawn {
                return None;
            }
            // The text ends in the ellipsis whether it was cut or not: one, never two.
            let ellipsis = self.paint.symbols.ellipsis;
            let (text, _) = cut_to(&shown.still, room.checked_sub(columns(ellipsis))?);
            return Some(format!("{ERASE_LINE}{text}{ellipsis}"));
        }
        let frames = self.paint.symbols.spinner;
        let frame = frames[self.frame % frames.len()];
        let after_frame = room.checked_sub(columns(frame))?;
        self.frame = self.frame.wrapping_add(1);
        let text = after_frame
            .checked_sub(1)
            .and_then(|room| self.fitted(&shown.moving, room))
            .filter(|text| !text.is_empty())
            .map(|text| format!(" {text}"))
            .unwrap_or_default();
        let hide = if self.may_hide_cursor && !self.cursor_hidden {
            self.cursor_hidden = true;
            HIDE_CURSOR
        } else {
            ""
        };
        Some(format!(
            "{ERASE_LINE}{hide}{}{text}",
            self.paint.attention(frame)
        ))
    }

    /// Tells the terminal's own progress bar what to show, when that changed. Nothing is
    /// ever written to a terminal that is not known to draw one: others show the sequence
    /// as a notification.
    fn draw_bar(&mut self) {
        // The pulse is an animation, which Reduce motion turns off.
        let wanted = match self.bar_wanted {
            Bar::Pulse if self.still => Bar::Off,
            wanted => wanted,
        };
        if self.bar_enabled && wanted != self.bar_shown {
            let _ = self.writer.write_all(wanted.sequence().as_bytes());
            let _ = self.writer.flush();
            self.bar_shown = wanted;
        }
    }

    /// Draws the line as it is now. Errors are dropped: a status line that can't be
    /// written must never stop the command it reports on.
    fn draw(&mut self) {
        if self.stopped {
            return;
        }
        self.draw_bar();
        if let Some(line) = self.line() {
            let _ = self.writer.write_all(line.as_bytes());
            let _ = self.writer.flush();
            self.drawn = true;
        }
    }

    /// Takes the line off the screen and gives the cursor back. What is shown is kept, so
    /// the next draw puts it back.
    fn erase(&mut self) {
        if !self.drawn && !self.cursor_hidden {
            return;
        }
        let show = if self.cursor_hidden { SHOW_CURSOR } else { "" };
        let _ = self
            .writer
            .write_all(format!("{ERASE_LINE}{show}").as_bytes());
        let _ = self.writer.flush();
        self.drawn = false;
        self.cursor_hidden = false;
    }

    /// The wait is over: the line goes, and so does the terminal's progress bar.
    fn clear(&mut self) {
        self.shown = None;
        self.bar_wanted = Bar::Off;
        self.erase();
        self.draw_bar();
    }

    /// Takes the line away for good.
    fn end(&mut self) {
        self.clear();
        self.stopped = true;
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// The thread that draws the line, once a command has said something on a terminal.
    ticker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Shared {
    /// A status line must never take the command down with it, so a lock that a panic left
    /// behind is used as it is.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes the line off the screen for good on the ways out that can't wait: Ctrl+C and a
    /// panic. Whoever holds the lock is writing a frame or a line and is done in no time,
    /// so it is asked for a few times. A holder that is stuck, because the terminal itself
    /// is stopped (Ctrl+S), is not waited for: nothing could be written to that terminal
    /// from here either.
    fn leave(&self) {
        for _ in 0..20 {
            match self.state.try_lock() {
                Ok(mut state) => return state.end(),
                Err(TryLockError::Poisoned(poisoned)) => return poisoned.into_inner().end(),
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    /// Starts what keeps the line alive on a terminal, the first time a command says
    /// something: the wait for Ctrl+C, and the thread that draws a frame every [`FRAME`]. A
    /// command that never waits starts neither.
    fn start(shared: &Arc<Self>) {
        // The cursor is hidden only once Ctrl+C is sure to show it again.
        let watched = Self::watch_for_ctrl_c(shared);
        let on_stderr = {
            let mut state = shared.lock();
            state.may_hide_cursor = watched;
            // The same goes for the terminal's progress bar: Ctrl+C has to take it away.
            state.bar_enabled &= watched;
            state.on_stderr
        };
        if on_stderr && ON_STDERR.set(Arc::clone(shared)).is_ok() {
            // A panic's message goes to stderr too: the line makes room for it.
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                if let Some(line) = ON_STDERR.get() {
                    line.leave();
                }
                before(panic);
            }));
        }
        let drawing = Arc::clone(shared);
        let ticker = std::thread::Builder::new()
            .name("status-line".to_string())
            .spawn(move || {
                let mut state = drawing.lock();
                while !state.stopped {
                    state.draw();
                    state = drawing
                        .wake
                        .wait_timeout(state, FRAME)
                        .map(|(state, _)| state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner().0);
                }
            })
            .ok();
        *shared
            .ticker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ticker;
    }

    /// Listens for Ctrl+C from now on, and says whether it does. Without a listener Ctrl+C
    /// ends the process at once, and a hidden cursor would stay hidden in the shell.
    ///
    /// The listener stays for the rest of the run, also after the line is gone: once Ctrl+C
    /// is listened for, it does nothing by itself any more. The run ends with 130, the exit
    /// code a shell gives a command that Ctrl+C ended.
    fn watch_for_ctrl_c(shared: &Arc<Self>) -> bool {
        #[cfg(any(unix, windows))]
        {
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return false;
            };
            let _inside = runtime.enter();
            // Made here and not in the task below, so it listens before the first frame.
            #[cfg(unix)]
            let pressed = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
            #[cfg(windows)]
            let pressed = tokio::signal::windows::ctrl_c();
            let Ok(mut pressed) = pressed else {
                return false;
            };
            let line = Arc::clone(shared);
            runtime.spawn(async move {
                if pressed.recv().await.is_some() {
                    line.leave();
                    std::process::exit(130);
                }
            });
            true
        }
        // Nothing to listen with: the cursor is never hidden.
        #[cfg(not(any(unix, windows)))]
        {
            let _ = shared;
            false
        }
    }
}

/// The status line of one run. On a terminal it is drawn by a thread of its own, so it keeps
/// turning while the command waits. Dropping it erases the line and gives the cursor back.
pub(crate) struct Progress {
    shared: Arc<Shared>,
}

impl Progress {
    /// The status line on this process's stderr. Ctrl+C erases it and gives the cursor back
    /// before the run ends.
    pub(crate) fn on_stderr(paint: Paint, still: bool, bar: bool) -> Self {
        let progress = Self::build(
            Box::new(std::io::stderr()),
            paint,
            still,
            Width::Terminal,
            true,
        );
        {
            let mut state = progress.shared.lock();
            state.on_stderr = true;
            state.bar_enabled = bar;
        }
        progress
    }

    /// A status line written to `writer`, `width` columns wide, drawn only by
    /// [`Progress::tick`].
    #[cfg(test)]
    pub(crate) fn new(writer: Sink, paint: Paint, still: bool, width: usize) -> Self {
        Self::build(writer, paint, still, Width::Columns(width), false)
    }

    fn build(writer: Sink, paint: Paint, still: bool, width: Width, live: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    writer,
                    paint,
                    still,
                    width,
                    shown: None,
                    frame: 0,
                    drawn: false,
                    bar_enabled: false,
                    bar_wanted: Bar::Off,
                    bar_shown: Bar::Off,
                    may_hide_cursor: !live,
                    cursor_hidden: false,
                    stopped: false,
                    live,
                    started: false,
                    on_stderr: false,
                    #[cfg(test)]
                    said: Vec::new(),
                }),
                wake: Condvar::new(),
                ticker: Mutex::new(None),
            }),
        }
    }

    /// The same line on a terminal that draws a progress bar of its own, for tests.
    #[cfg(test)]
    pub(crate) fn with_bar(self) -> Self {
        self.shared.lock().bar_enabled = true;
        self
    }

    /// What a command uses to say what it is doing.
    pub(crate) fn status(&self) -> Status {
        Status(Some(Arc::clone(&self.shared)))
    }

    /// Draws the next frame now. The ticker does this on its own; tests call it.
    #[cfg(test)]
    pub(crate) fn tick(&self) {
        self.shared.lock().draw();
    }

    /// Every text a command showed next to the spinner, in order.
    #[cfg(test)]
    pub(crate) fn said(&self) -> Vec<String> {
        self.shared.lock().said.clone()
    }

    /// Takes the line off the screen until the returned guard is dropped, so something else
    /// can be printed where it was. The line comes back with the next frame.
    pub(crate) fn pause(&self) -> impl Drop + '_ {
        let mut state = self.shared.lock();
        state.erase();
        state
    }

    /// Ends the status line: nothing is shown until a command shows something again.
    pub(crate) fn clear(&self) {
        self.status().clear();
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.shared.lock().end();
        self.shared.wake.notify_all();
        let ticker = self
            .shared
            .ticker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(ticker) = ticker {
            let _ = ticker.join();
        }
    }
}

/// Where log lines go: stderr, with the status line taken off the screen first. A log line
/// written next to the spinner would run on from it.
pub(crate) fn stderr_for_logs() -> impl Write {
    AroundTheLine {
        line: ON_STDERR.get().cloned(),
        to: std::io::stderr(),
    }
}

/// A writer that takes a status line off the screen before each write. The line comes back
/// with its next frame.
struct AroundTheLine<W: Write> {
    line: Option<Arc<Shared>>,
    to: W,
}

impl<W: Write> Write for AroundTheLine<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _paused = self.line.as_ref().map(|line| {
            let mut state = line.lock();
            state.erase();
            state
        });
        self.to.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.to.flush()
    }
}

/// A handle on the status line that a command, or the callbacks it hands out, can keep.
/// Without a status line (a pipe, `--json`) every call does nothing.
#[derive(Clone)]
pub(crate) struct Status(Option<Arc<Shared>>);

/// Takes the status line away when it is dropped, however the wait ends.
pub(crate) struct Waiting<'a>(&'a Status);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
impl Status {
    /// The same text with the spinner and under Reduce motion, until it is cleared.
    pub(crate) fn say(&self, text: &str) {
        self.show(text, text, Bar::Off);
    }
}

impl Status {
    /// A handle that shows nothing.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// Says what the command is doing. `moving` is the text next to the spinner and can
    /// change as often as it likes. `still` is the one line Reduce motion shows: it stays
    /// as first shown until the line is cleared. `bar` is what the terminal's own progress
    /// bar shows meanwhile, where there is one.
    pub(crate) fn show(&self, moving: &str, still: &str, bar: Bar) {
        let Some(shared) = &self.0 else {
            return;
        };
        let mut state = shared.lock();
        // The run is over: its status line is gone for good.
        if state.stopped {
            return;
        }
        let still = match &state.shown {
            Some(shown) if state.still => shown.still.clone(),
            _ => still.to_string(),
        };
        #[cfg(test)]
        state.said.push(moving.to_string());
        state.shown = Some(Shown {
            moving: moving.to_string(),
            still,
        });
        state.bar_wanted = bar;
        if state.live && !state.started {
            state.started = true;
            drop(state);
            Shared::start(shared);
        }
    }

    /// Says `text`, with the spinner and under Reduce motion alike, until the returned
    /// guard is dropped. There is no count to show, so the terminal's progress bar pulses.
    pub(crate) fn waiting(&self, text: &str) -> Waiting<'_> {
        self.show(text, text, Bar::Pulse);
        Waiting(self)
    }

    /// A guard that takes away whatever is shown from now on once it is dropped.
    pub(crate) fn until_done(&self) -> Waiting<'_> {
        Waiting(self)
    }

    /// Takes the line away: the command is done waiting.
    pub(crate) fn clear(&self) {
        let Some(shared) = &self.0 else {
            return;
        };
        shared.lock().clear();
    }
}

/// Counts a check or an update through its addons: `Checking 3 of 48`, and the name of the
/// addon with a request out while exactly one has. With none or several out it names none,
/// as the TUI's status line does.
pub(crate) struct Tally {
    label: &'static str,
    /// The name of each addon that is counted, by its folder.
    names: HashMap<String, String>,
    /// The folders that are done, and whether one of them failed.
    done: HashSet<String>,
    trouble: bool,
    /// The folders with a request out, in the order they started.
    active: Vec<String>,
}

impl Tally {
    /// A count of the addons among `selected` that a source is asked about: the linked
    /// ones, without those from Wago.io when it has no key.
    pub(crate) fn new(label: &'static str, selected: &[&AddonRecord], has_wago_key: bool) -> Self {
        Self {
            label,
            names: selected
                .iter()
                .filter(|addon| match addon.source {
                    SourceKind::Manual => false,
                    SourceKind::Wago => has_wago_key,
                    _ => true,
                })
                .map(|addon| (addon.folder.clone(), addon.display_name().to_string()))
                .collect(),
            done: HashSet::new(),
            trouble: false,
            active: Vec::new(),
        }
    }

    /// The request for the addon in `folder` is going out.
    pub(crate) fn started(&mut self, folder: &str) {
        if self.names.contains_key(folder) && !self.active.iter().any(|active| active == folder) {
            self.active.push(folder.to_string());
        }
    }

    /// The addon in `folder` is done; `failed` when it could not be checked or updated.
    pub(crate) fn finished(&mut self, folder: &str, failed: bool) {
        if self.names.contains_key(folder) {
            self.done.insert(folder.to_string());
            self.trouble |= failed;
        }
        self.active.retain(|active| active != folder);
    }

    /// `Checking 3 of 48`, or `Updating 3 of 5: Deadly Boss Mods`.
    pub(crate) fn moving(&self) -> String {
        let total = self.names.len();
        let shown = (self.done.len() + 1).min(total);
        let counted = format!("{} {shown} of {total}", self.label);
        match self.active.as_slice() {
            [folder] => match self.names.get(folder) {
                Some(name) => format!("{counted}: {name}"),
                None => counted,
            },
            _ => counted,
        }
    }

    /// `Checking 48 addons`, the line Reduce motion shows.
    pub(crate) fn still(&self) -> String {
        match self.names.len() {
            1 => format!("{} 1 addon", self.label),
            total => format!("{} {total} addons", self.label),
        }
    }

    /// What the terminal's progress bar shows: how much is done, yellow once an addon
    /// failed.
    pub(crate) fn bar(&self) -> Bar {
        Bar::of(self.done.len(), self.names.len(), self.trouble)
    }

    /// Shows where the count stands. With no addon to ask a source about, nothing is
    /// waited for and nothing is shown.
    pub(crate) fn show(&self, status: &Status) {
        if !self.names.is_empty() {
            status.show(&self.moving(), &self.still(), self.bar());
        }
    }
}

/// A status line a test can read: it writes to a buffer and is drawn by [`Progress::tick`].
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A writer the test can read back while the status line still owns it.
    #[derive(Clone, Default)]
    pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("buffer").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buffer {
        /// Everything written so far, left where it is.
        pub(crate) fn peek(&self) -> String {
            String::from_utf8(self.0.lock().expect("buffer").clone()).expect("utf-8")
        }

        /// What was written since the last look.
        pub(crate) fn take(&self) -> String {
            String::from_utf8(std::mem::take(&mut *self.0.lock().expect("buffer"))).expect("utf-8")
        }
    }

    /// A status line that draws itself into a buffer, as the one on stderr does.
    pub(crate) fn live(paint: Paint, still: bool, width: usize) -> (Buffer, Progress) {
        let buffer = Buffer::default();
        let progress = Progress::build(
            Box::new(buffer.clone()),
            paint,
            still,
            Width::Columns(width),
            true,
        );
        (buffer, progress)
    }

    pub(crate) fn progress(paint: Paint, still: bool, width: usize) -> (Progress, Buffer) {
        let buffer = Buffer::default();
        (
            Progress::new(Box::new(buffer.clone()), paint, still, width),
            buffer,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::testing::progress;

    const AMBER: &str = "\x1b[38;5;3m";
    const END: &str = "\x1b[0m";

    #[test]
    fn the_status_line_shows_only_on_a_terminal_in_a_readable_run() {
        assert!(wanted(OutputMode::Human, true, Some("xterm-256color")));
        assert!(wanted(OutputMode::Human, true, None));
        // A pipe, `--json`, and a terminal that knows no escape codes.
        assert!(!wanted(OutputMode::Human, false, Some("xterm-256color")));
        assert!(!wanted(OutputMode::Json, true, Some("xterm-256color")));
        assert!(!wanted(OutputMode::Human, true, Some("dumb")));
    }

    #[test]
    fn the_spinner_turns_next_to_what_the_command_says() {
        let (progress, written) = progress(Paint::terminal(), false, 80);
        let status = progress.status();

        // Nothing is drawn before a command says something.
        progress.tick();
        assert_eq!(written.take(), "");

        status.say("Checking 1 of 3");
        progress.tick();
        // The cursor is hidden once, with the first frame. The spinner is in the attention
        // color and the text is not colored.
        assert_eq!(
            written.take(),
            format!("\r\x1b[K\x1b[?25l{AMBER}⠋{END} Checking 1 of 3")
        );
        status.say("Checking 2 of 3");
        progress.tick();
        assert_eq!(
            written.take(),
            format!("\r\x1b[K{AMBER}⠙{END} Checking 2 of 3")
        );
        progress.tick();
        progress.tick();
        progress.tick();
        let frames = written.take();
        assert!(frames.contains("⠸") && frames.contains("⠴"), "{frames:?}");
        // After the last frame comes the first again.
        assert!(
            frames.ends_with(&format!("{AMBER}⠋{END} Checking 2 of 3")),
            "{frames:?}"
        );

        status.clear();
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");
        // Cleared, it stays away, and clearing again writes nothing.
        progress.tick();
        status.clear();
        assert_eq!(written.take(), "");
    }

    #[test]
    fn simple_symbols_and_no_color_draw_the_ascii_spinner_plain() {
        let paint = Paint::PLAIN.with_symbols(crate::theme::Symbols::SIMPLE);
        let (progress, written) = progress(paint, false, 80);
        progress.status().say("Asking TukUI");
        for _ in 0..5 {
            progress.tick();
        }
        assert_eq!(
            written.take(),
            "\r\x1b[K\x1b[?25l| Asking TukUI\r\x1b[K/ Asking TukUI\r\x1b[K- Asking TukUI\
             \r\x1b[K\\ Asking TukUI\r\x1b[K| Asking TukUI"
        );
    }

    #[test]
    fn the_line_never_wraps_and_never_carries_control_characters() {
        let (progress, written) = progress(Paint::PLAIN, false, 24);
        let status = progress.status();
        status.say("Updating 3 of 5: A Very Long Addon Name Indeed");
        progress.tick();
        let line = written.take();
        let text = line.rsplit("\x1b[?25l").next().expect("text");
        assert_eq!(text, "⠋ Updating 3 of 5: A V…");
        assert!(columns(text) < 24, "{text:?}");

        // A name is what a source or a file says it is.
        status.say("Downloading Evil\x1b]0;x\x07\nName");
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[K⠙ Downloading Evil ]0;…");
    }

    #[test]
    fn reduce_motion_shows_one_unchanging_line() {
        let (progress, written) = progress(Paint::terminal(), true, 80);
        let status = progress.status();
        status.show("Checking 1 of 48", "Checking 48 addons", Bar::Off);
        progress.tick();
        // No spinner, no color, and the cursor stays as it is.
        assert_eq!(written.take(), "\r\x1b[KChecking 48 addons…");

        // The count moves on; the line does not.
        status.show("Checking 2 of 48: Pawn", "Checking 48 addons", Bar::Off);
        status.show("Checking 3 of 48", "something else", Bar::Off);
        progress.tick();
        progress.tick();
        assert_eq!(written.take(), "");

        // Something printed in between takes the line away, and it comes back once.
        drop(progress.pause());
        assert_eq!(written.take(), "\r\x1b[K");
        progress.tick();
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[KChecking 48 addons…");

        // A new wait after a clear says its own line.
        status.clear();
        assert_eq!(written.take(), "\r\x1b[K");
        status.show("Updating 1 of 2", "Updating 2 addons", Bar::Off);
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[KUpdating 2 addons…");
    }

    #[test]
    fn a_pause_takes_the_line_away_and_the_next_frame_brings_it_back() {
        let (progress, written) = progress(Paint::PLAIN, false, 80);
        progress.status().say("Asking TukUI");
        progress.tick();
        written.take();

        {
            let _paused = progress.pause();
            // The line is gone and the cursor is back while something else is printed.
            assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");
        }
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25l⠙ Asking TukUI");

        // A pause with nothing on screen writes nothing.
        progress.clear();
        written.take();
        drop(progress.pause());
        assert_eq!(written.take(), "");
    }

    #[test]
    fn dropping_the_status_line_erases_it_and_gives_the_cursor_back() {
        let (progress, written) = progress(Paint::PLAIN, false, 80);
        progress.status().say("Asking TukUI");
        progress.tick();
        written.take();
        drop(progress);
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");

        // A handle that outlives the line, or has none, does nothing.
        let (progress, written) = testing::progress(Paint::PLAIN, false, 80);
        let status = progress.status();
        drop(progress);
        status.say("late");
        status.clear();
        assert_eq!(written.take(), "");
        Status::none().say("nobody is looking");
        Status::none().clear();
    }

    fn row(name: &str, folder: &str, source: SourceKind) -> AddonRecord {
        AddonRecord::new(name, folder, source)
    }

    #[test]
    fn a_tally_counts_the_addons_a_source_is_asked_about() {
        let rows = [
            row("Deadly Boss Mods", "DBM-Core", SourceKind::Wago),
            row("Pawn", "Pawn", SourceKind::WowInterface),
            row("ElvUI", "ElvUI", SourceKind::Tukui),
            // Not linked: no source is asked about it, so it is not counted.
            row("Mine", "Mine", SourceKind::Manual),
        ];
        let selected = rows.iter().collect::<Vec<_>>();
        let mut tally = Tally::new("Updating", &selected, true);
        assert_eq!(tally.moving(), "Updating 1 of 3");
        assert_eq!(tally.still(), "Updating 3 addons");

        tally.started("DBM-Core");
        assert_eq!(tally.moving(), "Updating 1 of 3: Deadly Boss Mods");
        // With several out, none is named.
        tally.started("Pawn");
        assert_eq!(tally.moving(), "Updating 1 of 3");
        tally.finished("DBM-Core", false);
        assert_eq!(tally.moving(), "Updating 2 of 3: Pawn");
        // An addon that is done twice is counted once.
        tally.finished("DBM-Core", false);
        assert_eq!(tally.moving(), "Updating 2 of 3: Pawn");
        // An addon that is not counted changes nothing, and neither does another folder
        // that differs only in case: folders are told apart as the rows have them.
        tally.started("Mine");
        tally.finished("Mine", false);
        tally.finished("Unknown", false);
        tally.finished("pawn", false);
        assert_eq!(tally.moving(), "Updating 2 of 3: Pawn");
        tally.finished("Pawn", false);
        tally.started("ElvUI");
        assert_eq!(tally.moving(), "Updating 3 of 3: ElvUI");
        // The count never passes the total.
        tally.finished("ElvUI", false);
        assert_eq!(tally.moving(), "Updating 3 of 3");
        assert_eq!(tally.still(), "Updating 3 addons");

        // Without a Wago.io key its addons are skipped, not asked about.
        let without_key = Tally::new("Checking", &selected, false);
        assert_eq!(without_key.moving(), "Checking 1 of 2");
        assert_eq!(without_key.still(), "Checking 2 addons");
    }

    #[test]
    fn a_tally_of_one_reads_right_and_one_of_none_shows_nothing() {
        let pawn = row("Pawn", "Pawn", SourceKind::Wago);
        let one = Tally::new("Checking", &[&pawn], true);
        assert_eq!(one.moving(), "Checking 1 of 1");
        assert_eq!(one.still(), "Checking 1 addon");

        // Nothing to ask a source about: nothing is waited for, so nothing is said.
        let (progress, _) = testing::progress(Paint::PLAIN, false, 80);
        let status = progress.status();
        Tally::new("Checking", &[], true).show(&status);
        Tally::new("Checking", &[&pawn], false).show(&status);
        assert_eq!(progress.said(), Vec::<String>::new());
        one.show(&status);
        assert_eq!(progress.said(), ["Checking 1 of 1"]);
    }

    #[test]
    fn reduce_motion_ends_a_line_that_was_cut_with_one_ellipsis() {
        for (symbols, line) in [
            (crate::theme::Symbols::FANCY, "Downloading https://gi…"),
            (crate::theme::Symbols::SIMPLE, "Downloading https://..."),
        ] {
            let (progress, written) =
                testing::progress(Paint::PLAIN.with_symbols(symbols), true, 24);
            let status = progress.status();
            let _waiting = status.waiting("Downloading https://github.com/WeakAuras/WeakAuras2");
            progress.tick();
            let drawn = written.take();
            assert_eq!(drawn, format!("\r\x1b[K{line}"));
            assert!(columns(line) < 24, "{line:?}");
        }
    }

    #[test]
    fn a_terminal_too_narrow_for_the_text_gets_the_spinner_alone_or_nothing() {
        let frame = |width: usize| {
            let (progress, written) = testing::progress(Paint::PLAIN, false, width);
            progress
                .status()
                .show("Checking 1 of 3", "Checking 3 addons", Bar::Off);
            progress.tick();
            written.take()
        };
        // No size given counts as 80 columns.
        assert_eq!(frame(0), "\r\x1b[K\x1b[?25l⠋ Checking 1 of 3");
        // Room for the text, cut.
        assert_eq!(frame(10), "\r\x1b[K\x1b[?25l⠋ Checki…");
        // Room for the spinner and no more.
        assert_eq!(frame(4), "\r\x1b[K\x1b[?25l⠋ …");
        assert_eq!(frame(3), "\r\x1b[K\x1b[?25l⠋");
        assert_eq!(frame(2), "\r\x1b[K\x1b[?25l⠋");
        // No room at all: nothing is drawn, and nothing has to be erased.
        assert_eq!(frame(1), "");

        let (progress, written) = testing::progress(Paint::PLAIN, true, 3);
        progress
            .status()
            .show("Checking 1 of 3", "Checking 3 addons", Bar::Off);
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[KC…");
        let (progress, written) = testing::progress(Paint::PLAIN, true, 1);
        progress
            .status()
            .show("Checking 1 of 3", "Checking 3 addons", Bar::Off);
        progress.tick();
        assert_eq!(written.take(), "");
    }

    #[test]
    fn a_wait_ends_its_line_however_it_ends() {
        let (progress, written) = testing::progress(Paint::PLAIN, false, 80);
        let status = progress.status();
        let wait = || -> Result<(), ()> {
            let _waiting = status.waiting("Downloading elvui");
            progress.tick();
            Err(())?;
            Ok(())
        };
        wait().expect_err("gave up");
        assert_eq!(
            written.take(),
            "\r\x1b[K\x1b[?25l⠋ Downloading elvui\r\x1b[K\x1b[?25h"
        );
        progress.tick();
        assert_eq!(written.take(), "");

        // The same for a line that changes while it shows.
        {
            let _line = status.until_done();
            status.show("Checking 1 of 2", "Checking 2 addons", Bar::Off);
            status.show("Checking 2 of 2", "Checking 2 addons", Bar::Off);
            progress.tick();
            written.take();
        }
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");
    }

    #[test]
    fn a_log_line_takes_the_status_line_off_the_screen_first() {
        let (progress, screen) = testing::progress(Paint::PLAIN, false, 80);
        progress.status().say("Asking TukUI");
        progress.tick();
        screen.take();

        let mut logs = AroundTheLine {
            line: Some(Arc::clone(&progress.shared)),
            to: screen.clone(),
        };
        logs.write_all(b"{\"level\":\"WARN\"}\n").expect("log");
        // The line is gone before the log line, which so starts at the left edge.
        assert_eq!(screen.take(), "\r\x1b[K\x1b[?25h{\"level\":\"WARN\"}\n");
        progress.tick();
        assert_eq!(screen.take(), "\r\x1b[K\x1b[?25l⠙ Asking TukUI");

        // Under Reduce motion the one line comes back too.
        let (progress, screen) = testing::progress(Paint::PLAIN, true, 80);
        progress.status().say("Asking TukUI");
        progress.tick();
        screen.take();
        let mut logs = AroundTheLine {
            line: Some(Arc::clone(&progress.shared)),
            to: screen.clone(),
        };
        logs.write_all(b"log\n").expect("log");
        progress.tick();
        assert_eq!(screen.take(), "\r\x1b[Klog\n\r\x1b[KAsking TukUI…");

        // Without a status line a log line is written as it is.
        let mut logs = AroundTheLine {
            line: None,
            to: screen.clone(),
        };
        logs.write_all(b"log\n").expect("log");
        assert_eq!(screen.take(), "log\n");
    }

    #[test]
    fn a_live_line_draws_itself_and_never_hides_a_cursor_it_could_not_show_again() {
        // Outside a runtime nothing listens for Ctrl+C, so the cursor is left alone.
        // The same goes for the terminal's progress bar: nothing would take it away.
        let (screen, progress) = testing::live(Paint::PLAIN, false, 80);
        let progress = progress.with_bar();
        let status = progress.status();
        // Waits for the line's own thread to have drawn `text`, however busy the machine is.
        let drawn = |text: &str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !screen.peek().contains(text) {
                assert!(std::time::Instant::now() < deadline, "never drew {text:?}");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        status.show("Checking 1 of 2", "Checking 2 addons", Bar::of(0, 2, false));
        // The spinner turns with nobody driving it.
        drawn("⠙ Checking 1 of 2");
        status.show("Checking 2 of 2", "Checking 2 addons", Bar::of(1, 2, true));
        drawn(" Checking 2 of 2");
        // Dropping it joins the thread, so nothing is drawn after this line.
        drop(progress);
        let all = screen.take();
        assert!(!all.contains("\x1b[?25"), "{all:?}");
        assert!(!all.contains("]9;"), "{all:?}");
        assert!(
            all.starts_with("\r\x1b[K⠋ Checking 1 of 2\r\x1b[K"),
            "{all:?}"
        );
        assert!(all.ends_with("Checking 2 of 2\r\x1b[K"), "{all:?}");
        // Gone for good: saying more draws nothing.
        status.show("late", "late", Bar::Off);
        std::thread::sleep(FRAME * 3);
        assert_eq!(screen.take(), "");
    }

    #[test]
    fn a_way_out_that_cant_wait_takes_the_line_away_for_good() {
        let (progress, written) = testing::progress(Paint::PLAIN, false, 80);
        let status = progress.status();
        status.show("Checking 1 of 2", "Checking 2 addons", Bar::Off);
        progress.tick();
        written.take();

        // Ctrl+C and a panic end the line without waiting for the command.
        progress.shared.leave();
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");
        // No frame is drawn after it, whatever the command still says.
        status.show("Checking 2 of 2", "Checking 2 addons", Bar::Off);
        progress.tick();
        assert_eq!(written.take(), "");

        // A holder that never lets go, as one stuck writing to a stopped terminal, is not
        // waited for: the way out stays open, and nothing is written from here.
        let (progress, written) = testing::progress(Paint::PLAIN, false, 80);
        progress
            .status()
            .show("Asking TukUI", "Asking TukUI", Bar::Off);
        progress.tick();
        written.take();
        let shared = Arc::clone(&progress.shared);
        let held = progress.shared.lock();
        std::thread::spawn(move || shared.leave())
            .join()
            .expect("left without the lock");
        assert!(!held.stopped);
        drop(held);
        assert_eq!(written.take(), "");
    }

    const BAR_OFF: &str = "\x1b]9;4;0;0\x1b\\";

    #[test]
    fn the_terminals_progress_bar_follows_the_count_and_goes_with_the_line() {
        let (progress, written) = progress(Paint::PLAIN, false, 80);
        let progress = progress.with_bar();
        let status = progress.status();

        status.show("Checking 1 of 4", "Checking 4 addons", Bar::of(0, 4, false));
        progress.tick();
        assert_eq!(
            written.take(),
            "\x1b]9;4;1;0\x1b\\\r\x1b[K\x1b[?25l⠋ Checking 1 of 4"
        );
        // An unchanged bar is not sent again with every frame.
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[K⠙ Checking 1 of 4");
        // It moves with the count, and turns yellow once an addon failed.
        status.show("Checking 2 of 4", "Checking 4 addons", Bar::of(1, 4, false));
        progress.tick();
        assert!(written.take().starts_with("\x1b]9;4;1;25\x1b\\"));
        status.show("Checking 3 of 4", "Checking 4 addons", Bar::of(2, 4, true));
        progress.tick();
        assert!(written.take().starts_with("\x1b]9;4;4;50\x1b\\"));

        // A warning takes the line away for a moment, and leaves the bar alone.
        drop(progress.pause());
        assert_eq!(written.take(), "\r\x1b[K\x1b[?25h");
        progress.tick();
        assert!(!written.take().contains("]9;4"));

        // When the wait is over the bar goes with the line, once.
        status.clear();
        assert_eq!(written.take(), format!("\r\x1b[K\x1b[?25h{BAR_OFF}"));
        status.clear();
        drop(progress);
        assert_eq!(written.take(), "");
    }

    #[test]
    fn every_way_out_takes_the_terminals_progress_bar_away() {
        // A guard that is dropped, as on an early return.
        let (progress, written) = progress(Paint::PLAIN, false, 80);
        let progress = progress.with_bar();
        let status = progress.status();
        {
            let _waiting = status.waiting("Asking TukUI");
            progress.tick();
            // No count to show: the bar pulses.
            assert!(written.take().starts_with("\x1b]9;4;3;0\x1b\\"));
        }
        assert!(written.take().ends_with(BAR_OFF));

        // The line being dropped with the run.
        let (progress, written) = super::testing::progress(Paint::PLAIN, false, 80);
        let progress = progress.with_bar();
        progress
            .status()
            .show("Updating 1 of 2", "Updating 2 addons", Bar::of(0, 2, false));
        progress.tick();
        written.take();
        drop(progress);
        assert!(written.take().ends_with(BAR_OFF));

        // Ctrl+C and a panic.
        let (progress, written) = super::testing::progress(Paint::PLAIN, false, 80);
        let progress = progress.with_bar();
        progress
            .status()
            .show("Updating 1 of 2", "Updating 2 addons", Bar::of(0, 2, false));
        progress.tick();
        written.take();
        progress.shared.leave();
        assert!(written.take().ends_with(BAR_OFF));
    }

    #[test]
    fn the_bar_needs_a_terminal_that_draws_it_and_the_setting_on() {
        assert!(bar_in_use(true, true));
        // The setting is off, or the terminal would show a notification.
        assert!(!bar_in_use(true, false));
        assert!(!bar_in_use(false, true));
        assert!(!bar_in_use(false, false));
    }

    #[test]
    fn a_terminal_that_draws_no_bar_is_never_sent_one() {
        // Other terminals show the sequence as a notification, the one that removes the bar
        // included.
        let (progress, written) = progress(Paint::PLAIN, false, 80);
        let status = progress.status();
        status.show("Checking 1 of 4", "Checking 4 addons", Bar::of(0, 4, true));
        progress.tick();
        let _waiting = status.waiting("Asking TukUI");
        progress.tick();
        status.clear();
        progress.shared.leave();
        drop(progress);
        let all = written.take();
        assert!(!all.contains("]9;"), "{all:?}");
    }

    #[test]
    fn reduce_motion_keeps_the_count_in_the_bar_and_drops_the_pulse() {
        let (progress, written) = progress(Paint::PLAIN, true, 80);
        let progress = progress.with_bar();
        let status = progress.status();
        // The line stays as it is; the bar still moves, since it is no animation.
        status.show("Checking 1 of 4", "Checking 4 addons", Bar::of(0, 4, false));
        progress.tick();
        assert_eq!(
            written.take(),
            "\x1b]9;4;1;0\x1b\\\r\x1b[KChecking 4 addons…"
        );
        status.show("Checking 3 of 4", "Checking 4 addons", Bar::of(2, 4, false));
        progress.tick();
        assert_eq!(written.take(), "\x1b]9;4;1;50\x1b\\");
        status.clear();
        written.take();

        // The pulse is an animation.
        let _waiting = status.waiting("Asking TukUI");
        progress.tick();
        assert_eq!(written.take(), "\r\x1b[KAsking TukUI…");
    }

    #[test]
    fn a_tally_fills_the_bar_and_marks_trouble() {
        let rows = [
            row("Pawn", "Pawn", SourceKind::Tukui),
            row("Plater", "Plater", SourceKind::Tukui),
            row("ElvUI", "ElvUI", SourceKind::Tukui),
            row("Details", "Details", SourceKind::Tukui),
        ];
        let selected = rows.iter().collect::<Vec<_>>();
        let mut tally = Tally::new("Checking", &selected, true);
        assert_eq!(tally.bar(), Bar::of(0, 4, false));
        tally.started("Pawn");
        tally.finished("Pawn", false);
        assert_eq!(
            tally.bar(),
            Bar::Percent {
                percent: 25,
                trouble: false
            }
        );
        // Yellow from the first addon that failed, to the end.
        tally.finished("Plater", true);
        assert_eq!(
            tally.bar(),
            Bar::Percent {
                percent: 50,
                trouble: true
            }
        );
        tally.finished("ElvUI", false);
        tally.finished("Details", false);
        assert_eq!(
            tally.bar(),
            Bar::Percent {
                percent: 100,
                trouble: true
            }
        );
        // A failure of an addon that is not counted is not this run's trouble.
        let mut other = Tally::new("Checking", &selected, true);
        other.finished("Unknown", true);
        assert_eq!(other.bar(), Bar::of(0, 4, false));
    }
}
