//! GUI application state and event loop

use super::{graph::{DataSeries, LineGraph}, header::HeaderBar, theme::Theme};
use anyhow::Result;
use gartk_core::{InputEvent, Key, Rect};
use gartk_render::Renderer;
use gartk_x11::{Connection, EventLoop, EventLoopConfig, Window, WindowConfig};
use gartop_ipc::{Command, CpuStats, MemoryStats, Response, StatusInfo};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

/// Header bar height.
const HEADER_HEIGHT: u32 = 36;

/// Default window dimensions.
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

/// Data refresh interval (seconds).
const REFRESH_INTERVAL: f64 = 1.0;

/// GUI application.
pub struct App {
    window: Window,
    renderer: Renderer,
    gc: u32,
    theme: Theme,
    header: HeaderBar,
    should_quit: bool,
    width: u32,
    height: u32,
    daemon_available: bool,
    last_refresh: Instant,
    status: Option<StatusInfo>,
    cpu_stats: Option<CpuStats>,
    memory_stats: Option<MemoryStats>,
    cpu_history: Vec<CpuStats>,
    memory_history: Vec<MemoryStats>,
}

impl App {
    /// Create a new GUI application.
    pub fn new() -> Result<Self> {
        let conn = Connection::connect(None)?;

        // Get primary monitor for centering
        let monitor = gartk_x11::primary_monitor(&conn)?;

        let width = DEFAULT_WIDTH.min(monitor.rect.width);
        let height = DEFAULT_HEIGHT.min(monitor.rect.height);
        let x = monitor.rect.x + (monitor.rect.width as i32 - width as i32) / 2;
        let y = monitor.rect.y + (monitor.rect.height as i32 - height as i32) / 2;

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

        // Create renderer
        let theme = Theme::default();
        let renderer = Renderer::new(width, height)?;

        // Create header bar
        let header = HeaderBar::new(Rect::new(0, 0, width, HEADER_HEIGHT));

        // Check if daemon is available
        let daemon_available = Self::check_daemon();

        Ok(Self {
            window,
            renderer,
            gc,
            theme,
            header,
            should_quit: false,
            width,
            height,
            daemon_available,
            last_refresh: Instant::now() - std::time::Duration::from_secs(10), // force immediate refresh
            status: None,
            cpu_stats: None,
            memory_stats: None,
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
        })
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
    /// Creates a fresh connection for each command since daemon closes after response.
    fn send_command(&self, cmd: &Command) -> Option<Response> {
        let path = gartop_ipc::socket_path();
        let mut stream = UnixStream::connect(&path).ok()?;

        // Send command
        let json = serde_json::to_string(cmd).ok()?;
        writeln!(stream, "{}", json).ok()?;
        stream.flush().ok()?;

        // Read response
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

        // Update daemon availability based on whether any command succeeded
        self.daemon_available = any_success;
        self.last_refresh = Instant::now();
    }

    /// Update header bar with current stats.
    fn update_header(&mut self) {
        let uptime = self.status.as_ref().map(|s| s.uptime_secs).unwrap_or(0);
        let cpu = self.cpu_stats.as_ref().map(|s| s.usage_percent as f32).unwrap_or(0.0);
        let mem = self.memory_stats.as_ref().map(|s| s.usage_percent as f32).unwrap_or(0.0);
        self.header.update(uptime, cpu, mem);
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

        // Render main content area
        let content_y = HEADER_HEIGHT as i32;
        let content_height = self.height.saturating_sub(HEADER_HEIGHT);
        let content_rect = Rect::new(0, content_y, self.width, content_height);

        self.renderer.fill_rect(content_rect, self.theme.panel_bg)?;

        // Show connection status or stats
        self.render_content(content_rect)?;

        self.renderer.flush();
        Ok(())
    }

    /// Render main content area.
    fn render_content(&self, bounds: Rect) -> Result<()> {
        use gartk_render::TextStyle;

        let style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 12.0,
            color: self.theme.text,
            ..Default::default()
        };

        let dim_style = TextStyle {
            color: self.theme.text_secondary,
            ..style.clone()
        };

        let padding = 12;
        let graph_height = 120u32;
        let text_line_height = 18.0;

        if !self.daemon_available {
            self.renderer.text(
                "Not connected to daemon",
                padding as f64,
                bounds.y as f64 + 30.0,
                &TextStyle {
                    color: self.theme.swap_color,
                    ..style.clone()
                },
            )?;
            self.renderer.text(
                "Run: gartop daemon",
                padding as f64,
                bounds.y as f64 + 50.0,
                &dim_style,
            )?;
            return Ok(());
        }

        // Get Cairo context for graph rendering
        let ctx = self.renderer.context()?;
        let graph = LineGraph::default();

        let mut y = bounds.y + padding as i32;
        let graph_width = bounds.width - (padding * 2) as u32;

        // CPU Section
        let cpu_label = if let Some(cpu) = &self.cpu_stats {
            format!("CPU: {:.1}%", cpu.usage_percent)
        } else {
            "CPU: --".to_string()
        };
        self.renderer.text(
            &cpu_label,
            padding as f64,
            y as f64 + 14.0,
            &TextStyle { color: self.theme.cpu_color, ..style.clone() },
        )?;
        y += 20;

        // CPU Graph
        let cpu_rect = Rect::new(padding as i32, y, graph_width, graph_height);
        let mut cpu_series = DataSeries::new("CPU", self.theme.cpu_color);
        cpu_series.set_values(self.cpu_history.iter().map(|s| s.usage_percent).collect());
        graph.render(&ctx, cpu_rect, &[cpu_series], &self.theme);
        y += graph_height as i32 + padding as i32;

        // Memory Section
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
            padding as f64,
            y as f64 + 14.0,
            &TextStyle { color: self.theme.memory_color, ..style.clone() },
        )?;
        y += 20;

