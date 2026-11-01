//! Heads-ups: short nudges from data on this Mac, decided by fixed rules rather than a
//! model, so they are instant, explainable and nothing leaves the Mac. There are three
//! kinds: something on the calendar about to start, reminders past due, and people
//! waiting on a reply. A "leave now" heads-up would need to know where the user is,
//! which Buntini never learns, so there is none.
//!
//! Staying out of the way matters more than being early: each item is mentioned once
//! (hashed keys that outlive restarts and clearing history), each kind has its own rate
//! limit, replies and reminders wait for the hours the user is usually around (learned
//! from when they use Buntini, or 8:00–21:00), nothing is said during a Focus Buntini can
//! see, and Settings → Heads-ups is the one switch. Background reads never open Mail.
//!
//! A heads-up only opens a conversation with the context in it. Replying or marking a
//! reminder done is a new request the user sends, through the usual approval sheet.
use crate::{
    apps::{Event, Reminder},
    controller::AppState,
    store::Store,
    triage,
    types::*,
};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Meeting,
    Reminders,
    Reply,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Kind::Meeting => "meeting",
            Kind::Reminders => "reminders",
            Kind::Reply => "reply",
        }
    }
    /// The least time between two heads-ups of this kind.
    fn gap(self) -> Duration {
        match self {
            Kind::Meeting => Duration::minutes(10),
            Kind::Reminders => Duration::hours(3),
            Kind::Reply => Duration::hours(4),
        }
    }
    /// The most heads-ups of this kind in a day.
    fn per_day(self) -> usize {
        match self {
            Kind::Meeting => 8,
            Kind::Reminders => 3,
            Kind::Reply => 2,
        }
    }
}

/// Lock-screen safe: the kind of heads-up only, never a title, name, place or subject.
pub fn body(kind: Kind, minutes: i64, several: bool) -> String {
    match kind {
        Kind::Meeting if minutes <= 1 => "Something on your calendar starts in a minute.".into(),
        Kind::Meeting => format!("Something on your calendar starts in {minutes} minutes."),
        Kind::Reminders if several => "Some reminders are past due.".into(),
        Kind::Reminders => "A reminder is past due.".into(),
        Kind::Reply if several => "A few people are waiting on a reply from you.".into(),
        Kind::Reply => "Someone is waiting on a reply from you.".into(),
    }
}

/// A reminder past due, as the conversation lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DueReminder {
    pub title: String,
    pub due: String,
    pub list: String,
}

/// An email waiting on a reply, numbered as the conversation lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub email: triage::Listed,
    pub reason: String,
}

/// What the heads-up's conversation starts from, read when it was decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Seed {
    #[serde(rename_all = "camelCase")]
    Meeting {
        title: String,
        start: String,
        end: String,
        start_at: String,
        location: String,
        attendees: Vec<String>,
    },
    Reminders { items: Vec<DueReminder> },
    Reply { emails: Vec<Waiting> },
}

/// One heads-up as saved. `title` and `detail` are for the window only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nudge {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    pub created_at: String,
    pub expires_at: String,
    pub seed: Seed,
    #[serde(default)]
    pub task_id: Option<String>,
}

/// What the window lists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    pub created_at: String,
}

/// A heads-up the rules would give now, before rate limits and quiet hours.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    /// One per item it mentions; any one not mentioned before makes it worth saying.
    pub keys: Vec<String>,
    pub title: String,
    pub detail: String,
    pub body: String,
    pub expires: DateTime<Utc>,
    pub seed: Seed,
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS nudges (id TEXT PRIMARY KEY, data TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT NOT NULL, state TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS nudge_marks (key TEXT PRIMARY KEY, kind TEXT NOT NULL, at TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS nudge_sent (kind TEXT NOT NULL, at TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())
}

/// "YYYY-MM-DD HH:MM" on this Mac's clock.
fn local(stamp: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M").ok()?;
    Local.from_local_datetime(&naive).earliest()
}

fn event_start(event: &Event) -> Option<DateTime<Local>> {
    event
        .start_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Local))
        .or_else(|| local(&event.start))
}

fn event_end(event: &Event) -> Option<DateTime<Local>> {
    event
        .end_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Local))
        .or_else(|| local(&event.end))
}

fn clock(at: DateTime<Local>) -> String {
    at.format("%H:%M").to_string()
}

fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// How long before an event its heads-up comes: inside this window, and not when it is
/// about to start anyway.
const LEAD_MINUTES: i64 = 15;
const TOO_LATE_MINUTES: i64 = 2;
/// Blocks this long ("Work", "Out of office") are not meetings to prepare for.
const LONGEST_MEETING_HOURS: i64 = 6;

/// The days whose events the meeting rule reads: today, and tomorrow too when the lead
/// window crosses midnight.
pub fn meeting_days(now: DateTime<Local>) -> (String, Option<String>) {
    let today = now.format("%Y-%m-%d").to_string();
    let ahead = (now + Duration::minutes(LEAD_MINUTES)).format("%Y-%m-%d").to_string();
    (today.clone(), (ahead != today).then_some(ahead))
}

/// The next timed event starting within the lead window.
pub fn meeting(now: DateTime<Local>, events: &[Event]) -> Option<Candidate> {
    let event = events
        .iter()
        .filter(|event| !event.all_day)
        .filter_map(|event| Some((event, event_start(event)?)))
        .filter(|(event, start)| {
            let lead = *start - now;
            lead > Duration::minutes(TOO_LATE_MINUTES)
                && lead <= Duration::minutes(LEAD_MINUTES)
                && event_end(event).is_none_or(|end| end - *start < Duration::hours(LONGEST_MEETING_HOURS))
        })
        .min_by_key(|(_, start)| *start);
    let (event, start) = event?;
    let end = event_end(event).unwrap_or(start + Duration::minutes(30));
    let minutes = ((start - now).num_seconds() + 59) / 60;
    let title = if event.title.trim().is_empty() { "Untitled event".to_string() } else { event.title.trim().to_string() };
    let detail = if !event.location.trim().is_empty() {
        event.location.trim().to_string()
    } else if !event.attendees.is_empty() {
        format!("With {}", list(&event.attendees.iter().take(3).cloned().collect::<Vec<_>>()))
    } else {
        "Starts soon".into()
    };
    Some(Candidate {
        kind: Kind::Meeting,
        keys: vec![format!("meeting|{}|{}", event.id.as_deref().unwrap_or(&title), start.to_rfc3339())],
        title: format!("{title} at {}", clock(start)),
        detail,
        body: body(Kind::Meeting, minutes, false),
        expires: end.with_timezone(&Utc),
        seed: Seed::Meeting {
            title,
            start: clock(start),
            end: clock(end),
            start_at: start.to_rfc3339(),
            location: event.location.trim().to_string(),
            attendees: event.attendees.clone(),
        },
    })
}

