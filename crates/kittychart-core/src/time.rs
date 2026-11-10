//! Date handling for time scales, matching `chartjs-adapter-date-fns` in UTC.
//!
//! Chart.js delegates dates to an adapter. KittyChart mirrors the most common one (date-fns):
//! its `parseISO`, `startOf`/`endOf`, `add`, `diff` and `format` semantics, and its default
//! display formats. All math is in UTC so output is the same on every machine.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::Offset;
use jiff::{Timestamp, ToSpan};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Unit {
    pub const ALL: [Unit; 9] = [
        Unit::Millisecond,
        Unit::Second,
        Unit::Minute,
        Unit::Hour,
        Unit::Day,
        Unit::Week,
        Unit::Month,
        Unit::Quarter,
        Unit::Year,
    ];

    pub fn parse(s: &str) -> Option<Unit> {
        Some(match s {
            "millisecond" => Unit::Millisecond,
            "second" => Unit::Second,
            "minute" => Unit::Minute,
            "hour" => Unit::Hour,
            "day" => Unit::Day,
            "week" => Unit::Week,
            "month" => Unit::Month,
            "quarter" => Unit::Quarter,
            "year" => Unit::Year,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Unit::Millisecond => "millisecond",
            Unit::Second => "second",
            Unit::Minute => "minute",
            Unit::Hour => "hour",
            Unit::Day => "day",
            Unit::Week => "week",
            Unit::Month => "month",
            Unit::Quarter => "quarter",
            Unit::Year => "year",
        }
    }

    /// `INTERVALS[unit]`: (common, size in ms, steps).
    pub fn interval(self) -> (bool, f64, Option<f64>) {
        match self {
            Unit::Millisecond => (true, 1.0, Some(1000.0)),
            Unit::Second => (true, 1000.0, Some(60.0)),
            Unit::Minute => (true, 60000.0, Some(60.0)),
            Unit::Hour => (true, 3600000.0, Some(24.0)),
            Unit::Day => (true, 86400000.0, Some(30.0)),
            Unit::Week => (false, 604800000.0, Some(4.0)),
            Unit::Month => (true, 2.628e9, Some(12.0)),
            Unit::Quarter => (false, 7.884e9, Some(4.0)),
            Unit::Year => (true, 3.154e10, None),
        }
    }

    /// The adapter's default display format for this unit.
    pub fn default_format(self) -> &'static str {
        match self {
            Unit::Millisecond => "h:mm:ss.SSS aaaa",
            Unit::Second => "h:mm:ss aaaa",
            Unit::Minute => "h:mm aaaa",
            Unit::Hour => "ha",
            Unit::Day => "MMM d",
            Unit::Week => "PP",
            Unit::Month => "MMM yyyy",
            Unit::Quarter => "qqq - yyyy",
            Unit::Year => "yyyy",
        }
    }
}

pub const DATETIME_FORMAT: &str = "MMM d, yyyy, h:mm:ss aaaa";

fn to_dt(ms: f64) -> Option<DateTime> {
    let ts = Timestamp::from_millisecond(ms as i64).ok()?;
    Some(Offset::UTC.to_datetime(ts))
}

fn to_ms(dt: DateTime) -> f64 {
    Offset::UTC.to_timestamp(dt).map_or(f64::NAN, |t| t.as_millisecond() as f64)
}

/// The adapter's `parse`: numbers are timestamps (ms); strings use date-fns `parseISO`
/// (a missing offset means local time, which is UTC here).
pub fn parse(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => parse_iso(s.trim()),
        _ => None,
    }
}

fn parse_iso(s: &str) -> Option<f64> {
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Some(ts.as_millisecond() as f64);
    }
    if let Ok(dt) = s.parse::<DateTime>() {
        return Some(to_ms(dt));
    }
    if let Ok(d) = s.parse::<Date>() {
        return Some(to_ms(d.to_datetime(Time::midnight())));
    }
    // Reduced precision: "YYYY-MM" and "YYYY".
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y, m] if y.len() == 4 && m.len() == 2 => {
            let d = Date::new(y.parse().ok()?, m.parse().ok()?, 1).ok()?;
            Some(to_ms(d.to_datetime(Time::midnight())))
        }
        [y] if y.len() == 4 => {
            let d = Date::new(y.parse().ok()?, 1, 1).ok()?;
            Some(to_ms(d.to_datetime(Time::midnight())))
        }
        _ => None,
    }
}

