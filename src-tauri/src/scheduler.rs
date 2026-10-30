use crate::{
    controller::{web_tool, AppState},
    store::Store,
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

fn save_started(
    store: &Store,
    search: &SavedSearch,
    task: &TaskRecord,
    scheduled_for: Option<&str>,
) -> Result<String, String> {
    let transaction = store
        .db
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if scheduled_for.is_some() {
        store.assign_scheduled_search_conversation(task, search)?;
    }
    store.save_task(task)?;
    store.save_search(search)?;
    let run_id = crate::delivery::monitor_started(
        store,
        &format!("saved-search-{}", search.id),
        scheduled_for,
        chrono::Utc::now(),
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(run_id)
}

/// Keep the task and its monitor baseline in the same commit. Cancelled checks
/// remain in history, but never replace the last result used for change detection.
fn save_finished(
    store: &Store,
    id: &str,
    task: &TaskRecord,
    run_id: Option<&str>,
) -> Result<bool, String> {
    let transaction = store
        .db
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let Some(mut current) = store.saved_search(id)? else {
        if let Some(run_id) = run_id {
            crate::delivery::monitor_finished(
                store,
                run_id,
                "cancelled",
                Some("The saved search was deleted."),
            )?;
        }
        store.delete_task(&task.id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(false);
    };
    store.save_task(task)?;
    if let Some(run_id) = run_id {
        crate::delivery::monitor_finished(
            store,
            run_id,
            &task.state,
            (task.state != "completed").then_some(task.answer.as_str()),
        )?;
    }
    if task.state != "cancelled" {
        current.last_result_task_id = Some(task.id.clone());
        current.last_error = (task.state == "error").then(|| task.answer.clone());
        store.save_search(&current)?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(true)
}

async fn run(app: &tauri::AppHandle, id: &str, scheduled: bool) -> Result<TaskRecord, String> {
    let state = app.state::<AppState>();
    let mut search = {
        let store = state.store.lock().unwrap();
        store.saved_search(id)?.ok_or("Saved search not found.")?
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
        research_report: None,
        prices: vec![],
        cards: vec![],
        backend: String::new(),
        touched: vec![],
        handoff: None,
        action: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let active_key = format!("saved-search-{id}");
    let settings;
    let previous;
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
        previous = search
            .last_result_task_id
            .as_deref()
            .map(|id| store.task(id))
            .transpose()?
            .flatten();
        if scheduled
            && (!search.enabled
                || !chrono::DateTime::parse_from_rfc3339(&search.next_run_at)
                    .map(|date| date <= chrono::Utc::now())
                    .unwrap_or(false))
        {
            return Err("This saved search is paused or no longer due.".into());
        }
        let scheduled_for = scheduled.then(|| search.next_run_at.clone());
        search.next_run_at = next_run(search.interval_minutes);
        active.insert(active_key.clone(), cancel.clone());
        save_started(&store, &search, &task, scheduled_for.as_deref())
    };
    let run_id = match saved {
        Ok(run_id) => run_id,
        Err(error) => {
            state.active.lock().unwrap().remove(&active_key);
            return Err(error);
        }
    };
    let _claim = crate::notifications::ActiveMonitor::new(&state, active_key, cancel.clone());
    let result = web_tool(&state, &settings, &search.query, cancel.clone()).await;
    if cancel.load(Ordering::Relaxed) {
        task.state = "cancelled".into();
        task.answer = "Saved search cancelled.".into();
    } else {
        match result {
            Ok((sources, warning)) => {
                if let Some(warning) = warning {
                    let _ = tauri::Emitter::emit(app, "budget-warning", warning.clone());
                    crate::notifications::deliver(app, "Search budget", &warning).await;
                }
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
    let persisted = save_finished(&state.store.lock().unwrap(), id, &task, Some(&run_id));
    if !persisted? {
        return Err("This saved search was deleted while running.".into());
    }
    state.audit("saved_search.ran", &format!("{id}: {}", task.state));
    let _ = tauri::Emitter::emit(
        app,
        "background-updated",
        serde_json::json!({"monitorKey":format!("saved-search-{id}")}),
    );
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
    crate::delivery::start(app.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            if !state.active.lock().unwrap().is_empty() {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
            crate::routines::run_due(&app).await;
            crate::nudges::run_due(&app).await;
            // The sleep's clock stops while the Mac sleeps; a wake cuts it short.
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
}

static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Checks for due searches, price watches and routines now instead of at the next tick.
/// Timers need no nudge: their claim loop looks every second.
pub fn tick_now() {
    WAKE.notify_one();
}

/// Catches up as soon as the Mac wakes, so overdue checks and timers do not
/// wait out a sleep-stretched tick. Call once, on the main thread.
pub fn observe_wake() {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
        use objc2_foundation::NSNotification;
        let block = block2::RcBlock::new(|_: std::ptr::NonNull<NSNotification>| tick_now());
        let observer = unsafe {
            NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceDidWakeNotification),
                    None,
                    None,
                    &block,
                )
        };
        // Observed for the app's whole life.
        std::mem::forget(observer);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn search() -> SavedSearch {
        SavedSearch {
            id: "search".into(),
            query: "news".into(),
            interval_minutes: 60,
            enabled: true,
            created_at: now(),
            next_run_at: now(),
            last_result_task_id: Some("previous".into()),
            last_error: None,
        }
    }

    fn task(state: &str) -> TaskRecord {
        serde_json::from_value(serde_json::json!({
            "id": "check", "prompt": "news", "answer": "result", "state": state,
            "createdAt": now(), "sources": [], "files": []
        }))
        .unwrap()
    }

    #[test]
    fn saved_search_interval_bounds() {
        assert!(validate("news", 14).is_err());
        assert!(validate("", 15).is_err());
        assert!(validate("news", 15).is_ok());
        assert!(validate("news", 525601).is_err());
    }

    #[test]
    fn cancelled_checks_preserve_the_last_monitor_baseline() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.save_search(&search()).unwrap();
        let task = task("cancelled");
        assert!(save_finished(&store, "search", &task, None).unwrap());
        assert_eq!(store.task("check").unwrap().unwrap().state, "cancelled");
        assert_eq!(
            store
                .saved_search("search")
                .unwrap()
                .unwrap()
                .last_result_task_id
                .as_deref(),
            Some("previous")
        );
    }

    #[test]
    fn search_results_and_baselines_commit_together() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut search = search();
        store.save_search(&search).unwrap();
        store.save_task(&task("running")).unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_monitor_update BEFORE INSERT ON saved_searches BEGIN SELECT RAISE(ABORT, 'disk failure'); END;").unwrap();
        assert!(save_finished(&store, "search", &task("completed"), None).is_err());
        assert_eq!(
            store.task("check").unwrap().unwrap().state,
            "running",
            "failed baseline writes must roll back the completed result"
        );
        assert_eq!(
            store
                .saved_search("search")
                .unwrap()
                .unwrap()
                .last_result_task_id
                .as_deref(),
            Some("previous")
        );
        store
            .db
            .execute_batch("DROP TRIGGER reject_monitor_update;")
            .unwrap();
        // A pause or reschedule during the fetch must survive the result commit.
        search.enabled = false;
        search.next_run_at = "2030-01-01T00:00:00Z".into();
        store.save_search(&search).unwrap();
        assert!(save_finished(&store, "search", &task("completed"), None).unwrap());
        let saved = store.saved_search("search").unwrap().unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.next_run_at, "2030-01-01T00:00:00Z");
        assert_eq!(saved.last_result_task_id.as_deref(), Some("check"));
        store.delete_search("search").unwrap();
        assert!(!save_finished(&store, "search", &task("completed"), None).unwrap());
        assert!(store.task("check").unwrap().is_none());
    }

    #[test]
    fn failed_search_claims_never_leave_orphan_running_tasks() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_monitor_insert BEFORE INSERT ON saved_searches BEGIN SELECT RAISE(ABORT, 'disk failure'); END;").unwrap();
        assert!(save_started(&store, &search(), &task("running"), None).is_err());
        assert!(store.task("check").unwrap().is_none());
    }
    #[test]
    fn schedule_task_and_journal_claim_roll_back_together() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let original = search();
        store.save_search(&original).unwrap();
        let mut advanced = original.clone();
        advanced.next_run_at = "2030-01-01T00:00:00Z".into();
        store.db.execute_batch("CREATE TRIGGER fail_run_journal BEFORE INSERT ON monitor_runs BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(save_started(
            &store,
            &advanced,
            &task("running"),
            Some(&original.next_run_at)
        )
        .is_err());
        assert!(store.task("check").unwrap().is_none());
        assert!(store.conversation_summary("saved-search:search").is_err());
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT COUNT(*) FROM conversation_turns WHERE task_id='check'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store.saved_search("search").unwrap().unwrap().next_run_at,
            original.next_run_at
        );
    }

    #[test]
    fn recurring_results_stay_in_their_search_thread_while_manual_checks_use_the_selected_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        let store = Store::open(&path).unwrap();
        let personal = store.active_conversation_id().unwrap();
        let mut question = task("completed");
        question.id = "personal-question".into();
        store.save_task(&question).unwrap();
        let search = search();
        let mut scheduled = task("running");
        scheduled.id = "scheduled-first".into();
        let run = save_started(&store, &search, &scheduled, Some(&search.next_run_at)).unwrap();
        scheduled.state = "completed".into();
        save_finished(&store, &search.id, &scheduled, Some(&run)).unwrap();
        let search_thread = store.task_conversation(&scheduled.id).unwrap();
        assert_ne!(search_thread, personal);
        assert_eq!(store.active_conversation_id().unwrap(), personal);
        assert_eq!(
            store
                .conversation_page(&personal, None, 100)
                .unwrap()
                .turns
                .len(),
            1
        );
        assert!(store
            .conversation_history_for_task(&question.id)
            .unwrap()
            .is_empty());
        let other = store.new_conversation().unwrap().conversation.id;
        drop(store);
        let store = Store::open(&path).unwrap();
        scheduled.id = "scheduled-second".into();
        scheduled.state = "running".into();
        save_started(&store, &search, &scheduled, Some(&search.next_run_at)).unwrap();
        assert_eq!(
            store.task_conversation(&scheduled.id).unwrap(),
            search_thread
        );
        assert_eq!(store.active_conversation_id().unwrap(), other);
        assert!(store
            .conversation_page(&other, None, 100)
            .unwrap()
            .turns
            .is_empty());
        let mut different_search = search.clone();
        different_search.id = "another-search".into();
        scheduled.id = "another-search-result".into();
        save_started(
            &store,
            &different_search,
            &scheduled,
            Some(&search.next_run_at),
        )
        .unwrap();
        assert_ne!(
            store.task_conversation(&scheduled.id).unwrap(),
            search_thread
        );
        let mut manual = task("running");
        manual.id = "check-now".into();
        let run = save_started(&store, &search, &manual, None).unwrap();
        manual.state = "completed".into();
        save_finished(&store, &search.id, &manual, Some(&run)).unwrap();
        assert_eq!(store.task_conversation(&manual.id).unwrap(), other);
        assert!(store
            .conversation_history_for_task(&manual.id)
            .unwrap()
            .is_empty());
        assert_eq!(store.active_conversation_id().unwrap(), other);
    }
}
