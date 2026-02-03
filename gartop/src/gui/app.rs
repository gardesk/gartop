//! GUI application state and event loop

use super::{
    graph::{DataSeries, LineGraph},
    header::HeaderBar,
    process_list::ProcessList,
    tabs::{Tab, TabBar, TAB_BAR_HEIGHT},
    theme::Theme,
};
use anyhow::Result;
use gartk_core::{InputEvent, Key, Point, Rect};
use gartk_render::{Renderer, TextStyle};
use gartk_x11::{Connection, EventLoop, EventLoopConfig, Window, WindowConfig};
use gartop_ipc::{Command, CpuStats, MemoryStats, ProcessInfo, Response, SortField, StatusInfo};
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
    tab_bar: TabBar,
    process_list: ProcessList,
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
    processes: Vec<ProcessInfo>,
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

        // Create components
        let header = HeaderBar::new(Rect::new(0, 0, width, HEADER_HEIGHT));
        let tab_bar = TabBar::new(Rect::new(0, HEADER_HEIGHT as i32, width, TAB_BAR_HEIGHT));
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
            status: None,
            cpu_stats: None,
            memory_stats: None,
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
            processes: Vec::new(),
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

        // Get processes sorted by current tab's resource
        let sort_field = match self.tab_bar.active() {
            Tab::Cpu => SortField::Cpu,
            Tab::Memory => SortField::Memory,
        };
        if let Some(resp) = self.send_command(&Command::GetProcesses {
            sort_by: Some(sort_field),
            limit: Some(100),
        }) {
            if resp.success {
                self.processes = resp.data.and_then(|d| serde_json::from_value(d).ok()).unwrap_or_default();
                self.process_list.set_processes(self.processes.clone());
                self.process_list.set_sort(sort_field);
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

    /// Render the content for the active tab.
    fn render_tab_content(&self, content_y: i32, _content_height: u32) -> Result<()> {
        let graph_width = self.width - (CONTENT_PADDING * 2);

        // Get Cairo context for graph rendering
        let ctx = self.renderer.context()?;
        let graph = LineGraph {
            fill_opacity: 0.25,
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
        }

        // Render process list
        self.process_list.render(&self.renderer, &self.theme)?;

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
        self.process_list.set_processes(self.processes.clone());

        Ok(())
    }

    /// Handle mouse click.
    fn handle_click(&mut self, pos: Point) -> bool {
        // Check tab bar
        if let Some(tab) = self.tab_bar.on_click(pos) {
            if tab != self.tab_bar.active() {
                self.tab_bar.set_active(tab);
                // Re-sort processes for new tab
                let sort_field = match tab {
                    Tab::Cpu => SortField::Cpu,
                    Tab::Memory => SortField::Memory,
                };
                self.process_list.set_sort(sort_field);
                // Force refresh to get re-sorted processes
                self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                return true;
            }
        }

        // Check process list
        if self.process_list.on_click(pos).is_some() {
            return true;
        }

        false
    }

    /// Handle scroll.
    fn handle_scroll(&mut self, delta: i32) -> bool {
        self.process_list.on_scroll(delta);
        true
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
                    match key_event.key {
                        Key::Escape | Key::Char('q') => {
                            self.should_quit = true;
                        }
                        Key::Char('r') => {
                            self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                        }
                        Key::Char('1') => {
                            self.tab_bar.set_active(Tab::Cpu);
                            self.process_list.set_sort(SortField::Cpu);
                            self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                            ev_loop.request_redraw();
                        }
                        Key::Char('2') => {
                            self.tab_bar.set_active(Tab::Memory);
                            self.process_list.set_sort(SortField::Memory);
                            self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                            ev_loop.request_redraw();
                        }
                        Key::Tab => {
                            let new_tab = match self.tab_bar.active() {
                                Tab::Cpu => Tab::Memory,
                                Tab::Memory => Tab::Cpu,
                            };
                            self.tab_bar.set_active(new_tab);
                            let sort_field = match new_tab {
                                Tab::Cpu => SortField::Cpu,
                                Tab::Memory => SortField::Memory,
                            };
                            self.process_list.set_sort(sort_field);
                            self.last_refresh = Instant::now() - std::time::Duration::from_secs(10);
                            ev_loop.request_redraw();
                        }
                        _ => {}
                    }
                }

                InputEvent::CloseRequested => {
                    self.should_quit = true;
                }

                InputEvent::Idle => {
                    if self.last_refresh.elapsed().as_secs_f64() >= REFRESH_INTERVAL {
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
