use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use acanthus_io::{ExitInfo, Scheduler, TimerHandle};
use chrono::{DateTime, Local, TimeZone};

use super::{Clock, Cmd, Done, Exec, Notice, Notifier, Running, Translator};

/// A timer callback; it returns the delay of its next run, or `None` when done.
pub type TimerFn = Box<dyn FnMut() -> Option<Duration>>;
/// A registered timer: empty once cancelled or finished.
pub type TimerSlot = Rc<RefCell<Option<TimerFn>>>;
/// The timers registered with a manual scheduler, with their delay in milliseconds.
pub type TimerList = Rc<RefCell<Vec<(u64, TimerSlot)>>>;

/// A scheduler that never fires by itself: tests fire timers on demand.
#[derive(Clone, Default)]
pub struct ManualScheduler {
    timers: TimerList,
}

impl ManualScheduler {
    /// A scheduler with no timers.
    pub fn new() -> ManualScheduler {
        ManualScheduler::default()
    }

    /// The scheduler to hand to code under test.
    pub fn scheduler(&self) -> Scheduler {
        let registry = self.timers.clone();
        Rc::new(move |delay, f| {
            let slot: TimerSlot = Rc::new(RefCell::new(Some(f)));
            registry.borrow_mut().push((delay.as_millis() as u64, slot.clone()));
            Some(TimerHandle::from_fn(move || {
                slot.borrow_mut().take();
            }))
        })
    }

    /// The registry of timers.
    pub fn timers(&self) -> TimerList {
        self.timers.clone()
    }

    /// The number of timers ever registered, fired or not.
    pub fn registered(&self) -> usize {
        self.timers.borrow().len()
    }

    /// The number of timers that can still fire.
    pub fn live(&self) -> usize {
        self.timers.borrow().iter().filter(|(_, slot)| slot.borrow().is_some()).count()
    }

    /// The number of live timers registered with this delay in milliseconds.
    pub fn live_with_delay(&self, delay_ms: u64) -> usize {
        self.timers.borrow().iter().filter(|(d, slot)| *d == delay_ms && slot.borrow().is_some()).count()
    }

    /// The delays of all registered timers in milliseconds.
    pub fn delays(&self) -> Vec<u64> {
        self.timers.borrow().iter().map(|(delay, _)| *delay).collect()
    }

    /// Fires the timer at `index` and returns its next delay if it repeats.
    pub fn fire_at(&self, index: usize) -> Option<Duration> {
        let slot = self.timers.borrow().get(index).map(|(_, slot)| slot.clone())?;
        run_slot(&slot).flatten()
    }

    /// Fires every live timer with this delay and returns how many fired.
    pub fn fire_with_delay(&self, delay_ms: u64) -> usize {
        let slots: Vec<TimerSlot> =
            self.timers.borrow().iter().filter(|(d, _)| *d == delay_ms).map(|(_, slot)| slot.clone()).collect();
        slots.iter().filter(|slot| fire_slot(slot)).count()
    }

    /// Fires every live timer once, forgetting the registry, and returns how many fired.
    pub fn fire_all(&self) -> usize {
        let pending: Vec<TimerSlot> = self.timers.borrow_mut().drain(..).map(|(_, slot)| slot).collect();
        pending.iter().filter(|slot| fire_slot(slot)).count()
    }
}

fn run_slot(slot: &TimerSlot) -> Option<Option<Duration>> {
    let mut callback = slot.borrow_mut().take()?;
    let next = callback();
    if next.is_some() {
        *slot.borrow_mut() = Some(callback);
    }
    Some(next)
}

fn fire_slot(slot: &TimerSlot) -> bool {
    run_slot(slot).is_some()
}

/// A clock that only moves when told.
pub struct FakeClock {
    ms: Cell<f64>,
}

impl FakeClock {
    /// A clock at `ms` Unix milliseconds.
    pub fn from_ms(ms: f64) -> FakeClock {
        FakeClock { ms: Cell::new(ms) }
    }

    /// A clock at `secs` Unix seconds.
    pub fn from_secs(secs: f64) -> FakeClock {
        FakeClock::from_ms(secs * 1000.0)
    }

    /// A clock at `time`.
    pub fn at(time: DateTime<Local>) -> FakeClock {
        FakeClock::from_ms(time.timestamp_millis() as f64)
    }

