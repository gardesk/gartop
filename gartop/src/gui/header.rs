//! Header bar component

use gartk_core::Rect;
use gartk_render::{Renderer, TextStyle};
use super::theme::Theme;

/// Header bar showing app title and stats summary.
pub struct HeaderBar {
    bounds: Rect,
    uptime: String,
    cpu_usage: f32,
    memory_usage: f32,
    net_rate: f64,   // Combined rx+tx bytes/sec
    disk_rate: f64,  // Combined read+write bytes/sec
}

impl HeaderBar {
    /// Create a new header bar.
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            uptime: String::from("0s"),
            cpu_usage: 0.0,
            memory_usage: 0.0,
            net_rate: 0.0,
            disk_rate: 0.0,
        }
    }

    /// Update header with current stats.
    pub fn update(&mut self, uptime_secs: u64, cpu: f32, memory: f32, net_rate: f64, disk_rate: f64) {
        self.uptime = format_uptime(uptime_secs);
        self.cpu_usage = cpu;
        self.memory_usage = memory;
        self.net_rate = net_rate;
        self.disk_rate = disk_rate;
    }

    /// Render the header bar.
    pub fn render(&self, renderer: &Renderer, theme: &Theme) -> anyhow::Result<()> {
        // Background - use panel_bg for seamless transition to tab bar
        renderer.fill_rect(self.bounds, theme.panel_bg)?;

        // Title
        let title_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 14.0,
            color: theme.text,
            ..Default::default()
        };
        // Position text in upper portion of header (leave room below for visual separation)
        let text_y = self.bounds.y as f64 + (self.bounds.height as f64 * 0.4) + 6.0;
        renderer.text("gartop", 16.0, text_y, &title_style)?;

        // Stats summary on right side
        let stats_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 12.0,
            color: theme.text_secondary,
            ..Default::default()
        };

        // CPU indicator
        let cpu_text = format!("CPU: {:.1}%", self.cpu_usage);
        let cpu_style = TextStyle {
            color: theme.cpu_color,
            ..stats_style.clone()
        };

        // Memory indicator
        let mem_text = format!("MEM: {:.1}%", self.memory_usage);
        let mem_style = TextStyle {
            color: theme.memory_color,
            ..stats_style.clone()
        };

        // Network indicator
        let net_text = format!("NET: {}", format_rate(self.net_rate));
        let net_style = TextStyle {
            color: theme.network_color,
            ..stats_style.clone()
        };

        // Disk indicator
        let disk_text = format!("DISK: {}", format_rate(self.disk_rate));
        let disk_style = TextStyle {
            color: theme.disk_color,
            ..stats_style.clone()
        };

        // Uptime
        let uptime_text = format!("up {}", self.uptime);

        // Position from right side
        let right_margin = 16.0;
        let spacing = 16.0;
        let y = self.bounds.y as f64 + (self.bounds.height as f64 * 0.4) + 6.0;

        let uptime_width = renderer.measure_text(&uptime_text, &stats_style)?.width as f64;
        let disk_width = renderer.measure_text(&disk_text, &disk_style)?.width as f64;
        let net_width = renderer.measure_text(&net_text, &net_style)?.width as f64;
        let mem_width = renderer.measure_text(&mem_text, &mem_style)?.width as f64;
        let cpu_width = renderer.measure_text(&cpu_text, &cpu_style)?.width as f64;

        let right_edge = (self.bounds.x + self.bounds.width as i32) as f64;

        let uptime_x = right_edge - right_margin - uptime_width;
        let disk_x = uptime_x - spacing - disk_width;
        let net_x = disk_x - spacing - net_width;
        let mem_x = net_x - spacing - mem_width;
        let cpu_x = mem_x - spacing - cpu_width;

        renderer.text(&cpu_text, cpu_x, y, &cpu_style)?;
        renderer.text(&mem_text, mem_x, y, &mem_style)?;
        renderer.text(&net_text, net_x, y, &net_style)?;
        renderer.text(&disk_text, disk_x, y, &disk_style)?;
        renderer.text(&uptime_text, uptime_x, y, &stats_style)?;

        Ok(())
    }

    /// Get header height.
    pub fn height(&self) -> u32 {
        self.bounds.height
    }
}

/// Format uptime seconds to human-readable string.
fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

/// Format rate (bytes/second) to compact human-readable string.
fn format_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_sec >= GIB {
        format!("{:.1}G/s", bytes_per_sec / GIB)
    } else if bytes_per_sec >= MIB {
        format!("{:.1}M/s", bytes_per_sec / MIB)
    } else if bytes_per_sec >= KIB {
        format!("{:.0}K/s", bytes_per_sec / KIB)
    } else if bytes_per_sec > 0.0 {
        format!("{:.0}B/s", bytes_per_sec)
    } else {
        "0".to_string()
    }
}
