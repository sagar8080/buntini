//! Opt-in, real-model checks of the chat brains and attachments: the person's own Codex and
//! Copilot as the brain, and Apple Intelligence, on ordinary questions, a real PDF read by
//! the Apple helper, a picture and a scanned PDF. Each answer is written to a report so it
//! can be read, not just scored.
//!
//! `BUNTINI_CHAT_REPORT=<file> BUNTINI_CHAT_FIXTURES=<folder with lease.pdf, lease-photo.png,
//! scanned-lease.pdf> BUNTINI_CHAT_BRAINS=codex,copilot,apple cargo test --lib live_chat -- --ignored --nocapture`
use super::*;
use std::io::Write;

struct Case {
    asked: &'static str,
    /// The file attached, from the fixtures folder.
    attached: Option<&'static str>,
    /// Every one of these must be in the answer (any one of each `|` group).
    says: &'static [&'static str],
    /// None of these may be.
    never: &'static [&'static str],
    /// A read the answer needs, or a proposal it should make.
    tool: Option<&'static str>,
    /// The turn should stop with this in its message instead of answering.
    refused: Option<&'static str>,
}

const fn case(asked: &'static str, attached: Option<&'static str>, says: &'static [&'static str], never: &'static [&'static str], tool: Option<&'static str>, refused: Option<&'static str>) -> Case {
    Case { asked, attached, says, never, tool, refused }
}

const CASES: &[Case] = &[
    case("Who wrote Pride and Prejudice?", None, &["austen"], &[], None, None),
    case("What's 18% of 2,340?", None, &["421.2"], &[], None, None),
    case("How much memory is my Mac using right now?", None, &["gb|%"], &[], Some("system_info"), None),
    case("Remind me to water the plants tomorrow at 9am", None, &[], &[], Some("add_reminder"), None),
    case("I'm meeting Sam at Blue Bottle on Friday at 3. Put it on my reminders an hour before.", None, &[], &[], Some("add_reminder"), None),
    case("How much is the security deposit?", Some("lease.pdf"), &["3,675|3675"], &[], None, None),
    case("Can I have a dog here, and what would it cost me up front?", Some("lease.pdf"), &["30|thirty", "500"], &[], None, None),
    case("When does this lease end and how much notice do I have to give?", Some("lease.pdf"), &["february 28, 2027|feb 28, 2027|28 february 2027|2027-02-28|february 28th, 2027", "60"], &[], None, None),
    case("Does the lease say anything about a swimming pool?", Some("lease.pdf"), &["no|not|doesn"], &["yes, "], None, None),
    case("Who pays for internet?", Some("lease.pdf"), &["tenant|you"], &["landlord pays internet|landlord pays for internet"], None, None),
    case("Which parking space comes with the unit?", Some("lease-photo.png"), &["17"], &[], None, None),
    case("What's the monthly rent?", Some("scanned-lease.pdf"), &[], &[], None, Some("any text")),
];

fn use_brain(settings: &mut Settings, brain: &str) {
    if brain == "apple" {
        settings.model_backend = "apple".into();
        return;
    }
    settings.model_backend = "cloud".into();
    settings.cloud_provider = brain.into();
    settings.cloud_model = String::new();
    // Only what these checks need: Mac health for the memory question.
    settings.external_shares = vec!["system".into()];
}

