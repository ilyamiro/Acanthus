use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use acanthus_core::{Effect, Signal, batch, untrack};
use acanthus_io::{TimerHandle, timer};
use calloop::LoopHandle;
use chrono::{Datelike, Local, NaiveDateTime, Timelike};

use crate::qt_format::{format_qt, locale_for};

/// Default Qt pattern of the time (`HH:mm:ss`).
pub const DEFAULT_TIME_FORMAT: &str = "HH:mm:ss";
/// Default Qt pattern of the long date.
pub const DEFAULT_FULL_DATE: &str = "dddd, MMMM dd";

/// A Qt time pattern split into the parts a clock shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeFormat {
    /// The whole pattern.
    pub full: String,
    /// Pattern of the hour.
    pub hour: &'static str,
    /// Pattern of the minute.
    pub minute: &'static str,
    /// Pattern of the seconds, empty when the format has none.
    pub second: &'static str,
    /// Pattern of the AM/PM marker, empty in 24-hour formats.
    pub am_pm: &'static str,
    /// Whether the format uses a 12-hour clock.
    pub is_12_hour: bool,
    /// Whether the format shows seconds.
    pub has_seconds: bool,
}

impl TimeFormat {
    /// Splits a Qt time `format` into its parts.
    pub fn parse(format: &str) -> TimeFormat {
        let has = |s: &str| format.contains(s);
        let hour = if has("HH") {
            "HH"
        } else if has("hh") {
            "hh"
        } else if has("H") {
            "H"
        } else if has("h") {
            "h"
        } else {
            "HH"
        };
        let minute = if has("mm") {
            "mm"
        } else if has("m") {
            "m"
        } else {
            "mm"
        };
        let second = if has("ss") {
            "ss"
        } else if has("s") {
            "s"
        } else {
            ""
        };
        let am_pm = if has("AP") {
            "AP"
        } else if has("ap") {
            "ap"
        } else if has("A") || has("a") {
            "AP"
        } else {
            ""
        };
        TimeFormat {
            full: format.to_owned(),
            hour,
            minute,
            second,
            am_pm,
            is_12_hour: !am_pm.is_empty() || hour == "hh" || hour == "h",
            has_seconds: !second.is_empty(),
        }
    }
}

type Source = Rc<dyn Fn() -> NaiveDateTime>;

/// Current local date and time.
pub fn local_now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Reactive clock: formatted time and date as signals, refreshed on a timer.
#[derive(Clone)]
pub struct Clock(Rc<Inner>);

struct Inner {
    source: Source,
    format: Signal<String>,
    language: Signal<String>,
    full_date_pattern: Signal<String>,
    last_day: Cell<Option<u32>>,
    last_minute: Cell<Option<u32>>,
    last_language: RefCell<String>,
    effect: RefCell<Option<Effect>>,
    timer: RefCell<Option<TimerHandle>>,
    now: Signal<NaiveDateTime>,
    time: Signal<String>,
    time_short: Signal<String>,
    time_long: Signal<String>,
    hour: Signal<String>,
    minute: Signal<String>,
    second: Signal<String>,
    am_pm: Signal<String>,
    full_date: Signal<String>,
    short_date: Signal<String>,
    date_badge: Signal<String>,
    day: Signal<String>,
    day_short: Signal<String>,
    day_name: Signal<String>,
    day_name_short: Signal<String>,
    month: Signal<String>,
    month_short: Signal<String>,
    year: Signal<String>,
    has_seconds: Signal<bool>,
    is_12_hour: Signal<bool>,
}

macro_rules! accessors {
    ($($name:ident: $ty:ty),* $(,)?) => {
        $(#[doc = concat!("Signal of the `", stringify!($name), "` text or flag.")] pub fn $name(&self) -> Signal<$ty> {
            self.0.$name.clone()
        })*
    };
}

impl Clock {
    /// Clock on the local time zone with a fixed Qt time `format`, English names and the default long
    /// date; the right choice for an app that has no `I18n` or `Config`.
    pub fn simple(format: &str) -> Clock {
        Clock::new(
            local_now,
            Signal::new(format.to_owned()),
            Signal::new("en".to_owned()),
            Signal::new(DEFAULT_FULL_DATE.to_owned()),
        )
    }

    /// Clock fed by `source`, formatted with the Qt pattern in `format`, the names of `language` and the long date pattern in `full_date_pattern`.
    pub fn new(
        source: impl Fn() -> NaiveDateTime + 'static,
        format: Signal<String>,
        language: Signal<String>,
        full_date_pattern: Signal<String>,
    ) -> Clock {
        let text = || Signal::new(String::new());
        let initial = source();
        let clock = Clock(Rc::new(Inner {
            source: Rc::new(source),
            format,
            language,
            full_date_pattern,
            last_day: Cell::new(None),
            last_minute: Cell::new(None),
            last_language: RefCell::default(),
            effect: RefCell::new(None),
            timer: RefCell::new(None),
            now: Signal::new(initial),
            time: text(),
            time_short: text(),
            time_long: text(),
            hour: text(),
            minute: text(),
            second: text(),
            am_pm: text(),
            full_date: text(),
            short_date: text(),
            date_badge: text(),
            day: text(),
            day_short: text(),
            day_name: text(),
            day_name_short: text(),
            month: text(),
            month_short: text(),
            year: text(),
            has_seconds: Signal::new(true),
            is_12_hour: Signal::new(false),
        }));
        let weak = Rc::downgrade(&clock.0);
        let effect = Effect::new(move || {
            let Some(inner) = weak.upgrade() else { return };
            inner.format.get();
            inner.language.get();
            inner.full_date_pattern.get();
            untrack(|| Clock(inner).update(true));
        });
        *clock.0.effect.borrow_mut() = Some(effect);
        clock
    }

