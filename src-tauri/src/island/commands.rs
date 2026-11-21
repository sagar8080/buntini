//! Long terminal commands in the island: one that runs past ten seconds shows beside the
//! notch with how long it has run, and when it ends a line says done or failed and after how
//! long ("cargo test · done in 3:12"). A zsh hook tells Buntini when a command starts and ends,
//! over a socket of Buntini's own, like the coding agents' hooks. It's added to `.zshrc` only
//! when switched on in Settings, and taken out the same way. Only a command's first word and
//! a plain second word ("cargo test", never arguments) reach Buntini, kept in memory while the
//! command runs. Editors, pagers, remote shells and coding agents, which run for as long as
//! they're open, never show.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::i18n::tf;
use crate::surface::cards::utilities::Vitals;
use crate::surface::{self, Card, CardBody, Offer, Pill, PillState, Source, Urgency};

/// The start of each running command's pill id.
pub const PREFIX: &str = "command.";
/// A command shows once it has run this long, and says how it ended only then.
const LONG: Duration = Duration::from_secs(10);
/// How long the line saying it ended shows.
const SAID_MS: u64 = 6_000;
/// A command never heard ending (its terminal closed) is forgotten after this.
const FORGOTTEN: Duration = Duration::from_secs(12 * 3600);
/// The most commands kept at once.
const MOST: usize = 8;
const TINT: &str = "#A1A1AA";
/// What runs for as long as it's open rather than doing a job.
const NEVER: [&str; 29] = [
    "vim", "nvim", "vi", "nano", "emacs", "micro", "hx", "less", "more", "man", "top", "htop", "btop", "ssh", "mosh", "tmux", "screen", "watch", "claude", "codex", "gemini", "muse",
    "opencode", "amp", "copilot", "cursor-agent", "zsh", "bash", "fish",
];

/// A command running in a terminal.
#[derive(Debug, Clone)]
struct Running {
    id: String,
    label: String,
    since: Instant,
    /// The terminal app it runs in, to bring forward.
    bundle: String,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

fn running() -> MutexGuard<'static, Vec<Running>> {
    RUNNING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What the hook said.
#[derive(Debug, PartialEq)]
pub enum Said {
    Start { id: String, label: String, bundle: String },
    End { id: String, code: i32 },
}

/// One line from the hook: `start`, id, label, terminal bundle; or `end`, id, exit code.
pub fn parse(line: &str) -> Option<Said> {
    let parts: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    let id = |id: &str| Some(id.to_string()).filter(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_digit() || c == '.'));
    match parts.as_slice() {
        ["start", identity, label, bundle, ..] => Some(Said::Start { id: id(identity)?, label: label_of(label)?, bundle: bundle.to_string() }),
        ["end", identity, code] => Some(Said::End { id: id(identity)?, code: code.trim().parse().ok()? }),
        _ => None,
    }
}

/// "cargo test" from what the hook sent: the program's name and a plain second word, or
/// nothing for a program that runs for as long as it's open.
pub fn label_of(text: &str) -> Option<String> {
    let mut words = text.split_whitespace();
    let program = words.next()?.rsplit('/').next()?;
    let program = if program == "sudo" { words.next()?.rsplit('/').next()? } else { program };
    let plain = |word: &str| !word.is_empty() && word.len() <= 24 && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+'));
    if !plain(program) || NEVER.contains(&program) {
        return None;
    }
    let second = words.next().filter(|word| plain(word) && word.starts_with(|c: char| c.is_ascii_lowercase()));
    Some(match second {
        Some(second) => format!("{program} {second}"),
        None => program.to_string(),
    })
}

/// "3:12" or "1:02:03".
pub fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The line when a long command ends.
pub fn ended(label: &str, code: i32, elapsed: Duration) -> String {
    let key = if code == 0 { "island.command.done" } else { "island.command.failed" };
    tf(key, &[("command", &label), ("time", &clock(elapsed))])
}

