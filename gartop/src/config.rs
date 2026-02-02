//! Configuration for gartop

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub gui: GuiConfig,
}

/// Daemon configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Sample interval in milliseconds.
    pub sample_interval_ms: u64,
    /// History buffer size (number of samples to keep).
    pub history_size: usize,
    /// Maximum number of processes to track.
    pub max_processes: usize,
}

/// GUI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    /// Initial window width.
    pub width: u32,
    /// Initial window height.
    pub height: u32,
    /// Refresh rate in FPS.
    pub refresh_rate: u32,
    /// Show legend on graphs.
    pub show_legend: bool,
    /// Font family for UI.
    pub font_family: String,
    /// Font size.
    pub font_size: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            gui: GuiConfig::default(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            sample_interval_ms: 1000,
            history_size: 300,
            max_processes: 100,
        }
    }
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            width: 600,
            height: 500,
            refresh_rate: 30,
            show_legend: true,
            font_family: "sans-serif".to_string(),
            font_size: 12.0,
        }
    }
}

impl Config {
    /// Load configuration from file.
    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = path.map(PathBuf::from).or_else(Self::default_path);

        if let Some(path) = config_path {
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let config: Config = toml::from_str(&content)?;
                tracing::info!("Loaded config from {}", path.display());
                return Ok(config);
            }
        }

        Ok(Self::default())
    }

    /// Get default configuration path.
    fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("gartop").join("config.toml"))
    }
}
