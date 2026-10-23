use super::*;

#[tokio::test]
async fn explicit_research_is_journaled_before_any_model_or_network_call() {
    for (prompt, expected, compound) in [
        ("Google \"Rust vs Go\" -ads", "\"Rust vs Go\" -ads", false),
        ("Search the web for calendar apps", "calendar apps", false),
        (
            "Search the web for train fares, then create a note",
            "train fares",
            true,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path().join("data"),
            dir.path().join("cache"),
            dir.path().into(),
        )
        .unwrap();
        let settings = Settings::default();
        let mut task: TaskRecord = serde_json::from_value(json!({"id":"research-routing","prompt":prompt,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[]})).unwrap();
        let channel = Channel::new(|_| Ok(()));
        execute_with_settings(
            &state,
            &mut task,
            "auto",
            &channel,
            Arc::new(AtomicBool::new(false)),
            settings,
            None,
        )
        .await
        .unwrap();
        assert_eq!(task.state, "waiting_online");
        assert_eq!(task.handoff.as_deref(), Some("online"));
        assert!(task.sources.is_empty());
        let flow = state
            .store
            .lock()
            .unwrap()
            .workflow(&task.id)
            .unwrap()
            .unwrap();
        assert_eq!(flow.goal, prompt);
        assert_eq!(flow.steps.len(), 1);
        assert_eq!(flow.steps[0].tool, "look_up_online");
        assert_eq!(flow.steps[0].arguments["query"], expected);
        assert_eq!(flow.steps[0].state, "waiting");
        if compound {
            assert!(flow
                .contract
                .outcomes
                .iter()
                .any(|o| o.tool.as_deref() == Some("create_note") && o.status != "complete"));
        }
    }
}

#[tokio::test]
async fn saved_lookup_approval_covers_browser_first_and_only_selected_api_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().into(),
    )
    .unwrap();
    let settings = Settings::default();
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"research-approval","prompt":"Google weather forecast","answer":"","state":"running","createdAt":now(),"sources":[],"files":[]})).unwrap();
    execute_with_settings(
        &state,
        &mut task,
        "auto",
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        settings.clone(),
        None,
    )
    .await
    .unwrap();
    let approval = crate::approvals::plan(
        &state,
        "weather forecast".into(),
        "online-plan".into(),
        false,
        Some(&task.id),
    )
    .await
    .unwrap();
    assert_eq!(approval.tool, "web_search");
    assert_eq!(approval.arguments["query"], "weather forecast");
    assert_eq!(approval.provider, settings.search_provider);
    assert!(approval
        .host
        .contains(crate::browser_workspace::SEARCH_HOST));
    assert!(approval.host.contains("only if needed"));
    assert!(approval.sends.iter().any(|f| f.label == "Page reads"));
    assert!(!approval.approved);
    assert!(crate::approvals::consume(
        &state,
        Some(&approval.id),
        "weather forecast",
        &settings.search_provider
    )
    .is_err());
}

#[test]
fn existing_browser_reads_require_app_access_and_the_selected_models_browser_permission() {
    use crate::privacy::Locality;
    let mut settings = Settings::default();
    for locality in [Locality::OnDevice, Locality::External] {
        assert!(!research_browser_read_allowed(&settings, locality, None));
        assert!(!research_browser_read_allowed(
            &settings,
            locality,
            Some("safari")
        ));
    }
    settings.apps = vec!["safari".into()];
    assert!(research_browser_read_allowed(
        &settings,
        Locality::OnDevice,
        Some("safari")
    ));
    assert!(!research_browser_read_allowed(
        &settings,
        Locality::External,
        Some("safari")
    ));
    settings.external_shares = vec!["history".into(), "browser".into()];
    assert!(research_browser_read_allowed(
        &settings,
        Locality::External,
        Some("safari")
    ));
    for locality in [Locality::OnDevice, Locality::External] {
        assert!(!research_browser_read_allowed(&settings, locality, None));
        assert!(!research_browser_read_allowed(
            &settings,
            locality,
            Some("chrome")
        ));
    }
    settings.apps.clear();
    assert!(!research_browser_read_allowed(
        &settings,
        Locality::External,
        Some("safari")
    ));
}

#[test]
fn browser_research_provenance_survives_transfer_storage_and_permission_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().into(),
    )
    .unwrap();
    let mut settings = Settings {
        model_backend: "cloud".into(),
        external_shares: vec!["files".into(), "browser".into(), "history".into()],
        ..Settings::default()
    };
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"browser-provenance","prompt":"Search the web for museum hours","answer":"The museum closes at five [1].","state":"completed","createdAt":now(),"sources":[],"files":[],"touched":["files"]})).unwrap();
    merge_touched(&mut task, &["browser".into(), "files".into(), "browser".into()]);
    assert_eq!(task.touched, ["files", "browser"]);
    let mut flow = crate::workflows::Workflow::new(
        &task,
        "auto",
        &settings,
        vec![],
        None,
        &["look_up_online"],
    );
    state
        .store
        .lock()
        .unwrap()
        .checkpoint(&mut flow, &task)
        .unwrap();
    let store = state.store.lock().unwrap();
    let restored = store.task(&task.id).unwrap().unwrap();
    let restored_flow = store.workflow(&task.id).unwrap().unwrap();
    assert_eq!(restored.touched, ["files", "browser"]);
    assert!(restored_flow.check_resume(&restored, &settings).is_ok());
    let (history, inherited) = crate::conversation::history_with_shares(
        std::slice::from_ref(&restored),
        None,
        1000,
        privacy::Locality::External,
        &settings,
    );
    assert_eq!(history.len(), 2);
    assert_eq!(inherited, ["files", "browser"]);
    settings.external_shares.retain(|id| id != "browser");
    assert!(restored_flow.check_resume(&restored, &settings).is_err());
    let (history, inherited) = crate::conversation::history_with_shares(
        &[restored],
        None,
        1000,
        privacy::Locality::External,
        &settings,
    );
    assert!(history.is_empty());
    assert!(inherited.is_empty());
}

