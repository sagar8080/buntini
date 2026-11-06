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
/// "Hide the Dock", "turn off wifi", "switch to dark mode", "change my wallpaper".
fn changes_mac_setting(tokens: &[&str]) -> bool {
    let has = |word: &str| tokens.contains(&word);
    let pair = |a: &str, b: &str| tokens.windows(2).any(|w| w[0] == a && w[1] == b);
    has("wallpaper")
        || has("dock")
        || has("wifi")
        || has("screensaver")
        || pair("wi", "fi")
        || pair("menu", "bar")
        || pair("dark", "mode")
        || pair("light", "mode")
        || pair("hidden", "files")
        || pair("desktop", "icons")
        || pair("desktop", "background")
        || pair("screen", "saver")
}

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

/// Writes that always wait for a sheet, so words in them never run a direct command.
pub(crate) const NOT_A_DIRECT_COMMAND: [&str; 14] = [
    "add_reminder", "add_calendar_event", "create_note", "draft_email", "reply_email", "send_message", "start_call",
    "update_calendar_event", "reschedule_reminder", "complete_reminder", "append_to_note", "set_timer", "create_saved_search", "create_price_watch",
];

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
                "add" | "put" | "throw" if has(&["cart", "basket"]) || tokens.windows(2).any(|pair| pair[0] == "my" && pair[1] == "bag") => Some(crate::cart::TOOL),
                // Before the Mac-setting words below: "set a reminder to turn on dark mode".
                "set" | "add" | "create" | "make" if tokens.iter().take(3).any(|token| *token == "reminder") => Some("add_reminder"),
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
                "hide" | "show" | "turn" | "switch" | "change" | "set" | "move" | "enable"
                | "disable" | "make" | "put" | "use"
                    if changes_mac_setting(&tokens) =>
                {
                    Some(crate::mac_settings::TOOL)
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
                // A text to one person through Messages; never email, WhatsApp or the like.
                "text" | "imessage" | "message"
                    if tokens.len() > 1 && !has(&["email", "mail", "whatsapp", "slack", "telegram", "signal"]) =>
                {
                    Some("send_message")
                }
                "send"
                    if has(&["text", "texts", "imessage", "sms"])
                        && !has(&["email", "mail", "whatsapp", "slack", "telegram", "signal"]) =>
                {
                    Some("send_message")
                }
                // A call to one person through FaceTime; never another service, or "call me".
                "call" | "facetime" | "ring" | "phone"
                    if tokens.len() > 1
                        && !["me", "us", "it", "this", "that", "the", "a", "an", "back", "up", "off", "number", "numbers"].contains(&tokens[1])
                        && !has(&["whatsapp", "zoom", "teams", "slack", "meet", "skype", "api", "function", "method"]) =>
                {
                    Some("start_call")
                }
                "video" if tokens.get(1) == Some(&"call") && tokens.len() > 2 && !has(&["whatsapp", "zoom", "teams", "slack", "meet", "skype"]) => {
                    Some("start_call")
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
                // Answering an invite through Buntini isn't possible; "Accept Saturday" in a
                // draft's instructions isn't this.
                "accept" | "decline" if has(&["invite", "invitation", "invites", "invitations", "meeting", "event"]) => {
                    unsupported = true;
                    None
                }
                "send" | "email" | "text" | "post" | "publish" | "book" | "reserve" | "buy"
                | "purchase" | "order" | "pay" | "delete" | "cancel" | "move" | "update"
                | "change" | "forward" | "archive" | "unsubscribe" | "rsvp" | "trash" => {
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
        "forwarded",
        "archived",
        "unsubscribed",
        "accepted",
        "declined",
        "replied",
        "trashed",
        "blocked",
        "marked",
    ];
    if tokens.first().is_some_and(|first| *first == "done") && tokens.len() <= 4 {
        return true;
    }
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

/// The apps whose contents an answer may only state from this turn's own reads. Earlier
/// turns may be out of date, and the on-device model has stitched them into a schedule
/// that was never read ("Buntini test planning: 4:00 PM - 5:00 PM").
const PERSONAL: [(&str, &[&str], &[&str]); 3] = [
    (
        "calendar",
        &["calendar", "schedule", "meeting", "meetings", "event", "events", "appointment", "appointments"],
        &["read_calendar", "add_calendar_event", "update_calendar_event", "search_memory"],
    ),
    (
        "reminders",
        &["reminder", "reminders", "to-do", "todo", "to-dos"],
        &["read_reminders", "add_reminder", "complete_reminder", "reschedule_reminder", "search_memory"],
    ),
    (
        "email",
        &["email", "emails", "inbox", "mail"],
        &["read_mail", "reply_email", "draft_email", "search_memory"],
    ),
];

/// Clock times written in `text`: "9:00", "4 pm", "16:45", "10am".
fn clock_times(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let mut times = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':');
        let (number, suffix) = word.split_at(word.find(|c: char| !c.is_ascii_digit() && c != ':').unwrap_or(word.len()));
        let meridiem = ["am", "pm", "a.m", "p.m"].contains(&suffix.trim_end_matches('.'))
            || (suffix.is_empty() && words.get(index + 1).is_some_and(|next| ["am", "pm", "a.m.", "p.m."].contains(&next.trim_end_matches(|c: char| c == ',' || c == ')'))));
        let digits: Vec<&str> = number.split(':').collect();
        let clock = match digits[..] {
            [hour, minute] => hour.len() <= 2 && minute.len() == 2 && hour.parse::<u32>().is_ok_and(|h| h < 24) && minute.parse::<u32>().is_ok_and(|m| m < 60),
            [hour] => meridiem && hour.parse::<u32>().is_ok_and(|h| (1..=12).contains(&h)),
            _ => false,
        };
        if clock && !number.is_empty() {
            times.push(number.to_string());
        }
    }
    times
}

