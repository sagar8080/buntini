//! Bind explicitly named note and reminder fields to the user's literal instruction.
//! This small grammar performs no date inference and still uses normal approval.
use crate::{actions, planning::BoundAction, workflows::Step};
use serde_json::{json, Value};

fn quote_end(ch: char, preceding: Option<char>) -> Option<char> {
    match ch {
        '"' => Some('"'),
        '“' => Some('”'),
        '‘' => Some('’'),
        '`' => Some('`'),
        '\'' if !preceding.is_some_and(char::is_alphanumeric) => Some('\''),
        _ => None,
    }
}

/// Byte positions outside quotation marks, without interpreting quoted commands.
fn outside(text: &str) -> Result<Vec<usize>, String> {
    let mut positions = Vec::new();
    let mut quote = None;
    let mut preceding = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if let Some(end) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == end {
                quote = None;
            }
        } else if let Some(end) = quote_end(ch, preceding) {
            quote = Some(end);
        } else {
            positions.push(index);
        }
        preceding = Some(ch);
    }
    if quote.is_some() {
        return Err(
            "Close the quotation marks around the reminder's title or date before continuing."
                .into(),
        );
    }
    Ok(positions)
}

fn starts(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn word<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if !starts(text, prefix) {
        return None;
    }
    let tail = &text[prefix.len()..];
    if tail.is_empty() || tail.starts_with(char::is_whitespace) {
        Some(tail.trim_start())
    } else {
        None
    }
}

fn clauses(text: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut begin = 0;
    for index in outside(text)? {
        if index < begin {
            continue;
        }
        let tail = &text[index..];
        let separator = if starts(tail, ", then ") {
            Some(7)
        } else if starts(tail, " and then ") {
            Some(10)
        } else if tail.starts_with(';') || tail.starts_with('\n') {
            Some(1)
        } else {
            None
        };
        if let Some(length) = separator {
            parts.push(text[begin..index].trim());
            begin = index + length;
        }
    }
    parts.push(text[begin..].trim());
    Ok(parts)
}

fn literal(text: &str) -> Result<String, String> {
    let text = text.trim();
    let Some(first) = text.chars().next() else {
        return Err("The reminder needs both its exact title and due time.".into());
    };
    if let Some(end) = quote_end(first, None) {
        if text.chars().count() < 2 || !text.ends_with(end) {
            return Err("Keep the title or date entirely inside its quotation marks, with no extra instructions after it.".into());
        }
        let inner = &text[first.len_utf8()..text.len() - end.len_utf8()];
        // Escaped values require another interpretation layer. Ask for plain
        // literal fields rather than silently changing the user's characters.
        if inner.contains('\\') || inner.contains(end) {
            return Err("Use one pair of plain quotation marks without escape sequences for this reminder field.".into());
        }
        Ok(inner.to_string())
    } else {
        Ok(text.to_string())
    }
}

fn instruction(clause: &str) -> &str {
    let mut text = clause.trim();
    loop {
        let previous = text;
        for prefix in [
            "please",
            "can you",
            "could you",
            "would you",
            "i want you to",
            "i need you to",
            "then",
        ] {
            if let Some(rest) = word(text, prefix) {
                text = rest;
                break;
            }
        }
        if text == previous {
            break;
        }
    }
    text
}

fn begins_request(text: &str) -> bool {
    let text = instruction(text);
    [
        "read",
        "find",
        "search",
        "review",
        "check",
        "open",
        "summarize",
        "summarise",
        "extract",
        "tell",
        "give",
        "create",
        "save",
        "write",
        "add",
        "make",
        "take",
        "set",
        "schedule",
        "remind me",
        "do not",
        "don't",
        "never",
        "explain",
        "show me how",
    ]
    .iter()
    .any(|prefix| word(text, prefix).is_some())
}

/// Keep quoted fields intact, and scope negation/explanation over coordinated
/// clauses. Only the original user goal enters this parser, never tool text.
fn literal_sentences(goal: &str) -> Option<Vec<&str>> {
    let mut sentences = Vec::new();
    let mut begin = 0;
    for index in outside(goal).ok()? {
        let tail = &goal[index..];
        let character = tail.chars().next()?;
        let period = character == '.' && tail[1..].chars().next().is_none_or(char::is_whitespace);
        if [';', '\n', '!', '?'].contains(&character) || period {
            sentences.push(goal[begin..index].trim());
            begin = index + character.len_utf8();
        }
    }
    sentences.push(goal[begin..].trim());
    Some(sentences)
}

fn literal_requests(goal: &str) -> Option<Vec<&str>> {
    requests_in(goal, false)
}

/// The request after a sentence's opening that says when ("On May 20, 2031, remind
/// me …", "Tomorrow, set …"), or the sentence as it is.
fn after_opening(sentence: &str) -> Option<&str> {
    if begins_request(sentence) {
        return Some(sentence);
    }
    let first = sentence.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    let when = [
        "on", "at", "by", "before", "after", "today", "tonight", "tomorrow", "this", "next",
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "january",
        "february", "march", "april", "may", "june", "july", "august", "september", "october",
        "november", "december",
    ]
    .contains(&first.trim_end_matches(','));
    let start = outside(sentence).ok()?.into_iter().find_map(|index| {
        let tail = &sentence[index..];
        (tail.starts_with(", ") && begins_request(&tail[2..])).then_some(index + 2)
    });
    Some(start.filter(|_| when).map_or(sentence, |start| &sentence[start..]))
}