/// `startOf(time, unit)` (weeks start on Sunday, date-fns' en-US default).
pub fn start_of(ms: f64, unit: Unit) -> f64 {
    let Some(dt) = to_dt(ms) else { return ms };
    let d = dt.date();
    let t = dt.time();
    let mk = |d: Date, h: i8, mi: i8, s: i8| {
        to_ms(d.to_datetime(Time::new(h, mi, s, 0).unwrap_or(Time::midnight())))
    };
    match unit {
        Unit::Millisecond => ms,
        Unit::Second => mk(d, t.hour(), t.minute(), t.second()),
        Unit::Minute => mk(d, t.hour(), t.minute(), 0),
        Unit::Hour => mk(d, t.hour(), 0, 0),
        Unit::Day => mk(d, 0, 0, 0),
        Unit::Week => {
            let back = i64::from(d.weekday().to_sunday_zero_offset());
            mk(d.checked_sub(back.days()).unwrap_or(d), 0, 0, 0)
        }
        Unit::Month => mk(d.first_of_month(), 0, 0, 0),
        Unit::Quarter => {
            let m = ((d.month() - 1) / 3) * 3 + 1;
            mk(Date::new(d.year(), m, 1).unwrap_or(d), 0, 0, 0)
        }
        Unit::Year => mk(Date::new(d.year(), 1, 1).unwrap_or(d), 0, 0, 0),
    }
}

/// `endOf(time, unit)`: the last millisecond of the unit.
pub fn end_of(ms: f64, unit: Unit) -> f64 {
    add(start_of(ms, unit), 1, unit) - 1.0
}

/// `add(time, amount, unit)`; month-based units clamp to the end of shorter months.
pub fn add(ms: f64, amount: i64, unit: Unit) -> f64 {
    let fixed = |size: f64| ms + amount as f64 * size;
    match unit {
        Unit::Millisecond => fixed(1.0),
        Unit::Second => fixed(1000.0),
        Unit::Minute => fixed(60000.0),
        Unit::Hour => fixed(3600000.0),
        Unit::Day => fixed(86400000.0),
        Unit::Week => fixed(604800000.0),
        Unit::Month | Unit::Quarter | Unit::Year => {
            let Some(dt) = to_dt(ms) else { return ms };
            let months = match unit {
                Unit::Month => amount,
                Unit::Quarter => amount * 3,
                _ => amount * 12,
            };
            dt.checked_add(months.months()).map_or(f64::NAN, to_ms)
        }
    }
}

