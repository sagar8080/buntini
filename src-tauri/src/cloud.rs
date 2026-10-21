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

pub const PROVIDERS: [Provider; 3] = [
    Provider { id: "anthropic", name: "Anthropic", model: "claude-opus-5-5", input_per_m: 4.0, output_per_m: 20.0 },
    Provider { id: "openai", name: "OpenAI", model: "gpt-5-mini", input_per_m: 0.25, output_per_m: 2.0 },
    Provider { id: "gemini", name: "Google Gemini", model: "gemini-2.5-flash", input_per_m: 0.30, output_per_m: 2.50 },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// The usage table key, so cloud spending never mixes with search counts.
pub fn ledger_key(provider: &str) -> String {
    format!("cloud-{provider}")
}

pub fn model_valid(model: &str) -> bool {
    model.len() <= 100 && model.chars().all(|c| c.is_ascii_alphanumeric() || "._-:/@".contains(c))
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
    Usage { input: (chars / 3 + 64) as u64, output: payload["max_tokens"].as_u64().unwrap_or(1024) }
}

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
    value.as_str().map(str::to_string).unwrap_or_else(|| if value.is_null() { String::new() } else { value.to_string() })
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
        Value::Object(map) => Value::Object(map.iter().filter(|(key, _)| key.as_str() != "additionalProperties").map(|(key, value)| (key.clone(), gemini_schema(value))).collect()),
        Value::Array(items) => Value::Array(items.iter().map(gemini_schema).collect()),
        other => other.clone(),
    }
}

/// Builds the provider request from the generic payload.
pub fn build(provider: &str, model: &str, key: &str, payload: &Value) -> Result<Request, String> {
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let tools = payload["tools"].as_array().cloned().unwrap_or_default();
    let max_tokens = payload["max_tokens"].as_u64().unwrap_or(1024);
    let system = messages.iter().filter(|m| m["role"] == "system").map(|m| text(&m["content"])).collect::<Vec<_>>().join("\n\n");
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
                body["tools"] = tools.iter().map(|tool| json!({"type":"function","function":tool})).collect();
                body["parallel_tool_calls"] = false.into();
            }
            // GPT-5 models reason before answering; a light touch suits quick chat.
            if model.starts_with("gpt-5") {
                body["reasoning_effort"] = "low".into();
            }
            Ok(Request { url: "https://api.openai.com/v1/chat/completions".into(), headers: vec![("authorization", format!("Bearer {key}"))], body })
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
                        let id = out.iter().rev().find_map(|m| m["content"].as_array()?.iter().find(|b| b["type"] == "tool_use")?["id"].as_str().map(str::to_string)).unwrap_or_else(|| format!("toolu_buntini_{call}"));
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
            let mut headers = vec![("x-api-key", key.to_string()), ("anthropic-version", "2023-06-01".to_string())];
            // Current models think adaptively; low effort keeps a chat companion quick and cheap.
            if ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-fable-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"].iter().any(|prefix| model.starts_with(prefix)) {
                body["output_config"] = json!({"effort":"low"});
            }
            // If a safety classifier declines, the API re-runs the request on Anthropic's recommended fallback model.
            if ["claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"].iter().any(|prefix| model.starts_with(prefix)) {
                body["fallbacks"] = "default".into();
                headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".into()));
            }
            Ok(Request { url: "https://api.anthropic.com/v1/messages".into(), headers, body })
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
            let mut body = json!({"contents":contents,"generationConfig":{"maxOutputTokens":max_tokens}});
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
        _ => Err("Choose Anthropic, OpenAI or Gemini.".into()),
    }
}

const LINE_LIMIT: usize = 262_144;
const EVENT_LIMIT: usize = 1_048_576;

/// Splits a server-sent event stream into (event, data) pairs, with the same bounded
/// buffering as the local worker's line reader.
#[derive(Default)]
pub struct Sse {
    line: Vec<u8>,
    event: String,
    data: String,
}

