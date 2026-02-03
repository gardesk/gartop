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

use anyhow::Result;

/// Run the gartop GUI.
pub async fn run() -> Result<()> {
    let app = App::new()?;
    app.run()?;
    Ok(())
}