/// Sentences and lines of an answer, each with its own punctuation.
/// Every number in a text, ignoring commas and a whole number's ".00", so "$1,299.00" and
/// "1299" are the same number.
pub(crate) fn numbers(text: &str) -> std::collections::HashSet<String> {
    number_spans(text).into_iter().map(|(number, ..)| number).collect()
}

fn number_spans(text: &str) -> Vec<(String, Option<char>, Option<char>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && (chars[at].is_ascii_digit() || (matches!(chars[at], ',' | '.') && chars.get(at + 1).is_some_and(char::is_ascii_digit))) {
            at += 1;
        }
        let raw: String = chars[start..at].iter().filter(|c| **c != ',').collect();
        let number = raw.strip_suffix(".00").map(str::to_string).unwrap_or(raw);
        found.push((number, start.checked_sub(1).map(|before| chars[before]), chars.get(at).copied()));
    }
    found
}

/// The numbers an answer states as facts: amounts and percentages of any size, and others
/// long enough to be a code, a phone number or a count. Part of a name ("WH-1000XM5"), a
/// citation marker or a year isn't one.
pub(crate) fn stated_figures(text: &str) -> Vec<String> {
    number_spans(text)
        .into_iter()
        .filter(|(number, before, after)| {
            let in_a_name = before.is_some_and(char::is_alphabetic) || after.is_some_and(char::is_alphabetic) || *before == Some('-') && after.is_some_and(char::is_alphabetic);
            let marker = *before == Some('[') && *after == Some(']');
            let money = before.is_some_and(|c| "$€£¥".contains(c)) || *after == Some('%');
            let digits = number.chars().filter(char::is_ascii_digit).count();
            let year = digits == 4 && (number.starts_with("19") || number.starts_with("20"));
            !in_a_name && !marker && (money || (digits >= 3 && !year))
        })
        .map(|(number, ..)| number)
        .collect()
}

/// Sentences from a model's answer about the user's notes, contacts, files or memory that
/// state a number or address nothing it read holds. Those are where a made-up amount or
/// phone number does harm, and no record-written answer covers them.
fn without_unread_details(prompt: &str, answer: &str, steps: &[Step]) -> String {
    let reads: Vec<&Step> = steps
        .iter()
        .filter(|step| step.state == "succeeded" && ["find_notes", "find_contact", "search_files", "search_memory"].contains(&step.tool.as_str()))
        .collect();
    if reads.is_empty() {
        return answer.to_string();
    }
    let held = reads.iter().filter_map(|step| step.evidence.as_ref()).map(serde_json::Value::to_string).chain([prompt.to_string()]).collect::<Vec<_>>().join("\n");
    let held_numbers = numbers(&held);
    let lower = held.to_lowercase();
    let made_up = |sentence: &str| {
        stated_figures(sentence).iter().any(|figure| !held_numbers.contains(figure))
            || sentence.split_whitespace().any(|word| {
                let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                word.contains('@') && !lower.contains(&word.to_lowercase())
            })
    };
    if !sentences(answer).into_iter().any(|sentence| made_up(sentence)) {
        return answer.to_string();
    }
    let kept: String = sentences(answer).into_iter().filter(|sentence| !made_up(sentence)).collect::<String>().trim().to_string();
    if kept.chars().any(char::is_alphabetic) {
        kept
    } else {
        "I couldn’t find that in what I read.".into()
    }
}

pub(crate) fn sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    for line in text.split_inclusive('\n') {
        let mut start = 0;
        let bytes = line.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            let ends = matches!(byte, b'.' | b'!' | b'?') && bytes.get(index + 1).is_some_and(|next| *next == b' ');
            if ends {
                parts.push(&line[start..=index]);
                start = index + 1;
            }
        }
        parts.push(&line[start..]);
    }
    parts.into_iter().filter(|part| !part.trim().is_empty()).collect()
}

