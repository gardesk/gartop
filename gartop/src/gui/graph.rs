//! Line graph component for visualizing time series data

use cairo::Context;
use gartk_core::{Color, Rect};
use gartk_render::{fill_rounded_rect, hline, TextRenderer, TextStyle};
use super::theme::Theme;

/// A data series to plot on a graph.
pub struct DataSeries {
    /// Series label for legend.
    pub label: String,
    /// Line color.
    pub color: Color,
    /// Data values (0-100 typically).
    pub values: Vec<f64>,
}

impl DataSeries {
    /// Create a new data series.
    pub fn new(label: impl Into<String>, color: Color) -> Self {
        Self {
            label: label.into(),
            color,
            values: Vec::new(),
        }
    }

    /// Set all values at once.
    pub fn set_values(&mut self, values: Vec<f64>) {
        self.values = values;
    }
}

/// Line graph configuration and rendering.
pub struct LineGraph {
    /// Minimum Y value.
    pub y_min: f64,
    /// Maximum Y value.
    pub y_max: f64,
    /// Number of horizontal grid lines.
    pub grid_lines: usize,
    /// Whether to show legend.
    pub show_legend: bool,
    /// Line width in pixels.
    pub line_width: f64,
    /// Whether to fill under the line.
    pub fill: bool,
    /// Fill opacity (0.0-1.0).
    pub fill_opacity: f64,
}

impl Default for LineGraph {
    fn default() -> Self {
        Self {
            y_min: 0.0,
            y_max: 100.0,
            grid_lines: 4,
            show_legend: true,
            line_width: 1.5,
            fill: true,
            fill_opacity: 0.15,
        }
    }
}

impl LineGraph {
    /// Render the graph onto a Cairo context.
    pub fn render(&self, ctx: &Context, rect: Rect, series: &[DataSeries], theme: &Theme) {
        let x = rect.x as f64;
        let y = rect.y as f64;
        let w = rect.width as f64;
        let h = rect.height as f64;

        // Background
        fill_rounded_rect(ctx, rect, 4.0, theme.graph_bg);

        // Calculate graph area with margins (legend now inside graph, no extra bottom margin)
        let (ml, mr, mt, mb) = (36.0, 8.0, 8.0, 8.0);

        let gx = x + ml;
        let gy = y + mt;
        let gw = w - ml - mr;
        let gh = h - mt - mb;

        if gw <= 0.0 || gh <= 0.0 {
            return;
        }

        // Draw grid lines and Y-axis labels
        self.draw_grid(ctx, gx, gy, gw, gh, x, theme);

        // Draw each data series
        for data in series {
            if !data.values.is_empty() {
                self.draw_series(ctx, data, gx, gy, gw, gh);
            }
        }

        // Draw legend inside graph area (top-right corner)
        if self.show_legend && !series.is_empty() {
            self.draw_legend(ctx, series, gx, gy, gw, theme);
        }
    }

    fn draw_grid(&self, ctx: &Context, gx: f64, gy: f64, gw: f64, gh: f64, label_x: f64, theme: &Theme) {
        if self.grid_lines == 0 {
            return;
        }

        let y_range = self.y_max - self.y_min;
        if y_range <= 0.0 {
            return;
        }

        let step = gh / self.grid_lines as f64;
        let vstep = y_range / self.grid_lines as f64;

        let text_renderer = TextRenderer::new();
        let label_style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 9.0,
            color: theme.text_secondary,
            ..Default::default()
        };

        for i in 0..=self.grid_lines {
            let ly = gy + i as f64 * step;

            // Grid line
            hline(ctx, ly, gx, gx + gw, theme.graph_grid, 1.0);

            // Y-axis label
            if self.show_legend {
                let val = self.y_max - i as f64 * vstep;
                let label = format!("{:.0}", val);
                text_renderer.draw(ctx, &label, label_x + 2.0, ly - 4.0, &label_style);
            }
        }
    }

    fn draw_series(&self, ctx: &Context, series: &DataSeries, gx: f64, gy: f64, gw: f64, gh: f64) {
        let vals = &series.values;
        if vals.is_empty() {
            return;
        }

        let y_range = self.y_max - self.y_min;
        let x_step = if vals.len() > 1 {
            gw / (vals.len() - 1) as f64
        } else {
            gw
        };

        // Build the line path
        ctx.new_path();
        for (i, &v) in vals.iter().enumerate() {
            let px = gx + i as f64 * x_step;
            let norm = ((v - self.y_min) / y_range).clamp(0.0, 1.0);
            let py = gy + gh - norm * gh;

            if i == 0 {
                ctx.move_to(px, py);
            } else {
                ctx.line_to(px, py);
            }
        }

        // Fill under the line
        if self.fill && vals.len() > 1 {
            // Save the line path
            let path = ctx.copy_path().ok();

            // Close path to bottom
            let last_x = gx + (vals.len() - 1) as f64 * x_step;
            ctx.line_to(last_x, gy + gh);
            ctx.line_to(gx, gy + gh);
            ctx.close_path();

            // Fill with transparent color
            ctx.set_source_rgba(
                series.color.r,
                series.color.g,
                series.color.b,
                self.fill_opacity,
            );
            let _ = ctx.fill();

            // Restore line path for stroke
            if let Some(p) = path {
                ctx.new_path();
                ctx.append_path(&p);
            }
        }

        // Stroke the line
        ctx.set_source_rgba(
            series.color.r,
            series.color.g,
            series.color.b,
            1.0,
        );
        ctx.set_line_width(self.line_width);
        ctx.set_line_join(cairo::LineJoin::Round);
        ctx.set_line_cap(cairo::LineCap::Round);
        let _ = ctx.stroke();
    }

    fn draw_legend(&self, ctx: &Context, series: &[DataSeries], gx: f64, gy: f64, gw: f64, theme: &Theme) {
        let text_renderer = TextRenderer::new();
        let style = TextStyle {
            font_family: "monospace".to_string(),
            font_size: 9.0,
            color: theme.text_secondary,
            ..Default::default()
        };

        // Calculate total legend width first (right-align)
        let mut total_width = 0.0;
        for data in series {
            let size = text_renderer.measure(ctx, &data.label, &style);
            total_width += 12.0 + size.width as f64 + 12.0; // box + gap + text + spacing
        }
        total_width -= 12.0; // Remove trailing spacing

        // Position at top-right inside graph area
        let legend_x = gx + gw - total_width - 8.0;
        let legend_y = gy + 4.0;

        // Draw semi-transparent background for readability
        ctx.rectangle(legend_x - 4.0, legend_y - 2.0, total_width + 8.0, 16.0);
        ctx.set_source_rgba(
            theme.graph_bg.r,
            theme.graph_bg.g,
            theme.graph_bg.b,
            0.85,
        );
        let _ = ctx.fill();

        // Draw legend entries
        let mut lx = legend_x;
        for data in series {
            // Color box
            ctx.rectangle(lx, legend_y + 3.0, 8.0, 8.0);
            ctx.set_source_rgba(
                data.color.r,
                data.color.g,
                data.color.b,
                1.0,
            );
            let _ = ctx.fill();

            lx += 12.0;

            // Label
            text_renderer.draw(ctx, &data.label, lx, legend_y, &style);
            let size = text_renderer.measure(ctx, &data.label, &style);
            lx += size.width as f64 + 12.0;
        }
    }
}
