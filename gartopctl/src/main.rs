//! gartopctl - Control CLI for gartop daemon

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gartop_ipc::{Command, Response, SortField};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Parser)]
#[command(name = "gartopctl")]
#[command(about = "Control the gartop daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Get daemon status
    Status,
    /// Get current CPU stats
    Cpu,
    /// Get current memory stats
    Memory,
    /// Get CPU usage history
    CpuHistory {
        /// Number of samples
        #[arg(short, long)]
        count: Option<usize>,
    },
    /// Get memory usage history
    MemoryHistory {
        /// Number of samples
        #[arg(short, long)]
        count: Option<usize>,
    },
    /// List running processes
    Processes {
        /// Sort by field (cpu, memory, pid, name)
        #[arg(short, long, default_value = "cpu")]
        sort: String,
        /// Limit results
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Kill a process
    Kill {
        /// Process ID
        pid: i32,
        /// Signal number (default: 15 SIGTERM)
        #[arg(short, long)]
        signal: Option<i32>,
    },
    /// Reload configuration
    Reload,
    /// Stop the daemon
    Quit,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let cmd = match cli.command {
        Commands::Status => Command::Status,
        Commands::Cpu => Command::GetCpu,
        Commands::Memory => Command::GetMemory,
        Commands::CpuHistory { count } => Command::GetCpuHistory { count },
        Commands::MemoryHistory { count } => Command::GetMemoryHistory { count },
        Commands::Processes { sort, limit } => {
            let sort_by = match sort.as_str() {
                "memory" | "mem" => Some(SortField::Memory),
                "pid" => Some(SortField::Pid),
                "name" => Some(SortField::Name),
                _ => Some(SortField::Cpu),
            };
            Command::GetProcesses { sort_by, limit }
        }
        Commands::Kill { pid, signal } => Command::KillProcess { pid, signal },
        Commands::Reload => Command::Reload,
        Commands::Quit => Command::Quit,
    };

    let response = send_command(&cmd)?;

    if response.success {
        if let Some(data) = response.data {
            println!("{}", serde_json::to_string_pretty(&data)?);
        } else {
            println!("OK");
        }
    } else {
        eprintln!("Error: {}", response.error.unwrap_or_default());
        std::process::exit(1);
    }

    Ok(())
}

fn send_command(cmd: &Command) -> Result<Response> {
    let socket_path = gartop_ipc::socket_path();

    if !socket_path.exists() {
        anyhow::bail!("gartop daemon not running (socket not found at {})", socket_path.display());
    }

    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| "Failed to connect to gartop daemon")?;

    // Send command as JSON line
    let json = serde_json::to_string(cmd)?;
    writeln!(stream, "{}", json)?;
    stream.flush()?;

    // Read response
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response: Response = serde_json::from_str(&line)?;
    Ok(response)
}