/// A question about the topic in general, or text the user supplied: an answer can say
/// "you have" or "there are" without it being about their own apps.
fn may_speak_generally(prompt: &str) -> bool {
    let asked = prompt.to_lowercase().replace('’', "'");
    let words: Vec<&str> = asked.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).collect();
    ["how", "why", "explain", "define", "should"].contains(words.first().unwrap_or(&""))
        || asked.starts_with("what is a ")
        || asked.starts_with("what is an ")
        || asked.starts_with("what does ")
        || asked.chars().count() > 200
        || asked.contains(':')
        || words.iter().any(|word| {
            ["this", "screen", "selected", "selection", "draft", "write", "compose", "rewrite", "translate", "example", "sample", "template", "story", "pretend", "imagine"].contains(word)
        })
}

/// A sentence that states what's on the user's calendar or reminders with no clock time:
/// "You have a dentist visit tomorrow." Nothing this turn read anything, so it can't know.
fn states_day_or_reminder(prompt: &str, sentence: &str) -> bool {
    const NOUNS: [&str; 10] = ["calendar", "schedule", "meeting", "meetings", "event", "events", "appointment", "appointments", "reminder", "reminders"];
    const DAYS: [&str; 11] = ["today", "tomorrow", "tonight", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "weekend"];
    let text = sentence.to_lowercase().replace('’', "'");
    let asked = prompt.to_lowercase().replace('’', "'");
    let in_text = |set: &[&str]| text.split(|c: char| !c.is_alphanumeric()).any(|word| set.contains(&word));
    let hedged = ["if ", "when ", "whether", "?", "can't", "cannot", "don't", "haven't", "won't", "how to", "once ", "want me", "would you", "i can "].iter().any(|hedge| text.contains(hedge));
    if hedged {
        return false;
    }
    // "You're free" says as much about a calendar as "you have a meeting".
    if ["you're free", "you are free", "you're clear", "you are clear", "is clear", "are clear", "wide open", "you're not busy", "you are not busy", "nothing on ", "no meetings", "no events", "nothing scheduled", "you have nothing", "you've got nothing"].iter().any(|claim| text.contains(claim)) {
        return true;
    }
    let says_you_have = ["you have ", "you've got ", "you got "].iter().any(|claim| text.contains(claim));
    let there_is = ["there's ", "there is ", "there are ", "coming up", "on your calendar", "on your schedule", "is due", "are due"].iter().any(|claim| text.contains(claim));
    // A day the user brought up themselves ("I have a dentist visit tomorrow") can be repeated.
    let day_is_new = DAYS.iter().any(|day| text.split(|c: char| !c.is_alphanumeric()).any(|word| word == *day) && !asked.contains(day));
    (in_text(&NOUNS) && (says_you_have || there_is)) || (says_you_have && day_is_new)
}

/// When an answer built on a Mail read names something the read doesn't hold, the read
/// itself, in place of the answer. The model only saw that read, so a sender or subject
/// outside it is made up, and so is a count above what it found. The read can't lie;
/// the sentence around a made-up name can't be trusted, so it isn't repaired.
fn mail_read_instead(prompt: &str, answer: &str, steps: &[Step]) -> Option<String> {
    const NOT_NAMES: &[&str] = &[
        "your", "the", "a", "an", "my", "any", "anything", "these", "those", "this", "that", "it", "them", "each", "all", "some", "one", "more", "what", "which", "how", "who", "whom", "him",
        "her", "you", "me", "us", "there", "here", "their", "his", "its", "our", "today", "yesterday", "now", "last", "past", "recent", "earlier", "someone", "people", "emails",
        "email", "mail", "inbox", "day", "days", "week", "weeks", "month", "months", "several", "many",
    ];
    const SMALL: [&str; 13] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
    if steps.iter().any(|step| step.effect) {
        return None;
    }
    let read = steps.iter().rev().find(|step| step.tool == "read_mail" && step.state == "succeeded")?.evidence.as_ref()?;
    let shown = read["text"].as_str().map(str::trim).filter(|text| !text.is_empty())?;
    let mut known = prompt.to_lowercase();
    for evidence in steps.iter().filter(|step| step.state == "succeeded").filter_map(|step| step.evidence.as_ref()) {
        known += " ";
        known += &evidence["text"].as_str().unwrap_or_default().to_lowercase();
        for record in evidence["records"].as_array().into_iter().flatten() {
            for field in ["sender", "subject", "excerpt"] {
                known += " ";
                known += &record[field].as_str().unwrap_or_default().to_lowercase();
            }
        }
    }
    let lower = answer.to_lowercase().replace('’', "'");
    let words: Vec<&str> = lower.split_whitespace().map(|word| word.trim_matches(|c: char| !c.is_alphanumeric())).filter(|word| !word.is_empty()).collect();
    let backed = |word: &str| {
        let stem: String = word.chars().take(4).collect();
        known.contains(&stem)
    };
    let names_outside = words.iter().enumerate().filter(|(_, word)| ["from", "about"].contains(word)).any(|(at, _)| {
        words[at + 1..].iter().take(3).find(|word| !NOT_NAMES.contains(word) && !SMALL.contains(word)).is_some_and(|name| {
            name.chars().count() >= 3 && !name.chars().all(|c| c.is_ascii_digit()) && !backed(name)
        })
    });
    let found = read["total"].as_u64().unwrap_or(0).max(read["count"].as_u64().unwrap_or(0));
    let too_many = words.windows(3).any(|window| {
        let mail = |word: &str| ["email", "emails", "message", "messages"].contains(&word);
        let counted = mail(window[1]) || (["new", "unread"].contains(&window[1]) && mail(window[2]));
        let number = window[0].parse::<u64>().ok().or_else(|| SMALL.iter().position(|small| *small == window[0]).map(|at| at as u64));
        counted && number.is_some_and(|number| number > found)
    });
    (names_outside || too_many).then(|| shown.to_string())
}

