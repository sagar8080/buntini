//! Turning Buntini's hooks on and off in Claude Code's and Codex's own settings
//! (`~/.claude/settings.json`, `~/.codex/hooks.json`).
//!
//! Only Buntini's entries are added or removed; everything else in the file stays
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

/// The events each agent sends, with the hook's time limit in seconds. A
/// permission request waits for a click, so it gets the decision time and more.
/// A tool that ran tells Buntini its request was settled, in the terminal or here.
fn events(agent: Agent) -> &'static [(&'static str, u64)] {
    match agent {
        Agent::Claude => &[("UserPromptSubmit", 5), ("PermissionRequest", 120), ("PostToolUse", 5), ("PostToolUseFailure", 5), ("Stop", 5)],
        Agent::Codex => &[("UserPromptSubmit", 5), ("PermissionRequest", 70), ("PostToolUse", 5), ("Stop", 5)],
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
}

/// The hook: hands the event to Buntini and prints its answer. Without Buntini's
/// socket it does nothing and succeeds. `exec`, so ending the hook ends `nc`,
/// which is how Buntini learns a request was answered in the terminal.
pub fn hook_command(socket: &Path) -> String {
    let path = shell_quote(&socket.to_string_lossy());
    format!("[ -S {path} ] && exec /usr/bin/nc -U {path} 2>/dev/null; exit 0")
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
    };
    match std::env::var_os(variable) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(folder)),
    }
}

fn file_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "settings.json",
        Agent::Codex => "hooks.json",
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

/// The file's contents with Buntini's hooks added (`command` set) or removed.
/// None when there's nothing to change.
pub(crate) fn edited(bytes: Option<&[u8]>, agent: Agent, command: Option<&str>) -> Result<Option<Vec<u8>>, String> {
    let name = file_name(agent);
    if bytes.is_none() && command.is_none() {
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
            if events(agent).iter().any(|(ours, _)| ours == event) {
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
    if let Some(command) = command {
        for (event, timeout) in events(agent) {
            let mut hook = json!({ "type": "command", "command": command, "timeout": timeout });
            if agent == Agent::Codex && *event == "PermissionRequest" {
                hook["statusMessage"] = json!(CODEX_WAITING);
            }
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
    let temp = dir.join(format!(".{}.buntini-{}.tmp", file_name(agent), std::process::id()));
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
    let prefix = format!("{}-{}-", agent.id(), file_name(agent).trim_end_matches(".json"));
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
    };
    if let Some(dir) = dir.filter(|_| status.available) {
        match read(&dir.join(file_name(agent))).and_then(|bytes| bytes.map_or(Ok(false), |bytes| connected_in(&bytes, agent))) {
            Ok(connected) => status.connected = connected,
            Err(problem) => status.problem = problem,
        }
    }
    status
}

fn connect_at(agent: Agent, dir: &Path, socket_dir: &Path, on: bool) -> Result<(), String> {
    let path = dir.join(file_name(agent));
    let original = read(&path)?;
    let command = hook_command(&agent.socket(socket_dir));
    let Some(contents) = edited(original.as_deref(), agent, on.then_some(command.as_str()))? else {
        return Ok(());
    };
    write(&path, original.as_deref(), &contents, &socket_dir.join("agent-backups"), agent)
}

#[tauri::command]
pub fn coding_agents() -> Vec<CodingAgent> {
    Agent::ALL.into_iter().map(|agent| status_at(agent, config_dir(agent))).collect()
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
        edited(bytes, agent, Some(&hook_command(&agent.socket(socket_dir)))).unwrap().unwrap()
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
        for (event, timeout) in events(Agent::Claude) {
            let ours = value["hooks"][event].as_array().unwrap().last().unwrap();
            assert!(is_ours(ours["hooks"][0]["command"].as_str().unwrap(), Agent::Claude), "{event}");
            assert_eq!(ours["hooks"][0]["timeout"], *timeout);
        }
        assert!(connected_in(&after, Agent::Claude).unwrap());
        assert!(!connected_in(&after, Agent::Codex).unwrap());

        // Again: nothing to change, still one of ours per event.
        assert!(edited(Some(&after), Agent::Claude, Some(&hook_command(&Agent::Claude.socket(socket_dir.path())))).unwrap().is_none());
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
        let command = hook_command(&Agent::Claude.socket(socket_dir.path()));
        for odd in [&b"{ broken"[..], b"[1, 2]", br#"{"hooks": "x"}"#, br#"{"hooks": {"Stop": {"not": "a list"}}}"#, br#"{"hooks": {}, "hooks": {}}"#] {
            assert!(edited(Some(odd), Agent::Claude, Some(&command)).is_err(), "{}", String::from_utf8_lossy(odd));
        }
        // An event Buntini doesn't use stays as it is, whatever its shape.
        let kept = br#"{"hooks": {"Custom": "anything"}}"#;
        let after = edited(Some(kept), Agent::Claude, Some(&command)).unwrap().unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&after).unwrap()["hooks"]["Custom"], "anything");
        // A byte order mark survives.
        let marked = [&[0xEF, 0xBB, 0xBF][..], b"{\"model\": \"x\"}"].concat();
        assert!(edited(Some(&marked), Agent::Claude, Some(&command)).unwrap().unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn the_hook_command_quotes_its_path() {
        let command = hook_command(Path::new("/Users/o'neil/Library/Application Support/app.buntini.companion/claude.sock"));
        assert_eq!(command, r"[ -S '/Users/o'\''neil/Library/Application Support/app.buntini.companion/claude.sock' ] && exec /usr/bin/nc -U '/Users/o'\''neil/Library/Application Support/app.buntini.companion/claude.sock' 2>/dev/null; exit 0");
        assert!(is_ours(&command, Agent::Claude));
        assert!(!is_ours(&command, Agent::Codex));
        assert!(!is_ours("~/bin/guard.sh", Agent::Claude));
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
    fn an_agent_not_set_up_here_shows_as_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let status = status_at(Agent::Codex, Some(home.path().join(".codex")));
        assert!(!status.available && !status.connected);
    }
}
