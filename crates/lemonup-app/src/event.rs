use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    /// Text delivered through bracketed paste.
    Paste(String),
}

pub struct EventHandler {
    /// Each event with when it was read from the terminal.
    receiver: mpsc::UnboundedReceiver<(TerminalEvent, Instant)>,
    tick_rate_ms: Arc<AtomicU64>,
    last_read_at: Option<Instant>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let tick_rate_ms = Arc::new(AtomicU64::new(tick_rate.as_millis() as u64));
        let worker_tick_rate_ms = tick_rate_ms.clone();

        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                let tick_rate =
                    Duration::from_millis(worker_tick_rate_ms.load(Ordering::Relaxed).max(1));
                let timeout = tick_rate.saturating_sub(last_tick.elapsed());
                match event::poll(timeout) {
                    Ok(true) => match event::read() {
                        Ok(CrosstermEvent::Key(key)) => {
                            if sender
                                .send((TerminalEvent::Key(key), Instant::now()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(CrosstermEvent::Mouse(mouse)) => {
                            if sender
                                .send((TerminalEvent::Mouse(mouse), Instant::now()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(CrosstermEvent::Resize(width, height)) => {
                            if sender
                                .send((TerminalEvent::Resize(width, height), Instant::now()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(CrosstermEvent::Paste(text)) => {
                            if sender
                                .send((TerminalEvent::Paste(text), Instant::now()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }

                if last_tick.elapsed() >= tick_rate {
                    if sender.send((TerminalEvent::Tick, Instant::now())).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self {
            receiver,
            tick_rate_ms,
            last_read_at: None,
        }
    }

    pub fn set_tick_rate(&self, tick_rate: Duration) {
        self.tick_rate_ms
            .store(tick_rate.as_millis().max(1) as u64, Ordering::Relaxed);
    }

    pub async fn next(&mut self) -> Option<TerminalEvent> {
        let (event, read_at) = self.receiver.recv().await?;
        self.last_read_at = Some(read_at);
        Some(event)
    }

    /// When the event `next` last returned was read from the terminal. A key
    /// can wait in the queue while the app works, so this can be well before
    /// the app handles it.
    pub fn last_read_at(&self) -> Option<Instant> {
        self.last_read_at
    }
}
