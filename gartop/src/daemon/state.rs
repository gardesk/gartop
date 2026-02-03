//! Daemon state management

use crate::collector::{CpuCollector, DiskCollector, GpuCollector, History, MemoryCollector, NetworkCollector, ProcessCollector, TempCollector};
use crate::error::Result;
use gartop_ipc::{CpuStats, DiskStats, GpuStats, MemoryStats, NetworkStats, ProcessInfo, SortField, TempStats};
use procfs::Current;
use std::time::Instant;

/// Shared daemon state.
pub struct DaemonState {
    pub cpu_collector: CpuCollector,
    pub memory_collector: MemoryCollector,
    pub network_collector: NetworkCollector,
    pub disk_collector: DiskCollector,
    pub process_collector: ProcessCollector,
    pub temp_collector: TempCollector,
    pub gpu_collector: GpuCollector,
    pub cpu_history: History<CpuStats>,
    pub memory_history: History<MemoryStats>,
    pub network_history: History<Vec<NetworkStats>>,
    pub disk_history: History<Vec<DiskStats>>,
    pub processes: Vec<ProcessInfo>,
    pub temp_stats: Option<TempStats>,
    pub gpu_stats: Option<GpuStats>,
    pub started: Instant,
    pub sample_interval_ms: u64,
}

impl DaemonState {
    /// Create new daemon state.
    pub fn new(history_size: usize, sample_interval_ms: u64) -> Result<Self> {
        Ok(Self {
            cpu_collector: CpuCollector::new()?,
            memory_collector: MemoryCollector::new(),
            network_collector: NetworkCollector::new(),
            disk_collector: DiskCollector::new(),
            process_collector: ProcessCollector::new()?,
            temp_collector: TempCollector::new()?,
            gpu_collector: GpuCollector::new()?,
            cpu_history: History::new(history_size),
            memory_history: History::new(history_size),
            network_history: History::new(history_size),
            disk_history: History::new(history_size),
            processes: Vec::new(),
            temp_stats: None,
            gpu_stats: None,
            started: Instant::now(),
            sample_interval_ms,
        })
    }

    /// Collect CPU stats and add to history.
    pub fn collect_cpu(&mut self) -> Result<CpuStats> {
        let stats = self.cpu_collector.collect()?;
        self.cpu_history.push(stats.clone());
        Ok(stats)
    }

    /// Collect memory stats and add to history.
    pub fn collect_memory(&mut self) -> Result<MemoryStats> {
        let stats = self.memory_collector.collect()?;
        self.memory_history.push(stats.clone());
        Ok(stats)
    }

    /// Collect network stats and add to history.
    pub fn collect_network(&mut self) -> Result<Vec<NetworkStats>> {
        let stats = self.network_collector.collect()?;
        self.network_history.push(stats.clone());
        Ok(stats)
    }

    /// Collect disk stats and add to history.
    pub fn collect_disk(&mut self) -> Result<Vec<DiskStats>> {
        let stats = self.disk_collector.collect()?;
        self.disk_history.push(stats.clone());
        Ok(stats)
    }

    /// Collect temperature stats (no history, just current values).
    pub fn collect_temperature(&mut self) -> Result<TempStats> {
        let stats = self.temp_collector.collect()?;
        self.temp_stats = Some(stats.clone());
        Ok(stats)
    }

    /// Collect GPU stats (no history, just current values).
    pub fn collect_gpu(&mut self) -> Result<GpuStats> {
        let stats = self.gpu_collector.collect()?;
        self.gpu_stats = Some(stats.clone());
        Ok(stats)
    }

    /// Collect process list.
    pub fn collect_processes(
        &mut self,
        sort: SortField,
        limit: Option<usize>,
    ) -> Result<Vec<ProcessInfo>> {
        self.processes = self.process_collector.collect(sort, limit)?;
        Ok(self.processes.clone())
    }

    /// Get system uptime in seconds (from /proc/uptime).
    pub fn uptime_secs(&self) -> u64 {
        procfs::Uptime::current()
            .map(|u| u.uptime as u64)
            .unwrap_or(0)
    }
}
