use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use acanthus_core::Signal;
use acanthus_io::{IoError, Watch};
use calloop::LoopHandle;

use crate::kit::{Cmd, Exec};
use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;

/// Default change of the raise and lower actions, in percent.
pub const STEP: i32 = 5;
/// Directory of the kernel's backlight devices.
pub const BACKLIGHT_ROOT: &str = "/sys/class/backlight";
/// Directory of the kernel's LED devices.
pub const LEDS_ROOT: &str = "/sys/class/leds";
const DDC_TIMEOUT: Duration = Duration::from_secs(5);
const DDC_DETECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Where brightness is controlled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Backend {
    /// Not determined yet.
    #[default]
    Unknown,
    /// A kernel backlight device with this name, set through logind.
    Backlight(String),
    /// An external monitor over DDC/CI.
    Ddc,
    /// No way to control brightness.
    None,
}

/// A kernel backlight device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacklightDevice {
    /// Device name.
    pub name: String,
    /// Directory of the device.
    pub path: PathBuf,
    /// Current raw value.
    pub current: u64,
    /// Largest raw value.
    pub max: u64,
}

impl BacklightDevice {
    /// The brightness from 0 to 100, rounded.
    pub fn percent(&self) -> u32 {
        (self.current * 100 + self.max / 2).checked_div(self.max).map_or(0, |p| p.min(100) as u32)
    }
}

/// The raw value for `percent` (clamped to 0 to 100) on a device with the largest value `max`.
pub fn raw_for_percent(percent: i32, max: u64) -> u64 {
    let percent = percent.clamp(0, 100) as u64;
    (percent * max + 50) / 100
}

/// The backlight devices below `root`, the one with the largest range first.
pub fn read_backlights(root: &Path) -> Vec<BacklightDevice> {
    let mut devices: Vec<BacklightDevice> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let read = |name: &str| std::fs::read_to_string(path.join(name)).ok()?.trim().parse::<u64>().ok();
            Some(BacklightDevice {
                name: entry.file_name().to_string_lossy().into_owned(),
                current: read("brightness")?,
                max: read("max_brightness")?,
                path,
            })
        })
        .filter(|d| d.max > 0)
        .collect();
    devices.sort_by(|a, b| b.max.cmp(&a.max).then_with(|| a.name.cmp(&b.name)));
    devices
}

/// The I2C bus number in the output of the DDC detect command.
pub fn parse_ddc_bus(detect_output: &str) -> Option<u32> {
    detect_output.lines().find(|l| l.contains("I2C bus:")).and_then(|l| {
        let path = l.split_whitespace().nth(2)?;
        path.rsplit("i2c-").next()?.parse().ok()
    })
}

