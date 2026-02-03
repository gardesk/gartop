//! Process information collection from /proc/[pid]/

use crate::collector::SocketCollector;
use crate::error::{Error, Result};
use gartop_ipc::{ProcessInfo, SortField};
use procfs::process::{all_processes, FDTarget, Process};
use procfs::{Current, CurrentSI};
use std::collections::HashMap;
use std::time::Instant;

/// Previous process stats for delta calculations.
struct PrevProcessStats {
    /// Previous CPU time (utime + stime).
    cpu_time: u64,
    /// Previous system total CPU time.
    system_time: u64,
    /// Previous I/O read bytes.
    io_read_bytes: u64,
    /// Previous I/O write bytes.
    io_write_bytes: u64,
    /// Timestamp of previous sample.
    timestamp: Instant,
}

/// Process collector with previous stats for rate calculations.
pub struct ProcessCollector {
    /// Previous stats per PID for delta calculation.
    prev_stats: HashMap<i32, PrevProcessStats>,
    /// Total memory for percentage calculation.
    total_memory: u64,
    /// Socket collector for network stats.
    socket_collector: SocketCollector,
}

impl ProcessCollector {
    /// Create a new process collector.
    pub fn new() -> Result<Self> {
        let meminfo = procfs::Meminfo::current()?;

        Ok(Self {
            prev_stats: HashMap::new(),
            total_memory: meminfo.mem_total,
            socket_collector: SocketCollector::new(),
        })
    }

    /// Collect all running processes.
    pub fn collect(
        &mut self,
        sort_by: SortField,
        limit: Option<usize>,
    ) -> Result<Vec<ProcessInfo>> {
        let now = Instant::now();

        // Refresh socket cache for network stats
        self.socket_collector.refresh();

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

            match self.process_info(&proc, system_total, now) {
                Ok(info) => processes.push(info),
                Err(_) => continue, // Skip processes we can't read
            }
        }

        // Clean up old entries
        self.prev_stats.retain(|pid, _| current_pids.contains(pid));

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
            SortField::DiskRead => processes.sort_by(|a, b| {
                b.io_read_rate
                    .partial_cmp(&a.io_read_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::DiskWrite => processes.sort_by(|a, b| {
                b.io_write_rate
                    .partial_cmp(&a.io_write_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::DiskTotal => processes.sort_by(|a, b| {
                let a_total = a.io_read_rate + a.io_write_rate;
                let b_total = b.io_read_rate + b.io_write_rate;
                b_total
                    .partial_cmp(&a_total)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::NetConnections => processes.sort_by(|a, b| {
                b.net_connections.cmp(&a.net_connections)
            }),
            SortField::NetTcp => processes.sort_by(|a, b| {
                b.net_tcp.cmp(&a.net_tcp)
            }),
            SortField::NetBandwidth => processes.sort_by(|a, b| {
                let a_total = a.net_rx_rate + a.net_tx_rate;
                let b_total = b.net_rx_rate + b.net_tx_rate;
                b_total.partial_cmp(&a_total).unwrap_or(std::cmp::Ordering::Equal)
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
    fn process_info(&mut self, proc: &Process, system_total: u64, now: Instant) -> Result<ProcessInfo> {
        let stat = proc.stat()?;
        let status = proc.status()?;

        let pid = proc.pid();
        let name = stat.comm.clone();
        let cmdline = proc
            .cmdline()
            .map(|v| v.join(" "))
            .unwrap_or_else(|_| name.clone());

        // Get I/O stats (may fail for some processes due to permissions)
        let (io_read_bytes, io_write_bytes) = proc
            .io()
            .map(|io| (io.read_bytes, io.write_bytes))
            .unwrap_or((0, 0));

        // Calculate CPU percentage and I/O rates from previous stats
        let proc_time = stat.utime + stat.stime;
        let (cpu_percent, io_read_rate, io_write_rate) =
            if let Some(prev) = self.prev_stats.get(&pid) {
                let elapsed = now.duration_since(prev.timestamp).as_secs_f64();

                // CPU percentage
                let proc_delta = proc_time.saturating_sub(prev.cpu_time);
                let sys_delta = system_total.saturating_sub(prev.system_time);
                let cpu_pct = if sys_delta > 0 {
                    (proc_delta as f64 / sys_delta as f64) * 100.0
                } else {
                    0.0
                };

                // I/O rates
                let (read_rate, write_rate) = if elapsed > 0.0 {
                    let read_delta = io_read_bytes.saturating_sub(prev.io_read_bytes) as f64;
                    let write_delta = io_write_bytes.saturating_sub(prev.io_write_bytes) as f64;
                    (read_delta / elapsed, write_delta / elapsed)
                } else {
                    (0.0, 0.0)
                };

                (cpu_pct, read_rate, write_rate)
            } else {
                (0.0, 0.0, 0.0)
            };

        // Store current stats for next calculation
        self.prev_stats.insert(pid, PrevProcessStats {
            cpu_time: proc_time,
            system_time: system_total,
            io_read_bytes,
            io_write_bytes,
            timestamp: now,
        });

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

        // Count network connections (sockets) and collect socket inodes
        let mut net_connections = 0u32;
        let mut socket_inodes = Vec::new();
        if let Ok(fds) = proc.fd() {
            for fd in fds.filter_map(|fd| fd.ok()) {
                if let FDTarget::Socket(inode) = fd.target {
                    net_connections += 1;
                    socket_inodes.push(inode);
                }
            }
        }

        // Get detailed network stats from socket collector
        let net_stats = self.socket_collector.get_process_stats(&socket_inodes);
        let (net_rx_rate, net_tx_rate) = self.socket_collector.get_process_bandwidth(&socket_inodes);
        let net_tcp = net_stats.tcp_count;
        let net_udp = net_stats.udp_count;
        let net_listen = net_stats.listen_count;
        let net_established = net_stats.established_count;

        Ok(ProcessInfo {
            pid,
            ppid: stat.ppid,
            name,
            cmdline,
            cpu_percent,
            memory_percent,
            rss,
            vsize,
            io_read_bytes,
            io_write_bytes,
            io_read_rate,
            io_write_rate,
            net_connections,
            net_tcp,
            net_udp,
            net_listen,
            net_established,
            net_rx_rate,
            net_tx_rate,
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
