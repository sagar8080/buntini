//! Which existing item a change is for: the reminder to mark done, the event to move or
//! rename, the note to add to. Like the Contacts recipient in `source_fields`, the item
//! comes only from a typed read in the same task journal, matched against the words the
//! user wrote. One clear match is bound, several are offered in the approval sheet, and
//! none is a question back to the user. Model text, mail, notes and pages never supply
//! an item's identifier or decide which one it is.
use crate::{actions::Field, workflows::Step};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use serde_json::{json, Map, Value};

/// At most this many matches are offered to pick from.
const CHOICES: usize = 8;
/// How many days ahead Buntini looks for an event the user named without a day.
const SEARCH_DAYS: i64 = 7;

/// Words that say what kind of item, or join a phrase, rather than which one.
const GENERIC: &[&str] = &[
    "the",
    "my",
    "a",
    "an",
    "our",
    "reminder",
    "reminders",
    "task",
    "tasks",
    "todo",
    "to-do",
    "item",
    "one",
    "event",
    "events",
    "meeting",
    "meetings",
    "appointment",
    "appointments",
    "call",
    "calendar",
    "with",
    "at",
    "on",
    "for",
    "of",
    "in",
    "as",
    "called",
    "named",
    "about",
    "note",
    "notes",
    "s",
    "to",
    "today",
    "tonight",
    "tomorrow",
    "this",
    "next",
    "back",
    "forward",
];

/// Words that point at something else instead of saying what to add: "add that to …".
const POINTERS: &[&str] = &["this", "that", "it", "these", "those", "them", "everything"];

/// Lowercase words, keeping ':' and '-' inside them for "3:30pm" and "2026-11-03".
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace(['’', '‘'], "'")
        .split(|c: char| !(c.is_alphanumeric() || c == ':' || c == '-'))
        .map(|word| word.trim_matches(['-', ':']))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A time of day the user wrote. `exact` is false for "4" or "10:30", which could be
/// morning or afternoon.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clock {
    hour: u32,
    minute: u32,
    exact: bool,
}

