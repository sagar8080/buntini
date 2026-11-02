//! Answers Rust works out itself, where the on-device model was measured getting them
//! wrong: a walking plan that fits the opening hours the user gave, a pick made by the
//! user's own rule from the one note it was read from, and who does what by when in that
//! note. Each applies only when every fact it needs is stated plainly; otherwise the
//! model answers as usual.
use crate::workflows::Step;
use chrono::{Datelike, NaiveDate};

const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// A clock time in minutes after midnight: "13:20", "9:30", "9am", "1:20pm".
fn clock(word: &str) -> Option<u32> {
    let word = word.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | '(' | ')'));
    let (body, half) = match word.strip_suffix("am").or_else(|| word.strip_suffix("pm")) {
        Some(body) => (body, Some(word.ends_with("pm"))),
        None => (word, None),
    };
    let (hour, minute) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 && (1..=2).contains(&h.len()) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if half.is_some() && (1..=2).contains(&body.len()) => (body.parse::<u32>().ok()?, 0),
        _ => return None,
    };
    let hour = match half {
        Some(_) if !(1..=12).contains(&hour) => return None,
        Some(pm) => hour % 12 + if pm { 12 } else { 0 },
        None if hour > 23 => return None,
        None => hour,
    };
    (minute < 60).then_some(hour * 60 + minute)
}

fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Sentences and list items: split at ", ", "; ", ": " and a full stop before a space.
fn segments(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut begin = 0;
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if matches!(bytes[index], b',' | b';' | b':' | b'.') && bytes.get(index + 1).is_none_or(|next| *next == b' ') {
            parts.push(text[begin..index].trim());
            begin = index + 1;
        }
    }
    parts.push(text[begin..].trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn titled(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

struct Stop {
    name: String,
    opens: u32,
    closes: u32,
}

/// "Library opens 10:00-13:00", "the cafe is open 9am to 4pm".
fn stop(segment: &str) -> Option<Stop> {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let at = words.iter().position(|w| ["opens", "open"].contains(&w.to_lowercase().as_str()))?;
    let mut name = &words[..at];
    while let Some((first, rest)) = name.split_first() {
        if ["and", "the", "is", "are"].contains(&first.to_lowercase().as_str()) {
            name = rest;
        } else {
            break;
        }
    }
    if let Some((last, rest)) = name.split_last() {
        if ["is", "are"].contains(&last.to_lowercase().as_str()) {
            name = rest;
        }
    }
    if name.is_empty() || name.len() > 4 || !name.iter().all(|w| w.chars().all(|c| c.is_alphabetic() || c == '\'')) {
        return None;
    }
    let times: Vec<u32> = words[at + 1..]
        .iter()
        .flat_map(|w| w.split(['-', '–']))
        .filter(|w| !["from", "to", "until", "till", "between", "and", ""].contains(&w.to_lowercase().as_str()))
        .map(|w| clock(&w.to_lowercase()))
        .collect::<Option<_>>()?;
    let [opens, closes] = times[..] else {
        return None;
    };
    (closes > opens).then(|| Stop { name: name.join(" "), opens, closes })
}

/// "Plan a half-day walking itinerary using only these opening hours: Library opens
/// 10:00-13:00, cafe opens 09:00-16:00, museum opens 12:00-17:00. I start at 09:30 and
/// each visit takes 45 minutes; allow 15 minutes to walk between places."
pub fn itinerary(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase();
    if !(lower.contains("itinerary") || (lower.contains("plan") && lower.contains("visit"))) || goal.len() > 2000 {
        return None;
    }
    let parts = segments(goal);
    let stops: Vec<Stop> = parts.iter().filter_map(|part| stop(part)).collect();
    if !(2..=6).contains(&stops.len()) {
        return None;
    }
    let words: Vec<String> = lower.split_whitespace().map(str::to_owned).collect();
    let start = words.windows(3).find_map(|w| {
        (["start", "starting", "begin", "leave", "set"].contains(&w[0].as_str()) && w[1] == "at").then(|| clock(&w[2])).flatten()
    })?;
    let length = |cue: &[&str]| {
        let found: Vec<u32> = parts
            .iter()
            .filter(|part| {
                let part = part.to_lowercase();
                cue.iter().any(|c| part.contains(c))
            })
            .filter_map(|part| crate::explicit_fields::timer_seconds(part))
            .map(|seconds| seconds as u32 / 60)
            .collect();
        match found[..] {
            [minutes] if minutes > 0 => Some(minutes),
            _ => None,
        }
    };
    let visit = length(&["visit", "stay", "spend", "each stop"])?;
    let walk = length(&["walk"])?;
    // Every order of the stops; the one that ends earliest, waiting least and starting
    // soonest wins.
    let mut best: Option<((u32, u32, u32), Vec<(usize, u32, u32)>)> = None;
    let mut order: Vec<usize> = (0..stops.len()).collect();
    let mut permute = |order: &[usize]| {
        let mut time = start;
        let mut waited = 0;
        let mut plan = Vec::new();
        for (step, &index) in order.iter().enumerate() {
            if step > 0 {
                time += walk;
            }
            let begin = time.max(stops[index].opens);
            waited += begin - time;
            if begin + visit > stops[index].closes {
                return;
            }
            plan.push((index, time, begin));
            time = begin + visit;
        }
        let rank = (time, waited, plan[0].2);
        if best.as_ref().is_none_or(|(best, _)| rank < *best) {
            best = Some((rank, plan));
        }
    };
    heap(&mut order, stops.len(), &mut permute);
    let Some((_, plan)) = best else {
        return Some(format!(
            "Those hours don’t leave room to visit all {} places with {visit}-minute visits and {walk}-minute walks from {}.",
            stops.len(),
            hhmm(start)
        ));
    };
    let lines: Vec<String> = plan
        .iter()
        .map(|&(index, arrive, begin)| {
            let mut line = format!("{}–{} {}", hhmm(begin), hhmm(begin + visit), titled(&stops[index].name));
            if begin > arrive {
                line += &format!(" (you’ll arrive at {} and wait for it to open)", hhmm(arrive));
            }
            line
        })
        .collect();
    Some(format!("This order fits the opening hours, with {walk}-minute walks between stops:\n{}", lines.join("\n")))
}

/// Visits every permutation of `items[..k]` in a fixed order (Heap's algorithm).
fn heap(items: &mut Vec<usize>, k: usize, visit: &mut impl FnMut(&[usize])) {
    if k <= 1 {
        visit(items);
        return;
    }
    for i in 0..k {
        heap(items, k - 1, visit);
        let swap = if k % 2 == 0 { i } else { 0 };
        if i + 1 < k {
            items.swap(swap, k - 1);
        }
    }
}

/// The text of the one note this task read, when the read returned all of it.
fn note_text(steps: &[Step]) -> Option<String> {
    let reads: Vec<&Step> = steps
        .iter()
        .filter(|s| s.state == "succeeded" && !s.effect && s.tool == "find_notes")
        .collect();
    let [read] = reads[..] else {
        return None;
    };
    let records = read.evidence.as_ref()?["records"].as_array()?;
    let [record] = &records[..] else {
        return None;
    };
    if record["contentComplete"] != true {
        return None;
    }
    let text = record["excerpt"].as_str()?;
    Some(record["title"].as_str().and_then(|title| text.strip_prefix(title)).unwrap_or(text).trim().to_owned())
}

/// Days from `today` until a weekday or a month-and-day the text names once.
fn days_until(text: &str, today: NaiveDate) -> Option<i64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if let Some(day) = WEEKDAYS.iter().position(|w| w == word) {
            let from = today.weekday().num_days_from_monday() as i64;
            found.push((day as i64 - from).rem_euclid(7));
        } else if let Some(month) = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"].iter().position(|m| m == word) {
            if let Some(day) = words.get(at + 1).and_then(|d| d.parse::<u32>().ok()) {
                let year = words.get(at + 2).and_then(|y| y.parse::<i32>().ok()).filter(|y| *y > 1900);
                let date = NaiveDate::from_ymd_opt(year.unwrap_or(today.year()), month as u32 + 1, day)?;
                let date = if year.is_none() && date < today { date.with_year(today.year() + 1)? } else { date };
                found.push((date - today).num_days());
            }
        }
    }
    match found[..] {
        [days] => Some(days),
        _ => None,
    }
}

