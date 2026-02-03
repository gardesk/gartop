//! Process list component for displaying running processes

use gartk_core::{Point, Rect};
use gartk_render::{Renderer, TextStyle};
use gartop_ipc::{ProcessInfo, SortField};
use super::theme::Theme;

/// Format rate (bytes/second) to human-readable compact string.
fn format_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_sec >= GIB {
        format!("{:.1}G", bytes_per_sec / GIB)
    } else if bytes_per_sec >= MIB {
        format!("{:.1}M", bytes_per_sec / MIB)
    } else if bytes_per_sec >= KIB {
        format!("{:.0}K", bytes_per_sec / KIB)
    } else if bytes_per_sec > 0.0 {
        format!("{:.0}B", bytes_per_sec)
    } else {
        "0".to_string()
    }
}

/// Row height for process list.
const ROW_HEIGHT: u32 = 22;

/// Header row height.
const HEADER_HEIGHT: u32 = 24;

/// Process list component - renders process data without owning it.
pub struct ProcessList {
    bounds: Rect,
    scroll_offset: usize,
    selected: Option<usize>,
    sort_field: SortField,
    visible_rows: usize,
    process_count: usize,
}

impl ProcessList {
    /// Create a new process list.
    pub fn new(bounds: Rect) -> Self {
        let visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
        Self {
            bounds,
            scroll_offset: 0,
            selected: None,
            sort_field: SortField::Cpu,
            visible_rows,
            process_count: 0,
        }
    }

