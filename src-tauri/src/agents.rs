//! Coding agents on this Mac: Claude Code and Codex.
//!
//! Each agent runs a hook on a few events (agent_setup.rs writes them). The hook is
//! macOS's own `nc`, which hands the event to Buntini over a socket only this user
//! can open, and prints back whatever Buntini answers. Only a permission request
//! waits for an answer, and only a click in Buntini allows anything. With no
//! answer (Buntini closed, the request left in the terminal, time up) the hook
//! prints nothing and the agent asks in its own terminal as it always does.
//!
//! Nothing here goes to a model or leaves the Mac.

mod codex;
mod dialect;
mod diff;
pub mod recap;
pub mod session;
mod surface;
pub mod terminal;
pub mod usage;

use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt, Interest};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

use crate::i18n::{t, tf};
use crate::surface::cards::agents::{AgentRule, DiffLine};

/// Longest event read from a hook. Events Buntini listens to are a few KB.
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
/// How long a hook may take to hand over its event.
const READ_BUDGET: Duration = Duration::from_secs(5);
/// How long a request waits for a click before the terminal takes it back. The
/// hook itself is installed with a little more (agent_setup.rs). Claude Code shows
/// its own prompt meanwhile; Codex waits for the hook first, so it gets less.
fn decision_budget(agent: Agent) -> Duration {
    match agent {
        Agent::Codex => Duration::from_secs(60),
        _ => Duration::from_secs(110),
    }
}
/// How often a waiting request checks that its hook is still running.
const HOOK_CHECK: Duration = Duration::from_millis(400);
/// A turn this long is worth saying it finished.
const LONG_TURN: Duration = Duration::from_secs(20);
/// A session quiet this long leaves the island.
const SESSION_IDLE: Duration = Duration::from_secs(15 * 60);
/// Longest command, path or plan shown on a request, whole. A longer one is cut, and then
/// only the terminal can allow it.
const MAX_DETAIL: usize = 16_000;
/// Most lines of an edit shown, and the longest line. Past them, only the terminal can allow it.
const MAX_CHANGE_LINES: usize = 400;
const MAX_CHANGE_CHARS: usize = 400;
/// How long a request shows before a click on it counts, as in the chat's card
/// (src/AgentRequests.tsx) and on the island.
pub const ARMING_MS: u64 = 700;
/// macOS keeps socket paths under 104 bytes, the terminating zero included.
const MAX_SOCKET_PATH: usize = 103;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Copilot,
    Cursor,
    Gemini,
    Antigravity,
    #[serde(rename = "opencode")]
    OpenCode,
    Amp,
    Hermes,
}

impl Agent {
    pub const ALL: [Agent; 9] = [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::Cursor, Agent::Gemini, Agent::Antigravity, Agent::OpenCode, Agent::Amp, Agent::Hermes];

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Copilot => "copilot",
            Agent::Cursor => "cursor",
            Agent::Gemini => "gemini",
            Agent::Antigravity => "antigravity",
            Agent::OpenCode => "opencode",
            Agent::Amp => "amp",
            Agent::Hermes => "hermes",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Copilot => "Copilot CLI",
            Agent::Cursor => "Cursor",
            Agent::Gemini => "Gemini CLI",
            Agent::Antigravity => "Antigravity",
            Agent::OpenCode => "OpenCode",
            Agent::Amp => "Amp",
            Agent::Hermes => "Hermes",
        }
    }

    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|agent| agent.id() == id)
    }

    /// The socket this agent's hooks talk to. One per agent, so the hook command
    /// needs nothing but the path.
    pub fn socket(self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.sock", self.id()))
    }
}

/// A request shown on the cat.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub id: String,
    pub agent: String,
    pub project: String,
    pub title: String,
    pub detail: String,
    /// `detail` is a command, a path or a patch.
    pub code: bool,
    /// Set when the agent is asking the user something rather than asking to act.
    pub questions: Vec<AgentQuestion>,
    /// "Always" can be offered: the agent suggested rules the card can show, or this is its plan.
    pub always: bool,
    /// The agent's plan, waiting to be started.
    pub plan: bool,
    /// "Allow" can be offered: everything it approves is on the card. Otherwise only Deny
    /// and In terminal are.
    pub allow: bool,
    /// What an edit changes, line by line.
    pub changes: Vec<DiffLine>,
    /// The rules "Always" keeps, and where.
    pub rules: Vec<AgentRule>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuestion {
    pub question: String,
    pub header: String,
    pub options: Vec<AgentOption>,
    pub multi_select: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFinished {
    pub agent: String,
    pub project: String,
    pub message: String,
}

/// What the user clicked.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    Allow,
    /// Allow, and keep allowing it: the rule the agent itself suggested, or for a plan,
    /// accepting edits from now on.
    Always,
    Deny,
    /// Leave it to the agent's own prompt.
    Terminal,
    /// Answers to an agent's questions, keyed by question text.
    Answers { answers: Map<String, Value> },
}

struct Waiting {
    view: AgentRequest,
    agent: Agent,
    session: String,
    tool: String,
    input: Value,
    reply: Option<oneshot::Sender<Answer>>,
    /// When it showed, on the island's clock.
    at_ms: u64,
}

#[derive(Default)]
struct Hub {
    app: Option<AppHandle>,
    next: u64,
    waiting: Vec<Waiting>,
    sessions: HashMap<(Agent, String), session::Session>,
    codex_app_server_threads: HashMap<String, usize>,
    /// Last week's recap while it shows.
    recap: Option<(crate::surface::Card, u64)>,
    /// Each agent's plan windows, as last heard.
    usage: HashMap<Agent, crate::surface::cards::agents::Usage>,
    /// A diff the person opened from the ticker: its card and when.
    diff: Option<(crate::surface::Card, u64)>,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(|| Mutex::new(Hub::default()));
static DIR: OnceLock<PathBuf> = OnceLock::new();
static RECAP: LazyLock<Mutex<recap::Recap>> = LazyLock::new(|| Mutex::new(socket_dir().map(|dir| recap::Recap::load(&recap::path(dir))).unwrap_or_default()));

/// Counts something for the week and saves it.
fn count(record: impl FnOnce(&mut recap::Recap)) {
    let mut recap = RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    record(&mut recap);
    if let Some(dir) = socket_dir() {
        recap.save(&recap::path(dir));
    }
}

fn hub() -> MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where the sockets live: Buntini's own data folder.
pub fn socket_dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// Starts listening for every agent. Called once at launch.
pub fn start(app: AppHandle, dir: PathBuf) {
    hub().app = Some(app);
    surface::listen();
    let dir = DIR.get_or_init(|| dir).clone();
    // Hooks written by an earlier Buntini listen to fewer events. Codex's plan windows wait
    // for its next turn (`finished`), as PRIVACY.md says.
    std::thread::spawn(crate::agent_setup::refresh);
    // On Monday morning, last week once.
    tauri::async_runtime::spawn(async {
        loop {
            surface::offer_recap();
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        }
    });
    for agent in Agent::ALL {
        let path = agent.socket(&dir);
        tauri::async_runtime::spawn(async move {
            match bind(&path) {
                Ok(listener) => accept(listener, agent).await,
                Err(error) => eprintln!("{} requests unavailable: {error}", agent.name()),
            }
        });
    }
}

/// Forgets what Buntini holds about the agents in memory: the week so far, the sessions on
/// the island and their plan windows. For Delete everything (`agent_setup::erase`), once
/// the files are gone, so nothing is saved back.
pub fn forget() {
    *RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = recap::Recap::default();
    let mut hub = hub();
    hub.recap = None;
    hub.diff = None;
    hub.sessions.clear();
    hub.usage.clear();
    surface::publish(&hub);
}

/// Removes the sockets, so hooks find nothing and stay out of the way.
pub fn stop() {
    let Some(dir) = socket_dir() else { return };
    for agent in Agent::ALL {
        remove_socket(&agent.socket(dir));
    }
}

fn heard_marker(agent: Agent) -> Option<PathBuf> {
    socket_dir().map(|dir| dir.join(format!("{}.heard", agent.id())))
}

/// The agent has sent Buntini an event at least once, so its hooks run. Codex only
/// runs hooks the user has trusted, so until then Settings says how.
pub fn heard(agent: Agent) -> bool {
    heard_marker(agent).is_some_and(|marker| marker.exists())
}

fn mark_heard(agent: Agent) {
    if let Some(marker) = heard_marker(agent).filter(|marker| !marker.exists()) {
        let _ = std::fs::write(marker, b"");
    }
}

fn remove_socket(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket()) {
        let _ = std::fs::remove_file(path);
    }
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(std::io::Error::other("the socket path is too long"));
    }
    match std::fs::symlink_metadata(path) {
        // Left by a Buntini that didn't quit cleanly.
        Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(path)?,
        Ok(_) => return Err(std::io::Error::other("something else is at the socket path")),
        Err(_) => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

async fn accept(listener: UnixListener, agent: Agent) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tauri::async_runtime::spawn(serve(stream, agent));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

/// One hook run: read its event, act on it, and answer a permission request.
async fn serve(mut stream: UnixStream, agent: Agent) {
    // Only this user's processes; the socket's permissions say so too.
    let ours = stream
        .peer_cred()
        .is_ok_and(|cred| cred.uid() == unsafe { libc::getuid() });
    if !ours {
        return;
    }
    let Ok(Some((host, hint, payload))) = tokio::time::timeout(READ_BUDGET, read_payload(&mut stream)).await
    else {
        return;
    };
    mark_heard(agent);
    // Antigravity waits for a word on every tool (`decision` is required). "allow" is its
    // neutral one: "allows the tool call to proceed, delegating to the next hook or default
    // policy" (antigravity.google/docs/hooks), so its own rules and prompts still decide.
    // Approving outright would be `auto_approve`, which Buntini never sends.
    if agent == Agent::Antigravity && hint == "PreToolUse" {
        let _ = stream.write_all(b"{\"decision\":\"allow\"}\n").await;
        let _ = stream.shutdown().await;
    }
    // Claude Code's status line, wrapped by Buntini: its plan windows.
    if hint == "statusline" {
        if let Some(usage) = usage::from_status_line(&payload) {
            let mut hub = hub();
            if hub.usage.get(&Agent::Claude) != Some(&usage) {
                hub.usage.insert(Agent::Claude, usage);
                surface::publish(&hub);
            }
        }
        return;
    }
    let socket_agent = agent;
    let Some((agent, payload)) = dialect::normalize(agent, &hint, &payload) else { return };
    let Some(mut event) = HookEvent::from_payload(&payload) else { return };
    // Buntini's own brain runs (`cli.rs`) aren't the person's sessions.
    if crate::cli::is_brain_folder(&event.cwd) {
        return;
    }
    event.host = host;
    match event.name.as_str() {
        "SessionStart" => track(agent, &event, |_, _| {}),
        "UserPromptSubmit" => {
            started(agent, &event);
            track(agent, &event, |session, now| session.prompt(&event.prompt, now));
        }
        "PreToolUse" => {
            if matches!(event.tool.as_str(), "Bash" | "shell" | "local_shell" | "exec_command" | "unified_exec" | "run_command" | "run_shell_command") {
                count(|recap| recap.command(recap::today()));
            }
            track(agent, &event, |session, now| session.before(&event.tool, &event.input, now));
        }
        "PostToolUse" | "PostToolUseFailure" => {
            settled(agent, &event);
            let failed = event.name == "PostToolUseFailure";
            let mut edit = None;
            track(agent, &event, |session, now| edit = session.after(&event.tool, &event.input, failed, now));
            if let Some((file, added, removed)) = edit {
                count(|recap| recap.edit(recap::today(), &file, added as u64, removed as u64));
            }
        }
        "Stop" | "StopFailure" => finished(agent, &event),
        // The agent is asking in its own terminal.
        "Notification" => {
            let kind = payload.get("notification_type").or_else(|| payload.get("notificationType")).and_then(Value::as_str).unwrap_or_default().to_string();
            if kind == "permission_prompt" && agent != Agent::Claude && agent != Agent::Codex {
                track(agent, &event, |session, _| session.asking = true);
            }
        }
        "SessionEnd" => {
            let mut hub = hub();
            hub.sessions.remove(&(agent, event.session.clone()));
            surface::publish(&hub);
        }
        "PermissionRequest" => {
            // App Server asks after the hooks and includes the execution directory and
            // permissions. Leave shell decisions to it for threads on its connection.
            if agent == Agent::Codex && is_shell(&event.tool) && hub().codex_app_server_threads.contains_key(&event.session) {
                return;
            }
            if let Some(reply) = ask(agent, socket_agent, &event, &stream).await {
                let mut line = reply.into_bytes();
                line.push(b'\n');
                let _ = stream.write_all(&line).await;
                let _ = stream.shutdown().await;
            }
        }
        _ => {}
    }
}

/// The event as JSON, after the line of terminal details the note hook puts first (see
/// `agent_setup::note_command`). `nc` closes its sending side once the agent's input
/// ends, but the value is taken as soon as it's whole either way.
async fn read_payload(stream: &mut UnixStream) -> Option<(session::Host, String, Value)> {
    let mut buffer = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        buffer.extend_from_slice(&chunk[..read]);
        let (host, hint, rest) = split_header(&buffer)?;
        let body = rest.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(rest);
        match serde_json::Deserializer::from_slice(body).into_iter::<Value>().next() {
            Some(Ok(value)) => return Some((host, hint, value)),
            Some(Err(error)) if !error.is_eof() => return None,
            _ => {}
        }
        if read == 0 || buffer.len() > MAX_PAYLOAD {
            return None;
        }
    }
}

