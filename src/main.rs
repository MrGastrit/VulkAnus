mod app;
mod config;
mod graphics;
mod window;

use anyhow::{Result, bail};
use std::sync::atomic::Ordering;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let Some(config) = config::Config::from_args(std::env::args().skip(1))? else {
        return Ok(());
    };
    let mut app = app::Application::new(config)?;
    let validation = app.validation_state();
    let result = app.run();
    // Include destruction in the validation result, including device/instance teardown.
    drop(app);
    result?;
    let errors = validation.errors.load(Ordering::Relaxed);
    if errors != 0 {
        bail!("Vulkan validation reported {errors} error(s)");
    }
    log::info!("Shutdown complete; validation errors: {errors}");
    Ok(())
}
