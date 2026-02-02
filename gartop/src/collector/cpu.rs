//! CPU statistics collection from /proc/stat

use crate::error::Result;
use gartop_ipc::CpuStats;
use procfs::CurrentSI;
use std::time::{SystemTime, UNIX_EPOCH};

/// CPU times for calculating deltas.
#[derive(Debug, Clone, Default)]
struct CpuTimes {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

impl CpuTimes {
    fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }

    fn active(&self) -> u64 {
        self.user + self.nice + self.system + self.irq + self.softirq + self.steal
    }
}

/// CPU collector with state for delta calculations.
pub struct CpuCollector {
    prev_total: CpuTimes,
    prev_per_core: Vec<CpuTimes>,
    core_count: usize,
}

impl CpuCollector {
    /// Create a new CPU collector.
    pub fn new() -> Result<Self> {
        let kernel_stats = procfs::KernelStats::current()?;
        let core_count = kernel_stats.cpu_time.len().max(1);

        Ok(Self {
            prev_total: CpuTimes::default(),
            prev_per_core: vec![CpuTimes::default(); core_count],
            core_count,
        })
    }

    /// Collect current CPU statistics.
    pub fn collect(&mut self) -> Result<CpuStats> {
        let kernel_stats = procfs::KernelStats::current()?;

        // Parse total CPU times
        let total = &kernel_stats.total;
        let curr_total = CpuTimes {
            user: total.user,
            nice: total.nice,
            system: total.system,
            idle: total.idle,
            iowait: total.iowait.unwrap_or(0),
            irq: total.irq.unwrap_or(0),
            softirq: total.softirq.unwrap_or(0),
            steal: total.steal.unwrap_or(0),
        };

        // Calculate overall usage
        let total_delta = curr_total.total().saturating_sub(self.prev_total.total());
        let active_delta = curr_total.active().saturating_sub(self.prev_total.active());
        let usage_percent = if total_delta > 0 {
            (active_delta as f64 / total_delta as f64) * 100.0
        } else {
            0.0
        };

        // Calculate per-core usage
        let mut per_core = Vec::with_capacity(self.core_count);
        for (i, cpu) in kernel_stats.cpu_time.iter().enumerate() {
            if i >= self.core_count {
                break;
            }
            let curr = CpuTimes {
                user: cpu.user,
                nice: cpu.nice,
                system: cpu.system,
                idle: cpu.idle,
                iowait: cpu.iowait.unwrap_or(0),
                irq: cpu.irq.unwrap_or(0),
                softirq: cpu.softirq.unwrap_or(0),
                steal: cpu.steal.unwrap_or(0),
            };

            let prev: &CpuTimes = &self.prev_per_core[i];
            let total_d = curr.total().saturating_sub(prev.total());
            let active_d = curr.active().saturating_sub(prev.active());
            let core_usage = if total_d > 0 {
                (active_d as f64 / total_d as f64) * 100.0
            } else {
                0.0
            };
            per_core.push(core_usage);

            self.prev_per_core[i] = curr;
        }

        self.prev_total = curr_total;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(CpuStats {
            usage_percent,
            per_core,
            core_count: self.core_count,
            timestamp,
        })
    }
}
