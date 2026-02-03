//! GUI application state and event loop

use super::{
    graph::{DataSeries, LineGraph},
    header::HeaderBar,
    process_list::ProcessList,
    tabs::{Tab, TabBar, TAB_BAR_HEIGHT},
    theme::Theme,
};
use crate::config::GuiConfig;
use anyhow::Result;
use gartk_core::{InputEvent, Key, Point, Rect};
use gartk_render::{Renderer, TextStyle};
use gartk_x11::{Connection, EventLoop, EventLoopConfig, Window, WindowConfig};
use gartop_ipc::{Command, CpuStats, DiskStats, MemoryStats, NetworkStats, ProcessInfo, Response, SortField, StatusInfo, TempStats};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

/// Header bar height.
const HEADER_HEIGHT: u32 = 56;

/// Graph height.
const GRAPH_HEIGHT: u32 = 150;

/// Horizontal content padding.
const CONTENT_PADDING: u32 = 16;

/// Vertical gap between sections.
const SECTION_GAP: u32 = 12;

/// Max history points per process
const PROCESS_HISTORY_LEN: usize = 60;

/// Per-process CPU/memory history for sparkline graphs.
struct ProcessHistory {
    cpu: Vec<f64>,
    memory: Vec<f64>,
}

impl ProcessHistory {
    fn new() -> Self {
        Self {
            cpu: Vec::with_capacity(PROCESS_HISTORY_LEN),
            memory: Vec::with_capacity(PROCESS_HISTORY_LEN),
        }
    }

    fn push(&mut self, cpu: f64, memory: f64) {
        if self.cpu.len() >= PROCESS_HISTORY_LEN {
            self.cpu.remove(0);
            self.memory.remove(0);
        }
        self.cpu.push(cpu);
        self.memory.push(memory);
    }
}

/// GUI application.
pub struct App {
    window: Window,
    renderer: Renderer,
    gc: u32,
    theme: Theme,
    header: HeaderBar,
    tab_bar: TabBar,
    process_list: ProcessList,
    should_quit: bool,
    width: u32,
    height: u32,
    daemon_available: bool,
    last_refresh: Instant,
    refresh_interval: f64,
    show_legend: bool,
    /// Freeze mode - pause process list updates for navigation
    frozen: bool,
    /// Tree view mode - show process hierarchy
    tree_view: bool,
    /// Search mode - filter processes by name
    search_mode: bool,
    /// Search query string
    search_query: String,
    /// Help overlay visible
    show_help: bool,
    /// Type-to-jump fuzzy pattern
    jump_pattern: String,
    /// Last time a jump character was typed (for timeout)
    jump_time: Option<Instant>,
    /// Kill confirmation: (pid, signal, process_name)
    kill_confirm: Option<(i32, i32, String)>,
    /// Process detail view: PID of process being viewed
    detail_pid: Option<i32>,
    status: Option<StatusInfo>,
    cpu_stats: Option<CpuStats>,
    memory_stats: Option<MemoryStats>,
    network_stats: Vec<NetworkStats>,
    disk_stats: Vec<DiskStats>,
    temp_stats: Option<TempStats>,
    cpu_history: Vec<CpuStats>,
    memory_history: Vec<MemoryStats>,
    network_history: Vec<Vec<NetworkStats>>,
    disk_history: Vec<Vec<DiskStats>>,
    processes: Vec<ProcessInfo>,
    /// Per-process CPU/memory history for sparkline graphs
    process_history: HashMap<i32, ProcessHistory>,
}

impl App {
    /// Create a new GUI application.
    pub fn new(config: GuiConfig) -> Result<Self> {
        let conn = Connection::connect(None)?;

        // Get primary monitor for centering
        let monitor = gartk_x11::primary_monitor(&conn)?;

        let width = config.width.min(monitor.rect.width);
        let height = config.height.min(monitor.rect.height);
        let x = monitor.rect.x + (monitor.rect.width as i32 - width as i32) / 2;
        let y = monitor.rect.y + (monitor.rect.height as i32 - height as i32) / 2;

        // Calculate refresh interval from FPS
        let refresh_interval = 1.0 / config.refresh_rate.max(1) as f64;

        let window = Window::create(
            conn.clone(),
            WindowConfig::default()
                .title("gartop")
                .class("gartop")
                .position(x, y)
                .size(width, height)
                .transparent(false),
        )?;
        conn.flush()?;

        // Create graphics context for blitting
        let gc = conn.generate_id()?;
        conn.inner().create_gc(gc, window.id(), &Default::default())?;
        conn.flush()?;

        // Create renderer and theme
        let theme = Theme::with_font(config.font_family.clone(), config.font_size);
        let show_legend = config.show_legend;
        let renderer = Renderer::new(width, height)?;

        // Create components
        let header = HeaderBar::new(Rect::new(0, 0, width, HEADER_HEIGHT));
        let mut tab_bar = TabBar::new(Rect::new(0, HEADER_HEIGHT as i32, width, TAB_BAR_HEIGHT));

        // Set initial tab from config (--pane flag for garbar integration)
        if let Some(ref pane) = config.default_pane {
            let tab = match pane.as_str() {
                "cpu" => Tab::Cpu,
                "memory" => Tab::Memory,
                "network" => Tab::Network,
                "disk" => Tab::Disk,
                _ => Tab::Cpu,
            };
            tab_bar.set_active(tab);
        }

        let process_list = Self::create_process_list(width, height);

        // Check if daemon is available
        let daemon_available = Self::check_daemon();

        Ok(Self {
            window,
            renderer,
            gc,
            theme,
            header,
            tab_bar,
            process_list,
            should_quit: false,
            width,
            height,
            daemon_available,
            last_refresh: Instant::now() - std::time::Duration::from_secs(10),
            refresh_interval,
            show_legend,
            frozen: false,
            tree_view: false,
            search_mode: false,
            search_query: String::new(),
            show_help: false,
            jump_pattern: String::new(),
            jump_time: None,
            kill_confirm: None,
            detail_pid: None,
            status: None,
            cpu_stats: None,
            memory_stats: None,
            network_stats: Vec::new(),
            disk_stats: Vec::new(),
            temp_stats: None,
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
            network_history: Vec::new(),
            disk_history: Vec::new(),
            processes: Vec::new(),
            process_history: HashMap::new(),
        })
    }

    /// Create process list with correct bounds.
    fn create_process_list(width: u32, height: u32) -> ProcessList {
        // Account for: header + tab bar + section gap + graph label + graph + section gap
        let content_start = HEADER_HEIGHT + TAB_BAR_HEIGHT + SECTION_GAP + 20 + GRAPH_HEIGHT + SECTION_GAP;
        let list_height = height.saturating_sub(content_start);
        let list_width = width.saturating_sub(CONTENT_PADDING * 2);
        ProcessList::new(Rect::new(CONTENT_PADDING as i32, content_start as i32, list_width, list_height))
    }

