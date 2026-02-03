//! Disk I/O statistics collection from /proc/diskstats

use crate::error::Result;
use gartop_ipc::DiskStats;
use std::collections::HashMap;
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Disk device data for rate calculation.
struct DiskData {
    read_bytes: u64,
    write_bytes: u64,
    reads: u64,
    writes: u64,
    timestamp: Instant,
}

/// Disk I/O statistics collector.
pub struct DiskCollector {
    prev_data: HashMap<String, DiskData>,
}

impl DiskCollector {
    /// Create a new disk collector.
    pub fn new() -> Self {
        Self {
            prev_data: HashMap::new(),
        }
    }

    /// Collect current disk I/O statistics for all block devices.
    pub fn collect(&mut self) -> Result<Vec<DiskStats>> {
        let contents = fs::read_to_string("/proc/diskstats")?;
        let now = Instant::now();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut stats = Vec::new();

        for line in contents.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 14 {
                continue;
            }

            let device = parts[2].to_string();

            // Only include physical disks, not partitions or device-mapper
            // Include: sda, nvme0n1, vda, etc.
            // Exclude: sda1, nvme0n1p1, loop*, dm-*, ram*
            if !Self::is_physical_disk(&device) {
                continue;
            }

            // Fields from /proc/diskstats (sector size is typically 512 bytes)
            let reads: u64 = parts[3].parse().unwrap_or(0);
            let read_sectors: u64 = parts[5].parse().unwrap_or(0);
            let writes: u64 = parts[7].parse().unwrap_or(0);
            let write_sectors: u64 = parts[9].parse().unwrap_or(0);

            // Convert sectors to bytes (512 bytes per sector)
            let read_bytes = read_sectors * 512;
            let write_bytes = write_sectors * 512;

            // Calculate rates from previous data
            let (read_rate, write_rate) = if let Some(prev) = self.prev_data.get(&device) {
                let elapsed = now.duration_since(prev.timestamp).as_secs_f64();
                if elapsed > 0.0 {
                    let read_delta = read_bytes.saturating_sub(prev.read_bytes) as f64;
                    let write_delta = write_bytes.saturating_sub(prev.write_bytes) as f64;
                    (read_delta / elapsed, write_delta / elapsed)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };

            // Store current data for next calculation
            self.prev_data.insert(
                device.clone(),
                DiskData {
                    read_bytes,
                    write_bytes,
                    reads,
                    writes,
                    timestamp: now,
                },
            );

            stats.push(DiskStats {
                device,
                read_bytes,
                write_bytes,
                reads,
                writes,
                read_rate,
                write_rate,
                timestamp,
            });
        }

        Ok(stats)
    }

    /// Check if a device name is a physical disk (not a partition or virtual device).
    fn is_physical_disk(device: &str) -> bool {
        // Exclude loop devices
        if device.starts_with("loop") {
            return false;
        }
        // Exclude device-mapper
        if device.starts_with("dm-") {
            return false;
        }
        // Exclude ram disks
        if device.starts_with("ram") {
            return false;
        }
        // Exclude partitions (sda1, nvme0n1p1, etc.)
        if device.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            // Check if it's a partition
            if device.starts_with("sd") || device.starts_with("hd") || device.starts_with("vd") {
                // sda, sdb are disks; sda1, sda2 are partitions
                let num_digits = device.chars().rev().take_while(|c| c.is_ascii_digit()).count();
                let alpha_part: String = device.chars().take(device.len() - num_digits).collect();
                // If alpha part ends with letter, it's a partition
                if alpha_part.len() > 2 {
                    return false;
                }
            }
            // NVMe: nvme0n1 is disk, nvme0n1p1 is partition
            if device.contains("nvme") && device.contains('p') {
                return false;
            }
        }
        true
    }
}

impl Default for DiskCollector {
    fn default() -> Self {
        Self::new()
    }
}
