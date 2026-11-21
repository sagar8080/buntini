//! Buntini's actions in Shortcuts and Spotlight: Ask Buntini, Today on the Mac, Start Focus
//! Timer and Keep Mac Awake (native/surface/Sources/BuntiniIntents). The system runs them in
//! Buntini itself, and Swift hands each to the one handler here, which does what the chat would.
//! Only the person, through Shortcuts, Spotlight or Siri on this Mac, can run them.
use serde_json::{json, Value};

use crate::surface::{Action, Source};

type Handler = extern "C" fn(*const u8, usize, *mut u8, usize) -> isize;

#[cfg(target_os = "macos")]
extern "C" {
    fn buntini_intents_start(handler: Handler);
}

/// Gives Swift the handler. Call once at launch.
pub fn start() {
    #[cfg(target_os = "macos")]
    // SAFETY: the handler is a plain function that lives for the whole program.
    unsafe {
        buntini_intents_start(handle)
    }
}

extern "C" fn handle(bytes: *const u8, length: usize, out: *mut u8, capacity: usize) -> isize {
    if bytes.is_null() || out.is_null() || length > 64 * 1024 {
        return -1;
    }
    // SAFETY: Swift passes a buffer of `length` bytes it keeps alive through the call.
    let request = unsafe { std::slice::from_raw_parts(bytes, length) };
    let reply = match serde_json::from_slice::<Value>(request) {
        Ok(request) => answer(&request),
        Err(_) => json!({ "error": "Buntini didn't understand that." }),
    };
    let reply = reply.to_string().into_bytes();
    if reply.len() > capacity {
        return -1;
    }
    // SAFETY: Swift's buffer has room for `capacity` bytes.
    unsafe { std::ptr::copy_nonoverlapping(reply.as_ptr(), out, reply.len()) };
    reply.len() as isize
}

fn minutes(request: &Value, most: u64) -> Option<u64> {
    request.get("minutes").and_then(Value::as_u64).filter(|minutes| (1..=most).contains(minutes))
}

fn said(outcome: Result<crate::skills::Outcome, String>) -> Value {
    match outcome {
        Ok(crate::skills::Outcome::Done { answer, .. }) => json!({ "text": answer }),
        Ok(_) => json!({ "text": "" }),
        Err(problem) => json!({ "error": problem }),
    }
}

/// What an intent asks, answered.
pub fn answer(request: &Value) -> Value {
    match request.get("intent").and_then(Value::as_str).unwrap_or_default() {
        // The question goes to the chat, as one typed in the island does.
        "ask" => {
            let question = request.get("question").and_then(Value::as_str).unwrap_or_default().trim();
            if question.is_empty() || question.chars().count() > 8_000 {
                return json!({ "error": "Ask Buntini needs a question." });
            }
            crate::surface::act(Action { source: Source::Island, card: None, name: "ask".into(), value: Some(Value::String(question.to_string())) });
            json!({ "text": "" })
        }
        "today" => match crate::island::upcoming::today() {
            Some((events, reminders)) => json!({ "text": crate::island::assistants::today_lines(&events, &reminders) }),
            None => json!({ "error": "Buntini hasn't read the calendar yet: it reads it while heads-ups are on." }),
        },
        "focus" => said(crate::utilities::run_focus(crate::utilities::focus::Command::Start { minutes: minutes(request, 240) })),
        "awake" => said(crate::utilities::awake::run(crate::utilities::awake::Command::Start { seconds: minutes(request, 1440).map(|minutes| minutes * 60) })),
        _ => json!({ "error": "Buntini doesn't know that action." }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The intents in the Swift file, the metadata the system reads, and what this handler
    /// answers must all be the same four.
    #[test]
    fn the_intents_the_metadata_and_the_handler_agree() {
        let source = include_str!("../../native/surface/Sources/BuntiniIntents/Intents.swift");
        let declared: Vec<&str> = source.lines().filter_map(|line| line.strip_prefix("struct ")?.strip_suffix(": AppIntent {")).collect();
        assert_eq!(declared, ["AskBuntini", "TodayOnMac", "StartFocus", "KeepMacAwake"]);
        let metadata: Value = serde_json::from_str(include_str!("../../native/intents/Metadata.appintents/extract.actionsdata")).unwrap();
        let mut actions: Vec<&str> = metadata["actions"].as_object().unwrap().keys().map(String::as_str).collect();
        actions.sort();
        let mut expected = declared.clone();
        expected.sort();
        assert_eq!(actions, expected, "run native/intents/metadata.sh after changing an intent");
        for action in metadata["actions"].as_object().unwrap().values() {
            assert!(action["mangledTypeName"].as_str().unwrap().contains("BuntiniIntents"), "made from the BuntiniIntents module");
        }
        assert_eq!(metadata["autoShortcuts"].as_array().unwrap().len(), 4);
        for intent in ["ask", "today", "focus", "awake"] {
            assert!(source.contains(&format!("\"intent\": \"{intent}\"")), "{intent}");
        }
    }

    #[test]
    fn every_language_names_the_actions() {
        let english = include_str!("../../native/intents/Localizable/en.lproj/Localizable.strings");
        for key in ["Ask Buntini", "Question", "Today on the Mac", "Start Focus Timer", "Minutes", "Keep Mac Awake"] {
            assert!(english.contains(&format!("\"{key}\" = ")), "{key}");
        }
        let languages = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../native/intents/Localizable")).unwrap().count();
        assert_eq!(languages, 16);
    }

    #[test]
    fn unknown_and_empty_requests_are_turned_away() {
        assert!(answer(&json!({ "intent": "ask", "question": "  " }))["error"].is_string());
        assert!(answer(&json!({ "intent": "delete everything" }))["error"].is_string());
        assert_eq!(minutes(&json!({ "minutes": 0 }), 240), None);
        assert_eq!(minutes(&json!({ "minutes": 25 }), 240), Some(25));
        assert_eq!(minutes(&json!({ "minutes": 999 }), 240), None);
        let mut out = vec![0u8; 256];
        let request = br#"{"intent":"nope"}"#;
        let written = handle(request.as_ptr(), request.len(), out.as_mut_ptr(), out.len());
        let reply: Value = serde_json::from_slice(&out[..written as usize]).unwrap();
        assert!(reply["error"].is_string());
        assert_eq!(handle(request.as_ptr(), request.len(), out.as_mut_ptr(), 4), -1);
    }
}
