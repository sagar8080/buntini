//! Bind a small, explicitly supplied table to calculation inputs, and an email draft's
//! recipient to the one person in Contacts the user named. These parsers never infer
//! amounts, statuses or addresses, and never follow instructions inside quoted data.
use crate::workflows::Step;
use serde_json::{json, Value};
use std::collections::HashSet;

/// Text outside quotation marks, keeping its case: quoted text is content the user is
/// passing along (a message to rewrite, an email to answer), not who to write to.
fn outside_quotes(text: &str) -> String {
    let text = text.replace(['’', '‘'], "'").replace(['“', '”'], "\"");
    let chars: Vec<char> = text.chars().collect();
    let mut quote = None;
    let mut result = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
                result.push(' ');
            }
        } else if ch == '"'
            || ch == '`'
            || (ch == '\''
                && !index
                    .checked_sub(1)
                    .and_then(|i| chars.get(i))
                    .is_some_and(|c| c.is_alphanumeric()))
        {
            quote = Some(ch);
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Words that start a clause or a name position but are never a person to look up.
const NOT_NAMES: &[&str] = &[
    "i", "me", "my", "myself", "you", "your", "we", "us", "our", "they", "them", "their", "he",
    "him", "his", "she", "her", "it", "this", "that", "these", "those", "the", "a", "an",
    "everyone", "everybody", "all", "buntini", "monday", "tuesday", "wednesday", "thursday",
    "friday", "saturday", "sunday", "today", "tomorrow", "tonight", "january", "february",
    "march", "april", "may", "june", "july", "august", "september", "october", "november",
    "december",
];

/// The person a requested email draft is for, by name, when the user's own words say
/// exactly one: "Draft an email to Ana Silva", "reply to Ana's email", "write back to
/// Sam". Nothing when they gave an address, named several people or only a role.
pub fn named_recipient(goal: &str) -> Option<String> {
    let text = outside_quotes(goal);
    if text.contains('@') {
        return None;
    }
    // Each word with whether a clause or name ends after it ("Ana," or "Ana's").
    let words: Vec<(String, bool)> = text
        .split_whitespace()
        .filter_map(|raw| {
            let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
            if word.is_empty() {
                return None;
            }
            let (word, possessive) = match word.strip_suffix("'s") {
                Some(stem) if !stem.is_empty() => (stem, true),
                _ => (word, false),
            };
            let ends = possessive || !raw.ends_with(char::is_alphanumeric);
            Some((word.to_string(), ends))
        })
        .collect();
    let capitalised = |word: &str| {
        word.chars().next().is_some_and(char::is_uppercase)
            && word.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'')
            && !NOT_NAMES.contains(&word.to_lowercase().as_str())
    };
    let mut found: Vec<String> = Vec::new();
    for (index, (word, ends)) in words.iter().enumerate() {
        if *ends || !["to", "for"].contains(&word.to_lowercase().as_str()) {
            continue;
        }
        // "email to", "reply to", "write back to": within the same short clause.
        let drafting = words[..index]
            .iter()
            .rev()
            .take(5)
            .take_while(|(_, ends)| !ends)
            .any(|(word, _)| {
                ["email", "mail", "message", "reply", "respond", "back", "draft"]
                    .contains(&word.to_lowercase().as_str())
            });
        if !drafting {
            continue;
        }
        let mut name = Vec::new();
        let mut next = index + 1;
        while let Some((word, ends)) = words.get(next) {
            if name.len() == 3 || !capitalised(word) {
                break;
            }
            name.push(word.as_str());
            next += 1;
            if *ends {
                break;
            }
        }
        if name.is_empty() {
            continue;
        }
        // "to Ana and Sam": a draft goes to one person, so neither is chosen.
        let last_ended = words[next - 1].1;
        if !last_ended
            && words.get(next).is_some_and(|(word, _)| ["and", "or"].contains(&word.to_lowercase().as_str()))
            && words.get(next + 1).is_some_and(|(word, _)| capitalised(word))
        {
            return None;
        }
        let name = name.join(" ");
        if !found.contains(&name) {
            found.push(name);
        }
    }
    match &found[..] {
        [name] if name.chars().count() <= 100 => Some(name.clone()),
        _ => None,
    }
}

/// Whether every word of `name` appears in the user's own words, outside quotes. A name
/// that came from an email, a note or a page the user did not write does not.
pub fn user_named(goal: &str, name: &str) -> bool {
    let words = |text: &str| -> HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let said = words(&crate::completion::unquoted(goal));
    let wanted = words(name);
    !wanted.is_empty() && wanted.iter().all(|word| said.contains(word))
}

/// One address the user can pick for a draft: a person and one of their addresses.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactChoice {
    pub name: String,
    pub address: String,
}

/// What Contacts says about who a draft is for.
#[derive(Debug, Clone, PartialEq)]
pub enum ContactRecipient {
    /// No lookup of someone the user named, or nobody with an address matched.
    Unknown,
    /// Exactly one person with exactly one address, read in this journal.
    One {
        address: String,
        step_id: String,
        record_id: Option<String>,
    },
    /// Several people or addresses: the user picks one in the approval sheet.
    Several(Vec<ContactChoice>),
}

