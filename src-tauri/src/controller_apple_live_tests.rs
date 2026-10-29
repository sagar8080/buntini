//! Opt-in, real Apple Intelligence workflows over disposable state and sources.
//! A case cannot pass without an actual Foundation Models SDK invocation.
use super::*;
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path, time::Instant};

#[path = "controller_writing_checks.rs"]
mod writing_checks;

pub(super) fn record_live_case(row: Value) -> Value {
    let path = std::env::var("BUNTINI_APPLE_WORKFLOW_REPORT").unwrap();
    let mut out = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(out, "{row}").unwrap();
    out.flush().unwrap();
    eprintln!(
        "BUNTINI_REAL_WORKFLOW {}",
        json!({"id":row["id"],"passed":row["passed"],"apple_calls":row["apple_calls"]})
    );
    row
}

#[derive(serde::Serialize)]
pub(super) struct LiveTurn {
    pub task: TaskRecord,
    pub workflow: Option<Workflow>,
    pub apple_calls: usize,
    pub error: Option<String>,
    pub duration_ms: u128,
}

pub(super) fn live_state(dir: &Path) -> AppState {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut state = AppState::new(dir.join("data"), dir.join("cache"), root.clone()).unwrap();
    state.models = ModelHub::new(root.join("worker/main.py"), root.join("native/buntini-fm"));
    state.models.set_ledger(state.store.clone());
    state
        .store
        .lock()
        .unwrap()
        .save_settings(&Settings {
            model_backend: "apple".into(),
            pet_roam: false,
            pet_awareness: false,
            pet_nudges: false,
            apps: vec![],
            external_shares: vec![],
            ..Settings::default()
        })
        .unwrap();
    state
}

pub(super) async fn run_live_prompt(state: &AppState, prompt: &str) -> LiveTurn {
    let started = Instant::now();
    let before = state.models.apple.model_calls();
    let mut task: TaskRecord = serde_json::from_value(json!({
        "id":uuid::Uuid::new_v4().to_string(), "prompt":prompt, "answer":"",
        "state":"running", "createdAt":now(), "sources":[], "files":[], "backend":"apple"
    }))
    .unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let channel = Channel::new(|_| Ok(()));
    let settings = state.store.lock().unwrap().settings().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(150),
        execute_with_settings(
            state,
            &mut task,
            "auto",
            &channel,
            cancel.clone(),
            settings,
            None,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            cancel.store(true, Ordering::SeqCst);
            state.models.reset().await;
            Err("Workflow deadline exceeded.".into())
        }
    };
    let error = result.as_ref().err().cloned();
    let persisted = state.store.lock().unwrap().workflow(&task.id).unwrap();
    if let Some(mut flow) = persisted {
        finish_workflow(state, &mut flow, &mut task, result, false).unwrap();
    } else {
        task.state = if result.is_ok() { "completed" } else { "error" }.into();
        if let Some(message) = &error {
            task.answer = message.clone();
        }
        state.store.lock().unwrap().save_task(&task).unwrap();
    }
    let workflow = state.store.lock().unwrap().workflow(&task.id).unwrap();
    LiveTurn {
        task,
        workflow,
        apple_calls: state.models.apple.model_calls() - before,
        error,
        duration_ms: started.elapsed().as_millis(),
    }
}

pub(super) async fn approve_live_action(state: &AppState, id: &str, args: Value) -> LiveTurn {
    let started = Instant::now();
    let before = state.models.apple.model_calls();
    let channel = Channel::new(|_| Ok(()));
    let result = run_workflow_action(state, id, args, &channel).await;
    let error = result.as_ref().err().cloned();
    let task = match result {
        Ok(task) => task,
        Err(_) => {
            let store = state.store.lock().unwrap();
            let flow = store.workflow(id).unwrap().unwrap();
            store.task(&flow.id).unwrap().unwrap()
        }
    };
    let workflow = state.store.lock().unwrap().workflow(&task.id).unwrap();
    LiveTurn {
        task,
        workflow,
        apple_calls: state.models.apple.model_calls() - before,
        error,
        duration_ms: started.elapsed().as_millis(),
    }
}

fn case(id: &str, title: &str, turn: &LiveTurn, checks: Vec<(&str, bool)>) -> Value {
    let passed =
        turn.apple_calls > 0 && turn.task.state == "completed" && checks.iter().all(|(_, ok)| *ok);
    json!({"id":id,"title":title,"passed":passed,"apple_calls":turn.apple_calls,
        "checks":checks.into_iter().map(|(name,passed)|json!({"name":name,"passed":passed})).collect::<Vec<_>>(),
        "observation":turn,"source_kind":"user-provided synthetic text in a real production controller request","real_app_writes":0})
}

