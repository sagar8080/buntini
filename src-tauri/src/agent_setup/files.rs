//! The files Buntini writes for agents whose hooks live in a file or plugin of their own:
//! Copilot CLI's hook file, the OpenCode and Amp plugins, and the Hermes plugin. Each only
//! tells Buntini what the agent does; none decides anything for it.
use std::path::Path;

use serde_json::json;

use super::note_command_for;

/// Copilot CLI: Claude's event names where Copilot takes them (its payloads then name the
/// event too), its own camelCase names elsewhere. No permission hook: Copilot asks it about
/// every tool, before its own rules, so holding it would hold tools Copilot already allows.
/// Its own prompt shows on the island as a waiting pill instead.
pub fn copilot(socket: &Path) -> Vec<u8> {
    let hook = |command: String, seconds: u64| json!([{ "type": "command", "bash": command, "timeoutSec": seconds }]);
    let note = |event: &str| hook(note_command_for(socket, event), 5);
    let value = json!({
        "version": 1,
        "hooks": {
            "SessionStart": note(""),
            "UserPromptSubmit": note(""),
            "PreToolUse": note(""),
            "postToolUse": note("postToolUse"),
            "postToolUseFailure": note("postToolUseFailure"),
            "Stop": note(""),
            "sessionEnd": hook(note_command_for(socket, "sessionEnd"), 3),
            "notification": note("notification"),
        }
    });
    let mut bytes = serde_json::to_vec_pretty(&value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// The socket path as a JavaScript or Python string literal.
fn literal(socket: &Path) -> String {
    serde_json::to_string(&socket.to_string_lossy()).unwrap_or_else(|_| "\"\"".into())
}

/// OpenCode: one file for OpenCode 2 (`setup`) and OpenCode 1 (`server`).
pub fn opencode(socket: &Path) -> Vec<u8> {
    format!(
        r#"// Tells Buntini what OpenCode is doing, for its island. Written by Buntini; turning
// OpenCode off in Buntini removes it.
import net from "node:net"

const SOCKET = {socket}
const send = (event) => {{
  try {{
    const stream = net.createConnection(SOCKET)
    stream.on("error", () => {{}})
    stream.on("connect", () => stream.end(JSON.stringify(event)))
  }} catch {{}}
}}
const args = (input) => (input && typeof input === "object" ? input : {{}})

export default {{
  id: "buntini",
  async setup(ctx) {{
    const cwd = ctx.location?.directory ?? ""
    const stop = new AbortController()
    ;(async () => {{
      try {{
        for await (const event of ctx.event.subscribe({{ types: ["session.created", "session.status"], signal: stop.signal }})) {{
          const data = event.data ?? {{}}
          if (event.type === "session.created") send({{ hook_event_name: "SessionStart", session_id: data.id ?? data.sessionID ?? "", cwd }})
          if (event.type === "session.status" && data.status?.type === "idle") send({{ hook_event_name: "Stop", session_id: data.sessionID ?? "", cwd }})
        }}
      }} catch {{}}
    }})()
    try {{ await ctx.session?.hook?.("prompt", (event) => {{ send({{ hook_event_name: "UserPromptSubmit", session_id: event.sessionID ?? "", cwd, prompt: event.prompt?.text ?? "" }}) }}) }} catch {{}}
    try {{ await ctx.tool?.hook?.("execute.before", (event) => {{ send({{ hook_event_name: "PreToolUse", session_id: event.sessionID ?? "", cwd, tool_name: event.tool, tool_input: args(event.input) }}) }}) }} catch {{}}
    try {{ await ctx.tool?.hook?.("execute.after", (event) => {{ send({{ hook_event_name: event.status === "error" ? "PostToolUseFailure" : "PostToolUse", session_id: event.sessionID ?? "", cwd, tool_name: event.tool, tool_input: args(event.input) }}) }}) }} catch {{}}
    return () => stop.abort()
  }},
  async server({{ directory }}) {{
    return {{
      event: async ({{ event }}) => {{
        const data = event.properties ?? {{}}
        if (event.type === "session.created") send({{ hook_event_name: "SessionStart", session_id: data.info?.id ?? "", cwd: directory }})
        if (event.type === "session.idle") send({{ hook_event_name: "Stop", session_id: data.sessionID ?? "", cwd: directory }})
      }},
      "tool.execute.before": async (input, output) => {{ send({{ hook_event_name: "PreToolUse", session_id: input.sessionID ?? "", cwd: directory, tool_name: input.tool, tool_input: args(output?.args) }}) }},
      "tool.execute.after": async (input, output) => {{ send({{ hook_event_name: "PostToolUse", session_id: input.sessionID ?? "", cwd: directory, tool_name: input.tool, tool_input: args(output?.args) }}) }},
    }}
  }},
}}
"#,
        socket = literal(socket)
    )
    .into_bytes()
}

/// Amp: a plugin that tells Buntini what each tool did, once it has run. It has no
/// `tool.call` handler: Amp's plugin API has no answer there that leaves the call to Amp's own
/// permissions ("allow" lets it run, and a handler must answer), so a tool shows on the
/// ticker when its result comes.
pub fn amp(socket: &Path) -> Vec<u8> {
    format!(
        r#"// Tells Buntini what Amp is doing, for its island. Written by Buntini; turning Amp off in
// Buntini removes it. It only listens: it never answers for a tool.
import net from "node:net"

const SOCKET = {socket}
function send(event: Record<string, unknown>) {{
  try {{
    const stream = net.createConnection(SOCKET)
    stream.on("error", () => {{}})
    stream.on("connect", () => stream.end(JSON.stringify(event)))
  }} catch {{}}
}}
const thread = (event: any, ctx: any) => String(event?.thread?.id ?? ctx?.thread?.id ?? "")
const cwd = () => {{ try {{ return process.cwd() }} catch {{ return "" }} }}

export default function (amp: any) {{
  amp.on("session.start", async (event: any, ctx: any) => {{ send({{ hook_event_name: "SessionStart", session_id: thread(event, ctx), cwd: cwd() }}) }})
  amp.on("agent.start", async (event: any, ctx: any) => {{ send({{ hook_event_name: "UserPromptSubmit", session_id: thread(event, ctx), cwd: cwd(), prompt: typeof event?.message === "string" ? event.message : "" }}) }})
  amp.on("tool.result", async (event: any, ctx: any) => {{ send({{ hook_event_name: event?.status === "error" ? "PostToolUseFailure" : "PostToolUse", session_id: thread(event, ctx), cwd: cwd(), tool_name: String(event?.tool ?? ""), tool_input: event?.input ?? {{}} }}) }})
  amp.on("agent.end", async (event: any, ctx: any) => {{ send({{ hook_event_name: "Stop", session_id: thread(event, ctx), cwd: cwd(), last_assistant_message: typeof event?.message === "string" ? event.message : "" }}) }})
}}
"#,
        socket = literal(socket)
    )
    .into_bytes()
}

/// Hermes: `plugin.yaml` and `__init__.py`.
pub fn hermes(socket: &Path) -> (Vec<u8>, Vec<u8>) {
    let manifest = "name: buntini\nversion: \"1.0\"\ndescription: Tells Buntini what Hermes is doing, for its island.\nprovides_hooks:\n  - on_session_start\n  - pre_llm_call\n  - pre_tool_call\n  - post_tool_call\n  - post_llm_call\n  - on_session_finalize\n";
    let code = format!(
        r#""""Tells Buntini what Hermes is doing, for its island. Written by Buntini; turning Hermes
off in Buntini removes it. It never blocks or changes a tool call."""
import json
import os
import socket

SOCKET = {socket}


def _send(event):
    if not os.path.exists(SOCKET):
        return
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            stream.settimeout(1)
            stream.connect(SOCKET)
            stream.sendall(json.dumps(event).encode())
            stream.shutdown(socket.SHUT_WR)
    except OSError:
        pass


def _base(name, kwargs):
    return {{"hook_event_name": name, "session_id": str(kwargs.get("session_id") or kwargs.get("task_id") or ""), "cwd": os.getcwd()}}


def register(ctx):
    def started(*args, **kwargs):
        _send(_base("SessionStart", kwargs))

    def prompt(*args, **kwargs):
        event = _base("UserPromptSubmit", kwargs)
        event["prompt"] = str(kwargs.get("user_message") or "")
        _send(event)

    def before(tool_name=None, args=None, *rest, **kwargs):
        event = _base("PreToolUse", kwargs)
        event["tool_name"] = str(tool_name or kwargs.get("tool_name") or "")
        event["tool_input"] = args if isinstance(args, dict) else {{}}
        _send(event)
        return None

    def after(tool_name=None, args=None, result=None, *rest, **kwargs):
        failed = str(kwargs.get("status") or "") in ("error", "blocked")
        event = _base("PostToolUseFailure" if failed else "PostToolUse", kwargs)
        event["tool_name"] = str(tool_name or kwargs.get("tool_name") or "")
        event["tool_input"] = args if isinstance(args, dict) else {{}}
        _send(event)

    def finished(*args, **kwargs):
        event = _base("Stop", kwargs)
        event["last_assistant_message"] = str(kwargs.get("assistant_response") or "")
        _send(event)

    def ended(*args, **kwargs):
        _send(_base("SessionEnd", kwargs))

    for name, callback in (("on_session_start", started), ("pre_llm_call", prompt), ("pre_tool_call", before), ("post_tool_call", after), ("post_llm_call", finished), ("on_session_finalize", ended)):
        try:
            ctx.register_hook(name, callback)
        except Exception:
            pass
"#,
        socket = literal(socket)
    );
    (manifest.as_bytes().to_vec(), code.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_s_file_is_its_own_and_never_fails_a_tool() {
        let bytes = copilot(Path::new("/x/buntini/copilot.sock"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["version"], 1);
        assert!(value["hooks"].get("PermissionRequest").is_none(), "Copilot asks about every tool, rules or not");
        for (event, hooks) in value["hooks"].as_object().unwrap() {
            let command = hooks[0]["bash"].as_str().unwrap();
            assert!(!command.contains("exec ") && command.ends_with("exit 0"), "{event}: a failed hook would refuse the tool");
        }
        assert!(value["hooks"]["postToolUse"][0]["bash"].as_str().unwrap().contains("'postToolUse'"));
        assert_eq!(value["hooks"]["sessionEnd"][0]["timeoutSec"], 3);
    }

    #[test]
    fn plugins_carry_the_socket_as_a_quoted_string() {
        let socket = Path::new("/Users/o\"neil/buntini/opencode.sock");
        let js = String::from_utf8(opencode(socket)).unwrap();
        assert!(js.contains(r#"const SOCKET = "/Users/o\"neil/buntini/opencode.sock""#));
        let ts = String::from_utf8(amp(socket)).unwrap();
        assert!(ts.contains(r#"const SOCKET = "/Users/o\"neil/buntini/opencode.sock""#));
        // It never answers for a tool: no `tool.call` handler, nothing returned.
        assert!(!ts.contains("tool.call") && !ts.contains("action"), "{ts}");
        assert!(ts.contains("amp.on(\"tool.result\""));
        let (manifest, code) = hermes(socket);
        assert!(String::from_utf8(manifest).unwrap().starts_with("name: buntini\n"));
        assert!(String::from_utf8(code).unwrap().contains(r#"SOCKET = "/Users/o\"neil/buntini/opencode.sock""#));
    }

    /// The Hermes plugin, run by this Mac's Python against a socket: it sends what it says.
    #[test]
    fn the_hermes_plugin_sends_events() {
        if !Path::new("/usr/bin/python3").exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("h.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let (_, code) = hermes(&socket);
        std::fs::write(dir.path().join("plugin.py"), code).unwrap();
        let script = "import sys; sys.path.insert(0, sys.argv[1]); import plugin\nclass Ctx:\n    def __init__(self): self.hooks = {}\n    def register_hook(self, name, callback): self.hooks[name] = callback\nctx = Ctx(); plugin.register(ctx)\nctx.hooks['pre_tool_call']('terminal', {'command': 'ls'}, session_id='h1')\n";
        let run = std::process::Command::new("/usr/bin/python3").arg("-I").arg("-c").arg(script).arg(dir.path()).status().unwrap();
        assert!(run.success());
        let (mut stream, _) = listener.accept().unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut stream, &mut text).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!((value["hook_event_name"].as_str(), value["session_id"].as_str(), value["tool_name"].as_str()), (Some("PreToolUse"), Some("h1"), Some("terminal")));
        assert_eq!(value["tool_input"]["command"], "ls");
    }
}
