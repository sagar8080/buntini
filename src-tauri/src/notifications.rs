#[cfg(target_os = "macos")]
mod native {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSettings, UNUserNotificationCenter,
    };
    use std::{ptr::NonNull, sync::Mutex};

    pub async fn request() -> Result<bool, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        {
            let block = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(error.is_null() && granted.as_bool());
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                    &block,
                );
        }
        tokio::time::timeout(std::time::Duration::from_secs(60), rx)
            .await
            .map_err(|_| "Notification permission request timed out.".to_string())?
            .map_err(|_| "Notification permission response was lost.".into())
    }
    pub async fn status() -> Result<String, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        {
            let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
                let status = unsafe { settings.as_ref() }.authorizationStatus().0;
                let text = match status {
                    0 => "notDetermined",
                    1 => "denied",
                    2 => "authorized",
                    3 => "provisional",
                    _ => "unknown",
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(text.to_string());
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .getNotificationSettingsWithCompletionHandler(&block);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .map_err(|_| "Notification status check timed out.".to_string())?
            .map_err(|_| "Notification status response was lost.".into())
    }
    pub fn enqueue(title: &str, body: &str) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(title));
            content.setBody(&NSString::from_str(body));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&uuid::Uuid::new_v4().to_string()),
                &content,
                None,
            );
            let block = RcBlock::new(move |error: *mut NSError| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _=tx.send(if error.is_null(){Ok(())}else{Err("macOS did not accept the notification. Review Buntini's notification permissions in System Settings.".to_string())});
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&block));
        }
        rx
    }
}

