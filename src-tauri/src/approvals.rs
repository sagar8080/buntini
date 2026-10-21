use crate::{controller::AppState, types::*};
use tauri::State;

fn current(approval: &PendingApproval) -> bool {
    chrono::DateTime::parse_from_rfc3339(&approval.expires_at)
        .map(|date| date > chrono::Utc::now())
        .unwrap_or(false)
}
#[tauri::command]
pub fn create_web_approval(
    prompt: String,
    state: State<'_, AppState>,
) -> Result<PendingApproval, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() || prompt.len() > 2000 {
        return Err("Web queries must contain 1–2,000 characters.".into());
    }
    let provider = state.store.lock().unwrap().settings()?.search_provider;
    let approval = PendingApproval {
        id: uuid::Uuid::new_v4().to_string(),
        prompt,
        provider,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
        approved: false,
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
    state.audit("web.approval.requested", &approval.id);
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
            .ok_or("This approval no longer exists. Review the web query again.")?;
        if !current(approval) || approval.provider != provider {
            return Err(
                "This approval expired or its provider changed. Review the query again.".into(),
            );
        }
        approval.approved = true;
        approval.clone()
    };
    state.audit("web.approval.granted", &id);
    Ok(approval)
}
pub fn consume(
    state: &AppState,
    id: Option<&str>,
    prompt: &str,
    provider: &str,
) -> Result<(), String> {
    let id =
        id.ok_or("Review and approve the exact web query before sending it to a search provider.")?;
    let approval = state
        .approvals
        .lock()
        .unwrap()
        .remove(id)
        .ok_or("This approval has already been used or no longer exists.")?;
    validate(&approval, prompt, provider)?;
    state.audit("web.approval.consumed", id);
    Ok(())
}
fn validate(approval: &PendingApproval, prompt: &str, provider: &str) -> Result<(), String> {
    if !approval.approved
        || !current(approval)
        || approval.prompt != prompt
        || approval.provider != provider
    {
        return Err("A current approval for this exact query and provider is required.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_is_exact_scoped_and_expires() {
        let mut a = PendingApproval {
            id: "one".into(),
            prompt: "news".into(),
            provider: "brave".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
            approved: false,
        };
        assert!(validate(&a, "news", "brave").is_err());
        a.approved = true;
        assert!(validate(&a, "news", "brave").is_ok());
        assert!(validate(&a, "private data", "brave").is_err());
        assert!(validate(&a, "news", "tavily").is_err());
        a.expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
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
        let approval = PendingApproval {
            id: "one-use".into(),
            prompt: "news".into(),
            provider: "brave".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
            approved: true,
        };
        state
            .approvals
            .lock()
            .unwrap()
            .insert(approval.id.clone(), approval);
        consume(&state, Some("one-use"), "news", "brave").unwrap();
        assert!(consume(&state, Some("one-use"), "news", "brave").is_err());
    }
}
