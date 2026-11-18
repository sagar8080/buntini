//! What a question or a record states outright: days, spans of days, clock times, lengths of
//! time and amounts of money. Code reads these, so no model has to: asked for them, Apple's
//! small model invented days and filters. Every later step of the decision layer
//! (`decision.rs`) works from what this finds.
use chrono::{Datelike, Duration, NaiveDate};

/// What a piece of text states, with its days resolved from `base`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Literals {
    /// Days it names ("tomorrow", "Friday", "3 December"), in the order named.
    pub days: Vec<NaiveDate>,
    /// A run of days it names ("the next two weeks", "this week"), first and last.
    pub span: Option<(NaiveDate, NaiveDate)>,
    /// Clock times, in minutes after midnight.
    pub times: Vec<u32>,
    /// Lengths of time, in minutes ("a two-hour block", "45 min").
    pub minutes: Vec<i64>,
    /// Amounts of money.
    pub amounts: Vec<f64>,
}

pub fn read(text: &str, base: NaiveDate) -> Literals {
    let mut days: Vec<NaiveDate> = crate::task_contract::calendar_days(text, base).iter().filter_map(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()).collect();
    let span = span(text, base).or_else(|| {
        // "This week" or "the weekend": several days in a row.
        let run = days.len() > 1 && days.windows(2).all(|pair| pair[1] - pair[0] == Duration::days(1));
        run.then(|| (days[0], days[days.len() - 1]))
    });
    let counted = ahead(text, base);
    // "A week from today" names that day, not today.
    if !counted.is_empty() {
        days.retain(|day| *day != base);
    }
    for day in counted {
        if !days.contains(&day) {
            days.push(day);
        }
    }
    Literals { days, span, times: times(text), minutes: durations(text), amounts: amounts(text) }
}

fn words(text: &str) -> Vec<String> {
    crate::task_contract::spelled(text)
        .to_lowercase()
        .replace(['’', '‘'], "'")
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '.' && c != ':' && c != '$' && c != '£' && c != '€' && c != '-')
        .map(|word| word.trim_matches(|c: char| ".'-".contains(c)).to_string())
        .filter(|word| !word.is_empty())
        .collect()
}

/// "seven", "a", "couple", "7": a small count written either way.
pub fn count(word: &str) -> Option<i64> {
    const NAMES: [&str; 21] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty"];
    if let Some(at) = NAMES.iter().position(|name| *name == word) {
        return Some(at as i64);
    }
    match word {
        "a" | "an" => Some(1),
        "couple" => Some(2),
        "fortnight" => Some(14),
        "thirty" => Some(30),
        _ => word.parse::<i64>().ok().filter(|number| (0..=400).contains(number)),
    }
}

fn unit_days(word: &str) -> Option<i64> {
    match word.trim_end_matches('s') {
        "day" => Some(1),
        "week" => Some(7),
        "fortnight" => Some(14),
        _ => None,
    }
}

/// A run of days: "the next two weeks", "next 7 days", "the coming week", "the past 3 days".
pub fn span(text: &str, base: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let words = words(text);
    for (at, word) in words.iter().enumerate() {
        let forward = ["next", "coming", "upcoming", "following"].contains(&word.as_str());
        let back = ["last", "past", "previous"].contains(&word.as_str());
        if !forward && !back {
            continue;
        }
        let (number, unit) = match (words.get(at + 1).and_then(|word| count(word)), words.get(at + 2)) {
            (Some(number), Some(unit)) if unit_days(unit).is_some() => (number, unit_days(unit)?),
            _ => match words.get(at + 1).and_then(|unit| unit_days(unit)) {
                // "the coming week", "the next fortnight"; "next week" is the calendar week.
                Some(unit) if word != "next" || unit == 14 => (1, unit),
                _ => continue,
            },
        };
        let length = number * unit;
        if !(1..=120).contains(&length) {
            continue;
        }
        return Some(if forward { (base, base + Duration::days(length - 1)) } else { (base - Duration::days(length), base) });
    }
    None
}

