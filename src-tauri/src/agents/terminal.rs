//! The terminal or editor an agent session runs in: bringing it forward, starting a new
//! session there, and handing it a dropped file's path. Apps are opened through Launch
//! Services (`open -b`), since an app that isn't in front can't activate another one.
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::session::Host;
use super::{hub, Agent};

/// Terminals and editors, by `TERM_PROGRAM` and bundle id. `.command` scripts open in the
/// ones marked `scripts`.
struct App {
    term_program: &'static str,
    bundle: &'static str,
    scripts: bool,
}

const APPS: &[App] = &[
    App { term_program: "Apple_Terminal", bundle: "com.apple.Terminal", scripts: true },
    App { term_program: "iTerm.app", bundle: "com.googlecode.iterm2", scripts: true },
    App { term_program: "WarpTerminal", bundle: "dev.warp.Warp-Stable", scripts: false },
    App { term_program: "ghostty", bundle: "com.mitchellh.ghostty", scripts: false },
    App { term_program: "kitty", bundle: "net.kovidgoyal.kitty", scripts: false },
    App { term_program: "WezTerm", bundle: "com.github.wez.wezterm", scripts: false },
    App { term_program: "Hyper", bundle: "co.zeit.hyper", scripts: false },
    App { term_program: "Tabby", bundle: "org.tabby", scripts: false },
    App { term_program: "zed", bundle: "dev.zed.Zed", scripts: false },
    App { term_program: "", bundle: "org.alacritty", scripts: false },
    App { term_program: "", bundle: "com.todesktop.230313mzl4w4u92", scripts: false },
    App { term_program: "vscode", bundle: "com.microsoft.VSCode", scripts: false },
    App { term_program: "", bundle: "com.exafunction.windsurf", scripts: false },
    App { term_program: "", bundle: "com.anthropic.claudefordesktop", scripts: false },
];

const CLAUDE_APP: &str = "com.anthropic.claudefordesktop";

/// The app to bring forward for a session: the one its hook ran in.
pub fn app_for(host: &Host) -> Option<&'static str> {
    if host.entrypoint == "claude-desktop" {
        return Some(CLAUDE_APP);
    }
    // The bundle id is exact (VS Code and Cursor share `TERM_PROGRAM=vscode`).
    APPS.iter()
        .find(|app| !host.bundle.is_empty() && app.bundle.eq_ignore_ascii_case(&host.bundle))
        .or_else(|| APPS.iter().find(|app| !app.term_program.is_empty() && app.term_program == host.term_program))
        .map(|app| app.bundle)
}

fn valid_bundle(bundle: &str) -> bool {
    !bundle.is_empty() && bundle.len() < 200 && bundle.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn open_app(bundle: &str, file: Option<&Path>) -> Result<(), String> {
    if !valid_bundle(bundle) {
        return Err("That app can't be opened.".into());
    }
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg("-b").arg(bundle);
    if let Some(file) = file {
        command.arg(file);
    }
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("That app isn't on this Mac.".into()) }
}

/// Brings forward the terminal of the session behind an island pill (`agent:<id>:<session>`).
pub fn open_for_pill(pill: &str) -> Result<(), String> {
    let mut parts = pill.splitn(3, ':');
    let (Some("agent"), Some(agent), Some(session)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("That isn't a session.".into());
    };
    let host = {
        let hub = hub();
        hub.sessions.iter().find(|((kind, id), _)| kind.id() == agent && id == session).map(|(_, session)| session.host.clone())
    };
    let host = host.ok_or("That session has gone.")?;
    open_app(app_for(&host).ok_or("Buntini doesn't know that session's terminal.")?, None)
}

/// A single-quoted shell word.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The script that starts a session: it removes itself, goes to the project and runs the
/// agent with the prompt from an environment variable, so the prompt is never typed or
/// read as shell. The shell stays open after the agent ends.
pub fn launch_script(agent: Agent, project: &Path, prompt: &str) -> Result<String, String> {
    let program = match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        _ => return Err(format!("{} can't be started from Buntini yet.", agent.name())),
    };
    let run = if prompt.trim().is_empty() { program.to_string() } else { format!("{program} \"$BUNTINI_PROMPT\"") };
    Ok(format!(
        "#!/bin/sh\nrm -f \"$0\"\ncd {} || exit 1\nexport BUNTINI_PROMPT={}\nexec \"${{SHELL:-/bin/zsh}}\" -lic {}\n",
        quote(&project.to_string_lossy()),
        quote(prompt),
        quote(&format!("{run}; exec \"${{SHELL:-/bin/zsh}}\" -l")),
    ))
}

/// Starts a session of `agent` in `project`, in the terminal used most recently, or Terminal.
pub fn start(agent: Agent, project: &Path, prompt: &str, scripts_dir: &Path) -> Result<(), String> {
    if !project.is_dir() {
        return Err("That project folder isn't there.".into());
    }
    if prompt.chars().count() > 8_000 {
        return Err("That prompt is too long to start a session with.".into());
    }
    let script = launch_script(agent, project, prompt)?;
    std::fs::create_dir_all(scripts_dir).map_err(|error| error.to_string())?;
    let _ = std::fs::set_permissions(scripts_dir, std::fs::Permissions::from_mode(0o700));
    let path: PathBuf = scripts_dir.join(format!("buntini-{}-{}.command", agent.id(), uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|error| error.to_string())?;
    file.write_all(script.as_bytes()).map_err(|error| error.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(|error| error.to_string())?;
    let terminal = last_terminal().filter(|bundle| APPS.iter().any(|app| app.bundle == *bundle && app.scripts)).unwrap_or("com.apple.Terminal");
    open_app(terminal, Some(&path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&path);
    })
}

/// The terminal the most recent session ran in.
fn last_terminal() -> Option<&'static str> {
    let hub = hub();
    let mut sessions: Vec<_> = hub.sessions.values().collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_ms));
    sessions.into_iter().find_map(|session| app_for(&session.host))
}