    /// Update bounds (on resize).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
    }

    /// Update process count (for scroll calculations).
    pub fn set_process_count(&mut self, count: usize) {
        self.process_count = count;
        // Clamp scroll offset
        if self.scroll_offset > self.max_scroll() {
            self.scroll_offset = self.max_scroll();
        }
    }

    /// Set sort field.
    pub fn set_sort(&mut self, sort: SortField) {
        self.sort_field = sort;
    }

    /// Get current sort field.
    pub fn sort_field(&self) -> SortField {
        self.sort_field
    }

    /// Maximum scroll offset.
    fn max_scroll(&self) -> usize {
        self.process_count.saturating_sub(self.visible_rows)
    }

    /// Handle scroll (delta is number of rows to scroll, positive = down).
    pub fn on_scroll(&mut self, delta: i32) {
        if delta > 0 {
            self.scroll_offset = (self.scroll_offset + delta as usize).min(self.max_scroll());
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as usize);
        }
    }

    /// Handle click, returns selected process PID if any.
    pub fn on_click(&mut self, pos: Point, processes: &[ProcessInfo]) -> Option<i32> {
        if !self.bounds.contains_point(pos) {
            return None;
        }

        let local_y = pos.y - self.bounds.y;

        // Check if clicking header
        if local_y < HEADER_HEIGHT as i32 {
            return None;
        }

        // Calculate row index
        let row = ((local_y - HEADER_HEIGHT as i32) / ROW_HEIGHT as i32) as usize;
        let process_idx = self.scroll_offset + row;

        if process_idx < processes.len() {
            self.selected = Some(process_idx);
            return Some(processes[process_idx].pid);
        }

        None
    }

    /// Get selected process PID.
    pub fn selected_pid(&self, processes: &[ProcessInfo]) -> Option<i32> {
        self.selected.and_then(|idx| processes.get(idx).map(|p| p.pid))
    }

    /// Clear selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Render the process list.
    pub fn render(&self, renderer: &Renderer, theme: &Theme, processes: &[ProcessInfo]) -> anyhow::Result<()> {
        // Background
        renderer.fill_rect(self.bounds, theme.panel_bg)?;

        let header_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 11.0,
            color: theme.text_secondary,
            ..Default::default()
        };

        let text_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 11.0,
            color: theme.text,
            ..Default::default()
        };

        let dim_style = TextStyle {
            color: theme.text_secondary,
            ..text_style.clone()
        };

        // Column positions - adjusted for better spacing
        let x = self.bounds.x as f64;
        let col_pid = x + 8.0;
        let col_name = x + 80.0;
        let col_cpu = x + 240.0;   // CPU%/Read/Sock
        let col_mem = x + 310.0;   // Mem%/Write/Listen
        let col_extra = x + 380.0; // Estab (network only)
        let col_user = x + 450.0;  // User

        // Show different columns based on sort field
        let is_disk_sort = matches!(self.sort_field, SortField::DiskRead | SortField::DiskWrite | SortField::DiskTotal);
        let is_net_sort = matches!(self.sort_field, SortField::NetConnections | SortField::NetTcp | SortField::NetBandwidth);

        // Header - position text near top (Pango uses top-left positioning)
        let header_y = self.bounds.y as f64 + 4.0;
        renderer.text("PID", col_pid, header_y, &header_style)?;
        renderer.text("Name", col_name, header_y, &header_style)?;

        // Highlight active sort column
        let sort_style = TextStyle {
            color: match self.sort_field {
                SortField::Cpu => theme.cpu_color,
                SortField::Memory => theme.memory_color,
                SortField::DiskRead | SortField::DiskWrite | SortField::DiskTotal => theme.disk_color,
                SortField::NetConnections | SortField::NetTcp | SortField::NetBandwidth => theme.network_color,
                _ => theme.text_secondary,
            },
            ..header_style.clone()
        };

        if is_disk_sort {
            renderer.text("Read/s", col_cpu, header_y, &sort_style)?;
            renderer.text("Write/s", col_mem, header_y, &sort_style)?;
            renderer.text("User", col_user, header_y, &header_style)?;
        } else if is_net_sort {
            renderer.text("Sock", col_cpu, header_y, &sort_style)?;
            renderer.text("Listen", col_mem, header_y, &sort_style)?;
            renderer.text("Estab", col_extra, header_y, &sort_style)?;
            renderer.text("User", col_user, header_y, &header_style)?;
        } else if self.sort_field == SortField::Cpu {
            renderer.text("CPU%", col_cpu, header_y, &sort_style)?;
            renderer.text("Mem%", col_mem, header_y, &header_style)?;
            renderer.text("User", col_user, header_y, &header_style)?;
        } else {
            renderer.text("CPU%", col_cpu, header_y, &header_style)?;
            renderer.text("Mem%", col_mem, header_y, &sort_style)?;
            renderer.text("User", col_user, header_y, &header_style)?;
        }

        // Header separator
        let sep_y = (self.bounds.y + HEADER_HEIGHT as i32) as f64;
        renderer.line(
            x + 4.0,
            sep_y,
            (self.bounds.x + self.bounds.width as i32 - 4) as f64,
            sep_y,
            theme.border,
            1.0,
        )?;

        // Process rows
        let start_y = self.bounds.y + HEADER_HEIGHT as i32;
        for (i, process) in processes.iter()
            .skip(self.scroll_offset)
            .take(self.visible_rows)
            .enumerate()
        {
            let row_y = start_y + (i as i32 * ROW_HEIGHT as i32);
            let text_y = row_y as f64 + 4.0; // Pango uses top-left positioning
            let process_idx = self.scroll_offset + i;

            // Selection highlight
            if self.selected == Some(process_idx) {
                let row_rect = Rect::new(
                    self.bounds.x + 2,
                    row_y + 2,
                    self.bounds.width - 4,
                    ROW_HEIGHT - 4,
                );
                renderer.fill_rounded_rect(row_rect, 2.0, theme.header_bg)?;
            }

            // PID
            renderer.text(&process.pid.to_string(), col_pid, text_y, &dim_style)?;

            // Name (truncate if too long)
            let name = if process.name.len() > 18 {
                format!("{}...", &process.name[..15])
            } else {
                process.name.clone()
            };
            renderer.text(&name, col_name, text_y, &text_style)?;

            // Show CPU/Memory or I/O or Network depending on sort field
            if is_disk_sort {
                // Read rate
                let read_style = if process.io_read_rate > 1_000_000.0 {
                    TextStyle { color: theme.disk_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&format_rate(process.io_read_rate), col_cpu, text_y, &read_style)?;

                // Write rate
                let write_style = if process.io_write_rate > 1_000_000.0 {
                    TextStyle { color: theme.disk_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&format_rate(process.io_write_rate), col_mem, text_y, &write_style)?;
            } else if is_net_sort {
                // Total sockets
                let sock_style = if process.net_connections > 10 {
                    TextStyle { color: theme.network_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&process.net_connections.to_string(), col_cpu, text_y, &sock_style)?;

                // Listen count (servers)
                let listen_style = if process.net_listen > 0 {
                    TextStyle { color: theme.network_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&process.net_listen.to_string(), col_mem, text_y, &listen_style)?;

                // Established count (active connections)
                let estab_style = if process.net_established > 5 {
                    TextStyle { color: theme.network_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&process.net_established.to_string(), col_extra, text_y, &estab_style)?;
            } else {
                // CPU %
                let cpu_style = if process.cpu_percent > 50.0 {
                    TextStyle { color: theme.cpu_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&format!("{:.1}", process.cpu_percent), col_cpu, text_y, &cpu_style)?;

                // Memory %
                let mem_style = if process.memory_percent > 10.0 {
                    TextStyle { color: theme.memory_color, ..text_style.clone() }
                } else {
                    dim_style.clone()
                };
                renderer.text(&format!("{:.1}", process.memory_percent), col_mem, text_y, &mem_style)?;
            }

            // User
            let user = if process.user.len() > 10 {
                format!("{}...", &process.user[..7])
            } else {
                process.user.clone()
            };
            renderer.text(&user, col_user, text_y, &dim_style)?;
        }

        // Scroll indicator if needed
        if processes.len() > self.visible_rows {
            let scroll_info = format!(
                "{}-{} of {}",
                self.scroll_offset + 1,
                (self.scroll_offset + self.visible_rows).min(processes.len()),
                processes.len()
            );
            let info_style = TextStyle {
                font_size: 9.0,
                ..dim_style
            };
            let info_x = (self.bounds.x + self.bounds.width as i32 - 80) as f64;
            let info_y = self.bounds.y as f64 + 16.0;
            renderer.text(&scroll_info, info_x, info_y, &info_style)?;
        }

        Ok(())
    }

    /// Get row height.
    pub fn row_height(&self) -> u32 {
        ROW_HEIGHT
    }
}
