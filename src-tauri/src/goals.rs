//! Goals: aims that take days, such as "help me prepare for the Lisbon trip by Friday",
//! "follow up with Ana until she replies" or "remind me to work on the tax return every
//! evening until it's done". The user approves each goal once (`create_goal`), with its
//! aim, what each check-in does, when check-ins happen, its last day and what ends it.
//!
//! Check-ins are routines under the hood (`routines.rs`, one per goal, id
//! `goal-…/check-in`), so they are scheduled, claimed once, caught up after sleep and give
//! way to the user exactly as routines do. A check-in runs its request headless through
//! the normal controller, or, for a reminder, says so without a model; its result goes in
//! the goal's own conversation and anything it would change waits under Waiting for you.
//!
//! A goal ends only on evidence: its last day passes, an email from the person it waits
//! on arrives (checked in Mail's headers, only while Mail is already open), or the user
//! marks it done. Nothing a model says ends a goal.
use crate::{
    actions::{ActionOutcome, ActionReceipt, Field},
    controller::AppState,
    routines,
    store::Store,
    types::{now, Routine, TaskRecord},
};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{Manager, State};

pub const TOOL: &str = "create_goal";
/// Goals that are on or paused at once; finished ones don't count.
const MAX_GOALS: usize = 10;
const MAX_AIM: usize = 200;
const MAX_CHECK_IN: usize = 500;
const MAX_PERSON: usize = 60;
/// Every goal ends: its last day is at most this far away.
const MAX_DAYS: i64 = 90;
/// Without a date, waiting on a reply stops after two weeks and "until it's done" after
/// a month. The sheet shows the day, so the user can change it.
const REPLY_DAYS: i64 = 14;
const DONE_DAYS: i64 = 30;
const ARGS: [&str; 8] = ["aim", "check_in", "style", "days", "time", "until", "done_when", "person"];

/// A goal the user approved. Its check-ins are a routine row; this is what the goal is
/// for and how it ends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    /// What it is for, in the user's words: "Prepare for the Lisbon trip".
    pub aim: String,
    /// What each check-in does: a request run on its own, or what to remind about.
    pub check_in: String,
    /// run (the check-in is a request) or remind (it reminds the user).
    pub style: String,
    /// When check-ins happen, as routines store it: every_day, weekdays, … and HH:MM.
    pub days: String,
    pub time: String,
    /// The last day, YYYY-MM-DD on this Mac's calendar.
    pub until: String,
    /// What ends it before then: date (nothing else), reply (an email from `person`) or
    /// you (the user marks it done).
    pub done_when: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    /// active, paused, done or ended.
    pub state: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Why it ended, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    /// Why the last check-in was skipped or didn't finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn open(goal: &Goal) -> bool {
    matches!(goal.state.as_str(), "active" | "paused")
}

pub fn initialize(store: &Store) -> Result<(), String> {
    store
        .db
        .execute_batch("CREATE TABLE IF NOT EXISTS goals (id TEXT PRIMARY KEY, data TEXT NOT NULL, created_at TEXT NOT NULL);")
        .map_err(|e| e.to_string())
}
/// Every goal, oldest first.
pub fn all(store: &Store) -> Result<Vec<Goal>, String> {
    let mut statement = store.db.prepare("SELECT data FROM goals ORDER BY created_at, id").map_err(|e| e.to_string())?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    rows.map(|row| serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())).collect()
}
pub fn get(store: &Store, id: &str) -> Result<Option<Goal>, String> {
    let data: Option<String> = store
        .db
        .query_row("SELECT data FROM goals WHERE id=?1", [id], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    data.map(|data| serde_json::from_str(&data).map_err(|e| e.to_string())).transpose()
}
pub fn save(store: &Store, goal: &Goal) -> Result<(), String> {
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO goals(id,data,created_at) VALUES(?1,?2,?3)",
            rusqlite::params![goal.id, serde_json::to_string(goal).map_err(|e| e.to_string())?, goal.created_at],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The routine that holds a goal's check-ins.
pub fn check_in_id(goal_id: &str) -> String {
    format!("{goal_id}/check-in")
}
/// The goal a routine belongs to, if it is a goal's check-in.
pub fn owner(routine_id: &str) -> Option<&str> {
    routine_id.strip_suffix("/check-in").filter(|id| id.starts_with("goal-"))
}
/// The goal's own conversation, where every check-in reports.
pub(crate) fn thread(store: &Store, goal_id: &str) -> Result<(String, String), String> {
    let goal = get(store, goal_id)?.ok_or("This goal was deleted.")?;
    Ok((format!("goal:{goal_id}"), format!("Goal: {}", goal.aim.chars().take(85).collect::<String>())))
}

/// A goal asked for in chat, all from the user's own words.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub aim: String,
    pub check_in: String,
    pub style: &'static str,
    pub days: &'static str,
    pub time: String,
    pub until: NaiveDate,
    pub done_when: &'static str,
    pub person: Option<String>,
}
impl Request {
    pub fn arguments(&self) -> Value {
        let mut arguments = json!({
            "aim": self.aim, "check_in": self.check_in, "style": self.style, "days": self.days,
            "time": self.time, "until": self.until.format("%Y-%m-%d").to_string(), "done_when": self.done_when,
        });
        if let Some(person) = &self.person {
            arguments["person"] = json!(person);
        }
        arguments
    }
}

fn capitalized(text: &str) -> String {
    let mut out: String = text.trim().into();
    if let Some(first) = out.chars().next() {
        out.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    out
}
/// "Work on the tax return" → "work on the tax return", leaving "Ana" or "iPhone" alone.
fn lowered(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && second.is_lowercase() => {
            let mut out: String = first.to_lowercase().collect();
            out.push_str(&text[first.len_utf8()..]);
            out
        }
        _ => text.into(),
    }
}

fn weekday(word: &str) -> Option<chrono::Weekday> {
    routines::DAY_NAMES.iter().find_map(|(name, day)| {
        (*name == word || word.len() >= 3 && name.starts_with(word)).then_some(*day)
    })
}
fn month(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    MONTHS.iter().position(|name| *name == word || word.len() >= 3 && name.starts_with(word)).map(|index| index as u32 + 1)
}
/// "6", "6th", "21st".
fn day_number(word: &str, suffix_required: bool) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &word[digits.len()..];
    if !(suffix.is_empty() && !suffix_required || ["st", "nd", "rd", "th"].contains(&suffix)) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|day| (1..=31).contains(day))
}
fn month_day(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year >= today {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, month, day)
    }
}

/// A day from the user's words: "Friday", "next Friday", "tomorrow", "November 6",
/// "the 6th", "6 November", "2026-11-06". A weekday is the next one after today, so "by
/// Friday" said on a Friday is a week away; the sheet shows the exact day.
pub(crate) fn parse_day(words: &[&str], today: NaiveDate) -> Option<NaiveDate> {
    let words: Vec<&str> = words.iter().copied().filter(|word| !matches!(*word, "the" | "of" | "on")).collect();
    match words.as_slice() {
        ["today" | "tonight"] => Some(today),
        ["tomorrow"] => today.succ_opt(),
        [word] if NaiveDate::parse_from_str(word, "%Y-%m-%d").is_ok() => NaiveDate::parse_from_str(word, "%Y-%m-%d").ok(),
        [word] | ["this" | "next" | "coming", word] if weekday(word).is_some() => {
            let wanted = weekday(word)?;
            (1..=7).map(|offset| today + chrono::Duration::days(offset)).find(|day| day.weekday() == wanted)
        }
        [first, second] if month(first).is_some() && day_number(second, false).is_some() => month_day(month(first)?, day_number(second, false)?, today),
        [first, second] if day_number(first, false).is_some() && month(second).is_some() => month_day(month(second)?, day_number(first, false)?, today),
        [word] if day_number(word, true).is_some() => {
            let day = day_number(word, true)?;
            let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), day).filter(|date| *date >= today);
            this_month.or_else(|| {
                let next = today.with_day(1)? + chrono::Duration::days(32);
                NaiveDate::from_ymd_opt(next.year(), next.month(), day)
            })
        }
        _ => None,
    }
}

