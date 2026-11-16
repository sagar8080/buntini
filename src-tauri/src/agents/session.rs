//! What each agent session is doing, step by step, from its hook events: the step ticker,
//! the diffs of its edits and whether it is working, waiting, done or failed. Pure, so the
//! rules are tested without sockets. After Coucou's ticker and SessionBoard (MIT).
use std::collections::VecDeque;
use std::path::Path;

use serde_json::Value;

use super::diff::{self, Diff};
use crate::i18n::{t, tf};

/// Steps kept per session.
pub const MAX_STEPS: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub label: String,
    pub detail: Option<String>,
    /// The tool call it came from, so its result lands on the same step.
    pub tool: String,
    pub input_key: String,
    pub diff: Option<Diff>,
    pub file: Option<String>,
    pub failed: bool,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Working,
    Done,
    Error,
}

/// Where the session's terminal is, from the hook's environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    pub term_program: String,
    pub bundle: String,
    pub iterm_session: String,
    /// `claude-desktop` for Claude Code in the Claude app's Code tab.
    pub entrypoint: String,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub agent: String,
    pub id: String,
    pub cwd: String,
    pub steps: VecDeque<Step>,
    pub phase: Phase,
    /// When the current turn began, in board milliseconds.
    pub turn_started_ms: Option<u64>,
    pub last_ms: u64,
    pub host: Host,
    pub message: String,
    /// The turn that just ended was long enough to say so.
    pub announce: bool,
    /// The agent is asking in its own terminal (agents Buntini can't answer for).
    pub asking: bool,
    /// The prompt the current turn began with: agents that report it before every model
    /// call (Antigravity, Hermes) send it again and again.
    last_prompt: String,
}

impl Session {
    pub fn new(agent: &str, id: &str, cwd: &str, now_ms: u64) -> Session {
        Session {
            agent: agent.into(),
            id: id.into(),
            cwd: cwd.into(),
            steps: VecDeque::new(),
            phase: Phase::Idle,
            turn_started_ms: None,
            last_ms: now_ms,
            host: Host::default(),
            message: String::new(),
            announce: false,
            asking: false,
            last_prompt: String::new(),
        }
    }

