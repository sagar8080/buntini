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

/// Compare two quotes whose excerpts give one explicit euro price and say
/// whether assembly is included. Leave other quote formats to the answer phase.
pub(crate) fn compare_vendor_quotes(goal: &str, files: &[crate::types::FileHit]) -> Option<String> {
    let ask = crate::completion::unquoted(goal);
    if !ask.starts_with("compare ") || !ask.contains("quotes") || !ask.contains("by how many euros")
        || !ask.contains("assembly") || crate::completion::requested_actions(goal) != (vec![], false)
    {
        return None;
    }
    let [first, second] = files else { return None; };
    let read = |file: &crate::types::FileHit| {
        let excerpt = file.excerpt.as_deref()?;
        let (label, values) = excerpt.split_once(':')?;
        if !label.to_ascii_lowercase().contains("vendor quote") {
            return None;
        }
        let (name, values) = values.trim().split_once(" costs ")?;
        if name.is_empty() || !name.chars().all(|ch| ch.is_alphanumeric() || ch == ' ' || ch == '-') {
            return None;
        }
        let (price, terms) = values.split_once(" euros")?;
        let price = price.trim().parse::<u64>().ok()?;
        let included = match terms.trim_matches([' ', ',', '.']).to_ascii_lowercase().as_str() {
            "including assembly" | "with assembly included" => true,
            "excluding assembly" | "with assembly not included" => false,
            _ => return None,
        };
        Some((name.to_string(), price, included))
    };
    let a = read(first)?;
    let b = read(second)?;
    if a.0.eq_ignore_ascii_case(&b.0) {
        return None;
    }
    let describe = |quote: &(String, u64, bool)| {
        let key = if quote.2 { "chat.answer.quoteAssemblyIncluded" } else { "chat.answer.quoteAssemblyExcluded" };
        crate::i18n::tf(key, &[("vendor", &quote.0), ("price", &quote.1)])
    };
    let comparison = if a.1 == b.1 {
        crate::i18n::t("chat.answer.quotePriceTie")
    } else {
        let cheaper = if a.1 < b.1 { &a.0 } else { &b.0 };
        crate::i18n::tf("chat.answer.quoteCheaper", &[("vendor", cheaper), ("difference", &a.1.abs_diff(b.1))])
    };
    Some(format!("{comparison} {} {}", describe(&a), describe(&b)))
}

/// The first worked answer that applies to this request and journal.
pub fn answer(goal: &str, steps: &[Step], today: NaiveDate) -> Option<String> {
    itinerary(goal)
        .filter(|_| steps.iter().all(|s| s.state != "succeeded" || s.effect))
        .or_else(|| rule_choice(goal, steps, today))
        .or_else(|| assignments(goal, steps))
        .or_else(|| conversion(goal))
        .or_else(|| countdown(goal, today))
        .or_else(|| text_work(goal))
}

