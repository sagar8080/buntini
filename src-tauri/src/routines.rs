//! Routines: requests Buntini runs on its own at a set time, such as "every weekday at 8,
//! brief me". The user approves each routine once, with its exact request, days and time.
//! Every run goes through the normal task controller without the window
//! (`controller::run_task_headless`), so it has the same contract, privacy rules and
//! receipts as a chat. Anything a run wants to change waits in Waiting for you; a routine
//! never approves its own proposals.
//!
//! The morning brief is the built-in routine. It still runs the first time you open
//! Buntini each day, as it always has, and its on/off state mirrors the old setting.
use crate::{
    actions::{ActionOutcome, ActionReceipt, Field},
    controller::AppState,
    store::Store,
    types::{now, Routine},
};
use chrono::{DateTime, Datelike, Local, TimeZone, Utc, Weekday};
use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};
use std::sync::atomic::Ordering;
use tauri::{Manager, State};

pub const TOOL: &str = "create_routine";
/// The built-in morning brief's id.
pub const MORNING_BRIEF: &str = "morning-brief";
/// A run missed while the Mac slept or Buntini was closed still happens when Buntini is
/// back within this long. Later than that it waits for its next time: a brief from
/// breakfast is no use at dinner.
const CATCH_UP_HOURS: i64 = 4;
const MAX_ROUTINES: usize = 20;
const MAX_TASK: usize = 500;

const DAY_NAMES: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

/// A routine asked for in chat: what to do, which days and at what time, all taken
/// from the user's own words.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub task: String,
    pub days: &'static str,
    pub time: String,
}
impl Request {
    pub fn arguments(&self) -> Value {
        json!({"task": self.task, "days": self.days, "time": self.time})
    }
}

/// Words of a request in lowercase, with the punctuation that ends or joins clauses
/// split off, and where each word starts in the original text.
fn tokens(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_whitespace() || matches!(ch, ',' | ';' | '?' | '!' | '—') {
            if let Some(begin) = start.take() {
                out.push((begin, clean(&text[begin..index])));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        out.push((begin, clean(&text[begin..])));
    }
    out.retain(|(_, word)| !word.is_empty());
    out
}
fn clean(word: &str) -> String {
    word.to_lowercase()
        .replace(['’', '‘'], "'")
        .trim_end_matches(['.', ':'])
        .to_string()
}

fn day_name(word: &str) -> Option<&'static str> {
    let word = word.strip_suffix('s').unwrap_or(word);
    DAY_NAMES
        .iter()
        .map(|(name, _)| *name)
        .find(|name| *name == word || name.starts_with(word) && word.len() >= 3 && word != "sun")
        .or((word == "sun").then_some("sunday"))
}
fn period(word: &str) -> Option<(u32, u32)> {
    match word {
        "morning" => Some((8, 0)),
        "afternoon" => Some((13, 0)),
        "evening" => Some((18, 0)),
        "night" => Some((21, 0)),
        _ => None,
    }
}

