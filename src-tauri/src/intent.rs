//! What a request asks Buntini to do, by meaning, when no word rule recognised it.
//!
//! The request is compared with example requests for each action in `intent_examples.json`,
//! using the meaning model that is bundled for memory search. It only proposes: whatever
//! it picks still goes through the request's checks and the approval sheet. Questions,
//! things Buntini can't do and anything without a clear winner are left to the answer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// The closest examples of a kind that count towards its score.
const NEAREST: usize = 3;
/// How close a request must be to an action's examples.
const LEAST: f32 = 0.70;
/// How far ahead of the closest non-action it must be.
const MARGIN: f32 = 0.01;
/// How long to wait while the meaning model is busy indexing.
const WAIT: Duration = Duration::from_secs(2);

static ENABLED: AtomicBool = AtomicBool::new(false);
static EXAMPLES: OnceLock<Option<Vec<(String, Vec<f32>)>>> = OnceLock::new();

/// Turns matching on. The app does this at startup; unit tests leave it off so word rules
/// alone decide, whatever another test loaded.
pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
}

/// Loads the examples' vectors ahead of the first request.
pub fn warm() {
    if ENABLED.load(Ordering::SeqCst) {
        examples();
    }
}

fn unit(vector: &[i8]) -> Vec<f32> {
    vector.iter().map(|value| f32::from(*value) / 127.0).collect()
}

fn examples() -> Option<&'static [(String, Vec<f32>)]> {
    EXAMPLES
        .get_or_init(|| {
            let kinds: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(include_str!("intent_examples.json")).ok()?;
            let pairs: Vec<(String, String)> = kinds
                .iter()
                .flat_map(|(kind, texts)| {
                    texts.as_array().into_iter().flatten().filter_map(|text| Some((kind.clone(), text.as_str()?.to_string())))
                })
                .collect();
            let texts: Vec<String> = pairs.iter().map(|(_, text)| text.clone()).collect();
            let vectors = crate::embedding::embed_within(&texts, false, WAIT).ok()?;
            Some(pairs.into_iter().zip(vectors).filter_map(|((kind, _), vector)| Some((kind, unit(&vector?)))).collect())
        })
        .as_deref()
}

/// Words a question starts with. "Can you", "could you", "would you" and "will you" start
/// polite requests instead.
const ASKING: &[&str] = &[
    "what", "whats", "what's", "when", "whens", "when's", "where", "wheres", "where's", "who", "whos",
    "who's", "whom", "whose", "why", "how", "hows", "how's", "is", "isn't", "are", "aren't", "am", "was",
    "were", "do", "does", "did", "didn't", "don't", "have", "has", "had", "should", "shall", "would",
    "will", "any", "anything", "anyone", "which", "can", "could", "may", "might", "must",
];

/// Whether the request is phrased as a question rather than asking for something done.
pub(crate) fn asks(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    if lower.ends_with('?') {
        return true;
    }
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')).filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        [] => true,
        ["can" | "could" | "would" | "will", "you" | "u", ..] => false,
        // "Give me the start and end of the dentist appointment" asks to be told something.
        ["give" | "tell" | "show", "me", ..] => true,
        [first, ..] => ASKING.contains(&first.replace('’', "'").as_str()),
    }
}

/// The words that say what to do: "make this sound polite: send me the report" is about
/// the text before the colon. "1:1" isn't a colon that starts anything.
fn instruction(text: &str) -> &str {
    match text.split_once(": ") {
        Some((head, _)) if head.split_whitespace().count() >= 2 => head,
        _ => text,
    }
}

/// Words of each action's own: an action counts only when the request also says one, the
/// way a tool's name tells a large model what it's for. "Give me ideas for a team lunch"
/// is near a calendar event in meaning, but asks for no event.
const CUES: &[(&str, &[&str])] = &[
    ("add_reminder", &["remind", "reminder", "reminders", "todo", "to-do", "todos", "list", "forget"]),
    ("add_calendar_event", &["calendar", "diary", "event", "meeting", "appointment", "schedule", "book", "pencil", "block"]),
    ("create_note", &["note", "notes", "jot", "write down"]),
    ("complete_reminder", &["done", "tick", "cross", "complete", "completed", "finished", "check off"]),
    ("reschedule_reminder", &["move", "push", "reschedule", "change", "shift", "postpone", "snooze", "later", "earlier", "instead", "due"]),
    ("update_calendar_event", &["move", "push", "reschedule", "change", "shift", "postpone", "rename", "later", "earlier", "instead", "start"]),
    ("draft_email", &["email", "mail", "e-mail", "reply"]),
    ("send_message", &["text", "message", "imessage", "sms", "tell", "let"]),
    ("start_call", &["call", "ring", "phone", "facetime"]),
    ("open_app", &["open", "launch", "start", "bring up", "fire up"]),
    ("set_timer", &["timer", "countdown", "pomodoro", "time"]),
    ("control_music", &["play", "pause", "skip", "next", "previous", "resume", "song", "track", "music", "playlist", "album", "stop"]),
    ("change_mac_setting", &["dark", "light", "mode", "wifi", "wi-fi", "dock", "wallpaper", "screen saver", "hidden files", "desktop icons", "menu bar"]),
];

