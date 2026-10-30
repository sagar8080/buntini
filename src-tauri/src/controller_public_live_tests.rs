//! Opt-in real free-provider requests plus actual Apple inference, entered at
//! the production approved integration controller boundary (not through UI).
use super::apple_live_tests::{live_state, record_live_case, LiveTurn};
use super::*;
use std::time::Instant;

async fn approved_turn(state: &AppState, prompt: &str, call: integrations::Call) -> LiveTurn {
    let before = state.models.apple.model_calls();
    let started = Instant::now();
    let settings = state.store.lock().unwrap().settings().unwrap();
    let mut task:TaskRecord=serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"prompt":prompt,"answer":"","state":"running","createdAt":now(),"sources":[],"files":[],"backend":"apple"})).unwrap();
    state.store.lock().unwrap().save_task(&task).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let channel = Channel::new(|_| Ok(()));
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(180),
        execute_with_settings(
            state,
            &mut task,
            "web",
            &channel,
            cancel.clone(),
            settings,
            Some(call),
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            cancel.store(true, Ordering::Relaxed);
            state.models.reset().await;
            Err("Public workflow exceeded 180 seconds.".into())
        }
    };
    let error = result.as_ref().err().cloned();
    task.state = if result.is_ok() { "completed" } else { "error" }.into();
    if let Some(error) = &error {
        task.answer = error.clone();
    }
    state.store.lock().unwrap().save_task(&task).unwrap();
    LiveTurn {
        task,
        workflow: None,
        apple_calls: state.models.apple.model_calls() - before,
        error,
        duration_ms: started.elapsed().as_millis(),
    }
}

fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn first_number(text: &str) -> Option<String> {
    let start = text
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit() || *c == '-')?
        .0;
    Some(
        text[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '.'))
            .collect(),
    )
}

fn valid_citations(answer: &str, count: usize, required: &[usize]) -> bool {
    let mut found = std::collections::BTreeSet::new();
    for suffix in answer.split('[').skip(1) {
        if let Some((number, _)) = suffix.split_once(']') {
            if let Ok(number) = number.parse::<usize>() {
                if number == 0 || number > count {
                    return false;
                }
                found.insert(number);
            }
        }
    }
    !found.is_empty() && required.iter().all(|number| found.contains(number))
}

