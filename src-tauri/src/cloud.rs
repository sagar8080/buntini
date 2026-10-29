//! Cloud models (Anthropic, OpenAI, Gemini) with the user's own key from the Keychain.
//!
//! Callers send the same generic payload the local workers take (`messages`, `tools`,
//! `max_tokens`); this module translates it per provider, streams the reply over SSE and
//! returns the same `Turn`. Spending is reserved before each call by `ModelHub` and
//! settled here from the usage the provider reports.
use crate::worker::Turn;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// Suggested model; the user can name any model their key can use.
    pub model: &'static str,
    /// Suggested list prices per million tokens (USD), as published in October 2026.
    pub input_per_m: f64,
    pub output_per_m: f64,
}

pub const PROVIDERS: [Provider; 4] = [
    Provider {
        id: "anthropic",
        name: "Anthropic",
        model: "claude-opus-5-5",
        input_per_m: 4.0,
        output_per_m: 20.0,
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        model: "gpt-5-mini",
        input_per_m: 0.25,
        output_per_m: 2.0,
    },
    Provider {
        id: "gemini",
        name: "Google Gemini",
        model: "gemini-2.5-flash",
        input_per_m: 0.30,
        output_per_m: 2.50,
    },
    // Your own ChatGPT plan through Sign in with ChatGPT: no key and no per-token cost;
    // the model comes from the account's own list.
    Provider {
        id: "chatgpt",
        name: "ChatGPT",
        model: "",
        input_per_m: 0.0,
        output_per_m: 0.0,
    },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// The usage table key, so cloud spending never mixes with search counts.
pub fn ledger_key(provider: &str) -> String {
    format!("cloud-{provider}")
}

pub fn model_valid(model: &str) -> bool {
    model.len() <= 100
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:/@".contains(c))
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

/// Dollar cost of `usage` at the user's per-million-token prices.
pub fn price(usage: Usage, input_per_m: f64, output_per_m: f64) -> f64 {
    usage.input as f64 * input_per_m / 1e6 + usage.output as f64 * output_per_m / 1e6
}

/// A cautious upper estimate before sending: about three characters per input token,
/// and the full reply allowance.
pub fn estimate(payload: &Value) -> Usage {
    let chars = payload["messages"].to_string().len() + payload["tools"].to_string().len();
    Usage {
        input: (chars / 3 + 64) as u64,
        output: payload["max_tokens"].as_u64().unwrap_or(1024),
    }
}

#[derive(Debug)]
pub enum Failure {
    /// The provider refused the request outright (bad key, unknown model, rate limit).
    /// Nothing was generated, so nothing is charged.
    Rejected(String),
    /// Anything after the request was accepted: the reservation is kept.
    Failed(String),
}

pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

fn text(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| {
        if value.is_null() {
            String::new()
        } else {
            value.to_string()
        }
    })
}

/// The one tool call an assistant message made, if any, as (name, arguments).
fn assistant_call(message: &Value) -> Option<(String, Value)> {
    let function = &message["tool_calls"][0]["function"];
    let name = function["name"].as_str()?.to_string();
    let arguments = match &function["arguments"] {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or_else(|_| json!({})),
        Value::Null => json!({}),
        other => other.clone(),
    };
    Some((name, arguments))
}

/// Gemini accepts a subset of JSON Schema: drop the keys it rejects.
fn gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "additionalProperties")
                .map(|(key, value)| (key.clone(), gemini_schema(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(gemini_schema).collect()),
        other => other.clone(),
    }
}

