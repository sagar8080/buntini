//! Approved, read-only lookups against fixed public APIs.
//!
//! The local model may only *propose* one call. Rust validates and normalizes its
//! arguments here, the user approves exactly what will be sent, and the approved
//! call is executed once against a constant HTTPS host. Responses are bounded and
//! become result cards; their text stays untrusted data for the summarizer.
pub mod duffel;
pub mod ebay;
pub mod everyday;
pub mod places;
mod summary;
mod weather_advice;
pub use summary::render_card_summary;
#[cfg(all(test, debug_assertions))]
pub(crate) use weather_advice::has_walk_advice;
pub use weather_advice::with_walk_advice;

use crate::{controller::AppState, types::*, web};
use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const USER_AGENT: &str = "Buntini/0.1 (private desktop companion)";
const MAX_CARDS: usize = 10;

pub struct Tool {
    pub name: &'static str,
    pub provider: &'static str,
    /// Keychain entry the tool needs, if any.
    pub key: Option<&'static str>,
}
pub const TOOLS: [Tool; 7] = [
    Tool {
        name: "web_search",
        provider: "Web search",
        key: None,
    },
    Tool {
        name: "weather",
        provider: "Open-Meteo",
        key: None,
    },
    Tool {
        name: "currency",
        provider: "Frankfurter (ECB)",
        key: None,
    },
    Tool {
        name: "wikipedia",
        provider: "Wikipedia",
        key: None,
    },
    Tool {
        name: "places",
        provider: "OpenStreetMap",
        key: None,
    },
    Tool {
        name: "ebay_search",
        provider: "eBay",
        key: Some("ebay"),
    },
    Tool {
        name: "flight_search",
        provider: "Duffel",
        key: Some("duffel"),
    },
];

/// A validated, normalized tool call. `arguments` include filled-in defaults so the
/// approval sheet and the request show the same values.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub tool: String,
    pub arguments: Value,
}

pub struct Preview {
    pub provider: String,
    pub host: String,
    pub summary: String,
    pub sends: Vec<CardField>,
    pub uses_key: bool,
}

pub struct Outcome {
    pub cards: Vec<ResultCard>,
}

pub fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Tools the planner may offer: keyless ones always, keyed ones once their key is saved.
pub fn available() -> Vec<&'static str> {
    TOOLS
        .iter()
        .filter(|tool| tool.key.map_or(true, |key| web::credential(key).is_ok()))
        .map(|tool| tool.name)
        .collect()
}

pub fn validate(tool_name: &str, arguments: &Value, prompt: &str) -> Result<Call, String> {
    let today = chrono::Local::now().date_naive();
    let arguments = match tool_name {
        // Web search keeps the policy that only the user's exact words reach a search engine.
        "web_search" => {
            let query = prompt.trim();
            if query.is_empty() || query.len() > 2000 {
                return Err("Web queries must contain 1–2,000 characters.".into());
            }
            json!({ "query": query })
        }
        "weather" => everyday::validate_weather(arguments)?,
        "currency" => everyday::validate_currency(arguments)?,
        "wikipedia" => everyday::validate_wikipedia(arguments)?,
        "places" => places::validate(arguments)?,
        "ebay_search" => ebay::validate(arguments)?,
        "flight_search" => duffel::validate(arguments, today)?,
        _ => return Err(format!("The tool '{tool_name}' is not available.")),
    };
    Ok(Call {
        tool: tool_name.into(),
        arguments,
    })
}

pub fn preview(call: &Call, settings: &Settings) -> Preview {
    let args = &call.arguments;
    match call.tool.as_str() {
        "weather" => everyday::preview_weather(args),
        "currency" => everyday::preview_currency(args),
        "wikipedia" => everyday::preview_wikipedia(args),
        "places" => places::preview(args),
        "ebay_search" => ebay::preview(args, &settings.ebay_marketplace),
        "flight_search" => duffel::preview(args),
        _ => {
            let (provider, host) = if settings.search_provider == "tavily" {
                ("Tavily", "api.tavily.com")
            } else {
                ("Brave Search", "api.search.brave.com")
            };
            Preview {
                provider: format!("Google in your browser; {provider} fallback"),
                host: format!("{}; {host} only if needed", crate::browser_session::SEARCH_HOST),
                summary: "Search Google in a dedicated tab in your existing or default browser, read relevant sources, and produce a sourced answer".into(),
                sends: vec![
                    CardField::new("Query", text_of(args, "query")),
                    CardField::new("Browser", "Uses your browser's normal profile. Existing cookies and sign-ins may apply, and pages may enter its history. Buntini reads only its research tab when permitted."),
                    CardField::new("Page reads", "Up to six selected public HTTPS result pages. If browser reading is unavailable, use the selected search API and public page retrieval."),
                ],
                uses_key: false,
            }
        }
    }
}

