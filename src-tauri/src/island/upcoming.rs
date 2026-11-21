//! What's next, beside the notch: a meeting about to start, counting down with Join when it
//! has a call link, and reminders past due with Done. Nothing is read here: the heads-ups'
//! reads (nudges.rs) hand over what they found, so with heads-ups off, during a Focus or for
//! an app switched off in Settings → Apps, nothing is read and nothing shows. Unlike a
//! heads-up, this isn't said once: it shows while it holds, and never opens the island.
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::apps::{Event, Reminder};
use crate::nudges;
use crate::surface::cards::island::Upcoming;
use crate::surface::{Card, CardBody, Offer, Pill, PillState, Source, Urgency};

pub const MEETING: &str = "upcoming.meeting";
pub const REMINDERS: &str = "upcoming.reminders";
/// A meeting shows this long before it starts, and for a moment after.
const LEAD_MINUTES: i64 = 15;
const AFTER_MINUTES: i64 = 5;
/// Blocks this long ("Work", "Out of office") are not meetings.
const LONGEST_MEETING_HOURS: i64 = 6;
/// Reads older than this no longer count: heads-ups were switched off, or stopped reading.
const EVENTS_FRESH: Duration = Duration::from_secs(30 * 60);
const REMINDERS_FRESH: Duration = Duration::from_secs(45 * 60);
/// How often what shows is worked out again, so the countdown moves.
pub const TICK: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Seen {
    events: Vec<Event>,
    events_at: Option<Instant>,
    reminders: Vec<Reminder>,
    reminders_at: Option<Instant>,
    /// Reminders marked done from the island, until Reminders says so too.
    done: HashSet<String>,
}

static SEEN: LazyLock<Mutex<Seen>> = LazyLock::new(Default::default);

fn seen() -> std::sync::MutexGuard<'static, Seen> {
    SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The events the heads-ups just read, today's (and tomorrow's near midnight).
pub fn events(events: &[Event]) {
    let mut seen = seen();
    seen.events = events.to_vec();
    seen.events_at = Some(Instant::now());
    drop(seen);
    super::refresh();
}

/// The open reminders the heads-ups just read.
pub fn reminders(reminders: &[Reminder]) {
    let mut seen = seen();
    let open: HashSet<&str> = reminders.iter().filter_map(|reminder| reminder.id.as_deref()).collect();
    // Done here and gone there: forget it.
    let done: HashSet<String> = seen.done.iter().filter(|id| open.contains(id.as_str())).cloned().collect();
    seen.done = done;
    seen.reminders = reminders.to_vec();
    seen.reminders_at = Some(Instant::now());
    drop(seen);
    super::refresh();
}

/// A reminder was marked done from the island.
pub fn done(id: &str) {
    seen().done.insert(id.to_string());
    super::refresh();
}

/// The reminder the island's card would mark done, by its id, with the title it was read with.
pub fn reminder(id: &str) -> Option<(String, String)> {
    seen().reminders.iter().find(|reminder| reminder.id.as_deref() == Some(id)).map(|reminder| (id.to_string(), reminder.title.clone()))
}

/// What shows now: pills beside the notch and their cards.
pub fn shown(now: DateTime<Local>) -> (Vec<Pill>, Vec<Offer>) {
    let seen = seen();
    let events: &[Event] = if seen.events_at.is_some_and(|at| at.elapsed() < EVENTS_FRESH) { &seen.events } else { &[] };
    let reminders: Vec<&Reminder> = if seen.reminders_at.is_some_and(|at| at.elapsed() < REMINDERS_FRESH) {
        seen.reminders.iter().filter(|reminder| !reminder.id.as_ref().is_some_and(|id| seen.done.contains(id))).collect()
    } else {
        Vec::new()
    };
    let mut pills = Vec::new();
    let mut offers = Vec::new();
    for (pill, card) in [meeting(now, events), past_due(now, &reminders)].into_iter().flatten() {
        offers.push(Offer { card, urgency: Urgency::Info, at_ms: 0, ttl_ms: None });
        pills.push(pill);
    }
    (pills, offers)
}

/// The next timed meeting starting within the lead, or just begun: its start time beside
/// the notch, with a ring running down to it.
fn meeting(now: DateTime<Local>, events: &[Event]) -> Option<(Pill, Card)> {
    let (event, start) = events
        .iter()
        .filter(|event| !event.all_day && !event.title.trim().is_empty())
        .filter_map(|event| Some((event, nudges::event_start(event)?)))
        .filter(|(event, start)| {
            let lead = (*start - now).num_seconds();
            lead <= LEAD_MINUTES * 60
                && lead >= -AFTER_MINUTES * 60
                && nudges::event_end(event).is_none_or(|end| end - *start < chrono::Duration::hours(LONGEST_MEETING_HOURS))
        })
        .min_by_key(|(_, start)| *start)?;
    let left = (start - now).num_seconds().max(0) as f64;
    let title = event.title.trim().to_string();
    let location = event.location.trim();
    let pill = Pill {
        id: MEETING.into(),
        source: Source::Island,
        label: title.clone(),
        tint: "#FF453A".into(),
        state: PillState::Working,
        badge: Some(start.format("%H:%M").to_string()),
        progress: Some((left / (LEAD_MINUTES * 60) as f64).clamp(0.0, 1.0)),
        card: None,
    };
    let card = Card {
        id: MEETING.into(),
        source: Source::Island,
        body: CardBody::Upcoming(Upcoming {
            item: "meeting".into(),
            title,
            starts: Some(start.timestamp()),
            due: None,
            detail: (!location.is_empty()).then(|| location.to_string()),
            link: call_link(location),
            id: None,
            more: 0,
        }),
    };
    Some((pill, card))
}