async fn writing_workflows() -> Vec<Value> {
    let cases: &[(&str,&str,&str,&[&str])] = &[
        ("email-rewrite", "Rewrite a late-delivery email", "Rewrite this message as a polite professional email, preserving the deadline: 'Hey Maya, the monitor arrived damaged. Please arrange a replacement by November 6. Order QZ742.'", &["Maya","November 6","QZ742","replacement"]),
        ("meeting-actions", "Turn meeting notes into assigned next steps", "Turn these meeting notes into a concise action checklist with owners and deadlines: Maya sends the revised budget by Friday; Leo books the venue by Tuesday; I approve the final agenda on Thursday. Preserve all three assignments.", &["Maya","Friday","Leo","Tuesday","Thursday"]),
        ("travel-itinerary", "Plan a realistic half-day itinerary", "Plan a half-day walking itinerary using only these supplied opening hours: Library opens 10:00-13:00, cafe opens 09:00-16:00, museum opens 12:00-17:00. I start at 09:30 and each visit takes 45 minutes; allow 15 minutes to walk between places. Include a start time for each stop, and visit all three while they are open.", &["09:30","10:30","12:00"]),
        ("expense-split", "Split a shared weekend bill", "Split a $240 cabin and $84 groceries equally between four friends. Then add a $16 parking fee paid only by Maya and Leo equally. State the total owed by each of Maya, Leo, Noor, and Sam. Show the arithmetic so I can check it.", &["89","81","Maya","Leo","Noor","Sam"]),
        ("subscription-audit", "Review recurring subscriptions", "Review these monthly subscriptions and tell me how much I save per month and per year by cancelling only the unused ones: Music $12 used; Video $18 unused; Storage $3 used; Fitness $15 unused. Keep the used subscriptions.", &["33","396","Video","Fitness"]),
        ("compare-laptops", "Compare supplied laptop offers", "Compare only these supplied laptop offers for a student who needs at least 16GB RAM: Alder 8GB RAM $699, Birch 16GB RAM $849, Cedar 16GB RAM $999. Which qualifying option is cheapest, and how much less is it than the other qualifying option? Do not invent specifications.", &["Birch","849","150"]),
        ("reply-draft", "Draft a reply from a pasted invitation", "Draft a friendly reply to this invitation. Accept Saturday November 7 at 2pm, mention I will bring a vegetarian dish, and ask whether there is parking: 'Hi Casey, can you join lunch on Saturday November 7 at 2pm? - Maya'. Do not send anything.", &["November 7","2","vegetarian","parking"]),
        ("packing-checklist", "Build a tailored packing checklist", "Create a packing checklist for a two-night train trip. Forecast supplied by me: rain and 8-12°C. I need to work on my laptop, and the hotel provides towels and a hair dryer. Include rain protection, warmth, train ticket and laptop charging; omit towels and hair dryer from things to pack.", &["rain","warm","ticket","charg"]),
        ("recipe-shopping", "Scale a recipe into a shopping list", "Scale this recipe from 2 servings to 6 and give the shopping amounts: 150g pasta, 200g tomatoes, 1 onion, 2 tablespoons olive oil. Preserve the units and list exactly these four ingredients.", &["450","600","3","6","pasta","tomato","onion","oil"]),
        ("document-summary", "Summarize an office policy accurately", "Summarize this office policy into three bullets: Remote work is permitted on Tuesdays and Thursdays. Requests must reach the manager by 4pm the previous working day. New starters are ineligible during their first 30 days. Preserve the days, deadline, and eligibility rule.", &["Tuesday","Thursday","4","30"]),
        ("announcement-tone", "Rewrite a team announcement", "Rewrite this announcement in a warm concise tone without changing facts: 'The office will close at 3pm on November 12. Submit expenses by noon November 11. Emergency support remains available at extension 204.'", &["3","November 12","noon","November 11","204"]),
        ("translation", "Translate a practical travel message", "Translate this into Spanish, preserving the name, time and booking code: 'Hello Maya, our train arrives at 18:40. Could we check in after 19:00? Booking code RX318.'", &["Maya","18:40","19:00","RX318"]),
    ];
    let mut results = Vec::new();
    for (id, title, prompt, required) in cases {
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let turn = run_live_prompt(&state, prompt).await;
        let lower = turn.task.answer.to_lowercase();
        let mut checks = writing_checks::checks(id, &turn.task.answer).unwrap_or_else(|| {
            required
                .iter()
                .map(|fact| (*fact, lower.contains(&fact.to_lowercase())))
                .collect()
        });
        checks.extend(writing_checks::general_checks(&turn.task.answer));
        results.push(record_live_case(case(id, title, &turn, checks)));
        state.models.shutdown().await;
    }
    results
}

