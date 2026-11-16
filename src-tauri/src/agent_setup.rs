//! Turning Buntini's hooks on and off in each coding agent's own settings:
//! - merged into a settings file's `hooks`: Claude Code (`~/.claude/settings.json`, which
//!   Cursor runs too), Codex (`~/.codex/hooks.json`) and Gemini CLI (`~/.gemini/settings.json`);
//! - a key of Buntini's own in a shared file: Antigravity (`~/.gemini/config/hooks.json`);
//! - a file of Buntini's own: Copilot CLI (`~/.copilot/hooks/buntini.json`) and the OpenCode
//!   and Amp plugins;
//! - a plugin folder of Buntini's own, turned on with the agent's own command: Hermes.
//!
//! Only Buntini's entries are added or removed; everything else in a shared file stays
//! as it was, in its order. The file as it was is copied to Buntini's data folder
//! first. A file that isn't the JSON expected, or that changes while Buntini is
//! writing it, is left untouched.

use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::de::{Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Map, Value};

use crate::agents::{self, Agent};

mod files;

/// The events each agent sends, with the hook's time limit in seconds and whether the
/// agent waits for Buntini's answer. A permission request waits for a click, so it gets
/// the decision time and more. The rest only tell Buntini what is happening: the step
/// ticker, and that a request was settled in the terminal.
fn events(agent: Agent) -> &'static [(&'static str, u64, bool)] {
    match agent {
        Agent::Claude => &[
            ("SessionStart", 5, false),
            ("UserPromptSubmit", 5, false),
            ("PreToolUse", 5, false),
            ("PermissionRequest", 120, true),
            ("PostToolUse", 5, false),
            ("PostToolUseFailure", 5, false),
            ("Stop", 5, false),
            ("StopFailure", 5, false),
            ("SessionEnd", 2, false),
        ],
        Agent::Codex => &[
            ("SessionStart", 5, false),
            ("UserPromptSubmit", 5, false),
            ("PreToolUse", 5, false),
            ("PermissionRequest", 70, true),
            ("PostToolUse", 5, false),
            ("Stop", 5, false),
            ("SessionEnd", 2, false),
        ],
        // Gemini CLI can't be answered from a hook: it only tells Buntini what it does.
        Agent::Gemini => &[
            ("SessionStart", 5, false),
            ("BeforeAgent", 5, false),
            ("BeforeTool", 5, false),
            ("AfterTool", 5, false),
            ("AfterAgent", 5, false),
            ("SessionEnd", 2, false),
        ],
        _ => &[],
    }
}

/// How Buntini's hooks get into each agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Merged,
    Keyed,
    Own,
    Plugin,
}

fn kind(agent: Agent) -> Option<Kind> {
    match agent {
        Agent::Claude | Agent::Codex | Agent::Gemini => Some(Kind::Merged),
        Agent::Antigravity => Some(Kind::Keyed),
        Agent::Copilot | Agent::OpenCode | Agent::Amp => Some(Kind::Own),
        Agent::Hermes => Some(Kind::Plugin),
        // Cursor runs Claude Code's hooks itself.
        Agent::Cursor => None,
    }
}
/// Shown in Codex while its request waits in Buntini.
const CODEX_WAITING: &str = "Waiting for you in Buntini";
/// Copies of the files as they were, kept per agent.
const BACKUPS_KEPT: usize = 10;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgent {
    pub id: String,
    pub name: String,
    /// The agent is set up on this Mac.
    pub available: bool,
    /// Buntini's hooks are in the agent's settings.
    pub connected: bool,
    /// The agent has talked to Buntini at least once.
    pub heard: bool,
    pub problem: String,
    /// Claude Code: its plan's windows come through Buntini's status line wrapper.
    pub usage: Option<bool>,
}

/// The hook: hands the event to Buntini and prints its answer. Without Buntini's
/// socket it does nothing and succeeds. `exec`, so ending the hook ends `nc`,
/// which is how Buntini learns a request was answered in the terminal.
pub fn hook_command(socket: &Path) -> String {
    let path = shell_quote(&socket.to_string_lossy());
    format!("[ -S {path} ] && exec /usr/bin/nc -U {path} 2>/dev/null; exit 0")
}

/// The hook for events that only tell Buntini something. It also sends which terminal the
/// agent runs in, on a line of its own before the event, so the island can bring that
/// terminal forward. Nothing comes back.
pub fn note_command(socket: &Path) -> String {
    note_command_for(socket, "")
}

/// The note hook naming its event, for agents whose payloads don't.
fn note_command_for(socket: &Path, event: &str) -> String {
    let path = shell_quote(&socket.to_string_lossy());
    format!(
        "[ -S {path} ] && {{ printf 'BUNTINI\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \"$TERM_PROGRAM\" \"$__CFBundleIdentifier\" \"$ITERM_SESSION_ID\" \"$CLAUDE_CODE_ENTRYPOINT\" {event}; cat; }} | /usr/bin/nc -U {path} >/dev/null 2>&1; exit 0",
        event = shell_quote(event),
    )
}

