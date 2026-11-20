//! Mail in the island and the app (stream E): every inbox with filters for unread,
//! important (flagged) and today; reading and replying; changes that wait for Allow, are read
//! back from Mail and are logged with Undo; sorting by VIPs, banned words and rules; and a
//! plan the brain prepares that runs only on Run. Nothing is ever deleted and nothing is
//! sent: replies, merge mail and dropped files become drafts.
//!
//! Background reads happen only while Mail is open and macOS already lets Buntini script it,
//! so they never open Mail or put an Automation prompt on screen.
pub mod log;
pub mod merge;
pub mod plan;
pub mod rules;
pub mod script;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::i18n::{t, tf};
use crate::surface::cards::mail as card;
use crate::surface::{self, Action, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency};
use rules::Why;

/// Emails read from each inbox.
const DEPTH: usize = 150;
/// Rows a listing shows at first, and how many more each "more" adds.
const PAGE: usize = 12;
const MAX_SHOWN: usize = 60;
/// How long a change waits for Allow.
const PENDING_FOR: Duration = Duration::from_secs(10 * 60);
/// How often Mail is looked at in the background while it is open.
const POLL: Duration = Duration::from_secs(120);
/// How long the card stays after the last touch.
const CARD_MS: u64 = 90_000;
const MAX_FILES: usize = 20;
const MAX_ATTACHED: u64 = 25 * 1024 * 1024;
const TINT: &str = "#3D8BFD";

/// A mailbox by its account's name (empty for On My Mac) and its path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mailbox {
    pub account: String,
    pub path: String,
}

/// An email as a listing reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Mail's own number, which changes when the email moves.
    pub id: i64,
    /// The Message-ID header, which doesn't.
    pub header: String,
    /// Unix seconds.
    pub at: i64,
    pub read: bool,
    pub flagged: bool,
    pub junk: bool,
    pub sender: String,
    pub subject: String,
    pub mailbox: Mailbox,
}

impl Message {
    pub fn target(&self) -> Target {
        Target {
            id: self.id,
            header: self.header.clone(),
            mailbox: self.mailbox.clone(),
            sender: self.sender.clone(),
            subject: self.subject.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    Read,
    Flagged,
    Junk,
}

impl Mark {
    fn word(self) -> &'static str {
        match self {
            Mark::Read => "read",
            Mark::Flagged => "flagged",
            Mark::Junk => "junk",
        }
    }
}

/// The email a step changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: i64,
    pub header: String,
    pub mailbox: Mailbox,
    pub sender: String,
    pub subject: String,
}

/// One change in Mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Mark { target: Target, what: Mark, value: bool },
    Move { target: Target, to: Mailbox },
    Junk { target: Target },
    Reply { target: Target, body: String },
    Draft { to: String, subject: String, body: String, files: Vec<PathBuf>, shown: bool },
}

/// Saved under `settings.mail`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {
    pub rules: Vec<rules::Rule>,
    pub banned: Vec<String>,
    pub vips: Vec<String>,
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        rules::validate(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Filter {
    #[default]
    All,
    Unread,
    /// Flagged in Mail.
    Important,
    Today,
}

impl Filter {
    fn word(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Unread => "unread",
            Filter::Important => "important",
            Filter::Today => "today",
        }
    }

    fn from_word(word: &str) -> Option<Filter> {
        serde_json::from_value(Value::String(word.into())).ok()
    }
}

