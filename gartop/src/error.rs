//! Error types for gartop

use thiserror::Error;

/// Library errors
#[derive(Debug, Error)]
pub enum Error {
    #[error("procfs error: {0}")]
    Procfs(#[from] procfs::ProcError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("process not found: {0}")]
    ProcessNotFound(i32),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
