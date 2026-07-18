use std::time::Instant;

use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, Signal, System};

/// A single process's stats for one refresh cycle.
#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
}

/// A network interface's throughput, in bytes/sec, since the last refresh.
pub struct NetworkInfo {
    pub name: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
}

/// A mounted disk's capacity and I/O throughput (bytes/sec) since the last refresh.
pub struct DiskInfo {
    pub mount_point: String,
    pub total_space: u64,
    pub available_space: u64,
    pub read_rate: f64,
    pub write_rate: f64,
}

/// A snapshot of system metrics for one refresh cycle.
pub struct Snapshot {
    pub cpu_usage_per_core: Vec<f32>,
    pub memory_used: u64,
    pub memory_total: u64,
    pub processes: Vec<ProcessInfo>,
    pub networks: Vec<NetworkInfo>,
    pub disks: Vec<DiskInfo>,
}

/// Owns the sysinfo handles and refreshes them on demand.
pub struct Collector {
    system: System,
    networks: Networks,
    disks: Disks,
    last_refresh: Instant,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            last_refresh: Instant::now(),
        }
    }

    pub fn refresh(&mut self) -> Snapshot {
        // Bytes-since-last-refresh counters need real elapsed time (not the
        // configured tick rate) to convert into an accurate bytes/sec rate.
        let elapsed_secs = self.last_refresh.elapsed().as_secs_f64().max(0.001);
        self.last_refresh = Instant::now();

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

        self.networks.refresh(true);
        let mut networks: Vec<NetworkInfo> = self
            .networks
            .iter()
            .map(|(name, data)| NetworkInfo {
                name: name.clone(),
                rx_rate: data.received() as f64 / elapsed_secs,
                tx_rate: data.transmitted() as f64 / elapsed_secs,
            })
            .collect();
        networks.sort_by(|a, b| a.name.cmp(&b.name));

        self.disks.refresh(true);
        let mut disks: Vec<DiskInfo> = self
            .disks
            .list()
            .iter()
            .map(|disk| {
                let usage = disk.usage();
                DiskInfo {
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    total_space: disk.total_space(),
                    available_space: disk.available_space(),
                    read_rate: usage.read_bytes as f64 / elapsed_secs,
                    write_rate: usage.written_bytes as f64 / elapsed_secs,
                }
            })
            .collect();
        disks.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));

        Snapshot {
            cpu_usage_per_core: self.system.cpus().iter().map(|c| c.cpu_usage()).collect(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
            processes,
            networks,
            disks,
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