/// What "until …" says ends the goal.
#[derive(Debug, Clone, PartialEq)]
enum End {
    Day(NaiveDate),
    /// The user says it's done.
    Done,
    /// Someone replies: a name, or None for "she", "he" or "they".
    Reply(Option<String>),
}

const DONE: [&str; 26] = [
    "it's done", "it is done", "its done", "done", "it's finished", "it is finished", "finished", "i'm done", "im done",
    "i am done", "i'm finished", "i am finished", "i've finished", "i have finished", "i finish", "i finish it",
    "i've done it", "i have done it", "i'm done with it", "i am done with it", "it's complete", "it is complete",
    "it's completed", "complete", "it's sorted", "it's filed",
];
const REPLIES: [&str; 13] = [
    "replies", "responds", "answers", "writes back", "gets back to me", "gets back", "replies to me", "responds to me",
    "emails me back", "emails back", "writes back to me", "emails me", "answers me",
];

/// Who a person is, as the user wrote them: a name of up to three words or an address.
fn person_ok(person: &str) -> bool {
    let person = person.trim();
    !person.is_empty()
        && person.chars().count() <= MAX_PERSON
        && (crate::apps::plain_address(person)
            || person.split_whitespace().count() <= 3
                && person.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\'' | '.'))
                && person.chars().any(char::is_alphabetic))
}

