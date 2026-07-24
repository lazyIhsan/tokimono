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

/// Which layout the process table renders: flat, a parent/child tree, or
/// grouped by cgroup/systemd unit. Cycled with `t`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessView {
    Flat,
    Tree,
    Grouped,
}

impl ProcessView {
    fn next(self) -> Self {
        match self {
            ProcessView::Flat => ProcessView::Tree,
            ProcessView::Tree => ProcessView::Grouped,
            ProcessView::Grouped => ProcessView::Flat,
        }
    }
}

/// Processes with no cgroup label (mostly kernel threads at the cgroup
/// root) are bucketed here in `Grouped` view rather than dropped, so every
/// process stays visible somewhere.
const UNGROUPED_LABEL: &str = "(ungrouped)";

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
    pub view: ProcessView,
    /// Pids with their children folded in tree view. Presence = folded;
    /// absence = expanded, so a newly-spawned child defaults to visible
    /// without any bookkeeping. Not used in `Grouped` view — group headers
    /// aren't foldable (yet).
    collapsed: HashSet<u32>,
    pub theme: Theme,
    tick_rate: Duration,
}

/// One row of `App::visible_processes`.
pub enum ProcessRow<'a> {
    /// A non-selectable group header in `Grouped` view.
    Header {
        label: String,
        count: usize,
        total_cpu: f32,
        total_mem: u64,
    },
    /// A process, plus its position in the (optional) tree/group.
    Process {
        process: &'a ProcessInfo,
        depth: usize,
        has_children: bool,
    },
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
            view: ProcessView::Flat,
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
            KeyCode::Char('t') => self.cycle_view(),
            KeyCode::Left | KeyCode::Char('h') => self.collapse_selected(),
            KeyCode::Right | KeyCode::Char('l') => self.expand_selected(),
            _ => {}
        }
    }

    fn cycle_view(&mut self) {
        self.view = self.view.next();
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
        let selectable = self.selectable_pids();
        let still_present = self
            .selected_pid
            .is_some_and(|pid| selectable.contains(&pid));
        if !still_present {
            self.selected_pid = selectable.first().copied();
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let selectable = self.selectable_pids();
        if selectable.is_empty() {
            return;
        }
        let current = self
            .selected_pid
            .and_then(|pid| selectable.iter().position(|&p| p == pid))
            .unwrap_or(0);
        let len = selectable.len() as i32;
        let next = (current as i32 + delta).clamp(0, len - 1);
        self.selected_pid = Some(selectable[next as usize]);
    }

    /// Pids of just the selectable (`Process`) rows in `visible_processes()`
    /// order, skipping `Header` rows — selection can never land on a header.
    fn selectable_pids(&self) -> Vec<u32> {
        self.visible_processes()
            .into_iter()
            .filter_map(|row| match row {
                ProcessRow::Process { process, .. } => Some(process.pid),
                ProcessRow::Header { .. } => None,
            })
            .collect()
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

    /// Rows to navigate/render. Flat lists the filtered processes as-is;
    /// Tree/Grouped only apply when no filter is active — matches and
    /// non-matching ancestors/groupmates don't compose cleanly with either
    /// structure, so both simply suspend themselves (falling back to flat)
    /// until the filter is cleared.
    pub fn visible_processes(&self) -> Vec<ProcessRow<'_>> {
        if self.view == ProcessView::Flat || !self.filter.is_empty() {
            return self
                .filtered_processes()
                .into_iter()
                .map(|process| ProcessRow::Process {
                    process,
                    depth: 0,
                    has_children: false,
                })
                .collect();
        }

        match self.view {
            ProcessView::Tree => self.tree_rows(),
            ProcessView::Grouped => self.grouped_rows(),
            ProcessView::Flat => unreachable!(),
        }
    }

    fn tree_rows(&self) -> Vec<ProcessRow<'_>> {
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
        rows: &mut Vec<ProcessRow<'a>>,
    ) {
        let Some(&process) = by_pid.get(&pid) else {
            return;
        };
        let kids = children.get(&pid).map(Vec::as_slice).unwrap_or(&[]);
        rows.push(ProcessRow::Process {
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

    /// Buckets processes by cgroup/systemd-unit label (root-cgroup
    /// processes go in `UNGROUPED_LABEL` rather than vanishing), sums each
    /// group's CPU/memory, and orders groups by total CPU descending — the
    /// one generally-useful default ("what's actually costing me
    /// something"), independent of whatever `sort_key` the table is set
    /// to. Members within a group keep the table's existing sort order
    /// (the process list is already sorted before grouping, same
    /// "sibling order comes free" trick the tree view uses).
    fn grouped_rows(&self) -> Vec<ProcessRow<'_>> {
        let mut order: Vec<&str> = Vec::new();
        let mut groups: HashMap<&str, Vec<&ProcessInfo>> = HashMap::new();
        for p in &self.latest.processes {
            let label = p.group.as_deref().unwrap_or(UNGROUPED_LABEL);
            groups
                .entry(label)
                .or_insert_with(|| {
                    order.push(label);
                    Vec::new()
                })
                .push(p);
        }

        order.sort_by(|a, b| {
            let total =
                |label: &str| -> f32 { groups[label].iter().map(|p| p.cpu_usage).sum::<f32>() };
            total(b).total_cmp(&total(a))
        });

        let mut rows = Vec::with_capacity(self.latest.processes.len() + order.len());
        for label in order {
            let members = &groups[label];
            rows.push(ProcessRow::Header {
                label: label.to_string(),
                count: members.len(),
                total_cpu: members.iter().map(|p| p.cpu_usage).sum(),
                total_mem: members.iter().map(|p| p.memory).sum(),
            });
            for &process in members {
                rows.push(ProcessRow::Process {
                    process,
                    depth: 1,
                    has_children: false,
                });
            }
        }
        rows
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
            group: None,
        }
    }

    fn child(pid: u32, name: &str, ppid: u32) -> ProcessInfo {
        ProcessInfo {
            ppid: Some(ppid),
            ..process(pid, name)
        }
    }

    fn grouped(pid: u32, name: &str, group: &str, cpu_usage: f32) -> ProcessInfo {
        ProcessInfo {
            cpu_usage,
            group: Some(group.to_string()),
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

    /// Pulls `(pid, depth, has_children)` out of the `Process` rows only,
    /// panicking if a `Header` row shows up unexpectedly.
    fn process_rows(rows: &[ProcessRow<'_>]) -> Vec<(u32, usize, bool)> {
        rows.iter()
            .map(|r| match r {
                ProcessRow::Process {
                    process,
                    depth,
                    has_children,
                } => (process.pid, *depth, *has_children),
                ProcessRow::Header { .. } => panic!("unexpected header row"),
            })
            .collect()
    }

    #[test]
    fn flat_mode_ignores_tree_structure() {
        let app = app_with_processes(vec![process(1, "root"), child(2, "child", 1)]);
        let rows = process_rows(&app.visible_processes());
        assert_eq!(rows, vec![(1, 0, false), (2, 0, false)]);
    }

    #[test]
    fn tree_mode_orders_depth_first_with_correct_depths() {
        let mut app = app_with_processes(vec![
            process(1, "root"),
            child(2, "mid", 1),
            child(3, "leaf", 2),
            process(4, "other-root"),
        ]);
        app.view = ProcessView::Tree;
        let rows = process_rows(&app.visible_processes());
        assert_eq!(
            rows,
            vec![
                (1, 0, true),  // root has mid
                (2, 1, true),  // mid has leaf
                (3, 2, false), // leaf
                (4, 0, false), // other-root
            ]
        );
    }

    #[test]
    fn tree_mode_treats_missing_parent_as_root() {
        let mut app = app_with_processes(vec![child(5, "orphan", 999)]);
        app.view = ProcessView::Tree;
        let rows = process_rows(&app.visible_processes());
        assert_eq!(rows, vec![(5, 0, false)]);
    }

    #[test]
    fn tree_mode_filter_falls_back_to_flat() {
        let mut app = app_with_processes(vec![process(1, "root"), child(2, "child", 1)]);
        app.view = ProcessView::Tree;
        app.filter = "child".to_string();
        let rows = process_rows(&app.visible_processes());
        assert_eq!(rows, vec![(2, 0, false)]);
    }

    #[test]
    fn collapsing_hides_descendants_but_not_self() {
        let mut app = app_with_processes(vec![
            process(1, "root"),
            child(2, "mid", 1),
            child(3, "leaf", 2),
        ]);
        app.view = ProcessView::Tree;
        app.selected_pid = Some(1);
        app.collapse_selected();
        let rows = process_rows(&app.visible_processes());
        assert_eq!(rows, vec![(1, 0, true)]);

        app.expand_selected();
        let pids: Vec<u32> = process_rows(&app.visible_processes())
            .into_iter()
            .map(|(pid, ..)| pid)
            .collect();
        assert_eq!(pids, vec![1, 2, 3]);
    }

    #[test]
    fn grouped_mode_orders_groups_by_total_cpu_descending() {
        let mut app = app_with_processes(vec![
            grouped(1, "a", "low.service", 1.0),
            grouped(2, "b", "high.service", 10.0),
            grouped(3, "c", "high.service", 5.0),
        ]);
        app.view = ProcessView::Grouped;
        let rows = app.visible_processes();
        let labels: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                ProcessRow::Header { label, .. } => Some(label.clone()),
                ProcessRow::Process { .. } => None,
            })
            .collect();
        assert_eq!(labels, vec!["high.service", "low.service"]);

        let high_header = rows
            .iter()
            .find_map(|r| match r {
                ProcessRow::Header {
                    label,
                    count,
                    total_cpu,
                    ..
                } if label == "high.service" => Some((*count, *total_cpu)),
                _ => None,
            })
            .unwrap();
        assert_eq!(high_header, (2, 15.0));
    }

    #[test]
    fn grouped_mode_buckets_ungrouped_processes_together() {
        let mut app = app_with_processes(vec![process(1, "kworker"), process(2, "kthreadd")]);
        app.view = ProcessView::Grouped;
        let rows = app.visible_processes();
        assert_eq!(rows.len(), 3); // 1 header + 2 members
        assert!(matches!(
            &rows[0],
            ProcessRow::Header { label, count: 2, .. } if label == UNGROUPED_LABEL
        ));
    }

    #[test]
    fn grouped_mode_selection_never_lands_on_a_header() {
        let mut app = app_with_processes(vec![grouped(1, "a", "svc.service", 0.0)]);
        app.view = ProcessView::Grouped;
        app.reconcile_selection();
        assert_eq!(app.selected_pid, Some(1));
    }

    #[test]
    fn grouped_mode_filter_falls_back_to_flat() {
        let mut app = app_with_processes(vec![grouped(1, "a", "svc.service", 0.0)]);
        app.view = ProcessView::Grouped;
        app.filter = "a".to_string();
        let rows = app.visible_processes();
        assert!(matches!(rows.as_slice(), [ProcessRow::Process { .. }]));
    }
}
