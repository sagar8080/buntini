//! Opt-in Apple workflows over real, uniquely namespaced Notes records and
//! production Notes/Reminders/Calendar writes. No personal note query is allowed.
use super::apple_live_tests::{live_state, record_live_case, LiveTurn};
use super::*;
use std::time::Instant;

fn owner_task_deadline(answer: &str, owner: &str, task: &str, deadline: &str) -> bool {
    let matches =
        |text: &str| text.contains(owner) && text.contains(task) && text.contains(deadline);
    answer.lines().any(matches)
        || answer.split("\n\n").any(|block| {
            matches(block)
                && ["alex", "mina"]
                    .into_iter()
                    .filter(|other| *other != owner)
                    .all(|other| !block.contains(other))
        })
}

fn recommends_quote_b(answer: &str) -> bool {
    let text = answer.to_lowercase().replace(['*', '_', '`'], "");
    text.split(['\n', '.', '!', '?']).any(|clause| {
        (clause.contains("quote b") || clause.contains("option b"))
            && [
                "recommend",
                "choose",
                "select",
                "pick",
                "prefer",
                "best",
                "go with",
            ]
            .iter()
            .any(|verb| clause.contains(verb))
            && !["do not", "don't", "not recommend", "avoid"]
                .iter()
                .any(|negation| clause.contains(negation))
    })
}

#[test]
fn comparison_checker_requires_the_rule_selected_winner_not_a_source_mention() {
    assert!(!recommends_quote_b(
        "Quote B costs $120 and is ready Wednesday. I recommend Quote A at $115."
    ));
    assert!(recommends_quote_b(
        "I recommend Quote B: it costs only $5 more and is ready Wednesday rather than Friday."
    ));
    assert!(!recommends_quote_b("Do not choose Quote B."));
}

#[test]
fn handoff_associations_accept_nested_format_but_reject_swapped_deadlines() {
    let nested = "1. Alex\n - Task: Landing-page draft\n - Deadline: Monday\n\n2. Mina\n - Task: Accessibility checklist review\n - Deadline: Tuesday".to_lowercase();
    assert!(owner_task_deadline(&nested, "alex", "draft", "monday"));
    assert!(owner_task_deadline(
        &nested,
        "mina",
        "accessibility",
        "tuesday"
    ));
    let swapped = nested
        .replace("monday", "temporary")
        .replace("tuesday", "monday")
        .replace("temporary", "tuesday");
    assert!(!owner_task_deadline(&swapped, "alex", "draft", "monday"));
    assert!(!owner_task_deadline(
        &swapped,
        "mina",
        "accessibility",
        "tuesday"
    ));
}

async fn read_owned_notes(
    state: &AppState,
    namespace: &str,
    tool: String,
    args: serde_json::Map<String, Value>,
    settings: &Settings,
) -> Result<Value, String> {
    if tool != "find_notes"
        || !args
            .get("query")
            .and_then(Value::as_str)
            .is_some_and(|query| query.to_lowercase().contains(&namespace.to_lowercase()))
    {
        return Err(
            "Real-app acceptance refused a read outside its exact test-note namespace.".into(),
        );
    }
    crate::apps::run_evidence(&tool, &args, state.models.apple_helper(), settings, crate::apps::Launch::for_request()).await
}

async fn initial_turn(state: &AppState, prompt: &str, namespace: &str) -> LiveTurn {
    let started = Instant::now();
    let before = state.models.apple.model_calls();
    let settings = state.store.lock().unwrap().settings().unwrap();
    let mut task: TaskRecord = serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"prompt":prompt,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"backend":"apple"})).unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let offered = chat_tools(&settings, state.models.locality(&settings), false, true);
    let messages = vec![
        json!({"role":"system","content":instructions(state,&settings,false,"")}),
        json!({"role":"user","content":prompt}),
    ];
    let mut flow = Workflow::new(&task, "auto", &settings, messages, None, &offered);
    let channel = Channel::new(|_| Ok(()));
    let cancel = Arc::new(AtomicBool::new(false));
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(150),
        continue_workflow_using(
            state,
            &mut task,
            &channel,
            cancel.clone(),
            &settings,
            &mut flow,
            |tool, args| read_owned_notes(state, namespace, tool, args, &settings),
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            cancel.store(true, Ordering::Relaxed);
            state.models.reset().await;
            Err("Real-app workflow exceeded 150 seconds.".into())
        }
    };
    let error = result.as_ref().err().cloned();
    finish_workflow(state, &mut flow, &mut task, result, false).unwrap();
    LiveTurn {
        task,
        workflow: Some(flow),
        apple_calls: state.models.apple.model_calls() - before,
        error,
        duration_ms: started.elapsed().as_millis(),
    }
}