fn cued(tool: &str, request: &str) -> bool {
    let words: Vec<String> = request.to_lowercase().split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\'')).filter(|w| !w.is_empty()).map(str::to_string).collect();
    let text = format!(" {} ", words.join(" "));
    CUES.iter().find(|(name, _)| *name == tool).is_some_and(|(_, cues)| cues.iter().any(|cue| text.contains(&format!(" {cue} "))))
}

/// What a request asks for by meaning, when it's plain enough to say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Intent {
    /// An action Buntini has, still to be approved.
    Action(&'static str),
    /// Something Buntini can't do ("turn the volume down", "pay the bill").
    Unsupported,
    /// Something only a look online can answer: weather, prices, scores, opening hours.
    LookUp,
}

/// Whether the request says which of the user's apps to read from: "from my email give the
/// Nest energy change and from my reminders the car task" asks to be told, not to add anything.
fn reads_from_an_app(request: &str) -> bool {
    const APPS: [&str; 8] = ["mail", "email", "emails", "inbox", "calendar", "reminders", "notes", "messages"];
    let words: Vec<String> = request.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    words.windows(2).any(|pair| pair[0] == "from" && APPS.contains(&pair[1].as_str()))
        || words.windows(3).any(|run| run[0] == "from" && ["my", "the"].contains(&run[1].as_str()) && APPS.contains(&run[2].as_str()))
}