#[tokio::test]
async fn saved_browser_evidence_is_blocked_before_cloud_model_start_or_output() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().into(),
    )
    .unwrap();
    let settings = Settings {
        model_backend: "cloud".into(),
        external_shares: vec!["history".into()],
        ..Settings::default()
    };
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"private-browser-evidence","prompt":"Search the web for museum hours","answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"touched":["browser"]})).unwrap();
    let emitted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = emitted.clone();
    let channel = Channel::new(move |_| {
        captured.fetch_add(1, Ordering::Relaxed);
        Ok(())
    });
    let error = synthesize_research(
        &state,
        &mut task,
        &channel,
        Arc::new(AtomicBool::new(false)),
        &settings,
    )
    .await
    .unwrap_err();
    assert_eq!(error, "Browser sharing is off for this model. Enable Browser sharing or use an on-device model to summarize these saved browser sources.");
    assert_eq!(emitted.load(Ordering::Relaxed), 0);
    assert!(task.answer.is_empty());
}

#[test]
fn research_approval_discloses_existing_profile_and_preserves_exact_outgoing_query() {
    let query = "\"C++ museum\" -tickets site:example.org";
    let call = integrations::validate("web_search", &json!({"query":query}), query).unwrap();
    for (provider, host) in [("brave", "api.search.brave.com"), ("tavily", "api.tavily.com")] {
        let settings = Settings {
            search_provider: provider.into(),
            ..Settings::default()
        };
        let preview = integrations::preview(&call, &settings);
        assert!(preview.summary.contains("existing or default browser"));
        assert!(!preview.summary.contains("isolated"));
        assert!(preview.host.contains("www.google.com"));
        assert!(preview.host.contains(host));
        assert!(preview.host.contains("only if needed"));
        let value = serde_json::to_value(&preview.sends).unwrap();
        let fields = value.as_array().unwrap();
        assert_eq!(fields.iter().find(|f| f["label"] == "Query").unwrap()["value"], query);
        let browser = fields
            .iter()
            .find(|f| f["label"] == "Browser")
            .unwrap()["value"]
            .as_str()
            .unwrap();
        for disclosure in ["normal profile", "cookies", "sign-ins", "history", "only its research tab"] {
            assert!(browser.contains(disclosure), "Missing disclosure: {disclosure}");
        }
        let page_reads = fields
            .iter()
            .find(|f| f["label"] == "Page reads")
            .unwrap()["value"]
            .as_str()
            .unwrap();
        assert!(page_reads.contains("six"));
        assert!(page_reads.contains("browser reading is unavailable"));
        assert!(!preview.uses_key);
    }
}

#[tokio::test]
#[ignore = "uses local Apple Intelligence with synthetic public research; no private apps or network"]
async fn apple_research_summary_meets_production_coverage_and_citation_gate() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut state = AppState::new(
        dir.path().join("data"),
        dir.path().join("cache"),
        root.into(),
    )
    .unwrap();
    state.models = ModelHub::new(root.join("worker/main.py"), root.join("native/buntini-fm"));
    let settings = Settings {
        model_backend: "apple".into(),
        ..Settings::default()
    };
    let mut task: TaskRecord = serde_json::from_value(json!({"id":"research-summary","prompt":"Search the web for the admission price and hours of the Fictional Museum","answer":"","state":"running","createdAt":now(),"sources":[],"files":[]})).unwrap();
    let source = Source {
        title: "Fictional Museum visitor guide".into(),
        url: "https://museum.example/visit".into(),
        snippet: "Visitor information".into(),
        retrieved_at: now(),
    };
    let page = crate::research::ResearchContent {url:source.url.clone(),title:source.title.clone(),text:"Synthetic fixture: the Fictional Museum charges $12 admission for adults and is open Tuesday through Sunday from 10 am to 5 pm. It is closed on Mondays. These are the published visitor hours and admission fees in this controlled test.".into(),retrieved_at:now(),truncated:false};
    task.sources = vec![source];
    task.research = vec![page];
    task.research_report = Some(crate::search_pipeline::Report {
        intent: crate::search_intent::SearchIntent::from_lookup(
            &task.prompt,
            "Fictional Museum admission hours",
        ),
        sources: task.sources.clone(),
        pages: task.research.clone(),
        attempts: vec![],
        limitations: vec![],
        evidence_ready: true,
        answer_verified: false,
    });
    let result = synthesize_research(
        &state,
        &mut task,
        &Channel::new(|_| Ok(())),
        Arc::new(AtomicBool::new(false)),
        &settings,
    )
    .await;
    state.models.shutdown().await;
    println!(
        "RESEARCH_SUMMARY_EVAL {}",
        json!({"backend":"apple","synthetic":true,"passed":result.is_ok(),"answer":task.answer,"error":result.as_ref().err(),"researchReport":task.research_report.as_ref().map(crate::search_pipeline::Report::proof)})
    );
    result.unwrap();
    assert!(task.answer.contains("12"), "{}", task.answer);
    assert!(task.answer.contains("[1]"), "{}", task.answer);
    assert!(task.research_report.unwrap().answer_verified);
}
