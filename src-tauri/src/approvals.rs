use crate::{controller::AppState, helper, integrations, types::*};
use serde_json::Value;
use std::sync::{atomic::AtomicBool, Arc};
use tauri::State;

fn current(approval: &PendingApproval) -> bool {
    chrono::DateTime::parse_from_rfc3339(&approval.expires_at)
        .map(|date| date > chrono::Utc::now())
        .unwrap_or(false)
}

/// Lets the local model pick one online tool for the prompt (or uses an exact web search
/// when `web_only`), then holds the validated call for the user to review. Nothing is
/// sent anywhere until the user approves it and `run_task` consumes the approval.
#[tauri::command]
pub async fn plan_online_request(
    prompt: String,
    plan_id: String,
    web_only: bool,
    state: State<'_, AppState>,
) -> Result<PendingApproval, String> {
    plan(&state, prompt, plan_id, web_only, None).await
}

pub(crate) async fn plan(
    state: &AppState,
    prompt: String,
    plan_id: String,
    web_only: bool,
    task_id: Option<&str>,
) -> Result<PendingApproval, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() || prompt.chars().count() > 2000 {
        return Err("Online requests must contain 1–2,000 characters.".into());
    }
    if plan_id.is_empty() || plan_id.len() > 64 {
        return Err("Invalid request id.".into());
    }
    // A routine run gives way to what the user asks for now.
    crate::controller::wait_for_slot(state, std::time::Duration::from_secs(15)).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut origin = helper::Origin { goal: prompt.clone(), touched: vec![], attended: true };
    let settings = {
        let mut active = state.active.lock().unwrap();
        if !active.is_empty() {
            return Err("Finish or cancel the current task first.".into());
        }
        let store = state.store.lock().unwrap();
        let settings = store.settings()?;
        if let Some(task_id) = task_id {
            let flow = store
                .workflow(task_id)?
                .ok_or("This task no longer exists.")?;
            let task = store.task(&flow.id)?.ok_or("This task no longer exists.")?;
            flow.check_resume(&task, &settings)?;
            if flow.state != "waiting_online"
                || flow
                    .steps
                    .last()
                    .and_then(|s| s.arguments["query"].as_str())
                    != Some(prompt.as_str())
            {
                return Err("This saved online step was cancelled or changed.".into());
            }
            origin = helper::Origin { goal: flow.goal, touched: task.touched, attended: flow.attended };
        }
        active.insert(plan_id.clone(), cancel.clone());
        settings
    };
    let _claim = crate::controller::ActionClaim {
        state,
        id: plan_id.clone(),
    };
    // A saved look_up_online step is always the browser-first research route.
    // Replanning its stripped query could otherwise switch to an unrelated API.
    let planned = if web_only || task_id.is_some() {
        integrations::validate("web_search", &Value::Null, &prompt).map(|call| (call, None))
    } else {
        integrations::plan(&state, &settings, &prompt, cancel).await
    };
    let (call, note) = planned?;
    // Only a web search about to be offered is worth asking the Keychain for.
    let chatgpt_ready = call.tool == "web_search" && !web_only && helper::ready(state);
    let call = routed(call, chatgpt_ready, &prompt)?;
    let always = helper::may_skip_sheet(helper::always_allowed(state), &call.tool, &origin, &prompt);
    let approval = hold(state, call, prompt, note, always, &settings)?;
    if always {
        state.audit("online.approval.always", &approval.id);
    }
    Ok(approval)
}

/// A web search goes to ChatGPT when it can take one.
fn routed(call: integrations::Call, chatgpt_ready: bool, prompt: &str) -> Result<integrations::Call, String> {
    if call.tool == "web_search" && chatgpt_ready {
        integrations::validate(helper::SEARCH_TOOL, &Value::Null, prompt)
    } else {
        Ok(call)
    }
}

/// "Ask ChatGPT …" and the button on a reply: the sheet, with the user's words.
#[tauri::command]
pub fn plan_ask_chatgpt(question: String, state: State<'_, AppState>) -> Result<PendingApproval, String> {
    let question = question.trim().to_string();
    if question.chars().count() > 2000 {
        return Err("Online requests must contain 1–2,000 characters.".into());
    }
    if !helper::usable(&state) {
        return Err("Sign in with ChatGPT in Settings → Brain first.".into());
    }
    let call = integrations::validate(helper::ASK_TOOL, &Value::Null, &question)?;
    let settings = state.store.lock().unwrap().settings()?;
    hold(&state, call, question, None, false, &settings)
}

