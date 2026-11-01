use std::error::Error;
use std::path::PathBuf;

use acanthus_runtime::{I18n, system_language};
use acanthus_wayland::{App, WindowConfig};

mod controller;
mod model;
mod view;

#[cfg(test)]
mod tests;

fn i18n() -> I18n {
    let dir = std::env::var_os("NOTES_I18N")
        .map_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n")), PathBuf::from);
    I18n::new(&dir, &system_language(|key| std::env::var(key).ok()))
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut app = App::new()?;
    let i18n = i18n();
    let mut keep = acanthus_anim::Keep::default();
    let controller = controller::Controller::new(&mut keep);
    let config =
        WindowConfig::new(i18n.t("notes.window")).app_id("notes").size(model::WIDTH, model::HEIGHT).min_size(480, 320);
    let mut view_keep = None;
    app.window(config, |scene| view_keep = Some(view::build(scene, &controller, &i18n)))?;
    app.run()?;
    drop((keep, view_keep, controller));
    Ok(())
}
