//! System data collectors
//!
//! Collects CPU, memory, process, network, and disk data from procfs.

mod cpu;
mod disk;
mod history;
mod memory;
mod network;
mod process;
mod socket;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use history::History;
pub use memory::MemoryCollector;
pub use network::NetworkCollector;
pub use process::ProcessCollector;
pub use socket::{SocketCollector, ProcessNetStats};