/// `BUNTINI<TAB>term<TAB>bundle<TAB>iTerm session<TAB>entry point<TAB>event` before the JSON,
/// when the hook sent it; the event only for agents that leave it out of the payload.
fn split_header(buffer: &[u8]) -> Option<(session::Host, String, &[u8])> {
    const MARK: &[u8] = b"BUNTINI\t";
    if !buffer.starts_with(MARK) {
        return Some((session::Host::default(), String::new(), buffer));
    }
    let end = buffer.iter().position(|byte| *byte == b'\n');
    let Some(end) = end else {
        let rest: &[u8] = if buffer.len() > 4096 { &buffer[buffer.len()..] } else { &[] };
        return Some((session::Host::default(), String::new(), rest));
    };
    let line = String::from_utf8_lossy(&buffer[MARK.len()..end]);
    let mut parts = line.split('\t').map(|part| part.chars().filter(|c| !c.is_control()).take(200).collect::<String>());
    let host = session::Host {
        term_program: parts.next().unwrap_or_default(),
        bundle: parts.next().unwrap_or_default(),
        iterm_session: parts.next().unwrap_or_default(),
        entrypoint: parts.next().unwrap_or_default(),
    };
    let hint = parts.next().unwrap_or_default();
    Some((host, hint, &buffer[end + 1..]))
}

/// The parts of a hook event Buntini uses. Claude Code and Codex share these names.
#[derive(Debug, Clone)]
struct HookEvent {
    name: String,
    session: String,
    cwd: String,
    tool: String,
    input: Value,
    last_message: String,
    prompt: String,
    /// Rules the agent suggests for "don't ask again" (Claude Code's `permission_suggestions`).
    suggestions: Vec<Value>,
    host: session::Host,
}

impl HookEvent {
    fn from_payload(payload: &Value) -> Option<HookEvent> {
        let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let name = text("hook_event_name");
        if name.is_empty() {
            return None;
        }
        Some(HookEvent {
            name,
            session: text("session_id"),
            cwd: text("cwd"),
            tool: text("tool_name"),
            input: payload.get("tool_input").cloned().unwrap_or(Value::Null),
            last_message: text("last_assistant_message"),
            prompt: text("prompt"),
            suggestions: payload.get("permission_suggestions").and_then(Value::as_array).cloned().unwrap_or_default(),
            host: session::Host::default(),
        })
    }
}

fn project(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn started(agent: Agent, event: &HookEvent) {
    let mut hub = hub();
    // A new prompt means anything still showing for this session was settled in the terminal.
    let before = hub.waiting.len();
    hub.waiting.retain(|waiting| !(waiting.agent == agent && waiting.session == event.session));
    if hub.waiting.len() != before {
        emit_requests(&hub);
    }
}

/// Updates the session the event belongs to, creating it on first sight.
fn track(agent: Agent, event: &HookEvent, update: impl FnOnce(&mut session::Session, u64)) {
    if event.session.is_empty() {
        return;
    }
    let now = crate::surface::now_ms();
    let mut hub = hub();
    let idle = SESSION_IDLE.as_millis() as u64;
    hub.sessions.retain(|_, session| now.saturating_sub(session.last_ms) < idle);
    if hub.sessions.len() >= 64 && !hub.sessions.contains_key(&(agent, event.session.clone())) {
        return;
    }
    let session = hub.sessions.entry((agent, event.session.clone())).or_insert_with(|| session::Session::new(agent.id(), &event.session, &event.cwd, now));
    if !event.cwd.is_empty() {
        session.cwd = event.cwd.clone();
    }
    if event.host != session::Host::default() {
        session.host = event.host.clone();
    }
    update(session, now);
    surface::publish(&hub);
    crate::island::agent_activity();
}

/// Sessions a dropped file's path can be pasted into, newest first: (pill, label). Only
/// those running in a terminal app.
pub fn drop_targets() -> Vec<(String, String)> {
    let hub = hub();
    let mut sessions: Vec<&session::Session> = hub.sessions.values().filter(|session| terminal::terminal_for(&session.host).is_some()).collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_ms));
    sessions
        .into_iter()
        .map(|session| {
            let name = Agent::from_id(&session.agent).map_or(session.agent.as_str(), |agent| agent.name());
            let project = session.project();
            (format!("agent:{}:{}", session.agent, session.id), if project.is_empty() { name.to_string() } else { format!("{name} · {project}") })
        })
        .collect()
}

/// The tool ran, so its request was answered, here or in the terminal. Agents
/// don't end a waiting hook when the terminal answers, so this is how its card goes.
fn settled(agent: Agent, event: &HookEvent) {
    let mut hub = hub();
    let before = hub.waiting.len();
    hub.waiting.retain(|waiting| {
        !(waiting.agent == agent && waiting.session == event.session && waiting.tool == event.tool && same_input(&waiting.input, &event.input))
    });
    if hub.waiting.len() != before {
        emit_requests(&hub);
    }
}

/// The input a request was shown with is still in what ran (an agent may add to it,
/// as Claude Code adds the answers to a question).
fn same_input(asked: &Value, ran: &Value) -> bool {
    match (asked, ran) {
        (Value::Object(asked), Value::Object(ran)) => asked.iter().all(|(key, value)| ran.get(key) == Some(value)),
        (asked, ran) => asked == ran,
    }
}

