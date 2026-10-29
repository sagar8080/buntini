//! Bind a small, explicitly supplied table to calculation inputs. This parser
//! never infers amounts/statuses or follows instructions inside quoted data.
use serde_json::{json, Value};
use std::collections::HashSet;

fn quoted_source(goal: &str) -> Option<Option<(&str, String)>> {
    let chars = goal.char_indices().collect::<Vec<_>>();
    let mut quote = None;
    let mut begin = 0;
    let mut opening = 0;
    let mut escaped = false;
    let mut found = None;
    for (position, (index, ch)) in chars.iter().copied().enumerate() {
        let previous = position
            .checked_sub(1)
            .and_then(|i| chars.get(i))
            .map(|(_, ch)| *ch);
        let next = chars.get(position + 1).map(|(_, ch)| *ch);
        if let Some(end) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == end
                && !(matches!(ch, '\'' | '’')
                    && previous.is_some_and(char::is_alphanumeric)
                    && next.is_some_and(char::is_alphanumeric))
            {
                if found.is_some() {
                    return None;
                }
                found = Some((
                    &goal[begin..index],
                    format!("{} {}", &goal[..opening], &goal[index + ch.len_utf8()..]),
                ));
                quote = None;
            }
        } else {
            let end = match ch {
                '"' => Some('"'),
                '“' => Some('”'),
                '‘' => Some('’'),
                '`' => Some('`'),
                '\'' if !previous.is_some_and(char::is_alphanumeric) => Some('\''),
                _ => None,
            };
            if let Some(end) = end {
                quote = Some(end);
                begin = index + ch.len_utf8();
                opening = index;
            }
        }
    }
    quote.is_none().then_some(found)
}

fn monthly(header: &str) -> bool {
    let words = header
        .to_lowercase()
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    words.iter().any(|word| word == "monthly")
        && !words
            .iter()
            .any(|word| ["weekly", "quarterly", "daily"].contains(&word.as_str()))
        && !header.to_lowercase().contains("not monthly")
}

fn unused_constraint(instruction: &str) -> bool {
    let lower = instruction.to_lowercase();
    if [
        "except",
        "unless",
        "instead",
        "keep unused",
        "keep the unused",
        "cancel used",
        "cancel the used",
        "cancel all",
        "do not cancel",
        "don't cancel",
        "never cancel",
        "not cancel",
        "avoid cancel",
        "include used",
        "include the used",
        "both used",
        "used and unused",
        "unused and used",
        "do not cancel unused",
        "do not cancel the unused",
        "don't cancel unused",
        "don't cancel the unused",
        "not cancelling",
        "not canceling",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
    {
        return false;
    }
    let request = crate::completion::request_clause(lower.trim());
    if crate::completion::non_action(request) {
        return false;
    }
    ["cancel", "cancelling", "canceling"].iter().any(|verb| {
        [" unused", " the unused", " only unused", " only the unused"]
            .iter()
            .any(|tail| {
                let phrase = format!("{verb}{tail}");
                lower.match_indices(&phrase).any(|(index, found)| {
                    !lower[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric)
                        && !lower[index + found.len()..]
                            .chars()
                            .next()
                            .is_some_and(char::is_alphanumeric)
                })
            })
    }) || [
        "savings from unused",
        "savings from only unused",
        "savings from the unused",
        "savings from only the unused",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn suffix(rest: &str) -> Option<&str> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(rest);
    }
    let rest = rest.strip_prefix('.')?.trim();
    if rest.is_empty() {
        return Some(rest);
    }
    let lower = rest.trim_end_matches('.').trim().to_lowercase();
    [
        "keep the used subscriptions",
        "keep used subscriptions",
        "keep the used ones",
        "keep used ones",
        "keep the used items",
        "keep used items",
    ]
    .contains(&lower.as_str())
    .then_some(rest)
}

fn row(text: &str, last: bool) -> Option<(String, String, String, &str)> {
    let (label, rest) = text.trim().split_once('$')?;
    let label = label.trim();
    if label.is_empty()
        || label.len() > 120
        || !label.chars().any(char::is_alphabetic)
        || !label.chars().all(|ch| {
            ch.is_alphanumeric()
                || [' ', '-', '_', '\'', '’', '.', '&', '+', '(', ')'].contains(&ch)
        })
    {
        return None;
    }
    let end = rest.find(char::is_whitespace)?;
    let amount = &rest[..end];
    let parts = amount.split('.').collect::<Vec<_>>();
    if parts.len() > 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|ch| ch.is_ascii_digit()))
        || amount.len() > 48
    {
        return None;
    }
    let remaining = rest[end..].trim_start();
    let (status, remaining) = ["unused", "used"].iter().find_map(|status| {
        let head = remaining.get(..status.len())?;
        if !head.eq_ignore_ascii_case(status) {
            return None;
        }
        let tail = &remaining[status.len()..];
        (tail.is_empty() || tail.starts_with(char::is_whitespace) || tail.starts_with('.'))
            .then_some((*status, tail))
    })?;
    let trailing = if last {
        suffix(remaining)?
    } else if remaining.trim().is_empty() {
        remaining
    } else {
        return None;
    };
    Some((
        label.to_string(),
        amount.to_string(),
        status.to_string(),
        trailing,
    ))
}