/// The emails a filter and search show, newest first. A search looks at sender and subject;
/// `from:` and `subject:` narrow a word to one of them, and every word must match.
pub fn filtered(pool: &[Message], filter: Filter, query: &str, now: chrono::DateTime<chrono::Local>) -> Vec<Message> {
    let today = now.date_naive();
    let words: Vec<(Option<&str>, String)> = query
        .split_whitespace()
        .map(|word| match word.split_once(':') {
            Some((field, rest)) if ["from", "subject"].contains(&field.to_lowercase().as_str()) && !rest.is_empty() => {
                (Some(if field.eq_ignore_ascii_case("from") { "from" } else { "subject" }), rules::fold(rest))
            }
            _ => (None, rules::fold(word)),
        })
        .collect();
    let mut out: Vec<Message> = pool
        .iter()
        .filter(|m| match filter {
            Filter::All => true,
            Filter::Unread => !m.read,
            Filter::Important => m.flagged,
            Filter::Today => chrono::Local.timestamp_opt(m.at, 0).single().is_some_and(|at| at.date_naive() == today),
        })
        .filter(|m| {
            let (sender, subject) = (rules::fold(&m.sender), rules::fold(&m.subject));
            words.iter().all(|(field, word)| match field {
                Some("from") => sender.contains(word.as_str()),
                Some(_) => subject.contains(word.as_str()),
                None => sender.contains(word.as_str()) || subject.contains(word.as_str()),
            })
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    out
}

/// The sender's name, or their address when there is none.
pub fn who(sender: &str) -> String {
    match sender.rsplit_once('<') {
        Some((name, address)) => {
            let name = name.trim().trim_matches('"').trim();
            if name.is_empty() { address.trim_end_matches('>').trim().to_string() } else { name.to_string() }
        }
        None => sender.trim().to_string(),
    }
}

// ----- What the app and the island see -----

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub from: String,
    pub address: String,
    pub subject: String,
    pub unread: bool,
    pub flagged: bool,
    pub at: i64,
    pub mailbox: String,
}

impl From<&Message> for Row {
    fn from(m: &Message) -> Self {
        Row {
            id: m.id.to_string(),
            from: who(&m.sender),
            address: rules::address(&m.sender),
            subject: m.subject.clone(),
            unread: !m.read,
            flagged: m.flagged,
            at: m.at,
            mailbox: log::leaf(&m.mailbox.path).to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub filter: Filter,
    pub query: String,
    pub messages: Vec<Row>,
    pub unread: usize,
    pub sortable: usize,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub id: String,
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRow {
    pub id: String,
    pub label: String,
    pub at: i64,
    pub undoable: bool,
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub id: String,
    pub summary: String,
    pub lines: Vec<String>,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub changed: Vec<String>,
    pub failed: Vec<String>,
}

// ----- State -----

struct Pending {
    view: Waiting,
    steps: Vec<(Step, Why)>,
    made: Instant,
}

struct Planned {
    plan: plan::Plan,
    messages: Vec<Message>,
    pages: Vec<merge::Page>,
}

#[derive(Default)]
struct Hub {
    app: Option<AppHandle>,
    dir: Option<PathBuf>,
    /// Mailboxes read instead of every inbox. Only live tests set it.
    boxes: Vec<Mailbox>,
    filter: Filter,
    query: String,
    shown: usize,
    pool: Vec<Message>,
    read: bool,
    closed: bool,
    sortable: usize,
    open: Option<(Message, script::Opened)>,
    pending: Option<Pending>,
    planned: Option<Planned>,
    log: log::Log,
    status: Option<String>,
    /// When the card was last touched, on the board's clock; none while it's hidden.
    card_at: Option<u64>,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(|| Mutex::new(Hub { shown: PAGE, ..Default::default() }));
/// One run of changes at a time.
static WORK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn hub() -> std::sync::MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn app_settings() -> Settings {
    let app = hub().app.clone();
    app.and_then(|app| app.state::<crate::controller::AppState>().store.lock().ok()?.settings().ok()).map(|s| s.mail).unwrap_or_default()
}

fn not_allowed() -> String {
    t("mail.error.notAllowed")
}

/// Whether Mail is allowed in Settings → Apps. Nothing here reads or changes Mail otherwise.
fn allowed() -> Result<(), String> {
    let Some(app) = hub().app.clone() else { return Ok(()) };
    let settings = app.state::<crate::controller::AppState>().store.lock().map_err(|_| not_allowed())?.settings()?;
    if settings.apps.iter().any(|app| app == "mail") {
        Ok(())
    } else {
        Err(not_allowed())
    }
}

fn audit(action: &str, detail: &str) {
    if let Some(app) = hub().app.clone() {
        app.state::<crate::controller::AppState>().audit(action, detail);
    }
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join("log.json")
}

/// Delete everything: the log of changes, the pages and files copied for drafts, and every
/// email, plan and change held in memory. Waits for a run under way to finish first.
pub async fn erase() -> Result<(), String> {
    let _one = WORK.lock().await;
    let dir = {
        let mut hub = hub();
        hub.log = log::Log::default();
        hub.pending = None;
        hub.planned = None;
        hub.open = None;
        hub.pool.clear();
        hub.query.clear();
        hub.status = None;
        hub.dir.clone()
    };
    publish();
    match dir {
        Some(dir) => match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err("Couldn’t remove Mail’s log and copied files.".into()),
            _ => Ok(()),
        },
        None => Ok(()),
    }
}

/// Starts the island's mail card and the background look at Mail.
pub fn start(app: AppHandle) {
    let dir = app.path().app_data_dir().ok().map(|data| data.join("mail"));
    {
        let mut hub = hub();
        if let Some(dir) = &dir {
            hub.log = log::Log::load(&log_path(dir));
            merge::tidy(&dir.join("merge"));
            merge::tidy(&dir.join("attachments"));
        }
        hub.dir = dir;
        hub.app = Some(app);
    }
    surface::on_action(Source::Mail, |action| {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = act(action).await {
                hub().status = Some(error);
                publish();
            }
        });
    });
    tauri::async_runtime::spawn(async {
        // Mail is left to settle after launch before anything reads it.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let _ = refresh(true).await;
            tokio::time::sleep(POLL).await;
        }
    });
}

// ----- Reading -----

/// The newest emails in `boxes`, or in every inbox when it's empty, newest first.
pub async fn list(boxes: &[Mailbox]) -> Result<Vec<Message>, String> {
    let mut argv = vec![DEPTH.to_string()];
    for mailbox in boxes {
        argv.push(mailbox.account.clone());
        argv.push(mailbox.path.clone());
    }
    let mut messages = script::parse_list(&script::run(script::LIST, &argv, 90).await?)?;
    messages.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    Ok(messages)
}

/// Every mailbox mail can be moved into.
pub async fn mailboxes() -> Result<Vec<Mailbox>, String> {
    allowed()?;
    let mut boxes = script::parse_mailboxes(&script::run(script::MAILBOXES, &[], 60).await?);
    boxes.retain(|mailbox| !rules::forbidden_destination(mailbox));
    Ok(boxes)
}

async fn read(message: &Message) -> Result<script::Opened, String> {
    let argv = [message.mailbox.account.clone(), message.mailbox.path.clone(), message.id.to_string(), message.header.clone()];
    script::parse_read(&script::run(script::READ, &argv, 40).await?)
}

/// Reads every inbox again. A `quiet` read is one nobody asked for: it goes ahead only when
/// macOS won't ask about it, and Mail's own guard keeps a closed Mail closed.
pub async fn refresh(quiet: bool) -> Result<(), String> {
    if let Err(error) = allowed() {
        {
            let mut hub = hub();
            hub.pool.clear();
            hub.read = false;
            hub.open = None;
        }
        publish();
        return Err(error);
    }
    if quiet && !crate::apps::may_script_silently("Mail").await {
        return Err("Mail hasn’t been allowed yet.".into());
    }
    let boxes = hub().boxes.clone();
    let listed = list(&boxes).await;
    let settings = app_settings();
    {
        let mut hub = hub();
        match listed {
            Ok(pool) => {
                hub.sortable = rules::sort(&settings, &pool).len();
                hub.pool = pool;
                hub.read = true;
                hub.closed = false;
            }
            Err(error) if script::closed(&error) => {
                hub.pool.clear();
                hub.closed = true;
                hub.sortable = 0;
            }
            Err(error) => return Err(error),
        }
    }
    publish();
    Ok(())
}

pub fn listing() -> Listing {
    let hub = hub();
    let rows = filtered(&hub.pool, hub.filter, &hub.query, chrono::Local::now());
    Listing {
        filter: hub.filter,
        query: hub.query.clone(),
        messages: rows.iter().take(hub.shown).map(Row::from).collect(),
        unread: hub.pool.iter().filter(|m| !m.read).count(),
        sortable: hub.sortable,
        closed: hub.closed,
    }
}

fn message(id: &str) -> Result<Message, String> {
    let id: i64 = id.trim().parse().map_err(|_| t("mail.error.pickEmail"))?;
    let hub = hub();
    hub.pool
        .iter()
        .find(|m| m.id == id)
        .or_else(|| hub.open.as_ref().map(|(m, _)| m).filter(|m| m.id == id))
        .cloned()
        .ok_or_else(|| t("mail.error.notInList"))
}

pub async fn open(id: &str) -> Result<script::Opened, String> {
    allowed()?;
    let message = message(id)?;
    let opened = read(&message).await?;
    hub().open = Some((message, opened.clone()));
    publish();
    Ok(opened)
}

// ----- Proposing -----

fn propose(title: String, lines: Vec<String>, steps: Vec<(Step, Why)>) -> Waiting {
    let view = Waiting { id: uuid::Uuid::new_v4().to_string(), title, lines };
    {
        let mut hub = hub();
        hub.pending = Some(Pending { view: view.clone(), steps, made: Instant::now() });
        hub.status = None;
    }
    publish();
    view
}

fn about(target: &Target) -> String {
    format!("{} · {}", who(&target.sender), target.subject)
}

pub fn propose_mark(id: &str, what: Mark, value: bool) -> Result<Waiting, String> {
    let message = message(id)?;
    let title = match (what, value) {
        (Mark::Read, true) => t("mail.pending.markRead"),
        (Mark::Read, false) => t("mail.pending.markUnread"),
        (Mark::Flagged, true) => t("surface.mail.flag"),
        (Mark::Flagged, false) => t("surface.mail.unflag"),
        (Mark::Junk, true) => t("surface.mail.junk"),
        (Mark::Junk, false) => t("mail.pending.notJunk"),
    };
    let target = message.target();
    Ok(propose(title, vec![about(&target)], vec![(Step::Mark { target, what, value }, Why::Manual)]))
}

pub fn propose_junk(id: &str) -> Result<Waiting, String> {
    let target = message(id)?.target();
    Ok(propose(t("surface.mail.junk"), vec![about(&target)], vec![(Step::Junk { target }, Why::Manual)]))
}

pub fn propose_move(id: &str, to: Mailbox) -> Result<Waiting, String> {
    let target = message(id)?.target();
    if rules::same_box(&to, &target.mailbox) {
        return Err(t("mail.error.alreadyThere"));
    }
    if rules::forbidden_destination(&to) {
        return Err(tf("mail.error.notMovedTo", &[("mailbox", &to.path)]));
    }
    let title = tf("mail.pending.moveTo", &[("mailbox", &log::leaf(&to.path))]);
    Ok(propose(title, vec![about(&target)], vec![(Step::Move { target, to }, Why::Manual)]))
}

fn reply_subject(subject: &str) -> String {
    let lower = subject.trim_start().to_lowercase();
    if lower.starts_with("re:") {
        subject.trim().to_string()
    } else {
        format!("Re: {}", subject.trim())
    }
}

pub fn propose_reply(id: &str, body: &str) -> Result<Waiting, String> {
    let target = message(id)?.target();
    let body = body.trim();
    if body.is_empty() || body.chars().count() > 4000 {
        return Err(t("mail.error.replyLength"));
    }
    let to = rules::address(&target.sender);
    if !plan::plain_address(&to) {
        return Err(t("mail.error.noReplyAddress"));
    }
    let lines = vec![tf("mail.pending.replyTo", &[("address", &to)]), reply_subject(&target.subject), body.to_string()];
    Ok(propose(t("mail.step.replyDraft"), lines, vec![(Step::Reply { target, body: body.into() }, Why::Manual)]))
}

/// Files dropped on the island, as attachments on a new draft.
pub fn propose_attach(files: &[String]) -> Result<Waiting, String> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(tf("mail.error.attachCount", &[("max", &MAX_FILES)]));
    }
    let mut paths = Vec::new();
    let mut total = 0;
    for file in files {
        let path = PathBuf::from(file);
        let meta = std::fs::metadata(&path).map_err(|_| tf("mail.error.missingFile", &[("file", file)]))?;
        if !path.is_absolute() || !meta.is_file() {
            return Err(t("mail.error.onlyFiles"));
        }
        total += meta.len();
        paths.push(path);
    }
    if total > MAX_ATTACHED {
        return Err(t("mail.error.tooLarge"));
    }
    let names: Vec<String> = paths.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    let subject = paths[0].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let step = Step::Draft { to: String::new(), subject, body: String::new(), files: paths, shown: true };
    Ok(propose(t("mail.pending.newDraft"), names, vec![(step, Why::Manual)]))
}