fn finished(agent: Agent, event: &HookEvent) {
    // The turn is over: nothing of it is still waiting.
    {
        let mut hub = hub();
        let before = hub.waiting.len();
        hub.waiting.retain(|waiting| !(waiting.agent == agent && waiting.session == event.session));
        if hub.waiting.len() != before {
            emit_requests(&hub);
        }
    }
    let failed = event.name == "StopFailure";
    let mut turn = None;
    let mut folder = String::new();
    track(agent, event, |session, now| {
        turn = session.turn_ms(now);
        folder = session.project();
        session.stop(&event.last_message, failed, now);
        session.announce = failed || turn.is_some_and(|ms| ms >= LONG_TURN.as_millis() as u64);
    });
    if let Some(ms) = turn {
        count(|recap| recap.turn(recap::today(), agent.id(), &event.session, &folder, ms / 1000));
    }
    if agent == Agent::Codex {
        refresh_codex_usage();
    }
    let hub = hub();
    if turn.is_none_or(|ms| ms < LONG_TURN.as_millis() as u64) {
        return;
    }
    let note = AgentFinished {
        agent: agent.name().into(),
        project: project(&event.cwd),
        message: first_line(&event.last_message, 160),
    };
    if let Some(app) = &hub.app {
        let _ = app.emit("agent-finished", note);
    }
}

/// A card belongs to a single live request. Dropping its owner withdraws it.
struct Pending(String);

impl Pending {
    fn show(view: AgentRequest, agent: Agent, session: String, tool: String, input: Value) -> (Self, oneshot::Receiver<Answer>) {
        let pending = Self(view.id.clone());
        let (sender, receiver) = oneshot::channel();
        let mut hub = hub();
        hub.waiting.push(Waiting { view, agent, session, tool, input, reply: Some(sender), at_ms: crate::surface::now_ms() });
        emit_requests(&hub);
        (pending, receiver)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut hub = hub();
        hub.waiting.retain(|waiting| waiting.view.id != self.0);
        emit_requests(&hub);
    }
}

/// Shows the request and waits for the click, the hook going away, or the time limit.
/// Returns what the hook prints back, if anything.
async fn ask(agent: Agent, socket_agent: Agent, event: &HookEvent, stream: &UnixStream) -> Option<String> {
    let view = describe(agent, event)?;
    let (pending, receiver) = Pending::show(view, agent, event.session.clone(), event.tool.clone(), event.input.clone());
    let answer = tokio::select! {
        answer = receiver => answer.ok(),
        _ = hook_gone(stream) => None,
        _ = tokio::time::sleep(decision_budget(agent)) => None,
    };
    drop(pending);
    let answer = answer?;
    let line = decision(&answer, event).map(|decision| dialect::reply(socket_agent, decision).to_string());
    if line.is_some() && event.tool != "AskUserQuestion" {
        count(|recap| recap.decision(recap::today(), !matches!(answer, Answer::Deny)));
    }
    line
}

/// Resolves once the hook has gone: answered in the terminal, the agent stopped, or
/// the hook's own time ran out. `nc` has already closed its sending side, so the
/// sign is the receiving side closing too.
async fn hook_gone(stream: &UnixStream) {
    loop {
        match stream.ready(Interest::WRITABLE).await {
            Ok(ready) if ready.is_write_closed() => return,
            Ok(_) => tokio::time::sleep(HOOK_CHECK).await,
            Err(_) => return,
        }
    }
}

fn emit_requests(hub: &Hub) {
    if let Some(app) = &hub.app {
        let _ = app.emit("agent-requests", views(hub));
    }
    surface::publish(hub);
}

fn views(hub: &Hub) -> Vec<AgentRequest> {
    hub.waiting.iter().map(|waiting| waiting.view.clone()).collect()
}

fn next_id() -> String {
    let mut hub = hub();
    hub.next += 1;
    format!("agent-{}-{}", std::process::id(), hub.next)
}

fn is_shell(tool: &str) -> bool {
    matches!(tool, "Bash" | "shell" | "local_shell" | "exec_command" | "unified_exec")
}

/// What the request is, in words. None for something Buntini can't show at all, which then
/// stays with the terminal. Allow is offered only when everything it approves is on the
/// card: the whole command, every line an edit changes, no setting that changes how it runs.
fn describe(agent: Agent, event: &HookEvent) -> Option<AgentRequest> {
    let input = &event.input;
    let string = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let path = |key: &str| relative(&string(key), &event.cwd);
    let mut questions = Vec::new();
    // Nothing Allow would approve is missing from the card.
    let mut shown = true;
    let mut changes = Vec::new();
    let (title, detail, code) = match event.tool.as_str() {
        "AskUserQuestion" => {
            questions = parse_questions(input)?;
            (String::new(), String::new(), false)
        }
        tool if is_shell(tool) => {
            let command = match input.get("command").or_else(|| input.get("cmd")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                Some(Value::String(command)) => command.trim().to_string(),
                _ => return None,
            };
            // Claude Code's own settings for the run; outside the sandbox is one the card doesn't show.
            shown = only(input, &["command", "cmd", "description", "timeout", "run_in_background", "dangerouslyDisableSandbox"])
                && input.get("dangerouslyDisableSandbox").is_none_or(|value| *value == Value::Bool(false))
                // Codex's hook isn't told whether the command would run outside its sandbox or
                // with more permissions, which its own prompt shows.
                && agent != Agent::Codex;
            let said = string("description");
            (if said.is_empty() { t("agents.request.runCommand") } else { said }, command, true)
        }
        "apply_patch" => {
            let patch = match input.get("command").or_else(|| input.get("patch")).or_else(|| input.get("input")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
                Some(Value::String(patch)) => patch.clone(),
                _ => String::new(),
            };
            match patch_lines(&patch) {
                Some(lines) if !patch.trim().is_empty() && only(input, &["command", "patch", "input"]) => changes = lines,
                _ => shown = false,
            }
            (t("agents.request.changeFiles"), patch_files(&patch, &event.cwd).join("\n"), true)
        }
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            match edit_changes(&event.tool, input, &event.cwd) {
                Some(lines) => changes = lines,
                None => shown = false,
            }
            let notebook = event.tool == "NotebookEdit";
            let file = file_name(&string(if notebook { "notebook_path" } else { "file_path" }));
            let title = match (event.tool.as_str(), file) {
                ("Write", Some(file)) => tf("agents.request.write", &[("file", &file)]),
                ("Write", None) => t("agents.request.writeAFile"),
                (_, Some(file)) => tf("agents.request.edit", &[("file", &file)]),
                (_, None) => t("agents.request.editAFile"),
            };
            let mut detail = path(if notebook { "notebook_path" } else { "file_path" });
            // Which cell, and whether it is replaced, added or removed.
            if notebook {
                let cell = [string("edit_mode"), string("cell_id"), string("cell_type")].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
                if !cell.is_empty() {
                    detail = format!("{detail}\n{cell}");
                }
            }
            (title, detail, true)
        }
        "Read" => (file_name(&string("file_path")).map_or_else(|| t("agents.request.readAFile"), |file| tf("agents.request.read", &[("file", &file)])), path("file_path"), true),
        "Glob" | "Grep" => {
            let place = string("path");
            (t("agents.request.searchFiles"), [string("pattern"), relative(&place, &event.cwd)].iter().filter(|part| !part.is_empty()).cloned().collect::<Vec<_>>().join("  ·  "), true)
        }
        "WebFetch" => (t("agents.request.openWebPage"), string("url"), true),
        "WebSearch" => (t("agents.request.searchWeb"), string("query"), false),
        "ExitPlanMode" => (t("agents.request.startPlan"), string("plan"), false),
        tool if tool.starts_with("mcp__") => {
            let mut parts = tool.splitn(3, "__").skip(1);
            let server = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            (tf("agents.request.useFrom", &[("tool", &words(name)), ("server", &server)]), compact(input), true)
        }
        "" => return None,
        tool => (tf("agents.request.use", &[("tool", &words(tool))]), compact(input), true),
    };
    let plan = event.tool == "ExitPlanMode";
    let allow = questions.is_empty() && shown && detail.chars().count() <= MAX_DETAIL;
    // Always keeps exactly the rules the agent suggested; the card shows each and where it's kept.
    let rules = if agent == Agent::Claude && allow && !plan { kept_rules(&event.suggestions).unwrap_or_default() } else { Vec::new() };
    let always = agent == Agent::Claude && allow && (plan || !rules.is_empty());
    Some(AgentRequest {
        id: next_id(),
        agent: agent.name().into(),
        project: visible(&project(&event.cwd)),
        title: visible(&clip(&title, 140)),
        detail: visible(&clip(&detail, MAX_DETAIL)),
        code,
        questions,
        always,
        plan,
        allow,
        changes,
        rules,
        created_at: crate::types::now(),
    })
}

/// The input has no fields but these.
fn only(input: &Value, keys: &[&str]) -> bool {
    input.as_object().is_some_and(|fields| fields.keys().all(|key| keys.contains(&key.as_str())))
}

