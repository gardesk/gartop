//! Shared IPC protocol types for gartop.
//!
//! This crate defines the request/response types used for communication
//! between gartop daemon and clients (gartopctl, GUI, garbar).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// IPC commands sent to the gartop daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Get daemon status.
    Status,
    /// Get current CPU stats.
    GetCpu,
    /// Get current memory stats.
    GetMemory,
    /// Get CPU history.
    GetCpuHistory {
        #[serde(default)]
        count: Option<usize>,
    },
    /// Get memory history.
    GetMemoryHistory {
        #[serde(default)]
        count: Option<usize>,
    },
    /// Get current network stats.
    GetNetwork,
    /// Get network history.
    GetNetworkHistory {
        #[serde(default)]
        count: Option<usize>,
    },
    /// Get current disk I/O stats.
    GetDisk,
    /// Get disk I/O history.
    GetDiskHistory {
        #[serde(default)]
        count: Option<usize>,
    },
    /// Get running processes.
    GetProcesses {
        #[serde(default)]
        sort_by: Option<SortField>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Kill a process by PID.
    KillProcess {
        pid: i32,
        #[serde(default)]
        signal: Option<i32>,
    },
    /// Subscribe to real-time updates.
    Subscribe { events: Vec<String> },
    /// Unsubscribe from events.
    Unsubscribe { events: Vec<String> },
    /// Reload configuration.
    Reload,
    /// Quit the daemon.
    Quit,
}

/// Sort field for process list.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Cpu,
    Memory,
    DiskRead,
    DiskWrite,
    DiskTotal,
    NetConnections,
    NetTcp,
    NetBandwidth,
    Pid,
    Name,
}

/// IPC response from gartop daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    /// Create a successful response with no data.
    pub fn ok() -> Self {
        Self {
            success: true,
            data: None,
            error: None,
        }
    }

    /// Create a successful response with data.
    pub fn ok_with_data(data: impl Serialize) -> Self {
        Self {
            success: true,
            data: serde_json::to_value(data).ok(),
            error: None,
        }
    }

    /// Create an error response.
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// CPU usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuStats {
    /// Overall CPU usage percentage (0-100).
    pub usage_percent: f64,
    /// Per-core usage percentages.
    pub per_core: Vec<f64>,
    /// Number of cores.
    pub core_count: usize,
    /// Timestamp (milliseconds since epoch).
    pub timestamp: u64,
}

/// Memory usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Total memory in bytes.
    pub total: u64,
    /// Used memory in bytes (excluding cache/buffers).
    pub used: u64,
    /// Free memory in bytes.
    pub free: u64,
    /// Available memory in bytes.
    pub available: u64,
    /// Swap total in bytes.
    pub swap_total: u64,
    /// Swap used in bytes.
    pub swap_used: u64,
    /// Usage percentage (0-100).
    pub usage_percent: f64,
    /// Timestamp (milliseconds since epoch).
    pub timestamp: u64,
}

/// Process information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    /// Process ID.
    pub pid: i32,
    /// Process name.
    pub name: String,
    /// Command line.
    pub cmdline: String,
    /// CPU usage percentage.
    pub cpu_percent: f64,
    /// Memory usage percentage.
    pub memory_percent: f64,
    /// Resident set size in bytes.
    pub rss: u64,
    /// Virtual memory size in bytes.
    pub vsize: u64,
    /// Disk read bytes (cumulative).
    pub io_read_bytes: u64,
    /// Disk write bytes (cumulative).
    pub io_write_bytes: u64,
    /// Disk read rate in bytes/sec.
    pub io_read_rate: f64,
    /// Disk write rate in bytes/sec.
    pub io_write_rate: f64,
    /// Number of open network sockets (total).
    pub net_connections: u32,
    /// Number of TCP connections.
    pub net_tcp: u32,
    /// Number of UDP sockets.
    pub net_udp: u32,
    /// Number of listening sockets.
    pub net_listen: u32,
    /// Number of established TCP connections.
    pub net_established: u32,
    /// Network receive rate in bytes/sec.
    pub net_rx_rate: f64,
    /// Network transmit rate in bytes/sec.
    pub net_tx_rate: f64,
    /// Process state.
    pub state: String,
    /// User owning the process.
    pub user: String,
}

/// Network interface statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    /// Interface name (e.g., "eth0", "wlan0").
    pub interface: String,
    /// Bytes received.
    pub rx_bytes: u64,
    /// Bytes transmitted.
    pub tx_bytes: u64,
    /// Packets received.
    pub rx_packets: u64,
    /// Packets transmitted.
    pub tx_packets: u64,
    /// Receive rate in bytes/sec (calculated from delta).
    pub rx_rate: f64,
    /// Transmit rate in bytes/sec (calculated from delta).
    pub tx_rate: f64,
    /// Timestamp (milliseconds since epoch).
    pub timestamp: u64,
}

/// Disk I/O statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskStats {
    /// Device name (e.g., "sda", "nvme0n1").
    pub device: String,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Read operations completed.
    pub reads: u64,
    /// Write operations completed.
    pub writes: u64,
    /// Read rate in bytes/sec (calculated from delta).
    pub read_rate: f64,
    /// Write rate in bytes/sec (calculated from delta).
    pub write_rate: f64,
    /// Timestamp (milliseconds since epoch).
    pub timestamp: u64,
}

/// Events broadcast to subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// CPU stats updated.
    CpuUpdate(CpuStats),
    /// Memory stats updated.
    MemoryUpdate(MemoryStats),
    /// Process list updated.
    ProcessUpdate { processes: Vec<ProcessInfo> },
    /// Network stats updated.
    NetworkUpdate { interfaces: Vec<NetworkStats> },
    /// Disk stats updated.
    DiskUpdate { disks: Vec<DiskStats> },
}

/// Daemon status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    /// Daemon version.
    pub version: String,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Sample interval in milliseconds.
    pub sample_interval_ms: u64,
    /// History buffer size.
    pub history_size: usize,
}

/// Get the default socket path for gartop IPC.
pub fn socket_path() -> PathBuf {
    let runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("gartop.sock")
}
