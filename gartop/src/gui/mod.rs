//! GUI implementation using gartk

mod app;
mod graph;
mod header;
mod process_list;
mod tabs;
pub mod theme;

pub use app::App;

use crate::config::GuiConfig;
use anyhow::Result;

/// Run the gartop GUI.
pub async fn run(config: GuiConfig) -> Result<()> {
    let app = App::new(config)?;
    app.run()?;
    Ok(())
}