/// `lower` is the text after "until" in lowercase; `original` the same text as written.
fn parse_end(lower: &str, original: &str, today: NaiveDate) -> Option<End> {
    let lower = lower.trim().trim_end_matches(['.', '!', '?', ',']).trim();
    if DONE.contains(&lower) {
        return Some(End::Done);
    }
    for reply in REPLIES {
        let Some(who) = lower.strip_suffix(reply).and_then(|who| who.strip_suffix(' ')) else {
            continue;
        };
        let who = who.trim();
        if matches!(who, "she" | "he" | "they") {
            return Some(End::Reply(None));
        }
        if ["someone", "somebody", "anyone", "anybody", "it", "i", "you", "we"].contains(&who) || who.is_empty() {
            return None;
        }
        let name = original.trim()[..who.len()].trim();
        return person_ok(name).then(|| End::Reply(Some(name.to_string())));
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    parse_day(&words, today).map(End::Day)
}

/// The person an aim is about, for "until she replies": the first capitalised name after
/// the verb ("follow up with Ana", "remind me to call Mum"), or the word after "with".
fn person_in(aim: &str) -> Option<String> {
    let words: Vec<&str> = aim.split_whitespace().collect();
    let bare = |word: &str| word.trim_end_matches([',', '.', ';', ':', '!', '?']).to_string();
    let start = words.iter().enumerate().skip(1).find(|(_, word)| {
        let word = bare(word);
        word != "I" && word.chars().next().is_some_and(char::is_uppercase) && word.chars().all(|c| c.is_alphabetic() || matches!(c, '-' | '\''))
    });
    if let Some((index, _)) = start {
        let name: Vec<String> = words[index..]
            .iter()
            .take(3)
            .take_while(|word| bare(word).chars().next().is_some_and(char::is_uppercase))
            .map(|word| bare(word))
            .collect();
        let name = name.join(" ");
        return person_ok(&name).then_some(name);
    }
    let after = words.iter().position(|word| word.eq_ignore_ascii_case("with"))?;
    let name = bare(words.get(after + 1)?);
    person_ok(&name).then_some(name)
}

const FOLLOW_UPS: [&str; 6] = ["follow up with ", "follow-up with ", "keep following up with ", "chase ", "keep chasing ", "nudge "];
/// Ways a request opens that only say something about the user, not ask for anything.
const STATEMENTS: [&str; 17] = [
    "i", "i'm", "im", "we", "we're", "my", "our", "it", "it's", "there", "he", "she", "they", "you", "don't", "never", "stop",
];

/// Where each of `markers` occurs in `lower`, as byte ranges, rightmost first.
fn last_marker(lower: &str, markers: &[&str]) -> Vec<(usize, usize)> {
    let mut found: Vec<(usize, usize)> = markers
        .iter()
        .flat_map(|marker| lower.match_indices(marker).map(|(index, _)| (index, index + marker.len())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

pub fn parse_request(text: &str) -> Option<Request> {
    parse_at(text, Local::now().date_naive())
}

/// "Help me prepare for the Lisbon trip by Friday", "Follow up with Ana until she
/// replies", "Remind me to work on the tax return every evening until it's done". A
/// goal needs an end the user said: "by" a day after "help me" or "my goal is to", or
/// "until" a day, a reply or "it's done". Without a check-in time it picks one the sheet
/// shows. Questions and statements about the user ("I'm away until Friday") are not goals.
pub(crate) fn parse_at(text: &str, today: NaiveDate) -> Option<Request> {
    let text = text.trim();
    if text.is_empty() || text.len() > 2000 || text.contains('\n') {
        return None;
    }
    let (body, asked) = routines::without_polite(text);
    if body.trim_end().ends_with('?') && !asked {
        return None;
    }
    let body = body.trim().trim_end_matches(['.', '!', '?']).trim().replace(['’', '‘'], "'");
    let lower = body.to_lowercase();
    // Offsets below are shared between the two, which needs case changes that keep length.
    if lower.len() != body.len() {
        return None;
    }
    let helped = lower.starts_with("help me ") || lower.starts_with("my goal is to ");
    let (aim, end, by) = last_marker(&lower, &[" until ", " till ", " til "])
        .into_iter()
        .find_map(|(start, end)| Some((&body[..start], parse_end(&lower[end..], &body[end..], today)?, None)))
        .or_else(|| {
            if !helped {
                return None;
            }
            last_marker(&lower, &[" by ", " before "]).into_iter().find_map(|(start, end)| {
                let words: Vec<&str> = lower[end..].split_whitespace().collect();
                let day = parse_day(&words, today)?;
                let day = if lower[start..end].contains("before") { day.pred_opt()? } else { day };
                Some((&body[..start], End::Day(day), Some(day)))
            })
        })?;
    let aim = aim.trim().trim_end_matches([',', ';', '-']).trim();
    let (aim, schedule) = match routines::split_schedule(aim) {
        Some((rest, days, time)) => (rest, Some((days, time))),
        None => (aim.to_string(), None),
    };
    let aim_lower = aim.to_lowercase();
    let statement = STATEMENTS.contains(&aim_lower.split_whitespace().next().unwrap_or_default()) || aim_lower.starts_with("do not ");
    if aim.split_whitespace().count() < 2 || statement && !helped || aim_lower.starts_with("search ") {
        return None;
    }
    let follow_up = FOLLOW_UPS.iter().find(|prefix| aim_lower.starts_with(**prefix));
    let (style, goal_aim, check_in) = if let Some(rest) = aim_lower.strip_prefix("remind me to ").map(|_| &aim["remind me to ".len()..]) {
        ("remind", capitalized(rest), capitalized(rest))
    } else if aim_lower.starts_with("help me ") {
        ("run", capitalized(&aim["help me ".len()..]), capitalized(&aim))
    } else if aim_lower.starts_with("my goal is to ") {
        let rest = &aim["my goal is to ".len()..];
        ("run", capitalized(rest), format!("Help me {}", rest.trim()))
    } else if let Some(prefix) = follow_up {
        ("run", capitalized(&aim), format!("Draft a short follow-up email to {}", aim[prefix.len()..].trim()))
    } else {
        ("run", capitalized(&aim), capitalized(&aim))
    };
    // "Until Friday" on its own is common in plain requests; it makes a goal only with a
    // schedule, a reminder or "help me".
    let deliberate = helped || style == "remind" || schedule.is_some() || follow_up.is_some();
    let (until, done_when, person) = match end {
        End::Day(day) if by.is_some() || deliberate => (day, "date", None),
        End::Done if deliberate => (today + chrono::Duration::days(DONE_DAYS), "you", None),
        End::Reply(who) => {
            let person = who.or_else(|| person_in(&aim))?;
            (today + chrono::Duration::days(REPLY_DAYS), "reply", Some(person))
        }
        _ => return None,
    };
    let (days, time) = schedule.unwrap_or(match (style, done_when) {
        ("remind", _) => ("every_day", "18:00".into()),
        (_, "reply") => ("weekdays", "09:00".into()),
        _ => ("every_day", "09:00".into()),
    });
    let request = Request { aim: goal_aim, check_in, style, days, time, until, done_when, person };
    // Only what would pass approval is proposed; anything else goes to the model.
    validate_on(&request.arguments(), today).ok()?;
    Some(request)
}

fn text<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}
fn until_label(until: NaiveDate) -> String {
    until.format("%A, %B %-d, %Y").to_string()
}
/// The last day as stored, or as the sheet shows it.
fn parse_until(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(text, "%A, %B %d, %Y"))
        .ok()
        .or_else(|| {
            let lower = text.to_lowercase().replace(',', " ");
            parse_day(&lower.split_whitespace().collect::<Vec<_>>(), today)
        })
}

pub fn validate(args: &Value) -> Result<Map<String, Value>, String> {
    validate_on(args, Local::now().date_naive())
}

/// Checks and normalizes create_goal's arguments, from the controller or the sheet.
fn validate_on(args: &Value, today: NaiveDate) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    if let Some(extra) = args.keys().find(|key| !ARGS.contains(&key.as_str())) {
        return Err(format!("{TOOL} does not accept {extra}."));
    }
    let clean = |value: &str, max: usize| value.chars().count() <= max && !value.chars().any(char::is_control);
    let aim = text(args, "aim").filter(|aim| clean(aim, MAX_AIM)).ok_or("Say what the goal is, in up to 200 characters.")?;
    let check_in = text(args, "check_in")
        .filter(|check_in| clean(check_in, MAX_CHECK_IN))
        .ok_or("Say what each check-in should do, in up to 500 characters.")?;
    // A check-in runs while you're away; "this" would mean whatever is on screen then.
    if crate::focus::mentions_this(check_in) {
        return Err("A goal checks in on its own, so it can’t use what’s on your screen. Say what it should look at instead.".into());
    }
    if routines::split_schedule(check_in).is_some() || routines::parse_request(check_in).is_some() {
        return Err("Give the goal one schedule.".into());
    }
    if parse_at(check_in, today).is_some() {
        return Err("A goal’s check-in can’t set up another goal.".into());
    }
    let style = text(args, "style").filter(|style| ["run", "remind"].contains(style)).ok_or("Choose whether a check-in runs a request or reminds you.")?;
    let days = text(args, "days").and_then(routines::normalize_days).ok_or("Choose every day, weekdays, weekends or one day of the week.")?;
    let time = text(args, "time").and_then(routines::normalize_time).ok_or("Give check-ins a time, like 9:00 AM.")?;
    let latest = today + chrono::Duration::days(MAX_DAYS);
    let until = text(args, "until")
        .and_then(|until| parse_until(until, today))
        .filter(|until| *until >= today && *until <= latest)
        .ok_or_else(|| format!("Give the goal a last day between today and {}.", until_label(latest)))?;
    let done_when = text(args, "done_when").filter(|done| ["date", "reply", "you"].contains(done)).ok_or("Say what ends the goal.")?;
    let person = text(args, "person");
    let mut out = Map::new();
    match (done_when, person) {
        ("reply", Some(person)) if person_ok(person) => {
            out.insert("person".into(), json!(person));
        }
        ("reply", _) => return Err("Say whose reply ends the goal: a name or an email address.".into()),
        (_, Some(_)) => return Err("Only a goal that waits for a reply names a person.".into()),
        _ => {}
    }
    out.insert("aim".into(), json!(aim));
    out.insert("check_in".into(), json!(check_in));
    out.insert("style".into(), json!(style));
    out.insert("days".into(), json!(days));
    out.insert("time".into(), json!(time));
    out.insert("until".into(), json!(until.format("%Y-%m-%d").to_string()));
    out.insert("done_when".into(), json!(done_when));
    Ok(out)
}

fn ends_label(done_when: &str, person: &str, until: &str) -> String {
    match done_when {
        "reply" => format!("When an email from {person} arrives, or after {until}"),
        "you" => format!("When you mark it done, or after {until}"),
        _ => format!("After {until}, or when you mark it done"),
    }
}

pub fn describe(args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let value = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let until = NaiveDate::parse_from_str(&value("until"), "%Y-%m-%d").map(until_label).unwrap_or_else(|_| value("until"));
    let field = |key: &str, label: &str, value: String, kind| Field { key: key.into(), label: label.into(), value, kind };
    let mut fields = vec![
        field("aim", "Goal", value("aim"), "long"),
        field("check_in", if value("style") == "remind" { "I’ll remind you to" } else { "Each check-in, I’ll do" }, value("check_in"), "long"),
        field("days", "Check-ins", routines::days_label(&value("days")), "text"),
        field("time", "Time", routines::time_label(&value("time")), "text"),
        field("until", "Last day", until.clone(), "text"),
        field("style", "Check-in", value("style"), "hidden"),
        field("done_when", "Ends", value("done_when"), "hidden"),
    ];
    if value("done_when") == "reply" {
        fields.push(field("person", "Waiting for a reply from", value("person"), "text"));
    }
    fields.push(field("ends", "Ends", ends_label(&value("done_when"), &value("person"), &until), "fixed"));
    let mail = if value("done_when") == "reply" {
        " Before each check-in I look for their email in Mail, only while Mail is already open; a check-in waits if I can’t."
    } else {
        ""
    };
    (
        "Set up this goal?".into(),
        fields,
        format!("Each check-in happens on its own and reports in this goal’s conversation, with a notification. Anything it would change, like a draft, waits for your OK under Waiting for you.{mail} Check-ins run while Buntini is open and your Mac is awake. Pause, finish or delete the goal in History → Goals."),
    )
}

fn failure(message: String, outcome: ActionOutcome) -> ActionReceipt {
    ActionReceipt {
        outcome,
        tool: TOOL.into(),
        object_id: None,
        destination: Some("Buntini · Goals".into()),
        message: message.clone(),
        evidence: json!({}),
        error: Some(message),
    }
}
fn matches(goal: &Goal, args: &Map<String, Value>) -> bool {
    let value = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
    goal.aim == value("aim")
        && goal.check_in == value("check_in")
        && goal.style == value("style")
        && goal.days == value("days")
        && goal.time == value("time")
        && goal.until == value("until")
        && goal.done_when == value("done_when")
        && goal.person.as_deref().unwrap_or_default() == value("person")
}
/// When the goal's last day is over, in this Mac's time zone.
fn after_last_day(until: &str) -> Option<DateTime<Utc>> {
    let next = NaiveDate::parse_from_str(until, "%Y-%m-%d").ok()?.succ_opt()?.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&next).earliest().map(|at| at.with_timezone(&Utc))
}

