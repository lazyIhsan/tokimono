use std::time::Instant;

use sysinfo::{
    Components, Disks, LoadAvg, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, Signal,
    System,
};

/// We only ever read pid/name/cpu/memory per process. sysinfo's
/// `refresh_processes` convenience method fetches a lot more than that by
/// default — notably per-thread task info, which on Linux means walking
/// `/proc/<pid>/task/<tid>/` for every thread of every process. On a system
/// with hundreds of processes (some heavily multi-threaded, e.g. browsers)
/// that dwarfs everything else this collector does, so we ask for exactly
/// what we use instead.
///
/// `ProcessRefreshKind::nothing()` still defaults `tasks` to `true` (that's
/// sysinfo's own choice, not an oversight) so `.without_tasks()` is required
/// even after starting from `nothing()`.
fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .without_tasks()
}

/// Reads a process's current nice value via `getpriority(2)`. sysinfo exposes
/// no nice/priority API at all, so this goes straight to libc. `getpriority`
/// can legitimately return `-1` as a valid nice value, so errno has to be
/// cleared and checked to tell that apart from a failed lookup (e.g. the
/// process exited between listing and this call).
fn read_nice(pid: u32) -> Option<i32> {
    unsafe {
        *libc::__errno_location() = 0;
        let value = libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t);
        (!(value == -1 && *libc::__errno_location() != 0)).then_some(value)
    }
}

/// A single process's stats for one refresh cycle.
#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
    /// `None` if the nice value couldn't be read (e.g. the process exited
    /// between listing and the read).
    pub nice: Option<i32>,
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
    pub swap_used: u64,
    pub swap_total: u64,
    pub load_avg: LoadAvg,
    /// Hottest reading across all temperature sensors, in °C. `None` when
    /// the system exposes no sensors (common in VMs/containers).
    pub cpu_temp: Option<f32>,
    pub processes: Vec<ProcessInfo>,
    pub networks: Vec<NetworkInfo>,
    pub disks: Vec<DiskInfo>,
}

/// Owns the sysinfo handles and refreshes them on demand.
pub struct Collector {
    system: System,
    networks: Networks,
    disks: Disks,
    components: Components,
    last_refresh: Instant,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            // `System::new_all()` would do a one-time full refresh here,
            // including the same expensive per-thread task walk `refresh()`
            // below avoids on every tick. Nothing reads this before the
            // first tick's `refresh()` call populates it anyway.
            system: System::new(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            components: Components::new_with_refreshed_list(),
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
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_refresh_kind(),
        );

        let processes = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessInfo {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                cpu_usage: process.cpu_usage(),
                memory: process.memory(),
                nice: read_nice(pid.as_u32()),
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

        self.components.refresh(true);
        let cpu_temp = self
            .components
            .iter()
            .filter_map(|c| c.temperature())
            .fold(None, |hottest: Option<f32>, t| {
                Some(hottest.map_or(t, |h| h.max(t)))
            });

        Snapshot {
            cpu_usage_per_core: self.system.cpus().iter().map(|c| c.cpu_usage()).collect(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
            swap_used: self.system.used_swap(),
            swap_total: self.system.total_swap(),
            load_avg: System::load_average(),
            cpu_temp,
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

    /// Adjusts a process's nice value by `delta`, clamped to the valid
    /// range. Returns `false` if the current value couldn't be read or the
    /// change was rejected (e.g. lowering nice/raising priority without
    /// `CAP_SYS_NICE` — the same permission caveat as `kill_process`).
    pub fn renice_process(&self, pid: u32, delta: i32) -> bool {
        match read_nice(pid) {
            Some(current) => {
                let new_nice = (current + delta).clamp(-20, 19);
                unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, new_nice) == 0 }
            }
            None => false,
        }
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}
