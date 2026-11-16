//! Buntini's own updates.
//!
//! - **When it looks:** once a day after you agree, and whenever you ask from the menu bar.
//!   It looks for a newer published release of Buntini on GitHub.
//! - **What Install checks:**
//!   - the zip's size and SHA-256 against the release's `SHA256SUMS.txt`;
//!   - the app inside: bundle `app.buntini.companion`, Developer ID team `UU6JNBQ2T8`, the
//!     release's version, a strict signature check, and Gatekeeper.
//! - **Restart:** only after all of that, and when you press Restart, does the new copy take
//!   this one's place and open.
//! - **What's new:** shows once, after the update.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::integrations::USER_AGENT;
use crate::surface::cards::utilities::{Update, WhatsNew};
use crate::surface::{self, Card, CardBody, Offer, Source, Urgency};

const LATEST: &str = "https://api.github.com/repos/sagar8080/buntini/releases/latest";
const DOWNLOADS: &str = "https://github.com/sagar8080/buntini/releases/download/";
const PAGES: &str = "https://github.com/sagar8080/buntini/releases/";
const ZIP: &str = "Buntini-macOS-arm64.zip";
const SUMS: &str = "SHA256SUMS.txt";
const BUNDLE: &str = "app.buntini.companion";
const TEAM: &str = "UU6JNBQ2T8";
const MAX_ZIP: u64 = 400 * 1024 * 1024;
const CARD: &str = "update";
/// You were asked once whether Buntini may look for updates.
const ASKED: &str = "updates.asked";
const CHECKED_AT: &str = "updates.checkedAt";
/// What's new in the version being installed, shown once it runs.
const WHATS_NEW: &str = "updates.whatsNew";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asset {
    url: String,
    size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Release {
    version: String,
    notes: Vec<String>,
    zip: Asset,
    sums: Asset,
    page: String,
}

#[derive(Debug, Clone)]
enum Stage {
    Idle,
    Asking,
    Current,
    Available(Release),
    Downloading(Release, f64),
    Checking(Release),
    Ready(Release, PathBuf),
    Failed(Option<Release>, String),
}

static STAGE: Mutex<Stage> = Mutex::new(Stage::Idle);

fn stage() -> Stage {
    STAGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Sets what the card says and shows it, or takes it away.
fn show(app: &AppHandle, next: Stage) {
    *STAGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = next.clone();
    let current = app.package_info().version.to_string();
    let body = |version: &str, notes: Vec<String>, stage: &str, progress: Option<f64>| Update { version: version.into(), notes, stage: Some(stage.into()), progress };
    let (update, urgency, ttl) = match next {
        Stage::Idle => return super::withdraw(CARD),
        Stage::Asking => (body(&current, vec![], "ask", None), Urgency::Info, None),
        Stage::Current => (body(&current, vec![], "current", None), Urgency::Alert, Some(5_000)),
        Stage::Available(release) => (body(&release.version, release.notes, "available", None), Urgency::Info, None),
        Stage::Downloading(release, done) => (body(&release.version, release.notes, "downloading", Some(done)), Urgency::Info, None),
        Stage::Checking(release) => (body(&release.version, release.notes, "checking", None), Urgency::Info, None),
        Stage::Ready(release, _) => (body(&release.version, release.notes, "ready", None), Urgency::Alert, None),
        Stage::Failed(release, why) => {
            let version = release.map_or(current, |release| release.version);
            (body(&version, vec![why], "failed", None), Urgency::Info, None)
        }
    };
    super::publish(vec![Offer {
        card: Card { id: CARD.into(), source: Source::Island, body: CardBody::Update(update) },
        urgency,
        at_ms: surface::now_ms(),
        ttl_ms: ttl,
    }]);
}

// MARK: Starting and checking

/// Shows What's new after an update, asks once whether to look for updates, and looks daily
/// once you agree.
pub fn start(app: AppHandle) {
    whats_new(&app);
    std::thread::spawn(move || {
        // After the greeting.
        std::thread::sleep(Duration::from_secs(90));
        if !allowed(&app) && preference(&app, ASKED).is_none() && matches!(stage(), Stage::Idle) {
            show(&app, Stage::Asking);
        }
        loop {
            let checked = preference(&app, CHECKED_AT).and_then(|at| at.parse::<u64>().ok()).unwrap_or(0);
            if allowed(&app) && surface::now_ms().saturating_sub(checked) >= DAY_MS && matches!(stage(), Stage::Idle | Stage::Current) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { check(&app, false).await });
            }
            std::thread::sleep(Duration::from_secs(60 * 60));
        }
    });
}

