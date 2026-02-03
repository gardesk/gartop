//! IPC server for gartop daemon

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::net::{UnixListener, UnixStream};

/// IPC server for accepting client connections.
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server.
    pub async fn new() -> Result<Self> {
        let socket_path = gartop_ipc::socket_path();

        // Remove existing socket if present
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        // Create parent directory if needed
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("Failed to bind to {}", socket_path.display()))?;

        // Set permissions to user-only (0600)
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

        tracing::info!("IPC server listening on {}", socket_path.display());

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Get the socket path.
    pub fn path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Accept a new client connection.
    pub async fn accept(&self) -> Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream)
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Clean up socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