pub(super) async fn public_workflows() -> Vec<Value> {
    let cases=vec![
        ("public-seattle-walk-weather","weather",json!({"place":"Seattle, Washington, United States","units":"celsius"}),"I am planning a walk in Seattle. Check the current weather in Celsius, report the current temperature and wind, and give one practical suggestion based only on the returned conditions. Cite the retrieved results and say they were just checked."),
        ("public-space-needle-background","wikipedia",json!({"query":"Space Needle","language":"en"}),"Look up the Space Needle and give me a concise explanation of what it is, which city and state it is in, and why it is a landmark. Use only the returned Wikipedia facts and cite them."),
        ("public-pike-place-cafes","places",json!({"category":"cafe","near":"Pike Place Market, Seattle, Washington","radius_m":800}),"Find cafes within 800 meters of Pike Place Market in Seattle. Copy the exact names and street addresses of the first two returned cafes so I can compare them. Cite each result; do not invent ratings, current availability, opening hours or directions."),
    ];
    let mut rows = Vec::new();
    for (id, tool, arguments, prompt) in cases {
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let call = integrations::validate(tool, &arguments, prompt).unwrap();
        assert!(
            integrations::tool(tool).unwrap().key.is_none(),
            "Live acceptance must not use saved keys or paid providers"
        );
        let settings = state.store.lock().unwrap().settings().unwrap();
        let preview = integrations::preview(&call, &settings);
        let approved = json!({"tool":call.tool,"arguments":call.arguments,"host":preview.host,"service":preview.provider,"sends":preview.sends,"usesKey":preview.uses_key});
        let turn = approved_turn(&state, prompt, call).await;
        let answer = turn.task.answer.to_lowercase();
        let compact = normalized(&answer);
        let cards = &turn.task.cards;
        let provider_evidence = integrations::cards_text(cards);
        let actual_summary = !answer.contains("summary unavailable")
            && !answer.contains("summary was unavailable")
            && turn.task.answer != provider_evidence;
        let required_sources: &[usize] = if tool == "places" { &[1, 2] } else { &[1] };
        let citations = valid_citations(&turn.task.answer, cards.len(), required_sources);
        let mut facts = Vec::new();
        match tool {
            "weather" => {
                let current = cards.first();
                let temperature = current
                    .and_then(|card| {
                        card.fields
                            .iter()
                            .find(|field| field.label == "Temperature")
                    })
                    .and_then(|field| first_number(&field.value));
                let wind = current
                    .and_then(|card| card.fields.iter().find(|field| field.label == "Wind"))
                    .and_then(|field| first_number(&field.value));
                facts.push((
                    "currentTemperatureFromProvider",
                    temperature.is_some_and(|n| answer.contains(&n)),
                ));
                facts.push((
                    "windFromProvider",
                    wind.is_some_and(|n| answer.contains(&n)) && answer.contains("wind"),
                ));
                facts.push((
                    "requestedPlaceAndUnits",
                    answer.contains("seattle")
                        && (answer.contains("°c") || answer.contains("celsius")),
                ));
                facts.push((
                    "honestRetrievalFreshness",
                    answer.contains("retrieved just now") || answer.contains("just checked"),
                ));
                facts.push((
                    "requestedPracticalWalkingSuggestion",
                    integrations::has_walk_advice(&turn.task.answer),
                ));
            }
            "wikipedia" => {
                facts.push((
                    "actualSpaceNeedleArticle",
                    cards
                        .first()
                        .is_some_and(|card| card.title == "Space Needle"),
                ));
                facts.push((
                    "groundedLocationAndType",
                    answer.contains("space needle")
                        && answer.contains("seattle")
                        && answer.contains("washington")
                        && (answer.contains("tower") || answer.contains("landmark")),
                ));
            }
            "places" => {
                facts.push(("twoRealProviderResults", cards.len() >= 2));
                let names = cards
                    .iter()
                    .take(2)
                    .all(|card| compact.contains(&normalized(&card.title)));
                let addresses = cards.iter().take(2).all(|card| {
                    card.fields
                        .iter()
                        .find(|field| field.label == "Address")
                        .is_some_and(|field| {
                            let expected = normalized(&field.value);
                            !expected.is_empty() && compact.contains(&expected)
                        })
                });
                facts.push(("exactFirstTwoBusinessNames", names));
                facts.push(("providerStreetAddressFacts", addresses));
            }
            _ => unreachable!(),
        }
        let persisted = state
            .store
            .lock()
            .unwrap()
            .task(&turn.task.id)
            .unwrap()
            .is_some_and(|saved| {
                saved.answer == turn.task.answer && saved.cards.len() == cards.len()
            });
        let checks = json!({"realProviderCards":!cards.is_empty(),"actualAppleSummary":actual_summary,"validSourceCitations":citations,"requiredSources":required_sources,"persistedTaskAndCards":persisted,"facts":facts.iter().map(|(name,passed)|json!({"name":name,"passed":passed})).collect::<Vec<_>>()});
        let passed = turn.error.is_none()
            && turn.task.state == "completed"
            && turn.apple_calls > 0
            && !cards.is_empty()
            && actual_summary
            && citations
            && persisted
            && facts.iter().all(|(_, passed)| *passed);
        let helper_hash = std::fs::read(state.models.apple_helper())
            .ok()
            .map(|bytes| crate::security::sha256_hex(bytes));
        rows.push(record_live_case(json!({"id":id,"kind":"real_public_integration_workflow","passed":passed,"apple_calls":turn.apple_calls,"request":prompt,"approvedRequest":approved,"checks":checks,"observation":turn,"helper_sha256":helper_hash,"entry":"production execute_with_settings(web, approvedCall) and integration_task; direct approved controller boundary, UI not exercised","source_kind":"actual fixed public free-provider response cards; no synthetic records or saved keys","personal_location_used":false,"external_messages_sent":false})));
        state.models.shutdown().await;
    }
    rows
}