/// Antigravity's tool hook: it must always answer, so without Buntini it lets the agent's
/// own rules decide.
fn antigravity_tool_command(socket: &Path) -> String {
    let path = shell_quote(&socket.to_string_lossy());
    format!(
        "if [ -S {path} ]; then {{ printf 'BUNTINI\\t%s\\t%s\\t\\t\\tPreToolUse\\n' \"$TERM_PROGRAM\" \"$__CFBundleIdentifier\"; cat; }} | /usr/bin/nc -U {path} 2>/dev/null || printf '{{\"decision\":\"allow\"}}\\n'; else printf '{{\"decision\":\"allow\"}}\\n'; fi; exit 0"
    )
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A hook command Buntini wrote for this agent, wherever its data folder was.
fn is_ours(command: &str, agent: Agent) -> bool {
    command.contains("/usr/bin/nc -U")
        && command.contains("buntini")
        && command.contains(&format!("/{}.sock'", agent.id()))
}

fn config_dir(agent: Agent) -> Option<PathBuf> {
    let (variable, folder) = match agent {
        Agent::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
        Agent::Codex => ("CODEX_HOME", ".codex"),
        Agent::Copilot => ("COPILOT_HOME", ".copilot"),
        Agent::Gemini => ("", ".gemini"),
        Agent::Antigravity => ("", ".gemini/config"),
        Agent::OpenCode => ("", ".config/opencode"),
        Agent::Amp => ("", ".config/amp"),
        Agent::Hermes => ("", ".hermes"),
        Agent::Cursor => return None,
    };
    match std::env::var_os(variable).filter(|_| !variable.is_empty()) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(folder)),
    }
}

/// The file Buntini edits or owns, inside the agent's folder.
fn file_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude | Agent::Gemini => "settings.json",
        Agent::Copilot => "hooks/buntini.json",
        Agent::OpenCode => "plugins/buntini.js",
        Agent::Amp => "plugins/buntini.ts",
        Agent::Hermes => "plugins/buntini/__init__.py",
        _ => "hooks.json",
    }
}

/// One of Buntini's hook entries for an event.
fn hook_entry(agent: Agent, event: &str, timeout: u64, waits: bool, socket: &Path) -> Value {
    let command = if waits { hook_command(socket) } else { note_command(socket) };
    match agent {
        // Gemini counts in milliseconds and names its hooks.
        Agent::Gemini => json!({ "name": "buntini", "type": "command", "command": command, "timeout": timeout * 1000 }),
        _ => {
            let mut hook = json!({ "type": "command", "command": command, "timeout": timeout });
            if agent == Agent::Codex && event == "PermissionRequest" {
                hook["statusMessage"] = json!(CODEX_WAITING);
            }
            hook
        }
    }
}

/// A JSON object read with its keys in order and each value exactly as written.
struct Doc {
    entries: Vec<(String, Box<RawValue>)>,
}

impl<'de> Deserialize<'de> for Doc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Doc, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Doc;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Doc, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Box<RawValue>>()? {
                    entries.push(entry);
                }
                Ok(Doc { entries })
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

fn parse(bytes: &[u8], name: &str) -> Result<Doc, String> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Doc { entries: Vec::new() });
    }
    serde_json::from_slice(body).map_err(|_| format!("{name} isn’t the JSON Buntini expects, so it was left alone."))
}

fn render(doc: &Doc, bom: bool) -> Vec<u8> {
    let mut out = String::new();
    if bom {
        out.push('\u{feff}');
    }
    if doc.entries.is_empty() {
        out.push_str("{}\n");
        return out.into_bytes();
    }
    out.push_str("{\n");
    for (index, (key, value)) in doc.entries.iter().enumerate() {
        let key = serde_json::to_string(key).unwrap_or_default();
        let comma = if index + 1 < doc.entries.len() { "," } else { "" };
        out.push_str(&format!("  {key}: {}{comma}\n", value.get()));
    }
    out.push_str("}\n");
    out.into_bytes()
}

/// `value` written the way the rest of the file is: two spaces, one level in.
fn raw(value: &Value) -> Box<RawValue> {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into()).replace('\n', "\n  ");
    RawValue::from_string(text).unwrap_or_else(|_| RawValue::from_string("{}".into()).unwrap())
}

