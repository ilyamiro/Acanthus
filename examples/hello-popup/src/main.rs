use std::error::Error;
use std::time::Duration;

use acanthus_app::Actions;
use acanthus_io::{AppId, Dirs, IpcRegistry, IpcServer};
use acanthus_wayland::{Anchor, App, KeyboardInteractivity, Layer, LayerConfig};

mod controller;
mod model;
mod view;

#[cfg(test)]
mod tests;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut app = App::new()?;
    let controller = controller::Controller::new();

    let actions = Actions::new(model::STACK);
    actions.add(std::rc::Rc::new(controller.clone()));
    let ipc = IpcRegistry::new();
    actions.serve(&ipc, "main", "handleCommand");
    let dirs = Dirs::from_env(&AppId::new("hello-popup", "HELLO_POPUP"));
    let server = IpcServer::bind(&app.loop_handle(), &dirs.ipc_socket, ipc)?;
    tracing::info!("listening on {}", server.path().display());

    let scene_controller = controller.clone();
    app.per_output_lazy(
        controller.open.clone(),
        Duration::from_millis(model::LINGER_MS),
        |_| {
            LayerConfig::new("hello-popup")
                .layer(Layer::Overlay)
                .anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
                .keyboard(KeyboardInteractivity::OnDemand)
                .exclusive_zone(-1)
        },
        move |scene, _| view::build(scene, scene_controller.clone()),
    );

    app.run()?;
    drop((server, actions));
    Ok(())
}
