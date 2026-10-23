//! Executable corpus against the production deterministic router. Never runs a skill.
//! A fixture pass establishes the routing contract, not model or device accuracy.
use crate::skills::{self, browser, mac, Skill};
use serde_json::{json, Value};

fn observed(prompt: &str, recent: &[String]) -> Value {
    match skills::recognise(prompt, recent) {
        None => json!({"skill":"model"}),
        Some(Skill::Music(command)) => {
            json!({"skill":"music", "action":command.action, "query":command.query})
        }
        Some(Skill::Browser(command)) => match command {
            browser::Command::OpenSite { url, .. } => {
                json!({"skill":"browser", "action":"open_site", "url":url})
            }
            browser::Command::CloseTab => json!({"skill":"browser", "action":"close_tab"}),
            browser::Command::Page => json!({"skill":"browser", "action":"read_page"}),
            other => json!({"skill":"browser", "action":format!("{other:?}")}),
        },
        Some(Skill::Mac(command)) => match command {
            mac::Command::Timer { seconds } => {
                json!({"skill":"mac", "action":"timer", "seconds":seconds})
            }
            mac::Command::CancelTimers => json!({"skill":"mac", "action":"cancel_timers"}),
            mac::Command::CancelAllTimers => json!({"skill":"mac", "action":"cancel_all_timers"}),
            mac::Command::Volume(mac::Volume::Set(level)) => {
                json!({"skill":"mac", "action":"volume", "level":level})
            }
            other => json!({"skill":"mac", "action":format!("{other:?}")}),
        },
    }
}

#[test]
fn scenario_evals_route_corpus_without_unintended_direct_actions() {
    let corpus: Value = serde_json::from_str(include_str!("../../evals/scenarios.json")).unwrap();
    let mut failures = Vec::new();
    let mut samples = 0;
    let mut unintended = 0;
    for case in corpus["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["kind"] == "routing")
    {
        samples += 1;
        let recent: Vec<String> = serde_json::from_value(case["recent"].clone()).unwrap();
        let result = observed(case["prompt"].as_str().unwrap(), &recent);
        if case["expected"]["skill"] == "model" && result["skill"] != "model" {
            unintended += 1;
        }
        if !case["expected"]
            .as_object()
            .unwrap()
            .iter()
            .all(|(field, expected)| result.get(field) == Some(expected))
        {
            failures.push(format!(
                "{}: expected {}, got {}",
                case["id"], case["expected"], result
            ));
        }
    }
    println!(
        "BUNTINI_SCENARIO_METRICS {}",
        json!({"suite":"deterministic_routing", "samples":samples, "passed":samples-failures.len(), "unintended_direct_actions":unintended, "model_calls":0, "app_writes":0})
    );
    assert!(
        samples >= 30,
        "Keep a meaningful corpus, not an empty green gate"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