/// The file's contents with Buntini's hooks added (for the agent's `socket`) or removed.
/// None when there's nothing to change.
pub(crate) fn edited(bytes: Option<&[u8]>, agent: Agent, socket: Option<&Path>) -> Result<Option<Vec<u8>>, String> {
    let name = file_name(agent);
    if bytes.is_none() && socket.is_none() {
        return Ok(None);
    }
    let original = bytes.unwrap_or_default();
    let bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let mut doc = parse(original, name)?;
    let odd = |what: &str| format!("{what} in {name} isn’t what Buntini expects, so the file was left alone.");
    let at: Vec<usize> = doc.entries.iter().enumerate().filter(|(_, (key, _))| key == "hooks").map(|(index, _)| index).collect();
    if at.len() > 1 {
        return Err(odd("\"hooks\""));
    }
    let mut hooks = match at.first() {
        Some(&index) => match serde_json::from_str::<Value>(doc.entries[index].1.get()) {
            Ok(Value::Object(hooks)) => hooks,
            _ => return Err(odd("\"hooks\"")),
        },
        None => Map::new(),
    };
    let before = hooks.clone();

    // Take out every hook of ours, then put the current ones back.
    for (event, list) in hooks.iter_mut() {
        let Value::Array(groups) = list else {
            if events(agent).iter().any(|(ours, _, _)| ours == event) {
                return Err(odd(&format!("\"hooks\".\"{event}\"")));
            }
            continue;
        };
        groups.retain_mut(|group| {
            let Some(Value::Array(entries)) = group.get_mut("hooks") else { return true };
            let count = entries.len();
            entries.retain(|entry| !entry.get("command").and_then(Value::as_str).is_some_and(|command| is_ours(command, agent)));
            count == entries.len() || !entries.is_empty()
        });
    }
    hooks.retain(|_, list| !matches!(list, Value::Array(groups) if groups.is_empty()));
    if let Some(socket) = socket {
        for (event, timeout, waits) in events(agent) {
            let hook = hook_entry(agent, event, *timeout, *waits, socket);
            match hooks.entry(event.to_string()).or_insert_with(|| json!([])) {
                Value::Array(groups) => groups.push(json!({ "hooks": [hook] })),
                _ => return Err(odd(&format!("\"hooks\".\"{event}\""))),
            }
        }
    }
    if hooks == before && bytes.is_some() {
        return Ok(None);
    }

    match (at.first(), hooks.is_empty()) {
        (Some(&index), true) => {
            doc.entries.remove(index);
        }
        (Some(&index), false) => doc.entries[index].1 = raw(&Value::Object(hooks)),
        (None, false) => doc.entries.push(("hooks".into(), raw(&Value::Object(hooks)))),
        (None, true) => {}
    }
    Ok(Some(render(&doc, bom)))
}

fn connected_in(bytes: &[u8], agent: Agent) -> Result<bool, String> {
    let doc = parse(bytes, file_name(agent))?;
    Ok(doc.entries.iter().filter(|(key, _)| key == "hooks").any(|(_, value)| {
        serde_json::from_str::<Value>(value.get()).ok().is_some_and(|hooks| {
            hooks
                .as_object()
                .into_iter()
                .flat_map(|events| events.values())
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(Value::as_array))
                .flatten()
                .filter_map(|entry| entry.get("command").and_then(Value::as_str))
                .any(|command| is_ours(command, agent))
        })
    }))
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Buntini couldn’t read {}: {error}", path.display())),
    }
}

/// Writes `contents` over `path` in one step, after keeping a copy of `original`.
/// A file kept elsewhere through a link is written where it really is.
fn write(path: &Path, original: Option<&[u8]>, contents: &[u8], backups: &Path, agent: Agent) -> Result<(), String> {
    let target = if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        std::fs::canonicalize(path).map_err(|error| format!("Buntini couldn’t follow {}: {error}", path.display()))?
    } else {
        path.to_path_buf()
    };
    let dir = target.parent().ok_or("That settings file has no folder.")?;
    std::fs::create_dir_all(dir).map_err(|error| format!("Buntini couldn’t create {}: {error}", dir.display()))?;
    if let Some(original) = original {
        keep_backup(backups, agent, original)?;
    }
    let mode = std::fs::metadata(&target).map(|meta| meta.permissions().mode() & 0o777).unwrap_or(0o644);
    let name = target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.buntini-{}.tmp", std::process::id()));
    let result = (|| {
        std::fs::write(&temp, contents).map_err(|error| error.to_string())?;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode)).map_err(|error| error.to_string())?;
        // Changed by someone else since it was read: theirs stands.
        if read(&target)?.as_deref() != original {
            return Err(format!("{} changed while Buntini was saving it. Try again.", file_name(agent)));
        }
        std::fs::rename(&temp, &target).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|error| if error.contains("changed while") { error } else { format!("Buntini couldn’t save {}: {error}", target.display()) })
}

fn keep_backup(backups: &Path, agent: Agent, original: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(backups).map_err(|error| format!("Buntini couldn’t keep a copy of the file: {error}"))?;
    let _ = std::fs::set_permissions(backups, std::fs::Permissions::from_mode(0o700));
    let prefix = format!("{}-{}-", agent.id(), file_name(agent).trim_end_matches(".json").replace('/', "-"));
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    std::fs::write(backups.join(format!("{prefix}{stamp}.json")), original)
        .map_err(|error| format!("Buntini couldn’t keep a copy of the file: {error}"))?;
    let mut kept: Vec<PathBuf> = std::fs::read_dir(backups)
        .map(|entries| entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(&prefix))).collect())
        .unwrap_or_default();
    kept.sort();
    let extra = kept.len().saturating_sub(BACKUPS_KEPT);
    for old in kept.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

