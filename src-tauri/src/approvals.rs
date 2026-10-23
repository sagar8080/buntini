use crate::{controller::AppState, integrations, types::*};
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
    if prompt.is_empty() || prompt.len() > 2000 {
        return Err("Online requests must contain 1–2,000 characters.".into());
    }
    if plan_id.is_empty() || plan_id.len() > 64 {
        return Err("Invalid request id.".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
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
    let preview = integrations::preview(&call, &settings);
    let approval = PendingApproval {
        id: uuid::Uuid::new_v4().to_string(),
        provider: if call.tool == "web_search" {
            settings.search_provider.clone()
        } else {
            call.tool.clone()
        },
        prompt,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
        approved: false,
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
    state: State<'_, AppState>,
) -> Result<PendingApproval, String> {
    let provider = state.store.lock().unwrap().settings()?.search_provider;
    let approval = {
        let mut pending = state.approvals.lock().unwrap();
        let approval = pending
            .get_mut(&id)
            .ok_or("This approval no longer exists. Review the request again.")?;
        if !current(approval) || (approval.tool == "web_search" && approval.provider != provider) {
            return Err(
                "This approval expired or its search provider changed. Review the request again."
                    .into(),
            );
        }
        approval.approved = true;
        approval.clone()
    };
    state.audit("online.approval.granted", &id);
    Ok(approval)
}
/// Takes a one-use approval and returns the exact call it authorizes.
pub fn consume(
    state: &AppState,
    id: Option<&str>,
    prompt: &str,
    search_provider: &str,
) -> Result<integrations::Call, String> {
    let id = id.ok_or("Review and approve the online request before anything is sent.")?;
    let approval = state
        .approvals
        .lock()
        .unwrap()
        .remove(id)
        .ok_or("This approval has already been used or no longer exists.")?;
    let call = validate(&approval, prompt, search_provider)?;
    state.audit("online.approval.consumed", &format!("{id} {}", call.tool));
    Ok(call)
}
fn validate(
    approval: &PendingApproval,
    prompt: &str,
    search_provider: &str,
) -> Result<integrations::Call, String> {
    if !approval.approved
        || !current(approval)
        || approval.prompt != prompt
        || (approval.tool == "web_search" && approval.provider != search_provider)
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
        PendingApproval {
            id: id.into(),
            prompt: "news".into(),
            provider: provider.into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
            approved: false,
            tool: tool.into(),
            arguments,
            service: String::new(),
            summary: String::new(),
            sends: vec![],
            host: String::new(),
            uses_key: false,
            note: None,
        }
    }
    #[test]
    fn approval_is_exact_scoped_and_expires() {
        let mut a = approval("one", "web_search", json!({"query":"news"}), "brave");
        assert!(validate(&a, "news", "brave").is_err());
        a.approved = true;
        assert_eq!(
            validate(&a, "news", "brave").unwrap().arguments,
            json!({"query":"news"})
        );
        assert!(validate(&a, "private data", "brave").is_err());
        assert!(validate(&a, "news", "tavily").is_err());
        a.expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        assert!(validate(&a, "news", "brave").is_err());
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
        assert_eq!(validate(&a, "news", "brave").unwrap().tool, "weather");
        a.arguments = json!({"place":"Lisbon","units":"celsius","extra":"x"});
        assert!(validate(&a, "news", "brave").is_err());
        a.arguments = json!({"place":"Lisbon"});
        assert!(
            validate(&a, "news", "brave").is_err(),
            "defaults must have been approved explicitly"
        );
        a.tool = "run_shell".into();
        assert!(validate(&a, "news", "brave").is_err());
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
        consume(&state, Some("one-use"), "news", "brave").unwrap();
        assert!(consume(&state, Some("one-use"), "news", "brave").is_err());
    }
}