/// A time at `words[index]`, with how many words it used: "4", "4pm", "4 pm", "4:30pm",
/// "16:00", "noon".
fn clock(words: &[String], index: usize) -> Option<(Clock, usize)> {
    let word = words.get(index)?.as_str();
    match word {
        "noon" | "midday" => {
            return Some((
                Clock {
                    hour: 12,
                    minute: 0,
                    exact: true,
                },
                1,
            ))
        }
        "midnight" => {
            return Some((
                Clock {
                    hour: 0,
                    minute: 0,
                    exact: true,
                },
                1,
            ))
        }
        _ => {}
    }
    let (digits, mut half) = match word.strip_suffix("pm").or_else(|| word.strip_suffix("am")) {
        Some(digits) => (digits, Some(word.ends_with("pm"))),
        None => (word, None),
    };
    let mut used = 1;
    if half.is_none() {
        if let Some(next) = words
            .get(index + 1)
            .filter(|next| ["pm", "am"].contains(&next.as_str()))
        {
            half = Some(next == "pm");
            used = 2;
        }
    }
    let (hour, minute) = match digits.split_once(':') {
        Some((hour, minute)) if minute.len() == 2 => {
            (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?)
        }
        Some(_) => return None,
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if digits.is_empty() || digits.len() > 5 || minute > 59 {
        return None;
    }
    let clock = match half {
        Some(pm) if (1..=12).contains(&hour) => Clock {
            hour: hour % 12 + if pm { 12 } else { 0 },
            minute,
            exact: true,
        },
        Some(_) => return None,
        None if hour <= 23 => Clock {
            hour,
            minute,
            exact: hour == 0 || hour >= 13,
        },
        None => return None,
    };
    Some((clock, used))
}

/// Whether a clock the user wrote names `hour:minute`: "3" or "3pm" is 15:00, "3" is 03:00.
fn names_time(clock: Clock, hour: u32, minute: u32) -> bool {
    minute == clock.minute && (hour == clock.hour || (!clock.exact && hour == clock.hour + 12))
}

/// The hour for a new time, reading "4" as the one nearer the event's own hour: moving a
/// 3pm event "to 4" means 4pm.
fn resolve(clock: Clock, near: u32) -> u32 {
    if clock.exact || clock.hour > 12 || clock.hour == 0 {
        return clock.hour;
    }
    let morning = clock.hour % 12;
    let afternoon = morning + 12;
    if near.abs_diff(afternoon) <= near.abs_diff(morning) {
        afternoon
    } else {
        morning
    }
}

/// A day the user named, relative to `today`. A weekday is the next one after today.
fn day(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    let weekday = match word {
        "today" | "tonight" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        "monday" | "mon" => chrono::Weekday::Mon,
        "tuesday" | "tue" | "tues" => chrono::Weekday::Tue,
        "wednesday" | "wed" => chrono::Weekday::Wed,
        "thursday" | "thu" | "thur" | "thurs" => chrono::Weekday::Thu,
        "friday" | "fri" => chrono::Weekday::Fri,
        "saturday" | "sat" => chrono::Weekday::Sat,
        "sunday" | "sun" => chrono::Weekday::Sun,
        _ => return NaiveDate::parse_from_str(word, "%Y-%m-%d").ok(),
    };
    (1..=7)
        .map(|ahead| today + chrono::Duration::days(ahead))
        .find(|date| date.weekday() == weekday)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Ordinal {
    Nth(usize),
    Last,
    Next,
}

fn ordinal(word: &str) -> Option<Ordinal> {
    Some(match word {
        "first" | "1st" => Ordinal::Nth(1),
        "second" | "2nd" => Ordinal::Nth(2),
        "third" | "3rd" => Ordinal::Nth(3),
        "fourth" | "4th" => Ordinal::Nth(4),
        "fifth" | "5th" => Ordinal::Nth(5),
        "last" => Ordinal::Last,
        "next" => Ordinal::Next,
        _ => return None,
    })
}

/// Which item the user named: words of its title, or its place in the list ("the first
/// one"), or its time and day ("my 3pm tomorrow").
#[derive(Debug, Clone, Default, PartialEq)]
struct Target {
    phrase: String,
    words: Vec<String>,
    ordinal: Option<Ordinal>,
    clock: Option<Clock>,
    day: Option<NaiveDate>,
}

fn target(text: &str, today: NaiveDate) -> Target {
    let all = words(text);
    let mut found = Target {
        phrase: text.trim().to_string(),
        ..Target::default()
    };
    let mut index = 0;
    while index < all.len() {
        let word = all[index].as_str();
        if let Some((clock, used)) = clock(&all, index) {
            found.clock = Some(clock);
            index += used;
            continue;
        }
        // "next week's" is a day, "the next one" an order; only the second is supported.
        if let Some(order) = ordinal(word).filter(|_| {
            !(word == "next" && all.get(index + 1).is_some_and(|w| day(w, today).is_some()))
        }) {
            found.ordinal = Some(order);
        } else if let Some(date) = day(word, today) {
            found.day = Some(date);
        } else if !GENERIC.contains(&word) {
            found.words.push(word.to_string());
        }
        index += 1;
    }
    found
}

/// The part of the request that asks for `tool`, in the user's own case.
pub(crate) fn clause(goal: &str, tool: &str) -> Option<String> {
    let goal = goal.replace("Additional details: ", "\n");
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = goal.char_indices().collect();
    for (position, (index, ch)) in chars.iter().copied().enumerate() {
        let ends = matches!(ch, ';' | '\n' | '!' | '?')
            || (ch == '.'
                && chars
                    .get(position + 1)
                    .is_none_or(|(_, next)| next.is_whitespace()));
        if ends {
            sentences.push(&goal[start..index]);
            start = index + ch.len_utf8();
        }
    }
    sentences.push(&goal[start..]);
    let asks = |text: &str| crate::completion::requested_actions(text).0.contains(&tool);
    for sentence in sentences {
        if !asks(sentence) {
            continue;
        }
        // A list being added to a note keeps its "and"s and commas.
        if tool == "append_to_note" {
            return Some(sentence.trim().to_string());
        }
        let lower = sentence.to_ascii_lowercase();
        let mut cuts = vec![0];
        for separator in [" and then ", " then ", " and ", ", "] {
            cuts.extend(lower.match_indices(separator).map(|(at, _)| at));
        }
        cuts.sort_unstable();
        cuts.dedup();
        cuts.push(sentence.len());
        for pair in cuts.windows(2) {
            let part = sentence[pair[0]..pair[1]]
                .trim_start_matches([',', ' '])
                .trim_start_matches("and then ")
                .trim_start_matches("then ")
                .trim_start_matches("and ");
            if asks(part) {
                return Some(part.trim().to_string());
            }
        }
        return Some(sentence.trim().to_string());
    }
    None
}

/// The request with polite openings removed, in the user's own case.
fn instruction(text: &str) -> &str {
    let lower = text.to_ascii_lowercase();
    let rest = crate::completion::request_clause(lower.trim());
    &text.trim()[lower.trim().len() - rest.len()..]
}

/// A user's text without one pair of quotation marks around all of it.
fn unquote(text: &str) -> (&str, bool) {
    let text = text.trim();
    for (open, close) in [('"', '"'), ('“', '”'), ('\'', '\''), ('‘', '’')] {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            return (inner.trim(), true);
        }
    }
    (text, false)
}

/// What the user asked to change, from their own words.
#[derive(Debug, Clone, PartialEq)]
enum Request {
    Complete(Target),
    Event { target: Target, change: EventChange },
    Append { note: String, text: Option<String> },
}

#[derive(Debug, Clone, Default, PartialEq)]
struct EventChange {
    day: Option<NaiveDate>,
    clock: Option<Clock>,
    title: Option<String>,
    location: Option<String>,
}

fn request(goal: &str, tool: &str, today: NaiveDate) -> Result<Request, String> {
    let part = clause(goal, tool).ok_or("Say which item to change.")?;
    let text = instruction(&part);
    match tool {
        "complete_reminder" => Ok(Request::Complete(complete_target(text, today))),
        "update_calendar_event" => event_request(text, today),
        _ => append_request(text)
            .ok_or_else(|| "Say which note to add to, like “add milk to my shopping note”.".into()),
    }
}

/// "mark X done", "mark X as complete", "tick off X", "check X off", "complete X".
fn complete_target(text: &str, today: NaiveDate) -> Target {
    let all = words(text);
    let rest: Vec<String> = match all.first().map(String::as_str) {
        Some("mark") => {
            let end = all
                .iter()
                .rposition(|w| ["done", "complete", "completed", "finished"].contains(&w.as_str()))
                .unwrap_or(all.len());
            all[1..end.max(1)].to_vec()
        }
        Some("tick" | "check" | "cross") if all.get(1).is_some_and(|w| w == "off") => {
            all[2..].to_vec()
        }
        Some("tick" | "check" | "cross") if all.last().is_some_and(|w| w == "off") => {
            all[1..all.len() - 1].to_vec()
        }
        Some(_) => all[1..].to_vec(),
        None => vec![],
    };
    let mut found = target(&rest.join(" "), today);
    if found.words.iter().all(|w| POINTERS.contains(&w.as_str())) {
        found.words.clear();
        found.phrase.clear();
    }
    found
}

/// "move my 3pm to 4", "push the dentist to Friday", "rename my 10:30 to Planning",
/// "change the location of lunch to Café Rio".
fn event_request(text: &str, today: NaiveDate) -> Result<Request, String> {
    let lower = text.to_ascii_lowercase();
    let verb_end = lower
        .find(' ')
        .ok_or("Say which event to change and how.")?;
    let verb = &lower[..verb_end];
    // A new title or place may itself say "to" ("rename my 3pm to Talk to Sam"): it starts
    // at a quote after "to", or else at the first "to" after the event. A new time is
    // after the last one ("move my trip to Paris to 4").
    let first_to = lower[verb_end..].find(" to ").map(|at| at + verb_end);
    let names_field = first_to.is_some_and(|at| {
        words(&lower[verb_end..at]).iter().any(|w| {
            ["title", "name", "location", "place", "venue", "where"].contains(&w.as_str())
        })
    });
    let quoted = [" to \"", " to “", " to '", " to ‘"]
        .iter()
        .filter_map(|opening| lower.find(opening))
        .min();
    let split = if verb == "rename" || (verb == "change" && names_field) {
        quoted.or(first_to)
    } else {
        lower.rfind(" to ")
    }
    .map(|at| (at, 4))
        .or_else(|| {
            ["reschedule", "postpone"]
                .contains(&verb)
                .then(|| lower.rfind(" for ").map(|at| (at, 5)))
                .flatten()
        })
        .filter(|(at, _)| *at > verb_end)
        .ok_or("Say what to change it to, like “move my 3pm to 4”.")?;
    let named = &lower[verb_end..split.0];
    let value = unquote(
        text[split.0 + split.1..]
            .trim_end_matches(['.', '!', '?', ',', ' '])
            .trim(),
    )
    .0
    .to_string();
    if value.is_empty() {
        return Err("Say what to change it to, like “move my 3pm to 4”.".into());
    }
    let mut change = EventChange::default();
    let mentions = |names: &[&str]| words(named).iter().any(|w| names.contains(&w.as_str()));
    let mut named = named.to_string();
    if verb == "rename" || (verb == "change" && mentions(&["title", "name"])) {
        change.title = Some(value);
        for strip in [
            "the title of ",
            "title of ",
            "the name of ",
            "name of ",
            "'s title",
            "'s name",
        ] {
            named = named.replace(strip, " ");
        }
    } else if verb == "change" && mentions(&["location", "place", "venue", "where"]) {
        change.location = Some(value);
        for strip in [
            "the location of ",
            "location of ",
            "the place of ",
            "the venue of ",
            "'s location",
            "where ",
        ] {
            named = named.replace(strip, " ");
        }
    } else {
        let all = words(&value);
        let mut index = 0;
        while index < all.len() {
            if let Some((clock, used)) = clock(&all, index) {
                change.clock = Some(clock);
                index += used;
                continue;
            }
            let word = all[index].as_str();
            if let Some(date) = day(word, today) {
                change.day = Some(date);
            } else if !["at", "on", "the", "next", "this", "o", "clock"].contains(&word) {
                return Err("I can move an event to a new day or time, like “Friday at 4”. To change where it is, say “change the location of … to …”.".into());
            }
            index += 1;
        }
        if change.day.is_none() && change.clock.is_none() {
            return Err("Say when to move it to, like “Friday at 4”.".into());
        }
    }
    let target = target(&named, today);
    Ok(Request::Event { target, change })
}

/// "add milk and eggs to my shopping note": the text and the note's name. The text is
/// the user's own; "add that to my note" points elsewhere and needs their words instead.
fn append_request(text: &str) -> Option<Request> {
    let lower = text.to_ascii_lowercase();
    let spans: Vec<(usize, usize)> = lower
        .split_whitespace()
        .map(|word| {
            let start = word.as_ptr() as usize - lower.as_ptr() as usize;
            (start, start + word.len())
        })
        .collect();
    let bare = |index: usize| {
        lower[spans[index].0..spans[index].1].trim_matches(|c: char| !c.is_alphanumeric())
    };
    for end in 0..spans.len() {
        if !["note", "notes"].contains(&bare(end)) {
            continue;
        }
        let Some(to) = (0..end).rev().take(6).find(|&index| bare(index) == "to") else {
            continue;
        };
        let name: Vec<&str> = (to + 1..end)
            .map(bare)
            .skip_while(|word| ["my", "the", "our"].contains(word))
            .collect();
        if name.is_empty() || name.len() > 4 || name.iter().any(|word| word.is_empty()) {
            continue;
        }
        let Some(verb) = (0..to)
            .rev()
            .find(|&index| ["add", "append", "put", "jot"].contains(&bare(index)))
        else {
            continue;
        };
        let raw = text[spans[verb].1..spans[to].0].trim();
        let raw = raw.strip_prefix("down ").unwrap_or(raw);
        let (added, quoted) = unquote(raw);
        let pointer = words(added)
            .first()
            .is_some_and(|word| POINTERS.contains(&word.as_str()));
        let text = (!added.is_empty() && (quoted || !pointer)).then(|| added.to_string());
        return Some(Request::Append {
            note: name.join(" "),
            text,
        });
    }
    None
}

/// One item that matches, with the change's arguments for it and how the sheet shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub step_id: String,
    pub record_id: String,
    pub arguments: Value,
    pub label: String,
    pub now: String,
}