    /// The current time in Unix milliseconds.
    pub fn get(&self) -> f64 {
        self.ms.get()
    }

    /// Sets the time in Unix milliseconds.
    pub fn set(&self, ms: f64) {
        self.ms.set(ms);
    }

    /// Sets the time in Unix seconds.
    pub fn set_secs(&self, secs: f64) {
        self.ms.set(secs * 1000.0);
    }

    /// Sets the time.
    pub fn set_time(&self, time: DateTime<Local>) {
        self.ms.set(time.timestamp_millis() as f64);
    }

    /// Moves the time forward by `ms` milliseconds.
    pub fn advance_ms(&self, ms: f64) {
        self.ms.set(self.ms.get() + ms);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Local> {
        Local.timestamp_millis_opt(self.ms.get() as i64).single().expect("a representable instant")
    }

    fn unix_ms(&self) -> f64 {
        self.ms.get()
    }
}

type Translate = Box<dyn Fn(&str, &[(&str, &str)]) -> String>;

/// A translator driven by a function.
pub struct FakeTranslator {
    translate: Translate,
}

impl FakeTranslator {
    /// A translator calling `translate` with the key and the arguments.
    pub fn from_fn(translate: impl Fn(&str, &[(&str, &str)]) -> String + 'static) -> FakeTranslator {
        FakeTranslator { translate: Box::new(translate) }
    }

    /// A translator returning the key followed by `|name=value` for every argument.
    pub fn echo() -> FakeTranslator {
        FakeTranslator::from_fn(|key, args| {
            let mut text = key.to_owned();
            for (name, value) in args {
                text = format!("{text}|{name}={value}");
            }
            text
        })
    }

    /// A translator returning the last dot-separated segment of the key.
    pub fn last_segment() -> FakeTranslator {
        FakeTranslator::from_fn(|key, _| key.rsplit('.').next().unwrap_or(key).to_owned())
    }

    /// The translator as a shared trait object.
    pub fn rc(self) -> Rc<dyn Translator> {
        Rc::new(self)
    }
}

impl Translator for FakeTranslator {
    fn t(&self, key: &str) -> String {
        (self.translate)(key, &[])
    }

    fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        (self.translate)(key, args)
    }
}

struct PendingRun {
    argv: Vec<String>,
    done: Option<Done>,
    killed: Rc<Cell<bool>>,
}

type LineFn = Box<dyn FnMut(&str)>;

struct FakeStream {
    on_line: Option<LineFn>,
    on_exit: Option<Done>,
    stopped: Rc<Cell<bool>>,
}

#[derive(Default)]
struct ExecState {
    calls: Vec<Cmd>,
    detached: Vec<Cmd>,
    replies: VecDeque<ExitInfo>,
    rules: Vec<(Vec<String>, ExitInfo)>,
    pending: Vec<PendingRun>,
    streams: Vec<FakeStream>,
    missing: HashSet<String>,
    refuse_spawn: bool,
    killed_runs: usize,
}

type Responder = Box<dyn Fn(&[String]) -> Option<ExitInfo>>;

/// An [`Exec`] that records commands and answers them from rules, queued replies or a responder, without starting processes.
pub struct FakeExec {
    state: Rc<RefCell<ExecState>>,
    immediate: Cell<bool>,
    responders: RefCell<Vec<Responder>>,
}

struct FakeRunning {
    pid: u32,
    flag: Rc<Cell<bool>>,
    state: Rc<RefCell<ExecState>>,
    kills_on_drop: bool,
}

impl Running for FakeRunning {
    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn kill(&self) {
        if !self.flag.replace(true) && !self.kills_on_drop {
            self.state.borrow_mut().killed_runs += 1;
        }
    }
}

impl Drop for FakeRunning {
    fn drop(&mut self) {
        if self.kills_on_drop {
            self.flag.set(true);
        }
    }
}

impl FakeExec {
    /// A fake that holds started commands until released.
    pub fn new() -> Rc<FakeExec> {
        Rc::new(FakeExec {
            state: Rc::new(RefCell::new(ExecState::default())),
            immediate: Cell::new(false),
            responders: RefCell::new(Vec::new()),
        })
    }

