//! GUI implementation using gartk

mod app;
mod header;
pub mod theme;

pub use app::App;
pub use theme::Theme;

use anyhow::Result;

/// Run the gartop GUI.
pub async fn run() -> Result<()> {
    let app = App::new()?;
    app.run()?;
    Ok(())
}