fn sort_steps(settings: &Settings, pool: &[Message]) -> Vec<(Step, Why, String)> {
    rules::sort(settings, pool)
        .into_iter()
        .map(|(message, decision)| {
            let target = message.target();
            let line = about(&target);
            let (step, line) = match decision.verdict {
                rules::Verdict::Flag => (Step::Mark { target, what: Mark::Flagged, value: true }, format!("{} · {line}", t("surface.mail.flag"))),
                rules::Verdict::Read => (Step::Mark { target, what: Mark::Read, value: true }, format!("{} · {line}", t("mail.step.read"))),
                rules::Verdict::Junk => (Step::Junk { target }, format!("{} · {line}", t("surface.mail.junk"))),
                rules::Verdict::Move(to) => {
                    let line = format!("{} · {line}", tf("mail.step.moveTo", &[("mailbox", &log::leaf(&to.path))]));
                    (Step::Move { target, to }, line)
                }
            };
            (step, decision.why, line)
        })
        .collect()
}

/// Every change sorting would make now, listed for Allow. None when there's nothing to do.
pub fn propose_sort() -> Result<Option<Waiting>, String> {
    let settings = app_settings();
    let pool = hub().pool.clone();
    let steps = sort_steps(&settings, &pool);
    if steps.is_empty() {
        return Ok(None);
    }
    let lines = steps.iter().map(|(_, _, line)| line.clone()).collect();
    let title = tf("surface.mail.sort", &[("count", &steps.len())]);
    Ok(Some(propose(title, lines, steps.into_iter().map(|(step, why, _)| (step, why)).collect())))
}

