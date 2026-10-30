//! "Use Fake ok to echo hello": a request that names a tool the user added goes to that
//! server, waits for the approval sheet unless the user marked the tool safe to read, and
//! answers from its reply. A small Python MCP server stands in for a real one.
use super::*;

/// A model that calls the added tool first, then answers from its result.
const WORKER: &str = r#"import json,sys
print(json.dumps({'type':'ready'}),flush=True)
for line in sys.stdin:
    req=json.loads(line)
    def emit(kind,**fields): print(json.dumps(dict(id=req['id'],type=kind,**fields)),flush=True)
    names=[t['name'] for t in req.get('tools',[])]
    results=[m for m in req['messages'] if m.get('role')=='tool']
    if not results:
        emit('tool_call',name='mcp_fake_ok_echo',arguments={'text':'hello'})
        emit('done',text='',finish_reason='tool_call')
    else:
        emit('done',text='It replied hello.',finish_reason='stop')
"#;

async fn journey(dir: &std::path::Path, goal: &str, safe: bool) -> (AppState, Settings, TaskRecord, Workflow, Result<(), String>) {
    let script = dir.join("mcp_worker.py");
    std::fs::write(&script, WORKER).unwrap();
    let model = dir.join("model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("config.json"), "{}").unwrap();
    let settings = Settings {
        model_backend: "mlx".into(),
        python_path: "python3".into(),
        model_path: model.to_string_lossy().into(),
        ..Settings::default()
    };
    let mut state = AppState::new(dir.join("data"), dir.join("cache"), dir.into()).unwrap();
    state.models = ModelHub::new(script, PathBuf::new());
    state.store.lock().unwrap().save_settings(&settings).unwrap();
    let mut server = crate::mcp::tests::fake_server(dir, "ok");
    crate::mcp::connect(&mut server).await.unwrap();
    if safe {
        server.safe.push("echo".into());
    }
    state.store.lock().unwrap().save_mcp_server(&server).unwrap();
    let mut task: TaskRecord = serde_json::from_value(json!({"id":format!("mcp-{}", server.id),"prompt":goal,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"backend":"mlx"})).unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let offered = chat_tools(&settings, state.models.locality(&settings), false, true);
    let messages = vec![
        json!({"role":"system","content":instructions(&state,&settings,true,"")}),
        json!({"role":"user","content":goal}),
    ];
    let mut flow = Workflow::new(&task, "auto", &settings, messages, None, &offered);
    let servers = state.store.lock().unwrap().mcp_servers().unwrap();
    assert!(crate::mcp::route(&mut flow.contract, &servers, goal, state.models.locality(&settings), &settings).unwrap());
    let result = continue_workflow_using(
        &state,
        &mut task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        &settings,
        &mut flow,
        |_, _| async { Err("No app read expected".into()) },
    )
    .await;
    (state, settings, task, flow, result)
}

#[tokio::test]
async fn a_call_to_an_added_tool_waits_for_approval_then_answers_from_its_reply() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _, task, flow, result) = journey(dir.path(), "Use Fake ok to echo hello", false).await;
    result.unwrap();
    assert_eq!(task.state, "waiting_action", "{}", task.answer);
    let step = flow.steps.last().unwrap();
    assert!(step.effect, "an added tool's call is a write");
    assert_eq!(step.arguments, json!({"text":"hello"}));
    let proposal = task.action.clone().unwrap();
    assert_eq!(proposal["tool"], "mcp_fake_ok_echo");
    assert_eq!(proposal["title"], "Let Fake ok run echo?");
    assert!(proposal["fields"].as_array().unwrap().iter().all(|field| field["kind"] == "fixed"));
    assert!(task.touched.contains(&"tools".to_string()));
    // The sheet sends nothing back; the call runs exactly as shown.
    let done = run_workflow_action(&state, &step.id, json!({"text":"changed"}), &Channel::new(|_| Ok(())))
        .await
        .unwrap();
    assert_eq!(done.state, "completed", "{}", done.answer);
    assert!(done.answer.starts_with("Fake ok ran echo and replied. Buntini can’t check what it changed."), "{}", done.answer);
    assert!(done.answer.contains("It replied hello."));
    let saved = state.store.lock().unwrap().workflow(&task.id).unwrap().unwrap();
    let receipt = &saved.steps[0].evidence.as_ref().unwrap()["receipt"];
    assert_eq!(receipt["evidence"]["reply"], "hello");
    assert_eq!(saved.steps[0].arguments, json!({"text":"hello"}));
    // Approving again never replays the call.
    let again = run_workflow_action(&state, &step.id, json!({}), &Channel::new(|_| Ok(()))).await.unwrap();
    assert_eq!(again.state, "completed");
    crate::mcp::stop(task.id.trim_start_matches("mcp-"));
}

#[tokio::test]
async fn a_tool_the_user_marked_safe_to_read_runs_without_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let (_, _, task, flow, result) = journey(dir.path(), "Ask Fake ok to echo hello", true).await;
    result.unwrap();
    assert_eq!(task.state, "completed", "{}", task.answer);
    assert!(!flow.steps[0].effect);
    assert!(task.action.is_none());
    assert!(task.answer.contains("It replied hello."));
    crate::mcp::stop(task.id.trim_start_matches("mcp-"));
}

#[tokio::test]
async fn a_safe_tool_still_asks_when_its_details_are_not_the_users_own_words() {
    let dir = tempfile::tempdir().unwrap();
    // The model fills in "hello", which the request never said: it might have come from
    // the screen or an email, so the sheet shows it before it leaves.
    let (_, _, task, flow, result) = journey(dir.path(), "Ask Fake ok to echo what I said earlier", true).await;
    result.unwrap();
    assert_eq!(task.state, "waiting_action", "{}", task.answer);
    let step = flow.steps.last().unwrap();
    assert!(step.effect);
    assert_eq!(step.evidence.as_ref().unwrap()["addedTool"]["tool"], "echo", "the sheet's tool is kept with it");
    crate::mcp::stop(task.id.trim_start_matches("mcp-"));
}

#[tokio::test]
async fn external_brains_need_the_tools_share_and_unnamed_requests_never_reach_added_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = crate::mcp::tests::fake_server(dir.path(), "ok");
    crate::mcp::connect(&mut server).await.unwrap();
    crate::mcp::stop(&server.id);
    let servers = vec![server];
    let mut settings = Settings::default();
    let mut contract = crate::task_contract::TaskContract::from_request("Use Fake ok to echo hello");
    assert_eq!(
        crate::mcp::route(&mut contract, &servers, "Use Fake ok to echo hello", privacy::Locality::External, &settings).unwrap_err(),
        crate::mcp::WITHHELD
    );
    settings.external_shares = vec!["tools".into()];
    assert!(crate::mcp::route(&mut contract, &servers, "Use Fake ok to echo hello", privacy::Locality::External, &settings).unwrap());
    assert_eq!(contract.outcomes.len(), 2);
    assert_eq!(contract.outcomes[0].kind, "write");
    // Without naming the tool, its definitions are never offered, even to answer.
    let plain = crate::task_contract::TaskContract::from_request("echo hello");
    let offered = Value::Array(crate::mcp::definitions(&servers, &crate::mcp::routed_tools(&servers.iter().collect::<Vec<_>>())));
    assert!(plain.relevant_tools(&[], &offered).as_array().unwrap().is_empty());
    assert!(plain.validate_proposal("mcp_fake_ok_echo", &json!({"text":"hello"}), &[]).is_err());
}
