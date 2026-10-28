use acanthus_core::Signal;
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::{OwnedVariant, Variant};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::{BusKind, wait_for_owner};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.gnome.desktop.interface";
const KEY: &str = "enable-animations";

enum Event {
    Animations(bool),
    Unavailable,
}

/// Observable state of the desktop's animation preference.
#[derive(Clone)]
pub struct PortalSettingsState {
    /// True once the portal answered.
    pub ready: Signal<bool>,
    /// True when the desktop asks for fewer animations (`enable-animations` is off).
    pub reduced_motion: Signal<bool>,
}

/// The desktop preferences an application follows, read from the XDG Settings portal.
pub struct PortalSettings {
    /// Its observable state.
    pub state: PortalSettingsState,
    _service: ServiceHandle<()>,
}

/// The boolean inside a portal reply, which may be wrapped in one or more variants.
pub fn unwrap_bool(value: &Variant<'_>) -> Option<bool> {
    fn peel(value: &Variant<'_>, depth: u8) -> Option<bool> {
        if let Ok(flag) = value.downcast_ref::<bool>() {
            return Some(flag);
        }
        let inner = value.downcast_ref::<&Variant<'_>>().ok()?;
        if depth == 0 { None } else { peel(inner, depth - 1) }
    }
    peel(value, 3)
}

async fn read_animations(proxy: &zbus::Proxy<'_>) -> zbus::Result<bool> {
    let reply: OwnedVariant = proxy.call("Read", &(NAMESPACE, KEY)).await?;
    unwrap_bool(&reply).ok_or_else(|| zbus::Error::Failure("enable-animations is not a boolean".to_owned()))
}

async fn run(bus: BusKind, events: EventSink<Event>, _commands: async_channel::Receiver<()>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(&connection)
            .destination(SERVICE)
            .path(PATH)
            .interface(INTERFACE)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let mut changes = proxy.receive_signal("SettingChanged").await?;
        events.send(Event::Animations(read_animations(&proxy).await?));
        while let Some(message) = changes.next().await {
            let Ok((namespace, key, value)): zbus::Result<(String, String, OwnedVariant)> =
                message.body().deserialize()
            else {
                continue;
            };
            if namespace == NAMESPACE
                && key == KEY
                && let Some(enabled) = unwrap_bool(&value)
            {
                events.send(Event::Animations(enabled));
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("portal settings unavailable: {e}");
    }
    events.send(Event::Unavailable);
    if let Ok(connection) = bus.connect_shared().await {
        wait_for_owner(&connection, &[SERVICE]).await.ok();
    }
}

impl PortalSettings {
    /// Connects to the portal on `bus` on a helper thread; the state stays not ready where no portal runs.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind) -> Result<PortalSettings, IoError> {
        let state = PortalSettingsState { ready: Signal::new(false), reduced_motion: Signal::new(false) };
        let target = state.clone();
        let service = spawn_supervised(
            handle,
            "acanthus-portal-settings",
            move |event| match event {
                Event::Animations(enabled) => acanthus_core::batch(|| {
                    target.reduced_motion.set(!enabled);
                    target.ready.set(true);
                }),
                Event::Unavailable => target.ready.set(false),
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        Ok(PortalSettings { state, _service: service })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::interface;
    use zbus::object_server::SignalEmitter;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    struct Fake {
        animations: bool,
    }

    #[interface(name = "org.freedesktop.portal.Settings")]
    impl Fake {
        fn read(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedVariant> {
            if namespace == NAMESPACE && key == KEY {
                let inner = Variant::from(self.animations);
                Ok(OwnedVariant::try_from(Variant::new(inner)).unwrap())
            } else {
                Err(zbus::fdo::Error::Failed("unknown key".into()))
            }
        }

        fn flip(&mut self) -> bool {
            self.animations = !self.animations;
            self.animations
        }

        #[zbus(signal)]
        async fn setting_changed(
            emitter: &SignalEmitter<'_>,
            namespace: &str,
            key: &str,
            value: Variant<'_>,
        ) -> zbus::Result<()>;
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn nested_variants_unwrap_to_the_boolean() {
        assert_eq!(unwrap_bool(&Variant::from(true)), Some(true));
        assert_eq!(unwrap_bool(&Variant::new(Variant::from(false))), Some(false));
        assert_eq!(unwrap_bool(&Variant::from("no")), None);
    }

    #[test]
    fn reduced_motion_follows_the_portal() {
        let bus = TestBus::start();
        let (ready_tx, ready_rx) = mpsc::channel();
        let address = bus.address.clone();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at(PATH, Fake { animations: false })
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                async_io::Timer::after(Duration::from_millis(1500)).await;
                let emitter = SignalEmitter::new(&connection, PATH).unwrap();
                Fake::setting_changed(&emitter, NAMESPACE, KEY, Variant::new(Variant::from(true))).await.unwrap();
                std::future::pending::<()>().await;
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = PortalSettings::start(&event_loop.handle(), bus.kind()).unwrap();
        let state = service.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert!(state.reduced_motion.get_untracked());
        assert!(pump_until(&mut event_loop, || !state.reduced_motion.get_untracked()));
    }

    #[test]
    fn missing_portal_leaves_the_state_not_ready() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = PortalSettings::start(&event_loop.handle(), bus.kind()).unwrap();
        let end = Instant::now() + Duration::from_millis(800);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(!service.state.ready.get_untracked());
        assert!(!service.state.reduced_motion.get_untracked());
    }
}