/// The current and maximum value in the output of a DDC read command.
pub fn parse_vcp(output: &str) -> Option<(u32, u32)> {
    let fields: Vec<&str> = output.split_whitespace().collect();
    let (current, maximum) = (fields.get(3)?.parse::<u32>().ok()?, fields.get(4)?.parse::<u32>().ok()?);
    (maximum > 0).then_some((current, maximum))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DdcOp {
    Get,
    Set(i32),
    Adjust(i32),
}

#[derive(Default)]
struct DdcState {
    bus: Option<u32>,
    maximum: Option<u32>,
    queue: VecDeque<DdcOp>,
    busy: bool,
}

/// Brightness control of an external monitor through a DDC/CI program; operations are queued and run one at a time.
pub struct Ddc {
    exec: Rc<dyn Exec>,
    program: String,
    state: RefCell<DdcState>,
    percent: Signal<Option<u32>>,
}

impl Ddc {
    /// Creates a controller running `program` through `exec` and publishing the level into `percent`.
    pub fn new(exec: Rc<dyn Exec>, program: &str, percent: Signal<Option<u32>>) -> Rc<Ddc> {
        Rc::new(Ddc { exec, program: program.to_owned(), state: RefCell::default(), percent })
    }

    fn run_text(&self, argv: Vec<String>, timeout: Duration, done: Box<dyn FnOnce(Option<String>)>) {
        self.exec
            .run(Cmd::new(argv).timeout(timeout), Box::new(move |info| done(info.success().then_some(info.stdout))));
    }

    /// Queues an operation; a new `Set` replaces queued ones.
    pub fn submit(self: &Rc<Self>, op: DdcOpPublic) {
        let op = match op {
            DdcOpPublic::Get => DdcOp::Get,
            DdcOpPublic::Set(v) => DdcOp::Set(v),
            DdcOpPublic::Adjust(d) => DdcOp::Adjust(d),
        };
        {
            let mut state = self.state.borrow_mut();
            if let DdcOp::Set(_) = op {
                state.queue.retain(|queued| !matches!(queued, DdcOp::Set(_)));
            }
            state.queue.push_back(op);
            if state.busy {
                return;
            }
            state.busy = true;
        }
        self.next();
    }

    fn next(self: &Rc<Self>) {
        let op = {
            let mut state = self.state.borrow_mut();
            match state.queue.pop_front() {
                Some(op) => op,
                None => {
                    state.busy = false;
                    return;
                }
            }
        };
        self.execute(op, 0);
    }

    fn finish(self: &Rc<Self>) {
        self.next();
    }

    fn argv(&self, rest: &[&str]) -> Vec<String> {
        std::iter::once(self.program.as_str()).chain(rest.iter().copied()).map(str::to_owned).collect()
    }

    fn with_bus(self: &Rc<Self>, then: impl FnOnce(&Rc<Ddc>, Option<u32>) + 'static) {
        let cached = self.state.borrow().bus;
        if let Some(bus) = cached {
            return then(self, Some(bus));
        }
        let this = self.clone();
        self.run_text(
            self.argv(&["detect", "--brief"]),
            DDC_DETECT_TIMEOUT,
            Box::new(move |out| {
                let bus = out.as_deref().and_then(parse_ddc_bus);
                this.state.borrow_mut().bus = bus;
                then(&this, bus);
            }),
        );
    }

    fn read_vcp(self: &Rc<Self>, bus: u32, then: impl FnOnce(&Rc<Ddc>, Option<(u32, u32)>) + 'static) {
        let this = self.clone();
        self.run_text(
            self.argv(&["--bus", &bus.to_string(), "--brief", "getvcp", "10"]),
            DDC_TIMEOUT,
            Box::new(move |out| {
                let parsed = out.as_deref().and_then(parse_vcp);
                if let Some((_, maximum)) = parsed {
                    this.state.borrow_mut().maximum = Some(maximum);
                }
                then(&this, parsed);
            }),
        );
    }

    fn forget(&self) {
        let mut state = self.state.borrow_mut();
        state.bus = None;
        state.maximum = None;
    }

    fn execute(self: &Rc<Self>, op: DdcOp, attempt: u32) {
        let retry = move |this: &Rc<Ddc>| {
            this.forget();
            if attempt == 0 {
                this.execute(op, 1);
            } else {
                this.finish();
            }
        };
        self.with_bus(move |this, bus| {
            let Some(bus) = bus else { return retry(this) };
            match op {
                DdcOp::Get => this.read_vcp(bus, move |this, vcp| match vcp {
                    Some((current, maximum)) => {
                        this.percent.set(Some(current * 100 / maximum));
                        this.finish();
                    }
                    None => retry(this),
                }),
                DdcOp::Adjust(delta) => this.read_vcp(bus, move |this, vcp| match vcp {
                    Some((current, maximum)) => {
                        let target = (current as i32 * 100 / maximum as i32 + delta).clamp(0, 100);
                        this.write(bus, maximum, target, attempt);
                    }
                    None => retry(this),
                }),
                DdcOp::Set(target) => {
                    let known = this.state.borrow().maximum;
                    match known {
                        Some(maximum) => this.write(bus, maximum, target.clamp(0, 100), attempt),
                        None => this.read_vcp(bus, move |this, vcp| match vcp {
                            Some((_, maximum)) => this.write(bus, maximum, target.clamp(0, 100), attempt),
                            None => retry(this),
                        }),
                    }
                }
            }
        });
    }

    fn write(self: &Rc<Self>, bus: u32, maximum: u32, percent: i32, attempt: u32) {
        let raw = (percent as u32 * maximum) / 100;
        let this = self.clone();
        self.run_text(
            self.argv(&["--bus", &bus.to_string(), "setvcp", "10", &raw.to_string()]),
            DDC_TIMEOUT,
            Box::new(move |out| {
                if out.is_some() {
                    this.percent.set(Some(percent as u32));
                    this.finish();
                } else {
                    this.forget();
                    if attempt == 0 {
                        this.execute(DdcOp::Set(percent), 1);
                    } else {
                        this.finish();
                    }
                }
            }),
        );
    }
}

/// An operation on a DDC monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdcOpPublic {
    /// Read the current level.
    Get,
    /// Set the level in percent.
    Set(i32),
    /// Change the level by this many percent.
    Adjust(i32),
}