/// A clock time: "8", "8am", "8:30", "8.30 pm", "20:00", "noon". `next` is the word
/// after it, in case it is "am" or "pm". Returns the hour (24h, or None when unsaid
/// whether morning or evening), the minute and how many words were used.
fn clock(word: &str, next: Option<&str>) -> Option<(u32, u32, Option<bool>, usize)> {
    match word {
        "noon" | "midday" => return Some((12, 0, Some(true), 1)),
        "midnight" => return Some((0, 0, Some(false), 1)),
        _ => {}
    }
    let (digits, mut meridiem) = if let Some(rest) = word.strip_suffix("am").or_else(|| word.strip_suffix("a.m")) {
        (rest, Some(false))
    } else if let Some(rest) = word.strip_suffix("pm").or_else(|| word.strip_suffix("p.m")) {
        (rest, Some(true))
    } else {
        (word, None)
    };
    let (hour, minute) = match digits.split_once([':', '.']) {
        Some((hour, minute)) if minute.len() == 2 => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        Some(_) => return None,
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if hour > 23 || minute > 59 || digits.is_empty() {
        return None;
    }
    let mut used = 1;
    if meridiem.is_none() {
        match next {
            Some("am" | "a.m") => {
                meridiem = Some(false);
                used = 2;
            }
            Some("pm" | "p.m") => {
                meridiem = Some(true);
                used = 2;
            }
            _ => {}
        }
    }
    if meridiem.is_some() && !(1..=12).contains(&hour) {
        return None;
    }
    // "20:00" and "08:00" already say which.
    if meridiem.is_none() && (hour >= 13 || hour == 0 || digits.starts_with('0')) {
        return Some((hour, minute, Some(hour >= 12), used));
    }
    let hour = match meridiem {
        Some(true) if hour < 12 => hour + 12,
        Some(false) if hour == 12 => 0,
        _ => hour,
    };
    Some((hour, minute, meridiem.map(|_| hour >= 12), used))
}

/// The schedule starting at `words[0]`, when the words from there say one: the days,
/// the time as HH:MM, and how many words it took.
fn schedule(words: &[&str]) -> Option<(&'static str, String, usize)> {
    let mut index = 0;
    let mut days = None;
    let mut time_of_day = None;
    match words.first().copied()? {
        "every" | "each" => {
            index = 1;
            let mut word = *words.get(1)?;
            if word == "single" {
                index += 1;
                word = words.get(index)?;
            }
            days = Some(match word {
                "day" => "every_day",
                "weekday" | "workday" => "weekdays",
                "weekend" => "weekends",
                _ if period(word).is_some() => {
                    time_of_day = period(word);
                    "every_day"
                }
                _ => day_name(word).filter(|_| !word.ends_with('s'))?,
            });
            index += 1;
        }
        "daily" => {
            index = 1;
            days = Some("every_day");
        }
        "on" => {
            let word = *words.get(1)?;
            days = Some(match word {
                "weekdays" | "workdays" => "weekdays",
                "weekends" => "weekends",
                _ if word.ends_with('s') => day_name(word)?,
                _ => return None,
            });
            index = 2;
        }
        "weekdays" => {
            index = 1;
            days = Some("weekdays");
        }
        _ => {}
    }
    let days = days?;
    // "every Sunday evening", "every day in the morning".
    if time_of_day.is_none() {
        if let Some(found) = words.get(index).and_then(|w| period(w)) {
            time_of_day = Some(found);
            index += 1;
        }
    }
    let mut at = None;
    if words.get(index) == Some(&"at") {
        let (hour, minute, said, used) = clock(words.get(index + 1)?, words.get(index + 2).copied())?;
        at = Some((hour, minute, said));
        index += 1 + used;
        if words.get(index) == Some(&"in") && words.get(index + 1) == Some(&"the") {
            if let Some(found) = words.get(index + 2).and_then(|w| period(w)) {
                time_of_day = Some(found);
                index += 3;
            }
        } else if let Some(found) = words.get(index).and_then(|w| period(w)) {
            // "at 7 tonight" is rare; "at 8 every morning" isn't supported either way.
            time_of_day = Some(found);
            index += 1;
        }
    }
    let (hour, minute) = match (at, time_of_day) {
        (Some((hour, minute, Some(_))), _) => (hour, minute),
        // "at 6 in the evening", "every Sunday evening at 7".
        (Some((hour, minute, None)), Some((period_hour, _))) if period_hour >= 13 && hour < 12 => (hour + 12, minute),
        (Some((hour, minute, None)), Some(_)) => (hour, minute),
        // A bare "at 6" is far more often the evening than the small hours.
        (Some((hour, minute, None)), None) if (1..=6).contains(&hour) => (hour + 12, minute),
        (Some((hour, minute, None)), None) => (hour, minute),
        (None, Some(found)) => found,
        (None, None) => return None,
    };
    Some((days, format!("{hour:02}:{minute:02}"), index))
}

const POLITE: [&str; 6] = ["please ", "can you ", "could you ", "would you ", "will you ", "buntini "];
fn without_polite(mut text: &str) -> (&str, bool) {
    let mut asked = false;
    loop {
        let lower = text.to_lowercase();
        let Some(prefix) = POLITE.iter().find(|prefix| lower.starts_with(**prefix)) else {
            return (text, asked);
        };
        asked |= *prefix != "buntini " && *prefix != "please ";
        text = text[prefix.len()..].trim_start_matches([' ', ',']);
    }
}

/// "Every weekday at 8, brief me", "Every Sunday evening tell me what's on next week",
/// "Brief me every weekday at 8". The schedule must open or close the request and say
/// a time or a part of the day. Recurring reminders, timers, events and searches have
/// their own tools and are left to them.
pub fn parse_request(goal: &str) -> Option<Request> {
    let goal = goal.trim();
    if goal.len() > 2000 || goal.contains('\n') {
        return None;
    }
    let (body, asked) = without_polite(goal);
    let found = tokens(body);
    let words: Vec<&str> = found.iter().map(|(_, word)| word.as_str()).collect();
    let task = if let Some((days, time, used)) = schedule(&words) {
        let rest = found.get(used).map(|(start, _)| &body[*start..]).unwrap_or("");
        let rest = rest.trim_start_matches([',', ':', ';', '-', '—', ' ']);
        let rest = rest.strip_prefix("then ").unwrap_or(rest);
        (rest.trim().trim_end_matches('.').to_string(), days, time)
    } else {
        // The schedule closes the request: find where it starts.
        let (start, (days, time)) = (1..words.len()).find_map(|start| {
            let (days, time, used) = schedule(&words[start..])?;
            (start + used == words.len()).then_some((start, (days, time)))
        })?;
        // "What do I have every Monday at 9?" is a question, not a routine.
        if body.trim_end().ends_with('?') && !asked {
            return None;
        }
        let rest = &body[..found[start].0];
        (rest.trim().trim_end_matches([',', ';', '-', '—']).trim().to_string(), days, time)
    };
    let (text, days, time) = task;
    let (text, _) = without_polite(&text);
    let text = text.trim_end_matches(['?', '.', '!']).trim();
    if text.is_empty() || text.chars().count() > MAX_TASK {
        return None;
    }
    let (writes, _) = crate::completion::requested_actions(text);
    let lower = text.to_lowercase().replace('’', "'");
    // "Search for … every day" is a saved search, which says when results change.
    let search = lower.starts_with("search ");
    // "I go to the gym every Monday at 7" tells Buntini something; it asks for nothing.
    let statement = ["i", "i'm", "im", "we", "we're", "my", "our", "it", "it's", "there", "he", "she", "they", "you"]
        .contains(&lower.split_whitespace().next().unwrap_or_default());
    if search
        || statement
        || writes.iter().any(|tool| {
            ["add_reminder", "set_timer", "add_calendar_event", "create_saved_search", "create_price_watch"].contains(tool)
        })
    {
        return None;
    }
    let mut task: String = text.into();
    if let Some(first) = task.chars().next() {
        task.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    Some(Request { task, days, time })
}

fn normalize_days(text: &str) -> Option<&'static str> {
    let text = text.trim().to_lowercase().replace('_', " ");
    let text = text
        .strip_prefix("every ")
        .or_else(|| text.strip_prefix("on "))
        .unwrap_or(&text)
        .trim();
    match text {
        "day" | "daily" | "every day" | "days" => Some("every_day"),
        "weekday" | "weekdays" | "workdays" => Some("weekdays"),
        "weekend" | "weekends" => Some("weekends"),
        _ => day_name(text),
    }
}
fn normalize_time(text: &str) -> Option<String> {
    let text = text.trim().to_lowercase();
    let words: Vec<&str> = text.split_whitespace().collect();
    let (hour, minute, said, used) = clock(words.first()?, words.get(1).copied())?;
    if used != words.len() {
        return None;
    }
    let hour = if said.is_none() && (1..=6).contains(&hour) { hour + 12 } else { hour };
    Some(format!("{hour:02}:{minute:02}"))
}

/// Checks and normalizes create_routine's arguments, from the model or the sheet.
pub fn validate(args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    if let Some(extra) = args.keys().find(|key| !["task", "days", "time"].contains(&key.as_str())) {
        return Err(format!("{TOOL} does not accept {extra}."));
    }
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|task| !task.is_empty() && task.chars().count() <= MAX_TASK && !task.chars().any(char::is_control))
        .ok_or("Say what the routine should do, in up to 500 characters.")?;
    // A routine runs while you're away; "this" would mean whatever happens to be on screen.
    if crate::focus::mentions_this(task) {
        return Err("A routine runs on its own, so it can’t use what’s on your screen. Say what it should look at instead.".into());
    }
    if parse_request(task).is_some() {
        return Err("Give the routine one schedule.".into());
    }
    let days = args
        .get("days")
        .and_then(Value::as_str)
        .and_then(normalize_days)
        .ok_or("Choose every day, weekdays, weekends or one day of the week.")?;
    let time = args
        .get("time")
        .and_then(Value::as_str)
        .and_then(normalize_time)
        .ok_or("Give the routine a time, like 8:00 AM.")?;
    let mut out = Map::new();
    out.insert("task".into(), json!(task));
    out.insert("days".into(), json!(days));
    out.insert("time".into(), json!(time));
    Ok(out)
}

