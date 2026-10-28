use std::error::Error;

use acanthus_core::Memo;
use acanthus_runtime::Clock;
use acanthus_services::platform::dbus::BusKind;
use acanthus_services::platform::upower::UPower;
use acanthus_wayland::{Anchor, App, Layer, LayerConfig};

mod model;
mod view;

#[cfg(test)]
mod tests;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut app = App::new()?;
    let handle = app.loop_handle();

    let clock = Clock::simple("HH:mm");
    clock.attach(&handle)?;

    let upower = UPower::start(&handle, BusKind::System)?;
    let device = upower.display_device.clone();
    let (battery, battery_binding) =
        Memo::new(move || model::battery_label(device.ready.get(), device.percentage.get(), device.is_charging()))
            .split();

    let time = clock.time();
    app.per_output(
        |_| {
            LayerConfig::new("hello-bar")
                .layer(Layer::Top)
                .anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT)
                .size(0, model::BAR_HEIGHT)
                .exclusive_zone(model::BAR_HEIGHT as i32)
        },
        move |scene, _| view::build(scene, view::Inputs { time: time.clone(), battery: battery.clone() }),
    );

    app.run()?;
    drop((battery_binding, upower, clock));
    Ok(())
}
