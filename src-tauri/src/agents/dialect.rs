//! Each agent's hook events, in Claude Code's words: `hook_event_name`, `session_id`, `cwd`,
//! `tool_name`, `tool_input`, `prompt` and `last_assistant_message`. Agents that leave the
//! event's name out of the payload get it from the hook's first line (`hint`). Field names
//! from the agents' own documentation (Copilot CLI, Gemini CLI, Antigravity, Cursor).
use serde_json::{Map, Value};

use super::Agent;

/// The event in Claude Code's shape, and the agent it really came from: Cursor runs Claude
/// Code's hooks too, and says so in its payload.
pub fn normalize(agent: Agent, hint: &str, payload: &Value) -> Option<(Agent, Value)> {
    let object = payload.as_object()?;
    let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let first = |keys: &[&str]| keys.iter().map(|key| text(key)).find(|value| !value.is_empty()).unwrap_or_default();
    let mut out = Map::new();
    let mut from = agent;
    let name = match agent {
        // Copilot names a permission request `hookName`, in camelCase.
        Agent::Copilot => copilot_event(&first(&["hook_event_name", "hookName"]).if_empty(hint)),
        Agent::Gemini => gemini_event(&first(&["hook_event_name"]).if_empty(hint), object),
        Agent::Antigravity => antigravity_event(hint, object)?,
        _ => first(&["hook_event_name"]).if_empty(hint),
    };
    if name.is_empty() {
        return None;
    }
    if object.contains_key("cursor_version") {
        from = Agent::Cursor;
    }
    out.insert("hook_event_name".into(), Value::String(name));
    out.insert("session_id".into(), Value::String(first(&["session_id", "sessionId", "conversation_id", "conversationId"])));
    let roots = |key: &str| object.get(key).and_then(Value::as_array).and_then(|roots| roots.first()).and_then(Value::as_str).map(str::to_string);
    let cwd = first(&["cwd"]).if_empty(&roots("workspace_roots").or_else(|| roots("workspacePaths")).unwrap_or_default());
    out.insert("cwd".into(), Value::String(cwd));
    let call = object.get("toolCall");
    let tool = first(&["tool_name", "toolName"]).if_empty(call.and_then(|call| call.get("name")).and_then(Value::as_str).unwrap_or_default());
    out.insert("tool_name".into(), Value::String(tool));
    let input = object
        .get("tool_input")
        .or_else(|| object.get("toolArgs"))
        .or_else(|| object.get("toolInput"))
        .or_else(|| call.and_then(|call| call.get("args")))
        .cloned()
        .unwrap_or(Value::Null);
    // Some agents send the arguments as a JSON string.
    let input = match input {
        Value::String(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
        other => other,
    };
    out.insert("tool_input".into(), input);
    out.insert("prompt".into(), Value::String(first(&["prompt", "lastUserInput"])));
    out.insert("last_assistant_message".into(), Value::String(first(&["last_assistant_message", "prompt_response"])));
    for key in ["permission_suggestions", "notification_type", "notificationType"] {
        if let Some(value) = object.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    Some((from, Value::Object(out)))
}

trait OrElse {
    fn if_empty(self, other: &str) -> String;
}

impl OrElse for String {
    fn if_empty(self, other: &str) -> String {
        if self.is_empty() { other.to_string() } else { self }
    }
}

/// Copilot CLI: camelCase names, or Claude's own when its config uses PascalCase.
fn copilot_event(name: &str) -> String {
    match name {
        "sessionStart" => "SessionStart",
        "userPromptSubmitted" => "UserPromptSubmit",
        "preToolUse" => "PreToolUse",
        "permissionRequest" => "PermissionRequest",
        "postToolUse" => "PostToolUse",
        "postToolUseFailure" => "PostToolUseFailure",
        "agentStop" => "Stop",
        "sessionEnd" => "SessionEnd",
        "notification" => "Notification",
        other => other,
    }
    .to_string()
}

/// Gemini CLI: before and after the agent and its tools. A tool that failed carries an error.
fn gemini_event(name: &str, object: &Map<String, Value>) -> String {
    match name {
        "BeforeAgent" => "UserPromptSubmit".into(),
        "BeforeTool" => "PreToolUse".into(),
        "AfterTool" => {
            let error = object.get("tool_response").and_then(|response| response.get("error"));
            let failed = error.is_some_and(|error| !error.is_null() && error.as_str() != Some(""));
            (if failed { "PostToolUseFailure" } else { "PostToolUse" }).into()
        }
        "AfterAgent" => "Stop".into(),
        other => other.into(),
    }
}

/// Antigravity: its payloads don't name the event, so the hook does. Its turn ends when the
/// agent is fully idle.
fn antigravity_event(hint: &str, object: &Map<String, Value>) -> Option<String> {
    Some(match hint {
        "PreInvocation" => "UserPromptSubmit".into(),
        "PostToolUse" if object.get("error").is_some_and(|error| !error.is_null() && error.as_str() != Some("")) => "PostToolUseFailure".into(),
        "Stop" if object.get("fullyIdle").and_then(Value::as_bool) == Some(false) => return None,
        "" => return None,
        other => other.into(),
    })
}

/// What the hook prints back for a decision, in the agent's own words.
pub fn reply(agent: Agent, decision: Value) -> Value {
    match agent {
        // Copilot reads `behavior` at the top; with Claude-style hooks it reads Claude's shape.
        Agent::Copilot => {
            let mut flat = decision.as_object().cloned().unwrap_or_default();
            flat.insert("hookSpecificOutput".into(), serde_json::json!({ "hookEventName": "PermissionRequest", "decision": decision }));
            Value::Object(flat)
        }
        _ => serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get(value: &Value, key: &str) -> String {
        value[key].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn copilot_s_camel_case_reads_as_claude_s() {
        let (agent, event) = normalize(Agent::Copilot, "postToolUse", &json!({ "sessionId": "c1", "cwd": "/w", "toolName": "bash", "toolArgs": "{\"command\":\"ls\"}", "timestamp": 1 })).unwrap();
        assert_eq!(agent, Agent::Copilot);
        assert_eq!((get(&event, "hook_event_name"), get(&event, "session_id"), get(&event, "tool_name")), ("PostToolUse".into(), "c1".into(), "bash".into()));
        assert_eq!(event["tool_input"], json!({ "command": "ls" }));
        let (_, pascal) = normalize(Agent::Copilot, "", &json!({ "hook_event_name": "PreToolUse", "session_id": "c1", "tool_name": "Bash", "tool_input": { "command": "rm x" } })).unwrap();
        assert_eq!(get(&pascal, "hook_event_name"), "PreToolUse");
        // As Copilot CLI 1.0.105 sends it, captured on this Mac.
        let (_, asked) = normalize(Agent::Copilot, "", &json!({ "hookName": "permissionRequest", "sessionId": "c1", "cwd": "/w", "toolName": "bash", "toolInput": { "command": "echo hi > dbg.txt" }, "permissionSuggestions": [] })).unwrap();
        assert_eq!((get(&asked, "hook_event_name"), get(&asked, "tool_name")), ("PermissionRequest".into(), "bash".into()));
        assert_eq!(asked["tool_input"]["command"], "echo hi > dbg.txt");
    }

    #[test]
    fn gemini_s_turns_and_tools_read_as_claude_s() {
        let (_, start) = normalize(Agent::Gemini, "", &json!({ "hook_event_name": "BeforeAgent", "session_id": "g", "cwd": "/w", "prompt": "fix it" })).unwrap();
        assert_eq!((get(&start, "hook_event_name"), get(&start, "prompt")), ("UserPromptSubmit".into(), "fix it".into()));
        let (_, failed) = normalize(Agent::Gemini, "", &json!({ "hook_event_name": "AfterTool", "session_id": "g", "tool_name": "run_shell_command", "tool_response": { "error": "exit 1" } })).unwrap();
        assert_eq!(get(&failed, "hook_event_name"), "PostToolUseFailure");
        let (_, ran) = normalize(Agent::Gemini, "", &json!({ "hook_event_name": "AfterTool", "session_id": "g", "tool_response": { "error": null } })).unwrap();
        assert_eq!(get(&ran, "hook_event_name"), "PostToolUse");
        let (_, done) = normalize(Agent::Gemini, "", &json!({ "hook_event_name": "AfterAgent", "session_id": "g", "prompt_response": "All done" })).unwrap();
        assert_eq!((get(&done, "hook_event_name"), get(&done, "last_assistant_message")), ("Stop".into(), "All done".into()));
    }

    #[test]
    fn antigravity_events_are_named_by_the_hook() {
        let payload = json!({ "conversationId": "a1", "workspacePaths": ["/w/shop"], "toolCall": { "name": "run_command", "args": { "CommandLine": "ls" } }, "lastUserInput": "list it" });
        let (_, tool) = normalize(Agent::Antigravity, "PreToolUse", &payload).unwrap();
        assert_eq!((get(&tool, "hook_event_name"), get(&tool, "session_id"), get(&tool, "cwd"), get(&tool, "tool_name")), ("PreToolUse".into(), "a1".into(), "/w/shop".into(), "run_command".into()));
        assert_eq!(tool["tool_input"], json!({ "CommandLine": "ls" }));
        let (_, prompt) = normalize(Agent::Antigravity, "PreInvocation", &payload).unwrap();
        assert_eq!((get(&prompt, "hook_event_name"), get(&prompt, "prompt")), ("UserPromptSubmit".into(), "list it".into()));
        assert!(normalize(Agent::Antigravity, "Stop", &json!({ "conversationId": "a1", "fullyIdle": false })).is_none(), "still working");
        assert!(normalize(Agent::Antigravity, "", &payload).is_none());
    }

    #[test]
    fn cursor_running_claude_s_hooks_is_cursor() {
        let (agent, event) = normalize(Agent::Claude, "", &json!({ "hook_event_name": "PreToolUse", "conversation_id": "cur", "cursor_version": "2.1", "workspace_roots": ["/w/app"], "tool_name": "Shell", "tool_input": { "command": "ls" } })).unwrap();
        assert_eq!(agent, Agent::Cursor);
        assert_eq!((get(&event, "session_id"), get(&event, "cwd")), ("cur".into(), "/w/app".into()));
    }

    #[test]
    fn copilot_gets_its_decision_both_ways() {
        let line = reply(Agent::Copilot, json!({ "behavior": "allow" }));
        assert_eq!(line["behavior"], "allow");
        assert_eq!(line["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(reply(Agent::Claude, json!({ "behavior": "deny" })).get("behavior").is_none());
    }
}
