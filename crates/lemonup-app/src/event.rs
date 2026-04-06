use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub enum TerminalEvent {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
}

pub struct EventHandler {
    receiver: mpsc::UnboundedReceiver<TerminalEvent>,
    tick_rate_ms: Arc<AtomicU64>,
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
                            if sender.send(TerminalEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(CrosstermEvent::Mouse(mouse)) => {
                            if sender.send(TerminalEvent::Mouse(mouse)).is_err() {
                                break;
                            }
                        }
                        Ok(CrosstermEvent::Resize(width, height)) => {
                            if sender.send(TerminalEvent::Resize(width, height)).is_err() {
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
                    if sender.send(TerminalEvent::Tick).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self {
            receiver,
            tick_rate_ms,
        }
    }

    pub fn set_tick_rate(&self, tick_rate: Duration) {
        self.tick_rate_ms
            .store(tick_rate.as_millis().max(1) as u64, Ordering::Relaxed);
    }

    pub async fn next(&mut self) -> Option<TerminalEvent> {
        self.receiver.recv().await
    }
}
