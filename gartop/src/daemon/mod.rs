//! Daemon mode implementation

mod state;

use crate::ipc::{ClientHandler, Command, IpcServer, Response};
use anyhow::Result;
use gartop_ipc::{SortField, StatusInfo};
use state::DaemonState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

/// Run the gartop daemon.
pub async fn run(_config_path: Option<String>, _foreground: bool) -> Result<()> {
    let state = Arc::new(Mutex::new(DaemonState::new(300, 1000)?));
    let server = IpcServer::new().await?;

    // CPU/Memory collection loop (every second)
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let mut s = state_clone.lock().await;
            if let Err(e) = s.collect_cpu() {
                debug!("CPU collect error: {}", e);
            }
            if let Err(e) = s.collect_memory() {
                debug!("Memory collect error: {}", e);
            }
        }
    });

    // Process collection loop (every 2 seconds)
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let mut s = state_clone.lock().await;
            if let Err(e) = s.collect_processes(SortField::Cpu, Some(100)) {
                debug!("Process collect error: {}", e);
            }
        }
    });

    info!("Daemon running, waiting for connections");

    loop {
        match server.accept().await {
            Ok(stream) => {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state).await {
                        debug!("Client error: {}", e);
                    }
                });
            }
            Err(e) => error!("Accept error: {}", e),
        }
    }
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    state: Arc<Mutex<DaemonState>>,
) -> Result<()> {
    let mut client = ClientHandler::new(stream);

    while let Some(cmd) = client.read_command().await? {
        let response = {
            let mut s = state.lock().await;
            match cmd {
                Command::Status => Response::ok_with_data(StatusInfo {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime_secs: s.uptime_secs(),
                    sample_interval_ms: s.sample_interval_ms,
                    history_size: s.cpu_history.len(),
                }),
                Command::GetCpu => match s.cpu_history.latest() {
                    Some(stats) => Response::ok_with_data(stats),
                    None => Response::err("No data yet"),
                },
                Command::GetMemory => match s.memory_history.latest() {
                    Some(stats) => Response::ok_with_data(stats),
                    None => Response::err("No data yet"),
                },
                Command::GetCpuHistory { count } => {
                    let data = match count {
                        Some(n) => s.cpu_history.last_n(n),
                        None => s.cpu_history.to_vec(),
                    };
                    Response::ok_with_data(data)
                }
                Command::GetMemoryHistory { count } => {
                    let data = match count {
                        Some(n) => s.memory_history.last_n(n),
                        None => s.memory_history.to_vec(),
                    };
                    Response::ok_with_data(data)
                }
                Command::GetProcesses { sort_by, limit } => {
                    match s.collect_processes(sort_by.unwrap_or_default(), limit) {
                        Ok(procs) => Response::ok_with_data(procs),
                        Err(e) => Response::err(e.to_string()),
                    }
                }
                Command::KillProcess { pid, signal } => {
                    match s.process_collector.kill(pid, signal.unwrap_or(15)) {
                        Ok(()) => Response::ok(),
                        Err(e) => Response::err(e.to_string()),
                    }
                }
                Command::Subscribe { events } => {
                    client.subscribe(&events);
                    Response::ok()
                }
                Command::Unsubscribe { events } => {
                    client.unsubscribe(&events);
                    Response::ok()
                }
                Command::Reload => {
                    info!("Reload requested");
                    Response::ok()
                }
                Command::Quit => {
                    info!("Quit command received, shutting down");
                    std::process::exit(0);
                }
            }
        };

        client.send_response(&response).await?;

        // Non-subscription connections close after response
        if !client.has_subscriptions() {
            break;
        }
    }

    Ok(())
}