/// Listens for the hook on `terminal.sock` in Buntini's data folder.
pub fn start() {
    let Some(dir) = crate::agents::socket_dir() else { return };
    let path = socket(dir);
    tauri::async_runtime::spawn(async move {
        let listener = match crate::agents::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Terminal commands unavailable: {error}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tauri::async_runtime::spawn(async move {
                        let mut lines = BufReader::new(stream).lines();
                        for _ in 0..4 {
                            let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_secs(2), lines.next_line()).await else { break };
                            if line.len() <= 1024 {
                                if let Some(said) = parse(&line) {
                                    heard(said, Instant::now());
                                }
                            }
                        }
                    });
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    });
}

pub fn socket(dir: &Path) -> PathBuf {
    dir.join("terminal.sock")
}

fn heard(said: Said, now: Instant) {
    match said {
        Said::Start { id, label, bundle } => {
            let mut running = running();
            running.retain(|command| now.duration_since(command.since) < FORGOTTEN && command.id != id);
            if running.len() >= MOST {
                running.remove(0);
            }
            running.push(Running { id, label, since: now, bundle });
            drop(running);
            // It shows once it has run long enough.
            tauri::async_runtime::spawn(async {
                tokio::time::sleep(LONG + Duration::from_millis(100)).await;
                super::refresh();
            });
        }
        Said::End { id, code } => {
            let mut running = running();
            let Some(at) = running.iter().position(|command| command.id == id) else { return };
            let command = running.remove(at);
            drop(running);
            let elapsed = now.duration_since(command.since);
            if elapsed >= LONG {
                super::publish(vec![Offer {
                    card: Card { id: format!("{PREFIX}ended"), source: Source::Island, body: CardBody::Vitals(Vitals { line: ended(&command.label, code, elapsed) }) },
                    urgency: Urgency::Alert,
                    at_ms: surface::now_ms(),
                    ttl_ms: Some(SAID_MS),
                }]);
            } else {
                super::refresh();
            }
        }
    }
}

/// The commands running long enough to show, as pills.
pub fn shown() -> Vec<Pill> {
    let now = Instant::now();
    running()
        .iter()
        .filter(|command| now.duration_since(command.since) >= LONG)
        .map(|command| Pill {
            id: format!("{PREFIX}{}", command.id),
            source: Source::Island,
            label: command.label.clone(),
            tint: TINT.into(),
            state: PillState::Working,
            badge: Some(crate::utilities::focus::badge(now.duration_since(command.since).as_millis() as i64)),
            progress: None,
            card: None,
        })
        .collect()
}

/// A click on a running command's pill: its terminal, in front.
pub fn open(pill: &str) {
    let Some(id) = pill.strip_prefix(PREFIX) else { return };
    let bundle = running().iter().find(|command| command.id == id).map(|command| command.bundle.clone());
    if let Some(bundle) = bundle.filter(|bundle| !bundle.is_empty() && bundle.len() < 200 && bundle.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')) {
        let _ = std::process::Command::new("/usr/bin/open").args(["-b", &bundle]).spawn();
    }
}

// ----- The hook in zsh -----

const BEGIN: &str = "# >>> buntini terminal commands >>>";
const END: &str = "# <<< buntini terminal commands <<<";

/// The hook, sourced from `.zshrc`. It only ever writes to Buntini's socket, in the
/// background, so a prompt never waits; without Buntini it does nothing.
pub fn script(socket: &Path) -> String {
    let socket = shell_quote(&socket.to_string_lossy());
    format!(
        r#"# Buntini: long commands beside the notch. Settings → Apps turns this off.
[[ -o interactive ]] || return 0
zmodload zsh/datetime 2>/dev/null
autoload -Uz add-zsh-hook
typeset -g __buntini_socket={socket}
typeset -g __buntini_command=''
__buntini_preexec() {{
  [[ -S $__buntini_socket ]] || return 0
  __buntini_command="$$.$EPOCHREALTIME"
  local -a words
  words=(${{(z)1}})
  local second="${{words[2]}}"
  [[ $second =~ '^[a-z][A-Za-z0-9._:+-]{{0,23}}$' ]] || second=''
  {{ printf 'start\t%s\t%s %s\t%s\n' "$__buntini_command" "${{words[1]}}" "$second" "$__CFBundleIdentifier" | /usr/bin/nc -U "$__buntini_socket" }} &>/dev/null &!
}}
__buntini_precmd() {{
  local code=$?
  [[ -n $__buntini_command && -S $__buntini_socket ]] || {{ __buntini_command=''; return 0 }}
  {{ printf 'end\t%s\t%s\n' "$__buntini_command" "$code" | /usr/bin/nc -U "$__buntini_socket" }} &>/dev/null &!
  __buntini_command=''
}}
add-zsh-hook preexec __buntini_preexec
add-zsh-hook precmd __buntini_precmd
"#
    )
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `.zshrc` with Buntini's lines added: a source of its script, between markers.
pub fn added(rc: &str, script: &Path) -> String {
    let rc = removed(rc);
    let mut out = rc.trim_end_matches('\n').to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("{BEGIN}\n[[ -r {path} ]] && source {path}\n{END}\n", path = shell_quote(&script.to_string_lossy())));
    out
}

