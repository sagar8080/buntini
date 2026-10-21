use crate::{
    controller::{web_tool, AppState},
    types::*,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager, State};

pub fn validate(query: &str, interval: u32) -> Result<(), String> {
    if query.trim().is_empty() || query.len() > 2000 {
        return Err("Enter a saved web query of up to 2,000 characters.".into());
    }
    if !(15..=525600).contains(&interval) {
        return Err("Saved search intervals must be between 15 minutes and one year.".into());
    }
    Ok(())
}
fn next_run(interval: u32) -> String {
    (chrono::Utc::now() + chrono::Duration::minutes(i64::from(interval))).to_rfc3339()
}
#[tauri::command]
pub fn save_search(
    query: String,
    interval_minutes: u32,
    state: State<'_, AppState>,
) -> Result<SavedSearch, String> {
    validate(&query, interval_minutes)?;
    let store = state.store.lock().unwrap();
    if store.saved_searches()?.len() >= 20 {
        return Err("You can save up to 20 searches. Delete one before adding another.".into());
    }
    let search = SavedSearch {
        id: uuid::Uuid::new_v4().to_string(),
        query: query.trim().into(),
        interval_minutes,
        enabled: true,
        created_at: now(),
        next_run_at: next_run(interval_minutes),
        last_result_task_id: None,
        last_error: None,
    };
    store.save_search(&search)?;
    store.audit(
        "saved_search.allowed",
        &format!(
            "{} every {} minutes; explicit user authorization for repeated web searches",
            search.id, interval_minutes
        ),
    )?;
    Ok(search)
}
#[tauri::command]
pub fn update_saved_search(
    id: String,
    enabled: bool,
    interval_minutes: u32,
    state: State<'_, AppState>,
) -> Result<SavedSearch, String> {
    let active = state.active.lock().unwrap();
    if !enabled {
        if let Some(cancel) = active.get(&format!("saved-search-{id}")) {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    let store = state.store.lock().unwrap();
    let mut search = store.saved_search(&id)?.ok_or("Saved search not found.")?;
    validate(&search.query, interval_minutes)?;
    search.enabled = enabled;
    search.interval_minutes = interval_minutes;
    search.next_run_at = next_run(interval_minutes);
    store.save_search(&search)?;
    store.audit("saved_search.updated", &format!("{id}: enabled={enabled}"))?;
    Ok(search)
}
#[tauri::command]
pub fn delete_saved_search(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let active = state.active.lock().unwrap();
    if let Some(cancel) = active.get(&format!("saved-search-{id}")) {
        cancel.store(true, Ordering::SeqCst);
    }
    state.store.lock().unwrap().delete_search(&id)
}

fn sources_equal(a: &TaskRecord, b: &TaskRecord) -> bool {
    a.state == b.state
        && a.sources.len() == b.sources.len()
        && a.sources
            .iter()
            .zip(&b.sources)
            .all(|(a, b)| a.url == b.url && a.title == b.title && a.snippet == b.snippet)
        && (a.state != "error" || a.answer == b.answer)
}
async fn run(app: &tauri::AppHandle, id: &str, scheduled: bool) -> Result<TaskRecord, String> {
    let state = app.state::<AppState>();
    let (mut search, previous) = {
        let store = state.store.lock().unwrap();
        let s = store.saved_search(id)?.ok_or("Saved search not found.")?;
        let previous = s
            .last_result_task_id
            .as_deref()
            .map(|id| store.task(id))
            .transpose()?
            .flatten();
        (s, previous)
    };
    validate(&search.query, search.interval_minutes)?;
    let mut task = TaskRecord {
        id: uuid::Uuid::new_v4().to_string(),
        prompt: search.query.clone(),
        answer: String::new(),
        state: "running".into(),
        created_at: now(),
        sources: Vec::new(),
        files: Vec::new(),
        system: None,
        research: vec![],
        prices: vec![],
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let active_key = format!("saved-search-{id}");
    let settings;
    let saved = {
        let mut active = state.active.lock().unwrap();
        if !active.is_empty() {
            return Err("Another task is running. Try this search again after it finishes.".into());
        }
        let store = state.store.lock().unwrap();
        settings = store.settings()?;
        // Re-read under the claim lock: a queued search may have been paused,
        // rescheduled, or deleted while an earlier search was awaiting network.
        search = store.saved_search(id)?.ok_or("Saved search not found.")?;
        if scheduled
            && (!search.enabled
                || !chrono::DateTime::parse_from_rfc3339(&search.next_run_at)
                    .map(|date| date <= chrono::Utc::now())
                    .unwrap_or(false))
        {
            return Err("This saved search is paused or no longer due.".into());
        }
        search.next_run_at = next_run(search.interval_minutes);
        active.insert(active_key.clone(), cancel.clone());
        store
            .save_task(&task)
            .and_then(|_| store.save_search(&search))
    };
    if let Err(e) = saved {
        state.active.lock().unwrap().remove(&active_key);
        return Err(e);
    }
    let _claim = crate::notifications::ActiveMonitor::new(&state, active_key, cancel.clone());
    let result = web_tool(&state, &settings, &search.query, cancel.clone()).await;
    if cancel.load(Ordering::Relaxed) {
        task.state = "cancelled".into();
        task.answer = "Saved search cancelled.".into();
    } else {
        match result {
            Ok(sources) => {
                task.state = "completed".into();
                task.answer = if sources.is_empty() {
                    "No results were returned for this saved search.".into()
                } else {
                    format!("Saved search found {} sources. Open a source to inspect the latest details.",sources.len())
                };
                task.sources = sources;
            }
            Err(error) => {
                task.state = "error".into();
                task.answer = error;
            }
        }
    }
    let persisted = (|| -> Result<bool, String> {
        let store = state.store.lock().unwrap();
        let current = store.saved_search(id)?;
        if current.is_none() {
            store
                .db
                .execute("DELETE FROM tasks WHERE id=?1", [&task.id])
                .map_err(|e| e.to_string())?;
            return Ok(false);
        }
        let result = store.save_task(&task);
        if let Some(mut current) = current {
            current.last_result_task_id = Some(task.id.clone());
            current.last_error = if task.state == "error" {
                Some(task.answer.clone())
            } else {
                None
            };
            let _ = store.save_search(&current);
        }
        result.map(|_| true)
    })();
    if !persisted? {
        return Err("This saved search was deleted while running.".into());
    }
    state.audit("saved_search.ran", &format!("{id}: {}", task.state));
    if !cancel.load(Ordering::Relaxed)
        && previous
            .as_ref()
            .map(|p| !sources_equal(p, &task))
            .unwrap_or(true)
    {
        crate::notifications::dispatch_monitor(
            app,
            crate::notifications::Monitor::Search { id, scheduled },
            &cancel,
            "saved-search-result",
            serde_json::json!({"searchId":id,"task":task}),
            Some((
                "Saved search update",
                if task.state == "error" {
                    "A saved search needs attention. Open Buntini to review it."
                } else {
                    "A saved search has new results. Open Buntini to review them."
                },
            )),
        )
        .await?;
    }
    Ok(task)
}
#[tauri::command]
pub async fn run_saved_search(id: String, app: tauri::AppHandle) -> Result<TaskRecord, String> {
    run(&app, &id, false).await
}
pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let state = app.state::<AppState>();
            if !state.active.lock().unwrap().is_empty() {
                continue;
            }
            let due = state
                .store
                .lock()
                .unwrap()
                .saved_searches()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| {
                    s.enabled
                        && chrono::DateTime::parse_from_rfc3339(&s.next_run_at)
                            .map(|d| d <= chrono::Utc::now())
                            .unwrap_or(false)
                })
                .map(|s| s.id)
                .collect::<Vec<_>>();
            for id in due {
                let _ = run(&app, &id, true).await;
            }
            let price_due = state
                .store
                .lock()
                .unwrap()
                .price_watches()
                .unwrap_or_default()
                .into_iter()
                .filter(|watch| {
                    watch.enabled
                        && chrono::DateTime::parse_from_rfc3339(&watch.next_run_at)
                            .map(|date| date <= chrono::Utc::now())
                            .unwrap_or(false)
                })
                .map(|watch| watch.id)
                .collect::<Vec<_>>();
            for id in price_due {
                let _ = crate::watches::run(&app, &id, true).await;
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_search_interval_bounds() {
        assert!(validate("news", 14).is_err());
        assert!(validate("", 15).is_err());
        assert!(validate("news", 15).is_ok());
        assert!(validate("news", 525601).is_err());
    }
}