pub async fn execute(
    call: &Call,
    settings: &Settings,
    cancel: Arc<AtomicBool>,
) -> Result<Outcome, String> {
    let key = match tool(&call.tool).and_then(|tool| tool.key) {
        Some(name) => Some(web::credential(name).map_err(|_| {
            format!(
                "Add your {} credentials in Settings first.",
                tool(&call.tool).unwrap().provider
            )
        })?),
        None => None,
    };
    let args = &call.arguments;
    let throttle_cancel = cancel.clone();
    let work = async move {
        let cancel = throttle_cancel;
        match call.tool.as_str() {
            "weather" => everyday::weather(args).await,
            "currency" => everyday::currency(args).await,
            "wikipedia" => everyday::wikipedia(args).await,
            "places" => places::search(args, cancel.clone()).await,
            "ebay_search" => {
                ebay::search(
                    args,
                    key.as_deref().unwrap_or(""),
                    &settings.ebay_marketplace,
                )
                .await
            }
            "flight_search" => duffel::search(args, key.as_deref().unwrap_or("")).await,
            other => Err(format!("The tool '{other}' cannot run here.")),
        }
    };
    let mut cards = cancellable(work, cancel).await?;
    cards.truncate(MAX_CARDS);
    Ok(Outcome { cards })
}

/// Explicit online research has one route: the integrated browser-first web
/// workflow. A narrow direct request such as a forecast or conversion may still
/// use its specific API. Invalid proposals fall back to the exact public query.
pub async fn plan(
    state: &AppState,
    settings: &Settings,
    prompt: &str,
    cancel: Arc<AtomicBool>,
) -> Result<(Call, Option<String>), String> {
    let web = || validate("web_search", &Value::Null, prompt).map(|call| (call, None));
    if crate::search_intent::explicit_request(prompt).is_some() {
        return web();
    }
    if state.models.ready(settings).is_err() {
        return web();
    }
    let tools: Vec<Value> = available()
        .into_iter()
        .map(|name| json!({ "name": name }))
        .collect();
    let today = chrono::Local::now().format("%A %-d %B %Y");
    let messages = json!([
        {"role":"system","content":format!("Route the user's request to exactly one read-only online lookup and reply only with that tool call. Today is {today}. Prefer the most specific tool: weather for forecasts, currency for exchange rates, wikipedia for encyclopedic facts about a topic, places for nearby businesses or amenities, ebay_search for buying or pricing items, flight_search for flights (use IATA codes when you know them). Use web_search for news and everything else. Fill arguments only with details the user wrote; never invent dates, places or personal information.")},
        {"role":"user","content":prompt}
    ]);
    let request = json!({"messages":messages,"tools":tools,"max_tokens":256});
    match state
        .models
        .generate(settings, request, cancel, |_| {})
        .await
    {
        Ok(turn) => {
            let Some(call) = turn.tool else { return web() };
            let name = call.get("name").and_then(Value::as_str).unwrap_or("");
            if !available().contains(&name) {
                return web();
            }
            match validate(name, call.get("arguments").unwrap_or(&Value::Null), prompt) {
                Ok(call) => Ok((call, None)),
                Err(error) => {
                    state.audit("integration.plan_rejected", &format!("{name}: {error}"));
                    web().map(|(call, _)| (call, Some(format!("I couldn't fill in {name} from your message ({error}) so this will be a web search instead."))))
                }
            }
        }
        Err(error) if error == "Task cancelled." => Err(error),
        Err(_) => web().map(|(call, _)| {
            (
                call,
                Some("The local model is unavailable, so this will be a plain web search.".into()),
            )
        }),
    }
}

/// Plain-text rendering of cards: the answer when no model is available, and the
/// numbered evidence the summarizer must cite.
pub fn cards_text(cards: &[ResultCard]) -> String {
    cards
        .iter()
        .enumerate()
        .map(|(index, card)| {
            let mut text = format!("[{}] {}", index + 1, card.title);
            if !card.subtitle.is_empty() {
                text.push_str(&format!(" — {}", card.subtitle));
            }
            for field in &card.fields {
                text.push_str(&format!("\n{}: {}", field.label, field.value));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) async fn cancellable<T>(
    work: impl std::future::Future<Output = Result<T, String>>,
    cancel: Arc<AtomicBool>,
) -> Result<T, String> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(40)) => if cancel.load(Ordering::Relaxed) { return Err("Task cancelled.".into()); },
        }
    }
}