/// Single days counted from now: "in 3 days", "a week from today", "10 days from now".
fn ahead(text: &str, base: NaiveDate) -> Vec<NaiveDate> {
    let words = words(text);
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        let Some(unit) = unit_days(word) else { continue };
        let Some(number) = at.checked_sub(1).and_then(|before| count(&words[before])) else { continue };
        let before = at.checked_sub(2).map(|before| words[before].as_str()).unwrap_or_default();
        let after: Vec<&str> = words.iter().skip(at + 1).take(2).map(String::as_str).collect();
        let from_now = matches!(after.as_slice(), ["from", "today" | "now" | "tomorrow"] | ["from", "today" | "now" | "tomorrow", _]);
        if before == "in" || from_now {
            let start = if after.get(1) == Some(&"tomorrow") { base + Duration::days(1) } else { base };
            found.push(start + Duration::days(number * unit));
        }
    }
    found
}

/// Every clock time: "3pm", "3:30 pm", "15:00", "noon", "midnight". A bare "3" isn't one.
pub fn times(text: &str) -> Vec<u32> {
    let lower = text.to_lowercase().replace(['.', '\u{202f}', '\u{a0}'], " ").replace("a m", "am").replace("p m", "pm");
    let tokens: Vec<&str> = lower.split(|c: char| c.is_whitespace() || ",;()?!–—/".contains(c)).filter(|token| !token.is_empty()).collect();
    let mut found = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        match *token {
            "noon" | "midday" => found.push(12 * 60),
            "midnight" => found.push(0),
            _ => {}
        }
        let token = token.trim_matches(|c: char| !c.is_alphanumeric() && c != ':');
        let digits = token.find(|c: char| !c.is_ascii_digit() && c != ':').unwrap_or(token.len());
        let (number, suffix) = token.split_at(digits);
        if number.is_empty() || number.starts_with(':') || number.ends_with(':') {
            continue;
        }
        let suffix = if suffix.is_empty() { tokens.get(at + 1).copied().filter(|next| ["am", "pm"].contains(next)).unwrap_or("") } else { suffix };
        if !["", "am", "pm", "h"].contains(&suffix) {
            continue;
        }
        let (hour, minute) = number.split_once(':').unwrap_or((number, "0"));
        let (Ok(hour), Ok(minute)) = (hour.parse::<u32>(), minute.parse::<u32>()) else { continue };
        if minute >= 60 || (!number.contains(':') && suffix.is_empty()) || (number.contains(':') && number.split(':').nth(1).is_some_and(|m| m.len() != 2)) {
            continue;
        }
        let hour = match suffix {
            "pm" if (1..12).contains(&hour) => hour + 12,
            "am" if hour == 12 => 0,
            "am" | "pm" if hour > 12 || hour == 0 => continue,
            _ => hour,
        };
        if hour < 24 || (hour == 24 && minute == 0) {
            found.push((hour % 24) * 60 + minute);
        }
    }
    found
}

/// Lengths of time in minutes: "90 minute", "a two-hour block", "2 hours", "half an hour",
/// "1h 30m", "45 min".
pub fn durations(text: &str) -> Vec<i64> {
    let words = words(&text.to_lowercase().replace("half an hour", "30 minutes").replace("half-hour", "30 minutes").replace("quarter of an hour", "15 minutes").replace("an hour and a half", "90 minutes"));
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        // "two-hour", "90-minute", "1h", "30m", "45min"
        let (number, unit) = match word.split_once('-') {
            Some((number, unit)) => (count(number), unit.to_string()),
            None => {
                let digits = word.find(|c: char| !c.is_ascii_digit()).unwrap_or(word.len());
                if digits > 0 && digits < word.len() {
                    (word[..digits].parse::<i64>().ok(), word[digits..].to_string())
                } else {
                    (at.checked_sub(1).and_then(|before| count(&words[before])), word.clone())
                }
            }
        };
        let Some(number) = number else { continue };
        let scale = match unit.as_str() {
            "minute" | "minutes" | "min" | "mins" | "m" => 1,
            "hour" | "hours" | "hr" | "hrs" | "h" => 60,
            _ => continue,
        };
        if number > 0 {
            found.push(number * scale);
        }
    }
    found
}