fn status_at(agent: Agent, dir: Option<PathBuf>) -> CodingAgent {
    let mut status = CodingAgent {
        id: agent.id().into(),
        name: agent.name().into(),
        available: dir.as_ref().is_some_and(|dir| dir.is_dir()),
        connected: false,
        heard: agents::heard(agent),
        problem: String::new(),
        usage: None,
    };
    let Some(dir) = dir.filter(|_| status.available) else { return status };
    if agent == Agent::Claude {
        status.usage = Some(read(&dir.join(file_name(agent))).ok().flatten().is_some_and(|bytes| status_line_of(&bytes).is_some_and(|line| crate::agents::usage::is_status_line_ours(&line))));
    }
    let connected = match kind(agent) {
        Some(Kind::Merged) => read(&dir.join(file_name(agent))).and_then(|bytes| bytes.map_or(Ok(false), |bytes| connected_in(&bytes, agent))),
        Some(Kind::Keyed) => read(&dir.join(file_name(agent))).and_then(|bytes| bytes.map_or(Ok(false), |bytes| keyed_in(&bytes, agent))),
        Some(Kind::Own | Kind::Plugin) => Ok(read(&dir.join(file_name(agent))).ok().flatten().is_some_and(|bytes| String::from_utf8_lossy(&bytes).contains(&format!("/{}.sock", agent.id())))),
        None => Ok(false),
    };
    match connected {
        Ok(connected) => status.connected = connected,
        Err(problem) => status.problem = problem,
    }
    status
}

fn connect_at(agent: Agent, dir: &Path, socket_dir: &Path, on: bool) -> Result<(), String> {
    let path = dir.join(file_name(agent));
    let socket = agent.socket(socket_dir);
    let backups = socket_dir.join("agent-backups");
    match kind(agent).ok_or("That agent runs Claude Code's hooks.")? {
        Kind::Merged => {
            let original = read(&path)?;
            let Some(contents) = edited(original.as_deref(), agent, on.then_some(socket.as_path()))? else { return Ok(()) };
            write(&path, original.as_deref(), &contents, &backups, agent)
        }
        Kind::Keyed => {
            let original = read(&path)?;
            let Some(contents) = keyed(original.as_deref(), agent, on.then_some(socket.as_path()))? else { return Ok(()) };
            write(&path, original.as_deref(), &contents, &backups, agent)
        }
        Kind::Own | Kind::Plugin => {
            if !on && agent == Agent::Hermes {
                let _ = hermes(&["plugins", "disable", "buntini"]);
            }
            for (name, contents) in own_files(agent, &socket) {
                let target = dir.join(name);
                if on {
                    let original = read(&target)?;
                    if original.as_deref() != Some(contents.as_slice()) {
                        write(&target, original.as_deref(), &contents, &backups, agent)?;
                    }
                } else if std::fs::symlink_metadata(&target).is_ok() {
                    std::fs::remove_file(&target).map_err(|error| format!("Buntini couldn’t remove {}: {error}", target.display()))?;
                }
            }
            if agent == Agent::Hermes {
                if on {
                    hermes(&["plugins", "enable", "buntini"])?;
                } else {
                    let _ = std::fs::remove_dir(dir.join("plugins/buntini"));
                }
            }
            Ok(())
        }
    }
}

/// Copilot CLI's hook file, for the live check.
#[cfg(test)]
pub(crate) fn copilot_hooks(socket: &Path) -> Vec<u8> {
    files::copilot(socket)
}

/// The files Buntini owns in an agent's folder, by path inside it.
fn own_files(agent: Agent, socket: &Path) -> Vec<(&'static str, Vec<u8>)> {
    match agent {
        Agent::Copilot => vec![("hooks/buntini.json", files::copilot(socket))],
        Agent::OpenCode => vec![("plugins/buntini.js", files::opencode(socket))],
        Agent::Amp => vec![("plugins/buntini.ts", files::amp(socket))],
        Agent::Hermes => {
            let (manifest, code) = files::hermes(socket);
            vec![("plugins/buntini/plugin.yaml", manifest), ("plugins/buntini/__init__.py", code)]
        }
        _ => Vec::new(),
    }
}

/// Runs Hermes's own command, which is how its plugins are turned on and off.
fn hermes(args: &[&str]) -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let found = [home.join(".local/bin/hermes"), PathBuf::from("/opt/homebrew/bin/hermes"), PathBuf::from("/usr/local/bin/hermes")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or("In Hermes, run: hermes plugins enable buntini")?;
    let status = std::process::Command::new(found).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("In Hermes, run: hermes plugins enable buntini".into()) }
}

/// Antigravity's hooks file holds named hook sets; Buntini's is `buntini`.
fn antigravity_hooks(socket: &Path) -> Value {
    let handler = |command: String, seconds: u64| json!({ "type": "command", "command": command, "timeout": seconds });
    json!({
        "enabled": true,
        "SessionStart": [handler(note_command_for(socket, "SessionStart"), 5)],
        "PreInvocation": [handler(note_command_for(socket, "PreInvocation"), 5)],
        "PreToolUse": [{ "matcher": "*", "hooks": [handler(antigravity_tool_command(socket), 10)] }],
        "PostToolUse": [{ "matcher": "*", "hooks": [handler(note_command_for(socket, "PostToolUse"), 5)] }],
        "Stop": [handler(note_command_for(socket, "Stop"), 5)],
    })
}

