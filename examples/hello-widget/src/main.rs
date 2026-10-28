use std::error::Error;

use acanthus_core::Memo;
use acanthus_runtime::Clock;
use acanthus_services::platform::dbus::BusKind;
use acanthus_services::platform::mpris::Mpris;
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

    let mpris = Mpris::start(&handle, BusKind::Session)?;
    let active = mpris.active.clone();
    let (track, track_binding) = Memo::new(move || model::now_playing(active.get().as_ref())).split();
    let (title_track, artist_track) = (track.clone(), track);
    let (title, title_binding) = Memo::new(move || title_track.get().0).split();
    let (artist, artist_binding) = Memo::new(move || artist_track.get().1).split();

    let time = clock.time();
    app.per_output(
        |_| {
            LayerConfig::new("hello-widget")
                .layer(Layer::Background)
                .anchor(Anchor::TOP | Anchor::RIGHT)
                .size(model::WIDTH, model::HEIGHT)
                .margin(model::MARGIN, model::MARGIN, 0, 0)
                .exclusive_zone(-1)
        },
        move |scene, _| {
            view::build(scene, view::Inputs { time: time.clone(), title: title.clone(), artist: artist.clone() })
        },
    );

    app.run()?;
    drop((track_binding, title_binding, artist_binding, mpris, clock));
    Ok(())
}
