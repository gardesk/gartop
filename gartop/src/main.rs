//! gartop - System monitor for gar desktop
//!
//! A daemon-based system monitor with real-time CPU, memory, and process
//! monitoring. Uses gartk for GUI rendering.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod collector;
mod config;
mod daemon;
mod error;
mod gui;
mod ipc;

/// gartop - System monitor for gar desktop
#[derive(Parser)]
#[command(name = "gartop")]
#[command(about = "System monitor for gar desktop", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in foreground (don't daemonize)
    #[arg(short, long)]
    foreground: bool,

    /// Configuration file path
    #[arg(short, long)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (default)
    Daemon {
        /// Run in foreground
        #[arg(short, long)]
        foreground: bool,
    },
    /// Open the GUI window (connects to daemon)
    Gui,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon { foreground }) => {
            tracing::info!("Starting gartop daemon");
            daemon::run(cli.config, foreground || cli.foreground).await
        }
        Some(Commands::Gui) => {
            tracing::info!("Starting gartop GUI");
            gui::run().await
        }
        None => {
            // Default: start daemon
            tracing::info!("Starting gartop daemon");
            daemon::run(cli.config, cli.foreground).await
        }
    }
}
