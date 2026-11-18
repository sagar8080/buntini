//! The details of an everyday request read from the user's own words: what a reminder,
//! event or note is called and when, what a text says, which app, which music control.
//!
//! A small model wrote "remind me to call the plumber monday morning" as the title and
//! dropped the day. What is read here is bound before any model call; anything it can't
//! read plainly is left for the model and the approval sheet. It never guesses a person or
//! an address.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde_json::{json, Map, Value};

/// A day and time the request named, and the words that said it.
#[derive(Debug, Clone, PartialEq)]
pub struct When {
    pub date: Option<NaiveDate>,
    pub time: Option<(u32, u32)>,
    pub all_day: bool,
    /// Indexes of the words that said when, so they leave the title.
    used: Vec<usize>,
}

/// Lowercase words with their place in the text, keeping "5:30pm" and "1:1" whole.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|raw| raw.trim_matches(|c: char| !(c.is_alphanumeric() || c == ':' || c == '-' || c == '\'' || c == '’')).trim_matches('-').to_lowercase().replace('’', "'"))
        .collect()
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" | "weds" => Weekday::Wed,
        "thursday" | "thu" | "thur" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

fn month(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    if word.len() < 3 {
        return None;
    }
    let position = MONTHS.iter().position(|name| word.starts_with(name))?;
    // "march" and "mar", but not "market"; "may" is only ever the month here.
    let full = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    (word == MONTHS[position] || word == full[position] || (word == "sept" && position == 8)).then_some(position as u32 + 1)
}

/// "1st", "15th", "3rd", "20".
fn day_of_month(word: &str) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_alphabetic());
    let suffix = &word[digits.len()..];
    if digits.is_empty() || !["", "st", "nd", "rd", "th"].contains(&suffix) {
        return None;
    }
    digits.parse().ok().filter(|day| (1..=31).contains(day))
}

/// The next date with this day of the month, today included.
fn next_day_of_month(day: u32, today: NaiveDate) -> Option<NaiveDate> {
    (0..13).find_map(|ahead| {
        let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?.checked_add_months(chrono::Months::new(ahead))?;
        NaiveDate::from_ymd_opt(first.year(), first.month(), day).filter(|date| *date >= today)
    })
}

