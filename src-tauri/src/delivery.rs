//! Durable timers and an inspectable journal for scheduled work. A claimed timer
//! is never retried automatically: a crash after dispatch has an unknown outcome.
use crate::{controller::AppState, store::Store};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

pub const LIMITATION: &str = "Checks run while Buntini is open and your Mac is awake. Closing the window keeps Buntini running. With desktop notifications on, macOS also holds each timer's alert, so it can appear after you quit Buntini. Sleep still delays alerts, and overdue work runs once when Buntini returns or your Mac wakes.";

/// Timer alerts handed to macOS as well, so one can appear while Buntini is
/// quit. Each claim, cancel and erase withdraws the request before acting, so a
/// timer alerts once.
pub trait OsNotifier: Sync {
    fn schedule(&self, identifier: &str, seconds: u64, title: &str, body: &str);
    /// Stops alerts that have not appeared yet.
    fn remove_pending(&self, identifiers: &[String]);
    /// Stops pending alerts and clears shown ones from Notification Center.
    fn remove(&self, identifiers: &[String]);
    fn remove_all(&self);
}

pub(crate) fn os_identifier(timer_id: &str) -> String {
    format!("timer-{timer_id}")
}

#[cfg(not(test))]
pub(crate) fn os() -> &'static dyn OsNotifier {
    &crate::notifications::SystemNotifier
}
#[cfg(test)]
pub(crate) fn os() -> &'static dyn OsNotifier {
    &fake::ThreadNotifier
}