/// Reminders past due, as the heads-ups count them: the first beside the notch, with Done.
fn past_due(now: DateTime<Local>, reminders: &[&Reminder]) -> Option<(Pill, Card)> {
    let due: Vec<&&Reminder> = reminders.iter().filter(|reminder| nudges::past_due(now, reminder)).collect();
    let first = due.first()?;
    let title = first.title.trim().to_string();
    let pill = Pill {
        id: REMINDERS.into(),
        source: Source::Island,
        label: title.clone(),
        tint: "#FF9F0A".into(),
        state: PillState::Working,
        badge: (due.len() > 1).then(|| due.len().to_string()),
        progress: None,
        card: None,
    };
    let card = Card {
        id: REMINDERS.into(),
        source: Source::Island,
        body: CardBody::Upcoming(Upcoming {
            item: "reminders".into(),
            title,
            starts: None,
            due: Some(first.due.trim().to_string()),
            detail: (!first.list.trim().is_empty()).then(|| first.list.trim().to_string()),
            link: None,
            id: first.id.clone(),
            more: (due.len() - 1) as u32,
        }),
    };
    Some((pill, card))
}

/// A call link in an event's location, where Zoom, Meet and Teams put theirs: https only.
pub fn call_link(location: &str) -> Option<String> {
    location
        .split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '"' | ',' | ';'))
        .filter(|word| word.starts_with("https://"))
        .find_map(|word| reqwest::Url::parse(word).ok().filter(|url| url.scheme() == "https" && url.host_str().is_some()))
        .map(|url| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(stamp: &str) -> DateTime<Local> {
        Local.from_local_datetime(&chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M").unwrap()).unwrap()
    }

    fn event(title: &str, start: &str, end: &str, location: &str) -> Event {
        serde_json::from_value(serde_json::json!({"id": title, "calendar": "Work", "start": start, "end": end, "allDay": false, "title": title, "location": location})).unwrap()
    }

    fn reminder(id: &str, title: &str, due: &str) -> Reminder {
        serde_json::from_value(serde_json::json!({"id": id, "title": title, "due": due, "list": "Home"})).unwrap()
    }

    #[test]
    fn a_meeting_counts_down_beside_the_notch_and_offers_its_call() {
        let events = [
            event("Standup", "2026-11-21 10:00", "2026-11-21 10:15", "https://zoom.us/j/123?pwd=x"),
            event("Lunch", "2026-11-21 12:00", "2026-11-21 13:00", "Cafe"),
            event("Work", "2026-11-21 09:00", "2026-11-21 17:00", ""),
        ];
        let (pill, card) = meeting(at("2026-11-21 09:52"), &events).unwrap();
        assert_eq!((pill.label.as_str(), pill.badge.as_deref(), pill.state), ("Standup", Some("10:00"), PillState::Working));
        assert!((pill.progress.unwrap() - 8.0 / 15.0).abs() < 0.01);
        let CardBody::Upcoming(upcoming) = card.body else { panic!() };
        assert_eq!(upcoming.link.as_deref(), Some("https://zoom.us/j/123?pwd=x"));
        // Too early, just begun, and too late.
        assert!(meeting(at("2026-11-21 09:40"), &events).is_none());
        assert!(meeting(at("2026-11-21 10:03"), &events).unwrap().0.progress == Some(0.0));
        assert!(meeting(at("2026-11-21 10:06"), &events).is_none());
        // An all-day block is no meeting.
        assert!(meeting(at("2026-11-21 08:50"), &events).is_none());
    }

    #[test]
    fn reminders_past_due_show_the_first_with_how_many_more() {
        let open = [reminder("r1", "Call the bank", "2026-11-21 09:00"), reminder("r2", "Buy milk", "2026-11-21 08:00"), reminder("r3", "Later", "2026-11-21 18:00")];
        let refs: Vec<&Reminder> = open.iter().collect();
        let (pill, card) = past_due(at("2026-11-21 10:00"), &refs).unwrap();
        assert_eq!((pill.label.as_str(), pill.badge.as_deref()), ("Call the bank", Some("2")));
        let CardBody::Upcoming(upcoming) = card.body else { panic!() };
        assert_eq!((upcoming.id.as_deref(), upcoming.more, upcoming.due.as_deref()), (Some("r1"), 1, Some("2026-11-21 09:00")));
        // Reminders' own alert comes first: not yet half an hour late.
        assert!(past_due(at("2026-11-21 08:20"), &refs).is_none());
    }

    #[test]
    fn only_an_https_link_with_a_host_is_a_call() {
        assert_eq!(call_link("Room 4 <https://meet.google.com/abc-defg-hij>").as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        assert_eq!(call_link("http://zoom.us/j/1"), None);
        assert_eq!(call_link("https:// nothing"), None);
        assert_eq!(call_link("Cafe on 5th"), None);
    }
}