/// Saves an approved goal with its check-in routine and reads both back. As with
/// routines, `intent_id` comes from the durable journal: replaying the approval verifies
/// the goal it made, and a deleted goal is not brought back.
pub fn execute(store: &Store, intent_id: &str, raw: &Map<String, Value>) -> ActionReceipt {
    let args = match validate(&Value::Object(raw.clone())) {
        Ok(args) => args,
        Err(error) => return failure(error, ActionOutcome::Failed),
    };
    if intent_id.is_empty()
        || intent_id.len() > 200
        || !intent_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return failure("This goal has no valid approval to go with it.".into(), ActionOutcome::Failed);
    }
    let id = format!("goal-{intent_id}");
    let transaction = match store.db.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => return failure(error.to_string(), ActionOutcome::Failed),
    };
    // Take SQLite's write lock before checking the approval and the limit.
    if let Err(error) = store.db.execute("UPDATE goals SET data=data WHERE 0", []) {
        return failure(error.to_string(), ActionOutcome::Failed);
    }
    let value = |key: &str| args[key].as_str().unwrap_or_default().to_string();
    let result = (|| -> Result<(Goal, Option<Routine>), String> {
        let serialized = serde_json::to_string(&args).map_err(|e| e.to_string())?;
        let previous: Option<(String, String)> = store
            .db
            .query_row("SELECT tool,args FROM monitor_intents WHERE id=?1", [intent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((tool, previous)) = &previous {
            if tool != TOOL || previous != &serialized {
                return Err("This approval was already used for something else. Ask again to set up the goal.".into());
            }
            if get(store, &id)?.is_none() {
                return Err("This goal was set up and later deleted. Ask again if you want it back.".into());
            }
        }
        store
            .db
            .execute("INSERT OR IGNORE INTO monitor_intents(id,tool,args,monitor_id) VALUES(?1,?2,?3,?4)", rusqlite::params![intent_id, TOOL, serialized, id])
            .map_err(|e| e.to_string())?;
        if let Some(existing) = get(store, &id)? {
            if !matches(&existing, &args) {
                return Err("This goal already exists with different details. It wasn’t changed.".into());
            }
        } else {
            if all(store)?.iter().filter(|goal| open(goal)).count() >= MAX_GOALS {
                return Err(format!("You can have up to {MAX_GOALS} goals at once. Finish or delete one in History → Goals first."));
            }
            let next = routines::next_after(&value("days"), &value("time"), Local::now()).ok_or("That time couldn’t be scheduled.")?;
            if after_last_day(&value("until")).is_none_or(|end| next >= end) {
                return Err("The first check-in would come after the goal’s last day. Choose a later day or an earlier time.".into());
            }
            let created_at = now();
            save(
                store,
                &Goal {
                    id: id.clone(),
                    aim: value("aim"),
                    check_in: value("check_in"),
                    style: value("style"),
                    days: value("days"),
                    time: value("time"),
                    until: value("until"),
                    done_when: value("done_when"),
                    person: args.get("person").and_then(Value::as_str).map(str::to_string),
                    state: "active".into(),
                    created_at: created_at.clone(),
                    ended_at: None,
                    outcome: None,
                    last_task_id: None,
                    note: None,
                },
            )?;
            store.save_routine(&Routine {
                id: check_in_id(&id),
                task: value("check_in"),
                days: value("days"),
                time: value("time"),
                enabled: true,
                built_in: false,
                created_at,
                next_run_at: next.to_rfc3339(),
                last_run_at: None,
                last_task_id: None,
                last_error: None,
            })?;
            store.audit("goal.allowed", &format!("{id}: approved goal until {}, check-ins {} at {}", value("until"), value("days"), value("time")))?;
        }
        let saved = get(store, &id)?.ok_or("The goal couldn’t be read back.")?;
        let routine = store.routine(&check_in_id(&id))?;
        let scheduled = routine.as_ref().is_some_and(|routine| {
            routine.task == saved.check_in && routine.days == saved.days && routine.time == saved.time && routine.enabled == (saved.state == "active")
        });
        // A goal that has already ended has no check-ins left to read back.
        if !matches(&saved, &args) || (open(&saved) && !scheduled) {
            return Err("The saved goal didn’t match what you approved.".into());
        }
        Ok((saved, routine))
    })();
    let (saved, routine) = match result {
        Ok(saved) => saved,
        Err(error) => return failure(error, ActionOutcome::Failed),
    };
    if let Err(error) = transaction.commit() {
        return failure(format!("I couldn’t confirm the goal was saved: {error}. Check History → Goals before asking again."), ActionOutcome::Unknown);
    }
    let until = NaiveDate::parse_from_str(&saved.until, "%Y-%m-%d").map(until_label).unwrap_or_else(|_| saved.until.clone());
    let first = routine.map(|routine| format!(" The first is {}.", routines::when(&routine.next_run_at))).unwrap_or_default();
    ActionReceipt {
        outcome: ActionOutcome::Succeeded,
        tool: TOOL.into(),
        object_id: Some(id),
        destination: Some("Buntini · Goals".into()),
        message: format!(
            "Set up your goal “{}”: check-ins {} at {} until {until}.{first}",
            saved.aim,
            routines::days_label(&saved.days),
            routines::time_label(&saved.time),
        ),
        evidence: json!({"verified": true, "record": saved, "networkRequestMade": false}),
        error: None,
    }
}

/// A turn in the goal's conversation that Buntini writes itself, without a model: a
/// reminder, or why the goal ended. `touched` names the local data it was based on.
fn turn(answer: String, touched: &[&str]) -> Result<TaskRecord, String> {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4().to_string(), "prompt": "", "answer": answer, "state": "completed",
        "createdAt": now(), "sources": [], "files": [], "backend": "goal", "touched": touched,
    }))
    .map_err(|e| e.to_string())
}

/// An email from `person` that arrived after `since` ("YYYY-MM-DD HH:MM", local), as
/// (received, sender). An automated sender never counts.
fn reply_in(found: &[(String, String)], person: &str, since: &str) -> Option<(String, String)> {
    let person = person.trim();
    found
        .iter()
        .find(|(received, sender)| {
            received.as_str() > since
                && !crate::triage::automated(sender)
                && if crate::apps::plain_address(person) {
                    crate::focus::sender_address(sender).as_deref() == Some(person.to_lowercase().as_str())
                } else {
                    crate::triage::from_sender(&person.to_lowercase(), sender)
                }
        })
        .cloned()
}

/// Looks in Mail, without opening it, for an email from the person since the goal began.
async fn look_for_reply(goal: &Goal) -> Result<Option<(String, String)>, String> {
    let person = goal.person.as_deref().ok_or("This goal names no one to wait for.")?;
    let since = DateTime::parse_from_rfc3339(&goal.created_at).map_err(|e| e.to_string())?.with_timezone(&Local);
    let days = ((Local::now() - since).num_days() + 1).clamp(1, MAX_DAYS + 1) as u32;
    // Mail's own filter only narrows the list; the match is checked here.
    let query = if crate::apps::plain_address(person) { person } else { person.split_whitespace().next().unwrap_or(person) };
    let found = crate::apps::inbox_senders(query, days, crate::apps::Launch::IfRunning).await?;
    Ok(reply_in(&found, person, &since.format("%Y-%m-%d %H:%M").to_string()))
}