/// What to do next for a requested change.
#[derive(Debug, Clone, PartialEq)]
pub enum EditPlan {
    /// Read this first: the item can only come from a read in this task.
    Read {
        tool: &'static str,
        arguments: Value,
    },
    One(Candidate),
    /// The user picks one in the approval sheet.
    Several(Vec<Candidate>),
}

/// The latest verified read by `tool` whose arguments satisfy `wanted`.
fn latest_read<'a>(
    steps: &'a [Step],
    tool: &str,
    wanted: impl Fn(&Value) -> bool,
) -> Option<&'a Step> {
    steps.iter().rev().find(|step| {
        step.tool == tool
            && !step.effect
            && step.state == "succeeded"
            && wanted(&step.arguments)
            && step.evidence.as_ref().is_some_and(|evidence| {
                crate::apps::validate_evidence(tool, &step.arguments, evidence, false).is_ok()
            })
    })
}

fn records(step: &Step) -> Vec<Value> {
    step.evidence
        .as_ref()
        .and_then(|e| e["records"].as_array())
        .into_iter()
        .flatten()
        .filter(|record| record["id"].as_str().is_some_and(|id| !id.is_empty()))
        .cloned()
        .collect()
}

fn field<'a>(record: &'a Value, key: &str) -> &'a str {
    record[key].as_str().unwrap_or("")
}

/// "2026-11-03 15:00" as "Tue 3 Nov 15:00", and a bare date as "Tue 3 Nov".
fn pretty(stamp: &str) -> String {
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M")
        .map(|at| at.format("%a %-d %b %H:%M").to_string())
        .or_else(|_| {
            NaiveDate::parse_from_str(stamp, "%Y-%m-%d")
                .map(|day| day.format("%a %-d %b").to_string())
        })
        .unwrap_or_else(|_| stamp.to_string())
}

/// Whether every word the user wrote is a word of the title.
fn titled(record: &Value, wanted: &[String]) -> bool {
    let have = words(field(record, "title"));
    wanted.iter().all(|word| have.contains(word))
}

/// Several matches narrow to one when its title is exactly what the user wrote.
fn narrowed(matches: Vec<Candidate>, records: &[Value], phrase: &[String]) -> Vec<Candidate> {
    let exact: Vec<Candidate> = matches
        .iter()
        .filter(|c| {
            records
                .iter()
                .any(|r| field(r, "id") == c.record_id && words(field(r, "title")) == phrase)
        })
        .cloned()
        .collect();
    if exact.len() == 1 {
        exact
    } else {
        matches
    }
}

/// One match is bound only when the read saw every item it looked for. From a partial
/// read (Notes returns 6 matches, Reminders 50, Calendar 40 a day) another match may be
/// one Buntini didn't see, so the one it found waits for the user to pick it.
fn decide(mut matches: Vec<Candidate>, complete: bool) -> EditPlan {
    if matches.len() == 1 && complete {
        EditPlan::One(matches.remove(0))
    } else {
        if !complete {
            for candidate in &mut matches {
                candidate.label += " · others may match";
            }
        }
        EditPlan::Several(matches.into_iter().take(CHOICES).collect())
    }
}

fn read_complete(step: &Step) -> bool {
    step.evidence.as_ref().is_some_and(|e| e["complete"] == true)
}

fn pick<'a>(
    records: &'a [Value],
    order: Ordinal,
    now: NaiveDateTime,
    complete: bool,
) -> Option<&'a Value> {
    match order {
        Ordinal::Nth(n) => records.get(n - 1),
        Ordinal::Last if complete => records.last(),
        Ordinal::Last => None,
        Ordinal::Next => records.iter().find(|record| {
            NaiveDateTime::parse_from_str(field(record, "startLocal"), "%Y-%m-%d %H:%M")
                .is_ok_and(|at| at > now)
        }),
    }
}

/// The words one change is for: the clause its outcome was made from (`description`,
/// with or without the read's prefix) when that clause asks for `tool`, so the second of
/// two edits of the same kind binds its own item; otherwise the whole request.
pub fn outcome_goal<'a>(description: &'a str, goal: &'a str, tool: &str) -> &'a str {
    let own = description
        .strip_prefix(crate::task_contract::EDIT_READ)
        .unwrap_or(description);
    if clause(own, tool).is_some() {
        own
    } else {
        goal
    }
}

/// What to do next for the change the user asked for with `tool`, from their words and
/// the reads already in this task.
pub fn edit_plan(goal: &str, tool: &str, steps: &[Step]) -> Result<EditPlan, String> {
    edit_plan_at(goal, tool, steps, chrono::Local::now().naive_local())
}

pub(crate) fn edit_plan_at(
    goal: &str,
    tool: &str,
    steps: &[Step],
    now: NaiveDateTime,
) -> Result<EditPlan, String> {
    let today = now.date();
    match request(goal, tool, today)? {
        Request::Complete(target) => reminder_plan(&target, steps, now),
        Request::Event { target, change } => event_plan(&target, &change, steps, now),
        Request::Append { note, text } => note_plan(&note, text, steps),
    }
}

