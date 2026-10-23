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
}
