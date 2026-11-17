//! Relays one Codex TUI connection to its App Server over private Unix sockets.
//! Both Codex processes run in the user's terminal, including the one that can execute tools.
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener as StdListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use super::{
    count, decision_budget, hub, next_id, project, recap, visible, Agent, AgentRequest, Answer,
    Pending, MAX_DETAIL,
};

const COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
const CONNECT_LIMIT: Duration = Duration::from_secs(5);

pub(super) struct Relay {
    folder: PathBuf,
    listener: StdListener,
}

impl Relay {
    pub(super) fn new() -> io::Result<Self> {
        // macOS Unix socket paths are limited to 103 bytes.
        let folder = Path::new("/tmp").join(format!("buntini-codex-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&folder)?;
        let listener = StdListener::bind(folder.join("tui.sock")).and_then(|listener| {
            listener.set_nonblocking(true)?;
            Ok(listener)
        });
        match listener {
            Ok(listener) => Ok(Self { folder, listener }),
            Err(error) => {
                let _ = std::fs::remove_dir_all(folder);
                Err(error)
            }
        }
    }

    /// Start the server from the terminal so its tool subprocesses have the terminal's
    /// permissions. Older CLI versions and a server that fails to start use the ordinary TUI.
    pub(super) fn command(&self, plain: &str, has_prompt: bool) -> String {
        let front = super::terminal::quote(&format!("unix://{}/tui.sock", self.folder.display()));
        let back = super::terminal::quote(&format!("unix://{}/server.sock", self.folder.display()));
        let back_file =
            super::terminal::quote(&self.folder.join("server.sock").display().to_string());
        let prompt = if has_prompt {
            r#" -- "$buntini_prompt""#
        } else {
            ""
        };
        format!(
            r#"if codex --help 2>/dev/null | /usr/bin/grep -q -- '--remote' && codex app-server --help 2>/dev/null | /usr/bin/grep -q 'unix://'; then
  (
    unset BUNTINI_PROMPT
    codex app-server --listen {back} </dev/null >/dev/null 2>&1 &
    buntini_server_pid=$!
    trap 'kill "$buntini_server_pid" 2>/dev/null || true; wait "$buntini_server_pid" 2>/dev/null || true' EXIT
    trap 'exit 130' INT TERM HUP
    buntini_attempt=0
    until test -S {back_file}; do
      if ! kill -0 "$buntini_server_pid" 2>/dev/null || test "$buntini_attempt" -ge 30; then exit 75; fi
      buntini_attempt=$((buntini_attempt + 1))
      /bin/sleep 0.1
    done
    codex --remote {front}{prompt}
    # The TUI may have submitted the prompt. Never retry it based on the TUI exit status.
    exit 0
  ) || {{ if test "$?" -eq 75; then {plain}; fi; }}
else
  {plain}
fi"#
        )
    }

    pub(super) async fn serve(self) {
        let Ok(listener) = self.listener.try_clone().and_then(UnixListener::from_std) else {
            return;
        };
        let Ok(Ok((stream, _))) = timeout(Duration::from_secs(30), listener.accept()).await else {
            return;
        };
        drop(listener);
        let Ok(Ok(mut tui)) = timeout(CONNECT_LIMIT, tokio_tungstenite::accept_async(stream)).await
        else {
            return;
        };
        let Ok(backend) = UnixStream::connect(self.folder.join("server.sock")).await else {
            return;
        };
        let Ok(Ok((mut server, _))) = timeout(
            CONNECT_LIMIT,
            tokio_tungstenite::client_async("ws://localhost", backend),
        )
        .await
        else {
            return;
        };

        let mut threads = Threads::default();
        let mut experimental = false;
        let mut pending = JoinSet::new();
        let mut requests: HashMap<String, tokio::task::AbortHandle> = HashMap::new();
        loop {
            let delivered = tokio::select! {
                from_tui = tui.next() => {
                    let Some(Ok(message)) = from_tui else { break };
                    if let Message::Text(raw) = &message {
                        if let Ok(value) = serde_json::from_str::<Value>(raw.as_str()) {
                            match value["method"].as_str() {
                                Some("initialize") => experimental = value["params"]["capabilities"]["experimentalApi"] == true,
                                Some("turn/start") if experimental => threads.add(value["params"]["threadId"].as_str()),
                                _ => {}
                            }
                        }
                    }
                    match message {
                        Message::Text(_) => server.send(message).await,
                        Message::Close(_) => break,
                        _ => tui.flush().await,
                    }
                }
                from_server = server.next() => {
                    let Some(Ok(message)) = from_server else { break };
                    if let Message::Text(raw) = &message {
                        if let Ok(value) = serde_json::from_str::<Value>(raw.as_str()) {
                            match value["method"].as_str() {
                                Some("serverRequest/resolved") => {
                                    if let Some(request) = requests.remove(&value["params"]["requestId"].to_string()) {
                                        request.abort();
                                    }
                                }
                                Some(COMMAND_APPROVAL) if experimental && pending.len() < 16 => {
                                    if let Some(approval) = Approval::from_request(&value, &threads) {
                                        let key = approval.id.to_string();
                                        if !requests.contains_key(&key) {
                                            requests.insert(key, pending.spawn(approval.wait(message)));
                                            continue;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    match message {
                        Message::Text(_) => tui.send(message).await,
                        Message::Close(_) => break,
                        _ => server.flush().await,
                    }
                }
                decision = pending.join_next(), if !pending.is_empty() => {
                    let Some(Ok((id, reply, original))) = decision else { continue };
                    // The server may have cancelled the turn while the decision was queued.
                    if requests.remove(&id).is_none() { continue; }
                    match reply {
                        Some(reply) => server.send(reply).await,
                        None => tui.send(original).await,
                    }
                }
            };
            if delivered.is_err() {
                break;
            }
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[derive(Default)]
struct Threads(HashSet<String>);

impl Threads {
    fn add(&mut self, thread: Option<&str>) {
        if let Some(thread) = thread.filter(|thread| !thread.is_empty() && thread.len() <= 200) {
            if self.0.len() < 64 && self.0.insert(thread.into()) {
                *hub()
                    .codex_app_server_threads
                    .entry(thread.into())
                    .or_default() += 1;
            }
        }
    }
}

impl Drop for Threads {
    fn drop(&mut self) {
        let mut hub = hub();
        for thread in &self.0 {
            if let Some(count) = hub.codex_app_server_threads.get_mut(thread) {
                *count -= 1;
                if *count == 0 {
                    hub.codex_app_server_threads.remove(thread);
                }
            }
        }
    }
}

struct Approval {
    id: Value,
    thread: String,
    cwd: String,
    detail: String,
}

impl Approval {
    fn from_request(message: &Value, threads: &Threads) -> Option<Self> {
        let id = message.get("id")?;
        if !id.is_string() && !id.is_i64() {
            return None;
        }
        let params = &message["params"];
        // New protocol fields may carry execution scope this card cannot display.
        if !super::only(
            params,
            &[
                "additionalPermissions",
                "approvalId",
                "availableDecisions",
                "command",
                "commandActions",
                "cwd",
                "environmentId",
                "itemId",
                "kind",
                "networkApprovalContext",
                "proposedExecpolicyAmendment",
                "proposedNetworkPolicyAmendments",
                "reason",
                "startedAtMs",
                "threadId",
                "turnId",
            ],
        ) {
            return None;
        }
        let thread = params["threadId"].as_str()?;
        if !threads.0.contains(thread) {
            return None;
        }
        let command = params["command"].as_str()?;
        let cwd = params["cwd"].as_str()?;
        let actions = params["availableDecisions"].as_array()?;
        // One command, in the local environment, by its full text and real directory.
        // An approval can run outside the sandbox. Any separately requested permission,
        // network destination, remote executor, or terminal input uses Codex's own prompt.
        if !(params["kind"].is_null() || params["kind"] == "command")
            || !(params["environmentId"].is_null() || params["environmentId"] == "local")
            || !params["approvalId"].is_null()
            || !params["networkApprovalContext"].is_null()
            || !params["additionalPermissions"].is_null()
            || !params["proposedNetworkPolicyAmendments"].is_null()
            || !actions.iter().any(|action| action == "accept")
            || !actions.iter().any(|action| action == "decline")
            || !Path::new(cwd).is_absolute()
            || cwd.contains(['\n', '\t'])
            || command.trim().is_empty()
        {
            return None;
        }
        let detail = format!("{cwd}\n\n{command}");
        if detail.chars().count() > MAX_DETAIL || visible(&detail) != detail {
            return None;
        }
        Some(Self {
            id: id.clone(),
            thread: thread.into(),
            cwd: cwd.into(),
            detail,
        })
    }

    async fn wait(self, original: Message) -> (String, Option<Message>, Message) {
        let card = AgentRequest {
            id: next_id(),
            agent: Agent::Codex.name().into(),
            project: visible(&project(&self.cwd)),
            title: crate::i18n::t("agents.request.codexCommandScope"),
            detail: self.detail,
            code: true,
            questions: vec![],
            always: false,
            plan: false,
            allow: true,
            changes: vec![],
            rules: vec![],
            created_at: crate::types::now(),
        };
        let (_pending, receiver) =
            Pending::show(card, Agent::Codex, self.thread, "Bash".into(), Value::Null);
        let decision = match timeout(decision_budget(Agent::Codex), receiver).await {
            Ok(Ok(Answer::Allow)) => Some("accept"),
            Ok(Ok(Answer::Deny)) => Some("decline"),
            _ => None,
        };
        if let Some(decision) = decision {
            count(|recap| recap.decision(recap::today(), decision == "accept"));
        }
        let reply = decision.map(|decision| {
            Message::Text(
                json!({ "id": self.id, "result": { "decision": decision } })
                    .to_string()
                    .into(),
            )
        });
        (self.id.to_string(), reply, original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_local_command_approvals_can_move_from_codex_to_the_island() {
        let threads = Threads(HashSet::from(["thread".into()]));
        let message = json!({
            "id": 7,
            "method": COMMAND_APPROVAL,
            "params": {
                "threadId": "thread",
                "command": "git diff\ngit status",
                "cwd": "/Users/me/Project",
                "environmentId": "local",
                "kind": "command",
                "availableDecisions": ["accept", "acceptForSession", "decline", "cancel"],
            },
        });
        let approval = Approval::from_request(&message, &threads).unwrap();
        assert_eq!(approval.detail, "/Users/me/Project\n\ngit diff\ngit status");
        for (field, value) in [
            ("threadId", json!("other")),
            ("cwd", json!("relative")),
            ("command", json!("echo \u{202E}hidden")),
            ("command", json!("x".repeat(MAX_DETAIL))),
            ("environmentId", json!("remote")),
            ("kind", json!("writeStdin")),
            ("approvalId", json!("subcommand")),
            ("networkApprovalContext", json!({"host": "example.com"})),
            (
                "additionalPermissions",
                json!({"network": {"enabled": true}}),
            ),
            ("availableDecisions", json!(["acceptForSession", "decline"])),
            ("newScope", json!("future server field")),
        ] {
            let mut rejected = message.clone();
            rejected["params"][field] = value;
            assert!(
                Approval::from_request(&rejected, &threads).is_none(),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn relay_answers_one_request_and_hands_the_next_back_to_the_tui() {
        const THREAD: &str = "codex-relay-test";
        let relay = Relay::new().unwrap();
        let backend = UnixListener::bind(relay.folder.join("server.sock")).unwrap();
        let frontend = UnixStream::connect(relay.folder.join("tui.sock"))
            .await
            .unwrap();
        let relay = tokio::spawn(relay.serve());
        let scenario = async {
            let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", frontend)
                .await
                .unwrap();
            let (backend, _) = backend.accept().await.unwrap();
            let mut server = tokio_tungstenite::accept_async(backend).await.unwrap();
            let send = |value: Value| Message::Text(value.to_string().into());

            for message in [
                json!({"id": 1, "method": "initialize", "params": {"capabilities": {"experimentalApi": true}}}),
                json!({"id": 2, "method": "turn/start", "params": {"threadId": THREAD}}),
            ] {
                let message = send(message);
                tui.send(message.clone()).await.unwrap();
                assert_eq!(server.next().await.unwrap().unwrap(), message);
            }

            for (id, answer) in [(3, Answer::Allow), (4, Answer::Terminal)] {
                let original = send(json!({
                    "id": id, "method": COMMAND_APPROVAL, "params": {
                        "threadId": THREAD,
                        "cwd": "/tmp/buntini-codex",
                        "command": "git status",
                        "availableDecisions": ["accept", "decline"],
                    },
                }));
                server.send(original.clone()).await.unwrap();
                let card = loop {
                    let card = hub()
                        .waiting
                        .iter()
                        .find(|request| request.session == THREAD)
                        .map(|request| request.view.id.clone());
                    if let Some(card) = card {
                        break card;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                };
                let allow = matches!(answer, Answer::Allow);
                tokio::time::sleep(Duration::from_millis(super::super::ARMING_MS + 20)).await;
                super::super::answer_agent(card, answer).unwrap();
                if allow {
                    let reply = server.next().await.unwrap().unwrap();
                    assert_eq!(
                        reply,
                        send(json!({"id": id, "result": {"decision": "accept"}}))
                    );
                } else {
                    assert_eq!(tui.next().await.unwrap().unwrap(), original);
                }
            }
        };
        timeout(Duration::from_secs(10), scenario).await.unwrap();
        relay.await.unwrap();
        assert!(!hub().codex_app_server_threads.contains_key(THREAD));
    }
}