fn reminder_plan(target: &Target, steps: &[Step], now: NaiveDateTime) -> Result<EditPlan, String> {
    if target.words.is_empty() && target.ordinal.is_none() {
        return Err("Say which reminder to mark done.".into());
    }
    let Some(step) = latest_read(steps, "read_reminders", |_| true) else {
        return Ok(EditPlan::Read {
            tool: "read_reminders",
            arguments: json!({}),
        });
    };
    let complete = step
        .evidence
        .as_ref()
        .is_some_and(|e| e["complete"] == true);
    let all = records(step);
    let chosen: Vec<&Value> = match target.ordinal.filter(|_| target.words.is_empty()) {
        Some(order) => pick(&all, order, now, complete).into_iter().collect(),
        None => all
            .iter()
            .filter(|record| titled(record, &target.words))
            .collect(),
    };
    let candidates: Vec<Candidate> = chosen
        .into_iter()
        .map(|record| {
            let title = field(record, "title").trim();
            let mut label = title.to_string();
            if !field(record, "dueLocal").is_empty() {
                label += &format!(" · due {}", pretty(field(record, "dueLocal")));
            }
            if !field(record, "list").is_empty() {
                label += &format!(" · {}", field(record, "list"));
            }
            Candidate {
                step_id: step.id.clone(),
                record_id: field(record, "id").into(),
                arguments: json!({"id":field(record, "id"), "title":title}),
                now: format!("{label} · not done yet"),
                label,
            }
        })
        .collect();
    if candidates.is_empty() {
        let named = if target.words.is_empty() {
            "that one".to_string()
        } else {
            format!("“{}”", target.words.join(" "))
        };
        return Err(if complete {
            format!("I couldn’t find an open reminder matching {named}. Say its name as it is in Reminders.")
        } else {
            format!("I couldn’t find an open reminder matching {named} among the ones I read. Say its name as it is in Reminders.")
        });
    }
    // "The first one" is the first of what was read, however many there are.
    let ordered = target.ordinal.is_some() && target.words.is_empty();
    Ok(decide(narrowed(candidates, &all, &target.words), complete || ordered))
}

fn event_plan(
    target: &Target,
    change: &EventChange,
    steps: &[Step],
    now: NaiveDateTime,
) -> Result<EditPlan, String> {
    let today = now.date();
    // A day the user named is the only day read. A time or "my next meeting" means
    // today; an event named by its title is looked for over the coming week.
    let days: Vec<NaiveDate> = match target.day {
        Some(day) => vec![day],
        None if target.words.is_empty() => vec![today],
        None => (0..SEARCH_DAYS)
            .map(|ahead| today + chrono::Duration::days(ahead))
            .collect(),
    };
    for day in &days {
        let stamp = day.format("%Y-%m-%d").to_string();
        let Some(step) = latest_read(steps, "read_calendar", |args| args["day"] == stamp.as_str())
        else {
            return Ok(EditPlan::Read {
                tool: "read_calendar",
                arguments: json!({"day":stamp}),
            });
        };
        let complete = step
            .evidence
            .as_ref()
            .is_some_and(|e| e["complete"] == true);
        let records = records(step);
        let matching: Vec<&Value> = records
            .iter()
            .filter(|record| titled(record, &target.words))
            .filter(|record| {
                target.clock.is_none_or(|clock| {
                    NaiveDateTime::parse_from_str(field(record, "startLocal"), "%Y-%m-%d %H:%M")
                        .is_ok_and(|at| {
                            record["all_day"] != true && names_time(clock, at.hour(), at.minute())
                        })
                })
            })
            .collect();
        // An all-day event the user named is the one they meant, not one on a later day.
        if !matching.is_empty() && matching.iter().all(|record| record["all_day"] == true) {
            return Err(
                "That’s an all-day event, which Buntini can’t move yet. Change it in Calendar."
                    .into(),
            );
        }
        let timed: Vec<Value> = matching
            .into_iter()
            .filter(|record| record["all_day"] != true)
            .cloned()
            .collect();
        let chosen: Vec<&Value> = match target.ordinal {
            Some(order) => pick(&timed, order, now, complete).into_iter().collect(),
            None => timed.iter().collect(),
        };
        if chosen.is_empty() {
            continue;
        }
        let mut candidates = Vec::new();
        for record in chosen {
            candidates.push(event_candidate(step, record, change)?);
        }
        let plan = decide(narrowed(candidates, &timed, &target.words), complete || target.ordinal.is_some());
        if let EditPlan::One(candidate) = &plan {
            let args = &candidate.arguments;
            if args["start"] == args["occurrence"]
                && args["title"] == args["was_title"]
                && change.location.is_none()
            {
                return Err("That event already has that time and title.".into());
            }
        }
        return Ok(plan);
    }
    let named = if target.words.is_empty() {
        target.phrase.trim().to_string()
    } else {
        target.words.join(" ")
    };
    Err(match target.day {
        Some(day) => format!("I couldn’t find “{named}” on your calendar for {}. Say its name or time as it is in Calendar.", day.format("%A %-d %B")),
        None if target.words.is_empty() => format!("I couldn’t find “{named}” on today’s calendar. Say which day it’s on."),
        None => format!("I couldn’t find “{named}” on your calendar this coming week. Say which day it’s on."),
    })
}

fn event_candidate(step: &Step, record: &Value, change: &EventChange) -> Result<Candidate, String> {
    let parse =
        |key: &str| NaiveDateTime::parse_from_str(field(record, key), "%Y-%m-%d %H:%M").ok();
    let (Some(start), Some(end)) = (parse("startLocal"), parse("endLocal")) else {
        return Err("Your calendar didn’t say when that event is. Read it again with the current Apple helper.".into());
    };
    let minutes = (end - start).num_minutes();
    if !(5..=1440).contains(&minutes) {
        return Err(
            "That event is longer than a day, which Buntini can’t move yet. Change it in Calendar."
                .into(),
        );
    }
    let date = change.day.unwrap_or(start.date());
    let (hour, minute) = change
        .clock
        .map(|clock| (resolve(clock, start.hour()), clock.minute))
        .unwrap_or((start.hour(), start.minute()));
    let moved = date
        .and_hms_opt(hour, minute, 0)
        .ok_or("That isn’t a time of day.")?;
    let was = field(record, "title").trim();
    let title = change.title.clone().unwrap_or_else(|| was.to_string());
    let location = change
        .location
        .clone()
        .unwrap_or_else(|| field(record, "location").trim().to_string());
    let mut arguments = json!({"id":field(record, "id"), "occurrence":start.format("%Y-%m-%d %H:%M").to_string(),
        "was_title":was, "title":title, "start":moved.format("%Y-%m-%d %H:%M").to_string(), "minutes":minutes});
    if !location.is_empty() {
        arguments["location"] = json!(location);
    }
    let label = format!(
        "{} · {}–{}",
        field(record, "title"),
        pretty(field(record, "startLocal")),
        end.format("%H:%M")
    );
    let mut now = label.clone();
    if !field(record, "location").is_empty() {
        now += &format!(" · {}", field(record, "location"));
    }
    if record["recurring"] == true {
        now += " · repeats";
    }
    Ok(Candidate {
        step_id: step.id.clone(),
        record_id: field(record, "id").into(),
        arguments,
        label,
        now,
    })
}