fn dollars(word: &str) -> Option<f64> {
    word.strip_prefix('$')?.trim_end_matches(|c: char| !c.is_ascii_digit()).replace(',', "").parse().ok()
}

fn money(amount: f64) -> String {
    if amount.fract() == 0.0 { format!("${amount:.0}") } else { format!("${amount:.2}") }
}

struct Choice {
    label: String,
    parts: Vec<f64>,
    total: f64,
    when: String,
    days: i64,
}

/// "Quote A: $75 parts plus $40 labor, ready Friday. Quote B: $120 total, ready
/// Wednesday. Prefer the lowest total cost unless the difference is $5 or less, then
/// choose the earlier date." The pick is the user's rule applied in Rust.
pub fn rule_choice(goal: &str, steps: &[Step], today: NaiveDate) -> Option<String> {
    let asked = goal.to_lowercase();
    if !asked.contains("rule") || !["recommend", "which", "choose", "pick"].iter().any(|w| asked.contains(w)) {
        return None;
    }
    let text = note_text(steps)?;
    let sentences: Vec<&str> = text.split(". ").map(str::trim).collect();
    let rule = sentences.iter().find(|s| {
        let s = s.to_lowercase();
        (s.contains("lowest") || s.contains("cheapest")) && s.contains("unless")
    })?;
    let lower = rule.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let within = words.windows(3).find_map(|w| (w[1] == "or" && ["less", "under"].contains(&w[2].trim_end_matches([',', '.']))).then(|| dollars(w[0])).flatten())
        .or_else(|| words.windows(2).find_map(|w| (w[0] == "within").then(|| dollars(w[1])).flatten()))?;
    if !["earlier", "earliest", "sooner", "soonest"].iter().any(|w| lower.contains(w)) {
        return None;
    }
    // Each option runs from its label ("quote A", "Option 2") to the next label or the rule.
    let body = &text[..text.find(rule)?];
    let tokens: Vec<(usize, &str)> = body.split_whitespace().map(|w| (w.as_ptr() as usize - body.as_ptr() as usize, w)).collect();
    let mut starts = Vec::new();
    for pair in tokens.windows(2) {
        let kind = pair[0].1.to_lowercase();
        let tag = pair[1].1.trim_end_matches([':', ',', '.']);
        if ["quote", "option", "offer", "bid", "plan"].contains(&kind.as_str()) && tag.chars().count() <= 2 && tag.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            starts.push((pair[0].0, format!("{} {tag}", titled(&kind))));
        }
    }
    if starts.len() < 2 {
        return None;
    }
    let mut choices = Vec::new();
    for (index, (begin, label)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(body.len(), |next| next.0);
        let segment = &body[*begin..end];
        let seg = segment.to_lowercase();
        if seg.split(|c: char| !c.is_alphanumeric()).any(|w| ["discount", "off", "minus", "refund", "per", "each", "monthly"].contains(&w)) {
            return None;
        }
        let amounts: Vec<f64> = seg.split_whitespace().filter_map(dollars).collect();
        let total_given = seg.split_whitespace().collect::<Vec<_>>().windows(2).find_map(|w| (w[1].starts_with("total")).then(|| dollars(w[0])).flatten());
        let (parts, total) = match total_given {
            Some(total) => (vec![total], total),
            None if !amounts.is_empty() => (amounts.clone(), amounts.iter().sum()),
            None => return None,
        };
        let days = days_until(segment, today)?;
        let when = segment.split(|c: char| !c.is_alphanumeric()).find(|w| WEEKDAYS.contains(&w.to_lowercase().as_str())).map(titled).unwrap_or_else(|| "its date".into());
        choices.push(Choice { label: label.clone(), parts, total, when, days });
    }
    let cheapest = choices.iter().map(|c| c.total).fold(f64::INFINITY, f64::min);
    if choices.iter().filter(|c| c.total == cheapest).count() != 1 {
        return None;
    }
    let close: Vec<&Choice> = choices.iter().filter(|c| c.total - cheapest <= within + 1e-9).collect();
    let soonest = close.iter().map(|c| c.days).min()?;
    if close.iter().filter(|c| c.days == soonest).count() != 1 {
        return None;
    }
    let pick = close.iter().find(|c| c.days == soonest)?;
    let mut lines: Vec<String> = choices
        .iter()
        .map(|c| {
            let sum = if c.parts.len() > 1 {
                format!("{} = {}", c.parts.iter().map(|p| money(*p)).collect::<Vec<_>>().join(" + "), money(c.total))
            } else {
                money(c.total)
            };
            format!("{}: {sum}, ready {}.", c.label, c.when)
        })
        .collect();
    let lowest = choices.iter().find(|c| c.total == cheapest)?;
    lines.push(if close.len() > 1 {
        let gap = choices.iter().filter(|c| c.total != cheapest).map(|c| c.total - cheapest).fold(0.0, f64::max);
        format!("They’re {} apart, within your {} rule, so the earlier one wins: I recommend {} ({}).", money(gap), money(within), pick.label, pick.when)
    } else {
        format!("{} costs least by more than {}, so I recommend {} ({}).", lowest.label, money(within), lowest.label, money(lowest.total))
    });
    Some(lines.join("\n"))
}

