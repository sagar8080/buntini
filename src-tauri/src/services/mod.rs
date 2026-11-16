//! Developer services (stream D): read-only pollers for GitHub, GitLab, Gitea, Vercel, Stripe,
//! n8n, Notion, Cal.com and Resend.
//!
//! Each service is asked a fixed, read-only question on a fixed HTTPS host, or on the person's
//! own host for GitLab, Gitea and n8n. Its key stays in the Keychain (through `web::keychain`)
//! and never goes back to the UI. Every request is reserved against a daily and monthly
//! allowance first. A service is polled only while something watches it: the island (unless the
//! person hid its pill there) or Settings → Connections, and only while someone is at the Mac.
//! Errors and rate limits push the next poll back. What a service shows becomes a tinted pill
//! and a card on the island.
mod calcom;
mod fetch;
mod gitea;
mod github;
mod gitlab;
mod n8n;
mod notion;
mod publish;
mod resend;
mod snapshot;
mod stripe;
mod vercel;

use std::collections::{BTreeSet, HashMap};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, LazyLock, Mutex, Once,
};

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

use crate::controller::AppState;
use crate::surface::{Action, Source};
use fetch::{Failure, Net, RateLimit};
pub use snapshot::{Row, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Github,
    Gitlab,
    Gitea,
    Vercel,
    Stripe,
    N8n,
    Notion,
    Calcom,
    Resend,
}

/// Keychain entries, one per service. `web::provider_valid` accepts these, and Delete
/// everything removes them.
pub const KEY_IDS: [&str; 9] = ["github", "gitlab", "gitea", "vercel", "stripe", "n8n", "notion", "calcom", "resend"];

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Github,
        Kind::Gitlab,
        Kind::Gitea,
        Kind::Vercel,
        Kind::Stripe,
        Kind::N8n,
        Kind::Notion,
        Kind::Calcom,
        Kind::Resend,
    ];

    /// The Keychain entry, the ledger key and the id on cards.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Github => "github",
            Kind::Gitlab => "gitlab",
            Kind::Gitea => "gitea",
            Kind::Vercel => "vercel",
            Kind::Stripe => "stripe",
            Kind::N8n => "n8n",
            Kind::Notion => "notion",
            Kind::Calcom => "calcom",
            Kind::Resend => "resend",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Github => "GitHub",
            Kind::Gitlab => "GitLab",
            Kind::Gitea => "Gitea",
            Kind::Vercel => "Vercel",
            Kind::Stripe => "Stripe",
            Kind::N8n => "n8n",
            Kind::Notion => "Notion",
            Kind::Calcom => "Cal.com",
            Kind::Resend => "Resend",
        }
    }

    /// The pill's tint on the black island.
    pub fn tint(self) -> &'static str {
        match self {
            Kind::Github => "#A371F7",
            Kind::Gitlab => "#FC6D26",
            Kind::Gitea => "#7CB342",
            Kind::Vercel => "#F5F5F5",
            Kind::Stripe => "#7A73FF",
            Kind::N8n => "#EA4B71",
            Kind::Notion => "#C9C2B8",
            Kind::Calcom => "#5B9BF8",
            Kind::Resend => "#3DD6B5",
        }
    }

    /// The fixed API host; `None` where the person names the host.
    fn api_host(self) -> Option<&'static str> {
        match self {
            Kind::Github => Some(github::HOST),
            Kind::Vercel => Some(vercel::HOST),
            Kind::Stripe => Some(stripe::HOST),
            Kind::Notion => Some(notion::HOST),
            Kind::Calcom => Some(calcom::HOST),
            Kind::Resend => Some(resend::HOST),
            Kind::Gitlab | Kind::Gitea | Kind::N8n => None,
        }
    }

    /// Seconds between polls while nothing is running.
    fn every(self) -> i64 {
        match self {
            Kind::Vercel => 120,
            Kind::Github | Kind::Gitlab | Kind::N8n => 180,
            Kind::Gitea | Kind::Stripe => 300,
            Kind::Notion | Kind::Resend => 600,
            Kind::Calcom => 900,
        }
    }

    /// Where a link from this service may lead, besides its own host.
    fn web_hosts(self) -> &'static [&'static str] {
        match self {
            Kind::Github => &["github.com"],
            Kind::Vercel => &["vercel.com", ".vercel.app"],
            Kind::Stripe => &["dashboard.stripe.com"],
            Kind::Notion => &["www.notion.so", "notion.so", ".notion.site"],
            Kind::Calcom => &["app.cal.com", "cal.com"],
            Kind::Resend => &["resend.com"],
            Kind::Gitlab | Kind::Gitea | Kind::N8n => &[],
        }
    }
}