impl Sse {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
        let mut events = Vec::new();
        for &byte in bytes {
            if byte != b'\n' {
                if self.line.len() >= LINE_LIMIT {
                    return Err("The cloud model sent a line larger than Buntini accepts.".into());
                }
                self.line.push(byte);
                continue;
            }
            let line = String::from_utf8(std::mem::take(&mut self.line)).map_err(|_| "The cloud model returned invalid text.".to_string())?;
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push((std::mem::take(&mut self.event), std::mem::take(&mut self.data)));
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

/// Turns one provider's stream events into text, at most one tool call, and usage.
pub struct Decoder {
    provider: &'static str,
    text: String,
    usage: Usage,
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
        Self { provider, text: String::new(), usage: Usage::default(), blocks: BTreeMap::new(), partial_json: BTreeMap::new(), parts: Vec::new(), call: None, stop: String::new(), done: false }
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
        let value: Value = serde_json::from_str(data).map_err(|_| "The cloud model sent an unreadable update.".to_string())?;
        if let Some(message) = value["error"]["message"].as_str() {
            return Err(format!("The cloud model reported an error: {}", message.chars().take(400).collect::<String>()));
        }
        match self.provider {
            "openai" => {
                let choice = &value["choices"][0];
                if let Some(piece) = choice["delta"]["content"].as_str() {
                    self.say(piece, on_token)?;
                }
                for call in choice["delta"]["tool_calls"].as_array().into_iter().flatten() {
                    if call["index"].as_u64().unwrap_or(0) != 0 {
                        continue;
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
                    self.usage = Usage { input: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0), output: value["usage"]["completion_tokens"].as_u64().unwrap_or(0) };
                }
            }
            "anthropic" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;
                match value["type"].as_str().unwrap_or("") {
                    "message_start" => {
                        let usage = &value["message"]["usage"];
                        self.usage.input = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"].iter().map(|key| usage[key].as_u64().unwrap_or(0)).sum();
                        self.usage.output = usage["output_tokens"].as_u64().unwrap_or(0);
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
                                append(self.blocks.entry(index).or_insert_with(|| json!({"type":"text","text":""})), "text", &piece);
                            }
                            "thinking_delta" => append(self.blocks.entry(index).or_insert_with(|| json!({"type":"thinking","thinking":""})), "thinking", &piece("thinking")),
                            "signature_delta" => {
                                self.blocks.entry(index).or_insert_with(|| json!({"type":"thinking","thinking":""}))["signature"] = piece("signature").into();
                            }
                            "input_json_delta" => {
                                let partial = self.partial_json.entry(index).or_default();
                                partial.push_str(&piece("partial_json"));
                                if partial.len() > 65_536 {
                                    return Err("The cloud model sent oversized tool arguments.".into());
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if let Some(raw) = self.partial_json.remove(&index) {
                            let input: Value = if raw.trim().is_empty() { json!({}) } else { serde_json::from_str(&raw).map_err(|_| "The cloud model sent unreadable tool arguments.".to_string())? };
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
                            self.usage = iterations.iter().fold(Usage::default(), |total, step| Usage {
                                input: total.input + ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"].iter().map(|key| step[key].as_u64().unwrap_or(0)).sum::<u64>(),
                                output: total.output + step["output_tokens"].as_u64().unwrap_or(0),
                            });
                        } else if let Some(output) = usage["output_tokens"].as_u64() {
                            self.usage.output = output;
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
                for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
                    if part["functionCall"].is_object() {
                        // One tool per step: later calls in the same reply are dropped.
                        if self.parts.iter().any(|kept| kept["functionCall"].is_object()) {
                            continue;
                        }
                    } else if let (Some(piece), false) = (part["text"].as_str(), part["thought"].as_bool().unwrap_or(false)) {
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
                    self.usage = Usage { input: usage["promptTokenCount"].as_u64().unwrap_or(0), output: usage["candidatesTokenCount"].as_u64().unwrap_or(0) + usage["thoughtsTokenCount"].as_u64().unwrap_or(0) };
                }
            }
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
                let tool = match self.call {
                    Some(_) if cut_off => return Err("The cloud model ran out of room mid-request. Try again.".into()),
                    Some((name, arguments)) => {
                        let arguments: Value = if arguments.trim().is_empty() { json!({}) } else { serde_json::from_str(&arguments).map_err(|_| "The cloud model sent unreadable tool arguments.".to_string())? };
                        Some(json!({"name":name,"arguments":arguments}))
                    }
                    None if !self.done => return Err("The cloud model stopped before finishing. Try again.".into()),
                    None => None,
                };
                Ok((Turn { text: self.text, tool, raw: None }, usage))
            }
            "anthropic" => {
                if self.stop == "refusal" {
                    return Err(REFUSED.into());
                }
                if !self.done {
                    return Err("The cloud model stopped before finishing. Try again.".into());
                }
                let mut blocks: Vec<Value> = self.blocks.into_values().collect();
                // After a fallback, only text from the declined attempt is sent back.
                if let Some(boundary) = blocks.iter().rposition(|block| block["type"] == "fallback") {
                    blocks = blocks.into_iter().enumerate().filter(|(index, block)| *index > boundary || block["type"] == "text").map(|(_, block)| block).collect();
                }
                let mut seen_tool = false;
                blocks.retain(|block| block["type"] != "tool_use" || !std::mem::replace(&mut seen_tool, true));
                let tool = blocks.iter().find(|block| block["type"] == "tool_use").map(|block| json!({"name":block["name"],"arguments":block["input"]}));
                if tool.is_some() && cut_off {
                    return Err("The cloud model ran out of room mid-request. Try again.".into());
                }
                Ok((Turn { text: self.text, tool, raw: Some(Value::Array(blocks)) }, usage))
            }
            _ => {
                if ["SAFETY", "PROHIBITED_CONTENT", "BLOCKLIST", "SPII", "RECITATION"].contains(&self.stop.as_str()) {
                    return Err(REFUSED.into());
                }
                let tool = self.parts.iter().find_map(|part| part["functionCall"].as_object()).map(|call| json!({"name":call.get("name").cloned().unwrap_or_default(),"arguments":call.get("args").cloned().unwrap_or_else(|| json!({}))}));
                if tool.is_some() && cut_off {
                    return Err("The cloud model ran out of room mid-request. Try again.".into());
                }
                if self.stop.is_empty() {
                    return Err("The cloud model stopped before finishing. Try again.".into());
                }
                Ok((Turn { text: self.text, tool, raw: Some(Value::Array(self.parts)) }, usage))
            }
        }
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
    let message = detail["error"]["message"].as_str().or_else(|| detail[0]["error"]["message"].as_str()).unwrap_or("").chars().take(300).collect::<String>();
    match status {
        401 | 403 => format!("{name} didn’t accept the API key. Check it in Settings → Brain."),
        404 => format!("{name} doesn’t recognise that model name. Check it in Settings → Brain."),
        429 => format!("{name} is limiting requests or your account is out of credit. {message}"),
        400 => format!("{name} couldn’t use this request: {message}"),
        _ => format!("{name} had a problem ({status}). Try again in a moment."),
    }
}

/// Streams one reply. Tokens go to `on_token` as they arrive.
pub async fn generate<F>(provider: &'static Provider, model: &str, key: &str, payload: &Value, cancel: Arc<AtomicBool>, mut on_token: F) -> Result<(Turn, Usage), Failure>
where
    F: FnMut(String) + Send,
{
    let request = build(provider.id, model, key, payload).map_err(Failure::Rejected)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| Failure::Rejected("Cannot prepare a secure connection.".into()))?;
    let mut builder = client.post(&request.url).json(&request.body);
    for (name, value) in &request.headers {
        builder = builder.header(*name, value);
    }
    let response = builder.send().await.map_err(|error| {
        if error.is_connect() || error.is_timeout() {
            Failure::Rejected(format!("Couldn’t reach {}. Check your internet connection.", provider.name))
        } else {
            Failure::Failed(format!("The connection to {} failed.", provider.name))
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Failure::Rejected(rejection(provider.name, status.as_u16(), &body.chars().take(65_536).collect::<String>())));
    }
    let mut sse = Sse::default();
    let mut decoder = Decoder::new(provider.id);
    let mut stream = response.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Failure::Failed("Task cancelled.".into()));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(Failure::Failed(format!("{} took too long to answer.", provider.name)));
        }
        tokio::select! {
            chunk = stream.next() => match chunk {
                None => break,
                Some(Err(_)) => return Err(Failure::Failed(format!("The connection to {} dropped. Try again.", provider.name))),
                Some(Ok(bytes)) => {
                    for (_, data) in sse.push(&bytes).map_err(Failure::Failed)? {
                        if decoder.event(&data, &mut on_token).map_err(Failure::Failed)? {
                            return decoder.finish().map_err(Failure::Failed);
                        }
                    }
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    decoder.finish().map_err(Failure::Failed)
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
                if decoder.event(&data, &mut |token| streamed.push_str(&token)).unwrap() {
                    return (decoder.finish(), streamed);
                }
            }
        }
        (decoder.finish(), streamed)
    }

    #[test]
    fn sse_lines_are_split_and_bounded() {
        let mut sse = Sse::default();
        let mut events = sse.push(b"event: a\r\ndata: one\r\n\r\n: comment\ndata: two\ndata:three\n").unwrap();
        events.extend(sse.push(b"\n").unwrap());
        assert_eq!(events, vec![("a".to_string(), "one".to_string()), (String::new(), "two\nthree".to_string())]);
        assert!(Sse::default().push(&vec![b'x'; LINE_LIMIT + 1]).is_err());
    }

    #[test]
    fn openai_requests_number_tool_calls_and_stream_answers() {
        let request = build("openai", "gpt-5-mini", "sk-test", &payload(None)).unwrap();
        assert_eq!(request.headers[0], ("authorization", "Bearer sk-test".to_string()));
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages[4]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[4]["tool_calls"][0]["function"]["arguments"], "{\"query\":\"lease\"}");
        assert_eq!(messages[5]["tool_call_id"], "call_1");
        assert_eq!(request.body["tools"][0]["function"]["name"], "search_files");
        assert_eq!(request.body["reasoning_effort"], "low");

        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":5}}\n\ndata: [DONE]\n\n";
        let (result, streamed) = decode("openai", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!((turn.text.as_str(), streamed.as_str()), ("Hello", "Hello"));
        assert_eq!(usage, Usage { input: 20, output: 5 });

        let call = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"search_files\",\"arguments\":\"{\\\"qu\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ery\\\":\\\"tax\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let (turn, _) = decode("openai", call).0.unwrap();
        assert_eq!(turn.tool.unwrap(), json!({"name":"search_files","arguments":{"query":"tax"}}));
        assert!(decode("openai", "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n").0.is_err(), "a stream that never finished is not an answer");
    }

    #[test]
    fn anthropic_keeps_thinking_for_the_next_step() {
        let raw = json!([{"type":"thinking","thinking":"","signature":"sig"},{"type":"tool_use","id":"toolu_1","name":"search_files","input":{"query":"lease"}}]);
        let request = build("anthropic", "claude-opus-5-5", "key", &payload(Some(raw.clone()))).unwrap();
        assert_eq!(request.body["system"], "Be brief.");
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5, "the system prompt is top-level");
        assert_eq!(messages[3]["content"], raw);
        assert_eq!(messages[4]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(request.body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(request.body["output_config"]["effort"], "low");
        assert_eq!(request.body["fallbacks"], "default");
        assert!(request.headers.contains(&("anthropic-beta", "server-side-fallback-2026-07-01".into())));
        let plain = build("anthropic", "claude-haiku-4-5", "key", &payload(None)).unwrap();
        assert!(plain.body.get("output_config").is_none() && plain.body.get("fallbacks").is_none());
        assert_eq!(plain.body["messages"][3]["content"][0]["id"], plain.body["messages"][4]["content"][0]["tool_use_id"]);

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
        assert_eq!(usage, Usage { input: 40, output: 42 });
        assert_eq!(turn.tool.unwrap(), json!({"name":"search_files","arguments":{"query":"lease"}}));
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
        assert_eq!(usage, Usage { input: 22, output: 9 });
        let types = turn.raw.unwrap().as_array().unwrap().iter().map(|block| block["type"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(types, ["text", "text"], "the fallback marker is optional to send back");
    }

    #[test]
    fn gemini_sends_the_key_in_a_header_and_keeps_signatures() {
        let raw = json!([{"functionCall":{"name":"search_files","args":{"query":"lease"}},"thoughtSignature":"sig"}]);
        let request = build("gemini", "gemini-2.5-flash", "AIza-test", &payload(Some(raw.clone()))).unwrap();
        assert!(!request.url.contains("AIza"), "the key never goes in the URL");
        assert_eq!(request.headers[0], ("x-goog-api-key", "AIza-test".to_string()));
        assert_eq!(request.body["systemInstruction"]["parts"][0]["text"], "Be brief.");
        let contents = request.body["contents"].as_array().unwrap();
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[3]["parts"], raw);
        assert_eq!(contents[4]["parts"][0]["functionResponse"]["name"], "search_files");
        assert!(request.body["tools"][0]["functionDeclarations"][0]["parameters"].get("additionalProperties").is_none());
        assert!(build("gemini", "../../x", "k", &payload(None)).is_err());

        let stream = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Sunny \"}]}}]}\r\n\r\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"today.\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":12,\"candidatesTokenCount\":4,\"thoughtsTokenCount\":6}}\r\n\r\n";
        let (result, streamed) = decode("gemini", stream);
        let (turn, usage) = result.unwrap();
        assert_eq!((turn.text.as_str(), streamed.as_str()), ("Sunny today.", "Sunny today."));
        assert_eq!(usage, Usage { input: 12, output: 10 });
        let blocked = "data: {\"candidates\":[{\"finishReason\":\"SAFETY\"}]}\n\n";
        assert_eq!(decode("gemini", blocked).0.unwrap_err(), REFUSED);
    }

    #[test]
    fn spending_is_estimated_high_and_priced_per_million() {
        let guess = estimate(&payload(None));
        assert_eq!(guess.output, 512);
        assert!(guess.input > 100);
        assert!((price(Usage { input: 1_000_000, output: 500_000 }, 4.0, 20.0) - 14.0).abs() < 1e-9);
        assert!(model_valid("claude-opus-5-5") && !model_valid("gpt 5") && !model_valid(&"a".repeat(101)));
        assert!(rejection("OpenAI", 401, "{}").contains("API key"));
        assert!(rejection("Anthropic", 400, "{\"error\":{\"message\":\"bad field\"}}").contains("bad field"));
    }
}