pub(crate) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .connect_timeout(std::time::Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .https_only(true)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| "Cannot initialize a secure connection.".to_string())
}

/// Reads a JSON response of at most `cap` bytes, mapping HTTP failures to plain advice.
pub(crate) async fn json_response(
    response: Result<reqwest::Response, reqwest::Error>,
    provider: &str,
    cap: usize,
) -> Result<Value, String> {
    let response = response.map_err(|error| {
        if error.is_timeout() {
            format!("{provider} took too long to answer. Try again in a moment.")
        } else {
            format!("Could not reach {provider}. Check your internet connection.")
        }
    })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => {
                format!("{provider} rejected the saved credentials. Check them in Settings.")
            }
            404 => format!("{provider} found nothing for this request."),
            400 | 422 => {
                format!("{provider} could not use these details. Check the values and try again.")
            }
            429 => format!("{provider} is limiting requests right now. Try again in a minute."),
            _ => format!("{provider} is temporarily unavailable (HTTP {status})."),
        });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| format!("The {provider} response was interrupted."))?;
        if bytes.len() + chunk.len() > cap {
            return Err(format!(
                "The {provider} response exceeded the safe size limit."
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("{provider} returned an unreadable response."))
}

pub(crate) fn card(
    kind: &str,
    title: impl Into<String>,
    subtitle: impl Into<String>,
    fields: Vec<CardField>,
    url: Option<String>,
    attribution: &str,
) -> ResultCard {
    ResultCard {
        kind: kind.into(),
        title: clip(&title.into(), 160),
        subtitle: clip(&subtitle.into(), 200),
        fields: fields
            .into_iter()
            .filter(|field| !field.value.trim().is_empty())
            .take(12)
            .map(|field| CardField::new(&field.label, clip(&field.value, 1600)))
            .collect(),
        url: url.filter(|url| safe_link(url)),
        attribution: attribution.into(),
        retrieved_at: now(),
    }
}

/// Only plain public HTTPS links become clickable.
pub(crate) fn safe_link(url: &str) -> bool {
    url.len() <= 2048 && crate::research::validate_public_url(url).is_ok()
}

pub(crate) fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}…", text.chars().take(limit - 1).collect::<String>())
    }
}

pub(crate) fn text_of(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

// Argument validation. Models emit Qwen tags where every value is a string, so
// numbers are accepted as JSON numbers or numeric strings.
pub(crate) fn object<'a>(
    value: &'a Value,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let map = value
        .as_object()
        .ok_or("Tool arguments must be an object.")?;
    if let Some(unknown) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unexpected argument '{unknown}'."));
    }
    Ok(map)
}

pub(crate) fn text(
    map: &Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > max || value.chars().any(char::is_control) {
                return Err(format!("'{key}' must be at most {max} visible characters."));
            }
            Ok(Some(value))
        }
        Some(_) => Err(format!("'{key}' must be text.")),
    }
}

pub(crate) fn required(map: &Map<String, Value>, key: &str, max: usize) -> Result<String, String> {
    text(map, key, max)?.ok_or_else(|| format!("'{key}' is required."))
}

pub(crate) fn number(
    map: &Map<String, Value>,
    key: &str,
    min: f64,
    max: f64,
) -> Result<Option<f64>, String> {
    let value = match map.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) if text.trim().is_empty() => return Ok(None),
        Some(Value::String(text)) => text.trim().replace(',', "").parse::<f64>().ok(),
        Some(_) => None,
    };
    match value {
        Some(value) if value.is_finite() && (min..=max).contains(&value) => Ok(Some(value)),
        _ => Err(format!("'{key}' must be a number between {min} and {max}.")),
    }
}

pub(crate) fn integer(
    map: &Map<String, Value>,
    key: &str,
    min: i64,
    max: i64,
) -> Result<Option<i64>, String> {
    match number(map, key, min as f64, max as f64) {
        Ok(Some(value)) if value.fract() == 0.0 => Ok(Some(value as i64)),
        Ok(None) => Ok(None),
        _ => Err(format!(
            "'{key}' must be a whole number between {min} and {max}."
        )),
    }
}