/// Lengths as a reply writes them, with "3 hours 30 minutes" or "1 h and 15 min" as one.
pub fn durations_whole(text: &str) -> Vec<i64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || ",;()".contains(c)).filter(|word| !word.is_empty()).collect();
    let mut found = Vec::new();
    let mut at = 0;
    let unit_of = |word: &str| -> Option<i64> {
        match word.trim_end_matches(['.', '!', '?']) {
            "minute" | "minutes" | "min" | "mins" | "m" => Some(1),
            "hour" | "hours" | "hr" | "hrs" | "h" => Some(60),
            _ => None,
        }
    };
    // (number, unit) at a word: "3 hours", "3h", "two-hour".
    let quantity = |at: usize| -> Option<(i64, usize)> {
        let word = words.get(at)?.trim_end_matches(['.', '!', '?']);
        if let Some((number, unit)) = word.split_once('-') {
            return Some((count(number)? * unit_of(unit)?, 1));
        }
        let digits = word.find(|c: char| !c.is_ascii_digit()).unwrap_or(word.len());
        if digits > 0 && digits < word.len() {
            return Some((word[..digits].parse::<i64>().ok()? * unit_of(&word[digits..])?, 1));
        }
        let number = word.parse::<f64>().ok().or_else(|| count(word).map(|number| number as f64))?;
        let unit = unit_of(words.get(at + 1)?)?;
        Some(((number * unit as f64).round() as i64, 2))
    };
    while at < words.len() {
        let Some((mut minutes, used)) = quantity(at) else {
            at += 1;
            continue;
        };
        at += used;
        // "and 30 minutes" or "30 minutes" right after hours belongs to them.
        let skip = usize::from(words.get(at) == Some(&"and"));
        if minutes % 60 == 0 {
            if let Some((more, used)) = quantity(at + skip).filter(|(more, _)| *more < 60) {
                minutes += more;
                at += skip + used;
            }
        }
        if minutes > 0 {
            found.push(minutes);
        }
    }
    found
}

/// Amounts of money: "$1,240", "£3.50", "€12", "18.40 dollars".
pub fn amounts(text: &str) -> Vec<f64> {
    let mut found = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        if ['$', '£', '€'].contains(&chars[at]) {
            let mut end = at + 1;
            while end < chars.len() && (chars[end].is_ascii_digit() || (chars[end] == ',' && chars.get(end + 1).is_some_and(char::is_ascii_digit)) || (chars[end] == '.' && chars.get(end + 1).is_some_and(char::is_ascii_digit))) {
                end += 1;
            }
            let number: String = chars[at + 1..end].iter().filter(|c| **c != ',').collect();
            if let Ok(value) = number.parse::<f64>() {
                found.push(value);
            }
            at = end;
        } else {
            at += 1;
        }
    }
    found
}

