use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};

pub enum TuiEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
}

pub struct EventHandler {
    rx: tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
    _thread: std::thread::JoinHandle<()>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let thread = std::thread::spawn(move || loop {
            if event::poll(tick_rate).unwrap_or(false) {
                match event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        if tx.send(TuiEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Resize(w, h)) => {
                        if tx.send(TuiEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            } else if tx.send(TuiEvent::Tick).is_err() {
                break;
            }
        });
        Self {
            rx,
            _thread: thread,
        }
    }

    pub async fn next(&mut self) -> Option<TuiEvent> {
        self.rx.recv().await
    }
}