/// Whether an answer says what's in the user's inbox when nothing this turn read anything.
/// On a real Mac the on-device model answered "whats on my mail" with a made-up message
/// from the user's boss, with no clock time for the check below to catch. The whole answer
/// goes, since the sentences around such a claim are made up too. Text the user supplied,
/// and questions about email in general, can say these things.
fn invents_inbox(prompt: &str, answer: &str, steps: &[Step]) -> bool {
    if steps.iter().any(|step| step.state == "succeeded" && !step.effect) || may_speak_generally(prompt) {
        return false;
    }
    sentences(answer).into_iter().any(|sentence| {
        let text = sentence.to_lowercase().replace('’', "'");
        let about_mail = text
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| ["email", "emails", "mail", "inbox", "message", "messages"].contains(&word));
        let states = ["you have ", "you've got ", "you got ", "there's ", "there is ", "there are ", "your inbox has", "your inbox is", "your inbox contains", "waiting", "new message from", "new email from", "email from your", "message from your"]
            .iter()
            .any(|claim| text.contains(claim));
        let hedged = ["if ", "when ", "whether", "?", "can't", "cannot", "don't", "haven't", "won't", "how to", "once "]
            .iter()
            .any(|hedge| text.contains(hedge));
        about_mail && states && !hedged
    })
}

/// An answer that states what is in the user's calendar, reminders or email at a given
/// time, when nothing this turn read that app, keeps only what it can stand behind. The
/// sentences with those claims go; what is left (a greeting, a reply to what was asked)
/// stays. When nothing is left, Buntini offers to look if the user asked about that app,
/// and otherwise just answers the person. Times the user wrote themselves don't count,
/// nor times a file or note read this turn states ("the repair appointment is 14:20").
pub fn grounded_personal_answer(prompt: &str, answer: &str, steps: &[Step]) -> String {
    let answer = &without_unread_details(prompt, answer, steps);
    if invents_inbox(prompt, answer, steps) {
        return "I haven’t looked at your email, so I won’t guess. Ask me “what’s in my inbox?” and I’ll check.".into();
    }
    if let Some(read) = mail_read_instead(prompt, answer, steps) {
        return read;
    }
    let mut asked = clock_times(prompt);
    for evidence in steps.iter().filter(|s| s.state == "succeeded" && !s.effect).filter_map(|s| s.evidence.as_ref()) {
        let mut read = evidence.as_str().unwrap_or_default().to_string();
        read += evidence["text"].as_str().unwrap_or_default();
        for record in evidence["records"].as_array().into_iter().flatten() {
            read += " ";
            read += record["excerpt"].as_str().unwrap_or_default();
        }
        asked.extend(clock_times(&read));
    }
    // Without a read, a day or reminder needs no clock time to be made up.
    let read_nothing = !steps.iter().any(|step| step.state == "succeeded" && !step.effect) && !may_speak_generally(prompt);
    let claims = |text: &str| {
        clock_times(text).into_iter().any(|time| !asked.contains(&time))
            || (read_nothing && sentences(text).into_iter().any(|sentence| states_day_or_reminder(prompt, sentence)))
    };
    let mentions = |text: &str, nouns: &[&str]| {
        text.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '-').any(|word| nouns.contains(&word))
    };
    let unread: Vec<(&str, &[&str])> = PERSONAL
        .iter()
        .filter(|(_, _, reads)| !steps.iter().any(|step| step.state == "succeeded" && reads.contains(&step.tool.as_str())))
        .map(|(app, nouns, _)| (*app, *nouns))
        .collect();
    let unsupported = |text: &str| unread.iter().any(|(_, nouns)| mentions(text, nouns));
    let stated = read_nothing && sentences(answer).into_iter().any(|sentence| states_day_or_reminder(prompt, sentence));
    if !claims(answer) || (!unsupported(answer) && !stated) {
        return answer.to_string();
    }
    // A claim's sentence goes, and so does any sentence about an app that wasn't read
    // ("Here's what's on your calendar:" would otherwise introduce nothing).
    let kept: String = sentences(answer)
        .into_iter()
        .filter(|part| !claims(part) && !unsupported(part))
        .collect::<String>()
        .trim()
        .to_string();
    if kept.chars().any(char::is_alphabetic) {
        return kept;
    }
    let asks_if_free = ["free", "busy", "available", "clear"].iter().any(|word| prompt.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|said| said == *word));
    let about = unread.iter().find(|(_, nouns)| mentions(prompt, nouns)).map(|(app, _)| *app).or_else(|| (asks_if_free && unread.iter().any(|(app, _)| *app == "calendar")).then_some("calendar"));
    match about {
        Some(app) => {
            let example = match app {
                "calendar" => "what’s on my calendar today?",
                "reminders" => "what are my reminders?",
                _ => "what needs a reply?",
            };
            format!("I haven’t looked at your {app} for that, so I won’t guess. Ask me something like “{example}” and I’ll check.")
        }
        None => "I’m here. What can I do for you?".into(),
    }
}

