//! Mastodon, signed in with the person's own account on their own server: unread mentions
//! beside the notch with the newest on the card, and posting an answer from the island after
//! Allow. The browser handles sign-in: Buntini registers itself with the server, listens once
//! on 127.0.0.1 for the callback, exchanges the code with PKCE and keeps the token in the
//! Keychain. Only the server's name and the account's handle are kept in a file. Buntini never
//! marks anything read or posts anything the person didn't allow.
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KEYCHAIN_ID: &str = "mastodon";
/// What Buntini asks to do: read mentions and post.
const SCOPES: &str = "read:notifications read:accounts write:statuses";
/// How often mentions are read.
const EVERY: Duration = Duration::from_secs(120);
/// The most a post says.
const LONGEST_POST: usize = 500;

/// Signed in: the server and the handle, kept in a file. The token is in the Keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub server: String,
    pub handle: String,
}

/// The token and the app's own registration on the server, kept together in the Keychain.
#[derive(Serialize, Deserialize)]
struct Secret {
    token: String,
    client_id: String,
    client_secret: String,
}

/// Unread mentions, as last read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mentions {
    pub count: usize,
    /// "Ana: Sounds good, see you at 3."
    pub newest: Option<String>,
}

static MENTIONS: Mutex<Mentions> = Mutex::new(Mentions { count: 0, newest: None });

fn mentions() -> MutexGuard<'static, Mentions> {
    MENTIONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// "mastodon.social" from what was typed: an address or a name, without a path.
pub fn server_name(typed: &str) -> Result<String, String> {
    let typed = typed.trim().trim_end_matches('/');
    let host = typed.strip_prefix("https://").or_else(|| typed.strip_prefix("http://")).unwrap_or(typed);
    let host = host.split(['/', '?', '#']).next().unwrap_or_default().trim_start_matches('@');
    // "@ana@mastodon.social" names the server after the second @.
    let host = host.rsplit('@').next().unwrap_or_default().to_ascii_lowercase();
    let valid = host.contains('.')
        && host.len() <= 253
        && !host.starts_with('.')
        && host.split('.').all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-') && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && !host.split('.').all(|label| label.chars().all(|c| c.is_ascii_digit()));
    if valid {
        Ok(host)
    } else {
        Err("That doesn’t look like a Mastodon server, such as mastodon.social.".into())
    }
}

/// HTML from a post, as plain text on one line.
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    let mut tag = String::new();
    for c in html.chars() {
        match (inside, c) {
            (false, '<') => {
                inside = true;
                tag.clear();
            }
            (true, '>') => {
                inside = false;
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
                if ["br", "p", "div", "li"].contains(&name.as_str()) {
                    out.push(' ');
                }
            }
            (true, c) => tag.push(c),
            (false, c) => out.push(c),
        }
    }
    let out = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Unread mentions from the server's notifications, newest first, after the last one read.
pub fn read_mentions(notifications: &Value) -> Mentions {
    let list = notifications.as_array().cloned().unwrap_or_default();
    let newest = list.first().and_then(|first| {
        let who = first.pointer("/account/display_name").and_then(Value::as_str).filter(|name| !name.trim().is_empty()).or_else(|| first.pointer("/account/acct").and_then(Value::as_str))?;
        let said = plain(first.pointer("/status/content").and_then(Value::as_str).unwrap_or_default());
        let said: String = said.chars().take(140).collect();
        Some(if said.is_empty() { who.trim().to_string() } else { format!("{}: {said}", who.trim()) })
    });
    Mentions { count: list.len(), newest }
}

fn data_dir() -> Option<PathBuf> {
    crate::agents::socket_dir().map(PathBuf::from)
}

fn account_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("mastodon.json"))
}

/// Who's signed in, if anyone.
pub fn account() -> Option<Account> {
    account_path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn secret() -> Option<Secret> {
    crate::web::credential(KEYCHAIN_ID).ok().and_then(|text| serde_json::from_str(&text).ok())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(20)).user_agent(concat!("Buntini/", env!("CARGO_PKG_VERSION"))).build().unwrap_or_default()
}

