use std::error::Error;
use std::time::Duration;

use acanthus_app::{Actions, Keybindings, Verb};
use acanthus_io::{AppId, Dirs, IpcClient, IpcRegistry, IpcServer};
use acanthus_services::platform::compositor::CompositorKind;
use acanthus_services::platform::dbus::BusKind;
use acanthus_services::platform::shortcuts::{Launcher, Strategy};
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

    let dirs = Dirs::from_env(&AppId::new("hello-popup", "HELLO_POPUP"));
    let args: Vec<String> = std::env::args().collect();
    if let [_, command, id] = args.as_slice()
        && command == "activate"
    {
        IpcClient::call(&dirs.ipc_socket, "keys", "activate", &[id.as_str().into()])?;
        return Ok(());
    }

    let mut app = App::new()?;
    let controller = controller::Controller::new();

    let actions = Actions::new(model::STACK);
    actions.add(std::rc::Rc::new(controller.clone()));
    let ipc = IpcRegistry::new();
    actions.serve(&ipc, "main", "handleCommand");
    let keys = std::rc::Rc::new(Keybindings::new().bind(
        model::SHORTCUT,
        "Toggle the greeting",
        "Super+G",
        Verb::Toggle,
        model::NAME,
        "",
    )?);
    keys.serve(&actions, &ipc, "keys", "activate");
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let shortcuts = keys.start(
        &app.loop_handle(),
        &actions,
        Launcher::new([exe.as_str(), "activate", "{id}"]),
        CompositorKind::detect(|key| std::env::var(key).ok()),
        Strategy::choose(|key| std::env::var(key).ok(), false),
        BusKind::Session,
    )?;
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
    drop((server, actions, shortcuts));
    Ok(())
}