pub fn propose_undo(entry: &str) -> Result<Waiting, String> {
    let (steps, label) = {
        let hub = hub();
        let found = hub.log.get(entry).ok_or_else(|| t("mail.error.notInLog"))?;
        (found.undo()?, found.label())
    };
    let why = Why::Undo { of: entry.to_string() };
    Ok(propose(t("surface.undo"), vec![label], steps.into_iter().map(|step| (step, why.clone())).collect()))
}

pub fn deny(id: &str) {
    let mut hub = hub();
    if hub.pending.as_ref().is_some_and(|p| p.view.id == id) {
        hub.pending = None;
    }
    drop(hub);
    publish();
}

// ----- Running -----

async fn mail_open() -> Result<(), String> {
    script::run("  return \"open\"\n", &[], 15).await.map(|_| ())
}

fn box_args(target: &Target) -> [String; 4] {
    [target.mailbox.account.clone(), target.mailbox.path.clone(), target.id.to_string(), target.header.clone()]
}

/// Makes one change, reads it back from Mail, and says what changed.
async fn run_step(step: &Step, why: Why) -> Result<log::Entry, String> {
    use log::Change;
    match step {
        Step::Mark { target, what, value } => {
            let mut argv = box_args(target).to_vec();
            argv.extend([what.word().to_string(), value.to_string()]);
            let (was, now, id) = script::parse_mark(&script::run(script::MARK, &argv, 40).await?)?;
            if now != *value {
                return Err(t("mail.error.notKept"));
            }
            Ok(log::Entry::new(Change::Mark { what: *what, was, now, at: target.mailbox.clone(), id }, Some(target), &target.subject, why))
        }
        Step::Move { target, to } => {
            if rules::same_box(to, &target.mailbox) {
                return Err(t("mail.error.alreadyThere"));
            }
            if rules::forbidden_destination(to) {
                return Err(tf("mail.error.notMovedTo", &[("mailbox", &to.path)]));
            }
            // Without its Message-ID an email can't be told from others like it, here or after.
            if target.header.trim().is_empty() {
                return Err(t("mail.error.incompleteEmail"));
            }
            let mut argv = box_args(target).to_vec();
            argv.extend([to.account.clone(), to.path.clone()]);
            let (id, left, path) = script::parse_move(&script::run(script::MOVE, &argv, 60).await?)?;
            // A copy still where it was means Mail didn't move it, whatever is in the destination.
            if left > 0 {
                return Err(t("mail.error.notKept"));
            }
            let to = Mailbox { account: to.account.clone(), path };
            Ok(log::Entry::new(Change::Move { from: target.mailbox.clone(), to, id }, Some(target), &target.subject, why))
        }
        Step::Junk { target } => {
            if target.header.trim().is_empty() {
                return Err(t("mail.error.incompleteEmail"));
            }
            let (was, now, at, id) = script::parse_junk(&script::run(script::JUNK, &box_args(target), 60).await?)?;
            if !now {
                return Err(t("mail.error.notKept"));
            }
            Ok(log::Entry::new(Change::Junk { from: target.mailbox.clone(), to: at, was, id }, Some(target), &target.subject, why))
        }
        Step::Reply { target, body } => {
            let to = rules::address(&target.sender);
            let mut args = serde_json::Map::new();
            for (key, value) in [
                ("message", target.id.to_string()),
                ("message_id", target.header.clone()),
                ("to", to.clone()),
                ("subject", reply_subject(&target.subject)),
                ("body", body.clone()),
            ] {
                args.insert(key.into(), Value::String(value));
            }
            let receipt = crate::actions::execute_with("reply_email", &args, Path::new("")).await;
            if receipt.outcome != crate::actions::ActionOutcome::Succeeded {
                return Err(receipt.message);
            }
            Ok(log::Entry::new(Change::Reply { to }, Some(target), &reply_subject(&target.subject), why))
        }
        Step::Draft { to, subject, body, files, shown } => {
            let mut argv = vec![to.clone(), subject.clone(), body.clone(), if *shown { "1" } else { "0" }.to_string()];
            argv.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
            let saved = script::parse_draft(&script::run(script::DRAFT, &argv, 90).await?)?;
            if saved.subject.trim() != subject.trim() {
                return Err(t("mail.error.otherSubject"));
            }
            if !to.is_empty() && saved.to.iter().map(|a| a.to_lowercase()).collect::<Vec<_>>() != [to.to_lowercase()] {
                return Err(t("mail.error.otherRecipient"));
            }
            // The very files handed over, by name, not just as many of them.
            // Compared without accents, which Mail may hand back decomposed.
            let key = |name: &str| rules::fold(&name.chars().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>());
            let mut wanted: Vec<String> = files.iter().filter_map(|f| f.file_name()).map(|n| key(&n.to_string_lossy())).collect();
            let mut attached: Vec<String> = saved.attachments.iter().map(|name| key(name)).collect();
            wanted.sort();
            attached.sort();
            if attached != wanted {
                return Err(t("mail.error.attachmentsLost"));
            }
            Ok(log::Entry::new(Change::Draft { id: saved.id, to: saved.to, attachments: saved.attachments }, None, subject, why))
        }
    }
}