/// From the menu bar: look now, and say so when this is the newest.
pub fn check_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check(&app, true).await });
}

async fn check(app: &AppHandle, asked: bool) {
    if matches!(stage(), Stage::Downloading(..) | Stage::Checking(..) | Stage::Ready(..)) {
        return show(app, stage());
    }
    let latest = match latest().await {
        Ok(latest) => latest,
        Err(why) => {
            if asked {
                show(app, Stage::Failed(None, why));
            }
            return;
        }
    };
    set_preference(app, CHECKED_AT, &surface::now_ms().to_string());
    let current = app.package_info().version.to_string();
    match latest {
        Some(release) if newer(&release.version, &current) => show(app, Stage::Available(release)),
        _ if asked => show(app, Stage::Current),
        _ => {}
    }
}

async fn latest() -> Result<Option<Release>, String> {
    let client = crate::integrations::client()?;
    let response = client.get(LATEST).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28").send().await;
    let value = crate::integrations::json_response(response, "GitHub", 512 * 1024).await?;
    Ok(release(&value))
}

/// The release, if it is published, is Buntini's, and has the Mac zip and its checksums.
pub(crate) fn release(value: &Value) -> Option<Release> {
    if value["draft"].as_bool() != Some(false) || value["prerelease"].as_bool() != Some(false) {
        return None;
    }
    let version = value["tag_name"].as_str()?.trim().trim_start_matches('v').to_string();
    parse_version(&version)?;
    let asset = |name: &str| {
        let found = value["assets"].as_array()?.iter().find(|asset| asset["name"].as_str() == Some(name))?;
        let url = found["browser_download_url"].as_str()?;
        url.starts_with(DOWNLOADS).then(|| Asset { url: url.to_string(), size: found["size"].as_u64().unwrap_or(0) })
    };
    let zip = asset(ZIP).filter(|zip| zip.size > 0 && zip.size <= MAX_ZIP)?;
    let sums = asset(SUMS)?;
    let page = value["html_url"].as_str().filter(|url| url.starts_with(PAGES)).unwrap_or(PAGES).to_string();
    Some(Release { version, notes: notes(value["body"].as_str().unwrap_or_default()), zip, sums, page })
}

/// `major.minor.patch`; anything else (a pre-release) isn't offered.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub(crate) fn newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(latest), Some(current)) if latest > current)
}

/// Up to four points from the release notes' bullets, plain; else their first line.
pub(crate) fn notes(body: &str) -> Vec<String> {
    let plain = |line: &str| {
        let text = line.replace("**", "").replace('`', "");
        // The first sentence is the point; the rest is detail for the release page.
        let text = text.split(". ").next().unwrap_or_default().trim().trim_end_matches('.');
        let mut cut: String = text.chars().take(120).collect();
        if text.chars().count() > 120 {
            cut = format!("{}…", cut.trim_end());
        }
        cut
    };
    let bullets: Vec<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- ").or_else(|| line.trim().strip_prefix("* ")))
        .map(plain)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect();
    if !bullets.is_empty() {
        return bullets;
    }
    body.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')).map(|line| vec![plain(line)]).unwrap_or_default()
}

// MARK: Installing

async fn install(app: AppHandle, release: Release) {
    match prepare(&app, &release).await {
        Ok(new_app) => show(&app, Stage::Ready(release, new_app)),
        Err(why) => show(&app, Stage::Failed(Some(release), why)),
    }
}

