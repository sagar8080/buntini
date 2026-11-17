//! Changing existing items in controlled journeys. Fixtures supply every read and the
//! write receipt; nothing is dispatched to a personal app, and no model is asked.
use super::*;

fn journey(dir: &std::path::Path, goal: &str) -> (AppState, Settings, TaskRecord, Workflow) {
    let script = dir.join("edit_worker.py");
    std::fs::write(
        &script,
        "raise RuntimeError('Changes to existing items bind without inference')",
    )
    .unwrap();
    let model = dir.join("model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("config.json"), "{}").unwrap();
    let settings = Settings {
        model_backend: "mlx".into(),
        python_path: "python3".into(),
        model_path: model.to_string_lossy().into(),
        apps: vec!["notes".into(), "reminders".into(), "calendar".into()],
        ..Settings::default()
    };
    let mut state = AppState::new(dir.join("data"), dir.join("cache"), dir.into()).unwrap();
    state.models = ModelHub::new(script, PathBuf::new());
    state
        .store
        .lock()
        .unwrap()
        .save_settings(&settings)
        .unwrap();
    let task: TaskRecord = serde_json::from_value(json!({"id":"edit-journey","prompt":goal,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"backend":"mlx"})).unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let offered = chat_tools(&settings, state.models.locality(&settings), false, true);
    let messages = vec![
        json!({"role":"system","content":instructions(&state,&settings,true,"")}),
        json!({"role":"user","content":goal}),
    ];
    let flow = Workflow::new(&task, "auto", &settings, messages, None, &offered);
    (state, settings, task, flow)
}

fn open(id: &str, title: &str) -> crate::apps::Reminder {
    crate::apps::Reminder {
        id: Some(id.into()),
        title: title.into(),
        list: "Home".into(),
        ..Default::default()
    }
}

fn receipt(tool: String, args: serde_json::Map<String, Value>) -> crate::actions::ActionReceipt {
    crate::actions::ActionReceipt {
        outcome: crate::actions::ActionOutcome::Succeeded,
        tool,
        object_id: args.get("id").and_then(Value::as_str).map(str::to_owned),
        destination: Some("Controlled fixture".into()),
        message: "Verified fixture change.".into(),
        evidence: json!({"readBack":args}),
        error: None,
    }
}

async fn plan_with(
    state: &AppState,
    settings: &Settings,
    task: &mut TaskRecord,
    flow: &mut Workflow,
    reminders: Vec<crate::apps::Reminder>,
) -> Vec<String> {
    let reads = std::sync::Mutex::new(Vec::new());
    let found = crate::apps::Agenda {
        total: reminders.len(),
        items: reminders,
    };
    continue_workflow_using(
        state,
        task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        settings,
        flow,
        |tool, _| {
            reads.lock().unwrap().push(tool.clone());
            let evidence = crate::apps::reminder_evidence(&found);
            async move {
                if tool == "read_reminders" {
                    Ok(evidence)
                } else {
                    Err("Unexpected app read".into())
                }
            }
        },
    )
    .await
    .unwrap();
    reads.into_inner().unwrap()
}

#[tokio::test]
async fn one_matching_reminder_is_bound_from_the_read_and_shown_as_it_is_now() {
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, mut flow) = journey(dir.path(), "Mark buy milk as done");
    let reads = plan_with(
        &state,
        &settings,
        &mut task,
        &mut flow,
        vec![open("r-1", "Buy milk"), open("r-2", "Call mum")],
    )
    .await;
    assert_eq!(reads, ["read_reminders"]);
    assert_eq!(task.state, "waiting_action");
    let action = task.action.as_ref().unwrap();
    assert_eq!(action["tool"], "complete_reminder");
    assert_eq!(action["fields"][0]["key"], "now");
    assert_eq!(action["fields"][0]["kind"], "fixed");
    assert!(action["fields"][0]["value"]
        .as_str()
        .unwrap()
        .starts_with("Buy milk"));
    assert!(action.get("choices").is_none());
    assert!(task.touched.iter().any(|share| share == "calendar"));
    let proposal = flow.steps.last().unwrap();
    assert_eq!(proposal.arguments, json!({"id":"r-1","title":"Buy milk"}));
    assert_eq!(
        flow.contract.provenance[&proposal.id]["id"].origin,
        "source_record"
    );
    assert_eq!(
        flow.contract.provenance[&proposal.id]["id"]
            .source_step_id
            .as_deref(),
        Some(flow.steps[0].id.as_str())
    );
    // Approving runs exactly that change and finishes from its receipt.
    run_workflow_action_with_reads(
        &state,
        &proposal.id,
        proposal.arguments.clone(),
        &Channel::new(|_| Ok(())),
        |tool, args, _| async move { receipt(tool, args) },
        |_, _, _| async { Err("Unexpected app read".into()) },
    )
    .await
    .unwrap();
    let saved = state.store.lock().unwrap().task(&task.id).unwrap().unwrap();
    assert_eq!(saved.state, "completed");
    assert_eq!(saved.answer, "Verified fixture change.");
    state.models.shutdown().await;
}

