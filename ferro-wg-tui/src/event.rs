//! Async event handler — polls terminal input and emits tick events.

use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

/// Events dispatched by the [`EventHandler`].
#[derive(Debug)]
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// A periodic tick (used for refresh and animation).
    Tick,
}

/// Spawns a background task that reads terminal events and sends
/// them through an async channel.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler with the given tick interval.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                // Poll crossterm for input, with timeout = tick_rate.
                let has_event =
                    tokio::task::block_in_place(|| event::poll(tick_rate).unwrap_or(false));

                if has_event {
                    if let Ok(Event::Key(key)) = event::read()
                        && tx.send(AppEvent::Key(key)).is_err()
                    {
                        break;
                    }
                } else if tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        });

        Self { rx }
    }

    /// Wait for the next event.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}
