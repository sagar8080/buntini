//! The terminal or editor an agent session runs in: bringing it forward, starting a new
//! session there, and handing it a dropped file's path. Apps are opened through Launch
//! Services (`open -b`), since an app that isn't in front can't activate another one.
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::session::Host;
use super::{hub, Agent};

/// Terminals and editors, by `TERM_PROGRAM` and bundle id. `.command` scripts open in the
/// ones marked `scripts`. A dropped file's path is pasted only into a `terminal`: an editor
/// may have a source file in front instead of its terminal.
struct App {
    term_program: &'static str,
    bundle: &'static str,
    scripts: bool,
    terminal: bool,
}

const APPS: &[App] = &[
    App { term_program: "Apple_Terminal", bundle: "com.apple.Terminal", scripts: true, terminal: true },
    App { term_program: "iTerm.app", bundle: "com.googlecode.iterm2", scripts: true, terminal: true },
    App { term_program: "WarpTerminal", bundle: "dev.warp.Warp-Stable", scripts: false, terminal: true },
    App { term_program: "ghostty", bundle: "com.mitchellh.ghostty", scripts: false, terminal: true },
    App { term_program: "kitty", bundle: "net.kovidgoyal.kitty", scripts: false, terminal: true },
    App { term_program: "WezTerm", bundle: "com.github.wez.wezterm", scripts: false, terminal: true },
    App { term_program: "Hyper", bundle: "co.zeit.hyper", scripts: false, terminal: true },
    App { term_program: "Tabby", bundle: "org.tabby", scripts: false, terminal: true },
    App { term_program: "zed", bundle: "dev.zed.Zed", scripts: false, terminal: false },
    App { term_program: "", bundle: "org.alacritty", scripts: false, terminal: true },
    App { term_program: "", bundle: "com.todesktop.230313mzl4w4u92", scripts: false, terminal: false },
    App { term_program: "vscode", bundle: "com.microsoft.VSCode", scripts: false, terminal: false },
    App { term_program: "", bundle: "com.exafunction.windsurf", scripts: false, terminal: false },
    App { term_program: "", bundle: "com.anthropic.claudefordesktop", scripts: false, terminal: false },
];

const CLAUDE_APP: &str = "com.anthropic.claudefordesktop";

/// The app to bring forward for a session: the one its hook ran in.
pub fn app_for(host: &Host) -> Option<&'static str> {
    app(host).map(|app| app.bundle)
}

fn app(host: &Host) -> Option<&'static App> {
    if host.entrypoint == "claude-desktop" {
        return APPS.iter().find(|app| app.bundle == CLAUDE_APP);
    }
    // The bundle id is exact (VS Code and Cursor share `TERM_PROGRAM=vscode`).
    APPS.iter()
        .find(|app| !host.bundle.is_empty() && app.bundle.eq_ignore_ascii_case(&host.bundle))
        .or_else(|| APPS.iter().find(|app| !app.term_program.is_empty() && app.term_program == host.term_program))
}

/// The terminal app a session runs in, for pasting a dropped file's path. None in an editor
/// or the Claude app, where the front of the window may not be the session.
pub fn terminal_for(host: &Host) -> Option<&'static str> {
    app(host).filter(|app| app.terminal).map(|app| app.bundle)
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
    open_app(app_for(&host_for(pill)?).ok_or("Buntini doesn't know that session's terminal.")?, None)
}

/// Where the session behind an island pill runs.
fn host_for(pill: &str) -> Result<Host, String> {
    let mut parts = pill.splitn(3, ':');
    let (Some("agent"), Some(agent), Some(session)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("That isn't a session.".into());
    };
    let hub = hub();
    hub.sessions.iter().find(|((kind, id), _)| kind.id() == agent && id == session).map(|(_, session)| session.host.clone()).ok_or_else(|| "That session has gone.".into())
}

