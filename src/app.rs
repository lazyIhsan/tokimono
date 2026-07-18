use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::event::{Event, EventHandler};
use crate::metrics::{Collector, Snapshot};
use crate::ui;

const TICK_RATE: Duration = Duration::from_millis(250);
const HISTORY_CAP: usize = 240;

pub struct App {
    running: bool,
    collector: Collector,
    pub latest: Snapshot,
    pub cpu_history: VecDeque<Vec<f32>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            collector: Collector::new(),
            latest: Snapshot {
                cpu_usage_per_core: Vec::new(),
                memory_used: 0,
                memory_total: 0,
            },
            cpu_history: VecDeque::new(),
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        let mut events = EventHandler::new(TICK_RATE);

        while self.running {
            terminal.draw(|frame| ui::draw(frame, &self))?;

            match events.next().await? {
                Event::Tick => {
                    self.latest = self.collector.refresh();
                    self.cpu_history
                        .push_back(self.latest.cpu_usage_per_core.clone());
                    if self.cpu_history.len() > HISTORY_CAP {
                        self.cpu_history.pop_front();
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key.code),
                Event::Key(_) | Event::Resize(..) => {}
            }
        }

        Ok(())
    }

    fn on_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            _ => {}
        }
    }
}
