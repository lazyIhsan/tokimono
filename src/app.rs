use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use sysinfo::Signal;

use crate::config::{Config, Theme};
use crate::event::{Event, EventHandler};
use crate::metrics::{Collector, ProcessInfo, Snapshot};
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
    /// Committed/active process filter. Empty means no filter.
    pub filter: String,
    /// `Some(buf)` while the filter text box is being edited.
    pub filter_input: Option<String>,
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
                cpu_temp: None,
                processes: Vec::new(),
                networks: Vec::new(),
                disks: Vec::new(),
            },
            cpu_history: VecDeque::new(),
            sort_key: SortKey::Cpu,
            sort_desc: true,
            selected_pid: None,
            confirm_kill: None,
            filter: String::new(),
            filter_input: None,
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

        if self.filter_input.is_some() {
            match code {
                KeyCode::Enter => {
                    self.filter = self.filter_input.take().unwrap_or_default();
                    self.reconcile_selection();
                }
                KeyCode::Esc => self.filter_input = None,
                KeyCode::Backspace => {
                    if let Some(buf) = &mut self.filter_input {
                        buf.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(buf) = &mut self.filter_input {
                        buf.push(c);
                    }
                }
                _ => {}
            }
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
            KeyCode::Char('/') => self.filter_input = Some(self.filter.clone()),
            KeyCode::Char('[') => self.renice_selected(-1),
            KeyCode::Char(']') => self.renice_selected(1),
            _ => {}
        }
    }

    fn renice_selected(&mut self, delta: i32) {
        if let Some(pid) = self.selected_pid {
            self.collector.renice_process(pid, delta);
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

    /// Keeps the selection on a valid, currently-visible process, defaulting
    /// to the top row once the previous selection disappears (killed, or
    /// filtered out).
    fn reconcile_selection(&mut self) {
        let visible = self.filtered_processes();
        let still_present = self
            .selected_pid
            .is_some_and(|pid| visible.iter().any(|p| p.pid == pid));
        if !still_present {
            self.selected_pid = visible.first().map(|p| p.pid);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let visible = self.filtered_processes();
        if visible.is_empty() {
            return;
        }
        let current = self
            .selected_pid
            .and_then(|pid| visible.iter().position(|p| p.pid == pid))
            .unwrap_or(0);
        let len = visible.len() as i32;
        let next = (current as i32 + delta).clamp(0, len - 1);
        self.selected_pid = Some(visible[next as usize].pid);
    }

    /// Processes matching the active filter, in current sort order. Empty
    /// filter matches everything.
    pub fn filtered_processes(&self) -> Vec<&ProcessInfo> {
        self.latest
            .processes
            .iter()
            .filter(|p| process_matches(p, &self.filter))
            .collect()
    }
}

/// Case-insensitive substring match on name, or substring match on pid.
fn process_matches(process: &ProcessInfo, filter: &str) -> bool {
    filter.is_empty()
        || process.name.to_lowercase().contains(&filter.to_lowercase())
        || process.pid.to_string().contains(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cpu_usage: 0.0,
            memory: 0,
            nice: None,
        }
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(process_matches(&process(1, "firefox"), ""));
    }

    #[test]
    fn name_match_is_case_insensitive_substring() {
        assert!(process_matches(&process(1, "Firefox"), "fire"));
        assert!(process_matches(&process(1, "Firefox"), "FIRE"));
        assert!(!process_matches(&process(1, "Firefox"), "chrome"));
    }

    #[test]
    fn pid_match_is_substring() {
        assert!(process_matches(&process(12345, "sh"), "234"));
        assert!(!process_matches(&process(12345, "sh"), "999"));
    }
}
