//! How much of their plan Claude Code and Codex have used: the five-hour and weekly
//! windows. Claude Code hands them to its status line, which Buntini wraps (the person's
//! own status line keeps running); Codex answers through its own `codex app-server`,
//! asked at most once a minute. Nothing of Buntini's is sent anywhere.
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::surface::cards::agents::{Usage, UsageWindow};

/// Claude Code's `rate_limits`, from what its status line receives.
pub fn from_status_line(status: &Value) -> Option<Usage> {
    let limits = status.get("rate_limits")?.as_object()?;
    let window = |key: &str, label: &str| {
        let window = limits.get(key)?;
        Some(UsageWindow { label: label.into(), percent: window.get("used_percentage")?.as_f64()?.clamp(0.0, 100.0), resets_at: window.get("resets_at").and_then(Value::as_i64) })
    };
    let windows: Vec<UsageWindow> = [window("five_hour", "5 hours"), window("seven_day", "Week")].into_iter().flatten().collect();
    (!windows.is_empty()).then(|| Usage { agent: "Claude".into(), windows })
}

/// Codex's answer to `account/rateLimits/read`.
pub fn from_codex(result: &Value) -> Option<Usage> {
    let snapshot = result.get("rateLimitsByLimitId").and_then(|by| by.get("codex")).or_else(|| result.get("rateLimits"))?;
    let window = |key: &str| {
        let window = snapshot.get(key)?;
        let minutes = window.get("windowDurationMins").and_then(Value::as_i64);
        let label = match minutes {
            Some(300) => "5 hours".to_string(),
            Some(10_080) => "Week".to_string(),
            Some(minutes) if minutes % 1_440 == 0 => format!("{} days", minutes / 1_440),
            Some(minutes) if minutes % 60 == 0 => format!("{} hours", minutes / 60),
            _ => if key == "primary" { "Short" } else { "Long" }.to_string(),
        };
        Some(UsageWindow { label, percent: window.get("usedPercent")?.as_f64()?.clamp(0.0, 100.0), resets_at: window.get("resetsAt").and_then(Value::as_i64) })
    };
    let windows: Vec<UsageWindow> = [window("primary"), window("secondary")].into_iter().flatten().collect();
    (!windows.is_empty()).then(|| Usage { agent: "Codex".into(), windows })
}

/// Where a program installed for the person's own shell is, without running that shell:
/// GUI apps don't get the PATH that npm, nvm or Homebrew set up.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let mut dirs = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), home.join(".local/bin"), home.join(".npm-global/bin"), home.join(".bun/bin"), home.join(".volta/bin")];
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut versions: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path().join("bin")).collect();
        versions.sort();
        dirs.extend(versions.into_iter().rev());
    }
    dirs.into_iter().map(|dir| dir.join(name)).find(|path| path.is_file())
}

static LAST_CODEX: Mutex<Option<Instant>> = Mutex::new(None);

/// Asks Codex for its plan's windows, at most once a minute. Blocks up to 15 seconds.
pub fn read_codex() -> Option<Usage> {
    {
        let mut last = LAST_CODEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            return None;
        }
        *last = Some(Instant::now());
    }
    let codex = find_program("codex")?;
    // Node-based installs need their own folder on PATH for `node`.
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin", codex.parent()?.display());
    let mut child = Command::new(&codex).arg("app-server").env("PATH", path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if sender.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let answer = |id: i64| -> Option<Value> {
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let value = receiver.recv_timeout(left).ok()?;
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return value.get("result").cloned();
            }
        }
    };
    let mut send = |value: Value| writeln!(stdin, "{value}").ok();
    let result = (|| {
        send(json!({ "method": "initialize", "id": 0, "params": { "clientInfo": { "name": "buntini", "title": "Buntini", "version": env!("CARGO_PKG_VERSION") } } }))?;
        answer(0)?;
        send(json!({ "method": "initialized" }))?;
        send(json!({ "method": "account/rateLimits/read", "id": 1 }))?;
        answer(1)
    })();
    let _ = child.kill();
    let _ = child.wait();
    from_codex(&result?)
}

/// Claude Code's status line with Buntini's wrapper: it hands Buntini what the status line
/// receives, then runs the person's own status line, if any, on the same input.
pub fn status_line_command(socket: &Path, previous: Option<&str>) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let path = quote(&socket.to_string_lossy());
    let theirs = previous.map(|command| format!(" printf '%s' \"$input\" | /bin/sh -c {};", quote(command))).unwrap_or_default();
    format!("input=$(cat); [ -S {path} ] && printf 'BUNTINI\\t\\t\\t\\t\\tstatusline\\n%s' \"$input\" | /usr/bin/nc -U {path} >/dev/null 2>&1;{theirs} exit 0")
}

pub fn is_status_line_ours(command: &str) -> bool {
    command.contains("/usr/bin/nc -U") && command.contains("buntini") && command.contains("statusline")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_s_windows_read_from_its_status_line() {
        let status = json!({ "rate_limits": { "five_hour": { "used_percentage": 37.5, "resets_at": 1763300000 }, "seven_day": { "used_percentage": 12 } } });
        let usage = from_status_line(&status).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!((usage.windows[0].label.as_str(), usage.windows[0].percent, usage.windows[0].resets_at), ("5 hours", 37.5, Some(1763300000)));
        assert_eq!(usage.windows[1].resets_at, None);
        assert!(from_status_line(&json!({ "model": {} })).is_none(), "no plan, no card");
    }

    #[test]
    fn codex_s_windows_are_named_by_their_length() {
        let result = json!({ "rateLimits": { "primary": { "usedPercent": 64, "windowDurationMins": 300, "resetsAt": 1763301000 }, "secondary": { "usedPercent": 20, "windowDurationMins": 10080, "resetsAt": null } } });
        let usage = from_codex(&result).unwrap();
        assert_eq!(usage.windows.iter().map(|window| window.label.as_str()).collect::<Vec<_>>(), ["5 hours", "Week"]);
        assert_eq!(usage.windows[0].percent, 64.0);
    }

    /// The wrapped status line, run by sh: Buntini gets the input, the person's own status
    /// line still prints what it printed.
    #[test]
    fn the_wrapper_keeps_the_person_s_status_line() {
        let dir = tempfile::Builder::new().prefix("buntini").tempdir().unwrap();
        let socket = dir.path().join("claude.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let command = status_line_command(&socket, Some(r#"grep -q Opus && echo "it's mine""#));
        assert!(is_status_line_ours(&command));
        let mut child = Command::new("/bin/sh").arg("-c").arg(&command).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(br#"{"model":{"display_name":"Opus"},"rate_limits":{"five_hour":{"used_percentage":5}}}"#).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        let mut sent = String::new();
        std::io::Read::read_to_string(&mut stream, &mut sent).unwrap();
        // Buntini closes its end once it has read the event, which lets nc go.
        drop(stream);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(sent.starts_with("BUNTINI\t\t\t\t\tstatusline\n{"), "{sent}");
        let printed = String::from_utf8_lossy(&output.stdout);
        assert_eq!(printed.trim(), "it's mine", "their status line got the same input");
        // No status line of their own: nothing printed, still fine.
        let bare = status_line_command(Path::new("/tmp/none/claude.sock"), None);
        let output = Command::new("/bin/sh").arg("-c").arg(&bare).stdin(Stdio::null()).output().unwrap();
        assert!(output.status.success() && output.stdout.is_empty());
    }
}