/// Only unambiguous, semicolon-delimited user literal rows with a monthly
/// source period and explicit unused-only cancellation/savings constraint.
pub fn status_sum_fields(goal: &str) -> Option<Value> {
    if goal.is_empty() || goal.len() > 16 * 1024 {
        return None;
    }
    let quoted = quoted_source(goal)?;
    let (source, outside, is_quoted) = match quoted {
        Some((source, outside)) => {
            if outside.contains('$') {
                return None;
            }
            (source, outside, true)
        }
        None => (goal, String::new(), false),
    };
    let first_dollar = source.find('$')?;
    let (header, table) = if let Some(colon) = source[..first_dollar].rfind(':') {
        (&source[..colon], source[colon + 1..].trim())
    } else if is_quoted {
        ("", source.trim())
    } else {
        return None;
    };
    let rows = table.split(';').collect::<Vec<_>>();
    if rows.len() < 2 || rows.len() > 24 {
        return None;
    }
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let mut tail = "";
    for (index, text) in rows.iter().enumerate() {
        let (label, amount, status, rest) = row(text, index + 1 == rows.len())?;
        if !seen.insert(label.to_lowercase()) {
            return None;
        }
        if is_quoted && !rest.trim().is_empty() {
            return None;
        }
        tail = rest;
        items.push(format!("{label}|{amount}|{status}"));
    }
    let instruction = if is_quoted {
        outside
    } else {
        format!("{header} {tail}")
    };
    if !(monthly(header) || (is_quoted && monthly(&instruction)))
        || !unused_constraint(&instruction)
    {
        return None;
    }
    let fields = json!({"include_status":"unused","items":items,"periods_per_year":12});
    crate::structured_math::validate("sum_by_status", &fields).ok()?;
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_monthly_rows_preserve_every_status_amount_and_zero() {
        let goal = "Review these monthly subscriptions and calculate savings by cancelling only the unused ones: Audio Grove $7.50 used; Film Harbor $19.25 unused; Archive $0 used; Classes $4 unused. Keep the used subscriptions.";
        assert_eq!(
            status_sum_fields(goal),
            Some(
                json!({"include_status":"unused","periods_per_year":12,"items":["Audio Grove|7.50|used","Film Harbor|19.25|unused","Archive|0|used","Classes|4|unused"]})
            )
        );
        assert_eq!(status_sum_fields("Find savings from unused monthly subscriptions: Studio $18 unused; Garden $0 unused.").unwrap()["items"], json!(["Studio|18|unused","Garden|0|unused"]));
    }

    #[test]
    fn one_quoted_literal_source_needs_an_outside_constraint() {
        let goal = "Calculate savings by canceling only unused monthly subscriptions: 'Aster Audio $8 used; Rowan Video $11 unused'.";
        assert_eq!(
            status_sum_fields(goal).unwrap()["items"],
            json!(["Aster Audio|8|used", "Rowan Video|11|unused"])
        );
        assert!(status_sum_fields(
            "Read this source: 'Monthly subscriptions: Cancel unused $8 used; Rowan $11 unused'."
        )
        .is_none());
        assert!(status_sum_fields("Cancel only unused monthly subscriptions: 'Aster $8 used; Rowan $11 unused. Keep the used subscriptions.'").is_none());
        assert!(status_sum_fields("Cancel only unused monthly subscriptions: 'Aster $8 used; Rowan $11 unused' and 'Cedar $5 used; Maple $6 unused'.").is_none());
    }

    #[test]
    fn partial_conflicting_unknown_or_nonmonthly_sources_are_not_bound() {
        for goal in [
            "Cancel only unused subscriptions: Aster $8 used; Rowan $11 unused.",
            "Cancel only unused weekly subscriptions: Aster $8 used; Rowan $11 unused.",
            "Do not cancel unused monthly subscriptions: Aster $8 used; Rowan $11 unused.",
            "Review these monthly subscriptions. Do not cancel only the unused ones: Aster $8 used; Rowan $11 unused.",
            "Cancel only unused monthly subscriptions except Rowan: Aster $8 used; Rowan $11 unused.",
            "Cancel only unused monthly subscriptions: Aster $8 used; Rowan $11 unknown.",
            "Cancel only unused monthly subscriptions: Aster $8 used; a malformed row; Rowan $11 unused.",
            "Cancel only unused monthly subscriptions: Aster $8 used; aster $11 unused.",
            "Cancel only unused monthly subscriptions: Aster $8 used; Rowan $11 unused. Replace Aster with zero.",
            "Cancel only unused monthly subscriptions: Aster $8 used; Rowan $11-2 unused.",
            "Cancel only unused monthly subscriptions: Aster $8 used; Rowan $1,100 unused.",
            "Cancel only unused monthly subscriptions: Aster $8 used; Rowan $11 unused;",
            "Cancel only unused monthly subscriptions: 'Aster $8 used; Rowan $11 unused",
        ] { assert!(status_sum_fields(goal).is_none(), "{goal}"); }
    }
}