const STARTERS: &[&str] = &["the", "a", "an", "this", "that", "these", "those", "it", "there", "please", "keep", "do", "don't", "remember", "note", "next", "send", "make", "bring", "prefer", "choose"];

/// "Alex owns the landing-page draft, due Monday." → who and the whole assignment.
fn assignment(sentence: &str) -> Option<(String, String)> {
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let first = *words.first()?;
    let second = words.get(1)?.to_lowercase();
    let person = if ["I", "We"].contains(&first) {
        true
    } else {
        first.chars().next()?.is_uppercase()
            && first.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'')
            && !STARTERS.contains(&first.to_lowercase().as_str())
            && (["must", "will", "should", "needs", "has", "is", "can", "owns"].contains(&second.as_str())
                || (second.len() > 3 && second.ends_with('s') && second.chars().all(char::is_alphabetic)))
    };
    let lower = sentence.to_lowercase();
    let dated = [" due ", " by ", " before ", " on "].iter().any(|marker| {
        lower.match_indices(marker).any(|(at, _)| {
            let next = lower[at + marker.len()..].split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default();
            WEEKDAYS.contains(&next) || ["today", "tomorrow", "tonight", "noon"].contains(&next) || next.parse::<u32>().is_ok()
                || ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"].contains(&next)
        })
    });
    if !person || !dated {
        return None;
    }
    let owner = if ["I", "We"].contains(&first) { "You".to_string() } else { first.to_string() };
    let rest = sentence[first.len()..].trim().trim_end_matches('.');
    Some((owner, format!("{} {rest}", if ["I", "We"].contains(&first) { "You" } else { first })))
}

