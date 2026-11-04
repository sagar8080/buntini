//! ChatGPT beside the main brain. It gets one question the user can see and nothing else:
//! no history, memory, files or sharing ticks. The user approves each question, unless they
//! chose Always allow for public internet questions, which `may_skip_sheet` keeps narrow.
use crate::{
    brain::{self, Plan},
    controller::AppState,
    integrations::Preview,
    types::*,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::State;

/// A public internet question the main brain wanted to look up.
pub const SEARCH_TOOL: &str = "chatgpt_search";
/// The user asked for ChatGPT by name, or pressed the button on a reply.
pub const ASK_TOOL: &str = "ask_chatgpt";
/// The journal's and the reply's record of the exact question that left the Mac.
pub const ROUTE: &str = "chatgpt";

const ALWAYS_ALLOW: &str = "chatgpt-always-allow";
const SEARCH_PAUSED_UNTIL: &str = "chatgpt-search-paused-until";
const UNTIL_NEXT_SIGN_IN: &str = "next-sign-in";
const SEARCH_PAUSE_MINUTES: i64 = 10;
const MOST_SOURCES: usize = 10;
/// Longer than this, a question goes past the sheet only if it's looked at first.
const LONGEST_UNREVIEWED: usize = 300;
const INSTRUCTIONS: &str = "Answer the question. Search the web when the answer depends on current or specific public information. Be brief.";

pub fn is_tool(name: &str) -> bool {
    name == SEARCH_TOOL || name == ASK_TOOL
}

/// Whether this Mac's sign-in can ask ChatGPT anything.
pub fn usable(state: &AppState) -> bool {
    brain::plan(state) == Plan::Usable
}

/// Whether internet questions go to ChatGPT right now.
pub fn ready(state: &AppState) -> bool {
    let paused = preference(state, SEARCH_PAUSED_UNTIL);
    !paused_at(&paused, chrono::Utc::now()) && usable(state)
}

fn preference(state: &AppState, key: &str) -> String {
    state.store.lock().unwrap().preference(key).ok().flatten().unwrap_or_default()
}

fn paused_at(value: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    value == UNTIL_NEXT_SIGN_IN
        || chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|until| until > now)
}

/// What the failure means for the next internet question: None to leave things as they are,
/// or the value that holds ChatGPT back from searching.
fn pause_for(message: &str, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    if !offers_browser(message) {
        None
    } else if message.starts_with("ChatGPT doesn’t support")
        || message.starts_with("ChatGPT doesn’t allow")
        || message.contains("couldn’t answer (400)")
    {
        Some(UNTIL_NEXT_SIGN_IN.into())
    } else {
        Some((now + chrono::Duration::minutes(SEARCH_PAUSE_MINUTES)).to_rfc3339())
    }
}

/// A question ChatGPT couldn't take can go to the browser route, after its own approval. One
/// it turned down on its merits, or the user stopped, can't.
pub fn offers_browser(message: &str) -> bool {
    message != "Task cancelled." && !message.contains("declined to answer")
}

/// Holds ChatGPT back from searching after it couldn't take a question.
pub fn pause_search(state: &AppState, message: &str) {
    if let Some(value) = pause_for(message, chrono::Utc::now()) {
        let _ = state.store.lock().unwrap().set_preference(SEARCH_PAUSED_UNTIL, &value);
    }
}

pub fn always_allowed(state: &AppState) -> bool {
    !preference(state, ALWAYS_ALLOW).is_empty()
}

pub fn set_always_allowed(state: &AppState, on: bool) -> Result<(), String> {
    state.store.lock().unwrap().set_preference(ALWAYS_ALLOW, if on { "1" } else { "" })?;
    state.audit("chatgpt.always_allow", if on { "On" } else { "Off" });
    Ok(())
}

/// A new sign-in gets a fresh answer about searching.
pub fn signed_in(store: &crate::store::Store) -> Result<(), String> {
    store.set_preference(SEARCH_PAUSED_UNTIL, "")
}