pub(crate) fn choice(
    map: &Map<String, Value>,
    key: &str,
    options: &[&str],
) -> Result<Option<String>, String> {
    match text(map, key, 40)? {
        None => Ok(None),
        Some(value) => {
            let value = value.to_lowercase().replace([' ', '-'], "_");
            if options.contains(&value.as_str()) {
                Ok(Some(value))
            } else {
                Err(format!("'{key}' must be one of: {}.", options.join(", ")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_always_uses_the_exact_prompt() {
        let call = validate(
            "web_search",
            &json!({"query":"something the model rewrote"}),
            "  latest rust release ",
        )
        .unwrap();
        assert_eq!(call.arguments, json!({"query":"latest rust release"}));
        assert!(validate("web_search", &Value::Null, " ").is_err());
    }

    #[test]
    fn unknown_tools_and_arguments_are_rejected() {
        assert!(validate("run_shell", &json!({}), "x").is_err());
        assert!(validate(
            "weather",
            &json!({"place":"Lisbon","url":"https://evil.example"}),
            "x"
        )
        .is_err());
        assert!(validate("weather", &json!("Lisbon"), "x").is_err());
    }

    #[test]
    fn numbers_accept_model_strings_but_stay_bounded() {
        let map = json!({"a":"3","b":2,"c":"2.5","d":"lots","e":true})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(integer(&map, "a", 1, 7).unwrap(), Some(3));
        assert_eq!(integer(&map, "b", 1, 7).unwrap(), Some(2));
        assert!(integer(&map, "c", 1, 7).is_err());
        assert!(integer(&map, "d", 1, 7).is_err());
        assert!(integer(&map, "e", 1, 7).is_err());
        assert!(integer(&map, "a", 4, 7).is_err());
        assert_eq!(integer(&map, "missing", 1, 7).unwrap(), None);
    }

    #[test]
    fn text_is_trimmed_bounded_and_free_of_control_characters() {
        let map = json!({"a":"  New   York ","b":"x\u{7}y","c":"   "})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(text(&map, "a", 20).unwrap().as_deref(), Some("New York"));
        assert!(text(&map, "a", 3).is_err());
        assert!(text(&map, "b", 20).is_err());
        assert_eq!(text(&map, "c", 20).unwrap(), None);
        assert!(required(&map, "c", 20).is_err());
    }

    #[test]
    fn result_links_reject_private_destinations_and_nonstandard_ports() {
        for url in [
            "https://127.0.0.1/admin",
            "https://[::1]/admin",
            "https://192.168.1.1/",
            "https://169.254.169.254/latest/meta-data/",
            "https://localhost/",
            "https://office.local/",
            "https://tauri.localhost/",
            "https://example.com:8443/",
            "https://name:password@example.com/",
        ] {
            assert!(!safe_link(url), "{url}");
        }
        assert!(safe_link("https://example.com/article?topic=rust#section"));
    }

    #[test]
    fn cards_drop_unsafe_links_and_render_numbered_text() {
        let unsafe_card = card(
            "place",
            "Café",
            "",
            vec![CardField::new("Hours", "9–5")],
            Some("javascript:alert(1)".into()),
            "OSM",
        );
        assert_eq!(unsafe_card.url, None);
        let http = card(
            "place",
            "Café",
            "",
            vec![],
            Some("http://example.com".into()),
            "OSM",
        );
        assert_eq!(http.url, None);
        let text = cards_text(&[
            unsafe_card.clone(),
            card("rate", "1 USD = 0.9 EUR", "ECB", vec![], None, "ECB"),
        ]);
        assert!(text.starts_with("[1] Café\nHours: 9–5"));
        assert!(text.contains("[2] 1 USD = 0.9 EUR — ECB"));
    }

    #[tokio::test]
    #[ignore = "contacts public APIs; run with --ignored live_keyless"]
    async fn live_keyless_lookups() {
        let settings = Settings::default();
        let cancel = Arc::new(AtomicBool::new(false));
        for (tool, args) in [
            ("weather", json!({"place":"Lisbon, Portugal"})),
            (
                "currency",
                json!({"from":"USD","to":"EUR,GBP","amount":100}),
            ),
            ("wikipedia", json!({"query":"Alan Turing"})),
            (
                "places",
                json!({"category":"cafe","near":"Baixa, Lisbon","radius_m":600}),
            ),
        ] {
            let call = validate(tool, &args, "x").unwrap();
            let outcome = execute(&call, &settings, cancel.clone())
                .await
                .unwrap_or_else(|error| panic!("{tool}: {error}"));
            assert!(!outcome.cards.is_empty(), "{tool}");
            println!("--- {tool}\n{}", cards_text(&outcome.cards));
        }
    }

    #[test]
    fn every_catalog_tool_validates_or_previews() {
        let settings = Settings::default();
        for tool in TOOLS {
            let preview = preview(
                &Call {
                    tool: tool.name.into(),
                    arguments: json!({"query":"x"}),
                },
                &settings,
            );
            assert!(!preview.provider.is_empty() && !preview.host.is_empty());
        }
    }
}