    /// A fake that answers commands as soon as they start.
    pub fn immediate() -> Rc<FakeExec> {
        let fake = FakeExec::new();
        fake.immediate.set(true);
        fake
    }

    /// The fake as a shared [`Exec`].
    pub fn exec(self: &Rc<FakeExec>) -> Rc<dyn Exec> {
        self.clone()
    }

    /// Sets whether commands are answered as soon as they start.
    pub fn set_immediate(&self, immediate: bool) {
        self.immediate.set(immediate);
    }

    /// Queues a reply for the next command no rule matches.
    pub fn reply(&self, info: ExitInfo) {
        self.state.borrow_mut().replies.push_back(info);
    }

    /// Answers commands whose arguments start with `prefix` with `info`.
    pub fn rule(&self, prefix: &[&str], info: ExitInfo) {
        self.state.borrow_mut().rules.push((prefix.iter().map(|part| (*part).to_owned()).collect(), info));
    }

    /// Answers commands starting with `prefix` with a successful run printing `stdout`.
    pub fn rule_ok(&self, prefix: &[&str], stdout: &str) {
        self.rule(prefix, ExitInfo { code: Some(0), stdout: stdout.to_owned(), ..Default::default() });
    }

    /// Queues a successful reply printing `stdout`.
    pub fn reply_ok(&self, stdout: &str) {
        self.reply(ExitInfo { code: Some(0), stdout: stdout.to_owned(), ..Default::default() });
    }

    /// Queues a failed reply.
    pub fn reply_fail(&self) {
        self.reply(ExitInfo { code: Some(22), ..Default::default() });
    }

    /// The commands started so far.
    pub fn cmds(&self) -> Vec<Cmd> {
        self.state.borrow().calls.clone()
    }

    /// The argument vectors of the commands started so far.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.state.borrow().calls.iter().map(|cmd| cmd.argv.clone()).collect()
    }

    /// The number of commands started so far.
    pub fn call_count(&self) -> usize {
        self.state.borrow().calls.len()
    }

    /// Forgets the recorded commands.
    pub fn clear_calls(&self) {
        self.state.borrow_mut().calls.clear();
    }

    /// The last argument of every command, which is the URL for HTTP requests.
    pub fn urls(&self) -> Vec<String> {
        self.calls().iter().filter_map(|argv| argv.last().cloned()).collect()
    }

    /// The commands started detached.
    pub fn detached(&self) -> Vec<Vec<String>> {
        self.state.borrow().detached.iter().map(|cmd| cmd.argv.clone()).collect()
    }

    /// The number of commands waiting to be released.
    pub fn pending(&self) -> usize {
        self.state.borrow().pending.len()
    }

    /// The number of commands that were killed.
    pub fn killed(&self) -> usize {
        self.state.borrow().killed_runs
    }

    /// Makes the program names count as not installed.
    pub fn missing(&self, names: &[&str]) {
        self.state.borrow_mut().missing.extend(names.iter().map(|name| (*name).to_owned()));
    }

    /// Makes the program name count as installed again.
    pub fn present(&self, name: &str) {
        self.state.borrow_mut().missing.remove(name);
    }

    /// Makes starting commands fail when `refuse` is true.
    pub fn refuse_spawn(&self, refuse: bool) {
        self.state.borrow_mut().refuse_spawn = refuse;
    }

    /// Adds a function that answers commands; the first function that returns a reply wins over rules and queued replies.
    pub fn responder(&self, respond: impl Fn(&[String]) -> Option<ExitInfo> + 'static) {
        self.responders.borrow_mut().push(Box::new(respond));
    }

    fn reply_for(&self, argv: &[String]) -> ExitInfo {
        let answered = self.responders.borrow().iter().find_map(|respond| respond(argv));
        if let Some(info) = answered {
            return info;
        }
        let mut state = self.state.borrow_mut();
        let matched =
            state.rules.iter().rev().find(|(prefix, _)| argv.starts_with(prefix)).map(|(_, info)| info.clone());
        matched.or_else(|| state.replies.pop_front()).unwrap_or(ExitInfo { code: Some(7), ..Default::default() })
    }

    /// Finishes the oldest waiting command; false when none waits.
    pub fn release_next(&self) -> bool {
        let run = {
            let mut state = self.state.borrow_mut();
            if state.pending.is_empty() { None } else { Some(state.pending.remove(0)) }
        };
        let Some(mut run) = run else { return false };
        let info = if run.killed.get() {
            ExitInfo { code: None, signal: Some(15), ..Default::default() }
        } else {
            self.reply_for(&run.argv)
        };
        if let Some(done) = run.done.take() {
            done(info);
        }
        true
    }

    /// Finishes all waiting commands and returns how many.
    pub fn release(&self) -> usize {
        let mut released = 0;
        while self.release_next() {
            released += 1;
        }
        released
    }

    /// The number of streaming commands started.
    pub fn streams(&self) -> usize {
        self.state.borrow().streams.len()
    }

    /// The number of streaming commands still running.
    pub fn streams_alive(&self) -> usize {
        self.state.borrow().streams.iter().filter(|stream| !stream.stopped.get()).count()
    }

    /// Delivers a line of output to the streaming command at `index`.
    pub fn send_line(&self, index: usize, text: &str) {
        let taken = self.state.borrow_mut().streams.get_mut(index).and_then(|stream| stream.on_line.take());
        if let Some(mut on_line) = taken {
            on_line(text);
            if let Some(stream) = self.state.borrow_mut().streams.get_mut(index) {
                stream.on_line = Some(on_line);
            }
        }
    }

    /// Ends the streaming command at `index` with a successful exit.
    pub fn end_stream(&self, index: usize) {
        let taken = self.state.borrow_mut().streams.get_mut(index).and_then(|stream| stream.on_exit.take());
        if let Some(on_exit) = taken {
            on_exit(ExitInfo { code: Some(0), ..Default::default() });
        }
    }
}

