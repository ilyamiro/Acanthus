use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::proxy;

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;

const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const TYPE_BATTERY: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatteryState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
}

impl BatteryState {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => BatteryState::Charging,
            2 => BatteryState::Discharging,
            3 => BatteryState::Empty,
            4 => BatteryState::FullyCharged,
            5 => BatteryState::PendingCharge,
            6 => BatteryState::PendingDischarge,
            _ => BatteryState::Unknown,
        }
    }
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Device {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "Type")]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn energy_rate(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceSnapshot {
    pub state: BatteryState,
    pub percentage: f64,
    pub time_to_empty: i64,
    pub time_to_full: i64,
    pub is_present: bool,
    pub is_laptop_battery: bool,
    pub change_rate: f64,
}

enum Event {
    Snapshot(DeviceSnapshot),
    Unavailable,
}

#[derive(Clone)]
pub struct DisplayDevice {
    pub ready: Signal<bool>,
    pub state: Signal<BatteryState>,
    pub percentage: Signal<f64>,
    pub time_to_empty: Signal<i64>,
    pub time_to_full: Signal<i64>,
    pub is_present: Signal<bool>,
    pub is_laptop_battery: Signal<bool>,
    pub change_rate: Signal<f64>,
}

impl Default for DisplayDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayDevice {
    /// Creates a device with every signal at its unknown or zero value, before the first D-Bus snapshot.
    pub fn new() -> Self {
        DisplayDevice {
            ready: Signal::new(false),
            state: Signal::new(BatteryState::Unknown),
            percentage: Signal::new(0.0),
            time_to_empty: Signal::new(0),
            time_to_full: Signal::new(0),
            is_present: Signal::new(false),
            is_laptop_battery: Signal::new(false),
            change_rate: Signal::new(0.0),
        }
    }

    pub fn apply(&self, snapshot: &DeviceSnapshot) {
        batch(|| {
            self.state.set(snapshot.state);
            self.percentage.set(snapshot.percentage);
            self.time_to_empty.set(snapshot.time_to_empty);
            self.time_to_full.set(snapshot.time_to_full);
            self.is_present.set(snapshot.is_present);
            self.is_laptop_battery.set(snapshot.is_laptop_battery);
            self.change_rate.set(snapshot.change_rate);
            self.ready.set(true);
        });
    }

    pub fn is_charging(&self) -> bool {
        matches!(self.state.get(), BatteryState::Charging | BatteryState::PendingCharge)
    }
}

pub struct UPower {
    pub display_device: DisplayDevice,
    _service: ServiceHandle<()>,
}

async fn read_snapshot(proxy: &DeviceProxy<'_>) -> zbus::Result<DeviceSnapshot> {
    let device_type = proxy.device_type().await?;
    let power_supply = proxy.power_supply().await.unwrap_or(false);
    Ok(DeviceSnapshot {
        state: BatteryState::from_raw(proxy.state().await?),
        percentage: proxy.percentage().await?,
        time_to_empty: proxy.time_to_empty().await.unwrap_or(0),
        time_to_full: proxy.time_to_full().await.unwrap_or(0),
        is_present: proxy.is_present().await.unwrap_or(false),
        is_laptop_battery: device_type == TYPE_BATTERY && power_supply,
        change_rate: proxy.energy_rate().await.unwrap_or(0.0),
    })
}

async fn run(bus: BusKind, events: EventSink<Event>, _commands: async_channel::Receiver<()>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let proxy = DeviceProxy::builder(&connection)
            .path(DISPLAY_DEVICE)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let properties = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination("org.freedesktop.UPower")
            .path(DISPLAY_DEVICE)
            .build()
            .await?;
        let mut changes = properties.receive_properties_changed().await?;
        events.send(Event::Snapshot(read_snapshot(&proxy).await?));
        while changes.next().await.is_some() {
            events.send(Event::Snapshot(read_snapshot(&proxy).await?));
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("upower unavailable: {e}");
        events.send(Event::Unavailable);
    }
}

impl UPower {
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind) -> Result<UPower, IoError> {
        let display_device = DisplayDevice::new();
        let target = display_device.clone();
        let service = spawn_supervised(
            handle,
            "serp-upower",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(&snapshot),
                Event::Unavailable => target.ready.set(false),
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        Ok(UPower { display_device, _service: service })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::interface;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    #[derive(Clone)]
    struct FakeDevice {
        state: u32,
        percentage: f64,
    }

    #[interface(name = "org.freedesktop.UPower.Device")]
    impl FakeDevice {
        #[zbus(property)]
        fn state(&self) -> u32 {
            self.state
        }
        #[zbus(property)]
        fn percentage(&self) -> f64 {
            self.percentage
        }
        #[zbus(property)]
        fn time_to_empty(&self) -> i64 {
            7200
        }
        #[zbus(property)]
        fn time_to_full(&self) -> i64 {
            0
        }
        #[zbus(property)]
        fn is_present(&self) -> bool {
            true
        }
        #[zbus(property, name = "Type")]
        fn device_type(&self) -> u32 {
            2
        }
        #[zbus(property)]
        fn energy_rate(&self) -> f64 {
            11.5
        }
        #[zbus(property)]
        fn power_supply(&self) -> bool {
            true
        }
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    fn serve_fake(address: String, updates: mpsc::Receiver<(u32, f64)>, ready: mpsc::Sender<()>) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name("org.freedesktop.UPower")
                    .serve_at(DISPLAY_DEVICE, FakeDevice { state: 2, percentage: 80.0 })
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                while let Ok((state, percentage)) = updates.recv() {
                    let iface = connection.object_server().interface::<_, FakeDevice>(DISPLAY_DEVICE).await.unwrap();
                    {
                        let mut device = iface.get_mut().await;
                        device.state = state;
                        device.percentage = percentage;
                    }
                    let emitter = iface.signal_emitter();
                    iface.get().await.state_changed(emitter).await.unwrap();
                    iface.get().await.percentage_changed(emitter).await.unwrap();
                }
            });
        });
    }

    #[test]
    fn display_device_follows_the_bus() {
        let bus = TestBus::start();
        let (update_tx, update_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve_fake(bus.address.clone(), update_rx, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let upower = UPower::start(&event_loop.handle(), bus.kind()).unwrap();
        let device = upower.display_device.clone();
        assert!(!device.ready.get_untracked());
        assert!(pump_until(&mut event_loop, || device.ready.get_untracked()));
        assert_eq!(device.state.get_untracked(), BatteryState::Discharging);
        assert_eq!(device.percentage.get_untracked(), 80.0);
        assert_eq!(device.time_to_empty.get_untracked(), 7200);
        assert!(device.is_laptop_battery.get_untracked());
        assert!(device.is_present.get_untracked());
        assert_eq!(device.change_rate.get_untracked(), 11.5);
        assert!(!device.is_charging());

        update_tx.send((1, 81.0)).unwrap();
        assert!(pump_until(&mut event_loop, || device.state.get_untracked() == BatteryState::Charging));
        assert_eq!(device.percentage.get_untracked(), 81.0);
        assert!(device.is_charging());
    }

    #[test]
    fn missing_service_leaves_the_device_not_ready() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let upower = UPower::start(&event_loop.handle(), bus.kind()).unwrap();
        let end = Instant::now() + Duration::from_millis(800);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(!upower.display_device.ready.get_untracked());
    }

    #[test]
    fn battery_states_decode() {
        assert_eq!(BatteryState::from_raw(1), BatteryState::Charging);
        assert_eq!(BatteryState::from_raw(4), BatteryState::FullyCharged);
        assert_eq!(BatteryState::from_raw(99), BatteryState::Unknown);
    }
}
