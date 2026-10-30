//! Sign in with ChatGPT, using OpenAI's flow for open-source, locally hosted apps:
//! each person signs in with their own ChatGPT account and Buntini uses their plan.
//!
//! The browser handles sign-in and consent; Buntini listens once on 127.0.0.1 for the
//! callback, exchanges the code with PKCE, and keeps the tokens in the Keychain. The app
//! registers itself on first sign-in (no client secret); the issued client ID and this
//! Mac's host ID are kept so later sign-ins reuse them.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
const ISSUER: &str = "https://auth.openai.com";
pub const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const CALLBACK_PATH: &str = "/auth/callback";
/// Where the session lives in the Keychain (alongside API keys, under its own name).
pub const KEYCHAIN_ID: &str = "chatgpt";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub id_token: String,
    pub sub: String,
    pub email: String,
    pub scopes: Vec<String>,
}
impl Session {
    pub fn plan_allowed(&self) -> bool {
        self.scopes.iter().any(|scope| scope == PLAN_SCOPE)
    }
}

fn random(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|_| "Couldn’t prepare a secure sign-in.".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// PKCE: the challenge is the base64url SHA-256 of the verifier, without padding.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn load() -> Option<Session> {
    crate::web::credential(KEYCHAIN_ID)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}
fn save(session: &Session) -> Result<(), String> {
    crate::web::set_credential(
        KEYCHAIN_ID,
        &serde_json::to_string(session).map_err(|e| e.to_string())?,
    )
}
fn forget() -> Result<(), String> {
    crate::web::set_credential(KEYCHAIN_ID, "")
}

/// The claims Buntini relies on from the ID token. It arrives straight from the token
/// endpoint over TLS, so (as OpenID Connect allows for that case) the issuer is trusted
/// through TLS; the audience, expiry and nonce are still checked.
pub fn verify_id_token(
    token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<(String, String), String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or("ChatGPT returned an unreadable sign-in.")?;
    let claims: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| "ChatGPT returned an unreadable sign-in.")?,
    )
    .map_err(|_| "ChatGPT returned an unreadable sign-in.")?;
    let audience_ok = match &claims["aud"] {
        serde_json::Value::String(aud) => aud == client_id,
        serde_json::Value::Array(list) => list.iter().any(|aud| aud == client_id),
        _ => false,
    };
    let authorized_party_ok = match claims.get("azp") {
        Some(party) => party.as_str() == Some(client_id),
        None => claims["aud"]
            .as_array()
            .is_none_or(|audiences| audiences.len() == 1),
    };
    if claims["iss"] != ISSUER
        || !audience_ok
        || !authorized_party_ok
        || claims["nonce"] != nonce
        || claims["exp"].as_i64().unwrap_or(0) <= now()
    {
        return Err("ChatGPT’s sign-in didn’t check out. Please try again.".into());
    }
    let sub = claims["sub"]
        .as_str()
        .filter(|sub| !sub.is_empty())
        .ok_or("ChatGPT’s sign-in didn’t include an account.")?;
    Ok((
        sub.to_string(),
        claims["email"].as_str().unwrap_or("").to_string(),
    ))
}

#[derive(Debug, PartialEq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: String,
    pub client_id: Option<String>,
    pub error: Option<String>,
}

/// Reads the first line of the browser's request: `GET /auth/callback?… HTTP/1.1`.
pub fn parse_callback(request_line: &str) -> Option<Callback> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    if !matches!(parts.next()?, "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
        || !target.starts_with('/')
        || target.starts_with("//")
    {
        return None;
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != CALLBACK_PATH {
        return None;
    }
    // Reject ambiguous query parameters rather than choosing one interpretation.
    let pairs: Vec<_> = url.query_pairs().collect();
    for key in ["code", "state", "client_id", "error"] {
        if pairs.iter().filter(|(name, _)| name == key).count() > 1 {
            return None;
        }
    }
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.to_string())
    };
    let state = get("state").filter(|value| !value.is_empty())?;
    let code = get("code").filter(|value| !value.is_empty());
    let error = get("error").filter(|value| !value.is_empty());
    if code.is_some() == error.is_some() {
        return None;
    }
    Some(Callback {
        code,
        state,
        client_id: get("client_id"),
        error,
    })
}