/// Every line an edit changes, from its input alone. None when it can't all be shown: a
/// field the card doesn't show (Edit's "every match", say), or too many or too long lines.
fn edit_changes(tool: &str, input: &Value, cwd: &str) -> Option<Vec<DiffLine>> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let one_match = |value: &Value| value.get("replace_all").is_none_or(|all| *all == Value::Bool(false));
    let changes = match tool {
        "Edit" => {
            if !only(input, &["file_path", "old_string", "new_string", "replace_all"]) || !one_match(input) {
                return None;
            }
            diff::diff(&text(input, "old_string")?, &text(input, "new_string")?)
        }
        "MultiEdit" => {
            let edits = input.get("edits")?.as_array()?;
            let each = |edit: &Value| only(edit, &["old_string", "new_string", "replace_all"]) && one_match(edit) && text(edit, "old_string").is_some() && text(edit, "new_string").is_some();
            if !only(input, &["file_path", "edits"]) || edits.is_empty() || !edits.iter().all(each) {
                return None;
            }
            session::edit_diff("MultiEdit", input, cwd)?.1
        }
        "Write" => {
            if !only(input, &["file_path", "content"]) {
                return None;
            }
            diff::diff("", &text(input, "content")?)
        }
        "NotebookEdit" => {
            if !only(input, &["notebook_path", "new_source", "cell_id", "cell_type", "edit_mode"]) {
                return None;
            }
            match text(input, "edit_mode").as_deref().unwrap_or("replace") {
                "delete" => diff::Diff::default(),
                "replace" | "insert" => diff::diff("", &text(input, "new_source")?),
                _ => return None,
            }
        }
        _ => return None,
    };
    // Counted only, too long to compare line by line.
    if changes.lines.is_empty() && changes.added + changes.removed > 0 {
        return None;
    }
    change_lines(changes.lines.into_iter().map(|line| (line.kind, line.text)).collect())
}

/// A patch's every line, its headers and context included, as the island shows them.
fn patch_lines(patch: &str) -> Option<Vec<DiffLine>> {
    change_lines(
        patch
            .lines()
            .filter(|line| !matches!(line.trim(), "*** Begin Patch" | "*** End Patch"))
            .map(|line| match (line.strip_prefix('+'), line.strip_prefix('-')) {
                (Some(added), _) => (diff::Kind::Add, added.to_string()),
                (_, Some(removed)) => (diff::Kind::Remove, removed.to_string()),
                _ => (diff::Kind::Same, line.to_string()),
            })
            .collect(),
    )
}

fn change_lines(lines: Vec<(diff::Kind, String)>) -> Option<Vec<DiffLine>> {
    if lines.len() > MAX_CHANGE_LINES || lines.iter().any(|(_, text)| text.chars().count() > MAX_CHANGE_CHARS) {
        return None;
    }
    Some(
        lines
            .into_iter()
            .map(|(kind, text)| DiffLine {
                kind: match kind {
                    diff::Kind::Add => "add",
                    diff::Kind::Remove => "remove",
                    diff::Kind::Same => "same",
                    diff::Kind::Gap => "gap",
                }
                .into(),
                text: visible(&text),
            })
            .collect(),
    )
}

/// The rules Claude Code suggested for "don't ask again", each in its own words and where it
/// would be kept. None unless every one can be shown and is kept no wider than this project:
/// nothing in the person's own settings for every project, nothing that widens the mode.
fn kept_rules(suggestions: &[Value]) -> Option<Vec<AgentRule>> {
    let mut rules = Vec::new();
    for suggestion in suggestions {
        if !only(suggestion, &["type", "rules", "behavior", "destination", "mode", "directories"]) {
            return None;
        }
        let field = |key: &str| suggestion.get(key).and_then(Value::as_str);
        let place = match field("destination")? {
            "session" => t("agents.rule.session"),
            "localSettings" => t("agents.rule.project"),
            "projectSettings" => t("agents.rule.shared"),
            // `userSettings` reaches every project; anything else is unknown.
            _ => return None,
        };
        match field("type")? {
            "addRules" if field("behavior") == Some("allow") => {
                let list = suggestion.get("rules")?.as_array()?;
                if list.is_empty() {
                    return None;
                }
                for rule in list {
                    let tool = rule.get("toolName")?.as_str().filter(|tool| !tool.is_empty())?;
                    if !only(rule, &["toolName", "ruleContent"]) {
                        return None;
                    }
                    let text = match rule.get("ruleContent") {
                        None => tool.to_string(),
                        Some(Value::String(content)) => format!("{tool}({content})"),
                        Some(_) => return None,
                    };
                    rules.push(AgentRule { rule: visible(&text), place: place.clone() });
                }
            }
            // Edits asked one by one from now on, or accepted: never past them.
            "setMode" if field("mode") == Some("acceptEdits") => rules.push(AgentRule { rule: t("agentRequests.acceptEdits"), place }),
            "addDirectories" => {
                let folders = suggestion.get("directories")?.as_array()?;
                if folders.is_empty() {
                    return None;
                }
                for folder in folders {
                    rules.push(AgentRule { rule: tf("agents.rule.folder", &[("folder", &visible(folder.as_str()?))]), place: place.clone() });
                }
            }
            _ => return None,
        }
    }
    (!rules.is_empty()).then_some(rules)
}

/// `text` with the characters that hide or reorder what's around them written out, so a
/// command reads on the card as it runs: control characters other than new lines and tabs,
/// the direction marks and overrides, and zero-width characters.
fn visible(text: &str) -> String {
    let hidden = |c: char| {
        (c.is_control() && c != '\n' && c != '\t')
            || matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
    };
    if !text.chars().any(hidden) {
        return text.to_string();
    }
    text.chars().map(|c| if hidden(c) { format!("<U+{:04X}>", c as u32) } else { c.to_string() }).collect()
}