#[tokio::test]
async fn several_matching_reminders_are_offered_and_only_a_read_one_can_run() {
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, mut flow) = journey(dir.path(), "Tick off milk");
    plan_with(
        &state,
        &settings,
        &mut task,
        &mut flow,
        vec![open("r-1", "Buy milk"), open("r-2", "Buy milk powder")],
    )
    .await;
    let action = task.action.as_ref().unwrap();
    let choices = action["choices"].as_array().unwrap();
    assert_eq!(
        choices
            .iter()
            .map(|c| c["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["r-1", "r-2"]
    );
    assert!(choices.iter().all(|c| c["field"] == "id"));
    assert_eq!(choices[1]["also"]["title"], "Buy milk powder");
    let proposal = flow.steps.last().unwrap().clone();
    assert!(
        proposal.arguments.get("id").is_none(),
        "nothing is picked for the user"
    );
    let never =
        |_: String, _: serde_json::Map<String, Value>, _| async { panic!("nothing may run") };
    for (args, error) in [
        (json!({"title":"Buy milk"}), "Pick which one"),
        (
            json!({"id":"r-9","title":"Buy milk"}),
            "only an item it read",
        ),
        (
            json!({"id":"r-1","title":"Buy milk powder"}),
            "only an item it read",
        ),
    ] {
        let refused = run_workflow_action_with_reads(
            &state,
            &proposal.id,
            args,
            &Channel::new(|_| Ok(())),
            never,
            |_, _, _| async { Err("Unexpected app read".into()) },
        )
        .await;
        assert!(refused.unwrap_err().contains(error));
    }
    run_workflow_action_with_reads(
        &state,
        &proposal.id,
        json!({"id":"r-2","title":"Buy milk powder"}),
        &Channel::new(|_| Ok(())),
        |tool, args, _| async move { receipt(tool, args) },
        |_, _, _| async { Err("Unexpected app read".into()) },
    )
    .await
    .unwrap();
    let saved = state
        .store
        .lock()
        .unwrap()
        .workflow(&task.id)
        .unwrap()
        .unwrap();
    let step = saved.steps.last().unwrap();
    assert_eq!(
        (step.tool.as_str(), step.state.as_str()),
        ("complete_reminder", "succeeded")
    );
    assert_eq!(step.arguments["id"], "r-2");
    state.models.shutdown().await;
}

#[tokio::test]
async fn no_matching_reminder_asks_instead_of_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, mut flow) = journey(dir.path(), "Mark the plumber done");
    let found = crate::apps::Agenda {
        total: 1,
        items: vec![open("r-1", "Buy milk")],
    };
    let result = continue_workflow_using(
        &state,
        &mut task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        &settings,
        &mut flow,
        |_, _| {
            let evidence = crate::apps::reminder_evidence(&found);
            async move { Ok(evidence) }
        },
    )
    .await;
    assert!(result
        .unwrap_err()
        .contains("couldn’t find an open reminder"));
    assert!(
        flow.steps.iter().all(|step| !step.effect),
        "no change was proposed"
    );
    state.models.shutdown().await;
}