/// Dates a record states relative to its own day ("in 4 days", "the day after tomorrow",
/// "on Friday"), each as written and as a date. An email that says "tomorrow" means the day
/// after it was sent, not after today.
pub fn relative_dates(text: &str, sent: NaiveDate) -> Vec<(String, NaiveDate)> {
    let lower = text.to_lowercase().replace(['’', '‘'], "'");
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|word| !word.is_empty()).collect();
    const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    let mut found: Vec<(String, NaiveDate)> = Vec::new();
    let mut add = |phrase: String, day: NaiveDate| {
        if !found.iter().any(|(seen, _)| *seen == phrase) {
            found.push((phrase, day));
        }
    };
    for (at, word) in words.iter().enumerate() {
        let before = |back: usize| at.checked_sub(back).map(|index| words[index]).unwrap_or_default();
        match *word {
            "tomorrow" if before(1) == "after" && before(2) == "day" => add("the day after tomorrow".into(), sent + Duration::days(2)),
            "tomorrow" => add("tomorrow".into(), sent + Duration::days(1)),
            "today" | "tonight" => add((*word).into(), sent),
            "yesterday" => add("yesterday".into(), sent - Duration::days(1)),
            word if WEEKDAYS.contains(&word) => {
                let wanted = WEEKDAYS.iter().position(|name| *name == word).unwrap_or(0) as i64;
                let from = i64::from(sent.weekday().num_days_from_monday());
                // The coming one: a weekday named on that same weekday means a week later.
                let ahead = match (wanted - from).rem_euclid(7) {
                    0 => 7,
                    ahead => ahead,
                };
                let phrase = if before(1) == "next" { format!("next {word}") } else { word.to_string() };
                add(phrase, sent + Duration::days(ahead));
            }
            _ => {}
        }
        if let Some(unit) = unit_days(word) {
            if let Some(number) = at.checked_sub(1).and_then(|index| count(words[index])) {
                let opener = before(2);
                let after: Vec<&str> = words.iter().skip(at + 1).take(2).copied().collect();
                let later = matches!(after.as_slice(), ["from", "now" | "today"] | ["from", "now" | "today", _] | ["time"] | ["time", _]);
                if opener == "in" || later {
                    let written = if opener == "in" { format!("in {} {word}", words[at - 1]) } else { format!("{} {word} {}", words[at - 1], after.join(" ")) };
                    add(written.trim().to_string(), sent + Duration::days(number * unit));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn spans_and_days_are_read_from_the_words() {
        let base = day("2026-11-17");
        assert_eq!(span("how many choir rehearsals are left in the next two weeks", base), Some((base, day("2026-11-30"))));
        assert_eq!(span("list my personal stuff for the next 7 days incl today", base), Some((base, day("2026-11-23"))));
        assert_eq!(span("anything in the past 3 days", base), Some((day("2026-11-14"), base)));
        assert_eq!(span("what's on next week", base), None);
        assert_eq!(read("have i got anything on a week from today", base).days, vec![day("2026-11-24")]);
        assert_eq!(read("what about in 3 days", base).days, vec![day("2026-11-20")]);
        let week = read("anything this week", base);
        assert_eq!(week.span, Some((base, day("2026-11-22"))));
    }

    #[test]
    fn clock_times_lengths_and_amounts() {
        assert_eq!(times("whats the earliest 90 minute slot i can grab tomorrow between 9am and 6pm"), vec![9 * 60, 18 * 60]);
        assert_eq!(times("am I free at 3:30 pm or 15:45?"), vec![15 * 60 + 30, 15 * 60 + 45]);
        assert_eq!(times("is it before noon"), vec![12 * 60]);
        assert!(times("I have 3 meetings on 12 Nov").is_empty());
        assert_eq!(durations("whats the earliest 90 minute slot"), vec![90]);
        assert_eq!(durations("a two-hour block, or half an hour"), vec![120, 30]);
        assert_eq!(durations("1h 30m"), vec![60, 30]);
        assert_eq!(amounts("Your invoice of $1,240 is due; the fee is £3.50."), vec![1240.0, 3.5]);
    }

    #[test]
    fn an_email_says_its_dates_from_its_own_day() {
        let sent = day("2026-11-13"); // a Friday
        let found = relative_dates("The renewal call is in 4 days. Bring it the day after tomorrow, or on Tuesday.", sent);
        assert!(found.contains(&("in 4 days".into(), day("2026-11-17"))));
        assert!(found.contains(&("the day after tomorrow".into(), day("2026-11-15"))));
        assert!(found.contains(&("tuesday".into(), day("2026-11-17"))));
        assert!(!found.iter().any(|(phrase, _)| phrase == "tomorrow"));
    }
}