    accessors! {
        now: NaiveDateTime, time: String, time_short: String, time_long: String, hour: String, minute: String,
        second: String, am_pm: String, full_date: String, short_date: String, date_badge: String, day: String,
        day_short: String, day_name: String, day_name_short: String, month: String, month_short: String,
        year: String, has_seconds: bool, is_12_hour: bool,
    }

    /// Formats the current time with a Qt `pattern` in the clock's language.
    pub fn format(&self, pattern: &str) -> String {
        let locale = locale_for(&self.0.language.get_untracked());
        format_qt(&self.0.now.get_untracked(), pattern, locale)
    }

    /// Reads the time source and updates the signals that changed.
    pub fn tick(&self) {
        self.update(false);
    }

    fn update(&self, force: bool) {
        let inner = &self.0;
        let now = (inner.source)();
        let format = TimeFormat::parse(&inner.format.get_untracked());
        let language = inner.language.get_untracked();
        let locale = locale_for(&language);
        let fmt = |pattern: &str| format_qt(&now, pattern, locale);

        batch(|| {
            inner.now.set(now);
            inner.has_seconds.set(format.has_seconds);
            inner.is_12_hour.set(format.is_12_hour);
            inner.second.set(if format.has_seconds { fmt(format.second) } else { String::new() });
            let time = fmt(&format.full);
            inner.time.set(time.clone());
            inner.time_long.set(if format.full == DEFAULT_TIME_FORMAT { time } else { fmt(DEFAULT_TIME_FORMAT) });

            let minute_key = now.hour() * 60 + now.minute();
            if force || inner.last_minute.get() != Some(minute_key) {
                inner.last_minute.set(Some(minute_key));
                inner.minute.set(fmt(format.minute));
                inner.hour.set(fmt(format.hour));
                inner.am_pm.set(if format.am_pm.is_empty() { String::new() } else { fmt(format.am_pm) });
                let short = if format.am_pm.is_empty() {
                    format!("{}:{}", format.hour, format.minute)
                } else {
                    format!("{}:{} {}", format.hour, format.minute, format.am_pm)
                };
                inner.time_short.set(fmt(&short));
            }

            let day = now.day();
            if force || inner.last_day.get() != Some(day) || *inner.last_language.borrow() != language {
                inner.last_day.set(Some(day));
                *inner.last_language.borrow_mut() = language.clone();
                let pattern = inner.full_date_pattern.get_untracked();
                inner.full_date.set(fmt(if pattern.is_empty() { DEFAULT_FULL_DATE } else { &pattern }));
                let short = fmt("d MMM");
                inner.date_badge.set(short.to_uppercase());
                inner.short_date.set(short);
                inner.day.set(fmt("dd"));
                inner.day_short.set(fmt("d"));
                inner.day_name.set(fmt("dddd"));
                inner.day_name_short.set(fmt("ddd"));
                inner.month.set(fmt("MMMM"));
                inner.month_short.set(fmt("MMM"));
                inner.year.set(fmt("yyyy"));
            }
        });
    }

    /// Time until the next change of the displayed time: the next second, or the next minute when no seconds are shown.
    pub fn next_delay(&self) -> Duration {
        let now = (self.0.source)();
        let sub_ms = u64::from(now.nanosecond() / 1_000_000 % 1000);
        let to_second = 1000 - sub_ms;
        let ms = if self.0.has_seconds.get_untracked() {
            to_second
        } else {
            u64::from(59 - now.second()) * 1000 + to_second
        };
        Duration::from_millis(ms.max(1))
    }

