//! Process list component for displaying running processes

use gartk_core::{Point, Rect};
use gartk_render::{Renderer, TextStyle};
use gartop_ipc::{ProcessInfo, SortField};
use std::collections::HashMap;
use std::time::Instant;
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

/// Build tree-ordered list of processes with indent levels.
/// Returns Vec of (process_index_in_original, indent_level, is_last_sibling).
fn build_tree_order(processes: &[ProcessInfo]) -> Vec<(usize, usize, bool)> {
    // Build parent -> children map
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    let pid_to_idx: HashMap<i32, usize> = processes.iter()
        .enumerate()
        .map(|(i, p)| (p.pid, i))
        .collect();

    // Group processes by parent
    for (idx, proc) in processes.iter().enumerate() {
        children.entry(proc.ppid).or_default().push(idx);
    }

    // Find root processes (parent not in our list, or ppid=0/1)
    let mut roots: Vec<usize> = Vec::new();
    for (idx, proc) in processes.iter().enumerate() {
        if proc.ppid == 0 || proc.ppid == 1 || !pid_to_idx.contains_key(&proc.ppid) {
            roots.push(idx);
        }
    }

    // Sort roots by CPU usage (descending)
    roots.sort_by(|&a, &b| {
        processes[b].cpu_percent
            .partial_cmp(&processes[a].cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // DFS to build ordered list
    let mut result = Vec::new();
    fn visit(
        idx: usize,
        depth: usize,
        is_last: bool,
        children: &HashMap<i32, Vec<usize>>,
        processes: &[ProcessInfo],
        result: &mut Vec<(usize, usize, bool)>,
    ) {
        result.push((idx, depth, is_last));
        if let Some(child_indices) = children.get(&processes[idx].pid) {
            let mut sorted_children = child_indices.clone();
            // Sort children by CPU
            sorted_children.sort_by(|&a, &b| {
                processes[b].cpu_percent
                    .partial_cmp(&processes[a].cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let len = sorted_children.len();
            for (i, &child_idx) in sorted_children.iter().enumerate() {
                visit(child_idx, depth + 1, i == len - 1, children, processes, result);
            }
        }
    }

    let root_count = roots.len();
    for (i, root_idx) in roots.into_iter().enumerate() {
        visit(root_idx, 0, i == root_count - 1, &children, processes, &mut result);
    }

    result
}

/// Row height for process list.
const ROW_HEIGHT: u32 = 22;

/// Header row height.
const HEADER_HEIGHT: u32 = 24;

/// Grace period after user navigation before auto-scrolling resumes (ms)
const NAVIGATION_GRACE_MS: u64 = 400;

/// Process list component - renders process data without owning it.
/// Tracks selection by PID so cursor follows the process when list reorders.
pub struct ProcessList {
    bounds: Rect,
    scroll_offset: usize,
    /// Selected process PID (tracks process across reorders)
    selected_pid: Option<i32>,
    /// Cached index of selected PID (updated on sync)
    selected_index: Option<usize>,
    sort_field: SortField,
    visible_rows: usize,
    process_count: usize,
    /// Last time user navigated (for grace period)
    last_nav_time: Option<Instant>,
    /// Whether cursor lost its target and needs visual indicator
    cursor_lost: bool,
}

impl ProcessList {
    /// Create a new process list.
    pub fn new(bounds: Rect) -> Self {
        let visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
        Self {
            bounds,
            scroll_offset: 0,
            selected_pid: None,
            selected_index: None,
            sort_field: SortField::Cpu,
            visible_rows,
            process_count: 0,
            last_nav_time: None,
            cursor_lost: false,
        }
    }

    /// Update bounds (on resize).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.visible_rows = ((bounds.height.saturating_sub(HEADER_HEIGHT)) / ROW_HEIGHT) as usize;
    }

    /// Sync selection with updated process list.
    /// Call this after refreshing process data to update the cursor position.
    /// Returns true if selection is still valid.
    pub fn sync_selection(&mut self, processes: &[ProcessInfo]) -> bool {
        self.process_count = processes.len();
        self.cursor_lost = false;

        // Clamp scroll offset
        if self.scroll_offset > self.max_scroll() {
            self.scroll_offset = self.max_scroll();
        }

        // Check if we're in grace period after user navigation
        let in_grace_period = self.last_nav_time
            .map(|t| t.elapsed().as_millis() < NAVIGATION_GRACE_MS as u128)
            .unwrap_or(false);

        // Find the selected PID in the new list
        if let Some(pid) = self.selected_pid {
            if let Some(idx) = processes.iter().position(|p| p.pid == pid) {
                // Process found - update index
                self.selected_index = Some(idx);

                // Only auto-scroll if NOT in grace period
                if !in_grace_period {
                    // Keep cursor visible if it moved
                    if idx < self.scroll_offset {
                        self.scroll_offset = idx;
                    } else if idx >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = idx.saturating_sub(self.visible_rows - 1);
                    }
                }
                return true;
            } else {
                // Process no longer exists - jump to last visible row
                self.cursor_lost = true;
                let last_visible = (self.scroll_offset + self.visible_rows - 1).min(processes.len().saturating_sub(1));
                if !processes.is_empty() {
                    self.selected_index = Some(last_visible);
                    self.selected_pid = Some(processes[last_visible].pid);
                } else {
                    self.selected_pid = None;
                    self.selected_index = None;
                }
                return false;
            }
        }

        true
    }

    /// Update process count (for scroll calculations) - legacy method.
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
            self.last_nav_time = Some(Instant::now());
            self.cursor_lost = false;
            let pid = processes[process_idx].pid;
            self.selected_pid = Some(pid);
            self.selected_index = Some(process_idx);
            return Some(pid);
        }

        None
    }

    /// Get selected process PID.
    pub fn selected_pid(&self) -> Option<i32> {
        self.selected_pid
    }

    /// Clear selection.
    pub fn clear_selection(&mut self) {
        self.selected_pid = None;
        self.selected_index = None;
    }

    /// Select by index and PID (for fuzzy jump).
    pub fn select_by_index(&mut self, idx: usize, pid: i32) {
        self.last_nav_time = Some(Instant::now());
        self.cursor_lost = false;
        self.selected_index = Some(idx);
        self.selected_pid = Some(pid);

        // Scroll to make selection visible
        if idx < self.scroll_offset {
            self.scroll_offset = idx;
        } else if idx >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = idx.saturating_sub(self.visible_rows - 1);
        }
    }

    /// Move selection down by one row.
    pub fn select_next(&mut self, processes: &[ProcessInfo]) {
        if processes.is_empty() {
            return;
        }
        self.last_nav_time = Some(Instant::now());
        self.cursor_lost = false;

        match self.selected_index {
            None => {
                // Select first visible item
                let idx = self.scroll_offset.min(processes.len() - 1);
                self.selected_index = Some(idx);
                self.selected_pid = Some(processes[idx].pid);
            }
            Some(idx) => {
                if idx + 1 < processes.len() {
                    let new_idx = idx + 1;
                    self.selected_index = Some(new_idx);
                    self.selected_pid = Some(processes[new_idx].pid);
                    // Auto-scroll if selection goes below visible area
                    if new_idx >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = (new_idx + 1).saturating_sub(self.visible_rows);
                    }
                }
            }
        }
    }

    /// Move selection up by one row.
    pub fn select_prev(&mut self, processes: &[ProcessInfo]) {
        if processes.is_empty() {
            return;
        }
        self.last_nav_time = Some(Instant::now());
        self.cursor_lost = false;

        match self.selected_index {
            None => {
                // Select first visible item
                let idx = self.scroll_offset.min(processes.len() - 1);
                self.selected_index = Some(idx);
                self.selected_pid = Some(processes[idx].pid);
            }
            Some(idx) => {
                if idx > 0 {
                    let new_idx = idx - 1;
                    self.selected_index = Some(new_idx);
                    self.selected_pid = Some(processes[new_idx].pid);
                    // Auto-scroll if selection goes above visible area
                    if new_idx < self.scroll_offset {
                        self.scroll_offset = new_idx;
                    }
                }
            }
        }
    }

    /// Select first item (Home key).
    pub fn select_first(&mut self, processes: &[ProcessInfo]) {
        if !processes.is_empty() {
            self.last_nav_time = Some(Instant::now());
            self.cursor_lost = false;
            self.selected_index = Some(0);
            self.selected_pid = Some(processes[0].pid);
            self.scroll_offset = 0;
        }
    }

    /// Select last item (End key).
    pub fn select_last(&mut self, processes: &[ProcessInfo]) {
        if !processes.is_empty() {
            self.last_nav_time = Some(Instant::now());
            self.cursor_lost = false;
            let last_idx = processes.len() - 1;
            self.selected_index = Some(last_idx);
            self.selected_pid = Some(processes[last_idx].pid);
            self.scroll_offset = self.max_scroll();
        }
    }

    /// Get the currently selected index.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Check if cursor just lost its target (for visual feedback).
    pub fn is_cursor_lost(&self) -> bool {
        self.cursor_lost
    }

    /// Render the process list.
    pub fn render(&self, renderer: &Renderer, theme: &Theme, processes: &[ProcessInfo], tree_view: bool) -> anyhow::Result<()> {
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

        // Build tree order if needed
        let tree_order = if tree_view {
            Some(build_tree_order(processes))
        } else {
            None
        };

        // Process rows
        let start_y = self.bounds.y + HEADER_HEIGHT as i32;
        let row_count = if tree_view {
            tree_order.as_ref().map(|t| t.len()).unwrap_or(0)
        } else {
            processes.len()
        };

        for i in 0..self.visible_rows {
            let display_idx = self.scroll_offset + i;
            if display_idx >= row_count {
                break;
            }

            // Get process and tree info
            let (process, indent, _is_last) = if let Some(ref tree) = tree_order {
                let (orig_idx, indent, is_last) = tree[display_idx];
                (&processes[orig_idx], indent, is_last)
            } else {
                (&processes[display_idx], 0, false)
            };

            let row_y = start_y + (i as i32 * ROW_HEIGHT as i32);
            let text_y = row_y as f64 + 4.0;

            // Selection highlight
            if self.selected_index == Some(display_idx) {
                let row_rect = Rect::new(
                    self.bounds.x + 2,
                    row_y + 2,
                    self.bounds.width - 4,
                    ROW_HEIGHT - 4,
                );
                let highlight_color = if self.cursor_lost {
                    gartk_core::Color::new(0.6, 0.2, 0.6, 1.0)
                } else {
                    theme.header_bg
                };
                renderer.fill_rounded_rect(row_rect, 2.0, highlight_color)?;
            }

            // PID
            renderer.text(&process.pid.to_string(), col_pid, text_y, &dim_style)?;

            // Name with tree prefix
            let name_with_prefix = if tree_view && indent > 0 {
                let prefix = "  ".repeat(indent.saturating_sub(1)) + "├─";
                let max_name_len = 18usize.saturating_sub(prefix.len());
                if process.name.len() > max_name_len {
                    format!("{}{:.width$}..", prefix, process.name, width = max_name_len.saturating_sub(2))
                } else {
                    format!("{}{}", prefix, process.name)
                }
            } else if process.name.len() > 18 {
                format!("{}...", &process.name[..15])
            } else {
                process.name.clone()
            };
            renderer.text(&name_with_prefix, col_name, text_y, &text_style)?;

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