#[tokio::test]
async fn an_event_named_without_a_day_is_looked_for_day_by_day() {
    let dir = tempfile::tempdir().unwrap();
    let today = chrono::Local::now().date_naive();
    let later = today + chrono::Duration::days(2);
    let goal = "Push the dentist to 4pm";
    let (state, settings, mut task, mut flow) = journey(dir.path(), goal);
    let reads = std::sync::Mutex::new(Vec::new());
    continue_workflow_using(
        &state,
        &mut task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        &settings,
        &mut flow,
        |tool, args| {
            let day = args["day"].as_str().unwrap_or_default().to_string();
            reads.lock().unwrap().push((tool.clone(), day.clone()));
            let items = if day == later.format("%Y-%m-%d").to_string() {
                vec![crate::apps::Event {
                    id: Some("e-7".into()),
                    title: "Dentist".into(),
                    start: format!("{day} 11:00"),
                    end: format!("{day} 11:30"),
                    calendar: "Home".into(),
                    ..Default::default()
                }]
            } else {
                vec![]
            };
            let evidence = crate::apps::calendar_evidence(
                &day,
                &crate::apps::Agenda {
                    total: items.len(),
                    items,
                },
            );
            async move {
                if tool == "read_calendar" {
                    Ok(evidence)
                } else {
                    Err("Unexpected app read".into())
                }
            }
        },
    )
    .await
    .unwrap();
    let days: Vec<String> = (0..3)
        .map(|ahead| {
            (today + chrono::Duration::days(ahead))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    assert_eq!(
        reads.into_inner().unwrap(),
        days.iter()
            .map(|day| ("read_calendar".to_string(), day.clone()))
            .collect::<Vec<_>>()
    );
    let proposal = flow.steps.last().unwrap();
    assert_eq!(proposal.tool, "update_calendar_event");
    assert_eq!(proposal.arguments["start"], format!("{} 16:00", days[2]));
    assert_eq!(proposal.arguments["minutes"], 30);
    assert_eq!(task.action.as_ref().unwrap()["fields"][0]["key"], "now");
    state.models.shutdown().await;
}

#[tokio::test]
async fn a_reminder_moves_by_the_users_words_and_keeps_its_time() {
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, mut flow) = journey(dir.path(), "Push the dentist reminder back a day");
    let reads = plan_with(
        &state,
        &settings,
        &mut task,
        &mut flow,
        vec![
            crate::apps::Reminder { due: "2026-11-03 09:00".into(), ..open("r-1", "Dentist") },
            open("r-2", "Buy milk"),
        ],
    )
    .await;
    assert_eq!(reads, ["read_reminders"]);
    let action = task.action.as_ref().unwrap();
    assert_eq!(action["tool"], "reschedule_reminder");
    assert!(action["fields"][0]["value"].as_str().unwrap().contains("due Tue 3 Nov 09:00"));
    let proposal = flow.steps.last().unwrap().clone();
    assert_eq!(proposal.arguments, json!({"id":"r-1","title":"Dentist","was_due":"2026-11-03 09:00","due":"2026-11-04 09:00"}));
    assert_eq!(flow.contract.provenance[&proposal.id]["due"].origin, "source_record");
    // The new due is the user's to edit; which reminder and its due as read are not.
    let never = |_: String, _: serde_json::Map<String, Value>, _| async { panic!("nothing may run") };
    let refused = run_workflow_action_with_reads(
        &state,
        &proposal.id,
        json!({"id":"r-1","title":"Dentist","was_due":"2026-11-02 09:00","due":"2026-11-04 09:00"}),
        &Channel::new(|_| Ok(())),
        never,
        |_, _, _| async { Err("Unexpected app read".into()) },
    )
    .await;
    assert!(refused.unwrap_err().contains("only an item it read"));
    run_workflow_action_with_reads(
        &state,
        &proposal.id,
        json!({"id":"r-1","title":"Dentist","was_due":"2026-11-03 09:00","due":"2026-11-05 08:30"}),
        &Channel::new(|_| Ok(())),
        |tool, args, _| async move { receipt(tool, args) },
        |_, _, _| async { Err("Unexpected app read".into()) },
    )
    .await
    .unwrap();
    let saved = state.store.lock().unwrap().workflow(&task.id).unwrap().unwrap();
    let step = saved.steps.last().unwrap();
    assert_eq!((step.tool.as_str(), step.state.as_str(), step.arguments["due"].as_str()), ("reschedule_reminder", "succeeded", Some("2026-11-05 08:30")));
    state.models.shutdown().await;
}

#[tokio::test]
async fn release_note_append_preserves_quoted_text_and_binds_the_named_record() {
    let title = "Buntini QA 57f9a95 Note Edited";
    let added = "Verification step two passed.";
    let goal = format!("Append the exact text \"{added}\" to my note named {title}.");
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, mut flow) = journey(dir.path(), &goal);
    let channel = Channel::new(|_| Ok(()));
    continue_workflow_using(
        &state, &mut task, &channel, Arc::new(AtomicBool::new(false)), &settings, &mut flow,
        |tool, args| async move {
            assert_eq!(tool, "find_notes");
            assert_eq!(args["query"], title);
            let output = format!("2026-11-17 09:00\tnote-fixture\t{title}\tOriginal verification text\t26\n__buntini_summary__\t1\t1\ttrue");
            crate::apps::notes_evidence(title, &output)
        },
    ).await.unwrap();
    assert_eq!(task.state, "waiting_action");
    let proposal = flow.steps.last().unwrap();
    assert_eq!(proposal.tool, "append_to_note");
    assert_eq!(proposal.arguments, json!({"id":"note-fixture","title":title,"text":added}));
    let saved = run_workflow_action_with_reads(
        &state, &proposal.id, proposal.arguments.clone(), &channel,
        |tool, args, _| async move {
            assert_eq!(tool, "append_to_note");
            assert_eq!(args["id"], "note-fixture");
            assert_eq!(args["text"], added);
            receipt(tool, args)
        },
        |_, _, _| async { Err("Unexpected source read after append.".into()) },
    ).await.unwrap();
    assert_eq!(saved.state, "completed", "{}", saved.answer);
    assert!(saved.answer.contains("Verified fixture change"));
}