/// Text the question itself gives, worked on as asked, which Apple's model got wrong on two
/// macOS versions: a word spelled backwards, words in title case, a letter counted, a number
/// with thousands separators, a clock time in the other format.
fn text_work(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase().replace(['’', '‘'], "'");
    let quoted = quoted_text(goal);
    let words: Vec<String> = lower.split(|c: char| c.is_whitespace()).map(|word| word.trim_matches(|c: char| ",.?!:;\"“”".contains(c)).to_string()).filter(|word| !word.is_empty()).collect();
    let at = |word: &str| words.iter().position(|seen| seen == word);
    // "spell accommodate backwards", "spell 'calendar' backwards", "reverse the word lemon".
    if lower.contains("backwards") || lower.contains("backward") || words.first().is_some_and(|word| word == "reverse") {
        let target = quoted.clone().or_else(|| {
            let start = at("spell").or_else(|| at("reverse"))? + 1;
            let end = words.iter().position(|word| word.starts_with("backward")).unwrap_or(words.len());
            let picked: Vec<&String> = words.get(start..end)?.iter().filter(|word| !["the", "word", "name"].contains(&word.as_str())).collect();
            (picked.len() == 1).then(|| picked[0].trim_matches('\'').to_string())
        })?;
        let reversed: String = target.chars().rev().collect();
        return Some(format!("“{target}” backwards is **{reversed}**."));
    }
    // "Put 'the quick brown fox' into title case."
    if lower.contains("title case") {
        let target = quoted?;
        let titled: Vec<String> = target.split(' ').map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
        }).collect();
        return Some(format!("**{}**", titled.join(" ")));
    }
    // "how many e's are in tennessee", "how many times does r appear in strawberry".
    if let Some(many) = words.windows(2).position(|pair| pair[0] == "how" && pair[1] == "many") {
        let letter = words.get(many + 2).and_then(|word| {
            let bare = word.trim_end_matches("'s").trim_end_matches('s');
            (bare.chars().count() == 1 && bare.chars().all(char::is_alphabetic)).then(|| bare.chars().next()).flatten()
        }).or_else(|| (words.get(many + 2).map(String::as_str) == Some("times")).then(|| words.iter().position(|word| word == "letter").and_then(|at| words.get(at + 1)).and_then(|word| word.chars().next())).flatten());
        let target = quoted.clone().or_else(|| words.iter().rposition(|word| word == "in").and_then(|at| words.get(at + 1)).filter(|word| word.chars().all(char::is_alphabetic)).cloned());
        if let (Some(letter), Some(target)) = (letter, target) {
            let count = target.to_lowercase().chars().filter(|c| *c == letter).count();
            return Some(format!("There {} **{count}** {letter}’s in “{target}”.", if count == 1 { "is" } else { "are" }));
        }
    }
    // "Write 1234567 with commas as thousands separators."
    if lower.contains("comma") || lower.contains("thousands") {
        let number = words.iter().find(|word| word.len() >= 4 && word.chars().all(|c| c.is_ascii_digit()))?;
        let digits: Vec<char> = number.chars().collect();
        let mut grouped = String::new();
        for (at, digit) in digits.iter().enumerate() {
            if at > 0 && (digits.len() - at) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(*digit);
        }
        return Some(format!("**{grouped}**"));
    }
    // "21:45 in 12 hour time", "what's 9:45 pm in 24-hour time".
    let twelve = lower.contains("12 hour") || lower.contains("12-hour") || lower.contains("am/pm");
    let twenty_four = lower.contains("24 hour") || lower.contains("24-hour");
    // "Tomorrow's calendar with am/pm times" is about their day; only a time written out is converted.
    let theirs = ["my", "tomorrow", "today", "tomorrow's", "today's", "calendar", "meetings", "events"].iter().any(|word| words.iter().any(|seen| seen == word));
    if (twelve || twenty_four) && !theirs {
        let times = crate::literals::times(goal);
        let [minutes] = times.as_slice() else { return None };
        let (hour, minute) = (minutes / 60, minutes % 60);
        return Some(if twelve {
            let half = if hour < 12 { "am" } else { "pm" };
            let shown = match hour % 12 { 0 => 12, other => other };
            format!("**{shown}:{minute:02} {half}**")
        } else {
            format!("**{hour:02}:{minute:02}**")
        });
    }
    None
}