impl Exec for FakeExec {
    fn run(&self, cmd: Cmd, done: Done) -> Option<Box<dyn Running>> {
        let killed = Rc::new(Cell::new(false));
        let argv = cmd.argv.clone();
        let (refuse, pid) = {
            let mut state = self.state.borrow_mut();
            state.calls.push(cmd);
            (state.refuse_spawn, 1000 + state.calls.len() as u32)
        };
        if refuse {
            done(ExitInfo { code: None, ..Default::default() });
            return None;
        }
        let running = FakeRunning { pid, flag: killed.clone(), state: self.state.clone(), kills_on_drop: false };
        if self.immediate.get() {
            done(self.reply_for(&argv));
        } else {
            self.state.borrow_mut().pending.push(PendingRun { argv, done: Some(done), killed });
        }
        Some(Box::new(running))
    }

    fn spawn(&self, cmd: Cmd, on_line: Box<dyn FnMut(&str)>, on_exit: Done) -> Option<Box<dyn Running>> {
        let stopped = Rc::new(Cell::new(false));
        let mut state = self.state.borrow_mut();
        state.calls.push(cmd);
        if state.refuse_spawn {
            return None;
        }
        let pid = 1000 + state.calls.len() as u32;
        state.streams.push(FakeStream { on_line: Some(on_line), on_exit: Some(on_exit), stopped: stopped.clone() });
        Some(Box::new(FakeRunning { pid, flag: stopped, state: self.state.clone(), kills_on_drop: true }))
    }

    fn detached(&self, cmd: Cmd) {
        self.state.borrow_mut().detached.push(cmd);
    }