/// "What is my workshop project's name?" when nothing this turn read, and nothing in the
/// conversation, saved memories or screen (`known`) mentions it: Buntini asks rather
/// than pass on a name the model made up. None when the model's answer may stand.
pub fn unknown_personal_fact(prompt: &str, known: &str, steps: &[Step]) -> Option<String> {
    if steps.iter().any(|step| step.state == "succeeded") {
        return None;
    }
    let plain = unquoted(prompt).to_lowercase().replace('’', "'");
    let start = ["what is my ", "what's my ", "what was my ", "do you know my ", "do you remember my ", "remind me what my "]
        .iter()
        .find_map(|question| plain.find(question).map(|at| at + question.len()))?;
    let subject = plain[start..].split(['?', '.', '!', ',', ';', '\n']).next()?.trim();
    let subject = subject.strip_suffix(" is").unwrap_or(subject).trim();
    let words: Vec<&str> = subject
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !["name", "the", "called", "number"].contains(w))
        .collect();
    // A read or the device answers these, and "what's my name?" is in the profile.
    let looked_up = [
        "calendar", "schedule", "meeting", "meetings", "event", "events", "reminder", "reminders", "email", "emails",
        "mail", "inbox", "day", "week", "battery", "cpu", "disk", "storage", "wifi", "mac", "computer", "volume",
        "song", "music", "playing", "location", "weather", "file", "files", "note", "notes", "screen", "tab",
    ];
    if words.is_empty() || words.iter().any(|w| looked_up.contains(w)) || subject.chars().count() > 80 {
        return None;
    }
    let known = known.to_lowercase();
    if words.iter().any(|w| known.contains(w)) {
        return None;
    }
    Some(format!("I don’t know your {subject} yet. What is it?"))
}

/// Streams an answer to the window a sentence at a time, each one checked the way the
/// final answer is: a sentence stating unread calendar, reminder or email facts is never
/// shown or spoken. After the first such sentence nothing more streams; the checked final
/// answer replaces what was shown when the task finishes.
pub struct StreamGate {
    prompt: String,
    steps: Vec<Step>,
    held: String,
    stopped: bool,
}

impl StreamGate {
    pub fn new(prompt: &str, steps: &[Step]) -> Self {
        Self { prompt: prompt.into(), steps: steps.to_vec(), held: String::new(), stopped: false }
    }

    /// The text ready to show after `token` arrives: whole sentences that pass.
    pub fn push(&mut self, token: &str) -> Option<String> {
        if self.stopped {
            return None;
        }
        self.held.push_str(token);
        let bytes = self.held.as_bytes();
        let end = (0..bytes.len()).rev().find(|&index| {
            bytes[index] == b'\n' || (matches!(bytes[index], b'.' | b'!' | b'?') && bytes.get(index + 1) == Some(&b' '))
        })?;
        let ready: String = self.held.drain(..=end).collect();
        self.release(ready)
    }