/// With `dated_openings`, a request that opens with when it happens counts from its
/// verb. Only titles use that: a due time bound without its opening would lose its date.
fn requests_in(goal: &str, dated_openings: bool) -> Option<Vec<&str>> {
    let mut requests = Vec::new();
    for sentence in literal_sentences(goal)? {
        let sentence = if dated_openings { after_opening(sentence)? } else { sentence };
        let plain = crate::completion::unquoted(sentence);
        if crate::completion::non_action(crate::completion::request_clause(&plain))
            || !begins_request(sentence)
        {
            continue;
        }
        let mut begin = 0;
        let mut parts = Vec::new();
        for index in outside(sentence).ok()? {
            if index < begin {
                continue;
            }
            let tail = &sentence[index..];
            if let Some(separator) = [
                ", then ",
                " and then ",
                " then ",
                ", and ",
                " and ",
                " also ",
                ", ",
            ]
            .iter()
            .find(|separator| starts(tail, separator) && begins_request(&tail[separator.len()..]))
            {
                parts.push(sentence[begin..index].trim());
                begin = index + separator.len();
            }
        }
        parts.push(sentence[begin..].trim());
        for part in parts {
            let plain = crate::completion::unquoted(part);
            if crate::completion::non_action(crate::completion::request_clause(&plain)) {
                break;
            }
            requests.push(part);
        }
    }
    Some(requests)
}

fn is_word_at(text: &str, index: usize, keyword: &str) -> bool {
    (index == 0 || text[..index].ends_with(char::is_whitespace))
        && literal_word(&text[index..], keyword).is_some()
}

// Literal fields may be followed immediately by a clause delimiter. Keep this
// separate from the executable-field grammar's whitespace-only `word` helper:
// a filename such as "note.txt" or a word such as "notebook" is not a delimiter.
fn literal_word<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    if !starts(text, keyword) {
        return None;
    }
    let tail = &text[keyword.len()..];
    let boundary = tail.is_empty()
        || tail.starts_with(char::is_whitespace)
        || tail.starts_with([',', ';', ':', '!', '?'])
        || (tail.starts_with('.') && tail[1..].chars().next().is_none_or(char::is_whitespace));
    boundary.then(|| tail.trim_start())
}

fn title_marker<'a>(text: &'a str, kind: Option<&str>) -> Option<&'a str> {
    let mut text = text.trim();
    for prefix in ["with", "using", "use"] {
        if let Some(rest) = word(text, prefix) {
            text = rest;
            break;
        }
    }
    if let Some(rest) = word(text, "exactly") {
        text = rest;
    }
    if let Some(rest) = word(text, "the") {
        text = rest;
    }
    if let Some(kind) = kind {
        if let Some(rest) = word(text, kind) {
            text = rest;
        }
    }
    let mut value = ["titled", "named", "called", "title"]
        .iter()
        .find_map(|marker| word(text, marker))?;
    if let Some(rest) = word(value, "is") {
        value = rest;
    }
    if let Some(rest) = word(value, "exactly") {
        value = rest;
    }
    Some(value)
}

