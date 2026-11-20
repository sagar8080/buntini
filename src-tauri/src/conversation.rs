//! Conversation memory: which earlier turns a model sees with a new message.
//!
//! Turns are packed newest-first into the backend's history budget (Apple ≈ 1.2K tokens,
//! MLX ≈ 6K, cloud ≈ 12K). The caller scopes turns to the task's saved conversation.
//! The timestamp boundary remains available for legacy callers/tests. External models only
//! receive turns whose local data they are allowed to see.
use crate::{
    privacy::{self, Locality, Share},
    types::{Settings, TaskRecord},
};
use serde_json::{json, Value};

/// At most this many turns: the MLX worker accepts 40 messages including tool calls.
const MAX_TURNS: usize = 12;

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

/// Whether a model running at `locality` may see this turn again.
fn shareable(turn: &TaskRecord, locality: Locality, settings: &Settings) -> bool {
    locality == Locality::OnDevice
        || (privacy::allows(locality, settings, Share::History)
            && turn
                .touched
                .iter()
                .all(|id| settings.external_shares.iter().any(|share| share == id)))
}

/// Earlier turns as user/assistant messages in chronological order. `turns` is newest
/// first (as the store lists them); `boundary` is the last "New topic" time.
#[cfg(test)]
pub fn history(
    turns: &[TaskRecord],
    boundary: Option<&str>,
    budget_tokens: usize,
    locality: Locality,
    settings: &Settings,
) -> Vec<Value> {
    history_with_shares(turns, boundary, budget_tokens, locality, settings).0
}

/// Pack history together with its original data provenance. A new answer can repeat
/// facts from an earlier answer, so it must inherit the included turns' categories.
pub fn history_with_shares(
    turns: &[TaskRecord],
    boundary: Option<&str>,
    budget_tokens: usize,
    locality: Locality,
    settings: &Settings,
) -> (Vec<Value>, Vec<String>) {
    let mut picked = Vec::new();
    let mut shares = Vec::new();
    let mut used = 0;
    for turn in turns {
        if boundary.is_some_and(|boundary| turn.created_at.as_str() <= boundary) {
            break;
        }
        // Briefs have no prompt of their own and aren't part of the back-and-forth.
        if turn.state != "completed"
            || turn.prompt.trim().is_empty()
            || turn.answer.trim().is_empty()
            || !shareable(turn, locality, settings)
        {
            continue;
        }
        let (prompt, answer) = (
            clip(turn.prompt.trim(), 1200),
            clip(turn.answer.trim(), 1500),
        );
        let cost = (prompt.len() + answer.len()) / 4 + 8;
        if used + cost > budget_tokens || picked.len() == MAX_TURNS {
            break;
        }
        used += cost;
        for share in &turn.touched {
            if !shares.contains(share) {
                shares.push(share.clone());
            }
        }
        picked.push([
            json!({"role":"user","content":prompt}),
            json!({"role":"assistant","content":answer}),
        ]);
    }
    (picked.into_iter().rev().flatten().collect(), shares)
}