fn proposal_matches(case: &Value, namespace: &str, tool: &str, args: &Value) -> bool {
    if case["writeTool"] != tool
        || args["title"] != case["writeTitle"]
        || !args["title"]
            .as_str()
            .is_some_and(|title| title.starts_with(namespace))
    {
        return false;
    }
    if tool == "create_note" {
        let text = args["body"].as_str().unwrap_or("").to_lowercase();
        if let Some(body) = case["expectedWrite"]["body"].as_str() {
            return args["body"] == body;
        }
        if case["bodyMustBeChecklist"] == true
            && text
                .lines()
                .filter(|line| {
                    let line = line.trim();
                    ["- ", "* ", "1.", "2.", "[ ]", "☐"]
                        .iter()
                        .any(|prefix| line.starts_with(prefix))
                })
                .count()
                < 2
        {
            return false;
        }
        if case["id"] == "create-meeting-followup-note"
            && !(text
                .lines()
                .any(|line| line.contains("mina") && line.contains("accessibility"))
                && text.lines().any(|line| {
                    line.contains("alex") && line.contains("screenshots") && line.contains("monday")
                }))
        {
            return false;
        }
        if case["id"] == "notes-project-next-step"
            && !text
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| ["me", "myself", "you", "i", "user", "buntini"].contains(&word))
        {
            return false;
        }
        return case["writeFacts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|fact| text.contains(&fact.as_str().unwrap().to_lowercase()));
    }
    let fields = if tool == "add_reminder" {
        vec!["due"]
    } else {
        vec!["start", "minutes", "location"]
    };
    fields
        .into_iter()
        .all(|key| args[key] == case["expectedWrite"][key])
}

async fn approve_turn(
    state: &AppState,
    turn: &LiveTurn,
    namespace: &str,
    case: &Value,
) -> LiveTurn {
    let started = Instant::now();
    let before = state.models.apple.model_calls();
    let Some(step) = turn.workflow.as_ref().and_then(|flow| flow.steps.last()) else {
        return LiveTurn {
            task: turn.task.clone(),
            workflow: turn.workflow.clone(),
            apple_calls: 0,
            error: Some("No durable action proposal.".into()),
            duration_ms: 0,
        };
    };
    if !proposal_matches(case, namespace, &step.tool, &step.arguments) {
        return LiveTurn{task:turn.task.clone(),workflow:turn.workflow.clone(),apple_calls:0,error:Some("Generated action failed the exact title/date/content acceptance gate; no write was approved.".into()),duration_ms:0};
    }
    let expected = step.arguments.clone();
    let channel = Channel::new(|_| Ok(()));
    let result = run_workflow_action_with_reads(
        state,
        &step.id,
        expected.clone(),
        &channel,
        |tool, args, _cancel| async move {
            let raw = crate::acceptance_apps::execute_exact(
                namespace,
                &tool,
                &Value::Object(args),
                &expected,
            )
            .await;
            serde_json::from_value(raw).expect("Production action receipt")
        },
        |tool, args, settings| async move {
            read_owned_notes(state, namespace, tool, args, &settings).await
        },
    )
    .await;
    let error = result.as_ref().err().cloned();
    let task = result.unwrap_or_else(|_| {
        state
            .store
            .lock()
            .unwrap()
            .task(&turn.task.id)
            .unwrap()
            .unwrap()
    });
    let workflow = state.store.lock().unwrap().workflow(&task.id).unwrap();
    LiveTurn {
        task,
        workflow,
        apple_calls: state.models.apple.model_calls() - before,
        error,
        duration_ms: started.elapsed().as_millis(),
    }
}