#[tokio::test]
#[ignore = "runs the person's own Codex and Copilot, and Apple Intelligence; set BUNTINI_CHAT_REPORT and BUNTINI_CHAT_FIXTURES"]
async fn live_chat_brains_and_attachments() {
    let report = std::env::var("BUNTINI_CHAT_REPORT").expect("BUNTINI_CHAT_REPORT");
    let fixtures = std::path::PathBuf::from(std::env::var("BUNTINI_CHAT_FIXTURES").expect("BUNTINI_CHAT_FIXTURES"));
    let brains = std::env::var("BUNTINI_CHAT_BRAINS").unwrap_or_else(|_| "codex,copilot,apple".into());
    let only = std::env::var("BUNTINI_CHAT_ONLY").ok();
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(report).unwrap();
    for brain in brains.split(',') {
        let (mut passed, mut asked) = (0, 0);
        for case in CASES {
            if only.as_deref().is_some_and(|only| !case.asked.contains(only)) {
                continue;
            }
            let dir = tempfile::tempdir().unwrap();
            let state = super::apple_live_tests::live_state(dir.path());
            let mut settings = state.store.lock().unwrap().settings().unwrap();
            settings.apps = vec!["reminders".into()];
            use_brain(&mut settings, brain);
            state.store.lock().unwrap().save_settings(&settings).unwrap();
            if brain == "apple" && state.models.probe_apple().await.0 {
                // Ready.
            } else if brain == "apple" {
                eprintln!("CHAT skipping apple: not available");
                break;
            }
            asked += 1;
            let mut task: TaskRecord = serde_json::from_value(json!({
                "id": uuid::Uuid::new_v4().to_string(), "prompt": case.asked, "answer": "", "state": "running",
                "createdAt": now(), "sources": [], "files": [], "backend": settings.model_backend,
            }))
            .unwrap();
            state.store.lock().unwrap().save_task(&task).unwrap();
            if let Some(file) = case.attached {
                crate::attach::stage(&task.id, vec![crate::attach::Attachment::File { path: fixtures.join(file).display().to_string(), brain: None }]);
            }
            let started = std::time::Instant::now();
            let streamed = Arc::new(Mutex::new(String::new()));
            let heard = streamed.clone();
            let channel = Channel::new(move |body| {
                if let tauri::ipc::InvokeResponseBody::Json(text) = body {
                    if let Ok(event) = serde_json::from_str::<Value>(&text) {
                        if event["type"] == "delta" {
                            heard.lock().unwrap().push_str(event["text"].as_str().unwrap_or_default());
                        }
                    }
                }
                Ok(())
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                execute_with_settings(&state, &mut task, "auto", &channel, Arc::new(AtomicBool::new(false)), settings.clone(), None),
            )
            .await
            .unwrap_or_else(|_| Err("timed out".into()));
            let error = result.as_ref().err().cloned();
            let persisted = state.store.lock().unwrap().workflow(&task.id).unwrap();
            let steps: Vec<Value> = persisted.as_ref().map(|flow| flow.steps.iter().map(|step| json!({"tool":step.tool,"arguments":step.arguments,"state":step.state})).collect()).unwrap_or_default();
            let given: Vec<Value> = persisted.as_ref().map(|flow| {
                let offered: Vec<&str> = flow.offered.iter().map(String::as_str).collect();
                flow.contract.tools_for_large_brain(&flow.steps, &crate::tools::offer(&offered)).as_array().into_iter().flatten().map(|tool| tool["name"].clone()).collect()
            }).unwrap_or_default();
            let outcomes: Vec<Value> = persisted.as_ref().map(|flow| flow.contract.outcomes.iter().map(|o| json!({"kind":o.kind,"tool":o.tool,"status":o.status})).collect()).unwrap_or_default();
            if let Some(mut flow) = persisted {
                let _ = finish_workflow(&state, &mut flow, &mut task, result, false);
            } else if let Err(message) = result {
                task.state = "error".into();
                task.answer = message;
            } else {
                task.state = "completed".into();
            }
            crate::attach::release(&task.id, &task.state);
            let shown = task.answer.replace('\\', "");
            let lower = shown.to_lowercase();
            let says = case.says.iter().all(|group| group.split('|').any(|word| lower.contains(word)));
            let clean = !case.never.iter().any(|group| group.split('|').any(|word| lower.contains(word)));
            let tool_ok = case.tool.is_none_or(|tool| steps.iter().any(|step| step["tool"] == tool) || task.action.as_ref().is_some_and(|action| action["tool"] == tool));
            let ok = match case.refused {
                Some(reason) => error.as_deref().is_some_and(|error| error.contains(reason)),
                // A brain that can't see pictures says so instead of answering.
                None if case.attached.is_some_and(|file| file.ends_with(".png")) && brain == "apple" => error.as_deref().is_some_and(|error| error.contains("can’t see pictures")),
                None => error.is_none() && says && clean && tool_ok,
            };
            passed += usize::from(ok);
            let row = json!({
                "brain": brain, "asked": case.asked, "attached": case.attached, "passed": ok, "says": says, "clean": clean, "tool": tool_ok,
                "state": task.state, "answer": shown, "error": error, "steps": steps, "proposal": task.action.as_ref().map(|action| action["tool"].clone()),
                "touched": task.touched, "given": given, "outcomes": outcomes, "streamed": streamed.lock().unwrap().len(), "seconds": started.elapsed().as_secs_f32(),
            });
            writeln!(out, "{row}").unwrap();
            out.flush().unwrap();
            eprintln!("CHAT {brain} {} {:.1}s {:?} → {}", if ok { "pass" } else { "FAIL" }, started.elapsed().as_secs_f32(), case.asked, shown.replace('\n', " ⏎ "));
            state.models.shutdown().await;
        }
        eprintln!("CHAT {brain}: {passed} of {asked} passed");
    }
}
