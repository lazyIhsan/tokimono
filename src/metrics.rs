use sysinfo::System;

/// A snapshot of system metrics for one refresh cycle.
pub struct Snapshot {
    pub cpu_usage_per_core: Vec<f32>,
    pub memory_used: u64,
    pub memory_total: u64,
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

        Snapshot {
            cpu_usage_per_core: self.system.cpus().iter().map(|c| c.cpu_usage()).collect(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
        }
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}