/// Conversation history belongs to the current topic. An explicit question about a
/// different topic can be answered without inspecting or guessing another topic's history.
pub(crate) fn asks_about_another_topic(prompt: &str) -> bool {
    let plain = crate::completion::unquoted(prompt);
    let request = crate::completion::request_clause(&plain);
    if !["what ", "which ", "when ", "where ", "who ", "how ", "do you remember "]
        .iter().any(|opening| request.starts_with(opening))
    {
        return false;
    }
    let words: Vec<&str> = request.split(|ch: char| !ch.is_alphabetic()).filter(|word| !word.is_empty()).collect();
    words.windows(3).any(|window| {
        ["in", "from", "during", "about"].contains(&window[0])
            && ["previous", "last", "earlier", "other", "different", "separate"].contains(&window[1])
            && ["chat", "conversation", "topic", "thread"].contains(&window[2])
    }) || words.windows(4).any(|window| {
        ["in", "from", "during", "about"].contains(&window[0])
            && ["my", "our", "the", "a", "another"].contains(&window[1])
            && ["previous", "last", "earlier", "other", "different", "separate"].contains(&window[2])
            && ["chat", "conversation", "topic", "thread"].contains(&window[3])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_cross_topic_questions_are_distinct_from_followups() {
        assert!(asks_about_another_topic("What number did you calculate in my previous conversation?"));
        assert!(asks_about_another_topic("What did you say in the last chat?"));
        assert!(!asks_about_another_topic("Now double that answer."));
        assert!(!asks_about_another_topic("Translate \"What did you say in the last chat?\""));
    }


    fn turn(id: &str, at: &str, prompt: &str, answer: &str, touched: &[&str]) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            prompt: prompt.into(),
            answer: answer.into(),
            state: "completed".into(),
            created_at: at.into(),
            sources: vec![],
            files: vec![],
            system: None,
            glance: None,
            research: vec![],
        research_report: None,
            prices: vec![],
            cards: vec![],
            backend: "apple".into(),
            touched: touched.iter().map(|id| id.to_string()).collect(),
            handoff: None,
            action: None,
            choices: vec![],
            apps: vec![],
        }
    }

    #[test]
    fn recent_turns_come_back_in_order_within_budget() {
        let turns = vec![
            turn(
                "3",
                "2026-10-21T12:03:00Z",
                "and tomorrow?",
                "Cooler, 18°C.",
                &[],
            ),
            turn(
                "2",
                "2026-10-21T12:02:00Z",
                "weather in Porto",
                "Sunny, 22°C.",
                &[],
            ),
            turn(
                "1",
                "2026-10-21T12:01:00Z",
                "x".repeat(4000).as_str(),
                "long",
                &[],
            ),
        ];
        let messages = history(&turns, None, 200, Locality::OnDevice, &Settings::default());
        let contents = messages
            .iter()
            .map(|m| m["content"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            contents,
            [
                "weather in Porto",
                "Sunny, 22°C.",
                "and tomorrow?",
                "Cooler, 18°C."
            ],
            "the oversized oldest turn is left out"
        );
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn new_topic_and_failed_turns_are_skipped() {
        let mut failed = turn("2", "2026-10-21T12:02:00Z", "oops", "error text", &[]);
        failed.state = "error".into();
        let turns = vec![
            failed,
            turn("1", "2026-10-21T12:01:00Z", "hi", "hello", &[]),
            turn("0", "2026-10-21T11:00:00Z", "old topic", "old", &[]),
        ];
        let messages = history(
            &turns,
            Some("2026-10-21T11:30:00Z"),
            1000,
            Locality::OnDevice,
            &Settings::default(),
        );
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "hi");
    }

    #[test]
    fn external_models_only_see_turns_they_may_see() {
        let turns = vec![
            turn(
                "2",
                "2026-10-21T12:02:00Z",
                "find my lease",
                "lease.pdf in Documents",
                &["activity", "files"],
            ),
            turn(
                "1",
                "2026-10-21T12:01:00Z",
                "tell me a joke",
                "Why did the cat…",
                &[],
            ),
        ];
        let mut settings = Settings::default();
        assert!(
            history(&turns, None, 1000, Locality::External, &settings).is_empty(),
            "no history without permission"
        );
        settings.external_shares = vec!["history".into()];
        let messages = history(&turns, None, 1000, Locality::External, &settings);
        assert_eq!(
            messages.len(),
            2,
            "only the turn that touched no local data"
        );
        assert_eq!(messages[0]["content"], "tell me a joke");
        settings.external_shares = vec!["history".into(), "activity".into(), "files".into()];
        assert_eq!(
            history(&turns, None, 1000, Locality::External, &settings).len(),
            4
        );
    }

    #[test]
    fn a_my_day_turn_is_not_shared_without_calendar_permission() {
        let touched: Vec<&str> = crate::skills::day::shares(
            &crate::brief::Day {
                events: Some(vec![("09:00".into(), "Dentist".into())]),
                reminders: None,
                calendar_allowed: true,
                reminders_allowed: false,
                calendar_coverage: None,
                reminders_coverage: None,
                mail: Some(Default::default()),
                mail_allowed: true,
                mail_skipped: false,
            },
            false,
        )
        .into_iter()
        .map(Share::id)
        .collect();
        assert_eq!(touched, ["calendar", "mail"]);
        let day = turn("1", "2026-10-21T08:00:00Z", "my day", "Dentist at 9.", &touched);
        let mut settings = Settings {
            external_shares: vec!["history".into(), "mail".into()],
            ..Settings::default()
        };
        assert!(!shareable(&day, Locality::External, &settings));
        assert!(history(&[day.clone()], None, 100, Locality::External, &settings).is_empty());
        settings.external_shares.push("calendar".into());
        assert!(shareable(&day, Locality::External, &settings));
    }

    #[test]
    fn history_inherits_only_the_categories_of_included_turns() {
        let turns = vec![
            turn(
                "3",
                "2026-10-21T12:03:00Z",
                "what does it say?",
                "The appointment is at 4.",
                &["mail", "activity"],
            ),
            turn(
                "2",
                "2026-10-21T12:02:00Z",
                "read my latest mail",
                "An appointment at 4.",
                &["mail"],
            ),
            turn(
                "1",
                "2026-10-21T12:01:00Z",
                "older private files",
                "x".repeat(2000).as_str(),
                &["files"],
            ),
        ];
        let (messages, shares) =
            history_with_shares(&turns, None, 100, Locality::OnDevice, &Settings::default());
        assert_eq!(messages.len(), 4);
        assert_eq!(shares, vec!["mail", "activity"]);
        assert!(
            history_with_shares(&turns, None, 0, Locality::OnDevice, &Settings::default())
                .1
                .is_empty()
        );

        // A follow-up may paraphrase mail without calling the mail tool again.
        // Once it inherits this provenance, cloud replay still needs mail access.
        let followup = turn(
            "4",
            "2026-10-21T12:04:00Z",
            "remind me of the time",
            "At 4.",
            &shares.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let mut settings = Settings {
            external_shares: vec!["history".into(), "activity".into()],
            ..Settings::default()
        };
        assert!(history(
            &[followup.clone()],
            None,
            100,
            Locality::External,
            &settings
        )
        .is_empty());
        settings.external_shares.push("mail".into());
        assert_eq!(
            history(&[followup], None, 100, Locality::External, &settings).len(),
            2
        );
    }
}