fn field_prefix(text: &str, max: usize, read: bool) -> Option<String> {
    let text = text.trim();
    let first = text.chars().next()?;
    if let Some(end) = quote_end(first, None) {
        let finish = text
            .char_indices()
            .skip(1)
            .find_map(|(index, ch)| (ch == end).then_some(index))?;
        let value = literal(&text[..finish + end.len_utf8()]).ok()?;
        let rest = text[finish + end.len_utf8()..].trim_start();
        if rest.starts_with(['"', '“', '‘', '`', '\'']) {
            return None;
        }
        return (!value.trim().is_empty()
            && value.chars().count() <= max
            && !value.chars().any(|ch| ch.is_control()))
        .then_some(value);
    }
    let mut finish = text.len();
    for index in outside(text).ok()? {
        let tail = &text[index..];
        let character = tail.chars().next()?;
        let punctuation = [',', ';', '\n', '!', '?'].contains(&character)
            || (character == '.' && tail[1..].chars().next().is_none_or(char::is_whitespace));
        let metadata = [
            "with body",
            "with the body",
            "whose body",
            "that summarizes",
            "that summarises",
            "which summarizes",
            "which summarises",
            "using the",
            "using that",
            "based on",
            "from that",
            "from the",
            "with exactly",
            "with the title",
            "with title",
            "due",
            "starting",
            "lasting",
            "at the time",
            "on the date",
            "at its",
            "on its",
            "with location",
            "in the same",
        ]
        .iter()
        .any(|marker| is_word_at(text, index, marker));
        let dated = ["on ", "at ", "for "].iter().any(|prefix| {
            starts(tail, prefix)
                && (index == 0 || text[..index].ends_with(char::is_whitespace))
                && tail[prefix.len()..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit())
        });
        let coordinated = [" and ", " then ", " also "].iter().any(|prefix| {
            starts(tail, prefix)
                && (read
                    || begins_request(&tail[prefix.len()..])
                    || ["body", "due", "start", "location", "duration"]
                        .iter()
                        .any(|field| word(&tail[prefix.len()..], field).is_some()))
        });
        let read_scope = read
            && [
                "in notes",
                "in my notes",
                "in the notes app",
                "in apple notes",
                "from notes",
            ]
            .iter()
            .any(|scope| is_word_at(text, index, scope));
        if punctuation || metadata || dated || coordinated || read_scope {
            finish = index;
            break;
        }
    }
    let value = text[..finish].trim().trim_end_matches('.').trim();
    (!value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}

fn without_article(mut text: &str) -> &str {
    loop {
        if let Some(rest) = ["a", "an", "the", "my", "this", "new", "existing", "another"]
            .iter()
            .find_map(|article| word(text, article))
        {
            text = rest;
        } else {
            return text;
        }
    }
}

/// An exact user-supplied note identifier/title, suitable for pinning a read
/// query. This is not a request executor, a source lookup or permission grant.
pub(crate) fn read_query(goal: &str) -> Option<String> {
    let mut queries = Vec::new();
    let mut note_reads = 0;
    for request in literal_requests(goal)? {
        let text = instruction(request);
        let Some(rest) = [
            "read",
            "find",
            "search",
            "review",
            "check",
            "open",
            "summarize",
            "summarise",
        ]
        .iter()
        .find_map(|verb| word(text, verb)) else {
            continue;
        };
        let rest = word(rest, "for").unwrap_or(rest);
        let mut prefix = without_article(rest);
        if let Some(tail) = word(prefix, "note").or_else(|| word(prefix, "notes")) {
            note_reads += 1;
            let value = [
                "mentioning",
                "containing",
                "that mentions",
                "which mentions",
            ]
            .iter()
            .find_map(|marker| word(tail, marker))
            .or_else(|| title_marker(tail, None));
            if let Some(value) = value {
                if let Some(query) =
                    field_prefix(value, 500, true).filter(|query| query.len() <= 500)
                {
                    queries.push(query);
                }
            }
            continue;
        }
        // "my <literal title> note" is distinct from a later new-note title.
        if word(rest, "my").is_some() {
            prefix = word(rest, "my").unwrap();
            if let Some(index) = outside(prefix)
                .ok()?
                .into_iter()
                .find(|index| is_word_at(prefix, *index, "note"))
            {
                note_reads += 1;
                let tail = literal_word(&prefix[index..], "note")?;
                let identified = ["mentioning", "containing", "that mentions"]
                    .iter()
                    .find_map(|marker| word(tail, marker));
                let query = if let Some(value) = identified {
                    field_prefix(value, 500, true)
                } else {
                    literal(prefix[..index].trim()).ok()
                };
                if let Some(query) = query.filter(|query| !query.is_empty() && query.len() <= 500) {
                    queries.push(query);
                }
            }
        }
    }
    (note_reads == 1 && queries.len() == 1).then(|| queries.remove(0))
}

fn action_tail<'a>(clause: &'a str, tool: &str) -> Option<&'a str> {
    let text = instruction(clause);
    let verbs: &[&str] = match tool {
        "create_note" => &["create", "save", "write", "add", "make", "take"],
        "add_reminder" => &["add", "create", "set", "make"],
        "add_calendar_event" => &["add", "create", "make", "schedule"],
        _ => return None,
    };
    let rest = without_article(verbs.iter().find_map(|verb| word(text, verb))?);
    match tool {
        "create_note" => word(rest, "note"),
        "add_reminder" => word(rest, "reminder"),
        "add_calendar_event" => word(rest, "calendar event")
            .or_else(|| word(rest, "event"))
            .or_else(|| word(rest, "meeting")),
        _ => None,
    }
}

/// Bind only a positively requested write's explicit title. Body, due time,
/// event start and every execution/approval decision remain with the caller.
pub(crate) fn action_title(goal: &str, tool: &str) -> Option<String> {
    let kind = match tool {
        "create_note" => "note",
        "add_reminder" => "reminder",
        "add_calendar_event" => "event",
        _ => return None,
    };
    let mut actions = Vec::new();
    let mut total_writes = 0;
    let mut titles = Vec::new();
    for request in requests_in(goal, true)? {
        let anchored = ["create_note", "add_reminder", "add_calendar_event"]
            .into_iter()
            .find(|candidate| action_tail(request, candidate).is_some());
        let (requested, unsupported) = if let Some(anchored) = anchored {
            (vec![anchored], false)
        } else {
            crate::completion::requested_actions(request)
        };
        total_writes += requested.len() + usize::from(unsupported);
        if requested == [tool] {
            actions.push(request);
            let value = if let Some(tail) = action_tail(request, tool) {
                title_marker(tail, Some(kind))
            } else {
                // Noun aliases such as "add that workshop to my calendar"
                // rely on the same positive-action classifier as the task
                // contract. Only an explicit exact-title marker is pinned.
                outside(request).ok()?.into_iter().find_map(|index| {
                    [
                        "using exactly the title",
                        "with exactly the title",
                        "using exactly the reminder title",
                        "with exactly reminder title",
                    ]
                    .iter()
                    .any(|marker| is_word_at(request, index, marker))
                    .then(|| title_marker(&request[index..], Some(kind)))
                    .flatten()
                })
            };
            if let Some(value) = value.and_then(|value| field_prefix(value, 200, false)) {
                titles.push(value);
            }
        }
    }
    if actions.len() != 1 {
        return None;
    }
    if total_writes == 1 {
        let action_offset = actions[0].as_ptr() as usize - goal.as_ptr() as usize;
        for sentence in literal_sentences(goal)? {
            if sentence.as_ptr() as usize - goal.as_ptr() as usize <= action_offset {
                continue;
            }
            let text = instruction(sentence);
            if [
                "use exactly",
                "use the title exactly",
                "use the reminder title exactly",
                "use the note title exactly",
                "use the event title exactly",
            ]
            .iter()
            .any(|prefix| word(text, prefix).is_some())
            {
                if let Some(value) =
                    title_marker(text, Some(kind)).and_then(|value| field_prefix(value, 200, false))
                {
                    titles.push(value);
                }
            }
        }
    }
    titles.sort();
    titles.dedup();
    (titles.len() == 1).then(|| titles.remove(0))
}

fn count_word(word: &str) -> Option<u64> {
    Some(match word {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "fifteen" => 15,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "ninety" => 90,
        digits => digits.parse().ok().filter(|n| (1..=86_400).contains(n))?,
    })
}