    /// Whatever is left once the model has finished.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.held);
        if self.stopped || rest.is_empty() { None } else { self.release(rest) }
    }

    fn release(&mut self, ready: String) -> Option<String> {
        let mut shown = String::new();
        for part in sentences(&ready) {
            if grounded_personal_answer(&self.prompt, part, &self.steps) != part {
                self.stopped = true;
                break;
            }
            shown.push_str(part);
        }
        (!shown.is_empty()).then_some(shown)
    }
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
            crate::mac_settings::TOOL,
            crate::glance::QUIT_TOOL,
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
            "That part is something I can’t do yet."
        } else {
            "I couldn’t confirm the rest, so I’m not calling it done."
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
        // A reply that only repeats its receipt says it once.
        if receipt_only_goal(prompt) || text.trim() == summary {
            return Ok(summary);
        }
        if text.trim().is_empty() {
            return Err(format!("{summary}\n\nI still owe you the rest. Continue to finish it."));
        }
        if claims_own_write(text) {
            return Err(format!("{summary}\n\nI still owe you the rest. Continue to finish it."));
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
    fn answers_stream_by_sentence_and_stop_before_an_unread_schedule() {
        let mut gate = StreamGate::new("hey", &[]);
        assert_eq!(gate.push("Hi Sa"), None);
        assert_eq!(gate.push("gar! How "), Some("Hi Sagar!".into()));
        assert_eq!(gate.push("are you? Your meeting is at 4:00 PM. "), Some(" How are you?".into()));
        assert_eq!(gate.push("More text. "), None, "nothing streams after a held-back claim");
        assert_eq!(gate.finish(), None);
        let mut plain = StreamGate::new("hey", &[]);
        assert_eq!(plain.push("Hello there"), None);
        assert_eq!(plain.finish(), Some("Hello there".into()));
    }

    #[test]
    fn schedules_are_stated_only_from_this_turns_reads() {
        let made_up = "Here's what's on your calendar tomorrow:\n1. Planning: 4:00 PM - 5:00 PM\n2. Dentist: 9:00 AM";
        // A greeting gets a greeting, not a lecture about the calendar.
        assert_eq!(grounded_personal_answer("Hey, Bantini.", made_up, &[]), "I’m here. What can I do for you?");
        let greeted = "Hi Sagar! Your planning meeting is at 4:00 PM tomorrow. How can I help?";
        assert_eq!(grounded_personal_answer("Hey, Bantini.", greeted, &[]), "Hi Sagar! How can I help?");
        // Asked about the calendar with no read: offer to look.
        let answer = grounded_personal_answer("anything on my calendar tomorrow?", made_up, &[]);
        assert!(answer.starts_with("I haven’t looked at your calendar"), "{answer}");
        let read = Step { id: "1".into(), tool: "read_calendar".into(), arguments: serde_json::json!({}), state: "succeeded".into(), effect: false,
            proposed_at: String::new(), approved_at: None, finished_at: None, evidence: None };
        assert_eq!(grounded_personal_answer("what's on tomorrow", made_up, &[read]), made_up);
        // Times the user gave, and answers with no times, are left alone.
        let echo = "Your meeting at 3pm is in two hours.";
        assert_eq!(grounded_personal_answer("my meeting is at 3pm, how long until then?", echo, &[]), echo);
        assert_eq!(grounded_personal_answer("hi", "Hi! Want me to check your calendar?", &[]), "Hi! Want me to check your calendar?");
        assert_eq!(grounded_personal_answer("hm", "You have a reminder to call mum at 18:00.", &[]), "I’m here. What can I do for you?");
        assert_eq!(clock_times("from 9:00 to 4 pm, or 10am"), ["9:00", "4", "10"]);
    }


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
                    unverified.contains("I still owe you the rest") && !unverified.contains("I sent the email")
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

    #[test]
    fn an_inbox_nobody_read_is_never_described() {
        // The reply a real Mac gave to "whats on my mail".
        let invented = "Hey, Sagar!\nI can’t read your inbox directly, but I can help you with a few things:\n\n1. Quick Summary:\n\n* You have a few emails waiting.\n* Some are from friends, and others are from work.\n* There’s a new message from your boss about a project deadline.";
        for prompt in ["whats on my mail", "More like these"] {
            let answer = grounded_personal_answer(prompt, invented, &[]);
            assert!(answer.starts_with("I haven’t looked at your email"), "{prompt}: {answer}");
        }
        let mut gate = StreamGate::new("More like these", &[]);
        let shown = gate.push("Sure. You have a few emails waiting. Some are from work. ").unwrap_or_default();
        assert_eq!(shown.trim(), "Sure.", "the claim never reaches the screen");

        let read = Step { id: "1".into(), tool: "read_mail".into(), arguments: serde_json::json!({}), state: "succeeded".into(), effect: false, proposed_at: String::new(), approved_at: None, finished_at: None, evidence: Some(serde_json::json!({"text":"1. From Ana"})) };
        assert_eq!(grounded_personal_answer("whats on my mail", "You have a new email from Ana.", &[read]), "You have a new email from Ana.");
        for (prompt, answer) in [
            ("how do I filter unread mail", "There are two ways to filter unread messages."),
            ("what is an inbox", "There is one inbox for your mail."),
            ("summarise this: you have a new email from Ana", "You have a new email from Ana."),
            ("hi", "Hi! I can’t read your inbox yet."),
            ("hi", "If you have unread email, I can help."),
            ("what's the capital of France", "Paris."),
        ] {
            assert_eq!(grounded_personal_answer(prompt, answer, &[]), answer, "{prompt}");
        }
    }

    fn mail_read() -> Step {
        Step {
            id: "1".into(), tool: "read_mail".into(), arguments: serde_json::json!({}), state: "succeeded".into(), effect: false, proposed_at: String::new(), approved_at: None, finished_at: None,
            evidence: Some(serde_json::json!({"text":"1. From Ana Silva · Wed 09:00 · unread\n   Subject: Lease renewal\n   Can you sign by Friday","count":2,"total":8,"records":[
                {"sender":"Ana Silva <ana@x.com>","subject":"Lease renewal","excerpt":"Can you sign by Friday"},{"sender":"Bob","subject":"Invoice"}]})),
        }
    }

    #[test]
    fn a_mail_answer_can_only_name_what_the_read_found() {
        let read = mail_read();
        let shown = "1. From Ana Silva · Wed 09:00 · unread\n   Subject: Lease renewal\n   Can you sign by Friday";
        for made_up in [
            "There’s a new message from your boss about a project deadline.",
            "Some are from friends, and others are from work.",
            "You have an email from Ana about your tax refund.",
            "You have 20 unread emails.",
            "You have twelve new messages.",
        ] {
            assert_eq!(grounded_personal_answer("anything urgent in my mail", made_up, std::slice::from_ref(&read)), shown, "{made_up}");
        }
        for fine in [
            "You have an email from Ana about the lease renewal.",
            "Bob sent an invoice, and Ana needs a signature by Friday.",
            "Nothing urgent. There are 8 emails, and I can tell you more about any of them.",
            "You have 2 emails from the last two weeks.",
        ] {
            assert_eq!(grounded_personal_answer("anything urgent in my mail", fine, std::slice::from_ref(&read)), fine);
        }
        assert_eq!(grounded_personal_answer("anything from Sam in my mail", "Nothing from Sam.", std::slice::from_ref(&read)), "Nothing from Sam.");
    }

    #[test]
    fn a_day_or_reminder_nobody_read_is_not_stated_even_without_a_time() {
        for (prompt, made_up) in [
            ("More like these", "You have a dentist visit tomorrow."),
            ("More like these", "There’s a meeting with Ana on your calendar."),
            ("More like these", "You have a reminder to call mum."),
            ("hm", "You’ve got two appointments on Friday."),
        ] {
            assert_eq!(grounded_personal_answer(prompt, made_up, &[]), "I’m here. What can I do for you?", "{made_up}");
        }
        assert_eq!(grounded_personal_answer("hm", "Hi Sagar! You have a dentist visit tomorrow. Anything else?", &[]), "Hi Sagar! Anything else?");
        let answer = grounded_personal_answer("anything on my calendar tomorrow?", "You have a dentist visit tomorrow.", &[]);
        assert_eq!(answer, "You have a dentist visit tomorrow.", "the day was asked about, and the read outcome handles this one");
        for (prompt, fine) in [
            ("I have a dentist visit tomorrow, what should I bring?", "Since you have a dentist visit tomorrow, bring your insurance card."),
            ("how do I share a calendar", "There are two ways to share a calendar."),
            ("hi", "Want me to check your calendar?"),
            ("hi", "If you have a meeting tomorrow, I can help."),
            ("what's the capital of France", "Paris."),
            ("any tips for tomorrow", "You have plenty of time."),
        ] {
            assert_eq!(grounded_personal_answer(prompt, fine, &[]), fine, "{prompt}");
        }
    }

    #[test]
    fn a_personal_fact_nothing_mentions_is_asked_for_not_guessed() {
        let prompt = "What is my workshop project's name? If I have not supplied it in this conversation, ask me instead of guessing.";
        assert_eq!(
            unknown_personal_fact(prompt, "", &[]).as_deref(),
            Some("I don’t know your workshop project's name yet. What is it?")
        );
        // Said earlier in the thread, saved as a memory, or on screen: the answer stands.
        assert!(unknown_personal_fact(prompt, "For this conversation only, my workshop project is named Larkspur.", &[]).is_none());
        // Questions a read, the device or the profile answers are left alone.
        for prompt in ["What's my next meeting?", "What is my name?", "What is my battery level?", "Tell me a joke"] {
            assert!(unknown_personal_fact(prompt, "", &[]).is_none(), "{prompt}");
        }
    }

    #[test]
    fn a_time_a_file_states_is_not_an_unread_calendar_claim() {
        let read = Step {
            id: "files".into(), tool: "search_files".into(), arguments: serde_json::json!({"query":"Cobalt"}), state: "succeeded".into(),
            effect: false, proposed_at: String::new(), approved_at: None, finished_at: None,
            evidence: Some(serde_json::json!("misc-014.txt — “Cobalt bicycle repair note: the rear brake cable needs replacing. The repair appointment is Tuesday at 14:20.”")),
        };
        let prompt = "Find the Cobalt bicycle repair note by its contents. What needs replacing and when is the appointment?";
        let answer = "The rear brake cable needs replacing. The repair appointment is Tuesday at 14:20.";
        assert_eq!(grounded_personal_answer(prompt, answer, &[read]), answer);
        // With nothing read, the same claim about an appointment still goes.
        assert_ne!(grounded_personal_answer(prompt, answer, &[]), answer);
    }

    #[test]
    fn calling_one_person_is_a_call_and_other_things_are_not() {
        for prompt in ["Call mom", "FaceTime Ana", "video call Raj", "ring my dad", "Please call Priya"] {
            assert_eq!(requested_actions(prompt).0, vec!["start_call"], "{prompt}");
        }
        for prompt in ["Call me when it's done", "Call the API again", "Call Raj on WhatsApp", "Remind me to call Ana", "phone number for Ana", "video call"] {
            assert!(!requested_actions(prompt).0.contains(&"start_call"), "{prompt}");
        }
    }

    #[test]
    fn writes_that_wait_for_a_sheet_are_never_direct_commands() {
        for prompt in ["Set a reminder at 5 to check the volume", "Set a reminder to turn on dark mode tonight", "Remind me to lock my screen at 6"] {
            assert!(requested_actions(prompt).0.iter().any(|tool| NOT_A_DIRECT_COMMAND.contains(tool)), "{prompt}");
        }
        for prompt in ["set the volume to 30", "turn on dark mode", "lock my screen", "play Hey There Delilah", "open github.com"] {
            assert!(!requested_actions(prompt).0.iter().any(|tool| NOT_A_DIRECT_COMMAND.contains(tool)), "{prompt}");
        }
    }

    #[test]
    fn saying_youre_free_takes_a_read_of_the_calendar() {
        let said = grounded_personal_answer("am I free at 3pm", "Yes, you're free at 3pm.", &[]);
        assert!(!said.contains("you're free") && said.contains("calendar"), "{said}");
        let said = grounded_personal_answer("is my afternoon clear", "Your afternoon is clear.", &[]);
        assert!(!said.contains("is clear"), "{said}");
        assert_eq!(grounded_personal_answer("what's a free API for weather", "Open-Meteo has a free API.", &[]), "Open-Meteo has a free API.");
    }

    #[test]
    fn what_buntini_cant_do_is_never_announced_as_done() {
        for prompt in ["forward Ana's email to Sam", "archive that email", "unsubscribe me from this", "accept the invite"] {
            assert!(requested_actions(prompt).1, "{prompt}");
        }
        assert!(!requested_actions("Accept Saturday at 2pm and mention a vegetarian dish").1);
        for said in ["I forwarded Ana's email to Sam.", "I archived it.", "Done, archived.", "Done."] {
            assert!(claims_own_write(said), "{said}");
        }
    }

    #[test]
    fn figures_are_compared_as_numbers() {
        assert!(numbers("Now $1,299.00 or 3 for 40").contains("1299"));
        assert_eq!(stated_figures("It's $249 at Best Buy [1], about 12% off the WH-1000XM5 from 2026, with 3 left."), ["249", "12"]);
        assert_eq!(stated_figures("Call 415-555-0100."), ["415", "555", "0100"]);
    }

    #[test]
    fn a_number_nothing_read_holds_doesnt_reach_the_user() {
        let mut note = saved("find_notes", "");
        note.effect = false;
        note.evidence = Some(json!({"records":[{"title":"Lease","text":"The deposit is $1,200, paid in May."}]}));
        let fine = "Your deposit is $1,200.";
        assert_eq!(grounded_personal_answer("what's my deposit", fine, std::slice::from_ref(&note)), fine);
        let said = grounded_personal_answer("what's my deposit", "Your deposit is $1,500.", std::slice::from_ref(&note));
        assert!(!said.contains("1,500"), "{said}");
        let said = grounded_personal_answer("what's Ana's phone number", "Ana's number is 415-555-0100.", std::slice::from_ref(&note));
        assert!(!said.contains("555"), "{said}");
    }

    #[test]
    fn negations_and_explanations_ask_for_nothing() {
        for goal in [
            "Explain how to create a note and add a reminder",
            "Do not create a note and add a reminder",
            "Please don't create a note, then add a reminder",
            "Can you explain how to create a note and then add a reminder?",
        ] {
            assert!(requested_actions(goal).0.is_empty(), "{goal}");
        }
        assert_eq!(requested_actions("Create a note. Add a reminder.").0, ["create_note", "add_reminder"]);
        assert_eq!(requested_actions("Don't create a note; add a reminder.").0, ["add_reminder"]);
    }

    #[test]
    fn done_is_not_believed_without_a_receipt_or_the_read_asked_for() {
        for goal in ["Remember my preference", "Start a timer for ten minutes", "Hide the Dock", "Switch to dark mode", "Pause music and switch to my recipe tab"] {
            assert!(validate_answer(goal, "Done", &[]).is_err(), "{goal}");
        }
        let goal = "Read my calendar, then add an event";
        let event = saved("add_calendar_event", "Verified event.");
        assert!(validate_answer(goal, "Done", std::slice::from_ref(&event)).is_err());
        let mut read = saved("read_calendar", "Read calendar.");
        read.effect = false;
        read.evidence = Some(json!({"events":[],"complete":true}));
        assert!(validate_answer(goal, "Done", &[read, event]).is_ok());
        assert!(validate_answer("Read my email then create a note", "Done", &[saved("create_note", "Verified note.")]).is_err());
    }

    #[test]
    fn texting_one_person_is_a_text_and_other_apps_are_not() {
        for prompt in ["Text mom that I'm running late", "message Raj about dinner", "Send a text to Ana saying hi", "imessage Sam that I'm outside"] {
            assert_eq!(requested_actions(prompt), (vec!["send_message"], false), "{prompt}");
        }
        for prompt in ["Send an email to Ana", "Message Raj on WhatsApp", "text"] {
            assert!(!requested_actions(prompt).0.contains(&"send_message"), "{prompt}");
        }
    }
}
