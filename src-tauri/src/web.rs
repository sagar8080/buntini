use crate::types::{now, Source};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Debug)]
pub struct SearchError {
    pub message: String,
    pub retry_after: Option<std::time::Duration>,
}
impl From<String> for SearchError {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}
impl From<&str> for SearchError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
fn transient(message: &str, delay: std::time::Duration) -> SearchError {
    SearchError {
        message: message.into(),
        retry_after: Some(delay),
    }
}
fn retry_delay(header: Option<&str>) -> Option<std::time::Duration> {
    let seconds = match header {
        None => return Some(std::time::Duration::from_millis(500)),
        Some(value) => value.parse::<u64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc2822(value)
                .ok()
                .map(|date| {
                    (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                        .num_seconds()
                        .max(0) as u64
                })
        })?,
    };
    (seconds <= 3).then(|| std::time::Duration::from_secs(seconds))
}
pub async fn with_retry<F, Fut, R>(
    mut attempt: F,
    mut reserve: R,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<Source>, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Source>, SearchError>>,
    R: FnMut() -> Result<(), String>,
{
    for number in 0..2 {
        if cancel.load(Ordering::Relaxed) {
            return Err("Task cancelled.".into());
        }
        reserve()?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Task cancelled.".into());
        }
        let pending = attempt();
        tokio::pin!(pending);
        let result = loop {
            tokio::select! {result=&mut pending=>break result,_=tokio::time::sleep(std::time::Duration::from_millis(40))=>if cancel.load(Ordering::Relaxed){return Err("Task cancelled.".into());}}
        };
        match result {
            Ok(sources) => return Ok(sources),
            Err(error) => {
                if number == 1 || error.retry_after.is_none() {
                    return Err(error.message);
                }
                let delay = tokio::time::sleep(error.retry_after.unwrap());
                tokio::pin!(delay);
                loop {
                    tokio::select! {_=&mut delay=>break,_=tokio::time::sleep(std::time::Duration::from_millis(40))=>if cancel.load(Ordering::Relaxed){return Err("Task cancelled.".into());}}
                }
            }
        }
    }
    unreachable!()
}

pub fn provider_valid(provider: &str) -> Result<(), String> {
    if ["brave", "tavily", "ebay", "duffel"].contains(&provider) {
        Ok(())
    } else {
        Err("Unknown search provider.".into())
    }
}
/// Services whose keys Buntini keeps in the Keychain: online lookups and cloud models.
pub fn credential_valid(provider: &str) -> Result<(), String> {
    if provider_valid(provider).is_ok() || crate::cloud::provider(provider).is_some() {
        Ok(())
    } else {
        Err("Unknown service.".into())
    }
}
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "app.buntini.companion";
#[cfg(target_os = "macos")]
const LEGACY_KEYCHAIN_SERVICE: &str = "app.nexon.companion";
#[cfg(target_os = "macos")]
pub fn credential(provider: &str) -> Result<String, String> {
    use security_framework::passwords::*;
    credential_valid(provider)?;
    get_generic_password(KEYCHAIN_SERVICE, provider)
        .or_else(|_| {
            // A key saved under the former Nexon name moves to the current entry once.
            let bytes = get_generic_password(LEGACY_KEYCHAIN_SERVICE, provider)?;
            if set_generic_password(KEYCHAIN_SERVICE, provider, &bytes).is_ok() {
                let _ = delete_generic_password(LEGACY_KEYCHAIN_SERVICE, provider);
            }
            Ok(bytes)
        })
        .map_err(|_: security_framework::base::Error| {
            format!("Add your {provider} API key in Settings.")
        })
        .and_then(|bytes| {
            String::from_utf8(bytes).map_err(|_| "Invalid credential encoding.".into())
        })
}
#[cfg(not(target_os = "macos"))]
pub fn credential(_provider: &str) -> Result<String, String> {
    Err("API keys require macOS Keychain in this build.".into())
}
#[cfg(target_os = "macos")]
pub fn set_credential(provider: &str, key: &str) -> Result<(), String> {
    credential_valid(provider)?;
    if key.len() > 4096 || key.contains(['\r', '\n']) {
        return Err("Invalid API key.".into());
    }
    // An unmigrated legacy key must not reappear after the user replaces or clears it.
    let _ =
        security_framework::passwords::delete_generic_password(LEGACY_KEYCHAIN_SERVICE, provider);
    if key.trim().is_empty() {
        let _ = security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, provider);
        return Ok(());
    }
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        provider,
        key.trim().as_bytes(),
    )
    .map_err(|_| "macOS Keychain could not save the credential.".into())
}
#[cfg(not(target_os = "macos"))]
pub fn set_credential(_provider: &str, _key: &str) -> Result<(), String> {
    Err("API keys require macOS Keychain.".into())
}