    pub fn project(&self) -> String {
        Path::new(&self.cwd).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn push(&mut self, step: Step) {
        self.steps.push_back(step);
        while self.steps.len() > MAX_STEPS {
            self.steps.pop_front();
        }
    }

    /// The person sent a prompt: a new turn.
    pub fn prompt(&mut self, prompt: &str, now_ms: u64) {
        if self.phase == Phase::Working && !prompt.is_empty() && prompt == self.last_prompt {
            self.last_ms = now_ms;
            return;
        }
        self.last_prompt = prompt.to_string();
        self.asking = false;
        self.phase = Phase::Working;
        self.turn_started_ms = Some(now_ms);
        self.last_ms = now_ms;
        self.message.clear();
        self.announce = false;
        let text = first_line(prompt, 60);
        if !text.is_empty() {
            self.push(Step { label: tf("agents.step.prompt", &[("text", &text)]), detail: None, tool: String::new(), input_key: String::new(), diff: None, file: None, failed: false, done: true });
        }
    }

    /// A tool is about to run.
    pub fn before(&mut self, tool: &str, input: &Value, now_ms: u64) {
        self.asking = false;
        self.phase = Phase::Working;
        self.turn_started_ms.get_or_insert(now_ms);
        self.last_ms = now_ms;
        let (label, detail) = label(tool, input, &self.cwd);
        self.push(Step { label, detail, tool: tool.into(), input_key: key(input), diff: None, file: None, failed: false, done: false });
    }

    /// A tool ran (or failed). Edits get their diff here, which is returned as the file and
    /// the lines added and removed.
    pub fn after(&mut self, tool: &str, input: &Value, failed: bool, now_ms: u64) -> Option<(String, usize, usize)> {
        self.last_ms = now_ms;
        self.asking = false;
        let input_key = key(input);
        // Copilot names a tool `Bash` before it runs and `bash` after.
        let found = self.steps.iter().rposition(|step| !step.done && step.tool.eq_ignore_ascii_case(tool) && step.input_key == input_key);
        let index = match found {
            Some(index) => index,
            None => {
                // No PreToolUse seen (the hook came later): add the step now.
                self.before(tool, input, now_ms);
                self.steps.len() - 1
            }
        };
        let cwd = self.cwd.clone();
        let step = &mut self.steps[index];
        step.done = true;
        step.failed = failed;
        if failed {
            return None;
        }
        let (file, diff) = edit_diff(tool, input, &cwd)?;
        let changed = (file.clone(), diff.added, diff.removed);
        step.file = Some(file);
        step.diff = Some(diff);
        Some(changed)
    }

    /// The turn ended.
    pub fn stop(&mut self, message: &str, failed: bool, now_ms: u64) {
        self.asking = false;
        self.phase = if failed { Phase::Error } else { Phase::Done };
        self.last_ms = now_ms;
        self.message = first_line(message, 160);
        for step in self.steps.iter_mut() {
            step.done = true;
        }
    }

    /// How long the turn that just ended took.
    pub fn turn_ms(&self, now_ms: u64) -> Option<u64> {
        self.turn_started_ms.map(|start| now_ms.saturating_sub(start))
    }

}

/// A stable key for a tool call's input, so the result finds its step.
fn key(input: &Value) -> String {
    let text = serde_json::to_string(input).unwrap_or_default();
    text.chars().take(400).collect()
}

/// Test runners, by the start of the command.
const TEST_RUNNERS: &[&str] = &[
    "cargo test", "cargo nextest", "npm test", "npm run test", "pnpm test", "yarn test", "bun test", "npx jest", "npx vitest", "jest", "vitest",
    "pytest", "python -m pytest", "python3 -m pytest", "python -m unittest", "python3 -m unittest", "go test", "swift test", "xcodebuild test",
    "bundle exec rspec", "rspec", "mvn test", "gradle test", "./gradlew test", "dotnet test", "mix test", "phpunit", "deno test",
];

/// The step's words: what the agent does, and to what.
pub fn label(tool: &str, input: &Value, cwd: &str) -> (String, Option<String>) {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let file = |key: &str| {
        // Agents name the path differently: Claude `file_path`, OpenCode `filePath`,
        // Antigravity `TargetFile` or `AbsolutePath`.
        let path = [key, "filePath", "path", "TargetFile", "AbsolutePath"].iter().map(|key| text(key)).find(|path| !path.is_empty()).unwrap_or_default();
        let name = Path::new(&path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        (name, relative(&path, cwd))
    };
    match tool {
        "Bash" | "shell" | "Shell" | "local_shell" | "exec_command" | "unified_exec" | "run_command" | "run_shell_command" | "bash" | "terminal" => {
            let command = match input.get("command").or_else(|| input.get("cmd")).or_else(|| input.get("CommandLine")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                Some(Value::String(command)) => command.trim().to_string(),
                _ => String::new(),
            };
            let short = first_line(&command, 48);
            let plain = strip_prefixes(&command);
            let runs = |runner: &&str| plain == *runner || plain.strip_prefix(runner).is_some_and(|rest| rest.starts_with(' '));
            if TEST_RUNNERS.iter().any(runs) {
                (tf("agents.step.tests", &[("command", &first_line(plain, 40))]), Some(command))
            } else {
                (tf("agents.step.runs", &[("command", &short)]), Some(command))
            }
        }
        "Read" | "read" | "read_file" | "view" | "view_file" => {
            let (name, path) = file("file_path");
            (if name.is_empty() { t("agents.step.readsAFile") } else { tf("agents.step.reads", &[("file", &name)]) }, Some(path))
        }
        "Edit" | "MultiEdit" | "edit" | "replace" | "str_replace_editor" | "edit_file" | "patch" | "replace_file_content" | "multi_replace_file_content" => {
            let (name, path) = file("file_path");
            (if name.is_empty() { t("agents.step.editsAFile") } else { tf("agents.step.edits", &[("file", &name)]) }, Some(path))
        }
        "Write" | "write" | "write_file" | "write_to_file" | "create" => {
            let (name, path) = file("file_path");
            (if name.is_empty() { t("agents.step.writesAFile") } else { tf("agents.step.writes", &[("file", &name)]) }, Some(path))
        }
        "NotebookEdit" => {
            let (name, path) = file("notebook_path");
            (if name.is_empty() { t("agents.step.editsANotebook") } else { tf("agents.step.edits", &[("file", &name)]) }, Some(path))
        }
        "apply_patch" => (t("agents.step.editsFiles"), None),
        "Glob" | "Grep" | "glob" | "grep" | "search_file_content" | "list_directory" | "LS" => (t("agents.step.searchesFiles"), Some(text("pattern"))),
        "WebFetch" | "web_fetch" => (t("agents.step.opensAPage"), Some(text("url"))),
        "WebSearch" | "google_web_search" | "web_search" => (tf("agents.step.searches", &[("query", &first_line(&text("query"), 40))]), None),
        "Task" | "Agent" => (t("agents.step.handsOffATask"), Some(first_line(&text("description"), 80))),
        "TodoWrite" | "update_plan" => (t("agents.step.plansItsSteps"), None),
        "AskUserQuestion" => (t("agents.step.asksYou"), None),
        "ExitPlanMode" => (t("agents.step.planReady"), None),
        tool if tool.starts_with("mcp__") => {
            let mut parts = tool.splitn(3, "__").skip(1);
            let server = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default().replace(['_', '-'], " ");
            (tf("agents.step.uses", &[("tool", &name)]), Some(server.to_string()))
        }
        "" => (t("agents.step.works"), None),
        tool => (tf("agents.step.uses", &[("tool", &tool.replace(['_', '-'], " "))]), None),
    }
}

/// A command with its `cd …&&` and environment settings taken off the front.
fn strip_prefixes(command: &str) -> &str {
    let mut rest = command.trim();
    loop {
        if let Some(index) = rest.find("&&").filter(|_| rest.starts_with("cd ")) {
            rest = rest[index + 2..].trim_start();
            continue;
        }
        let first = rest.split_whitespace().next().unwrap_or_default();
        if first.contains('=') && !first.starts_with('=') && !first.contains('/') {
            rest = rest[first.len()..].trim_start();
            continue;
        }
        return rest;
    }
}

/// The file and diff an edit tool makes, from its input.
pub fn edit_diff(tool: &str, input: &Value, cwd: &str) -> Option<(String, Diff)> {
    let text = |value: &Value, keys: &[&str]| keys.iter().filter_map(|key| value.get(*key).and_then(Value::as_str)).next().unwrap_or_default().to_string();
    let path = || relative(&text(input, &["file_path", "filePath", "TargetFile"]), cwd);
    let old = |value: &Value| text(value, &["old_string", "oldString"]);
    let new = |value: &Value| text(value, &["new_string", "newString"]);
    match tool {
        "Edit" | "edit" | "replace" => Some((path(), diff::diff(&old(input), &new(input)))),
        "MultiEdit" => {
            let edits = input.get("edits")?.as_array()?;
            let mut total = Diff::default();
            for edit in edits {
                let one = diff::diff(&old(edit), &new(edit));
                total.added += one.added;
                total.removed += one.removed;
                if !total.lines.is_empty() && !one.lines.is_empty() {
                    total.lines.push(diff::Line { kind: diff::Kind::Gap, text: String::new() });
                }
                total.lines.extend(one.lines);
            }
            Some((path(), total))
        }
        "Write" | "write" | "write_file" | "write_to_file" | "create" => {
            let content = text(input, &["content", "CodeContent"]);
            Some((path(), diff::diff("", &content)))
        }
        "apply_patch" => {
            let patch = match input.get("command").or_else(|| input.get("patch")).or_else(|| input.get("input")) {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
                Some(Value::String(patch)) => patch.clone(),
                _ => return None,
            };
            Some(patch_diff(&patch, cwd))
        }
        _ => None,
    }
}

/// Counts and lines from an `apply_patch` patch.
fn patch_diff(patch: &str, cwd: &str) -> (String, Diff) {
    let mut files = Vec::new();
    let mut out = Diff::default();
    for line in patch.lines() {
        if let Some(path) = ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|prefix| line.trim().strip_prefix(prefix)) {
            files.push(relative(path.trim(), cwd));
        } else if line.starts_with("***") || line.starts_with("@@") {
            continue;
        } else if let Some(text) = line.strip_prefix('+') {
            out.added += 1;
            out.lines.push(diff::Line { kind: diff::Kind::Add, text: text.into() });
        } else if let Some(text) = line.strip_prefix('-') {
            out.removed += 1;
            out.lines.push(diff::Line { kind: diff::Kind::Remove, text: text.into() });
        }
    }
    out.lines.truncate(400);
    (files.join(", "), out)
}

fn relative(path: &str, cwd: &str) -> String {
    if cwd.is_empty() {
        return path.to_string();
    }
    match Path::new(path).strip_prefix(cwd) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

pub fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().map(|line| line.trim().trim_start_matches('#').trim()).find(|line| !line.is_empty()).unwrap_or_default();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_say_what_the_agent_does() {
        let cwd = "/Users/me/shop";
        assert_eq!(label("Read", &json!({ "file_path": "/Users/me/shop/src/lib.rs" }), cwd), ("Reads · lib.rs".into(), Some("src/lib.rs".into())));
        assert_eq!(label("Bash", &json!({ "command": "ls -la" }), cwd).0, "Runs · ls -la");
        assert_eq!(label("Bash", &json!({ "command": "cd src-tauri && cargo test --locked" }), cwd).0, "Tests · cargo test --locked");
        assert_eq!(label("Bash", &json!({ "command": "RUST_LOG=1 cargo test" }), cwd).0, "Tests · cargo test");
        assert_eq!(label("exec_command", &json!({ "cmd": ["npm", "test"] }), cwd).0, "Tests · npm test");
        assert_eq!(label("Edit", &json!({ "file_path": "/Users/me/shop/a.rs" }), cwd).0, "Edits · a.rs");
        assert_eq!(label("mcp__github__create_issue", &json!({}), cwd).0, "Uses · create issue");
        assert_eq!(label("Bash", &json!({ "command": "cargo testify" }), cwd).0, "Runs · cargo testify");
    }

    #[test]
    fn a_result_lands_on_its_own_step_with_its_diff() {
        let mut session = Session::new("claude", "s1", "/Users/me/shop", 0);
        session.prompt("Fix the bug in the cart\nthen run tests", 0);
        let edit = json!({ "file_path": "/Users/me/shop/cart.rs", "old_string": "let total = 0;", "new_string": "let total = 1;\nlet tax = 0;" });
        session.before("Edit", &edit, 10);
        session.before("Bash", &json!({ "command": "cargo test" }), 20);
        session.after("Edit", &edit, false, 30);
        session.after("Bash", &json!({ "command": "cargo test" }), true, 40);
        let labels: Vec<_> = session.steps.iter().map(|step| step.label.as_str()).collect();
        assert_eq!(labels, ["“Fix the bug in the cart”", "Edits · cart.rs", "Tests · cargo test"]);
        let edited = &session.steps[1];
        assert_eq!(edited.file.as_deref(), Some("cart.rs"));
        assert_eq!(edited.diff.as_ref().map(|diff| (diff.added, diff.removed)), Some((2, 1)));
        assert!(session.steps[2].failed);
    }

    #[test]
    fn a_result_without_its_start_still_shows() {
        let mut session = Session::new("codex", "s1", "", 0);
        session.after("apply_patch", &json!({ "command": ["apply_patch", "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n+more\n*** End Patch"] }), false, 5);
        assert_eq!(session.steps.len(), 1);
        assert_eq!(session.steps[0].diff.as_ref().map(|diff| (diff.added, diff.removed)), Some((2, 1)));
        assert_eq!(session.steps[0].file.as_deref(), Some("a.txt"));
    }

    #[test]
    fn other_agents_tools_read_the_same() {
        assert_eq!(label("edit", &json!({ "filePath": "/w/a.ts" }), "/w").0, "Edits · a.ts");
        assert_eq!(label("run_command", &json!({ "CommandLine": "go test ./..." }), "/w").0, "Tests · go test ./...");
        assert_eq!(label("Shell", &json!({ "command": "ls" }), "/w").0, "Runs · ls");
        let (file, diff) = edit_diff("edit", &json!({ "filePath": "/w/a.ts", "oldString": "a", "newString": "b" }), "/w").unwrap();
        assert_eq!((file.as_str(), diff.added, diff.removed), ("a.ts", 1, 1));
    }

    #[test]
    fn a_prompt_sent_again_in_the_same_turn_is_one_step() {
        let mut session = Session::new("antigravity", "a1", "", 0);
        session.prompt("list files", 0);
        session.before("run_command", &json!({ "CommandLine": "ls" }), 1);
        session.prompt("list files", 2);
        assert_eq!(session.steps.len(), 2);
        assert_eq!(session.turn_started_ms, Some(0));
        session.stop("", false, 3);
        session.prompt("list files", 4);
        assert_eq!(session.steps.len(), 3, "a new turn with the same words is a new turn");
    }

    #[test]
    fn only_the_last_twenty_steps_are_kept() {
        let mut session = Session::new("claude", "s1", "", 0);
        for n in 0..30 {
            session.before("Read", &json!({ "file_path": format!("/f{n}.rs") }), n);
        }
        assert_eq!(session.steps.len(), MAX_STEPS);
        assert_eq!(session.steps.front().unwrap().label, "Reads · f10.rs");
    }

    #[test]
    fn a_turn_ends_done_or_failed() {
        let mut session = Session::new("claude", "s1", "", 0);
        session.prompt("go", 1_000);
        session.before("Read", &json!({ "file_path": "/a" }), 2_000);
        session.stop("## Done\nAll good", false, 31_000);
        assert_eq!(session.phase, Phase::Done);
        assert_eq!(session.message, "Done");
        assert_eq!(session.turn_ms(31_000), Some(30_000));
        assert!(session.steps.iter().all(|step| step.done));
        session.stop("", true, 32_000);
        assert_eq!(session.phase, Phase::Error);
    }
}