#[tokio::test]
async fn release_browser_requests_wait_for_the_isolated_snapshot() {
    for goal in [
        "Open https://example.com/ in Buntini’s private browser workspace and tell me the visible page heading.",
        "Read the page at https://example.com/ using the isolated browser workspace. What is the page heading?",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (state, settings, mut task, _) = journey(dir.path(), goal);
        let channel = Channel::new(|_| Ok(()));
        execute_with_settings(
            &state, &mut task, "auto", &channel, Arc::new(AtomicBool::new(false)), settings, None,
        ).await.unwrap();
        assert_eq!(task.state, "waiting_action", "{}", task.answer);
        let flow = state.store.lock().unwrap().workflow(&task.id).unwrap().unwrap();
        let proposal = flow.steps.last().unwrap();
        assert_eq!(proposal.tool, "browse_page");
        assert_eq!(proposal.arguments["url"], "https://example.com/");
        assert_eq!(proposal.state, "waiting");
        let saved = run_workflow_action_with_reads(
            &state, &proposal.id, proposal.arguments.clone(), &channel,
            |tool, args, _| async move {
                assert_eq!(tool, "browse_page");
                assert_eq!(args["url"], "https://example.com/");
                let mut observed = receipt(tool, args);
                observed.object_id = Some("snapshot".into());
                observed.evidence = json!({
                    "url":"https://example.com/",
                    "title":"Browser tab label",
                    "heading":"Example Domain",
                    "verification":"native_url_and_bounded_page_snapshot"
                });
                observed
            },
            |_, _, _| async { Err("Unexpected app read.".into()) },
        ).await.unwrap();
        assert_eq!(saved.state, "completed", "{}", saved.answer);
        assert!(saved.answer.contains("Example Domain"), "{}", saved.answer);
        assert!(!saved.answer.contains("Browser tab label"));
    }
}

#[tokio::test]
async fn release_calendar_moves_keep_the_selected_calendar_date() {
    let date = chrono::Local::now().date_naive() + chrono::Duration::days(10);
    let day = date.format("%Y-%m-%d").to_string();
    let title = "Buntini QA Calendar Move";
    for goal in [
        format!("Move my {title} on {day} from 15:10 to 16:00. Keep its duration and location."),
        format!("On {day}, move my 15:10 to 16:00."),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (state, settings, mut task, mut flow) = journey(dir.path(), &goal);
        let channel = Channel::new(|_| Ok(()));
        let observed = crate::apps::calendar_evidence(&day, &crate::apps::Agenda {
            total: 1,
            items: vec![crate::apps::Event {
                id: Some("fixture-event".into()), calendar_id: Some("fixture-calendar".into()),
                calendar: "Fixture".into(), title: title.into(), location: "Room 7".into(),
                start: format!("{day} 15:10"), end: format!("{day} 15:45"), recurring: Some(false),
                ..Default::default()
            }],
        });
        continue_workflow_using(
            &state, &mut task, &channel, Arc::new(AtomicBool::new(false)), &settings, &mut flow,
            |tool, args| {
                assert_eq!(tool, "read_calendar");
                assert_eq!(args["day"], day);
                let observed = observed.clone();
                async move { Ok(observed) }
            },
        ).await.unwrap();
        assert_eq!(task.state, "waiting_action");
        let proposal = flow.steps.last().unwrap();
        assert_eq!(proposal.tool, "update_calendar_event");
        assert_eq!(proposal.arguments["id"], "fixture-event");
        assert_eq!(proposal.arguments["occurrence"], format!("{day} 15:10"));
        assert_eq!(proposal.arguments["start"], format!("{day} 16:00"));
        assert_eq!(proposal.arguments["minutes"], 35);
        assert_eq!(proposal.arguments["location"], "Room 7");
        let done = run_workflow_action_using(
            &state, &proposal.id, proposal.arguments.clone(), &channel,
            |tool, args, _| async move { receipt(tool, args) },
        ).await.unwrap();
        assert_eq!(done.state, "completed", "{}", done.answer);
    }
}