/// Projects agents worked in lately, newest first.
pub fn recent_projects() -> Vec<String> {
    let hub = hub();
    let mut sessions: Vec<_> = hub.sessions.values().filter(|session| !session.cwd.is_empty()).collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_ms));
    let mut seen = Vec::new();
    for session in sessions {
        if !seen.contains(&session.cwd) {
            seen.push(session.cwd.clone());
        }
    }
    seen
}

/// A path written the way dragging it into a terminal writes it.
pub fn escape_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_whitespace() || "\\'\"`$&|;<>()[]{}*?!#~=%^,".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Pastes `paths` into the session's terminal: its app comes forward and gets the paths as
/// typed by a drag. The clipboard gets back what it held.
pub fn drop_files(pill: &str, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > 20 || paths.iter().any(|path| !Path::new(path).exists()) {
        return Err("Those files aren't there.".into());
    }
    let text = paths.iter().map(|path| escape_path(path)).collect::<Vec<_>>().join(" ") + " ";
    open_for_pill(pill)?;
    #[cfg(target_os = "macos")]
    paste::paste(&text)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod paste {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation};
    use objc2_foundation::{NSData, NSString};

    /// The V key on an ANSI keyboard.
    const KEY_V: u16 = 9;

    pub fn paste(text: &str) -> Result<(), String> {
        if !crate::ax::trusted(false) {
            return Err("Buntini needs Accessibility to paste there.".into());
        }
        let board = NSPasteboard::generalPasteboard();
        // Everything the clipboard held, to put back.
        let saved: Vec<(String, Vec<u8>)> = board
            .types()
            .map(|types| types.iter().filter_map(|kind| board.dataForType(&kind).map(|data| (kind.to_string(), data.to_vec()))).collect())
            .unwrap_or_default();
        board.clearContents();
        board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
        // Clipboard managers leave transient items out of their history.
        board.setData_forType(Some(&NSData::new()), &NSString::from_str("org.nspasteboard.TransientType"));
        let ours = board.changeCount();
        // Give the app a moment to come forward.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(source.as_deref(), KEY_V, down).ok_or("The paste couldn't be sent.")?;
            CGEvent::set_flags(Some(&event), CGEventFlags::MaskCommand);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let board = NSPasteboard::generalPasteboard();
            // Only if nothing else was copied meanwhile.
            if board.changeCount() == ours {
                board.clearContents();
                for (kind, data) in &saved {
                    board.setData_forType(Some(&NSData::with_bytes(data)), &NSString::from_str(kind));
                }
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_s_own_app_comes_forward() {
        let host = |term: &str, bundle: &str, entry: &str| Host { term_program: term.into(), bundle: bundle.into(), iterm_session: String::new(), entrypoint: entry.into() };
        assert_eq!(app_for(&host("Apple_Terminal", "", "")), Some("com.apple.Terminal"));
        // Cursor's terminal says vscode; its bundle id says Cursor.
        assert_eq!(app_for(&host("vscode", "com.todesktop.230313mzl4w4u92", "")), Some("com.todesktop.230313mzl4w4u92"));
        assert_eq!(app_for(&host("vscode", "", "")), Some("com.microsoft.VSCode"));
        assert_eq!(app_for(&host("", "", "claude-desktop")), Some(CLAUDE_APP));
        assert_eq!(app_for(&host("tmux", "", "")), None);
        assert!(!valid_bundle("com.apple.Terminal; rm -rf ~"));
    }

    #[test]
    fn the_prompt_never_becomes_shell() {
        let script = launch_script(Agent::Claude, Path::new("/Users/me/it's here"), "say $(whoami) and 'quote' `x`").unwrap();
        assert!(script.contains("cd '/Users/me/it'\\''s here' || exit 1"));
        assert!(script.contains("export BUNTINI_PROMPT='say $(whoami) and '\\''quote'\\'' `x`'"));
        assert!(script.contains("claude \"$BUNTINI_PROMPT\""));
        // Run it through sh with an echo standing in for the agent: the prompt arrives intact.
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude");
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s' \"$1\" > \"$OUT\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = dir.path().join("out");
        let run = script.replace("exec \"${SHELL:-/bin/zsh}\" -lic", "exec /bin/sh -c").replace("; exec \"${SHELL:-/bin/zsh}\" -l", "");
        let path = dir.path().join("start.command");
        std::fs::write(&path, run.replace(&format!("cd {} ", quote("/Users/me/it's here")), &format!("cd {} ", quote(&dir.path().to_string_lossy())))).unwrap();
        let status = std::process::Command::new("/bin/sh")
            .arg(&path)
            .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
            .env("OUT", &out)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "say $(whoami) and 'quote' `x`");
        assert!(!path.exists(), "the script removes itself");
    }

    #[test]
    fn dropped_paths_read_as_a_drag_writes_them() {
        assert_eq!(escape_path("/Users/me/My File (1).png"), "/Users/me/My\\ File\\ \\(1\\).png");
        assert_eq!(escape_path("/tmp/plain.txt"), "/tmp/plain.txt");
        assert!(launch_script(Agent::Gemini, Path::new("/tmp"), "x").is_err());
    }
}