fn set_note(store: &Store, goal_id: &str, note: Option<String>, last_task_id: Option<&str>) -> Result<(), String> {
    if let Some(mut goal) = get(store, goal_id)? {
        goal.note = note;
        if let Some(task_id) = last_task_id {
            goal.last_task_id = Some(task_id.into());
        }
        save(store, &goal)?;
    }
    Ok(())
}

/// Skips this check-in and says why under the goal; the next one happens as planned.
fn skip(store: &Store, routine: &Routine, goal_id: &str, scheduled_for: &str, note: String) -> Result<(), String> {
    routines::pass(store, &routine.id, scheduled_for)?;
    store.audit("goal.check_in_skipped", &format!("{goal_id}: {note}"))?;
    set_note(store, goal_id, Some(note), None)
}

/// What a due check-in comes to.
#[derive(Debug)]
enum Next {
    /// Nothing to do this time; the goal says why, if there is a reason to give.
    Skipped,
    /// The evidence says the goal is over.
    End { status: &'static str, outcome: String, touched: Vec<&'static str> },
    /// A reminder was written in the goal's conversation for this check-in.
    Reminded { task: TaskRecord, run_id: String },
    /// Run the check-in's request; `touched` is what was read to decide that.
    Run { touched: Vec<&'static str> },
}

/// Decides a due check-in. `look` reads Mail for a reply, and is only called for a goal
/// waiting on one, with Mail allowed.
async fn decide<F, Fut>(state: &AppState, routine: &Routine, goal_id: &str, scheduled_for: &str, today: NaiveDate, look: F) -> Result<Next, String>
where
    F: FnOnce(Goal) -> Fut,
    Fut: std::future::Future<Output = Result<Option<(String, String)>, String>>,
{
    let goal = get(&state.store.lock().unwrap(), goal_id)?.ok_or("This goal was deleted.")?;
    if goal.state != "active" {
        routines::pass(&state.store.lock().unwrap(), &routine.id, scheduled_for)?;
        return Ok(Next::Skipped);
    }
    if past(&goal, today) {
        return Ok(Next::End { status: "ended", outcome: last_day_outcome(&goal), touched: vec![] });
    }
    {
        let store = state.store.lock().unwrap();
        // A proposal from the last check-in is still waiting: don't pile up another.
        let waiting = routine.last_task_id.as_deref().is_some_and(|task_id| {
            store.pending_approvals().is_ok_and(|pending| pending.iter().any(|item| item.task_id == task_id))
        });
        if waiting {
            skip(&store, routine, goal_id, scheduled_for, "The last check-in is still waiting for your OK under Waiting for you, so I skipped this one.".into())?;
            return Ok(Next::Skipped);
        }
    }
    let mut touched = Vec::new();
    if goal.done_when == "reply" {
        let person = goal.person.clone().unwrap_or_default();
        let allowed = crate::apps::allowed(&state.store.lock().unwrap().settings()?, "mail");
        let found = if allowed { look(goal.clone()).await } else { Err(format!("Mail isn’t allowed in Settings → Apps, so I couldn’t check for a reply from {person}.")) };
        match found {
            Ok(Some((received, sender))) => {
                let when = chrono::NaiveDateTime::parse_from_str(&received, "%Y-%m-%d %H:%M")
                    .map(|at| at.format("%A, %B %-d at %-I:%M %p").to_string())
                    .unwrap_or(received);
                let sender: String = sender.chars().take(80).collect();
                let outcome = format!("An email from {sender} arrived on {when}, so this goal is done and I’ve stopped following up.");
                return Ok(Next::End { status: "done", outcome, touched: vec!["mail"] });
            }
            Ok(None) => touched.push("mail"),
            Err(error) => {
                let note = if crate::apps::is_not_running(&error) {
                    format!("Mail wasn’t open, so I couldn’t check for a reply from {person}. I’ll look again at the next check-in.")
                } else if !allowed {
                    error
                } else {
                    format!("I couldn’t check Mail for a reply from {person}: {error}")
                };
                skip(&state.store.lock().unwrap(), routine, goal_id, scheduled_for, note)?;
                return Ok(Next::Skipped);
            }
        }
    }
    if goal.style != "remind" {
        return Ok(Next::Run { touched });
    }
    let task = turn(format!("Time to {}. When it’s done, mark this goal Done in History → Goals.", lowered(goal.check_in.trim_end_matches('.'))), &touched)?;
    let store = state.store.lock().unwrap();
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    let run_id = routines::claim(&store, &routine.id, scheduled_for, &task.id)?;
    store.save_task(&task)?;
    set_note(&store, goal_id, None, Some(&task.id))?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(Next::Reminded { task, run_id })
}

/// One check-in, when its routine is due: end the goal if its evidence says so, else do
/// what the check-in says.
pub(crate) async fn check_in(app: &tauri::AppHandle, routine: &Routine, goal_id: &str, scheduled_for: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let touched = match decide(&state, routine, goal_id, scheduled_for, Local::now().date_naive(), |goal| async move { look_for_reply(&goal).await }).await? {
        Next::Skipped => return Ok(()),
        Next::End { status, outcome, touched } => return end(app, goal_id, status, outcome, &touched, true).await,
        Next::Reminded { task, run_id } => return routines::finish(app, routine, Some(&run_id), &task).await,
        Next::Run { touched } => touched,
    };
    let task = routines::run_claimed(app, routine, &routine.task, scheduled_for).await?;
    let store = state.store.lock().unwrap();
    // The run itself didn't read Mail; this check-in did, before it.
    if let Some(mut saved) = store.task(&task.id)?.filter(|_| !touched.is_empty()) {
        for share in &touched {
            if !saved.touched.iter().any(|id| id == share) {
                saved.touched.push(share.to_string());
            }
        }
        store.save_task(&saved)?;
    }
    let note = (task.state == "error").then(|| "The last check-in didn’t finish. Open its conversation to see why.".to_string());
    set_note(&store, goal_id, note, Some(&task.id))
}

fn past(goal: &Goal, today: NaiveDate) -> bool {
    NaiveDate::parse_from_str(&goal.until, "%Y-%m-%d").is_ok_and(|until| today > until)
}
fn last_day_outcome(goal: &Goal) -> String {
    let until = NaiveDate::parse_from_str(&goal.until, "%Y-%m-%d").map(until_label).unwrap_or_else(|_| goal.until.clone());
    match (goal.done_when.as_str(), &goal.person) {
        ("reply", Some(person)) => format!("This goal’s last day, {until}, has passed, so I’ve stopped checking for a reply from {person}."),
        _ => format!("This goal’s last day, {until}, has passed, so I’ve stopped checking in."),
    }
}
/// Open goals whose last day is over.
fn overdue(goals: &[Goal], today: NaiveDate) -> Vec<Goal> {
    goals.iter().filter(|goal| open(goal) && past(goal, today)).cloned().collect()
}

/// Ends a goal: its check-ins stop and are removed, and an active goal says why in its
/// conversation, with a notification. Locks the running map before the store, as
/// everywhere else.
fn close(state: &AppState, goal_id: &str, status: &str, outcome: String, touched: &[&str], tell: bool) -> Result<Option<TaskRecord>, String> {
    let active = state.active.lock().unwrap();
    let store = state.store.lock().unwrap();
    let Some(mut goal) = get(&store, goal_id)? else {
        return Ok(None);
    };
    if !open(&goal) {
        return Ok(None);
    }
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    if let Some(routine) = store.routine(&check_in_id(goal_id))? {
        routines::stop_running(&active, &routine);
        store.delete_routine(&routine.id)?;
    }
    let task = if tell && goal.state == "active" {
        let task = turn(outcome.clone(), touched)?;
        routines::assign_thread(&store, &task.id, &thread(&store, goal_id)?)?;
        store.save_task(&task)?;
        goal.last_task_id = Some(task.id.clone());
        Some(task)
    } else {
        None
    };
    goal.state = status.into();
    goal.ended_at = Some(now());
    goal.outcome = Some(outcome);
    goal.note = None;
    save(&store, &goal)?;
    store.audit("goal.ended", &format!("{goal_id}: {status}"))?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(task)
}

async fn end(app: &tauri::AppHandle, goal_id: &str, status: &str, outcome: String, touched: &[&str], tell: bool) -> Result<(), String> {
    use tauri::Emitter;
    let task = close(&app.state::<AppState>(), goal_id, status, outcome, touched, tell)?;
    if let Some(task) = task {
        let _ = crate::notifications::deliver_routine(app, &task, true).await;
        let _ = app.emit("background-updated", json!({"goalId": goal_id, "taskId": task.id}));
    }
    Ok(())
}

/// Ends every goal whose last day has passed, also while Buntini was closed. Paused goals
/// end quietly.
pub(crate) async fn end_overdue(app: &tauri::AppHandle) -> Result<(), String> {
    let goals = all(&app.state::<AppState>().store.lock().unwrap())?;
    for goal in overdue(&goals, Local::now().date_naive()) {
        end(app, &goal.id, "ended", last_day_outcome(&goal), &[], true).await?;
    }
    Ok(())
}

/// Pause, resume or mark a goal done. Resuming picks the next check-in from now, never a
/// missed one. Done is the user's word, the one end no evidence is needed for.
#[tauri::command]
pub fn update_goal(id: String, action: String, state: State<'_, AppState>) -> Result<Goal, String> {
    update_in(&state, &id, &action)
}
fn update_in(state: &AppState, id: &str, action: &str) -> Result<Goal, String> {
    if action == "done" {
        close(state, id, "done", "You marked it done.".into(), &[], false)?;
        return get(&state.store.lock().unwrap(), id)?.ok_or("This goal no longer exists.".into());
    }
    let active = state.active.lock().unwrap();
    let store = state.store.lock().unwrap();
    let mut goal = get(&store, id)?.ok_or("This goal no longer exists.")?;
    let mut routine = store.routine(&check_in_id(id))?.ok_or("This goal has already ended.")?;
    match (action, goal.state.as_str()) {
        ("pause", "active") => {
            routines::stop_running(&active, &routine);
            routine.enabled = false;
            goal.state = "paused".into();
        }
        ("resume", "paused") => {
            if past(&goal, Local::now().date_naive()) {
                return Err("This goal’s last day has passed.".into());
            }
            routine.next_run_at = routines::next_after(&routine.days, &routine.time, Local::now()).ok_or("That time couldn’t be scheduled.")?.to_rfc3339();
            routine.enabled = true;
            goal.state = "active".into();
        }
        ("pause" | "resume", _) => return Ok(goal),
        _ => return Err("Choose pause, resume or done.".into()),
    }
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    store.save_routine(&routine)?;
    save(&store, &goal)?;
    store.audit("goal.updated", &format!("{id}: {}", goal.state))?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(goal)
}

/// Deletes a goal and its check-ins. Its conversation stays in History like any other.
#[tauri::command]
pub fn delete_goal(id: String, state: State<'_, AppState>) -> Result<(), String> {
    delete_in(&state, &id)
}
fn delete_in(state: &AppState, id: &str) -> Result<(), String> {
    let active = state.active.lock().unwrap();
    let store = state.store.lock().unwrap();
    get(&store, id)?.ok_or("This goal no longer exists.")?;
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    if let Some(routine) = store.routine(&check_in_id(id))? {
        routines::stop_running(&active, &routine);
        store.delete_routine(&routine.id)?;
    }
    store.db.execute("DELETE FROM goals WHERE id=?1", [id]).map_err(|e| e.to_string())?;
    store.audit("goal.deleted", id)?;
    transaction.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};

