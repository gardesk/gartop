//! Client connection handler

use anyhow::{Context, Result};
use gartop_ipc::{Command, Event, Response};
use std::collections::HashSet;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Handle a single client connection.
pub struct ClientHandler {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    subscriptions: HashSet<String>,
}

impl ClientHandler {
    /// Create a new client handler.
    pub fn new(stream: UnixStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            subscriptions: HashSet::new(),
        }
    }

    /// Read a command from the client.
    pub async fn read_command(&mut self) -> Result<Option<Command>> {
        let mut line = String::new();

        match self.reader.read_line(&mut line).await {
            Ok(0) => Ok(None), // EOF
            Ok(_) => {
                let cmd: Command =
                    serde_json::from_str(&line).context("Failed to parse command")?;
                Ok(Some(cmd))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Send a response to the client.
    pub async fn send_response(&mut self, response: &Response) -> Result<()> {
        let json = serde_json::to_string(response)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Send an event to the client (for subscriptions).
    pub async fn send_event(&mut self, event: &Event) -> Result<()> {
        let json = serde_json::to_string(event)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Subscribe to event types.
    pub fn subscribe(&mut self, events: &[String]) {
        self.subscriptions.extend(events.iter().cloned());
    }

    /// Unsubscribe from event types.
    pub fn unsubscribe(&mut self, events: &[String]) {
        for event in events {
            self.subscriptions.remove(event);
        }
    }

    /// Check if subscribed to an event type.
    pub fn is_subscribed(&self, event_type: &str) -> bool {
        self.subscriptions.contains(event_type) || self.subscriptions.contains("*")
    }

    /// Check if client has any subscriptions (persistent connection).
    pub fn has_subscriptions(&self) -> bool {
        !self.subscriptions.is_empty()
    }
}
