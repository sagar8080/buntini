//! What a service looked like at its last poll, and the small pieces every parser shares:
//! reading fields, times as people read them, money and the activity graph.
use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc, Weekday};
use serde_json::Value;

use crate::agents::recap::month_short;
use crate::i18n::{t, tf};
use crate::surface::PillState;

/// Rows a card carries; the island shows the first few.
pub const MAX_ROWS: usize = 8;
/// Days in the activity graph: 53 weeks, starting on a Sunday, ending today.
pub const GRAPH_WEEKS: usize = 53;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// The card's title.
    pub title: String,
    /// What the pill shows beside the service's name.
    pub badge: Option<String>,
    pub rows: Vec<Row>,
    /// Daily counts, oldest first, starting on a Sunday; the last is today.
    pub graph: Option<Vec<u32>>,
    /// Something is still running, so look again sooner.
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    pub state: Option<PillState>,
    pub url: Option<String>,
    /// A CI failure or a review request, worth interrupting for when it is new: what it is,
    /// for the alert's title.
    pub alert: Option<String>,
}

impl Row {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self { id: id.into(), title: clip(&title.into(), 120), detail: None, state: None, url: None, alert: None }
    }

    pub fn detail(mut self, parts: impl IntoIterator<Item = Option<String>>) -> Self {
        let parts: Vec<String> = parts.into_iter().flatten().filter(|part| !part.trim().is_empty()).collect();
        self.detail = (!parts.is_empty()).then(|| clip(&parts.join(" · "), 160));
        self
    }

    pub fn state(mut self, state: Option<PillState>) -> Self {
        self.state = state;
        self
    }

    pub fn url(mut self, url: Option<String>) -> Self {
        self.url = url;
        self
    }

    pub fn alert(mut self, label: impl Into<String>, when: bool) -> Self {
        self.alert = when.then(|| label.into());
        self
    }
}

impl Snapshot {
    /// The pill's state: a review waits on the person, then failures, then work in progress.
    pub fn pill_state(&self) -> PillState {
        let has = |state| self.rows.iter().any(|row| row.state == Some(state));
        if self.rows.iter().any(|row| row.alert.is_some() && row.state == Some(PillState::Waiting)) {
            PillState::Waiting
        } else if has(PillState::Error) {
            PillState::Error
        } else if self.busy || has(PillState::Working) {
            PillState::Working
        } else {
            PillState::Idle
        }
    }
}

// Reading fields. Every service's JSON is untrusted: a missing or odd field is skipped,
// never a panic.

