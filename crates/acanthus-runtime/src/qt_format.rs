use chrono::{Datelike, Locale, NaiveDateTime, Timelike};

pub fn locale_for(lang: &str) -> Locale {
    let name = match lang {
        "en" => "en_US",
        "ar" => "ar_SA",
        "az" => "az_AZ",
        "bg" => "bg_BG",
        "de" => "de_DE",
        "es" => "es_ES",
        "fa" => "fa_IR",
        "fr" => "fr_FR",
        "hi" => "hi_IN",
        "hy" => "hy_AM",
        "id" => "id_ID",
        "it" => "it_IT",
        "ja" => "ja_JP",
        "ko" => "ko_KR",
        "nl" => "nl_NL",
        "pl" => "pl_PL",
        "pt" => "pt_BR",
        "ru" => "ru_RU",
        "tr" => "tr_TR",
        "ua" | "uk" => "uk_UA",
        "vi" => "vi_VN",
        "zh" => "zh_CN",
        "zh_tw" => "zh_TW",
        other => other,
    };
    Locale::try_from(name).unwrap_or(Locale::en_US)
}

fn localized(dt: &NaiveDateTime, spec: &str, locale: Locale) -> String {
    dt.and_utc().format_localized(spec, locale).to_string()
}

fn run_length(chars: &[char], at: usize) -> usize {
    chars[at..].iter().take_while(|c| **c == chars[at]).count()
}

pub fn format_qt(dt: &NaiveDateTime, pattern: &str, locale: Locale) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let twelve_hour = pattern_has_meridiem(&chars);
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                i += 1;
                if chars.get(i) == Some(&'\'') {
                    out.push('\'');
                    i += 1;
                    continue;
                }
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            out.push('\'');
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            'd' => {
                let n = run_length(&chars, i);
                i += n.min(4);
                match n.min(4) {
                    1 => out.push_str(&dt.day().to_string()),
                    2 => out.push_str(&format!("{:02}", dt.day())),
                    3 => out.push_str(&localized(dt, "%a", locale)),
                    _ => out.push_str(&localized(dt, "%A", locale)),
                }
            }
            'M' => {
                let n = run_length(&chars, i);
                i += n.min(4);
                match n.min(4) {
                    1 => out.push_str(&dt.month().to_string()),
                    2 => out.push_str(&format!("{:02}", dt.month())),
                    3 => out.push_str(&localized(dt, "%b", locale)),
                    _ => out.push_str(&localized(dt, "%B", locale)),
                }
            }
            'y' => {
                let n = run_length(&chars, i);
                i += n;
                if n <= 2 {
                    out.push_str(&format!("{:02}", dt.year().rem_euclid(100)));
                } else {
                    out.push_str(&format!("{:04}", dt.year()));
                }
            }
            'h' | 'H' => {
                let n = run_length(&chars, i).min(2);
                i += n;
                let hour = if c == 'h' && twelve_hour { twelve(dt.hour()) } else { dt.hour() };
                if n == 2 {
                    out.push_str(&format!("{hour:02}"));
                } else {
                    out.push_str(&hour.to_string());
                }
            }
            'm' | 's' => {
                let n = run_length(&chars, i).min(2);
                i += n;
                let value = if c == 'm' { dt.minute() } else { dt.second() };
                if n == 2 {
                    out.push_str(&format!("{value:02}"));
                } else {
                    out.push_str(&value.to_string());
                }
            }
            'z' => {
                let n = run_length(&chars, i).min(3);
                i += n;
                let ms = dt.nanosecond() / 1_000_000;
                if n == 3 {
                    out.push_str(&format!("{ms:03}"));
                } else {
                    out.push_str(&ms.to_string());
                }
            }
            'A' => {
                i += if chars.get(i + 1) == Some(&'P') { 2 } else { 1 };
                out.push_str(&meridiem(dt, locale).to_uppercase());
            }
            'a' => {
                i += if chars.get(i + 1) == Some(&'p') { 2 } else { 1 };
                out.push_str(&meridiem(dt, locale).to_lowercase());
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

fn pattern_has_meridiem(chars: &[char]) -> bool {
    let mut quoted = false;
    for c in chars {
        match c {
            '\'' => quoted = !quoted,
            'A' | 'a' if !quoted => return true,
            _ => {}
        }
    }
    false
}

fn twelve(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        h => h,
    }
}

fn meridiem(dt: &NaiveDateTime, locale: Locale) -> String {
    let text = localized(dt, "%p", locale);
    if text.is_empty() { (if dt.hour() < 12 { "AM" } else { "PM" }).to_owned() } else { text }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_milli_opt(h, m, s, 45).unwrap()
    }

    fn en(dt: &NaiveDateTime, pattern: &str) -> String {
        format_qt(dt, pattern, Locale::en_US)
    }

    #[test]
    fn twenty_four_hour_patterns() {
        assert_eq!(en(&at(7, 5, 9), "HH:mm:ss"), "07:05:09");
        assert_eq!(en(&at(7, 5, 9), "H:m:s"), "7:5:9");
        assert_eq!(en(&at(0, 0, 0), "HH:mm"), "00:00");
    }

    #[test]
    fn twelve_hour_patterns_with_meridiem() {
        assert_eq!(en(&at(15, 4, 0), "h:mm AP"), "3:04 PM");
        assert_eq!(en(&at(15, 4, 0), "hh:mm a"), "03:04 pm");
        assert_eq!(en(&at(0, 30, 0), "h:mm AP"), "12:30 AM");
        assert_eq!(en(&at(12, 30, 0), "h:mm AP"), "12:30 PM");
        assert_eq!(en(&at(15, 4, 0), "h:mm"), "15:04");
    }

    #[test]
    fn date_patterns() {
        assert_eq!(en(&at(1, 1, 1), "dddd, MMMM dd"), "Thursday, October 22");
        assert_eq!(en(&at(1, 1, 1), "ddd d MMM yyyy"), "Thu 22 Oct 2026");
        assert_eq!(en(&at(1, 1, 1), "dd.MM.yy"), "22.10.26");
        assert_eq!(en(&at(1, 1, 1), "d/M/yyyy"), "22/10/2026");
    }

    #[test]
    fn quoted_literals_and_milliseconds() {
        assert_eq!(en(&at(15, 4, 0), "'at' HH 'o''clock'"), "at 15 o'clock");
        assert_eq!(en(&at(1, 1, 1), "ss.zzz"), "01.045");
    }

    #[test]
    fn localised_names_differ_by_language() {
        let dt = at(1, 1, 1);
        assert_eq!(format_qt(&dt, "dddd", locale_for("de")), "Donnerstag");
        assert_ne!(format_qt(&dt, "MMMM", locale_for("ua")), "October");
        assert_eq!(format_qt(&dt, "ddd", locale_for("zz")), "Thu");
    }

    #[test]
    fn every_bundled_language_has_a_locale() {
        for lang in [
            "en", "ar", "az", "bg", "de", "es", "fa", "fr", "hi", "hy", "id", "it", "ja", "ko", "nl", "pl", "pt", "ru",
            "tr", "ua", "vi", "zh", "zh_tw",
        ] {
            let text = format_qt(&at(1, 1, 1), "dddd MMMM", locale_for(lang));
            assert!(!text.trim().is_empty(), "{lang}");
            if lang != "en" {
                assert_ne!(text, "Thursday October", "{lang} fell back to English");
            }
        }
    }
}
