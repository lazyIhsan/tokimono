use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

/// A single process's stats for one refresh cycle.
#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
}

/// A snapshot of system metrics for one refresh cycle.
pub struct Snapshot {
    pub cpu_usage_per_core: Vec<f32>,
    pub memory_used: u64,
    pub memory_total: u64,
    pub processes: Vec<ProcessInfo>,
}

/// Owns the sysinfo handle and refreshes it on demand.
pub struct Collector {
    system: System,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
        }
    }

    pub fn refresh(&mut self) -> Snapshot {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes(ProcessesToUpdate::All, true);

        let processes = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessInfo {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                cpu_usage: process.cpu_usage(),
                memory: process.memory(),
            })
            .collect();

        Snapshot {
            cpu_usage_per_core: self.system.cpus().iter().map(|c| c.cpu_usage()).collect(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
            processes,
        }
    }

    /// Sends `signal` to the given process. Returns `false` if the process
    /// no longer exists, the signal isn't supported on this platform, or
    /// sending it failed (e.g. insufficient permissions).
    pub fn kill_process(&self, pid: u32, signal: Signal) -> bool {
        match self.system.process(Pid::from_u32(pid)) {
            Some(process) => process.kill_with(signal).unwrap_or(false),
            None => false,
        }
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}
