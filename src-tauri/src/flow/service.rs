//! The audio helper in service mode: one long-lived `buntini-audio --service` speaking JSON
//! lines. Rust sends `{"op": …}` commands; the helper answers with `{"event": …}` lines.
//! Voice bands go straight to the pill; every other event goes to whoever listens.
use crate::i18n::t;
use serde_json::Value;
use std::{path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
};

/// The protocol both sides speak; the helper says it in its `ready` line.
pub const PROTOCOL: u64 = 3;
/// Longest line the helper may send: a call's segments can be long.
const LINE_LIMIT: usize = 8 << 20;

struct Running {
    child: Child,
    input: ChildStdin,
}

pub struct Service {
    helper: PathBuf,
    models: PathBuf,
    running: Mutex<Option<Running>>,
    events: mpsc::UnboundedSender<Value>,
}

impl Service {
    pub fn new(helper: PathBuf, models: PathBuf, events: mpsc::UnboundedSender<Value>) -> Self {
        Self { helper, models, running: Mutex::new(None), events }
    }

    pub async fn running(&self) -> bool {
        let mut running = self.running.lock().await;
        match running.as_mut() {
            Some(service) => matches!(service.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Sends one command, starting the helper first when it isn't running.
    pub async fn send(&self, command: Value) -> Result<(), String> {
        self.write(command, true).await
    }

    /// Sends one command to the helper that is running; none is started for it.
    pub async fn send_running(&self, command: Value) -> Result<(), String> {
        self.write(command, false).await
    }

    async fn write(&self, command: Value, start: bool) -> Result<(), String> {
        let mut running = self.running.lock().await;
        if let Some(service) = running.as_mut() {
            if !matches!(service.child.try_wait(), Ok(None)) {
                *running = None;
            }
        }
        if running.is_none() {
            if !start {
                return Err(t("flow.error.helperStopped"));
            }
            *running = Some(self.spawn()?);
        }
        let service = running.as_mut().expect("just started");
        let mut line = serde_json::to_vec(&command).map_err(|_| t("flow.error.helperTalk"))?;
        line.push(b'\n');
        if service.input.write_all(&line).await.is_err() || service.input.flush().await.is_err() {
            *running = None;
            return Err(t("flow.error.helperStopped"));
        }
        Ok(())
    }

    /// Closes the helper. It undoes its own muting and deletes unfinished recordings when
    /// its input closes, so this is also the clean way out.
    pub async fn stop(&self) {
        if let Some(mut service) = self.running.lock().await.take() {
            let _ = service.input.write_all(b"{\"op\":\"quit\"}\n").await;
            drop(service.input);
            if tokio::time::timeout(std::time::Duration::from_secs(3), service.child.wait()).await.is_err() {
                let _ = service.child.kill().await;
            }
        }
    }

    fn spawn(&self) -> Result<Running, String> {
        if !self.helper.is_file() {
            return Err(t("flow.error.helperMissing"));
        }
        let mut child = Command::new(&self.helper)
            .arg("--service")
            .arg("--models")
            .arg(&self.models)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| t("flow.error.helperStart"))?;
        let input = child.stdin.take().ok_or_else(|| t("flow.error.helperInput"))?;
        let output = child.stdout.take().ok_or_else(|| t("flow.error.helperOutput"))?;
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::with_capacity(1 << 16, output).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.len() > LINE_LIMIT {
                    continue;
                }
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(values) = bands(&event) {
                    crate::surface::levels(values);
                    continue;
                }
                if events.send(event).is_err() {
                    break;
                }
            }
            let _ = events.send(serde_json::json!({"event": "exited"}));
        });
        Ok(Running { child, input })
    }
}

/// Nine voice bands from a `bands` event.
pub fn bands(event: &Value) -> Option<[f32; 9]> {
    if event.get("event")?.as_str()? != "bands" {
        return None;
    }
    let values = event.get("values")?.as_array()?;
    if values.len() != 9 {
        return None;
    }
    let mut out = [0f32; 9];
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = (value.as_f64()? as f32).clamp(0.0, 1.0);
    }
    Some(out)
}

/// The event's name, or "".
pub fn name(event: &Value) -> &str {
    event.get("event").and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bands_are_read_and_clamped() {
        let event = json!({"event": "bands", "id": 3, "values": [0.0, 0.5, 1.5, -1.0, 0.25, 0.1, 0.2, 0.3, 0.4]});
        assert_eq!(bands(&event), Some([0.0, 0.5, 1.0, 0.0, 0.25, 0.1, 0.2, 0.3, 0.4]));
        assert_eq!(bands(&json!({"event": "bands", "values": [0.1]})), None);
        assert_eq!(bands(&json!({"event": "final", "values": vec![0.0; 9]})), None);
    }

    /// A stand-in helper that answers each command with the event a real one would send.
    fn fake(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("fake-audio");
        std::fs::write(
            &path,
            r#"#!/usr/bin/env python3
import json, sys
assert sys.argv[1:3] == ["--service", "--models"], sys.argv
print(json.dumps({"event": "ready", "protocolVersion": 3, "parakeet": False}), flush=True)
for line in sys.stdin:
    command = json.loads(line)
    if command["op"] == "dictate":
        print(json.dumps({"event": "bands", "id": command["id"], "values": [0.5] * 9}), flush=True)
        print(json.dumps({"event": "listening", "id": command["id"]}), flush=True)
    elif command["op"] == "stop":
        print(json.dumps({"event": "final", "id": command["id"], "text": "hello there"}), flush=True)
    elif command["op"] == "quit":
        break
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    async fn next(events: &mut mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn commands_start_the_helper_and_events_come_back_without_the_bands() {
        let dir = tempfile::tempdir().unwrap();
        let (sender, mut events) = mpsc::unbounded_channel();
        let service = Service::new(fake(dir.path()), dir.path().join("models"), sender);
        assert!(!service.running().await);
        service.send(json!({"op": "dictate", "id": 1})).await.unwrap();
        assert_eq!(name(&next(&mut events).await), "ready");
        // The bands went to the pill, not here.
        assert_eq!(next(&mut events).await, json!({"event": "listening", "id": 1}));
        service.send(json!({"op": "stop", "id": 1})).await.unwrap();
        assert_eq!(next(&mut events).await["text"], "hello there");
        assert!(service.running().await);
        service.stop().await;
        assert_eq!(name(&next(&mut events).await), "exited");
        assert!(!service.running().await);
        // Some commands mustn't start a helper just for themselves.
        assert!(service.send_running(json!({"op": "unmute", "devices": []})).await.is_err());
        assert!(!service.running().await);
    }

    #[tokio::test]
    async fn a_missing_helper_says_so() {
        let (sender, _events) = mpsc::unbounded_channel();
        let service = Service::new(PathBuf::from("/nonexistent/buntini-audio"), PathBuf::new(), sender);
        assert!(service.send(json!({"op": "dictate"})).await.unwrap_err().contains("missing"));
    }
}