static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
static SESSION_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn cancel_sign_in() {
    if let Some(flag) = CANCEL.lock().unwrap().as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Buntini</title><body style=\"font:16px -apple-system,system-ui;display:grid;place-items:center;height:90vh;margin:0\"><p>{message}</p></body>";

/// Waits for the browser to come back to the loopback address, answering it with a
/// short page. Other requests (such as a favicon) get a 404.
async fn wait_for_callback(
    listener: tokio::net::TcpListener,
    cancel: Arc<AtomicBool>,
    expected_state: &str,
) -> Result<Callback, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("Sign-in was cancelled.".into());
        }
        if tokio::time::Instant::now() > deadline {
            return Err("Sign-in timed out. Try again when you’re ready.".into());
        }
        let Ok(Ok((mut stream, _))) =
            tokio::time::timeout(Duration::from_millis(250), listener.accept()).await
        else {
            continue;
        };
        let mut buffer = vec![0u8; 8192];
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(0);
        let text = String::from_utf8_lossy(&buffer[..read]).to_string();
        match parse_callback(text.lines().next().unwrap_or("")) {
            Some(callback) if callback.state == expected_state => {
                let message = if callback.error.is_some() {
                    "Sign-in was cancelled. You can close this tab."
                } else {
                    "You’re signed in to Buntini. You can close this tab."
                };
                let body = PAGE.replace("{message}", message);
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    stream.write_all(reply.as_bytes()),
                )
                .await;
                return Ok(callback);
            }
            _ => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                )
                .await;
            }
        }
    }
}

fn client() -> Result<reqwest::Client, String> {
    crate::web::private_client_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Cannot prepare a secure connection.".to_string())
}