fn cases(namespace: &str) -> Vec<Value> {
    let mut source = crate::acceptance_apps::corpus(namespace)
        .as_array()
        .unwrap()
        .clone();
    let mut result = vec![
        source[0].clone(),
        source[2].clone(),
        source[3].clone(),
        source[4].clone(),
    ];
    result[3]["answerMustContain"] = json!(["42"]);
    result[3]["answerAnyOf"] = json!(["coffee", "☕"]);
    let title = format!("{namespace} Launch checklist");
    source[5]["prompt"]=json!(format!("Read my note mentioning {namespace} Project next step, then create a new note titled {title} that summarizes the unfinished task, prerequisite and owner as a short checklist. Do not change DNS or send anything."));
    source[5]
        .as_object_mut()
        .unwrap()
        .remove("answerMustContain");
    source[5]["writeTool"] = json!("create_note");
    source[5]["writeTitle"] = json!(title);
    source[5]["writeFacts"] = json!(["DNS", "backup"]);
    source[5]["bodyMustBeChecklist"] = json!(true);
    result.push(source[5].clone());
    let title = format!("{namespace} Leave for station");
    source[1]["prompt"]=json!(format!("Read my note mentioning {namespace} Travel. On May 20, 2031, remind me to leave for the station 15 minutes before my stated station-arrival time. Use exactly the reminder title {title}."));
    source[1]
        .as_object_mut()
        .unwrap()
        .remove("answerMustContain");
    source[1]["writeTool"] = json!("add_reminder");
    source[1]["writeTitle"] = json!(title);
    source[1]["expectedWrite"] = json!({"due":"2031-05-20 07:25"});
    result.push(source[1].clone());
    let seed_title = format!("{namespace} Workshop details");
    let write_title = format!("{namespace} Workshop");
    result.push(json!({"id":"notes-to-calendar-workshop","seeds":[{"tool":"create_note","arguments":{"title":seed_title,"body":"The workshop is on May 20, 2031. It starts at 13:20 and ends at 14:05 in Study Room. There are no attendees to invite."}}],
        "readQuery":seed_title,"prompt":format!("Read my note mentioning {namespace} Workshop details, then add that workshop to my calendar using exactly the title {write_title}. Derive the duration from the note. Do not invite anyone."),"writeTool":"add_calendar_event","writeTitle":write_title,"expectedWrite":{"start":"2031-05-20 13:20","minutes":45,"location":"Study Room"}}));
    let title = format!("{namespace} Meeting follow-up");
    result.push(json!({"id":"create-meeting-followup-note","seeds":[],"readRequired":false,"prompt":format!("Turn these rough meeting follow-ups into a two-item checklist and save it in a new note titled \"{title}\": accessibility review belongs to Mina; screenshots are Alex's responsibility and due Monday. Do not send any messages."),"writeTool":"create_note","writeTitle":title,"bodyMustBeChecklist":true,"writeFacts":["Mina","accessibility","Alex","screenshots","Monday"]}));
    let title = format!("{namespace} Return library books");
    result.push(json!({"id":"create-library-return-reminder","seeds":[],"readRequired":false,"prompt":format!("My library books must be returned by 18:00 on May 21, 2031. Set a reminder fifteen minutes beforehand, using exactly the title \"{title}\"."),"writeTool":"add_reminder","writeTitle":title,"expectedWrite":{"due":"2031-05-21 17:45"}}));
    let title = format!("{namespace} Training session");
    result.push(json!({"id":"create-training-calendar-event","seeds":[],"readRequired":false,"prompt":format!("My training session runs from 14:15 to 14:50 on May 21, 2031 in Training Room. Add it to my calendar with exactly the title \"{title}\". Derive its duration from those times, and do not invite anyone."),"writeTool":"add_calendar_event","writeTitle":title,"expectedWrite":{"start":"2031-05-21 14:15","minutes":35,"location":"Training Room"}}));
    result
}

