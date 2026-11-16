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

use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt, Interest};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

/// Longest event read from a hook. Events Buntini listens to are a few KB.
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
/// How long a hook may take to hand over its event.
const READ_BUDGET: Duration = Duration::from_secs(5);
/// How long a request waits for a click before the terminal takes it back. The
/// hook itself is installed with a little more (agent_setup.rs). Claude Code shows
/// its own prompt meanwhile; Codex waits for the hook first, so it gets less.
fn decision_budget(agent: Agent) -> Duration {
    match agent {
        Agent::Claude => Duration::from_secs(110),
        Agent::Codex => Duration::from_secs(60),
    }
}
/// How often a waiting request checks that its hook is still running.
const HOOK_CHECK: Duration = Duration::from_millis(400);
/// A turn this long is worth saying it finished.
const LONG_TURN: Duration = Duration::from_secs(20);
/// Longest command, path or plan shown on a request.
const MAX_DETAIL: usize = 1_200;
/// macOS keeps socket paths under 104 bytes, the terminating zero included.
const MAX_SOCKET_PATH: usize = 103;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub const ALL: [Agent; 2] = [Agent::Claude, Agent::Codex];

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
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
}

#[derive(Default)]
struct Hub {
    app: Option<AppHandle>,
    next: u64,
    waiting: Vec<Waiting>,
    turns: HashMap<(Agent, String), Instant>,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(|| Mutex::new(Hub::default()));
static DIR: OnceLock<PathBuf> = OnceLock::new();

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
    let dir = DIR.get_or_init(|| dir).clone();
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
    let Ok(Some(payload)) = tokio::time::timeout(READ_BUDGET, read_payload(&mut stream)).await
    else {
        return;
    };
    let Some(event) = HookEvent::from_payload(&payload) else { return };
    mark_heard(agent);
    match event.name.as_str() {
        "UserPromptSubmit" => started(agent, &event),
        "PostToolUse" | "PostToolUseFailure" => settled(agent, &event),
        "Stop" => finished(agent, &event),
        "PermissionRequest" => {
            if let Some(reply) = ask(agent, &event, &stream).await {
                let mut line = reply.into_bytes();
                line.push(b'\n');
                let _ = stream.write_all(&line).await;
                let _ = stream.shutdown().await;
            }
        }
        _ => {}
    }
}

