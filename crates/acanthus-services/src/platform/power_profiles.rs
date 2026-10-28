use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::OwnedVariant;

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::{BusKind, wait_for_owner, while_owned};

const ENDPOINTS: [(&str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

/// A power profile of power-profiles-daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// Lowest consumption.
    PowerSaver,
    /// The default profile.
    #[default]
    Balanced,
    /// Highest performance.
    Performance,
}

impl Profile {
    /// The profile with this daemon name; unknown names are balanced.
    pub fn from_name(name: &str) -> Profile {
        match name {
            "power-saver" => Profile::PowerSaver,
            "performance" => Profile::Performance,
            _ => Profile::Balanced,
        }
    }

    /// The daemon's name of the profile.
    pub fn name(self) -> &'static str {
        match self {
            Profile::PowerSaver => "power-saver",
            Profile::Balanced => "balanced",
            Profile::Performance => "performance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Snapshot {
    profile: Profile,
    has_performance: bool,
}

enum Event {
    Snapshot(Snapshot),
    Unavailable,
}

enum Item {
    Changed,
    Command(Profile),
}

/// Observable state of the power profiles service.
#[derive(Clone)]
pub struct PowerProfilesState {
    /// True once the daemon answered.
    pub ready: Signal<bool>,
    /// The active profile.
    pub profile: Signal<Profile>,
    /// True when the machine offers the performance profile.
    pub has_performance: Signal<bool>,
}

/// Power profile control through power-profiles-daemon.
pub struct PowerProfiles {
    /// Its observable state.
    pub state: PowerProfilesState,
    service: ServiceHandle<Profile>,
}

async fn read_snapshot(proxy: &zbus::Proxy<'_>) -> zbus::Result<Snapshot> {
    let active: String = proxy.get_property("ActiveProfile").await?;
    let profiles: Vec<std::collections::HashMap<String, OwnedVariant>> =
        proxy.get_property("Profiles").await.unwrap_or_default();
    let has_performance = profiles
        .iter()
        .filter_map(|entry| entry.get("Profile"))
        .filter_map(|value| <&str>::try_from(value).ok())
        .any(|name| name == "performance");
    Ok(Snapshot { profile: Profile::from_name(&active), has_performance })
}

async fn locate(connection: &zbus::Connection) -> Option<(zbus::Proxy<'static>, &'static str, &'static str)> {
    for (service, path) in ENDPOINTS {
        let proxy: zbus::Proxy<'static> = zbus::proxy::Builder::new(connection)
            .destination(service)
            .path(path)
            .interface(service)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .ok()?;
        if proxy.get_property::<String>("ActiveProfile").await.is_ok() {
            return Some((proxy, service, path));
        }
    }
    None
}

async fn run(bus: BusKind, events: EventSink<Event>, commands: async_channel::Receiver<Profile>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let Some((proxy, service, path)) = locate(&connection).await else {
            return Err(zbus::Error::Failure("no power profiles daemon".to_owned()));
        };
        while_owned(&connection, service, async {
            let properties =
                zbus::fdo::PropertiesProxy::builder(&connection).destination(service).path(path).build().await?;
            let changes = properties.receive_properties_changed().await?.map(|_| Item::Changed);
            let commands = commands.map(Item::Command);
            let mut stream = std::pin::pin!(changes.or(commands));
            events.send(Event::Snapshot(read_snapshot(&proxy).await?));
            while let Some(item) = stream.next().await {
                if let Item::Command(profile) = item
                    && let Err(e) = proxy.set_property("ActiveProfile", profile.name()).await
                {
                    tracing::info!("power profile change failed: {e}");
                }
                if let Ok(snapshot) = read_snapshot(&proxy).await {
                    events.send(Event::Snapshot(snapshot));
                }
            }
            Ok(())
        })
        .await
    }
    .await;
    if let Err(e) = result {
        tracing::info!("power profiles unavailable: {e}");
        events.send(Event::Unavailable);
        if let Ok(connection) = bus.connect_shared().await {
            wait_for_owner(&connection, &ENDPOINTS.map(|(service, _)| service)).await.ok();
        }
    }
}

impl PowerProfiles {
    /// Connects to the daemon on `bus` on a helper thread.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind) -> Result<PowerProfiles, IoError> {
        let state = PowerProfilesState {
            ready: Signal::new(false),
            profile: Signal::new(Profile::Balanced),
            has_performance: Signal::new(false),
        };
        let target = state.clone();
        let service = spawn_supervised(
            handle,
            "acanthus-power-profiles",
            move |event| match event {
                Event::Snapshot(snapshot) => batch(|| {
                    target.profile.set(snapshot.profile);
                    target.has_performance.set(snapshot.has_performance);
                    target.ready.set(true);
                }),
                Event::Unavailable => target.ready.set(false),
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        Ok(PowerProfiles { state, service })
    }

    /// Requests `profile`; false when the service is gone.
    pub fn set(&self, profile: Profile) -> bool {
        self.service.send(profile)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::Variant;
    use zbus::interface;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    struct Fake {
        active: String,
        performance: bool,
    }

    #[interface(name = "org.freedesktop.UPower.PowerProfiles")]
    impl Fake {
        #[zbus(property)]
        fn active_profile(&self) -> String {
            self.active.clone()
        }
        #[zbus(property)]
        fn set_active_profile(&mut self, value: String) {
            self.active = value;
        }
        #[zbus(property)]
        fn profiles(&self) -> Vec<HashMap<String, OwnedVariant>> {
            let mut names = vec!["power-saver", "balanced"];
            if self.performance {
                names.push("performance");
            }
            names
                .into_iter()
                .map(|name| {
                    HashMap::from([("Profile".to_owned(), OwnedVariant::try_from(Variant::from(name)).unwrap())])
                })
                .collect()
        }
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    fn serve(address: String, performance: bool, ready: mpsc::Sender<()>) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name("org.freedesktop.UPower.PowerProfiles")
                    .serve_at(
                        "/org/freedesktop/UPower/PowerProfiles",
                        Fake { active: "balanced".to_owned(), performance },
                    )
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
    }

    #[test]
    fn profile_names_round_trip() {
        for profile in [Profile::PowerSaver, Profile::Balanced, Profile::Performance] {
            assert_eq!(Profile::from_name(profile.name()), profile);
        }
        assert_eq!(Profile::from_name("something"), Profile::Balanced);
    }

    #[test]
    fn profile_follows_the_bus_and_commands_write_back() {
        let bus = TestBus::start();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), true, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = PowerProfiles::start(&event_loop.handle(), bus.kind()).unwrap();
        let state = service.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert_eq!(state.profile.get_untracked(), Profile::Balanced);
        assert!(state.has_performance.get_untracked());

        assert!(service.set(Profile::PowerSaver));
        assert!(pump_until(&mut event_loop, || state.profile.get_untracked() == Profile::PowerSaver));
    }

    #[test]
    fn missing_performance_profile_is_reported() {
        let bus = TestBus::start();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), false, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = PowerProfiles::start(&event_loop.handle(), bus.kind()).unwrap();
        let state = service.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert!(!state.has_performance.get_untracked());
    }

    #[test]
    fn missing_daemon_leaves_the_service_not_ready() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = PowerProfiles::start(&event_loop.handle(), bus.kind()).unwrap();
        let end = Instant::now() + Duration::from_millis(800);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(!service.state.ready.get_untracked());
    }
}