fn unit_seconds(word: &str) -> Option<u64> {
    match word {
        "second" | "seconds" | "sec" | "secs" => Some(1),
        "minute" | "minutes" | "min" | "mins" => Some(60),
        "hour" | "hours" | "hr" | "hrs" => Some(3600),
        _ => None,
    }
}

/// A timer's length in seconds from the words of its own clause: "a 12-minute timer",
/// "a timer for 1 hour and 30 minutes", "half an hour", "forty-five minutes". Nothing
/// when the clause names no length, or more than one.
pub(crate) fn timer_seconds(clause: &str) -> Option<u64> {
    let plain = crate::completion::unquoted(clause).to_lowercase();
    let words: Vec<&str> = plain
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    // Each length is a run of amounts, largest unit first, side by side or joined by
    // "and": "1 hour 30 minutes", "2 minutes and 30 seconds".
    let mut lengths: Vec<(u64, u64)> = Vec::new();
    let mut joined = false;
    let mut last_end = None;
    let mut at = 0;
    while at < words.len() {
        let amount = if words[at] == "half"
            && words.get(at + 1).is_some_and(|w| ["a", "an"].contains(w))
            && words.get(at + 2).and_then(|w| unit_seconds(w)).is_some()
        {
            let unit = unit_seconds(words[at + 2]).unwrap();
            Some((unit / 2, unit, at + 3))
        } else {
            count_word(words[at]).and_then(|count| {
                let (count, next) = match words.get(at + 1).and_then(|w| count_word(w)) {
                    Some(ones) if count >= 20 && count % 10 == 0 && ones < 10 => (count + ones, at + 2),
                    _ => (count, at + 1),
                };
                let unit = unit_seconds(words.get(next)?)?;
                let half = words.get(next + 1..next + 4) == Some(&["and", "a", "half"][..]);
                Some((count * unit + if half { unit / 2 } else { 0 }, unit, if half { next + 4 } else { next + 1 }))
            })
        };
        match amount {
            Some((seconds, unit, next)) => {
                let continues = joined || last_end == Some(at);
                match lengths.last_mut() {
                    Some((total, smallest)) if continues && unit < *smallest => {
                        *total += seconds;
                        *smallest = unit;
                    }
                    _ => lengths.push((seconds, unit)),
                }
                joined = false;
                last_end = Some(next);
                at = next;
            }
            None => {
                // "5 or 10 minutes", "5 to 10 minutes", "5-10 minutes": no one length.
                if !["a", "an"].contains(&words[at])
                    && count_word(words[at]).is_some()
                    && words.get(at + 1).is_some_and(|w| ["or", "to"].contains(w) || count_word(w).is_some())
                {
                    return None;
                }
                joined = words[at] == "and" && !lengths.is_empty();
                at += 1;
            }
        }
    }
    match lengths[..] {
        [(seconds, _)] if (1..=86_400).contains(&seconds) => Some(seconds),
        _ => None,
    }
}

fn reminder_instruction(clause: &str) -> bool {
    let text = instruction(clause);
    if word(text, "remind me").is_some() {
        return true;
    }
    let Some(mut rest) = ["add", "create", "set", "make"]
        .iter()
        .find_map(|verb| word(text, verb))
    else {
        return false;
    };
    if let Some(tail) = word(rest, "a") {
        rest = tail;
    }
    word(rest, "reminder").is_some() || word(rest, "reminders").is_some()
}

fn fields(clause: &str) -> Result<Option<Value>, String> {
    let text = instruction(clause);
    let Some(mut rest) = ["add", "create", "set"]
        .iter()
        .find_map(|verb| word(text, verb))
    else {
        return Ok(None);
    };
    if let Some(tail) = word(rest, "a") {
        rest = tail;
    }
    let Some(rest) = word(rest, "reminder") else {
        return Ok(None);
    };
    let Some(rest) = ["titled", "named", "called"]
        .iter()
        .find_map(|marker| word(rest, marker))
    else {
        return Ok(None);
    };
    let mut delimiters = Vec::new();
    for index in outside(rest)? {
        if (index == 0 || rest[..index].ends_with(char::is_whitespace))
            && word(&rest[index..], "due").is_some()
        {
            delimiters.push(index);
        }
    }
    if delimiters.len() != 1 {
        return Err("Give this reminder one title followed by ‘due YYYY-MM-DD HH:MM’. Put a title containing the word ‘due’ in quotation marks.".into());
    }
    let delimiter = delimiters[0];
    let title = literal(&rest[..delimiter])?;
    let due = literal(rest[delimiter + 3..].trim().trim_end_matches('.').trim())?;
    let bytes = due.as_bytes();
    if bytes.len() < 16
        || !matches!(bytes.get(10), Some(b' ' | b'T'))
        || bytes.get(13) != Some(&b':')
    {
        return Err("Provide an exact reminder date and time such as 2026-10-23 09:00. Buntini will not guess a missing time or interpret a relative date here.".into());
    }
    Ok(Some(Value::Object(actions::validate(
        "add_reminder",
        &json!({"title":title,"due":due}),
    )?)))
}