fn timer_body(seconds: u64) -> String {
    format!(
        "Your {} timer is done.",
        crate::skills::mac::spoken(seconds)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerRecord {
    pub id: String,
    pub seconds: u64,
    pub created_at: String,
    pub due_at: String,
    pub state: String,
    pub claimed_at: Option<String>,
    pub finished_at: Option<String>,
    pub late_seconds: i64,
    pub delivery_state: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorRun {
    pub id: String,
    pub monitor_key: String,
    pub scheduled_for: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub state: String,
    pub late_seconds: i64,
    pub error: Option<String>,
    pub delivery_state: Option<String>,
    pub delivery_error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatus {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub enabled: bool,
    pub next_run_at: String,
    pub overdue_seconds: i64,
    pub last_run: Option<MonitorRun>,
    pub last_success_at: Option<String>,
    pub last_delivery_error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundStatus {
    pub execution_mode: String,
    pub limitation: String,
    /// What macOS reports for Open at login, not just the saved setting.
    pub open_at_login: crate::login::LoginStatus,
    pub timers: Vec<TimerRecord>,
    pub monitors: Vec<MonitorStatus>,
}

pub fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS timers (id TEXT PRIMARY KEY, due_at TEXT NOT NULL, state TEXT NOT NULL, data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS timers_due ON timers(state, due_at);
        CREATE TABLE IF NOT EXISTS monitor_runs (id TEXT PRIMARY KEY, monitor_key TEXT NOT NULL, started_at TEXT NOT NULL, state TEXT NOT NULL, data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS monitor_intents (id TEXT PRIMARY KEY, tool TEXT NOT NULL, args TEXT NOT NULL, monitor_id TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS control_intents (id TEXT PRIMARY KEY, tool TEXT NOT NULL, args TEXT NOT NULL, object_id TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS monitor_runs_key ON monitor_runs(monitor_key, started_at);")
        .map_err(|e| e.to_string())?;
    // Called once at process startup, before any scheduler can claim work.
    let mut statement = db
        .prepare("SELECT data FROM timers WHERE state='claimed'")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    for data in records {
        let mut timer: TimerRecord = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        timer.state = "delivery_unknown".into();
        timer.error = Some("Buntini closed during this timer's alert. It may already have appeared; the alert was not repeated.".into());
        write_timer(db, &timer)?;
    }
    let mut statement = db
        .prepare("SELECT data FROM monitor_runs")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    for data in records {
        let mut run: MonitorRun = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        let interrupted = run.state == "running";
        let delivery_unknown = run.delivery_state.as_deref() == Some("pending");
        if interrupted {
            run.state = "interrupted".into();
            run.finished_at = Some(Utc::now().to_rfc3339());
            run.error = Some("Buntini closed during this check. The last successful result is preserved; use Check now to retry.".into());
        }
        if delivery_unknown {
            run.delivery_state = Some("unknown".into());
            run.delivery_error = Some("Buntini closed while sending this update. The alert may already have appeared and was not repeated; inspect the saved result.".into());
        }
        if interrupted || delivery_unknown {
            write_run(db, &run)?;
        }
    }
    Ok(())
}
fn write_timer(db: &Connection, timer: &TimerRecord) -> Result<(), String> {
    db.execute(
        "INSERT OR REPLACE INTO timers(id,due_at,state,data) VALUES(?1,?2,?3,?4)",
        params![
            timer.id,
            timer.due_at,
            timer.state,
            serde_json::to_string(timer).map_err(|e| e.to_string())?
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
fn write_run(db: &Connection, run: &MonitorRun) -> Result<(), String> {
    db.execute("INSERT OR REPLACE INTO monitor_runs(id,monitor_key,started_at,state,data) VALUES(?1,?2,?3,?4,?5)", params![run.id, run.monitor_key, run.started_at, run.state, serde_json::to_string(run).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn timers(store: &Store) -> Result<Vec<TimerRecord>, String> {
    let mut statement = store.db.prepare("SELECT data FROM timers ORDER BY CASE WHEN state IN ('pending','claimed') THEN 0 ELSE 1 END, due_at DESC LIMIT 100").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.map(|row| {
        serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    })
    .collect()
}
pub fn timer(store: &Store, id: &str) -> Result<Option<TimerRecord>, String> {
    let data: Option<String> = store
        .db
        .query_row("SELECT data FROM timers WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| error.to_string())?;
    data.map(|data| serde_json::from_str(&data).map_err(|error| error.to_string()))
        .transpose()
}
pub fn schedule(store: &Store, seconds: u64, at: DateTime<Utc>) -> Result<TimerRecord, String> {
    if !(1..=86400).contains(&seconds) {
        return Err("Set a timer between one second and 24 hours.".into());
    }
    let count: u32 = store
        .db
        .query_row(
            "SELECT count(*) FROM timers WHERE state IN ('pending','claimed')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count >= 20 {
        return Err("You already have 20 timers. Cancel one before adding another.".into());
    }
    let timer = TimerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        seconds,
        created_at: at.to_rfc3339(),
        due_at: (at + chrono::Duration::seconds(seconds as i64)).to_rfc3339(),
        state: "pending".into(),
        claimed_at: None,
        finished_at: None,
        late_seconds: 0,
        delivery_state: None,
        error: None,
    };
    write_timer(&store.db, &timer)?;
    if store.settings()?.notifications_enabled {
        os().schedule(
            &os_identifier(&timer.id),
            seconds,
            "Timer",
            &timer_body(seconds),
        );
    }
    Ok(timer)
}
pub fn cancel(store: &Store, id: Option<&str>) -> Result<usize, String> {
    let transaction = store
        .db
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let mut cancelled = vec![];
    for mut timer in timers(store)? {
        if timer.state == "pending" && id.is_none_or(|id| id == timer.id) {
            timer.state = "cancelled".into();
            timer.finished_at = Some(Utc::now().to_rfc3339());
            write_timer(&store.db, &timer)?;
            cancelled.push(os_identifier(&timer.id));
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    os().remove(&cancelled);
    Ok(cancelled.len())
}
/// Keeps macOS's copies in step with the timers and the notification setting:
/// future pending timers are (re)registered when notifications are on and
/// withdrawn when they are off. Past-due timers are left to the claim.
pub fn sync_os_requests(store: &Store, at: DateTime<Utc>) -> Result<(), String> {
    let enabled = store.settings()?.notifications_enabled;
    let mut withdrawn = vec![];
    for timer in timers(store)?.into_iter().filter(|t| t.state == "pending") {
        let remaining = DateTime::parse_from_rfc3339(&timer.due_at)
            .map(|due| (due.with_timezone(&Utc) - at).num_seconds())
            .unwrap_or(0);
        if enabled && remaining > 0 {
            // The same identifier replaces any earlier copy instead of adding one.
            os().schedule(
                &os_identifier(&timer.id),
                remaining as u64,
                "Timer",
                &timer_body(timer.seconds),
            );
        } else if !enabled {
            withdrawn.push(os_identifier(&timer.id));
        }
    }
    os().remove_pending(&withdrawn);
    Ok(())
}
/// After a quit, a past-due timer whose macOS alert is still in Notification
/// Center already alerted the person: record that instead of alerting again.
/// The rest stay pending and the claim delivers them now, marked late.
pub fn reconcile(
    store: &Store,
    delivered: &std::collections::HashSet<String>,
    at: DateTime<Utc>,
) -> Result<Vec<TimerRecord>, String> {
    let transaction = store
        .db
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let mut settled = vec![];
    for mut timer in timers(store)? {
        let due = DateTime::parse_from_rfc3339(&timer.due_at)
            .map(|due| due.with_timezone(&Utc) <= at)
            .unwrap_or(false);
        if timer.state == "pending" && due && delivered.contains(&os_identifier(&timer.id)) {
            timer.state = "completed".into();
            timer.finished_at = Some(at.to_rfc3339());
            timer.delivery_state = Some("os_scheduled".into());
            write_timer(&store.db, &timer)?;
            settled.push(timer);
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(settled)
}
/// Delete everything also clears every Buntini alert macOS still holds,
/// scheduled or shown, since their text came from the erased data.
pub fn erase_os_requests() {
    os().remove_all();
}
fn lateness(due: &str, at: DateTime<Utc>) -> i64 {
    DateTime::parse_from_rfc3339(due)
        .map(|due| (at - due.with_timezone(&Utc)).num_seconds().max(0))
        .unwrap_or(0)
}
/// The write lock and transaction give each timer one delivery claim across
/// independent database connections. No external effect happens in this function.
fn claim_due(store: &Store, at: DateTime<Utc>) -> Result<Option<TimerRecord>, String> {
    let transaction = store
        .db
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    // Obtain SQLite's write lock before reading a candidate, avoiding upgrade races.
    store
        .db
        .execute(
            "UPDATE timers SET state=state WHERE state='pending' AND 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    let data: Option<String> = store.db.query_row("SELECT data FROM timers WHERE state='pending' AND julianday(due_at)<=julianday(?1) ORDER BY julianday(due_at),id LIMIT 1", [at.to_rfc3339()], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    let timer = data
        .map(|data| -> Result<_, String> {
            let mut timer: TimerRecord = serde_json::from_str(&data).map_err(|e| e.to_string())?;
            timer.state = "claimed".into();
            timer.claimed_at = Some(at.to_rfc3339());
            timer.late_seconds = lateness(&timer.due_at, at);
            write_timer(&store.db, &timer)?;
            Ok(timer)
        })
        .transpose()?;
    transaction.commit().map_err(|e| e.to_string())?;
    if let Some(timer) = &timer {
        // Shown copies stay: the claim checks them before alerting again.
        os().remove_pending(&[os_identifier(&timer.id)]);
    }
    Ok(timer)
}
fn finish_timer(
    store: &Store,
    mut timer: TimerRecord,
    delivery: Result<String, String>,
) -> Result<TimerRecord, String> {
    timer.finished_at = Some(Utc::now().to_rfc3339());
    match delivery {
        Ok(state) => {
            timer.state = "completed".into();
            timer.delivery_state = Some(state);
        }
        Err(error) => {
            timer.state = "notification_failed".into();
            timer.delivery_state = Some("failed".into());
            timer.error = Some(error);
        }
    }
    let exists: bool = store
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM timers WHERE id=?1 AND state='claimed')",
            [&timer.id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("The timer was removed before its delivery finished.".into());
    }
    write_timer(&store.db, &timer)?;
    // Bound completed history while preserving every active timer.
    store.db.execute("DELETE FROM timers WHERE state NOT IN ('pending','claimed') AND id NOT IN (SELECT id FROM timers ORDER BY due_at DESC LIMIT 100)", []).map_err(|e| e.to_string())?;
    Ok(timer)
}
pub fn monitor_started(
    store: &Store,
    key: &str,
    scheduled_for: Option<&str>,
    at: DateTime<Utc>,
) -> Result<String, String> {
    let run = MonitorRun {
        id: uuid::Uuid::new_v4().to_string(),
        monitor_key: key.into(),
        scheduled_for: scheduled_for.map(str::to_string),
        started_at: at.to_rfc3339(),
        finished_at: None,
        state: "running".into(),
        late_seconds: scheduled_for.map(|due| lateness(due, at)).unwrap_or(0),
        error: None,
        delivery_state: None,
        delivery_error: None,
    };
    write_run(&store.db, &run)?;
    Ok(run.id)
}
pub fn monitor_finished(
    store: &Store,
    run_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let data: String = store
        .db
        .query_row(
            "SELECT data FROM monitor_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut run: MonitorRun = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    run.state = status.into();
    run.finished_at = Some(Utc::now().to_rfc3339());
    run.error = error.map(str::to_string);
    write_run(&store.db, &run)?;
    // Keep the last success and delivery attempt even through a long run of
    // failures/quiet checks so they stay useful in the status view.
    store.db.execute("DELETE FROM monitor_runs WHERE monitor_key=?1 AND state!='running'
        AND id NOT IN (SELECT id FROM monitor_runs WHERE monitor_key=?1 ORDER BY started_at DESC LIMIT 100)
        AND id NOT IN (SELECT id FROM monitor_runs WHERE monitor_key=?1 AND state='completed' ORDER BY started_at DESC LIMIT 1)
        AND id NOT IN (SELECT id FROM monitor_runs WHERE monitor_key=?1 AND json_extract(data,'$.deliveryState') IS NOT NULL ORDER BY started_at DESC LIMIT 1)", [&run.monitor_key]).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn monitor_delivery(
    store: &Store,
    key: &str,
    result: Result<&str, &str>,
) -> Result<(), String> {
    let data: Option<String> = store
        .db
        .query_row(
            "SELECT data FROM monitor_runs WHERE monitor_key=?1 ORDER BY started_at DESC LIMIT 1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(data) = data {
        let mut run: MonitorRun = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        match result {
            Ok(state) => {
                run.delivery_state = Some(state.into());
                run.delivery_error = None;
            }
            Err(error) => {
                run.delivery_state = Some("failed".into());
                run.delivery_error = Some(error.into());
            }
        }
        write_run(&store.db, &run)?;
    }
    Ok(())
}
fn monitor_status(
    store: &Store,
    id: String,
    kind: &str,
    label: String,
    enabled: bool,
    next_run_at: String,
) -> Result<MonitorStatus, String> {
    let key = format!("{kind}-{id}");
    let last: Option<String> = store
        .db
        .query_row(
            "SELECT data FROM monitor_runs WHERE monitor_key=?1 ORDER BY started_at DESC LIMIT 1",
            [&key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let last_run = last
        .map(|data| serde_json::from_str::<MonitorRun>(&data).map_err(|e| e.to_string()))
        .transpose()?;
    let success: Option<String> = store.db.query_row("SELECT data FROM monitor_runs WHERE monitor_key=?1 AND state='completed' ORDER BY started_at DESC LIMIT 1", [&key], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    let last_success_at = success
        .map(|data| {
            serde_json::from_str::<MonitorRun>(&data)
                .map(|run| run.finished_at)
                .map_err(|e| e.to_string())
        })
        .transpose()?
        .flatten();
    let delivery: Option<String> = store.db.query_row("SELECT data FROM monitor_runs WHERE monitor_key=?1 AND json_extract(data,'$.deliveryState') IS NOT NULL ORDER BY started_at DESC LIMIT 1", [&key], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    let last_delivery_error = delivery
        .map(|data| {
            serde_json::from_str::<MonitorRun>(&data)
                .map(|run| run.delivery_error)
                .map_err(|e| e.to_string())
        })
        .transpose()?
        .flatten();
    Ok(MonitorStatus {
        id,
        kind: kind.into(),
        label,
        enabled,
        overdue_seconds: if enabled {
            lateness(&next_run_at, Utc::now())
        } else {
            0
        },
        next_run_at,
        last_run,
        last_success_at,
        last_delivery_error,
    })
}
pub fn status(store: &Store) -> Result<BackgroundStatus, String> {
    let mut monitors = vec![];
    for search in store.saved_searches()? {
        monitors.push(monitor_status(
            store,
            search.id,
            "saved-search",
            search.query,
            search.enabled,
            search.next_run_at,
        )?);
    }
    for watch in store.price_watches()? {
        monitors.push(monitor_status(
            store,
            watch.id,
            "price-watch",
            watch.label,
            watch.enabled,
            watch.next_run_at,
        )?);
    }
    Ok(BackgroundStatus {
        execution_mode: "app_open_and_awake".into(),
        limitation: LIMITATION.into(),
        open_at_login: crate::login::status(),
        timers: timers(store)?,
        monitors,
    })
}
#[tauri::command]
pub fn background_status(state: State<'_, AppState>) -> Result<BackgroundStatus, String> {
    status(&state.store.lock().unwrap())
}
#[tauri::command]
pub fn list_timers(state: State<'_, AppState>) -> Result<Vec<TimerRecord>, String> {
    timers(&state.store.lock().unwrap())
}
#[tauri::command]
pub fn cancel_timer(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if cancel(&state.store.lock().unwrap(), Some(&id))? == 0 {
        return Err("This timer has already finished, started alerting, or been cancelled.".into());
    }
    Ok(())
}
pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        {
            // Before the first claim: settle timers macOS already showed while
            // Buntini was quit, then re-register the future ones.
            let delivered = crate::notifications::delivered_identifiers().await;
            let state = app.state::<AppState>();
            let store = state.store.lock().unwrap();
            if let Err(error) = reconcile(&store, &delivered, Utc::now())
                .and_then(|_| sync_os_requests(&store, Utc::now()))
            {
                let _ = store.audit("timer.reconcile_failed", &error);
            }
        }
        loop {
            let state = app.state::<AppState>();
            let next = claim_due(&state.store.lock().unwrap(), Utc::now());
            match next {
                Ok(Some(timer)) => {
                    let length = crate::skills::mac::spoken(timer.seconds);
                    let message = if timer.late_seconds >= 30 {
                        format!("Your {length} timer finished {} seconds ago. This alert arrived late because Buntini could not deliver it at the scheduled time.", timer.late_seconds)
                    } else {
                        format!("Your {length} timer is done.")
                    };
                    // The claim withdrew macOS's pending copy; one already shown
                    // counts as the desktop alert, so it is not repeated.
                    let shown = crate::notifications::delivered_identifiers()
                        .await
                        .contains(&os_identifier(&timer.id));
                    // Claim is committed before either externally observable effect.
                    let app_result = app.emit("timer-done", &message).map_err(|e| {
                        format!("The timer finished but its in-app alert failed: {e}")
                    });
                    let desktop = if shown {
                        Ok("os_scheduled".to_string())
                    } else {
                        crate::notifications::deliver_timer(&app, &timer.id, "Timer", &message)
                            .await
                    };
                    let delivery = match (app_result, desktop) {
                        (Ok(()), Ok(channel)) => Ok(channel),
                        (Err(_error), Ok(channel))
                            if channel == "desktop_accepted" || channel == "os_scheduled" =>
                        {
                            Ok(channel)
                        }
                        (Err(error), _) => Err(error),
                        (_, Err(error)) => Err(error),
                    };
                    let final_timer = finish_timer(&state.store.lock().unwrap(), timer, delivery);
                    match final_timer {
                        Ok(timer) => {
                            let _ = app.emit("timer-updated", timer);
                        }
                        Err(error) => state.audit("timer.persistence_failed", &error),
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => state.audit("timer.claim_failed", &error),
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// A stand-in for macOS's notification center, one per test thread so tests
/// running in parallel never see each other's requests.
#[cfg(test)]
pub(crate) mod fake {
    use std::{cell::RefCell, collections::BTreeMap};
    thread_local! {
        static PENDING: RefCell<BTreeMap<String, u64>> = const { RefCell::new(BTreeMap::new()) };
        static SCHEDULED: RefCell<usize> = const { RefCell::new(0) };
    }
    pub struct ThreadNotifier;
    impl super::OsNotifier for ThreadNotifier {
        fn schedule(&self, identifier: &str, seconds: u64, _title: &str, body: &str) {
            assert!(!body.is_empty());
            PENDING.with(|p| p.borrow_mut().insert(identifier.into(), seconds));
            SCHEDULED.with(|count| *count.borrow_mut() += 1);
        }
        fn remove_pending(&self, identifiers: &[String]) {
            PENDING.with(|p| p.borrow_mut().retain(|id, _| !identifiers.contains(id)));
        }
        fn remove(&self, identifiers: &[String]) {
            self.remove_pending(identifiers);
        }
        fn remove_all(&self) {
            PENDING.with(|p| p.borrow_mut().clear());
        }
    }
    /// Requests macOS would still show, with their delay in seconds.
    pub fn pending() -> BTreeMap<String, u64> {
        PENDING.with(|p| p.borrow().clone())
    }
    pub fn scheduled() -> usize {
        SCHEDULED.with(|count| *count.borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notifying(store: &Store) {
        let mut settings = store.settings().unwrap();
        settings.notifications_enabled = true;
        store.save_settings(&settings).unwrap();
    }
    #[test]
    fn scheduling_registers_one_os_request_and_claim_removes_it() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        notifying(&store);
        let before = fake::scheduled();
        let timer = schedule(&store, 90, at()).unwrap();
        assert_eq!(fake::scheduled(), before + 1, "one OS request per timer");
        assert_eq!(fake::pending().get(&os_identifier(&timer.id)), Some(&90));
        let claimed = claim_due(&store, at() + chrono::Duration::seconds(90))
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, timer.id);
        assert!(
            !fake::pending().contains_key(&os_identifier(&timer.id)),
            "the in-app claim withdraws the OS copy before alerting"
        );
        let cancelled = schedule(&store, 60, at()).unwrap();
        assert!(fake::pending().contains_key(&os_identifier(&cancelled.id)));
        cancel(&store, Some(&cancelled.id)).unwrap();
        assert!(!fake::pending().contains_key(&os_identifier(&cancelled.id)));
    }
    #[test]
    fn timers_without_notifications_stay_in_app() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let timer = schedule(&store, 60, at()).unwrap();
        assert!(!fake::pending().contains_key(&os_identifier(&timer.id)));
        notifying(&store);
        sync_os_requests(&store, at() + chrono::Duration::seconds(20)).unwrap();
        assert_eq!(
            fake::pending().get(&os_identifier(&timer.id)),
            Some(&40),
            "turning notifications on registers the time that is left"
        );
        let mut settings = store.settings().unwrap();
        settings.notifications_enabled = false;
        store.save_settings(&settings).unwrap();
        sync_os_requests(&store, at() + chrono::Duration::seconds(30)).unwrap();
        assert!(!fake::pending().contains_key(&os_identifier(&timer.id)));
    }
    #[test]
    fn reconcile_does_not_realert_a_delivered_timer() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        notifying(&store);
        let shown = schedule(&store, 60, at()).unwrap();
        let missed = schedule(&store, 60, at()).unwrap();
        let future = schedule(&store, 3600, at()).unwrap();
        let later = at() + chrono::Duration::minutes(10);
        let delivered = [os_identifier(&shown.id), os_identifier(&future.id)]
            .into_iter()
            .collect();
        let settled = reconcile(&store, &delivered, later).unwrap();
        assert_eq!(
            settled.len(),
            1,
            "only a past-due timer macOS showed is settled"
        );
        let record = timer(&store, &shown.id).unwrap().unwrap();
        assert_eq!(record.state, "completed");
        assert_eq!(record.delivery_state.as_deref(), Some("os_scheduled"));
        // The one macOS never showed is still claimed and alerted now, marked late.
        let claimed = claim_due(&store, later).unwrap().unwrap();
        assert_eq!(claimed.id, missed.id);
        assert_eq!(claimed.late_seconds, 540);
        assert!(claim_due(&store, later).unwrap().is_none());
        assert_eq!(timer(&store, &future.id).unwrap().unwrap().state, "pending");
    }
    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    #[test]
    fn timers_survive_restart_and_catch_up_once_without_repeating_unknown_alerts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delivery.sqlite");
        let store = Store::open(&path).unwrap();
        initialize(&store.db).unwrap();
        let timer = schedule(&store, 60, at()).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        initialize(&store.db).unwrap();
        assert_eq!(timers(&store).unwrap()[0].id, timer.id);
        assert!(claim_due(&store, at() + chrono::Duration::seconds(59))
            .unwrap()
            .is_none());
        let claimed = claim_due(&store, at() + chrono::Duration::seconds(600))
            .unwrap()
            .unwrap();
        assert_eq!(claimed.late_seconds, 540);
        assert!(claim_due(&store, at() + chrono::Duration::seconds(601))
            .unwrap()
            .is_none());
        drop(store);
        let store = Store::open(&path).unwrap();
        initialize(&store.db).unwrap();
        assert_eq!(timers(&store).unwrap()[0].state, "delivery_unknown");
        assert!(claim_due(&store, at() + chrono::Duration::hours(1))
            .unwrap()
            .is_none());
    }
    #[test]
    fn cancelled_and_finished_timers_never_reappear_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delivery.sqlite");
        let store = Store::open(&path).unwrap();
        initialize(&store.db).unwrap();
        let first = schedule(&store, 60, at()).unwrap();
        schedule(&store, 120, at()).unwrap();
        assert_eq!(cancel(&store, Some(&first.id)).unwrap(), 1);
        let due = claim_due(&store, at() + chrono::Duration::minutes(3))
            .unwrap()
            .unwrap();
        finish_timer(
            &store,
            due,
            Err("Notifications are denied in System Settings.".into()),
        )
        .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        initialize(&store.db).unwrap();
        assert!(claim_due(&store, at() + chrono::Duration::hours(1))
            .unwrap()
            .is_none());
        assert!(timers(&store)
            .unwrap()
            .iter()
            .any(|timer| timer.state == "notification_failed" && timer.error.is_some()));
        assert_eq!(cancel(&store, None).unwrap(), 0);
    }
    #[test]
    fn separate_connections_claim_a_timer_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delivery.sqlite");
        let first = Store::open(&path).unwrap();
        initialize(&first.db).unwrap();
        let second = Store::open(&path).unwrap();
        initialize(&second.db).unwrap();
        schedule(&first, 1, at()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let first_barrier = barrier.clone();
        let a = std::thread::spawn(move || {
            first_barrier.wait();
            claim_due(&first, at() + chrono::Duration::seconds(2))
                .unwrap()
                .is_some()
        });
        let b = std::thread::spawn(move || {
            barrier.wait();
            claim_due(&second, at() + chrono::Duration::seconds(2))
                .unwrap()
                .is_some()
        });
        assert_eq!(
            usize::from(a.join().unwrap()) + usize::from(b.join().unwrap()),
            1
        );
    }
    #[test]
    fn monitor_journal_exposes_missed_runs_cancellation_and_last_success() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        initialize(&store.db).unwrap();
        let key = "saved-search-news";
        let first = monitor_started(
            &store,
            key,
            Some(&at().to_rfc3339()),
            at() + chrono::Duration::hours(2),
        )
        .unwrap();
        monitor_finished(&store, &first, "completed", None).unwrap();
        monitor_delivery(&store, key, Err("Desktop notifications are denied.")).unwrap();
        let before = monitor_status(
            &store,
            "news".into(),
            "saved-search",
            "News".into(),
            true,
            at().to_rfc3339(),
        )
        .unwrap();
        assert_eq!(before.last_run.unwrap().late_seconds, 7200);
        assert_eq!(
            before.last_delivery_error.as_deref(),
            Some("Desktop notifications are denied.")
        );
        let second = monitor_started(&store, key, None, at() + chrono::Duration::hours(3)).unwrap();
        monitor_finished(&store, &second, "cancelled", Some("Cancelled by you.")).unwrap();
        let status = monitor_status(
            &store,
            "news".into(),
            "saved-search",
            "News".into(),
            true,
            (at() + chrono::Duration::days(1)).to_rfc3339(),
        )
        .unwrap();
        assert_eq!(status.last_run.unwrap().state, "cancelled");
        assert!(status.last_success_at.is_some());
        assert_eq!(
            status.last_delivery_error.as_deref(),
            Some("Desktop notifications are denied."),
            "quiet/cancelled checks must not hide a notification failure"
        );
        monitor_started(&store, key, None, at() + chrono::Duration::hours(4)).unwrap();
        initialize(&store.db).unwrap();
        let status = monitor_status(
            &store,
            "news".into(),
            "saved-search",
            "News".into(),
            true,
            at().to_rfc3339(),
        )
        .unwrap();
        assert_eq!(status.last_run.unwrap().state, "interrupted");
        assert!(status.last_success_at.is_some());
    }
    #[test]
    fn claimed_deliveries_survive_interruption_without_resending_and_deleted_data_stays_deleted() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let run_id = monitor_started(&store, "saved-search-one", None, at()).unwrap();
        monitor_finished(&store, &run_id, "completed", None).unwrap();
        monitor_delivery(&store, "saved-search-one", Ok("pending")).unwrap();
        initialize(&store.db).unwrap();
        let status = monitor_status(
            &store,
            "one".into(),
            "saved-search",
            "One".into(),
            true,
            at().to_rfc3339(),
        )
        .unwrap();
        assert_eq!(
            status.last_run.unwrap().delivery_state.as_deref(),
            Some("unknown")
        );
        assert!(status.last_delivery_error.is_some());
        schedule(&store, 1, at()).unwrap();
        let timer = claim_due(&store, at() + chrono::Duration::seconds(2))
            .unwrap()
            .unwrap();
        store.db.execute("DELETE FROM timers", []).unwrap();
        assert!(finish_timer(&store, timer, Ok("desktop_accepted".into())).is_err());
        assert!(timers(&store).unwrap().is_empty());
    }

    #[test]
    fn timer_limits_and_successful_delivery_are_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.sqlite");
        let store = Store::open(&path).unwrap();
        assert!(schedule(&store, 0, at()).is_err());
        assert!(schedule(&store, 86401, at()).is_err());
        for _ in 0..20 {
            schedule(&store, 1, at()).unwrap();
        }
        assert!(schedule(&store, 1, at()).is_err());
        let timer = claim_due(&store, at() + chrono::Duration::seconds(1))
            .unwrap()
            .unwrap();
        finish_timer(&store, timer, Ok("in_app_only".into())).unwrap();
        assert_eq!(cancel(&store, None).unwrap(), 19);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(claim_due(&store, at() + chrono::Duration::days(1))
            .unwrap()
            .is_none());
        assert!(timers(&store)
            .unwrap()
            .iter()
            .any(|timer| timer.state == "completed"
                && timer.delivery_state.as_deref() == Some("in_app_only")));
    }
}
