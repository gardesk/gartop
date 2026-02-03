//! gartop - System monitor for gar desktop
//!
//! A daemon-based system monitor with real-time CPU, memory, and process
//! monitoring. Uses gartk for GUI rendering.

#![allow(dead_code)] // Many fields/methods are for future features (Phase 5, 6)

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

    /// Open to specific pane (for garbar integration)
    #[arg(short, long, value_parser = ["cpu", "memory", "network", "disk"])]
    pane: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon
    Daemon {
        /// Run in foreground
        #[arg(short, long)]
        foreground: bool,
    },
    /// Open the GUI window (connects to daemon)
    Gui {
        /// Open to specific pane
        #[arg(short, long, value_parser = ["cpu", "memory", "network", "disk"])]
        pane: Option<String>,
    },
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
        Some(Commands::Gui { pane }) => {
            tracing::info!("Starting gartop GUI");
            let mut cfg = config::Config::load(cli.config.as_deref())?;
            if let Some(p) = pane {
                cfg.gui.default_pane = Some(p);
            }
            gui::run(cfg.gui).await
        }
        None => {
            // Default: start GUI (use --pane if provided)
            tracing::info!("Starting gartop GUI");
            let mut cfg = config::Config::load(cli.config.as_deref())?;
            if let Some(p) = cli.pane {
                cfg.gui.default_pane = Some(p);
            }
            gui::run(cfg.gui).await
        }
    }
}