async fn prepare(app: &AppHandle, release: &Release) -> Result<PathBuf, String> {
    this_app()?;
    let folder = updates_dir(app)?.join(&release.version);
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).map_err(|_| "Couldn't make room for the download.".to_string())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| "Cannot initialize a secure connection.".to_string())?;

    let expected = sums(&String::from_utf8_lossy(&fetch(&client, &release.sums.url, 64 * 1024, |_| {}, None).await?)).ok_or("The release has no checksum for the Mac app.")?;
    let zip = folder.join(ZIP);
    let mut last = 0.0;
    let digest = fetch(&client, &release.zip.url, release.zip.size, |done| {
        let done = done as f64 / release.zip.size as f64;
        if done - last >= 0.02 || done >= 1.0 {
            last = done;
            show(app, Stage::Downloading(release.clone(), done));
        }
    }, Some(&zip))
    .await?;
    let size = std::fs::metadata(&zip).map(|meta| meta.len()).unwrap_or(0);
    if size != release.zip.size || hex(&digest) != expected {
        return Err("The download didn't match the release. Nothing was changed.".into());
    }

    show(app, Stage::Checking(release.clone()));
    let version = release.version.clone();
    let unpacked = folder.join("app");
    tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        run("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), unpacked.as_os_str()]).map_err(|_| "The download couldn't be opened.".to_string())?;
        let _ = std::fs::remove_file(&zip);
        let new_app = unpacked.join("Buntini.app");
        verify(&new_app, &version)?;
        Ok(new_app)
    })
    .await
    .map_err(|_| "The check stopped.".to_string())?
}

/// Downloads `url` (following GitHub's redirect to its file servers), at most `cap` bytes,
/// into `file` or memory. Answers the SHA-256 when saving to a file, else the bytes.
async fn fetch(client: &reqwest::Client, url: &str, cap: u64, mut progress: impl FnMut(u64), file: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut url = reqwest::Url::parse(url).map_err(|_| "The release link is unusable.".to_string())?;
    let mut response = None;
    for _ in 0..4 {
        if !github_host(&url) {
            return Err("The release pointed somewhere other than GitHub.".into());
        }
        let answer = client.get(url.clone()).send().await.map_err(|_| "Couldn't reach GitHub. Check your internet connection.".to_string())?;
        if answer.status().is_redirection() {
            let to = answer.headers().get("location").and_then(|value| value.to_str().ok()).ok_or("GitHub gave a broken link.")?;
            url = url.join(to).map_err(|_| "GitHub gave a broken link.".to_string())?;
            continue;
        }
        if !answer.status().is_success() {
            return Err(format!("GitHub couldn't give the download (HTTP {}).", answer.status().as_u16()));
        }
        response = Some(answer);
        break;
    }
    let response = response.ok_or("GitHub redirected too many times.")?;
    let mut out = match file {
        Some(path) => Some(std::fs::File::create(path).map_err(|_| "Couldn't save the download.".to_string())?),
        None => None,
    };
    let mut hasher = Sha256::new();
    let mut bytes = Vec::new();
    let mut done = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "The download was interrupted.".to_string())?;
        done += chunk.len() as u64;
        if done > cap {
            return Err("The download was bigger than the release said.".into());
        }
        match out.as_mut() {
            Some(out) => {
                out.write_all(&chunk).map_err(|_| "Couldn't save the download.".to_string())?;
                hasher.update(&chunk);
            }
            None => bytes.extend_from_slice(&chunk),
        }
        progress(done);
    }
    Ok(if out.is_some() { hasher.finalize().to_vec() } else { bytes })
}

/// GitHub and its file servers, over plain HTTPS on the usual port.
pub(crate) fn github_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && (host == "github.com" || host == "api.github.com" || host.ends_with(".githubusercontent.com"))
}