#[test]
fn release_note_searches_pin_the_exact_query_without_a_shared_folder() {
    for (goal, query) in [
        ("Find my note called Buntini QA 57f9a95 Note Edited and tell me its exact reference.", "Buntini QA 57f9a95 Note Edited"),
        ("Find my note containing cobalt-plover-5291.", "cobalt-plover-5291"),
    ] {
        assert!(!files_off(goal, false));
        let mut contract = crate::task_contract::TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = contract.next_outcome().unwrap();
        assert_eq!(read.accepted_tools, ["find_notes"]);
        assert_eq!(read.required_arguments["query"], query);
        assert!(contract.validate_proposal("find_notes", &json!({"query":"different"}), &[]).is_err());
    }
}

#[tokio::test]
async fn release_goal_keeps_its_follow_up_schedule_in_one_approval() {
    let goal = "Help me finish the Buntini QA checklist by Friday. Remind me to review it every evening at 10:48 PM.";
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, _) = journey(dir.path(), goal);
    execute_with_settings(
        &state, &mut task, "auto", &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)), settings, None,
    ).await.unwrap();
    assert_eq!(task.state, "waiting_action", "{}", task.answer);
    let flow = state.store.lock().unwrap().workflow(&task.id).unwrap().unwrap();
    assert_eq!(flow.steps.len(), 1);
    let step = &flow.steps[0];
    assert_eq!(step.tool, crate::goals::TOOL);
    assert_eq!(step.arguments["aim"], "Finish the Buntini QA checklist");
    assert_eq!(step.arguments["check_in"], "Review it");
    assert_eq!(step.arguments["style"], "remind");
    assert_eq!(step.arguments["days"], "every_day");
    assert_eq!(step.arguments["time"], "22:48");
    assert_eq!(step.arguments["done_when"], "date");
    let state_ref = &state;
    let done = run_workflow_action_using(
        &state, &step.id, step.arguments.clone(), &Channel::new(|_| Ok(())),
        |tool, args, _| async move {
            assert_eq!(tool, crate::goals::TOOL);
            crate::goals::execute(&state_ref.store.lock().unwrap(), &step.id, &args)
        },
    ).await.unwrap();
    assert_eq!(done.state, "completed", "{}", done.answer);
    let store = state.store.lock().unwrap();
    let saved = crate::goals::get(&store, &format!("goal-{}", step.id)).unwrap().unwrap();
    assert_eq!(saved.time, "22:48");
    assert_eq!(saved.check_in, "Review it");
}

#[tokio::test]
async fn release_weather_keeps_the_full_query_and_stops_before_reading_an_attachment() {
    let goal = "Check the current weather in Seattle. Give the temperature and wind, and cite your source.";
    let dir = tempfile::tempdir().unwrap();
    let (state, settings, mut task, _) = journey(dir.path(), goal);
    let events = Channel::new(|_| Ok(()));
    execute_with_settings(
        &state, &mut task, "auto", &events,
        Arc::new(AtomicBool::new(false)), settings.clone(), None,
    ).await.unwrap();
    assert_eq!(task.handoff.as_deref(), Some("online"));
    let flow = state.store.lock().unwrap().workflow(&task.id).unwrap().unwrap();
    assert_eq!(flow.steps[0].state, "waiting");
    assert_eq!(flow.steps[0].arguments["query"], goal);

    task.id = "attachment-weather".into();
    crate::attach::stage(&task.id, vec![crate::attach::Attachment::File {
        path: dir.path().join("must-not-be-opened.txt").display().to_string(), brain: None,
    }]);
    let result = execute_with_settings(
        &state, &mut task, "auto", &events, Arc::new(AtomicBool::new(false)), settings, None,
    ).await;
    crate::attach::release(&task.id, "error");
    assert_eq!(result.unwrap_err(), crate::i18n::t("chat.answer.liveWeatherAttachment"));
    assert!(state.store.lock().unwrap().workflow(&task.id).unwrap().is_none());
}