/// Signing out also takes back Always allow.
pub fn signed_out(store: &crate::store::Store) -> Result<(), String> {
    store.set_preference(SEARCH_PAUSED_UNTIL, "")?;
    store.set_preference(ALWAYS_ALLOW, "")
}

#[tauri::command]
pub fn chatgpt_always_allow_off(state: State<'_, AppState>) -> Result<(), String> {
    set_always_allowed(&state, false)
}

/// Where a question came from, for deciding whether it needs the sheet.
pub struct Origin {
    /// What the user asked, in their words.
    pub goal: String,
    /// Kinds of local data this task has read.
    pub touched: Vec<String>,
    /// False for a routine, a goal, or anything else that ran with nobody at the window.
    pub attended: bool,
}

/// Every word of the question is one the user wrote.
fn own_words(goal: &str, question: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let asked: HashSet<String> = words(goal).into_iter().collect();
    let sent = words(question);
    !sent.is_empty() && sent.iter().all(|word| asked.contains(word))
}

/// Always allow covers a public internet question the user is there to see, in their own
/// words, in a task that read nothing local. Anything else shows the sheet, so private text
/// can't leave inside a question nobody reviewed, including through instructions hidden in
/// something the model read, and a question the model reworded from what it remembers.
pub fn may_skip_sheet(always: bool, tool: &str, origin: &Origin, question: &str) -> bool {
    always
        && tool == SEARCH_TOOL
        && origin.attended
        && origin.touched.is_empty()
        && question.chars().count() <= LONGEST_UNREVIEWED
        && own_words(&origin.goal, question)
}

/// Everything ChatGPT receives for one question.
pub fn payload(question: &str) -> Value {
    json!({
        "messages": [
            {"role":"system","content":INSTRUCTIONS},
            {"role":"user","content":question},
        ],
        "tools": [],
        "web_search": true,
    })
}

pub fn preview(arguments: &Value) -> Preview {
    let question = arguments["query"].as_str().unwrap_or_default();
    Preview {
        provider: "ChatGPT".into(),
        host: "api.openai.com".into(),
        summary: "Only this question is sent.".into(),
        sends: vec![CardField::new("Question", question)],
        uses_key: false,
    }
}

/// The pages ChatGPT's answer cited, from the finished reply's annotations.
pub fn sources(raw: Option<&Value>) -> Vec<Source> {
    let mut found: Vec<Source> = Vec::new();
    let items = raw.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let citations = items
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().map(Vec::as_slice).unwrap_or_default())
        .flat_map(|part| part["annotations"].as_array().map(Vec::as_slice).unwrap_or_default())
        .filter(|annotation| annotation["type"] == "url_citation");
    for citation in citations {
        let Some(url) = citation["url"].as_str() else { continue };
        if found.len() == MOST_SOURCES
            || crate::research::validate_public_url(url).is_err()
            || found.iter().any(|source| source.url == url)
        {
            continue;
        }
        let title = citation["title"].as_str().filter(|title| !title.trim().is_empty());
        let host = reqwest::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string));
        found.push(Source {
            title: title.map(str::to_string).or(host).unwrap_or_else(|| url.to_string()),
            url: url.to_string(),
            snippet: String::new(),
            retrieved_at: now(),
        });
    }
    found
}

/// The record of an answer ChatGPT gave: the sources it named, and the question that was sent.
pub fn report(goal: &str, question: &str, sources: Vec<Source>, answered: bool) -> crate::search_pipeline::Report {
    crate::search_pipeline::Report {
        intent: crate::search_intent::SearchIntent::from_lookup(goal, question),
        attempts: vec![crate::search_pipeline::Attempt {
            route: ROUTE.into(),
            status: if answered { "answered" } else { "failed" }.into(),
            detail: question.to_string(),
        }],
        sources,
        pages: vec![],
        limitations: vec![],
        evidence_ready: answered,
        answer_verified: answered,
    }
}