/// `diff(max, min, unit)`: whole units (date-fns `differenceIn*`, truncated toward zero).
pub fn diff(max: f64, min: f64, unit: Unit) -> f64 {
    let fixed = |size: f64| ((max - min) / size).trunc();
    match unit {
        Unit::Millisecond => fixed(1.0),
        Unit::Second => fixed(1000.0),
        Unit::Minute => fixed(60000.0),
        Unit::Hour => fixed(3600000.0),
        Unit::Day => fixed(86400000.0),
        Unit::Week => fixed(604800000.0),
        Unit::Month | Unit::Quarter | Unit::Year => {
            let (Some(a), Some(b)) = (to_dt(max), to_dt(min)) else { return 0.0 };
            let (later, earlier, sign) = if a >= b { (a, b, 1.0) } else { (b, a, -1.0) };
            let mut months =
                i64::from(later.year() - earlier.year()) * 12 + i64::from(later.month() - earlier.month());
            // The last month only counts if it is complete.
            if earlier.checked_add(months.months()).is_ok_and(|x| x > later) {
                months -= 1;
            }
            let n = match unit {
                Unit::Month => months,
                Unit::Quarter => months / 3,
                _ => months / 12,
            };
            sign * n as f64
        }
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// date-fns `format` for the common tokens (en-US). `'quoted'` text is literal.
pub fn format(ms: f64, fmt: &str) -> String {
    let Some(dt) = to_dt(ms) else { return String::new() };
    let (d, t) = (dt.date(), dt.time());
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            // Quoted literal; '' is an escaped quote.
            let mut j = i + 1;
            if chars.get(j) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            while j < chars.len() && chars[j] != '\'' {
                out.push(chars[j]);
                j += 1;
            }
            i = j + 1;
            continue;
        }
        if !ch.is_ascii_alphabetic() {
            out.push(ch);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == ch {
            n += 1;
        }
        let h12 = {
            let h = t.hour() % 12;
            if h == 0 { 12 } else { h }
        };
        let pm = t.hour() >= 12;
        let pad = |v: i64, w: usize| format!("{v:0w$}");
        match (ch, n) {
            ('y', 2) => out.push_str(&pad(i64::from(d.year()) % 100, 2)),
            ('y', w) => out.push_str(&pad(i64::from(d.year()), w)),
            ('M', 1) => out.push_str(&d.month().to_string()),
            ('M', 2) => out.push_str(&pad(i64::from(d.month()), 2)),
            ('M', 3) => out.push_str(&MONTHS[(d.month() - 1) as usize][..3]),
            ('M', _) => out.push_str(MONTHS[(d.month() - 1) as usize]),
            ('q' | 'Q', 1) => out.push_str(&(((d.month() - 1) / 3) + 1).to_string()),
            ('q' | 'Q', 3) => out.push_str(&format!("Q{}", ((d.month() - 1) / 3) + 1)),
            ('q' | 'Q', _) => {
                let q = ((d.month() - 1) / 3) + 1;
                out.push_str(&format!("{q}{} quarter", ordinal_suffix(i64::from(q))));
            }
            ('d', 1) => out.push_str(&d.day().to_string()),
            ('d', _) => out.push_str(&pad(i64::from(d.day()), 2)),
            ('E', 1..=3) => out.push_str(&DAYS[d.weekday().to_sunday_zero_offset() as usize][..3]),
            ('E', _) => out.push_str(DAYS[d.weekday().to_sunday_zero_offset() as usize]),
            ('h', 1) => out.push_str(&h12.to_string()),
            ('h', _) => out.push_str(&pad(i64::from(h12), 2)),
            ('H', 1) => out.push_str(&t.hour().to_string()),
            ('H', _) => out.push_str(&pad(i64::from(t.hour()), 2)),
            ('m', 1) => out.push_str(&t.minute().to_string()),
            ('m', _) => out.push_str(&pad(i64::from(t.minute()), 2)),
            ('s', 1) => out.push_str(&t.second().to_string()),
            ('s', _) => out.push_str(&pad(i64::from(t.second()), 2)),
            ('S', w) => {
                let ms = i64::from(t.millisecond());
                let s = format!("{ms:03}");
                out.push_str(&s[..w.min(3)]);
                for _ in 3..w {
                    out.push('0');
                }
            }
            ('a', 1..=2) => out.push_str(if pm { "PM" } else { "AM" }),
            ('a', 3) => out.push_str(if pm { "pm" } else { "am" }),
            ('a', 4) => out.push_str(if pm { "p.m." } else { "a.m." }),
            ('a', _) => out.push_str(if pm { "p" } else { "a" }),
            // Localized dates (en-US): P = 01/02/2026, PP = Jan 2, 2026, PPP = January 2nd, 2026.
            ('P', 1) => out.push_str(&format!("{:02}/{:02}/{}", d.month(), d.day(), d.year())),
            ('P', 2) => {
                out.push_str(&format!("{} {}, {}", &MONTHS[(d.month() - 1) as usize][..3], d.day(), d.year()))
            }
            ('P', _) => out.push_str(&format!(
                "{} {}{}, {}",
                MONTHS[(d.month() - 1) as usize],
                d.day(),
                ordinal_suffix(i64::from(d.day())),
                d.year()
            )),
            _ => {
                for _ in 0..n {
                    out.push(ch);
                }
            }
        }
        i += n;
    }
    out
}

fn ordinal_suffix(n: i64) -> &'static str {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => "st",
        (2, r) if r != 12 => "nd",
        (3, r) if r != 13 => "rd",
        _ => "th",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> f64 {
        parse(&json!(s)).unwrap()
    }

    #[test]
    fn parses_iso_forms_as_utc() {
        assert_eq!(t("2026-01-05"), 1767571200000.0);
        assert_eq!(t("2026-01-05T10:30:00"), 1767571200000.0 + 10.5 * 3600000.0);
        assert_eq!(t("2026-01-05T10:30:00Z"), t("2026-01-05T10:30:00"));
        assert_eq!(t("2026-01"), t("2026-01-01"));
        assert_eq!(parse(&json!(1767571200000.0)), Some(1767571200000.0));
    }

    #[test]
    fn calendar_math() {
        assert_eq!(add(t("2026-01-31"), 1, Unit::Month), t("2026-02-28")); // clamps like date-fns
        assert_eq!(start_of(t("2026-01-07T13:45:00"), Unit::Week), t("2026-01-04")); // Sunday
        assert_eq!(start_of(t("2026-05-20"), Unit::Quarter), t("2026-04-01"));
        assert_eq!(diff(t("2026-03-15"), t("2026-01-20"), Unit::Month), 1.0);
        assert_eq!(end_of(t("2026-02-10"), Unit::Month), t("2026-03-01") - 1.0);
    }

    #[test]
    fn formats_like_date_fns() {
        let v = t("2026-01-05T15:07:09.042");
        assert_eq!(format(v, DATETIME_FORMAT), "Jan 5, 2026, 3:07:09 p.m.");
        assert_eq!(format(v, "ha"), "3PM");
        assert_eq!(format(v, "MMM d"), "Jan 5");
        assert_eq!(format(v, "PP"), "Jan 5, 2026");
        assert_eq!(format(v, "qqq - yyyy"), "Q1 - 2026");
        assert_eq!(format(v, "h:mm:ss.SSS aaaa"), "3:07:09.042 p.m.");
        assert_eq!(format(v, "yyyy-MM-dd 'at' HH:mm"), "2026-01-05 at 15:07");
    }
}
