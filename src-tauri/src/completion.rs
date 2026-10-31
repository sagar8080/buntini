//! Final-answer guardrails for the task journal. Receipts, not model prose,
//! describe app writes. This deliberately narrow English-language intent filter
//! catches common omissions and false completion claims; it is not a semantic
//! proof that an arbitrary goal has been satisfied.
use crate::{
    actions::{ActionOutcome, ActionReceipt, TOOLS},
    workflows::Step,
};

/// Remove quoted examples and code before interpreting the assistant's own words.
/// Apostrophes inside words are contractions, not the beginning of a quotation.
pub(crate) fn unquoted(text: &str) -> String {
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
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_explanation(text: &str) -> bool {
    [
        "how ",
        "what ",
        "why ",
        "when ",
        "which ",
        "where ",
        "explain ",
        "show me how ",
        "tell me how ",
        "help me understand ",
        "can you explain ",
        "could you explain ",
        "is it possible ",
        "what's ",
        "what’s ",
    ]
    .iter()
    .any(|prefix| text.trim_start().starts_with(prefix))
}

pub(crate) fn request_clause(mut clause: &str) -> &str {
    clause = clause.trim();
    loop {
        let previous = clause;
        for prefix in [
            "please ",
            "can you ",
            "could you ",
            "would you ",
            "i want you to ",
            "i need you to ",
            "help me ",
            "now ",
            // "Tell me what's on my calendar tomorrow" asks the same as without it.
            "tell me ",
            "show me ",
            "give me ",
            "let me know ",
        ] {
            if let Some(rest) = clause.strip_prefix(prefix) {
                clause = rest.trim_start();
                break;
            }
        }
        if clause == previous {
            return clause;
        }
    }
}

pub(crate) fn non_action(clause: &str) -> bool {
    is_explanation(clause)
        || ["don't ", "do not ", "never ", "no need to "]
            .iter()
            .any(|s| clause.starts_with(s))
}

/// Words after "reply to this" that make it a question for Buntini to answer in chat
/// ("respond to this question in one sentence"), not a person or an email.
pub(crate) const ANSWERED_IN_CHAT: &[&str] = &[
    "question", "questions", "prompt", "following", "text", "sentence", "statement",
    "paragraph", "quiz", "riddle", "puzzle", "query", "above", "below", "poem", "joke",
];

/// Whether the words after "reply to" are something to answer in chat: "the following",
/// "this question", but not "her question", which is still a person's email.
pub(crate) fn answered_in_chat(object: &[&str]) -> bool {
    match object {
        [first, ..] if ANSWERED_IN_CHAT.contains(first) => true,
        ["the" | "this" | "that" | "these" | "those" | "my" | "your", noun, ..] => {
            ANSWERED_IN_CHAT.contains(noun)
        }
        _ => false,
    }
}

/// Whether "reply …", "respond …" or "write back …" is addressed to a person or an email,
/// so it needs a Mail draft. "Reply in French", "respond with one word" and "reply to the
/// following in a friendly tone" are about Buntini's own answer.
fn addressed_reply(tokens: &[&str]) -> bool {
    let mut rest = &tokens[1..];
    if rest.first() == Some(&"back") {
        rest = &rest[1..];
    } else if tokens[0] == "write" {
        return false;
    }
    let Some((&"to", object)) = rest.split_first() else {
        // "write back saying yes" still answers whoever wrote; "reply with an email" too.
        return tokens.get(1) == Some(&"back")
            || rest.iter().any(|word| ["email", "mail", "message"].contains(word));
    };
    !object.is_empty() && !answered_in_chat(object)
}

/// "Draft a reply to this email", "write a response to the second one": the same form
/// `focus::reply_object` reads the target from.
fn noun_reply(tokens: &[&str]) -> bool {
    let mut rest = &tokens[1..];
    if rest.first().is_some_and(|word| ["a", "an", "my"].contains(word)) {
        rest = &rest[1..];
    }
    match rest {
        [noun, "to", object @ ..] if ["reply", "response"].contains(noun) => !object.is_empty() && !answered_in_chat(object),
        _ => false,
    }
}

/// Whether a clause asks to change a home device ("turn on the living room lights", "lock
/// the front door", "set the thermostat to 20"), which Buntini does with one of the
/// user's shortcuts. Mac settings ("turn on dark mode") are not devices.
fn home_device(verb: &str, tokens: &[&str]) -> bool {
    const DEVICES: &[&str] = &[
        "light", "lights", "lamp", "lamps", "fan", "fans", "heater", "heating", "thermostat",
        "aircon", "tv", "plug", "plugs", "blinds", "curtains", "kettle", "sprinklers", "purifier",
        "humidifier", "garage", "door", "doors", "gate",
    ];
    let has = |set: &[&str]| tokens.iter().any(|token| set.contains(token));
    // A later or repeated change ("at 7", "every night") is not something to run now.
    if !has(DEVICES)
        || has(&[
            "mode", "timer", "reminder", "volume", "at", "tomorrow", "tonight", "later", "every",
            "when", "minutes", "hours",
        ])
    {
        return false;
    }
    match verb {
        "turn" | "switch" => has(&["on", "off"]),
        "dim" | "brighten" => has(&["light", "lights", "lamp", "lamps"]),
        "lock" | "unlock" => has(&["door", "doors", "garage", "gate"]),
        _ => has(&["thermostat", "heating", "aircon"]),
    }
}

/// Event words that make "move", "rename" or "change" about the calendar.
const EVENT_WORDS: &[&str] = &[
    "event", "events", "meeting", "meetings", "appointment", "appointments", "calendar",
];

/// A day or time of day, as in "to Friday", "to 4", "to 4:30pm", "to tomorrow".
fn temporal(word: &str) -> bool {
    let digits = word.trim_end_matches("pm").trim_end_matches("am");
    [
        "today", "tonight", "tomorrow", "noon", "midday", "monday", "tuesday", "wednesday",
        "thursday", "friday", "saturday", "sunday", "am", "pm",
    ]
    .contains(&word)
        || (!digits.is_empty() && digits.len() <= 4 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// "move my 3pm to 4", "push the dentist to Friday", "rename my 10:30 to Planning":
/// changing an existing event. Other things can be moved too (tabs, files, money), so
/// "move" needs an event word or a day or time to move it to.
fn edits_event(verb: &str, tokens: &[&str]) -> bool {
    let has = |targets: &[&str]| tokens.iter().any(|token| targets.contains(token));
    let destination = tokens
        .iter()
        .rposition(|word| *word == "to" || *word == "for")
        .map(|at| &tokens[at + 1..])
        .unwrap_or(&[]);
    if has(&["tab", "tabs", "file", "files", "folder", "folders", "money", "email", "emails", "mail", "reminder", "reminders", "note", "notes"]) {
        return false;
    }
    match verb {
        "reschedule" | "postpone" => true,
        // "push my 3pm back by an hour", "move lunch 30 minutes later"
        "move" | "push" | "shift" | "bump" | "delay" | "pull" | "bring" if crate::edit_targets::shift(tokens).is_some() => true,
        "move" | "push" | "shift" | "bump" | "delay" | "pull" => {
            tokens.contains(&"to") && (has(EVENT_WORDS) || destination.iter().any(|word| temporal(word)))
        }
        "rename" => tokens.contains(&"to") && (has(EVENT_WORDS) || tokens.get(1).is_some_and(|word| ["my", "the"].contains(word))),
        "change" => tokens.contains(&"to") && (has(EVENT_WORDS) || has(&["location", "venue"])),
        _ => false,
    }
}

/// "move the dentist reminder to Friday", "push my call Ana reminder back a day": a new
/// due date for a reminder the user already has.
fn reschedules_reminder(verb: &str, tokens: &[&str]) -> bool {
    if !tokens.iter().any(|token| ["reminder", "reminders"].contains(token)) {
        return false;
    }
    let destination = tokens
        .iter()
        .rposition(|word| *word == "to" || *word == "for")
        .map(|at| &tokens[at + 1..])
        .unwrap_or(&[]);
    match verb {
        "move" | "push" | "shift" | "bump" | "delay" | "pull" | "bring" | "reschedule" | "postpone" | "change" => {
            crate::edit_targets::shift(tokens).is_some() || destination.iter().any(|word| temporal(word))
        }
        _ => false,
    }
}

/// "add milk to my shopping note": adding to an existing note, not writing a new one.
fn appends_to_note(tokens: &[&str]) -> bool {
    let Some(to) = tokens.iter().rposition(|word| *word == "to") else {
        return false;
    };
    let name: Vec<&&str> = tokens[to + 1..]
        .iter()
        .skip_while(|word| ["my", "the", "our"].contains(*word))
        .collect();
    // What is added may be quoted, and so already removed from these words.
    to >= 1
        && name.last().is_some_and(|word| ["note", "notes"].contains(*word))
        && (2..=5).contains(&name.len())
        && !(["a", "an", "new"].contains(&tokens[1]) && tokens.get(2) == Some(&"note"))
}

/// "add milk, eggs and bread to my shopping note" is one request: join its list so the
/// clause splitting at "and" and commas below keeps it together. Lowercase text only.
pub(crate) fn joined_edit_lists(text: &str) -> String {
    let mut result = String::new();
    for sentence in text.split_inclusive([';', '\n', '.', '!', '?']) {
        let spans: Vec<(usize, &str)> = sentence
            .split_whitespace()
            .map(|word| (word.as_ptr() as usize - sentence.as_ptr() as usize, word.trim_matches(|c: char| !c.is_alphanumeric())))
            .collect();
        let head = spans.iter().position(|(_, word)| ["note", "notes"].contains(word)).and_then(|end| {
            let to = (0..end).rev().take(6).find(|&index| spans[index].1 == "to")?;
            let verb = (0..to).rev().find(|&index| ["add", "append", "put", "jot"].contains(&spans[index].1))?;
            Some((spans[verb].0, spans[to].0))
        });
        match head {
            Some((start, end)) => {
                result.push_str(&sentence[..start]);
                result.push_str(&sentence[start..end].replace(" and ", " & ").replace(',', " &"));
                result.push_str(&sentence[end..]);
            }
            None => result.push_str(sentence),
        }
    }
    result
}

/// A clause cut from lowercased `joined_edit_lists` text, back in the user's own words:
/// their capitals, and each " &" the joining wrote as the "and" or comma it replaced
/// wherever `original` says so. "Add oat milk and eggs to my note" then adds "oat milk
/// and eggs", not "oat milk & eggs".
pub(crate) fn unjoined(clause: &str, original: &str) -> String {
    let parts: Vec<&str> = clause.split(" &").collect();
    let joins = parts.len() - 1;
    if joins > 6 {
        return clause.into();
    }
    // Lowercasing that changes a text's length can't map a match back to its letters.
    let lower = original.to_lowercase();
    let same_length = lower.len() == original.len();
    for mut choice in 0..3usize.pow(joins as u32) {
        let mut text = parts[0].to_string();
        for part in &parts[1..] {
            text.push_str([" and", ",", " &"][choice % 3]);
            choice /= 3;
            text.push_str(part);
        }
        if let Some(start) = lower.find(&text.to_lowercase()) {
            return match same_length && original.is_char_boundary(start) && original.is_char_boundary(start + text.len()) {
                true => original[start..start + text.len()].to_string(),
                false => text,
            };
        }
    }
    clause.into()
}

pub(crate) fn requested_actions(prompt: &str) -> (Vec<&'static str>, bool) {
    // "Follow up with Ana until she replies" asks for a goal now; each follow-up is a
    // check-in's own task.
    if crate::goals::parse_request(prompt).is_some() {
        return (vec![crate::goals::TOOL], false);
    }
    // "Every weekday at 8, draft…" asks for a routine now; the draft is for each run.
    if crate::routines::parse_request(prompt).is_some() {
        return (vec![crate::routines::TOOL], false);
    }
    let prompt = joined_edit_lists(&unquoted(&prompt.replace("Additional details: ", "\n")));
    let mut expected = Vec::new();
    let mut unsupported = false;
    // An explanation or negation scopes over its coordinated clauses. Splitting
    // "don't create a note and add a reminder" first would invent permission.
    for sentence in prompt.split([';', '\n', '.', '!', '?']) {
        let sentence = request_clause(sentence);
        if non_action(sentence) {
            continue;
        }
        let clauses = sentence
            .replace(" and then ", ";")
            .replace(" then ", ";")
            .replace(" and ", ";")
            .replace(" also ", ";");
        for clause in clauses.split([';', '\n', ',']) {
            let clause = request_clause(clause);
            if non_action(clause) {
                continue;
            }
            let tokens = words(clause);
            let Some(verb) = tokens.first().copied() else {
                continue;
            };
            let has = |targets: &[&str]| tokens.iter().any(|token| targets.contains(token));
            let tool = match verb {
                "remember" => Some("remember"),
                // Changing an item the user already has: its id comes from a read.
                "mark" if has(&["done", "complete", "completed", "finished"]) => {
                    Some("complete_reminder")
                }
                "tick" | "check" | "cross"
                    if tokens.get(1) == Some(&"off") || tokens.last() == Some(&"off") =>
                {
                    Some("complete_reminder")
                }
                "complete" | "finish" if has(&["reminder", "reminders", "task", "todo"]) => {
                    Some("complete_reminder")
                }
                verb if edits_event(verb, &tokens) => Some("update_calendar_event"),
                verb if reschedules_reminder(verb, &tokens) => Some("reschedule_reminder"),
                "add" | "append" | "put" | "jot" if appends_to_note(&tokens) => {
                    Some("append_to_note")
                }
                "set" | "start" | "create" | "add" if has(&["timer", "timers", "countdown"]) => {
                    Some("set_timer")
                }
                "play" | "pause" | "resume" | "skip" | "stop"
                    if !has(&["timer", "task", "search", "watch"]) =>
                {
                    Some("control_music")
                }
                "switch" | "go" | "focus" if has(&["tab", "tabs"]) => Some("switch_tab"),
                "save" | "create" | "add" | "set" if has(&["search", "searches"]) => {
                    Some("create_saved_search")
                }
                "search" | "check"
                    if has(&["every", "hourly", "daily", "weekly", "regularly"])
                        && !has(&["price", "prices"]) =>
                {
                    Some("create_saved_search")
                }
                "create" | "add" | "set" | "watch" | "track" | "monitor" | "check" | "notify"
                | "keep"
                    if has(&["price", "prices"]) =>
                {
                    Some("create_price_watch")
                }
                "remind" if tokens.get(1) == Some(&"me") => Some("add_reminder"),
                "add" | "create" | "set" | "make" if has(&["reminder", "reminders"]) => {
                    Some("add_reminder")
                }
                "add" | "create" | "make" | "schedule"
                    if has(&["calendar", "event", "events", "meeting", "appointment"]) =>
                {
                    Some("add_calendar_event")
                }
                "schedule" if !has(&["email", "message", "delivery", "payment"]) => {
                    Some("add_calendar_event")
                }
                "create" | "make" | "write" | "save" | "add" | "take"
                    if has(&["note", "notes"]) =>
                {
                    Some("create_note")
                }
                // A reply answers one email, in its thread ("reply to this", "write back
                // to her", "draft a reply to this email"); a new email is a draft. Neither
                // is ever sent: the user sends it from Mail.
                "reply" | "respond" | "write" if addressed_reply(&tokens) => Some("reply_email"),
                // Only when the reply is what to write ("draft a reply to this email");
                // "an email to my landlord in response to …" is a new email.
                "draft" | "prepare" | "write" if noun_reply(&tokens) => Some("reply_email"),
                "draft" | "prepare" | "write" if has(&["email", "mail", "message"]) => {
                    Some("draft_email")
                }
                "open"
                    if has(&["page", "site", "website", "url", "link", "https"])
                        || clause.contains("https://") =>
                {
                    Some("open_link")
                }
                "open" if tokens.len() == 1 => {
                    if prompt.contains("page") || prompt.contains("https://") {
                        Some("open_link")
                    } else {
                        None
                    }
                }
                "open" => Some("open_app"),
                "run" if has(&["shortcut", "shortcuts"]) => Some("run_shortcut"),
                // Home devices are run through the user's own shortcuts, never guessed.
                "turn" | "switch" | "dim" | "brighten" | "lock" | "unlock" | "set"
                    if home_device(verb, &tokens) =>
                {
                    Some("run_shortcut")
                }
                "send" | "email" | "text" | "post" | "publish" | "book" | "reserve" | "buy"
                | "purchase" | "order" | "pay" | "delete" | "cancel" | "move" | "update"
                | "change" => {
                    unsupported = true;
                    None
                }
                _ => None,
            };
            if let Some(tool) = tool {
                // Separate clauses requesting the same tool are separate obligations.
                // Explicit counts before the object noun also require that many receipts.
                let object = tokens.iter().position(|word| {
                    [
                        "reminder",
                        "reminders",
                        "note",
                        "notes",
                        "event",
                        "events",
                        "timer",
                        "timers",
                        "draft",
                        "drafts",
                    ]
                    .contains(word)
                });
                let count = object
                    .filter(|index| tokens[*index].ends_with('s'))
                    .and_then(|index| {
                        tokens.iter().take(index).enumerate().skip(1).find_map(
                            |(position, word)| {
                                if tokens.get(position + 1).is_some_and(|next| {
                                    [
                                        "second", "seconds", "minute", "minutes", "hour", "hours",
                                        "day", "days",
                                    ]
                                    .contains(next)
                                }) {
                                    return None;
                                }
                                match *word {
                                    "two" | "both" | "couple" => Some(2),
                                    "three" => Some(3),
                                    "four" => Some(4),
                                    "five" => Some(5),
                                    "six" => Some(6),
                                    "seven" => Some(7),
                                    "eight" => Some(8),
                                    "nine" => Some(9),
                                    "ten" => Some(10),
                                    "eleven" => Some(11),
                                    "twelve" => Some(12),
                                    other => other.parse::<usize>().ok().filter(|count| *count > 0),
                                }
                            },
                        )
                    })
                    .unwrap_or(1)
                    .min(crate::workflows::MAX_STEPS + 1);
                expected.extend(std::iter::repeat_n(tool, count));
            }
        }
    }
    (expected, unsupported)
}

/// Untrusted source text may suggest actions, but only the saved user goal may
/// authorize proposing one. Ambiguous goals can be clarified through Resume.
pub fn validate_proposal(goal: &str, tool: &str, steps: &[Step]) -> Result<(), String> {
    if steps
        .iter()
        .any(|step| matches!(step.state.as_str(), "failed" | "unknown" | "running"))
    {
        return Err("An earlier step is unresolved. Buntini cannot propose another action until its result is known.".into());
    }
    if !action_remaining(goal, tool, steps) {
        return Err(format!("Buntini proposed {tool}, but could not establish that your request asks for it. No action ran. Add an explicit instruction in the task’s additional details to continue."));
    }
    if next_action(goal, steps) != Some(tool) {
        return Err("An earlier requested action is still unfinished. Buntini must complete the task in the requested order before proposing this action.".into());
    }
    require_reads(goal, steps)?;
    Ok(())
}

/// A completed obligation removes its write tool from subsequent planning turns.
/// This uses only the user's saved goal and controller-owned step states. Source
/// text and model narration cannot add an obligation or restore a consumed one.
pub fn action_remaining(goal: &str, tool: &str, steps: &[Step]) -> bool {
    let requested = requested_actions(goal)
        .0
        .iter()
        .filter(|name| **name == tool)
        .count();
    let finished = steps
        .iter()
        .filter(|step| step.effect && step.tool == tool && step.state == "succeeded")
        .count();
    requested > finished
}

fn next_action(goal: &str, steps: &[Step]) -> Option<&'static str> {
    let mut completed = steps
        .iter()
        .filter(|step| step.effect && step.state == "succeeded")
        .map(|step| step.tool.as_str())
        .collect::<Vec<_>>();
    for expected in requested_actions(goal).0 {
        if let Some(index) = completed.iter().position(|tool| *tool == expected) {
            completed.remove(index);
        } else {
            return Some(expected);
        }
    }
    None
}

/// Offer only writes justified by this task, before inference. The dispatch-time
/// validation above remains mandatory, including for providers ignoring schemas.
pub fn planning_tools(
    goal: &str,
    steps: &[Step],
    definitions: &serde_json::Value,
) -> serde_json::Value {
    let unresolved = steps
        .iter()
        .any(|step| matches!(step.state.as_str(), "failed" | "unknown" | "running"));
    let tools = definitions.as_array().into_iter().flatten().filter_map(|definition| {
        let name = definition["name"].as_str()?;
        let write = TOOLS.contains(&name) || crate::monitor_actions::TOOLS.contains(&name) || crate::control_actions::TOOLS.contains(&name);
        let allowed = !unresolved && next_action(goal, steps) == Some(name) && require_reads(goal, steps).is_ok();
        if name == "control_music" && !allowed {
            let mut read_only = definition.clone();
            read_only["parameters"]["properties"]["action"]["enum"] = serde_json::json!(["now_playing"]);
            read_only["description"] = serde_json::json!("Read what is currently playing. Playback changes are unavailable for this task.");
            Some(read_only)
        } else if write && !allowed { None } else { Some(definition.clone()) }
    }).collect::<Vec<_>>();
    serde_json::Value::Array(tools)
}

fn require_reads(goal: &str, steps: &[Step]) -> Result<(), String> {
    // A routine's or a goal's reads happen when it runs, not when it is set up.
    if crate::routines::parse_request(goal).is_some() || crate::goals::parse_request(goal).is_some() {
        return Ok(());
    }
    // Online topic words are not requests to read private apps. The durable
    // task contract separately requires the integrated online evidence.
    let search_clauses = crate::search_intent::explicit_clause_spans(goal);
    if search_clauses.len() == 1
        && search_clauses[0].0 == 0
        && search_clauses[0].1 == goal.len()
        && requested_actions(goal) == (vec![], false)
    {
        return Ok(());
    }
    let text = joined_edit_lists(&unquoted(goal))
        .replace("additional details: ", "\n")
        .replace(" and then ", ";")
        .replace(" then ", ";")
        .replace(" and ", ";");
    let mut missing = Vec::new();
    for clause in text.split([';', '\n', ',', '.', '!', '?']) {
        let clause = clause
            .trim()
            .trim_start_matches("please ")
            .trim_start_matches("can you ")
            .trim_start_matches("could you ");
        if crate::search_intent::explicit_request(clause).is_some() {
            continue;
        }
        let tokens = words(clause);
        if !tokens.first().is_some_and(|word| {
            [
                "read",
                "check",
                "find",
                "search",
                "list",
                "summarise",
                "summarize",
                "review",
                "compare",
            ]
            .contains(word)
        }) {
            continue;
        }
        let required = crate::task_contract::read_tools(clause);
        if !required.is_empty()
            && !steps.iter().any(|step| {
                !step.effect
                    && step.state == "succeeded"
                    && step.evidence.is_some()
                    && required.contains(&step.tool.as_str())
            })
        {
            missing.push(required.join(" or "));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("A requested source has not been read successfully ({}). This task is unfinished; continue it to gather that evidence before relying on a result or approving dependent actions.", missing.join(", ")))
    }
}

pub(crate) fn claims_own_write(text: &str) -> bool {
    let text = unquoted(text);
    let tokens = words(&text);
    let writes = [
        "created",
        "added",
        "saved",
        "sent",
        "booked",
        "reserved",
        "purchased",
        "ordered",
        "scheduled",
        "deleted",
        "cancelled",
        "canceled",
        "updated",
        "changed",
        "moved",
        "ran",
        "opened",
        "submitted",
        "posted",
        "paid",
        "transferred",
        "emailed",
        "set",
        "enabled",
    ];
    for (index, token) in tokens.iter().enumerate() {
        if !["i", "i've", "we", "we've"].contains(token) {
            continue;
        }
        let mut tail = &tokens[index + 1..];
        while tail
            .first()
            .is_some_and(|w| ["have", "just", "already", "successfully", "now"].contains(w))
        {
            tail = &tail[1..];
        }
        if tail.first().is_some_and(|word| writes.contains(word)) {
            return true;
        }
    }
    // Direct result announcements often omit the assistant's pronoun. Limit
    // these patterns to sentence/phrase starts so how-to explanations still work.
    for clause in text.split(['.', '!', '\n', ';', ',']) {
        let clause_words = words(clause.trim());
        let prefix = &clause_words[..clause_words.len().min(12)];
        if prefix.first().is_some_and(|word| writes.contains(word))
            && prefix
                .get(1)
                .is_some_and(|word| ["your", "the", "a", "an", "it", "to"].contains(word))
        {
            return true;
        }
        if prefix.first() == Some(&"your")
            && !prefix.iter().any(|word| {
                [
                    "says",
                    "said",
                    "describes",
                    "mentions",
                    "not",
                    "never",
                    "isn't",
                    "wasn't",
                    "hasn't",
                ]
                .contains(word)
            })
            && prefix
                .iter()
                .any(|word| ["is", "was", "been"].contains(word))
            && prefix.iter().any(|word| writes.contains(word))
        {
            return true;
        }
    }
    false
}

fn receipt_summary(messages: &[String]) -> String {
    if messages.len() == 1 {
        messages[0].clone()
    } else {
        messages
            .iter()
            .map(|message| format!("• {message}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Receipt-only completion is reserved for recognizable action/read clauses.
/// Any other continuation keeps the answer phase, including verbs we have never
/// classified (a blacklist would silently drop new kinds of requested work).
pub(crate) fn receipt_only_goal(goal: &str) -> bool {
    if crate::routines::parse_request(goal).is_some() || crate::goals::parse_request(goal).is_some() {
        return true;
    }
    let plain = joined_edit_lists(&unquoted(&goal.replace("Additional details: ", "\n")))
        .replace(" and then ", ";")
        .replace(" then ", ";")
        .replace(" also ", ";");
    let mut saw_action = false;
    let mut previous_action = false;
    let mut previous_calendar_read = false;
    let mut previous_read = false;
    for sentence in plain.split([';', '\n', '.', '!', '?']) {
        for segment in sentence.split(',') {
            let segment = request_clause(segment);
            if segment.is_empty() {
                continue;
            }
            // In this explicit content form the rest of the clause is the note
            // body. Coordinating words inside it do not request separate work.
            let literal_body =
                segment.contains(" with body ") && requested_actions(segment).0 == ["create_note"];
            let clauses = if literal_body {
                vec![segment]
            } else {
                segment.split(" and ").collect()
            };
            for clause in clauses {
                let clause = request_clause(clause);
                if clause.is_empty() {
                    continue;
                }
                let (actions, unsupported) = requested_actions(clause);
                if !actions.is_empty() && !unsupported {
                    saw_action = true;
                    previous_action = true;
                    previous_calendar_read = false;
                    previous_read = false;
                    continue;
                }
                if previous_action && clause.starts_with("due ") {
                    continue;
                }
                if previous_calendar_read {
                    let dates: Vec<_> = clause
                        .split_whitespace()
                        .filter(|word| !["on", "for"].contains(word))
                        .collect();
                    if !dates.is_empty()
                        && dates
                            .iter()
                            .all(|word| chrono::NaiveDate::parse_from_str(word, "%Y-%m-%d").is_ok())
                    {
                        continue;
                    }
                }
                let tokens = words(clause);
                let coordinated_read = previous_read
                    && tokens
                        .first()
                        .is_some_and(|w| ["my", "the", "this", "these"].contains(w))
                    && tokens.iter().any(|w| {
                        [
                            "calendar",
                            "events",
                            "appointments",
                            "mail",
                            "email",
                            "emails",
                            "inbox",
                            "reminders",
                            "note",
                            "notes",
                            "page",
                            "website",
                            "webpage",
                            "tabs",
                            "files",
                            "folder",
                            "folders",
                        ]
                        .contains(w)
                    });
                let read = coordinated_read
                    || tokens
                        .first()
                        .is_some_and(|word| ["read", "check", "find", "search"].contains(word))
                        && tokens.iter().any(|word| {
                            [
                                "calendar",
                                "events",
                                "appointments",
                                "mail",
                                "email",
                                "emails",
                                "inbox",
                                "reminders",
                                "note",
                                "notes",
                                "page",
                                "website",
                                "webpage",
                                "tabs",
                                "files",
                                "folder",
                                "folders",
                            ]
                            .contains(word)
                        });
                if !read {
                    return false;
                }
                previous_action = false;
                previous_read = read;
                previous_calendar_read = tokens
                    .iter()
                    .any(|word| ["calendar", "events", "appointments"].contains(word));
            }
        }
    }
    saw_action
}

/// Pure action workflows finish from their verified receipts. Requests with
/// additional work retain a model answer and the usual completion checks.
pub fn completed_action_answer(goal: &str, steps: &[Step]) -> Option<String> {
    if !receipt_only_goal(goal)
        || steps
            .iter()
            .any(|step| !matches!(step.state.as_str(), "succeeded" | "superseded"))
    {
        return None;
    }
    validate_answer(goal, "", steps)
        .ok()
        .filter(|answer| !answer.trim().is_empty())
}

/// Validate a final response, replacing untrusted success narration with actual
/// action receipts. Err means the goal remains incomplete and must not display a
/// completed task badge. Read-only answers remain model-generated.
pub fn validate_answer(prompt: &str, text: &str, steps: &[Step]) -> Result<String, String> {
    require_reads(prompt, steps)?;
    let (expected, unsupported) = requested_actions(prompt);
    let mut verified = Vec::new();
    let mut messages = Vec::new();
    let mut observations = Vec::new();
    for step in steps.iter().filter(|step| {
        step.tool == "browse_page"
            || (step.effect
                && (TOOLS.contains(&step.tool.as_str())
                    || crate::monitor_actions::TOOLS.contains(&step.tool.as_str())
                    || crate::control_actions::TOOLS.contains(&step.tool.as_str())))
    }) {
        if step.state != "succeeded" {
            return Err("An app action has no confirmed success receipt. Review its saved progress before relying on this task.".into());
        }
        let receipt = step.evidence.as_ref().and_then(|e| e.get("receipt")).cloned()
            .and_then(|value| serde_json::from_value::<ActionReceipt>(value).ok())
            .ok_or("An app action is missing its verification receipt. This task cannot be confirmed complete.")?;
        if receipt.outcome != ActionOutcome::Succeeded
            || receipt.tool != step.tool
            || receipt.message.trim().is_empty()
        {
            return Err("An app action has an inconsistent verification receipt. This task cannot be confirmed complete.".into());
        }
        if [
            "add_reminder",
            "add_calendar_event",
            "create_note",
            "draft_email",
            "reply_email",
            "create_saved_search",
            "create_price_watch",
            crate::routines::TOOL,
            crate::goals::TOOL,
            "remember",
            "set_timer",
            "control_music",
            "switch_tab",
            "browse_page",
            "complete_reminder",
            "reschedule_reminder",
            "update_calendar_event",
            "append_to_note",
        ]
        .contains(&step.tool.as_str())
            && receipt.object_id.as_deref().is_none_or(str::is_empty)
        {
            return Err("The created record has no destination identifier. This task cannot be confirmed complete.".into());
        }
        if step.tool == "browse_page" {
            verified.push("open_link");
            observations.push(receipt.message);
        } else {
            verified.push(step.tool.as_str());
            messages.push(receipt.message);
        }
    }
    let mut remaining = verified;
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|tool| {
            if let Some(index) = remaining.iter().position(|done| done == tool) {
                remaining.remove(index);
                false
            } else {
                true
            }
        })
        .collect();
    if unsupported || !missing.is_empty() {
        let explanation = if unsupported {
            "The requested task includes an action Buntini cannot verify or perform through its current tools. That part is unfinished."
        } else {
            "A requested app action has no verified receipt. That part is unfinished; Buntini has not confirmed that it was done."
        };
        let mut answer = if messages.is_empty() {
            explanation.into()
        } else {
            format!("{}\n\n{explanation}", receipt_summary(&messages))
        };
        let lower = text.to_lowercase();
        if !claims_own_write(text)
            && (text.contains('?')
                || [
                    "cannot",
                    "can't",
                    "can’t",
                    "unable",
                    "need you",
                    "please provide",
                ]
                .iter()
                .any(|phrase| lower.contains(phrase)))
        {
            answer = format!("{}\n\n{answer}", text.trim());
        }
        return Err(answer);
    }
    if !messages.is_empty() {
        let summary = receipt_summary(&messages);
        if receipt_only_goal(prompt) {
            return Ok(summary);
        }
        if text.trim().is_empty() {
            return Err(format!("{summary}\n\nThe requested explanation or additional answer is still missing. Continue the task to finish that part."));
        }
        if claims_own_write(text) {
            return Err(format!("{summary}\n\nThe requested explanation or additional answer is still unfinished. Buntini could not provide it without an unverified action claim; continue the task to finish that part."));
        }
        return Ok(format!("{summary}\n\n{}", text.trim()));
    }
    if !observations.is_empty() {
        let observed = receipt_summary(&observations);
        return Ok(if claims_own_write(text) {
            observed
        } else {
            format!("{}\n\n{observed}", text.trim())
        });
    }
    if claims_own_write(text) {
        let plain = unquoted(prompt);
        let request = request_clause(&plain);
        let verb = words(request).first().copied().unwrap_or("");
        let (actions, unsupported) = requested_actions(prompt);
        if actions.is_empty() && !unsupported
            && ["rewrite", "rephrase", "polish", "proofread", "translate", "draft"].contains(&verb)
        {
            // A generated message may speak in the user's first person ("I
            // ordered a monitor"). Render it explicitly as quoted text so it
            // cannot be mistaken for an assistant execution announcement.
            return Ok(format!("Text for review:\n\n{}",text.lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")));
        }
        return Err("The response claimed an action without a verified receipt. Buntini has not confirmed that it was done.".into());
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_rewritten_first_person_message_is_quoted_instead_of_claiming_execution() {
        let answer = validate_answer("Rewrite this as a polite email: 'I ordered a monitor and it arrived damaged.'", "Hi Maya, I ordered a monitor and it arrived damaged. Could you replace it?", &[]).unwrap();
        assert!(answer.starts_with("Text for review:\n\n> "));
        assert!(!claims_own_write(&answer));
        assert!(validate_answer("Thanks", "I ordered a monitor.", &[]).is_err());
        assert!(validate_answer("Rewrite this and add a reminder", "I created the reminder.", &[]).is_err());
    }

    #[test]
    fn requests_mixing_this_iterations_writes_each_get_their_own() {
        // Replies, changes to existing items, shortcut runs and routines share one grammar:
        // each clause keeps its own write, and a schedule makes the whole request a routine.
        for (prompt, expected) in [
            ("Mark buy milk as done and reply to the second one", vec!["complete_reminder", "reply_email"]),
            ("Turn on the living room lights and move my 3pm to 4", vec!["run_shortcut", "update_calendar_event"]),
            ("Add milk to my shopping note and lock the front door", vec!["append_to_note", "run_shortcut"]),
            ("Every weekday at 8, mark buy milk as done", vec!["create_routine"]),
            ("Every day at 7pm, turn off the living room lights", vec!["create_routine"]),
            ("Reply to Ana's email", vec!["reply_email"]),
        ] {
            assert_eq!(requested_actions(prompt), (expected, false), "{prompt}");
        }
    }

    #[test]
    fn home_device_requests_run_one_of_the_users_shortcuts() {
        for prompt in [
            "Turn on the living room lights",
            "please switch off the kitchen lights",
            "Dim the bedroom lamp",
            "Lock the front door",
            "Set the thermostat to 20",
            "turn the fan off",
        ] {
            assert_eq!(requested_actions(prompt), (vec!["run_shortcut"], false), "{prompt}");
        }
        assert_eq!(
            requested_actions("Turn on the lights and lock the front door").0,
            ["run_shortcut", "run_shortcut"]
        );
        for prompt in [
            "Turn on dark mode",
            "turn the volume down",
            "Don't turn on the lights",
            "How do I turn on the lights in Home?",
            "Turn on the lights at 7pm",
            "turn on the lights every night",
            "Lock my screen",
            "switch to the lights tab",
        ] {
            assert!(!requested_actions(prompt).0.contains(&"run_shortcut"), "{prompt}");
        }
    }

    fn saved(tool: &str, message: &str) -> Step {
        Step {
            id: "intent-1".into(),
            tool: tool.into(),
            arguments: json!({}),
            state: "succeeded".into(),
            effect: true,
            proposed_at: "now".into(),
            approved_at: Some("now".into()),
            finished_at: Some("now".into()),
            evidence: Some(json!({"receipt":ActionReceipt {
                outcome: ActionOutcome::Succeeded, tool: tool.into(), object_id: Some("record-123".into()), destination: Some("Fixture".into()), message: message.into(), evidence: json!({}), error: None
            }})),
        }
    }

    #[test]
    fn planning_offers_only_remaining_requested_writes() {
        let all = crate::tools::offer(&[
            "read_calendar",
            "add_reminder",
            "create_note",
            "control_music",
        ]);
        let names = |value: &serde_json::Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let read_only = planning_tools("What is my first calendar event?", &[], &all);
        assert_eq!(names(&read_only), ["read_calendar", "control_music"]);
        assert_eq!(
            read_only[1]["parameters"]["properties"]["action"]["enum"],
            json!(["now_playing"])
        );
        let dependent = "Read my calendar, then create a reminder";
        assert!(!names(&planning_tools(dependent, &[], &all)).contains(&"add_reminder".into()));
        let mut calendar = saved("read_calendar", "Observed calendar");
        calendar.effect = false;
        assert!(
            names(&planning_tools(dependent, &[calendar], &all)).contains(&"add_reminder".into())
        );
        let goal = "Create a note, then add a reminder";
        let note = saved("create_note", "Saved.");
        assert!(names(&planning_tools(goal, &[], &all)).contains(&"create_note".into()));
        assert!(!names(&planning_tools(goal, &[], &all)).contains(&"add_reminder".into()));
        assert!(validate_proposal(goal, "add_reminder", &[]).is_err());
        assert!(
            !names(&planning_tools(goal, &[note.clone()], &all)).contains(&"create_note".into())
        );
        assert!(validate_proposal(goal, "create_note", &[note.clone()]).is_err());
        assert!(validate_proposal(goal, "add_reminder", &[note.clone()]).is_ok());
        assert!(action_remaining(
            "Create two notes",
            "create_note",
            &[note.clone()]
        ));
        for state in ["failed", "unknown", "running"] {
            let mut unresolved = note.clone();
            unresolved.state = state.into();
            assert_eq!(
                names(&planning_tools(goal, &[unresolved], &all)),
                ["read_calendar", "control_music"]
            );
        }
    }

    #[test]
    fn coordinated_negations_and_explanations_do_not_authorize_later_writes() {
        for goal in [
            "Explain how to create a note and add a reminder",
            "Do not create a note and add a reminder",
            "Please don't create a note, then add a reminder",
            "Can you explain how to create a note and then add a reminder?",
        ] {
            assert!(requested_actions(goal).0.is_empty(), "{goal}");
            assert!(
                validate_proposal(goal, "add_reminder", &[]).is_err(),
                "{goal}"
            );
        }
        assert_eq!(
            requested_actions("Create a note. Add a reminder.").0,
            ["create_note", "add_reminder"]
        );
        assert_eq!(
            requested_actions("Don't create a note; add a reminder.").0,
            ["add_reminder"]
        );
    }

    #[test]
    fn pure_action_tasks_finish_from_receipts_without_another_model_decision() {
        let note = saved("create_note", "Verified note.");
        assert_eq!(
            completed_action_answer("Create a note", &[note.clone()]).as_deref(),
            Some("Verified note.")
        );
        assert!(
            completed_action_answer("Create a note and add a reminder", &[note.clone()]).is_none()
        );
        assert!(
            completed_action_answer("Create a note and explain its contents", &[note.clone()])
                .is_none()
        );
        assert!(
            completed_action_answer("Read my calendar and create a note", &[note.clone()])
                .is_none()
        );
        let mut unverified = note;
        unverified.evidence = None;
        assert!(completed_action_answer("Create a note", &[unverified]).is_none());
    }

    #[test]
    fn additional_answers_are_not_skipped_or_replaced_by_action_receipts() {
        let note = saved("create_note", "Verified note.");
        let with_body =
            "Create a note titled Budget with body Review spending, then calculate 15% of 80.";
        assert!(completed_action_answer(with_body, &[note.clone()]).is_none());
        assert_eq!(
            validate_answer(with_body, "15% of 80 is 12.", &[note.clone()]).unwrap(),
            "Verified note.\n\n15% of 80 is 12."
        );
        for (continuation, answer) in [
            ("calculate 15% of 80", "15% of 80 is 12."),
            ("translate hello into French", "Hello in French is bonjour."),
            (
                "explain what a budget is",
                "A budget is a plan for income and spending.",
            ),
            ("invent a short poem", "A little plan, a little light."),
        ] {
            for separator in [", then ", " and ", ". "] {
                let goal = format!("Create a note{separator}{continuation}");
                assert!(
                    completed_action_answer(&goal, &[note.clone()]).is_none(),
                    "{goal}"
                );
                assert_eq!(
                    validate_answer(&goal, answer, &[note.clone()]).unwrap(),
                    format!("Verified note.\n\n{answer}")
                );
                assert!(validate_answer(&goal, "", &[note.clone()]).is_err());
                let unverified = validate_answer(
                    &goal,
                    "I sent the email. Here is the answer.",
                    &[note.clone()],
                )
                .unwrap_err();
                assert!(
                    unverified.contains("unfinished") && !unverified.contains("I sent the email")
                );
            }
        }
    }

    #[test]
    fn supported_note_and_calendar_reminder_tasks_keep_receipt_completion() {
        let note = saved("create_note", "Verified note.");
        let reminder = saved("add_reminder", "Verified reminder.");
        let note_goal = "Create a note titled Packing with body Passport and adapter, then add a reminder titled Check Packing due 2026-10-23 09:00.";
        let done = completed_action_answer(note_goal, &[note, reminder.clone()]).unwrap();
        assert!(done.contains("Verified note.") && done.contains("Verified reminder."));
        let mut read = saved("read_calendar", "Observed calendar.");
        read.effect = false;
        let calendar_goal = "Read my calendar for 2026-10-23, then create a reminder to prepare for my first event, due 30 minutes before it starts.";
        assert_eq!(
            completed_action_answer(calendar_goal, &[read, reminder]).as_deref(),
            Some("Verified reminder.")
        );
        assert!(completed_action_answer(
            "Create a note, then check the weather",
            &[saved("create_note", "Verified note.")]
        )
        .is_none());
    }

    #[test]
    fn unsupported_and_omitted_actions_cannot_be_announced_as_done() {
        for (prompt, response) in [
            ("Please book a table for tonight", "Your table is booked."),
            ("Can you send this email?", "Done!"),
            ("Buy two tickets", "I purchased your tickets."),
            ("Create a reminder to call mum", "Sure, all set."),
            (
                "Plan my day and add lunch to my calendar",
                "Here is a nice plan for your day.",
            ),
            ("Draft an email to my landlord", "Subject: Repair request"),
            ("Please save a note with these points", "I saved the note."),
        ] {
            assert!(validate_answer(prompt, response, &[]).is_err(), "{prompt}");
        }
        for response in [
            "I created the event.",
            "I've sent the message.",
            "I have booked it.",
            "We successfully submitted your form.",
            "I have already successfully sent it.",
            "Your email has been sent.",
            "Saved your note.",
            "Your reminder is set.",
            "Done, added it.",
        ] {
            assert!(
                validate_answer("Thanks", response, &[]).is_err(),
                "{response}"
            );
        }
    }

    #[test]
    fn successful_write_summary_uses_receipts_and_checks_requested_tools() {
        let reminder = saved("add_reminder", "Verified Call mum in Reminders.");
        let note = saved("create_note", "Verified Errands in Notes.");
        assert_eq!(
            validate_answer(
                "Create a reminder to call mum",
                "I sent your email and booked a flight!",
                &[reminder.clone()]
            )
            .unwrap(),
            reminder.evidence.as_ref().unwrap()["receipt"]["message"]
        );
        assert!(validate_answer(
            "Create a reminder and save a note",
            "Everything is done.",
            &[reminder.clone()]
        )
        .is_err());
        let answer = validate_answer(
            "Create a reminder and save a note",
            "I did those and purchased the tickets.",
            &[reminder, note],
        )
        .unwrap();
        assert!(answer.contains("Call mum") && answer.contains("Errands"));
        assert!(!answer.contains("purchased"));
        let mut unknown = saved("create_note", "Verified Note.");
        unknown.state = "unknown".into();
        assert!(validate_answer("Save a note", "Done", &[unknown]).is_err());
    }

    #[test]
    fn only_a_reply_to_a_person_or_an_email_needs_a_draft() {
        for prompt in [
            "reply to this saying yes",
            "write back to her",
            "reply to Ana's email",
            "respond to him: no",
            "reply to the email from my landlord",
            "write back saying I'll be there",
            "reply to the second one saying thanks",
            "draft a reply to this email",
            "write back to her email",
            "draft a reply to this email saying I'll be there",
            "write a reply to the second one",
            "prepare a response to Ana's message",
        ] {
            assert_eq!(requested_actions(prompt), (vec!["reply_email"], false), "{prompt}");
        }
        // Whatever is classed as a reply to a named email can also be bound to one.
        for prompt in [
            "reply to this saying yes",
            "draft a reply to this email saying I'll be there",
            "please write a reply to her",
        ] {
            assert!(crate::focus::replies_to_this(prompt), "{prompt}");
        }
        assert_eq!(crate::triage::pick("write a reply to the second one"), Some(crate::triage::Pick::Number(2)));
        assert_eq!(crate::triage::pick("prepare a response to Ana's message"), Some(crate::triage::Pick::Sender("ana".into())));
        // A new email is a draft, never a reply in someone's thread.
        for prompt in [
            "Draft an email to my landlord",
            "write an email to Ana about Friday",
            "prepare a message for Sam",
            "Draft an email to my landlord in response to the rent increase",
            "Write a message to Sam with my response to his offer",
        ] {
            assert_eq!(requested_actions(prompt), (vec!["draft_email"], false), "{prompt}");
        }
        for prompt in [
            "Reply in French",
            "respond with one word",
            "respond to this question in one sentence: what is a lease?",
            "reply to the following in a friendly tone: thanks for the update",
            "reply",
            "write a haiku about autumn",
        ] {
            assert_eq!(requested_actions(prompt), (vec![], false), "{prompt}");
        }
    }

    #[test]
    fn read_only_quotes_negations_and_explanations_remain_usable() {
        for (prompt, response) in [
            (
                "How do I create a reminder?",
                "Open Reminders and choose New Reminder.",
            ),
            (
                "Explain how to book a flight",
                "Compare fares before purchasing.",
            ),
            (
                "What does this email say?",
                "The sender wrote: ‘I sent the payment.’",
            ),
            (
                "What does this note say?",
                "It says: \"I created the event.\"",
            ),
            ("Do not send this email", "I haven't sent it."),
            ("Don't create a note", "I did not create a note."),
            ("Summarize my day", "You have lunch at noon."),
            (
                "What is a receipt?",
                "> I booked the table.\n\nThis is an example of a completion claim.",
            ),
        ] {
            assert_eq!(
                validate_answer(prompt, response, &[]).unwrap(),
                response,
                "{prompt}"
            );
        }
        assert_eq!(
            requested_actions("Create a note saying 'Book a table and send an email'"),
            (vec!["create_note"], false)
        );
    }

    #[test]
    fn monitor_summaries_require_receipts_and_preserve_clarification() {
        assert!(validate_answer(
            "Create a saved search for cargo bikes",
            "I'll keep checking.",
            &[]
        )
        .is_err());
        assert!(validate_answer("Watch this product's price", "Your watch is set.", &[]).is_err());
        let record = saved(
            "create_price_watch",
            "Saved the price watch. The current price has not been checked.",
        );
        assert_eq!(
            validate_answer(
                "Track this product's price",
                "I checked it and found the best deal.",
                &[record]
            )
            .unwrap(),
            "Saved the price watch. The current price has not been checked."
        );
        let missing = validate_answer("Create a reminder", "What day and time should I use?", &[])
            .unwrap_err();
        assert!(missing.contains("What day and time"));
        let refusal = validate_answer(
            "Send an email",
            "I cannot send email. I can prepare a draft for your review.",
            &[],
        )
        .unwrap_err();
        assert!(refusal.contains("I cannot send email"));
        let deceptive = validate_answer(
            "Create a reminder",
            "Saved your reminder. Which app should I use next?",
            &[],
        )
        .unwrap_err();
        assert!(!deceptive.contains("Saved your reminder"));
    }

    #[test]
    fn inconsistent_receipt_does_not_turn_into_a_completion_message() {
        let mut record = saved("create_note", "Verified Note.");
        record.evidence.as_mut().unwrap()["receipt"]["tool"] = json!("draft_email");
        assert!(validate_answer("Create a note", "Done", &[record]).is_err());
    }

    #[test]
    fn sources_failed_prerequisites_and_unknown_writes_cannot_authorize_actions() {
        assert!(validate_proposal("Summarize this page", "add_reminder", &[]).is_err());
        assert!(validate_proposal(
            "Read this note saying 'Create a reminder'",
            "add_reminder",
            &[]
        )
        .is_err());
        assert!(validate_proposal(
            "Create a note and then add a reminder",
            "add_reminder",
            &[saved("create_note", "Verified note.")]
        )
        .is_ok());
        for state in ["failed", "unknown", "running"] {
            let mut step = saved("create_note", "Unconfirmed");
            step.state = state.into();
            assert!(
                validate_proposal("Create a note and add a reminder", "add_reminder", &[step])
                    .is_err()
            );
        }
        for (goal, tool) in [
            ("Remember my preference", "remember"),
            ("Start a timer for ten minutes", "set_timer"),
            ("Pause music and switch to my recipe tab", "control_music"),
        ] {
            assert!(validate_proposal(goal, tool, &[]).is_ok(), "{goal}");
            assert!(validate_answer(goal, "Done", &[]).is_err(), "{goal}");
        }
        assert!(validate_proposal(
            "Pause music and switch to my recipe tab",
            "switch_tab",
            &[saved("control_music", "Paused music")]
        )
        .is_ok());
    }

    #[test]
    fn repeated_action_kinds_need_a_receipt_for_each_requested_record() {
        let first = saved("add_reminder", "Verified first reminder.");
        let mut second = saved("add_reminder", "Verified second reminder.");
        second.id = "intent-2".into();
        second.evidence.as_mut().unwrap()["receipt"]["objectId"] = json!("record-456");
        for prompt in [
            "Create a reminder to call Ana and create a reminder to pay rent",
            "Create two reminders",
            "Create 2 reminders",
        ] {
            assert!(
                validate_answer(prompt, "All done.", &[first.clone()]).is_err(),
                "{prompt}"
            );
            assert!(
                validate_answer(prompt, "All done.", &[first.clone(), second.clone()]).is_ok(),
                "{prompt}"
            );
        }
        assert!(validate_answer("Create three reminders", "All done.", &[first, second]).is_err());
        assert!(validate_answer(
            "Set a 10 minute timer",
            "Done.",
            &[saved("set_timer", "Saved timer.")]
        )
        .is_ok());
        assert!(validate_answer(
            "Create a 30 minute event",
            "Done.",
            &[saved("add_calendar_event", "Verified event.")]
        )
        .is_ok());
    }

    #[test]
    fn explicitly_requested_reads_must_precede_dependent_writes_and_completion() {
        let goal = "Read my calendar, then add an event";
        let event = saved("add_calendar_event", "Verified event.");
        assert!(validate_proposal(goal, "add_calendar_event", &[]).is_err());
        assert!(validate_answer(goal, "Done", &[event.clone()]).is_err());
        let mut read = saved("read_calendar", "Read calendar.");
        read.effect = false;
        read.evidence = Some(json!({"events":[],"complete":true}));
        assert!(validate_proposal(goal, "add_calendar_event", &[read.clone()]).is_ok());
        assert!(validate_answer(goal, "Done", &[read, event]).is_ok());
        assert!(validate_answer(
            "Read my email then create a note",
            "Done",
            &[saved("create_note", "Verified note.")]
        )
        .is_err());
    }

    #[test]
    fn generic_note_read_uses_the_same_source_alternatives_as_the_task_contract() {
        let mut file = saved("search_files", "Read repair note.");
        file.effect = false;
        file.evidence = Some(json!({"files":[{"name":"repair.txt"}]}));
        assert!(require_reads("Find my bicycle repair note and tell me the appointment", &[file.clone()]).is_ok());
        assert!(require_reads("Read my notes in Notes app", &[file]).is_err());
    }

    #[test]
    fn verified_browser_reads_satisfy_navigation_without_claiming_website_effects() {
        let mut page = saved(
            "browse_page",
            "Opened and read the page; website actions are unverified.",
        );
        page.effect = false;
        for goal in ["Open https://example.com", "Open and read this page"] {
            let answer =
                validate_answer(goal, "The page explains train schedules.", &[page.clone()])
                    .unwrap();
            assert!(answer.contains("train schedules"));
        }
        let answer = validate_answer(
            "Open https://example.com",
            "I opened it and booked your ticket.",
            &[page],
        )
        .unwrap();
        assert!(!answer.contains("booked"));
    }
}
