//! Tab bar component for switching between CPU and Memory views

use gartk_core::{Point, Rect};
use gartk_render::{Renderer, TextStyle};
use super::theme::Theme;

/// Available tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Cpu,
    Memory,
}

impl Tab {
    /// Get all tabs in order.
    pub fn all() -> &'static [Tab] {
        &[Tab::Cpu, Tab::Memory]
    }

    /// Get tab label.
    pub fn label(&self) -> &'static str {
        match self {
            Tab::Cpu => "CPU",
            Tab::Memory => "Memory",
        }
    }
}

/// Tab bar height.
pub const TAB_BAR_HEIGHT: u32 = 32;

/// Tab bar component.
pub struct TabBar {
    bounds: Rect,
    active: Tab,
    tab_bounds: Vec<Rect>,
    hovered: Option<usize>,
}

impl TabBar {
    /// Create a new tab bar.
    pub fn new(bounds: Rect) -> Self {
        let tab_bounds = Self::calculate_tab_bounds(bounds);
        Self {
            bounds,
            active: Tab::default(),
            tab_bounds,
            hovered: None,
        }
    }

    /// Calculate bounds for each tab.
    fn calculate_tab_bounds(bounds: Rect) -> Vec<Rect> {
        let tabs = Tab::all();
        let tab_width = 100u32;
        let tab_height = bounds.height - 8; // Leave room for padding
        let mut result = Vec::with_capacity(tabs.len());

        for (i, _) in tabs.iter().enumerate() {
            let x = bounds.x + 8 + (i as i32 * (tab_width as i32 + 4));
            let y = bounds.y + 4; // More top padding
            result.push(Rect::new(x, y, tab_width, tab_height));
        }

        result
    }

    /// Update bounds (on resize).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.tab_bounds = Self::calculate_tab_bounds(bounds);
    }

    /// Get the active tab.
    pub fn active(&self) -> Tab {
        self.active
    }

    /// Set the active tab.
    pub fn set_active(&mut self, tab: Tab) {
        self.active = tab;
    }

    /// Handle click event, returns which tab was clicked (if any).
    pub fn on_click(&self, pos: Point) -> Option<Tab> {
        if !self.bounds.contains_point(pos) {
            return None;
        }

        let tabs = Tab::all();
        for (i, tab_rect) in self.tab_bounds.iter().enumerate() {
            if tab_rect.contains_point(pos) {
                return Some(tabs[i]);
            }
        }
        None
    }

    /// Handle mouse move, returns true if hover state changed.
    pub fn on_mouse_move(&mut self, pos: Point) -> bool {
        let old_hovered = self.hovered;

        if !self.bounds.contains_point(pos) {
            self.hovered = None;
        } else {
            self.hovered = None;
            for (i, tab_rect) in self.tab_bounds.iter().enumerate() {
                if tab_rect.contains_point(pos) {
                    self.hovered = Some(i);
                    break;
                }
            }
        }

        old_hovered != self.hovered
    }

    /// Render the tab bar.
    pub fn render(&self, renderer: &Renderer, theme: &Theme) -> anyhow::Result<()> {
        // Background
        renderer.fill_rect(self.bounds, theme.panel_bg)?;

        let tabs = Tab::all();
        for (i, tab_rect) in self.tab_bounds.iter().enumerate() {
            let tab = tabs[i];
            let is_active = tab == self.active;
            let is_hovered = self.hovered == Some(i);

            // Tab background
            let bg_color = if is_active {
                theme.header_bg
            } else if is_hovered {
                theme.graph_bg
            } else {
                theme.panel_bg
            };
            renderer.fill_rounded_rect(*tab_rect, 4.0, bg_color)?;

            // Tab text
            let text_color = if is_active {
                match tab {
                    Tab::Cpu => theme.cpu_color,
                    Tab::Memory => theme.memory_color,
                }
            } else {
                theme.text_secondary
            };

            let style = TextStyle {
                font_family: "monospace".to_string(),
                font_size: 12.0,
                color: text_color,
                ..Default::default()
            };

            // Center text in tab
            let text = tab.label();
            let text_size = renderer.measure_text(text, &style)?;
            let text_x = tab_rect.x as f64 + (tab_rect.width as f64 - text_size.width as f64) / 2.0;
            let text_y = tab_rect.y as f64 + (tab_rect.height as f64 + style.font_size) / 2.0;
            renderer.text(text, text_x, text_y, &style)?;

            // Active indicator line
            if is_active {
                let indicator_y = (tab_rect.y + tab_rect.height as i32 - 2) as f64;
                renderer.line(
                    tab_rect.x as f64 + 4.0,
                    indicator_y,
                    (tab_rect.x + tab_rect.width as i32 - 4) as f64,
                    indicator_y,
                    text_color,
                    2.0,
                )?;
            }
        }

        Ok(())
    }

    /// Get tab bar height.
    pub fn height(&self) -> u32 {
        self.bounds.height
    }
}
