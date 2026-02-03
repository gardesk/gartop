//! Network statistics collection from /proc/net/dev

use crate::error::Result;
use gartop_ipc::NetworkStats;
use std::collections::HashMap;
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Network interface data for rate calculation.
struct InterfaceData {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_packets: u64,
    tx_packets: u64,
    timestamp: Instant,
}

/// Network statistics collector.
pub struct NetworkCollector {
    prev_data: HashMap<String, InterfaceData>,
}

impl NetworkCollector {
    /// Create a new network collector.
    pub fn new() -> Self {
        Self {
            prev_data: HashMap::new(),
        }
    }

    /// Collect current network statistics for all interfaces.
    pub fn collect(&mut self) -> Result<Vec<NetworkStats>> {
        let contents = fs::read_to_string("/proc/net/dev")?;
        let now = Instant::now();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut stats = Vec::new();

        for line in contents.lines().skip(2) {
            // Skip header lines
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Parse: interface: rx_bytes rx_packets ... tx_bytes tx_packets ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 10 {
                continue;
            }

            let interface = parts[0].trim_end_matches(':').to_string();

            // Skip loopback and virtual interfaces
            if interface == "lo" || interface.starts_with("veth") || interface.starts_with("docker") {
                continue;
            }

            let rx_bytes: u64 = parts[1].parse().unwrap_or(0);
            let rx_packets: u64 = parts[2].parse().unwrap_or(0);
            let tx_bytes: u64 = parts[9].parse().unwrap_or(0);
            let tx_packets: u64 = parts[10].parse().unwrap_or(0);

            // Calculate rates from previous data
            let (rx_rate, tx_rate) = if let Some(prev) = self.prev_data.get(&interface) {
                let elapsed = now.duration_since(prev.timestamp).as_secs_f64();
                if elapsed > 0.0 {
                    let rx_delta = rx_bytes.saturating_sub(prev.rx_bytes) as f64;
                    let tx_delta = tx_bytes.saturating_sub(prev.tx_bytes) as f64;
                    (rx_delta / elapsed, tx_delta / elapsed)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };

            // Store current data for next calculation
            self.prev_data.insert(
                interface.clone(),
                InterfaceData {
                    rx_bytes,
                    tx_bytes,
                    rx_packets,
                    tx_packets,
                    timestamp: now,
                },
            );

            stats.push(NetworkStats {
                interface,
                rx_bytes,
                tx_bytes,
                rx_packets,
                tx_packets,
                rx_rate,
                tx_rate,
                timestamp,
            });
        }

        Ok(stats)
    }
}

impl Default for NetworkCollector {
    fn default() -> Self {
        Self::new()
    }
}
