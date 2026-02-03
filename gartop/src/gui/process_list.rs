//! Process list component for displaying running processes

use gartk_core::{Point, Rect};
use gartk_render::{Renderer, TextStyle};
use gartop_ipc::{ProcessInfo, SortField};
use super::theme::Theme;

/// Row height for process list.
const ROW_HEIGHT: u32 = 22;

/// Header row height.
const HEADER_HEIGHT: u32 = 24;

/// Process list component.
pub struct ProcessList {
    bounds: Rect,
    processes: Vec<ProcessInfo>,
    scroll_offset: usize,
    selected: Option<usize>,
    sort_field: SortField,
    visible_rows: usize,
}

impl ProcessList {
    /// Create a new process list.
    pub fn new(bounds: Rect) -> Self {
        let visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
        Self {
            bounds,
            processes: Vec::new(),
            scroll_offset: 0,
            selected: None,
            sort_field: SortField::Cpu,
            visible_rows,
        }
    }

    /// Update bounds (on resize).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
    }

    /// Set processes to display.
    pub fn set_processes(&mut self, processes: Vec<ProcessInfo>) {
        self.processes = processes;
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
        self.processes.len().saturating_sub(self.visible_rows)
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
    pub fn on_click(&mut self, pos: Point) -> Option<i32> {
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

        if process_idx < self.processes.len() {
            self.selected = Some(process_idx);
            return Some(self.processes[process_idx].pid);
        }

        None
    }

    /// Get selected process PID.
    pub fn selected_pid(&self) -> Option<i32> {
        self.selected.and_then(|idx| self.processes.get(idx).map(|p| p.pid))
    }

    /// Clear selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Render the process list.
    pub fn render(&self, renderer: &Renderer, theme: &Theme) -> anyhow::Result<()> {
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

        // Column positions
        let x = self.bounds.x as f64;
        let col_pid = x + 8.0;
        let col_name = x + 70.0;
        let col_cpu = x + 220.0;
        let col_mem = x + 290.0;
        let col_user = x + 380.0;

        // Header
        let header_y = self.bounds.y as f64 + 16.0;
        renderer.text("PID", col_pid, header_y, &header_style)?;
        renderer.text("Name", col_name, header_y, &header_style)?;

        // Highlight active sort column
        let sort_style = TextStyle {
            color: match self.sort_field {
                SortField::Cpu => theme.cpu_color,
                SortField::Memory => theme.memory_color,
                _ => theme.text_secondary,
            },
            ..header_style.clone()
        };

        if self.sort_field == SortField::Cpu {
            renderer.text("CPU%", col_cpu, header_y, &sort_style)?;
            renderer.text("Mem%", col_mem, header_y, &header_style)?;
        } else {
            renderer.text("CPU%", col_cpu, header_y, &header_style)?;
            renderer.text("Mem%", col_mem, header_y, &sort_style)?;
        }
        renderer.text("User", col_user, header_y, &header_style)?;

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
        for (i, process) in self.processes.iter()
            .skip(self.scroll_offset)
            .take(self.visible_rows)
            .enumerate()
        {
            let row_y = start_y + (i as i32 * ROW_HEIGHT as i32);
            let text_y = row_y as f64 + 16.0;
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

            // User
            let user = if process.user.len() > 10 {
                format!("{}...", &process.user[..7])
            } else {
                process.user.clone()
            };
            renderer.text(&user, col_user, text_y, &dim_style)?;
        }

        // Scroll indicator if needed
        if self.processes.len() > self.visible_rows {
            let scroll_info = format!(
                "{}-{} of {}",
                self.scroll_offset + 1,
                (self.scroll_offset + self.visible_rows).min(self.processes.len()),
                self.processes.len()
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