    fn has_program(&self, name: &str) -> bool {
        !self.state.borrow().missing.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_register_with_their_delay_and_fire_once() {
        let manual = ManualScheduler::new();
        let schedule = manual.scheduler();
        let hits = Rc::new(Cell::new(0));
        let counter = hits.clone();
        let _handle = schedule(
            Duration::from_millis(250),
            Box::new(move || {
                counter.set(counter.get() + 1);
                None
            }),
        );
        assert_eq!(manual.delays(), [250]);
        assert_eq!(manual.live(), 1);
        assert_eq!(manual.fire_at(0), None);
        assert_eq!(hits.get(), 1);
        assert_eq!(manual.live(), 0);
        manual.fire_at(0);
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn a_repeating_callback_stays_alive() {
        let manual = ManualScheduler::new();
        let schedule = manual.scheduler();
        let hits = Rc::new(Cell::new(0));
        let counter = hits.clone();
        let _handle = schedule(
            Duration::from_millis(10),
            Box::new(move || {
                counter.set(counter.get() + 1);
                Some(Duration::from_millis(10))
            }),
        );
        assert_eq!(manual.fire_at(0), Some(Duration::from_millis(10)));
        manual.fire_at(0);
        assert_eq!(hits.get(), 2);
        assert_eq!(manual.live_with_delay(10), 1);
    }

    #[test]
    fn dropping_the_handle_cancels_the_timer() {
        let manual = ManualScheduler::new();
        let schedule = manual.scheduler();
        let handle = schedule(Duration::from_millis(5), Box::new(|| None)).unwrap();
        assert_eq!(manual.live(), 1);
        drop(handle);
        assert_eq!(manual.live(), 0);
        assert_eq!(manual.registered(), 1);
        assert_eq!(manual.fire_all(), 0);
        assert_eq!(manual.registered(), 0);
    }

    #[test]
    fn firing_by_delay_only_touches_matching_timers() {
        let manual = ManualScheduler::new();
        let schedule = manual.scheduler();
        let hits = Rc::new(Cell::new(0));
        let handles: Vec<_> = [100u64, 200, 100]
            .into_iter()
            .map(|ms| {
                let counter = hits.clone();
                schedule(
                    Duration::from_millis(ms),
                    Box::new(move || {
                        counter.set(counter.get() + ms);
                        None
                    }),
                )
            })
            .collect();
        assert_eq!(manual.fire_with_delay(100), 2);
        assert_eq!(hits.get(), 200);
        assert_eq!(manual.live(), 1);
        drop(handles);
    }

    #[test]
    fn the_fake_clock_reports_one_instant_in_every_unit() {
        let clock = FakeClock::from_secs(1_700_000_000.5);
        assert_eq!(clock.unix_ms(), 1_700_000_000_500.0);
        assert_eq!(clock.unix_secs(), 1_700_000_000);
        assert_eq!(clock.unix_secs_f64(), 1_700_000_000.5);
        clock.advance_ms(1500.0);
        assert_eq!(clock.unix_secs(), 1_700_000_002);
        clock.set_secs(10.0);
        assert_eq!(clock.get(), 10_000.0);
    }

    #[test]
    fn the_fake_translators_format_keys_and_arguments() {
        assert_eq!(FakeTranslator::echo().t("a.b"), "a.b");
        assert_eq!(FakeTranslator::echo().t_args("a", &[("n", "1")]), "a|n=1");
        assert_eq!(FakeTranslator::last_segment().t("weather.desc.offline"), "offline");
    }

    fn collect() -> (Rc<RefCell<Vec<ExitInfo>>>, Done) {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        (seen, Box::new(move |info| sink.borrow_mut().push(info)))
    }

    #[test]
    fn replies_are_deferred_until_released_in_order() {
        let fake = FakeExec::new();
        let exec = fake.exec();
        fake.reply_ok("first");
        fake.reply_ok("second");
        let (seen, done) = collect();
        exec.run(Cmd::new(["a"]), done);
        let (more, done) = collect();
        exec.run(Cmd::new(["b"]), done);
        assert_eq!(fake.pending(), 2);
        assert!(seen.borrow().is_empty());
        assert_eq!(fake.release(), 2);
        assert_eq!(seen.borrow()[0].stdout, "first");
        assert_eq!(more.borrow()[0].stdout, "second");
        assert_eq!(fake.calls(), [["a"], ["b"]]);
    }

    #[test]
    fn immediate_mode_replies_at_once_and_defaults_to_failure() {
        let fake = FakeExec::immediate();
        let exec = fake.exec();
        fake.reply_ok("x");
        let (seen, done) = collect();
        exec.run(Cmd::new(["a"]), done);
        let (empty, done) = collect();
        exec.run(Cmd::new(["b"]), done);
        assert_eq!(seen.borrow()[0].stdout, "x");
        assert!(!empty.borrow()[0].success());
        assert_eq!(fake.pending(), 0);
    }

    #[test]
    fn a_killed_run_completes_with_a_signal_and_is_counted() {
        let fake = FakeExec::new();
        let exec = fake.exec();
        fake.reply_ok("ignored");
        let (seen, done) = collect();
        let running = exec.run(Cmd::new(["a"]), done).unwrap();
        assert!(running.pid().is_some());
        running.kill();
        running.kill();
        assert_eq!(fake.killed(), 1);
        fake.release();
        assert_eq!(seen.borrow()[0].signal, Some(15));
    }

    #[test]
    fn streams_deliver_lines_and_stop_when_the_handle_drops() {
        let fake = FakeExec::new();
        let exec = fake.exec();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let sink = lines.clone();
        let (exits, done) = collect();
        let running = exec
            .spawn(Cmd::new(["cava"]), Box::new(move |line| sink.borrow_mut().push(line.to_owned())), done)
            .unwrap();
        fake.send_line(0, "1;2");
        fake.send_line(0, "3;4");
        assert_eq!(*lines.borrow(), ["1;2", "3;4"]);
        assert_eq!(fake.streams_alive(), 1);
        drop(running);
        assert_eq!(fake.streams_alive(), 0);
        fake.end_stream(0);
        assert_eq!(exits.borrow().len(), 1);
    }

    #[test]
    fn refused_spawns_fail_and_missing_programs_are_reported() {
        let fake = FakeExec::new();
        let exec = fake.exec();
        fake.missing(&["ddcutil"]);
        assert!(!exec.has_program("ddcutil"));
        assert!(exec.has_program("sh"));
        fake.refuse_spawn(true);
        let (seen, done) = collect();
        assert!(exec.run(Cmd::new(["a"]), done).is_none());
        assert!(!seen.borrow()[0].success());
        assert!(exec.spawn(Cmd::new(["a"]), Box::new(|_| {}), Box::new(|_| {})).is_none());
        exec.detached(Cmd::new(["b"]));
        assert_eq!(fake.detached(), [["b"]]);
    }
}

/// A [`Notifier`] that records the notices it receives.
#[derive(Default)]
pub struct FakeNotifier {
    sent: RefCell<Vec<Notice>>,
}

impl FakeNotifier {
    /// A notifier with nothing recorded.
    pub fn new() -> Rc<FakeNotifier> {
        Rc::new(FakeNotifier::default())
    }

    /// The notifier as a shared trait object.
    pub fn notifier(self: &Rc<FakeNotifier>) -> Rc<dyn Notifier> {
        self.clone()
    }

    /// The number of notices received.
    pub fn count(&self) -> usize {
        self.sent.borrow().len()
    }

    /// The summaries of the notices received.
    pub fn summaries(&self) -> Vec<String> {
        self.sent.borrow().iter().map(|notice| notice.summary.clone()).collect()
    }

    /// Reads the notice at `index`.
    pub fn with<R>(&self, index: usize, read: impl FnOnce(&Notice) -> R) -> R {
        read(&self.sent.borrow()[index])
    }

    /// Runs the action handler of the notice at `index` with the action id.
    pub fn choose(&self, index: usize, action: &str) {
        let handler = self.sent.borrow_mut()[index].on_action.take();
        if let Some(handler) = handler {
            handler(action.to_owned());
        }
    }
}

impl Notifier for FakeNotifier {
    fn notify(&self, notice: Notice) {
        self.sent.borrow_mut().push(notice);
    }
}

/// A complete [`Ctx`] built from fakes, plus handles to the fakes.
pub struct TestCtx {
    /// The context to hand to code under test.
    pub ctx: super::Ctx,
    /// The fake executor.
    pub exec: Rc<FakeExec>,
    /// The manual scheduler.
    pub scheduler: ManualScheduler,
    /// The fake clock.
    pub clock: Rc<FakeClock>,
    /// The recorded notifications.
    pub notices: Rc<FakeNotifier>,
}

impl TestCtx {
    /// Builds a context whose directories are below `root`.
    pub fn new(root: &std::path::Path) -> TestCtx {
        let exec = FakeExec::new();
        let scheduler = ManualScheduler::new();
        let clock = Rc::new(FakeClock::from_secs(1_000_000.0));
        let notices = FakeNotifier::new();
        let ctx = super::Ctx {
            exec: exec.exec(),
            blocking: super::Blocking::inline(),
            http: super::Http::new(exec.exec()),
            schedule: scheduler.scheduler(),
            clock: clock.clone(),
            i18n: FakeTranslator::echo().rc(),
            notify: notices.notifier(),
            dirs: super::Dirs::under(&acanthus_io::AppId::new("demo", "DM"), root),
        };
        TestCtx { ctx, exec, scheduler, clock, notices }
    }
}