/// Keeps one request for the user to allow, or as allowed already under Always allow.
fn hold(
    state: &AppState,
    call: integrations::Call,
    prompt: String,
    note: Option<String>,
    approved: bool,
    settings: &Settings,
) -> Result<PendingApproval, String> {
    let preview = integrations::preview(&call, settings);
    let approval = PendingApproval {
        id: uuid::Uuid::new_v4().to_string(),
        provider: if call.tool == "web_search" {
            settings.search_provider.clone()
        } else {
            call.tool.clone()
        },
        prompt,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
        approved,
        tool: call.tool,
        arguments: call.arguments,
        service: preview.provider,
        summary: preview.summary,
        sends: preview.sends,
        host: preview.host,
        uses_key: preview.uses_key,
        note,
    };
    {
        let mut pending = state.approvals.lock().unwrap();
        pending.retain(|_, approval| current(approval));
        if pending.len() >= 10 {
            return Err(
                "Too many pending approvals. Wait for an existing approval to expire.".into(),
            );
        }
        pending.insert(approval.id.clone(), approval.clone());
    }
    state.audit(
        "online.approval.requested",
        &format!("{} {} → {}", approval.id, approval.tool, approval.host),
    );
    Ok(approval)
}
#[tauri::command]
pub fn approve_web_approval(
    id: String,
    always: Option<bool>,
    state: State<'_, AppState>,
) -> Result<PendingApproval, String> {
    grant(&state, &id, always.unwrap_or(false))
}

fn grant(state: &AppState, id: &str, always: bool) -> Result<PendingApproval, String> {
    let settings = state.store.lock().unwrap().settings()?;
    let approval = {
        let mut pending = state.approvals.lock().unwrap();
        let approval = pending
            .get_mut(id)
            .ok_or("This approval no longer exists. Review the request again.")?;
        if !current(approval) || !preview_matches(approval, &settings) {
            return Err(
                "This approval expired or its request settings changed. Review the request again."
                    .into(),
            );
        }
        approval.approved = true;
        approval.clone()
    };
    state.audit("online.approval.granted", id);
    // Only a public internet question can start this; "Ask ChatGPT" always asks.
    if always && approval.tool == helper::SEARCH_TOOL {
        helper::set_always_allowed(state, true)?;
    }
    Ok(approval)
}
/// Takes a one-use approval and returns the exact call it authorizes.
pub fn consume(
    state: &AppState,
    id: Option<&str>,
    prompt: &str,
    settings: &Settings,
) -> Result<integrations::Call, String> {
    let id = id.ok_or("Review and approve the online request before anything is sent.")?;
    let approval = state
        .approvals
        .lock()
        .unwrap()
        .remove(id)
        .ok_or("This approval has already been used or no longer exists.")?;
    let call = validate(&approval, prompt, settings)?;
    state.audit("online.approval.consumed", &format!("{id} {}", call.tool));
    Ok(call)
}
/// Execution settings can affect the outgoing request even when the model's
/// arguments are unchanged (for example, eBay marketplace and price currency).
/// Require the current request preview to match every detail the user reviewed.
fn preview_matches(approval: &PendingApproval, settings: &Settings) -> bool {
    if approval.tool == "web_search" && approval.provider != settings.search_provider {
        return false;
    }
    let preview = integrations::preview(
        &integrations::Call {
            tool: approval.tool.clone(),
            arguments: approval.arguments.clone(),
        },
        settings,
    );
    approval.service == preview.provider
        && approval.summary == preview.summary
        && approval.sends == preview.sends
        && approval.host == preview.host
        && approval.uses_key == preview.uses_key
}