/// Copies files to be attached into Buntini's own folder, so Mail reads them from there.
fn stage(files: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let dir = hub().dir.clone().ok_or_else(|| t("mail.error.notReady"))?.join("attachments").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).map_err(|_| t("mail.error.filesNotCopied"))?;
    files
        .iter()
        .map(|file| {
            let name = file.file_name().map(|n| merge::safe_name(&n.to_string_lossy())).unwrap_or_else(|| "File".into());
            let mut copy = dir.join(&name);
            // Two files with one name each keep their own copy.
            let mut n = 2;
            while copy.exists() {
                let path = Path::new(&name);
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                copy = dir.join(match path.extension() {
                    Some(ext) => format!("{stem} {n}.{}", ext.to_string_lossy()),
                    None => format!("{stem} {n}"),
                });
                n += 1;
            }
            std::fs::copy(file, &copy).map_err(|_| tf("mail.error.fileNotCopied", &[("file", &file.display())]))?;
            Ok(copy)
        })
        .collect()
}

/// Runs steps in order, logging each. `keep_going` carries on past a failure.
async fn run_all(steps: Vec<(Step, Why)>, keep_going: bool) -> Done {
    let _one = WORK.lock().await;
    let mut done = Done { changed: vec![], failed: vec![] };
    if let Err(error) = allowed() {
        done.failed.push(error);
        return done;
    }
    if let Err(error) = mail_open().await {
        done.failed.push(error);
        return done;
    }
    for (step, why) in steps {
        let step = match step {
            Step::Draft { to, subject, body, files, shown } if !files.is_empty() => match stage(&files) {
                Ok(files) => Step::Draft { to, subject, body, files, shown },
                Err(error) => {
                    done.failed.push(error);
                    if keep_going { continue } else { break }
                }
            },
            other => other,
        };
        match run_step(&step, why).await {
            Ok(entry) => {
                audit("mail.change", &entry.label());
                done.changed.push(entry.label());
                let mut hub = hub();
                hub.log.push(entry);
            }
            Err(error) => {
                let what = match &step {
                    Step::Draft { subject, .. } => subject.clone(),
                    Step::Mark { target, .. } | Step::Move { target, .. } | Step::Junk { target } | Step::Reply { target, .. } => about(target),
                };
                done.failed.push(format!("{what}: {error}"));
                if !keep_going {
                    break;
                }
            }
        }
    }
    save_log();
    done
}

fn save_log() {
    let (dir, log) = {
        let hub = hub();
        (hub.dir.clone(), hub.log.clone())
    };
    if let Some(dir) = dir {
        if let Err(error) = log.save(&log_path(&dir)) {
            eprintln!("{error}");
        }
    }
}

/// Runs the change waiting for Allow.
pub async fn approve(id: &str) -> Result<Done, String> {
    let pending = {
        let mut hub = hub();
        match hub.pending.take() {
            Some(p) if p.view.id == id && p.made.elapsed() < PENDING_FOR => p,
            Some(p) if p.view.id != id => {
                hub.pending = Some(p);
                return Err(t("mail.error.noLongerWaiting"));
            }
            _ => return Err(t("mail.error.noLongerWaiting")),
        }
    };
    let undo_of = pending.steps.iter().find_map(|(_, why)| match why {
        Why::Undo { of } => Some(of.clone()),
        _ => None,
    });
    let done = run_all(pending.steps, false).await;
    if done.failed.is_empty() {
        if let Some(of) = undo_of {
            hub().log.mark_undone(&of);
            save_log();
        }
    }
    finish(&done).await;
    Ok(done)
}

async fn finish(done: &Done) {
    hub().status = done.failed.first().cloned();
    if refresh(false).await.is_err() {
        publish();
    }
}