/// "5pm", "5 pm", "17:00", "9:30am", "noon". A bare hour like "5" only counts after "at".
fn clock(words: &[String], index: usize, after_at: bool) -> Option<((u32, u32), usize)> {
    let word = words.get(index)?.as_str();
    match word {
        "noon" | "midday" => return Some(((12, 0), 1)),
        "midnight" => return Some(((0, 0), 1)),
        _ => {}
    }
    let (digits, mut half, mut used) = match word.strip_suffix("pm").or_else(|| word.strip_suffix("am")) {
        Some(digits) => (digits, Some(word.ends_with("pm")), 1),
        None => (word, None, 1),
    };
    if half.is_none() {
        if let Some(next) = words.get(index + 1).filter(|next| ["pm", "am", "p.m", "a.m"].contains(&next.as_str())) {
            half = Some(next.starts_with('p'));
            used = 2;
        }
    }
    let (hour, minute) = match digits.split_once(':') {
        Some((hour, minute)) if minute.len() == 2 => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        Some(_) => return None,
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if digits.is_empty() || minute > 59 {
        return None;
    }
    let hour = match half {
        Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        // "at 17:00", "at 9:30"; a bare "at 5" is in the day's waking hours: 5pm, 9am.
        None if digits.contains(':') && hour <= 23 => hour,
        None if after_at && (1..=12).contains(&hour) => if hour <= 7 { hour + 12 } else { hour },
        None => return None,
    };
    Some(((hour, minute), used))
}

/// When a part of the day is, when the request gives no hour.
fn part_of_day(word: &str) -> Option<(u32, u32)> {
    Some(match word {
        "morning" => (9, 0),
        "afternoon" => (14, 0),
        "evening" => (18, 0),
        "tonight" | "night" => (20, 0),
        _ => return None,
    })
}

/// The day and time a request names, relative to `today`. "Next Thursday" is Thursday of
/// next week; a weekday alone is the next one after today.
pub(crate) fn when(text: &str, today: NaiveDate) -> When {
    let words = words(text);
    let mut found = When { date: None, time: None, all_day: false, used: Vec::new() };
    let mut index = 0;
    while index < words.len() {
        let word = words[index].as_str();
        let next = words.get(index + 1).map(String::as_str);
        let take = |found: &mut When, count: usize| found.used.extend(index..index + count);
        match word {
            "today" if found.date.is_none() => { found.date = Some(today); take(&mut found, 1); }
            "tonight" if found.date.is_none() => {
                found.date = Some(today);
                found.time = found.time.or(Some((20, 0)));
                take(&mut found, 1);
            }
            "tomorrow" | "tmrw" | "tmr" | "tomorow" if found.date.is_none() => { found.date = today.succ_opt(); take(&mut found, 1); }
            "this" if next.and_then(part_of_day).is_some() && found.time.is_none() => {
                found.date = found.date.or(Some(today));
                found.time = next.and_then(part_of_day);
                take(&mut found, 2);
                index += 1;
            }
            "all" if next == Some("day") => { found.all_day = true; take(&mut found, 2); index += 1; }
            _ if found.date.is_none() && NaiveDate::parse_from_str(word, "%Y-%m-%d").is_ok() => {
                found.date = NaiveDate::parse_from_str(word, "%Y-%m-%d").ok();
                take(&mut found, 1);
            }
            "next" if next.and_then(weekday).is_some() && found.date.is_none() => {
                let wanted = next.and_then(weekday).unwrap();
                let monday = today + Duration::days(7 - i64::from(today.weekday().num_days_from_monday()));
                found.date = Some(monday + Duration::days(i64::from(wanted.num_days_from_monday())));
                take(&mut found, 2);
                index += 1;
            }
            _ if weekday(word).is_some() && found.date.is_none() => {
                let wanted = weekday(word).unwrap();
                found.date = (1..=7).map(|ahead| today + Duration::days(ahead)).find(|date| date.weekday() == wanted);
                take(&mut found, 1);
            }
            _ if month(word).is_some() && found.date.is_none() && next.and_then(day_of_month).is_some() => {
                let (m, d) = (month(word).unwrap(), next.and_then(day_of_month).unwrap());
                found.date = (0..2).find_map(|y| NaiveDate::from_ymd_opt(today.year() + y, m, d).filter(|date| *date >= today));
                take(&mut found, 2);
                index += 1;
            }
            _ if day_of_month(word).is_some() && found.date.is_none() && next.and_then(month).is_some() => {
                let (d, m) = (day_of_month(word).unwrap(), next.and_then(month).unwrap());
                found.date = (0..2).find_map(|y| NaiveDate::from_ymd_opt(today.year() + y, m, d).filter(|date| *date >= today));
                take(&mut found, 2);
                index += 1;
            }
            // "on the 1st", "the 20th": a day of the month needs its ordinal or "the".
            "the" if found.date.is_none() && next.is_some_and(|n| day_of_month(n).is_some() && n.chars().any(char::is_alphabetic)) => {
                found.date = next.and_then(day_of_month).and_then(|day| next_day_of_month(day, today));
                take(&mut found, 2);
                index += 1;
            }
            _ if part_of_day(word).is_some() && found.time.is_none() && index > 0 && found.used.contains(&(index - 1)) => {
                found.time = part_of_day(word);
                take(&mut found, 1);
            }
            _ if found.time.is_none() => {
                let after_at = index > 0 && words[index - 1] == "at";
                if let Some((time, used)) = clock(&words, index, after_at) {
                    found.time = Some(time);
                    take(&mut found, used);
                    index += used - 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    // "on", "at", "by", "for" that only led into when.
    let leading: Vec<usize> = found.used.iter().filter_map(|&at| at.checked_sub(1)).filter(|before| {
        !found.used.contains(before) && ["on", "at", "by", "for", "from", "until", "due"].contains(&words[*before].as_str())
    }).collect();
    found.used.extend(leading);
    found.used.sort_unstable();
    found.used.dedup();
    found
}

impl When {
    /// A time with no day is the next time it comes round: today, or tomorrow once past.
    fn on_next(mut self, now: chrono::NaiveDateTime) -> Self {
        if let (None, Some((hour, minute))) = (self.date, self.time) {
            let today = now.date();
            let passed = chrono::NaiveTime::from_hms_opt(hour, minute, 0).is_some_and(|time| time <= now.time());
            self.date = if passed { today.succ_opt() } else { Some(today) };
        }
        self
    }

    /// "YYYY-MM-DD HH:MM", with a day's default hour when only a day was named.
    pub fn stamp(&self, default_hour: u32) -> Option<String> {
        let date = self.date?;
        let (hour, minute) = self.time.unwrap_or((default_hour, 0));
        Some(format!("{} {hour:02}:{minute:02}", date.format("%Y-%m-%d")))
    }
}

/// The text without the words that said when.
fn without_when(text: &str, when: &When) -> String {
    text.split_whitespace().enumerate().filter(|(index, _)| !when.used.contains(index)).map(|(_, word)| word).collect::<Vec<_>>().join(" ")
}

/// Removes the first opening that matches, ignoring case: "remind me to ".
fn strip_opening<'a>(text: &'a str, openings: &[&str]) -> &'a str {
    let lower = text.to_lowercase();
    openings.iter().find(|opening| lower.starts_with(*opening)).map_or(text, |opening| text[opening.len()..].trim_start())
}

/// Removes the first ending that matches, ignoring case: " to my reminders".
fn strip_ending<'a>(text: &'a str, endings: &[&str]) -> &'a str {
    let lower = text.to_lowercase();
    endings.iter().find(|ending| lower.ends_with(*ending)).map_or(text, |ending| text[..text.len() - ending.len()].trim_end())
}

/// Polite openings that aren't part of what to do.
fn plain(request: &str) -> &str {
    let mut text = request.trim().trim_end_matches(['.', '!']);
    loop {
        let next = strip_opening(text, &["please ", "can you ", "could you ", "would you ", "will you ", "hey buntini, ", "hey buntini ", "buntini, ", "ok ", "okay "]);
        if next == text {
            return text;
        }
        text = next;
    }
}

/// Trims words that only joined the title to what was removed, and capitalises it.
fn titled(text: &str) -> Option<String> {
    const LOOSE: &[&str] = &["on", "at", "by", "for", "to", "and", "the", "a", "an", "in", "this", "next", "please", "-", ":", ","];
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while words.last().is_some_and(|word| LOOSE.contains(&word.to_lowercase().trim_end_matches(',')) || word.ends_with(':')) {
        words.pop();
    }
    while words.first().is_some_and(|word| [":", "-", ","].contains(word) || (words.len() > 1 && ["the", "a", "an"].contains(&word.to_lowercase().as_str()))) {
        words.remove(0);
    }
    let joined = words.join(" ");
    let joined = joined.trim_matches(|c: char| c == ',' || c == ':' || c == '-' || c.is_whitespace());
    let mut chars = joined.chars();
    let first = chars.next()?;
    let title: String = first.to_uppercase().chain(chars).collect();
    (title.chars().count() <= 200).then_some(title)
}

const REMINDER_OPENINGS: &[&str] = &[
    "remind me to ", "remind me about ", "remind me ", "don't let me forget to ", "dont let me forget to ",
    "don't let me forget ", "add a reminder to ", "add a reminder for ", "add a reminder about ", "add a reminder: ",
    "add a reminder ", "set a reminder to ", "set a reminder for ", "set a reminder about ", "set a reminder: ",
    "set a reminder ", "create a reminder to ", "create a reminder for ", "create a reminder: ", "create a reminder ",
    "make a reminder to ", "new reminder: ", "new reminder ", "reminder to ", "reminder for ", "reminder: ",
    "make a todo to ", "make a to do to ", "add a todo to ", "add a todo: ",
    // "Put a reminder on my list to call the bank": the list is where, the rest is what.
    "put a reminder on my list to ", "put a reminder on my to do list to ", "put a reminder on my to-do list to ",
    "add a reminder on my list to ", "stick a reminder on my list to ", "stick a reminder on my to do list to ",
    "put a reminder to ", "stick a reminder to ", "add ", "put ",
];
const LIST_ENDINGS: &[&str] = &[
    " to my reminders", " to reminders", " to my reminder list", " to my to do list", " to my to-do list",
    " to my todo list", " to my todos", " to my list", " on my list", " on my to do list", " on my to-do list",
    " on my reminders", " in my reminders",
];

fn reminder(request: &str, now: chrono::NaiveDateTime) -> Map<String, Value> {
    let text = plain(request);
    let when = when(text, now.date()).on_next(now);
    let mut fields = Map::new();
    let rest = without_when(text, &when);
    let rest = strip_ending(strip_opening(&rest, REMINDER_OPENINGS), LIST_ENDINGS);
    // "Remind me about this": what "this" is comes from the screen, not from here.
    let pointer = ["this", "that", "it", "these", "those", "them"].contains(&rest.trim().to_lowercase().as_str());
    if let Some(title) = titled(rest).filter(|_| !pointer) {
        fields.insert("title".into(), title.into());
    }
    if let Some(due) = when.stamp(9) {
        fields.insert("due".into(), due.into());
    }
    fields
}

const EVENT_OPENINGS: &[&str] = &[
    "add a calendar event: ", "add a calendar event called ", "add a calendar event for ", "add a calendar event ",
    "add an event called ", "add an event: ", "add an event for ", "add an event ", "create an event called ",
    "create an event: ", "create an event for ", "create an event ", "new event: ", "new event ", "calendar: ",
    "schedule a meeting with ", "schedule an appointment for ", "schedule an appointment with ", "schedule a ",
    "schedule an ", "schedule ", "book a ", "book an ", "book ", "put a ", "put an ", "put ", "add a ", "add an ",
    "add ", "set up a ", "set up an ", "pencil in ", "block out ", "plan a ", "plan an ",
];
const CALENDAR_ENDINGS: &[&str] = &[
    " on my calendar", " on the calendar", " in my calendar", " in the calendar", " to my calendar",
    " to the calendar", " in my diary", " on my diary", " to my diary", " on calendar", " in calendar",
];

fn event(request: &str, now: chrono::NaiveDateTime) -> Map<String, Value> {
    let text = plain(request);
    let when = when(text, now.date()).on_next(now);
    let mut fields = Map::new();
    let rest = without_when(text, &when);
    // "put lunch with Ana on the calendar for Wednesday": the calendar words can sit
    // before when, so they go after when has left.
    let rest = strip_ending(strip_opening(&rest, EVENT_OPENINGS), CALENDAR_ENDINGS);
    let rest = rest.replace(" on my calendar ", " ").replace(" on the calendar ", " ").replace(" in my calendar ", " ");
    let rest = strip_opening(&rest, &["a meeting with ", "an appointment with "]);
    if let Some(title) = titled(rest) {
        fields.insert("title".into(), title.into());
    }
    if when.all_day {
        if let Some(date) = when.date {
            fields.insert("start".into(), format!("{} 00:00", date.format("%Y-%m-%d")).into());
            fields.insert("minutes".into(), json!(1440));
        }
    } else if when.time.is_some() {
        if let Some(start) = when.stamp(0) {
            fields.insert("start".into(), start.into());
        }
    }
    fields
}

const NOTE_OPENINGS: &[&str] = &[
    "make a note called ", "make a note titled ", "make a note named ", "create a note called ", "create a note titled ",
    "create a note named ", "new note called ", "new note titled ", "write a note called ", "save a note called ",
    "start a note called ", "start a note for ", "take a note called ",
];
const BODY_OPENINGS: &[&str] = &[
    "jot down a note: ", "jot down a note ", "jot this down: ", "jot down that ", "jot down ", "make a note: ",
    "make a note that ", "make a note of ", "make a note ", "take a note: ", "take a note that ", "take a note ",
    "write a note: ", "write a note about ", "write a note ", "write down that ", "write down ", "new note: ",
    "new note ", "create a note: ", "create a note ", "save a note: ", "add a note: ", "put this in my notes: ",
    "notes: ", "note: ", "note that ", "note ",
];

fn note(request: &str) -> Map<String, Value> {
    let text = plain(request);
    let mut fields = Map::new();
    let named = strip_opening(text, NOTE_OPENINGS);
    let (title, body) = if named.len() != text.len() {
        match named.split_once(':') {
            Some((title, body)) if !body.trim().is_empty() => (titled(title), body.trim().to_string()),
            _ => (titled(named), named.trim().to_string()),
        }
    } else {
        let body = strip_opening(text, BODY_OPENINGS);
        if body.len() == text.len() {
            return fields;
        }
        // A title from the note's own first words: "wifi password at the flat".
        let head = [" is ", " are ", ": ", " - ", ", ", ". "].iter().filter_map(|cut| body.find(cut)).min().map_or(body, |end| &body[..end]);
        let head: Vec<&str> = head.split_whitespace().take(6).collect();
        (titled(&head.join(" ")), body.trim().to_string())
    };
    if let Some(title) = title {
        fields.insert("title".into(), title.into());
    }
    if !body.is_empty() && body.chars().count() <= 4000 {
        let mut chars = body.chars();
        let body: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
        fields.insert("body".into(), body.into());
    }
    fields
}

/// What a text says: "text Sam I'm running late", "send Ana a message saying the 14th
/// works", "message dad: landed". Only the words to send; who it goes to is the
/// recipient lookup's.
fn message(request: &str) -> Map<String, Value> {
    let text = plain(request);
    let lower = text.to_lowercase();
    let mut fields = Map::new();
    let said = ["saying ", "that says ", "that ", ": "]
        .iter()
        .filter_map(|cue| lower.find(cue).map(|at| (at, cue.len())))
        .min()
        .map(|(at, len)| text[at + len..].trim().to_string())
        .or_else(|| {
            // "text Sam I'm running late": the words after the person's name.
            let mut words = text.split_whitespace();
            let verb = words.next()?.to_lowercase();
            if !["text", "message", "imessage", "sms"].contains(&verb.as_str()) {
                return None;
            }
            // The person's name is as many words as the recipient lookup reads ("Sam Lee").
            let named = crate::source_fields::named_recipient(text).map_or(1, |name| name.split_whitespace().count());
            let first = words.next()?;
            let skip = if first.eq_ignore_ascii_case("my") { 1 } else { named - 1 };
            for _ in 0..skip {
                words.next()?;
            }
            let rest: Vec<&str> = words.collect();
            (!rest.is_empty()).then(|| rest.join(" "))
        });
    if let Some(said) = said.filter(|said| !said.is_empty() && said.chars().count() <= 2000) {
        let mut chars = said.chars();
        let said: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
        fields.insert("text".into(), said.into());
    }
    fields
}

/// "quit Spotify", "open the Music app".
fn app(request: &str, key: &str) -> Map<String, Value> {
    let text = plain(request);
    let rest = strip_opening(text, &["quit ", "close ", "exit ", "shut down ", "kill ", "open up ", "open ", "launch ", "start ", "fire up ", "bring up "]);
    let rest = strip_ending(strip_opening(rest, &["the "]), &[" for me", " app", " application"]);
    let mut fields = Map::new();
    if rest.len() != text.len() && !rest.is_empty() && rest.split_whitespace().count() <= 4 && rest.chars().count() <= 100 {
        fields.insert(key.into(), titled(rest).unwrap_or_default().into());
    }
    fields
}

/// "skip this song", "pause", "play my chill playlist on Spotify".
fn music(request: &str) -> Map<String, Value> {
    let text = plain(request);
    let lower: Vec<String> = words(text);
    let has = |list: &[&str]| lower.iter().any(|word| list.contains(&word.as_str()));
    let mut fields = Map::new();
    let action = if has(&["skip", "next"]) {
        "next"
    } else if has(&["previous", "back", "last"]) && has(&["song", "track", "go"]) {
        "previous"
    } else if has(&["pause", "stop"]) {
        "pause"
    } else if has(&["play", "resume", "unpause", "continue"]) {
        "play"
    } else {
        return fields;
    };
    fields.insert("action".into(), action.into());
    if has(&["spotify"]) {
        fields.insert("app".into(), "spotify".into());
    }
    fields
}

/// The details of a request for `tool`, as far as its own words say them plainly. `today`
/// is the day the request was made; on that day, the clock decides whether "at 7" is today.
pub fn fields(tool: &str, request: &str, today: NaiveDate) -> Map<String, Value> {
    let local = chrono::Local::now().naive_local();
    let now = if local.date() == today { local } else { today.and_hms_opt(0, 0, 0).unwrap_or(local) };
    fields_at(tool, request, now)
}

fn fields_at(tool: &str, request: &str, now: chrono::NaiveDateTime) -> Map<String, Value> {
    match tool {
        "add_reminder" => reminder(request, now),
        "add_calendar_event" => event(request, now),
        "create_note" => note(request),
        "send_message" => message(request),
        "quit_app" => app(request, "app"),
        "open_app" => app(request, "name"),
        "control_music" => music(request),
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saturday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()
    }

    fn read(tool: &str, request: &str) -> Value {
        Value::Object(fields_at(tool, request, saturday().and_hms_opt(12, 0, 0).unwrap()))
    }

    #[test]
    fn reminders_keep_the_task_and_its_time_apart() {
        assert_eq!(read("add_reminder", "remind me to pick up the dry cleaning tomorrow at 5pm"), json!({"title":"Pick up the dry cleaning","due":"2026-11-08 17:00"}));
        assert_eq!(read("add_reminder", "add a reminder to pay rent on the 1st"), json!({"title":"Pay rent","due":"2026-12-01 09:00"}));
        assert_eq!(read("add_reminder", "put a reminder on my list to call the bank"), json!({"title":"Call the bank"}));
        assert_eq!(read("add_reminder", "remind me to call the plumber monday morning"), json!({"title":"Call the plumber","due":"2026-11-09 09:00"}));
        assert_eq!(read("add_reminder", "remind me to water the plants this evening"), json!({"title":"Water the plants","due":"2026-11-07 18:00"}));
        assert_eq!(read("add_reminder", "add eggs and bread to my reminders"), json!({"title":"Eggs and bread"}));
        assert_eq!(read("add_reminder", "remind me to renew my passport by friday"), json!({"title":"Renew my passport","due":"2026-11-13 09:00"}));
        assert_eq!(read("add_reminder", "remind me to text Jo at 7"), json!({"title":"Text Jo","due":"2026-11-07 19:00"}));
        assert_eq!(read("add_reminder", "remind me to stretch at 9am"), json!({"title":"Stretch","due":"2026-11-08 09:00"}));
        assert_eq!(read("add_reminder", "remind me about this tomorrow at 9"), json!({"due":"2026-11-08 09:00"}));
        assert_eq!(read("add_reminder", "Add a reminder to call Mum on 2026-11-04 at 09:00."), json!({"title":"Call Mum","due":"2026-11-04 09:00"}));
    }

    #[test]
    fn events_get_a_title_and_a_start_only_when_a_time_is_given() {
        assert_eq!(read("add_calendar_event", "put lunch with Ana on the calendar for Wednesday at 1pm"), json!({"title":"Lunch with Ana","start":"2026-11-11 13:00"}));
        assert_eq!(read("add_calendar_event", "schedule a haircut tomorrow at 9:30am"), json!({"title":"Haircut","start":"2026-11-08 09:30"}));
        assert_eq!(read("add_calendar_event", "book a dinner with mum friday at 7pm"), json!({"title":"Dinner with mum","start":"2026-11-13 19:00"}));
        assert_eq!(read("add_calendar_event", "add a calendar event: team offsite next Thursday all day"), json!({"title":"Team offsite","start":"2026-11-12 00:00","minutes":1440}));
        assert_eq!(read("add_calendar_event", "add the school play to my calendar on the 20th"), json!({"title":"School play"}));
    }

    #[test]
    fn notes_keep_their_words() {
        assert_eq!(read("create_note", "make a note called Packing list: passport, charger, sunglasses"), json!({"title":"Packing list","body":"Passport, charger, sunglasses"}));
        assert_eq!(read("create_note", "note: wifi password at the flat is blue-harbor-42"), json!({"title":"Wifi password at the flat","body":"Wifi password at the flat is blue-harbor-42"}));
        assert_eq!(read("create_note", "jot down a note: ideas for sam's birthday - climbing gym pass, book, whisky"), json!({"title":"Ideas for sam's birthday","body":"Ideas for sam's birthday - climbing gym pass, book, whisky"}));
        assert_eq!(read("create_note", "what's in my notes"), json!({}));
    }

    #[test]
    fn texts_say_only_what_the_user_said() {
        assert_eq!(read("send_message", "text Sam I'm running 10 mins late"), json!({"text":"I'm running 10 mins late"}));
        assert_eq!(read("send_message", "text mum happy birthday"), json!({"text":"Happy birthday"}));
        assert_eq!(read("send_message", "send Ana a message saying the 14th works for me"), json!({"text":"The 14th works for me"}));
        assert_eq!(read("send_message", "message dad: landed"), json!({"text":"Landed"}));
        assert_eq!(read("send_message", "text my sister see you soon"), json!({"text":"See you soon"}));
        assert_eq!(read("send_message", "text Sam Lee on my way"), json!({"text":"On my way"}));
    }

    #[test]
    fn apps_and_music() {
        assert_eq!(read("quit_app", "quit Spotify"), json!({"app":"Spotify"}));
        assert_eq!(read("open_app", "open the Music app"), json!({"name":"Music"}));
        assert_eq!(read("control_music", "skip this song"), json!({"action":"next"}));
        assert_eq!(read("control_music", "pause the music"), json!({"action":"pause"}));
        assert_eq!(read("control_music", "go back to the previous song"), json!({"action":"previous"}));
    }

    #[test]
    fn days_and_times() {
        let at = |text: &str| when(text, saturday()).stamp(9);
        assert_eq!(at("next thursday"), Some("2026-11-12 09:00".into()));
        assert_eq!(at("thursday"), Some("2026-11-12 09:00".into()));
        assert_eq!(at("on 20 november at noon"), Some("2026-11-20 12:00".into()));
        assert_eq!(at("nov 3"), Some("2027-11-03 09:00".into()));
        assert_eq!(at("tonight"), Some("2026-11-07 20:00".into()));
        assert_eq!(at("call Mum on 2026-11-04 at 09:00"), Some("2026-11-04 09:00".into()));
        assert_eq!(at("move my 1:1 to 2pm"), None);
        assert_eq!(at("buy 2 apples"), None);
    }
}
