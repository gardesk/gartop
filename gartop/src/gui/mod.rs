//! GUI implementation using gartk

mod app;
mod graph;
mod header;
pub mod theme;

pub use app::App;
pub use graph::{DataSeries, LineGraph};

use anyhow::Result;

/// Run the gartop GUI.
pub async fn run() -> Result<()> {
    let app = App::new()?;
    app.run()?;
    Ok(())
}