/// `.zshrc` without Buntini's lines, everything else as it was.
pub fn removed(rc: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in rc.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        if bare == BEGIN {
            inside = true;
        } else if bare == END && inside {
            inside = false;
        } else if !inside {
            out.push(line);
        }
    }
    let text = out.concat();
    // The blank line `added` put before the block goes with it.
    match text.strip_suffix("\n\n") {
        Some(text) if rc.contains(BEGIN) => format!("{text}\n"),
        _ => text,
    }
}

pub fn is_added(rc: &str) -> bool {
    rc.lines().any(|line| line == BEGIN)
}

fn rc_path() -> Option<PathBuf> {
    let dir = std::env::var_os("ZDOTDIR").filter(|dir| !dir.is_empty()).or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(dir).join(".zshrc"))
}

/// Whether the hook is in `.zshrc`.
#[tauri::command]
pub fn terminal_commands() -> bool {
    rc_path().and_then(|path| std::fs::read_to_string(path).ok()).is_some_and(|rc| is_added(&rc))
}

/// Adds the hook to `.zshrc`, or takes it out, keeping a copy of the file as it was. New
/// terminal windows pick it up.
#[tauri::command]
pub fn set_terminal_commands(on: bool) -> Result<bool, String> {
    let dir = crate::agents::socket_dir().ok_or("Buntini is still starting.")?;
    let rc = rc_path().ok_or("There's no home folder.")?;
    if std::fs::symlink_metadata(&rc).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("Your .zshrc is a link to another file, so Buntini leaves it alone.".into());
    }
    let before = std::fs::read_to_string(&rc).or_else(|error| if error.kind() == std::io::ErrorKind::NotFound { Ok(String::new()) } else { Err(error) }).map_err(|error| error.to_string())?;
    let script_path = dir.join("terminal.zsh");
    let after = if on {
        std::fs::write(&script_path, script(&socket(dir))).map_err(|error| error.to_string())?;
        added(&before, &script_path)
    } else {
        removed(&before)
    };
    if after != before {
        if !before.is_empty() {
            let _ = std::fs::write(dir.join("zshrc.before-buntini"), &before);
        }
        std::fs::write(&rc, after).map_err(|error| error.to_string())?;
    }
    if !on {
        let _ = std::fs::remove_file(&script_path);
    }
    Ok(on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_program_and_a_plain_second_word_are_kept() {
        assert_eq!(label_of("cargo test").as_deref(), Some("cargo test"));
        assert_eq!(label_of("/usr/bin/make ").as_deref(), Some("make"));
        assert_eq!(label_of("npm run").as_deref(), Some("npm run"));
        assert_eq!(label_of("sudo softwareupdate").as_deref(), Some("softwareupdate"));
        assert_eq!(label_of("curl -H").as_deref(), Some("curl"));
        assert_eq!(label_of("export TOKEN=abc").as_deref(), Some("export"));
        for editor in ["vim notes.md", "ssh host", "claude", "less log", "top"] {
            assert_eq!(label_of(editor), None, "{editor}");
        }
        assert_eq!(label_of(""), None);
        assert_eq!(label_of("$(rm -rf) x"), None);
    }

    #[test]
    fn the_hook_lines_read_back() {
        assert_eq!(parse("start\t812.1795280000.123456\tcargo test\tcom.mitchellh.ghostty"), Some(Said::Start { id: "812.1795280000.123456".into(), label: "cargo test".into(), bundle: "com.mitchellh.ghostty".into() }));
        assert_eq!(parse("end\t812.1795280000.123456\t101\n"), Some(Said::End { id: "812.1795280000.123456".into(), code: 101 }));
        assert_eq!(parse("start\tx;rm\tcargo test\t"), None);
        assert_eq!(parse("start\t1.2\tvim a\t"), None);
        assert_eq!(parse("end\t1.2\tnot"), None);
        assert_eq!(parse("hello"), None);
    }

    #[test]
    fn a_long_command_says_how_it_ended_and_after_how_long() {
        assert_eq!(clock(Duration::from_secs(192)), "3:12");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(ended("cargo test", 0, Duration::from_secs(192)), tf("island.command.done", &[("command", &"cargo test"), ("time", &"3:12")]));
        assert_eq!(ended("make", 2, Duration::from_secs(11)), tf("island.command.failed", &[("command", &"make"), ("time", &"0:11")]));
    }

    #[test]
    fn the_zshrc_gets_its_lines_once_and_loses_only_them() {
        let script = Path::new("/Users/me/Library/Application Support/app.buntini.companion/terminal.zsh");
        let rc = "export PATH=/opt/homebrew/bin:$PATH\nalias ll='ls -l'\n";
        let on = added(rc, script);
        assert!(is_added(&on) && on.starts_with(rc));
        assert!(on.contains("source '/Users/me/Library/Application Support/app.buntini.companion/terminal.zsh'"));
        assert_eq!(added(&on, script), on, "added once");
        assert_eq!(removed(&on), rc);
        assert!(!is_added(&removed(&on)));
        assert_eq!(removed(rc), rc);
        assert!(is_added(&added("", script)));
        assert_eq!(removed(&added("", script)), "");
    }

    #[test]
    fn the_hook_names_its_socket_and_sends_no_arguments() {
        let script = script(Path::new("/tmp/it's/terminal.sock"));
        assert!(script.contains(r"typeset -g __buntini_socket='/tmp/it'\''s/terminal.sock'"));
        assert!(script.contains("&!"), "never makes the prompt wait");
        assert!(!script.contains("$1\""), "never sends the whole command line");
    }

    /// The hook in a real zsh, without touching `.zshrc`: `cargo test --lib
    /// island::commands::tests::live -- --ignored --nocapture`. Runs `sleep 1` and a command
    /// that fails in an interactive zsh with only the hook, and prints what reached the socket.
    #[test]
    #[ignore]
    fn live() {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::Builder::new().prefix("bt").tempdir_in("/tmp").unwrap();
        let socket = dir.path().join("t.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let script_path = dir.path().join("terminal.zsh");
        std::fs::write(&script_path, script(&socket)).unwrap();
        let mut zsh = std::process::Command::new("/bin/zsh").args(["-f", "-i"]).env("__CFBundleIdentifier", "com.apple.Terminal").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let mut input = zsh.stdin.take().unwrap();
        writeln!(input, "source {}", shell_quote(&script_path.to_string_lossy())).unwrap();
        writeln!(input, "sleep 1").unwrap();
        writeln!(input, "ls /no/such/folder").unwrap();
        writeln!(input, "exit").unwrap();
        drop(input);
        let mut said = Vec::new();
        listener.set_nonblocking(false).unwrap();
        while said.len() < 4 {
            let (stream, _) = listener.accept().unwrap();
            for line in std::io::BufReader::new(stream).lines() {
                let line = line.unwrap();
                println!("{line}");
                said.push(parse(&line).expect("a line Buntini reads"));
            }
        }
        zsh.wait().unwrap();
        assert!(matches!(&said[0], Said::Start { label, bundle, .. } if label == "sleep" && bundle == "com.apple.Terminal"));
        assert!(matches!(&said[1], Said::End { code: 0, .. }));
        assert!(matches!(&said[2], Said::Start { label, .. } if label == "ls"));
        assert!(matches!(&said[3], Said::End { code, .. } if *code != 0));
    }
}
