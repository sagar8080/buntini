//! Adding Buntini's tools to each assistant on this Mac, from Settings → Apps → Assistants:
//! - Claude (the desktop app): an entry in its `claude_desktop_config.json`, read when it opens;
//! - Claude Code and Codex: their own `mcp add` and `mcp remove` commands;
//! - Cursor and VS Code: their install links, which ask in the app before adding; taken out of
//!   their `mcp.json` the same way.
//!
//! Only Buntini's entry is added or removed; everything else in a file stays as it was, in its
//! order, after a copy of the file is kept in Buntini's data folder.
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{json, Value};

/// The name Buntini goes by in each assistant's list of tool servers.
const NAME: &str = "buntini";

/// An assistant Buntini can be added to, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Claude,
    ClaudeCode,
    Codex,
    Cursor,
    VsCode,
}

pub const TARGETS: [Target; 5] = [Target::Claude, Target::ClaudeCode, Target::Codex, Target::Cursor, Target::VsCode];

impl Target {
    /// The name it goes by, which is also what it calls itself to Buntini (assistants.rs).
    pub fn name(self) -> &'static str {
        match self {
            Target::Claude => "Claude",
            Target::ClaudeCode => "Claude Code",
            Target::Codex => "Codex",
            Target::Cursor => "Cursor",
            Target::VsCode => "VS Code",
        }
    }

    pub fn by_name(name: &str) -> Option<Target> {
        TARGETS.into_iter().find(|target| target.name() == name)
    }

    /// The file that holds its tool servers, and the key they're under.
    fn file(self, home: &Path) -> (PathBuf, &'static str) {
        let support = home.join("Library/Application Support");
        match self {
            Target::Claude => (support.join("Claude/claude_desktop_config.json"), "mcpServers"),
            Target::ClaudeCode => (home.join(".claude.json"), "mcpServers"),
            Target::Codex => (std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex")).join("config.toml"), "mcp_servers"),
            Target::Cursor => (home.join(".cursor/mcp.json"), "mcpServers"),
            Target::VsCode => (support.join("Code/User/mcp.json"), "servers"),
        }
    }

    /// On this Mac.
    pub fn installed(self) -> bool {
        match self {
            Target::Claude => crate::actions::find_app("Claude").is_some(),
            Target::ClaudeCode => crate::cli::installed().iter().any(|id| id == "claude-code"),
            Target::Codex => crate::cli::installed().iter().any(|id| id == "codex"),
            Target::Cursor => crate::actions::find_app("Cursor").is_some(),
            Target::VsCode => crate::actions::find_app("Visual Studio Code").is_some(),
        }
    }

    /// Buntini is in its list of tool servers.
    pub fn added(self, home: &Path) -> bool {
        let (path, key) = self.file(home);
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        if self == Target::Codex {
            return text.lines().any(|line| line.trim() == format!("[{key}.{NAME}]"));
        }
        serde_json::from_str::<Value>(&text).ok().and_then(|value| value.get(key)?.get(NAME).cloned()).is_some()
    }
}

/// The server entry each assistant gets: the relay to Buntini's socket.
fn entry(socket: &Path) -> Value {
    json!({ "command": "/usr/bin/nc", "args": ["-U", socket.to_string_lossy()] })
}

/// `bytes` of a JSON settings file with Buntini's server set under `key`, or taken out.
/// Everything else keeps its place. None when nothing changes.
pub fn edited(bytes: &[u8], key: &str, server: Option<&Value>, name: &str) -> Result<Option<Vec<u8>>, String> {
    use crate::agent_setup::{parse, raw, render};
    let mut doc = parse(bytes, name)?;
    let at = doc.entries.iter().position(|(entry, _)| entry == key);
    let mut servers = match at {
        Some(at) => parse(doc.entries[at].1.get().as_bytes(), name)?,
        None => parse(b"{}", name)?,
    };
    let had = servers.entries.iter().position(|(entry, _)| entry == NAME);
    match (server, had) {
        (Some(server), Some(index)) if serde_json::from_str::<Value>(servers.entries[index].1.get()).ok().as_ref() == Some(server) => return Ok(None),
        (Some(server), Some(index)) => servers.entries[index].1 = raw(server),
        (Some(server), None) => servers.entries.push((NAME.to_string(), raw(server))),
        (None, Some(index)) => {
            servers.entries.remove(index);
        }
        (None, None) => return Ok(None),
    }
    let servers = String::from_utf8(render(&servers, false)).map_err(|error| error.to_string())?;
    let servers = serde_json::value::RawValue::from_string(servers.trim_end().replace('\n', "\n  ")).map_err(|error| error.to_string())?;
    match at {
        Some(at) => doc.entries[at].1 = servers,
        None => doc.entries.push((key.to_string(), servers)),
    }
    Ok(Some(render(&doc, bytes.starts_with(&[0xEF, 0xBB, 0xBF]))))
}