enum Event {
    SetFailed(String),
}

enum Command {
    Set { device: String, raw: u64 },
}

async fn run(bus: BusKind, events: EventSink<Event>, commands: async_channel::Receiver<Command>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let session = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
        )
        .await?;
        let mut commands = std::pin::pin!(commands);
        while let Some(Command::Set { device, raw }) = futures_lite::StreamExt::next(&mut commands).await {
            let reply: zbus::Result<()> =
                session.call("SetBrightness", &("backlight", device.as_str(), raw as u32)).await;
            if let Err(e) = reply {
                events.send(Event::SetFailed(e.to_string()));
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("logind brightness unavailable: {e}");
    }
}

/// Screen brightness from the best available backend, as signals.
pub struct Brightness {
    /// Brightness from 0 to 100, `None` while unknown.
    pub percent: Signal<Option<u32>>,
    /// The backend in use.
    pub backend: Signal<Backend>,
    root: PathBuf,
    service: ServiceHandle<Command>,
    ddc: Option<Rc<Ddc>>,
    watches: Vec<Watch>,
    last_error: Signal<String>,
}

fn pick_backend(root: &Path, ddc_available: bool) -> Backend {
    match read_backlights(root).into_iter().next() {
        Some(device) => Backend::Backlight(device.name),
        None if ddc_available => Backend::Ddc,
        None => Backend::None,
    }
}