/// Saved under `settings.services`: only the hosts of the self-hosted services. Keys live in the
/// Keychain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {
    /// `host` or `host:port`; empty is gitlab.com.
    pub gitlab_host: String,
    pub gitea_host: String,
    pub n8n_host: String,
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        for kind in [Kind::Gitlab, Kind::Gitea, Kind::N8n] {
            let host = self.raw_host(kind);
            if !host.is_empty() && normalize_host(host).as_deref() != Ok(host) {
                return Err(format!("Enter {}'s address as a host name, like git.example.com.", kind.name()));
            }
        }
        Ok(())
    }

    fn raw_host(&self, kind: Kind) -> &str {
        match kind {
            Kind::Gitlab => &self.gitlab_host,
            Kind::Gitea => &self.gitea_host,
            Kind::N8n => &self.n8n_host,
            _ => "",
        }
    }

    /// Where the service's API lives, once it is known.
    pub fn host(&self, kind: Kind) -> Option<String> {
        if let Some(host) = kind.api_host() {
            return Some(host.into());
        }
        match self.raw_host(kind) {
            "" if kind == Kind::Gitlab => Some("gitlab.com".into()),
            "" => None,
            host => Some(host.into()),
        }
    }
}

/// `https://Git.Example.com:8443/` → `git.example.com:8443`. HTTPS only, and no path, user or
/// query.
pub fn normalize_host(input: &str) -> Result<String, String> {
    let lower = input.trim().to_ascii_lowercase();
    if lower.starts_with("http://") {
        return Err("Only https addresses work.".into());
    }
    let rest = lower.strip_prefix("https://").unwrap_or(&lower).trim_end_matches('/');
    let bad = || "Enter a host name, like git.example.com.".to_string();
    if rest.is_empty() || rest.len() > 253 || rest.contains(['/', '?', '#', '@', '\\']) || rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let url = reqwest::Url::parse(&format!("https://{rest}/")).map_err(|_| bad())?;
    let host = url.host_str().filter(|host| !host.is_empty()).ok_or_else(bad)?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// What a service remembers between polls: slow-changing answers and when they were fetched.
#[derive(Debug, Clone, Default)]
pub struct Memo {
    at: HashMap<&'static str, i64>,
    /// The activity graph, fetched every few hours.
    pub graph: Option<Vec<u32>>,
    /// The account's name or team, as the service needs it.
    pub user: Option<String>,
    /// n8n workflow names by id.
    pub names: HashMap<String, String>,
    /// Gitea pull requests' head commits: `owner/repo#n` → (updated at, sha).
    pub heads: HashMap<String, (String, String)>,
}

impl Memo {
    pub fn due(&self, what: &'static str, every: i64, now: i64) -> bool {
        self.at.get(what).is_none_or(|at| now - at >= every)
    }

    pub fn mark(&mut self, what: &'static str, now: i64) {
        self.at.insert(what, now);
    }
}

/// How a service is doing, as Settings shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    /// Not asked yet, or no key.
    #[default]
    Waiting,
    Ok,
    /// The key was refused.
    Rejected,
    /// The service asked Buntini to wait.
    Limited,
    /// Today's or this month's allowance is used up.
    Budget,
    /// Unreachable or answering errors.
    Failing,
}

#[derive(Default)]
struct Live {
    snapshot: Option<Snapshot>,
    memo: Memo,
    health: Health,
    message: Option<String>,
    failures: u32,
    /// Unix seconds.
    next_at: i64,
    checked_at: Option<i64>,
    /// The cancel flag of the poll in flight.
    in_flight: Option<Arc<AtomicBool>>,
    /// Bumped when the key or host changes, so an older poll's answer is dropped.
    generation: u64,
    /// The host the last poll asked.
    host: Option<String>,
    shown: publish::Shown,
}

