//! GUI implementation using gartk

mod app;
mod graph;
mod header;
mod process_list;
mod tabs;
pub mod theme;

pub use app::App;
pub use graph::{DataSeries, LineGraph};
pub use process_list::ProcessList;
pub use tabs::{Tab, TabBar, TAB_BAR_HEIGHT};

use crate::config::GuiConfig;
use anyhow::Result;

/// Run the gartop GUI.
pub async fn run(config: GuiConfig) -> Result<()> {
    let app = App::new(config)?;
    app.run()?;
    Ok(())
}