    /// Check if daemon is available by attempting a connection.
    fn check_daemon() -> bool {
        let path = gartop_ipc::socket_path();
        match UnixStream::connect(&path) {
            Ok(_) => {
                tracing::info!("Daemon available at {}", path.display());
                true
            }
            Err(e) => {
                tracing::warn!("Daemon not available: {}", e);
                false
            }
        }
    }

    /// Send a command to the daemon and get response.
    fn send_command(&self, cmd: &Command) -> Option<Response> {
        let path = gartop_ipc::socket_path();
        let mut stream = UnixStream::connect(&path).ok()?;

        let json = serde_json::to_string(cmd).ok()?;
        writeln!(stream, "{}", json).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        serde_json::from_str(&line).ok()
    }

    /// Refresh data from daemon.
    fn refresh_data(&mut self) {
        let mut any_success = false;

        // Get status
        if let Some(resp) = self.send_command(&Command::Status) {
            any_success = true;
            if resp.success {
                self.status = resp.data.and_then(|d| serde_json::from_value(d).ok());
            }
        }

        // Get CPU stats
        if let Some(resp) = self.send_command(&Command::GetCpu) {
            any_success = true;
            if resp.success {
                self.cpu_stats = resp.data.and_then(|d| serde_json::from_value(d).ok());
            }
        }

        // Get memory stats
        if let Some(resp) = self.send_command(&Command::GetMemory) {
            any_success = true;
            if resp.success {
                self.memory_stats = resp.data.and_then(|d| serde_json::from_value(d).ok());
            }
        }

        // Get CPU history
        if let Some(resp) = self.send_command(&Command::GetCpuHistory { count: Some(60) }) {
            if resp.success {
                self.cpu_history = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get memory history
        if let Some(resp) = self.send_command(&Command::GetMemoryHistory { count: Some(60) }) {
            if resp.success {
                self.memory_history = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get network stats
        if let Some(resp) = self.send_command(&Command::GetNetwork) {
            if resp.success {
                self.network_stats = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get network history
        if let Some(resp) = self.send_command(&Command::GetNetworkHistory { count: Some(60) }) {
            if resp.success {
                self.network_history = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get disk stats
        if let Some(resp) = self.send_command(&Command::GetDisk) {
            if resp.success {
                self.disk_stats = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get disk history
        if let Some(resp) = self.send_command(&Command::GetDiskHistory { count: Some(60) }) {
            if resp.success {
                self.disk_history = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
            }
        }

        // Get temperature stats
        if let Some(resp) = self.send_command(&Command::GetTemperature) {
            if resp.success {
                self.temp_stats = resp.data.and_then(|d| serde_json::from_value(d).ok());
            }
        }

        // Get processes sorted by current tab's resource (skip when frozen)
        if !self.frozen {
            let sort_field = match self.tab_bar.active() {
                Tab::Cpu => SortField::Cpu,
                Tab::Memory => SortField::Memory,
                Tab::Network => SortField::NetConnections,
                Tab::Disk => SortField::DiskTotal,
            };
            if let Some(resp) = self.send_command(&Command::GetProcesses {
                sort_by: Some(sort_field),
                limit: Some(100),
            }) {
                if resp.success {
                    self.processes = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
                    // Sync selection - tracks process by PID across list reorders
                    self.process_list.sync_selection(&self.processes);
                    self.process_list.set_sort(sort_field);

                    // Record per-process history for sparklines
                    for proc in &self.processes {
                        self.process_history
                            .entry(proc.pid)
                            .or_insert_with(ProcessHistory::new)
                            .push(proc.cpu_percent, proc.memory_percent);
                    }
                    // Clean up history for dead processes
                    let active_pids: std::collections::HashSet<i32> =
                        self.processes.iter().map(|p| p.pid).collect();
                    self.process_history.retain(|pid, _| active_pids.contains(pid));
                }
            }
        }

        self.daemon_available = any_success;
        self.last_refresh = Instant::now();
    }

    /// Update header bar with current stats.
    fn update_header(&mut self) {
        let uptime = self.status.as_ref().map(|s| s.uptime_secs).unwrap_or(0);
        let cpu = self.cpu_stats.as_ref().map(|s| s.usage_percent as f32).unwrap_or(0.0);
        let mem = self.memory_stats.as_ref().map(|s| s.usage_percent as f32).unwrap_or(0.0);

        // Sum network rates across all interfaces
        let net_rate: f64 = self.network_stats.iter()
            .map(|s| s.rx_rate + s.tx_rate)
            .sum();

        // Sum disk rates across all devices
        let disk_rate: f64 = self.disk_stats.iter()
            .map(|s| s.read_rate + s.write_rate)
            .sum();

        self.header.update(uptime, cpu, mem, net_rate, disk_rate);

        // Get maximum temperature across all sensors
        let max_temp = self.temp_stats.as_ref().and_then(|ts| {
            ts.sensors.iter()
                .map(|s| s.temp_celsius)
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        });
        self.header.update_temp(max_temp);
    }

    /// Render the entire UI.
    fn render(&mut self) -> Result<()> {
        // Clear background
        self.renderer.fill_rect(
            Rect::new(0, 0, self.width, self.height),
            self.theme.background,
        )?;

        // Render header
        self.header.render(&self.renderer, &self.theme)?;

        // Freeze indicator
        if self.frozen {
            let freeze_style = TextStyle {
                font_family: "monospace".to_string(),
                font_size: 10.0,
                color: self.theme.swap_color,
                ..Default::default()
            };
            self.renderer.text("PAUSED", 80.0, (HEADER_HEIGHT as f64 * 0.4) + 6.0, &freeze_style)?;
        }

        // Render tab bar
        self.tab_bar.render(&self.renderer, &self.theme)?;

        // Render content area
        let content_y = (HEADER_HEIGHT + TAB_BAR_HEIGHT) as i32;
        let content_height = self.height.saturating_sub(HEADER_HEIGHT + TAB_BAR_HEIGHT);

        if !self.daemon_available {
            self.render_not_connected(content_y)?;
        } else {
            self.render_tab_content(content_y, content_height)?;
        }

        // Process detail view (rendered on top of content)
        if let Some(pid) = self.detail_pid {
            if let Some(process) = self.processes.iter().find(|p| p.pid == pid) {
                self.render_detail_view(process)?;
            }
        }

        // Help overlay (rendered on top)
        if self.show_help {
            self.render_help_overlay()?;
        }

        // Kill confirmation overlay (rendered on top of everything)
        if let Some((pid, signal, ref name)) = self.kill_confirm {
            self.render_kill_confirm(pid, signal, name)?;
        }

        self.renderer.flush();
        Ok(())
    }

    /// Render "not connected" message.
    fn render_not_connected(&self, content_y: i32) -> Result<()> {
        let style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 13.0,
            color: self.theme.swap_color,
            ..Default::default()
        };
        let dim_style = TextStyle {
            color: self.theme.text_secondary,
            ..style.clone()
        };

        self.renderer.text("Not connected to daemon", 20.0, content_y as f64 + 40.0, &style)?;
        self.renderer.text("Run: gartop daemon", 20.0, content_y as f64 + 65.0, &dim_style)?;
        Ok(())
    }

    /// Render help overlay with keybindings.
    fn render_help_overlay(&self) -> Result<()> {
        // Semi-transparent backdrop
        let backdrop = Rect::new(0, 0, self.width, self.height);
        self.renderer.fill_rect(backdrop, gartk_core::Color::new(0.0, 0.0, 0.0, 0.75))?;

        // Help box dimensions
        let box_width = 320u32;
        let box_height = 360u32;
        let box_x = (self.width.saturating_sub(box_width)) / 2;
        let box_y = (self.height.saturating_sub(box_height)) / 2;

        // Help box background
        let help_rect = Rect::new(box_x as i32, box_y as i32, box_width, box_height);
        self.renderer.fill_rounded_rect(help_rect, 8.0, self.theme.panel_bg)?;

        // Title
        let title_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 14.0,
            color: self.theme.text,
            ..Default::default()
        };
        let x = box_x as f64 + 16.0;
        let mut y = box_y as f64 + 20.0;
        self.renderer.text("Keyboard Shortcuts", x, y, &title_style)?;

        // Separator
        y += 22.0;
        self.renderer.line(x, y, x + box_width as f64 - 32.0, y, self.theme.border, 1.0)?;
        y += 12.0;

        // Keybindings
        let key_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 10.0,
            color: self.theme.cpu_color,
            ..Default::default()
        };
        let desc_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 10.0,
            color: self.theme.text_secondary,
            ..Default::default()
        };

        // Column offset for descriptions
        let desc_x = x + 90.0;

        let bindings = [
            ("?", "Toggle this help"),
            ("q / Esc", "Quit / clear"),
            ("", ""),
            ("1-4", "Switch tab"),
            ("Tab", "Cycle tabs"),
            ("", ""),
            ("\u{2191} / \u{2193}", "Navigate list"),
            ("Home / End", "First / last"),
            ("PgUp/PgDn", "Jump 10 rows"),
            ("Enter", "Process detail"),
            ("t", "Tree view toggle"),
            ("", ""),
            ("Alt+f", "Freeze list"),
            ("/", "Search filter"),
            ("a-z", "Fuzzy jump"),
            ("", ""),
            ("K", "Kill (SIGTERM)"),
            ("X", "Kill (SIGKILL)"),
            ("r", "Refresh"),
        ];

        for (key, desc) in bindings {
            if key.is_empty() {
                y += 6.0; // Spacer
            } else {
                self.renderer.text(key, x, y, &key_style)?;
                self.renderer.text(desc, desc_x, y, &desc_style)?;
                y += 16.0;
            }
        }

        // Footer
        y = box_y as f64 + box_height as f64 - 30.0;
        let footer_style = TextStyle {
            font_size: 9.0,
            color: self.theme.text_secondary,
            ..desc_style
        };
        self.renderer.text("Press ? or Esc to close", x, y, &footer_style)?;

        Ok(())
    }

    /// Render kill confirmation overlay.
    fn render_kill_confirm(&self, pid: i32, signal: i32, name: &str) -> Result<()> {
        // Semi-transparent backdrop
        let backdrop = Rect::new(0, 0, self.width, self.height);
        self.renderer.fill_rect(backdrop, gartk_core::Color::new(0.0, 0.0, 0.0, 0.6))?;

        // Confirmation box
        let box_width = 280u32;
        let box_height = 80u32;
        let box_x = (self.width.saturating_sub(box_width)) / 2;
        let box_y = (self.height.saturating_sub(box_height)) / 2;

        let confirm_rect = Rect::new(box_x as i32, box_y as i32, box_width, box_height);
        self.renderer.fill_rounded_rect(confirm_rect, 8.0, self.theme.panel_bg)?;

        // Border with warning color
        let border_color = if signal == 9 {
            gartk_core::Color::new(0.9, 0.3, 0.3, 1.0) // Red for SIGKILL
        } else {
            gartk_core::Color::new(0.9, 0.7, 0.3, 1.0) // Orange for SIGTERM
        };
        self.renderer.stroke_rounded_rect(confirm_rect, 8.0, border_color, 2.0)?;

        let x = box_x as f64 + 16.0;
        let mut y = box_y as f64 + 24.0;

        // Signal type
        let signal_name = if signal == 9 { "SIGKILL" } else { "SIGTERM" };
        let title_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 12.0,
            color: self.theme.text,
            ..Default::default()
        };
        let msg = format!("Kill {} (PID {}) with {}?", name, pid, signal_name);
        self.renderer.text(&msg, x, y, &title_style)?;

        // Prompt
        y += 28.0;
        let prompt_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 11.0,
            color: self.theme.text_secondary,
            ..Default::default()
        };
        self.renderer.text("[y] Yes   [n/Esc] Cancel", x, y, &prompt_style)?;

        Ok(())
    }

    /// Render process detail view overlay.
    fn render_detail_view(&self, process: &ProcessInfo) -> Result<()> {
        // Full-screen backdrop
        let backdrop = Rect::new(0, 0, self.width, self.height);
        self.renderer.fill_rect(backdrop, gartk_core::Color::new(0.0, 0.0, 0.0, 0.85))?;

        // Detail panel (slightly smaller than full screen)
        let margin = 24u32;
        let panel_rect = Rect::new(
            margin as i32,
            margin as i32,
            self.width - margin * 2,
            self.height - margin * 2,
        );
        self.renderer.fill_rounded_rect(panel_rect, 8.0, self.theme.panel_bg)?;

        let x = margin as f64 + 20.0;
        let right_col = (self.width / 2) as f64 + 20.0;
        let mut y = margin as f64 + 24.0;

        // Title
        let title_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 14.0,
            color: self.theme.text,
            ..Default::default()
        };
        let title = format!("Process: {} (PID {})", process.name, process.pid);
        self.renderer.text(&title, x, y, &title_style)?;

        // Back hint
        let hint_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 10.0,
            color: self.theme.text_secondary,
            ..Default::default()
        };
        self.renderer.text("[Esc] Back", (self.width - margin - 80) as f64, y, &hint_style)?;

        // Separator
        y += 20.0;
        self.renderer.line(
            x, y,
            (self.width - margin * 2) as f64 + x - 20.0, y,
            self.theme.border, 1.0
        )?;
        y += 16.0;

        // Info labels and values
        let label_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 11.0,
            color: self.theme.text_secondary,
            ..Default::default()
        };
        let value_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 11.0,
            color: self.theme.text,
            ..Default::default()
        };

        // Left column
        let row_height = 20.0;

        // State
        self.renderer.text("State:", x, y, &label_style)?;
        let state_color = match process.state.as_str() {
            "R" => self.theme.cpu_color,     // Running
            "S" => self.theme.memory_color,  // Sleeping
            "Z" => self.theme.swap_color,    // Zombie
            "T" => self.theme.disk_color,    // Stopped
            _ => self.theme.text,
        };
        let state_style = TextStyle { color: state_color, ..value_style.clone() };
        let state_name = match process.state.as_str() {
            "R" => "Running",
            "S" => "Sleeping",
            "D" => "Disk Sleep",
            "Z" => "Zombie",
            "T" => "Stopped",
            "t" => "Tracing",
            "X" => "Dead",
            _ => &process.state,
        };
        self.renderer.text(state_name, x + 80.0, y, &state_style)?;
        y += row_height;

        // User
        self.renderer.text("User:", x, y, &label_style)?;
        self.renderer.text(&process.user, x + 80.0, y, &value_style)?;
        y += row_height;

        // CPU
        self.renderer.text("CPU:", x, y, &label_style)?;
        let cpu_style = TextStyle { color: self.theme.cpu_color, ..value_style.clone() };
        self.renderer.text(&format!("{:.1}%", process.cpu_percent), x + 80.0, y, &cpu_style)?;
        y += row_height;

        // Memory
        self.renderer.text("Memory:", x, y, &label_style)?;
        let mem_style = TextStyle { color: self.theme.memory_color, ..value_style.clone() };
        self.renderer.text(
            &format!("{} ({:.1}%)", format_bytes(process.rss), process.memory_percent),
            x + 80.0, y, &mem_style
        )?;
        y += row_height;

        // Virtual
        self.renderer.text("Virtual:", x, y, &label_style)?;
        self.renderer.text(&format_bytes(process.vsize), x + 80.0, y, &value_style)?;

        // Right column - reset y
        y = margin as f64 + 24.0 + 20.0 + 16.0;

        // Disk I/O
        self.renderer.text("Disk Read:", right_col, y, &label_style)?;
        let disk_style = TextStyle { color: self.theme.disk_color, ..value_style.clone() };
        self.renderer.text(&format_rate(process.io_read_rate), right_col + 90.0, y, &disk_style)?;
        y += row_height;

        self.renderer.text("Disk Write:", right_col, y, &label_style)?;
        self.renderer.text(&format_rate(process.io_write_rate), right_col + 90.0, y, &disk_style)?;
        y += row_height;

        // Network
        self.renderer.text("Net Conn:", right_col, y, &label_style)?;
        let net_style = TextStyle { color: self.theme.network_color, ..value_style.clone() };
        self.renderer.text(
            &format!("{} ({} TCP, {} UDP)", process.net_connections, process.net_tcp, process.net_udp),
            right_col + 90.0, y, &net_style
        )?;
        y += row_height;

        self.renderer.text("Net Rate:", right_col, y, &label_style)?;
        self.renderer.text(
            &format!("↓{} ↑{}", format_rate(process.net_rx_rate), format_rate(process.net_tx_rate)),
            right_col + 90.0, y, &net_style
        )?;

        // Command line section
        y = margin as f64 + 24.0 + 20.0 + 16.0 + row_height * 6.0;
        self.renderer.text("Command:", x, y, &label_style)?;
        y += row_height;

        // Command line (truncate if too long)
        let cmd_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 10.0,
            color: self.theme.text,
            ..Default::default()
        };
        let max_cmd_len = ((self.width - margin * 2 - 40) / 6) as usize; // Approximate char width
        let cmdline = if process.cmdline.len() > max_cmd_len {
            format!("{}...", &process.cmdline[..max_cmd_len - 3])
        } else if process.cmdline.is_empty() {
            format!("[{}]", process.name)
        } else {
            process.cmdline.clone()
        };
        self.renderer.text(&cmdline, x, y, &cmd_style)?;

