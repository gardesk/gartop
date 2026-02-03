//! GUI application state and event loop

use super::{header::HeaderBar, theme::Theme};
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
            font_size: 13.0,
            color: self.theme.text,
            ..Default::default()
        };

        let dim_style = TextStyle {
            color: self.theme.text_secondary,
            ..style.clone()
        };

        let mut y = bounds.y as f64 + 30.0;
        let x = 20.0;
        let line_height = 24.0;

        if !self.daemon_available {
            self.renderer.text(
                "Not connected to daemon",
                x,
                y,
                &TextStyle {
                    color: self.theme.swap_color, // yellow
                    ..style.clone()
                },
            )?;
            y += line_height;
            self.renderer.text(
                "Run: gartop daemon",
                x,
                y,
                &dim_style,
            )?;
            return Ok(());
        }

        // CPU info
        if let Some(cpu) = &self.cpu_stats {
            self.renderer.text(
                &format!("CPU Usage: {:.1}%", cpu.usage_percent),
                x,
                y,
                &TextStyle {
                    color: self.theme.cpu_color,
                    ..style.clone()
                },
            )?;
            y += line_height;

            // Per-core display (first 8 cores max)
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
            self.renderer.text(&core_line, x, y, &dim_style)?;
            y += line_height * 1.5;
        }

        // Memory info
        if let Some(mem) = &self.memory_stats {
            self.renderer.text(
                &format!(
                    "Memory: {:.1}% ({} / {})",
                    mem.usage_percent,
                    format_bytes(mem.used),
                    format_bytes(mem.total)
                ),
                x,
                y,
                &TextStyle {
                    color: self.theme.memory_color,
                    ..style.clone()
                },
            )?;
            y += line_height;

            // Calculate swap percentage
            let swap_percent = if mem.swap_total > 0 {
                (mem.swap_used as f64 / mem.swap_total as f64) * 100.0
            } else {
                0.0
            };

            self.renderer.text(
                &format!(
                    "Swap: {:.1}% ({} / {})",
                    swap_percent,
                    format_bytes(mem.swap_used),
                    format_bytes(mem.swap_total)
                ),
                x,
                y,
                &TextStyle {
                    color: self.theme.swap_color,
                    ..style.clone()
                },
            )?;
            y += line_height;

            self.renderer.text(
                &format!(
                    "Available: {} | Free: {}",
                    format_bytes(mem.available),
                    format_bytes(mem.free)
                ),
                x,
                y,
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