fn note_plan(note: &str, text: Option<String>, steps: &[Step]) -> Result<EditPlan, String> {
    let text = text.ok_or("Tell me exactly what to add to the note, in your own words.")?;
    let Some(step) = latest_read(steps, "find_notes", |args| {
        args["query"]
            .as_str()
            .is_some_and(|q| q.eq_ignore_ascii_case(note))
    }) else {
        return Ok(EditPlan::Read {
            tool: "find_notes",
            arguments: json!({"query":note}),
        });
    };
    let wanted = words(note);
    let all = records(step);
    // A note mentioning the word in its text is not the note the user named.
    let candidates: Vec<Candidate> = all
        .iter()
        .filter(|record| titled(record, &wanted))
        .map(|record| {
            let title = field(record, "title").trim();
            let label = format!(
                "{title} · edited {}",
                pretty(field(record, "modifiedLocal"))
            );
            // The excerpt is the note's opening on one line, starting with its title.
            let excerpt = field(record, "excerpt").trim();
            let body = excerpt.strip_prefix(title).unwrap_or(excerpt).trim();
            let now = if body.is_empty() {
                label.clone()
            } else if body.chars().count() > 80 || record["contentComplete"] != true {
                format!(
                    "{title} · {}…",
                    body.chars().take(80).collect::<String>().trim_end()
                )
            } else {
                format!("{title} · {body}")
            };
            Candidate {
                step_id: step.id.clone(),
                record_id: field(record, "id").into(),
                arguments: json!({"id":field(record, "id"), "title":title, "text":text}),
                label,
                now,
            }
        })
        .collect();
    if candidates.is_empty() {
        return Err(format!(
            "I couldn’t find a note called “{note}”. Say its title as it is in Notes."
        ));
    }
    Ok(decide(narrowed(candidates, &all, &wanted), read_complete(step)))
}

/// The change's arguments when the user still has to pick: the first match's, without
/// its id, so nothing runs until they choose.
pub fn unpicked(candidates: &[Candidate]) -> Value {
    let mut arguments = candidates[0].arguments.clone();
    if let Some(fields) = arguments.as_object_mut() {
        fields.remove("id");
    }
    arguments
}

/// What the sheet adds for a change: what the item is like now, and the matches to pick
/// from when there are several.
/// `goals` are the words of each change of this kind the request asks for
/// (`TaskContract::edit_goals`); the sheet is for the one these arguments came from.
pub fn sheet(
    goals: &[String],
    tool: &str,
    arguments: &Value,
    steps: &[Step],
) -> (Option<Field>, Vec<crate::actions::Choice>) {
    sheet_at(
        goals,
        tool,
        arguments,
        steps,
        chrono::Local::now().naive_local(),
    )
}

fn sheet_at(
    goals: &[String],
    tool: &str,
    arguments: &Value,
    steps: &[Step],
    at: NaiveDateTime,
) -> (Option<Field>, Vec<crate::actions::Choice>) {
    if !crate::actions::EDIT_TOOLS.contains(&tool) {
        return (None, vec![]);
    }
    let now = |value: String| Field {
        key: "now".into(),
        label: "Now".into(),
        value,
        kind: "fixed",
    };
    let mut unpicked_arguments = arguments.clone();
    if let Some(fields) = unpicked_arguments.as_object_mut() {
        fields.remove("id");
    }
    let plan = goals.iter().find_map(|goal| match edit_plan_at(goal, tool, steps, at) {
        Ok(EditPlan::One(candidate)) if candidate.arguments["id"] == arguments["id"] => Some(EditPlan::One(candidate)),
        Ok(EditPlan::Several(candidates))
            if candidates.iter().any(|candidate| candidate.arguments["id"] == arguments["id"])
                || unpicked(&candidates) == unpicked_arguments =>
        {
            Some(EditPlan::Several(candidates))
        }
        _ => None,
    });
    match plan {
        Some(EditPlan::One(candidate)) => (Some(now(candidate.now)), vec![]),
        Some(EditPlan::Several(candidates)) => {
            // Every pick sets every field any match has, blank when its item has none, so
            // the first match's location never stays on another event.
            let keys: std::collections::BTreeSet<String> = candidates
                .iter()
                .flat_map(|candidate| candidate.arguments.as_object().into_iter().flatten())
                .map(|(key, _)| key.clone())
                .filter(|key| key != "id")
                .collect();
            let choices = candidates
                .into_iter()
                .map(|candidate| {
                    let mut also: std::collections::BTreeMap<String, String> = candidate
                        .arguments
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(key, _)| *key != "id")
                        .map(|(key, value)| {
                            (
                                key.clone(),
                                value
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| value.to_string()),
                            )
                        })
                        .collect();
                    for key in &keys {
                        also.entry(key.clone()).or_default();
                    }
                    also.insert("now".into(), candidate.now);
                    crate::actions::Choice {
                        field: "id".into(),
                        label: candidate.label,
                        value: candidate.record_id,
                        also,
                    }
                })
                .collect();
            (Some(now("Pick one below.".into())), choices)
        }
        _ => (None, vec![]),
    }
}

/// Fields that say which item a change is for. They must be exactly what was read; the
/// rest (a new time, the text to add) is the user's to edit in the sheet.
fn identity(tool: &str) -> &'static [&'static str] {
    match tool {
        "update_calendar_event" => &["id", "occurrence", "was_title"],
        _ => &["id", "title"],
    }
}