#[derive(Default)]
struct Hub {
    live: HashMap<Kind, Live>,
    /// Services the person hid from the island.
    hidden: BTreeSet<Kind>,
    hidden_loaded: bool,
    /// Settings → Connections is open until this Unix time.
    watched_until: i64,
    erasures: u64,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(Default::default);
static REGISTER: Once = Once::new();
const HIDDEN_KEY: &str = "services.hidden";
/// Asking for the status counts as watching for this long.
const WATCH_SECONDS: i64 = 120;
/// Nobody has touched the Mac for this long: nothing is watched.
const AWAY_SECONDS: f64 = 15.0 * 60.0;
/// Seconds between polls while something is still running there.
const BUSY_EVERY: i64 = 30;
/// The longest any wait is stretched to.
const LONGEST_WAIT: i64 = 3600;

/// Requests a service may make: a guard against a runaway loop, far above what polling uses.
fn limits() -> crate::budget::Limits {
    crate::budget::Limits { daily: 3_000, monthly: 60_000, budget_usd: f64::INFINITY, unit_cost: 0.0 }
}

/// When to ask again: sooner while something runs, later after errors, and never before a time
/// the service asked for.
fn next_poll(kind: Kind, result: &Result<bool, Failure>, failures: u32, limit: &RateLimit, now: i64) -> i64 {
    let at = match result {
        Ok(busy) => now + if *busy { BUSY_EVERY } else { kind.every() },
        Err(Failure::Cancelled) => now,
        Err(Failure::Limited { until, .. }) => (*until).max(now + 60),
        Err(Failure::Rejected(_)) => now + 6 * LONGEST_WAIT,
        Err(Failure::Budget(_)) => now + LONGEST_WAIT,
        Err(Failure::Other(_)) => now + (kind.every() << failures.clamp(1, 6).saturating_sub(1)).min(LONGEST_WAIT),
    };
    let asked = limit.wait_until(now).unwrap_or(0);
    let at = at.max(asked);
    // A key refused stays stopped longer than any rate limit.
    if matches!(result, Err(Failure::Rejected(_))) {
        at
    } else {
        at.min(now + LONGEST_WAIT)
    }
}

/// Seconds since anyone touched the keyboard, mouse or trackpad.
#[cfg(target_os = "macos")]
fn idle_seconds() -> f64 {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }
    // The combined session state and any input. Needs no permission and reveals no content.
    unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) }
}

#[cfg(not(target_os = "macos"))]
fn idle_seconds() -> f64 {
    0.0
}