impl Brightness {
    /// Picks a backlight device below `backlight_root`, or DDC when `ddc_exec` names a program, and starts the logind worker on `bus`.
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        bus: BusKind,
        backlight_root: &Path,
        ddc_exec: Option<(Rc<dyn Exec>, &str)>,
    ) -> Result<Brightness, IoError> {
        let percent = Signal::new(None);
        let last_error = Signal::new(String::new());
        let error_target = last_error.clone();
        let service = spawn_supervised(
            handle,
            "acanthus-brightness",
            move |event| match event {
                Event::SetFailed(message) => error_target.set(message),
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        let ddc = ddc_exec.map(|(exec, program)| Ddc::new(exec, program, percent.clone()));
        let backend = Signal::new(pick_backend(backlight_root, ddc.is_some()));
        let mut brightness = Brightness {
            percent,
            backend,
            root: backlight_root.to_path_buf(),
            service,
            ddc,
            watches: Vec::new(),
            last_error,
        };
        brightness.refresh();
        for device in read_backlights(backlight_root) {
            let target = brightness.percent.clone();
            let root = backlight_root.to_path_buf();
            let watched = device.name.clone();
            let watch = Watch::modify(handle, &device.path.join("brightness"), move || {
                if let Some(d) = read_backlights(&root).into_iter().find(|d| d.name == watched) {
                    target.set(Some(d.percent()));
                }
            })?;
            brightness.watches.push(watch);
        }
        Ok(brightness)
    }

    /// Rereads the level from the device.
    pub fn refresh(&self) {
        match self.backend.get_untracked() {
            Backend::Backlight(name) => {
                if let Some(device) = read_backlights(&self.root).into_iter().find(|d| d.name == name) {
                    self.percent.set(Some(device.percent()));
                }
            }
            Backend::Ddc => {
                if let Some(ddc) = &self.ddc {
                    ddc.submit(DdcOpPublic::Get);
                }
            }
            Backend::Unknown | Backend::None => {}
        }
    }

    /// Sets the level in percent, clamped to 0 to 100.
    pub fn set(&self, percent: i32) {
        let target = percent.clamp(0, 100);
        match self.backend.get_untracked() {
            Backend::Backlight(name) => {
                let Some(device) = read_backlights(&self.root).into_iter().find(|d| d.name == name) else { return };
                self.service.send(Command::Set { device: name, raw: raw_for_percent(target, device.max) });
                self.percent.set(Some(target as u32));
            }
            Backend::Ddc => {
                if let Some(ddc) = &self.ddc {
                    ddc.submit(DdcOpPublic::Set(target));
                }
            }
            Backend::Unknown | Backend::None => {}
        }
    }

    /// Changes the level by `delta` percent.
    pub fn adjust(&self, delta: i32) {
        match self.backend.get_untracked() {
            Backend::Backlight(_) => self.set(self.percent.get_untracked().map_or(0, |p| p as i32) + delta),
            Backend::Ddc => {
                if let Some(ddc) = &self.ddc {
                    ddc.submit(DdcOpPublic::Adjust(delta));
                }
            }
            Backend::Unknown | Backend::None => {}
        }
    }

    /// Raises the level by [`STEP`].
    pub fn raise(&self) {
        self.adjust(STEP);
    }

    /// Lowers the level by [`STEP`].
    pub fn lower(&self) {
        self.adjust(-STEP);
    }

    /// Message of the last failed change, empty when none failed.
    pub fn last_error(&self) -> Signal<String> {
        self.last_error.clone()
    }
}

#[cfg(test)]
mod tests {

    use serde_json::json;

    use super::*;
    use crate::kit::testing::FakeExec;

    fn queue(fake: &FakeExec, replies: &[Option<&str>]) {
        fake.set_immediate(true);
        for reply in replies {
            match reply {
                Some(text) => fake.reply_ok(text),
                None => fake.reply_fail(),
            }
        }
    }

    fn commands(fake: &FakeExec) -> Vec<String> {
        fake.calls().iter().map(|c| c[1..].join(" ")).collect()
    }

    const DETECT: &str = "Display 1\n   I2C bus:  /dev/i2c-5\n   Monitor: X\n";

    fn make_ddc(runner: &Rc<FakeExec>) -> (Rc<Ddc>, Signal<Option<u32>>) {
        let percent = Signal::new(None);
        (Ddc::new(runner.exec(), "ddcutil", percent.clone()), percent)
    }

    #[test]
    fn parses_detect_and_vcp_output() {
        assert_eq!(parse_ddc_bus(DETECT), Some(5));
        assert_eq!(parse_ddc_bus("No displays found"), None);
        assert_eq!(parse_vcp("VCP 10 C 30 100"), Some((30, 100)));
        assert_eq!(parse_vcp("VCP 10 C 30 0"), None);
        assert_eq!(parse_vcp("garbage"), None);
    }

    #[test]
    fn get_detects_the_bus_then_reads_the_percentage() {
        let runner = FakeExec::new();
        queue(&runner, &[Some(DETECT), Some("VCP 10 C 30 120")]);
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Get);
        assert_eq!(percent.get_untracked(), Some(25));
        assert_eq!(commands(&runner), ["detect --brief", "--bus 5 --brief getvcp 10"]);
    }

    #[test]
    fn set_reuses_the_cached_bus_and_maximum() {
        let runner = FakeExec::new();
        queue(&runner, &[Some(DETECT), Some("VCP 10 C 30 200"), Some(""), Some("")]);
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Get);
        ddc.submit(DdcOpPublic::Set(50));
        assert_eq!(percent.get_untracked(), Some(50));
        assert_eq!(commands(&runner).last().unwrap(), "--bus 5 setvcp 10 100");
        let before = runner.call_count();
        queue(&runner, &[Some("")]);
        ddc.submit(DdcOpPublic::Set(10));
        assert_eq!(runner.call_count(), before + 1);
        assert_eq!(commands(&runner).last().unwrap(), "--bus 5 setvcp 10 20");
    }

    #[test]
    fn adjust_reads_then_writes_with_clamping() {
        let runner = FakeExec::new();
        queue(&runner, &[Some(DETECT), Some("VCP 10 C 98 100"), Some("")]);
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Adjust(5));
        assert_eq!(percent.get_untracked(), Some(100));
        assert_eq!(commands(&runner).last().unwrap(), "--bus 5 setvcp 10 100");
    }

    #[test]
    fn failures_forget_the_cache_and_retry_once() {
        let runner = FakeExec::new();
        queue(&runner, &[Some(DETECT), None, Some(DETECT), Some("VCP 10 C 40 100")]);
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Get);
        assert_eq!(percent.get_untracked(), Some(40));
        assert_eq!(commands(&runner).iter().filter(|c| c.starts_with("detect")).count(), 2);

        let runner = FakeExec::new();
        queue(&runner, &[None, None]);
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Get);
        assert_eq!(percent.get_untracked(), None);
        assert_eq!(runner.call_count(), 2);
        queue(&runner, &[Some(DETECT), Some("VCP 10 C 10 100")]);
        ddc.submit(DdcOpPublic::Get);
        assert_eq!(percent.get_untracked(), Some(10));
    }

    #[test]
    fn queued_sets_coalesce_to_the_latest_while_busy() {
        let runner = FakeExec::new();
        let (ddc, percent) = make_ddc(&runner);
        ddc.submit(DdcOpPublic::Set(10));
        ddc.submit(DdcOpPublic::Set(20));
        ddc.submit(DdcOpPublic::Set(30));
        assert_eq!(runner.call_count(), 1);
        queue(&runner, &[Some(DETECT), Some("VCP 10 C 0 100"), Some(""), Some("")]);
        runner.release_next();
        assert_eq!(percent.get_untracked(), Some(30));
        let sets: Vec<String> = commands(&runner).into_iter().filter(|c| c.contains("setvcp")).collect();
        assert_eq!(sets, ["--bus 5 setvcp 10 10", "--bus 5 setvcp 10 30"]);
    }

    fn fake_backlight(brightness: u64, max: u64) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, cur, max) in [("intel_backlight", brightness, max), ("tiny", 1, 5)] {
            let device = dir.path().join(name);
            std::fs::create_dir_all(&device).unwrap();
            std::fs::write(device.join("brightness"), cur.to_string()).unwrap();
            std::fs::write(device.join("max_brightness"), max.to_string()).unwrap();
        }
        dir
    }

    #[test]
    fn backlight_devices_are_ranked_by_range_and_converted_to_percent() {
        let dir = fake_backlight(255, 1000);
        let devices = read_backlights(dir.path());
        assert_eq!(devices[0].name, "intel_backlight");
        assert_eq!(devices[0].percent(), 26);
        assert_eq!(raw_for_percent(50, 1000), 500);
        assert_eq!(raw_for_percent(150, 1000), 1000);
        assert_eq!(raw_for_percent(-3, 1000), 0);
        assert_eq!(pick_backend(dir.path(), true), Backend::Backlight("intel_backlight".into()));
        assert_eq!(pick_backend(Path::new("/nonexistent"), true), Backend::Ddc);
        assert_eq!(pick_backend(Path::new("/nonexistent"), false), Backend::None);
    }

    #[test]
    fn the_real_backlight_reads_without_panicking() {
        for device in read_backlights(Path::new("/sys/class/backlight")) {
            assert!(device.percent() <= 100);
        }
    }

    #[test]
    fn service_follows_external_changes_and_sends_logind_requests() {
        use calloop::EventLoop;
        use zbus::interface;

        use crate::platform::dbus::testbus::TestBus;

        struct Session(std::sync::mpsc::Sender<(String, String, u32)>);

        #[interface(name = "org.freedesktop.login1.Session")]
        impl Session {
            fn set_brightness(&self, subsystem: String, name: String, value: u32) {
                self.0.send((subsystem, name, value)).unwrap();
            }
        }

        let bus = TestBus::start();
        let (tx, rx) = std::sync::mpsc::channel();
        let address = bus.address.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name("org.freedesktop.login1")
                    .serve_at("/org/freedesktop/login1/session/auto", Session(tx))
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_secs(20));
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let dir = fake_backlight(200, 1000);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let brightness = Brightness::start(&event_loop.handle(), bus.kind(), dir.path(), None).unwrap();
        assert_eq!(brightness.percent.get_untracked(), Some(20));
        assert_eq!(brightness.backend.get_untracked(), Backend::Backlight("intel_backlight".into()));

        std::fs::write(dir.path().join("intel_backlight/brightness"), "600").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while brightness.percent.get_untracked() != Some(60) && std::time::Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert_eq!(brightness.percent.get_untracked(), Some(60));

        brightness.raise();
        assert_eq!(brightness.percent.get_untracked(), Some(65));
        brightness.set(250);
        let request = rx.recv_timeout(Duration::from_secs(10));
        let mut seen = Vec::new();
        if let Ok(r) = request {
            seen.push(r);
        }
        if let Ok(r) = rx.recv_timeout(Duration::from_secs(10)) {
            seen.push(r);
        }
        assert_eq!(seen[0], ("backlight".to_owned(), "intel_backlight".to_owned(), 650));
        assert_eq!(seen[1], ("backlight".to_owned(), "intel_backlight".to_owned(), 1000));
        let _ = json!(null);
    }
}