pub fn answered(report: Option<&crate::search_pipeline::Report>) -> bool {
    report.is_some_and(|report| report.attempts.iter().any(|attempt| attempt.route == ROUTE && attempt.status == "answered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(goal: &str) -> Origin {
        Origin { goal: goal.into(), touched: vec![], attended: true }
    }

    #[test]
    fn a_request_holds_only_the_question_and_can_search_on_this_route_alone() {
        let request = crate::cloud::build("chatgpt", "gpt-6", "token", &payload("Who won last night?")).unwrap();
        assert_eq!(request.body["input"], json!([{"role":"user","content":"Who won last night?"}]));
        assert_eq!(request.body["instructions"], INSTRUCTIONS);
        assert_eq!(request.body["tools"], json!([{"type":"web_search"}]));
        assert_eq!(request.body["store"], false);
        for field in ["memory", "context", "previous_response_id", "metadata", "user"] {
            assert!(request.body.get(field).is_none(), "{field}");
        }

        let main_brain = json!({"messages":[{"role":"user","content":"hi"}],"tools":[]});
        let plain = crate::cloud::build("chatgpt", "gpt-6", "token", &main_brain).unwrap();
        assert!(plain.body.get("tools").is_none(), "the main brain doesn't search on its own");
        let elsewhere = crate::cloud::build("openai", "gpt-5-mini", "key", &payload("Who won?")).unwrap();
        assert!(elsewhere.body.get("tools").is_none(), "only the plan route");
    }

    #[test]
    fn a_stream_with_a_search_gives_the_answer_and_its_sources() {
        let events = [
            json!({"type":"response.output_item.done","item":{"type":"web_search_call","id":"ws_1","status":"completed"}}),
            json!({"type":"response.output_text.delta","delta":"Ana won."}),
            json!({"type":"response.output_text.annotation.added","annotation":{"type":"url_citation","url":"https://example.com/a"}}),
            json!({"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Ana won.","annotations":[
                {"type":"url_citation","url":"https://example.com/a","title":"Match report","start_index":0,"end_index":8},
                {"type":"url_citation","url":"https://example.com/a","title":"Twice"},
                {"type":"url_citation","url":"https://news.example.org/b","title":""},
                {"type":"url_citation","url":"http://localhost/secret","title":"Not public"},
                {"type":"url_citation","title":"No address"},
                {"type":"file_citation","url":"https://example.com/file","title":"Another kind"},
            ]}]}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":2}}}),
        ];
        let mut decoder = crate::cloud::Decoder::new("chatgpt");
        for event in &events {
            decoder.event(&event.to_string(), &mut |_| {}).unwrap();
        }
        let (turn, _) = decoder.finish().unwrap();
        assert_eq!(turn.text, "Ana won.");
        assert!(turn.tool.is_none(), "a hosted search isn't a tool for Buntini to run");
        let found = sources(turn.raw.as_ref());
        let named: Vec<_> = found.iter().map(|source| (source.title.as_str(), source.url.as_str())).collect();
        assert_eq!(named, [("Match report", "https://example.com/a"), ("news.example.org", "https://news.example.org/b")]);
        assert!(sources(None).is_empty());
    }

    #[test]
    fn sources_stop_at_a_sensible_number() {
        let annotations: Vec<Value> = (0..30)
            .map(|at| json!({"type":"url_citation","url":format!("https://example.com/{at}"),"title":"Page"}))
            .collect();
        let raw = json!([{"type":"message","content":[{"annotations":annotations}]}]);
        assert_eq!(sources(Some(&raw)).len(), MOST_SOURCES);
    }

    #[test]
    fn always_allow_covers_only_a_public_question_in_the_users_own_words() {
        let question = "Who won the match last night?";
        assert!(may_skip_sheet(true, SEARCH_TOOL, &origin(question), question));
        assert!(!may_skip_sheet(false, SEARCH_TOOL, &origin(question), question), "not chosen");
        assert!(!may_skip_sheet(true, ASK_TOOL, &origin(question), question), "\"Ask ChatGPT\" always shows it");
        assert!(!may_skip_sheet(true, "web_search", &origin(question), question));

        let read_something = Origin { touched: vec!["mail".into()], ..origin(question) };
        assert!(!may_skip_sheet(true, SEARCH_TOOL, &read_something, question), "a task that read something local");
        let nobody_there = Origin { attended: false, ..origin(question) };
        assert!(!may_skip_sheet(true, SEARCH_TOOL, &nobody_there, question), "a routine or a goal");

        let reworded = "Who won the match near 14 Elm Street last night?";
        assert!(!may_skip_sheet(true, SEARCH_TOOL, &origin(question), reworded), "words the user didn't write");
        assert!(may_skip_sheet(true, SEARCH_TOOL, &origin("Please look up: who won last night's match?"), "WHO WON last night's match"));
        assert!(!may_skip_sheet(true, SEARCH_TOOL, &origin("?"), "?"), "nothing to send");
        let long = "word ".repeat(100);
        assert!(!may_skip_sheet(true, SEARCH_TOOL, &origin(&long), &long), "too long to go unseen");
    }

    #[test]
    fn a_failure_holds_searching_back_for_as_long_as_it_will_last() {
        let now = chrono::Utc::now();
        let limit = crate::chatgpt::plan_error("subscription_sharing_usage_limit_exceeded", "").unwrap();
        let soon = pause_for(&limit, now).unwrap();
        assert!(paused_at(&soon, now));
        assert!(!paused_at(&soon, now + chrono::Duration::minutes(SEARCH_PAUSE_MINUTES + 1)));

        let refused = crate::chatgpt::plan_error("subscription_sharing_unsupported_capability", "tools").unwrap();
        for message in [refused.as_str(), "ChatGPT doesn’t allow this request from your account or region.", "ChatGPT couldn’t answer (400)."] {
            assert_eq!(pause_for(message, now).as_deref(), Some(UNTIL_NEXT_SIGN_IN), "{message}");
        }
        assert!(paused_at(UNTIL_NEXT_SIGN_IN, now + chrono::Duration::days(400)));

        for message in ["Task cancelled.", "The cloud model declined to answer this. Try rephrasing."] {
            assert!(!offers_browser(message), "{message}");
            assert_eq!(pause_for(message, now), None, "{message}");
        }
        assert!(!paused_at("", now), "nothing saved");
        assert!(!paused_at("not a date", now));
    }

    #[test]
    fn signing_out_takes_back_always_allow_and_signing_in_retries_search() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().into()).unwrap();
        assert!(!always_allowed(&state), "never on by itself");
        set_always_allowed(&state, true).unwrap();
        assert!(always_allowed(&state));
        pause_search(&state, "ChatGPT doesn’t allow this request from your account or region.");
        assert_eq!(preference(&state, SEARCH_PAUSED_UNTIL), UNTIL_NEXT_SIGN_IN);

        signed_in(&state.store.lock().unwrap()).unwrap();
        assert_eq!(preference(&state, SEARCH_PAUSED_UNTIL), "");
        assert!(always_allowed(&state), "a renewed sign-in keeps it");

        signed_out(&state.store.lock().unwrap()).unwrap();
        assert!(!always_allowed(&state));

        set_always_allowed(&state, true).unwrap();
        state.store.lock().unwrap().erase_everything().unwrap();
        assert!(!always_allowed(&state), "Delete everything");
    }

    #[test]
    fn a_saved_reply_shows_what_was_asked() {
        let failed = report("Who won?", "Who won?", vec![], false);
        assert!(!answered(Some(&failed)) && !failed.answer_verified);
        assert_eq!(failed.attempts[0].detail, "Who won?", "it was still sent");
        let saved = report("Who won?", "Who won?", vec![], true);
        assert!(answered(Some(&saved)));
        assert_eq!(saved.attempts[0].detail, "Who won?");
        assert!(saved.answer_verified && saved.evidence_ready);
        assert_eq!(saved.proof()["attempts"][0]["detail"], "Who won?");
        assert!(!answered(None));
    }
}
