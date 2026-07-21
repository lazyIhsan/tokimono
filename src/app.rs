use std::collections::{HashMap, HashSet, VecDeque};
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
    pub tree_view: bool,
    /// Pids with their children folded in tree view. Presence = folded;
    /// absence = expanded, so a newly-spawned child defaults to visible
    /// without any bookkeeping.
    collapsed: HashSet<u32>,
    pub theme: Theme,
    tick_rate: Duration,
}

/// One row of `App::visible_processes`: a process plus its position in the
/// (optional) tree.
pub struct TreeRow<'a> {
    pub process: &'a ProcessInfo,
    pub depth: usize,
    pub has_children: bool,
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
                gpus: Vec::new(),
                gpu_error: None,
            },
            cpu_history: VecDeque::new(),
            sort_key: SortKey::Cpu,
            sort_desc: true,
            selected_pid: None,
            confirm_kill: None,
            filter: String::new(),
            filter_input: None,
            tree_view: false,
            collapsed: HashSet::new(),
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
                    let live_pids: HashSet<u32> =
                        self.latest.processes.iter().map(|p| p.pid).collect();
                    self.collapsed.retain(|pid| live_pids.contains(pid));
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
            KeyCode::Char('t') => self.toggle_tree_view(),
            KeyCode::Left | KeyCode::Char('h') => self.collapse_selected(),
            KeyCode::Right | KeyCode::Char('l') => self.expand_selected(),
            _ => {}
        }
    }

    fn toggle_tree_view(&mut self) {
        self.tree_view = !self.tree_view;
        self.reconcile_selection();
    }

    fn collapse_selected(&mut self) {
        if let Some(pid) = self.selected_pid {
            self.collapsed.insert(pid);
        }
    }

    fn expand_selected(&mut self) {
        if let Some(pid) = self.selected_pid {
            self.collapsed.remove(&pid);
        }
    }

    pub fn is_collapsed(&self, pid: u32) -> bool {
        self.collapsed.contains(&pid)
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
        let visible = self.visible_processes();
        let still_present = self
            .selected_pid
            .is_some_and(|pid| visible.iter().any(|r| r.process.pid == pid));
        if !still_present {
            self.selected_pid = visible.first().map(|r| r.process.pid);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let visible = self.visible_processes();
        if visible.is_empty() {
            return;
        }
        let current = self
            .selected_pid
            .and_then(|pid| visible.iter().position(|r| r.process.pid == pid))
            .unwrap_or(0);
        let len = visible.len() as i32;
        let next = (current as i32 + delta).clamp(0, len - 1);
        self.selected_pid = Some(visible[next as usize].process.pid);
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

    /// Rows to navigate/render: the flat filtered list, unless tree view is
    /// on and no filter is active, in which case it's a depth-first walk of
    /// the parent/child tree respecting folded nodes. An active filter
    /// always falls back to the flat list — matches and non-matching
    /// ancestors don't compose cleanly into a tree, so tree mode simply
    /// suspends itself until the filter is cleared.
    pub fn visible_processes(&self) -> Vec<TreeRow<'_>> {
        if !self.tree_view || !self.filter.is_empty() {
            return self
                .filtered_processes()
                .into_iter()
                .map(|process| TreeRow {
                    process,
                    depth: 0,
                    has_children: false,
                })
                .collect();
        }

        let pids: HashSet<u32> = self.latest.processes.iter().map(|p| p.pid).collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut roots: Vec<u32> = Vec::new();
        for p in &self.latest.processes {
            match p.ppid.filter(|ppid| pids.contains(ppid)) {
                Some(ppid) => children.entry(ppid).or_default().push(p.pid),
                None => roots.push(p.pid),
            }
        }
        let by_pid: HashMap<u32, &ProcessInfo> =
            self.latest.processes.iter().map(|p| (p.pid, p)).collect();

        let mut rows = Vec::with_capacity(self.latest.processes.len());
        for &root in &roots {
            self.push_tree_row(root, 0, &children, &by_pid, &mut rows);
        }
        rows
    }

    fn push_tree_row<'a>(
        &self,
        pid: u32,
        depth: usize,
        children: &HashMap<u32, Vec<u32>>,
        by_pid: &HashMap<u32, &'a ProcessInfo>,
        rows: &mut Vec<TreeRow<'a>>,
    ) {
        let Some(&process) = by_pid.get(&pid) else {
            return;
        };
        let kids = children.get(&pid).map(Vec::as_slice).unwrap_or(&[]);
        rows.push(TreeRow {
            process,
            depth,
            has_children: !kids.is_empty(),
        });
        if !self.collapsed.contains(&pid) {
            for &child in kids {
                self.push_tree_row(child, depth + 1, children, by_pid, rows);
            }
        }
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
            ppid: None,
        }
    }

    fn child(pid: u32, name: &str, ppid: u32) -> ProcessInfo {
        ProcessInfo {
            ppid: Some(ppid),
            ..process(pid, name)
        }
    }

    fn app_with_processes(processes: Vec<ProcessInfo>) -> App {
        let mut app = App::new(Config::default());
        app.latest.processes = processes;
        app
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

    #[test]
    fn flat_mode_ignores_tree_structure() {
        let app = app_with_processes(vec![process(1, "root"), child(2, "child", 1)]);
        let rows = app.visible_processes();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.depth == 0 && !r.has_children));
    }

    #[test]
    fn tree_mode_orders_depth_first_with_correct_depths() {
        let mut app = app_with_processes(vec![
            process(1, "root"),
            child(2, "mid", 1),
            child(3, "leaf", 2),
            process(4, "other-root"),
        ]);
        app.tree_view = true;
        let rows = app.visible_processes();
        let seen: Vec<(u32, usize)> = rows.iter().map(|r| (r.process.pid, r.depth)).collect();
        assert_eq!(seen, vec![(1, 0), (2, 1), (3, 2), (4, 0)]);
        assert!(rows[0].has_children); // root has mid
        assert!(rows[1].has_children); // mid has leaf
        assert!(!rows[2].has_children); // leaf
        assert!(!rows[3].has_children); // other-root
    }

    #[test]
    fn tree_mode_treats_missing_parent_as_root() {
        let mut app = app_with_processes(vec![child(5, "orphan", 999)]);
        app.tree_view = true;
        let rows = app.visible_processes();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn tree_mode_filter_falls_back_to_flat() {
        let mut app = app_with_processes(vec![process(1, "root"), child(2, "child", 1)]);
        app.tree_view = true;
        app.filter = "child".to_string();
        let rows = app.visible_processes();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].process.pid, 2);
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn collapsing_hides_descendants_but_not_self() {
        let mut app = app_with_processes(vec![
            process(1, "root"),
            child(2, "mid", 1),
            child(3, "leaf", 2),
        ]);
        app.tree_view = true;
        app.selected_pid = Some(1);
        app.collapse_selected();
        let rows = app.visible_processes();
        let pids: Vec<u32> = rows.iter().map(|r| r.process.pid).collect();
        assert_eq!(pids, vec![1]);
        assert!(rows[0].has_children);

        app.expand_selected();
        let rows = app.visible_processes();
        let pids: Vec<u32> = rows.iter().map(|r| r.process.pid).collect();
        assert_eq!(pids, vec![1, 2, 3]);
    }
}