/// "every day", "weekdays", "Mondays".
pub fn days_label(days: &str) -> String {
    match days {
        "every_day" => "every day".into(),
        "weekdays" => "weekdays".into(),
        "weekends" => "weekends".into(),
        day => {
            let mut label: String = day.into();
            if let Some(first) = label.chars().next() {
                label.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
            }
            format!("{label}s")
        }
    }
}
/// "8:00 AM".
pub fn time_label(time: &str) -> String {
    chrono::NaiveTime::parse_from_str(time, "%H:%M")
        .map(|time| time.format("%-I:%M %p").to_string())
        .unwrap_or_else(|_| time.into())
}

pub fn describe(args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    (
        "Set up this routine?".into(),
        vec![
            Field { key: "task".into(), label: "What I’ll do".into(), value: text("task"), kind: "long" },
            Field { key: "days".into(), label: "Days".into(), value: days_label(&text("days")), kind: "text" },
            Field { key: "time".into(), label: "Time".into(), value: time_label(&text("time")), kind: "text" },
        ],
        format!("Each time, I’ll do this on my own and put the result in its own conversation, with a notification. Anything it would change, like a draft or a reminder, waits for your OK under Waiting for you. It runs while Buntini is open and your Mac is awake; a run missed while your Mac slept happens when it wakes, within {CATCH_UP_HOURS} hours. Pause or delete it in History → Routines."),
    )
}

fn matches_day(days: &str, day: Weekday) -> bool {
    match days {
        "every_day" => true,
        "weekdays" => !matches!(day, Weekday::Sat | Weekday::Sun),
        "weekends" => matches!(day, Weekday::Sat | Weekday::Sun),
        name => DAY_NAMES.iter().any(|(known, weekday)| *known == name && *weekday == day),
    }
}

/// The first time on one of `days` at `time` after `after`, in the Mac's time zone. A
/// time skipped by a daylight-saving change runs an hour later that day.
pub fn next_after<Tz: TimeZone>(days: &str, time: &str, after: DateTime<Tz>) -> Option<DateTime<Utc>> {
    let clock = chrono::NaiveTime::parse_from_str(time, "%H:%M").ok()?;
    let zone = after.timezone();
    let start = after.naive_local().date();
    (0..=8).find_map(|offset| {
        let date = start + chrono::Duration::days(offset);
        if !matches_day(days, date.weekday()) {
            return None;
        }
        let local = date.and_time(clock);
        let at = zone
            .from_local_datetime(&local)
            .earliest()
            .or_else(|| zone.from_local_datetime(&(local + chrono::Duration::hours(1))).earliest())?;
        (at > after).then(|| at.with_timezone(&Utc))
    })
}

/// "Monday, November 2 at 8:00 AM".
fn when(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|at| at.with_timezone(&Local).format("%A, %B %-d at %-I:%M %p").to_string())
        .unwrap_or_else(|_| at.into())
}

/// Creates the built-in morning brief on first launch, on or off as the old setting was,
/// so nothing changes for people who already had it.
pub fn initialize(store: &Store) -> Result<(), String> {
    store
        .db
        .execute_batch("CREATE TABLE IF NOT EXISTS routines (id TEXT PRIMARY KEY, data TEXT NOT NULL, next_run_at TEXT NOT NULL, enabled INTEGER NOT NULL, created_at TEXT NOT NULL);")
        .map_err(|e| e.to_string())?;
    if store.routine(MORNING_BRIEF)?.is_none() {
        store.save_routine(&Routine {
            id: MORNING_BRIEF.into(),
            task: "Morning brief".into(),
            days: "every_day".into(),
            time: "05:00".into(),
            enabled: store.settings()?.morning_brief,
            built_in: true,
            created_at: now(),
            next_run_at: String::new(),
            last_run_at: None,
            last_task_id: None,
            last_error: None,
        })?;
    }
    Ok(())
}

