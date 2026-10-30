//! "Turn on the living room lights": the user's shortcuts are read with their own words
//! before any model call, and a run is proposed only with a name they really have.
//! Fixtures stand in for `shortcuts list`; nothing here runs a shortcut.
use super::*;

/// A model that must not be asked: shortcut runs are bound from the read alone.
const NO_MODEL: &str = r#"import json,sys
print(json.dumps({'type':'ready'}),flush=True)
for line in sys.stdin:
    req=json.loads(line)
    print(json.dumps({'id':req['id'],'type':'done','text':'The model was asked.','finish_reason':'stop'}),flush=True)
"#;

async fn shortcut_journey(
    dir: &std::path::Path,
    goal: &str,
    shortcuts: &[&str],
) -> (AppState, TaskRecord, Workflow, Result<(), String>, Vec<Value>) {
    let script = dir.join("shortcut_worker.py");
    std::fs::write(&script, NO_MODEL).unwrap();
    let model = dir.join("model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("config.json"), "{}").unwrap();
    let settings = Settings {
        model_backend: "mlx".into(),
        python_path: "python3".into(),
        model_path: model.to_string_lossy().into(),
        apps: vec!["shortcuts".into()],
        ..Settings::default()
    };
    let mut state = AppState::new(dir.join("data"), dir.join("cache"), dir.into()).unwrap();
    state.models = ModelHub::new(script, PathBuf::new());
    state.store.lock().unwrap().save_settings(&settings).unwrap();
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"shortcut-journey","prompt":goal,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"backend":"mlx"})).unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let offered = chat_tools(&settings, state.models.locality(&settings), false, true);
    let messages = vec![
        json!({"role":"system","content":instructions(&state,&settings,true,"")}),
        json!({"role":"user","content":goal}),
    ];
    let mut flow = Workflow::new(&task, "auto", &settings, messages, None, &offered);
    let names: Vec<String> = shortcuts.iter().map(|name| name.to_string()).collect();
    let reads = std::sync::Mutex::new(Vec::new());
    let result = continue_workflow_using(
        &state,
        &mut task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        &settings,
        &mut flow,
        |tool, args| {
            reads.lock().unwrap().push(json!({"tool":tool,"arguments":args}));
            let evidence = crate::apps::shortcut_evidence(args.get("query").and_then(Value::as_str).unwrap_or(""), &names);
            async move {
                if tool == "list_shortcuts" {
                    Ok(evidence)
                } else {
                    Err("Unexpected app read".into())
                }
            }
        },
    )
    .await;
    let reads = reads.into_inner().unwrap();
    (state, task, flow, result, reads)
}

#[tokio::test]
async fn a_home_device_request_runs_the_users_matching_shortcut_after_approval() {
    if !crate::apps::installed(crate::apps::app("shortcuts").unwrap()) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let goal = "Turn on the living room lights";
    let (state, task, flow, result, reads) = shortcut_journey(
        dir.path(),
        goal,
        &["Lights On", "Living Room Lights On", "Living Room Lights Off"],
    )
    .await;
    result.unwrap();
    assert_eq!(reads, [json!({"tool":"list_shortcuts","arguments":{"query":goal}})], "read once, with the user's words");
    assert_eq!(task.state, "waiting_action", "{}", task.answer);
    let action = task.action.as_ref().unwrap();
    assert_eq!(action["tool"], "run_shortcut");
    assert_eq!(action["fields"][0]["value"], "Living Room Lights On");
    // Its name covers everything the user asked for, so there is nothing to pick.
    assert!(action.get("choices").is_none());
    let run = flow.steps.last().unwrap();
    assert!(run.effect && run.state == "waiting");
    assert_eq!(flow.contract.provenance[&run.id]["name"].origin, "shortcut_record");
    assert_eq!(flow.contract.provenance[&run.id]["name"].source_step_id.as_deref(), Some(flow.steps[0].id.as_str()));
    state.models.shutdown().await;
}

#[tokio::test]
async fn several_fitting_shortcuts_are_offered_and_none_fitting_runs_nothing() {
    if !crate::apps::installed(crate::apps::app("shortcuts").unwrap()) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (state, task, _, result, _) = shortcut_journey(
        dir.path(),
        "Turn on the kitchen lights",
        &["Lights On", "Lights Off", "Kitchen On"],
    )
    .await;
    result.unwrap();
    let action = task.action.as_ref().unwrap();
    assert_eq!(action["fields"][0]["value"], "Lights On");
    let choices: Vec<&str> = action["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|choice| choice["value"].as_str().unwrap())
        .collect();
    assert_eq!(choices, ["Lights On", "Kitchen On"], "only names whose every word the user said");
    state.models.shutdown().await;

    let dir = tempfile::tempdir().unwrap();
    let (state, task, flow, result, _) =
        shortcut_journey(dir.path(), "Lock the front door", &["Lights On", "Unlock Front Door"]).await;
    assert!(result.unwrap_err().contains("None of your shortcuts fits"));
    assert!(task.action.is_none());
    assert!(!flow.steps.iter().any(|step| step.tool == "run_shortcut"));
    state.models.shutdown().await;
}
