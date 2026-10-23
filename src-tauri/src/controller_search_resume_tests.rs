//! Trusted clarification changes a pending query, not historical receipts or
//! permission to skip the replacement read. No model, browser or app executes.
use super::*;

fn failed_lookup(goal: &str) -> (TaskRecord, Workflow) {
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"revised-research","prompt":goal,"answer":"Lookup failed","state":"failed","createdAt":now(),"sources":[],"files":[]})).unwrap();
    let mut flow = Workflow::new(
        &task,
        "auto",
        &Settings::default(),
        vec![],
        None,
        &["look_up_online", "create_note"],
    );
    if flow
        .contract
        .next_outcome()
        .is_some_and(|outcome| outcome.tool.as_deref() == Some("create_note"))
    {
        let args = json!({"title":"Saved","body":"Keep this receipt"});
        let id = flow
            .begin("create_note", args.clone(), json!({}), true, true)
            .unwrap();
        flow.approve(&id, args).unwrap();
        flow.finish_step("succeeded",json!({"receipt":{"outcome":"succeeded","tool":"create_note","objectId":"note-before-search","destination":"Notes","message":"Created the note.","evidence":{},"error":null}})).unwrap();
    }
    let args = flow
        .contract
        .next_outcome()
        .unwrap()
        .required_arguments
        .clone();
    let id = flow
        .begin("look_up_online", args.clone(), json!({}), false, true)
        .unwrap();
    flow.approve(
        &id,
        json!({"query":args["query"],"approvedTool":"web_search","approvedArguments":args}),
    )
    .unwrap();
    flow.finish_step("failed",json!({"error":"Temporary source failure","partialSources":["https://rail.example.org/fares"]})).unwrap();
    flow.state = "failed".into();
    task.state = "failed".into();
    flow.refresh_contract();
    assert_eq!(flow.contract.next_outcome().unwrap().status, "blocked");
    (task, flow)
}

#[test]
fn revised_query_retires_only_failed_observation_and_survives_reopen() {
    let (mut task, mut flow) = failed_lookup("Search the web for train fares");
    let original = flow.steps[0].clone();
    apply_resume_guidance(&mut flow, &mut task, "Use official rail operators only");
    assert_eq!(flow.steps[0].state, "superseded");
    assert_eq!(flow.steps[0].evidence, original.evidence);
    assert_eq!(flow.steps[0].arguments, original.arguments);
    assert_eq!(flow.steps[0].approved_at, original.approved_at);
    assert_eq!(flow.steps[0].finished_at, original.finished_at);
    let next = flow.contract.next_outcome().unwrap();
    assert_eq!(next.status, "ready");
    assert_eq!(next.tool.as_deref(), Some("look_up_online"));
    assert_ne!(
        next.required_arguments["query"],
        original.arguments["query"]
    );
    assert!(next.required_arguments["query"]
        .as_str()
        .unwrap()
        .contains("Use official rail operators only"));
    assert!(flow
        .contract
        .validate_answer("Finished", &flow.steps)
        .is_err());
    assert!(flow
        .contract
        .validate_proposal(
            "create_note",
            &json!({"title":"Premature","body":"Unverified"}),
            &flow.steps
        )
        .is_err());
    let args = next.required_arguments.clone();
    assert!(flow
        .contract
        .validate_proposal("look_up_online", &args, &flow.steps)
        .is_ok());

    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(&dir.path().join("test.db")).unwrap();
    flow.state = "running".into();
    task.state = "running".into();
    store.checkpoint(&mut flow, &task).unwrap();
    drop(store);
    let store = crate::store::Store::open(&dir.path().join("test.db")).unwrap();
    let mut reopened = store.workflow(&task.id).unwrap().unwrap();
    assert_eq!(reopened.steps[0].state, "superseded");
    assert_eq!(reopened.steps[0].evidence, original.evidence);
    let id = reopened
        .begin("look_up_online", args.clone(), json!({}), false, true)
        .unwrap();
    assert_eq!(
        reopened.state, "waiting_online",
        "The changed query needs fresh approval"
    );
    assert!(reopened
        .contract
        .validate_answer("Finished", &reopened.steps)
        .is_err());
    reopened
        .approve(
            &id,
            json!({"query":args["query"],"approvedTool":"web_search","approvedArguments":args}),
        )
        .unwrap();
    reopened.finish_step("succeeded",json!({"sources":[{"url":"https://rail.example.org/fares"}],"cards":[],"pages":[{"url":"https://rail.example.org/fares","text":"Published train fares"}],"summary":"Published fares [1]","researchReport":{"evidenceReady":true,"answerVerified":true}})).unwrap();
    assert_eq!(reopened.contract.next_outcome().unwrap().kind, "answer");
    assert!(reopened
        .contract
        .validate_answer("Published train fares [1].", &reopened.steps)
        .is_ok());
}

