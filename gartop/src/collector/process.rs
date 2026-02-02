//! Process information collection from /proc/[pid]/

use crate::error::{Error, Result};
use gartop_ipc::{ProcessInfo, SortField};
use procfs::process::{all_processes, Process};
use procfs::{Current, CurrentSI};
use std::collections::HashMap;

/// Process collector with previous CPU times for percentage calculation.
pub struct ProcessCollector {
    /// Previous CPU times per PID for delta calculation.
    prev_times: HashMap<i32, (u64, u64)>, // (utime + stime, total_system_time)
    /// Total memory for percentage calculation.
    total_memory: u64,
}

impl ProcessCollector {
    /// Create a new process collector.
    pub fn new() -> Result<Self> {
        let meminfo = procfs::Meminfo::current()?;

        Ok(Self {
            prev_times: HashMap::new(),
            total_memory: meminfo.mem_total,
        })
    }

    /// Collect all running processes.
    pub fn collect(
        &mut self,
        sort_by: SortField,
        limit: Option<usize>,
    ) -> Result<Vec<ProcessInfo>> {
        let kernel_stats = procfs::KernelStats::current()?;
        let total = &kernel_stats.total;
        let system_total = total.user
            + total.nice
            + total.system
            + total.idle
            + total.iowait.unwrap_or(0)
            + total.irq.unwrap_or(0)
            + total.softirq.unwrap_or(0)
            + total.steal.unwrap_or(0);

        let mut processes = Vec::new();
        let mut current_pids = Vec::new();

        for proc_result in all_processes()? {
            let proc = match proc_result {
                Ok(p) => p,
                Err(_) => continue,
            };

            let pid = proc.pid();
            current_pids.push(pid);

            match self.process_info(&proc, system_total) {
                Ok(info) => processes.push(info),
                Err(_) => continue, // Skip processes we can't read
            }
        }

        // Clean up old entries
        self.prev_times.retain(|pid, _| current_pids.contains(pid));

        // Sort
        match sort_by {
            SortField::Cpu => processes.sort_by(|a, b| {
                b.cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::Memory => processes.sort_by(|a, b| {
                b.memory_percent
                    .partial_cmp(&a.memory_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::Pid => processes.sort_by_key(|p| p.pid),
            SortField::Name => processes.sort_by(|a, b| a.name.cmp(&b.name)),
        }

        // Limit
        if let Some(n) = limit {
            processes.truncate(n);
        }

        Ok(processes)
    }

    /// Get info for a single process.
    fn process_info(&mut self, proc: &Process, system_total: u64) -> Result<ProcessInfo> {
        let stat = proc.stat()?;
        let status = proc.status()?;

        let pid = proc.pid();
        let name = stat.comm.clone();
        let cmdline = proc
            .cmdline()
            .map(|v| v.join(" "))
            .unwrap_or_else(|_| name.clone());

        // Calculate CPU percentage
        let proc_time = stat.utime + stat.stime;
        let cpu_percent =
            if let Some((prev_proc_time, prev_sys_time)) = self.prev_times.get(&pid) {
                let proc_delta = proc_time.saturating_sub(*prev_proc_time);
                let sys_delta = system_total.saturating_sub(*prev_sys_time);
                if sys_delta > 0 {
                    (proc_delta as f64 / sys_delta as f64) * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };
        self.prev_times.insert(pid, (proc_time, system_total));

        // Memory
        let rss = stat.rss as u64 * 4096; // Pages to bytes
        let vsize = stat.vsize;
        let memory_percent = if self.total_memory > 0 {
            (rss as f64 / self.total_memory as f64) * 100.0
        } else {
            0.0
        };

        // State
        let state = match stat.state {
            'R' => "running",
            'S' => "sleeping",
            'D' => "disk sleep",
            'Z' => "zombie",
            'T' => "stopped",
            't' => "tracing stop",
            'X' | 'x' => "dead",
            _ => "unknown",
        }
        .to_string();

        // User
        let user = users::get_user_by_uid(status.ruid)
            .map(|u| u.name().to_string_lossy().to_string())
            .unwrap_or_else(|| status.ruid.to_string());

        Ok(ProcessInfo {
            pid,
            name,
            cmdline,
            cpu_percent,
            memory_percent,
            rss,
            vsize,
            state,
            user,
        })
    }

    /// Kill a process by PID.
    pub fn kill(&self, pid: i32, signal: i32) -> Result<()> {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        let sig = Signal::try_from(signal)
            .map_err(|_| Error::Ipc(format!("Invalid signal: {}", signal)))?;

        kill(Pid::from_raw(pid), sig).map_err(|e| match e {
            nix::errno::Errno::ESRCH => Error::ProcessNotFound(pid),
            nix::errno::Errno::EPERM => {
                Error::PermissionDenied(format!("Cannot kill PID {}", pid))
            }
            _ => Error::Io(std::io::Error::from_raw_os_error(e as i32)),
        })
    }
}