/// The zip's line in `SHA256SUMS.txt`.
pub(crate) fn sums(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == ZIP && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<std::process::Output, ()> {
    let output = Command::new(program).args(args).output().map_err(|_| ())?;
    if output.status.success() { Ok(output) } else { Err(()) }
}

/// The new copy is Buntini, from its developer, the version the release says, and Gatekeeper
/// lets it open.
fn verify(app: &Path, version: &str) -> Result<(), String> {
    if !app.join("Contents/Info.plist").is_file() {
        return Err("The download has no Buntini app in it.".into());
    }
    let path = app.as_os_str();
    run("/usr/bin/codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), path]).map_err(|_| "The new copy's signature is broken. Nothing was changed.".to_string())?;
    let details = Command::new("/usr/bin/codesign").args(["-dv".as_ref(), "--verbose=2".as_ref(), path]).output().map_err(|_| "Couldn't read the new copy's signature.".to_string())?;
    let (identifier, team) = signed_by(&String::from_utf8_lossy(&details.stderr));
    if identifier.as_deref() != Some(BUNDLE) || team.as_deref() != Some(TEAM) {
        return Err("The new copy isn't signed by Buntini's developer. Nothing was changed.".into());
    }
    let plist = app.join("Contents/Info.plist");
    let shown = run("/usr/libexec/PlistBuddy", &["-c".as_ref(), "Print :CFBundleShortVersionString".as_ref(), plist.as_os_str()]).map_err(|_| "Couldn't read the new copy's version.".to_string())?;
    if String::from_utf8_lossy(&shown.stdout).trim() != version {
        return Err("The new copy is a different version than the release. Nothing was changed.".into());
    }
    run("/usr/sbin/spctl", &["--assess".as_ref(), "--type".as_ref(), "execute".as_ref(), path]).map_err(|_| "Gatekeeper wouldn't open the new copy. Nothing was changed.".to_string())?;
    Ok(())
}

/// `Identifier=` and `TeamIdentifier=` from `codesign -dv`.
pub(crate) fn signed_by(text: &str) -> (Option<String>, Option<String>) {
    let field = |name: &str| text.lines().find_map(|line| line.strip_prefix(name)).map(|value| value.trim().to_string());
    (field("Identifier="), field("TeamIdentifier="))
}

/// This copy of Buntini, when it runs as an app it can replace.
fn this_app() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "Couldn't find this copy of Buntini.".to_string())?;
    let app = exe.ancestors().nth(3).filter(|path| path.extension().is_some_and(|ext| ext == "app")).map(Path::to_path_buf).ok_or("Updates install in the Buntini app.")?;
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Err("Move Buntini to Applications first.".into());
    }
    let parent = app.parent().ok_or("Couldn't find this copy of Buntini.")?;
    let probe = parent.join(".buntini-update");
    std::fs::write(&probe, b"").map_err(|_| format!("Buntini can't write to {}.", parent.display()))?;
    let _ = std::fs::remove_file(probe);
    Ok(app)
}

fn updates_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_cache_dir().map(|dir| dir.join("updates")).map_err(|_| "Couldn't find the cache folder.".to_string())
}

/// Puts the new copy in this one's place once Buntini has quit, and opens it.
const RELAUNCH: &str = r#"#!/bin/sh
pid="$1"; old="$2"; new="$3"; keep="$4"
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
if mv "$old" "$keep"; then
  if mv "$new" "$old"; then rm -rf "$keep"; else mv "$keep" "$old"; fi
fi
open "$old"
"#;

fn restart(app: &AppHandle, release: &Release, new_app: &Path) -> Result<(), String> {
    let old = this_app()?;
    if !new_app.is_dir() {
        return Err("The new copy is gone. Try again.".into());
    }
    let dir = updates_dir(app)?;
    let keep = dir.join("Buntini-previous.app");
    let _ = std::fs::remove_dir_all(&keep);
    let script = dir.join("relaunch.sh");
    std::fs::write(&script, RELAUNCH).map_err(|_| "Couldn't prepare the restart.".to_string())?;
    let notes = serde_json::json!({ "version": release.version, "lines": release.notes });
    set_preference(app, WHATS_NEW, &notes.to_string());
    use std::os::unix::process::CommandExt;
    Command::new("/bin/sh")
        .arg(&script)
        .arg(std::process::id().to_string())
        .arg(&old)
        .arg(new_app)
        .arg(&keep)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| "Couldn't start the restart.".to_string())?;
    app.exit(0);
    Ok(())
}

/// After an update: what's new in this version, once.
fn whats_new(app: &AppHandle) {
    let Some(saved) = preference(app, WHATS_NEW) else { return };
    set_preference(app, WHATS_NEW, "");
    let Ok(saved) = serde_json::from_str::<Value>(&saved) else { return };
    let version = saved["version"].as_str().unwrap_or_default();
    if version != app.package_info().version.to_string() {
        return;
    }
    let lines: Vec<String> = saved["lines"].as_array().into_iter().flatten().filter_map(|line| line.as_str().map(str::to_string)).collect();
    super::publish(vec![Offer {
        card: Card { id: "whatsNew".into(), source: Source::Island, body: CardBody::WhatsNew(WhatsNew { version: version.into(), lines }) },
        urgency: Urgency::Alert,
        at_ms: surface::now_ms(),
        ttl_ms: Some(15_000),
    }]);
}