/// The morning brief setting changed: the built-in routine follows it.
pub fn follow_setting(store: &Store, morning_brief: bool) -> Result<(), String> {
    if let Some(mut routine) = store.routine(MORNING_BRIEF)? {
        if routine.enabled != morning_brief {
            routine.enabled = morning_brief;
            store.save_routine(&routine)?;
        }
    }
    Ok(())
}

/// Whether today's brief should show as the window opens: on, set up, not shown yet
/// today and after 5 in the morning. The same rule as before routines.
pub fn brief_due(store: &Store, at: DateTime<Local>) -> Result<bool, String> {
    let enabled = store.routine(MORNING_BRIEF)?.is_some_and(|routine| routine.enabled);
    Ok(enabled
        && store.onboarded()?
        && store.preference(&brief_key(at))?.is_none()
        && chrono::Timelike::hour(&at) >= 5)
}
pub(crate) fn brief_key(at: DateTime<Local>) -> String {
    format!("brief-{}", at.format("%Y-%m-%d"))
}
/// Today's brief was shown: it isn't due again until tomorrow.
pub fn record_brief(store: &Store, task_id: &str) -> Result<(), String> {
    store.set_preference(&brief_key(Local::now()), "1")?;
    if let Some(mut routine) = store.routine(MORNING_BRIEF)? {
        routine.last_run_at = Some(now());
        routine.last_task_id = Some(task_id.into());
        store.save_routine(&routine)?;
    }
    Ok(())
}

fn failure(message: String, outcome: ActionOutcome) -> ActionReceipt {
    ActionReceipt {
        outcome,
        tool: TOOL.into(),
        object_id: None,
        destination: Some("Buntini · Routines".into()),
        message: message.clone(),
        evidence: json!({}),
        error: Some(message),
    }
}
fn matches(routine: &Routine, args: &Map<String, Value>) -> bool {
    routine.task == args["task"].as_str().unwrap_or_default()
        && routine.days == args["days"].as_str().unwrap_or_default()
        && routine.time == args["time"].as_str().unwrap_or_default()
        && !routine.built_in
}