pub async fn search(provider: &str, key: &str, query: &str) -> Result<Vec<Source>, SearchError> {
    provider_valid(provider)?;
    if query.trim().is_empty() || query.len() > 2000 {
        return Err("Enter a web query of up to 2,000 characters.".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| "Cannot initialize secure search.".to_string())?;
    let response=if provider=="brave" {
        client.get("https://api.search.brave.com/res/v1/web/search").header("X-Subscription-Token",key).query(&[("q",query),("count","6")]).send().await
    } else {
        client.post("https://api.tavily.com/search").bearer_auth(key).json(&json!({"query":query,"max_results":6,"search_depth":"basic","include_answer":false,"include_raw_content":false})).send().await
    }.map_err(|_|transient("Search could not connect. Check your internet connection and try again.",std::time::Duration::from_millis(500)))?;
    let status = response.status();
    if !status.is_success() {
        let message = match status.as_u16() {
            401 | 403 => "The search provider rejected this API key.",
            429 => "The provider quota or rate limit was reached.",
            _ => "The search provider returned an error. Please try again later.",
        };
        return Err(
            if [408, 429, 500, 502, 503, 504].contains(&status.as_u16()) {
                SearchError {
                    message: message.into(),
                    retry_after: retry_delay(
                        response
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|value| value.to_str().ok()),
                    ),
                }
            } else {
                message.into()
            },
        );
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            transient(
                "Search response was interrupted.",
                std::time::Duration::from_millis(500),
            )
        })?;
        if bytes.len() + chunk.len() > 1_048_576 {
            return Err("Search response exceeded the safe size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The search provider returned an invalid response.".to_string())?;
    let results = if provider == "brave" {
        data.pointer("/web/results")
    } else {
        data.get("results")
    }
    .and_then(Value::as_array)
    .ok_or("The search provider response did not contain results.")?;
    let retrieved_at = now();
    Ok(results
        .iter()
        .filter_map(|r| {
            let url = r.get("url")?.as_str()?;
            let parsed = reqwest::Url::parse(url).ok()?;
            if !["https", "http"].contains(&parsed.scheme()) || parsed.host_str().is_none() {
                return None;
            }
            Some(Source {
                title: r
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(url)
                    .chars()
                    .take(300)
                    .collect(),
                url: url.chars().take(4096).collect(),
                snippet: r
                    .get(if provider == "brave" {
                        "description"
                    } else {
                        "content"
                    })
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(2000)
                    .collect(),
                retrieved_at: retrieved_at.clone(),
            })
        })
        .take(6)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    #[tokio::test]
    async fn transient_transport_fixture_bills_each_attempt_and_budget_stops_retry() {
        let limits = |budget_usd| crate::budget::Limits {
            daily: 10,
            monthly: 10,
            budget_usd,
            unit_cost: 0.01,
        };
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = with_retry(
            || {
                let number = calls.get();
                calls.set(number + 1);
                std::future::ready(if number == 0 {
                    Err(transient("fixture503", std::time::Duration::from_millis(1)))
                } else {
                    Ok(vec![])
                })
            },
            || store.reserve("brave", limits(0.02)).map(|_| ()),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(calls.get(), 2);
        assert_eq!(store.usage("brave").unwrap().month, 2);
        assert!((store.usage("brave").unwrap().spend - 0.02).abs() < 0.000001);
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = with_retry(
            || {
                calls.set(calls.get() + 1);
                std::future::ready(Err(transient(
                    "fixture503",
                    std::time::Duration::from_millis(1),
                )))
            },
            || store.reserve("brave", limits(0.01)).map(|_| ()),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        assert_eq!(store.usage("brave").unwrap().month, 1);
    }
    #[tokio::test]
    async fn cancellation_prevents_second_charged_attempt() {
        let calls = std::cell::Cell::new(0);
        let charges = std::cell::Cell::new(0);
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let result = with_retry(
            || {
                calls.set(calls.get() + 1);
                token.store(true, Ordering::SeqCst);
                std::future::ready(Err(transient(
                    "fixture503",
                    std::time::Duration::from_secs(3),
                )))
            },
            || {
                charges.set(charges.get() + 1);
                Ok(())
            },
            cancel,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        assert_eq!(charges.get(), 1);
    }
    #[test]
    fn retry_after_longer_than_three_seconds_is_not_retried_early() {
        assert_eq!(retry_delay(Some("3")).unwrap().as_secs(), 3);
        assert!(retry_delay(Some("4")).is_none());
        assert!(retry_delay(Some("garbage")).is_none());
    }
}