const KEY: &str = "buntini";

/// The shared file with Buntini's hook set added (for `socket`) or removed. None when
/// there's nothing to change.
fn keyed(bytes: Option<&[u8]>, agent: Agent, socket: Option<&Path>) -> Result<Option<Vec<u8>>, String> {
    if bytes.is_none() && socket.is_none() {
        return Ok(None);
    }
    let original = bytes.unwrap_or_default();
    let bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let mut doc = parse(original, file_name(agent))?;
    let before: Vec<String> = doc.entries.iter().map(|(key, value)| format!("{key}={}", value.get())).collect();
    doc.entries.retain(|(key, _)| key != KEY);
    if let Some(socket) = socket {
        doc.entries.push((KEY.into(), raw(&antigravity_hooks(socket))));
    }
    let after: Vec<String> = doc.entries.iter().map(|(key, value)| format!("{key}={}", value.get())).collect();
    if before == after && bytes.is_some() {
        return Ok(None);
    }
    Ok(Some(render(&doc, bom)))
}

fn keyed_in(bytes: &[u8], agent: Agent) -> Result<bool, String> {
    let doc = parse(bytes, file_name(agent))?;
    Ok(doc.entries.iter().any(|(key, value)| key == KEY && value.get().contains(&format!("/{}.sock", agent.id()))))
}

/// The `statusLine` command in Claude Code's settings, if there is one.
fn status_line_of(bytes: &[u8]) -> Option<String> {
    let doc = parse(bytes, "settings.json").ok()?;
    let (_, value) = doc.entries.iter().find(|(key, _)| key == "statusLine")?;
    serde_json::from_str::<Value>(value.get()).ok()?.get("command")?.as_str().map(str::to_string)
}

/// Claude Code's settings with Buntini's status line wrapper put in (around the person's
/// own, kept in `previous`) or taken out (theirs put back). Returns the new file, if it
/// changes, and what to keep as theirs.
fn status_line_edited(bytes: Option<&[u8]>, socket: &Path, on: bool, previous: Option<Value>) -> Result<(Option<Vec<u8>>, Option<Value>), String> {
    let original = bytes.unwrap_or_default();
    let bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let mut doc = parse(original, "settings.json")?;
    let at = doc.entries.iter().position(|(key, _)| key == "statusLine");
    let current = at.and_then(|index| serde_json::from_str::<Value>(doc.entries[index].1.get()).ok());
    let ours = current.as_ref().and_then(|value| value.get("command")).and_then(Value::as_str).is_some_and(crate::agents::usage::is_status_line_ours);
    let theirs = if ours { previous } else { current.clone() };
    let next = if on {
        let command = theirs.as_ref().filter(|value| value.get("type").and_then(Value::as_str) == Some("command")).and_then(|value| value.get("command")).and_then(Value::as_str);
        let mut line = json!({ "type": "command", "command": crate::agents::usage::status_line_command(socket, command) });
        for key in ["padding", "refreshInterval"] {
            if let Some(value) = theirs.as_ref().and_then(|value| value.get(key)) {
                line[key] = value.clone();
            }
        }
        Some(line)
    } else if ours {
        theirs.clone()
    } else {
        return Ok((None, theirs));
    };
    if next == current {
        return Ok((None, theirs));
    }
    match (at, next) {
        (Some(index), Some(value)) => doc.entries[index].1 = raw(&value),
        (Some(index), None) => {
            doc.entries.remove(index);
        }
        (None, Some(value)) => doc.entries.push(("statusLine".into(), raw(&value))),
        (None, None) => return Ok((None, theirs)),
    }
    Ok((Some(render(&doc, bom)), theirs))
}

