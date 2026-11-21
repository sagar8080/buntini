//! Buntini's tools for other assistants (Claude, Cursor, Codex, VS Code, Zed, Raycast): an MCP
//! server on a socket of Buntini's own. An assistant starts `/usr/bin/nc -U <data>/assistants.sock`
//! as its tool server, so nothing listens on a network port and only this Mac's user reaches it.
//!
//! The first time an assistant calls a tool, the island asks whether it may use Buntini, and
//! Settings → Apps lists it to switch off later. Every change it asks for waits for Allow in the
//! island, whatever it says. Reads give only what Settings → Privacy lets external brains see.
//! What an assistant sends is data to Buntini, never instructions.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

use crate::i18n::tf;
use crate::privacy::{Locality, Share};
use crate::surface::cards::island::Request;
use crate::surface::{Card, CardBody, Offer, Pill, PillState, Source, Urgency};

/// The start of each request card's id and each progress pill's.
pub const PREFIX: &str = "assistant.";
const PROTOCOL: &str = "2025-06-18";
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// How long a question waits in the island before it counts as No.
const ASKED: Duration = Duration::from_secs(120);
/// A progress mark no assistant has updated for this long goes away.
const PROGRESS_KEPT: Duration = Duration::from_secs(10 * 60);
/// The longest line read from an assistant.
const LONGEST: usize = 1 << 20;
const TINT: &str = "#BF5AF2";

static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn socket(dir: &Path) -> PathBuf {
    dir.join("assistants.sock")
}

/// The command an assistant runs as Buntini's tool server.
pub fn command(dir: &Path) -> (String, Vec<String>) {
    ("/usr/bin/nc".into(), vec!["-U".into(), socket(dir).to_string_lossy().into_owned()])
}

/// The name an assistant goes by, from what it says it is.
pub fn display_name(client: &str) -> String {
    let client: String = client.chars().filter(|c| !c.is_control()).take(40).collect();
    match client.trim().to_lowercase().as_str() {
        "claude-ai" => "Claude".into(),
        "claude-code" => "Claude Code".into(),
        "cursor-vscode" | "cursor" => "Cursor".into(),
        "codex-mcp-client" | "codex" => "Codex".into(),
        "visual studio code" | "vscode" => "VS Code".into(),
        "zed" => "Zed".into(),
        "raycast" => "Raycast".into(),
        "gemini-cli-mcp-client" => "Gemini CLI".into(),
        "" => "An assistant".into(),
        _ => client.trim().to_string(),
    }
}

// ----- Which assistants may use Buntini -----

fn store_path() -> Option<PathBuf> {
    crate::agents::socket_dir().map(|dir| dir.join("assistants.json"))
}

