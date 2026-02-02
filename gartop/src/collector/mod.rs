//! System data collectors
//!
//! Collects CPU, memory, and process data from procfs.

mod cpu;
mod history;
mod memory;
mod process;

pub use cpu::CpuCollector;
pub use history::History;
pub use memory::MemoryCollector;
pub use process::ProcessCollector;