        // Actions footer
        let footer_y = (self.height - margin - 40) as f64;
        self.renderer.line(
            x, footer_y - 8.0,
            (self.width - margin * 2) as f64 + x - 20.0, footer_y - 8.0,
            self.theme.border, 1.0
        )?;

        let action_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 10.0,
            color: self.theme.cpu_color,
            ..Default::default()
        };
        self.renderer.text("[K] Kill (SIGTERM)   [X] Kill (SIGKILL)", x, footer_y, &action_style)?;

        // History sparklines (if we have history for this process)
        if let Some(history) = self.process_history.get(&process.pid) {
            if history.cpu.len() >= 2 {
                let spark_y = footer_y - 90.0;
                let spark_height = 50.0;
                let spark_width = ((self.width - margin * 2) as f64 - 40.0) / 2.0 - 10.0;

                // CPU sparkline
                self.renderer.text("CPU History:", x, spark_y - 14.0, &label_style)?;
                self.render_sparkline(
                    x, spark_y, spark_width, spark_height,
                    &history.cpu, self.theme.cpu_color,
                )?;

                // Memory sparkline
                let mem_x = x + spark_width + 20.0;
                self.renderer.text("Memory History:", mem_x, spark_y - 14.0, &label_style)?;
                self.render_sparkline(
                    mem_x, spark_y, spark_width, spark_height,
                    &history.memory, self.theme.memory_color,
                )?;
            }
        }

        Ok(())
    }

    /// Render a sparkline graph (small inline chart).
    fn render_sparkline(&self, x: f64, y: f64, w: f64, h: f64, values: &[f64], color: gartk_core::Color) -> Result<()> {
        if values.is_empty() || w <= 0.0 || h <= 0.0 {
            return Ok(());
        }

        let ctx = self.renderer.context()?;

        // Background
        ctx.set_source_rgba(
            self.theme.graph_bg.r,
            self.theme.graph_bg.g,
            self.theme.graph_bg.b,
            self.theme.graph_bg.a,
        );
        ctx.rectangle(x, y, w, h);
        let _ = ctx.fill();

        // Find max for scaling (at least 1% to avoid division by zero)
        let max_val = values.iter().cloned().fold(1.0_f64, f64::max);
        let scale = h / max_val.max(1.0);

        let step = w / (values.len().saturating_sub(1).max(1)) as f64;

        // Draw filled area
        ctx.set_source_rgba(color.r, color.g, color.b, 0.3);
        ctx.move_to(x, y + h);
        for (i, &val) in values.iter().enumerate() {
            let px = x + i as f64 * step;
            let py = y + h - (val * scale);
            ctx.line_to(px, py);
        }
        ctx.line_to(x + (values.len() - 1) as f64 * step, y + h);
        ctx.close_path();
        let _ = ctx.fill();

        // Draw line
        ctx.set_source_rgba(color.r, color.g, color.b, 1.0);
        ctx.set_line_width(1.5);
        let mut first = true;
        for (i, &val) in values.iter().enumerate() {
            let px = x + i as f64 * step;
            let py = y + h - (val * scale);
            if first {
                ctx.move_to(px, py);
                first = false;
            } else {
                ctx.line_to(px, py);
            }
        }
        let _ = ctx.stroke();

        Ok(())
    }

    /// Render the content for the active tab.
    fn render_tab_content(&self, content_y: i32, _content_height: u32) -> Result<()> {
        let graph_width = self.width - (CONTENT_PADDING * 2);

        // Get Cairo context for graph rendering
        let ctx = self.renderer.context()?;
        let graph = LineGraph {
            fill_opacity: 0.25,
            show_legend: self.show_legend,
            ..LineGraph::default()
        };

        let mut y = content_y + SECTION_GAP as i32;

        match self.tab_bar.active() {
            Tab::Cpu => {
                // CPU label
                let cpu_label = if let Some(cpu) = &self.cpu_stats {
                    format!("CPU: {:.1}%", cpu.usage_percent)
                } else {
                    "CPU: --".to_string()
                };
                self.renderer.text(
                    &cpu_label,
                    CONTENT_PADDING as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 12.0,
                        color: self.theme.cpu_color,
                        ..Default::default()
                    },
                )?;

                // Per-core info on right
                if let Some(cpu) = &self.cpu_stats {
                    let cores = cpu.per_core.len();
                    let core_info = format!("{} cores", cores);
                    self.renderer.text(
                        &core_info,
                        (self.width - 80) as f64,
                        y as f64 + 14.0,
                        &TextStyle {
                            font_family: "monospace".to_string(),
                            font_size: 10.0,
                            color: self.theme.text_secondary,
                            ..Default::default()
                        },
                    )?;
                }
                y += 20;

                // CPU Graph
                let graph_rect = Rect::new(CONTENT_PADDING as i32, y, graph_width, GRAPH_HEIGHT);
                let mut cpu_series = DataSeries::new("CPU", self.theme.cpu_color);
                cpu_series.set_values(self.cpu_history.iter().map(|s| s.usage_percent).collect());
                graph.render(&ctx, graph_rect, &[cpu_series], &self.theme);

                // Per-core bars below the graph
                if let Some(cpu) = &self.cpu_stats {
                    let bar_y = y + GRAPH_HEIGHT as i32 + 8;
                    let bar_height = 12.0;
                    let bar_spacing = 4.0;
                    let label_width = 50.0;
                    let max_bar_width = (graph_width as f64 - label_width - 8.0).max(100.0);

                    let label_style = TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 9.0,
                        color: self.theme.text_secondary,
                        ..Default::default()
                    };

                    let value_style = TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 9.0,
                        color: self.theme.text,
                        ..Default::default()
                    };

                    for (i, usage) in cpu.per_core.iter().enumerate() {
                        let core_y = bar_y as f64 + (i as f64 * (bar_height + bar_spacing));

                        // Core label
                        let label = format!("Core {}", i);
                        self.renderer.text(
                            &label,
                            CONTENT_PADDING as f64,
                            core_y + 9.0,
                            &label_style,
                        )?;

                        // Background bar
                        let bar_x = CONTENT_PADDING as f64 + label_width;
                        let bar_rect = Rect::new(
                            bar_x as i32,
                            core_y as i32,
                            max_bar_width as u32,
                            bar_height as u32,
                        );
                        self.renderer.fill_rect(bar_rect, self.theme.graph_bg)?;

                        // Usage bar
                        let usage_width = (max_bar_width * (*usage / 100.0)).max(1.0);
                        let usage_rect = Rect::new(
                            bar_x as i32,
                            core_y as i32,
                            usage_width as u32,
                            bar_height as u32,
                        );
                        self.renderer.fill_rect(usage_rect, self.theme.cpu_color)?;

                        // Percentage value
                        let pct_text = format!("{:.0}%", usage);
                        self.renderer.text(
                            &pct_text,
                            bar_x + max_bar_width + 4.0,
                            core_y + 9.0,
                            &value_style,
                        )?;
                    }
                }
            }

            Tab::Memory => {
                // Memory label
                let mem_label = if let Some(mem) = &self.memory_stats {
                    format!(
                        "Memory: {:.1}% ({} / {})",
                        mem.usage_percent,
                        format_bytes(mem.used),
                        format_bytes(mem.total)
                    )
                } else {
                    "Memory: --".to_string()
                };
                self.renderer.text(
                    &mem_label,
                    CONTENT_PADDING as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 12.0,
                        color: self.theme.memory_color,
                        ..Default::default()
                    },
                )?;

                // Swap info on right
                if let Some(mem) = &self.memory_stats {
                    let swap_pct = if mem.swap_total > 0 {
                        (mem.swap_used as f64 / mem.swap_total as f64) * 100.0
                    } else {
                        0.0
                    };
                    let swap_info = format!("Swap: {:.1}%", swap_pct);
                    self.renderer.text(
                        &swap_info,
                        (self.width - 80) as f64,
                        y as f64 + 14.0,
                        &TextStyle {
                            font_family: "monospace".to_string(),
                            font_size: 10.0,
                            color: self.theme.swap_color,
                            ..Default::default()
                        },
                    )?;
                }
                y += 20;

                // Memory Graph
                let graph_rect = Rect::new(CONTENT_PADDING as i32, y, graph_width, GRAPH_HEIGHT);
                let mut mem_series = DataSeries::new("Memory", self.theme.memory_color);
                let mut swap_series = DataSeries::new("Swap", self.theme.swap_color);
                mem_series.set_values(self.memory_history.iter().map(|s| s.usage_percent).collect());
                swap_series.set_values(self.memory_history.iter().map(|s| {
                    if s.swap_total > 0 {
                        (s.swap_used as f64 / s.swap_total as f64) * 100.0
                    } else {
                        0.0
                    }
                }).collect());
                graph.render(&ctx, graph_rect, &[mem_series, swap_series], &self.theme);
            }

            Tab::Network => {
                // Network label - show total rx/tx rates across all interfaces
                let (total_rx, total_tx) = self.network_stats.iter().fold((0.0, 0.0), |acc, s| {
                    (acc.0 + s.rx_rate, acc.1 + s.tx_rate)
                });
                let net_label = format!(
                    "Network: {} \u{2193} / {} \u{2191}",
                    format_rate(total_rx),
                    format_rate(total_tx)
                );
                self.renderer.text(
                    &net_label,
                    CONTENT_PADDING as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 12.0,
                        color: self.theme.network_color,
                        ..Default::default()
                    },
                )?;

                // Interface count on right
                let iface_info = format!("{} interfaces", self.network_stats.len());
                self.renderer.text(
                    &iface_info,
                    (self.width - 100) as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 10.0,
                        color: self.theme.text_secondary,
                        ..Default::default()
                    },
                )?;
                y += 20;

                // Network Graph - show rx/tx rates over time
                let graph_rect = Rect::new(CONTENT_PADDING as i32, y, graph_width, GRAPH_HEIGHT);
                let mut rx_series = DataSeries::new("Download", self.theme.network_color);
                let mut tx_series = DataSeries::new("Upload", self.theme.swap_color);

                // Calculate max rate for scaling
                let max_rate = self.network_history.iter()
                    .flat_map(|stats| stats.iter().map(|s| s.rx_rate.max(s.tx_rate)))
                    .fold(1.0, f64::max);

                // Convert rates to percentage of max for graph
                rx_series.set_values(
                    self.network_history.iter()
                        .map(|stats| {
                            let total: f64 = stats.iter().map(|s| s.rx_rate).sum();
                            (total / max_rate) * 100.0
                        })
                        .collect()
                );
                tx_series.set_values(
                    self.network_history.iter()
                        .map(|stats| {
                            let total: f64 = stats.iter().map(|s| s.tx_rate).sum();
                            (total / max_rate) * 100.0
                        })
                        .collect()
                );
                graph.render(&ctx, graph_rect, &[rx_series, tx_series], &self.theme);
            }

            Tab::Disk => {
                // Disk label - show total read/write rates
                let (total_read, total_write) = self.disk_stats.iter().fold((0.0, 0.0), |acc, s| {
                    (acc.0 + s.read_rate, acc.1 + s.write_rate)
                });
                let disk_label = format!(
                    "Disk: {} read / {} write",
                    format_rate(total_read),
                    format_rate(total_write)
                );
                self.renderer.text(
                    &disk_label,
                    CONTENT_PADDING as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 12.0,
                        color: self.theme.disk_color,
                        ..Default::default()
                    },
                )?;

                // Device count on right
                let dev_info = format!("{} devices", self.disk_stats.len());
                self.renderer.text(
                    &dev_info,
                    (self.width - 100) as f64,
                    y as f64 + 14.0,
                    &TextStyle {
                        font_family: "monospace".to_string(),
                        font_size: 10.0,
                        color: self.theme.text_secondary,
                        ..Default::default()
                    },
                )?;
                y += 20;

                // Disk Graph - show read/write rates over time
                let graph_rect = Rect::new(CONTENT_PADDING as i32, y, graph_width, GRAPH_HEIGHT);
                let mut read_series = DataSeries::new("Read", self.theme.disk_color);
                let mut write_series = DataSeries::new("Write", self.theme.swap_color);

                // Calculate max rate for scaling
                let max_rate = self.disk_history.iter()
                    .flat_map(|stats| stats.iter().map(|s| s.read_rate.max(s.write_rate)))
                    .fold(1.0, f64::max);

                // Convert rates to percentage of max for graph
                read_series.set_values(
                    self.disk_history.iter()
                        .map(|stats| {
                            let total: f64 = stats.iter().map(|s| s.read_rate).sum();
                            (total / max_rate) * 100.0
                        })
                        .collect()
                );
                write_series.set_values(
                    self.disk_history.iter()
                        .map(|stats| {
                            let total: f64 = stats.iter().map(|s| s.write_rate).sum();
                            (total / max_rate) * 100.0
                        })
                        .collect()
                );
                graph.render(&ctx, graph_rect, &[read_series, write_series], &self.theme);
            }
        }

        // Search bar (above process list when active)
        if self.search_mode || !self.search_query.is_empty() {
            let search_y = (HEADER_HEIGHT + TAB_BAR_HEIGHT + SECTION_GAP + 20 + GRAPH_HEIGHT + SECTION_GAP) as i32 - 20;
            let search_style = TextStyle {
                font_family: "monospace".to_string(),
                font_size: 11.0,
                color: if self.search_mode { self.theme.text } else { self.theme.text_secondary },
                ..Default::default()
            };
            let search_text = format!("/{}{}", self.search_query, if self.search_mode { "_" } else { "" });
            self.renderer.text(&search_text, CONTENT_PADDING as f64, search_y as f64, &search_style)?;
        }

        // Jump pattern indicator (right side, above process list)
        if !self.jump_pattern.is_empty() {
            let jump_y = (HEADER_HEIGHT + TAB_BAR_HEIGHT + SECTION_GAP + 20 + GRAPH_HEIGHT + SECTION_GAP) as i32 - 20;
            let jump_style = TextStyle {
                font_family: "monospace".to_string(),
                font_size: 11.0,
                color: self.theme.network_color,
                ..Default::default()
            };
            let jump_text = format!("jump: {}", self.jump_pattern);
            // Position on right side
            let text_width = self.renderer.measure_text(&jump_text, &jump_style)?.width as f64;
            let jump_x = (self.width as f64) - CONTENT_PADDING as f64 - text_width;
            self.renderer.text(&jump_text, jump_x, jump_y as f64, &jump_style)?;
        }

        // Filter processes if search query is active
        let display_processes: Vec<ProcessInfo> = if self.search_query.is_empty() {
            self.processes.clone()
        } else {
            let query = self.search_query.to_lowercase();
            self.processes
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&query) ||
                           p.cmdline.to_lowercase().contains(&query))
                .cloned()
                .collect()
        };

        // Render process list with filtered processes
        self.process_list.render(&self.renderer, &self.theme, &display_processes, self.tree_view)?;

        Ok(())
    }

    /// Blit surface to window.
    fn blit(&mut self) -> Result<()> {
        let data = {
            let surface = self.renderer.surface_mut();
            let data_ref = surface
                .data()
                .map_err(|e| anyhow::anyhow!("Failed to get surface data: {}", e))?;
            data_ref.to_vec()
        };

        let conn = self.window.connection();
        conn.inner().put_image(
            ImageFormat::Z_PIXMAP,
            self.window.id(),
            self.gc,
            self.width as u16,
            self.height as u16,
            0,
            0,
            0,
            self.window.depth(),
            &data,
        )?;
        conn.flush()?;

        Ok(())
    }

    /// Handle window resize.
    fn handle_resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }

        self.width = width;
        self.height = height;
        self.renderer.resize(width, height)?;

        // Update component bounds
        self.header = HeaderBar::new(Rect::new(0, 0, width, HEADER_HEIGHT));
        self.tab_bar.set_bounds(Rect::new(0, HEADER_HEIGHT as i32, width, TAB_BAR_HEIGHT));
        self.process_list = Self::create_process_list(width, height);
        self.process_list.set_process_count(self.processes.len());

        Ok(())
    }

    /// Handle mouse click.
    fn handle_click(&mut self, pos: Point) -> bool {
        // Check tab bar
        if let Some(tab) = self.tab_bar.on_click(pos) {
            if tab != self.tab_bar.active() {
                self.tab_bar.set_active(tab);
                // Re-sort cached processes locally (no daemon call)
                let sort_field = match tab {
                    Tab::Cpu => SortField::Cpu,
                    Tab::Memory => SortField::Memory,
                    Tab::Network => SortField::NetConnections,
                    Tab::Disk => SortField::DiskTotal,
                };
                self.sort_processes(sort_field);
                return true;
            }
        }

        // Check process list
        if self.process_list.on_click(pos, &self.processes).is_some() {
            return true;
        }

        false
    }

    /// Sort processes in place by the given field (no cloning).
    fn sort_processes(&mut self, sort_field: SortField) {
        match sort_field {
            SortField::Cpu => self.processes.sort_by(|a, b| {
                b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::Memory => self.processes.sort_by(|a, b| {
                b.memory_percent.partial_cmp(&a.memory_percent).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::DiskRead => self.processes.sort_by(|a, b| {
                b.io_read_rate.partial_cmp(&a.io_read_rate).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::DiskWrite => self.processes.sort_by(|a, b| {
                b.io_write_rate.partial_cmp(&a.io_write_rate).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::DiskTotal => self.processes.sort_by(|a, b| {
                let a_total = a.io_read_rate + a.io_write_rate;
                let b_total = b.io_read_rate + b.io_write_rate;
                b_total.partial_cmp(&a_total).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::NetConnections => self.processes.sort_by(|a, b| {
                b.net_connections.cmp(&a.net_connections)
            }),
            SortField::NetTcp => self.processes.sort_by(|a, b| {
                b.net_tcp.cmp(&a.net_tcp)
            }),
            SortField::NetBandwidth => self.processes.sort_by(|a, b| {
                let a_total = a.net_rx_rate + a.net_tx_rate;
                let b_total = b.net_rx_rate + b.net_tx_rate;
                b_total.partial_cmp(&a_total).unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::Pid => self.processes.sort_by_key(|p| p.pid),
            SortField::Name => self.processes.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        self.process_list.set_sort(sort_field);
    }

    /// Handle scroll.
    fn handle_scroll(&mut self, delta: i32) -> bool {
        self.process_list.on_scroll(delta);
        true
    }

    /// Kill a process by PID.
    fn kill_process(&mut self, pid: i32, signal: i32) {
        tracing::info!("Sending signal {} to process {}", signal, pid);
        if let Some(resp) = self.send_command(&Command::KillProcess {
            pid,
            signal: Some(signal),
        }) {
            if resp.success {
                tracing::info!("Process {} killed", pid);
                // Clear selection and trigger refresh
                self.process_list.clear_selection();
                self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
            } else {
                tracing::warn!("Failed to kill process {}: {:?}", pid, resp.error);
            }
        }
    }

    /// Handle type-to-jump fuzzy matching.
    fn handle_fuzzy_jump(&mut self, c: char) {
        // Add character to pattern
        self.jump_pattern.push(c);
        self.jump_time = Some(Instant::now());

        // Find first fuzzy match
        if let Some(idx) = self.find_fuzzy_match(&self.jump_pattern) {
            // Select and scroll to the match
            if idx < self.processes.len() {
                let pid = self.processes[idx].pid;
                self.process_list.select_by_index(idx, pid);
            }
        }
    }

    /// Find first process matching fuzzy pattern.
    /// Returns index of first match or None.
    fn find_fuzzy_match(&self, pattern: &str) -> Option<usize> {
        let pattern_lower = pattern.to_lowercase();

        for (idx, proc) in self.processes.iter().enumerate() {
            if fuzzy_match(&proc.name.to_lowercase(), &pattern_lower) {
                return Some(idx);
            }
        }
        None
    }

    /// Run the GUI event loop.
    pub fn run(mut self) -> Result<()> {
        let config = EventLoopConfig {
            fps: 30,
            continuous_redraw: false,
        };
        let mut event_loop = EventLoop::new(&self.window, config)?;

        event_loop.run(|ev_loop, event| {
            match event {
                InputEvent::Expose => {
                    ev_loop.request_redraw();
                }

                InputEvent::Resize { width, height } => {
                    if let Err(e) = self.handle_resize(width, height) {
                        tracing::error!("Resize error: {}", e);
                    }
                    ev_loop.request_redraw();
                }

                InputEvent::MousePress(mouse_event) => {
                    let pos = Point::new(mouse_event.position.x, mouse_event.position.y);
                    if self.handle_click(pos) {
                        ev_loop.request_redraw();
                    }
                }

                InputEvent::MouseMove(mouse_event) => {
                    let pos = Point::new(mouse_event.position.x, mouse_event.position.y);
                    if self.tab_bar.on_mouse_move(pos) {
                        ev_loop.request_redraw();
                    }
                }

                InputEvent::Scroll(scroll_event) => {
                    let delta = if scroll_event.delta_y > 0 { -1 } else { 1 };
                    if self.handle_scroll(delta) {
                        ev_loop.request_redraw();
                    }
                }

                InputEvent::Key(key_event) if key_event.pressed => {
                    // Kill confirmation mode - y confirms, n/Esc cancels
                    if let Some((pid, signal, _)) = self.kill_confirm.take() {
                        match key_event.key {
                            Key::Char('y') | Key::Char('Y') => {
                                self.kill_process(pid, signal);
                            }
                            _ => {} // Any other key cancels
                        }
                        ev_loop.request_redraw();
                    }
                    // Help overlay - only ? and Escape close it
                    else if self.show_help {
                        match key_event.key {
                            Key::Char('?') | Key::Escape => {
                                self.show_help = false;
                                ev_loop.request_redraw();
                            }
                            _ => {} // Ignore other keys when help is shown
                        }
                    }
                    // Search mode input handling
                    else if self.search_mode {
                        match key_event.key {
                            Key::Escape => {
                                if self.search_query.is_empty() {
                                    self.search_mode = false;
                                } else {
                                    self.search_query.clear();
                                }
                                ev_loop.request_redraw();
                            }
                            Key::Return => {
                                self.search_mode = false;
                                ev_loop.request_redraw();
                            }
                            Key::Backspace => {
                                self.search_query.pop();
                                ev_loop.request_redraw();
                            }
                            Key::Char(c) => {
                                if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                                    self.search_query.push(c);
                                    ev_loop.request_redraw();
                                }
                            }
                            _ => {}
                        }
                    }
                    // Detail view mode
                    else if self.detail_pid.is_some() {
                        match key_event.key {
                            Key::Escape => {
                                self.detail_pid = None;
                                ev_loop.request_redraw();
                            }
                            Key::Char('K') => {
                                if let Some(pid) = self.detail_pid {
                                    let name = self.processes.iter()
                                        .find(|p| p.pid == pid)
                                        .map(|p| p.name.clone())
                                        .unwrap_or_else(|| format!("PID {}", pid));
                                    self.kill_confirm = Some((pid, 15, name));
                                    ev_loop.request_redraw();
                                }
                            }
                            Key::Char('X') => {
                                if let Some(pid) = self.detail_pid {
                                    let name = self.processes.iter()
                                        .find(|p| p.pid == pid)
                                        .map(|p| p.name.clone())
                                        .unwrap_or_else(|| format!("PID {}", pid));
                                    self.kill_confirm = Some((pid, 9, name));
                                    ev_loop.request_redraw();
                                }
                            }
                            _ => {}
                        }
                    } else {
                        // Normal mode key handling
                        match key_event.key {
                            Key::Escape => {
                                if !self.jump_pattern.is_empty() {
                                    // Clear jump pattern first
                                    self.jump_pattern.clear();
                                    self.jump_time = None;
                                    ev_loop.request_redraw();
                                } else if !self.search_query.is_empty() {
                                    // Clear search filter second
                                    self.search_query.clear();
                                    ev_loop.request_redraw();
                                } else {
                                    self.should_quit = true;
                                }
                            }
                            Key::Char('q') => {
                                self.should_quit = true;
                            }
                            Key::Char('?') => {
                                self.show_help = true;
                                ev_loop.request_redraw();
                            }
                            Key::Char('/') => {
                                self.search_mode = true;
                                ev_loop.request_redraw();
                            }
                            Key::Char('r') => {
                                self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                            }
                            Key::Char('t') => {
                                self.tree_view = !self.tree_view;
                                ev_loop.request_redraw();
                            }
                            Key::Char('1') => {
                                self.tab_bar.set_active(Tab::Cpu);
                                self.sort_processes(SortField::Cpu);
                                ev_loop.request_redraw();
                            }
                            Key::Char('2') => {
                                self.tab_bar.set_active(Tab::Memory);
                                self.sort_processes(SortField::Memory);
                                ev_loop.request_redraw();
                            }
                            Key::Char('3') => {
                                self.tab_bar.set_active(Tab::Network);
                                self.sort_processes(SortField::NetConnections);
                                ev_loop.request_redraw();
                            }
                            Key::Char('4') => {
                                self.tab_bar.set_active(Tab::Disk);
                                self.sort_processes(SortField::DiskTotal);
                                ev_loop.request_redraw();
                            }
                            Key::Tab => {
                                let new_tab = match self.tab_bar.active() {
                                    Tab::Cpu => Tab::Memory,
                                    Tab::Memory => Tab::Network,
                                    Tab::Network => Tab::Disk,
                                    Tab::Disk => Tab::Cpu,
                                };
                                self.tab_bar.set_active(new_tab);
                                let sort_field = match new_tab {
                                    Tab::Cpu => SortField::Cpu,
                                    Tab::Memory => SortField::Memory,
                                    Tab::Network => SortField::NetConnections,
                                    Tab::Disk => SortField::DiskTotal,
                                };
                                self.sort_processes(sort_field);
                                ev_loop.request_redraw();
                            }
                            // Freeze mode toggle (Alt+f)
                            Key::Char('f') if key_event.modifiers.alt => {
                                self.frozen = !self.frozen;
                                ev_loop.request_redraw();
                            }
                            // Process list navigation (arrow keys only - j/k go to fuzzy jump)
                            Key::Down => {
                                self.process_list.select_next(&self.processes);
                                ev_loop.request_redraw();
                            }
                            Key::Up => {
                                self.process_list.select_prev(&self.processes);
                                ev_loop.request_redraw();
                            }
                            // Backspace - delete last character from jump pattern
                            Key::Backspace if !self.jump_pattern.is_empty() => {
                                self.jump_pattern.pop();
                                self.jump_time = Some(Instant::now());
                                // Re-run match with shorter pattern
                                if !self.jump_pattern.is_empty() {
                                    if let Some(idx) = self.find_fuzzy_match(&self.jump_pattern.clone()) {
                                        if idx < self.processes.len() {
                                            let pid = self.processes[idx].pid;
                                            self.process_list.select_by_index(idx, pid);
                                        }
                                    }
                                }
                                ev_loop.request_redraw();
                            }
                            Key::Home => {
                                self.process_list.select_first(&self.processes);
                                ev_loop.request_redraw();
                            }
                            Key::End => {
                                self.process_list.select_last(&self.processes);
                                ev_loop.request_redraw();
                            }
                            Key::PageDown => {
                                // Move selection down by visible rows
                                for _ in 0..10 {
                                    self.process_list.select_next(&self.processes);
                                }
                                ev_loop.request_redraw();
                            }
                            Key::PageUp => {
                                // Move selection up by visible rows
                                for _ in 0..10 {
                                    self.process_list.select_prev(&self.processes);
                                }
                                ev_loop.request_redraw();
                            }
                            // Enter - open process detail view
                            Key::Return => {
                                if let Some(pid) = self.process_list.selected_pid() {
                                    self.detail_pid = Some(pid);
                                    ev_loop.request_redraw();
                                }
                            }
                            // Kill selected process (K = SIGTERM, X = SIGKILL) - with confirmation
                            Key::Char('K') => {
                                if let Some(pid) = self.process_list.selected_pid() {
                                    let name = self.processes.iter()
                                        .find(|p| p.pid == pid)
                                        .map(|p| p.name.clone())
                                        .unwrap_or_else(|| format!("PID {}", pid));
                                    self.kill_confirm = Some((pid, 15, name));
                                    ev_loop.request_redraw();
                                }
                            }
                            Key::Char('X') => {
                                if let Some(pid) = self.process_list.selected_pid() {
                                    let name = self.processes.iter()
                                        .find(|p| p.pid == pid)
                                        .map(|p| p.name.clone())
                                        .unwrap_or_else(|| format!("PID {}", pid));
                                    self.kill_confirm = Some((pid, 9, name));
                                    ev_loop.request_redraw();
                                }
                            }
                            // Type-to-jump fuzzy matching (lowercase letters only, no modifiers)
                            Key::Char(c) if c.is_ascii_lowercase() && !key_event.modifiers.alt && !key_event.modifiers.ctrl => {
                                self.handle_fuzzy_jump(c);
                                ev_loop.request_redraw();
                            }
                            _ => {}
                        }
                    }
                }

                InputEvent::CloseRequested => {
                    self.should_quit = true;
                }

                InputEvent::Idle => {
                    // Clear stale jump pattern (1.5s timeout)
                    if let Some(t) = self.jump_time {
                        if t.elapsed().as_millis() > 1500 && !self.jump_pattern.is_empty() {
                            self.jump_pattern.clear();
                            self.jump_time = None;
                            ev_loop.request_redraw();
                        }
                    }

                    if self.last_refresh.elapsed().as_secs_f64() >= self.refresh_interval {
                        if self.daemon_available {
                            self.refresh_data();
                            self.update_header();
                            ev_loop.request_redraw();
                        } else {
                            self.daemon_available = Self::check_daemon();
                            if self.daemon_available {
                                ev_loop.request_redraw();
                            }
                            self.last_refresh = Instant::now();
                        }
                    }
                }

                _ => {}
            }

            if ev_loop.needs_redraw() {
                if let Err(e) = self.render() {
                    tracing::error!("Render error: {}", e);
                }
                if let Err(e) = self.blit() {
                    tracing::error!("Blit error: {}", e);
                }
                ev_loop.redraw_done();
            }

            Ok(!self.should_quit)
        })?;

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.window.connection().inner().free_gc(self.gc);
    }
}

/// Format bytes to human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format rate (bytes/second) to human-readable string.
fn format_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_sec >= GIB {
        format!("{:.1} GiB/s", bytes_per_sec / GIB)
    } else if bytes_per_sec >= MIB {
        format!("{:.1} MiB/s", bytes_per_sec / MIB)
    } else if bytes_per_sec >= KIB {
        format!("{:.1} KiB/s", bytes_per_sec / KIB)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

/// Fuzzy match: check if all characters of pattern appear in target in order.
/// Example: "grtrm" matches "garterm" because g-a-r-t-e-r-m contains g-r-t-r-m in order.
fn fuzzy_match(target: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();

    for c in target.chars() {
        if let Some(&p) = pattern_chars.peek() {
            if c == p {
                pattern_chars.next();
            }
        } else {
            // All pattern chars matched
            return true;
        }
    }

    // Check if all pattern chars were consumed
    pattern_chars.peek().is_none()
}
