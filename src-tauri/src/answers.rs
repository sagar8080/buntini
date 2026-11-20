//! Answers Rust works out from the records read: counts, what's next, how many days until, how
//! long, free time, which comes first, totals, and lists in the order or form asked. Each says
//! what question it answers, and the search model says whether one is the person's question.
//! Apple's model, writing these itself, got 6 of 34 right on a fresh exam, and asked to choose
//! one, never once said none fitted.
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::Value;
use std::time::Duration as Wait;

use crate::workflows::Step;

/// One answer Rust can stand behind, and the question it answers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Candidate {
    pub asks: String,
    pub answer: String,
    /// The one day, list ("*" for all of them) or calendar it's about, if any.
    day: Option<NaiveDate>,
    list: Option<String>,
    calendar: Option<String>,
}

#[derive(Debug, Clone)]
struct Mail {
    sender: String,
    subject: String,
    body: String,
    received: NaiveDateTime,
    unread: bool,
}

#[derive(Debug, Clone)]
struct Event {
    title: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
    location: String,
    calendar: String,
    attendees: Vec<String>,
}

#[derive(Debug, Clone)]
struct Reminder {
    title: String,
    list: String,
    due: Option<NaiveDate>,
    at: Option<NaiveTime>,
}

#[derive(Default)]
struct Records {
    mail: Vec<Mail>,
    events: Vec<Event>,
    reminders: Vec<Reminder>,
    read_mail: bool,
    read_calendar: bool,
    read_reminders: bool,
    /// The last day the calendar was read to.
    calendar_until: Option<NaiveDate>,
    /// What Mail was searched for and found in full.
    searched: Vec<String>,
}

fn text(record: &Value, name: &str) -> String {
    record[name].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn stamp(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").ok()
}

fn date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

fn records(reads: &[&Step]) -> Records {
    let mut found = Records::default();
    let mut seen: Vec<(String, String)> = Vec::new();
    for step in reads {
        let Some(evidence) = step.evidence.as_ref() else { continue };
        match step.tool.as_str() {
            "read_mail" => {
                found.read_mail = true;
                let query = evidence["scope"]["query"].as_str().unwrap_or_default();
                if !query.is_empty() && evidence["complete"] == true {
                    found.searched.push(query.to_lowercase());
                }
            }
            "read_calendar" => {
                found.read_calendar = true;
                let until = evidence["scope"]["until"].as_str().or(evidence["scope"]["day"].as_str()).and_then(date);
                found.calendar_until = found.calendar_until.max(until);
            }
            "read_reminders" => found.read_reminders = true,
            _ => continue,
        }
        for record in evidence["records"].as_array().into_iter().flatten() {
            let key = (step.tool.clone(), format!("{}{}{}{}{}", record["id"], record["startLocal"], record["title"], record["subject"], record["receivedLocal"]));
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            match step.tool.as_str() {
                "read_mail" => {
                    let Some(received) = stamp(&text(record, "receivedLocal")) else { continue };
                    let sender = text(record, "sender");
                    let sender = sender.split('<').next().unwrap_or(&sender).trim().trim_matches('"').to_string();
                    found.mail.push(Mail { sender, subject: text(record, "subject"), body: text(record, "excerpt"), received, unread: record["read"] == false });
                }
                "read_calendar" => {
                    let all_day = record["all_day"] == true || record["allDay"] == true;
                    let start = stamp(&text(record, "startLocal")).or_else(|| date(&text(record, "startLocal")).and_then(|day| day.and_hms_opt(0, 0, 0)));
                    let Some(start) = start else { continue };
                    let end = stamp(&text(record, "endLocal")).unwrap_or(start);
                    let attendees = record["attendees"].as_array().map(|people| people.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                    found.events.push(Event { title: text(record, "title"), start, end, all_day, location: text(record, "location"), calendar: text(record, "calendar"), attendees });
                }
                _ => {
                    let due = text(record, "dueLocal");
                    found.reminders.push(Reminder { title: text(record, "title"), list: text(record, "list"), due: date(&due), at: stamp(&due).map(|at| at.time()) });
                }
            }
        }
    }
    found.events.sort_by_key(|event| event.start);
    found.mail.sort_by(|a, b| b.received.cmp(&a.received));
    found
}

fn day_words(day: NaiveDate, today: NaiveDate) -> String {
    let named = day.format("%A %-d %B").to_string();
    match (day - today).num_days() {
        0 => format!("today ({named})"),
        1 => format!("tomorrow ({named})"),
        -1 => format!("yesterday ({named})"),
        ahead if ahead > 1 => format!("{named}, in {ahead} days"),
        back => format!("{named}, {} days ago", -back),
    }
}

fn short_day(day: NaiveDate) -> String {
    day.format("%a %-d %b").to_string()
}

fn length(minutes: i64) -> String {
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, _) => format!("{minutes} minutes"),
        (_, 0) => format!("{hours} hour{}", if hours == 1 { "" } else { "s" }),
        _ => format!("{hours} hour{} {rest} minutes", if hours == 1 { "" } else { "s" }),
    }
}

fn twelve(time: NaiveTime) -> String {
    let (pm, hour) = time.hour12();
    match time.minute() {
        0 => format!("{hour} {}", if pm { "pm" } else { "am" }),
        minute => format!("{hour}:{minute:02} {}", if pm { "pm" } else { "am" }),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Whether `word` (from the question) is one of `text`'s words, allowing for plurals and
/// endings: "climb" in "Bouldering and climbing", "insurance" in "Renew pet insurance".
fn holds(text: &str, word: &str) -> bool {
    // "stand-up" is held by "Team stand-up": each part of a hyphenated word.
    if word.contains(|c: char| !c.is_alphanumeric()) {
        return word.split(|c: char| !c.is_alphanumeric()).filter(|part| part.chars().count() >= 3).any(|part| holds(text, part));
    }
    let word = word.to_lowercase();
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|token| !token.is_empty()).any(|token| {
        token == word
            || token.strip_suffix('s') == Some(word.as_str())
            || word.strip_suffix('s') == Some(token)
            || (word.chars().count() >= 4 && token.starts_with(word.as_str()) && token.len() <= word.len() + 4)
            || (token.chars().count() >= 4 && word.starts_with(token) && word.len() <= token.len() + 4)
    })
}

fn matching<'a, T>(items: &'a [T], words: &[String], text: impl Fn(&T) -> String) -> Vec<&'a T> {
    let mut scored: Vec<(usize, usize)> = items.iter().enumerate().map(|(at, item)| (words.iter().filter(|word| holds(&text(item), word)).count(), at)).filter(|(hits, _)| *hits > 0).collect();
    let best = scored.iter().map(|(hits, _)| *hits).max().unwrap_or(0);
    scored.retain(|(hits, _)| *hits == best);
    scored.into_iter().map(|(_, at)| &items[at]).collect()
}