/// The text a question puts in quotes: 'calendar', "the quick brown fox", “fox”. A quote mark
/// inside a word ("what's") opens nothing.
fn quoted_text(goal: &str) -> Option<String> {
    for (open, close) in [('"', '"'), ('“', '”'), ('‘', '’'), ('\'', '\'')] {
        for (start, _) in goal.match_indices(open) {
            if goal[..start].chars().last().is_some_and(char::is_alphanumeric) {
                continue;
            }
            let rest = &goal[start + open.len_utf8()..];
            let Some(end) = rest.find(close) else { continue };
            let text = rest[..end].trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// A unit, its kind and how many of the kind's base unit it is. Cups, pints, quarts and
/// gallons are US measures, and say so.
fn unit(word: &str) -> Option<(&'static str, f64, &'static str)> {
    let word = word.trim_end_matches('.');
    Some(match word {
        "mm" | "millimetre" | "millimetres" | "millimeter" | "millimeters" => ("length", 0.001, "mm"),
        "cm" | "centimetre" | "centimetres" | "centimeter" | "centimeters" => ("length", 0.01, "cm"),
        "m" | "metre" | "metres" | "meter" | "meters" => ("length", 1.0, "metres"),
        "km" | "kms" | "kilometre" | "kilometres" | "kilometer" | "kilometers" => ("length", 1000.0, "km"),
        "in" | "inch" | "inches" => ("length", 0.0254, "inches"),
        "ft" | "foot" | "feet" => ("length", 0.3048, "feet"),
        "yd" | "yard" | "yards" => ("length", 0.9144, "yards"),
        "mi" | "mile" | "miles" => ("length", 1609.344, "miles"),
        "mg" | "milligram" | "milligrams" => ("mass", 0.001, "mg"),
        "g" | "gram" | "grams" | "gramme" | "grammes" => ("mass", 1.0, "grams"),
        "kg" | "kgs" | "kilo" | "kilos" | "kilogram" | "kilograms" => ("mass", 1000.0, "kg"),
        "oz" | "ounce" | "ounces" => ("mass", 28.349523125, "ounces"),
        "lb" | "lbs" | "pound" | "pounds" => ("mass", 453.59237, "pounds"),
        "stone" | "stones" | "st" => ("mass", 6350.29318, "stone"),
        "ml" | "millilitre" | "millilitres" | "milliliter" | "milliliters" => ("volume", 1.0, "ml"),
        "l" | "litre" | "litres" | "liter" | "liters" => ("volume", 1000.0, "litres"),
        "tsp" | "teaspoon" | "teaspoons" => ("volume", 4.92892159375, "teaspoons"),
        "tbsp" | "tablespoon" | "tablespoons" => ("volume", 14.78676478125, "tablespoons"),
        "cup" | "cups" => ("volume", 236.5882365, "US cups"),
        "pint" | "pints" => ("volume", 473.176473, "US pints"),
        "quart" | "quarts" => ("volume", 946.352946, "US quarts"),
        "gallon" | "gallons" => ("volume", 3785.411784, "US gallons"),
        "c" | "celsius" | "centigrade" => ("temperature", 0.0, "°C"),
        "f" | "fahrenheit" => ("temperature", 1.0, "°F"),
        "k" | "kelvin" => ("temperature", 2.0, "K"),
        _ => return None,
    })
}

fn amount(word: &str) -> Option<f64> {
    match word {
        "a" | "an" | "one" => Some(1.0),
        "half" => Some(0.5),
        _ => word.replace(',', "").parse::<f64>().ok().filter(|value| value.is_finite() && value.abs() < 1e12),
    }
}

fn shown(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "how many ounces in 2 pounds", "convert 5 miles to km", "72 fahrenheit in celsius".
/// Pounds that become dollars are money, not weight, and aren't answered here.
fn conversion(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase().replace(['?', '!'], " ").replace("°", " ").replace("degrees", " ").replace("fl oz", "floz");
    let words: Vec<&str> = lower.split_whitespace().collect();
    // From and to: "N from in|to to", or "how many to in|are in N from".
    let (value, from, to) = if let Some(at) = words.iter().position(|word| *word == "many") {
        let to = unit(words.get(at + 1)?)?;
        let mut rest = &words[at + 2..];
        while rest.first().is_some_and(|word| ["are", "is", "in", "per", "there"].contains(word)) {
            rest = &rest[1..];
        }
        let value = amount(rest.first()?)?;
        (value, unit(rest.get(1)?)?, to)
    } else {
        let at = words.iter().position(|word| amount(word).is_some_and(|_| !["a", "an", "one", "half"].contains(word)))?;
        let value = amount(words[at])?;
        let from = unit(words.get(at + 1)?)?;
        let link = words.get(at + 2)?;
        if !["in", "to", "into", "as"].contains(link) {
            return None;
        }
        (value, from, unit(words.get(at + 3)?)?)
    };
    if from.0 != to.0 || from.2 == to.2 {
        return None;
    }
    let result = if from.0 == "temperature" {
        let celsius = match from.1 as u8 { 0 => value, 1 => (value - 32.0) * 5.0 / 9.0, _ => value - 273.15 };
        match to.1 as u8 { 0 => celsius, 1 => celsius * 9.0 / 5.0 + 32.0, _ => celsius + 273.15 }
    } else {
        value * from.1 / to.1
    };
    // "1 litre", "1 foot": one of a unit is singular.
    let named = |count: f64, name: &str| -> String {
        if shown(count) != "1" {
            return name.to_string();
        }
        match name {
            "inches" => "inch".into(),
            "feet" => "foot".into(),
            _ => name.strip_suffix('s').unwrap_or(name).to_string(),
        }
    };
    let (from_name, to_name) = (named(value, from.2), named(result, to.2));
    let joiner = if from.0 == "temperature" { "" } else { " " };
    Some(format!("{}{joiner}{from_name} is {}{joiner}{to_name}.", shown(value), shown(result)))
}

/// "how many days until christmas", "how long till friday": counted from today.
fn countdown(goal: &str, today: NaiveDate) -> Option<String> {
    let lower = goal.to_lowercase().replace(['?', '!', '’'], " ").replace('\'', " ");
    let rest = ["days until ", "days till ", "days til ", "days to ", "days before ", "long until ", "long till ", "long til "]
        .iter()
        .find_map(|cue| lower.split_once(cue).map(|(_, rest)| rest.trim().to_string()))?;
    let rest = rest.trim_start_matches("it's ").trim_start_matches("its ").trim_start_matches("it is ");
    let year = today.year();
    let fixed = |month: u32, day: u32| {
        NaiveDate::from_ymd_opt(year, month, day).filter(|date| *date >= today).or_else(|| NaiveDate::from_ymd_opt(year + 1, month, day))
    };
    let (name, date) = if rest.starts_with("christmas eve") {
        ("Christmas Eve".to_string(), fixed(12, 24)?)
    } else if rest.starts_with("christmas") || rest.starts_with("xmas") {
        ("Christmas".to_string(), fixed(12, 25)?)
    } else if rest.starts_with("new year s eve") || rest.starts_with("new years eve") {
        ("New Year’s Eve".to_string(), fixed(12, 31)?)
    } else if rest.starts_with("new year") {
        ("New Year’s Day".to_string(), NaiveDate::from_ymd_opt(year + 1, 1, 1)?)
    } else if rest.starts_with("halloween") {
        ("Halloween".to_string(), fixed(10, 31)?)
    } else if rest.starts_with("valentine") {
        ("Valentine’s Day".to_string(), fixed(2, 14)?)
    } else if rest.starts_with("my ") {
        return None;
    } else {
        let when = crate::request_fields::when(rest, today);
        (String::new(), when.date?)
    };
    let days = (date - today).num_days();
    let day = date.format("%A %-d %B").to_string();
    Some(match (days, name.is_empty()) {
        (0, true) => format!("That’s today, {day}."),
        (0, false) => format!("{name} is today."),
        (1, true) => format!("1 day, until {day}."),
        (1, false) => format!("1 day until {name}, {day}."),
        (_, true) => format!("{days} days, until {day}."),
        (_, false) => format!("{days} days until {name}, {day}."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_the_question_gives_is_worked_on_by_rust() {
        let worked = |goal: &str| text_work(goal).unwrap_or_default().replace("**", "");
        assert_eq!(worked("Spell 'calendar' backwards."), "“calendar” backwards is radnelac.");
        assert_eq!(worked("spell accommodate backwards"), "“accommodate” backwards is etadommocca.");
        assert_eq!(worked("Put 'the quick brown fox' into title case."), "The Quick Brown Fox");
        assert_eq!(worked("how many e's are in tennessee"), "There are 4 e’s in “tennessee”.");
        assert_eq!(worked("how many r's in strawberry?"), "There are 3 r’s in “strawberry”.");
        assert_eq!(worked("Write 1234567 with commas as thousands separators."), "1,234,567");
        assert_eq!(worked("21:45 in 12 hour time?"), "9:45 pm");
        assert_eq!(worked("what's 9:45 pm in 24-hour time"), "21:45");
        assert_eq!(worked("what's 00:30 in 12-hour time"), "12:30 am");
        for other in ["what's on my calendar backwards compatible", "how many emails did I get", "what time is it", "write a note with commas", "tomorrow's work calendar pls, with am/pm times, not 24h", "list my meetings at 9:30 in 12-hour time"] {
            assert_eq!(text_work(other), None, "{other}");
        }
    }

    use serde_json::json;

    #[test]
    fn units_and_days_are_worked_out() {
        assert_eq!(conversion("how many ounces in 2 pounds").as_deref(), Some("2 pounds is 32 ounces."));
        assert_eq!(conversion("convert 5 miles to km").as_deref(), Some("5 miles is 8.05 km."));
        assert_eq!(conversion("what's 72 fahrenheit in celsius").as_deref(), Some("72°F is 22.22°C."));
        assert_eq!(conversion("how many cups in a litre").as_deref(), Some("1 litre is 4.23 US cups."));
        assert_eq!(conversion("convert 250 pounds to dollars"), None);
        assert_eq!(conversion("how many people live in london"), None);
        let saturday = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        assert_eq!(countdown("how many days until christmas", saturday).as_deref(), Some("48 days until Christmas, Friday 25 December."));
        assert_eq!(countdown("how long until friday?", saturday).as_deref(), Some("6 days, until Friday 13 November."));
        assert_eq!(countdown("how many days until my birthday", saturday), None);
    }

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