/// Shows Claude Code's plan windows on the island, or stops, putting the person's own
/// status line back.
pub fn set_plan_usage(on: bool) -> Result<(), String> {
    let socket_dir = agents::socket_dir().ok_or("Buntini is still starting. Try again in a moment.")?;
    let dir = config_dir(Agent::Claude).filter(|dir| dir.is_dir()).ok_or("Claude Code isn’t set up on this Mac.")?;
    let path = dir.join(file_name(Agent::Claude));
    let kept = socket_dir.join("claude-statusline-previous.json");
    let previous = std::fs::read(&kept).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).and_then(|value| value.get("previous").cloned()).filter(|value| !value.is_null());
    let original = read(&path)?;
    let (contents, theirs) = status_line_edited(original.as_deref(), &Agent::Claude.socket(socket_dir), on, previous)?;
    if on {
        std::fs::write(&kept, serde_json::to_vec(&json!({ "previous": theirs })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    }
    if let Some(contents) = contents {
        write(&path, original.as_deref(), &contents, &socket_dir.join("agent-backups"), Agent::Claude)?;
    }
    if !on {
        let _ = std::fs::remove_file(&kept);
    }
    Ok(())
}

/// Brings Buntini's hooks up to date in every agent they are on, so an update that
/// listens to more events doesn't need them turned off and on again.
pub fn refresh() {
    let Some(socket_dir) = agents::socket_dir() else { return };
    for agent in Agent::ALL.into_iter().filter(|agent| kind(*agent).is_some()) {
        let Some(dir) = config_dir(agent).filter(|dir| dir.is_dir()) else { continue };
        if status_at(agent, Some(dir.clone())).connected {
            if let Err(problem) = connect_at(agent, &dir, socket_dir, true) {
                eprintln!("{}'s hooks weren't brought up to date: {problem}", agent.name());
            }
        }
    }
}

#[tauri::command]
pub fn coding_agents() -> Vec<CodingAgent> {
    Agent::ALL.into_iter().filter(|agent| kind(*agent).is_some()).map(|agent| status_at(agent, config_dir(agent))).collect()
}

#[tauri::command]
pub fn connect_coding_agent(id: String, on: bool) -> Result<CodingAgent, String> {
    let agent = Agent::from_id(&id).ok_or("Unknown coding agent.")?;
    let socket_dir = agents::socket_dir().ok_or("Buntini is still starting. Try again in a moment.")?;
    let dir = config_dir(agent).filter(|dir| dir.is_dir()).ok_or_else(|| format!("{} isn’t set up on this Mac.", agent.name()))?;
    connect_at(agent, &dir, socket_dir, on)?;
    let status = status_at(agent, Some(dir));
    if status.connected != on {
        return Err(format!("{} didn’t take the change. Try again.", agent.name()));
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockets() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("buntini").tempdir().unwrap()
    }

    fn installed(bytes: Option<&[u8]>, agent: Agent, socket_dir: &Path) -> Vec<u8> {
        edited(bytes, agent, Some(&agent.socket(socket_dir))).unwrap().unwrap()
    }

    #[test]
    fn hooks_go_in_beside_everything_else_and_come_out_exactly() {
        let socket_dir = sockets();
        let original = br#"{
  "model": "opus",
  "permissions": {
    "allow": ["Bash(npm test)"],
    "deny": []
  },
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "~/bin/guard.sh" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "say done" }] }
    ]
  },
  "theme": "dark"
}
"#;
        let after = installed(Some(original), Agent::Claude, socket_dir.path());
        let text = String::from_utf8(after.clone()).unwrap();
        // The rest of the file, in its order and as it was written.
        assert!(text.starts_with("{\n  \"model\": \"opus\",\n  \"permissions\": {\n    \"allow\": [\"Bash(npm test)\"],\n    \"deny\": []\n  },\n  \"hooks\": {"));
        assert!(text.ends_with("  \"theme\": \"dark\"\n}\n"));
        let value: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "~/bin/guard.sh");
        assert_eq!(value["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        for (event, timeout, waits) in events(Agent::Claude) {
            let ours = value["hooks"][event].as_array().unwrap().last().unwrap();
            let command = ours["hooks"][0]["command"].as_str().unwrap();
            assert!(is_ours(command, Agent::Claude), "{event}");
            assert_eq!(command.contains("exec /usr/bin/nc"), *waits, "only a request waits for an answer: {event}");
            assert_eq!(ours["hooks"][0]["timeout"], *timeout);
        }
        assert!(connected_in(&after, Agent::Claude).unwrap());
        assert!(!connected_in(&after, Agent::Codex).unwrap());

        // Again: nothing to change, still one of ours per event.
        assert!(edited(Some(&after), Agent::Claude, Some(&Agent::Claude.socket(socket_dir.path()))).unwrap().is_none());
        assert_eq!(value["hooks"]["Stop"].as_array().unwrap().len(), 2);
        let twice = after;

        // Off: the file reads as it did, values and all.
        let removed = edited(Some(&twice), Agent::Claude, None).unwrap().unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&removed).unwrap(), serde_json::from_slice::<Value>(original).unwrap());
        assert!(!connected_in(&removed, Agent::Claude).unwrap());
        assert!(edited(Some(&removed), Agent::Claude, None).unwrap().is_none());
    }

    #[test]
    fn a_file_with_no_hooks_gets_them_and_loses_the_key_again() {
        let socket_dir = sockets();
        let original = b"{\n  \"model\": \"sonnet\"\n}\n";
        let after = installed(Some(original), Agent::Claude, socket_dir.path());
        let removed = edited(Some(&after), Agent::Claude, None).unwrap().unwrap();
        assert_eq!(removed, original);
        // No file at all: Codex gets one of its own, with the waiting message.
        let fresh = installed(None, Agent::Codex, socket_dir.path());
        let value: Value = serde_json::from_slice(&fresh).unwrap();
        assert_eq!(value["hooks"]["PermissionRequest"][0]["hooks"][0]["statusMessage"], CODEX_WAITING);
        assert!(value["hooks"]["Stop"][0]["hooks"][0].get("statusMessage").is_none());
        assert!(edited(None, Agent::Codex, None).unwrap().is_none());
        // Claude's hooks and Codex's never touch each other.
        let both = installed(Some(&installed(None, Agent::Claude, socket_dir.path())), Agent::Codex, socket_dir.path());
        let only_claude = edited(Some(&both), Agent::Codex, None).unwrap().unwrap();
        assert!(connected_in(&only_claude, Agent::Claude).unwrap());
        assert!(!connected_in(&only_claude, Agent::Codex).unwrap());
    }

    #[test]
    fn a_hook_sharing_a_group_with_ours_stays() {
        let socket_dir = sockets();
        let ours = hook_command(&Agent::Claude.socket(socket_dir.path()));
        let shared = json!({ "hooks": { "Stop": [{ "hooks": [
            { "type": "command", "command": ours },
            { "type": "command", "command": "say done" }
        ] }] } });
        let removed = edited(Some(shared.to_string().as_bytes()), Agent::Claude, None).unwrap().unwrap();
        let value: Value = serde_json::from_slice(&removed).unwrap();
        assert_eq!(value["hooks"]["Stop"][0]["hooks"], json!([{ "type": "command", "command": "say done" }]));
    }

    #[test]
    fn what_isnt_understood_is_left_alone() {
        let socket_dir = sockets();
        let socket = Agent::Claude.socket(socket_dir.path());
        let command = socket.as_path();
        for odd in [&b"{ broken"[..], b"[1, 2]", br#"{"hooks": "x"}"#, br#"{"hooks": {"Stop": {"not": "a list"}}}"#, br#"{"hooks": {}, "hooks": {}}"#] {
            assert!(edited(Some(odd), Agent::Claude, Some(command)).is_err(), "{}", String::from_utf8_lossy(odd));
        }
        // An event Buntini doesn't use stays as it is, whatever its shape.
        let kept = br#"{"hooks": {"Custom": "anything"}}"#;
        let after = edited(Some(kept), Agent::Claude, Some(command)).unwrap().unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&after).unwrap()["hooks"]["Custom"], "anything");
        // A byte order mark survives.
        let marked = [&[0xEF, 0xBB, 0xBF][..], b"{\"model\": \"x\"}"].concat();
        assert!(edited(Some(&marked), Agent::Claude, Some(command)).unwrap().unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn the_hook_command_quotes_its_path() {
        let command = hook_command(Path::new("/Users/o'neil/Library/Application Support/app.buntini.companion/claude.sock"));
        assert_eq!(command, r"[ -S '/Users/o'\''neil/Library/Application Support/app.buntini.companion/claude.sock' ] && exec /usr/bin/nc -U '/Users/o'\''neil/Library/Application Support/app.buntini.companion/claude.sock' 2>/dev/null; exit 0");
        assert!(is_ours(&command, Agent::Claude));
        assert!(!is_ours(&command, Agent::Codex));
        assert!(!is_ours("~/bin/guard.sh", Agent::Claude));
        let note = note_command(Path::new("/tmp/buntini-x/claude.sock"));
        assert!(is_ours(&note, Agent::Claude));
        assert!(note.ends_with(">/dev/null 2>&1; exit 0"));
    }

    #[test]
    fn saving_keeps_a_copy_follows_links_and_never_overwrites_a_newer_file() {
        let home = tempfile::tempdir().unwrap();
        let socket_dir = sockets();
        let claude = home.path().join(".claude");
        let dotfiles = home.path().join("dotfiles");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("settings.json");
        std::fs::write(&real, b"{\"model\": \"opus\"}\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, claude.join("settings.json")).unwrap();

        connect_at(Agent::Claude, &claude, socket_dir.path(), true).unwrap();
        // The link is still a link; the file it points to has the hooks and its mode.
        assert!(std::fs::symlink_metadata(claude.join("settings.json")).unwrap().file_type().is_symlink());
        assert!(connected_in(&std::fs::read(&real).unwrap(), Agent::Claude).unwrap());
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
        let status = status_at(Agent::Claude, Some(claude.clone()));
        assert!(status.available && status.connected && status.problem.is_empty());
        let backups: Vec<_> = std::fs::read_dir(socket_dir.path().join("agent-backups")).unwrap().filter_map(Result::ok).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"{\"model\": \"opus\"}\n");

        connect_at(Agent::Claude, &claude, socket_dir.path(), false).unwrap();
        assert_eq!(std::fs::read(&real).unwrap(), b"{\n  \"model\": \"opus\"\n}\n");
        assert!(!status_at(Agent::Claude, Some(claude.clone())).connected);

        // A file changed between reading and writing is theirs.
        let path = claude.join("settings.json");
        let before = std::fs::read(&path).unwrap();
        std::fs::write(&real, b"{\"model\": \"someone else\"}").unwrap();
        let error = write(&path, Some(&before), b"{}", &socket_dir.path().join("agent-backups"), Agent::Claude).unwrap_err();
        assert!(error.contains("changed while"), "{error}");
        assert_eq!(std::fs::read(&real).unwrap(), b"{\"model\": \"someone else\"}");

        // Unreadable JSON shows as a problem, and stays as it is.
        std::fs::write(&real, b"{ broken").unwrap();
        let status = status_at(Agent::Claude, Some(claude.clone()));
        assert!(!status.connected && !status.problem.is_empty());
        assert!(connect_at(Agent::Claude, &claude, socket_dir.path(), true).is_err());
        assert_eq!(std::fs::read(&real).unwrap(), b"{ broken");
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&dotfiles).unwrap().count(), 1);
    }

    #[test]
    fn antigravity_gets_a_hook_set_of_its_own_beside_the_others() {
        let socket_dir = sockets();
        let socket = Agent::Antigravity.socket(socket_dir.path());
        let theirs = b"{\n  \"guard\": {\"enabled\": true}\n}\n";
        let after = keyed(Some(theirs), Agent::Antigravity, Some(&socket)).unwrap().unwrap();
        let value: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(value["guard"], json!({ "enabled": true }));
        assert_eq!(value["buntini"]["enabled"], true);
        let tool = value["buntini"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(tool.contains(r#"printf '{"decision":"allow"}\n'"#), "{tool}");
        assert!(keyed_in(&after, Agent::Antigravity).unwrap());
        assert!(keyed(Some(&after), Agent::Antigravity, Some(&socket)).unwrap().is_none());
        let removed = keyed(Some(&after), Agent::Antigravity, None).unwrap().unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&removed).unwrap(), serde_json::from_slice::<Value>(theirs).unwrap());
    }

    /// Antigravity's tool hook always answers: with Buntini gone, its own rules decide.
    #[test]
    fn the_antigravity_hook_answers_without_buntini() {
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(antigravity_tool_command(Path::new("/tmp/no-buntini-here/antigravity.sock")))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), r#"{"decision":"allow"}"#);
    }

    #[test]
    fn own_files_come_and_go_whole() {
        let home = tempfile::tempdir().unwrap();
        let socket_dir = sockets();
        let copilot = home.path().join(".copilot");
        std::fs::create_dir_all(&copilot).unwrap();
        assert!(!status_at(Agent::Copilot, Some(copilot.clone())).connected);
        connect_at(Agent::Copilot, &copilot, socket_dir.path(), true).unwrap();
        assert!(status_at(Agent::Copilot, Some(copilot.clone())).connected);
        assert!(copilot.join("hooks/buntini.json").is_file());
        connect_at(Agent::Copilot, &copilot, socket_dir.path(), false).unwrap();
        assert!(!copilot.join("hooks/buntini.json").exists());
        assert!(!status_at(Agent::Copilot, Some(copilot)).connected);
        // Gemini merges like Claude, in milliseconds and by name.
        let after = installed(Some(b"{\"theme\": \"x\"}"), Agent::Gemini, socket_dir.path());
        let value: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(value["hooks"]["BeforeTool"][0]["hooks"][0]["timeout"], 5000);
        assert_eq!(value["hooks"]["BeforeTool"][0]["hooks"][0]["name"], "buntini");
        assert_eq!(value["theme"], "x");
    }

    #[test]
    fn the_status_line_wrapper_keeps_and_gives_back_the_person_s_own() {
        let socket_dir = sockets();
        let socket = Agent::Claude.socket(socket_dir.path());
        let original = br#"{"model": "opus", "statusLine": {"type": "command", "command": "~/bin/line.sh", "padding": 1}}"#;
        let (on, theirs) = status_line_edited(Some(original), &socket, true, None).unwrap();
        let on = on.unwrap();
        let line = status_line_of(&on).unwrap();
        assert!(crate::agents::usage::is_status_line_ours(&line));
        assert!(line.contains("'~/bin/line.sh'"), "{line}");
        assert_eq!(serde_json::from_slice::<Value>(&on).unwrap()["statusLine"]["padding"], 1);
        assert_eq!(theirs.as_ref().unwrap()["command"], "~/bin/line.sh");
        // Again: nothing changes.
        assert!(status_line_edited(Some(&on), &socket, true, theirs.clone()).unwrap().0.is_none());
        // Off: theirs comes back exactly.
        let off = status_line_edited(Some(&on), &socket, false, theirs).unwrap().0.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&off).unwrap(), serde_json::from_slice::<Value>(original).unwrap());
        // None of their own: the key comes and goes.
        let bare = br#"{"model": "opus"}"#;
        let on = status_line_edited(Some(bare), &socket, true, None).unwrap().0.unwrap();
        let off = status_line_edited(Some(&on), &socket, false, None).unwrap().0.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&off).unwrap(), serde_json::from_slice::<Value>(bare).unwrap());
        // Someone else's status line is never taken out.
        assert!(status_line_edited(Some(original), &socket, false, None).unwrap().0.is_none());
    }

    #[test]
    fn an_agent_not_set_up_here_shows_as_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let status = status_at(Agent::Codex, Some(home.path().join(".codex")));
        assert!(!status.available && !status.connected);
    }
}
