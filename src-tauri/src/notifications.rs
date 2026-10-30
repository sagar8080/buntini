#[cfg(target_os = "macos")]
// Tests swap the OS notifier for a fake, leaving the scheduling calls unused there.
#[cfg_attr(test, allow(dead_code))]
mod native {
    use block2::RcBlock;
    use objc2::{define_class, msg_send, rc::Retained, runtime::Bool, AnyThread};
    use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
        UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
        UNNotificationDefaultActionIdentifier, UNNotificationRequest, UNNotificationResponse,
        UNNotificationSettings, UNTimeIntervalNotificationTrigger, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::{ptr::NonNull, sync::Mutex};

    /// macOS throws (and takes the app down) when a process that isn't inside an .app
    /// bundle asks for the notification center, as `tauri dev` builds do.
    fn bundled() -> bool {
        std::env::current_exe()
            .is_ok_and(|path| path.to_string_lossy().contains(".app/Contents/MacOS/"))
    }
    const UNBUNDLED: &str =
        "Desktop notifications work in the installed app, not in a development build.";

    pub async fn request() -> Result<bool, String> {
        if !bundled() {
            return Err(UNBUNDLED.into());
        }
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
        if !bundled() {
            return Ok("unavailable".into());
        }
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
    fn content(
        title: &str,
        body: &str,
        category: Option<&str>,
    ) -> Retained<UNMutableNotificationContent> {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        if let Some(category) = category {
            content.setCategoryIdentifier(&NSString::from_str(category));
        }
        content
    }
    pub fn enqueue(title: &str, body: &str) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        enqueue_as(&uuid::Uuid::new_v4().to_string(), None, title, body)
    }
    /// A request with a known identifier replaces an earlier one with the same
    /// identifier, so one timer or approval never stacks two alerts.
    pub fn enqueue_as(
        identifier: &str,
        category: Option<&str>,
        title: &str,
        body: &str,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if !bundled() {
            let _ = tx.send(Err(UNBUNDLED.into()));
            return rx;
        }
        let tx = Mutex::new(Some(tx));
        {
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(identifier),
                &content(title, body, category),
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
    /// Hands macOS an alert to show after `seconds`, even if Buntini has quit by
    /// then. The timer row stays the record; this copy is best effort.
    pub fn schedule(identifier: &str, seconds: u64, title: &str, body: &str) {
        if !bundled() {
            return;
        }
        let trigger = UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(
            seconds.max(1) as f64,
            false,
        );
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(identifier),
            &content(title, body, None),
            Some(&trigger),
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
    fn identifiers(ids: &[String]) -> Retained<NSArray<NSString>> {
        let ids: Vec<_> = ids.iter().map(|id| NSString::from_str(id)).collect();
        NSArray::from_retained_slice(&ids)
    }
    pub fn remove_pending(ids: &[String]) {
        if bundled() && !ids.is_empty() {
            UNUserNotificationCenter::currentNotificationCenter()
                .removePendingNotificationRequestsWithIdentifiers(&identifiers(ids));
        }
    }
    pub fn remove(ids: &[String]) {
        if bundled() && !ids.is_empty() {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let ids = identifiers(ids);
            center.removePendingNotificationRequestsWithIdentifiers(&ids);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
        }
    }
    pub fn remove_all() {
        if bundled() {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            center.removeAllPendingNotificationRequests();
            center.removeAllDeliveredNotifications();
        }
    }
    /// Identifiers of the scheduled alerts macOS has not shown yet.
    pub async fn pending() -> Result<Vec<String>, String> {
        if !bundled() {
            return Ok(vec![]);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        {
            let block = RcBlock::new(move |list: NonNull<NSArray<UNNotificationRequest>>| {
                let ids = unsafe { list.as_ref() }
                    .iter()
                    .map(|request| request.identifier().to_string())
                    .collect();
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(ids);
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .getPendingNotificationRequestsWithCompletionHandler(&block);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .map_err(|_| "Reading pending notifications timed out.".to_string())?
            .map_err(|_| "The pending notification list was lost.".into())
    }
    /// Identifiers of the alerts still listed in Notification Center.
    pub async fn delivered() -> Result<Vec<String>, String> {
        if !bundled() {
            return Ok(vec![]);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        {
            let block = RcBlock::new(move |list: NonNull<NSArray<UNNotification>>| {
                let ids = unsafe { list.as_ref() }
                    .iter()
                    .map(|notification| notification.request().identifier().to_string())
                    .collect();
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(ids);
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .getDeliveredNotificationsWithCompletionHandler(&block);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .map_err(|_| "Reading delivered notifications timed out.".to_string())?
            .map_err(|_| "The delivered notification list was lost.".into())
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
        #[unsafe(super(NSObject))]
        #[name = "BuntiniNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion: &block2::DynBlock<dyn Fn()>,
            ) {
                let action = response.actionIdentifier();
                // Clicking the banner itself counts as Review. Later and dismiss do nothing.
                let review = action.to_string() == super::REVIEW
                    || &*action == unsafe { UNNotificationDefaultActionIdentifier };
                if review {
                    super::review_requested(
                        &response.notification().request().identifier().to_string(),
                    );
                }
                completion.call(());
            }
        }
    );

    /// Registers the approval category and the delegate that turns Review into an
    /// in-app event. Neither action approves anything.
    pub fn install() {
        if !bundled() {
            return;
        }
        let review = UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str(super::REVIEW),
            &NSString::from_str("Review"),
            UNNotificationActionOptions::Foreground,
        );
        let later = UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str("later"),
            &NSString::from_str("Later"),
            UNNotificationActionOptions::empty(),
        );
        let category =
            UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                &NSString::from_str(super::APPROVAL_CATEGORY),
                &NSArray::from_retained_slice(&[review, later]),
                &NSArray::from_retained_slice(&[]),
                UNNotificationCategoryOptions::empty(),
            );
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
        let delegate: Retained<Delegate> =
            unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
        center.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        // The center keeps only a weak reference; the delegate lives as long as the app.
        std::mem::forget(delegate);
    }
}

pub(crate) const APPROVAL_CATEGORY: &str = "buntini.approval";
const REVIEW: &str = "review";
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Call once at startup, before macOS can deliver a notification response.
pub fn install(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
    #[cfg(target_os = "macos")]
    native::install();
}

/// The identifier of an approval's notification. Only the approval id travels
/// with it, so the app looks the proposal up again before showing anything.
pub(crate) fn approval_identifier(approval_id: &str) -> String {
    format!("approval-{approval_id}")
}

/// The proposal a Review click asked for, until the panel takes it. A click that
/// launches a quit Buntini arrives before the panel can hear the event, so the
/// panel also asks for it once it has loaded.
static REVIEW_REQUEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The proposal to review, once: the live event and the panel's own check on
/// loading both lead here, so the sheet opens a single time.
#[tauri::command]
pub fn take_review_request() -> Option<String> {
    REVIEW_REQUEST.lock().unwrap().take()
}

/// Review only opens Buntini on the saved proposal; approving still happens in
/// the sheet. Runs on the main thread without making macOS wait.
fn review_requested(identifier: &str) {
    let Some(approval_id) = identifier.strip_prefix("approval-") else {
        return;
    };
    *REVIEW_REQUEST.lock().unwrap() = Some(approval_id.to_string());
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let approval_id = approval_id.to_string();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        use tauri::{Emitter, Manager};
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        let _ = handle.emit(
            "approval-open",
            serde_json::json!({ "approvalId": approval_id }),
        );
    });
}

/// Lock-screen safe wording: the kind of work only, never a recipient, title,
/// time or text from the proposal, because the body shows on the lock screen.
pub(crate) fn approval_body(tool: &str) -> &'static str {
    match tool {
        "draft_email" => "Buntini drafted an email and needs your OK.",
        "add_reminder" => "Buntini prepared a reminder and needs your OK.",
        "add_calendar_event" => "Buntini prepared a calendar event and needs your OK.",
        "create_note" => "Buntini wrote a note and needs your OK.",
        "run_shortcut" => "Buntini wants to run a shortcut and needs your OK.",
        "open_app" | "open_link" => "Buntini wants to open something for you and needs your OK.",
        "control_music" => "Buntini wants to change your music and needs your OK.",
        "switch_tab" => "Buntini wants to switch a browser tab and needs your OK.",
        "remember" => "Buntini wants to remember something and needs your OK.",
        "set_timer" => "Buntini wants to set a timer and needs your OK.",
        "create_saved_search" | "create_price_watch" => {
            "Buntini set up a recurring check and needs your OK."
        }
        "browse_page" => "Buntini wants to read a web page and needs your OK.",
        "look_up_online" => "Buntini wants to look something up online and needs your OK.",
        _ => "A task is waiting for your OK.",
    }
}

/// When a task stops at a proposal while Buntini is hidden or tucked away, say
/// so right away. Returns whether the task is waiting, so callers skip the
/// generic "Task finished" alert.
pub(crate) async fn announce_waiting(
    app: &tauri::AppHandle,
    task: &crate::types::TaskRecord,
) -> bool {
    use tauri::Manager;
    if !["waiting_action", "waiting_online"].contains(&task.state.as_str()) {
        return false;
    }
    let unobtrusive = app
        .get_webview_window("main")
        .map(|window| !window.is_visible().unwrap_or(true) || !crate::overlay::is_expanded())
        .unwrap_or(true);
    if !unobtrusive {
        return true;
    }
    let state = app.state::<AppState>();
    let step = state
        .store
        .lock()
        .unwrap()
        .workflow(&task.id)
        .ok()
        .flatten()
        .and_then(|flow| {
            flow.steps
                .last()
                .filter(|step| step.state == "waiting")
                .cloned()
        });
    if let Some(step) = step {
        if let Err(error) = deliver_approval(app, &step.id, &step.tool).await {
            state.audit("notification.failed", &error);
        }
    }
    true
}

async fn deliver_approval(
    app: &tauri::AppHandle,
    approval_id: &str,
    tool: &str,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    if !state
        .store
        .lock()
        .unwrap()
        .settings()?
        .notifications_enabled
    {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        notification_availability(&native::status().await?)?;
        let pending = native::enqueue_as(
            &approval_identifier(approval_id),
            Some(APPROVAL_CATEGORY),
            "Waiting for you",
            approval_body(tool),
        );
        finish_delivery(&state, pending).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (approval_id, tool);
        Err("Desktop notifications in this build require macOS.".into())
    }
}

/// Clears an approval's alert once the proposal is approved or cancelled, so a
/// stale banner does not linger in Notification Center.
pub(crate) fn withdraw_approval(approval_id: &str) {
    crate::delivery::os().remove(&[approval_identifier(approval_id)]);
}

/// macOS's own notification center, used in the app. Tests use a fake.
#[cfg_attr(test, allow(dead_code))]
pub(crate) struct SystemNotifier;
impl crate::delivery::OsNotifier for SystemNotifier {
    fn schedule(&self, identifier: &str, seconds: u64, title: &str, body: &str) {
        #[cfg(target_os = "macos")]
        native::schedule(identifier, seconds, title, body);
        #[cfg(not(target_os = "macos"))]
        let _ = (identifier, seconds, title, body);
    }
    fn remove_pending(&self, identifiers: &[String]) {
        #[cfg(target_os = "macos")]
        native::remove_pending(identifiers);
        #[cfg(not(target_os = "macos"))]
        let _ = identifiers;
    }
    fn remove(&self, identifiers: &[String]) {
        #[cfg(target_os = "macos")]
        native::remove(identifiers);
        #[cfg(not(target_os = "macos"))]
        let _ = identifiers;
    }
    fn remove_all(&self) {
        #[cfg(target_os = "macos")]
        native::remove_all();
    }
}

/// Identifiers still in Notification Center. Empty when unknown, which can only
/// lead to a repeated alert, never a missed one.
pub(crate) async fn delivered_identifiers() -> std::collections::HashSet<String> {
    #[cfg(target_os = "macos")]
    return native::delivered()
        .await
        .map(|ids| ids.into_iter().collect())
        .unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    std::collections::HashSet::new()
}

/// Scheduled alerts macOS still holds, when it can be trusted: only in the installed
/// app and while alerts are allowed (otherwise macOS may have refused the request, so
/// its absence proves nothing).
pub(crate) async fn pending_identifiers() -> Option<std::collections::HashSet<String>> {
    #[cfg(target_os = "macos")]
    {
        if !matches!(native::status().await.ok()?.as_str(), "authorized" | "provisional") {
            return None;
        }
        return native::pending().await.ok().map(|ids| ids.into_iter().collect());
    }
    #[cfg(not(target_os = "macos"))]
    None
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
) -> Result<(), String> {
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
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("macOS accepted the notification request"),
    );
    result
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
    #[cfg(target_os = "macos")]
    let permission = if notification.is_some() {
        native::status()
            .await
            .and_then(|status| notification_availability(&status))
    } else {
        Ok(())
    };
    let dispatched = with_monitor_gate(&state, monitor, token, |store| -> Result<_, String> {
        crate::delivery::monitor_delivery(store, &monitor.key(), Ok("pending"))?;
        let event_result = app.emit(event, payload).map_err(|error| error.to_string());
        let wants_desktop = notification.is_some()
            && store
                .settings()
                .is_ok_and(|settings| settings.notifications_enabled);
        #[cfg(target_os = "macos")]
        let delivery_error = wants_desktop
            .then(|| permission.as_ref().err().cloned())
            .flatten();
        #[cfg(not(target_os = "macos"))]
        let delivery_error =
            wants_desktop.then(|| "Desktop notifications in this build require macOS.".to_string());
        #[cfg(target_os = "macos")]
        let pending = notification
            .filter(|_| wants_desktop && delivery_error.is_none())
            .map(|(title, body)| {
                native::enqueue(
                    &title.chars().take(100).collect::<String>(),
                    &body.chars().take(400).collect::<String>(),
                )
            });
        #[cfg(not(target_os = "macos"))]
        let pending: Option<tokio::sync::oneshot::Receiver<Result<(), String>>> = None;
        Ok((event_result, pending, delivery_error))
    })?
    .transpose()?;
    if let Some((event_result, pending, delivery_error)) = dispatched {
        #[cfg(target_os = "macos")]
        let result = match pending {
            Some(pending) => finish_delivery(&state, pending)
                .await
                .map(|_| "desktop_accepted"),
            None => event_result.map(|_| "in_app_only"),
        };
        #[cfg(not(target_os = "macos"))]
        let result = {
            let _ = pending;
            event_result.map(|_| "in_app_only")
        };
        let result = delivery_error.map_or(result, Err);
        crate::delivery::monitor_delivery(
            &state.store.lock().unwrap(),
            &monitor.key(),
            result.as_ref().map(|state| *state).map_err(String::as_str),
        )?;
        let _ = app.emit(
            "background-updated",
            serde_json::json!({"monitorKey":monitor.key()}),
        );
    }
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
fn notification_availability(status: &str) -> Result<(), String> {
    match status {
        "authorized" | "provisional" => Ok(()),
        "denied" => Err("Desktop notifications are denied. Allow Buntini in System Settings → Notifications; the result remains in Buntini's history.".into()),
        "notDetermined" => Err("Desktop notification permission has not been granted. Enable notifications in Buntini settings; the result remains in history.".into()),
        _ => Err("Desktop notifications are unavailable in this build. The result remains in Buntini's history.".into()),
    }
}
/// "desktop_accepted" confirms enqueue, not that the person saw the alert.
/// Disabled notifications still leave the durable result and in-app event.
pub(crate) async fn deliver_checked(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<String, String> {
    deliver_with_timer_gate(app, title, body, None).await
}
pub(crate) async fn deliver_timer(
    app: &tauri::AppHandle,
    id: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    deliver_with_timer_gate(app, title, body, Some(id)).await
}
async fn deliver_with_timer_gate(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    timer_id: Option<&str>,
) -> Result<String, String> {
    use tauri::Manager;
    let state = app.state::<crate::controller::AppState>();
    let enabled = state
        .store
        .lock()
        .unwrap()
        .settings()?
        .notifications_enabled;
    if !enabled {
        return Ok("in_app_only".into());
    }
    #[cfg(target_os = "macos")]
    {
        let status = native::status().await?;
        notification_availability(&status)?;
        let pending = {
            let store = state.store.lock().unwrap();
            if !store.settings()?.notifications_enabled {
                return Ok("in_app_only".into());
            }
            if let Some(id) = timer_id {
                let exists: bool = store
                    .db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM timers WHERE id=?1 AND state='claimed')",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                if !exists {
                    return Err(
                        "This timer was removed before its desktop alert could be sent.".into(),
                    );
                }
            }
            // A timer's alert reuses its scheduled identifier, replacing rather
            // than stacking on a copy macOS may have shown.
            let identifier = timer_id
                .map(crate::delivery::os_identifier)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            native::enqueue_as(
                &identifier,
                None,
                &title.chars().take(100).collect::<String>(),
                &body.chars().take(400).collect::<String>(),
            )
        };
        finish_delivery(&state, pending).await?;
        return Ok("desktop_accepted".into());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body, timer_id);
        Err("Desktop notifications in this build require macOS. The result remains in Buntini's history.".into())
    }
}
pub async fn deliver(app: &tauri::AppHandle, title: &str, body: &str) {
    if let Err(error) = deliver_checked(app, title, body).await {
        use tauri::Manager;
        app.state::<AppState>().audit("notification.failed", &error);
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
    fn approval_body_has_no_field_values() {
        let args = serde_json::json!({"to":"ana@example.com","subject":"Divorce papers","body":"The PIN is 4417","title":"Call the clinic","due":"2026-11-02 09:00","url":"https://bank.example/reset"});
        let args = args.as_object().unwrap();
        for tool in crate::actions::TOOLS
            .iter()
            .chain(&crate::control_actions::TOOLS)
            .chain(&crate::monitor_actions::TOOLS)
            .chain(&crate::browser_workspace::TOOLS)
            .chain(&["look_up_online", "unknown_tool"])
        {
            let body = approval_body(tool);
            assert!(body.ends_with("your OK."), "{tool}: {body}");
            for value in args.values().filter_map(|value| value.as_str()) {
                assert!(!body.contains(value), "{tool} leaked {value}");
            }
            if crate::actions::TOOLS.contains(tool) {
                // The sheet shows these values; the lock screen must not.
                let (_, fields, _) = crate::actions::describe(tool, args);
                for field in fields.iter().filter(|field| !field.value.is_empty()) {
                    assert!(
                        !body.contains(&field.value),
                        "{tool} leaked {}",
                        field.value
                    );
                }
            }
        }
        assert_eq!(approval_identifier("abc"), "approval-abc");
    }

    #[test]
    fn denied_or_unknown_notifications_cannot_be_reported_as_delivered() {
        assert!(notification_availability("authorized").is_ok());
        assert!(notification_availability("provisional").is_ok());
        for status in ["denied", "notDetermined", "unknown", "unavailable"] {
            assert!(notification_availability(status).is_err());
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