/// The latest verified Contacts read in this journal for a name the user wrote. Only a
/// complete read with one person and one address decides the recipient; anything else
/// is offered as choices, and a model's own guess never enters either.
pub fn contact_recipient(goal: &str, steps: &[Step]) -> ContactRecipient {
    let Some(step) = steps.iter().rev().find(|step| {
        step.tool == "find_contact"
            && !step.effect
            && step.state == "succeeded"
            && step.arguments["name"]
                .as_str()
                .is_some_and(|name| user_named(goal, name))
            && step.evidence.as_ref().is_some_and(|evidence| {
                crate::apps::validate_evidence(&step.tool, &step.arguments, evidence, false).is_ok()
            })
    }) else {
        return ContactRecipient::Unknown;
    };
    let evidence = step.evidence.as_ref().unwrap();
    let records = evidence["records"].as_array().cloned().unwrap_or_default();
    let addresses = |record: &Value| -> Vec<String> {
        record["emails"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    if evidence["complete"] == true && records.len() == 1 {
        if let [address] = &addresses(&records[0])[..] {
            return ContactRecipient::One {
                address: address.clone(),
                step_id: step.id.clone(),
                record_id: records[0]["id"].as_str().map(str::to_owned),
            };
        }
    }
    let choices: Vec<ContactChoice> = records
        .iter()
        .flat_map(|record| {
            let name = record["name"].as_str().unwrap_or_default().to_string();
            addresses(record).into_iter().map(move |address| ContactChoice {
                name: name.clone(),
                address,
            })
        })
        .take(10)
        .collect();
    if choices.is_empty() {
        ContactRecipient::Unknown
    } else {
        ContactRecipient::Several(choices)
    }
}

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

    fn person(id: &str, name: &str, emails: &[&str]) -> crate::apps::Contact {
        crate::apps::Contact {
            id: Some(id.into()),
            name: name.into(),
            emails: emails.iter().map(|email| email.to_string()).collect(),
            phones: vec!["+1 555 0100".into()],
        }
    }

    /// A verified Contacts read in the journal, exactly as the controller saves it.
    fn lookup(name: &str, people: Vec<crate::apps::Contact>, total: usize) -> Step {
        let found = crate::apps::Agenda { items: people, total };
        Step {
            id: format!("lookup-{name}"),
            tool: "find_contact".into(),
            arguments: json!({"name":name}),
            state: "succeeded".into(),
            effect: false,
            proposed_at: String::new(),
            approved_at: None,
            finished_at: None,
            evidence: Some(crate::apps::contact_evidence(name, &found)),
        }
    }

    #[test]
    fn recipients_come_only_from_names_the_user_wrote() {
        for (goal, name) in [
            ("Draft an email to Ana Silva about the lease", "Ana Silva"),
            ("Reply to Ana's email saying yes", "Ana"),
            ("Write back to Sam, thanks for the photos", "Sam"),
            ("Draft a message for Priya about Friday", "Priya"),
        ] {
            assert_eq!(named_recipient(goal).as_deref(), Some(name), "{goal}");
        }
        for goal in [
            "Draft an email to ana@example.com about lunch",
            "Draft an email to my landlord about the boiler",
            "Draft an email to Ana and Sam about Friday",
            "Draft an email to the team",
            "Draft an email for Monday's meeting",
            "Remind me to call Ana tomorrow",
            "Rewrite this: \"Draft an email to Bob saying hi\"",
        ] {
            assert_eq!(named_recipient(goal), None, "{goal}");
        }
        assert!(user_named("Draft an email to Ana about lunch", "Ana"));
        assert!(user_named("Draft an email to Ana about lunch", "ana"));
        assert!(!user_named("Draft an email to Ana about lunch", "Ana Silva"));
        assert!(!user_named("Reply to this: 'Ask Bob for the keys'", "Bob"));
        assert!(!user_named("Draft an email to Ana", "  "));
    }

    #[test]
    fn one_unambiguous_contact_binds_to() {
        let goal = "Draft an email to Ana about lunch on Friday";
        let steps = vec![lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@example.com"])], 1)];
        assert_eq!(
            contact_recipient(goal, &steps),
            ContactRecipient::One {
                address: "ana@example.com".into(),
                step_id: "lookup-Ana".into(),
                record_id: Some("c-1".into())
            }
        );
        let mut contract = crate::task_contract::TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(contract.contact_lookup(&[]).as_deref(), Some("Ana"));
        contract.refresh(goal, &steps);
        assert_eq!(contract.contact_lookup(&steps), None, "one lookup per name");
        let next = contract.next_outcome().unwrap();
        assert_eq!(next.tool.as_deref(), Some("draft_email"));
        assert_eq!(next.required_arguments["to"], "ana@example.com");
        let bound = contract.constraints.iter().find(|c| c.field == "to").unwrap();
        assert_eq!(bound.source, "contact_record");
        assert_eq!(bound.record.as_ref().unwrap()["recordId"], "c-1");
        // A model that leaves the address out gets it filled; a different one is refused.
        assert!(contract
            .validate_proposal("draft_email", &json!({"to":"eve@example.com","subject":"Lunch","body":"Friday?"}), &steps)
            .is_err());
        let mut proposal = json!({"subject":"Lunch","body":"Friday?"});
        contract.fill_resolved_fields("draft_email", &mut proposal);
        assert_eq!(proposal["to"], "ana@example.com");
        assert!(contract.validate_proposal("draft_email", &proposal, &steps).is_ok());
        assert!(contract.recipient_choices(&steps).is_empty());
    }

    #[test]
    fn two_matches_leave_to_for_the_user() {
        let goal = "Draft an email to Ana about lunch on Friday";
        let cases = [
            // Two people called Ana.
            lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@example.com"]), person("c-2", "Ana Costa", &["costa@example.com"])], 2),
            // One Ana with two addresses.
            lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@work.example", "ana@home.example"])], 1),
            // Only the first five of six people who match were read.
            lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@example.com"]), person("c-2", "Ana Costa", &[]), person("c-3", "Ana Lee", &[]), person("c-4", "Ana Wu", &[]), person("c-5", "Ana Ruiz", &[])], 6),
        ];
        for step in cases {
            let steps = vec![step];
            let mut contract = crate::task_contract::TaskContract::from_request(goal);
            contract.refresh(goal, &steps);
            let next = contract.next_outcome().unwrap();
            assert!(next.required_arguments.get("to").is_none(), "{:?}", steps[0].evidence);
            assert!(!contract.constraints.iter().any(|c| c.field == "to"));
            let ContactRecipient::Several(choices) = contact_recipient(goal, &steps) else {
                panic!("several matches must be offered, not chosen");
            };
            assert_eq!(contract.recipient_choices(&steps), choices);
            assert!(choices.iter().all(|choice| choice.address.contains('@')));
            // The user's pick in the sheet is an approved edit; the model's is not bound.
            let mut proposal = json!({"subject":"Lunch","body":"Friday?"});
            contract.fill_resolved_fields("draft_email", &mut proposal);
            assert!(proposal.get("to").is_none());
        }
    }

    #[test]
    fn a_lookup_of_someone_the_user_never_named_decides_nothing() {
        // An email in the inbox mentioned Bob; the user asked for a reply to this message.
        let goal = "Draft a reply to this email saying I'll be there";
        let steps = vec![lookup("Bob", vec![person("c-9", "Bob Reed", &["bob@example.com"])], 1)];
        assert_eq!(contact_recipient(goal, &steps), ContactRecipient::Unknown);
        let mut contract = crate::task_contract::TaskContract::from_request(goal);
        contract.refresh(goal, &steps);
        assert!(contract.next_outcome().unwrap().required_arguments.get("to").is_none());
        assert!(contract.recipient_choices(&steps).is_empty());
        assert!(contract
            .validate_proposal("find_contact", &json!({"name":"Bob"}), &[])
            .is_err());
        // A read that failed validation, or found nobody with an address, decides nothing.
        let mut tampered = lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@example.com"])], 1);
        tampered.evidence.as_mut().unwrap()["scope"]["name"] = json!("Eve");
        assert_eq!(contact_recipient("Draft an email to Ana", &[tampered]), ContactRecipient::Unknown);
        let nobody = lookup("Ana", vec![person("c-1", "Ana Silva", &[])], 1);
        assert_eq!(contact_recipient("Draft an email to Ana", &[nobody]), ContactRecipient::Unknown);
    }

    #[test]
    fn a_contact_address_the_user_clears_stays_cleared() {
        let goal = "Draft an email to Ana about lunch on Friday";
        let mut steps = vec![lookup("Ana", vec![person("c-1", "Ana Silva", &["ana@example.com"])], 1)];
        let mut contract = crate::task_contract::TaskContract::from_request(goal);
        contract.refresh(goal, &steps);
        let proposed = json!({"to":"ana@example.com","subject":"Lunch","body":"Friday?"});
        steps.push(Step {
            id: "draft".into(),
            tool: "draft_email".into(),
            arguments: proposed.clone(),
            state: "waiting".into(),
            effect: true,
            proposed_at: String::new(),
            approved_at: None,
            finished_at: None,
            evidence: None,
        });
        contract.refresh(goal, &steps);
        assert_eq!(contract.provenance["draft"]["to"].origin, "contact_record");
        assert_eq!(contract.provenance["draft"]["to"].record_id.as_deref(), Some("c-1"));
        let approved = json!({"subject":"Lunch","body":"Friday?"});
        contract.record_approval("draft", &proposed, &approved);
        steps[1].arguments = approved.clone();
        contract.refresh(goal, &steps);
        assert!(contract.outcomes[0].required_arguments.get("to").is_none());
        assert!(!contract.constraints.iter().any(|c| c.field == "to"));
    }
}