pub fn log_rows(count: usize) -> Vec<LogRow> {
    hub()
        .log
        .recent(count)
        .into_iter()
        .map(|entry| LogRow { id: entry.id.clone(), label: entry.label(), at: entry.at, undoable: entry.undoable(), undone: entry.undone })
        .collect()
}

// ----- The plan -----

fn plan_view(planned: &Planned) -> PlanView {
    PlanView {
        id: planned.plan.id.clone(),
        summary: planned.plan.summary.clone(),
        lines: plan::lines(&planned.plan, &planned.messages),
        problems: planned.plan.problems.clone(),
    }
}

/// Asks the brain for a plan for `request`, with a PDF to split when there is one.
pub async fn make_plan(app: &AppHandle, request: &str, pdf: Option<PathBuf>) -> Result<PlanView, String> {
    let request = request.trim();
    if request.is_empty() || request.chars().count() > 2000 {
        return Err("Say what the plan should do, in up to 2,000 characters.".into());
    }
    let state = app.state::<crate::controller::AppState>();
    let settings = state.store.lock().map_err(|_| "Settings aren’t readable.")?.settings()?;
    let locality = state.models.locality(&settings);
    use crate::privacy::{allows, Share};
    if !allows(locality, &settings, Share::Mail) || (pdf.is_some() && !allows(locality, &settings, Share::Files)) {
        return Err("This brain isn’t allowed to see your mail. Choose what it may see in Settings → Privacy.".into());
    }
    refresh(false).await?;
    let (pool, closed) = {
        let hub = hub();
        (hub.pool.clone(), hub.closed)
    };
    if closed {
        return Err(script::closed_text());
    }
    let mailboxes = mailboxes().await?;
    let pages = match &pdf {
        Some(path) => merge::split(state.models.apple_helper(), path).await?,
        None => vec![],
    };
    let pdf = pdf.map(|path| plan::Pdf {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document.pdf".into()),
        pages: pages.iter().map(|p| p.text.clone()).collect(),
        path,
    });
    let messages: Vec<Message> = pool.into_iter().filter(|m| !m.junk).take(plan::LISTED).collect();
    let mut openings = HashMap::new();
    for message in messages.iter().take(plan::OPENED) {
        if let Ok(opened) = read(message).await {
            openings.insert(message.id, opened.body.chars().take(400).collect::<String>());
        }
    }
    // A small brain names emails by words from them; it picks numbers badly.
    let by_words = state.models.small(&settings);
    let cx = plan::Context { request, messages: &messages, mailboxes: &mailboxes, pdf: pdf.as_ref(), by_words };
    crate::controller::wait_for_slot(&state, Duration::from_secs(15)).await;
    let payload = json!({"messages": plan::prompt(&cx, &openings, &settings.mail), "tools": [], "max_tokens": 1800});
    let turn = state.models.generate(&settings, payload, Arc::new(AtomicBool::new(false)), |_| {}).await?;
    let plan = plan::parse(&turn.text, &cx)?;
    let planned = Planned { plan, messages, pages };
    let view = plan_view(&planned);
    {
        let mut hub = hub();
        hub.planned = Some(planned);
        hub.status = None;
    }
    audit("mail.plan", &view.summary);
    publish();
    Ok(view)
}

pub fn planned() -> Option<PlanView> {
    hub().planned.as_ref().map(plan_view)
}

pub fn drop_plan(id: &str) {
    let mut hub = hub();
    if hub.planned.as_ref().is_some_and(|p| p.plan.id == id) {
        hub.planned = None;
    }
    drop(hub);
    publish();
}

/// The plan's steps as changes in Mail. Merge pages are written for Mail to attach.
fn plan_steps(planned: &Planned, dir: &Path) -> Result<Vec<(Step, Why)>, String> {
    let mut steps = Vec::new();
    for step in &planned.plan.steps {
        let target = |n: &usize| planned.messages.get(*n).map(Message::target).ok_or_else(|| t("mail.error.planUnknownEmail"));
        let step = match step {
            plan::PlanStep::Mark { message, what } => Step::Mark { target: target(message)?, what: *what, value: true },
            plan::PlanStep::Junk { message } => Step::Junk { target: target(message)? },
            plan::PlanStep::Move { message, to } => Step::Move { target: target(message)?, to: to.clone() },
            plan::PlanStep::Reply { message, body } => Step::Reply { target: target(message)?, body: body.clone() },
            plan::PlanStep::Merge { subject, body, sends } => {
                let pdf = planned.plan.pdf.as_ref().ok_or_else(|| t("mail.error.planNoPdf"))?;
                let folder = dir.join("merge").join(&planned.plan.id);
                for (n, send) in sends.iter().enumerate() {
                    let page = planned.pages.get(send.page - 1).ok_or_else(|| t("mail.error.pdfChanged"))?;
                    // Each person's page in a folder of its own: two people whose names read
                    // alike never share, or overwrite, one file.
                    let own = folder.join(n.to_string());
                    std::fs::create_dir_all(&own).map_err(|_| t("mail.error.pagesNotSaved"))?;
                    let file = merge::page_file(&own, &pdf.name, &send.name);
                    if file.exists() {
                        return Err(t("mail.error.pagesNotSaved"));
                    }
                    std::fs::write(&file, &page.pdf).map_err(|_| t("mail.error.pagesNotSaved"))?;
                    let draft = Step::Draft {
                        to: send.email.clone(),
                        subject: plan::fill(subject, send),
                        body: plan::fill(body, send),
                        files: vec![file],
                        shown: false,
                    };
                    steps.push((draft, Why::Plan));
                }
                continue;
            }
        };
        steps.push((step, Why::Plan));
    }
    Ok(steps)
}

