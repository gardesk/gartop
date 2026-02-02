//! Memory statistics collection from /proc/meminfo

use crate::error::Result;
use gartop_ipc::MemoryStats;
use procfs::Current;
use std::time::{SystemTime, UNIX_EPOCH};

/// Memory collector.
pub struct MemoryCollector;

impl MemoryCollector {
    /// Create a new memory collector.
    pub fn new() -> Self {
        Self
    }

    /// Collect current memory statistics.
    pub fn collect(&self) -> Result<MemoryStats> {
        let meminfo = procfs::Meminfo::current()?;

        let total = meminfo.mem_total;
        let free = meminfo.mem_free;
        let available = meminfo.mem_available.unwrap_or(free);
        let buffers = meminfo.buffers;
        let cached = meminfo.cached;
        let slab_reclaimable = meminfo.s_reclaimable.unwrap_or(0);

        // Calculate used memory (excluding buffers/cache)
        let used = total.saturating_sub(free + buffers + cached + slab_reclaimable);

        let swap_total = meminfo.swap_total;
        let swap_free = meminfo.swap_free;
        let swap_used = swap_total.saturating_sub(swap_free);

        let usage_percent = if total > 0 {
            (used as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(MemoryStats {
            total,
            used,
            free,
            available,
            swap_total,
            swap_used,
            usage_percent,
            timestamp,
        })
    }
}

impl Default for MemoryCollector {
    fn default() -> Self {
        Self::new()
    }
}
