use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent};
use futures_util::StreamExt;
use tokio::time::{Interval, interval};

/// Events consumed by the app's main loop.
pub enum Event {
    /// A fixed-rate tick, used to refresh metrics.
    Tick,
    /// A key press from the terminal.
    Key(KeyEvent),
    /// The terminal window was resized.
    Resize(u16, u16),
}

/// Merges terminal input and a refresh timer into a single event stream.
pub struct EventHandler {
    reader: EventStream,
    tick: Interval,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        Self {
            reader: EventStream::new(),
            tick: interval(tick_rate),
        }
    }

    /// Waits for the next event, whichever comes first: a key press,
    /// a resize, or the next tick.
    pub async fn next(&mut self) -> color_eyre::Result<Event> {
        tokio::select! {
            _ = self.tick.tick() => Ok(Event::Tick),
            maybe_event = self.reader.next() => match maybe_event {
                Some(Ok(CrosstermEvent::Key(key))) => Ok(Event::Key(key)),
                Some(Ok(CrosstermEvent::Resize(w, h))) => Ok(Event::Resize(w, h)),
                Some(Ok(_)) => Ok(Event::Tick),
                Some(Err(err)) => Err(err.into()),
                None => Ok(Event::Tick),
            },
        }
    }
}
