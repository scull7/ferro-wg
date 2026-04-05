//! Async event handler — terminal input and periodic ticks.
//!
//! Uses [`crossterm::event::EventStream`] combined with
//! [`tokio::time::interval`] via `tokio::select!` to provide a fully
//! async, non-blocking event source with no busy-waiting or executor
//! starvation.

use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time;
use tracing::warn;

/// Errors produced by the event layer.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// A crossterm I/O error occurred while reading a terminal event.
    #[error("terminal event error: {0}")]
    Crossterm(#[from] std::io::Error),
    /// The event channel was closed (receiver dropped).
    #[error("event channel closed")]
    ChannelClosed,
}

/// Events dispatched by the [`EventHandler`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// A mouse event occurred.
    Mouse(MouseEvent),
    /// A periodic tick (used for UI refresh and animation).
    Tick,
}

/// Spawns a background task that reads terminal events asynchronously
/// and forwards them through an unbounded channel.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler with the given tick interval.
    ///
    /// Spawns the polling task as an action at the boundary; the
    /// returned [`EventHandler`] is a pure receiver.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if let Err(e) = Self::run(tx, tick_rate).await {
                // ChannelClosed is normal shutdown; everything else is unexpected.
                if !matches!(e, EventError::ChannelClosed) {
                    warn!("event loop exited with error: {e}");
                }
            }
        });
        Self { rx }
    }

    /// Core async event loop: combines `EventStream` and a ticker via
    /// `select!` with no blocking calls or busy-waiting.
    ///
    /// Returns `Ok(())` when the stream ends cleanly, or an
    /// [`EventError`] on fatal crossterm or channel errors.
    async fn run(
        tx: mpsc::UnboundedSender<AppEvent>,
        tick_rate: Duration,
    ) -> Result<(), EventError> {
        let mut stream = EventStream::new();
        let mut ticker = time::interval(tick_rate);

        loop {
            tokio::select! {
                biased; // prioritise events over ticks for responsiveness

                maybe_event = stream.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) => {
                            tx.send(AppEvent::Key(key))
                                .map_err(|_| EventError::ChannelClosed)?;
                        }
                        Some(Ok(Event::Mouse(mouse))) => {
                            tx.send(AppEvent::Mouse(mouse))
                                .map_err(|_| EventError::ChannelClosed)?;
                        }
                        Some(Ok(_)) => {} // ignore resize, etc.
                        Some(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => {} // EINTR — retry
                        Some(Err(e)) => return Err(EventError::Crossterm(e)),
                        None => return Ok(()), // stream ended cleanly
                    }
                }

                _ = ticker.tick() => {
                    tx.send(AppEvent::Tick)
                        .map_err(|_| EventError::ChannelClosed)?;
                }
            }
        }
    }

    /// Wait for the next event.
    ///
    /// Returns `None` when the background task has exited (e.g. on
    /// fatal terminal error or receiver drop).
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::sleep;

    use super::{AppEvent, EventHandler};

    #[tokio::test]
    #[ignore = "requires an attached terminal (EventStream panics without one)"]
    async fn tick_events_are_delivered() {
        let mut handler = EventHandler::new(Duration::from_millis(30));

        // Allow at least one tick to fire.
        sleep(Duration::from_millis(60)).await;

        let event = handler.next().await;
        assert!(
            matches!(event, Some(AppEvent::Tick)),
            "expected a Tick event, got {event:?}"
        );
    }
}