/// Writes `contents` over `path` in one step, after keeping a copy of what was there.
fn write(path: &Path, before: Option<&[u8]>, contents: &[u8], backups: &Path, target: Target) -> Result<(), String> {
    let dir = path.parent().ok_or("That settings file has no folder.")?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    if let Some(before) = before {
        std::fs::create_dir_all(backups).map_err(|error| error.to_string())?;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let _ = std::fs::write(backups.join(format!("{}-{stamp}.json", target.name().to_lowercase().replace(' ', "-"))), before);
    }
    let temp = dir.join(format!(".{NAME}-{}.tmp", std::process::id()));
    std::fs::write(&temp, contents).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        error.to_string()
    })
}

/// Adds Buntini to `target`, or takes it out. `home` stands in for the home folder in tests;
/// `open` opens an install link (Cursor and VS Code ask in the app before adding).
pub async fn set(target: Target, on: bool, home: &Path, data: &Path, open: impl Fn(&str)) -> Result<(), String> {
    let socket = super::assistants::socket(data);
    let backups = data.join("assistant-backups");
    let (path, key) = target.file(home);
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    match (target, on) {
        (Target::ClaudeCode, _) => {
            let args: Vec<String> = if on {
                ["mcp", "add", "--scope", "user", NAME, "--", "/usr/bin/nc", "-U"].map(str::to_string).into_iter().chain([socket.to_string_lossy().into_owned()]).collect()
            } else {
                ["mcp", "remove", "--scope", "user", NAME].map(str::to_string).to_vec()
            };
            crate::cli::run_own("claude-code", &args, Some(home)).await.map(|_| ())
        }
        (Target::Codex, _) => {
            let args: Vec<String> = if on {
                ["mcp", "add", NAME, "--", "/usr/bin/nc", "-U"].map(str::to_string).into_iter().chain([socket.to_string_lossy().into_owned()]).collect()
            } else {
                ["mcp", "remove", NAME].map(str::to_string).to_vec()
            };
            crate::cli::run_own("codex", &args, Some(home)).await.map(|_| ())
        }
        (Target::Cursor, true) => {
            let config = base64::engine::general_purpose::STANDARD.encode(entry(&socket).to_string());
            open(&format!("cursor://anysphere.cursor-deeplink/mcp/install?name={NAME}&config={config}"));
            Ok(())
        }
        (Target::VsCode, true) => {
            let mut server = entry(&socket);
            server["name"] = json!(NAME);
            let encoded: String = server.to_string().bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect();
            open(&format!("vscode:mcp/install?{encoded}"));
            Ok(())
        }
        // Claude's own file, and taking Buntini out of Cursor's and VS Code's.
        (Target::Claude, _) | (Target::Cursor | Target::VsCode, false) => {
            let before = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.to_string()),
            };
            let server = on.then(|| entry(&socket));
            if let Some(after) = edited(before.as_deref().unwrap_or_default(), key, server.as_ref(), &file_name)? {
                write(&path, before.as_deref(), &after, &backups, target)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buntini_s_entry_goes_in_and_out_leaving_the_rest_in_order() {
        let before = b"{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"github\": { \"command\": \"gh-mcp\" }\n  },\n  \"zoom\": 1\n}\n";
        let server = entry(Path::new("/data/assistants.sock"));
        let added = edited(before, "mcpServers", Some(&server), "claude_desktop_config.json").unwrap().unwrap();
        let value: Value = serde_json::from_slice(&added).unwrap();
        assert_eq!(value["mcpServers"]["buntini"], server);
        assert_eq!(value["mcpServers"]["github"]["command"], "gh-mcp");
        let text = String::from_utf8(added.clone()).unwrap();
        assert!(text.find("\"theme\"").unwrap() < text.find("\"mcpServers\"").unwrap() && text.find("\"mcpServers\"").unwrap() < text.find("\"zoom\"").unwrap());
        // Once is enough, and out again leaves the others.
        assert_eq!(edited(&added, "mcpServers", Some(&server), "x").unwrap(), None);
        let removed = edited(&added, "mcpServers", None, "x").unwrap().unwrap();
        let value: Value = serde_json::from_slice(&removed).unwrap();
        assert!(value["mcpServers"].get("buntini").is_none() && value["mcpServers"]["github"].is_object() && value["zoom"] == 1);
        assert_eq!(edited(before, "mcpServers", None, "x").unwrap(), None);
        // A file with none yet, or none at all.
        let fresh: Value = serde_json::from_slice(&edited(b"{\"theme\":\"dark\"}", "servers", Some(&server), "x").unwrap().unwrap()).unwrap();
        assert_eq!((fresh["theme"].as_str(), fresh["servers"]["buntini"]["command"].as_str()), (Some("dark"), Some("/usr/bin/nc")));
        assert!(edited(b"", "mcpServers", Some(&server), "x").unwrap().is_some());
        assert!(edited(b"not json", "mcpServers", Some(&server), "x").is_err());
    }

    #[test]
    fn claude_s_own_file_is_edited_in_a_home_folder() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (path, _) = Target::Claude.file(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"globalShortcut\":\"Alt+Space\"}").unwrap();
        runtime.block_on(set(Target::Claude, true, home.path(), data.path(), |_| {})).unwrap();
        assert!(Target::Claude.added(home.path()));
        assert!(data.path().join("assistant-backups").read_dir().unwrap().count() == 1);
        runtime.block_on(set(Target::Claude, false, home.path(), data.path(), |_| {})).unwrap();
        assert!(!Target::Claude.added(home.path()));
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["globalShortcut"], "Alt+Space");
    }

    #[test]
    fn cursor_and_vs_code_get_their_install_links() {
        let home = tempfile::tempdir().unwrap();
        let data = Path::new("/data");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let opened = std::sync::Mutex::new(Vec::new());
        runtime.block_on(set(Target::Cursor, true, home.path(), data, |link| opened.lock().unwrap().push(link.to_string()))).unwrap();
        runtime.block_on(set(Target::VsCode, true, home.path(), data, |link| opened.lock().unwrap().push(link.to_string()))).unwrap();
        let opened = opened.into_inner().unwrap();
        let config = opened[0].split("config=").nth(1).unwrap();
        let decoded: Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(config).unwrap()).unwrap();
        assert_eq!(decoded, entry(&data.join("assistants.sock")));
        assert!(opened[0].starts_with("cursor://anysphere.cursor-deeplink/mcp/install?name=buntini&config="));
        let encoded = opened[1].strip_prefix("vscode:mcp/install?").unwrap();
        let bytes: Vec<u8> = {
            let raw = encoded.as_bytes();
            let mut out = Vec::new();
            let mut at = 0;
            while at < raw.len() {
                if raw[at] == b'%' {
                    out.push(u8::from_str_radix(std::str::from_utf8(&raw[at + 1..at + 3]).unwrap(), 16).unwrap());
                    at += 3;
                } else {
                    out.push(raw[at]);
                    at += 1;
                }
            }
            out
        };
        let server: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!((server["name"].as_str(), server["command"].as_str()), (Some("buntini"), Some("/usr/bin/nc")));
        assert_eq!(server["args"][1].as_str(), Some("/data/assistants.sock"));
    }

    /// Claude Code's and Codex's own commands, on a home folder of the test's own, so nothing
    /// of the person's changes: `cargo test --lib island::assistant_setup::tests::live --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        for target in [Target::ClaudeCode, Target::Codex] {
            runtime.block_on(set(target, true, home.path(), data.path(), |_| {})).unwrap_or_else(|problem| panic!("{}: {problem}", target.name()));
            let (path, _) = target.file(home.path());
            println!("{}: {}", target.name(), std::fs::read_to_string(&path).unwrap_or_default().lines().filter(|line| line.contains("buntini") || line.contains("nc")).collect::<Vec<_>>().join(" | "));
            assert!(target.added(home.path()), "{} added", target.name());
            runtime.block_on(set(target, false, home.path(), data.path(), |_| {})).unwrap();
            assert!(!target.added(home.path()), "{} removed", target.name());
        }
    }
}