// MARK: The card's buttons

pub(super) fn act(app: &AppHandle, name: &str) -> Result<(), String> {
    match (name, stage()) {
        ("allow", _) => {
            set_preference(app, ASKED, "1");
            set_allowed(app, true)?;
            show(app, Stage::Idle);
            check_now_quietly(app);
        }
        ("notNow", _) => {
            set_preference(app, ASKED, "1");
            show(app, Stage::Idle);
        }
        ("later", _) => show(app, Stage::Idle),
        ("install", Stage::Available(release) | Stage::Failed(Some(release), _)) => {
            show(app, Stage::Downloading(release.clone(), 0.0));
            tauri::async_runtime::spawn(install(app.clone(), release));
        }
        ("restart", Stage::Ready(release, new_app)) => {
            if let Err(why) = restart(app, &release, &new_app) {
                show(app, Stage::Failed(Some(release), why));
            }
        }
        ("open", Stage::Available(release) | Stage::Failed(Some(release), _) | Stage::Ready(release, _)) => {
            let _ = Command::new("/usr/bin/open").arg(&release.page).spawn();
        }
        ("open", _) => {
            let _ = Command::new("/usr/bin/open").arg(PAGES).spawn();
        }
        _ => {}
    }
    Ok(())
}

fn check_now_quietly(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check(&app, false).await });
}

// MARK: Settings and preferences

fn allowed(app: &AppHandle) -> bool {
    super::settings(app).is_some_and(|settings| settings.island.updates)
}

fn set_allowed(app: &AppHandle, on: bool) -> Result<(), String> {
    let state = app.try_state::<crate::controller::AppState>().ok_or("Settings aren't ready.")?;
    let store = state.store.lock().map_err(|_| "Settings are busy.")?;
    let mut settings = store.settings()?;
    settings.island.updates = on;
    store.save_settings(&settings)?;
    drop(store);
    let _ = app.emit("settings-changed", ());
    Ok(())
}

fn preference(app: &AppHandle, key: &str) -> Option<String> {
    let state = app.try_state::<crate::controller::AppState>()?;
    let store = state.store.lock().ok()?;
    store.preference(key).ok().flatten().filter(|value| !value.is_empty())
}