/// "Summarize the three owners and deadlines as a checklist" from the one note read:
/// each assignment stays one line, with its owner, task and deadline together.
pub fn assignments(goal: &str, steps: &[Step]) -> Option<String> {
    let asked = goal.to_lowercase();
    if !(asked.contains("owner") || asked.contains("who ")) || !(asked.contains("deadline") || asked.contains("due")) {
        return None;
    }
    let text = note_text(steps)?;
    let found: Vec<(String, String)> = text.split(['.', ';', '\n']).filter_map(|s| assignment(s.trim())).collect();
    if found.len() < 2 {
        return None;
    }
    let mut owners: Vec<&str> = found.iter().map(|(owner, _)| owner.as_str()).collect();
    owners.dedup();
    let counts = [("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("2", 2), ("3", 3), ("4", 4), ("5", 5), ("6", 6)];
    if let Some((_, count)) = counts.iter().find(|(word, _)| asked.contains(&format!("{word} owners")) || asked.contains(&format!("{word} people"))) {
        if owners.len() != *count {
            return None;
        }
    }
    let checklist = asked.contains("checklist");
    Some(
        found
            .iter()
            .map(|(_, line)| format!("{}{line}.", if checklist { "- [ ] " } else { "- " }))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The first worked answer that applies to this request and journal.
pub fn answer(goal: &str, steps: &[Step], today: NaiveDate) -> Option<String> {
    itinerary(goal)
        .filter(|_| steps.iter().all(|s| s.state != "succeeded" || s.effect))
        .or_else(|| rule_choice(goal, steps, today))
        .or_else(|| assignments(goal, steps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn note(body: &str) -> Vec<Step> {
        vec![Step {
            id: "read".into(),
            tool: "find_notes".into(),
            arguments: json!({"query":"QA"}),
            state: "succeeded".into(),
            effect: false,
            proposed_at: "now".into(),
            approved_at: None,
            finished_at: Some("now".into()),
            evidence: Some(json!({"records":[{"title":"QA note","excerpt":format!("QA note {body}"),"contentComplete":true}]})),
        }]
    }

    #[test]
    fn an_itinerary_fits_every_visit_inside_its_opening_hours() {
        let goal = "Plan a half-day walking itinerary using only these supplied opening hours: Library opens 10:00-13:00, cafe opens 09:00-16:00, museum opens 12:00-17:00. I start at 09:30 and each visit takes 45 minutes; allow 15 minutes to walk between places. Include a start time for each stop, and visit all three while they are open.";
        assert_eq!(
            itinerary(goal).unwrap(),
            "This order fits the opening hours, with 15-minute walks between stops:\n09:30–10:15 Cafe\n10:30–11:15 Library\n12:00–12:45 Museum (you’ll arrive at 11:30 and wait for it to open)"
        );
        let impossible = goal.replace("Library opens 10:00-13:00", "Library opens 10:00-10:30");
        assert!(itinerary(&impossible).unwrap().starts_with("Those hours don’t leave room"));
        // Without a start time or walking time, the model plans.
        assert!(itinerary(&goal.replace("I start at 09:30 and ", "")).is_none());
        assert!(itinerary("Plan my week: the gym opens 06:00-22:00 and the pool opens 07:00-20:00.").is_none());
    }

    #[test]
    fn a_pick_follows_the_users_own_rule_at_its_boundary() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(); // a Sunday
        let goal = "Read my QA Quotes note, calculate both total prices, and recommend one using my stated rule.";
        let rule = "Prefer the lowest total cost unless the difference is $5 or less, then choose the earlier date.";
        let steps = note(&format!("Bike repair quote A: $75 parts plus $40 labor, ready Friday. Quote B: $120 total, ready Wednesday. {rule}"));
        assert_eq!(
            rule_choice(goal, &steps, today).unwrap(),
            "Quote A: $75 + $40 = $115, ready Friday.\nQuote B: $120, ready Wednesday.\nThey’re $5 apart, within your $5 rule, so the earlier one wins: I recommend Quote B (Wednesday)."
        );
        let wide = note(&format!("Bike repair quote A: $75 parts plus $40 labor, ready Friday. Quote B: $130 total, ready Wednesday. {rule}"));
        assert!(rule_choice(goal, &wide, today).unwrap().ends_with("Quote A costs least by more than $5, so I recommend Quote A ($115)."));
        // Two options ready the same day, or no rule, leave it to the model.
        let tied = note(&format!("Quote A: $115 total, ready Friday. Quote B: $120 total, ready Friday. {rule}"));
        assert!(rule_choice(goal, &tied, today).is_none());
        assert!(rule_choice(goal, &note("Quote A: $115 total, ready Friday. Quote B: $120 total, ready Wednesday."), today).is_none());
    }

    #[test]
    fn each_owner_keeps_their_task_and_deadline_on_one_line() {
        let goal = "Find my QA Handoff note and summarize the three owners and deadlines as a checklist. Do not send any messages.";
        let steps = note("Alex owns the landing-page draft, due Monday. Mina reviews the accessibility checklist, due Tuesday. I must send the finalized screenshots to Alex before Monday. Keep the reply as a checklist; do not send anything.");
        assert_eq!(
            assignments(goal, &steps).unwrap(),
            "- [ ] Alex owns the landing-page draft, due Monday.\n- [ ] Mina reviews the accessibility checklist, due Tuesday.\n- [ ] You must send the finalized screenshots to Alex before Monday."
        );
        // Asked for four owners when the note names three: the model answers.
        assert!(assignments(&goal.replace("three", "four"), &steps).is_none());
        // Instructions without a person are not assignments.
        assert!(assignments(goal, &note("Send the report by Friday. Keep it short.")).is_none());
    }
}
