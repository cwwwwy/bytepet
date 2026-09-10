mod app;
mod fonts;
mod greeting;
mod platform;

use bytepet_core::config::{AppConfig, AppPaths};
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let paths = AppPaths::default();
    if let Err(error) = paths.ensure() {
        eprintln!("cannot prepare BytePet data directory: {error}");
        return Ok(());
    }
    let config = AppConfig::load(&paths.config_file).unwrap_or_default();
    let app = match app::BytePetApp::new(paths, config) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("BytePet failed to start: {error:#}");
            return Ok(());
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BytePet")
            .with_inner_size([220.0, 280.0])
            .with_min_inner_size([140.0, 180.0])
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        "BytePet",
        options,
        Box::new(move |creation_context| {
            let mut app = app;
            app.initialize(creation_context);
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}