/// Runs the plan: the person clicked Run.
pub async fn run_plan(id: &str) -> Result<Done, String> {
    let planned = {
        let mut hub = hub();
        match hub.planned.take() {
            Some(p) if p.plan.id == id => p,
            other => {
                hub.planned = other;
                return Err(t("mail.error.planNoLongerWaiting"));
            }
        }
    };
    let dir = hub().dir.clone().ok_or_else(|| t("mail.error.notReady"))?;
    let steps = plan_steps(&planned, &dir)?;
    audit("mail.plan.run", &planned.plan.summary);
    let done = run_all(steps, true).await;
    finish(&done).await;
    Ok(done)
}

// ----- The island -----

fn touch() {
    hub().card_at = Some(surface::now_ms());
}

fn mail_card(hub: &Hub) -> card::MailCard {
    let rows = filtered(&hub.pool, hub.filter, &hub.query, chrono::Local::now());
    card::MailCard {
        filter: hub.filter.word().into(),
        messages: rows
            .iter()
            .take(hub.shown.min(MAX_SHOWN))
            .map(|m| card::MailRow {
                id: m.id.to_string(),
                from: who(&m.sender),
                subject: m.subject.clone(),
                preview: None,
                unread: !m.read,
                flagged: m.flagged,
                at: m.at,
            })
            .collect(),
        query: hub.query.clone(),
        unread: hub.pool.iter().filter(|m| !m.read).count() as u32,
        sortable: hub.sortable as u32,
        more: rows.len() > hub.shown && hub.shown < MAX_SHOWN,
        open: hub.open.as_ref().map(|(message, opened)| card::MailOpen {
            id: message.id.to_string(),
            from: who(&opened.sender),
            subject: opened.subject.clone(),
            at: opened.at,
            body: opened.body.chars().take(4000).collect(),
            attachments: opened.attachments.clone(),
            flagged: message.flagged,
            unread: !message.read,
        }),
        pending: hub.pending.as_ref().map(|p| card::MailPending { id: p.view.id.clone(), title: p.view.title.clone(), lines: p.view.lines.clone() }),
        plan: hub.planned.as_ref().map(|p| {
            let view = plan_view(p);
            card::MailPlan { id: view.id, summary: view.summary, lines: view.lines, problems: view.problems }
        }),
        log: hub
            .log
            .recent(5)
            .into_iter()
            .map(|entry| card::MailLogRow { id: entry.id.clone(), label: entry.label(), undoable: entry.undoable() })
            .collect(),
        status: hub.status.clone(),
        closed: hub.closed,
    }
}

/// What Mail shows in the island now.
fn contribution(hub: &Hub) -> Contribution {
    let waiting = hub.pending.is_some() || hub.planned.is_some();
    let unread = hub.pool.iter().filter(|m| !m.read).count();
    let mut out = Contribution::default();
    if !hub.read && !waiting && hub.card_at.is_none() {
        return out;
    }
    if !hub.closed && (unread > 0 || waiting || hub.card_at.is_some()) {
        out.pills.push(Pill {
            id: "mail".into(),
            source: Source::Mail,
            label: t("surface.mail.mail"),
            tint: TINT.into(),
            state: if waiting { PillState::Waiting } else { PillState::Idle },
            badge: (unread > 0).then(|| unread.to_string()),
            progress: None,
            card: None,
        });
    }
    if waiting || hub.card_at.is_some() {
        out.offers.push(Offer {
            card: Card { id: "mail".into(), source: Source::Mail, body: CardBody::Mail(mail_card(hub)) },
            urgency: if waiting { Urgency::Waiting } else { Urgency::Info },
            at_ms: hub.card_at.unwrap_or_else(surface::now_ms),
            ttl_ms: (!waiting).then_some(CARD_MS),
        });
    }
    out
}

pub fn publish() {
    let contribution = contribution(&hub());
    surface::publish(Source::Mail, contribution);
}

fn text_value(action: &Action) -> Result<String, String> {
    action.value.as_ref().and_then(Value::as_str).map(str::to_string).ok_or_else(|| t("mail.error.needsEmail"))
}