async fn token_body(response: reqwest::Response) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "ChatGPT’s sign-in response was interrupted.".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > 65_536 {
            return Err("ChatGPT’s sign-in response exceeded the safe size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "ChatGPT returned an unreadable sign-in.".into())
}

fn token_error(body: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    value["error"]
        .as_str()
        .or_else(|| value["error"]["code"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Turns a token response into a session, checking the ID token and the plan permission.
fn session_from(
    response: &serde_json::Value,
    client_id: &str,
    nonce: Option<&str>,
    previous: Option<&Session>,
) -> Result<Session, String> {
    let access_token = response["access_token"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("ChatGPT didn’t return a sign-in token.")?
        .to_string();
    let refresh_token = response["refresh_token"]
        .as_str()
        .map(str::to_string)
        .or_else(|| previous.map(|session| session.refresh_token.clone()))
        .ok_or("ChatGPT didn’t return a refresh token.")?;
    if refresh_token.is_empty() {
        return Err("ChatGPT didn’t return a refresh token.".into());
    }
    let id_token = response["id_token"]
        .as_str()
        .map(str::to_string)
        .or_else(|| previous.map(|session| session.id_token.clone()))
        .unwrap_or_default();
    let (sub, email) = match (nonce, previous) {
        (Some(nonce), _) => verify_id_token(&id_token, client_id, nonce)?,
        (None, Some(previous)) => (previous.sub.clone(), previous.email.clone()),
        (None, None) => return Err("ChatGPT sign-in is incomplete.".into()),
    };
    let scopes = response["scope"]
        .as_str()
        .map(|scope| scope.split_whitespace().map(str::to_string).collect())
        .or_else(|| previous.map(|session| session.scopes.clone()))
        .unwrap_or_default();
    let current_time = now();
    let expires_at = current_time
        .checked_add(response["expires_in"].as_i64().unwrap_or(3600))
        .filter(|expiry| *expiry > current_time)
        .ok_or("ChatGPT returned an invalid sign-in expiry.")?;
    Ok(Session {
        client_id: client_id.into(),
        access_token,
        refresh_token,
        expires_at,
        id_token,
        sub,
        email,
        scopes,
    })
}

/// Opens ChatGPT's sign-in in the browser and waits for the user to finish.
pub async fn sign_in(host_id: &str, saved_client_id: Option<String>) -> Result<Session, String> {
    let generation = SESSION_GENERATION.load(Ordering::SeqCst);
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:1455").await {
        Ok(listener) => listener,
        Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Couldn’t start sign-in on this Mac.".to_string())?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let (verifier, state, nonce) = (random(32)?, random(24)?, random(24)?);
    let first_time = saved_client_id.is_none();
    let client_id = saved_client_id
        .clone()
        .unwrap_or_else(|| "dynamic_agent_client".into());
    let mut params = vec![
        ("client_id", client_id.clone()),
        ("ext_agent_host_id", host_id.to_string()),
        ("response_type", "code".into()),
        ("redirect_uri", redirect_uri.clone()),
        ("scope", SCOPES.into()),
        ("resource", RESOURCE.into()),
        ("state", state.clone()),
        ("nonce", nonce.clone()),
        ("code_challenge_method", "S256".into()),
        ("code_challenge", challenge(&verifier)),
    ];
    if first_time {
        params.push(("agent_name_hint", "Buntini".into()));
    }
    let url = reqwest::Url::parse_with_params(AUTHORIZE, &params).map_err(|e| e.to_string())?;
    let cancel = Arc::new(AtomicBool::new(false));
    *CANCEL.lock().unwrap() = Some(cancel.clone());
    std::process::Command::new("/usr/bin/open")
        .arg(url.as_str())
        .status()
        .map_err(|_| "Couldn’t open your browser.".to_string())?;
    let callback = wait_for_callback(listener, cancel, &state).await;
    *CANCEL.lock().unwrap() = None;
    let callback = callback?;
    if callback.state != state {
        return Err("Sign-in didn’t match this request. Please try again.".into());
    }
    if callback.error.is_some() {
        return Err("Sign-in was cancelled.".into());
    }
    let issued = match (&saved_client_id, &callback.client_id) {
        (Some(saved), Some(returned)) if saved != returned => {
            return Err("ChatGPT returned a different app registration. Please try again.".into())
        }
        (Some(saved), _) => saved.clone(),
        (None, Some(returned)) if returned != "dynamic_agent_client" => returned.clone(),
        (None, _) => {
            return Err("ChatGPT didn’t finish registering Buntini. Please try again.".into())
        }
    };
    let code = callback
        .code
        .ok_or("ChatGPT didn’t return a sign-in code.")?;
    let response = client()?
        .post(TOKEN)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", issued.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .await
        .map_err(|_| "Couldn’t reach ChatGPT. Check your internet connection.".to_string())?;
    let status = response.status();
    let body = token_body(response).await?;
    if !status.is_success() {
        return Err(if token_error(&body) == "invalid_grant" {
            "That sign-in expired. Please try again.".into()
        } else {
            format!("ChatGPT sign-in failed ({status}).")
        });
    }
    let session = session_from(
        &serde_json::from_str(&body).map_err(|_| "ChatGPT returned an unreadable sign-in.")?,
        &issued,
        Some(&nonce),
        None,
    )?;
    let _guard = REFRESHING.lock().await;
    if SESSION_GENERATION.load(Ordering::SeqCst) != generation {
        return Err("Sign-in was cancelled.".into());
    }
    save(&session)?;
    Ok(session)
}

static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A usable access token, refreshing it shortly before it expires. Only one refresh runs
/// at a time, because each one replaces the refresh token.
pub async fn access_token() -> Result<String, String> {
    let _guard = REFRESHING.lock().await;
    let session =
        load().ok_or("Sign in with ChatGPT in Settings → Brain to use your ChatGPT plan.")?;
    if !session.plan_allowed() {
        return Err("Your ChatGPT sign-in doesn’t allow Buntini to use your plan. Sign in again and allow plan usage.".into());
    }
    if session.expires_at - 120 > now() {
        return Ok(session.access_token);
    }
    let response = client()?
        .post(TOKEN)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", session.client_id.as_str()),
            ("refresh_token", session.refresh_token.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .await
        .map_err(|_| "Couldn’t reach ChatGPT. Check your internet connection.".to_string())?;
    let status = response.status();
    let body = token_body(response).await?;
    if !status.is_success() {
        let code = token_error(&body);
        if [
            "invalid_grant",
            "invalid_refresh_token",
            "token_expired",
            "refresh_token_expired",
            "refresh_token_invalidated",
            "refresh_token_reused",
        ]
        .contains(&code.as_str())
        {
            forget()?;
            return Err(
                "Your ChatGPT sign-in has ended. Sign in again in Settings → Brain.".into(),
            );
        }
        return Err(format!(
            "ChatGPT couldn’t renew your sign-in right now ({status}). Try again in a moment."
        ));
    }
    let refreshed = session_from(
        &serde_json::from_str(&body).map_err(|_| "ChatGPT returned an unreadable sign-in.")?,
        &session.client_id,
        None,
        Some(&session),
    )?;
    save(&refreshed)?;
    Ok(refreshed.access_token)
}

/// Removes the local session and attempts remote refresh-token revocation. Returns
/// an error if local removal fails, otherwise whether revocation was confirmed.
pub async fn sign_out() -> Result<bool, String> {
    SESSION_GENERATION.fetch_add(1, Ordering::SeqCst);
    cancel_sign_in();
    // Serialize with token refresh, so an in-flight refresh cannot restore a
    // session after sign-out or Delete everything has removed it.
    let _guard = REFRESHING.lock().await;
    let session = load();
    forget()?;
    *MODELS.lock().unwrap() = None;
    let Some(session) = session else {
        return Ok(true);
    };
    let confirmed = async {
        let discovery = crate::integrations::json_response(
            client().ok()?.get(DISCOVERY).send().await,
            "ChatGPT",
            65_536,
        )
        .await
        .ok()?;
        let endpoint = revocation_endpoint(discovery["revocation_endpoint"].as_str()?)?;
        let response = client()
            .ok()?
            .post(endpoint)
            .form(&[
                ("token", session.refresh_token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", session.client_id.as_str()),
            ])
            .send()
            .await
            .ok()?;
        Some(response.status().is_success())
    }
    .await
    .unwrap_or(false);
    Ok(confirmed)
}

/// Discovery may describe endpoints, but it cannot change the origin to which
/// this app discloses a refresh token (or downgrade transport to plain HTTP).
fn revocation_endpoint(raw: &str) -> Option<reqwest::Url> {
    let endpoint = reqwest::Url::parse(raw).ok()?;
    (endpoint.scheme() == "https"
        && endpoint.host_str() == Some("auth.openai.com")
        && endpoint.port_or_known_default() == Some(443)
        && endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.fragment().is_none())
    .then_some(endpoint)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
}

/// Models this ChatGPT account may use, in the server's order.
pub fn listed_models(value: &serde_json::Value) -> Vec<ModelInfo> {
    value["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["visibility"] == "list")
        .filter_map(|model| {
            let slug = model["slug"].as_str()?.to_string();
            Some(ModelInfo {
                display_name: model["display_name"].as_str().unwrap_or(&slug).to_string(),
                slug,
            })
        })
        .collect()
}

static MODELS: Mutex<Option<(String, Vec<ModelInfo>)>> = Mutex::new(None);

pub async fn models() -> Result<Vec<ModelInfo>, String> {
    let token = access_token().await?;
    let session = load().map(|session| session.sub).unwrap_or_default();
    if let Some((owner, cached)) = MODELS.lock().unwrap().clone() {
        if owner == session && !cached.is_empty() {
            return Ok(cached);
        }
    }
    let response = client()?
        .get(format!("{RESOURCE}/models"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "Couldn’t reach ChatGPT.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "ChatGPT didn’t list models ({}).",
            response.status()
        ));
    }
    let list = listed_models(
        &crate::integrations::json_response(Ok(response), "ChatGPT", 1_048_576).await?,
    );
    *MODELS.lock().unwrap() = Some((session, list.clone()));
    Ok(list)
}

/// Plain-language advice for ChatGPT plan errors (HTTP responses or failed streams).
pub fn plan_error(code: &str, param: &str) -> Option<String> {
    Some(match code {
        "subscription_sharing_usage_limit_exceeded" => "You’ve reached the limit for using your ChatGPT plan in apps like Buntini. Check ChatGPT → Settings → Usage.".into(),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => "ChatGPT is busy right now. Try again in a moment.".into(),
        "subscription_sharing_invalid_user" => "ChatGPT didn’t accept this sign-in. If you disconnected Buntini in ChatGPT, sign in again in Settings → Brain.".into(),
        "subscription_sharing_user_not_eligible" => "Your ChatGPT workspace doesn’t allow its plan to be used in other apps.".into(),
        "subscription_sharing_unsupported_capability" => format!("ChatGPT doesn’t support {} here. Choose another model in Settings → Brain.", if param.is_empty() { "that request" } else { param }),
        _ => return None,
    })
}

/// A stable, opaque ID for this Mac, kept so later sign-ins reuse the same registration.
fn host_id(store: &crate::store::Store) -> Result<String, String> {
    if let Some(id) = store.preference("chatgpt-host-id")? {
        return Ok(id);
    }
    let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    store.set_preference("chatgpt-host-id", &id)?;
    Ok(id)
}

#[tauri::command]
pub async fn chatgpt_sign_in(
    state: tauri::State<'_, crate::controller::AppState>,
) -> Result<String, String> {
    let (host, saved) = {
        let store = state.store.lock().unwrap();
        (host_id(&store)?, store.preference("chatgpt-client-id")?)
    };
    let session = sign_in(&host, saved).await?;
    state
        .store
        .lock()
        .unwrap()
        .set_preference("chatgpt-client-id", &session.client_id)?;
    state.audit("chatgpt.signed_in", "Signed in with ChatGPT");
    if !session.plan_allowed() {
        return Err("You’re signed in, but Buntini wasn’t allowed to use your ChatGPT plan. Sign in again and allow plan usage.".into());
    }
    Ok(session.email)
}

#[tauri::command]
pub fn chatgpt_cancel_sign_in() {
    cancel_sign_in();
}

/// Signs out; false when ChatGPT couldn't confirm it revoked the sign-in (it is still
/// removed from this Mac).
#[tauri::command]
pub async fn chatgpt_sign_out(
    state: tauri::State<'_, crate::controller::AppState>,
) -> Result<bool, String> {
    let confirmed = sign_out().await?;
    state.audit(
        "chatgpt.signed_out",
        if confirmed {
            "Signed out of ChatGPT"
        } else {
            "Signed out on this Mac; ChatGPT didn’t confirm revocation"
        },
    );
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: serde_json::Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (a, b) = (random(32).unwrap(), random(32).unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
    }

    #[test]
    fn callbacks_are_read_from_the_request_line_only_on_the_callback_path() {
        let callback = parse_callback(
            "GET /auth/callback?code=abc&scope=openid&state=s1&client_id=oaiapp_123 HTTP/1.1",
        )
        .unwrap();
        assert_eq!(
            callback,
            Callback {
                code: Some("abc".into()),
                state: "s1".into(),
                client_id: Some("oaiapp_123".into()),
                error: None
            }
        );
        assert_eq!(
            parse_callback("GET /auth/callback?error=access_denied&state=s1 HTTP/1.1")
                .unwrap()
                .error
                .as_deref(),
            Some("access_denied")
        );
        assert!(parse_callback("GET /favicon.ico HTTP/1.1").is_none());
        assert!(
            parse_callback("GET /callback?code=a&state=s HTTP/1.1").is_none(),
            "only /auth/callback"
        );
        assert!(parse_callback("POST /auth/callback?code=a&state=s HTTP/1.1").is_none());
        assert!(
            parse_callback("GET /auth/callback?code=a HTTP/1.1").is_none(),
            "state is required"
        );
    }

    #[test]
    fn callbacks_reject_ambiguous_or_incomplete_credentials() {
        for line in [
            "GET //auth/callback?code=a&state=s HTTP/1.1",
            "GET /auth/callback?code=a&state=s",
            "GET /auth/callback?code=a&state=s HTTP/1.1 extra",
            "GET /auth/callback?code=a&state=s&state=other HTTP/1.1",
            "GET /auth/callback?code=a&code=b&state=s HTTP/1.1",
            "GET /auth/callback?code=a&error=denied&state=s HTTP/1.1",
            "GET /auth/callback?code=&state=s HTTP/1.1",
            "GET /auth/callback?code=a&state= HTTP/1.1",
        ] {
            assert!(parse_callback(line).is_none(), "{line}");
        }
    }

    #[tokio::test]
    async fn foreign_callback_cannot_consume_the_pending_sign_in() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let pending = tokio::spawn(wait_for_callback(
            listener,
            Arc::new(AtomicBool::new(false)),
            "expected",
        ));
        for state in ["foreign", "expected"] {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(format!("GET /auth/callback?code=abc&state={state} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes()).await.unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            assert!(reply.starts_with(if state == "expected" {
                "HTTP/1.1 200"
            } else {
                "HTTP/1.1 404"
            }));
        }
        assert_eq!(pending.await.unwrap().unwrap().state, "expected");
    }

    #[test]
    fn revocation_cannot_send_refresh_tokens_to_another_origin() {
        assert!(revocation_endpoint("https://auth.openai.com/oauth/revoke").is_some());
        for endpoint in [
            "http://auth.openai.com/oauth/revoke",
            "https://evil.example/revoke",
            "https://auth.openai.com.evil.example/revoke",
            "https://auth.openai.com:8443/revoke",
            "https://user@auth.openai.com/revoke",
            "https://auth.openai.com/revoke#fragment",
        ] {
            assert!(revocation_endpoint(endpoint).is_none(), "{endpoint}");
        }
    }

    #[test]
    fn id_tokens_must_match_this_sign_in() {
        let good = jwt(
            serde_json::json!({"iss": ISSUER, "aud": "oaiapp_1", "nonce": "n1", "exp": now() + 600, "sub": "user-1", "email": "sam@example.com"}),
        );
        assert_eq!(
            verify_id_token(&good, "oaiapp_1", "n1").unwrap(),
            ("user-1".to_string(), "sam@example.com".to_string())
        );
        assert!(
            verify_id_token(&good, "oaiapp_2", "n1").is_err(),
            "audience"
        );
        assert!(verify_id_token(&good, "oaiapp_1", "n2").is_err(), "nonce");
        let expired = jwt(
            serde_json::json!({"iss": ISSUER, "aud": ["oaiapp_1"], "nonce": "n1", "exp": now() - 1, "sub": "user-1"}),
        );
        assert!(
            verify_id_token(&expired, "oaiapp_1", "n1").is_err(),
            "expiry"
        );
        let foreign = jwt(
            serde_json::json!({"iss": "https://evil.example", "aud": "oaiapp_1", "nonce": "n1", "exp": now() + 600, "sub": "u"}),
        );
        assert!(
            verify_id_token(&foreign, "oaiapp_1", "n1").is_err(),
            "issuer"
        );
        let mut claims = serde_json::json!({"iss": ISSUER, "aud": ["oaiapp_1", "other-app"], "nonce": "n1", "exp": now() + 600, "sub": "u"});
        assert!(verify_id_token(&jwt(claims.clone()), "oaiapp_1", "n1").is_err());
        claims["azp"] = "other-app".into();
        assert!(verify_id_token(&jwt(claims.clone()), "oaiapp_1", "n1").is_err());
        claims["azp"] = "oaiapp_1".into();
        assert!(verify_id_token(&jwt(claims), "oaiapp_1", "n1").is_ok());
    }

    #[test]
    fn refreshes_keep_identity_and_take_the_new_tokens() {
        let first = Session {
            client_id: "oaiapp_1".into(),
            access_token: "a1".into(),
            refresh_token: "r1".into(),
            expires_at: 0,
            id_token: "i".into(),
            sub: "user-1".into(),
            email: "sam@example.com".into(),
            scopes: vec![PLAN_SCOPE.into()],
        };
        let refreshed = session_from(
            &serde_json::json!({"access_token": "a2", "refresh_token": "r2", "expires_in": 3600}),
            "oaiapp_1",
            None,
            Some(&first),
        )
        .unwrap();
        assert_eq!(
            (
                refreshed.access_token.as_str(),
                refreshed.refresh_token.as_str(),
                refreshed.sub.as_str()
            ),
            ("a2", "r2", "user-1")
        );
        assert!(refreshed.expires_at > now() + 3500 && refreshed.plan_allowed());
        for expiry in [i64::MAX, 0, -1] {
            assert!(session_from(
                &serde_json::json!({"access_token": "a2", "expires_in": expiry}),
                "oaiapp_1",
                None,
                Some(&first)
            )
            .is_err());
        }
        for field in ["access_token", "refresh_token"] {
            let mut response = serde_json::json!({"access_token": "a2", "refresh_token": "r2"});
            response[field] = "".into();
            assert!(session_from(&response, "oaiapp_1", None, Some(&first)).is_err());
        }
        let without_plan = Session {
            scopes: vec!["openid".into()],
            ..first
        };
        assert!(!without_plan.plan_allowed());
    }

    #[test]
    fn model_lists_and_plan_errors_read_naturally() {
        let list = listed_models(
            &serde_json::json!({"models": [{"slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list"}, {"slug": "hidden", "visibility": "hide"}]}),
        );
        assert_eq!(
            list,
            [ModelInfo {
                slug: "gpt-6.1-sol".into(),
                display_name: "GPT-6.1 Sol".into()
            }]
        );
        assert!(plan_error("subscription_sharing_usage_limit_exceeded", "")
            .unwrap()
            .contains("Settings → Usage"));
        assert!(
            plan_error("subscription_sharing_unsupported_capability", "tools")
                .unwrap()
                .contains("tools")
        );
        assert!(plan_error("something_else", "").is_none());
    }
}