/// A timed reminder counts once it is this late, so Reminders' own alert comes first.
const OVERDUE_MINUTES: i64 = 30;
/// A date-only reminder due today counts from this hour.
const DUE_TODAY_HOUR: u32 = 14;
/// Older than this, a reminder is something the user chose to leave, not something missed.
const OVERDUE_DAYS: i64 = 3;

fn past_due(now: DateTime<Local>, reminder: &Reminder) -> bool {
    let due = reminder.due.trim();
    if let Some(at) = local(due) {
        return now - at >= Duration::minutes(OVERDUE_MINUTES) && now - at <= Duration::days(OVERDUE_DAYS);
    }
    let Ok(day) = NaiveDate::parse_from_str(due, "%Y-%m-%d") else {
        return false;
    };
    let today = now.date_naive();
    (day < today && (today - day).num_days() <= OVERDUE_DAYS) || (day == today && now.hour() >= DUE_TODAY_HOUR)
}

fn due_label(due: &str) -> String {
    if let Some(at) = local(due) {
        return at.format("%a %-d %b %H:%M").to_string();
    }
    NaiveDate::parse_from_str(due, "%Y-%m-%d")
        .map(|day| day.format("%a %-d %b").to_string())
        .unwrap_or_else(|_| due.to_string())
}

/// Every open reminder past due, as one heads-up.
pub fn reminders(now: DateTime<Local>, reminders: &[Reminder]) -> Option<Candidate> {
    let due: Vec<&Reminder> = reminders.iter().filter(|reminder| past_due(now, reminder)).collect();
    if due.is_empty() {
        return None;
    }
    let titles: Vec<String> = due.iter().take(3).map(|reminder| reminder.title.trim().to_string()).collect();
    Some(Candidate {
        kind: Kind::Reminders,
        keys: due
            .iter()
            .map(|reminder| format!("reminder|{}|{}", reminder.id.as_deref().unwrap_or(&reminder.title), reminder.due))
            .collect(),
        title: if due.len() == 1 { format!("{} is past due", titles[0]) } else { "Reminders past due".into() },
        detail: list(&titles),
        body: body(Kind::Reminders, 0, due.len() > 1),
        expires: (now + Duration::hours(12)).with_timezone(&Utc),
        seed: Seed::Reminders {
            items: due
                .iter()
                .take(8)
                .map(|reminder| DueReminder {
                    title: reminder.title.trim().to_string(),
                    due: reminder.due.clone(),
                    list: reminder.list.clone(),
                })
                .collect(),
        },
    })
}

/// An email counts as waiting once it has sat this long.
const WAITING_HOURS: i64 = 3;

fn sender_name(sender: &str) -> String {
    match sender.split_once('<') {
        Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"').to_string(),
        _ => sender.trim().to_string(),
    }
}

