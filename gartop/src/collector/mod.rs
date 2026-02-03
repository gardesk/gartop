//! System data collectors
//!
//! Collects CPU, memory, process, network, disk, temperature, and GPU data from procfs/sysfs.

mod cpu;
mod disk;
mod gpu;
mod history;
mod memory;
mod network;
mod process;
mod socket;
mod temperature;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use gpu::GpuCollector;
pub use history::History;
pub use memory::MemoryCollector;
pub use network::NetworkCollector;
pub use process::ProcessCollector;
pub use socket::SocketCollector;
pub use temperature::TempCollector;