#[test]
fn query_revision_preserves_prior_verified_write_and_unrelated_failures() {
    let (mut task, mut flow) = failed_lookup("Create a note, then search the web for train fares");
    let note = flow.steps[0].clone();
    let mut unrelated = flow.steps[1].clone();
    unrelated.id = "different-query".into();
    unrelated.arguments["query"] = json!("different public question");
    flow.steps.push(unrelated.clone());
    flow.refresh_contract();
    apply_resume_guidance(&mut flow, &mut task, "Use official rail operators only");
    assert_eq!(flow.steps[0].state, "succeeded");
    assert_eq!(flow.steps[0].evidence, note.evidence);
    assert_eq!(flow.steps[0].arguments, note.arguments);
    assert_eq!(flow.steps[1].state, "superseded");
    assert_eq!(flow.steps[2].state, "failed");
    assert_eq!(flow.steps[2].evidence, unrelated.evidence);
    assert_eq!(flow.contract.outcomes[0].status, "complete");
    assert!(flow
        .contract
        .validate_proposal("create_note", &note.arguments, &flow.steps)
        .is_err());
}

#[test]
fn unchanged_query_does_not_supersede_failure_or_unlock_follow_up_write() {
    let (mut task, mut flow) = failed_lookup("Search the web for train fares, then create a note");
    let original = flow.steps[0].clone();
    apply_resume_guidance(&mut flow, &mut task, "The note should be concise");
    assert_eq!(flow.steps[0].state, "failed");
    assert_eq!(flow.steps[0].evidence, original.evidence);
    assert_eq!(
        flow.contract.next_outcome().unwrap().required_arguments["query"],
        original.arguments["query"]
    );
    assert!(flow
        .contract
        .validate_proposal(
            "create_note",
            &json!({"title":"Fares","body":"Unverified"}),
            &flow.steps
        )
        .is_err());
}

#[test]
fn guidance_never_rewrites_unknown_running_waiting_or_failed_writes() {
    for state in ["unknown", "running", "waiting", "succeeded"] {
        let (mut task, mut flow) = failed_lookup("Search the web for train fares");
        let mut preserved = flow.steps[0].clone();
        preserved.id = format!("preserved-{state}");
        preserved.state = state.into();
        flow.steps.insert(0, preserved.clone());
        let mut failed_write = preserved.clone();
        failed_write.id = "failed-effect".into();
        failed_write.effect = true;
        failed_write.state = "failed".into();
        flow.steps.insert(0, failed_write.clone());
        flow.refresh_contract();
        assert!(
            !flow.can_resume(),
            "The command blocks these histories before applying guidance"
        );
        apply_resume_guidance(&mut flow, &mut task, "Use official rail operators only");
        assert_eq!(flow.steps[0].state, failed_write.state);
        assert_eq!(flow.steps[0].evidence, failed_write.evidence);
        assert_eq!(flow.steps[1].state, preserved.state);
        assert_eq!(flow.steps[1].evidence, preserved.evidence);
    }
}