/// A single-quoted shell word.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The script that starts a session: it removes itself, goes to the project and runs the
/// agent with the prompt from an environment variable, so the prompt is never typed or
/// read as shell. The variable is read once and unset, so the agent and the shell after it
/// don't carry it. `--` keeps a prompt that starts with `-` from being read as an option
/// (both CLIs end their options there). Claude Code still takes a first word that is one of
/// its commands as that command (`claude -- doctor` runs doctor), so its prompt goes with a
/// space after it, which no command's name has. The shell stays open after the agent ends.
pub fn launch_script(agent: Agent, project: &Path, prompt: &str) -> Result<String, String> {
    let program = match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        _ => return Err(format!("{} can't be started from Buntini yet.", agent.name())),
    };
    let run = match (agent, prompt.trim().is_empty()) {
        (_, true) => program.to_string(),
        (Agent::Claude, false) => format!("{program} -- \"$buntini_prompt \""),
        (_, false) => format!("{program} -- \"$buntini_prompt\""),
    };
    Ok(format!(
        "#!/bin/sh\nrm -f \"$0\"\ncd {} || exit 1\nexport BUNTINI_PROMPT={}\nexec \"${{SHELL:-/bin/zsh}}\" -lic {}\n",
        quote(&project.to_string_lossy()),
        quote(prompt),
        quote(&format!("buntini_prompt=$BUNTINI_PROMPT; unset BUNTINI_PROMPT; {run}; unset buntini_prompt; exec \"${{SHELL:-/bin/zsh}}\" -l")),
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
/// typed by a drag. Only into a terminal app, only once it is in front, and only paths with
/// no control characters (a new line would run what came before it). The clipboard gets
/// back what it held.
pub fn drop_files(pill: &str, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > 20 || paths.iter().any(|path| !Path::new(path).exists()) {
        return Err("Those files aren't there.".into());
    }
    if paths.iter().any(|path| path.chars().any(char::is_control)) {
        return Err("That file's name can't be pasted.".into());
    }
    let terminal = terminal_for(&host_for(pill)?).ok_or("That session isn't in a terminal Buntini pastes into.")?;
    let text = paths.iter().map(|path| escape_path(path)).collect::<Vec<_>>().join(" ") + " ";
    open_app(terminal, None)?;
    #[cfg(target_os = "macos")]
    paste::paste(&text, terminal)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod paste {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSWorkspace};
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation};
    use objc2_foundation::{NSData, NSString};

    /// The V key on an ANSI keyboard.
    const KEY_V: u16 = 9;

    fn front_is(bundle: &str) -> bool {
        let front = objc2::rc::autoreleasepool(|_| NSWorkspace::sharedWorkspace().frontmostApplication().and_then(|app| app.bundleIdentifier()).map(|id| id.to_string()));
        front.is_some_and(|front| front.eq_ignore_ascii_case(bundle))
    }

    /// The terminal `bundle` is in front, or comes there within a second.
    fn in_front(bundle: &str) -> bool {
        for _ in 0..10 {
            if front_is(bundle) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    pub fn paste(text: &str, bundle: &str) -> Result<(), String> {
        if !crate::ax::trusted(false) {
            return Err("Buntini needs Accessibility to paste there.".into());
        }
        // Whatever else is in front never gets the paste.
        if !in_front(bundle) {
            return Err("That terminal didn't come forward.".into());
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
        let put_back = move || {
            let board = NSPasteboard::generalPasteboard();
            // Only if nothing else was copied meanwhile.
            if board.changeCount() == ours {
                board.clearContents();
                for (kind, data) in &saved {
                    board.setData_forType(Some(&NSData::with_bytes(data)), &NSString::from_str(kind));
                }
            }
        };
        // A moment for the window to settle, then the terminal must still be in front.
        std::thread::sleep(std::time::Duration::from_millis(150));
        if !front_is(bundle) {
            put_back();
            return Err("That terminal didn't stay in front.".into());
        }
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
        let keys: Option<Vec<_>> = [true, false].into_iter().map(|down| CGEvent::new_keyboard_event(source.as_deref(), KEY_V, down)).collect();
        let Some(keys) = keys else {
            put_back();
            return Err("The paste couldn't be sent.".into());
        };
        for event in keys {
            CGEvent::set_flags(Some(&event), CGEventFlags::MaskCommand);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            put_back();
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
        assert!(script.contains("claude -- \"$buntini_prompt \""));
        // Run through sh, with a stand-in for the agent that writes down each argument it
        // gets and whether the prompt's variable reached it.
        let run = |agent: Agent, prompt: &str| -> String {
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join(agent.id());
            std::fs::write(&fake, "#!/bin/sh\nfor a in \"$@\"; do printf '[%s]' \"$a\"; done > \"$OUT\"\nprintf ' env=%s' \"${BUNTINI_PROMPT-unset}\" >> \"$OUT\"\n").unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
            let out = dir.path().join("out");
            let script = launch_script(agent, dir.path(), prompt).unwrap();
            let script = script.replace("exec \"${SHELL:-/bin/zsh}\" -lic", "exec /bin/sh -c").replace("; exec \"${SHELL:-/bin/zsh}\" -l", "");
            let path = dir.path().join("start.command");
            std::fs::write(&path, script).unwrap();
            let status = std::process::Command::new("/bin/sh")
                .arg(&path)
                .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
                .env("OUT", &out)
                .status()
                .unwrap();
            assert!(status.success());
            assert!(!path.exists(), "the script removes itself");
            std::fs::read_to_string(&out).unwrap()
        };
        // The prompt arrives intact, after `--`, and the variable is gone by then.
        assert_eq!(run(Agent::Codex, "say $(whoami) and 'quote' `x`"), "[--][say $(whoami) and 'quote' `x`] env=unset");
        // An option is only a prompt.
        assert_eq!(run(Agent::Codex, "--dangerously-bypass-approvals-and-sandbox"), "[--][--dangerously-bypass-approvals-and-sandbox] env=unset");
        assert_eq!(run(Agent::Claude, "-p hi"), "[--][-p hi ] env=unset");
        // One of Claude Code's own commands is never exactly what it gets.
        assert_eq!(run(Agent::Claude, "purge"), "[--][purge ] env=unset");
        assert_eq!(run(Agent::Claude, ""), " env=unset");
    }

    #[test]
    fn dropped_paths_read_as_a_drag_writes_them() {
        assert_eq!(escape_path("/Users/me/My File (1).png"), "/Users/me/My\\ File\\ \\(1\\).png");
        // A name with a new line in it would run what came before it.
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("a\nrm -rf ~");
        std::fs::write(&odd, b"").unwrap();
        assert_eq!(drop_files("agent:claude:none", &[odd.to_string_lossy().into_owned()]).unwrap_err(), "That file's name can't be pasted.");
        // Only into a terminal: never an editor, whose front may be a source file.
        let host = |term: &str, bundle: &str, entry: &str| Host { term_program: term.into(), bundle: bundle.into(), iterm_session: String::new(), entrypoint: entry.into() };
        assert_eq!(terminal_for(&host("iTerm.app", "com.googlecode.iterm2", "")), Some("com.googlecode.iterm2"));
        assert_eq!(terminal_for(&host("vscode", "com.microsoft.VSCode", "")), None);
        assert_eq!(terminal_for(&host("vscode", "com.todesktop.230313mzl4w4u92", "")), None);
        assert_eq!(terminal_for(&host("zed", "dev.zed.Zed", "")), None);
        assert_eq!(terminal_for(&host("", "", "claude-desktop")), None);
        assert_eq!(escape_path("/tmp/plain.txt"), "/tmp/plain.txt");
        assert!(launch_script(Agent::Gemini, Path::new("/tmp"), "x").is_err());
    }
}