    /// Updates the clock on a timer in `handle`'s event loop.
    pub fn attach<D: 'static>(&self, handle: &LoopHandle<'static, D>) -> Result<(), acanthus_io::IoError> {
        let clock = self.clone();
        let timer = timer::dynamic(handle, self.next_delay(), move || {
            clock.tick();
            Some(clock.next_delay())
        })?;
        *self.0.timer.borrow_mut() = Some(timer);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    struct Fixture {
        clock: Clock,
        at: Rc<Cell<NaiveDateTime>>,
        format: Signal<String>,
        language: Signal<String>,
    }

    fn fixture(format: &str) -> Fixture {
        let at = Rc::new(Cell::new(
            NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_milli_opt(15, 4, 9, 250).unwrap(),
        ));
        let source = at.clone();
        let (format, language) = (Signal::new(format.to_owned()), Signal::new("en".to_owned()));
        let clock =
            Clock::new(move || source.get(), format.clone(), language.clone(), Signal::new(DEFAULT_FULL_DATE.into()));
        Fixture { clock, at, format, language }
    }

    #[test]
    fn simple_clock_formats_the_current_time() {
        let clock = Clock::simple("HH:mm");
        let shown = clock.time().get_untracked();
        assert_eq!(shown.len(), 5);
        assert_eq!(shown.as_bytes()[2], b':');
        assert!(!clock.has_seconds().get_untracked());
    }

    #[test]
    fn default_format_fields() {
        let f = fixture("HH:mm:ss");
        let c = &f.clock;
        assert_eq!(c.time().get_untracked(), "15:04:09");
        assert_eq!(c.time_long().get_untracked(), "15:04:09");
        assert_eq!(c.hour().get_untracked(), "15");
        assert_eq!(c.minute().get_untracked(), "04");
        assert_eq!(c.second().get_untracked(), "09");
        assert_eq!(c.am_pm().get_untracked(), "");
        assert_eq!(c.time_short().get_untracked(), "15:04");
        assert_eq!(c.full_date().get_untracked(), "Thursday, October 22");
        assert_eq!(c.short_date().get_untracked(), "22 Oct");
        assert_eq!(c.date_badge().get_untracked(), "22 OCT");
        assert_eq!(c.day().get_untracked(), "22");
        assert_eq!(c.day_name_short().get_untracked(), "Thu");
        assert_eq!(c.month().get_untracked(), "October");
        assert_eq!(c.year().get_untracked(), "2026");
        assert!(c.has_seconds().get_untracked());
        assert!(!c.is_12_hour().get_untracked());
    }

    #[test]
    fn twelve_hour_format_reacts_to_config_changes() {
        let f = fixture("HH:mm:ss");
        f.format.set("h:mm AP".into());
        let c = &f.clock;
        assert_eq!(c.time().get_untracked(), "3:04 PM");
        assert_eq!(c.hour().get_untracked(), "15");
        assert_eq!(c.am_pm().get_untracked(), "PM");
        assert_eq!(c.time_short().get_untracked(), "3:04 PM");
        assert_eq!(c.time_long().get_untracked(), "15:04:09");
        assert_eq!(c.second().get_untracked(), "");
        assert!(c.is_12_hour().get_untracked());
        assert!(!c.has_seconds().get_untracked());
    }

    #[test]
    fn language_change_relocalises_dates() {
        let f = fixture("HH:mm");
        f.language.set("de".into());
        assert_eq!(f.clock.day_name().get_untracked(), "Donnerstag");
        assert_eq!(f.clock.format("MMMM"), locale_month("de"));
    }

    fn locale_month(lang: &str) -> String {
        let at = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(0, 0, 0).unwrap();
        format_qt(&at, "MMMM", locale_for(lang))
    }

    #[test]
    fn tick_updates_only_what_changed() {
        let f = fixture("HH:mm:ss");
        let runs = Rc::new(Cell::new(0));
        let (r, minute) = (runs.clone(), f.clock.minute());
        let _e = Effect::new(move || {
            minute.get();
            r.set(r.get() + 1);
        });
        f.at.set(f.at.get() + chrono::Duration::seconds(1));
        f.clock.tick();
        assert_eq!(f.clock.second().get_untracked(), "10");
        assert_eq!(runs.get(), 1);
        f.at.set(f.at.get() + chrono::Duration::seconds(60));
        f.clock.tick();
        assert_eq!(f.clock.minute().get_untracked(), "05");
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn midnight_rollover_updates_the_date() {
        let f = fixture("HH:mm:ss");
        f.at.set(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(23, 59, 59).unwrap());
        f.clock.tick();
        f.at.set(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap().and_hms_opt(0, 0, 0).unwrap());
        f.clock.tick();
        assert_eq!(f.clock.day().get_untracked(), "23");
        assert_eq!(f.clock.day_name().get_untracked(), "Friday");
    }

    #[test]
    fn next_delay_aligns_to_wall_clock_boundaries() {
        let f = fixture("HH:mm:ss");
        assert_eq!(f.clock.next_delay(), Duration::from_millis(750));
        f.format.set("HH:mm".into());
        assert_eq!(f.clock.next_delay(), Duration::from_millis(50 * 1000 + 750));
        f.at.set(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_milli_opt(15, 4, 59, 999).unwrap());
        assert_eq!(f.clock.next_delay(), Duration::from_millis(1));
    }

    #[test]
    fn format_parsing_matches_the_original_rules() {
        let f = TimeFormat::parse("hh:mm:ss a");
        assert_eq!((f.hour, f.minute, f.second, f.am_pm), ("hh", "mm", "ss", "AP"));
        assert!(f.is_12_hour && f.has_seconds);
        let f = TimeFormat::parse("H:m");
        assert_eq!((f.hour, f.minute, f.second, f.am_pm), ("H", "m", "", ""));
        assert!(!f.is_12_hour && !f.has_seconds);
        assert_eq!(TimeFormat::parse("").hour, "HH");
    }
}