use crate::{controller::AppState, store::Store};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Keep cancellation reachable until every result event and native enqueue has
/// finished. A paused/deleted monitor must never leave a detached delivery tail.
pub(crate) struct ActiveMonitor<'a> {
    state: &'a AppState,
    key: String,
    token: Arc<AtomicBool>,
}
impl<'a> ActiveMonitor<'a> {
    pub fn new(state: &'a AppState, key: String, token: Arc<AtomicBool>) -> Self {
        Self { state, key, token }
    }
}
impl Drop for ActiveMonitor<'_> {
    fn drop(&mut self) {
        let mut active = self.state.active.lock().unwrap();
        if active
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.token))
        {
            active.remove(&self.key);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Monitor<'a> {
    Search { id: &'a str, scheduled: bool },
    Price { id: &'a str, scheduled: bool },
}
impl Monitor<'_> {
    fn key(self) -> String {
        match self {
            Self::Search { id, .. } => format!("saved-search-{id}"),
            Self::Price { id, .. } => format!("price-watch-{id}"),
        }
    }
    fn authorized(self, store: &Store) -> Result<bool, String> {
        match self {
            Self::Search { id, scheduled } => Ok(store
                .saved_search(id)?
                .is_some_and(|search| !scheduled || search.enabled)),
            Self::Price { id, scheduled } => Ok(store
                .price_watch(id)?
                .is_some_and(|watch| !scheduled || watch.enabled)),
        }
    }
}

fn with_monitor_gate<T>(
    state: &AppState,
    monitor: Monitor<'_>,
    token: &Arc<AtomicBool>,
    dispatch: impl FnOnce(&Store) -> T,
) -> Result<Option<T>, String> {
    // This is the same lock order as monitor claims and pause/delete. Dispatch
    // is synchronous: revocation either wins this gate or follows its enqueue.
    let active = state.active.lock().unwrap();
    if token.load(Ordering::SeqCst)
        || !active
            .get(&monitor.key())
            .is_some_and(|current| Arc::ptr_eq(current, token))
    {
        return Ok(None);
    }
    let store = state.store.lock().unwrap();
    if !monitor.authorized(&store)? || token.load(Ordering::SeqCst) {
        return Ok(None);
    }
    Ok(Some(dispatch(&store)))
}

#[cfg(target_os = "macos")]
async fn finish_delivery(
    state: &AppState,
    pending: tokio::sync::oneshot::Receiver<Result<(), String>>,
) {
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .map_err(|_| "Notification delivery timed out.".to_string())
        .and_then(|reply| reply.map_err(|_| "Notification response was lost.".to_string()))
        .and_then(|result| result);
    state.audit(
        if result.is_ok() {
            "notification.sent"
        } else {
            "notification.failed"
        },
        result
            .err()
            .as_deref()
            .unwrap_or("Meaningful monitor update delivered"),
    );
}

pub(crate) async fn dispatch_monitor(
    app: &tauri::AppHandle,
    monitor: Monitor<'_>,
    token: &Arc<AtomicBool>,
    event: &str,
    payload: serde_json::Value,
    notification: Option<(&str, &str)>,
) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let state = app.state::<AppState>();
    let pending = with_monitor_gate(&state, monitor, token, |store| {
        let _ = app.emit(event, payload);
        #[cfg(target_os = "macos")]
        return notification.and_then(|(title, body)| {
            store
                .settings()
                .ok()
                .filter(|settings| settings.notifications_enabled)
                .map(|_| {
                    native::enqueue(
                        &title.chars().take(100).collect::<String>(),
                        &body.chars().take(400).collect::<String>(),
                    )
                })
        });
        #[cfg(not(target_os = "macos"))]
        let _ = (store, notification);
    })?;
    #[cfg(target_os = "macos")]
    if let Some(Some(pending)) = pending {
        finish_delivery(&state, pending).await;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pending;
    Ok(())
}
#[tauri::command]
pub async fn request_notification_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    return native::request().await;
    #[cfg(not(target_os = "macos"))]
    Err("Desktop notifications in this build require macOS.".into())
}
#[tauri::command]
pub async fn notification_permission_status() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    return native::status().await;
    #[cfg(not(target_os = "macos"))]
    Ok("unknown".into())
}
pub async fn deliver(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri::Manager;
    let state = app.state::<crate::controller::AppState>();
    let enabled = state
        .store
        .lock()
        .unwrap()
        .settings()
        .map(|settings| settings.notifications_enabled)
        .unwrap_or(false);
    if !enabled {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let pending = native::enqueue(
            &title.chars().take(100).collect::<String>(),
            &body.chars().take(400).collect::<String>(),
        );
        finish_delivery(&state, pending).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{now, SavedSearch};

    fn state(folder: &std::path::Path) -> AppState {
        AppState::new(folder.join("data"), folder.join("cache"), folder.into()).unwrap()
    }
    fn search() -> SavedSearch {
        SavedSearch {
            id: "search".into(),
            query: "approved query".into(),
            interval_minutes: 15,
            enabled: true,
            created_at: now(),
            next_run_at: now(),
            last_result_task_id: None,
            last_error: None,
        }
    }

    #[test]
    fn monitor_delivery_rechecks_claim_existence_pause_and_cancellation() {
        let folder = tempfile::tempdir().unwrap();
        let state = state(folder.path());
        let token = Arc::new(AtomicBool::new(false));
        let scheduled = Monitor::Search {
            id: "search",
            scheduled: true,
        };
        let manual = Monitor::Search {
            id: "search",
            scheduled: false,
        };
        let mut search = search();
        state.store.lock().unwrap().save_search(&search).unwrap();
        state
            .active
            .lock()
            .unwrap()
            .insert(scheduled.key(), token.clone());
        assert_eq!(
            with_monitor_gate(&state, scheduled, &token, |_| 1).unwrap(),
            Some(1)
        );
        search.enabled = false;
        state.store.lock().unwrap().save_search(&search).unwrap();
        assert_eq!(
            with_monitor_gate(&state, scheduled, &token, |_| 1).unwrap(),
            None
        );
        // Explicit Check now can run a paused search; Pause during that run
        // still revokes its token and therefore suppresses delivery.
        assert_eq!(
            with_monitor_gate(&state, manual, &token, |_| 1).unwrap(),
            Some(1)
        );
        token.store(true, Ordering::SeqCst);
        assert_eq!(
            with_monitor_gate(&state, manual, &token, |_| 1).unwrap(),
            None
        );
        let replacement = Arc::new(AtomicBool::new(false));
        assert_eq!(
            with_monitor_gate(&state, manual, &replacement, |_| 1).unwrap(),
            None
        );
        state
            .active
            .lock()
            .unwrap()
            .insert(manual.key(), replacement.clone());
        state.store.lock().unwrap().delete_search("search").unwrap();
        assert_eq!(
            with_monitor_gate(&state, manual, &replacement, |_| 1).unwrap(),
            None
        );
        let price = Monitor::Price {
            id: "missing",
            scheduled: false,
        };
        state
            .active
            .lock()
            .unwrap()
            .insert(price.key(), replacement.clone());
        assert_eq!(
            with_monitor_gate(&state, price, &replacement, |_| 1).unwrap(),
            None
        );
    }

    #[test]
    fn monitor_revocation_and_enqueue_share_one_atomic_gate() {
        let folder = tempfile::tempdir().unwrap();
        let state = state(folder.path());
        let token = Arc::new(AtomicBool::new(false));
        let monitor = Monitor::Search {
            id: "search",
            scheduled: true,
        };
        state.store.lock().unwrap().save_search(&search()).unwrap();
        state
            .active
            .lock()
            .unwrap()
            .insert(monitor.key(), token.clone());
        let claim = ActiveMonitor::new(&state, monitor.key(), token.clone());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let (revoked_tx, revoked_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let delivery_state = &state;
            let delivery_token = &token;
            let delivery = scope.spawn(move || {
                with_monitor_gate(delivery_state, monitor, delivery_token, |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .unwrap()
            });
            entered_rx.recv().unwrap();
            let revoke = scope.spawn(|| {
                attempt_tx.send(()).unwrap();
                let _active = state.active.lock().unwrap();
                token.store(true, Ordering::SeqCst);
                state.store.lock().unwrap().delete_search("search").unwrap();
                revoked_tx.send(()).unwrap();
            });
            attempt_rx.recv().unwrap();
            assert!(revoked_rx
                .recv_timeout(std::time::Duration::from_millis(30))
                .is_err());
            release_tx.send(()).unwrap();
            assert_eq!(delivery.join().unwrap(), Some(()));
            revoke.join().unwrap();
        });
        assert!(state.active.lock().unwrap().contains_key(&monitor.key()));
        assert_eq!(
            with_monitor_gate(&state, monitor, &token, |_| ()).unwrap(),
            None
        );
        drop(claim);
        assert!(state.active.lock().unwrap().is_empty());
    }
}