fn validate(
    approval: &PendingApproval,
    prompt: &str,
    settings: &Settings,
) -> Result<integrations::Call, String> {
    if !approval.approved
        || !current(approval)
        || approval.prompt != prompt
        || !preview_matches(approval, settings)
    {
        return Err("A current approval for this exact request is required.".into());
    }
    // Re-validate so a tampered or stale approval can never widen what gets sent.
    let call = integrations::validate(&approval.tool, &approval.arguments, prompt)?;
    if call.arguments != approval.arguments {
        return Err("The approved request no longer matches. Review it again.".into());
    }
    Ok(call)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn saved_online_planning_checks_provenance_before_any_model_request() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path().join("data"),
            dir.path().join("cache"),
            dir.path().into(),
        )
        .unwrap();
        let settings = Settings {
            model_backend: "cloud".into(),
            external_shares: vec![],
            ..Settings::default()
        };
        state
            .store
            .lock()
            .unwrap()
            .save_settings(&settings)
            .unwrap();
        let task: TaskRecord = serde_json::from_value(json!({"id":"private-online","prompt":"Look up this appointment","answer":"","state":"waiting_online","createdAt":now(),"sources":[],"files":[],"touched":["mail"]})).unwrap();
        let mut flow = crate::workflows::Workflow::new(
            &task,
            "auto",
            &settings,
            vec![],
            None,
            &["look_up_online"],
        );
        flow.begin(
            "look_up_online",
            json!({"query":"private medical appointment from email"}),
            json!({}),
            false,
            true,
        )
        .unwrap();
        state
            .store
            .lock()
            .unwrap()
            .checkpoint(&mut flow, &task)
            .unwrap();
        let error = plan(
            &state,
            "private medical appointment from email".into(),
            "plan-private".into(),
            false,
            Some(&task.id),
        )
        .await
        .unwrap_err();
        assert!(error.contains("no longer share"));
        assert!(state.active.lock().unwrap().is_empty());
        assert!(state.approvals.lock().unwrap().is_empty());
    }

    fn approval(id: &str, tool: &str, arguments: Value, provider: &str) -> PendingApproval {
        let settings = Settings {
            search_provider: if tool == "web_search" {
                provider.into()
            } else {
                "brave".into()
            },
            ..Settings::default()
        };
        let preview = integrations::preview(
            &integrations::Call {
                tool: tool.into(),
                arguments: arguments.clone(),
            },
            &settings,
        );
        PendingApproval {
            id: id.into(),
            prompt: "news".into(),
            provider: provider.into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
            approved: false,
            tool: tool.into(),
            arguments,
            service: preview.provider,
            summary: preview.summary,
            sends: preview.sends,
            host: preview.host,
            uses_key: preview.uses_key,
            note: None,
        }
    }
    fn settings(provider: &str) -> Settings {
        Settings {
            search_provider: provider.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn approval_is_exact_scoped_and_expires() {
        let mut a = approval("one", "web_search", json!({"query":"news"}), "brave");
        assert!(validate(&a, "news", &settings("brave")).is_err());
        a.approved = true;
        assert_eq!(
            validate(&a, "news", &settings("brave")).unwrap().arguments,
            json!({"query":"news"})
        );
        assert!(validate(&a, "private data", &settings("brave")).is_err());
        assert!(validate(&a, "news", &settings("tavily")).is_err());
        a.expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        assert!(validate(&a, "news", &settings("brave")).is_err());
    }
    #[test]
    fn integration_approvals_cannot_be_widened() {
        let mut a = approval(
            "two",
            "weather",
            json!({"place":"Lisbon","units":"celsius"}),
            "weather",
        );
        a.approved = true;
        assert_eq!(
            validate(&a, "news", &settings("brave")).unwrap().tool,
            "weather"
        );
        a.arguments = json!({"place":"Lisbon","units":"celsius","extra":"x"});
        assert!(validate(&a, "news", &settings("brave")).is_err());
        a.arguments = json!({"place":"Lisbon"});
        assert!(
            validate(&a, "news", &settings("brave")).is_err(),
            "defaults must have been approved explicitly"
        );
        a.tool = "run_shell".into();
        assert!(validate(&a, "news", &settings("brave")).is_err());
    }
    #[test]
    fn marketplace_and_reviewed_details_cannot_change_after_approval() {
        let arguments = integrations::validate(
            "ebay_search",
            &json!({"query":"headphones","max_price":100}),
            "news",
        )
        .unwrap()
        .arguments;
        let mut grant = approval("marketplace", "ebay_search", arguments, "ebay_search");
        grant.approved = true;
        let mut current = settings("brave");
        assert!(validate(&grant, "news", &current).is_ok());
        current.ebay_marketplace = "EBAY_GB".into();
        assert!(validate(&grant, "news", &current).is_err());
        current.ebay_marketplace = "EBAY_US".into();
        assert!(validate(&grant, "news", &current).is_ok());
        grant.host = "unreviewed.example".into();
        assert!(validate(&grant, "news", &current).is_err());
    }

    #[test]
    fn a_web_search_goes_to_chatgpt_only_when_it_can_take_one() {
        let web = integrations::validate("web_search", &Value::Null, "who won?").unwrap();
        assert_eq!(routed(web.clone(), false, "who won?").unwrap(), web, "not signed in, or asked for the browser");
        let asked = routed(web, true, "who won?").unwrap();
        assert_eq!((asked.tool.as_str(), asked.arguments.clone()), (helper::SEARCH_TOOL, json!({"query":"who won?"})));
        let weather = integrations::validate("weather", &json!({"place":"Lisbon"}), "weather in Lisbon").unwrap();
        assert_eq!(routed(weather.clone(), true, "weather in Lisbon").unwrap(), weather, "a specific lookup keeps its route");
    }

    #[test]
    fn the_sheet_names_chatgpt_and_shows_the_exact_question() {
        let mut sheet = approval("c", helper::SEARCH_TOOL, json!({"query":"who won?"}), helper::SEARCH_TOOL);
        assert_eq!((sheet.service.as_str(), sheet.host.as_str()), ("ChatGPT", "api.openai.com"));
        assert_eq!((sheet.sends[0].label.as_str(), sheet.sends[0].value.as_str()), ("Question", "who won?"));
        assert!(!sheet.uses_key);
        sheet.prompt = "who won?".into();
        assert!(validate(&sheet, "who won?", &settings("brave")).is_err(), "not until it's allowed");
        sheet.approved = true;
        assert_eq!(validate(&sheet, "who won?", &settings("brave")).unwrap().tool, helper::SEARCH_TOOL);
        assert!(validate(&sheet, "and my address is", &settings("brave")).is_err(), "only what was shown");
        sheet.arguments = json!({"query":"something else"});
        assert!(validate(&sheet, "who won?", &settings("brave")).is_err());
    }

    #[test]
    fn a_model_can_never_pick_chatgpt_as_a_lookup_of_its_own() {
        assert!(!integrations::available().iter().any(|tool| helper::is_tool(tool)));
    }

    #[test]
    fn always_allow_starts_only_from_a_public_question() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().to_path_buf()).unwrap();
        let held = |id: &str, tool: &str| {
            let mut sheet = approval(id, tool, json!({"query":"who won?"}), tool);
            sheet.prompt = "who won?".into();
            state.approvals.lock().unwrap().insert(id.into(), sheet);
        };
        state.store.lock().unwrap().save_settings(&settings("brave")).unwrap();
        held("asked", helper::ASK_TOOL);
        grant(&state, "asked", true).unwrap();
        assert!(!helper::always_allowed(&state), "\"Ask ChatGPT\" always shows the sheet");
        held("once", helper::SEARCH_TOOL);
        grant(&state, "once", false).unwrap();
        assert!(!helper::always_allowed(&state));
        held("search", helper::SEARCH_TOOL);
        assert!(grant(&state, "search", true).unwrap().approved);
        assert!(helper::always_allowed(&state));
    }

    #[test]
    fn approved_grants_are_consumed_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path().join("data"),
            dir.path().join("cache"),
            dir.path().to_path_buf(),
        )
        .unwrap();
        let mut grant = approval("one-use", "web_search", json!({"query":"news"}), "brave");
        grant.approved = true;
        state
            .approvals
            .lock()
            .unwrap()
            .insert(grant.id.clone(), grant);
        consume(&state, Some("one-use"), "news", &settings("brave")).unwrap();
        assert!(consume(&state, Some("one-use"), "news", &settings("brave")).is_err());
    }
}
