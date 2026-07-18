use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use sysinfo::Signal;

use crate::config::{Config, Theme};
use crate::event::{Event, EventHandler};
use crate::metrics::{Collector, Snapshot};
use crate::ui;

const HISTORY_CAP: usize = 240;

/// Which process column drives the sort order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Memory,
    Pid,
    Name,
}

pub struct App {
    running: bool,
    collector: Collector,
    pub latest: Snapshot,
    pub cpu_history: VecDeque<Vec<f32>>,
    pub sort_key: SortKey,
    pub sort_desc: bool,
    pub selected_pid: Option<u32>,
    pub confirm_kill: Option<u32>,
    pub theme: Theme,
    tick_rate: Duration,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            running: true,
            collector: Collector::new(),
            latest: Snapshot {
                cpu_usage_per_core: Vec::new(),
                memory_used: 0,
                memory_total: 0,
                swap_used: 0,
                swap_total: 0,
                load_avg: Default::default(),
                processes: Vec::new(),
                networks: Vec::new(),
                disks: Vec::new(),
            },
            cpu_history: VecDeque::new(),
            sort_key: SortKey::Cpu,
            sort_desc: true,
            selected_pid: None,
            confirm_kill: None,
            theme: config.theme,
            tick_rate: config.tick_rate,
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        let mut events = EventHandler::new(self.tick_rate);

        while self.running {
            terminal.draw(|frame| ui::draw(frame, &self))?;

            match events.next().await? {
                Event::Tick => {
                    self.latest = self.collector.refresh();
                    self.resort_processes();
                    self.reconcile_selection();
                    self.cpu_history
                        .push_back(self.latest.cpu_usage_per_core.clone());
                    if self.cpu_history.len() > HISTORY_CAP {
                        self.cpu_history.pop_front();
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key.code),
                Event::Key(_) | Event::Resize => {}
            }
        }

        Ok(())
    }

    fn on_key(&mut self, code: KeyCode) {
        if let Some(pid) = self.confirm_kill {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.collector.kill_process(pid, Signal::Term);
                }
                _ => {}
            }
            self.confirm_kill = None;
            return;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Char('c') => self.set_sort(SortKey::Cpu),
            KeyCode::Char('m') => self.set_sort(SortKey::Memory),
            KeyCode::Char('p') => self.set_sort(SortKey::Pid),
            KeyCode::Char('n') => self.set_sort(SortKey::Name),
            KeyCode::Char('x') => self.confirm_kill = self.selected_pid,
            _ => {}
        }
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_key = key;
            self.sort_desc = matches!(key, SortKey::Cpu | SortKey::Memory);
        }
        self.resort_processes();
    }

    fn resort_processes(&mut self) {
        let (key, desc) = (self.sort_key, self.sort_desc);
        self.latest.processes.sort_by(|a, b| {
            let ordering = match key {
                SortKey::Cpu => a.cpu_usage.total_cmp(&b.cpu_usage),
                SortKey::Memory => a.memory.cmp(&b.memory),
                SortKey::Pid => a.pid.cmp(&b.pid),
                SortKey::Name => a.name.cmp(&b.name),
            };
            if desc { ordering.reverse() } else { ordering }
        });
    }

    /// Keeps the selection on a valid process, defaulting to the top row
    /// once the previously selected pid disappears (e.g. it exited).
    fn reconcile_selection(&mut self) {
        let still_present = self
            .selected_pid
            .is_some_and(|pid| self.latest.processes.iter().any(|p| p.pid == pid));
        if !still_present {
            self.selected_pid = self.latest.processes.first().map(|p| p.pid);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.latest.processes.is_empty() {
            return;
        }
        let current = self
            .selected_pid
            .and_then(|pid| self.latest.processes.iter().position(|p| p.pid == pid))
            .unwrap_or(0);
        let len = self.latest.processes.len() as i32;
        let next = (current as i32 + delta).clamp(0, len - 1);
        self.selected_pid = Some(self.latest.processes[next as usize].pid);
    }
}