fn set_preference(app: &AppHandle, key: &str, value: &str) {
    if let Some(state) = app.try_state::<crate::controller::AppState>() {
        if let Ok(store) = state.store.lock() {
            let _ = store.set_preference(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn published() -> Value {
        json!({
            "tag_name": "v0.3.0", "draft": false, "prerelease": false,
            "html_url": "https://github.com/sagar8080/buntini/releases/tag/v0.3.0",
            "body": "Buntini 0.3.0 adds the island.\r\n\r\n## New\r\n\r\n- **The island** at the notch\r\n- Dictation `anywhere`\r\n",
            "assets": [
                { "name": "Buntini-macOS-arm64.zip", "size": 74760869, "browser_download_url": "https://github.com/sagar8080/buntini/releases/download/v0.3.0/Buntini-macOS-arm64.zip" },
                { "name": "SHA256SUMS.txt", "size": 262, "browser_download_url": "https://github.com/sagar8080/buntini/releases/download/v0.3.0/SHA256SUMS.txt" }
            ]
        })
    }

    #[test]
    fn reads_a_published_release() {
        let release = release(&published()).expect("a release");
        assert_eq!(release.version, "0.3.0");
        assert_eq!(release.notes, ["The island at the notch", "Dictation anywhere"]);
        assert_eq!(release.zip.size, 74760869);
        assert!(release.page.ends_with("/tag/v0.3.0"));
    }

    #[test]
    fn offers_only_published_releases_with_the_mac_zip_from_this_repository() {
        let mut draft = published();
        draft["draft"] = json!(true);
        assert!(release(&draft).is_none());
        let mut beta = published();
        beta["tag_name"] = json!("v0.3.0-beta.1");
        assert!(release(&beta).is_none());
        let mut elsewhere = published();
        elsewhere["assets"][0]["browser_download_url"] = json!("https://example.com/Buntini-macOS-arm64.zip");
        assert!(release(&elsewhere).is_none());
        let mut huge = published();
        huge["assets"][0]["size"] = json!(MAX_ZIP + 1);
        assert!(release(&huge).is_none());
        let mut no_sums = published();
        no_sums["assets"].as_array_mut().unwrap().pop();
        assert!(release(&no_sums).is_none());
    }

    #[test]
    fn compares_versions_by_number() {
        assert!(newer("0.3.0", "0.2.1"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.2.1", "0.2.1"));
        assert!(!newer("0.2.0", "0.2.1"));
        assert!(!newer("1.0", "0.2.1"));
    }

    #[test]
    fn notes_fall_back_to_the_first_sentence() {
        assert_eq!(notes("## Title\n\nFaster answers. And more."), ["Faster answers"]);
        assert!(notes("").is_empty());
        assert!(notes(&format!("- {}", "a".repeat(200)))[0].ends_with('…'));
    }

    #[test]
    fn finds_the_zip_checksum() {
        let text = "40a9b0e206840efebaa0a13e433c60e7bbfc0f946c4d81f7120fc85b4d71cad8  Buntini-macOS-arm64.zip\n7ab9b794d3b6f174325ac2ab5ed494ef8ade636b9a32013d856c170b59c0ee6b  Buntini-macOS-arm64.dmg\n";
        assert_eq!(sums(text).as_deref(), Some("40a9b0e206840efebaa0a13e433c60e7bbfc0f946c4d81f7120fc85b4d71cad8"));
        assert!(sums("abc  Buntini-macOS-arm64.zip").is_none());
    }

    #[test]
    fn downloads_only_from_github() {
        let ok = |url: &str| github_host(&reqwest::Url::parse(url).unwrap());
        assert!(ok("https://github.com/sagar8080/buntini/releases/download/v0.3.0/x.zip"));
        assert!(ok("https://release-assets.githubusercontent.com/github-production-release-asset/1"));
        assert!(!ok("http://github.com/x"));
        assert!(!ok("https://github.com.example.com/x"));
        assert!(!ok("https://user@github.com/x"));
        assert!(!ok("https://github.com:8443/x"));
    }

    #[test]
    fn notes_keep_each_point_short() {
        assert_eq!(notes("- **Events from a note** take the note's date. Before, they didn't.\n"), ["Events from a note take the note's date"]);
    }

    #[test]
    fn refuses_an_app_from_another_developer() {
        let why = verify(Path::new("/System/Applications/Calculator.app"), "1.0").unwrap_err();
        assert!(why.contains("isn't signed by Buntini's developer"), "{why}");
        assert!(verify(Path::new("/nowhere/Buntini.app"), "1.0").is_err());
    }

    #[test]
    fn reads_who_signed_an_app() {
        let text = "Executable=/Applications/Buntini.app/Contents/MacOS/Buntini\nIdentifier=app.buntini.companion\nFormat=app bundle with Mach-O thin (arm64)\nTeamIdentifier=UU6JNBQ2T8\n";
        assert_eq!(signed_by(text), (Some(BUNDLE.into()), Some(TEAM.into())));
    }

    /// The real latest release, downloaded and checked as Install does, without replacing
    /// anything: `cargo test --lib island::updates::tests::live_release -- --ignored --nocapture`.
    #[test]
    #[ignore = "downloads the latest release from GitHub"]
    fn live_release() {
        tauri::async_runtime::block_on(async {
            let release = latest().await.expect("GitHub answers").expect("a release");
            println!("latest {} · {} bytes · {:?}", release.version, release.zip.size, release.notes);
            let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).https_only(true).user_agent(USER_AGENT).build().unwrap();
            let expected = sums(&String::from_utf8_lossy(&fetch(&client, &release.sums.url, 64 * 1024, |_| {}, None).await.unwrap())).expect("a checksum");
            let folder = std::env::temp_dir().join(format!("buntini-update-check-{}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            let zip = folder.join(ZIP);
            let digest = fetch(&client, &release.zip.url, release.zip.size, |_| {}, Some(&zip)).await.unwrap();
            assert_eq!(hex(&digest), expected, "checksum");
            let unpacked = folder.join("app");
            run("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), unpacked.as_os_str()]).expect("unzips");
            let checked = verify(&unpacked.join("Buntini.app"), &release.version);
            let _ = std::fs::remove_dir_all(&folder);
            checked.expect("signed by Buntini's developer and allowed by Gatekeeper");
            println!("verified");
        });
    }
}