/// Claude Code's AskUserQuestion: one to four questions, each with its options.
fn parse_questions(input: &Value) -> Option<Vec<AgentQuestion>> {
    let items = input.get("questions")?.as_array()?;
    if items.is_empty() {
        return None;
    }
    items
        .iter()
        .map(|item| {
            let question = item.get("question")?.as_str()?.trim().to_string();
            let options = item
                .get("options")?
                .as_array()?
                .iter()
                .map(|option| {
                    Some(AgentOption {
                        label: option.get("label")?.as_str()?.to_string(),
                        description: option.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            (!question.is_empty() && !options.is_empty()).then(|| AgentQuestion {
                question,
                header: item.get("header").and_then(Value::as_str).unwrap_or_default().to_string(),
                options,
                multi_select: item.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// The files a patch touches, from its `*** Update File:` style headers.
fn patch_files(patch: &str, cwd: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
                .map(|path| relative(path.trim(), cwd))
        })
        .collect()
}

fn relative(path: &str, cwd: &str) -> String {
    if cwd.is_empty() {
        return path.to_string();
    }
    match Path::new(path).strip_prefix(cwd) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned())
}

fn words(name: &str) -> String {
    name.replace(['_', '-'], " ")
}

fn compact(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        other => other.to_string(),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    clip(line, max)
}

/// What Claude Code's hook prints for the user's answer. Nothing allows anything unless
/// the user clicked Allow, or answered the agent's questions with its own options.
#[cfg(test)]
fn reply(answer: &Answer, event: &HookEvent) -> Option<String> {
    decision(answer, event).map(|decision| dialect::reply(Agent::Claude, decision).to_string())
}

/// The decision for the user's answer, before it's put in the agent's own words.
fn decision(answer: &Answer, event: &HookEvent) -> Option<Value> {
    Some(match answer {
        Answer::Terminal => return None,
        // Leaving plan mode: the plan goes back as it was, and the mode the terminal's
        // own choices set: edits asked one by one, or accepted from now on.
        Answer::Allow | Answer::Always if event.tool == "ExitPlanMode" => {
            let mode = if matches!(answer, Answer::Always) { "acceptEdits" } else { "default" };
            json!({
                "behavior": "allow",
                "updatedInput": event.input,
                "updatedPermissions": [{ "type": "setMode", "mode": mode, "destination": "session" }],
            })
        }
        // Only the rules the agent itself suggested for this request, unchanged, and only
        // when the card could show every one of them.
        Answer::Always if event.tool != "AskUserQuestion" && kept_rules(&event.suggestions).is_some() => {
            json!({ "behavior": "allow", "updatedPermissions": event.suggestions })
        }
        Answer::Allow if event.tool != "AskUserQuestion" => json!({ "behavior": "allow" }),
        Answer::Deny => json!({ "behavior": "deny", "message": "The user said no in Buntini." }),
        Answer::Answers { answers } if event.tool == "AskUserQuestion" && answers_fit(&event.input, answers) => {
            let mut input = event.input.as_object()?.clone();
            input.insert("answers".into(), Value::Object(answers.clone()));
            json!({ "behavior": "allow", "updatedInput": input })
        }
        _ => return None,
    })
}

/// True when `answers` answers exactly the questions asked, each with its own
/// options: one label for a single choice, a non-empty list of distinct labels
/// for a multiple choice.
fn answers_fit(input: &Value, answers: &Map<String, Value>) -> bool {
    let Some(questions) = parse_questions(input) else { return false };
    if questions.len() != answers.len() {
        return false;
    }
    questions.iter().all(|question| {
        let labels: Vec<&str> = question.options.iter().map(|option| option.label.as_str()).collect();
        match answers.get(&question.question) {
            Some(Value::String(pick)) if !question.multi_select => labels.contains(&pick.as_str()),
            Some(Value::Array(picks)) if question.multi_select => {
                let picks: Vec<&str> = picks.iter().filter_map(Value::as_str).collect();
                let mut distinct = picks.clone();
                distinct.sort_unstable();
                distinct.dedup();
                !picks.is_empty() && distinct.len() == picks.len() && picks.iter().all(|pick| labels.contains(pick))
            }
            _ => false,
        }
    })
}

/// Asks Codex for its plan windows in the background, at most once a minute.
fn refresh_codex_usage() {
    std::thread::spawn(|| {
        if let Some(usage) = usage::read_codex() {
            let mut hub = hub();
            hub.usage.insert(Agent::Codex, usage);
            surface::publish(&hub);
        }
    });
}

#[cfg(target_os = "macos")]
extern "C" {
    fn buntini_agents_recap_png(bytes: *const u8, length: usize, path: *const std::os::raw::c_char) -> bool;
}

/// Saves the latest week's picture to Downloads and shows it in Finder.
fn share_recap() -> Result<PathBuf, String> {
    let week = {
        let recap = RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let shown = hub().recap.as_ref().map(|(card, _)| card.id.trim_start_matches("recap:").to_string());
        recap.weeks.iter().rev().find(|week| shown.as_ref().is_none_or(|start| &week.start == start) && week.turns > 0).cloned()
    };
    let week = week.ok_or("There's no week to share yet.")?;
    // The picture leaves project names out.
    let body = crate::surface::cards::agents::Recap {
        week: recap::title(&week.start),
        stats: recap::stats(&week, |id| Agent::from_id(id).map_or_else(|| id.to_string(), |agent| agent.name().to_string()), false)
            .into_iter()
            .map(|(label, value)| crate::surface::cards::agents::RecapStat { label, value })
            .collect(),
    };
    let json = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let downloads = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Downloads")).ok_or("No home folder.")?;
    let mut path = downloads.join(format!("Buntini week {}.png", week.start));
    let mut n = 2;
    while path.exists() {
        path = downloads.join(format!("Buntini week {} ({n}).png", week.start));
        n += 1;
    }
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    // SAFETY: both buffers live for the call; Swift copies what it keeps.
    let made = unsafe { buntini_agents_recap_png(json.as_ptr(), json.len(), c_path.as_ptr()) };
    #[cfg(not(target_os = "macos"))]
    let made = false;
    if !made {
        return Err("The picture couldn't be made.".into());
    }
    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&path).status();
    Ok(path)
}

#[tauri::command]
pub fn agent_requests() -> Vec<AgentRequest> {
    views(&hub())
}

#[tauri::command]
pub fn answer_agent(id: String, answer: Answer) -> Result<(), String> {
    let mut hub = hub();
    let gone = || "That request was already answered.".to_string();
    let waiting = hub.waiting.iter_mut().find(|waiting| waiting.view.id == id).ok_or_else(gone)?;
    // A click in its first moment was aimed at whatever was there before.
    if crate::surface::now_ms().saturating_sub(waiting.at_ms) < ARMING_MS {
        return Err("That request only just showed.".into());
    }
    let asking = !waiting.view.questions.is_empty();
    let (allow, always) = (waiting.view.allow, waiting.view.always);
    let fits = match &answer {
        Answer::Allow => !asking && allow,
        Answer::Always => !asking && allow && always,
        Answer::Answers { .. } => asking,
        Answer::Deny | Answer::Terminal => true,
    };
    if !fits {
        return Err("That answer doesn't fit this request.".into());
    }
    let sender = waiting.reply.take().ok_or_else(gone)?;
    sender.send(answer).map_err(|_| gone())
}


/// Saved under `settings.agents`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What the UI asks of the agents beyond requests and hooks, through one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
    /// Folders agents worked in lately, newest first.
    Projects,
    /// Starts a session of `agent` in `project`, with `prompt` as its first message.
    Start { agent: String, project: String, prompt: String },
    /// Brings forward the terminal of the session behind an island pill.
    Open { pill: String },
    /// Pastes dropped files into a session's terminal.
    Drop { pill: String, paths: Vec<String> },
    /// The current week so far, and the weeks before it.
    Recap,
    /// Saves last week's picture to Downloads.
    ShareRecap,
    /// Claude Code's plan windows on the island, through its status line, or not.
    PlanUsage { on: bool },
}

#[tauri::command]
pub async fn agents_call(app: AppHandle, op: Op) -> Result<serde_json::Value, String> {
    use tauri::Manager;
    match op {
        Op::Ping => Ok(Value::Bool(true)),
        Op::Projects => Ok(json!(terminal::recent_projects())),
        Op::Start { agent, project, prompt } => {
            let agent = Agent::from_id(&agent).ok_or("Unknown coding agent.")?;
            let scripts = app.path().app_cache_dir().map_err(|error| error.to_string())?.join("agent-scripts");
            terminal::start(agent, Path::new(&project), &prompt, &scripts).map(|_| Value::Null)
        }
        Op::Open { pill } => terminal::open_for_pill(&pill).map(|_| Value::Null),
        Op::Drop { pill, paths } => terminal::drop_files(&pill, &paths).map(|_| Value::Null),
        Op::Recap => {
            let recap = RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let name = |id: &str| Agent::from_id(id).map_or_else(|| id.to_string(), |agent| agent.name().to_string());
            Ok(json!(recap.weeks.iter().rev().map(|week| json!({
                "week": recap::title(&week.start),
                "stats": recap::stats(week, name, true).into_iter().map(|(label, value)| json!({ "label": label, "value": value })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()))
        }
        Op::ShareRecap => share_recap().map(|path| json!(path.to_string_lossy())),
        Op::PlanUsage { on } => {
            crate::agent_setup::set_plan_usage(on)?;
            if !on {
                let mut hub = hub();
                hub.usage.remove(&Agent::Claude);
                surface::publish(&hub);
            }
            Ok(Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn event(tool: &str, input: Value) -> HookEvent {
        HookEvent {
            name: "PermissionRequest".into(),
            session: "s1".into(),
            cwd: "/Users/me/projects/shop".into(),
            tool: tool.into(),
            input,
            last_message: String::new(),
            prompt: String::new(),
            suggestions: Vec::new(),
            host: session::Host::default(),
        }
    }

    fn question_input() -> Value {
        json!({ "questions": [
            { "question": "Which database?", "header": "Database", "multiSelect": false,
              "options": [{ "label": "Postgres", "description": "Server" }, { "label": "SQLite", "description": "File" }] },
            { "question": "Which extras?", "header": "Extras", "multiSelect": true,
              "options": [{ "label": "Auth", "description": "" }, { "label": "Search", "description": "" }] }
        ] })
    }

    /// Does what the hook prints let anything through?
    fn allows(line: &str) -> bool {
        let value: Value = serde_json::from_str(line).unwrap();
        value["hookSpecificOutput"]["decision"]["behavior"] == "allow"
    }

    #[test]
    fn nothing_is_allowed_without_a_matching_click() {
        let command = event("Bash", json!({ "command": "rm -rf build" }));
        let asking = event("AskUserQuestion", question_input());
        assert!(allows(&reply(&Answer::Allow, &command).unwrap()));
        assert!(!allows(&reply(&Answer::Deny, &command).unwrap()));
        assert!(reply(&Answer::Terminal, &command).is_none());
        // Answers never stand in for an Allow, and Allow never answers a question.
        let answers = Map::from_iter([("Which database?".to_string(), json!("Postgres"))]);
        assert!(reply(&Answer::Answers { answers: answers.clone() }, &command).is_none());
        assert!(reply(&Answer::Allow, &asking).is_none());
        // Half the questions answered is no answer.
        assert!(reply(&Answer::Answers { answers }, &asking).is_none());
    }

    #[test]
    fn always_keeps_only_the_rule_the_agent_suggested() {
        let mut command = event("Bash", json!({ "command": "npm test" }));
        // Nothing suggested: no Always.
        assert!(reply(&Answer::Always, &command).is_none());
        assert!(!describe(Agent::Claude, &command).unwrap().always);
        let rule = json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }], "behavior": "allow", "destination": "localSettings" });
        command.suggestions = vec![rule.clone()];
        assert!(describe(Agent::Claude, &command).unwrap().always);
        assert!(!describe(Agent::Codex, &command).unwrap().always, "Codex has no rules to keep");
        let value: Value = serde_json::from_str(&reply(&Answer::Always, &command).unwrap()).unwrap();
        let decision = &value["hookSpecificOutput"]["decision"];
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedPermissions"], json!([rule]));
        // A question is never answered by Always.
        let mut asking = event("AskUserQuestion", question_input());
        asking.suggestions = command.suggestions.clone();
        assert!(reply(&Answer::Always, &asking).is_none());
    }

    #[test]
    fn always_shows_each_rule_and_where_and_never_keeps_one_for_every_project() {
        let mut command = event("Bash", json!({ "command": "npm test" }));
        command.suggestions = vec![
            json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }, { "toolName": "Read" }], "behavior": "allow", "destination": "localSettings" }),
            json!({ "type": "setMode", "mode": "acceptEdits", "destination": "session" }),
            json!({ "type": "addDirectories", "directories": ["/Users/me/other"], "destination": "session" }),
        ];
        let view = describe(Agent::Claude, &command).unwrap();
        assert!(view.always);
        let shown: Vec<(&str, &str)> = view.rules.iter().map(|rule| (rule.rule.as_str(), rule.place.as_str())).collect();
        assert_eq!(shown, [("Bash(npm test)", "This project"), ("Read", "This project"), ("Accept edits", "This session"), ("Access to /Users/me/other", "This session")]);
        // One the card can't show, or one kept for every project, takes Always away entirely.
        for odd in [
            json!({ "type": "addRules", "rules": [{ "toolName": "Bash" }], "behavior": "allow", "destination": "userSettings" }),
            json!({ "type": "addRules", "rules": [{ "toolName": "Bash" }], "behavior": "allow" }),
            json!({ "type": "addRules", "rules": [{ "toolName": "Bash" }], "behavior": "deny", "destination": "session" }),
            json!({ "type": "replaceRules", "rules": [{ "toolName": "Bash" }], "behavior": "allow", "destination": "session" }),
            json!({ "type": "setMode", "mode": "bypassPermissions", "destination": "session" }),
            json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "extra": 1 }], "behavior": "allow", "destination": "session" }),
            json!({ "type": "addRules", "rules": [], "behavior": "allow", "destination": "session" }),
        ] {
            let mut widened = command.clone();
            widened.suggestions.push(odd.clone());
            let view = describe(Agent::Claude, &widened).unwrap();
            assert!(!view.always && view.rules.is_empty(), "{odd}");
            assert!(view.allow, "Allow alone still fits: {odd}");
            assert!(reply(&Answer::Always, &widened).is_none(), "{odd}");
        }
        // The shared project file says so.
        let mut shared = event("Bash", json!({ "command": "make" }));
        shared.suggestions = vec![json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "make" }], "behavior": "allow", "destination": "projectSettings" })];
        assert_eq!(describe(Agent::Claude, &shared).unwrap().rules[0].place, "This project, shared");
    }

    #[test]
    fn only_a_command_shown_whole_and_as_it_runs_can_be_allowed_here() {
        let allowed = |agent: Agent, input: Value| describe(agent, &event("Bash", input)).unwrap();
        let plain = allowed(Agent::Claude, json!({ "command": "npm test", "description": "Test", "timeout": 120000, "run_in_background": false }));
        assert!(plain.allow);
        // Cut short, it goes to the terminal: no Allow, no Always.
        let mut long = event("Bash", json!({ "command": format!("echo {}", "x".repeat(MAX_DETAIL)) }));
        long.suggestions = vec![json!({ "type": "addRules", "rules": [{ "toolName": "Bash" }], "behavior": "allow", "destination": "session" })];
        let long = describe(Agent::Claude, &long).unwrap();
        assert!(!long.allow && !long.always);
        // Outside the sandbox, or with a setting the card doesn't know.
        assert!(!allowed(Agent::Claude, json!({ "command": "ls", "dangerouslyDisableSandbox": true })).allow);
        assert!(allowed(Agent::Claude, json!({ "command": "ls", "dangerouslyDisableSandbox": false })).allow);
        assert!(!allowed(Agent::Claude, json!({ "command": "ls", "shell": "/bin/zsh" })).allow);
        // Codex's hook doesn't say whether it would leave the sandbox.
        assert!(!allowed(Agent::Codex, json!({ "command": "git push", "description": "Push it" })).allow);
        // Newlines stay, so every line of a script shows.
        assert_eq!(allowed(Agent::Claude, json!({ "command": "cd x\nrm -rf y" })).detail, "cd x\nrm -rf y");
        // The answer must fit: Allow is refused when it wasn't offered.
        let request = allowed(Agent::Codex, json!({ "command": "ls" }));
        let (sender, _receiver) = oneshot::channel();
        let id = request.id.clone();
        hub().waiting.push(Waiting { view: request, agent: Agent::Codex, session: String::new(), tool: "Bash".into(), input: Value::Null, reply: Some(sender), at_ms: crate::surface::now_ms() });
        std::thread::sleep(Duration::from_millis(ARMING_MS + 50));
        assert!(answer_agent(id.clone(), Answer::Allow).is_err());
        assert!(answer_agent(id.clone(), Answer::Deny).is_ok());
        hub().waiting.retain(|waiting| waiting.view.id != id);
    }

    #[test]
    fn an_edit_shows_every_line_it_changes() {
        let changes = |tool: &str, input: Value| {
            let view = describe(Agent::Claude, &event(tool, input)).unwrap();
            (view.allow, view.changes.iter().map(|line| format!("{}{}", &line.kind[..1], line.text)).collect::<Vec<_>>())
        };
        let (allow, lines) = changes("Edit", json!({ "file_path": "/Users/me/projects/shop/a.rs", "old_string": "let a = 1;", "new_string": "let a = 2;\nlet b = 3;" }));
        assert!(allow);
        assert_eq!(lines, ["rlet a = 1;", "alet a = 2;", "alet b = 3;"]);
        let (allow, lines) = changes("Write", json!({ "file_path": "/Users/me/projects/shop/new.txt", "content": "one\ntwo" }));
        assert!(allow);
        assert_eq!(lines, ["aone", "atwo"]);
        let (allow, lines) = changes("MultiEdit", json!({ "file_path": "/a.rs", "edits": [{ "old_string": "x", "new_string": "y" }, { "old_string": "p", "new_string": "q" }] }));
        assert!(allow);
        assert_eq!(lines, ["rx", "ay", "g", "rp", "aq"]);
        let notebook = describe(Agent::Claude, &event("NotebookEdit", json!({ "notebook_path": "/Users/me/projects/shop/n.ipynb", "cell_id": "c7", "edit_mode": "insert", "cell_type": "code", "new_source": "print(1)" }))).unwrap();
        assert!(notebook.allow);
        assert_eq!(notebook.detail, "n.ipynb\ninsert · c7 · code");
        let patch = describe(Agent::Codex, &event("apply_patch", json!({ "command": "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch" }))).unwrap();
        assert!(patch.allow);
        assert_eq!(patch.changes.iter().map(|line| line.kind.as_str()).collect::<Vec<_>>(), ["same", "same", "remove", "add"]);
        // Every match, too many lines, a line too long, or nothing to compare: the terminal.
        assert!(!changes("Edit", json!({ "file_path": "/a.rs", "old_string": "a", "new_string": "b", "replace_all": true })).0);
        assert!(!changes("Write", json!({ "file_path": "/a.rs", "content": "line\n".repeat(MAX_CHANGE_LINES + 1) })).0);
        assert!(!changes("Write", json!({ "file_path": "/a.rs", "content": "x".repeat(MAX_CHANGE_CHARS + 1) })).0);
        assert!(!changes("Edit", json!({ "file_path": "/a.rs" })).0);
        assert!(!changes("Edit", json!({ "file_path": "/a.rs", "old_string": "a", "new_string": "b", "mode": "fuzzy" })).0);
    }

    #[test]
    fn hidden_characters_are_written_out() {
        let tricked = describe(Agent::Claude, &event("Bash", json!({ "command": "echo safe \u{202E}fdsa\u{202C} && rm\u{200B} -rf ~\recho", "description": "List \u{2066}files" }))).unwrap();
        assert_eq!(tricked.detail, "echo safe <U+202E>fdsa<U+202C> && rm<U+200B> -rf ~<U+000D>echo");
        assert_eq!(tricked.title, "List <U+2066>files");
        assert_eq!(visible("tabs\tand\nnew lines"), "tabs\tand\nnew lines");
        let edit = describe(Agent::Claude, &event("Edit", json!({ "file_path": "/a.rs", "old_string": "a", "new_string": "b\u{200F}" }))).unwrap();
        assert_eq!(edit.changes[1].text, "b<U+200F>");
    }

    #[test]
    fn a_plan_is_accepted_with_the_mode_to_work_in() {
        let plan = event("ExitPlanMode", json!({ "plan": "1. Do it" }));
        let view = describe(Agent::Claude, &plan).unwrap();
        assert!(view.always);
        for (answer, mode) in [(Answer::Allow, "default"), (Answer::Always, "acceptEdits")] {
            let value: Value = serde_json::from_str(&reply(&answer, &plan).unwrap()).unwrap();
            let decision = &value["hookSpecificOutput"]["decision"];
            assert_eq!(decision["behavior"], "allow");
            assert_eq!(decision["updatedInput"], json!({ "plan": "1. Do it" }));
            assert_eq!(decision["updatedPermissions"], json!([{ "type": "setMode", "mode": mode, "destination": "session" }]));
        }
        assert!(!allows(&reply(&Answer::Deny, &plan).unwrap()));
    }

    #[test]
    fn the_terminal_line_is_read_before_the_event() {
        let (host, hint, rest) = split_header(b"BUNTINI\tiTerm.app\tcom.googlecode.iterm2\tw0t0p0\tcli\tpostToolUse\n{\"a\":1}").unwrap();
        assert_eq!((host.term_program.as_str(), host.bundle.as_str(), host.iterm_session.as_str(), host.entrypoint.as_str(), hint.as_str()), ("iTerm.app", "com.googlecode.iterm2", "w0t0p0", "cli", "postToolUse"));
        assert_eq!(rest, b"{\"a\":1}");
        let (plain, hint, rest) = split_header(b"{\"a\":1}").unwrap();
        assert_eq!((plain, hint), (session::Host::default(), String::new()));
        assert_eq!(rest, b"{\"a\":1}");
        // Still arriving: nothing yet.
        assert_eq!(split_header(b"BUNTINI\tiTerm").unwrap().2, b"");
    }

    #[test]
    fn answers_go_back_as_the_question_with_its_answers() {
        let asking = event("AskUserQuestion", question_input());
        let answers = Map::from_iter([
            ("Which database?".to_string(), json!("SQLite")),
            ("Which extras?".to_string(), json!(["Auth", "Search"])),
        ]);
        let line = reply(&Answer::Answers { answers }, &asking).unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        let decision = &value["hookSpecificOutput"]["decision"];
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedInput"]["questions"], question_input()["questions"]);
        assert_eq!(decision["updatedInput"]["answers"]["Which database?"], "SQLite");
    }

    #[test]
    fn only_the_offered_choices_answer() {
        let input = question_input();
        let fit = |pairs: &[(&str, Value)]| answers_fit(&input, &Map::from_iter(pairs.iter().map(|(k, v)| (k.to_string(), v.clone()))));
        assert!(fit(&[("Which database?", json!("Postgres")), ("Which extras?", json!(["Search"]))]));
        assert!(!fit(&[("Which database?", json!("MySQL")), ("Which extras?", json!(["Search"]))]));
        assert!(!fit(&[("Which database?", json!(["Postgres"])), ("Which extras?", json!(["Search"]))]));
        assert!(!fit(&[("Which database?", json!("Postgres")), ("Which extras?", json!("Search"))]));
        assert!(!fit(&[("Which database?", json!("Postgres")), ("Which extras?", json!([]))]));
        assert!(!fit(&[("Which database?", json!("Postgres")), ("Which extras?", json!(["Auth", "Auth"]))]));
        assert!(!fit(&[("Which database?", json!("Postgres")), ("Which extras?", json!(["Auth"])), ("Other?", json!("x"))]));
    }

    #[test]
    fn requests_read_as_what_the_agent_wants_to_do() {
        let run = describe(Agent::Claude, &event("Bash", json!({ "command": "npm test", "description": "Run the tests" }))).unwrap();
        assert_eq!((run.agent.as_str(), run.project.as_str(), run.title.as_str(), run.detail.as_str(), run.code), ("Claude Code", "shop", "Run the tests", "npm test", true));
        let bare = describe(Agent::Codex, &event("Bash", json!({ "command": ["git", "push"] }))).unwrap();
        assert_eq!((bare.title.as_str(), bare.detail.as_str()), ("Run a command", "git push"));
        let edit = describe(Agent::Claude, &event("Edit", json!({ "file_path": "/Users/me/projects/shop/src/cart.ts" }))).unwrap();
        assert_eq!((edit.title.as_str(), edit.detail.as_str()), ("Edit cart.ts", "src/cart.ts"));
        let patch = describe(Agent::Codex, &event("apply_patch", json!({ "command": "*** Begin Patch\n*** Update File: /Users/me/projects/shop/a.rs\n@@\n*** Add File: b.rs\n*** End Patch" }))).unwrap();
        assert_eq!(patch.detail, "a.rs\nb.rs");
        let tool = describe(Agent::Claude, &event("mcp__github__create_issue", json!({ "title": "Bug" }))).unwrap();
        assert_eq!(tool.title, "Use create issue from github");
        let asking = describe(Agent::Claude, &event("AskUserQuestion", question_input())).unwrap();
        assert_eq!(asking.questions.len(), 2);
        assert!(asking.questions[1].multi_select);
        // What can't be shown faithfully stays with the terminal.
        assert!(describe(Agent::Claude, &event("AskUserQuestion", json!({ "questions": [] }))).is_none());
        assert!(describe(Agent::Claude, &event("Bash", json!({}))).is_none());
        let long = describe(Agent::Claude, &event("Bash", json!({ "command": "x".repeat(MAX_DETAIL * 2) }))).unwrap();
        assert!(long.detail.chars().count() <= MAX_DETAIL + 1);
        assert!(!long.allow);
    }

    #[test]
    fn a_finished_turn_reads_as_its_first_line() {
        assert_eq!(first_line("\n## Done\nAll tests pass.", 160), "Done");
        assert_eq!(first_line("", 160), "");
        assert_eq!(first_line(&"word ".repeat(100), 20).chars().count(), 20);
    }

    #[test]
    fn the_answer_must_fit_the_request() {
        let waiting = |questions: Vec<AgentQuestion>| {
            let (sender, receiver) = oneshot::channel();
            let id = next_id();
            let view = AgentRequest { id: id.clone(), agent: "Claude Code".into(), project: String::new(), title: String::new(), detail: String::new(), code: false, questions, always: false, plan: false, allow: true, changes: Vec::new(), rules: Vec::new(), created_at: String::new() };
            hub().waiting.push(Waiting { view, agent: Agent::Claude, session: String::new(), tool: String::new(), input: Value::Null, reply: Some(sender), at_ms: crate::surface::now_ms() });
            (id, receiver)
        };
        let (id, mut receiver) = waiting(Vec::new());
        // A click in the card's first moment was aimed at what was there before.
        assert!(answer_agent(id.clone(), Answer::Allow).is_err());
        assert!(receiver.try_recv().is_err());
        std::thread::sleep(Duration::from_millis(ARMING_MS + 50));
        assert!(answer_agent(id.clone(), Answer::Answers { answers: Map::new() }).is_err());
        assert!(answer_agent(id.clone(), Answer::Always).is_err(), "Always wasn't offered");
        assert!(answer_agent(id.clone(), Answer::Allow).is_ok());
        assert!(matches!(receiver.try_recv(), Ok(Answer::Allow)));
        // One click per request.
        assert!(answer_agent(id.clone(), Answer::Deny).is_err());
        let question = parse_questions(&question_input()).unwrap();
        let (asked, _receiver) = waiting(question);
        std::thread::sleep(Duration::from_millis(ARMING_MS + 50));
        assert!(answer_agent(asked.clone(), Answer::Allow).is_err());
        assert!(answer_agent(asked.clone(), Answer::Terminal).is_ok());
        assert!(answer_agent("missing".into(), Answer::Allow).is_err());
        hub().waiting.retain(|waiting| waiting.view.id != id && waiting.view.id != asked);
    }

    #[test]
    fn a_request_settled_in_the_terminal_leaves_the_cat() {
        let asked = event("Bash", json!({ "command": "cargo build", "description": "Build" }));
        let mut asked = asked;
        asked.session = "settled-session".into();
        let view = describe(Agent::Claude, &asked).unwrap();
        let id = view.id.clone();
        let (sender, mut receiver) = oneshot::channel();
        hub().waiting.push(Waiting { view, agent: Agent::Claude, session: asked.session.clone(), tool: asked.tool.clone(), input: asked.input.clone(), reply: Some(sender), at_ms: 0 });
        let showing = || agent_requests().iter().any(|request| request.id == id);
        // Another tool, another session or another agent leaves it be.
        let mut other = asked.clone();
        other.name = "PostToolUse".into();
        other.input = json!({ "command": "cargo test" });
        settled(Agent::Claude, &other);
        let mut elsewhere = asked.clone();
        elsewhere.session = "another".into();
        settled(Agent::Claude, &elsewhere);
        settled(Agent::Codex, &asked);
        assert!(showing());
        // The same command ran, with whatever the agent added to it: settled.
        let mut ran = asked.clone();
        ran.input = json!({ "command": "cargo build", "description": "Build", "timeout": 120000 });
        settled(Agent::Claude, &ran);
        assert!(!showing());
        // Its hook stops waiting, with nothing allowed.
        assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn a_hook_that_goes_away_is_noticed_and_one_that_waits_is_not() {
        runtime().block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.sock");
            let listener = bind(&path).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            let client_path = path.clone();
            let client = std::thread::spawn(move || {
                let mut client = std::os::unix::net::UnixStream::connect(&client_path).unwrap();
                client.write_all(b"{\"hook_event_name\":\"PermissionRequest\"}").unwrap();
                // Like nc once the agent's input ends: still there, still reading.
                client.shutdown(std::net::Shutdown::Write).unwrap();
                std::thread::sleep(Duration::from_millis(1500));
            });
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(read_payload(&mut stream).await.is_some());
            let early = tokio::time::timeout(Duration::from_millis(1000), hook_gone(&stream)).await;
            assert!(early.is_err(), "a waiting hook looked gone");
            client.join().unwrap();
            let late = tokio::time::timeout(Duration::from_secs(3), hook_gone(&stream)).await;
            assert!(late.is_ok(), "a hook that went away wasn't noticed");
        });
    }

    #[test]
    fn a_stale_socket_is_replaced_and_anything_else_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.sock");
        runtime().block_on(async {
            drop(bind(&path).unwrap());
            assert!(bind(&path).is_ok());
        });
        let file = dir.path().join("file.sock");
        std::fs::write(&file, b"mine").unwrap();
        runtime().block_on(async { assert!(bind(&file).is_err()) });
        assert_eq!(std::fs::read(&file).unwrap(), b"mine");
        remove_socket(&file);
        assert!(file.exists());
    }

    /// The real hook command, run by /bin/sh with macOS's nc, against Buntini's listener.
    #[test]
    fn the_installed_hook_carries_a_request_and_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = Agent::Claude.socket(dir.path());
        let command = crate::agent_setup::hook_command(&path);
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let listener = runtime.block_on(async { bind(&path).unwrap() });
        runtime.spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve(stream, Agent::Claude).await;
        });
        let run = std::thread::spawn(move || {
            let mut child = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&command)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(br#"{"hook_event_name":"PermissionRequest","session_id":"s9","cwd":"/tmp/shop","tool_name":"Bash","tool_input":{"command":"make","description":"Build it"}}"#)
                .unwrap();
            let mut out = String::new();
            child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
            (child.wait().unwrap(), out)
        });
        let shown = (0..50)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(100));
                agent_requests().into_iter().find(|request| request.title == "Build it")
            })
            .expect("the request never showed");
        assert_eq!((shown.detail.as_str(), shown.project.as_str()), ("make", "shop"));
        std::thread::sleep(Duration::from_millis(ARMING_MS));
        answer_agent(shown.id.clone(), Answer::Allow).unwrap();
        let (status, out) = run.join().unwrap();
        assert!(status.success());
        assert!(allows(out.trim()), "{out}");
        assert!(agent_requests().iter().all(|request| request.id != shown.id));

        // The note hook carries the terminal it ran in, and the event, to the session.
        let note = crate::agent_setup::note_command(&path);
        let listener = runtime.block_on(async {
            let _ = std::fs::remove_file(&path);
            bind(&path).unwrap()
        });
        runtime.spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve(stream, Agent::Claude).await;
        });
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&note)
            .env("TERM_PROGRAM", "iTerm.app")
            .env("__CFBundleIdentifier", "com.googlecode.iterm2")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"PreToolUse","session_id":"note-1","cwd":"/tmp/shop","tool_name":"Bash","tool_input":{"command":"cargo test"}}"#).unwrap();
        assert!(child.wait().unwrap().success());
        let seen = (0..50).find_map(|_| {
            std::thread::sleep(Duration::from_millis(50));
            hub().sessions.get(&(Agent::Claude, "note-1".to_string())).map(|session| (session.host.term_program.clone(), session.steps.back().map(|step| step.label.clone())))
        });
        assert_eq!(seen, Some(("iTerm.app".to_string(), Some("Tests · cargo test".to_string()))));

        // Nobody listening: the hook says nothing and succeeds.
        let missing = crate::agent_setup::hook_command(&dir.path().join("none.sock"));
        let output = std::process::Command::new("/bin/sh").arg("-c").arg(missing).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
    }

    /// Real Claude Code, through Buntini's listener and the hooks as installed: an
    /// Allow runs the command and a Deny doesn't. (`claude -p` never asks questions;
    /// those were checked in an interactive session.) Uses this Mac's Claude sign-in:
    /// `cargo test --lib claude_code_live -- --ignored`.
    #[test]
    #[ignore]
    fn claude_code_live() {
        let dir = tempfile::Builder::new().prefix("buntini").tempdir().unwrap();
        let work = tempfile::Builder::new().prefix("buntini-live").tempdir().unwrap();
        let socket = Agent::Claude.socket(dir.path());
        let listener = tauri::async_runtime::block_on(async { bind(&socket).unwrap() });
        tauri::async_runtime::spawn(accept(listener, Agent::Claude));
        let settings = dir.path().join("settings.json");
        let hooks = crate::agent_setup::edited(None, Agent::Claude, Some(&socket)).unwrap().unwrap();
        std::fs::write(&settings, hooks).unwrap();
        let project = project(&work.path().to_string_lossy());
        let ask = |prompt: &str, choose: &dyn Fn(&AgentRequest) -> Answer| -> (String, Vec<AgentRequest>) {
            let (prompt, settings, work) = (prompt.to_string(), settings.clone(), work.path().to_path_buf());
            let run = std::thread::spawn(move || {
                std::process::Command::new("claude")
                    .args(["-p", "--model", "haiku", "--settings"])
                    .arg(&settings)
                    .arg(&prompt)
                    .current_dir(&work)
                    .stdin(std::process::Stdio::null())
                    .output()
                    .unwrap()
            });
            let mut seen = Vec::new();
            while !run.is_finished() {
                std::thread::sleep(Duration::from_millis(200));
                for request in agent_requests().into_iter().filter(|request| request.project == project) {
                    if !seen.iter().any(|other: &AgentRequest| other.id == request.id) {
                        std::thread::sleep(Duration::from_millis(ARMING_MS));
                        answer_agent(request.id.clone(), choose(&request)).unwrap();
                        seen.push(request);
                    }
                }
            }
            let output = run.join().unwrap();
            (String::from_utf8_lossy(&output.stdout).into_owned(), seen)
        };

        let (_, seen) = ask("Use the Bash tool to run exactly this command: echo yes > allowed.txt . Then reply with the single word done.", &|_| Answer::Allow);
        assert!(work.path().join("allowed.txt").exists(), "{seen:?}");
        assert!(seen.iter().any(|request| request.detail.contains("allowed.txt") && request.agent == "Claude Code"), "{seen:?}");

        let (_, seen) = ask("Use the Bash tool to run exactly this command: echo no > denied.txt . If it is refused, reply with the single word refused.", &|_| Answer::Deny);
        assert!(!seen.is_empty());
        assert!(!work.path().join("denied.txt").exists());

        // Nothing is left showing once the turns are over.
        assert!(agent_requests().iter().all(|request| request.project != project));
    }

    /// Real Claude Code with every hook Buntini installs: the step ticker fills in, Always
    /// keeps Claude Code's own suggested rule, and a plan starts in the mode chosen.
    /// `cargo test --lib claude_code_ticker_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn claude_code_ticker_live() {
        let dir = tempfile::Builder::new().prefix("buntini").tempdir().unwrap();
        let work = tempfile::Builder::new().prefix("buntini-live").tempdir().unwrap();
        let socket = Agent::Claude.socket(dir.path());
        let listener = tauri::async_runtime::block_on(async { bind(&socket).unwrap() });
        tauri::async_runtime::spawn(accept(listener, Agent::Claude));
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, crate::agent_setup::edited(None, Agent::Claude, Some(&socket)).unwrap().unwrap()).unwrap();
        let folder = project(&work.path().to_string_lossy());
        let run = |args: &[&str], prompt: &str, choose: &dyn Fn(&AgentRequest) -> Answer| -> Vec<AgentRequest> {
            let mut command = std::process::Command::new("claude");
            command.args(["-p", "--model", "haiku", "--settings"]).arg(&settings).args(args).arg(prompt).current_dir(work.path()).stdin(std::process::Stdio::null());
            let child = std::thread::spawn(move || command.output().unwrap());
            let mut seen: Vec<AgentRequest> = Vec::new();
            while !child.is_finished() {
                std::thread::sleep(Duration::from_millis(200));
                for request in agent_requests().into_iter().filter(|request| request.project == folder) {
                    if !seen.iter().any(|other| other.id == request.id) {
                        println!("request: {} · {} · always {} · plan {}", request.title, request.detail.lines().next().unwrap_or_default(), request.always, request.plan);
                        std::thread::sleep(Duration::from_millis(ARMING_MS));
                        answer_agent(request.id.clone(), choose(&request)).unwrap();
                        seen.push(request);
                    }
                }
            }
            let output = child.join().unwrap();
            println!("claude said: {}", String::from_utf8_lossy(&output.stdout).trim());
            seen
        };

        let seen = run(&[], "Use the Bash tool to run exactly: echo always > always.txt . Then reply with the single word done.", &|request| if request.always { Answer::Always } else { Answer::Allow });
        assert!(work.path().join("always.txt").exists(), "{seen:?}");
        let steps: Vec<String> = hub().sessions.values().filter(|session| session.project() == folder).flat_map(|session| session.steps.iter().map(|step| step.label.clone())).collect();
        println!("steps: {steps:?}");
        assert!(steps.iter().any(|label| label.starts_with("Runs · echo always")), "{steps:?}");
        let rules = std::fs::read_to_string(work.path().join(".claude/settings.local.json")).unwrap_or_default();
        println!("rules kept: {rules}");
        if seen.iter().any(|request| request.always) {
            assert!(rules.contains("echo always"), "Always didn't keep the rule");
        }

        let seen = run(&["--permission-mode", "plan"], "Make a one-line plan to create a file named planned.txt containing ok, present it with ExitPlanMode, and once approved create the file with the Write tool.", &|_| Answer::Allow);
        println!("plan requests: {:?}", seen.iter().map(|request| (&request.title, request.plan)).collect::<Vec<_>>());
        assert!(seen.iter().any(|request| request.plan), "no plan was shown");
        assert!(work.path().join("planned.txt").exists(), "the plan didn't start");
    }

    /// Real Copilot CLI with Buntini's hook file: its camelCase and PascalCase events reach
    /// the same session and steps. The file goes in `~/.copilot/hooks` for the run and is
    /// removed after. `cargo test --lib copilot_ticker_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn copilot_ticker_live() {
        let dir = tempfile::Builder::new().prefix("buntini").tempdir().unwrap();
        let work = tempfile::Builder::new().prefix("buntini-live").tempdir().unwrap();
        let socket = Agent::Copilot.socket(dir.path());
        let listener = tauri::async_runtime::block_on(async { bind(&socket).unwrap() });
        tauri::async_runtime::spawn(accept(listener, Agent::Copilot));
        let hooks = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".copilot/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let file = hooks.join("buntini-live-test.json");
        std::fs::write(&file, crate::agent_setup::copilot_hooks(&socket)).unwrap();
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _remove = Remove(file);
        let copilot = usage::find_program("copilot").expect("copilot isn't installed");
        let mut command = std::process::Command::new(copilot);
        command.args(["-p", "Use your shell tool to run exactly: echo hi > copilot.txt . Then reply done.", "--allow-all-tools"]).current_dir(work.path()).stdin(std::process::Stdio::null());
        let child = std::thread::spawn(move || command.output().unwrap());
        // Copilot ends its session as it exits, so look while it runs.
        let mut sessions: Vec<(String, Vec<String>, String)> = Vec::new();
        let mut asked = Vec::new();
        while !child.is_finished() {
            std::thread::sleep(Duration::from_millis(100));
            let hub = hub();
            for (_, session) in hub.sessions.iter().filter(|((agent, _), _)| *agent == Agent::Copilot) {
                let seen = (session.project(), session.steps.iter().map(|step| step.label.clone()).collect::<Vec<_>>(), format!("{:?}", session.phase));
                sessions.retain(|(project, _, _)| *project != seen.0);
                sessions.push(seen);
            }
            for waiting in &hub.waiting {
                if !asked.contains(&waiting.view.title) {
                    asked.push(waiting.view.title.clone());
                }
            }
        }
        let output = child.join().unwrap();
        println!("copilot said: {}", String::from_utf8_lossy(&output.stdout).trim());
        let folder = project(&work.path().to_string_lossy());
        println!("sessions: {sessions:?}");
        println!("asked: {asked:?}");
        assert!(asked.is_empty(), "Copilot asks about every tool; Buntini must not hold them");
        assert!(work.path().join("copilot.txt").exists());
        assert!(sessions.iter().any(|(project, steps, _)| project == &folder && steps.iter().any(|label| label.contains("echo hi"))), "{sessions:?}");
    }

    /// Asks this Mac's Codex for its plan windows, as Buntini does after a Codex turn.
    /// `cargo test --lib codex_usage_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn codex_usage_live() {
        let usage = usage::read_codex();
        println!("{usage:?}");
        assert!(usage.is_some_and(|usage| !usage.windows.is_empty()));
    }
}