    /// Friday 30 October 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()
    }
    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn goals_are_recognised_from_the_users_own_words() {
        let lisbon = parse_at("Help me prepare for the Lisbon trip by Friday", today()).unwrap();
        assert_eq!(
            lisbon,
            Request {
                aim: "Prepare for the Lisbon trip".into(),
                check_in: "Help me prepare for the Lisbon trip".into(),
                style: "run",
                days: "every_day",
                time: "09:00".into(),
                until: day(11, 6),
                done_when: "date",
                person: None,
            },
            "Friday said on a Friday is next week's"
        );
        let ana = parse_at("Follow up with Ana until she replies", today()).unwrap();
        assert_eq!((ana.check_in.as_str(), ana.done_when, ana.person.as_deref()), ("Draft a short follow-up email to Ana", "reply", Some("Ana")));
        assert_eq!((ana.days, ana.time.as_str(), ana.until), ("weekdays", "09:00", day(11, 13)));
        let tax = parse_at("Remind me to work on the tax return every evening until it's done", today()).unwrap();
        assert_eq!(
            (tax.aim.as_str(), tax.check_in.as_str(), tax.style, tax.days, tax.time.as_str(), tax.done_when, tax.until),
            ("Work on the tax return", "Work on the tax return", "remind", "every_day", "18:00", "you", day(11, 29))
        );
        for (prompt, until, done_when, person) in [
            ("Can you help me finish the report before November 6?", day(11, 5), "date", None),
            ("My goal is to read two chapters a day by the 15th", day(11, 15), "date", None),
            ("Every weekday at 8, check flight prices to Lisbon until next Wednesday", day(11, 4), "date", None),
            ("Remind me to call Mum every Sunday at 6pm until she answers", day(11, 13), "reply", Some("Mum")),
            ("Chase Sam Lee about the invoice until Sam Lee replies", day(11, 13), "reply", Some("Sam Lee")),
            ("follow up with ana@example.com until ana@example.com writes back", day(11, 13), "reply", Some("ana@example.com")),
            ("Remind me to stretch till tomorrow", day(10, 31), "date", None),
        ] {
            let request = parse_at(prompt, today()).unwrap_or_else(|| panic!("{prompt}"));
            assert_eq!((request.until, request.done_when, request.person.as_deref()), (until, done_when, person), "{prompt}");
        }
        assert_eq!(parse_at("Remind me to call Mum every Sunday at 6pm until she answers", today()).unwrap().days, "sunday");
        assert_eq!(parse_at("Chase Sam Lee about the invoice until Sam Lee replies", today()).unwrap().check_in, "Draft a short follow-up email to Sam Lee about the invoice");
    }

    #[test]
    fn plain_requests_questions_and_statements_are_not_goals() {
        for prompt in [
            "Remind me to call mum by Friday",
            "I'm in Lisbon until Friday",
            "What's on my calendar until Friday?",
            "Keep the lights on until 10pm",
            "Play music until I fall asleep",
            "Book a table by the window",
            "Help me write an email to Ana",
            "Don't email Ana until she replies",
            "Email Ana until someone replies",
            "Summarise this every day until Friday",
            "Help me prepare by next year",
            "Remind me to stretch every day until it's done every day at 9",
            "Every weekday at 8, brief me",
            "Search for flights every day until Friday",
        ] {
            assert_eq!(parse_at(prompt, today()), None, "{prompt}");
        }
        // Goals are never mistaken for routines, and the other way round.
        assert_eq!(routines::parse_request("Remind me to work on the tax return every evening until it's done"), None);
        assert!(routines::parse_request("Every weekday at 8, brief me").is_some());
    }

    #[test]
    fn days_are_read_as_people_say_them() {
        let read = |text: &str| parse_day(&text.split_whitespace().collect::<Vec<_>>(), today());
        assert_eq!(read("friday"), Some(day(11, 6)));
        assert_eq!(read("next monday"), Some(day(11, 2)));
        assert_eq!(read("tomorrow"), Some(day(10, 31)));
        assert_eq!(read("november 6"), Some(day(11, 6)));
        assert_eq!(read("6th of november"), Some(day(11, 6)));
        assert_eq!(read("the 31st"), Some(day(10, 31)));
        assert_eq!(read("the 5th"), Some(day(11, 5)));
        assert_eq!(read("jan 3"), NaiveDate::from_ymd_opt(2027, 1, 3));
        assert_eq!(read("2026-12-01"), Some(day(12, 1)));
        for text in ["6", "10pm", "february 30", "next week", "the window", "i fall asleep"] {
            assert_eq!(read(text), None, "{text}");
        }
    }

    #[test]
    fn validation_normalizes_sheet_edits_and_bounds_every_goal() {
        let args = Value::Object(validate_on(&parse_at("Follow up with Ana until she replies", today()).unwrap().arguments(), today()).unwrap());
        assert_eq!(args["until"], "2026-11-13");
        // What the sheet shows and sends comes back unchanged after validation.
        let (_, fields, note) = describe(args.as_object().unwrap());
        assert!(note.contains("Waiting for you") && note.contains("only while Mail is already open"));
        assert_eq!(fields.iter().find(|field| field.key == "ends").unwrap().value, "When an email from Ana arrives, or after Friday, November 13, 2026");
        let sent: Map<String, Value> = fields.iter().filter(|field| field.kind != "fixed").map(|field| (field.key.clone(), json!(field.value))).collect();
        assert_eq!(Value::Object(validate_on(&Value::Object(sent), today()).unwrap()), args);
        let with = |key: &str, value: Value| {
            let mut changed = args.clone();
            changed[key] = value;
            validate_on(&changed, today())
        };
        assert_eq!(with("until", json!("Thursday, November 12, 2026")).unwrap()["until"], "2026-11-12");
        assert_eq!(with("until", json!("Friday, November 6, 2026")).unwrap()["until"], "2026-11-06");
        for (key, value) in [
            ("until", json!("2026-10-29")),
            ("until", json!("2027-03-01")),
            ("check_in", json!("Summarise this")),
            ("check_in", json!("Brief me every day at 8")),
            ("check_in", json!("Remind me to stretch every evening until it's done")),
            ("style", json!("auto_approve")),
            ("done_when", json!("model_says_so")),
            ("person", json!("")),
            ("person", json!("Ana, Sam and everyone in To")),
            ("days", json!("fortnightly")),
            ("run_now", json!(true)),
        ] {
            assert!(with(key, value.clone()).is_err(), "{key}: {value}");
        }
        let mut dated = args.clone();
        dated["done_when"] = json!("date");
        assert!(validate_on(&dated, today()).is_err(), "only a reply goal names a person");
        dated.as_object_mut().unwrap().remove("person");
        assert!(validate_on(&dated, today()).is_ok());
    }

    fn approved(store: &Store, prompt: &str, intent: &str) -> ActionReceipt {
        let request = parse_request(prompt).unwrap();
        execute(store, intent, &validate(&request.arguments()).unwrap())
    }

    #[test]
    fn approved_goals_read_back_with_their_check_in_and_replays_never_duplicate() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let receipt = approved(&store, "Remind me to work on the tax return every evening until it's done", "intent");
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        assert!(receipt.message.starts_with("Set up your goal “Work on the tax return”: check-ins every day at 6:00 PM until "), "{}", receipt.message);
        let id = receipt.object_id.clone().unwrap();
        let routine = store.routine(&check_in_id(&id)).unwrap().unwrap();
        assert_eq!((routine.task.as_str(), routine.enabled, owner(&routine.id)), ("Work on the tax return", true, Some(id.as_str())));
        assert_eq!(approved(&store, "Remind me to work on the tax return every evening until it's done", "intent").object_id, Some(id.clone()));
        assert_eq!(all(&store).unwrap().len(), 1);
        // The same approval can't make something else, and a deleted goal stays deleted.
        assert_eq!(approved(&store, "Help me prepare for the Lisbon trip by Friday", "intent").outcome, ActionOutcome::Failed);
        store.db.execute("DELETE FROM goals", []).unwrap();
        assert_eq!(approved(&store, "Remind me to work on the tax return every evening until it's done", "intent").outcome, ActionOutcome::Failed);
        // Plain routines can't touch a goal's check-in.
        assert_eq!(owner("action-intent"), None);
        assert_eq!(owner("goal-x/check-in"), Some("goal-x"));
    }

    #[test]
    fn a_goal_ends_on_evidence_not_on_say_so() {
        // A reply counts only from the person, after the goal began, and never from a robot.
        let since = "2026-10-30 09:00";
        let inbox = vec![
            ("2026-10-31 08:00".to_string(), "Banana Republic <offers@banana.example>".to_string()),
            ("2026-10-31 07:00".to_string(), "Ana Silva <noreply@ana.example>".to_string()),
            ("2026-10-29 10:00".to_string(), "Ana Silva <ana@example.com>".to_string()),
        ];
        assert_eq!(reply_in(&inbox, "Ana", since), None);
        let mut arrived = inbox.clone();
        arrived.push(("2026-10-30 12:30".into(), "Ana Silva <ana@example.com>".into()));
        assert_eq!(reply_in(&arrived, "Ana", since).unwrap().0, "2026-10-30 12:30");
        assert_eq!(reply_in(&arrived, "ana@example.com", since).unwrap().0, "2026-10-30 12:30");
        assert_eq!(reply_in(&arrived, "sam@example.com", since), None);
        // The last day passing ends it; the day itself still has its check-ins.
        let mut goal = Goal {
            id: "goal-a".into(),
            aim: "Prepare".into(),
            check_in: "Help me prepare".into(),
            style: "run".into(),
            days: "every_day".into(),
            time: "09:00".into(),
            until: "2026-11-06".into(),
            done_when: "date".into(),
            person: None,
            state: "active".into(),
            created_at: now(),
            ended_at: None,
            outcome: None,
            last_task_id: None,
            note: None,
        };
        assert!(overdue(std::slice::from_ref(&goal), day(11, 6)).is_empty());
        assert_eq!(overdue(std::slice::from_ref(&goal), day(11, 7)).len(), 1);
        goal.state = "done".into();
        assert!(overdue(&[goal], day(11, 7)).is_empty(), "a finished goal doesn't end twice");
    }

    #[test]
    fn ending_pausing_and_deleting_keep_check_ins_in_step() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().into()).unwrap();
        let id = approved(&state.store.lock().unwrap(), "Follow up with Ana until she replies", "one").object_id.unwrap();
        // A check-in in progress stops when the goal ends.
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let store = state.store.lock().unwrap();
            let mut routine = store.routine(&check_in_id(&id)).unwrap().unwrap();
            routine.last_task_id = Some("check-in-run".into());
            store.save_routine(&routine).unwrap();
        }
        state.active.lock().unwrap().insert("check-in-run".into(), cancel.clone());
        let task = close(&state, &id, "done", "An email from Ana arrived.".into(), &["mail"], true).unwrap().unwrap();
        state.active.lock().unwrap().clear();
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));
        let store = state.store.lock().unwrap();
        let goal = get(&store, &id).unwrap().unwrap();
        assert_eq!((goal.state.as_str(), goal.outcome.as_deref()), ("done", Some("An email from Ana arrived.")));
        assert!(store.routine(&check_in_id(&id)).unwrap().is_none());
        // Its reason is a turn in the goal's own conversation that records reading Mail.
        assert_eq!(store.task_conversation(&task.id).unwrap(), format!("goal:{id}"));
        assert_eq!(store.task(&task.id).unwrap().unwrap().touched, vec!["mail".to_string()]);
        drop(store);
        assert!(close(&state, &id, "ended", "again".into(), &[], true).unwrap().is_none(), "an ended goal stays as it ended");
        assert!(update_in(&state, &id, "resume").is_err(), "an ended goal has no check-ins to resume");
        // Pause and resume move the check-in with the goal.
        let other = approved(&state.store.lock().unwrap(), "Help me prepare for the Lisbon trip by Friday", "two").object_id.unwrap();
        let routine = |state: &AppState| state.store.lock().unwrap().routine(&check_in_id(&other)).unwrap();
        assert_eq!(update_in(&state, &other, "pause").unwrap().state, "paused");
        assert!(!routine(&state).unwrap().enabled);
        let mut stale = routine(&state).unwrap();
        stale.next_run_at = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        state.store.lock().unwrap().save_routine(&stale).unwrap();
        assert_eq!(update_in(&state, &other, "resume").unwrap().state, "active");
        let resumed = routine(&state).unwrap();
        assert!(resumed.enabled && DateTime::parse_from_rfc3339(&resumed.next_run_at).unwrap() > Utc::now(), "never a missed check-in");
        assert!(update_in(&state, &other, "approve_everything").is_err());
        // Done is the user's word; Delete removes the goal and its check-in together.
        let third = approved(&state.store.lock().unwrap(), "Remind me to work on the tax return every evening until it's done", "three").object_id.unwrap();
        let done = update_in(&state, &third, "done").unwrap();
        assert_eq!((done.state.as_str(), done.outcome.as_deref()), ("done", Some("You marked it done.")));
        assert!(state.store.lock().unwrap().routine(&check_in_id(&third)).unwrap().is_none());
        delete_in(&state, &other).unwrap();
        assert!(get(&state.store.lock().unwrap(), &other).unwrap().is_none() && routine(&state).is_none());
        assert_eq!(state.store.lock().unwrap().routines().unwrap().len(), 1, "only the morning brief is left");
    }

    #[tokio::test]
    async fn each_check_in_reads_only_what_it_needs_and_ends_only_on_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().into()).unwrap();
        let mut settings = state.store.lock().unwrap().settings().unwrap();
        settings.apps = vec!["mail".into()];
        state.store.lock().unwrap().save_settings(&settings).unwrap();
        let today = Local::now().date_naive();
        let due = |state: &AppState, id: &str| {
            let store = state.store.lock().unwrap();
            let mut routine = store.routine(&check_in_id(id)).unwrap().unwrap();
            routine.next_run_at = (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
            store.save_routine(&routine).unwrap();
            routine
        };
        let never = |_: Goal| async { panic!("only a goal waiting on a reply reads Mail") };
        // A reminder needs no model and no reads: it is written in the goal's conversation.
        let tax = approved(&state.store.lock().unwrap(), "Remind me to work on the tax return every evening until it's done", "tax").object_id.unwrap();
        let routine = due(&state, &tax);
        let Next::Reminded { task, .. } = decide(&state, &routine, &tax, &routine.next_run_at, today, never).await.unwrap() else { panic!("a reminder") };
        assert_eq!(task.answer, "Time to work on the tax return. When it’s done, mark this goal Done in History → Goals.");
        assert!(task.touched.is_empty());
        let store = state.store.lock().unwrap();
        assert_eq!(store.task_conversation(&task.id).unwrap(), format!("goal:{tax}"));
        assert_eq!(store.conversation_list(None, 10).unwrap().items.iter().find(|item| item.id == format!("goal:{tax}")).unwrap().title, "Goal: Work on the tax return");
        let after = store.routine(&check_in_id(&tax)).unwrap().unwrap();
        assert!(DateTime::parse_from_rfc3339(&after.next_run_at).unwrap() > Utc::now(), "each time happens once");
        assert_eq!(get(&store, &tax).unwrap().unwrap().last_task_id, Some(task.id.clone()));
        drop(store);
        // Waiting on Ana: no email from her means the check-in runs, having read Mail;
        // Mail closed means it waits for the next time instead of guessing.
        let ana = approved(&state.store.lock().unwrap(), "Follow up with Ana until she replies", "ana").object_id.unwrap();
        let routine = due(&state, &ana);
        let next = decide(&state, &routine, &ana, &routine.next_run_at, today, |_| async { Ok(None) }).await.unwrap();
        assert!(matches!(next, Next::Run { touched } if touched == vec!["mail"]));
        let closed = |_: Goal| async { Err(format!("Mail {}", crate::apps::NOT_RUNNING)) };
        assert!(matches!(decide(&state, &routine, &ana, &routine.next_run_at, today, closed).await.unwrap(), Next::Skipped));
        let goal = get(&state.store.lock().unwrap(), &ana).unwrap().unwrap();
        assert!(goal.note.unwrap().starts_with("Mail wasn’t open, so I couldn’t check for a reply from Ana."));
        assert_eq!(goal.state, "active");
        // Her email ends it, with what was read recorded; the model is never asked.
        let routine = due(&state, &ana);
        let replied = |_: Goal| async { Ok(Some(("2026-11-02 09:14".to_string(), "Ana Silva <ana@example.com>".to_string()))) };
        let Next::End { status, outcome, touched } = decide(&state, &routine, &ana, &routine.next_run_at, today, replied).await.unwrap() else { panic!("the reply ends it") };
        assert_eq!((status, touched), ("done", vec!["mail"]));
        assert_eq!(outcome, "An email from Ana Silva <ana@example.com> arrived on Monday, November 2 at 9:14 AM, so this goal is done and I’ve stopped following up.");
        // Without Mail allowed, nothing is read and nothing runs.
        settings.apps.clear();
        state.store.lock().unwrap().save_settings(&settings).unwrap();
        let routine = due(&state, &ana);
        assert!(matches!(decide(&state, &routine, &ana, &routine.next_run_at, today, never).await.unwrap(), Next::Skipped));
        // After the last day, the goal ends whatever the check-in would do.
        let routine = due(&state, &tax);
        let late = today + chrono::Duration::days(DONE_DAYS + 1);
        let Next::End { status, outcome, .. } = decide(&state, &routine, &tax, &routine.next_run_at, late, never).await.unwrap() else { panic!("the date ends it") };
        assert_eq!(status, "ended");
        assert!(outcome.ends_with("has passed, so I’ve stopped checking in."), "{outcome}");
    }

    #[test]
    fn a_goal_request_becomes_one_pinned_write() {
        let prompt = "Follow up with Ana until she replies";
        let mut contract = crate::task_contract::TaskContract::from_request(prompt);
        contract.refresh(prompt, &[]);
        let outcome = contract.next_outcome().unwrap();
        assert_eq!(outcome.accepted_tools, vec![TOOL.to_string()]);
        let arguments = parse_request(prompt).unwrap().arguments();
        assert_eq!(outcome.required_arguments, arguments);
        assert!(contract.validate_proposal(TOOL, &arguments, &[]).is_ok());
        // The model can't change who ends it or push the last day out.
        let mut moved = arguments.clone();
        moved["person"] = json!("Sam");
        assert!(contract.validate_proposal(TOOL, &moved, &[]).is_err());
        assert_eq!(crate::completion::requested_actions(prompt), (vec![TOOL], false));
        assert_eq!(crate::completion::requested_actions("Remind me to work on the tax return every evening until it's done"), (vec![TOOL], false));
    }
}