/// Signs in on `server` through the browser and keeps the token. Answers with the handle.
pub async fn sign_in(server: &str) -> Result<Account, String> {
    let server = server_name(server)?;
    let base = format!("https://{server}");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|_| "Couldn’t get ready for the sign-in.".to_string())?;
    let port = listener.local_addr().map_err(|error| error.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}{}", crate::chatgpt::CALLBACK_PATH);
    let http = client();
    let app: Value = http
        .post(format!("{base}/api/v1/apps"))
        .form(&[("client_name", "Buntini"), ("redirect_uris", redirect.as_str()), ("scopes", SCOPES)])
        .send()
        .await
        .map_err(|_| format!("Couldn’t reach {server}."))?
        .error_for_status()
        .map_err(|_| format!("{server} didn’t let Buntini register."))?
        .json()
        .await
        .map_err(|_| format!("{server} isn’t answering like a Mastodon server."))?;
    let client_id = app.get("client_id").and_then(Value::as_str).ok_or("The server didn’t say who Buntini is.")?.to_string();
    let client_secret = app.get("client_secret").and_then(Value::as_str).ok_or("The server didn’t say who Buntini is.")?.to_string();
    let (verifier, state) = (crate::chatgpt::random(32)?, crate::chatgpt::random(24)?);
    let mut authorize = reqwest::Url::parse(&format!("{base}/oauth/authorize")).map_err(|error| error.to_string())?;
    authorize
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("code_challenge", &crate::chatgpt::challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    std::process::Command::new("/usr/bin/open").arg(authorize.as_str()).spawn().map_err(|_| "Couldn’t open your browser.".to_string())?;
    let callback = crate::chatgpt::wait_for_callback(listener, Arc::new(AtomicBool::new(false)), &state).await?;
    let code = callback.code.filter(|_| callback.error.is_none()).ok_or("Sign-in was cancelled.")?;
    let token: Value = http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
            ("scope", SCOPES),
        ])
        .send()
        .await
        .map_err(|_| format!("Couldn’t reach {server}."))?
        .error_for_status()
        .map_err(|_| "The server didn’t accept the sign-in.".to_string())?
        .json()
        .await
        .map_err(|_| "The server’s answer didn’t read.".to_string())?;
    let token = token.get("access_token").and_then(Value::as_str).ok_or("The server didn’t give a token.")?.to_string();
    let me: Value = http.get(format!("{base}/api/v1/accounts/verify_credentials")).bearer_auth(&token).send().await.map_err(|_| format!("Couldn’t reach {server}."))?.json().await.map_err(|_| "The account didn’t read.".to_string())?;
    let handle = format!("@{}@{server}", me.get("acct").and_then(Value::as_str).unwrap_or("you"));
    let secret = serde_json::to_string(&Secret { token, client_id, client_secret }).map_err(|error| error.to_string())?;
    crate::web::set_credential(KEYCHAIN_ID, &secret)?;
    let account = Account { server, handle };
    let path = account_path().ok_or("Buntini is still starting.")?;
    std::fs::write(path, serde_json::to_vec(&account).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    Ok(account)
}

/// Signs out: the token is given back to the server and forgotten here.
pub async fn sign_out() -> Result<(), String> {
    if let (Some(account), Some(secret)) = (account(), secret()) {
        let _ = client()
            .post(format!("https://{}/oauth/revoke", account.server))
            .form(&[("client_id", secret.client_id.as_str()), ("client_secret", secret.client_secret.as_str()), ("token", secret.token.as_str())])
            .send()
            .await;
    }
    crate::web::set_credential(KEYCHAIN_ID, "")?;
    if let Some(path) = account_path() {
        let _ = std::fs::remove_file(path);
    }
    *mentions() = Mentions::default();
    crate::island::refresh();
    Ok(())
}

/// The unread mentions now: those after the server's "read up to here" for notifications.
async fn fetch(account: &Account, secret: &Secret) -> Result<Mentions, String> {
    let base = format!("https://{}", account.server);
    let http = client();
    let markers: Value = http.get(format!("{base}/api/v1/markers")).query(&[("timeline[]", "notifications")]).bearer_auth(&secret.token).send().await.map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
    let mut query = vec![("types[]", "mention".to_string()), ("limit", "40".to_string())];
    if let Some(last) = markers.pointer("/notifications/last_read_id").and_then(Value::as_str) {
        query.push(("since_id", last.to_string()));
    }
    let notifications: Value = http.get(format!("{base}/api/v1/notifications")).query(&query).bearer_auth(&secret.token).send().await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
    Ok(read_mentions(&notifications))
}