/// Everyone the cached sort says needs a reply, who has waited a while.
pub fn replies(now: DateTime<Local>, triage: &triage::Triage) -> Option<Candidate> {
    let waiting: Vec<&triage::Item> = triage
        .reply
        .iter()
        .filter(|item| local(&item.header.received).is_some_and(|at| now - at >= Duration::hours(WAITING_HOURS)))
        .collect();
    if waiting.is_empty() {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    for item in &waiting {
        let name = sender_name(&item.header.sender);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Some(Candidate {
        kind: Kind::Reply,
        keys: waiting
            .iter()
            .map(|item| format!("reply|{}", if item.header.message_id.is_empty() { &item.header.id } else { &item.header.message_id }))
            .collect(),
        title: if names.len() == 1 { format!("{} is waiting on a reply", names[0]) } else { "People are waiting on a reply".into() },
        detail: list(&names.iter().take(3).cloned().collect::<Vec<_>>()),
        body: body(Kind::Reply, 0, waiting.len() > 1),
        expires: (now + Duration::hours(12)).with_timezone(&Utc),
        seed: Seed::Reply {
            emails: waiting
                .iter()
                .take(8)
                .enumerate()
                .map(|(index, item)| Waiting {
                    email: triage::Listed {
                        number: index + 1,
                        id: item.header.id.clone(),
                        message_id: item.header.message_id.clone(),
                        sender: item.header.sender.clone(),
                        subject: item.header.subject.clone(),
                        received: item.header.received.clone(),
                    },
                    reason: item.reason.into(),
                })
                .collect(),
        },
    })
}

/// The hours the user is usually around: from wake (inclusive) to bed (exclusive).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Active {
    pub wake: u32,
    pub bed: u32,
}

impl Active {
    pub const DEFAULT: Active = Active { wake: 8, bed: 21 };
    pub fn contains(self, hour: u32) -> bool {
        self.wake <= hour && hour < self.bed
    }
}

/// Fewer requests than this in a month say too little about someone's day.
const ENOUGH_TURNS: usize = 20;

/// Active hours from the local hours of the user's own requests in the last month: the
/// middle 90% of them, kept to a sane day (waking 6–10, quiet from 19–23).
pub fn active_hours(hours: &[u32]) -> Active {
    if hours.len() < ENOUGH_TURNS {
        return Active::DEFAULT;
    }
    let mut sorted = hours.to_vec();
    sorted.sort_unstable();
    let low = sorted[sorted.len() * 5 / 100];
    let high = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
    Active { wake: low.clamp(6, 10), bed: (high + 1).clamp(19, 23) }
}

fn usual_hours(store: &Store, now: DateTime<Utc>) -> Active {
    let since = (now - Duration::days(30)).to_rfc3339();
    let hours = store
        .db
        // Routine runs and goal check-ins carry a prompt too, but they run on their own
        // schedule, not when the user is at the Mac.
        .prepare(
            "SELECT t.created_at FROM tasks t LEFT JOIN conversation_turns c ON c.task_id=t.id
             WHERE t.created_at>=?1 AND COALESCE(json_extract(t.data,'$.prompt'),'')<>''
               AND COALESCE(c.conversation_id,'') NOT LIKE 'routine:%' AND COALESCE(c.conversation_id,'') NOT LIKE 'goal:%'",
        )
        .and_then(|mut statement| {
            statement
                .query_map([since], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Local).hour())
        .collect::<Vec<_>>();
    active_hours(&hours)
}

/// Late morning and mid-afternoon: when a reminder about email is useful rather than
/// another interruption.
pub fn reply_moment(hour: u32, active: Active) -> bool {
    active.contains(hour) && ((10..12).contains(&hour) || (15..17).contains(&hour))
}

/// What has been said already, and when.
pub struct History<'a> {
    pub sent: &'a [(Kind, DateTime<Utc>)],
    pub marked: &'a dyn Fn(&str) -> bool,
}

/// Whether a heads-up the rules found may be given now.
pub fn admit(candidate: &Candidate, now: DateTime<Local>, active: Active, history: &History<'_>) -> bool {
    let hour = now.hour();
    // Something the user is about to attend is worth saying at any hour.
    if candidate.kind != Kind::Meeting && !active.contains(hour) {
        return false;
    }
    if candidate.kind == Kind::Reply && !reply_moment(hour, active) {
        return false;
    }
    if candidate.keys.iter().all(|key| (history.marked)(key)) {
        return false;
    }
    let now = now.with_timezone(&Utc);
    let mine: Vec<&DateTime<Utc>> = history.sent.iter().filter(|(kind, _)| *kind == candidate.kind).map(|(_, at)| at).collect();
    !mine.iter().any(|at| now - **at < candidate.kind.gap())
        && mine.iter().filter(|at| now - ***at < Duration::hours(24)).count() < candidate.kind.per_day()
}

/// Keys are stored hashed, so what was said outlives clearing history without keeping
/// any title, name or id.
fn hashed(key: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(key.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn marked(store: &Store, key: &str) -> bool {
    store
        .db
        .query_row("SELECT 1 FROM nudge_marks WHERE key=?1", [hashed(key)], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

fn sent(store: &Store) -> Vec<(Kind, DateTime<Utc>)> {
    let Ok(mut statement) = store.db.prepare("SELECT kind, at FROM nudge_sent") else {
        return vec![];
    };
    statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(|(kind, at)| {
                    let kind = [Kind::Meeting, Kind::Reminders, Kind::Reply].into_iter().find(|k| k.id() == kind)?;
                    Some((kind, DateTime::parse_from_rfc3339(&at).ok()?.with_timezone(&Utc)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Saves the heads-up, its keys and its send time in one commit, and forgets marks and
/// send times too old to matter.
pub fn record(store: &Store, candidate: &Candidate, at: DateTime<Utc>) -> Result<Nudge, String> {
    let nudge = Nudge {
        id: uuid::Uuid::new_v4().to_string(),
        kind: candidate.kind,
        title: candidate.title.chars().take(120).collect(),
        detail: candidate.detail.chars().take(160).collect(),
        created_at: at.to_rfc3339(),
        expires_at: candidate.expires.to_rfc3339(),
        seed: candidate.seed.clone(),
        task_id: None,
    };
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    store
        .db
        .execute(
            "INSERT INTO nudges(id,data,created_at,expires_at,state) VALUES(?1,?2,?3,?4,'new')",
            params![nudge.id, serde_json::to_string(&nudge).map_err(|e| e.to_string())?, nudge.created_at, nudge.expires_at],
        )
        .map_err(|e| e.to_string())?;
    for key in &candidate.keys {
        store
            .db
            .execute(
                "INSERT INTO nudge_marks(key,kind,at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET at=excluded.at",
                params![hashed(key), candidate.kind.id(), nudge.created_at],
            )
            .map_err(|e| e.to_string())?;
    }
    store
        .db
        .execute("INSERT INTO nudge_sent(kind,at) VALUES(?1,?2)", params![candidate.kind.id(), nudge.created_at])
        .map_err(|e| e.to_string())?;
    let month = (at - Duration::days(30)).to_rfc3339();
    let days = (at - Duration::days(2)).to_rfc3339();
    store.db.execute("DELETE FROM nudge_marks WHERE at<?1", [month]).map_err(|e| e.to_string())?;
    store.db.execute("DELETE FROM nudge_sent WHERE at<?1", [&days]).map_err(|e| e.to_string())?;
    // An opened one's conversation turn holds what it said; the row itself isn't kept.
    store.db.execute("DELETE FROM nudges WHERE expires_at<?1", [&days]).map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(nudge)
}

fn nudge(store: &Store, id: &str) -> Result<Option<(Nudge, String)>, String> {
    store
        .db
        .query_row("SELECT data, state FROM nudges WHERE id=?1", [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .optional()
        .map_err(|e| e.to_string())?
        .map(|(data, state)| serde_json::from_str(&data).map(|nudge| (nudge, state)).map_err(|e| e.to_string()))
        .transpose()
}

/// Heads-ups not yet opened or dismissed and still relevant, newest first.
pub fn entries(store: &Store, at: DateTime<Utc>) -> Result<Vec<Entry>, String> {
    let mut statement = store
        .db
        .prepare("SELECT data FROM nudges WHERE state='new' AND expires_at>?1 ORDER BY created_at DESC LIMIT 4")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([at.to_rfc3339()], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|data| serde_json::from_str::<Nudge>(&data).ok())
        .map(|nudge| Entry { id: nudge.id, kind: nudge.kind, title: nudge.title, detail: nudge.detail, created_at: nudge.created_at })
        .collect())
}

/// Whether a Focus (or Do Not Disturb) is on, when macOS lets Buntini see it. macOS
/// keeps this private without Full Disk Access; then the answer is "no", and the alert,
/// which is never marked time sensitive, is held by the Focus itself.
fn focus_on() -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    std::fs::read_to_string(std::path::Path::new(&home).join("Library/DoNotDisturb/DB/Assertions.json"))
        .ok()
        .is_some_and(|text| focus_in(&text))
}

/// A Focus turned on by hand leaves an assertion record here. One started by a schedule
/// or a smart trigger usually doesn't, so this is best effort: during those, the alert
/// (never marked time sensitive) is held by the Focus itself.
pub fn focus_in(assertions: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(assertions).ok().is_some_and(|value| {
        value["data"].as_array().is_some_and(|data| {
            data.iter().any(|entry| entry["storeAssertionRecords"].as_array().is_some_and(|records| !records.is_empty()))
        })
    })
}

/// How often the rules look. Meetings need a look every couple of minutes; the other
/// reads are spaced further apart below.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(120);
const REMINDERS_EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// A sort of the inbox this recent is reused rather than asking Mail again.
const TRIAGE_FRESH: std::time::Duration = std::time::Duration::from_secs(30 * 60);

static LAST_CHECK: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static LAST_REMINDERS: Mutex<Option<std::time::Instant>> = Mutex::new(None);

fn due(last: &Mutex<Option<std::time::Instant>>, every: std::time::Duration) -> bool {
    let mut last = last.lock().unwrap();
    if last.is_some_and(|at| at.elapsed() < every) {
        return false;
    }
    *last = Some(std::time::Instant::now());
    true
}

/// Looks for heads-ups to give, from the scheduler's loop. Only while Buntini is idle,
/// set up and switched on; never opens Mail.
pub async fn run_due(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if !state.active.lock().unwrap().is_empty() || !due(&LAST_CHECK, CHECK_EVERY) {
        return;
    }
    // A Delete everything (or switching heads-ups off) while the reads run wins.
    let erasures = state.erasures.load(std::sync::atomic::Ordering::SeqCst);
    let cleared = triage::generation();
    let (settings, active, history_sent) = {
        let store = state.store.lock().unwrap();
        let Ok(settings) = store.settings() else { return };
        if !settings.heads_ups || !store.onboarded().unwrap_or(false) {
            return;
        }
        (settings, usual_hours(&store, Utc::now()), sent(&store))
    };
    if focus_on() {
        return;
    }
    let now = Local::now();
    let helper = state.models.apple_helper().to_path_buf();
    let mut found = Vec::new();
    if crate::apps::allowed(&settings, "calendar") {
        let events = match meeting_days(now) {
            (today, None) => crate::apps::events(&helper, &today).await,
            (today, Some(tomorrow)) => crate::apps::events_between(&helper, &today, &tomorrow).await,
        };
        if let Ok(events) = events {
            found.extend(meeting(now, &events.items));
        }
    }
    if crate::apps::allowed(&settings, "reminders") && active.contains(now.hour()) && due(&LAST_REMINDERS, REMINDERS_EVERY) {
        if let Ok(open) = crate::apps::reminders(&helper).await {
            found.extend(reminders(now, &open.items));
        }
    }
    let reply_recent = history_sent.iter().any(|(kind, at)| *kind == Kind::Reply && Utc::now() - *at < Kind::Reply.gap());
    if crate::apps::allowed(&settings, "mail") && reply_moment(now.hour(), active) && !reply_recent {
        if let Ok(sorted) = triage::cached(crate::apps::Launch::IfRunning, TRIAGE_FRESH).await {
            found.extend(replies(now, &sorted));
        }
    }
    for candidate in found {
        // The user may have started something while the reads ran.
        if !state.active.lock().unwrap().is_empty() {
            return;
        }
        let saved = {
            let store = state.store.lock().unwrap();
            if !still_wanted(&store, &state, erasures, cleared) {
                return;
            }
            let check = |key: &str| marked(&store, key);
            let sent = sent(&store);
            if !admit(&candidate, now, active, &History { sent: &sent, marked: &check }) {
                continue;
            }
            record(&store, &candidate, Utc::now())
        };
        match saved {
            Ok(nudge) => {
                let delivery = crate::notifications::deliver_nudge(app, &nudge.id, &candidate.body).await;
                state.audit("nudge.given", &format!("{}: {}", nudge.kind.id(), delivery.unwrap_or_else(|error| error)));
                let _ = tauri::Emitter::emit(app, "background-updated", serde_json::json!({ "nudgeId": nudge.id }));
            }
            Err(error) => state.audit("nudge.failed", &error),
        }
    }
}

/// Whether heads-ups found by reads that began before are still wanted: still switched
/// on and set up, and nothing was deleted or cleared meanwhile.
fn still_wanted(store: &Store, state: &AppState, erasures: u64, cleared: u64) -> bool {
    state.erasures.load(std::sync::atomic::Ordering::SeqCst) == erasures
        && !triage::forgotten_since(cleared)
        && store.settings().is_ok_and(|settings| settings.heads_ups)
        && store.onboarded().unwrap_or(false)
}

fn day_label(stamp: &str) -> String {
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M")
        .map(|at| at.format("%a %-d %b").to_string())
        .or_else(|_| NaiveDate::parse_from_str(stamp.get(..10).unwrap_or(stamp), "%Y-%m-%d").map(|day| day.format("%a %-d %b").to_string()))
        .unwrap_or_else(|_| stamp.to_string())
}

/// Words in an event title too common to find related notes with.
const COMMON: &[&str] = &["meeting", "call", "with", "and", "the", "for", "sync", "chat", "catch", "weekly", "daily", "1:1", "event"];

/// What the conversation says, the request it offers, and the data it shows. `related`
/// is what the memory index found for a meeting (title, source, day).
pub fn compose(seed: &Seed, now: DateTime<Local>, related: &[(String, String, String)]) -> (String, Option<String>) {
    match seed {
        Seed::Meeting { title, start, end, start_at, location, attendees } => {
            let starts = DateTime::parse_from_rfc3339(start_at).map(|at| at.with_timezone(&Local)).ok();
            let minutes = starts.map(|at| (at - now).num_minutes());
            let mut lines = vec![match minutes {
                Some(m) if m > 1 => format!("“{title}” starts at {start}, in {m} minutes, and ends at {end}."),
                Some(m) if m >= 0 => format!("“{title}” starts at {start}, any moment now, and ends at {end}."),
                _ => format!("“{title}” started at {start} and ends at {end}."),
            }];
            if !location.is_empty() {
                lines.push(format!("Where: {location}."));
            }
            if !attendees.is_empty() {
                let shown: Vec<String> = attendees.iter().take(5).cloned().collect();
                let more = attendees.len().saturating_sub(5);
                lines.push(if more > 0 { format!("With {} and {more} more.", shown.join(", ")) } else { format!("With {}.", list(&shown)) });
            }
            if !related.is_empty() {
                lines.push("From your notes and email, these may be related:".into());
                lines.extend(related.iter().map(|(title, source, day)| format!("• “{title}” ({source}, {day})")));
            }
            (lines.join("\n"), None)
        }
        Seed::Reminders { items } => {
            let mut lines = vec![if items.len() == 1 { "This reminder is past due:".to_string() } else { "These reminders are past due:".to_string() }];
            lines.extend(items.iter().map(|item| {
                let list = if item.list.is_empty() { String::new() } else { format!(", {}", item.list) };
                format!("• {} (due {}{list})", item.title, due_label(&item.due))
            }));
            lines.push("Say “mark … as done” and I’ll ask for your OK before changing Reminders.".into());
            (lines.join("\n"), items.first().map(|item| format!("Mark {} as done", item.title)))
        }
        Seed::Reply { emails } => {
            let mut lines = vec![if emails.len() == 1 { "This email is waiting on a reply:".to_string() } else { "These emails are waiting on a reply:".to_string() }];
            lines.extend(emails.iter().map(|waiting| {
                let email = &waiting.email;
                let subject = if email.subject.is_empty() { "(no subject)" } else { &email.subject };
                format!("{}. {} — “{subject}” ({}): {}", email.number, sender_name(&email.sender), day_label(&email.received), waiting.reason)
            }));
            lines.push("Say “reply to the first one saying …” and I’ll draft it for your OK.".into());
            // Numbered, never "reply to it", which means the email open in Mail.
            (lines.join("\n"), Some("Reply to the first one saying ".into()))
        }
    }
}

fn related(store: &Store, settings: &Settings, title: &str) -> Vec<(String, String, String)> {
    let words: Vec<String> = crate::skills::words(title)
        .split(' ')
        .filter(|word| word.chars().count() >= 3 && !COMMON.contains(word))
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        return vec![];
    }
    let roots = store.roots().unwrap_or_default();
    // Keyword ranks only: no embedding run just for a heads-up.
    crate::memory_index::search_visible(&store.db, &words.join(" "), None, 3, |hit| {
        ["notes", "mail"].contains(&hit.source.as_str())
            && crate::memory_index::visible(hit, crate::privacy::Locality::OnDevice, settings, &roots)
    })
    .unwrap_or_default()
    .into_iter()
    .map(|hit| (hit.title, if hit.source == "notes" { "note".into() } else { "email".into() }, day_label(&hit.observed_at)))
    .collect()
}

/// The heads-ups conversation, apart from the one open in the window.
pub const THREAD: (&str, &str) = ("heads-ups", "Heads-ups");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub task_id: String,
    /// The request to offer in the composer; sending it goes through the approval sheet.
    pub suggestion: Option<String>,
}

/// Writes the heads-up's conversation turn, once, and hands back where it is.
pub fn open(store: &Store, id: &str, now: DateTime<Local>) -> Result<Opened, String> {
    let (mut nudge, _) = nudge(store, id)?.ok_or("This heads-up is gone. It may have been cleared with your history.")?;
    let settings = store.settings()?;
    let found = match &nudge.seed {
        Seed::Meeting { title, .. } => related(store, &settings, title),
        _ => vec![],
    };
    let (answer, suggestion) = compose(&nudge.seed, now, &found);
    if let Some(task_id) = nudge.task_id.clone().filter(|task_id| store.task(task_id).ok().flatten().is_some()) {
        return Ok(Opened { task_id, suggestion });
    }
    let mut touched: Vec<String> = match &nudge.seed {
        Seed::Meeting { .. } | Seed::Reminders { .. } => vec![crate::privacy::Share::Calendar.id().into()],
        Seed::Reply { .. } => vec![crate::privacy::Share::Mail.id().into()],
    };
    for (_, source, _) in &found {
        let share = if source == "note" { crate::privacy::Share::Notes } else { crate::privacy::Share::Mail };
        if !touched.iter().any(|id| id == share.id()) {
            touched.push(share.id().into());
        }
    }
    let task = TaskRecord {
        id: uuid::Uuid::new_v4().to_string(),
        prompt: String::new(),
        answer,
        state: "completed".into(),
        created_at: now.with_timezone(&Utc).to_rfc3339(),
        sources: vec![],
        files: vec![],
        system: None,
        research: vec![],
        research_report: None,
        prices: vec![],
        cards: vec![],
        backend: "nudge".into(),
        touched,
        handoff: None,
        action: None,
    };
    nudge.task_id = Some(task.id.clone());
    let transaction = store.db.unchecked_transaction().map_err(|e| e.to_string())?;
    crate::routines::assign_thread(store, &task.id, &(THREAD.0.into(), THREAD.1.into()))?;
    store.save_task(&task)?;
    // "Reply to the first one" in this conversation means the first email listed here.
    if let Seed::Reply { emails } = &nudge.seed {
        triage::save_listing(store, &task.id, emails.iter().map(|waiting| waiting.email.clone()).collect())?;
    }
    store
        .db
        .execute(
            "UPDATE nudges SET state='opened', data=?2 WHERE id=?1",
            params![id, serde_json::to_string(&nudge).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(Opened { task_id: task.id, suggestion })
}

pub(crate) fn identifier(id: &str) -> String {
    format!("nudge-{id}")
}

/// Every saved heads-up's alert identifier, so clearing history can withdraw them.
pub(crate) fn alert_identifiers(store: &Store) -> Vec<String> {
    let Ok(mut statement) = store.db.prepare("SELECT id FROM nudges") else {
        return vec![];
    };
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).map(|id| identifier(&id)).collect())
        .unwrap_or_default()
}

/// The heads-ups the window lists; none while they are switched off.
#[tauri::command]
pub fn heads_ups(state: tauri::State<'_, AppState>) -> Result<Vec<Entry>, String> {
    let store = state.store.lock().unwrap();
    if !store.settings()?.heads_ups {
        return Ok(vec![]);
    }
    entries(&store, Utc::now())
}

/// Opens a heads-up: its conversation turn is written and its alert withdrawn. The
/// window then shows that conversation and offers the suggested request.
#[tauri::command]
pub fn open_heads_up(id: String, state: tauri::State<'_, AppState>) -> Result<Opened, String> {
    let store = state.store.lock().unwrap();
    // An alert left from before heads-ups were switched off writes nothing.
    if !store.settings()?.heads_ups {
        crate::delivery::os().remove(&[identifier(&id)]);
        return Err("Heads-ups are off, so this one wasn’t opened. You can switch them on in Settings.".into());
    }
    let opened = open(&store, &id, Local::now())?;
    drop(store);
    crate::delivery::os().remove(&[identifier(&id)]);
    Ok(opened)
}

#[tauri::command]
pub fn dismiss_heads_up(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .lock()
        .unwrap()
        .db
        .execute("UPDATE nudges SET state='dismissed' WHERE id=?1 AND state='new'", [&id])
        .map_err(|e| e.to_string())?;
    crate::delivery::os().remove(&[identifier(&id)]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(stamp: &str) -> DateTime<Local> {
        local(stamp).unwrap()
    }

    fn event(id: &str, start: &str, end: &str, title: &str) -> Event {
        Event { id: Some(id.into()), start: start.into(), end: end.into(), title: title.into(), ..Default::default() }
    }

    fn reminder(id: &str, title: &str, due: &str) -> Reminder {
        Reminder { id: Some(id.into()), title: title.into(), due: due.into(), list: "Personal".into(), ..Default::default() }
    }

    fn header(id: &str, sender: &str, received: &str) -> crate::apps::MailHeader {
        crate::apps::MailHeader {
            received: received.into(),
            id: id.into(),
            read: false,
            flagged: true,
            to_me: Some(true),
            to_count: 1,
            sender: sender.into(),
            subject: format!("Subject {id}"),
            message_id: format!("m-{id}@x.com"),
        }
    }

    #[test]
    fn a_meeting_heads_up_comes_shortly_before_it_and_names_nothing_on_the_lock_screen() {
        let now = at("2026-10-30 09:50");
        let mut standup = event("e1", "2026-10-30 10:00", "2026-10-30 10:30", "Lease review");
        standup.location = "Room 4".into();
        standup.attendees = vec!["Ana Silva".into(), "Sam Lee".into()];
        let events = vec![
            event("e0", "2026-10-30 09:51", "2026-10-30 10:00", "Too late"),
            standup,
            event("e2", "2026-10-30 10:30", "2026-10-30 11:00", "Too early"),
            Event { all_day: true, ..event("e3", "2026-10-30 00:00", "2026-10-31 00:00", "Holiday") },
            event("e4", "2026-10-30 10:00", "2026-10-30 18:00", "Work block"),
        ];
        let found = meeting(now, &events).unwrap();
        assert_eq!(found.title, "Lease review at 10:00");
        assert_eq!(found.detail, "Room 4");
        assert_eq!(found.body, "Something on your calendar starts in 10 minutes.");
        for secret in ["Lease", "Room 4", "Ana", "Sam"] {
            assert!(!found.body.contains(secret));
        }
        assert!(meeting(at("2026-10-30 09:30"), &events).is_none(), "too early for any");
        let (text, suggestion) = compose(&found.seed, now, &[("Lease notes".into(), "note".into(), "Tue 27 Oct".into())]);
        assert!(text.starts_with("“Lease review” starts at 10:00, in 10 minutes, and ends at 10:30."), "{text}");
        assert!(text.contains("Where: Room 4.") && text.contains("With Ana Silva and Sam Lee."), "{text}");
        assert!(text.contains("• “Lease notes” (note, Tue 27 Oct)"), "{text}");
        assert_eq!(suggestion, None);
    }

    #[test]
    fn reminders_count_once_late_and_only_for_a_few_days() {
        let now = at("2026-10-30 15:00");
        let items = vec![
            reminder("r1", "Call mum", "2026-10-30 14:00"),
            reminder("r2", "Just due", "2026-10-30 14:45"),
            reminder("r3", "Pay rent", "2026-10-30"),
            reminder("r4", "Ancient", "2026-10-01"),
            reminder("r5", "Undated", ""),
            reminder("r6", "Yesterday", "2026-10-29"),
        ];
        let found = reminders(now, &items).unwrap();
        assert_eq!(found.keys.len(), 3);
        assert_eq!(found.body, "Some reminders are past due.");
        assert_eq!(found.detail, "Call mum, Pay rent and Yesterday");
        assert!(reminders(at("2026-10-30 13:00"), &items[2..3]).is_none(), "a date-only one waits for the afternoon");
        let (text, suggestion) = compose(&found.seed, now, &[]);
        assert!(text.contains("• Call mum (due Fri 30 Oct 14:00, Personal)"), "{text}");
        assert_eq!(suggestion.as_deref(), Some("Mark Call mum as done"));
        // The suggestion is a request the existing approval path recognises.
        assert_eq!(
            crate::edit_targets::clause("Mark Call mum as done", "complete_reminder").is_some(),
            true
        );
    }

    #[test]
    fn replies_count_after_a_wait_and_follow_ups_answer_the_listed_email() {
        let now = at("2026-10-30 10:30");
        let sorted = triage::sort(
            vec![header("1", "Ana Silva <ana@x.com>", "2026-10-29 16:00"), header("2", "Sam <sam@x.com>", "2026-10-30 10:00")],
            2,
            &Default::default(),
        );
        let found = replies(now, &sorted).unwrap();
        assert_eq!(found.keys, vec!["reply|m-1@x.com".to_string()], "Sam wrote half an hour ago");
        assert_eq!(found.title, "Ana Silva is waiting on a reply");
        assert_eq!(found.body, "Someone is waiting on a reply from you.");
        let (text, suggestion) = compose(&found.seed, now, &[]);
        assert!(text.contains("1. Ana Silva — “Subject 1” (Thu 29 Oct): flagged"), "{text}");
        // The offered request picks the listed email, not whatever is open in Mail.
        let suggestion = suggestion.unwrap();
        assert_eq!(triage::pick(&format!("{suggestion}yes")), Some(triage::Pick::Number(1)));
    }

    #[test]
    fn quiet_hours_follow_the_users_own_day_or_a_sane_default() {
        assert_eq!(active_hours(&[3; 5]), Active::DEFAULT, "too little to learn from");
        let mut hours = vec![9; 10];
        hours.extend([10, 12, 14, 16, 18, 19, 20, 20, 21, 22]);
        assert_eq!(active_hours(&hours), Active { wake: 9, bed: 23 });
        assert_eq!(active_hours(&[2; 40]), Active { wake: 6, bed: 19 }, "kept to a sane day");
        assert!(reply_moment(10, Active::DEFAULT) && reply_moment(16, Active::DEFAULT));
        assert!(!reply_moment(13, Active::DEFAULT) && !reply_moment(10, Active { wake: 11, bed: 21 }));
    }

    #[test]
    fn rate_limits_quiet_hours_and_marks_keep_heads_ups_rare() {
        let now = at("2026-10-30 10:30");
        let candidate = |kind: Kind| Candidate {
            kind,
            keys: vec!["k".into()],
            title: String::new(),
            detail: String::new(),
            body: String::new(),
            expires: now.with_timezone(&Utc),
            seed: Seed::Reminders { items: vec![] },
        };
        let nothing = |_: &str| false;
        let open = History { sent: &[], marked: &nothing };
        assert!(admit(&candidate(Kind::Reply), now, Active::DEFAULT, &open));
        assert!(!admit(&candidate(Kind::Reply), at("2026-10-30 13:00"), Active::DEFAULT, &open), "not a reply moment");
        assert!(!admit(&candidate(Kind::Reminders), at("2026-10-30 22:00"), Active::DEFAULT, &open), "quiet hours");
        assert!(admit(&candidate(Kind::Meeting), at("2026-10-30 06:50"), Active::DEFAULT, &open), "a meeting is worth saying early");
        let everything = |_: &str| true;
        assert!(!admit(&candidate(Kind::Meeting), now, Active::DEFAULT, &History { sent: &[], marked: &everything }), "said already");
        let utc = now.with_timezone(&Utc);
        let recent = [(Kind::Reminders, utc - Duration::hours(1))];
        assert!(!admit(&candidate(Kind::Reminders), now, Active::DEFAULT, &History { sent: &recent, marked: &nothing }));
        assert!(admit(&candidate(Kind::Meeting), now, Active::DEFAULT, &History { sent: &recent, marked: &nothing }), "limits are per kind");
        let busy: Vec<_> = (1..=3).map(|h| (Kind::Reminders, utc - Duration::hours(h * 4))).collect();
        assert!(!admit(&candidate(Kind::Reminders), now, Active::DEFAULT, &History { sent: &busy, marked: &nothing }), "three a day at most");
    }

    #[test]
    fn a_focus_is_seen_only_when_one_is_on() {
        assert!(focus_in(r#"{"data":[{"storeAssertionRecords":[{"assertionDetails":{"assertionDetailsModeIdentifier":"com.apple.focus.work"}}]}]}"#));
        assert!(!focus_in(r#"{"data":[{"storeAssertionRecords":[]}]}"#));
        assert!(!focus_in(r#"{"data":[{}]}"#));
        assert!(!focus_in("not json"));
    }

    #[test]
    fn heads_ups_survive_restarts_once_and_go_with_history_and_erasure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudges.db");
        let store = Store::open(&path).unwrap();
        let now = at("2026-10-30 10:30");
        let sorted = triage::sort(vec![header("1", "Ana Silva <ana@x.com>", "2026-10-29 16:00")], 1, &Default::default());
        let found = replies(now, &sorted).unwrap();
        let saved = record(&store, &found, now.with_timezone(&Utc)).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(marked(&store, &found.keys[0]), "dedupe outlives a restart");
        assert_eq!(sent(&store).len(), 1);
        assert_eq!(entries(&store, now.with_timezone(&Utc)).unwrap().len(), 1);
        // Opening writes one turn in the heads-ups thread, with the listing for follow-ups.
        let opened = open(&store, &saved.id, now).unwrap();
        assert_eq!(opened.task_id, open(&store, &saved.id, now).unwrap().task_id, "opened once");
        assert_eq!(store.task_conversation(&opened.task_id).unwrap(), THREAD.0);
        let turn = store.task(&opened.task_id).unwrap().unwrap();
        assert_eq!(turn.touched, ["mail"]);
        assert!(triage::recent_listing(&store, &[turn]).is_some());
        assert!(entries(&store, now.with_timezone(&Utc)).unwrap().is_empty(), "opened ones leave the list");
        let raw = |store: &Store| -> String {
            store.db.query_row("SELECT group_concat(data) FROM nudges", [], |row| row.get::<_, Option<String>>(0)).unwrap().unwrap_or_default()
        };
        assert!(raw(&store).contains("Ana Silva"));
        // Clearing history removes what they said but remembers that they were said.
        store.clear_history().unwrap();
        assert!(raw(&store).is_empty());
        assert!(marked(&store, &found.keys[0]));
        let keys: String = store.db.query_row("SELECT group_concat(key) FROM nudge_marks", [], |row| row.get(0)).unwrap();
        assert!(!keys.contains("m-1@x.com"), "only hashes are kept");
        assert!(open(&store, &saved.id, now).is_err());
        // Erasing forgets everything, marks and send times included.
        record(&store, &found, now.with_timezone(&Utc)).unwrap();
        store.erase_everything().unwrap();
        assert!(raw(&store).is_empty());
        assert!(!marked(&store, &found.keys[0]));
        assert!(sent(&store).is_empty());
    }

    #[test]
    fn opened_heads_ups_go_when_expired_or_when_their_turn_is_deleted() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = at("2026-10-30 10:30");
        let sorted = triage::sort(vec![header("1", "Ana Silva <ana@x.com>", "2026-10-29 16:00")], 1, &Default::default());
        let found = replies(now, &sorted).unwrap();
        let rows = |store: &Store| -> i64 { store.db.query_row("SELECT count(*) FROM nudges", [], |row| row.get(0)).unwrap() };
        let saved = record(&store, &found, now.with_timezone(&Utc)).unwrap();
        let opened = open(&store, &saved.id, now).unwrap();
        store.delete_task(&opened.task_id).unwrap();
        assert_eq!(rows(&store), 0, "deleting the turn deletes the heads-up that repeats it");
        let saved = record(&store, &found, now.with_timezone(&Utc)).unwrap();
        open(&store, &saved.id, now).unwrap();
        let later = triage::sort(vec![header("2", "Sam <sam@x.com>", "2026-11-03 16:00")], 1, &Default::default());
        record(&store, &replies(at("2026-11-04 10:30"), &later).unwrap(), at("2026-11-08 10:30").with_timezone(&Utc)).unwrap();
        let left: Option<String> = store.db.query_row("SELECT group_concat(data) FROM nudges", [], |row| row.get(0)).unwrap();
        assert!(!left.unwrap_or_default().contains("Ana Silva"), "an opened one is not kept past its time");
    }

    #[test]
    fn heads_ups_read_before_a_delete_or_switch_off_are_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().join("data"), dir.path().join("cache"), dir.path().into()).unwrap();
        {
            let store = state.store.lock().unwrap();
            store.save_settings(&crate::types::Settings { heads_ups: true, ..Default::default() }).unwrap();
            store.set_onboarded(true).unwrap();
        }
        let erasures = state.erasures.load(std::sync::atomic::Ordering::SeqCst);
        let cleared = triage::generation();
        assert!(still_wanted(&state.store.lock().unwrap(), &state, erasures, cleared) || triage::forgotten_since(cleared));
        state.erasures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert!(!still_wanted(&state.store.lock().unwrap(), &state, erasures, cleared), "a Delete everything meanwhile wins");
        let erasures = state.erasures.load(std::sync::atomic::Ordering::SeqCst);
        state.store.lock().unwrap().save_settings(&crate::types::Settings { heads_ups: false, ..Default::default() }).unwrap();
        assert!(!still_wanted(&state.store.lock().unwrap(), &state, erasures, triage::generation()), "switched off meanwhile");
    }

    #[test]
    fn meetings_just_after_midnight_are_read_from_tomorrow() {
        assert_eq!(meeting_days(at("2026-10-30 14:00")), ("2026-10-30".into(), None));
        assert_eq!(meeting_days(at("2026-10-30 23:50")), ("2026-10-30".into(), Some("2026-10-31".into())));
        let events = vec![event("e1", "2026-10-31 00:05", "2026-10-31 00:30", "Late call")];
        assert!(meeting(at("2026-10-30 23:52"), &events).is_some());
    }

    #[test]
    fn active_hours_learn_from_the_users_own_requests_not_routine_runs() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = Utc::now();
        for n in 0..30 {
            for (hour, thread) in [(6, Some("routine:r1")), (22, Some("goal:g1"))] {
                let created = (Local::now() - Duration::days(1 + n % 20)).with_hour(hour).unwrap().with_timezone(&Utc).to_rfc3339();
                let id = format!("t-{hour}-{n}");
                if let Some(thread) = thread {
                    crate::routines::assign_thread(&store, &id, &(thread.into(), "Routine".into())).unwrap();
                }
                store.db.execute("INSERT INTO tasks(id,data,created_at) VALUES(?1,?2,?3)", params![id, serde_json::json!({"prompt":"Check the news"}).to_string(), created]).unwrap();
            }
        }
        assert_eq!(usual_hours(&store, now), Active::DEFAULT, "scheduled runs say nothing about when the user is here");
    }

    #[test]
    fn dismissed_and_expired_heads_ups_leave_the_list() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = at("2026-10-30 09:50");
        let events = vec![event("e1", "2026-10-30 10:00", "2026-10-30 10:30", "Standup")];
        let saved = record(&store, &meeting(now, &events).unwrap(), now.with_timezone(&Utc)).unwrap();
        assert_eq!(entries(&store, now.with_timezone(&Utc)).unwrap()[0].title, "Standup at 10:00");
        assert!(entries(&store, at("2026-10-30 10:31").with_timezone(&Utc)).unwrap().is_empty(), "over once the event ends");
        store.db.execute("UPDATE nudges SET state='dismissed' WHERE id=?1", [&saved.id]).unwrap();
        assert!(entries(&store, now.with_timezone(&Utc)).unwrap().is_empty());
    }
}