fn decisions() -> HashMap<String, bool> {
    store_path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn decide(name: &str, allowed: bool) {
    let mut all = decisions();
    all.insert(name.to_string(), allowed);
    if let (Some(path), Ok(bytes)) = (store_path(), serde_json::to_vec_pretty(&all)) {
        let _ = std::fs::write(path, bytes);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Assistant {
    pub name: String,
    /// It may use Buntini: added to it from here and not switched off, or allowed when it asked.
    pub allowed: bool,
    /// For one Buntini can add itself to: whether it's in its list of tool servers.
    pub added: Option<bool>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The assistants on this Mac Buntini can be added to, and those that asked to use it.
#[tauri::command]
pub fn assistants() -> Vec<Assistant> {
    let decisions = decisions();
    let home = home();
    let mut all: Vec<Assistant> = super::assistant_setup::TARGETS
        .into_iter()
        .filter(|target| target.installed())
        .map(|target| {
            let added = home.as_deref().is_some_and(|home| target.added(home));
            Assistant { name: target.name().into(), allowed: added && decisions.get(target.name()) != Some(&false), added: Some(added) }
        })
        .collect();
    for (name, allowed) in decisions {
        if !all.iter().any(|assistant| assistant.name == name) {
            all.push(Assistant { name, allowed, added: None });
        }
    }
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    all
}

/// Switches an assistant on or off in Settings: Buntini is added to it or taken out, where it
/// can be, and it may use Buntini or not.
#[tauri::command]
pub async fn set_assistant(name: String, allowed: bool) -> Result<Vec<Assistant>, String> {
    match super::assistant_setup::Target::by_name(&name).filter(|target| target.installed()) {
        Some(target) => {
            let home = home().ok_or("There's no home folder.")?;
            let data = crate::agents::socket_dir().ok_or("Buntini is still starting.")?;
            super::assistant_setup::set(target, allowed, &home, data, |link| {
                let _ = std::process::Command::new("/usr/bin/open").arg(link).spawn();
            })
            .await?;
        }
        None if !decisions().contains_key(&name) => return Err("That assistant hasn't asked to use Buntini.".into()),
        None => {}
    }
    decide(&name, allowed);
    Ok(assistants())
}

// ----- Questions in the island -----

static WAITING: Mutex<Vec<(String, oneshot::Sender<bool>)>> = Mutex::new(Vec::new());
static ASKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn waiting() -> MutexGuard<'static, Vec<(String, oneshot::Sender<bool>)>> {
    WAITING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Shows `what` from `from` in the island with Deny and Allow, and waits for the answer. No
/// answer in time is No.
pub(crate) async fn ask(from: &str, what: String) -> bool {
    let id = format!("{PREFIX}ask.{}", ASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let (sender, receiver) = oneshot::channel();
    waiting().push((id.clone(), sender));
    super::publish(vec![Offer {
        card: Card { id: id.clone(), source: Source::Island, body: CardBody::Request(Request { from: from.to_string(), what }) },
        urgency: Urgency::Waiting,
        at_ms: crate::surface::now_ms(),
        ttl_ms: Some(ASKED.as_millis() as u64),
    }]);
    let answer = tokio::time::timeout(ASKED, receiver).await.ok().and_then(Result::ok).unwrap_or(false);
    waiting().retain(|(waiting, _)| *waiting != id);
    super::forget(&id);
    answer
}

/// Allow or Deny on a request card.
pub fn answer(card: &str, allowed: bool) {
    let mut waiting = waiting();
    if let Some(at) = waiting.iter().position(|(id, _)| id == card) {
        let (_, sender) = waiting.remove(at);
        let _ = sender.send(allowed);
    }
}

// ----- Progress beside the notch -----

struct Progress {
    label: String,
    progress: Option<f64>,
    at: Instant,
}

static PROGRESS: Mutex<Vec<(String, Progress)>> = Mutex::new(Vec::new());

/// The assistants' progress marks beside the notch.
pub fn shown() -> Vec<Pill> {
    let mut all = PROGRESS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    all.retain(|(_, progress)| progress.at.elapsed() < PROGRESS_KEPT);
    all.iter()
        .map(|(name, progress)| Pill {
            id: format!("{PREFIX}{}", name.to_lowercase().replace(' ', "-")),
            source: Source::Island,
            label: format!("{name} · {}", progress.label),
            tint: TINT.into(),
            state: PillState::Working,
            badge: progress.progress.map(|done| format!("{}%", (done * 100.0).round())),
            progress: progress.progress,
            card: None,
        })
        .collect()
}

// ----- The tools -----

fn tools() -> Value {
    json!([
        {
            "name": "today",
            "title": "Today on the Mac",
            "description": "Today's calendar events and the reminders due, as Buntini last read them on this Mac. Read only.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "now_playing",
            "title": "Now playing",
            "description": "What's playing on this Mac: the title, the artist or channel, and the app. Read only.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "add_reminder",
            "title": "Add a reminder",
            "description": "Adds a reminder in the Reminders app. The person sees it in Buntini's island and allows or denies it first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "What to be reminded of." },
                    "due": { "type": "string", "description": "When, as YYYY-MM-DD or YYYY-MM-DD HH:MM in local time. Optional." }
                },
                "required": ["title"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }
        },
        {
            "name": "show_progress",
            "title": "Show progress",
            "description": "Shows what you're working on beside the Mac's notch, with how far along it is, so the person can see it at a glance. Call again to update; done removes it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "A few words on what's running." },
                    "progress": { "type": "number", "minimum": 0, "maximum": 1, "description": "How far along, 0 to 1. Optional." },
                    "done": { "type": "boolean", "description": "True when finished." }
                },
                "required": ["label"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }
        }
    ])
}

fn text(result: Result<String, String>) -> Value {
    match result {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    }
}

fn settings() -> Option<crate::types::Settings> {
    let app = APP.get()?;
    app.try_state::<crate::controller::AppState>()?.store.lock().ok()?.settings().ok()
}

fn shared(share: Share) -> Result<(), String> {
    let settings = settings().ok_or("Buntini is still starting.")?;
    if crate::privacy::allows(Locality::External, &settings, share) {
        Ok(())
    } else {
        Err(format!("The person hasn't let outside assistants see {} (Buntini → Settings → Privacy).", share.id()))
    }
}

/// Today's events and reminders as lines.
pub fn today_lines(events: &[crate::apps::Event], reminders: &[crate::apps::Reminder]) -> String {
    let mut lines = Vec::new();
    for event in events.iter().filter(|event| !event.title.trim().is_empty()) {
        let when = if event.all_day {
            "all day".to_string()
        } else {
            match (crate::nudges::event_start(event), crate::nudges::event_end(event)) {
                (Some(start), Some(end)) => format!("{}–{}", start.format("%H:%M"), end.format("%H:%M")),
                (Some(start), None) => start.format("%H:%M").to_string(),
                _ => event.start.clone(),
            }
        };
        let place = event.location.trim();
        lines.push(format!("Event: {when} {}{}", event.title.trim(), if place.is_empty() { String::new() } else { format!(" ({place})") }));
    }
    for reminder in reminders.iter().filter(|reminder| !reminder.title.trim().is_empty()) {
        let due = reminder.due.trim();
        lines.push(format!("Reminder: {}{}", reminder.title.trim(), if due.is_empty() { String::new() } else { format!(" (due {due})") }));
    }
    if lines.is_empty() {
        "Nothing on the calendar today and no reminders due.".into()
    } else {
        lines.join("\n")
    }
}

async fn call(name: &str, arguments: &Map<String, Value>, client: &str) -> Result<String, String> {
    match name {
        "today" => {
            shared(Share::Calendar)?;
            let (events, reminders) = super::upcoming::today().ok_or("Buntini hasn't read the calendar yet: it reads it while heads-ups are on.")?;
            Ok(today_lines(&events, &reminders))
        }
        "now_playing" => {
            shared(Share::Music)?;
            Ok(match crate::media::playing_now() {
                Some((app, title, artist, playing)) => format!("{} {title}{} in {app}", if playing { "Playing" } else { "Paused:" }, artist.map(|artist| format!(" by {artist}")).unwrap_or_default()),
                None => "Nothing is playing.".into(),
            })
        }
        "add_reminder" => {
            let title = arguments.get("title").and_then(Value::as_str).map(str::trim).filter(|title| !title.is_empty()).ok_or("A reminder needs a title.")?;
            let title: String = title.chars().take(200).collect();
            let due = arguments.get("due").and_then(Value::as_str).map(str::trim).filter(|due| !due.is_empty());
            let settings = settings().ok_or("Buntini is still starting.")?;
            if !crate::apps::allowed(&settings, "reminders") {
                return Err("The person hasn't let Buntini use Reminders.".into());
            }
            let what = tf("assistants.reminder", &[("title", &title)]);
            if !ask(client, what).await {
                return Err("The person didn't allow it.".into());
            }
            let helper = APP.get().and_then(|app| app.try_state::<crate::controller::AppState>().map(|state| state.models.apple_helper().to_path_buf())).ok_or("Buntini is still starting.")?;
            let mut args = Map::new();
            args.insert("title".into(), Value::String(title.clone()));
            if let Some(due) = due {
                args.insert("due".into(), Value::String(due.to_string()));
            }
            let receipt = crate::actions::execute_with("add_reminder", &args, &helper).await;
            match receipt.outcome {
                crate::actions::ActionOutcome::Succeeded => Ok(format!("Added the reminder “{title}”.")),
                _ => Err(receipt.error.unwrap_or(receipt.message)),
            }
        }
        "show_progress" => {
            let label: String = arguments.get("label").and_then(Value::as_str).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
            let done = arguments.get("done").and_then(Value::as_bool).unwrap_or(false);
            let progress = arguments.get("progress").and_then(Value::as_f64).filter(|value| value.is_finite()).map(|value| value.clamp(0.0, 1.0));
            {
                let mut all = PROGRESS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                all.retain(|(name, _)| name != client);
                if !done && !label.is_empty() {
                    all.push((client.to_string(), Progress { label, progress, at: Instant::now() }));
                }
            }
            super::refresh();
            Ok(if done { "Removed.".into() } else { "Shown beside the notch.".into() })
        }
        _ => Err(format!("There's no tool named {name}.")),
    }
}

// ----- MCP over the socket -----

/// What one connection says, and Buntini's answer to it. `None` for a notification.
pub async fn respond(message: &Value, client: &mut Option<String>) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            *client = Some(display_name(params.pointer("/clientInfo/name").and_then(Value::as_str).unwrap_or_default()));
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
            let version = if PROTOCOLS.contains(&asked) { asked } else { PROTOCOL };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "buntini", "title": "Buntini", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Buntini is the person's Mac companion. Its tools read today's calendar and what's playing, add reminders the person allows in Buntini's island, and show your progress beside the notch."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let arguments = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            let who = client.clone().unwrap_or_else(|| display_name(""));
            Ok(text(match decisions().get(&who).copied() {
                Some(false) => Err(format!("The person hasn't let {who} use Buntini.")),
                Some(true) => call(&name, &arguments, &who).await,
                None => {
                    let what = tf("assistants.use", &[("name", &who)]);
                    let allowed = ask(&who, what).await;
                    decide(&who, allowed);
                    if allowed { call(&name, &arguments, &who).await } else { Err(format!("The person didn't let {who} use Buntini.")) }
                }
            }))
        }
        _ if id.is_none() => return None,
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    let id = id?;
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

/// Listens on `assistants.sock` in Buntini's data folder.
pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(dir) = crate::agents::socket_dir() else { return };
    let path = socket(dir);
    tauri::async_runtime::spawn(async move {
        let listener = match crate::agents::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Tools for other assistants unavailable: {error}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tauri::async_runtime::spawn(serve(stream));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    });
}

async fn serve(stream: tokio::net::UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut client = None;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > LONGEST || line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => respond(&message, &mut client).await,
            Err(_) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } })),
        };
        if let Some(reply) = reply {
            let mut bytes = reply.to_string().into_bytes();
            bytes.push(b'\n');
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(message: Value, client: &mut Option<String>) -> Option<Value> {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(respond(&message, client))
    }

    #[test]
    fn an_assistant_learns_what_buntini_offers() {
        let mut client = None;
        let reply = run(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","clientInfo":{"name":"claude-code","version":"2.1"}}}), &mut client).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "buntini");
        assert_eq!(client.as_deref(), Some("Claude Code"));
        assert_eq!(run(json!({"jsonrpc":"2.0","method":"notifications/initialized"}), &mut client), None);
        let tools = run(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), &mut client).unwrap();
        let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["today", "now_playing", "add_reminder", "show_progress"]);
        assert_eq!(run(json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}), &mut client).unwrap()["error"]["code"], -32601);
        // An unknown protocol gets Buntini's own.
        let reply = run(json!({"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}), &mut client).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL);
    }

    #[test]
    fn assistants_go_by_their_own_names() {
        assert_eq!(display_name("claude-ai"), "Claude");
        assert_eq!(display_name("cursor-vscode"), "Cursor");
        assert_eq!(display_name("Visual Studio Code"), "VS Code");
        assert_eq!(display_name("my-agent\u{7}"), "my-agent");
        assert_eq!(display_name(""), "An assistant");
        assert_eq!(display_name(&"x".repeat(80)).chars().count(), 40);
    }

    #[test]
    fn today_reads_as_plain_lines() {
        let event: crate::apps::Event = serde_json::from_value(json!({"id":"e","calendar":"Work","start":"2026-11-21 10:00","end":"2026-11-21 10:30","allDay":false,"title":"Standup","location":"Room 4"})).unwrap();
        let reminder: crate::apps::Reminder = serde_json::from_value(json!({"id":"r","title":"Call the bank","due":"2026-11-21 09:00","list":"Home"})).unwrap();
        assert_eq!(today_lines(&[event], &[reminder]), "Event: 10:00–10:30 Standup (Room 4)\nReminder: Call the bank (due 2026-11-21 09:00)");
        assert_eq!(today_lines(&[], &[]), "Nothing on the calendar today and no reminders due.");
    }

    #[test]
    fn a_question_in_the_island_waits_for_allow_or_deny() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            for allowed in [true, false] {
                let asked = tokio::spawn(ask("Claude Code", "Add the reminder “Call the bank”".into()));
                let id = loop {
                    if let Some((id, _)) = waiting().first() {
                        break id.clone();
                    }
                    tokio::task::yield_now().await;
                };
                assert!(id.starts_with(PREFIX));
                answer(&id, allowed);
                assert_eq!(asked.await.unwrap(), allowed);
                assert!(waiting().is_empty());
            }
            // An answer for a card no longer waiting changes nothing.
            answer("assistant.ask.999", true);
        });
    }

    /// An assistant's view, through the same `nc` relay it starts: `cargo test --lib
    /// island::assistants::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        use std::io::{BufRead, Write};
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let dir = tempfile::Builder::new().prefix("ba").tempdir_in("/tmp").unwrap();
        let path = socket(dir.path());
        let listener = runtime.block_on(async { tokio::net::UnixListener::bind(&path) }).unwrap();
        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream));
            }
        });
        let (program, args) = command(dir.path());
        let mut relay = std::process::Command::new(program).args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut input = relay.stdin.take().unwrap();
        let mut output = std::io::BufReader::new(relay.stdout.take().unwrap()).lines();
        let mut say = |message: Value| {
            writeln!(input, "{message}").unwrap();
            input.flush().unwrap();
        };
        say(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude-code","version":"2.1"}}}));
        let first: Value = serde_json::from_str(&output.next().unwrap().unwrap()).unwrap();
        println!("{first}");
        say(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        say(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        let second: Value = serde_json::from_str(&output.next().unwrap().unwrap()).unwrap();
        println!("{}", second["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect::<Vec<_>>().join(", "));
        assert_eq!(first["result"]["serverInfo"]["name"], "buntini");
        assert_eq!(second["result"]["tools"].as_array().unwrap().len(), 4);
        drop(say);
        let _ = relay.kill();
    }

    #[test]
    fn the_command_is_netcat_on_buntini_s_socket() {
        let (program, args) = command(Path::new("/Users/me/Library/Application Support/app.buntini.companion"));
        assert_eq!(program, "/usr/bin/nc");
        assert_eq!(args, ["-U", "/Users/me/Library/Application Support/app.buntini.companion/assistants.sock"]);
    }
}