pub(super) async fn app_workflows() -> Vec<Value> {
    let mut results = Vec::new();
    for index in 0..cases("BuntiniQA-case-count").len() {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let namespace = format!("BuntiniQA-{}", &nonce[..16]);
        let case = cases(&namespace).remove(index);
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let mut settings = state.store.lock().unwrap().settings().unwrap();
        settings.apps = vec!["notes".into(), "reminders".into(), "calendar".into()];
        state
            .store
            .lock()
            .unwrap()
            .save_settings(&settings)
            .unwrap();
        let mut seeds = Vec::new();
        let mut cleanup = Vec::new();
        let mut seed_ok = true;
        for seed in case["seeds"].as_array().unwrap() {
            let tool = seed["tool"].as_str().unwrap();
            let args = &seed["arguments"];
            let receipt = crate::acceptance_apps::execute_exact(&namespace, tool, args, args).await;
            seed_ok &= receipt["outcome"] == "succeeded";
            seeds.push(json!({"tool":tool,"arguments":args,"receipt":receipt}));
        }
        let mut first = None;
        let mut final_turn = None;
        if seed_ok {
            let initial = initial_turn(&state, case["prompt"].as_str().unwrap(), &namespace).await;
            if initial.task.state == "waiting_action" && case.get("writeTool").is_some() {
                final_turn = Some(approve_turn(&state, &initial, &namespace, &case).await);
            }
            first = Some(initial);
        }
        let used = final_turn.as_ref().or(first.as_ref());
        if let Some(turn) = used {
            if let Some(flow) = &turn.workflow {
                for step in flow.steps.iter().filter(|step| step.effect) {
                    if let Some(receipt) = step
                        .evidence
                        .as_ref()
                        .and_then(|value| value.get("receipt"))
                        .filter(|receipt| receipt["objectId"].is_string())
                    {
                        cleanup.push(
                            crate::acceptance_apps::cleanup_receipt(
                                &namespace,
                                receipt,
                                &step.arguments,
                            )
                            .await,
                        );
                    }
                }
            }
        }
        for seed in &seeds {
            if seed["receipt"]["objectId"].is_string() {
                cleanup.push(
                    crate::acceptance_apps::cleanup_receipt(
                        &namespace,
                        &seed["receipt"],
                        &seed["arguments"],
                    )
                    .await,
                );
            }
        }
        let apple_calls = first.as_ref().map(|turn| turn.apple_calls).unwrap_or(0)
            + final_turn
                .as_ref()
                .map(|turn| turn.apple_calls)
                .unwrap_or(0);
        let actual_read = used
            .and_then(|turn| turn.workflow.as_ref())
            .is_some_and(|flow| {
                flow.steps.iter().any(|step| {
                    step.tool == "find_notes"
                        && step.state == "succeeded"
                        && step
                            .evidence
                            .as_ref()
                            .and_then(|value| value["records"].as_array())
                            .is_some_and(|records| {
                                records.iter().any(|record| {
                                    seeds
                                        .iter()
                                        .any(|seed| seed["receipt"]["objectId"] == record["id"])
                                })
                            })
                        && step.arguments["query"].as_str().is_some_and(|query| {
                            query.to_lowercase().contains(&namespace.to_lowercase())
                        })
                })
            });
        let mut facts = case
            .get("answerMustContain")
            .and_then(Value::as_array)
            .map(|facts| {
                let answer = used
                    .map(|turn| turn.task.answer.to_lowercase())
                    .unwrap_or_default();
                facts
                    .iter()
                    .all(|fact| answer.contains(&fact.as_str().unwrap().to_lowercase()))
            })
            .unwrap_or(true);
        if let Some(alternatives) = case["answerAnyOf"].as_array() {
            let answer = used
                .map(|turn| turn.task.answer.to_lowercase())
                .unwrap_or_default();
            facts &= alternatives
                .iter()
                .any(|word| answer.contains(&word.as_str().unwrap().to_lowercase()));
        }
        if case["id"] == "notes-handoff-summary" {
            let answer = used
                .map(|turn| turn.task.answer.to_lowercase())
                .unwrap_or_default();
            facts &= owner_task_deadline(&answer, "alex", "draft", "monday")
                && owner_task_deadline(&answer, "mina", "accessibility", "tuesday");
        }
        if case["id"] == "notes-compare-quotes" {
            facts &= used.is_some_and(|turn| recommends_quote_b(&turn.task.answer));
        }
        let actual_write = case.get("writeTool").is_none()
            || used
                .and_then(|turn| turn.workflow.as_ref())
                .is_some_and(|flow| {
                    flow.steps.iter().any(|step| {
                        step.tool == case["writeTool"].as_str().unwrap()
                            && step.state == "succeeded"
                            && step
                                .evidence
                                .as_ref()
                                .is_some_and(|value| value["receipt"]["objectId"].is_string())
                    })
                });
        let completed =
            used.is_some_and(|turn| turn.task.state == "completed" && turn.error.is_none());
        let clean =
            cleanup.len() >= seeds.len() && cleanup.iter().all(|value| value["passed"] == true);
        let read_required = case["readRequired"] != false;
        let passed = seed_ok
            && completed
            && apple_calls > 0
            && (!read_required || actual_read)
            && actual_write
            && facts
            && clean;
        let helper_hash = std::fs::read(state.models.apple_helper())
            .ok()
            .map(|bytes| crate::security::sha256_hex(bytes));
        let row = json!({"id":case["id"],"kind":"real_app_workflow","passed":passed,"apple_calls":apple_calls,"namespace":namespace,"request":case["prompt"],"checks":{"seedsVerifiedInRealApp":seed_ok,"completed":completed,"namespacedReadRequired":read_required,"actualNamespacedNoteRead":actual_read,"actualRequiredWrite":actual_write,"answerFacts":facts,"exactRecordCleanup":clean},"initial":first,"approvedContinuation":final_turn,"realSeeds":seeds,"cleanup":cleanup,"helper_sha256":helper_hash,"source_kind":"real Notes records read through production app integration when requested; namespaced writes through production executor","notes_cleanup":"Removed only from original active folder; Notes may retain Recently Deleted copies; no trash purge."});
        results.push(record_live_case(row));
        state.models.shutdown().await;
    }
    results
}
