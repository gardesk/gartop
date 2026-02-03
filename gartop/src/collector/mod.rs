//! System data collectors
//!
//! Collects CPU, memory, process, network, disk, and temperature data from procfs/sysfs.

mod cpu;
mod disk;
mod history;
mod memory;
mod network;
mod process;
mod socket;
mod temperature;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use history::History;
pub use memory::MemoryCollector;
pub use network::NetworkCollector;
pub use process::ProcessCollector;
pub use socket::SocketCollector;
pub use temperature::TempCollector;