/// Only one explicit named reminder is bound at a time. An unhandled request
/// returns None; a recognized but ambiguous literal instruction returns Err.
pub fn explicit_reminder(goal: &str, steps: &[Step]) -> Result<Option<BoundAction>, String> {
    let clauses = clauses(goal)?;
    let mut candidates = Vec::new();
    for clause in &clauses {
        if let Some(arguments) = fields(clause)? {
            candidates.push((*clause, arguments));
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() != 1
        || clauses
            .iter()
            .filter(|clause| reminder_instruction(clause))
            .count()
            != 1
        || clauses
            .iter()
            .any(|clause| starts(clause, "Additional details:"))
    {
        return Err("This saved task contains multiple reminder instructions or later clarification. Start a new task with one final reminder title and exact due date so Buntini can use those fields without guessing.".into());
    }
    if steps
        .iter()
        .any(|step| matches!(step.state.as_str(), "failed" | "unknown" | "running"))
    {
        return Err("A previous step is unresolved. Finish checking its result before creating this reminder.".into());
    }
    if steps.iter().any(|step| {
        step.tool == "add_reminder"
            && matches!(step.state.as_str(), "waiting" | "succeeded" | "cancelled")
    }) {
        return Ok(None);
    }
    let (clause, arguments) = candidates.remove(0);
    Ok(Some(BoundAction {
        tool: "add_reminder",
        source: json!({"kind":"explicit_reminder", "source":"user_request", "clause":clause, "fields":arguments}),
        arguments,
    }))
}

fn note_fields(clause: &str) -> Result<Option<Value>, String> {
    let text = instruction(clause);
    let Some(mut rest) = ["create", "save", "write", "add", "make"]
        .iter()
        .find_map(|verb| word(text, verb))
    else {
        return Ok(None);
    };
    if let Some(tail) = word(rest, "a") {
        rest = tail;
    }
    let Some(rest) = word(rest, "note") else {
        return Ok(None);
    };
    let Some(rest) = ["titled", "named", "called"]
        .iter()
        .find_map(|marker| word(rest, marker))
    else {
        return Ok(None);
    };
    let delimiters: Vec<usize> = outside(rest)?
        .into_iter()
        .filter(|index| {
            (*index == 0 || rest[..*index].ends_with(char::is_whitespace))
                && word(&rest[*index..], "with body").is_some()
        })
        .collect();
    if delimiters.len() != 1 {
        return Err("Give this note one title followed by ‘with body’ and its exact contents. Quote any field that itself contains ‘with body’.".into());
    }
    let delimiter = delimiters[0];
    let title = literal(&rest[..delimiter]).map_err(|_| {
        "Give the note one complete literal title, with matching quotation marks if used."
            .to_string()
    })?;
    let body = rest[delimiter + "with body".len()..].trim();
    // A period outside a quoted body terminates the instruction, whereas
    // punctuation inside the quote (or in an unquoted body) remains note content.
    let quoted = body.chars().next().and_then(|ch| quote_end(ch, None));
    let body = if quoted.is_some_and(|end| !body.ends_with(end)) {
        body.strip_suffix('.').unwrap_or(body).trim_end()
    } else {
        body
    };
    if quoted.is_none() {
        for index in outside(body)? {
            let tail = &body[index..];
            let following = [" and ", " then ", ", "]
                .iter()
                .find_map(|prefix| starts(tail, prefix).then(|| &tail[prefix.len()..]));
            if following.is_some_and(|tail| {
                [
                    "add", "create", "save", "set", "send", "draft", "open", "read", "check",
                    "remind", "delete", "run",
                ]
                .iter()
                .any(|verb| word(tail, verb).is_some())
            }) {
                return Err("The note body and another instruction are mixed together. Quote the full body and separate the next action with ‘, then’.".into());
            }
        }
    }
    let body = literal(body).map_err(|_| {
        "Give the note one complete literal body, with matching quotation marks if used."
            .to_string()
    })?;
    Ok(Some(Value::Object(actions::validate(
        "create_note",
        &json!({"title":title,"body":body}),
    )?)))
}

/// Bind one explicitly titled note, optionally beside one explicit reminder.
/// Unmatched pieces after a separator are rejected so body text cannot vanish.
pub fn explicit_note(goal: &str, steps: &[Step]) -> Result<Option<BoundAction>, String> {
    let clauses = clauses(goal).map_err(|_| {
        "Close the quotation marks around the note's literal fields before continuing.".to_string()
    })?;
    let mut candidates = Vec::new();
    for (index, clause) in clauses.iter().enumerate() {
        if let Some(arguments) = note_fields(clause)? {
            candidates.push((index, *clause, arguments));
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() != 1
        || clauses
            .iter()
            .any(|clause| starts(clause, "Additional details:"))
    {
        return Err("This saved task has multiple note instructions or later clarification. Start a new task with one final title and complete body so no contents are guessed or lost.".into());
    }
    let (index, clause, arguments) = candidates.remove(0);
    let mut followups = 0;
    for (other, text) in clauses.iter().enumerate() {
        if other == index || text.is_empty() {
            continue;
        }
        if fields(text)?.is_none() {
            return Err("Part of this note request is outside its body or is an unclear next instruction. Quote the complete body and give the next action separately.".into());
        }
        followups += 1;
    }
    if followups > 1 {
        return Err("Use one explicit note and one reminder per saved task so Buntini can bind every field without guessing.".into());
    }
    if steps
        .iter()
        .any(|step| matches!(step.state.as_str(), "failed" | "unknown" | "running"))
    {
        return Err(
            "A previous step is unresolved. Finish checking its result before creating this note."
                .into(),
        );
    }
    if steps.iter().any(|step| {
        step.tool == "create_note"
            && matches!(step.state.as_str(), "waiting" | "succeeded" | "cancelled")
    }) {
        return Ok(None);
    }
    Ok(Some(BoundAction {
        tool: "create_note",
        source: json!({"kind":"explicit_note","source":"user_request","clause":clause,"fields":arguments}),
        arguments,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_queries_preserve_full_arbitrary_identifiers_and_quoted_titles() {
        for (goal,expected) in [
            ("Find the note mentioning OrchardLedger-7f4c8d91 and summarize it.","OrchardLedger-7f4c8d91"),
            ("Please read the note containing Café.v2_91cd-48ef, then tell me the amount.","Café.v2_91cd-48ef"),
            ("Search for a note mentioning Fleet-62ee713a in Notes and summarize it.","Fleet-62ee713a"),
            ("Read the note titled ‘Travel & receipts; M-82c’, then create a note titled Summary using that source.","Travel & receipts; M-82c"),
            ("Read my Harbor-9a8c note and tell me the departure.","Harbor-9a8c"),
            ("Read my \"Mum's garden; then flowers\" note, then summarize it.","Mum's garden; then flowers"),
        ] {
            assert_eq!(read_query(goal).as_deref(),Some(expected),"{goal}");
        }
    }

    #[test]
    fn possessive_note_titles_end_at_clause_punctuation_without_losing_identifiers() {
        for title in [
            "ACME-f231dd67b5aa4535 Quotes",
            "ACME-73be Handoff",
            "Cedar-915c Literal",
        ] {
            let goal = format!("Read my {title} note, calculate both total prices, and recommend one using my stated rule.");
            assert_eq!(read_query(&goal).as_deref(), Some(title), "{goal}");
        }
        assert_eq!(
            read_query("Read my \"ACME-73be; Quotes\" note, calculate the total.").as_deref(),
            Some("ACME-73be; Quotes")
        );
        assert!(read_query("Read my ACME-73be notebook, calculate the total.").is_none());
        assert!(read_query("Read my ACME-73be note.txt, calculate the total.").is_none());
        assert!(read_query("Do not read my ACME-73be Quotes note, calculate the total.").is_none());
    }

    #[test]
    fn partial_action_titles_do_not_require_or_invent_source_derived_fields() {
        for (goal,tool,expected) in [
            ("Read the note titled Source-91bc, then create a new note titled Fresh-74ed whose body contains its summary.","create_note","Fresh-74ed"),
            ("Read the note mentioning Fleet-62ee713a, then add a reminder with exactly the reminder title Review-Fleet-43be due at the time in that note.","add_reminder","Review-Fleet-43be"),
            ("Create a calendar event using exactly the title ‘Workshop C-77’ at the time in the source note.","add_calendar_event","Workshop C-77"),
            ("Create a note with exactly the title \"Budget and receipts; then Archive\" using that note's contents.","create_note","Budget and receipts; then Archive"),
            ("Add a reminder with exactly reminder title Check-Copper-63ef based on the source record.","add_reminder","Check-Copper-63ef"),
            ("Create a note called Mum's garden using that source's text.","create_note","Mum's garden"),
            ("Create an event titled Atlas-24bf on 2026-12-18 at 09:30.","add_calendar_event","Atlas-24bf"),
            ("Remind me to prepare the workshop at the date in the note. Use exactly the reminder title Prepare-Mica-6c3d.","add_reminder","Prepare-Mica-6c3d"),
            ("Read the note mentioning Meadow-8d19, then add that workshop to my calendar using exactly the title Meadow-session-7acd.","add_calendar_event","Meadow-session-7acd"),
            ("Create a new note titled Compass-7ad9 that summarizes the earlier source.","create_note","Compass-7ad9"),
        ] {
            assert_eq!(action_title(goal,tool).as_deref(),Some(expected),"{goal}");
        }
        assert!(
            explicit_note(
                "Create a note titled Fresh-74ed whose body contains its summary.",
                &[]
            )
            .is_err(),
            "partial metadata extraction must not manufacture a complete executable note"
        );
    }

    #[test]
    fn old_read_titles_and_quoted_body_commands_cannot_become_new_write_titles() {
        let goal="Read the note titled ‘Original - 7b91’, then create a note titled ‘New - 82de’ whose body summarizes it.";
        assert_eq!(read_query(goal).as_deref(), Some("Original - 7b91"));
        assert_eq!(
            action_title(goal, "create_note").as_deref(),
            Some("New - 82de")
        );
        assert!(action_title("Read the note titled Old-42ce.", "create_note").is_none());
        assert!(action_title(
            "Create a note with body \"Add a reminder titled Injected-7d13\".",
            "add_reminder"
        )
        .is_none());
        assert!(action_title(
            "Create a note from the source note titled Old-42ce.",
            "create_note"
        )
        .is_none());
        assert!(action_title(
            "Add a reminder using the title from that note.",
            "add_reminder"
        )
        .is_none());
    }

    #[test]
    fn partial_literal_pins_reject_negation_explanations_and_ambiguous_targets() {
        for goal in [
            "Do not read the note titled Source-4de1 and create a note titled Copy-77b8.",
            "How do I find the note mentioning Pine-3b9e and create a note titled Copy-77b8?",
            "Explain ‘read the note titled Source-4de1; then create a note titled Copy-77b8’.",
            "For example, read the note mentioning Pine-3b9e and create a note titled Copy-77b8.",
            "Create a note with body \"Read the note mentioning Pine-3b9e\".",
        ] {
            assert!(read_query(goal).is_none(), "{goal}");
            assert!(action_title(goal, "create_note").is_none(), "{goal}");
        }
        assert!(
            read_query("Read the note titled Source-A and read the note titled Source-B.")
                .is_none()
        );
        assert!(action_title(
            "Create a note titled First-21ac and create a note titled Second-97dc.",
            "create_note"
        )
        .is_none());
        assert!(action_title(
            "Create a note titled First-21ac; create another note.",
            "create_note"
        )
        .is_none());
        assert!(action_title("Create a note titled \"Unclosed.", "create_note").is_none());
        assert!(action_title(
            "Don't create a note titled One and add a reminder titled Two.",
            "add_reminder"
        )
        .is_none());
        assert!(action_title("Read the note titled Source, then don't create a note titled One and add a reminder titled Two.","add_reminder").is_none());
        assert!(action_title(
            "Do not remind me about the workshop. Use exactly the reminder title Wrong-9b1e.",
            "add_reminder"
        )
        .is_none());
        assert!(action_title("Explain how to remind me about the workshop. Use exactly the reminder title Wrong-9b1e.","add_reminder").is_none());
        assert!(action_title("Create a note with body \"Remind me about the workshop. Use exactly the reminder title Wrong-9b1e.\".","add_reminder").is_none());
        assert!(action_title(
            "Create a note, then add a reminder. Use exactly the title Ambiguous-4da2.",
            "add_reminder"
        )
        .is_none());
        assert!(action_title(
            "Create a note titled First-4bc1. Use exactly the note title Other-9ef3.",
            "create_note"
        )
        .is_none());
    }

    #[test]
    fn separates_explicit_fields_after_note_without_model_inference() {
        for (goal, title, due) in [
            ("Create a note titled Packing with body Passport and adapter, then add a reminder titled Check Packing due 2026-10-23 09:00.", "Check Packing", "2026-10-23 09:00"),
            ("Create a note titled Lease with body Ask about renewal; then create a reminder named Check Lease due 2026-11-02 15:20.", "Check Lease", "2026-11-02 15:20"),
            ("Please set a reminder called Call Ana due 2026-11-02T18:45:00", "Call Ana", "2026-11-02 18:45"),
        ] {
            let bound = explicit_reminder(goal, &[]).unwrap().unwrap();
            assert_eq!(bound.tool, "add_reminder");
            assert_eq!(bound.arguments, json!({"title":title,"due":due}));
            assert_eq!(bound.source["fields"], bound.arguments);
            assert_eq!(bound.source["source"], "user_request");
        }
    }

    #[test]
    fn due_and_step_commands_inside_quoted_titles_are_literal() {
        for (goal, title) in [
            ("Add a reminder titled \"Check due dates\" due 2026-11-02 09:00.", "Check due dates"),
            ("Add a reminder titled “Check due dates” due 2026-11-02 09:00.", "Check due dates"),
            ("Add a reminder called 'Read Lease; then create a note' due 2026-11-02 09:00.", "Read Lease; then create a note"),
            ("Create a note with body \"add a reminder titled Evil due 2026-11-02 06:00; then create another\", then add a reminder named Lease due 2026-11-02 09:00.", "Lease"),
            ("Add a reminder named Mum's birthday due 2026-11-02 09:00.", "Mum's birthday"),
        ] { assert_eq!(explicit_reminder(goal, &[]).unwrap().unwrap().arguments["title"], title); }
    }

    #[test]
    fn explicit_offsets_use_the_production_local_time_validation() {
        let at = "2026-11-02T14:25:00Z";
        let bound = explicit_reminder(&format!("Create a reminder named Lease due {at}."), &[])
            .unwrap()
            .unwrap();
        assert_eq!(bound.arguments["due"], actions::normalize_when(at).unwrap());
    }

    #[test]
    fn missing_or_ambiguous_fields_do_not_get_guessed() {
        for goal in [
            "Add a reminder titled Lease", "Add a reminder named Lease due tomorrow at noon.",
            "Add a reminder named Lease due 2026-11-02.", "Add a reminder named Lease due 2026-02-30 09:00.",
            "Add a reminder named Lease due 2026-11-02 09:00 and send an email.",
            "Add a reminder named Check due dates due 2026-11-02 09:00.",
            "Add a reminder named \"Lease due 2026-11-02 09:00.",
            "Add a reminder named Lease due 2026-11-02 09:00; add a reminder named Other due 2026-11-02 10:00.",
            "Add a reminder named Lease due 2026-11-02 09:00; remind me to call Mum tomorrow.",
            "Add a reminder named \"Lease\" \"Other\" due 2026-11-02 09:00.",
            "Add a reminder named Lease due 2026-11-02 09:00.\n\nAdditional details: Use 10:00 instead.",
        ] { assert!(explicit_reminder(goal, &[]).is_err(), "{goal}"); }
    }

    #[test]
    fn examples_negation_and_quoted_note_contents_never_become_reminders() {
        for goal in [
            "Do not add a reminder titled Lease due 2026-11-02 09:00.",
            "How do I add a reminder titled Lease due 2026-11-02 09:00?",
            "Explain ‘add a reminder titled Lease due 2026-11-02 09:00’.",
            "Create a note with body \"; add a reminder titled Lease due 2026-11-02 09:00\".",
            "Remind me to call mum tomorrow.",
        ] {
            assert!(explicit_reminder(goal, &[]).unwrap().is_none(), "{goal}");
        }
    }

    #[test]
    fn prior_attempted_reminders_are_never_repeated() {
        let goal = "Add a reminder titled Lease due 2026-11-02 09:00.";
        let mut step = Step {
            id: "reminder".into(),
            tool: "add_reminder".into(),
            arguments: json!({}),
            state: "succeeded".into(),
            effect: true,
            proposed_at: "now".into(),
            approved_at: Some("now".into()),
            finished_at: Some("now".into()),
            evidence: None,
        };
        for state in ["succeeded", "waiting", "cancelled"] {
            step.state = state.into();
            assert!(explicit_reminder(goal, &[step.clone()]).unwrap().is_none());
        }
        for state in ["failed", "unknown", "running"] {
            step.state = state.into();
            assert!(explicit_reminder(goal, &[step.clone()]).is_err());
        }
    }

    #[test]
    fn note_fields_stop_at_the_next_step_with_varied_titles_and_bodies() {
        for (goal, title, body) in [
            ("Create a note titled Packing with body Passport and adapter, then add a reminder titled Check Packing due 2026-10-23 09:00.", "Packing", "Passport and adapter"),
            ("Create a note titled Lease renewal with body Bring the signed lease and photo ID, then add a reminder titled Review lease renewal due 2026-11-02 15:20.", "Lease renewal", "Bring the signed lease and photo ID"),
            ("Please save a note named Exam preparation with body Calculator and pencils and then create a reminder called Check equipment due 2026-11-03 08:10.", "Exam preparation", "Calculator and pencils"),
            ("Create a note called Café ideas with body Meet Zoë at 4 pm.", "Café ideas", "Meet Zoë at 4 pm."),
        ] {
            let action = explicit_note(goal, &[]).unwrap().unwrap();
            assert_eq!(action.tool, "create_note");
            assert_eq!(action.arguments, json!({"title":title,"body":body}));
            assert_eq!(action.source["fields"], action.arguments);
        }
    }

    #[test]
    fn quoted_note_bodies_keep_commands_punctuation_and_line_breaks_as_content() {
        for (goal, title, body) in [
            ("Create a note titled \"Read with body language\" with body \"Passport; then add a reminder titled Example due 2026-11-02 08:00.\".", "Read with body language", "Passport; then add a reminder titled Example due 2026-11-02 08:00."),
            ("Save a note called ‘Trip’ with body ‘Passport\nAdapter; umbrella’, then add a reminder named Check trip due 2026-11-02 09:00.", "Trip", "Passport\nAdapter; umbrella"),
            ("Create a note titled Mum's list with body \"Buy apples and add a reminder for tomorrow\".", "Mum's list", "Buy apples and add a reminder for tomorrow"),
        ] {
            let action = explicit_note(goal, &[]).unwrap().unwrap();
            assert_eq!(action.arguments, json!({"title":title,"body":body}));
        }
    }

    #[test]
    fn ambiguous_note_fields_and_unmatched_instruction_pieces_are_not_discarded() {
        for goal in [
            "Create a note titled Packing", "Create a note titled Packing with body", "Create a note titled with body Passport",
            "Create a note titled Packing with body Passport; adapter", "Create a note titled Packing with body Passport\nAdapter",
            "Create a note titled Packing with body Passport and add a reminder titled Check due 2026-11-02 09:00.",
            "Create a note titled Packing with body Passport, then unknown extra content.",
            "Create a note titled Packing with body Passport; create a note named Other with body Adapter.",
            "Create a note titled Packing with body Passport.\n\nAdditional details: Change the body to Suitcase.",
            "Create a note titled \"Packing\" with body \"Passport\" extra contents.",
        ] { assert!(explicit_note(goal, &[]).is_err(), "{goal}"); }
    }

    #[test]
    fn note_examples_negation_and_prior_attempts_do_not_create_more_notes() {
        for goal in [
            "Do not create a note titled Packing with body Passport.",
            "How do I create a note titled Packing with body Passport?",
            "Explain ‘create a note titled Packing with body Passport’.",
        ] {
            assert!(explicit_note(goal, &[]).unwrap().is_none());
        }
        let mut step = Step {
            id: "note".into(),
            tool: "create_note".into(),
            arguments: json!({}),
            state: "succeeded".into(),
            effect: true,
            proposed_at: "now".into(),
            approved_at: Some("now".into()),
            finished_at: Some("now".into()),
            evidence: None,
        };
        let goal = "Create a note titled Packing with body Passport.";
        for state in ["succeeded", "waiting", "cancelled"] {
            step.state = state.into();
            assert!(explicit_note(goal, &[step.clone()]).unwrap().is_none());
        }
        for state in ["failed", "unknown", "running"] {
            step.state = state.into();
            assert!(explicit_note(goal, &[step.clone()]).is_err());
        }
    }

    #[test]
    fn a_reminder_that_opens_with_its_date_keeps_the_exact_title_given_after_it() {
        let goal = "Read my note mentioning QA-1bf3 Travel. On May 20, 2031, remind me to leave for the station 15 minutes before my stated station-arrival time. Use exactly the reminder title QA-1bf3 Leave for station.";
        assert_eq!(action_title(goal, "add_reminder").as_deref(), Some("QA-1bf3 Leave for station"));
        assert_eq!(
            action_title("Tomorrow, remind me to call Ana. Use exactly the reminder title Call Ana.", "add_reminder").as_deref(),
            Some("Call Ana")
        );
        // An opening that isn't a time is not a request to act.
        assert!(action_title("For example, create a note titled Copy-77b8.", "create_note").is_none());
    }

    #[test]
    fn a_timer_clause_gives_exactly_one_length() {
        for (clause, seconds) in [
            ("set a 12-minute timer for the rice", Some(720)),
            ("set a timer for ten minutes", Some(600)),
            ("start a timer for 1 hour and 30 minutes", Some(5400)),
            ("timer for 1 hour 30 minutes", Some(5400)),
            ("a timer for an hour and a half", Some(5400)),
            ("half an hour timer", Some(1800)),
            ("set a forty-five minute timer", Some(2700)),
            ("90 seconds for the eggs", Some(90)),
            ("set a timer for 5 or 10 minutes", None),
            ("set a timer for the pasta", None),
            ("set a timer at 5 pm", None),
            ("set a timer for 25 hours", None),
        ] {
            assert_eq!(timer_seconds(clause), seconds, "{clause}");
        }
    }
}