/// Builds the provider request from the generic payload.
pub fn build(provider: &str, model: &str, key: &str, payload: &Value) -> Result<Request, String> {
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let tools = payload["tools"].as_array().cloned().unwrap_or_default();
    let max_tokens = payload["max_tokens"].as_u64().unwrap_or(1024);
    let system = messages
        .iter()
        .filter(|m| m["role"] == "system")
        .map(|m| text(&m["content"]))
        .collect::<Vec<_>>()
        .join("\n\n");
    match provider {
        "openai" => {
            let mut out = Vec::new();
            let mut call = 0;
            for message in &messages {
                match message["role"].as_str().unwrap_or("") {
                    "assistant" => match assistant_call(message) {
                        Some((name, arguments)) => {
                            call += 1;
                            out.push(json!({"role":"assistant","content":text(&message["content"]),"tool_calls":[{"id":format!("call_{call}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}));
                        }
                        None => out.push(json!({"role":"assistant","content":text(&message["content"])})),
                    },
                    "tool" => out.push(json!({"role":"tool","tool_call_id":format!("call_{call}"),"content":text(&message["content"])})),
                    role => out.push(json!({"role":role,"content":text(&message["content"])})),
                }
            }
            let mut body = json!({"model":model,"messages":out,"stream":true,"stream_options":{"include_usage":true},"max_completion_tokens":max_tokens});
            if !tools.is_empty() {
                body["tools"] = tools
                    .iter()
                    .map(|tool| json!({"type":"function","function":tool}))
                    .collect();
                body["parallel_tool_calls"] = false.into();
            }
            // GPT-5 models reason before answering; a light touch suits quick chat.
            if model.starts_with("gpt-5") {
                body["reasoning_effort"] = "low".into();
            }
            Ok(Request {
                url: "https://api.openai.com/v1/chat/completions".into(),
                headers: vec![("authorization", format!("Bearer {key}"))],
                body,
            })
        }
        "anthropic" => {
            let mut out: Vec<Value> = Vec::new();
            let mut call = 0;
            for message in &messages {
                match message["role"].as_str().unwrap_or("") {
                    "system" => {}
                    "assistant" => {
                        // Replies from this conversation go back exactly as received, thinking included.
                        let content = match &message["provider_content"] {
                            Value::Array(blocks) => blocks.clone(),
                            _ => {
                                let mut blocks = Vec::new();
                                let said = text(&message["content"]);
                                if !said.trim().is_empty() {
                                    blocks.push(json!({"type":"text","text":said}));
                                }
                                if let Some((name, arguments)) = assistant_call(message) {
                                    blocks.push(json!({"type":"tool_use","id":format!("toolu_buntini_{}",call+1),"name":name,"input":arguments}));
                                }
                                blocks
                            }
                        };
                        if content.iter().any(|block| block["type"] == "tool_use") {
                            call += 1;
                        }
                        if !content.is_empty() {
                            out.push(json!({"role":"assistant","content":content}));
                        }
                    }
                    "tool" => {
                        let id = out
                            .iter()
                            .rev()
                            .find_map(|m| {
                                m["content"]
                                    .as_array()?
                                    .iter()
                                    .find(|b| b["type"] == "tool_use")?["id"]
                                    .as_str()
                                    .map(str::to_string)
                            })
                            .unwrap_or_else(|| format!("toolu_buntini_{call}"));
                        out.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":text(&message["content"])}]}));
                    }
                    _ => out.push(json!({"role":"user","content":text(&message["content"])})),
                }
            }
            let mut body = json!({"model":model,"max_tokens":max_tokens,"system":system,"messages":out,"stream":true});
            if !tools.is_empty() {
                body["tools"] = tools.iter().map(|tool| json!({"name":tool["name"],"description":tool["description"],"input_schema":tool["parameters"]})).collect();
                body["tool_choice"] = json!({"type":"auto","disable_parallel_tool_use":true});
            }
            let mut headers = vec![
                ("x-api-key", key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ];
            // Current models think adaptively; low effort keeps a chat companion quick and cheap.
            if [
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-haiku-5",
                "claude-fable-5",
                "claude-opus-4-8",
                "claude-opus-4-7",
                "claude-opus-4-6",
                "claude-sonnet-4-6",
            ]
            .iter()
            .any(|prefix| model.starts_with(prefix))
            {
                body["output_config"] = json!({"effort":"low"});
            }
            // If a safety classifier declines, the API re-runs the request on Anthropic's recommended fallback model.
            if ["claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"]
                .iter()
                .any(|prefix| model.starts_with(prefix))
            {
                body["fallbacks"] = "default".into();
                headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".into()));
            }
            Ok(Request {
                url: "https://api.anthropic.com/v1/messages".into(),
                headers,
                body,
            })
        }
        "gemini" => {
            let mut contents: Vec<Value> = Vec::new();
            let mut last_call = String::new();
            for message in &messages {
                match message["role"].as_str().unwrap_or("") {
                    "system" => {}
                    "assistant" => {
                        let parts = match &message["provider_content"] {
                            Value::Array(parts) => parts.clone(),
                            _ => {
                                let mut parts = Vec::new();
                                let said = text(&message["content"]);
                                if !said.trim().is_empty() {
                                    parts.push(json!({"text":said}));
                                }
                                if let Some((name, arguments)) = assistant_call(message) {
                                    parts.push(json!({"functionCall":{"name":name,"args":arguments}}));
                                }
                                parts
                            }
                        };
                        if let Some(name) = parts.iter().find_map(|part| part["functionCall"]["name"].as_str()) {
                            last_call = name.to_string();
                        }
                        if !parts.is_empty() {
                            contents.push(json!({"role":"model","parts":parts}));
                        }
                    }
                    "tool" => contents.push(json!({"role":"user","parts":[{"functionResponse":{"name":last_call,"response":{"content":text(&message["content"])}}}]})),
                    _ => contents.push(json!({"role":"user","parts":[{"text":text(&message["content"])}]})),
                }
            }
            let mut body =
                json!({"contents":contents,"generationConfig":{"maxOutputTokens":max_tokens}});
            if !system.is_empty() {
                body["systemInstruction"] = json!({"parts":[{"text":system}]});
            }
            if !tools.is_empty() {
                body["tools"] = json!([{"functionDeclarations":tools.iter().map(|tool| json!({"name":tool["name"],"description":tool["description"],"parameters":gemini_schema(&tool["parameters"])})).collect::<Vec<_>>()}]);
            }
            if !model_valid(model) || model.contains('/') {
                return Err("Enter a Gemini model name such as gemini-2.5-flash.".into());
            }
            // The key goes in a header, never in the URL where logs could keep it.
            Ok(Request { url: format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"), headers: vec![("x-goog-api-key", key.to_string())], body })
        }
        "chatgpt" => {
            // OpenAI's Responses API as Sign in with ChatGPT requires it: system text goes
            // in `instructions`, nothing is stored, and the reply always streams.
            let mut input: Vec<Value> = Vec::new();
            let mut call = 0;
            for message in &messages {
                match message["role"].as_str().unwrap_or("") {
                    "system" => {}
                    "assistant" => match &message["provider_content"] {
                        Value::Array(items) => {
                            if items.iter().any(|item| item["type"] == "function_call") {
                                call += 1;
                            }
                            input.extend(items.iter().cloned());
                        }
                        _ => {
                            let said = text(&message["content"]);
                            if !said.trim().is_empty() {
                                input.push(json!({"role":"assistant","content":said}));
                            }
                            if let Some((name, arguments)) = assistant_call(message) {
                                call += 1;
                                input.push(json!({"type":"function_call","call_id":format!("call_{call}"),"name":name,"arguments":arguments.to_string()}));
                            }
                        }
                    },
                    "tool" => {
                        let id = input
                            .iter()
                            .rev()
                            .find(|item| item["type"] == "function_call")
                            .and_then(|item| item["call_id"].as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_{call}"));
                        input.push(json!({"type":"function_call_output","call_id":id,"output":text(&message["content"])}));
                    }
                    _ => input.push(json!({"role":"user","content":text(&message["content"])})),
                }
            }
            if model.is_empty() || !model_valid(model) {
                return Err("Choose a ChatGPT model in Settings → Brain.".into());
            }
            let mut body = json!({"model":model,"instructions":system,"input":input,"stream":true,"store":false,"include":["reasoning.encrypted_content"]});
            if !tools.is_empty() {
                body["tools"] = tools.iter().map(|tool| json!({"type":"function","name":tool["name"],"description":tool["description"],"parameters":tool["parameters"]})).collect();
                body["tool_choice"] = "auto".into();
                body["parallel_tool_calls"] = false.into();
            }
            let _ = max_tokens; // A reply-length cap isn't accepted on this route.
            Ok(Request {
                url: format!("{}/responses", crate::chatgpt::RESOURCE),
                headers: vec![("authorization", format!("Bearer {key}"))],
                body,
            })
        }
        _ => Err("Choose Anthropic, OpenAI, Gemini or ChatGPT.".into()),
    }
}

const LINE_LIMIT: usize = 262_144;
const EVENT_LIMIT: usize = 1_048_576;
const STREAM_LIMIT: usize = 8 * 1_048_576;

/// Splits a server-sent event stream into (event, data) pairs, with the same bounded
/// buffering as the local worker's line reader.
#[derive(Default)]
pub struct Sse {
    received_bytes: usize,
    line: Vec<u8>,
    event: String,
    data: String,
}

impl Sse {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
        // Many individually small events can otherwise accumulate unbounded
        // thinking blocks, provider metadata or tool names in the decoder.
        self.received_bytes = self.received_bytes.saturating_add(bytes.len());
        if self.received_bytes > STREAM_LIMIT {
            return Err("The cloud response exceeded its safe stream size limit.".into());
        }
        let mut events = Vec::new();
        for &byte in bytes {
            if byte != b'\n' {
                if self.line.len() >= LINE_LIMIT {
                    return Err("The cloud model sent a line larger than Buntini accepts.".into());
                }
                self.line.push(byte);
                continue;
            }
            let line = String::from_utf8(std::mem::take(&mut self.line))
                .map_err(|_| "The cloud model returned invalid text.".to_string())?;
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push((
                        std::mem::take(&mut self.event),
                        std::mem::take(&mut self.data),
                    ));
                }
                self.event.clear();
            } else if let Some(data) = line.strip_prefix("data:") {
                if self.data.len() + data.len() > EVENT_LIMIT {
                    return Err("The cloud model sent an event larger than Buntini accepts.".into());
                }
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
            } else if let Some(event) = line.strip_prefix("event:") {
                self.event = event.trim().to_string();
            }
        }
        Ok(events)
    }
}

const REFUSED: &str = "The cloud model declined to answer this. Try rephrasing, or choose another brain in Settings → Brain.";
const MULTIPLE_TOOLS: &str =
    "The cloud model requested multiple tools in a single turn. Try again.";
const INCOMPLETE: &str = "The cloud model stopped before finishing. Try again.";
const CUT_OFF: &str = "The cloud model ran out of room before finishing. Try a shorter request.";

/// A malformed call must never be repaired into an executable request with default
/// arguments. The controller still validates the tool's name and full schema.
fn tool_call(name: &Value, arguments: Value) -> Result<Value, String> {
    let name = name
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .ok_or("The cloud model sent a tool call without a name.")?;
    if !arguments.is_object() {
        return Err("The cloud model sent tool arguments that are not an object.".into());
    }
    Ok(json!({"name":name,"arguments":arguments}))
}

fn tool_arguments(raw: &str) -> Result<Value, String> {
    // An omitted argument payload is also incomplete, even for a no-argument tool.
    serde_json::from_str(raw)
        .map_err(|_| "The cloud model sent unreadable tool arguments.".to_string())
}

/// Turns one provider's stream events into text, at most one tool call, and usage.
pub struct Decoder {
    provider: &'static str,
    text: String,
    usage: Usage,
    input_reported: bool,
    output_reported: bool,
    /// Anthropic content blocks or Gemini parts, kept to send back on the next step.
    blocks: BTreeMap<usize, Value>,
    partial_json: BTreeMap<usize, String>,
    parts: Vec<Value>,
    /// OpenAI tool call being streamed: (name, arguments text).
    call: Option<(String, String)>,
    stop: String,
    done: bool,
}

impl Decoder {
    pub fn new(provider: &'static str) -> Self {
        Self {
            provider,
            text: String::new(),
            usage: Usage::default(),
            input_reported: false,
            output_reported: false,
            blocks: BTreeMap::new(),
            partial_json: BTreeMap::new(),
            parts: Vec::new(),
            call: None,
            stop: String::new(),
            done: false,
        }
    }

    fn say(&mut self, piece: &str, on_token: &mut impl FnMut(String)) -> Result<(), String> {
        if piece.is_empty() {
            return Ok(());
        }
        if self.text.len() + piece.len() > 64_000 {
            return Err("The cloud response exceeded its safe size limit.".into());
        }
        self.text.push_str(piece);
        on_token(piece.to_string());
        Ok(())
    }

    /// Feeds one event; returns true once the provider says the reply is complete.
    pub fn event(&mut self, data: &str, on_token: &mut impl FnMut(String)) -> Result<bool, String> {
        if self.provider == "openai" && data.trim() == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|_| "The cloud model sent an unreadable update.".to_string())?;
        if let Some(message) = value["error"]["message"].as_str() {
            return Err(format!(
                "The cloud model reported an error: {}",
                message.chars().take(400).collect::<String>()
            ));
        }
        match self.provider {
            "openai" => {
                let choice = &value["choices"][0];
                if let Some(piece) = choice["delta"]["content"].as_str() {
                    self.say(piece, on_token)?;
                }
                for call in choice["delta"]["tool_calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if call["index"].as_u64().unwrap_or(0) != 0 {
                        return Err(MULTIPLE_TOOLS.into());
                    }
                    let (name, arguments) = self.call.get_or_insert_with(Default::default);
                    name.push_str(call["function"]["name"].as_str().unwrap_or(""));
                    arguments.push_str(call["function"]["arguments"].as_str().unwrap_or(""));
                    if arguments.len() > 65_536 {
                        return Err("The cloud model sent oversized tool arguments.".into());
                    }
                }
                if let Some(reason) = choice["finish_reason"].as_str() {
                    self.stop = reason.into();
                }
                if value["usage"].is_object() {
                    self.usage = Usage {
                        input: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                        output: value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
                    };
                    self.input_reported = value["usage"]["prompt_tokens"].as_u64().is_some();
                    self.output_reported = value["usage"]["completion_tokens"].as_u64().is_some();
                }
            }
            "anthropic" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;
                match value["type"].as_str().unwrap_or("") {
                    "message_start" => {
                        let usage = &value["message"]["usage"];
                        self.usage.input = [
                            "input_tokens",
                            "cache_creation_input_tokens",
                            "cache_read_input_tokens",
                        ]
                        .iter()
                        .map(|key| usage[key].as_u64().unwrap_or(0))
                        .sum();
                        self.usage.output = usage["output_tokens"].as_u64().unwrap_or(0);
                        self.input_reported = usage["input_tokens"].as_u64().is_some();
                    }
                    "content_block_start" => {
                        if self.blocks.len() >= 64 {
                            return Err("The cloud model sent too many parts.".into());
                        }
                        self.blocks.insert(index, value["content_block"].clone());
                    }
                    "content_block_delta" => {
                        let delta = &value["delta"];
                        let piece = |key: &str| delta[key].as_str().unwrap_or("").to_string();
                        match delta["type"].as_str().unwrap_or("") {
                            "text_delta" => {
                                let piece = piece("text");
                                self.say(&piece, on_token)?;
                                append(
                                    self.blocks
                                        .entry(index)
                                        .or_insert_with(|| json!({"type":"text","text":""})),
                                    "text",
                                    &piece,
                                );
                            }
                            "thinking_delta" => append(
                                self.blocks
                                    .entry(index)
                                    .or_insert_with(|| json!({"type":"thinking","thinking":""})),
                                "thinking",
                                &piece("thinking"),
                            ),
                            "signature_delta" => {
                                append(
                                    self.blocks.entry(index).or_insert_with(
                                        || json!({"type":"thinking","thinking":""}),
                                    ),
                                    "signature",
                                    &piece("signature"),
                                );
                            }
                            "input_json_delta" => {
                                let partial = self.partial_json.entry(index).or_default();
                                partial.push_str(&piece("partial_json"));
                                if partial.len() > 65_536 {
                                    return Err(
                                        "The cloud model sent oversized tool arguments.".into()
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if let Some(raw) = self.partial_json.remove(&index) {
                            let input: Value = if raw.trim().is_empty() {
                                json!({})
                            } else {
                                serde_json::from_str(&raw).map_err(|_| {
                                    "The cloud model sent unreadable tool arguments.".to_string()
                                })?
                            };
                            if let Some(block) = self.blocks.get_mut(&index) {
                                block["input"] = input;
                            }
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                            self.stop = reason.into();
                        }
                        let usage = &value["usage"];
                        // After a fallback, each attempt reports its own usage.
                        if let Some(iterations) = usage["iterations"].as_array() {
                            let complete = !iterations.is_empty()
                                && iterations.iter().all(|step| {
                                    step["input_tokens"].as_u64().is_some()
                                        && step["output_tokens"].as_u64().is_some()
                                });
                            self.input_reported = complete;
                            self.output_reported = complete;
                            self.usage =
                                iterations
                                    .iter()
                                    .fold(Usage::default(), |total, step| Usage {
                                        input: total.input
                                            + [
                                                "input_tokens",
                                                "cache_creation_input_tokens",
                                                "cache_read_input_tokens",
                                            ]
                                            .iter()
                                            .map(|key| step[key].as_u64().unwrap_or(0))
                                            .sum::<u64>(),
                                        output: total.output
                                            + step["output_tokens"].as_u64().unwrap_or(0),
                                    });
                        } else if let Some(output) = usage["output_tokens"].as_u64() {
                            self.usage.output = output;
                            self.output_reported = true;
                        }
                    }
                    "message_stop" => {
                        self.done = true;
                        return Ok(true);
                    }
                    _ => {}
                }
            }
            "gemini" => {
                if value["promptFeedback"]["blockReason"].is_string() {
                    return Err(REFUSED.into());
                }
                let candidate = &value["candidates"][0];
                for part in candidate["content"]["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if part["functionCall"].is_object() {
                        if self
                            .parts
                            .iter()
                            .any(|kept| kept["functionCall"].is_object())
                        {
                            return Err(MULTIPLE_TOOLS.into());
                        }
                    } else if let (Some(piece), false) = (
                        part["text"].as_str(),
                        part["thought"].as_bool().unwrap_or(false),
                    ) {
                        self.say(piece, on_token)?;
                    }
                    if self.parts.len() >= 256 {
                        return Err("The cloud model sent too many parts.".into());
                    }
                    self.parts.push(part.clone());
                }
                if let Some(reason) = candidate["finishReason"].as_str() {
                    self.stop = reason.into();
                }
                let usage = &value["usageMetadata"];
                if usage.is_object() {
                    self.usage = Usage {
                        input: usage["promptTokenCount"].as_u64().unwrap_or(0),
                        output: usage["candidatesTokenCount"].as_u64().unwrap_or(0)
                            + usage["thoughtsTokenCount"].as_u64().unwrap_or(0),
                    };
                    self.input_reported = usage["promptTokenCount"].as_u64().is_some();
                    self.output_reported = usage["candidatesTokenCount"].as_u64().is_some();
                }
            }
            "chatgpt" => match value["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    let piece = value["delta"].as_str().unwrap_or("").to_string();
                    self.say(&piece, on_token)?;
                }
                "response.output_item.done" => {
                    let item = value["item"].clone();
                    let extra_call = item["type"] == "function_call"
                        && self
                            .parts
                            .iter()
                            .any(|kept| kept["type"] == "function_call");
                    if extra_call {
                        return Err(MULTIPLE_TOOLS.into());
                    }
                    if item["content"]
                        .as_array()
                        .is_some_and(|content| content.iter().any(|part| part["type"] == "refusal"))
                    {
                        return Err(REFUSED.into());
                    }
                    if self.parts.len() >= 64 {
                        return Err("The cloud model sent too many parts.".into());
                    }
                    self.parts.push(item);
                }
                "response.completed" => {
                    let usage = &value["response"]["usage"];
                    self.usage = Usage {
                        input: usage["input_tokens"].as_u64().unwrap_or(0),
                        output: usage["output_tokens"].as_u64().unwrap_or(0),
                    };
                    self.input_reported = usage["input_tokens"].as_u64().is_some();
                    self.output_reported = usage["output_tokens"].as_u64().is_some();
                    self.stop = "completed".into();
                    self.done = true;
                    return Ok(true);
                }
                "response.incomplete" => {
                    self.stop = value["response"]["incomplete_details"]["reason"]
                        .as_str()
                        .unwrap_or("incomplete")
                        .to_string();
                    self.done = true;
                    return Ok(true);
                }
                "response.failed" | "error" => {
                    let error = if value["type"] == "error" {
                        &value
                    } else {
                        &value["response"]["error"]
                    };
                    let code = error["code"].as_str().unwrap_or("unknown_error");
                    return Err(crate::chatgpt::plan_error(
                        code,
                        error["param"].as_str().unwrap_or(""),
                    )
                    .unwrap_or_else(|| format!("ChatGPT couldn’t answer ({code}).")));
                }
                _ => {}
            },
            _ => return Err("Unknown cloud provider.".into()),
        }
        Ok(false)
    }

    /// The finished turn, or why it cannot be used.
    pub fn finish(self) -> Result<(Turn, Usage), String> {
        let usage = self.usage;
        let cut_off = matches!(self.stop.as_str(), "length" | "max_tokens" | "MAX_TOKENS");
        match self.provider {
            "openai" => {
                if self.stop == "content_filter" {
                    return Err(REFUSED.into());
                }
                if !self.done || self.stop.is_empty() {
                    return Err(INCOMPLETE.into());
                }
                if cut_off {
                    return Err(CUT_OFF.into());
                }
                let tool = match self.call {
                    Some((name, arguments)) => Some(tool_call(
                        &Value::String(name),
                        tool_arguments(&arguments)?,
                    )?),
                    None => None,
                };
                Ok((
                    Turn {
                        text: self.text,
                        tool,
                        raw: None,
                    },
                    usage,
                ))
            }
            "anthropic" => {
                if self.stop == "refusal" {
                    return Err(REFUSED.into());
                }
                if !self.done {
                    return Err(INCOMPLETE.into());
                }
                if cut_off {
                    return Err(CUT_OFF.into());
                }
                if !self.partial_json.is_empty() {
                    return Err(
                        "The cloud model stopped before finishing its tool arguments. Try again."
                            .into(),
                    );
                }
                let mut blocks: Vec<Value> = self.blocks.into_values().collect();
                // After a fallback, only text from the declined attempt is sent back.
                if let Some(boundary) = blocks.iter().rposition(|block| block["type"] == "fallback")
                {
                    blocks = blocks
                        .into_iter()
                        .enumerate()
                        .filter(|(index, block)| *index > boundary || block["type"] == "text")
                        .map(|(_, block)| block)
                        .collect();
                }
                if blocks
                    .iter()
                    .filter(|block| block["type"] == "tool_use")
                    .count()
                    > 1
                {
                    return Err(MULTIPLE_TOOLS.into());
                }
                let tool = blocks
                    .iter()
                    .find(|block| block["type"] == "tool_use")
                    .map(|block| tool_call(&block["name"], block["input"].clone()))
                    .transpose()?;
                Ok((
                    Turn {
                        text: self.text,
                        tool,
                        raw: Some(Value::Array(blocks)),
                    },
                    usage,
                ))
            }
            "chatgpt" => {
                if self.stop != "completed" {
                    return Err(match self.stop.as_str() {
                        "content_filter" => REFUSED.into(),
                        "" => "ChatGPT stopped before finishing. Try again.".into(),
                        _ => "ChatGPT stopped before finishing that reply. Try again.".into(),
                    });
                }
                let tool = self
                    .parts
                    .iter()
                    .find(|item| item["type"] == "function_call")
                    .map(|item| {
                        let raw = item["arguments"]
                            .as_str()
                            .ok_or("The cloud model sent unreadable tool arguments.")?;
                        tool_call(&item["name"], tool_arguments(raw)?)
                    })
                    .transpose()?;
                Ok((
                    Turn {
                        text: self.text,
                        tool,
                        raw: Some(Value::Array(self.parts)),
                    },
                    usage,
                ))
            }
            _ => {
                if [
                    "SAFETY",
                    "PROHIBITED_CONTENT",
                    "BLOCKLIST",
                    "SPII",
                    "RECITATION",
                ]
                .contains(&self.stop.as_str())
                {
                    return Err(REFUSED.into());
                }
                if cut_off {
                    return Err(CUT_OFF.into());
                }
                let tool = self
                    .parts
                    .iter()
                    .find(|part| part["functionCall"].is_object())
                    .map(|part| {
                        let call = &part["functionCall"];
                        // Gemini may omit `args` for a function with no parameters.
                        let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
                        tool_call(&call["name"], arguments)
                    })
                    .transpose()?;
                if self.stop.is_empty() {
                    return Err(INCOMPLETE.into());
                }
                Ok((
                    Turn {
                        text: self.text,
                        tool,
                        raw: Some(Value::Array(self.parts)),
                    },
                    usage,
                ))
            }
        }
    }

    /// Missing usage is different from a provider explicitly reporting zero. Keep
    /// the reservation unless both input and final output counts were supplied.
    fn finish_reported(self) -> Result<(Turn, Option<Usage>), String> {
        let reported = self.input_reported && self.output_reported;
        self.finish()
            .map(|(turn, usage)| (turn, reported.then_some(usage)))
    }
}

fn append(block: &mut Value, key: &str, piece: &str) {
    let current = block[key].as_str().unwrap_or("").to_string();
    if let Some(map) = block.as_object_mut() {
        map.insert(key.into(), Value::String(current + piece));
    }
}

/// Plain-language advice for a rejected request.
fn rejection(name: &str, status: u16, body: &str) -> String {
    let detail: Value = serde_json::from_str(body).unwrap_or_default();
    if let Some(advice) = detail["error"]["code"].as_str().and_then(|code| {
        crate::chatgpt::plan_error(code, detail["error"]["param"].as_str().unwrap_or(""))
    }) {
        return advice;
    }
    if name == "ChatGPT" {
        return match status {
            401 => "ChatGPT didn’t accept your sign-in. Sign in again in Settings → Brain.".into(),
            403 => "ChatGPT doesn’t allow this request from your account or region.".into(),
            429 => "You’ve reached the limit for using your ChatGPT plan in apps like Buntini. Check ChatGPT → Settings → Usage.".into(),
            503 => "ChatGPT is busy right now. Try again in a moment.".into(),
            _ => format!("ChatGPT couldn’t answer ({status})."),
        };
    }
    let message = detail["error"]["message"]
        .as_str()
        .or_else(|| detail[0]["error"]["message"].as_str())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect::<String>();
    match status {
        401 | 403 => format!("{name} didn’t accept the API key. Check it in Settings → Brain."),
        404 => format!("{name} doesn’t recognise that model name. Check it in Settings → Brain."),
        429 => format!("{name} is limiting requests or your account is out of credit. {message}"),
        400 => format!("{name} couldn’t use this request: {message}"),
        _ => format!("{name} had a problem ({status}). Try again in a moment."),
    }
}

/// Error pages are untrusted network input too. Read only enough for useful advice,
/// and keep cancellation responsive while a server trickles its response body.
async fn rejection_body<S, T, E>(
    mut stream: S,
    cancel: &AtomicBool,
    deadline: tokio::time::Instant,
) -> Result<String, Failure>
where
    S: futures_util::Stream<Item = Result<T, E>> + Unpin,
    T: AsRef<[u8]>,
{
    let mut body = Vec::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Failure::Rejected("Task cancelled.".into()));
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    let bytes = bytes.as_ref();
                    let take = bytes.len().min(65_536 - body.len());
                    body.extend_from_slice(&bytes[..take]);
                    if body.len() == 65_536 { break; }
                }
                Some(Err(_)) | None => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Streams one reply. Tokens go to `on_token` as they arrive.
pub async fn generate<F>(
    provider: &'static Provider,
    model: &str,
    key: &str,
    payload: &Value,
    cancel: Arc<AtomicBool>,
    mut on_token: F,
) -> Result<(Turn, Option<Usage>), Failure>
where
    F: FnMut(String) + Send,
{
    if cancel.load(Ordering::SeqCst) {
        return Err(Failure::Rejected("Task cancelled.".into()));
    }
    let request = build(provider.id, model, key, payload).map_err(Failure::Rejected)?;
    let client = crate::web::private_client_builder()
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| Failure::Rejected("Cannot prepare a secure connection.".into()))?;
    let mut builder = client.post(&request.url).json(&request.body);
    for (name, value) in &request.headers {
        builder = builder.header(*name, value);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let response = {
        let sending = builder.send();
        tokio::pin!(sending);
        loop {
            // Once sending starts the provider may have accepted the request, even
            // before response headers arrive. Keep its spending reservation then.
            if cancel.load(Ordering::SeqCst) {
                return Err(Failure::Failed("Task cancelled.".into()));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Failure::Failed(format!(
                    "{} took too long to answer.",
                    provider.name
                )));
            }
            tokio::select! {
                response = &mut sending => break response.map_err(|error| {
                    if error.is_connect() {
                        Failure::Rejected(format!("Couldn’t reach {}. Check your internet connection.", provider.name))
                    } else {
                        Failure::Failed(format!("The connection to {} failed.", provider.name))
                    }
                })?,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = rejection_body(response.bytes_stream(), &cancel, deadline).await?;
        return Err(Failure::Rejected(rejection(
            provider.name,
            status.as_u16(),
            &body,
        )));
    }
    let mut sse = Sse::default();
    let mut decoder = Decoder::new(provider.id);
    let mut stream = response.bytes_stream();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Failure::Failed("Task cancelled.".into()));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(Failure::Failed(format!(
                "{} took too long to answer.",
                provider.name
            )));
        }
        tokio::select! {
            chunk = stream.next() => match chunk {
                None => break,
                Some(Err(_)) => return Err(Failure::Failed(format!("The connection to {} dropped. Try again.", provider.name))),
                Some(Ok(bytes)) => {
                    for (_, data) in sse.push(&bytes).map_err(Failure::Failed)? {
                        if decoder.event(&data, &mut on_token).map_err(Failure::Failed)? {
                            return decoder.finish_reported().map_err(Failure::Failed);
                        }
                    }
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    decoder.finish_reported().map_err(Failure::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Value {
        crate::tools::offer(&["search_files"])
    }

    /// A conversation that already made one tool call, as the controller builds it.
    fn payload(provider_content: Option<Value>) -> Value {
        let mut assistant = json!({"role":"assistant","content":"","tool_calls":[{"type":"function","function":{"name":"search_files","arguments":{"query":"lease"}}}]});
        if let Some(raw) = provider_content {
            assistant["provider_content"] = raw;
        }
        json!({"messages":[{"role":"system","content":"Be brief."},{"role":"user","content":"hi"},{"role":"assistant","content":"hello"},{"role":"user","content":"find my lease"},assistant,{"role":"tool","content":"lease.pdf"}],"tools":tools(),"max_tokens":512})
    }

    fn decode(provider: &'static str, stream: &str) -> (Result<(Turn, Usage), String>, String) {
        let mut sse = Sse::default();
        let mut decoder = Decoder::new(provider);
        let mut streamed = String::new();
        // Feed in small, uneven pieces like a real network would.
        for chunk in stream.as_bytes().chunks(7) {
            for (_, data) in sse.push(chunk).unwrap() {
                if decoder
                    .event(&data, &mut |token| streamed.push_str(&token))
                    .unwrap()
                {
                    return (decoder.finish(), streamed);
                }
            }
        }
        (decoder.finish(), streamed)
    }

    fn events(provider: &'static str, events: &[Value]) -> Result<(Turn, Usage), String> {
        let mut decoder = Decoder::new(provider);
        for event in events {
            decoder.event(&event.to_string(), &mut |_| {})?;
        }
        decoder.finish()
    }

    #[test]
    fn sse_lines_are_split_and_bounded() {
        let mut sse = Sse::default();
        let mut events = sse
            .push(b"event: a\r\ndata: one\r\n\r\n: comment\ndata: two\ndata:three\n")
            .unwrap();
        events.extend(sse.push(b"\n").unwrap());
        assert_eq!(
            events,
            vec![
                ("a".to_string(), "one".to_string()),
                (String::new(), "two\nthree".to_string())
            ]
        );
        assert!(Sse::default().push(&vec![b'x'; LINE_LIMIT + 1]).is_err());
    }

    #[test]
    fn many_small_events_cannot_bypass_the_total_cloud_response_limit() {
        let event = format!("data: {}\n\n", "x".repeat(8192));
        let mut sse = Sse::default();
        for _ in 0..STREAM_LIMIT / event.len() {
            assert_eq!(sse.push(event.as_bytes()).unwrap().len(), 1);
        }
        assert!(sse
            .push(event.as_bytes())
            .unwrap_err()
            .contains("stream size limit"));
    }

    #[test]
    fn openai_requests_number_tool_calls_and_stream_answers() {
        let request = build("openai", "gpt-5-mini", "sk-test", &payload(None)).unwrap();
        assert_eq!(
            request.headers[0],
            ("authorization", "Bearer sk-test".to_string())
        );
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages[4]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            messages[4]["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"lease\"}"
        );
        assert_eq!(messages[5]["tool_call_id"], "call_1");
        assert_eq!(request.body["tools"][0]["function"]["name"], "search_files");
        assert_eq!(request.body["reasoning_effort"], "low");

        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":5}}\n\ndata: [DONE]\n\n";
        let (result, streamed) = decode("openai", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!((turn.text.as_str(), streamed.as_str()), ("Hello", "Hello"));
        assert_eq!(
            usage,
            Usage {
                input: 20,
                output: 5
            }
        );

        let call = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"search_files\",\"arguments\":\"{\\\"qu\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ery\\\":\\\"tax\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let (turn, _) = decode("openai", call).0.unwrap();
        assert_eq!(
            turn.tool.unwrap(),
            json!({"name":"search_files","arguments":{"query":"tax"}})
        );
        assert!(
            decode(
                "openai",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"
            )
            .0
            .is_err(),
            "a stream that never finished is not an answer"
        );
    }

    #[test]
    fn anthropic_keeps_thinking_for_the_next_step() {
        let raw = json!([{"type":"thinking","thinking":"","signature":"sig"},{"type":"tool_use","id":"toolu_1","name":"search_files","input":{"query":"lease"}}]);
        let request = build(
            "anthropic",
            "claude-opus-5-5",
            "key",
            &payload(Some(raw.clone())),
        )
        .unwrap();
        assert_eq!(request.body["system"], "Be brief.");
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5, "the system prompt is top-level");
        assert_eq!(messages[3]["content"], raw);
        assert_eq!(messages[4]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(request.body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(request.body["output_config"]["effort"], "low");
        assert_eq!(request.body["fallbacks"], "default");
        assert!(request
            .headers
            .contains(&("anthropic-beta", "server-side-fallback-2026-07-01".into())));
        let plain = build("anthropic", "claude-haiku-4-5", "key", &payload(None)).unwrap();
        assert!(plain.body.get("output_config").is_none() && plain.body.get("fallbacks").is_none());
        assert_eq!(
            plain.body["messages"][3]["content"][0]["id"],
            plain.body["messages"][4]["content"][0]["tool_use_id"]
        );

        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":30,\"cache_read_input_tokens\":10,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"abc\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Looking\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"search_files\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\": \\\"le\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ase\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":42}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let (result, streamed) = decode("anthropic", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!(streamed, "Looking");
        assert_eq!(
            usage,
            Usage {
                input: 40,
                output: 42
            }
        );
        assert_eq!(
            turn.tool.unwrap(),
            json!({"name":"search_files","arguments":{"query":"lease"}})
        );
        let raw = turn.raw.unwrap();
        assert_eq!(raw[0]["signature"], "abc");
        assert_eq!(raw[2]["input"]["query"], "lease");

        let refusal = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"},\"usage\":{\"output_tokens\":0}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        assert_eq!(decode("anthropic", refusal).0.unwrap_err(), REFUSED);
    }

    #[test]
    fn anthropic_fallback_drops_the_declined_attempts_private_blocks() {
        let stream = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"old\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"Part\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"fallback\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":3,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":3,\"delta\":{\"type\":\"text_delta\",\"text\":\"ial\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9,\"iterations\":[{\"input_tokens\":10,\"output_tokens\":2},{\"input_tokens\":12,\"output_tokens\":7}]}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let (turn, usage) = decode("anthropic", stream).0.unwrap();
        assert_eq!(
            usage,
            Usage {
                input: 22,
                output: 9
            }
        );
        let types = turn
            .raw
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|block| block["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            ["text", "text"],
            "the fallback marker is optional to send back"
        );
    }

    #[test]
    fn gemini_sends_the_key_in_a_header_and_keeps_signatures() {
        let raw = json!([{"functionCall":{"name":"search_files","args":{"query":"lease"}},"thoughtSignature":"sig"}]);
        let request = build(
            "gemini",
            "gemini-2.5-flash",
            "AIza-test",
            &payload(Some(raw.clone())),
        )
        .unwrap();
        assert!(
            !request.url.contains("AIza"),
            "the key never goes in the URL"
        );
        assert_eq!(
            request.headers[0],
            ("x-goog-api-key", "AIza-test".to_string())
        );
        assert_eq!(
            request.body["systemInstruction"]["parts"][0]["text"],
            "Be brief."
        );
        let contents = request.body["contents"].as_array().unwrap();
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[3]["parts"], raw);
        assert_eq!(
            contents[4]["parts"][0]["functionResponse"]["name"],
            "search_files"
        );
        assert!(
            request.body["tools"][0]["functionDeclarations"][0]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
        assert!(build("gemini", "../../x", "k", &payload(None)).is_err());

        let stream = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Sunny \"}]}}]}\r\n\r\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"today.\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":12,\"candidatesTokenCount\":4,\"thoughtsTokenCount\":6}}\r\n\r\n";
        let (result, streamed) = decode("gemini", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!(
            (turn.text.as_str(), streamed.as_str()),
            ("Sunny today.", "Sunny today.")
        );
        assert_eq!(
            usage,
            Usage {
                input: 12,
                output: 10
            }
        );
        let blocked = "data: {\"candidates\":[{\"finishReason\":\"SAFETY\"}]}\n\n";
        assert_eq!(decode("gemini", blocked).0.unwrap_err(), REFUSED);
    }

    #[test]
    fn chatgpt_uses_the_responses_api_without_storing_anything() {
        let raw = json!([{"type":"reasoning","id":"rs_1","encrypted_content":"opaque"},{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search_files","arguments":"{\"query\":\"lease\"}"}]);
        let request = build(
            "chatgpt",
            "gpt-6.1-sol",
            "token",
            &payload(Some(raw.clone())),
        )
        .unwrap();
        assert_eq!(request.url, "https://api.openai.com/v1/responses");
        assert_eq!(
            request.headers[0],
            ("authorization", "Bearer token".to_string())
        );
        assert_eq!(
            (
                request.body["store"].clone(),
                request.body["stream"].clone()
            ),
            (json!(false), json!(true))
        );
        assert_eq!(request.body["instructions"], "Be brief.");
        assert!(
            request.body.get("max_output_tokens").is_none()
                && request.body.get("temperature").is_none(),
            "unsupported on this route"
        );
        let input = request.body["input"].as_array().unwrap();
        assert!(
            input.iter().all(|item| item["role"] != "system"),
            "system text goes in instructions"
        );
        assert_eq!(input[3], raw[0], "reasoning goes back as received");
        assert_eq!(
            input[5],
            json!({"type":"function_call_output","call_id":"call_abc","output":"lease.pdf"})
        );
        assert_eq!(request.body["tools"][0]["name"], "search_files");
        assert!(
            build("chatgpt", "", "token", &payload(None)).is_err(),
            "a model must be chosen"
        );
        let plain = build("chatgpt", "gpt-6.1-sol", "t", &payload(None)).unwrap();
        assert_eq!(
            plain.body["input"][3]["type"], "function_call",
            "an empty assistant message adds no item"
        );
        assert_eq!(
            plain.body["input"][4]["call_id"],
            plain.body["input"][3]["call_id"]
        );

        let stream = concat!(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":30,\"output_tokens\":5}}}\n\n",
        );
        let (result, streamed) = decode("chatgpt", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!(
            (turn.text.as_str(), streamed.as_str(), usage),
            (
                "Hello",
                "Hello",
                Usage {
                    input: 30,
                    output: 5
                }
            )
        );
        let call = "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"system_info\",\"arguments\":\"{}\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{}}}\n\n";
        let (turn, _) = decode("chatgpt", call).0.unwrap();
        assert_eq!(
            turn.tool.unwrap(),
            json!({"name":"system_info","arguments":{}})
        );
        assert_eq!(turn.raw.unwrap()[0]["call_id"], "c1");
        let limit = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n";
        let mut decoder = Decoder::new("chatgpt");
        let error = Sse::default()
            .push(limit.as_bytes())
            .unwrap()
            .into_iter()
            .map(|(_, data)| decoder.event(&data, &mut |_| {}))
            .find_map(Result::err)
            .unwrap();
        assert!(error.contains("Settings → Usage"));
        assert!(rejection("ChatGPT", 429, "{}").contains("ChatGPT plan"));
        assert!(
            decode(
                "chatgpt",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n"
            )
            .0
            .is_err(),
            "no completed event, no answer"
        );
    }

    #[test]
    fn spending_is_estimated_high_and_priced_per_million() {
        let guess = estimate(&payload(None));
        assert_eq!(guess.output, 512);
        assert!(guess.input > 100);
        assert!(
            (price(
                Usage {
                    input: 1_000_000,
                    output: 500_000
                },
                4.0,
                20.0
            ) - 14.0)
                .abs()
                < 1e-9
        );
        assert!(
            model_valid("claude-opus-5-5")
                && !model_valid("gpt 5")
                && !model_valid(&"a".repeat(101))
        );
        assert!(rejection("OpenAI", 401, "{}").contains("API key"));
        assert!(
            rejection("Anthropic", 400, "{\"error\":{\"message\":\"bad field\"}}")
                .contains("bad field")
        );
    }

    #[test]
    fn a_disconnected_openai_tool_stream_is_never_executable() {
        let stream = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"system_info\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";
        assert_eq!(decode("openai", stream).0.unwrap_err(), INCOMPLETE);
        let finished = format!("{stream}data: [DONE]\n\n");
        assert_eq!(
            decode("openai", &finished).0.unwrap().0.tool.unwrap()["name"],
            "system_info"
        );
    }

    #[test]
    fn malformed_chatgpt_arguments_are_never_replaced_with_defaults() {
        for arguments in [
            json!("{bad}"),
            json!(""),
            json!("[]"),
            json!("null"),
            json!({}),
        ] {
            let result = events(
                "chatgpt",
                &[
                    json!({"type":"response.output_item.done","item":{"type":"function_call","name":"system_info","arguments":arguments}}),
                    json!({"type":"response.completed","response":{"usage":{}}}),
                ],
            );
            assert!(
                result.is_err(),
                "malformed arguments must not execute system_info: {arguments}"
            );
        }
    }

    #[test]
    fn cloud_models_must_finish_answers_before_they_are_accepted() {
        let openai = "data: {\"choices\":[{\"delta\":{\"content\":\"Half an answer\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n";
        assert_eq!(decode("openai", openai).0.unwrap_err(), CUT_OFF);
        assert_eq!(events("anthropic", &[
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Half an answer"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
            json!({"type":"message_stop"}),
        ]).unwrap_err(), CUT_OFF);
        assert_eq!(events("gemini", &[
            json!({"candidates":[{"content":{"parts":[{"text":"Half an answer"}]},"finishReason":"MAX_TOKENS"}]}),
        ]).unwrap_err(), CUT_OFF);
    }

    #[test]
    fn multiple_cloud_tool_calls_fail_instead_of_silently_losing_requests() {
        assert_eq!(events("openai", &[
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"system_info","arguments":"{}"}},{"index":1,"function":{"name":"read_mail","arguments":"{}"}}]}}]}),
        ]).unwrap_err(), MULTIPLE_TOOLS);
        assert_eq!(events("anthropic", &[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"system_info","input":{}}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","name":"read_mail","input":{}}}),
            json!({"type":"message_stop"}),
        ]).unwrap_err(), MULTIPLE_TOOLS);
        assert_eq!(events("gemini", &[
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"system_info","args":{}}},{"functionCall":{"name":"read_mail","args":{}}}]},"finishReason":"STOP"}]}),
        ]).unwrap_err(), MULTIPLE_TOOLS);
        assert_eq!(events("chatgpt", &[
            json!({"type":"response.output_item.done","item":{"type":"function_call","name":"system_info","arguments":"{}"}}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","name":"read_mail","arguments":"{}"}}),
        ]).unwrap_err(), MULTIPLE_TOOLS);
    }

    #[test]
    fn anthropic_preserves_split_signatures_and_rejects_unfinished_arguments() {
        let (turn, _) = events("anthropic", &[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"def"}}),
            json!({"type":"message_stop"}),
        ]).unwrap();
        assert_eq!(turn.raw.unwrap()[0]["signature"], "abcdef");
        assert!(events("anthropic", &[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"system_info","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}),
            json!({"type":"message_stop"}),
        ]).unwrap_err().contains("tool arguments"));
    }

    #[tokio::test]
    async fn a_cancelled_cloud_request_is_rejected_before_connecting() {
        let result = generate(
            provider("openai").unwrap(),
            "gpt-5-mini",
            "unused",
            &payload(None),
            Arc::new(AtomicBool::new(true)),
            |_| {},
        )
        .await;
        match result {
            Err(Failure::Rejected(message)) => assert_eq!(message, "Task cancelled."),
            _ => panic!("a cancelled request must never start a connection"),
        }
    }

    #[test]
    fn missing_or_partial_usage_keeps_the_reservation_but_reported_zero_can_settle() {
        for provider in ["openai", "anthropic", "gemini", "chatgpt"] {
            for complete in [false, true] {
                let mut decoder = Decoder::new(provider);
                let mut usage = json!({});
                let (input, output) = match provider {
                    "openai" => ("prompt_tokens", "completion_tokens"),
                    "gemini" => ("promptTokenCount", "candidatesTokenCount"),
                    _ => ("input_tokens", "output_tokens"),
                };
                usage[input] = json!(0);
                if complete {
                    usage[output] = json!(0);
                }
                let updates = match provider {
                    "openai" => vec![json!({"choices":[{"finish_reason":"stop"}],"usage":usage})],
                    "anthropic" => vec![
                        json!({"type":"message_start","message":{"usage":usage}}),
                        json!({"type":"message_delta","usage":if complete { json!({"output_tokens":0}) } else { json!({}) }}),
                        json!({"type":"message_stop"}),
                    ],
                    "gemini" => {
                        vec![json!({"candidates":[{"finishReason":"STOP"}],"usageMetadata":usage})]
                    }
                    _ => vec![json!({"type":"response.completed","response":{"usage":usage}})],
                };
                for update in updates {
                    decoder.event(&update.to_string(), &mut |_| {}).unwrap();
                }
                if provider == "openai" {
                    decoder.event("[DONE]", &mut |_| {}).unwrap();
                }
                assert_eq!(
                    decoder.finish_reported().unwrap().1,
                    complete.then_some(Usage::default()),
                    "{provider} usage must include both token counts"
                );
            }
        }
        let mut decoder = Decoder::new("openai");
        decoder
            .event(r#"{"choices":[{"finish_reason":"stop"}]}"#, &mut |_| {})
            .unwrap();
        decoder.event("[DONE]", &mut |_| {}).unwrap();
        assert_eq!(decoder.finish_reported().unwrap().1, None);
    }

    #[tokio::test]
    async fn rejection_bodies_are_bounded_and_cancellable_without_a_network() {
        let stream = futures_util::stream::iter([Ok::<_, ()>(vec![b'x'; 100_000])]);
        let body = rejection_body(
            stream,
            &AtomicBool::new(false),
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(body.len(), 65_536);

        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            token.store(true, Ordering::SeqCst);
        });
        let stream = futures_util::stream::pending::<Result<Vec<u8>, ()>>();
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            rejection_body(
                stream,
                &cancel,
                tokio::time::Instant::now() + Duration::from_secs(2),
            ),
        )
        .await
        .expect("cancelling a stalled rejection body must be prompt");
        assert!(matches!(result, Err(Failure::Rejected(message)) if message == "Task cancelled."));
    }
}