pub fn text(value: &Value, pointer: &str) -> Option<String> {
    match value.pointer(pointer)? {
        Value::String(text) => {
            let text = clip(text, 400);
            (!text.is_empty()).then_some(text)
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub fn number(value: &Value, pointer: &str) -> Option<i64> {
    match value.pointer(pointer)? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub fn flag(value: &Value, pointer: &str) -> bool {
    value.pointer(pointer).and_then(Value::as_bool).unwrap_or(false)
}

pub fn list<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    value.pointer(pointer).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// One line of visible text, at most `limit` characters.
pub fn clip(text: &str, limit: usize) -> String {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}…", text.chars().take(limit.saturating_sub(1)).collect::<String>())
    }
}

/// Reads the time formats these services use: RFC 3339, Postgres-style with a space, Unix
/// seconds or milliseconds.
pub fn time(value: &Value, pointer: &str) -> Option<DateTime<Utc>> {
    match value.pointer(pointer)? {
        Value::Number(number) => {
            let raw = number.as_i64()?;
            // Milliseconds once past the year 2286 in seconds.
            let seconds = if raw > 10_000_000_000 { raw / 1000 } else { raw };
            DateTime::from_timestamp(seconds, 0)
        }
        Value::String(text) => {
            let text = text.trim();
            DateTime::parse_from_rfc3339(text)
                .or_else(|_| DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%#z"))
                .or_else(|_| DateTime::parse_from_str(&format!("{text}:00"), "%Y-%m-%d %H:%M:%S%.f%:z"))
                .ok()
                .map(|time| time.with_timezone(&Utc))
        }
        _ => None,
    }
}

/// A time as people read it, in the Mac's time zone: `14:02` today, `Tue 14:02` within a
/// week either way, `12 Nov` further off.
pub fn when(at: DateTime<Utc>, now: DateTime<FixedOffset>) -> String {
    let local = at.with_timezone(now.offset());
    let days = (local.date_naive() - now.date_naive()).num_days();
    let clock = local.format("%H:%M").to_string();
    if days == 0 {
        clock
    } else if days.abs() < 7 {
        tf("services.when.weekday", &[("weekday", &weekday_short(local.weekday())), ("time", &clock)])
    } else if local.year() == now.year() {
        tf("services.when.date", &[("day", &local.day()), ("month", &month_short(local.month()))])
    } else {
        tf("services.when.dateYear", &[("day", &local.day()), ("month", &month_short(local.month())), ("year", &local.year())])
    }
}

/// A day of the week, short: `Tue`.
fn weekday_short(day: Weekday) -> String {
    match day {
        Weekday::Mon => t("common.weekdayShort.mon"),
        Weekday::Tue => t("common.weekdayShort.tue"),
        Weekday::Wed => t("common.weekdayShort.wed"),
        Weekday::Thu => t("common.weekdayShort.thu"),
        Weekday::Fri => t("common.weekdayShort.fri"),
        Weekday::Sat => t("common.weekdayShort.sat"),
        Weekday::Sun => t("common.weekdayShort.sun"),
    }
}

/// Midnight today in the Mac's time zone, as Unix seconds.
pub fn start_of_today(now: DateTime<FixedOffset>) -> i64 {
    now.date_naive().and_hms_opt(0, 0, 0).and_then(|midnight| midnight.and_local_timezone(*now.offset()).single()).map_or(now.timestamp(), |time| time.timestamp())
}

pub fn is_today(at: DateTime<Utc>, now: DateTime<FixedOffset>) -> bool {
    at.with_timezone(now.offset()).date_naive() == now.date_naive()
}

/// Counts per day laid out as the activity graph: `GRAPH_WEEKS` columns of seven days, from a
/// Sunday to today. Days without an entry count zero; days outside the range are dropped.
pub fn graph(days: &BTreeMap<NaiveDate, u32>, today: NaiveDate) -> Vec<u32> {
    let back = today.weekday().num_days_from_sunday() as i64 + 7 * (GRAPH_WEEKS as i64 - 1);
    let first = today - Duration::days(back);
    (0..=back).map(|offset| days.get(&(first + Duration::days(offset))).copied().unwrap_or(0)).collect()
}

/// Currencies whose amounts have no minor unit, and those with three decimals (Stripe's lists).
const ZERO_DECIMAL: [&str; 16] = ["bif", "clp", "djf", "gnf", "jpy", "kmf", "krw", "mga", "pyg", "rwf", "ugx", "vnd", "vuv", "xaf", "xof", "xpf"];
const THREE_DECIMAL: [&str; 5] = ["bhd", "jod", "kwd", "omr", "tnd"];

/// An amount in the currency's smallest unit, written the usual way: `€1,240.00`, `¥5,000`.
pub fn money(minor: i64, currency: &str) -> String {
    let code = currency.to_ascii_lowercase();
    let decimals = if ZERO_DECIMAL.contains(&code.as_str()) {
        0
    } else if THREE_DECIMAL.contains(&code.as_str()) {
        3
    } else {
        2
    };
    let scale = 10i64.pow(decimals);
    let (whole, fraction) = (minor.abs() / scale, minor.abs() % scale);
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let amount = if decimals == 0 { grouped } else { format!("{grouped}.{fraction:0width$}", width = decimals as usize) };
    let sign = if minor < 0 { "-" } else { "" };
    match code.as_str() {
        "usd" => format!("{sign}${amount}"),
        "eur" => format!("{sign}€{amount}"),
        "gbp" => format!("{sign}£{amount}"),
        "jpy" => format!("{sign}¥{amount}"),
        "inr" => format!("{sign}₹{amount}"),
        "cad" => format!("{sign}CA${amount}"),
        "aud" => format!("{sign}A${amount}"),
        _ => format!("{sign}{} {amount}", code.to_ascii_uppercase()),
    }
}

/// A short count for a pill: `7`, `1.2k`.
pub fn short(count: usize) -> String {
    if count < 1000 {
        count.to_string()
    } else {
        format!("{:.1}k", count as f64 / 1000.0)
    }
}

#[cfg(test)]
pub(super) fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_read_in_every_format_the_services_use() {
        let expected = DateTime::parse_from_rfc3339("2026-11-16T09:12:00Z").unwrap().with_timezone(&Utc);
        for raw in [
            json!("2026-11-16T09:12:00Z"),
            json!("2026-11-16T09:12:00.000Z"),
            json!("2026-11-16T10:12:00+01:00"),
            json!("2026-11-16 09:12:00.123+00"),
            json!("2026-11-16 09:12:00+00:00"),
            json!(1_794_820_320),
            json!(1_794_820_320_000i64),
        ] {
            let parsed = time(&json!({ "t": raw }), "/t").unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(parsed.timestamp() / 60, expected.timestamp() / 60, "{raw}");
        }
        assert_eq!(time(&json!({"t": "yesterday"}), "/t"), None);
        assert_eq!(time(&json!({"t": true}), "/t"), None);
    }

    #[test]
    fn times_read_as_people_say_them() {
        let now = at("2026-11-16T15:00:00+01:00");
        let utc = |text: &str| DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc);
        assert_eq!(when(utc("2026-11-16T13:02:00Z"), now), "14:02");
        assert_eq!(when(utc("2026-11-17T08:30:00Z"), now), "Tue 09:30");
        assert_eq!(when(utc("2026-11-12T08:30:00Z"), now), "Thu 09:30");
        assert_eq!(when(utc("2026-10-01T08:30:00Z"), now), "1 Oct");
        assert_eq!(when(utc("2025-10-01T08:30:00Z"), now), "1 Oct 2025");
        assert_eq!(start_of_today(now), at("2026-11-16T00:00:00+01:00").timestamp());
        assert!(is_today(utc("2026-11-15T23:30:00Z"), now));
        assert!(!is_today(utc("2026-11-15T22:30:00Z"), now));
    }

    #[test]
    fn money_follows_each_currencys_minor_unit() {
        assert_eq!(money(124_000, "eur"), "€1,240.00");
        assert_eq!(money(4_900, "USD"), "$49.00");
        assert_eq!(money(5_000, "jpy"), "¥5,000");
        assert_eq!(money(1_234_567, "kwd"), "KWD 1,234.567");
        assert_eq!(money(99, "chf"), "CHF 0.99");
        assert_eq!(money(-2_500, "gbp"), "-£25.00");
        assert_eq!(money(100_000_000, "usd"), "$1,000,000.00");
    }

    #[test]
    fn the_graph_runs_from_a_sunday_to_today() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(); // a Monday
        let mut days = BTreeMap::new();
        days.insert(today, 4);
        days.insert(today - Duration::days(1), 2);
        days.insert(today - Duration::days(400), 9);
        let cells = graph(&days, today);
        assert_eq!(cells.len(), 7 * (GRAPH_WEEKS - 1) + 2);
        assert_eq!(cells[cells.len() - 1], 4);
        assert_eq!(cells[cells.len() - 2], 2);
        assert_eq!(cells.iter().sum::<u32>(), 6);
        let first = today - Duration::days(cells.len() as i64 - 1);
        assert_eq!(first.weekday(), chrono::Weekday::Sun);
    }

    #[test]
    fn text_from_a_service_is_one_bounded_line() {
        assert_eq!(clip("  Fix\nthe\u{7} build  ", 40), "Fix the build");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        let row = Row::new("1", "x".repeat(500)).detail([Some("a".into()), None, Some(" ".into()), Some("b".into())]);
        assert_eq!(row.title.chars().count(), 120);
        assert_eq!(row.detail.as_deref(), Some("a · b"));
    }

    #[test]
    fn the_pill_shows_what_matters_most() {
        let row = |state| Row::new("r", "r").state(Some(state));
        let mut snapshot = Snapshot { rows: vec![row(PillState::Done)], ..Default::default() };
        assert_eq!(snapshot.pill_state(), PillState::Idle);
        snapshot.rows.push(row(PillState::Working));
        assert_eq!(snapshot.pill_state(), PillState::Working);
        snapshot.rows.push(row(PillState::Error));
        assert_eq!(snapshot.pill_state(), PillState::Error);
        snapshot.rows.push(row(PillState::Waiting).alert("Review requested", true));
        assert_eq!(snapshot.pill_state(), PillState::Waiting);
    }
}
