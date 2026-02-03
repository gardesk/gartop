//! Theme and styling for gartop GUI

use gartk_core::Color;

/// UI theme colors and fonts.
pub struct Theme {
    pub background: Color,
    pub panel_bg: Color,
    pub header_bg: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub accent: Color,
    pub cpu_color: Color,
    pub memory_color: Color,
    pub swap_color: Color,
    pub network_color: Color,
    pub disk_color: Color,
    pub temp_color: Color,
    pub gpu_color: Color,
    pub border: Color,
    pub graph_bg: Color,
    pub graph_grid: Color,
    pub font_family: String,
    pub font_size: f64,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::from_hex("#1e1e2e").unwrap_or(Color::BLACK),
            panel_bg: Color::from_hex("#313244").unwrap_or(Color::BLACK),
            header_bg: Color::from_hex("#45475a").unwrap_or(Color::BLACK),
            text: Color::from_hex("#cdd6f4").unwrap_or(Color::WHITE),
            text_secondary: Color::from_hex("#a6adc8").unwrap_or(Color::WHITE),
            accent: Color::from_hex("#89b4fa").unwrap_or(Color::WHITE),
            cpu_color: Color::from_hex("#f38ba8").unwrap_or(Color::WHITE),
            memory_color: Color::from_hex("#a6e3a1").unwrap_or(Color::WHITE),
            swap_color: Color::from_hex("#f9e2af").unwrap_or(Color::WHITE),
            network_color: Color::from_hex("#94e2d5").unwrap_or(Color::WHITE),
            disk_color: Color::from_hex("#cba6f7").unwrap_or(Color::WHITE),
            temp_color: Color::from_hex("#fab387").unwrap_or(Color::WHITE), // Catppuccin peach
            gpu_color: Color::from_hex("#89dceb").unwrap_or(Color::WHITE),  // Catppuccin sky
            border: Color::from_hex("#585b70").unwrap_or(Color::WHITE),
            graph_bg: Color::from_hex("#11111b").unwrap_or(Color::BLACK),
            graph_grid: Color::from_hex("#313244").unwrap_or(Color::BLACK),
            font_family: "monospace".to_string(),
            font_size: 12.0,
        }
    }
}

impl Theme {
    /// Create theme with custom font settings.
    pub fn with_font(font_family: String, font_size: f64) -> Self {
        Self {
            font_family,
            font_size,
            ..Self::default()
        }
    }
}