        // Memory Graph
        let mem_rect = Rect::new(padding as i32, y, graph_width, graph_height);
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
        graph.render(&ctx, mem_rect, &[mem_series, swap_series], &self.theme);
        y += graph_height as i32 + padding as i32;

        // Additional stats text
        if let Some(cpu) = &self.cpu_stats {
            let cores_to_show = cpu.per_core.len().min(8);
            let mut core_line = String::from("Cores: ");
            for (i, usage) in cpu.per_core.iter().take(cores_to_show).enumerate() {
                if i > 0 {
                    core_line.push_str(" | ");
                }
                core_line.push_str(&format!("{}:{:.0}%", i, usage));
            }
            if cpu.per_core.len() > cores_to_show {
                core_line.push_str(&format!(" (+{} more)", cpu.per_core.len() - cores_to_show));
            }
            self.renderer.text(&core_line, padding as f64, y as f64 + text_line_height, &dim_style)?;
            y += text_line_height as i32 + 4;
        }

        if let Some(mem) = &self.memory_stats {
            let swap_percent = if mem.swap_total > 0 {
                (mem.swap_used as f64 / mem.swap_total as f64) * 100.0
            } else {
                0.0
            };
            self.renderer.text(
                &format!(
                    "Swap: {:.1}% ({} / {}) | Available: {}",
                    swap_percent,
                    format_bytes(mem.swap_used),
                    format_bytes(mem.swap_total),
                    format_bytes(mem.available)
                ),
                padding as f64,
                y as f64 + text_line_height,
                &dim_style,
            )?;
        }

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
        self.header = HeaderBar::new(Rect::new(0, 0, width, HEADER_HEIGHT));

        Ok(())
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

                InputEvent::Key(key_event) if key_event.pressed => {
                    match key_event.key {
                        Key::Escape | Key::Char('q') => {
                            self.should_quit = true;
                        }
                        Key::Char('r') => {
                            // Force refresh
                            self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                        }
                        _ => {}
                    }
                }

                InputEvent::CloseRequested => {
                    self.should_quit = true;
                }

                InputEvent::Idle => {
                    // Periodic refresh
                    if self.last_refresh.elapsed().as_secs_f64() >= REFRESH_INTERVAL {
                        if self.daemon_available {
                            self.refresh_data();
                            self.update_header();
                            ev_loop.request_redraw();
                        } else {
                            // Try reconnecting
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

            // Render if needed
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