fn reminder_line(reminder: &Reminder, today: NaiveDate) -> String {
    match reminder.due {
        Some(day) => {
            let at = reminder.at.map(|time| format!(" at {}", time.format("%H:%M"))).unwrap_or_default();
            let late = (day < today).then(|| format!(", {} late", plural((today - day).num_days() as usize, "day", "days"))).unwrap_or_default();
            format!("{} (due {}{at}{late})", reminder.title, short_day(day))
        }
        None => format!("{} (no due date)", reminder.title),
    }
}

fn event_line(event: &Event) -> String {
    let when = if event.all_day { "all day".to_string() } else { format!("{}–{}", event.start.format("%H:%M"), event.end.format("%H:%M")) };
    format!("{} ({} {when})", event.title, short_day(event.start.date()))
}

fn by_due(reminders: &[&Reminder]) -> Vec<Reminder> {
    let mut sorted: Vec<Reminder> = reminders.iter().map(|reminder| (*reminder).clone()).collect();
    sorted.sort_by(|a, b| match (a.due, b.due) {
        (Some(x), Some(y)) => x.cmp(&y).then(a.at.cmp(&b.at)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    sorted
}

fn numbered(lines: &[String]) -> String {
    lines.iter().enumerate().map(|(at, line)| format!("{}. {line}", at + 1)).collect::<Vec<_>>().join("\n")
}

/// A clock window the question gives: "between 9 and 6", "from 10am to 4pm".
fn window(goal: &str) -> Option<(u32, u32)> {
    let times = crate::literals::times(goal);
    if times.len() == 2 && times[0] < times[1] {
        return Some((times[0], times[1]));
    }
    let words: Vec<String> = goal.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_string).collect();
    for at in 0..words.len().saturating_sub(3) {
        if ["between", "from"].contains(&words[at].as_str()) && ["and", "to", "till", "until"].contains(&words[at + 2].as_str()) {
            let (Ok(from), Ok(to)) = (words[at + 1].parse::<u32>(), words[at + 3].parse::<u32>()) else { continue };
            if !(1..=12).contains(&from) || !(1..=12).contains(&to) {
                continue;
            }
            // "between 9 and 6" is a working day: the later hour is in the afternoon.
            let from_hour = if from < 7 { from + 12 } else { from };
            let to_hour = if to <= from_hour % 12 || to < 8 { to + 12 } else { to };
            if from_hour < to_hour {
                return Some((from_hour * 60, to_hour * 60));
            }
        }
    }
    None
}

/// Free stretches between `from` and `to` (minutes after midnight) on a day, around its timed events.
fn free_between(events: &[&Event], from: u32, to: u32) -> Vec<(u32, u32)> {
    let mut busy: Vec<(u32, u32)> = events.iter().filter(|event| !event.all_day).map(|event| (event.start.hour() * 60 + event.start.minute(), if event.end.date() > event.start.date() { 24 * 60 } else { event.end.hour() * 60 + event.end.minute() })).collect();
    busy.sort_unstable();
    let mut free = Vec::new();
    let mut at = from;
    for (start, end) in busy {
        // What starts after the window doesn't shorten it.
        if start >= to {
            break;
        }
        if start > at {
            free.push((at, start));
        }
        at = at.max(end);
    }
    if at < to {
        free.push((at, to));
    }
    free.retain(|(start, end)| end > start);
    free
}

fn clock(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Every answer Rust can work out from these reads that may bear on the question.
pub fn candidates(goal: &str, reads: &[&Step], now: NaiveDateTime) -> Vec<Candidate> {
    let today = now.date();
    let found = records(reads);
    let words = crate::personal_answer::question_words_of(goal);
    let asked = crate::task_contract::spelled(goal);
    let literal = crate::literals::read(&asked, today);
    let mut out: Vec<Candidate> = Vec::new();
    // What the answers being added are about: a day, a list or a calendar.
    let scope: std::cell::RefCell<Candidate> = Default::default();
    let mut add = |asks: String, answer: String| {
        if !out.iter().any(|seen| seen.asks == asks) {
            let scope = scope.borrow();
            out.push(Candidate { asks, answer, day: scope.day, list: scope.list.clone(), calendar: scope.calendar.clone() });
        }
    };
    // A plain way to name a day in a question: "today", "on Thursday", "on Thursday 26 November".
    let phrase = |day: NaiveDate| match (day - today).num_days() {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        2..=6 => day.format("on %A").to_string(),
        _ => day.format("on %A %-d %B").to_string(),
    };

    // Reminders.
    if found.read_reminders {
        let all: Vec<&Reminder> = found.reminders.iter().collect();
        let listed = |items: &[&Reminder]| items.iter().map(|reminder| reminder_line(reminder, today)).collect::<Vec<_>>().join("; ");
        let overdue: Vec<&Reminder> = all.iter().copied().filter(|reminder| reminder.due.is_some_and(|day| day < today)).collect();
        let late = match overdue.len() {
            0 => "None of your reminders are overdue.".to_string(),
            count => format!("{} overdue: {}.", plural(count, "reminder is", "reminders are"), listed(&by_due(&overdue).iter().collect::<Vec<_>>())),
        };
        for asks in ["how many reminders are overdue", "which reminders am I behind on", "what's late on my to-do list"] {
            add(asks.into(), late.clone());
        }
        if let Some(oldest) = by_due(&overdue).first() {
            add("which overdue reminder has been waiting the longest".into(), format!("{} has been overdue longest: it was due {}, {} ago.", oldest.title, short_day(oldest.due.unwrap_or(today)), plural((today - oldest.due.unwrap_or(today)).num_days() as usize, "day", "days")));
        }
        for (asks, from, to) in [("today", 0, 0), ("tomorrow", 1, 1), ("in the next 7 days, not counting today", 1, 7), ("in the next 7 days including today", 0, 7)] {
            scope.borrow_mut().day = (from == to).then(|| today + chrono::Duration::days(from));
            let due: Vec<&Reminder> = all.iter().copied().filter(|reminder| reminder.due.is_some_and(|day| (day - today).num_days() >= from && (day - today).num_days() <= to)).collect();
            let answer = match due.len() {
                0 => format!("No reminders are due {asks}."),
                count => format!("{} due {asks}: {}.", plural(count, "reminder is", "reminders are"), listed(&by_due(&due).iter().collect::<Vec<_>>())),
            };
            add(format!("how many reminders are due {asks}, and which"), answer.clone());
            if from == to {
                add(format!("what's due {asks}"), answer);
            }
        }
        scope.borrow_mut().day = None;
        let upcoming: Vec<Reminder> = by_due(&all.iter().copied().filter(|reminder| reminder.due.is_some_and(|day| day > today)).collect::<Vec<_>>());
        if let Some(day) = upcoming.first().and_then(|next| next.due) {
            let then: Vec<&Reminder> = upcoming.iter().filter(|reminder| reminder.due == Some(day)).collect();
            add("what's my next reminder after today's".into(), match then.len() {
                1 => format!("After today, your next reminder is {}, due {}{}.", then[0].title, day_words(day, today), then[0].at.map(|time| format!(" at {}", time.format("%H:%M"))).unwrap_or_default()),
                _ => format!("After today, the next reminders are due {}: {}.", day_words(day, today), then.iter().map(|reminder| reminder_line(reminder, today)).collect::<Vec<_>>().join("; ")),
            });
        }
        let timed: Vec<Reminder> = by_due(&all.iter().copied().filter(|reminder| reminder.at.is_some() && reminder.due.zip(reminder.at).is_some_and(|(day, time)| day.and_time(time) >= now)).collect::<Vec<_>>());
        add("when's my next reminder that has a time set".into(), match timed.first() {
            Some(next) => format!("Your next reminder with a time is {}, due {} at {}.", next.title, day_words(next.due.unwrap_or(today), today), next.at.map(|time| time.format("%H:%M").to_string()).unwrap_or_default()),
            None => "None of your coming reminders has a time set.".into(),
        });
        let undated: Vec<&Reminder> = all.iter().copied().filter(|reminder| reminder.due.is_none()).collect();
        add("which reminders have no due date".into(), match undated.len() {
            0 => "Every reminder has a due date.".into(),
            count => format!("{} no due date: {}.", plural(count, "reminder has", "reminders have"), undated.iter().map(|reminder| reminder.title.clone()).collect::<Vec<_>>().join("; ")),
        });
        add("how many reminders do I have in all".into(), format!("You have {}.", plural(all.len(), "open reminder", "open reminders")));
        let mut lists: Vec<String> = all.iter().map(|reminder| reminder.list.clone()).filter(|list| !list.is_empty()).collect();
        lists.sort();
        lists.dedup();
        let mut groups: Vec<(String, Vec<&Reminder>)> = lists.iter().map(|list| (list.clone(), all.iter().copied().filter(|reminder| &reminder.list == list).collect())).collect();
        groups.push(("all".into(), all.clone()));
        for (list, items) in groups {
            scope.borrow_mut().list = Some(if list == "all" { "*".to_string() } else { list.clone() });
            let named = if list == "all" { "all my reminders".to_string() } else { format!("my {list} list") };
            if list != "all" {
                add(format!("how many things are on {named}, and what"), format!("Your {list} list has {}: {}.", plural(items.len(), "reminder", "reminders"), items.iter().map(|reminder| reminder.title.clone()).collect::<Vec<_>>().join("; ")));
            }
            let sorted = by_due(&items);
            add(format!("{named} sorted by due date, soonest first, as a numbered list"), format!("{}:\n{}", if list == "all" { "Your reminders, soonest first".to_string() } else { format!("Your {list} reminders, soonest first") }, numbered(&sorted.iter().map(|reminder| reminder_line(reminder, today)).collect::<Vec<_>>())));
            let mut alphabetical: Vec<String> = items.iter().map(|reminder| reminder.title.clone()).collect();
            alphabetical.sort_by_key(|title| title.to_lowercase());
            add(format!("{named} in alphabetical order"), format!("{}:\n{}", if list == "all" { "Your reminders, A to Z".to_string() } else { format!("Your {list} list, A to Z") }, alphabetical.iter().map(|title| format!("- {title}")).collect::<Vec<_>>().join("\n")));
        }
        scope.borrow_mut().list = None;
        for reminder in matching(&found.reminders, &words, |reminder| format!("{} {}", reminder.title, reminder.list)) {
            let title = reminder.title.to_lowercase();
            let due = match reminder.due {
                Some(day) => format!("“{}” is due {}{}.", reminder.title, day_words(day, today), reminder.at.map(|time| format!(" at {}", time.format("%H:%M"))).unwrap_or_else(|| ", with no time set".into())),
                None => format!("“{}” has no due date.", reminder.title),
            };
            add(format!("when is the reminder to {title} due"), due.clone());
            add(format!("how many days until the reminder to {title} is due"), due);
            add(
                format!("what time is the reminder to {title} set for"),
                match (reminder.due, reminder.at) {
                    (Some(day), Some(time)) => format!("“{}” is set for {} on {}.", reminder.title, time.format("%H:%M"), short_day(day)),
                    (Some(day), None) => format!("“{}” has no time set; it's due {}.", reminder.title, day_words(day, today)),
                    _ => format!("“{}” has no due date or time.", reminder.title),
                },
            );
        }
    }

    // Calendar.
    if found.read_calendar {
        let upcoming: Vec<&Event> = found.events.iter().filter(|event| if event.all_day { event.start.date() >= today } else { event.start > now }).collect();
        if let Some(next) = upcoming.first() {
            let place = if next.location.is_empty() { String::new() } else { format!(" at {}", next.location) };
            let when = if next.all_day { "all day".into() } else { format!("{}–{}", next.start.format("%H:%M"), next.end.format("%H:%M")) };
            add("what's next on my calendar".into(), format!("Next on your calendar: {}, {}, {when}{place}.", next.title, day_words(next.start.date(), today)));
        }
        let mut days: Vec<NaiveDate> = found.events.iter().map(|event| event.start.date()).collect();
        days.dedup();
        let clock_window = window(goal);
        for day in days.iter().copied().filter(|day| *day >= today && (*day - today).num_days() <= 14) {
            scope.borrow_mut().day = Some(day);
            let on: Vec<&Event> = found.events.iter().filter(|event| event.start.date() == day).collect();
            let named = day_words(day, today);
            let timed: Vec<&&Event> = on.iter().filter(|event| !event.all_day).collect();
            let booked: i64 = timed.iter().map(|event| (event.end - event.start).num_minutes()).sum();
            let said = phrase(day);
            let listing = format!("{}: {}{}: {}.", capitalised(&named), plural(on.len(), "event", "events"), if booked > 0 { format!(", {} booked", length(booked)) } else { String::new() }, on.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; "));
            for calendar in on.iter().map(|event| event.calendar.clone()).filter(|calendar| !calendar.is_empty()).collect::<std::collections::BTreeSet<_>>() {
                let mine: Vec<&&Event> = on.iter().filter(|event| event.calendar == calendar).collect();
                let minutes: i64 = mine.iter().filter(|event| !event.all_day).map(|event| (event.end - event.start).num_minutes()).sum();
                let answer = format!("{}, your {calendar} calendar has {}{}: {}.", capitalised(&named), plural(mine.len(), "event", "events"), if minutes > 0 { format!(", {} booked", length(minutes)) } else { String::new() }, mine.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; "));
                scope.borrow_mut().calendar = Some(calendar.clone());
                add(format!("what's on my {calendar} calendar {said}"), answer.clone());
                add(format!("how much time is booked on my {calendar} calendar {said}"), answer);
                scope.borrow_mut().calendar = None;
            }
            if found.read_reminders {
                let due: Vec<&Reminder> = found.reminders.iter().filter(|reminder| reminder.due == Some(day)).collect();
                add(
                    format!("how many things have I got {said}, events and reminders together"),
                    format!(
                        "{}: {} in all, {} and {}. Events: {}. Reminders: {}.",
                        capitalised(&named),
                        on.len() + due.len(),
                        plural(on.len(), "event", "events"),
                        plural(due.len(), "reminder", "reminders"),
                        if on.is_empty() { "none".into() } else { on.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; ") },
                        if due.is_empty() { "none".into() } else { due.iter().map(|reminder| reminder.title.clone()).collect::<Vec<_>>().join("; ") }
                    ),
                );
            }
            add(format!("what's on my calendar {said}"), listing.clone());
            add(format!("how many events do I have {said}"), listing.clone());
            add(format!("how much time is booked {said}"), listing);
            add(
                format!("write out my schedule {said} in 12-hour times"),
                format!("{}:\n{}", capitalised(&named), on.iter().map(|event| if event.all_day { format!("- All day: {}", event.title) } else { format!("- {}–{}: {}", twelve(event.start.time()), twelve(event.end.time()), event.title) }).collect::<Vec<_>>().join("\n")),
            );
            let (from, to) = clock_window.unwrap_or((9 * 60, 18 * 60));
            let free = free_between(&on, from, to);
            let longest = free.iter().max_by_key(|(start, end)| end - start);
            let span = format!("between {} and {}", clock(from), clock(to));
            add(
                format!("where are my free gaps {said} {span}, and which is longest"),
                match longest {
                    None => format!("{}: no free time {span}.", capitalised(&named)),
                    Some((start, end)) => format!(
                        "{}, {span}, you're free {}. The longest is {}–{} ({}).",
                        capitalised(&named),
                        free.iter().map(|(start, end)| format!("{}–{} ({})", clock(*start), clock(*end), length(i64::from(end - start)))).collect::<Vec<_>>().join(", "),
                        clock(*start),
                        clock(*end),
                        length(i64::from(end - start))
                    ),
                },
            );
            if timed.len() > 1 {
                let last = timed.iter().max_by_key(|event| event.end).unwrap();
                let first = timed.iter().min_by_key(|event| event.start).unwrap();
                add(format!("when does my first event start and my last end {said}"), format!("{}: the first starts at {} ({}), the last ends at {} ({}).", capitalised(&named), first.start.format("%H:%M"), first.title, last.end.format("%H:%M"), last.title));
            }
        }
        scope.borrow_mut().day = None;
        // By calendar.
        let mut calendars: Vec<String> = found.events.iter().map(|event| event.calendar.clone()).filter(|calendar| !calendar.is_empty()).collect();
        calendars.sort();
        calendars.dedup();
        for calendar in &calendars {
            scope.borrow_mut().calendar = Some(calendar.clone());
            let on: Vec<&&Event> = upcoming.iter().filter(|event| &event.calendar == calendar).collect();
            add(format!("what's on my {calendar} calendar from now on, in order"), format!("Your {calendar} calendar from now: {}.", if on.is_empty() { "nothing".into() } else { on.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; ") }));
        }
        scope.borrow_mut().calendar = None;
        // The events the question names.
        let named = matching(&found.events, &words, |event| format!("{} {}", event.title, event.location));
        for event in named.iter().take(4) {
            let place = if event.location.is_empty() { "no location set".to_string() } else { format!("at {}", event.location) };
            let when = if event.all_day { "all day".to_string() } else { format!("{}–{} ({})", event.start.format("%H:%M"), event.end.format("%H:%M"), length((event.end - event.start).num_minutes())) };
            let with = if event.attendees.is_empty() { String::new() } else { format!(", with {}", event.attendees.join(", ")) };
            let title = event.title.to_lowercase();
            let about = format!("{} is {}, {when}, {place}{with}.", event.title, day_words(event.start.date(), today));
            for asks in [format!("when is {title}"), format!("how many days until {title}"), format!("how long is {title}"), format!("where is {title}")] {
                add(asks, about.clone());
            }
            add(format!("who's going to {title}"), if event.attendees.is_empty() { format!("{} lists no one else.", event.title) } else { format!("{}: {}.", event.title, event.attendees.join(", ")) });
            // What else is on that day.
            let others: Vec<&Event> = found.events.iter().filter(|other| other.start.date() == event.start.date() && other.title != event.title).collect();
            add(format!("what else is on the day of {title}"), match others.len() {
                0 => format!("Nothing else is on {} besides {}.", day_words(event.start.date(), today), event.title),
                _ => format!("Also on {}: {}.", day_words(event.start.date(), today), others.iter().map(|other| event_line(other)).collect::<Vec<_>>().join("; ")),
            });
            for calendar in &calendars {
                scope.borrow_mut().calendar = Some(calendar.clone());
                let before: Vec<&&Event> = upcoming.iter().filter(|other| &other.calendar == calendar && other.start < event.start && other.title != event.title).collect();
                add(format!("how many {calendar} events between now and {}", event.title), format!("{} between now and {} ({}): {}.", plural(before.len(), &format!("{calendar} event"), &format!("{calendar} events")), event.title, short_day(event.start.date()), before.iter().map(|other| event_line(other)).collect::<Vec<_>>().join("; ")));
            }
        }
        scope.borrow_mut().calendar = None;
        if named.len() > 1 {
            let (a, b) = (named[0], named[1]);
            if !a.all_day && !b.all_day {
                let (la, lb) = ((a.end - a.start).num_minutes(), (b.end - b.start).num_minutes());
                let (long, short, more) = if la >= lb { (a, b, la - lb) } else { (b, a, lb - la) };
                add(format!("which is longer, {} or {}", a.title, b.title), if la == lb { format!("They're the same length: {} each.", length(la)) } else { format!("{} ({}) is longer than {} ({}), by {}.", long.title, length((long.end - long.start).num_minutes()), short.title, length((short.end - short.start).num_minutes()), length(more)) });
            }
        }
        // Upcoming events sharing the question's words: the next one, how many, and how long in all.
        // An event's calendar says what it is too: Bouldering on the Climbing calendar is a climb.
        // Within a run of days the question names ("over the next 2 weeks").
        let in_span: Vec<Event> = upcoming.iter().filter(|event| literal.span.is_none_or(|(from, to)| event.start.date() >= from && event.start.date() <= to)).map(|event| (*event).clone()).collect();
        let upcoming_named: Vec<&Event> = matching(&in_span, &words, |event| format!("{} {}", event.title, event.calendar)).into_iter().map(|event| found.events.iter().find(|seen| seen.title == event.title && seen.start == event.start).unwrap()).collect();
        if let Some(first) = upcoming_named.first() {
            let topic = words.iter().find(|word| holds(&format!("{} {}", first.title, first.calendar), word)).cloned().unwrap_or_default();
            add(format!("when is my next {topic}"), format!("The next {topic} event is {}, {}, {}–{}{}.", first.title, day_words(first.start.date(), today), first.start.format("%H:%M"), first.end.format("%H:%M"), if first.location.is_empty() { String::new() } else { format!(" at {}", first.location) }));
            let minutes: i64 = upcoming_named.iter().filter(|event| !event.all_day).map(|event| (event.end - event.start).num_minutes()).sum();
            add(format!("how many {topic} sessions are still booked, and how long in all"), format!("{} still booked, {} in all: {}.", plural(upcoming_named.len(), &format!("{topic} event"), &format!("{topic} events")), length(minutes), upcoming_named.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; ")));
        }
    }

    // A run of days the question names: "this week", "the next two weeks".
    if let Some((from, to)) = literal.span {
        let lower = asked.to_lowercase();
        let named = ["this week", "next week", "this weekend", "next weekend", "this month"].into_iter().find(|said| lower.contains(said)).map(str::to_string).unwrap_or_else(|| format!("from {} to {}", short_day(from), short_day(to)));
        let within = |day: NaiveDate| day >= from && day <= to;
        if found.read_reminders {
            let due: Vec<&Reminder> = found.reminders.iter().filter(|reminder| reminder.due.is_some_and(within)).collect();
            let answer = match due.len() {
                0 => format!("No reminders are due {named} ({} to {}).", short_day(from), short_day(to)),
                count => format!("{} due {named} ({} to {}): {}.", plural(count, "reminder is", "reminders are"), short_day(from), short_day(to), by_due(&due).iter().map(|reminder| reminder_line(reminder, today)).collect::<Vec<_>>().join("; ")),
            };
            add(format!("what's due {named}"), answer.clone());
            add(format!("how many reminders are due {named}"), answer);
        }
        if found.read_calendar {
            let on: Vec<&Event> = found.events.iter().filter(|event| within(event.start.date())).collect();
            let minutes: i64 = on.iter().filter(|event| !event.all_day).map(|event| (event.end - event.start).num_minutes()).sum();
            let answer = format!("{} to {}: {}{}{}", short_day(from), short_day(to), plural(on.len(), "event", "events"), if minutes > 0 { format!(", {} booked", length(minutes)) } else { String::new() }, if on.is_empty() { ".".into() } else { format!(": {}.", on.iter().map(|event| event_line(event)).collect::<Vec<_>>().join("; ")) });
            add(format!("what's on my calendar {named}"), answer.clone());
            add(format!("how many events do I have {named}"), answer.clone());
            add(format!("how much time is booked {named}"), answer);
        }
    }
    // Free or not at one time: "am I free tomorrow at 9am".
    if found.read_calendar && literal.times.len() == 1 {
        let day = literal.days.first().copied().unwrap_or(today);
        let minute = literal.times[0];
        if let Some(at) = day.and_hms_opt(minute / 60, minute % 60, 0) {
            let on: Vec<&Event> = found.events.iter().filter(|event| event.start.date() == day && !event.all_day).collect();
            let busy: Vec<&&Event> = on.iter().filter(|event| event.start <= at && at < event.end).collect();
            let next = on.iter().filter(|event| event.start > at).min_by_key(|event| event.start);
            let answer = match busy.first() {
                Some(event) => format!("No: {} runs {}–{} {}.", event.title, event.start.format("%H:%M"), event.end.format("%H:%M"), day_words(day, today)),
                None => format!("Yes: nothing on your calendar at {} {}.{}", at.format("%H:%M"), day_words(day, today), next.map(|event| format!(" Next is {} at {}.", event.title, event.start.format("%H:%M"))).unwrap_or_default()),
            };
            scope.borrow_mut().day = Some(day);
            add(format!("am I free {} at {}", phrase(day), twelve(at.time())), answer);
            scope.borrow_mut().day = None;
        }
    }

    // Mail.
    if found.read_mail {
        let unread: Vec<&Mail> = found.mail.iter().filter(|mail| mail.unread).collect();
        let unread_answer = format!("{} unread{}", plural(unread.len(), "email is", "emails are"), if unread.is_empty() { ".".into() } else { format!(": {}.", unread.iter().map(|mail| format!("{} ({})", mail.subject, mail.sender)).collect::<Vec<_>>().join("; ")) });
        add("how many unread emails do I have".into(), unread_answer.clone());
        add("do I have any unread emails".into(), unread_answer);
        let mut senders: Vec<String> = found.mail.iter().map(|mail| mail.sender.clone()).collect();
        senders.sort();
        senders.dedup();
        // Only a sender Mail was searched for in full is counted: the newest few aren't all of theirs.
        let in_full = |sender: &str| found.searched.iter().any(|query| holds(sender, query) || sender.to_lowercase().contains(query.as_str()));
        for sender in senders.iter().filter(|sender| words.iter().any(|word| holds(sender, word)) && in_full(sender)) {
            let from: Vec<&Mail> = found.mail.iter().filter(|mail| &mail.sender == sender).collect();
            add(format!("how many emails has {sender} sent me"), format!("{} from {sender}: {}.", plural(from.len(), "email", "emails"), from.iter().map(|mail| format!("“{}” ({})", mail.subject, short_day(mail.received.date()))).collect::<Vec<_>>().join("; ")));
        }
        let named = matching(&found.mail, &words, |mail| format!("{} {} {}", mail.sender, mail.subject, mail.body));
        for mail in named.iter().take(4) {
            for (said, day) in crate::literals::relative_dates(&format!("{} {}", mail.subject, mail.body), mail.received.date()).into_iter().filter(|(said, _)| !["today", "tonight", "this morning"].contains(&said.as_str())) {
                let phrase = said.as_str();
                add(
                    format!("what date is “{phrase}” in {}'s email about {}", mail.sender, mail.subject.to_lowercase()),
                    format!("{}'s email “{}” was sent {}, so “{phrase}” means {}.", mail.sender, mail.subject, short_day(mail.received.date()), day_words(day, today)),
                );
            }
        }
        // A sender the question names means all of theirs: "every card payment the bank emailed".
        let from_named: Vec<&Mail> = found.mail.iter().filter(|mail| words.iter().any(|word| holds(&mail.sender, word))).collect();
        let whole = from_named.iter().all(|mail| in_full(&mail.sender));
        let summed = if from_named.is_empty() { named.clone() } else if whole { from_named } else { vec![] };
        // "Split the Acme invoice evenly between two of us": the amount in the email the question
        // names, shared out by Rust between the people it counts or names. Asked in so many
        // words, so it's the answer when only one email holds an amount.
        let splitting = crate::skills::words(goal).split(' ').any(|word| ["split", "divide", "share", "halve"].contains(&word));
        if let Some(people) = crate::structured_math::sharers(goal).filter(|people| splitting && *people > 0) {
            let shares: Vec<(&Mail, String, char, f64)> = named.iter().filter_map(|mail| crate::decision::money_in(&format!("{} {}", mail.subject, mail.body)).into_iter().next().map(|(written, sign, amount)| (*mail, written, sign, amount))).collect();
            for (mail, written, sign, amount) in &shares {
                let asks = if shares.len() == 1 { crate::task_contract::spelled(goal) } else { format!("split “{}” evenly between {people}", mail.subject.to_lowercase()) };
                add(asks, format!("Split evenly between {people}: {written} (“{}”, {}) is {} each.", mail.subject, mail.sender, crate::decision::money(*sign, amount / people as f64)));
            }
        }
        let priced: Vec<(&Mail, f64)> = summed.iter().filter_map(|mail| crate::literals::amounts(&format!("{} {}", mail.subject, mail.body)).first().map(|amount| (*mail, *amount))).collect();
        if priced.len() > 1 {
            let total: f64 = priced.iter().map(|(_, amount)| amount).sum();
            let topic = words.iter().filter(|word| priced.iter().any(|(mail, _)| holds(&format!("{} {}", mail.subject, mail.body), word))).cloned().collect::<Vec<_>>().join(" ");
            add(format!("add up the amounts in my emails about {topic}"), format!("${total:.2} in all: {}.", priced.iter().map(|(mail, amount)| format!("“{}” ${amount:.2}", mail.subject)).collect::<Vec<_>>().join(", ")));
            if priced.len() == 2 {
                let ((a, x), (b, y)) = (priced[0], priced[1]);
                let (cheap, dear, gap) = if x <= y { (a, b, y - x) } else { (b, a, x - y) };
                add(format!("how much cheaper is “{}” than “{}”", cheap.subject.to_lowercase(), dear.subject.to_lowercase()), format!("“{}” is ${gap:.2} cheaper than “{}”.", cheap.subject, dear.subject));
            }
        }
    }

    // Which comes first, across the apps: what the question names, with a day each.
    let mut dated: Vec<(String, NaiveDate)> = Vec::new();
    for reminder in matching(&found.reminders, &words, |reminder| reminder.title.clone()).into_iter().take(2) {
        if let Some(day) = reminder.due {
            dated.push((format!("the reminder “{}”", reminder.title), day));
        }
    }
    for event in matching(&found.events, &words, |event| event.title.clone()).into_iter().take(2) {
        dated.push((event.title.clone(), event.start.date()));
    }
    if dated.len() >= 2 {
        let (a, b) = (&dated[0], &dated[1]);
        let gap = (b.1 - a.1).num_days();
        let answer = match gap {
            0 => format!("{} and {} are on the same day, {}.", capitalised(&a.0), b.0, day_words(a.1, today)),
            gap if gap > 0 => format!("{} ({}) comes before {} ({}), by {}.", capitalised(&a.0), short_day(a.1), b.0, short_day(b.1), plural(gap as usize, "day", "days")),
            gap => format!("{} ({}) comes after {} ({}), by {}.", capitalised(&a.0), short_day(a.1), b.0, short_day(b.1), plural((-gap) as usize, "day", "days")),
        };
        add(format!("is {} before or after {}", a.0, b.0), answer.clone());
        add(format!("how many days are there between {} and {}", a.0, b.0), answer);
    }
    drop(add);
    // A day, list or calendar the question names rules out answers about another one: "sort my
    // home reminders" isn't answered by all of them, nor "tomorrow's schedule" by today's.
    let days = literal.days.clone();
    let lists: Vec<String> = found.reminders.iter().map(|reminder| reminder.list.clone()).filter(|list| !list.is_empty() && holds(&asked, list)).collect();
    let calendars: Vec<String> = found.events.iter().map(|event| event.calendar.clone()).filter(|calendar| !calendar.is_empty() && holds(&asked, calendar)).collect();
    out.retain(|candidate| {
        candidate.day.is_none_or(|day| literal.span.is_none() && (days.is_empty() || days.contains(&day)))
            && candidate.list.as_ref().is_none_or(|list| lists.is_empty() || lists.contains(list))
            && candidate.calendar.as_ref().is_none_or(|calendar| calendars.is_empty() || calendars.contains(calendar))
            // "Booked on my work calendar tomorrow" isn't every calendar's day.
            && !(candidate.day.is_some() && candidate.calendar.is_none() && !calendars.is_empty())
    });
    out
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn unit(vector: &[i8]) -> Vec<f32> {
    let length = vector.iter().map(|value| f32::from(*value).powi(2)).sum::<f32>().sqrt().max(1.0);
    vector.iter().map(|value| f32::from(*value) / length).collect()
}

/// How near by meaning a candidate's question must be to the person's, and how far ahead of the
/// nearest one saying something else, to be the answer. Rightly matched questions scored 0.88
/// to 0.99, the wrong ones nearby 0.8 or so.
const NEAR: f32 = 0.88;
const CLEAR: f32 = 0.02;

/// The candidates by how near their questions are to the person's, nearest first, with how
/// near. Empty when the search model is busy or missing.
pub fn ranked(goal: &str, candidates: &[Candidate]) -> Vec<(Candidate, f32)> {
    let mut texts = vec![crate::task_contract::spelled(goal)];
    texts.extend(candidates.iter().map(|candidate| candidate.asks.clone()));
    let Ok(vectors) = crate::embedding::embed_within(&texts, false, Wait::from_secs(2)) else { return vec![] };
    if vectors.len() != texts.len() || vectors.iter().any(Option::is_none) {
        return vec![];
    }
    let vectors: Vec<Vec<f32>> = vectors.into_iter().map(|vector| unit(&vector.unwrap_or_default())).collect();
    let mut scored: Vec<(Candidate, f32)> = candidates.iter().cloned().zip(vectors[1..].iter().map(|vector| vector.iter().zip(&vectors[0]).map(|(a, b)| a * b).sum())).collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored
}

/// The answer, when one candidate's question is plainly the person's: near enough, and clear of
/// the nearest that answers differently. Otherwise none, and nothing is guessed.
pub fn confident(ranked: &[(Candidate, f32)]) -> Option<&Candidate> {
    let (best, score) = ranked.first()?;
    let rival = ranked.iter().skip(1).find(|(other, _)| other.answer != best.answer).map_or(0.0, |(_, score)| *score);
    (*score >= NEAR && score - rival >= CLEAR).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(tool: &str, evidence: Value) -> Step {
        Step { id: "s".into(), tool: tool.into(), arguments: json!({}), state: "succeeded".into(), effect: false, proposed_at: String::new(), approved_at: None, finished_at: None, evidence: Some(evidence) }
    }

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 18).unwrap().and_hms_opt(9, 30, 0).unwrap()
    }

    fn reminders() -> Step {
        step(
            "read_reminders",
            json!({"records":[
                {"title":"Book van for Wexmoor","list":"Work","dueLocal":"2026-11-17"},
                {"title":"Order sample jars","list":"Work","dueLocal":"2026-11-14"},
                {"title":"Renew Pilot's pet insurance","list":"Home","dueLocal":"2026-11-16"},
                {"title":"Water the balcony herbs","list":"Home","dueLocal":"2026-11-18 18:00"},
                {"title":"Buy Imogen's birthday present","list":"Home","dueLocal":"2026-11-24"},
                {"title":"Dog treats","list":"Groceries","dueLocal":""},
                {"title":"Oat milk","list":"Groceries","dueLocal":""}
            ]}),
        )
    }

    fn calendar() -> Step {
        step(
            "read_calendar",
            json!({"scope":{"day":"2026-11-18","until":"2026-12-02"},"records":[
                {"title":"Early session","startLocal":"2026-11-18 07:00","endLocal":"2026-11-18 08:15","location":"Basalt Wall","calendar":"Personal"},
                {"title":"Video call with Imogen","startLocal":"2026-11-18 20:45","endLocal":"2026-11-18 21:15","location":"","calendar":"Personal"},
                {"title":"Bouldering","startLocal":"2026-11-19 18:00","endLocal":"2026-11-19 19:30","location":"Basalt Wall","calendar":"Personal"},
                {"title":"Supplier call","startLocal":"2026-11-19 11:00","endLocal":"2026-11-19 11:30","location":"","calendar":"Work"},
                {"title":"Farm visit","startLocal":"2026-11-24 13:00","endLocal":"2026-11-24 15:00","location":"","calendar":"Work"},
                {"title":"Launch tasting","startLocal":"2026-11-26 10:00","endLocal":"2026-11-26 13:00","location":"","calendar":"Work"},
                {"title":"Food fair","startLocal":"2026-11-27","endLocal":"2026-11-27","all_day":true,"location":"Wexmoor","calendar":"Work"}
            ]}),
        )
    }

    fn says(found: &[Candidate], asks: &str) -> String {
        found.iter().find(|candidate| candidate.asks == asks).map(|candidate| candidate.answer.clone()).unwrap_or_else(|| panic!("no “{asks}” among {:?}", found.iter().map(|candidate| &candidate.asks).collect::<Vec<_>>()))
    }

    #[test]
    fn reminders_are_counted_and_ordered_by_rust() {
        let list = reminders();
        let found = candidates("how many of my reminders are overdue rn", &[&list], now());
        assert!(says(&found, "how many reminders are overdue").starts_with("3 reminders are overdue: Order sample jars (due Sat 14 Nov, 4 days late); Renew Pilot's pet insurance"));
        assert!(says(&found, "which overdue reminder has been waiting the longest").starts_with("Order sample jars has been overdue longest"));
        assert!(says(&found, "how many reminders are due today, and which").starts_with("1 reminder is due today: Water the balcony herbs (due Wed 18 Nov at 18:00)"));
        assert!(says(&found, "how many things are on my Home list, and what").starts_with("Your Home list has 3 reminders"));
        assert_eq!(says(&found, "my Groceries list in alphabetical order"), "Your Groceries list, A to Z:\n- Dog treats\n- Oat milk");
        assert!(says(&found, "what's my next reminder after today's").contains("Buy Imogen's birthday present, due Tuesday 24 November, in 6 days"));
        assert_eq!(says(&found, "when's my next reminder that has a time set"), "Your next reminder with a time is Water the balcony herbs, due today (Wednesday 18 November) at 18:00.");
        let found = candidates("what time is the imogen present reminder set for", &[&list], now());
        assert_eq!(says(&found, "what time is the reminder to buy imogen's birthday present set for"), "“Buy Imogen's birthday present” has no time set; it's due Tuesday 24 November, in 6 days.");
    }

    #[test]
    fn the_calendar_is_worked_out_by_rust() {
        let days = calendar();
        let found = candidates("whats next on my calender", &[&days], now());
        assert_eq!(says(&found, "what's next on my calendar"), "Next on your calendar: Video call with Imogen, today (Wednesday 18 November), 20:45–21:15.");
        let found = candidates("where are my free gaps tmrw between 9 and 6? which ones the longest", &[&days], now());
        assert_eq!(says(&found, "where are my free gaps tomorrow between 09:00 and 18:00, and which is longest"), "Tomorrow (Thursday 19 November), between 09:00 and 18:00, you're free 09:00–11:00 (2 hours), 11:30–18:00 (6 hours 30 minutes). The longest is 11:30–18:00 (6 hours 30 minutes).");
        let found = candidates("which is longer, the farm visit or the launch tasting", &[&days], now());
        assert_eq!(says(&found, "which is longer, Farm visit or Launch tasting"), "Launch tasting (3 hours) is longer than Farm visit (2 hours), by 1 hour.");
        let found = candidates("how many days till the food fair", &[&days], now());
        assert!(says(&found, "how many days until food fair").starts_with("Food fair is Friday 27 November, in 9 days, all day, at Wexmoor"));
        let found = candidates("how many work events have i got between now and the fair", &[&days], now());
        assert!(says(&found, "how many Work events between now and Food fair").starts_with("3 Work events between now and Food fair (Fri 27 Nov)"));
        let found = candidates("how much of tmrw is booked on my work calendar", &[&days], now());
        assert_eq!(says(&found, "how much time is booked on my Work calendar tomorrow"), "Tomorrow (Thursday 19 November), your Work calendar has 1 event, 30 minutes booked: Supplier call (Thu 19 Nov 11:00–11:30).");
        assert!(!found.iter().any(|candidate| candidate.asks == "how much time is booked tomorrow"));
        let found = candidates("write out tmrws schedule using 12 hour times", &[&days], now());
        assert_eq!(says(&found, "write out my schedule tomorrow in 12-hour times"), "Tomorrow (Thursday 19 November):\n- 11 am–11:30 am: Supplier call\n- 6 pm–7:30 pm: Bouldering");
    }

    #[test]
    fn a_named_day_or_list_rules_out_answers_about_another() {
        let (list, days) = (reminders(), calendar());
        let found = candidates("sort my home reminders by due date", &[&list], now());
        assert!(found.iter().any(|candidate| candidate.asks == "my Home list sorted by due date, soonest first, as a numbered list"));
        assert!(!found.iter().any(|candidate| candidate.asks.starts_with("all my reminders") || candidate.asks.starts_with("my Groceries")));
        let found = candidates("write out tmrws schedule using 12 hour times", &[&days], now());
        assert!(found.iter().any(|candidate| candidate.asks == "write out my schedule tomorrow in 12-hour times"));
        assert!(!found.iter().any(|candidate| candidate.asks == "write out my schedule today in 12-hour times"));
    }

    #[test]
    fn a_span_and_a_time_are_answered_as_asked() {
        let (list, days) = (reminders(), calendar());
        let found = candidates("what's due this week", &[&list], now());
        assert!(found.iter().any(|candidate| candidate.asks == "what's due this week"));
        assert!(!found.iter().any(|candidate| candidate.asks == "what's due today"), "a week isn't one day");
        let found = candidates("am I free tomorrow at 11:15", &[&days], now());
        assert_eq!(says(&found, "am I free tomorrow at 11:15 am"), "No: Supplier call runs 11:00–11:30 tomorrow (Thursday 19 November).");
        let found = candidates("am I free tomorrow at 9am", &[&days], now());
        assert_eq!(says(&found, "am I free tomorrow at 9 am"), "Yes: nothing on your calendar at 09:00 tomorrow (Thursday 19 November). Next is Supplier call at 11:00.");
    }

    #[test]
    fn an_event_after_the_window_doesnt_hide_its_last_gap() {
        let event = |title: &str, start: &str, end: &str| Event { title: title.into(), start: stamp(start).unwrap(), end: stamp(end).unwrap(), all_day: false, location: String::new(), calendar: "Work".into(), attendees: vec![] };
        let day = [event("Review", "2026-11-19 09:30", "2026-11-19 10:30"), event("Lunch", "2026-11-19 12:30", "2026-11-19 13:15"), event("1:1", "2026-11-19 15:00", "2026-11-19 15:45"), event("Class", "2026-11-19 18:30", "2026-11-19 20:00")];
        let on: Vec<&Event> = day.iter().collect();
        assert_eq!(free_between(&on, 9 * 60, 18 * 60), vec![(540, 570), (630, 750), (795, 900), (945, 1080)]);
    }

    #[test]
    fn across_apps_rust_says_which_comes_first() {
        let (list, days) = (reminders(), calendar());
        let found = candidates("is my reminder to buy imogens present before the launch tasting", &[&list, &days], now());
        assert_eq!(says(&found, "is the reminder “Buy Imogen's birthday present” before or after Launch tasting"), "The reminder “Buy Imogen's birthday present” (Tue 24 Nov) comes before Launch tasting (Thu 26 Nov), by 2 days.");
    }

    #[test]
    fn mail_is_counted_by_sender_and_its_days_resolved() {
        let inbox = step(
            "read_mail",
            json!({"records":[
                {"sender":"Fen Abara <fen@larkspur.example>","subject":"Van sorted","excerpt":"I booked the van.","receivedLocal":"2026-11-17 11:10","read":true},
                {"sender":"Fen Abara <fen@larkspur.example>","subject":"Fermentation log","excerpt":"Moved the log.","receivedLocal":"2026-11-09 09:05","read":true},
                {"sender":"Spoke & Sprocket <shop@spoke.example>","subject":"Your bike","excerpt":"Your bike will be ready to collect in 7 days.","receivedLocal":"2026-11-16 13:25","read":false}
            ]}),
        );
        assert!(!candidates("how many emails has fen sent me", &[&inbox], now()).iter().any(|candidate| candidate.asks.contains("Fen Abara sent")), "the newest few aren't all of Fen's");
        let mut inbox = inbox;
        inbox.evidence.as_mut().unwrap()["scope"] = json!({"query":"fen"});
        inbox.evidence.as_mut().unwrap()["complete"] = json!(true);
        let found = candidates("how many emails has fen sent me", &[&inbox], now());
        let said = says(&found, "how many emails has Fen Abara sent me");
        assert!(said.starts_with("2 emails from Fen Abara"), "{said}");
        let bill = step("read_mail", json!({"records":[{"sender":"Acme Billing <billing@acme.example>","subject":"Invoice 4821 for October","excerpt":"Your invoice of $1,240 is due on 20 November.","receivedLocal":"2026-11-16 16:40","read":true}]}));
        let goal = "Split the Acme invoice evenly between two of us. What's each share?";
        let found = candidates(goal, &[&bill], now());
        assert_eq!(says(&found, &crate::task_contract::spelled(goal)), "Split evenly between 2: $1,240 (“Invoice 4821 for October”, Acme Billing) is $620 each.");
        let found = candidates("is my bike ready to pick up yet", &[&inbox], now());
        assert_eq!(says(&found, "what date is “in 7 days” in Spoke & Sprocket's email about your bike"), "Spoke & Sprocket's email “Your bike” was sent Mon 16 Nov, so “in 7 days” means Monday 23 November, in 5 days.");
    }

    #[test]
    fn an_answer_is_given_only_when_its_question_is_plainly_the_one_asked() {
        let one = |asks: &str, answer: &str| Candidate { asks: asks.into(), answer: answer.into(), ..Default::default() };
        let clear = vec![(one("how many reminders are overdue", "3"), 0.94), (one("what's due today", "2"), 0.89)];
        assert_eq!(confident(&clear).map(|candidate| candidate.answer.as_str()), Some("3"));
        let close = vec![(one("a", "3"), 0.92), (one("b", "2"), 0.905)];
        assert!(confident(&close).is_none(), "too close to another answer");
        let same = vec![(one("a", "3"), 0.92), (one("b", "3"), 0.919), (one("c", "2"), 0.8)];
        assert!(confident(&same).is_some(), "two ways of asking for one answer aren't rivals");
        let far = vec![(one("a", "3"), 0.85)];
        assert!(confident(&far).is_none());
        assert!(confident(&[]).is_none());
    }
}