/// Whether a proposed or approved change is for an item this task read for what the
/// user named. Before approval the id may still be missing while the user picks one;
/// `approving` requires it.
pub fn check(
    goals: &[String],
    tool: &str,
    arguments: &Value,
    steps: &[Step],
    approving: bool,
) -> Result<(), String> {
    if !crate::actions::EDIT_TOOLS.contains(&tool) {
        return Ok(());
    }
    // Each change of this kind the user asked for has its own item; a change may be for
    // any of them, and only for an item read for one of them.
    let mut candidates = Vec::new();
    let mut several = false;
    let mut first_error = None;
    let mut unread = false;
    for goal in goals {
        match edit_plan(goal, tool, steps) {
            Ok(EditPlan::Read { .. }) => unread = true,
            Ok(EditPlan::One(candidate)) => candidates.push(candidate),
            Ok(EditPlan::Several(found)) => {
                several = true;
                candidates.extend(found);
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if candidates.is_empty() {
        return Err(match first_error {
            Some(error) if !unread => error,
            _ => "Buntini hasn’t read that item in this task yet.".into(),
        });
    }
    if arguments.get("id").is_none() {
        return if approving || !several {
            Err("Pick which one to change first.".into())
        } else {
            Ok(())
        };
    }
    let same = |candidate: &Candidate| {
        identity(tool)
            .iter()
            .all(|key| arguments.get(*key) == candidate.arguments.get(*key))
    };
    if candidates.iter().any(same) {
        Ok(())
    } else {
        Err("Buntini changes only an item it read for this request. Ask again so it reads it afresh.".into())
    }
}

/// Where the item for a requested change is read from, for the task contract: open
/// reminders, the day the user named (any day, when they named none), or notes by name.
pub fn source(tool: &str, clause: &str, today: NaiveDate) -> Option<(&'static str, Value)> {
    match tool {
        "complete_reminder" => Some(("read_reminders", json!({}))),
        "update_calendar_event" => {
            let named = match event_request(instruction(clause), today) {
                Ok(Request::Event { target, .. }) => target.day,
                _ => None,
            };
            Some((
                "read_calendar",
                named
                    .map(|day| json!({"day":day.format("%Y-%m-%d").to_string()}))
                    .unwrap_or_else(|| json!({})),
            ))
        }
        "append_to_note" => match append_request(instruction(clause)) {
            Some(Request::Append { note, .. }) => Some(("find_notes", json!({"query":note}))),
            _ => None,
        },
        _ => None,
    }
}

/// The provenance of a bound change: the read and record it came from. The text added to
/// a note is the user's own words; every other value comes from that record, changed
/// only as the user's words said.
pub fn provenance(tool: &str, candidate: &Candidate) -> Value {
    let user: Map<String, Value> = candidate
        .arguments
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| tool == "append_to_note" && *key == "text")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    json!({"kind":"source_record", "stepId":candidate.step_id, "recordId":candidate.record_id, "userFields":user})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{Agenda, Event, Reminder};

    /// Thursday 29 October 2026, 08:00.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 29)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap()
    }

    fn read(id: &str, tool: &str, arguments: Value, evidence: Value) -> Step {
        Step {
            id: id.into(),
            tool: tool.into(),
            arguments,
            state: "succeeded".into(),
            effect: false,
            proposed_at: String::new(),
            approved_at: None,
            finished_at: None,
            evidence: Some(evidence),
        }
    }

    fn reminder(id: &str, title: &str, due: &str) -> Reminder {
        Reminder {
            id: Some(id.into()),
            title: title.into(),
            due: due.into(),
            list: "Home".into(),
            ..Reminder::default()
        }
    }

    fn reminders(items: Vec<Reminder>) -> Step {
        let total = items.len();
        read(
            "reminders",
            "read_reminders",
            json!({}),
            crate::apps::reminder_evidence(&Agenda { items, total }),
        )
    }

    fn event(id: &str, title: &str, start: &str, end: &str) -> Event {
        Event {
            id: Some(id.into()),
            title: title.into(),
            start: start.into(),
            end: end.into(),
            calendar: "Work".into(),
            ..Event::default()
        }
    }

    fn calendar_day(date: &str, items: Vec<Event>) -> Step {
        let total = items.len();
        read(
            &format!("day-{date}"),
            "read_calendar",
            json!({"day":date}),
            crate::apps::calendar_evidence(date, &Agenda { items, total }),
        )
    }

    fn notes(query: &str, rows: &[(&str, &str, &str)]) -> Step {
        let mut output: String = rows
            .iter()
            .map(|(id, title, excerpt)| {
                format!(
                    "2026-10-22 12:30\t{id}\t{title}\t{excerpt}\t{}\n",
                    excerpt.len()
                )
            })
            .collect();
        output += &format!("__buntini_summary__\t{}\t{}\ttrue", rows.len(), rows.len());
        read(
            "notes",
            "find_notes",
            json!({"query":query}),
            crate::apps::notes_evidence(query, &output).unwrap(),
        )
    }

    fn plan(goal: &str, tool: &str, steps: &[Step]) -> Result<EditPlan, String> {
        edit_plan_at(goal, tool, steps, now())
    }

    fn one(goal: &str, tool: &str, steps: &[Step]) -> Value {
        match plan(goal, tool, steps) {
            Ok(EditPlan::One(candidate)) => candidate.arguments,
            other => panic!("{goal}: {other:?}"),
        }
    }

    #[test]
    fn everyday_change_requests_become_the_right_write() {
        for (goal, expected) in [
            ("Mark buy milk as done", vec!["complete_reminder"]),
            ("mark the first one done", vec!["complete_reminder"]),
            ("Tick off call mum", vec!["complete_reminder"]),
            ("check the dentist reminder off", vec!["complete_reminder"]),
            ("Move my 3pm to 4", vec!["update_calendar_event"]),
            ("Push the dentist to Friday", vec!["update_calendar_event"]),
            (
                "Reschedule my meeting with Ana to tomorrow at 10",
                vec!["update_calendar_event"],
            ),
            ("Rename my 10:30 to Planning", vec!["update_calendar_event"]),
            (
                "Change the location of the design review to Room 4",
                vec!["update_calendar_event"],
            ),
            ("Add milk to my shopping note", vec!["append_to_note"]),
            (
                "Add milk, eggs and bread to my shopping note",
                vec!["append_to_note"],
            ),
            (
                "Add milk to my shopping note and remind me to buy eggs tomorrow at 9",
                vec!["append_to_note", "add_reminder"],
            ),
            ("Add a note about the trip", vec!["create_note"]),
            ("Don't mark it done", vec![]),
            ("How do I mark a reminder done?", vec![]),
        ] {
            assert_eq!(
                crate::completion::requested_actions(goal).0,
                expected,
                "{goal}"
            );
        }
        // Moving something that isn't an event stays unsupported rather than a calendar change.
        assert_eq!(
            crate::completion::requested_actions("Move the file to Downloads"),
            (vec![], true)
        );
        assert!(crate::completion::receipt_only_goal(
            "Add milk, eggs and bread to my shopping note"
        ));
    }

    #[test]
    fn a_change_first_reads_where_the_item_lives() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let contract =
            crate::task_contract::TaskContract::from_request_at("Mark buy milk as done", day);
        let kinds: Vec<_> = contract
            .outcomes
            .iter()
            .map(|o| (o.kind.as_str(), o.tool.as_deref()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("read", Some("read_reminders")),
                ("write", Some("complete_reminder"))
            ]
        );
        assert!(!contract.outcomes[1]
            .required_fields
            .contains(&"id".to_string()));
        let notes = crate::task_contract::TaskContract::from_request_at(
            "Add milk and eggs to my Shopping note",
            day,
        );
        assert_eq!(
            notes.outcomes[0].required_arguments,
            json!({"query":"shopping"})
        );
        let event =
            crate::task_contract::TaskContract::from_request_at("Move tomorrow's 3pm to 4", day);
        assert_eq!(
            event.outcomes[0].required_arguments,
            json!({"day":"2026-10-30"})
        );
        let unnamed =
            crate::task_contract::TaskContract::from_request_at("Push the dentist to Friday", day);
        assert_eq!(unnamed.outcomes[0].required_arguments, json!({}));
    }

    #[test]
    fn reminders_are_bound_from_the_read_never_from_text() {
        assert_eq!(
            plan("Mark buy milk as done", "complete_reminder", &[]),
            Ok(EditPlan::Read {
                tool: "read_reminders",
                arguments: json!({})
            })
        );
        let steps = [reminders(vec![
            reminder("r-1", "Buy milk", "2026-10-29 09:00"),
            reminder("r-2", "Buy milk powder", ""),
            reminder("r-3", "Call mum", "2026-10-30"),
        ])];
        assert_eq!(
            one("Mark buy milk as done", "complete_reminder", &steps),
            json!({"id":"r-1","title":"Buy milk"})
        );
        assert_eq!(
            one("tick off call mum", "complete_reminder", &steps)["id"],
            "r-3"
        );
        assert_eq!(
            one("mark the first one done", "complete_reminder", &steps)["id"],
            "r-1"
        );
        assert_eq!(
            one(
                "mark the second reminder as complete",
                "complete_reminder",
                &steps
            )["id"],
            "r-2"
        );
        let Ok(EditPlan::Several(several)) = plan("Tick off milk", "complete_reminder", &steps)
        else {
            panic!()
        };
        assert_eq!(
            several
                .iter()
                .map(|c| c.record_id.as_str())
                .collect::<Vec<_>>(),
            ["r-1", "r-2"]
        );
        assert!(plan("Mark the plumber done", "complete_reminder", &steps)
            .unwrap_err()
            .contains("couldn’t find"));
        assert!(plan("Mark it done", "complete_reminder", &steps)
            .unwrap_err()
            .contains("which reminder"));
    }

    #[test]
    fn events_move_by_time_or_title_and_keep_their_length() {
        let today = calendar_day(
            "2026-10-29",
            vec![
                event("e-1", "Standup", "2026-10-29 09:00", "2026-10-29 09:15"),
                event(
                    "e-2",
                    "Design review",
                    "2026-10-29 15:00",
                    "2026-10-29 15:45",
                ),
            ],
        );
        assert_eq!(
            one(
                "Move my 3pm to 4",
                "update_calendar_event",
                &[today.clone()]
            ),
            json!({"id":"e-2","occurrence":"2026-10-29 15:00","was_title":"Design review","title":"Design review","start":"2026-10-29 16:00","minutes":45})
        );
        assert_eq!(
            one(
                "move my next meeting to 10:30",
                "update_calendar_event",
                &[today.clone()]
            )["start"],
            "2026-10-29 10:30"
        );
        assert_eq!(
            one(
                "Rename my 3pm to Planning",
                "update_calendar_event",
                &[today.clone()]
            )["title"],
            "Planning"
        );
        let place = one(
            "Change the location of the design review to Room 4",
            "update_calendar_event",
            &[today.clone()],
        );
        assert_eq!(
            (place["location"].as_str(), place["start"].as_str()),
            (Some("Room 4"), Some("2026-10-29 15:00"))
        );
        assert!(plan(
            "Move the design review to Room 4",
            "update_calendar_event",
            &[today.clone()]
        )
        .unwrap_err()
        .contains("new day or time"));
        // Named without a day: today first, then each next day until it is found.
        assert_eq!(
            plan(
                "Push the dentist to Friday",
                "update_calendar_event",
                &[today.clone()]
            ),
            Ok(EditPlan::Read {
                tool: "read_calendar",
                arguments: json!({"day":"2026-10-30"})
            })
        );
        let tomorrow = calendar_day(
            "2026-10-30",
            vec![event(
                "e-9",
                "Dentist",
                "2026-10-30 11:00",
                "2026-10-30 11:30",
            )],
        );
        let moved = one(
            "Push the dentist to Monday",
            "update_calendar_event",
            &[today.clone(), tomorrow],
        );
        assert_eq!(
            (moved["start"].as_str(), moved["minutes"].as_u64()),
            (Some("2026-11-02 11:00"), Some(30))
        );
        let mut all_day = event("e-5", "Holiday", "2026-10-29 00:00", "2026-10-30 00:00");
        all_day.all_day = true;
        let holiday = calendar_day("2026-10-29", vec![all_day]);
        assert!(plan(
            "Move my holiday to tomorrow",
            "update_calendar_event",
            &[holiday]
        )
        .unwrap_err()
        .contains("all-day"));
        assert!(
            plan("Move my 3pm to 3pm", "update_calendar_event", &[today])
                .unwrap_err()
                .contains("already")
        );
    }

    #[test]
    fn notes_match_by_title_and_keep_the_users_words() {
        assert_eq!(
            plan(
                "Add milk and eggs to my shopping note",
                "append_to_note",
                &[]
            ),
            Ok(EditPlan::Read {
                tool: "find_notes",
                arguments: json!({"query":"shopping"})
            })
        );
        let found = [notes(
            "shopping",
            &[
                ("n-1", "Shopping", "Shopping Bread"),
                ("n-2", "Trip plan", "Remember the shopping bags"),
            ],
        )];
        assert_eq!(
            one(
                "Add milk and eggs to my shopping note",
                "append_to_note",
                &found
            ),
            json!({"id":"n-1","title":"Shopping","text":"milk and eggs"})
        );
        assert_eq!(
            one(
                "Add \"call the plumber, then the landlord\" to my shopping note",
                "append_to_note",
                &found
            )["text"],
            "call the plumber, then the landlord"
        );
        assert!(
            plan("Add that to my shopping note", "append_to_note", &found)
                .unwrap_err()
                .contains("your own words")
        );
        let only_body = [notes(
            "bags",
            &[("n-2", "Trip plan", "Remember the shopping bags")],
        )];
        assert!(
            plan("Add tape to my bags note", "append_to_note", &only_body)
                .unwrap_err()
                .contains("couldn’t find a note")
        );
    }

    #[test]
    fn only_an_item_read_for_what_the_user_named_can_be_changed() {
        let steps = [
            reminders(vec![
                reminder("r-1", "Buy milk", ""),
                reminder("r-2", "Buy milk powder", ""),
            ]),
            // Another source's record ids never qualify.
            read(
                "mail",
                "read_mail",
                json!({}),
                json!({"records":[{"id":"r-9"}]}),
            ),
        ];
        let goal = "Tick off milk";
        let check =
            |args: Value, approving| check(&[goal.into()], "complete_reminder", &args, &steps, approving);
        assert!(
            check(json!({"title":"Buy milk"}), false).is_ok(),
            "the user still picks one"
        );
        assert!(check(json!({"title":"Buy milk"}), true)
            .unwrap_err()
            .contains("Pick"));
        assert!(check(json!({"id":"r-2","title":"Buy milk powder"}), true).is_ok());
        assert!(
            check(json!({"id":"r-9","title":"Buy milk"}), true).is_err(),
            "an id from elsewhere"
        );
        assert!(
            check(json!({"id":"r-2","title":"Buy milk"}), true).is_err(),
            "an id with another item's title"
        );
        assert!(check(json!({"id":"r-1","title":"Buy milk"}), true).is_ok());
        assert!(
            super::check(
                &["Mark buy milk done".into()],
                "complete_reminder",
                &json!({"id":"r-1","title":"Buy milk"}),
                &[],
                true
            )
            .is_err(),
            "nothing read yet"
        );
        assert!(super::check(&[goal.into()], "create_note", &json!({}), &[], true).is_ok());
    }

    #[test]
    fn several_matches_are_offered_with_what_each_pick_sets() {
        let today = calendar_day(
            "2026-10-29",
            vec![
                event(
                    "e-1",
                    "Call with Ana",
                    "2026-10-29 15:00",
                    "2026-10-29 15:30",
                ),
                event(
                    "e-2",
                    "Call with Sam",
                    "2026-10-29 15:00",
                    "2026-10-29 16:00",
                ),
            ],
        );
        let Ok(EditPlan::Several(candidates)) = plan(
            "Move my 3pm to 4",
            "update_calendar_event",
            &[today.clone()],
        ) else {
            panic!()
        };
        let unpicked = unpicked(&candidates);
        assert!(unpicked.get("id").is_none());
        let (now, choices) = sheet_at(
            &["Move my 3pm to 4".into()],
            "update_calendar_event",
            &unpicked,
            &[today],
            super::tests::now(),
        );
        assert_eq!(now.unwrap().kind, "fixed");
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[1].value, "e-2");
        assert_eq!(choices[1].also["was_title"], "Call with Sam");
        assert_eq!(choices[1].also["minutes"], "60");
        assert!(choices[1].also["now"].contains("15:00–16:00"));
    }

    #[test]
    fn two_changes_of_the_same_kind_each_bind_their_own_item() {
        let goal = "Add milk to my shopping note and add eggs to my grocery note";
        let contract = crate::task_contract::TaskContract::from_request(goal);
        let goals = contract.edit_goals("append_to_note");
        assert_eq!(goals.len(), 2, "{goals:?}");
        let steps = [
            notes("shopping", &[("n-1", "Shopping", "Shopping Bread")]),
            notes("grocery", &[("n-2", "Grocery", "Grocery Rice")]),
        ];
        let second = one(&goals[1], "append_to_note", &steps);
        assert_eq!((second["id"].as_str(), second["text"].as_str()), (Some("n-2"), Some("eggs")));
        let first = one(&goals[0], "append_to_note", &steps);
        assert_eq!((first["id"].as_str(), first["text"].as_str()), (Some("n-1"), Some("milk")));
        // Either may be approved; an item read for neither may not.
        assert!(check(&goals, "append_to_note", &second, &steps, true).is_ok());
        assert!(check(&goals, "append_to_note", &json!({"id":"n-9","title":"Other","text":"eggs"}), &steps, true).is_err());
        // The read for the second change is the read for its own note.
        let read = contract
            .outcomes
            .iter()
            .filter(|o| o.description.starts_with(crate::task_contract::EDIT_READ))
            .map(|o| outcome_goal(&o.description, goal, "append_to_note"))
            .collect::<Vec<_>>();
        assert_eq!(read, [goals[0].as_str(), goals[1].as_str()]);
    }

    #[test]
    fn a_new_title_may_say_to() {
        let steps = [calendar_day(
            "2026-10-29",
            vec![event("e-1", "Planning", "2026-10-29 15:00", "2026-10-29 16:00")],
        )];
        let renamed = one("Rename my 3pm to Talk to Sam", "update_calendar_event", &steps);
        assert_eq!(renamed["title"], "Talk to Sam");
        let quoted = one("rename my 3pm to “Back to back”", "update_calendar_event", &steps);
        assert_eq!(quoted["title"], "Back to back");
        let moved = one("move my 3pm to 4", "update_calendar_event", &steps);
        assert_eq!(moved["start"], "2026-10-29 16:00");
    }

    #[test]
    fn a_lone_match_from_a_partial_read_waits_for_a_pick() {
        // Notes found 9 matching notes and returned 1.
        let output = "2026-10-22 12:30\tn-1\tShopping trip ideas\tShopping trip ideas Rome\t24\n__buntini_summary__\t9\t1\ttrue";
        let step = read("notes", "find_notes", json!({"query":"shopping"}), crate::apps::notes_evidence("shopping", output).unwrap());
        let planned = plan("Add milk to my shopping note", "append_to_note", &[step]);
        let Ok(EditPlan::Several(found)) = planned else {
            panic!("a partial read never binds on its own: {planned:?}")
        };
        assert_eq!(found.len(), 1);
        assert!(found[0].label.contains("others may match"));
    }

    #[test]
    fn picking_an_event_clears_what_another_match_had() {
        let mut first = event("e-1", "Standup", "2026-10-29 09:00", "2026-10-29 09:15");
        first.location = "Room 1".into();
        let today = calendar_day(
            "2026-10-29",
            vec![first, event("e-2", "Standup", "2026-10-29 15:00", "2026-10-29 15:15")],
        );
        let Ok(EditPlan::Several(candidates)) = plan("push the standup to 4", "update_calendar_event", &[today.clone()]) else {
            panic!()
        };
        let (_, choices) = sheet_at(&["push the standup to 4".into()], "update_calendar_event", &unpicked(&candidates), &[today], now());
        assert_eq!(choices[0].also["location"], "Room 1");
        assert_eq!(choices[1].also["location"], "", "a pick blanks the first match's place");
    }

    #[test]
    fn times_and_days_read_the_way_people_say_them() {
        let w = |text: &str| words(text);
        assert_eq!(
            clock(&w("4"), 0).map(|(c, _)| (c.hour, c.exact)),
            Some((4, false))
        );
        assert_eq!(
            clock(&w("4 pm"), 0).map(|(c, used)| (c.hour, used)),
            Some((16, 2))
        );
        assert_eq!(clock(&w("12am"), 0).map(|(c, _)| c.hour), Some(0));
        assert_eq!(
            clock(&w("4:30pm"), 0).map(|(c, _)| (c.hour, c.minute)),
            Some((16, 30))
        );
        assert!(
            clock(&w("2026"), 0).is_none()
                && clock(&w("13pm"), 0).is_none()
                && clock(&w("4:7"), 0).is_none()
        );
        let four = clock(&w("4"), 0).unwrap().0;
        assert_eq!((resolve(four, 15), resolve(four, 9)), (16, 4));
        let thursday = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert_eq!(
            day("friday", thursday),
            NaiveDate::from_ymd_opt(2026, 10, 30)
        );
        assert_eq!(
            day("thursday", thursday),
            NaiveDate::from_ymd_opt(2026, 11, 5),
            "the next one, not today"
        );
    }

    #[test]
    fn looking_for_an_event_reads_only_the_next_day_it_needs() {
        let goal = "Push the dentist to Friday";
        let today = chrono::Local::now().date_naive();
        let stamp = |ahead: i64| (today + chrono::Duration::days(ahead)).format("%Y-%m-%d").to_string();
        let steps = [calendar_day(&stamp(0), vec![])];
        let mut contract = crate::task_contract::TaskContract::from_request(goal);
        contract.refresh(goal, &steps);
        let propose = |tool: &str, args: Value| contract.validate_proposal(tool, &args, &steps);
        assert!(propose("read_calendar", json!({"day":stamp(1)})).is_ok());
        assert!(propose("read_calendar", json!({"day":stamp(5)})).is_err(), "not a day out of turn");
        assert!(propose("read_mail", json!({})).is_err());
        // A write for an item nobody read is refused, whatever the model says its id is.
        assert!(propose("update_calendar_event", json!({"id":"invented","occurrence":format!("{} 09:00", stamp(0)),"was_title":"Dentist","title":"Dentist","start":format!("{} 10:00", stamp(0)),"minutes":30})).is_err());
    }
}