/// The event as JSON. `nc` closes its sending side once the agent's input ends,
/// but the value is taken as soon as it's whole either way.
async fn read_payload(stream: &mut UnixStream) -> Option<Value> {
    let mut buffer = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        buffer.extend_from_slice(&chunk[..read]);
        let body = buffer.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buffer);
        match serde_json::Deserializer::from_slice(body).into_iter::<Value>().next() {
            Some(Ok(value)) => return Some(value),
            Some(Err(error)) if !error.is_eof() => return None,
            _ => {}
        }
        if read == 0 || buffer.len() > MAX_PAYLOAD {
            return None;
        }
    }
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
    let changed = hub.waiting.len() != before;
    let now = Instant::now();
    hub.turns.retain(|_, began| now.duration_since(*began) < Duration::from_secs(24 * 3600));
    if hub.turns.len() < 500 {
        hub.turns.insert((agent, event.session.clone()), now);
    }
    if changed {
        emit_requests(&hub);
    }
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
    let mut hub = hub();
    // The turn is over: nothing of it is still waiting.
    let before = hub.waiting.len();
    hub.waiting.retain(|waiting| !(waiting.agent == agent && waiting.session == event.session));
    if hub.waiting.len() != before {
        emit_requests(&hub);
    }
    let Some(began) = hub.turns.remove(&(agent, event.session.clone())) else { return };
    if began.elapsed() < LONG_TURN {
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

/// Shows the request and waits for the click, the hook going away, or the time limit.
/// Returns what the hook prints back, if anything.
async fn ask(agent: Agent, event: &HookEvent, stream: &UnixStream) -> Option<String> {
    let view = describe(agent, event)?;
    let id = view.id.clone();
    let (sender, receiver) = oneshot::channel();
    {
        let mut hub = hub();
        hub.waiting.push(Waiting {
            view,
            agent,
            session: event.session.clone(),
            tool: event.tool.clone(),
            input: event.input.clone(),
            reply: Some(sender),
        });
        emit_requests(&hub);
    }
    let answer = tokio::select! {
        answer = receiver => answer.ok(),
        _ = hook_gone(stream) => None,
        _ = tokio::time::sleep(decision_budget(agent)) => None,
    };
    {
        let mut hub = hub();
        hub.waiting.retain(|waiting| waiting.view.id != id);
        emit_requests(&hub);
    }
    reply(&answer?, event)
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
}

fn views(hub: &Hub) -> Vec<AgentRequest> {
    hub.waiting.iter().map(|waiting| waiting.view.clone()).collect()
}

fn next_id() -> String {
    let mut hub = hub();
    hub.next += 1;
    format!("agent-{}-{}", std::process::id(), hub.next)
}

/// What the request is, in words. None for something Buntini can't show faithfully,
/// which then stays with the terminal.
fn describe(agent: Agent, event: &HookEvent) -> Option<AgentRequest> {
    let input = &event.input;
    let string = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let path = |key: &str| relative(&string(key), &event.cwd);
    let mut questions = Vec::new();
    let (title, detail, code) = match event.tool.as_str() {
        "AskUserQuestion" => {
            questions = parse_questions(input)?;
            (String::new(), String::new(), false)
        }
        "Bash" | "shell" | "local_shell" | "exec_command" | "unified_exec" => {
            let command = match input.get("command").or_else(|| input.get("cmd")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                Some(Value::String(command)) => command.trim().to_string(),
                _ => return None,
            };
            let said = string("description");
            (if said.is_empty() { "Run a command".into() } else { said }, command, true)
        }
        "apply_patch" => {
            let patch = match input.get("command").or_else(|| input.get("patch")).or_else(|| input.get("input")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
                Some(Value::String(patch)) => patch.clone(),
                _ => String::new(),
            };
            let files = patch_files(&patch, &event.cwd);
            ("Change files".into(), if files.is_empty() { patch } else { files.join("\n") }, true)
        }
        "Edit" | "MultiEdit" => (format!("Edit {}", file_name(&string("file_path"))), path("file_path"), true),
        "Write" => (format!("Write {}", file_name(&string("file_path"))), path("file_path"), true),
        "NotebookEdit" => (format!("Edit {}", file_name(&string("notebook_path"))), path("notebook_path"), true),
        "Read" => (format!("Read {}", file_name(&string("file_path"))), path("file_path"), true),
        "Glob" | "Grep" => {
            let place = string("path");
            ("Search files".into(), [string("pattern"), relative(&place, &event.cwd)].iter().filter(|part| !part.is_empty()).cloned().collect::<Vec<_>>().join("  ·  "), true)
        }
        "WebFetch" => ("Open a web page".into(), string("url"), true),
        "WebSearch" => ("Search the web".into(), string("query"), false),
        "ExitPlanMode" => ("Start on its plan".into(), string("plan"), false),
        tool if tool.starts_with("mcp__") => {
            let mut parts = tool.splitn(3, "__").skip(1);
            let server = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            (format!("Use {} from {}", words(name), server), compact(input), true)
        }
        "" => return None,
        tool => (format!("Use {}", words(tool)), compact(input), true),
    };
    Some(AgentRequest {
        id: next_id(),
        agent: agent.name().into(),
        project: project(&event.cwd),
        title: clip(&title, 140),
        detail: clip(&detail, MAX_DETAIL),
        code,
        questions,
        created_at: crate::types::now(),
    })
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

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a file".into())
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

/// What the hook prints for the user's answer. Nothing allows anything unless the
/// user clicked Allow, or answered the agent's questions with its own options.
fn reply(answer: &Answer, event: &HookEvent) -> Option<String> {
    let decision = match answer {
        Answer::Terminal => return None,
        Answer::Allow if event.tool != "AskUserQuestion" => json!({ "behavior": "allow" }),
        Answer::Deny => json!({ "behavior": "deny", "message": "The user said no in Buntini." }),
        Answer::Answers { answers } if event.tool == "AskUserQuestion" && answers_fit(&event.input, answers) => {
            let mut input = event.input.as_object()?.clone();
            input.insert("answers".into(), Value::Object(answers.clone()));
            json!({ "behavior": "allow", "updatedInput": input })
        }
        _ => return None,
    };
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string())
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

#[tauri::command]
pub fn agent_requests() -> Vec<AgentRequest> {
    views(&hub())
}

#[tauri::command]
pub fn answer_agent(id: String, answer: Answer) -> Result<(), String> {
    let mut hub = hub();
    let gone = || "That request was already answered.".to_string();
    let waiting = hub.waiting.iter_mut().find(|waiting| waiting.view.id == id).ok_or_else(gone)?;
    let asking = !waiting.view.questions.is_empty();
    let fits = match &answer {
        Answer::Allow => !asking,
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
}

#[tauri::command]
pub async fn agents_call(op: Op) -> Result<serde_json::Value, String> {
    match op {
        Op::Ping => Ok(serde_json::Value::Bool(true)),
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
        let long = describe(Agent::Claude, &event("Bash", json!({ "command": "x".repeat(5000) }))).unwrap();
        assert!(long.detail.chars().count() <= MAX_DETAIL + 1);
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
            let view = AgentRequest { id: id.clone(), agent: "Claude Code".into(), project: String::new(), title: String::new(), detail: String::new(), code: false, questions, created_at: String::new() };
            hub().waiting.push(Waiting { view, agent: Agent::Claude, session: String::new(), tool: String::new(), input: Value::Null, reply: Some(sender) });
            (id, receiver)
        };
        let (id, mut receiver) = waiting(Vec::new());
        assert!(answer_agent(id.clone(), Answer::Answers { answers: Map::new() }).is_err());
        assert!(answer_agent(id.clone(), Answer::Allow).is_ok());
        assert!(matches!(receiver.try_recv(), Ok(Answer::Allow)));
        // One click per request.
        assert!(answer_agent(id.clone(), Answer::Deny).is_err());
        let question = parse_questions(&question_input()).unwrap();
        let (asked, _receiver) = waiting(question);
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
        hub().waiting.push(Waiting { view, agent: Agent::Claude, session: asked.session.clone(), tool: asked.tool.clone(), input: asked.input.clone(), reply: Some(sender) });
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
        answer_agent(shown.id.clone(), Answer::Allow).unwrap();
        let (status, out) = run.join().unwrap();
        assert!(status.success());
        assert!(allows(out.trim()), "{out}");
        assert!(agent_requests().iter().all(|request| request.id != shown.id));

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
        let hooks = crate::agent_setup::edited(None, Agent::Claude, Some(&crate::agent_setup::hook_command(&socket))).unwrap().unwrap();
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
}