/// Reads mentions every couple of minutes while signed in and the screen is on.
pub fn start() {
    tauri::async_runtime::spawn(async {
        loop {
            if !crate::media::screen_away() {
                if let (Some(account), Some(secret)) = (account(), secret()) {
                    if let Ok(found) = fetch(&account, &secret).await {
                        let changed = *mentions() != found;
                        if changed {
                            *mentions() = found;
                            crate::island::refresh();
                        }
                    }
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// The unread mentions for the island: the count, and the newest.
pub fn unread() -> Option<Mentions> {
    account()?;
    Some(mentions().clone()).filter(|found| found.count > 0)
}

/// The notifications page, in the browser.
pub fn open() {
    if let Some(account) = account() {
        let _ = std::process::Command::new("/usr/bin/open").arg(format!("https://{}/notifications", account.server)).spawn();
    }
}

/// Posts `text` as the signed-in account. Call only after the person allowed it.
pub async fn post(text: &str) -> Result<(), String> {
    let (account, secret) = (account().ok_or("Not signed in to Mastodon.")?, secret().ok_or("Not signed in to Mastodon.")?);
    let text = text.trim();
    if text.is_empty() || text.chars().count() > LONGEST_POST {
        return Err("A post is 1 to 500 characters.".into());
    }
    client()
        .post(format!("https://{}/api/v1/statuses", account.server))
        .bearer_auth(&secret.token)
        .header("Idempotency-Key", crate::chatgpt::random(16)?)
        .json(&json!({ "status": text }))
        .send()
        .await
        .map_err(|_| format!("Couldn’t reach {}.", account.server))?
        .error_for_status()
        .map_err(|_| "The server didn’t take the post.".to_string())?;
    Ok(())
}

#[tauri::command]
pub fn mastodon_account() -> Option<Account> {
    account()
}

#[tauri::command]
pub async fn mastodon_sign_in(server: String) -> Result<Account, String> {
    let account = sign_in(&server).await?;
    crate::island::refresh();
    Ok(account)
}

#[tauri::command]
pub async fn mastodon_sign_out() -> Result<(), String> {
    sign_out().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_read_from_what_people_type() {
        for typed in ["mastodon.social", "https://mastodon.social/", "Mastodon.Social/@ana", "@ana@mastodon.social", " https://fosstodon.org/home "] {
            assert!(server_name(typed).is_ok(), "{typed}");
        }
        assert_eq!(server_name("@ana@Hachyderm.io").unwrap(), "hachyderm.io");
        for typed in ["", "localhost", "127.0.0.1", "mastodon", "evil.com:@x", "a..b", "-.com/x y", "mastodon.social:8443"] {
            assert!(server_name(typed).is_err(), "{typed}: {:?}", server_name(typed));
        }
        assert!(server_name("not a server").is_err());
    }

    #[test]
    fn a_post_reads_as_plain_text() {
        assert_eq!(plain("<p>Sounds good, <span class=\"h-card\"><a href=\"x\">@<span>sam</span></a></span> &amp; see you at 3.</p><p>Bring the <br/>keys</p>"), "Sounds good, @sam & see you at 3. Bring the keys");
        assert_eq!(plain(""), "");
    }

    #[test]
    fn unread_mentions_count_and_show_the_newest() {
        let found = read_mentions(&json!([
            { "type": "mention", "account": { "display_name": "Ana", "acct": "ana@example.social" }, "status": { "content": "<p>Lunch at 1?</p>" } },
            { "type": "mention", "account": { "display_name": "", "acct": "sam" }, "status": { "content": "<p>hi</p>" } }
        ]));
        assert_eq!(found, Mentions { count: 2, newest: Some("Ana: Lunch at 1?".into()) });
        assert_eq!(read_mentions(&json!([])), Mentions::default());
        let no_name = read_mentions(&json!([{ "account": { "display_name": " ", "acct": "sam" }, "status": { "content": "" } }]));
        assert_eq!(no_name.newest.as_deref(), Some("sam"));
    }
}
