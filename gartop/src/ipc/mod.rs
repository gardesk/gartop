//! IPC server and client handling

mod client;
mod server;

pub use client::ClientHandler;
pub use server::IpcServer;

// Re-export from gartop-ipc
pub use gartop_ipc::{Command, Response};
