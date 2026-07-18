mod app;
mod config;
mod event;
mod metrics;
mod sparkline;
mod ui;

use app::App;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let config = config::load();

    let terminal = ratatui::init();
    let result = App::new(config).run(terminal).await;
    ratatui::restore();

    result
}