/// Polls the services that are due. Called from the scheduler's tick, which already pauses
/// while a task runs and catches up after the Mac wakes.
pub async fn run_due(app: &tauri::AppHandle) {
    REGISTER.call_once(|| register(app.clone()));
    let state = app.state::<AppState>();
    let Ok(settings) = state.store.lock().unwrap().settings() else { return };
    let now = Utc::now().timestamp();
    let present = idle_seconds() < AWAY_SECONDS;
    let island = crate::surface::running();
    let erasures = state.erasures.load(Ordering::SeqCst);
    let due: Vec<(Kind, u64, Arc<AtomicBool>)> = {
        let mut hub = HUB.lock().unwrap();
        if hub.erasures != erasures {
            // Delete everything ran: forget what was shown and hidden.
            for live in hub.live.values() {
                if let Some(cancel) = &live.in_flight {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            *hub = Hub { erasures, ..Default::default() };
        }
        if !hub.hidden_loaded {
            hub.hidden = load_hidden(&state);
            hub.hidden_loaded = true;
        }
        let settings_open = hub.watched_until > now;
        let hidden = hub.hidden.clone();
        Kind::ALL
            .into_iter()
            .filter_map(|kind| {
                let watched = settings_open || (island && present && !hidden.contains(&kind));
                let live = hub.live.entry(kind).or_default();
                let host = settings.services.host(kind);
                if live.host != host {
                    // A new host is another account: start over there.
                    if let Some(cancel) = &live.in_flight {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    *live = Live { generation: live.generation + 1, host, ..Default::default() };
                }
                if !watched || live.in_flight.is_some() || live.next_at > now {
                    return None;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                live.in_flight = Some(cancel.clone());
                Some((kind, live.generation, cancel))
            })
            .collect()
    };
    for (kind, generation, cancel) in due {
        let app = app.clone();
        let settings = settings.services.clone();
        tauri::async_runtime::spawn(async move { poll(&app, kind, &settings, generation, cancel).await });
    }
}

async fn poll(app: &tauri::AppHandle, kind: Kind, settings: &Settings, generation: u64, cancel: Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let now = Local::now().fixed_offset();
    let (Ok(key), Some(host)) = (crate::web::credential(kind.id()), settings.host(kind)) else {
        // No key or no host yet: look again soon (it is only a Keychain read), asking nothing.
        let mut hub = HUB.lock().unwrap();
        let live = hub.live.entry(kind).or_default();
        if live.generation == generation {
            *live = Live { generation, host: live.host.take(), next_at: now.timestamp() + 60, ..Default::default() };
        }
        drop(hub);
        publish_now();
        return;
    };
    let memo = HUB.lock().unwrap().live.get(&kind).map(|live| live.memo.clone()).unwrap_or_default();
    let store = state.store.clone();
    let reserve = Box::new(move || {
        let reservation = store.lock().unwrap().reserve(kind.id(), limits())?;
        if let Some(warning) = reservation.warning {
            let _ = store.lock().unwrap().audit("services.budget", &warning);
        }
        Ok(())
    });
    let (result, limit, memo) = match Net::new(kind, key, &host, reserve, cancel) {
        Ok(net) => {
            let mut memo = memo;
            let result = match kind {
                Kind::Github => github::poll(&net, &mut memo, now).await,
                Kind::Gitlab => gitlab::poll(&net, &mut memo, now).await,
                Kind::Gitea => gitea::poll(&net, &mut memo, now).await,
                Kind::Vercel => vercel::poll(&net, &mut memo, now).await,
                Kind::Stripe => stripe::poll(&net, now).await,
                Kind::N8n => n8n::poll(&net, &mut memo, now).await,
                Kind::Notion => notion::poll(&net, now).await,
                Kind::Calcom => calcom::poll(&net, now).await,
                Kind::Resend => resend::poll(&net, now).await,
            };
            (result.map(|snapshot| publish::safe_links(kind, &host, snapshot)), net.limit(), memo)
        }
        Err(failure) => (Err(failure), RateLimit::default(), memo),
    };
    let notes = settle(kind, generation, result, &limit, memo, now.timestamp());
    for (action, detail) in notes {
        state.audit(&action, &detail);
    }
    publish_now();
}

/// Records a poll's outcome and returns the audit entries for what changed.
fn settle(kind: Kind, generation: u64, result: Result<Snapshot, Failure>, limit: &RateLimit, memo: Memo, now: i64) -> Vec<(String, String)> {
    let mut hub = HUB.lock().unwrap();
    let live = hub.live.entry(kind).or_default();
    if live.generation != generation {
        // The key changed while this poll ran.
        return vec![];
    }
    live.in_flight = None;
    let before = live.health;
    live.memo = memo;
    let outcome = result.as_ref().map(|snapshot| snapshot.busy).map_err(Clone::clone);
    match result {
        Ok(snapshot) => {
            live.failures = 0;
            live.health = Health::Ok;
            live.message = None;
            live.checked_at = Some(now);
            live.shown.absorb(live.snapshot.as_ref(), &snapshot, crate::surface::now_ms());
            live.snapshot = Some(snapshot);
        }
        Err(Failure::Cancelled) => {}
        Err(failure) => {
            live.health = match &failure {
                Failure::Rejected(_) => Health::Rejected,
                Failure::Limited { .. } => Health::Limited,
                Failure::Budget(_) => Health::Budget,
                _ => Health::Failing,
            };
            if matches!(failure, Failure::Other(_)) {
                live.failures += 1;
            }
            if matches!(failure, Failure::Rejected(_)) {
                live.snapshot = None;
                live.shown = Default::default();
            }
            live.message = Some(failure.message().to_string());
        }
    }
    live.next_at = next_poll(kind, &outcome, live.failures, limit, now);
    if before == live.health {
        return vec![];
    }
    let detail = match &live.message {
        Some(message) => format!("{}: {message}", kind.name()),
        None => kind.name().to_string(),
    };
    let action = match live.health {
        Health::Ok => "services.connected",
        Health::Rejected => "services.key_rejected",
        Health::Limited => "services.rate_limited",
        Health::Budget => "services.allowance_used",
        Health::Failing => "services.failing",
        Health::Waiting => return vec![],
    };
    vec![(action.into(), detail)]
}

/// Puts what every service shows on the island.
fn publish_now() {
    let contribution = {
        let hub = HUB.lock().unwrap();
        let views: Vec<publish::View> = Kind::ALL
            .into_iter()
            .filter(|kind| !hub.hidden.contains(kind))
            .filter_map(|kind| {
                let live = hub.live.get(&kind)?;
                Some(publish::View { kind, snapshot: live.snapshot.as_ref(), rejected: live.health == Health::Rejected, shown: &live.shown })
            })
            .collect();
        publish::contribution(&views)
    };
    crate::surface::publish(Source::Services, contribution);
}

fn load_hidden(state: &AppState) -> BTreeSet<Kind> {
    let raw = state.store.lock().unwrap().preference(HIDDEN_KEY).ok().flatten().unwrap_or_default();
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default().iter().filter_map(|id| Kind::from_id(id)).collect()
}

fn save_hidden(state: &AppState, hidden: &BTreeSet<Kind>) {
    let ids: Vec<&str> = hidden.iter().map(|kind| kind.id()).collect();
    let _ = state.store.lock().unwrap().set_preference(HIDDEN_KEY, &json!(ids).to_string());
}

/// Forgets what a service showed, after its key changed.
fn reset(kind: Kind) {
    let mut hub = HUB.lock().unwrap();
    let live = hub.live.entry(kind).or_default();
    if let Some(cancel) = &live.in_flight {
        cancel.store(true, Ordering::Relaxed);
    }
    *live = Live { generation: live.generation + 1, host: live.host.take(), ..Default::default() };
}

fn register(app: tauri::AppHandle) {
    crate::surface::on_action(Source::Services, move |action| act(&app, action));
}

/// The service whose card shows this link right now. Nothing else is ever opened.
fn link_owner(hub: &Hub, url: &str) -> Option<Kind> {
    Kind::ALL.into_iter().find(|kind| {
        let rows = hub.live.get(kind).filter(|_| !hub.hidden.contains(kind)).and_then(|live| live.snapshot.as_ref()).map(|snapshot| snapshot.rows.as_slice()).unwrap_or(&[]);
        rows.iter().any(|row| row.url.as_deref() == Some(url))
    })
}

/// What the person did on a service's pill or card.
fn act(app: &tauri::AppHandle, action: Action) {
    let state = app.state::<AppState>();
    let kind = action.card.as_deref().and_then(publish::kind_of).or_else(|| action.value.as_ref().and_then(Value::as_str).and_then(Kind::from_id));
    match (action.name.as_str(), kind) {
        ("open", _) => {
            let Some(url) = action.value.as_ref().and_then(Value::as_str) else { return };
            let Some(owner) = link_owner(&HUB.lock().unwrap(), url) else { return };
            let host = reqwest::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default();
            let opened = std::process::Command::new("/usr/bin/open").arg(url).status().is_ok_and(|status| status.success());
            state.audit("services.opened", &format!("{}: {host}{}", owner.name(), if opened { "" } else { " (didn't open)" }));
        }
        ("hide", Some(kind)) => {
            let hidden = {
                let mut hub = HUB.lock().unwrap();
                hub.hidden.insert(kind);
                hub.hidden.clone()
            };
            save_hidden(&state, &hidden);
            state.audit("services.hidden", kind.name());
            publish_now();
        }
        ("refresh", Some(kind)) => {
            if let Some(live) = HUB.lock().unwrap().live.get_mut(&kind) {
                live.next_at = 0;
            }
            crate::scheduler::tick_now();
        }
        ("show" | "select", Some(kind)) => {
            if let Some(live) = HUB.lock().unwrap().live.get_mut(&kind) {
                live.shown.bring_forward(crate::surface::now_ms());
            }
            publish_now();
        }
        _ => {}
    }
}

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
    /// Every service as Settings shows it. Asking counts as watching for two minutes.
    Status,
    /// Saves a service's key, or removes it when the key is empty.
    SaveKey { service: Kind, key: String },
    /// Shows a service on the island again after it was hidden there.
    Show { service: Kind },
}

/// One service as Settings shows it. Never the key: only whether one is saved.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: &'static str,
    pub name: &'static str,
    pub configured: bool,
    pub hidden: bool,
    pub health: Health,
    /// What the card says, once polled.
    pub title: Option<String>,
    pub message: Option<String>,
    /// Unix seconds.
    pub checked_at: Option<i64>,
}

fn statuses() -> Vec<Status> {
    let configured: Vec<bool> = Kind::ALL.iter().map(|kind| crate::web::credential_saved(kind.id())).collect();
    let hub = HUB.lock().unwrap();
    Kind::ALL
        .into_iter()
        .zip(configured)
        .map(|(kind, configured)| {
            let live = hub.live.get(&kind);
            Status {
                id: kind.id(),
                name: kind.name(),
                configured,
                hidden: hub.hidden.contains(&kind),
                health: live.map(|live| live.health).unwrap_or_default(),
                title: live.and_then(|live| live.snapshot.as_ref()).map(|snapshot| snapshot.title.clone()),
                message: live.and_then(|live| live.message.clone()),
                checked_at: live.and_then(|live| live.checked_at),
            }
        })
        .collect()
}

/// Checks a key's shape before it is saved. Stripe needs a restricted key, never a secret one.
fn check_key(kind: Kind, key: &str) -> Result<(), String> {
    if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("That doesn't look like a {} key.", kind.name()));
    }
    if kind == Kind::Stripe && !(key.starts_with("rk_live_") || key.starts_with("rk_test_")) {
        return Err("Use a Stripe restricted key (rk_…) that can only read charges.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn services_call(op: Op, app: tauri::AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    match op {
        Op::Ping => Ok(Value::Bool(true)),
        Op::Status => {
            let now = Utc::now().timestamp();
            let was_watched = {
                let mut hub = HUB.lock().unwrap();
                if !hub.hidden_loaded {
                    hub.hidden = load_hidden(&state);
                    hub.hidden_loaded = true;
                }
                std::mem::replace(&mut hub.watched_until, now + WATCH_SECONDS) > now
            };
            if !was_watched {
                crate::scheduler::tick_now();
            }
            serde_json::to_value(statuses()).map_err(|error| error.to_string())
        }
        Op::SaveKey { service, key } => {
            let _gate = state.config_gate.lock().await;
            let key = key.trim().to_string();
            if !key.is_empty() {
                check_key(service, &key)?;
            }
            crate::web::set_credential(service.id(), &key)?;
            state.audit(
                "credential.updated",
                &format!("{} credential {} in macOS Keychain", service.id(), if key.is_empty() { "removed" } else { "updated" }),
            );
            reset(service);
            publish_now();
            crate::scheduler::tick_now();
            serde_json::to_value(statuses()).map_err(|error| error.to_string())
        }
        Op::Show { service } => {
            let hidden = {
                let mut hub = HUB.lock().unwrap();
                hub.hidden.remove(&service);
                hub.hidden.clone()
            };
            save_hidden(&state, &hidden);
            if let Some(live) = HUB.lock().unwrap().live.get_mut(&service) {
                live.next_at = 0;
            }
            crate::scheduler::tick_now();
            serde_json::to_value(statuses()).map_err(|error| error.to_string())
        }
    }
}

#[cfg(test)]
pub(crate) mod recorded {
    //! Replays a recorded answer (status, headers, body) through the same reading path a live
    //! answer takes.
    use super::fetch::{read, Failure, RateLimit};
    use super::Kind;
    use serde_json::Value;

    pub const NOW: i64 = 1_794_820_320;

    pub async fn replay(kind: Kind, fixture: &Value, cap: usize) -> (Result<Value, Failure>, RateLimit) {
        let mut builder = tauri::http::Response::builder().status(fixture["status"].as_u64().unwrap() as u16);
        for (name, value) in fixture["headers"].as_object().into_iter().flatten() {
            builder = builder.header(name.as_str(), value.as_str().unwrap());
        }
        let body = serde_json::to_vec(&fixture["body"]).unwrap();
        let response = reqwest::Response::from(builder.body(body).unwrap());
        read(Ok(response), kind, cap, NOW).await
    }

    /// The recorded answers from each service to a key it doesn't know.
    pub fn refusals() -> Value {
        serde_json::from_str(include_str!("fixtures/recorded.json")).unwrap()
    }

    /// A service's fixture file.
    pub fn fixture(name: &str) -> Value {
        let raw = match name {
            "github" => include_str!("fixtures/github.json"),
            "gitlab" => include_str!("fixtures/gitlab.json"),
            "gitea" => include_str!("fixtures/gitea.json"),
            "vercel" => include_str!("fixtures/vercel.json"),
            "stripe" => include_str!("fixtures/stripe.json"),
            "n8n" => include_str!("fixtures/n8n.json"),
            "notion" => include_str!("fixtures/notion.json"),
            "calcom" => include_str!("fixtures/calcom.json"),
            "resend" => include_str!("fixtures/resend.json"),
            _ => panic!("no fixture {name}"),
        };
        serde_json::from_str(raw).unwrap()
    }

    /// An answer just over the cap is refused before it is parsed.
    pub async fn assert_capped(kind: Kind, cap: usize) {
        let padding = "x".repeat(cap);
        let fixture = serde_json::json!({"status": 200, "headers": {}, "body": {"padding": padding}});
        let (value, _) = replay(kind, &fixture, cap).await;
        match value {
            Err(Failure::Other(message)) => assert!(message.contains("safe size limit"), "{message}"),
            other => panic!("{kind:?} read an answer over its cap: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fetch::RateLimit;
    use super::recorded::{refusals, replay, NOW};
    use super::*;

    #[test]
    fn every_service_has_one_key_and_a_host() {
        let ids: Vec<&str> = Kind::ALL.iter().map(|kind| kind.id()).collect();
        assert_eq!(ids, KEY_IDS);
        for kind in Kind::ALL {
            assert_eq!(Kind::from_id(kind.id()), Some(kind));
            assert!(crate::web::provider_valid(kind.id()).is_ok(), "{kind:?}");
            assert!(crate::web::credential_valid(kind.id()).is_ok(), "{kind:?}");
            assert_eq!(crate::budget::display_name(kind.id()), kind.name());
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.id());
            assert!(kind.tint().len() == 7 && kind.tint().starts_with('#'));
        }
        let settings = Settings::default();
        assert_eq!(settings.host(Kind::Gitlab).as_deref(), Some("gitlab.com"));
        assert_eq!(settings.host(Kind::Gitea), None);
        assert_eq!(settings.host(Kind::N8n), None);
        assert_eq!(settings.host(Kind::Github).as_deref(), Some("api.github.com"));
    }

    #[test]
    fn hosts_are_https_names_without_paths() {
        assert_eq!(normalize_host(" https://Git.Example.com:8443/ ").unwrap(), "git.example.com:8443");
        assert_eq!(normalize_host("n8n.example.com").unwrap(), "n8n.example.com");
        assert_eq!(normalize_host("https://gitlab.example.com:443").unwrap(), "gitlab.example.com");
        for bad in ["http://git.example.com", "git.example.com/path", "user@git.example.com", "", "git example.com", "https://", "a?b", "javascript:alert(1)"] {
            assert!(normalize_host(bad).is_err(), "{bad}");
        }
        let mut settings = Settings { gitea_host: "git.example.com".into(), ..Default::default() };
        assert!(settings.validate().is_ok());
        settings.n8n_host = "https://n8n.example.com/".into();
        assert!(settings.validate().is_err());
        settings.n8n_host = "N8N.example.com".into();
        assert!(settings.validate().is_err());
        assert!(serde_json::from_value::<Settings>(json!({"gitlabHost": "x", "token": "y"})).is_err());
    }

    #[test]
    fn keys_are_checked_before_they_are_saved() {
        assert!(check_key(Kind::Stripe, "sk_live_abc").is_err());
        assert!(check_key(Kind::Stripe, "pk_live_abc").is_err());
        assert!(check_key(Kind::Stripe, "rk_live_abc").is_ok());
        assert!(check_key(Kind::Stripe, "rk_test_abc").is_ok());
        assert!(check_key(Kind::Github, "ghp_abc def").is_err());
        assert!(check_key(Kind::Github, "github_pat_abc").is_ok());
    }

    #[test]
    fn polls_slow_down_after_errors_and_wait_for_the_service() {
        let none = RateLimit::default();
        let github = Kind::Github;
        assert_eq!(next_poll(github, &Ok(false), 0, &none, NOW), NOW + 180);
        assert_eq!(next_poll(github, &Ok(true), 0, &none, NOW), NOW + BUSY_EVERY);
        let other = Err(Failure::Other("down".into()));
        assert_eq!(next_poll(github, &other, 1, &none, NOW), NOW + 180);
        assert_eq!(next_poll(github, &other, 2, &none, NOW), NOW + 360);
        assert_eq!(next_poll(github, &other, 3, &none, NOW), NOW + 720);
        assert_eq!(next_poll(github, &other, 30, &none, NOW), NOW + LONGEST_WAIT);
        // The service's own reset wins, but never beyond an hour.
        let spent = RateLimit { remaining: Some(0), reset_at: Some(NOW + 900), retry_at: None };
        assert_eq!(next_poll(github, &Ok(false), 0, &spent, NOW), NOW + 900);
        let far = RateLimit { remaining: Some(0), reset_at: Some(NOW + 86_400), retry_at: None };
        assert_eq!(next_poll(github, &Ok(false), 0, &far, NOW), NOW + LONGEST_WAIT);
        let limited = Err(Failure::Limited { until: 0, message: "slow".into() });
        assert_eq!(next_poll(github, &limited, 0, &none, NOW), NOW + 60);
        let retry = RateLimit { retry_at: Some(NOW + 300), ..Default::default() };
        assert_eq!(next_poll(github, &limited, 0, &retry, NOW), NOW + 300);
        // A refused key waits six hours; a used-up allowance an hour.
        assert_eq!(next_poll(github, &Err(Failure::Rejected("no".into())), 0, &none, NOW), NOW + 6 * 3600);
        assert_eq!(next_poll(github, &Err(Failure::Budget("used".into())), 0, &none, NOW), NOW + 3600);
        assert_eq!(next_poll(github, &Err(Failure::Cancelled), 0, &none, NOW), NOW);
    }

    #[tokio::test]
    async fn each_services_recorded_refusal_stops_the_key_or_waits() {
        let recorded = refusals();
        for (name, kind) in [
            ("github_graphql_badkey", Kind::Github),
            ("gitlab_graphql_badkey", Kind::Gitlab),
            ("gitea_user_badkey", Kind::Gitea),
            ("vercel_badkey", Kind::Vercel),
            ("stripe_badkey", Kind::Stripe),
            ("notion_badkey", Kind::Notion),
            ("calcom_badkey", Kind::Calcom),
            ("resend_badkey", Kind::Resend),
        ] {
            let (value, _) = replay(kind, &recorded[name], 65_536).await;
            assert!(matches!(value, Err(Failure::Rejected(_))), "{name}: {value:?}");
        }
        // GitHub without a key: a real rate limit, waited out until the reset it gave.
        let (value, limit) = replay(Kind::Github, &recorded["github_graphql_noauth"], 65_536).await;
        assert!(matches!(value, Err(Failure::Limited { .. })), "{value:?}");
        assert_eq!(limit.wait_until(NOW), Some(1_794_851_595));
        assert_eq!(next_poll(Kind::Github, &value.map(|_| false), 0, &limit, NOW), NOW + LONGEST_WAIT);
        // Gitea's structured header.
        let (_, limit) = replay(Kind::Gitea, &recorded["gitea_user_badkey"], 65_536).await;
        assert_eq!(limit.remaining, Some(1999));
    }

    #[test]
    fn settling_records_health_and_audits_only_changes() {
        let kind = Kind::Resend;
        reset(kind);
        let generation = HUB.lock().unwrap().live[&kind].generation;
        let snapshot = Snapshot { title: "3 sent today".into(), ..Default::default() };
        let notes = settle(kind, generation, Ok(snapshot.clone()), &RateLimit::default(), Memo::default(), NOW);
        assert_eq!(notes, vec![("services.connected".to_string(), "Resend".to_string())]);
        assert!(settle(kind, generation, Ok(snapshot), &RateLimit::default(), Memo::default(), NOW).is_empty());
        let notes = settle(kind, generation, Err(Failure::Rejected("refused".into())), &RateLimit::default(), Memo::default(), NOW);
        assert_eq!(notes[0].0, "services.key_rejected");
        {
            let hub = HUB.lock().unwrap();
            let live = &hub.live[&kind];
            assert!(live.snapshot.is_none() && live.health == Health::Rejected);
            assert_eq!(live.next_at, NOW + 6 * 3600);
        }
        // An answer from before the key changed is dropped.
        reset(kind);
        let notes = settle(kind, generation, Ok(Snapshot::default()), &RateLimit::default(), Memo::default(), NOW);
        assert!(notes.is_empty());
        assert!(HUB.lock().unwrap().live[&kind].snapshot.is_none());
    }

    #[test]
    fn only_a_link_a_card_shows_is_opened() {
        let mut hub = Hub::default();
        let row = Row::new("pr", "PR").url(Some("https://github.com/a/b/pull/1".into()));
        hub.live.insert(Kind::Github, Live { snapshot: Some(Snapshot { rows: vec![row], ..Default::default() }), ..Default::default() });
        assert_eq!(link_owner(&hub, "https://github.com/a/b/pull/1"), Some(Kind::Github));
        assert_eq!(link_owner(&hub, "https://github.com/a/b/pull/2"), None);
        assert_eq!(link_owner(&hub, "file:///etc/passwd"), None);
        hub.hidden.insert(Kind::Github);
        assert_eq!(link_owner(&hub, "https://github.com/a/b/pull/1"), None);
    }

    #[test]
    fn graph_and_names_are_fetched_again_only_when_due() {
        let mut memo = Memo::default();
        assert!(memo.due("graph", 3600, NOW));
        memo.mark("graph", NOW);
        assert!(!memo.due("graph", 3600, NOW + 3599));
        assert!(memo.due("graph", 3600, NOW + 3600));
    }
}