/// What a request asks for by meaning. A question can only be a look online; a model's
/// own answer to "will it rain tomorrow" is made up.
pub fn read(request: &str) -> Option<Intent> {
    if ENABLED.load(Ordering::SeqCst) && reads_from_an_app(request) {
        return None;
    }
    let question = asks(request);
    // "Remind me what the code was" asks; it isn't a reminder. A question can still be a look.
    if !ENABLED.load(Ordering::SeqCst) || (!question && crate::completion::non_action(&request.trim().to_lowercase())) {
        return None;
    }
    let examples = examples()?;
    let text = instruction(request.trim()).to_string();
    let vector = unit(&crate::embedding::embed_within(&[text], false, WAIT).ok()?.pop()??);
    let mut scores: Vec<(&str, Vec<f32>)> = Vec::new();
    for (kind, example) in examples {
        if question && !["look_up_online", "none", "unsupported"].contains(&kind.as_str()) {
            continue;
        }
        let similarity: f32 = vector.iter().zip(example).map(|(a, b)| a * b).sum();
        match scores.iter_mut().find(|(name, _)| name == kind) {
            Some((_, list)) => list.push(similarity),
            None => scores.push((kind.as_str(), vec![similarity])),
        }
    }
    let mut ranked: Vec<(&str, f32)> = scores
        .into_iter()
        .map(|(kind, mut list)| {
            list.sort_by(|a, b| b.total_cmp(a));
            let top = &list[..list.len().min(NEAREST)];
            (kind, top.iter().sum::<f32>() / top.len() as f32)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, score) = *ranked.first()?;
    match best {
        "none" => None,
        "unsupported" => (!question).then_some(Intent::Unsupported),
        _ => {
            let other = ranked.iter().filter(|(kind, _)| ["none", "unsupported"].contains(kind)).map(|(_, score)| *score).fold(f32::MIN, f32::max);
            if score < LEAST || score - other < MARGIN {
                return None;
            }
            if best == "look_up_online" {
                return Some(Intent::LookUp);
            }
            crate::tools::name(best).filter(|tool| cued(tool, request)).map(Intent::Action)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_told_from_requests() {
        for question in ["what's on tomorrow", "did Ana write back", "free at 11?", "can I freeze milk", "is wifi on", "any emails", "give me the start and end of the dentist appointment", "tell me the first appointment tomorrow"] {
            assert!(asks(question), "{question}");
        }
        for request in ["timer 25 min", "can you call mum", "brightness to 50%", "quit Spotify", "could you add eggs to my reminders"] {
            assert!(!asks(request), "{request}");
        }
    }

    #[test]
    fn the_instruction_is_before_its_text() {
        assert_eq!(instruction("make this sound polite: send me the report"), "make this sound polite");
        assert_eq!(instruction("note: wifi is blue"), "note: wifi is blue");
        assert_eq!(instruction("move my 1:1 to 2pm"), "move my 1:1 to 2pm");
    }

    #[test]
    fn every_example_kind_is_a_tool_or_says_none() {
        let kinds: serde_json::Map<String, serde_json::Value> = serde_json::from_str(include_str!("intent_examples.json")).unwrap();
        for kind in kinds.keys() {
            assert!(["none", "unsupported"].contains(&kind.as_str()) || crate::tools::name(kind).is_some(), "{kind}");
            assert!(!crate::actions::TOOLS.contains(&kind.as_str()) || crate::tools::definition(kind).is_some(), "{kind}");
        }
    }

    #[test]
    fn an_action_needs_its_own_words() {
        assert!(cued("set_timer", "timer 25 min"));
        assert!(cued("add_calendar_event", "put lunch with Ana on the calendar"));
        assert!(!cued("add_calendar_event", "give me ideas for a team lunch"));
        assert!(!cued("draft_email", "write a thank you message for my teacher"));
        for (tool, _) in CUES {
            assert!(crate::tools::name(tool).is_some(), "{tool}");
        }
    }

    #[test]
    fn naming_an_app_to_read_from_asks_rather_than_acts() {
        assert!(reads_from_an_app("From my email give the Nest energy change and from my reminders the car task."));
        assert!(reads_from_an_app("From Mail give the amount of the ride receipt"));
        assert!(!reads_from_an_app("add milk to my reminders"));
        assert!(!reads_from_an_app("put lunch with Ana in my calendar"));
    }

    #[test]
    fn off_unless_enabled() {
        assert_eq!(read("set a timer for 5 minutes"), None);
    }

    /// With the bundled meaning model: asking to be shown one's own things in some order or
    /// form is no action, while asking for something added or changed still is one.
    #[test]
    #[ignore = "loads the bundled meaning model; run with --ignored"]
    fn live_showing_is_not_changing() {
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        enable();
        let mut wrong = Vec::new();
        for request in [
            "Sort my reminders by due date, soonest first, with undated ones last.",
            "Alphabetise my reminder titles, one per line.",
            "Turn my reminders into one short sentence I could read aloud.",
            "order my tasks by when they're due",
            "list my to-dos from oldest to newest",
            "give me a quick rundown of my day in two sentences",
            "put this week's meetings in a numbered list",
            "show yesterday's unread emails",
            "read my reminders back to me",
            "summarise my inbox in three bullets",
            "arrange my events for saturday by start time",
            "count the meetings I have this week",
        ] {
            if let Some(intent) = read(request) {
                wrong.push(format!("{request} -> {intent:?}"));
            }
        }
        for (request, tool) in [
            ("remind me to buy milk tomorrow", "add_reminder"),
            ("add eggs to my reminders", "add_reminder"),
            ("put lunch with Ana in my calendar on friday at 1", "add_calendar_event"),
            ("book a dentist appointment on my calendar for next tuesday at 3", "add_calendar_event"),
            ("mark the laundry reminder as done", "complete_reminder"),
            ("push the call mum reminder to thursday", "reschedule_reminder"),
            ("move my dentist appointment to 4pm", "update_calendar_event"),
            ("create a note called packing list", "create_note"),
            ("remind me to call the plumber at 5", "add_reminder"),
            ("add bread and butter to my reminders", "add_reminder"),
            ("move my 3pm meeting to thursday", "update_calendar_event"),
            ("set a timer for 10 minutes", "set_timer"),
        ] {
            if read(request) != Some(Intent::Action(tool)) {
                wrong.push(format!("{request} -> {:?}, wanted {tool}", read(request)));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// The kinds nearest a request, best first, with their scores. For tuning the examples.
    #[test]
    #[ignore = "loads the bundled meaning model; prints scores for BUNTINI_INTENT_PROBE"]
    fn live_probe_scores() {
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        let examples = examples().unwrap();
        for request in std::env::var("BUNTINI_INTENT_PROBE").unwrap_or_default().split('|').filter(|r| !r.is_empty()) {
            let vector = unit(&crate::embedding::embed_within(&[instruction(request).to_string()], false, WAIT).unwrap().pop().unwrap().unwrap());
            let mut scores: Vec<(&str, Vec<f32>)> = Vec::new();
            for (kind, example) in examples {
                let similarity: f32 = vector.iter().zip(example).map(|(a, b)| a * b).sum();
                match scores.iter_mut().find(|(name, _)| name == kind) {
                    Some((_, list)) => list.push(similarity),
                    None => scores.push((kind.as_str(), vec![similarity])),
                }
            }
            let mut ranked: Vec<(&str, f32)> = scores.into_iter().map(|(kind, mut list)| {
                list.sort_by(|a, b| b.total_cmp(a));
                let top = &list[..list.len().min(NEAREST)];
                (kind, top.iter().sum::<f32>() / top.len() as f32)
            }).collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            println!("PROBE {request}: {:?}", &ranked[..3]);
        }
    }
}
