//! Conservative extraction of a location literally supplied in the user's
//! calendar request. This never reads model/tool text or infers a location.

fn outside(text: &str) -> Option<Vec<usize>> {
    let mut result = Vec::new();
    let mut quote = None;
    let mut previous = None;
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
        } else {
            quote = match ch {
                '"' => Some('"'),
                '“' => Some('”'),
                '‘' => Some('’'),
                '`' => Some('`'),
                '\'' if !previous.is_some_and(char::is_alphanumeric) => Some('\''),
                _ => None,
            };
            if quote.is_none() {
                result.push(index);
            }
        }
        previous = Some(ch);
    }
    quote.is_none().then_some(result)
}

fn location(tail: &str) -> Option<String> {
    let tail = tail.trim_start();
    let first = tail.chars().next()?;
    let closing = match first {
        '"' => Some('"'),
        '“' => Some('”'),
        '‘' => Some('’'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    };
    let value = if let Some(closing) = closing {
        let inner = &tail[first.len_utf8()..];
        let end = inner.find(closing)?;
        &inner[..end]
    } else {
        let lower = tail.to_ascii_lowercase();
        let mut end = tail.len();
        for stop in [
            ",",
            ";",
            "\n",
            "!",
            "?",
            " on ",
            " from ",
            " for ",
            " with ",
            " using ",
            " and ",
            " then ",
            " before ",
            " after ",
            " instead ",
        ] {
            if let Some(index) = lower.find(stop) {
                end = end.min(index);
            }
        }
        tail[..end].trim_end_matches('.')
    }
    .trim();
    if value.is_empty()
        || value.chars().count() > 200
        || !value.chars().next()?.is_alphabetic()
        || value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, ':' | '/' | '@' | '\\' | '"' | '`'))
    {
        return None;
    }
    let lower = value.to_lowercase();
    let words: Vec<_> = lower
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let forbidden = [
        "calendar",
        "event",
        "events",
        "note",
        "notes",
        "reminder",
        "title",
        "titled",
        "called",
        "named",
        "date",
        "time",
        "minute",
        "minutes",
        "hour",
        "hours",
        "today",
        "tomorrow",
        "yesterday",
        "morning",
        "afternoon",
        "evening",
        "tonight",
        "noon",
        "midday",
        "midnight",
        "next",
        "am",
        "pm",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
        "jan",
        "feb",
        "mar",
        "apr",
        "jun",
        "jul",
        "aug",
        "sep",
        "sept",
        "oct",
        "nov",
        "dec",
    ];
    if words.first().is_some_and(|word| {
        [
            "one",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine",
            "ten",
            "eleven",
            "twelve",
            "thirteen",
            "fourteen",
            "fifteen",
            "sixteen",
            "seventeen",
            "eighteen",
            "nineteen",
            "twenty",
            "half",
            "quarter",
        ]
        .contains(word)
    }) {
        return None;
    }
    if words.iter().any(|word| forbidden.contains(word))
        || ["st", "dr", "mt"].contains(&lower.as_str())
    {
        return None;
    }
    Some(value.to_string())
}

/// Return an unambiguous `at/in <location>` literal only for one positively
/// requested Calendar write. Dates, clocks, quoted commands and negations never
/// become location constraints. Ambiguous requests remain unbound for review.
pub fn supplied_location(goal: &str) -> Option<String> {
    if goal.len() > 8000
        || crate::completion::requested_actions(goal)
            .0
            .iter()
            .filter(|tool| **tool == "add_calendar_event")
            .count()
            != 1
    {
        return None;
    }
    let positions = outside(goal)?;
    let lower = goal.to_ascii_lowercase();
    let mut boundaries = vec![0];
    for &index in &positions {
        let ch = goal[index..].chars().next()?;
        if [';', '\n', '!', '?'].contains(&ch)
            || (ch == '.'
                && goal[index + 1..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace))
        {
            boundaries.push(index + ch.len_utf8());
        }
    }
    boundaries.push(goal.len());
    let mut found = Vec::new();
    for &index in &positions {
        if index > 0 && !goal[..index].chars().next_back()?.is_whitespace() {
            continue;
        }
        let length = if lower[index..].starts_with("in ") || lower[index..].starts_with("at ") {
            3
        } else {
            continue;
        };
        let begin = *boundaries.iter().filter(|start| **start <= index).max()?;
        let end = *boundaries.iter().filter(|end| **end > index).min()?;
        let sentence = &goal[begin..end];
        let plain = crate::completion::unquoted(sentence);
        if plain.trim().is_empty() {
            continue;
        }
        let request = crate::completion::request_clause(&plain);
        if crate::completion::non_action(request) {
            continue;
        }
        // A source folder/search scope preceding the Calendar instruction is
        // not the event's location.
        if ["read ", "find ", "search ", "review "]
            .iter()
            .any(|verb| request.starts_with(verb))
            && !crate::completion::requested_actions(&goal[begin..index])
                .0
                .contains(&"add_calendar_event")
        {
            continue;
        }
        if let Some(value) = location(&goal[index + length..end]) {
            if !found.contains(&value) {
                found.push(value);
            }
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_room_literals_are_preserved_without_inferencing_dates() {
        for (goal, expected) in [
            ("Add a calendar event on May 21, 2031 at 14:15 for 35 minutes in Training Room.", "Training Room"),
            ("My training runs from 14:15 to 14:50 on May 21, 2031 in Training Room. Add it to my calendar.", "Training Room"),
            ("Please schedule a calendar event at \"North Studio\" on 2031-05-21.", "North Studio"),
            ("Add a calendar event in Room 42 on 2031-05-21 at 10:00.", "Room 42"),
            ("Read my note in Work Folder, then add an event in Training Room.", "Training Room"),
        ] { assert_eq!(supplied_location(goal).as_deref(), Some(expected), "{goal}"); }
    }

    #[test]
    fn quoted_negated_temporal_and_ambiguous_locations_are_not_bound() {
        for goal in [
            "Do not add a calendar event in Training Room.",
            "Explain how to add a calendar event in Training Room.",
            "Summarize this example: 'Add a calendar event in Training Room'.",
            "> Add a calendar event in Example Room.\nAdd a calendar event tomorrow.",
            "Add a calendar event at 14:15 on May 21, 2031.",
            "Add a calendar event at noon.",
            "Add a calendar event at midnight.",
            "Add a calendar event at ten am.",
            "Add a calendar event in May 2031.",
            "Add a calendar event titled \"Meeting in Training Room\" on 2031-05-21.",
            "Add a calendar event in Study Room and another event in Training Room.",
            "Add two calendar events in Training Room.",
            "Add a calendar event in my calendar.",
        ] {
            assert_eq!(supplied_location(goal), None, "{goal}");
        }
    }
}