#[tokio::test]
#[ignore = "actual Apple Intelligence everyday workflow acceptance; uses disposable state/files and opt-in real app tests"]
async fn live_apple_day_to_day_workflows() {
    let path = std::env::var("BUNTINI_APPLE_WORKFLOW_REPORT")
        .expect("Set a fresh BUNTINI_APPLE_WORKFLOW_REPORT path");
    let mut out = std::fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(path)
        .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let fingerprints = [
        "native/buntini-fm",
        "native/fm/main.swift",
        "src-tauri/src/controller.rs",
        "src-tauri/src/controller_apple_live_tests.rs",
        "src-tauri/src/controller_file_live_tests.rs",
        "src-tauri/src/controller_apps_live_tests.rs",
        "src-tauri/src/controller_controls_live_tests.rs",
        "src-tauri/src/controller_public_live_tests.rs",
        "src-tauri/src/controller_writing_checks.rs",
        "src-tauri/src/calculation.rs",
        "src-tauri/src/structured_math.rs",
        "src-tauri/src/writing_context.rs",
        "src-tauri/src/source_fields.rs",
        "src-tauri/src/location_fields.rs",
        "src-tauri/src/integrations/weather_advice.rs",
        "src-tauri/src/local.rs",
        "src-tauri/src/indexer.rs",
        "src-tauri/src/tools.rs",
        "src-tauri/src/task_contract.rs",
        "src-tauri/src/completion.rs",
        "src-tauri/src/explicit_fields.rs",
        "src-tauri/src/apps.rs",
        "src-tauri/src/actions.rs",
        "src-tauri/src/control_actions.rs",
        "src-tauri/src/store.rs",
        "src-tauri/src/worker.rs",
        "src-tauri/src/lib.rs",
        "src-tauri/src/integrations/mod.rs",
        "src-tauri/src/integrations/summary.rs",
        "worker/tools.json",
        "worker/protocol.py",
    ]
    .into_iter()
    .map(|path| {
        (
            path,
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(root.join(path)).unwrap())
            ),
        )
    })
    .collect::<std::collections::BTreeMap<_, _>>();
    let test_binary = std::env::current_exe().unwrap();
    let test_binary_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&test_binary).unwrap()));
    writeln!(out,"{}",json!({"kind":"run","startedAt":now(),"fingerprints":fingerprints,"test_binary_sha256":test_binary_sha256,"profile":"debug opt-in acceptance; not a signed release bundle","entry":"production controller backend; UI not exercised","cloud":false})).unwrap();
    out.flush().unwrap();
    let mut all = Vec::new();
    let group = std::env::var("BUNTINI_APPLE_WORKFLOW_GROUP").unwrap_or_else(|_| "all".into());
    assert!(
        ["all", "writing", "files", "apps", "controls", "public"].contains(&group.as_str()),
        "Unknown live workflow group"
    );
    if group == "all" || group == "writing" {
        all.extend(writing_workflows().await);
    }
    if group == "all" || group == "files" {
        all.extend(super::file_live_tests::file_memory_workflows().await);
    }
    if group == "all" || group == "controls" {
        all.extend(super::controls_live_tests::control_workflows().await);
    }
    if group == "all" || group == "public" {
        all.extend(super::public_live_tests::public_workflows().await);
    }
    #[cfg(debug_assertions)]
    if group == "all" || group == "apps" {
        all.extend(super::apps_live_tests::app_workflows().await);
    }
    assert!(!all.is_empty(), "No live workflows were attempted");
    if group == "all" {
        assert!(
            all.len() >= 30,
            "The complete acceptance run must attempt at least thirty distinct workflows"
        );
    }
    let passed = all.iter().filter(|row| row["passed"] == true).count();
    let user_passed = all
        .iter()
        .filter(|row| row["passed"] == true && row["acceptance_kind"] != "boundary")
        .count();
    let distinct = all
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        distinct.len(),
        all.len(),
        "Each counted workflow must have a distinct goal ID"
    );
    let summary = json!({"kind":"summary","attempted":all.len(),"passed":passed,"failed":all.len()-passed,"successful_user_workflows":user_passed,"successful_boundary_checks":passed-user_passed,"actual_apple_model_calls":all.iter().filter_map(|r|r["apple_calls"].as_u64()).sum::<u64>(),"entry":"production controller backend; UI not exercised","model":"Apple Foundation Models system default","real_app_receipts":"only explicitly enabled app group; file/text cases use real disposable files/stores"});
    writeln!(out, "{}", summary).unwrap();
    eprintln!("BUNTINI_REAL_SUMMARY {summary}");
    if group == "all" {
        assert!(user_passed>=30,"At least thirty positive real user workflows must succeed; boundary checks do not count toward that goal");
    }
    assert_eq!(
        passed,
        all.len(),
        "Preserve failed evidence before fixing workflows"
    );
}