/// Saves an approved routine and reads it back. `intent_id` comes from the durable
/// journal, never from the model: replaying the same approval verifies the routine it
/// already made instead of making another, and a deleted routine is not brought back.
pub fn execute(store: &Store, intent_id: &str, raw: &Map<String, Value>) -> ActionReceipt {
    let args = match validate(&Value::Object(raw.clone())) {
        Ok(args) => args,
        Err(error) => return failure(error, ActionOutcome::Failed),
    };
    if intent_id.is_empty()
        || intent_id.len() > 200
        || !intent_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return failure("This routine request has no valid approval to go with it.".into(), ActionOutcome::Failed);
    }
    let id = format!("action-{intent_id}");
    let transaction = match store.db.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => return failure(error.to_string(), ActionOutcome::Failed),
    };
    // Take SQLite's write lock before checking the approval and the limit.
    if let Err(error) = store.db.execute("UPDATE routines SET enabled=enabled WHERE 0", []) {
        return failure(error.to_string(), ActionOutcome::Failed);
    }
    let result = (|| -> Result<Routine, String> {
        let serialized = serde_json::to_string(&args).map_err(|e| e.to_string())?;
        let previous: Option<(String, String)> = store
            .db
            .query_row("SELECT tool,args FROM monitor_intents WHERE id=?1", [intent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((tool, previous)) = &previous {
            if tool != TOOL || previous != &serialized {
                return Err("This approval was already used for something else. Ask again to set up the routine.".into());
            }
            if store.routine(&id)?.is_none() {
                return Err("This routine was set up and later deleted. Ask again if you want it back.".into());
            }
        }
        store
            .db
            .execute("INSERT OR IGNORE INTO monitor_intents(id,tool,args,monitor_id) VALUES(?1,?2,?3,?4)", rusqlite::params![intent_id, TOOL, serialized, id])
            .map_err(|e| e.to_string())?;
        if let Some(existing) = store.routine(&id)? {
            if !matches(&existing, &args) {
                return Err("This routine already exists with different details. It wasn’t changed.".into());
            }
        } else {
            if store.routines()?.iter().filter(|routine| !routine.built_in).count() >= MAX_ROUTINES {
                return Err(format!("You can have up to {MAX_ROUTINES} routines. Delete one in History → Routines first."));
            }
            let (days, time) = (args["days"].as_str().unwrap(), args["time"].as_str().unwrap());
            let next = next_after(days, time, Local::now()).ok_or("That time couldn’t be scheduled.")?;
            store.save_routine(&Routine {
                id: id.clone(),
                task: args["task"].as_str().unwrap().into(),
                days: days.into(),
                time: time.into(),
                enabled: true,
                built_in: false,
                created_at: now(),
                next_run_at: next.to_rfc3339(),
                last_run_at: None,
                last_task_id: None,
                last_error: None,
            })?;
            store.audit("routine.allowed", &format!("{id}: approved routine, {days} at {time}"))?;
        }
        let saved = store.routine(&id)?.ok_or("The routine couldn’t be read back.")?;
        if !matches(&saved, &args) || !saved.enabled {
            return Err("The saved routine didn’t match what you approved.".into());
        }
        Ok(saved)
    })();
    let saved = match result {
        Ok(saved) => saved,
        Err(error) => return failure(error, ActionOutcome::Failed),
    };
    if let Err(error) = transaction.commit() {
        return failure(format!("I couldn’t confirm the routine was saved: {error}. Check History → Routines before asking again."), ActionOutcome::Unknown);
    }
    ActionReceipt {
        outcome: ActionOutcome::Succeeded,
        tool: TOOL.into(),
        object_id: Some(id),
        destination: Some("Buntini · Routines".into()),
        message: format!(
            "Set up your routine “{}”: {} at {}. It first runs {}.",
            saved.task,
            days_label(&saved.days),
            time_label(&saved.time),
            when(&saved.next_run_at)
        ),
        evidence: json!({"verified": true, "record": saved, "networkRequestMade": false}),
        error: None,
    }
}

/// The thread a routine's results go to, so they never land in the conversation open
/// in the window.
fn thread(routine: &Routine) -> (String, String) {
    (
        format!("routine:{}", routine.id),
        format!("Routine: {}", routine.task.chars().take(85).collect::<String>()),
    )
}
fn assign_thread(store: &Store, task_id: &str, (id, title): &(String, String)) -> Result<(), String> {
    store
        .db
        .execute("INSERT INTO chat_threads(id,title,created_at,updated_at,legacy) VALUES(?1,?2,?3,?3,0) ON CONFLICT(id) DO NOTHING", rusqlite::params![id, title, now()])
        .map_err(|e| e.to_string())?;
    store
        .db
        .execute("INSERT INTO conversation_turns(task_id,conversation_id) VALUES(?1,?2) ON CONFLICT(task_id) DO NOTHING", rusqlite::params![task_id, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn monitor_key(id: &str) -> String {
    format!("routine-{id}")
}

/// Claims one scheduled run, in the same transaction as the run's task: the routine must
/// still be on and still due at `scheduled_for`, and its next time moves on before
/// anything runs, so a crash or a second check can never run it twice.
fn claim(store: &Store, id: &str, scheduled_for: &str, task_id: &str) -> Result<String, String> {
    let mut routine = store.routine(id)?.ok_or("This routine was deleted.")?;
    if !routine.enabled || routine.built_in || routine.next_run_at != scheduled_for {
        return Err("This routine is paused or no longer due.".into());
    }
    let at = Utc::now();
    routine.next_run_at = next_after(&routine.days, &routine.time, at.with_timezone(&Local))
        .ok_or("That time couldn’t be scheduled.")?
        .to_rfc3339();
    routine.last_run_at = Some(at.to_rfc3339());
    routine.last_task_id = Some(task_id.into());
    routine.last_error = None;
    store.save_routine(&routine)?;
    assign_thread(store, task_id, &thread(&routine))?;
    crate::delivery::monitor_started(store, &monitor_key(id), Some(scheduled_for), at)
}

/// What is due now: routines whose time has come, and those missed for longer than the
/// catch-up window, which move on to their next time without running.
fn due(store: &Store, at: DateTime<Utc>) -> Result<Vec<(String, String)>, String> {
    let mut ready = Vec::new();
    for mut routine in store.routines()? {
        let Ok(next) = DateTime::parse_from_rfc3339(&routine.next_run_at) else {
            continue;
        };
        if !routine.enabled || routine.built_in || next > at {
            continue;
        }
        if at - next.with_timezone(&Utc) > chrono::Duration::hours(CATCH_UP_HOURS) {
            routine.next_run_at = next_after(&routine.days, &routine.time, at.with_timezone(&Local))
                .ok_or("That time couldn’t be scheduled.")?
                .to_rfc3339();
            store.save_routine(&routine)?;
            store.audit("routine.missed", &format!("{}: not run, Buntini was closed or the Mac asleep at {}", routine.id, next.to_rfc3339()))?;
            continue;
        }
        ready.push((routine.id, routine.next_run_at));
    }
    Ok(ready)
}

/// Runs every routine that is due, one at a time, each only while nothing else runs.
pub async fn run_due(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let due = due(&state.store.lock().unwrap(), Utc::now()).unwrap_or_default();
    for (id, scheduled_for) in due {
        if !state.active.lock().unwrap().is_empty() {
            return;
        }
        if let Err(error) = run(app, &id, &scheduled_for).await {
            state.audit("routine.skipped", &format!("{id}: {error}"));
        }
    }
}

async fn run(app: &tauri::AppHandle, id: &str, scheduled_for: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let routine = state.store.lock().unwrap().routine(id)?.ok_or("This routine was deleted.")?;
    let (thread_id, _) = thread(&routine);
    let mut run_id = None;
    let task = crate::controller::run_task_headless(
        app,
        &routine.task,
        crate::controller::Headless {
            thread: &thread_id,
            events: tauri::ipc::Channel::new(|_| Ok(())),
            start: Box::new(|store, task| {
                run_id = Some(claim(store, id, scheduled_for, &task.id)?);
                Ok(())
            }),
        },
    )
    .await?;
    finish(app, &routine, run_id.as_deref(), &task).await
}

/// Journals the run and tells the user: a proposal asks for their OK; anything else
/// says the result is ready. The alert never shows the result itself, since it can
/// appear on the lock screen.
async fn finish(app: &tauri::AppHandle, routine: &Routine, run_id: Option<&str>, task: &crate::types::TaskRecord) -> Result<(), String> {
    use tauri::Emitter;
    let state = app.state::<AppState>();
    {
        let store = state.store.lock().unwrap();
        if let Some(run_id) = run_id {
            crate::delivery::monitor_finished(&store, run_id, &task.state, None)?;
        }
        if let Some(mut current) = store.routine(&routine.id)? {
            current.last_error = (task.state == "error").then(|| "The last run didn’t finish. Open its conversation to see why.".into());
            store.save_routine(&current)?;
        }
    }
    state.audit("routine.ran", &format!("{}: {}", routine.id, task.state));
    let delivery = if task.state == "cancelled" {
        Ok("in_app_only".to_string())
    } else {
        crate::notifications::deliver_routine(app, task).await
    };
    crate::delivery::monitor_delivery(
        &state.store.lock().unwrap(),
        &monitor_key(&routine.id),
        delivery.as_deref().map_err(String::as_str),
    )?;
    let _ = app.emit("background-updated", json!({"monitorKey": monitor_key(&routine.id), "taskId": task.id}));
    Ok(())
}

/// Stops a routine's run in progress, if there is one.
fn stop_running(state: &AppState, routine: &Routine) {
    if let Some(task_id) = &routine.last_task_id {
        if let Some(cancel) = state.active.lock().unwrap().get(task_id) {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Pause or resume. Resuming picks the next time from now, never a missed one; the
/// morning brief's switch is the same as its setting.
#[tauri::command]
pub async fn update_routine(id: String, enabled: bool, state: State<'_, AppState>) -> Result<Routine, String> {
    if id == MORNING_BRIEF {
        let mut settings = state.store.lock().unwrap().settings()?;
        settings.morning_brief = enabled;
        crate::controller::save_settings_in(&state, settings).await?;
        return state.store.lock().unwrap().routine(MORNING_BRIEF)?.ok_or("The morning brief is missing.".into());
    }
    let store = state.store.lock().unwrap();
    let mut routine = store.routine(&id)?.ok_or("This routine no longer exists.")?;
    if !enabled {
        stop_running(&state, &routine);
    }
    if enabled && !routine.enabled {
        routine.next_run_at = next_after(&routine.days, &routine.time, Local::now()).ok_or("That time couldn’t be scheduled.")?.to_rfc3339();
    }
    routine.enabled = enabled;
    store.save_routine(&routine)?;
    store.audit("routine.updated", &format!("{id}: enabled={enabled}"))?;
    Ok(routine)
}

#[tauri::command]
pub fn delete_routine(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let store = state.store.lock().unwrap();
    let routine = store.routine(&id)?.ok_or("This routine no longer exists.")?;
    if routine.built_in {
        return Err("The morning brief can be paused but not deleted.".into());
    }
    stop_running(&state, &routine);
    store.delete_routine(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn request(days: &'static str, time: &str, task: &str) -> Option<Request> {
        Some(Request { task: task.into(), days, time: time.into() })
    }

    #[test]
    fn routines_are_recognised_from_the_users_own_words() {
        for (prompt, expected) in [
            ("every weekday at 8, brief me", request("weekdays", "08:00", "Brief me")),
            ("Every Sunday evening tell me what's on next week", request("sunday", "18:00", "Tell me what's on next week")),
            ("Brief me every weekday at 8", request("weekdays", "08:00", "Brief me")),
            ("Can you brief me every weekday at 8:30am?", request("weekdays", "08:30", "Brief me")),
            ("Every day at 7 pm, what needs a reply?", request("every_day", "19:00", "What needs a reply")),
            ("every morning summarise my unread email", request("every_day", "08:00", "Summarise my unread email")),
            ("Every Friday at 5, draft an email to my team about the week", request("friday", "17:00", "Draft an email to my team about the week")),
            ("On weekends at 10:15 tell me the weather", request("weekends", "10:15", "Tell me the weather")),
            ("on Mondays at 20:00 tell me what's on this week", request("monday", "20:00", "Tell me what's on this week")),
            ("daily at noon: what's on my calendar", request("every_day", "12:00", "What's on my calendar")),
            ("Every day at 6 in the morning, play my focus playlist", request("every_day", "06:00", "Play my focus playlist")),
        ] {
            assert_eq!(parse_request(prompt), expected, "{prompt}");
        }
    }

    #[test]
    fn questions_one_offs_and_other_recurring_tools_are_not_routines() {
        for prompt in [
            "What do I have every Monday at 9?",
            "Brief me",
            "every weekday brief me",
            "Brief me on Monday at 8",
            "Every day at 8 remind me to take my pills",
            "Every day at 8 set a timer for 5 minutes",
            "Search for astronomy meetups every day at 9",
            "every weekday at 8",
            "Remind me every day at 8 to stretch",
            "What's the weather this morning?",
            "I go to the gym every Monday at 7",
            "Every Monday at 7 I go to the gym",
            "My standup is every weekday at 9:30",
        ] {
            assert_eq!(parse_request(prompt), None, "{prompt}");
        }
    }

    #[test]
    fn validation_normalizes_sheet_edits_and_refuses_the_screen() {
        let args = validate(&json!({"task":"Brief me","days":"Weekdays","time":"8:00 AM"})).unwrap();
        assert_eq!(Value::Object(args.clone()), json!({"task":"Brief me","days":"weekdays","time":"08:00"}));
        // What the sheet shows comes back unchanged after validation.
        let (_, fields, note) = describe(&args);
        let edited: Map<String, Value> = fields.iter().map(|field| (field.key.clone(), json!(field.value))).collect();
        assert_eq!(validate(&Value::Object(edited)).unwrap(), args);
        assert!(note.contains("Waiting for you"));
        assert_eq!(validate(&json!({"task":"x","days":"mondays","time":"20:30"})).unwrap()["days"], "monday");
        for invalid in [
            json!({"task":"Summarise this","days":"weekdays","time":"08:00"}),
            json!({"task":"Brief me","days":"fortnightly","time":"08:00"}),
            json!({"task":"Brief me","days":"weekdays","time":"25:00"}),
            json!({"task":"Brief me","days":"weekdays","time":"08:00","run_now":true}),
            json!({"task":"","days":"weekdays","time":"08:00"}),
            json!({"task":"Brief me every day at 8","days":"weekdays","time":"08:00"}),
        ] {
            assert!(validate(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn next_times_follow_days_and_daylight_saving() {
        let zone = FixedOffset::west_opt(4 * 3600).unwrap();
        // Friday 30 October 2026, 09:00.
        let friday = zone.with_ymd_and_hms(2026, 10, 30, 9, 0, 0).unwrap();
        let next = |days, time| next_after(days, time, friday).unwrap().with_timezone(&zone).format("%a %d %H:%M").to_string();
        assert_eq!(next("weekdays", "08:00"), "Mon 02 08:00");
        assert_eq!(next("weekdays", "10:00"), "Fri 30 10:00");
        assert_eq!(next("every_day", "09:00"), "Sat 31 09:00", "the same minute has passed");
        assert_eq!(next("weekends", "07:00"), "Sat 31 07:00");
        assert_eq!(next("friday", "08:59"), "Fri 06 08:59");
        assert_eq!(next("sunday", "18:00"), "Sun 01 18:00");
        assert_eq!(next_after("weekdays", "8:00 AM", friday), None, "only HH:MM is stored");
    }

    #[test]
    fn approved_routines_read_back_and_replays_never_duplicate_or_revive() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let args = validate(&json!({"task":"Brief me","days":"weekdays","time":"08:00"})).unwrap();
        let first = execute(&store, "intent", &args);
        assert_eq!(first.outcome, ActionOutcome::Succeeded, "{}", first.message);
        assert!(first.message.starts_with("Set up your routine “Brief me”: weekdays at 8:00 AM. It first runs "));
        let again = execute(&store, "intent", &args);
        assert_eq!(again.object_id, first.object_id);
        assert_eq!(store.routines().unwrap().len(), 2, "the brief and this one");
        let mut changed = args.clone();
        changed.insert("time".into(), json!("09:00"));
        assert_eq!(execute(&store, "intent", &changed).outcome, ActionOutcome::Failed);
        store.delete_routine(first.object_id.as_deref().unwrap()).unwrap();
        assert_eq!(execute(&store, "intent", &args).outcome, ActionOutcome::Failed);
        assert_eq!(store.routines().unwrap().len(), 1);
    }

    #[test]
    fn a_run_is_claimed_once_and_missed_runs_wait_for_their_next_time() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let args = validate(&json!({"task":"Brief me","days":"every_day","time":"08:00"})).unwrap();
        let id = execute(&store, "intent", &args).object_id.unwrap();
        let mut routine = store.routine(&id).unwrap().unwrap();
        let now = Utc::now();
        // An hour late: it still runs, once.
        routine.next_run_at = (now - chrono::Duration::hours(1)).to_rfc3339();
        store.save_routine(&routine).unwrap();
        let ready = due(&store, now).unwrap();
        assert_eq!(ready, vec![(id.clone(), routine.next_run_at.clone())]);
        claim(&store, &id, &routine.next_run_at, "run-1").unwrap();
        assert!(claim(&store, &id, &routine.next_run_at, "run-2").is_err(), "no second run of the same time");
        let after = store.routine(&id).unwrap().unwrap();
        assert!(DateTime::parse_from_rfc3339(&after.next_run_at).unwrap() > now);
        assert_eq!(after.last_task_id.as_deref(), Some("run-1"));
        assert!(due(&store, now).unwrap().is_empty());
        // Missed by a day: it moves on without running.
        let mut stale = after.clone();
        stale.next_run_at = (now - chrono::Duration::hours(30)).to_rfc3339();
        store.save_routine(&stale).unwrap();
        assert!(due(&store, now).unwrap().is_empty());
        assert!(DateTime::parse_from_rfc3339(&store.routine(&id).unwrap().unwrap().next_run_at).unwrap() > now);
        // Paused routines and the built-in brief never run on the clock.
        let mut paused = store.routine(&id).unwrap().unwrap();
        paused.enabled = false;
        paused.next_run_at = (now - chrono::Duration::minutes(5)).to_rfc3339();
        store.save_routine(&paused).unwrap();
        assert!(due(&store, now).unwrap().is_empty());
        assert!(claim(&store, &id, &paused.next_run_at, "run-3").is_err());
        assert!(claim(&store, MORNING_BRIEF, "", "run-4").is_err());
    }

    #[test]
    fn the_morning_brief_setting_becomes_the_built_in_routine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brief.sqlite");
        let store = Store::open(&path).unwrap();
        // People who had it on keep it on: the setting decides the new routine.
        store.db.execute("DELETE FROM routines", []).unwrap();
        let mut settings = store.settings().unwrap();
        settings.morning_brief = true;
        store.save_settings(&settings).unwrap();
        store.set_onboarded(true).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let brief = store.routine(MORNING_BRIEF).unwrap().unwrap();
        assert!(brief.enabled && brief.built_in);
        let morning = Local::now().date_naive().and_hms_opt(9, 0, 0).unwrap();
        let morning = Local.from_local_datetime(&morning).earliest().unwrap();
        let early = Local.from_local_datetime(&morning.date_naive().and_hms_opt(4, 0, 0).unwrap()).earliest().unwrap();
        assert!(brief_due(&store, morning).unwrap());
        assert!(!brief_due(&store, early).unwrap(), "not before 5 in the morning");
        // Turning it off in Settings pauses the routine, and the other way round.
        settings.morning_brief = false;
        store.save_settings(&settings).unwrap();
        assert!(!store.routine(MORNING_BRIEF).unwrap().unwrap().enabled);
        assert!(!brief_due(&store, morning).unwrap());
        settings.morning_brief = true;
        store.save_settings(&settings).unwrap();
        record_brief(&store, "brief-task").unwrap();
        assert!(!brief_due(&store, morning).unwrap(), "shown once a day");
        assert_eq!(store.routine(MORNING_BRIEF).unwrap().unwrap().last_task_id.as_deref(), Some("brief-task"));
        // New installs start with it off, as before.
        let fresh = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(!fresh.routine(MORNING_BRIEF).unwrap().unwrap().enabled);
    }

    /// A routine run with a scripted local model that always proposes `call`.
    async fn headless_run(prompt: &str, call: Value, apps: &[&str]) -> (AppState, crate::types::TaskRecord, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("worker.py");
        std::fs::write(
            &script,
            format!(
                r#"import json, sys
print(json.dumps({{'type':'ready'}}), flush=True)
for line in sys.stdin:
    req = json.loads(line)
    def emit(kind, **fields):
        print(json.dumps(dict(id=req['id'], type=kind, **fields)), flush=True)
    call = json.loads({call:?})
    emit('tool_call', name=call['name'], arguments=call['arguments'])
    emit('done', text='', finish_reason='tool_call')
"#,
                call = call.to_string()
            ),
        )
        .unwrap();
        let model = dir.path().join("model");
        std::fs::create_dir(&model).unwrap();
        std::fs::write(model.join("config.json"), "{}").unwrap();
        let mut state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().into()).unwrap();
        state.models = crate::model::ModelHub::new(script, std::path::PathBuf::new());
        let settings = crate::types::Settings {
            model_backend: "mlx".into(),
            python_path: "python3".into(),
            model_path: model.to_string_lossy().into_owned(),
            apps: apps.iter().map(|app| app.to_string()).collect(),
            ..crate::types::Settings::default()
        };
        state.store.lock().unwrap().save_settings(&settings).unwrap();
        let routine = Routine {
            id: "action-test".into(),
            task: prompt.into(),
            days: "every_day".into(),
            time: "08:00".into(),
            enabled: true,
            built_in: false,
            created_at: now(),
            next_run_at: now(),
            last_run_at: None,
            last_task_id: None,
            last_error: None,
        };
        let (thread_id, _) = thread(&routine);
        let task = crate::controller::run_headless_in(
            &state,
            prompt,
            crate::controller::Headless {
                thread: &thread_id,
                events: tauri::ipc::Channel::new(|_| Ok(())),
                start: Box::new(|store, task| assign_thread(store, &task.id, &thread(&routine))),
            },
        )
        .await
        .unwrap();
        state.models.shutdown().await;
        (state, task, thread_id, dir)
    }

    #[tokio::test]
    async fn a_routines_proposal_waits_for_you_in_its_own_thread() {
        let (state, task, thread_id, _dir) = headless_run(
            "Create a note titled Week plan with body Plan the week",
            json!({"name":"create_note","arguments":{"title":"Week plan","body":"Plan the week"}}),
            &["notes"],
        )
        .await;
        assert_eq!(task.state, "waiting_action", "{}", task.answer);
        let store = state.store.lock().unwrap();
        assert_eq!(store.task_conversation(&task.id).unwrap(), thread_id);
        let open = store.active_conversation_id().unwrap();
        assert!(store.conversation_page(&open, None, 100).unwrap().turns.is_empty(), "the window's conversation is left alone");
        let waiting = store.pending_approvals().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].task_id, task.id);
        assert_eq!(waiting[0].tool, "create_note");
        // Nothing was approved or run.
        let flow = store.workflow(&task.id).unwrap().unwrap();
        assert!(flow.steps.iter().all(|step| step.state == "waiting" && step.approved_at.is_none()));
        assert!(state.active.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_routine_never_changes_your_mac_directly() {
        // In a chat "pause the music" is done at once by a skill; in a routine it waits.
        let (state, task, _, _dir) = headless_run("Pause the music", json!({"name":"control_music","arguments":{"action":"pause"}}), &["music"]).await;
        assert_eq!(task.state, "waiting_action", "{}", task.answer);
        assert_eq!(state.store.lock().unwrap().pending_approvals().unwrap()[0].tool, "control_music");
    }

    #[test]
    fn a_routine_request_becomes_one_pinned_write() {
        let mut contract = crate::task_contract::TaskContract::from_request("Every weekday at 8, brief me");
        contract.refresh("Every weekday at 8, brief me", &[]);
        let outcome = contract.next_outcome().unwrap();
        assert_eq!(outcome.accepted_tools, vec![TOOL.to_string()]);
        assert_eq!(outcome.required_arguments, json!({"task":"Brief me","days":"weekdays","time":"08:00"}));
        assert!(contract.validate_proposal(TOOL, &json!({"task":"Brief me","days":"weekdays","time":"08:00"}), &[]).is_ok());
        // The model can't move the time or change the request.
        assert!(contract.validate_proposal(TOOL, &json!({"task":"Brief me","days":"weekdays","time":"09:00"}), &[]).is_err());
        assert!(contract.validate_proposal(TOOL, &json!({"task":"Email my boss","days":"weekdays","time":"08:00"}), &[]).is_err());
        assert_eq!(crate::completion::requested_actions("Every Friday at 5, draft an email to Ana"), (vec![TOOL], false));
    }
}