/// What the person did on the mail card or pill.
async fn act(action: Action) -> Result<(), String> {
    touch();
    hub().status = None;
    match action.name.as_str() {
        "show" | "pill" => {
            publish();
            refresh(false).await?;
        }
        "hide" => {
            let mut hub = hub();
            hub.card_at = None;
            hub.open = None;
        }
        "filter" => {
            let filter = Filter::from_word(&text_value(&action)?).ok_or_else(|| t("mail.error.notFilter"))?;
            let mut hub = hub();
            hub.filter = filter;
            hub.shown = PAGE;
        }
        "search" => {
            let query: String = action.value.as_ref().and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
            let mut hub = hub();
            hub.query = query;
            hub.shown = PAGE;
        }
        "more" => {
            let mut hub = hub();
            hub.shown = (hub.shown + PAGE).min(MAX_SHOWN);
        }
        "open" => {
            open(&text_value(&action)?).await?;
        }
        "close" => hub().open = None,
        "markRead" => drop(propose_mark(&text_value(&action)?, Mark::Read, true)?),
        "markUnread" => drop(propose_mark(&text_value(&action)?, Mark::Read, false)?),
        "flag" => drop(propose_mark(&text_value(&action)?, Mark::Flagged, true)?),
        "unflag" => drop(propose_mark(&text_value(&action)?, Mark::Flagged, false)?),
        "junk" => drop(propose_junk(&text_value(&action)?)?),
        "reply" => {
            let value = action.value.clone().unwrap_or(Value::Null);
            let id = value["id"].as_str().ok_or_else(|| t("mail.error.needsEmail"))?;
            drop(propose_reply(id, value["body"].as_str().unwrap_or(""))?);
        }
        "attach" => {
            let files: Vec<String> = serde_json::from_value(action.value.clone().unwrap_or(Value::Null)).map_err(|_| t("mail.error.dropFiles"))?;
            drop(propose_attach(&files)?);
        }
        "sort" => {
            if propose_sort()?.is_none() {
                hub().status = Some(t("mail.status.nothingToSort"));
            }
        }
        "undo" => drop(propose_undo(&text_value(&action)?)?),
        "allow" => {
            approve(&text_value(&action)?).await?;
        }
        "deny" => deny(&text_value(&action)?),
        "run" => {
            run_plan(&text_value(&action)?).await?;
        }
        "discard" => drop_plan(&text_value(&action)?),
        "openMail" => {
            // The person asked for Mail; this is the only place the hub opens it.
            let _ = tokio::process::Command::new("/usr/bin/open").args(["-b", "com.apple.mail"]).status().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            refresh(false).await?;
        }
        _ => return Err(t("mail.error.unknownAction")),
    }
    publish();
    Ok(())
}

// ----- The app's one command -----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkChange {
    Read,
    Unread,
    Flag,
    Unflag,
}

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
    /// Reads every inbox and lists what the filter and search show.
    List {
        #[serde(default)]
        filter: Filter,
        #[serde(default)]
        query: String,
    },
    Open { id: String },
    Mark { id: String, change: MarkChange },
    Junk { id: String },
    Move { id: String, to: Mailbox },
    Reply { id: String, body: String },
    Attach { files: Vec<String> },
    Sort,
    Undo { entry: String },
    Approve { id: String },
    Deny { id: String },
    Log,
    Mailboxes,
    Plan {
        request: String,
        #[serde(default)]
        pdf: Option<String>,
    },
    Planned,
    RunPlan { id: String },
    DropPlan { id: String },
    PickPdf,
    /// Shows the mail card in the island.
    Show,
}

fn value<T: Serialize>(out: T) -> Result<Value, String> {
    serde_json::to_value(out).map_err(|_| "Mail answered with something unreadable.".into())
}

#[tauri::command]
pub async fn mail_call(op: Op, app: AppHandle) -> Result<Value, String> {
    match op {
        Op::Ping => Ok(Value::Bool(true)),
        Op::List { filter, query } => {
            {
                let mut hub = hub();
                hub.filter = filter;
                hub.query = query.chars().take(200).collect();
                hub.shown = MAX_SHOWN;
            }
            refresh(false).await?;
            value(listing())
        }
        Op::Open { id } => {
            let opened = open(&id).await?;
            value(json!({"id": opened.id.to_string(), "from": who(&opened.sender), "subject": opened.subject, "at": opened.at,
                "to": opened.to, "attachments": opened.attachments, "body": opened.body, "cut": opened.cut}))
        }
        Op::Mark { id, change } => value(match change {
            MarkChange::Read => propose_mark(&id, Mark::Read, true)?,
            MarkChange::Unread => propose_mark(&id, Mark::Read, false)?,
            MarkChange::Flag => propose_mark(&id, Mark::Flagged, true)?,
            MarkChange::Unflag => propose_mark(&id, Mark::Flagged, false)?,
        }),
        Op::Junk { id } => value(propose_junk(&id)?),
        Op::Move { id, to } => value(propose_move(&id, to)?),
        Op::Reply { id, body } => value(propose_reply(&id, &body)?),
        Op::Attach { files } => value(propose_attach(&files)?),
        Op::Sort => value(propose_sort()?),
        Op::Undo { entry } => value(propose_undo(&entry)?),
        Op::Approve { id } => value(approve(&id).await?),
        Op::Deny { id } => {
            deny(&id);
            Ok(Value::Null)
        }
        Op::Log => value(log_rows(100)),
        // Settings asks for these on its own when it opens: never an Automation prompt for that.
        Op::Mailboxes => {
            if !crate::apps::may_script_silently("Mail").await {
                return value(Vec::<Mailbox>::new());
            }
            value(mailboxes().await?)
        }
        Op::Plan { request, pdf } => {
            let pdf = pdf.filter(|p| !p.trim().is_empty()).map(PathBuf::from);
            if pdf.as_ref().is_some_and(|p| !p.is_absolute() || p.extension().is_none_or(|e| !e.eq_ignore_ascii_case("pdf"))) {
                return Err("Choose a PDF.".into());
            }
            value(make_plan(&app, &request, pdf).await?)
        }
        Op::Planned => value(planned()),
        Op::RunPlan { id } => value(run_plan(&id).await?),
        Op::DropPlan { id } => {
            drop_plan(&id);
            Ok(Value::Null)
        }
        Op::PickPdf => Ok(rfd::AsyncFileDialog::new()
            .set_title(t("mail.choosePdf"))
            .add_filter("PDF", &["pdf"])
            .pick_file()
            .await
            .map(|file| Value::String(file.path().to_string_lossy().into_owned()))
            .unwrap_or(Value::Null)),
        Op::Show => {
            touch();
            publish();
            refresh(false).await?;
            Ok(Value::Null)
        }
    }
}

#[cfg(test)]
mod tests;
