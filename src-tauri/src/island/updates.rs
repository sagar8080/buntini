//! Buntini's own updates.
//!
//! - **When it looks:** once a day after you agree, and whenever you ask from the menu bar.
//!   It looks for a newer published release of Buntini on GitHub.
//! - **What Install checks:**
//!   - the zip's size and SHA-256 against the release's `SHA256SUMS.txt`;
//!   - before anything is unpacked, every entry in the zip: only `Buntini.app/…`, no absolute
//!     or `..` paths, links only to places inside the app, and one entry per name;
//!   - the app inside: a strict signature check that codesign itself holds to Buntini's code
//!     requirement (Developer ID team `UU6JNBQ2T8`, bundle `app.buntini.companion`, notarized),
//!     the same bundle id and the release's version in its Info.plist, and Gatekeeper, which
//!     must be on.
//! - **Restart:** only after all of that, and when you press Restart, does the new copy take
//!   this one's place and open. It is checked again first, since it may have waited for hours.
//!   It waits in a cache folder only you can open.
//! - **What's new:** shows once, after the update.
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::i18n::{t, tf};
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
/// What the zip may unpack to, and how many entries it may hold.
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
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
            let checked = preference(&app, CHECKED_AT).and_then(|at| at.parse::<u64>().ok());
            if allowed(&app) && check_due(checked, wall_ms()) && matches!(stage(), Stage::Idle | Stage::Current) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { check(&app, false).await });
            }
            std::thread::sleep(Duration::from_secs(60 * 60));
        }
    });
}

/// Unix milliseconds by the wall clock, which, unlike `surface::now_ms`, carries across launches.
fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// A day since the last look. A time from the future (the clock moved back) or from before
/// this was wall-clock time counts as long ago.
pub(crate) fn check_due(checked: Option<u64>, now: u64) -> bool {
    match checked {
        Some(at) if at <= now => now - at >= DAY_MS,
        _ => true,
    }
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
    set_preference(app, CHECKED_AT, &wall_ms().to_string());
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
    let dir = updates_dir(app)?;
    private_dir(&dir).map_err(|_| t("updates.error.noRoom"))?;
    let folder = dir.join(&release.version);
    let _ = std::fs::remove_dir_all(&folder);
    private_dir(&folder).map_err(|_| t("updates.error.noRoom"))?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| t("updates.error.secureConnection"))?;

    let expected = sums(&String::from_utf8_lossy(&fetch(&client, &release.sums.url, 64 * 1024, |_| {}, None).await?)).ok_or_else(|| t("updates.error.noChecksum"))?;
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
        return Err(t("updates.error.mismatch"));
    }

    show(app, Stage::Checking(release.clone()));
    let version = release.version.clone();
    let unpacked = folder.join("app");
    tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        // The checksums come from the same release as the zip, so they prove only that the
        // download is whole: nothing in it is unpacked until every entry is known to stay
        // inside Buntini.app.
        check_archive(&zip)?;
        private_dir(&unpacked).map_err(|_| t("updates.error.noRoom"))?;
        run("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), unpacked.as_os_str()]).map_err(|_| t("updates.error.unzip"))?;
        let _ = std::fs::remove_file(&zip);
        let new_app = unpacked.join("Buntini.app");
        verify(&new_app, &version)?;
        Ok(new_app)
    })
    .await
    .map_err(|_| t("updates.error.checkStopped"))?
}

/// Downloads `url` (following GitHub's redirect to its file servers), at most `cap` bytes,
/// into `file` or memory. Answers the SHA-256 when saving to a file, else the bytes.
async fn fetch(client: &reqwest::Client, url: &str, cap: u64, mut progress: impl FnMut(u64), file: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut url = reqwest::Url::parse(url).map_err(|_| t("updates.error.badLink"))?;
    let mut response = None;
    for _ in 0..4 {
        if !github_host(&url) {
            return Err(t("updates.error.notGitHub"));
        }
        let answer = client.get(url.clone()).send().await.map_err(|_| t("updates.error.unreachable"))?;
        if answer.status().is_redirection() {
            let to = answer.headers().get("location").and_then(|value| value.to_str().ok()).ok_or_else(|| t("updates.error.brokenLink"))?;
            url = url.join(to).map_err(|_| t("updates.error.brokenLink"))?;
            continue;
        }
        if !answer.status().is_success() {
            return Err(tf("updates.error.http", &[("status", &answer.status().as_u16())]));
        }
        response = Some(answer);
        break;
    }
    let response = response.ok_or_else(|| t("updates.error.tooManyRedirects"))?;
    let mut out = match file {
        Some(path) => Some(std::fs::File::create(path).map_err(|_| t("updates.error.save"))?),
        None => None,
    };
    let mut hasher = Sha256::new();
    let mut bytes = Vec::new();
    let mut done = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| t("updates.error.interrupted"))?;
        done += chunk.len() as u64;
        if done > cap {
            return Err(t("updates.error.tooBig"));
        }
        match out.as_mut() {
            Some(out) => {
                out.write_all(&chunk).map_err(|_| t("updates.error.save"))?;
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

/// The code requirement the new copy must meet, which codesign itself enforces: Apple's
/// Developer ID chain, issued to Buntini's team, for Buntini's bundle id, and (for an update)
/// notarized by Apple.
fn requirement(notarized: bool) -> String {
    format!(
        "={}anchor apple generic and identifier \"{BUNDLE}\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{TEAM}\"",
        if notarized { "notarized and " } else { "" }
    )
}

/// The new copy is Buntini, from its developer and notarized, the version the release says,
/// and Gatekeeper lets it open.
fn verify(app: &Path, version: &str) -> Result<(), String> {
    let plist = app.join("Contents/Info.plist");
    if !plist.is_file() {
        return Err(t("updates.error.noApp"));
    }
    signed_by_buntini(app, true)?;
    if plist_value(&plist, "CFBundleIdentifier").as_deref() != Some(BUNDLE) {
        return Err(t("updates.error.otherDeveloper"));
    }
    if plist_value(&plist, "CFBundleShortVersionString").ok_or_else(|| t("updates.error.readVersion"))? != version {
        return Err(t("updates.error.otherVersion"));
    }
    gatekeeper(app)
}

/// codesign checks the whole bundle strictly and holds it to Buntini's requirement. Only its
/// exit status counts, never what it prints: the bundle's author chooses names that appear
/// there.
fn signed_by_buntini(app: &Path, notarized: bool) -> Result<(), String> {
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R"])
        .arg(requirement(notarized))
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| t("updates.error.readSignature"))?;
    match status.code() {
        Some(0) => Ok(()),
        // Soundly signed, but not by Buntini's developer, or not notarized.
        Some(3) => Err(t("updates.error.otherDeveloper")),
        _ => Err(t("updates.error.brokenSignature")),
    }
}

/// One value from an Info.plist.
fn plist_value(plist: &Path, key: &str) -> Option<String> {
    let print = format!("Print :{key}");
    let shown = run("/usr/libexec/PlistBuddy", &["-c".as_ref(), print.as_ref(), plist.as_os_str()]).ok()?;
    Some(String::from_utf8_lossy(&shown.stdout).trim().to_string())
}

/// Gatekeeper accepts the copy for itself: an `override=` means assessments are switched off
/// and anything would pass.
fn gatekeeper(app: &Path) -> Result<(), String> {
    let output = Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute", "-vv"])
        .arg(app)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| t("updates.error.gatekeeper"))?;
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    gatekeeper_verdict(output.status.success(), &said)
}

pub(crate) fn gatekeeper_verdict(accepted: bool, said: &str) -> Result<(), String> {
    if said.contains("override=") {
        return Err(t("updates.error.gatekeeperOff"));
    }
    if accepted { Ok(()) } else { Err(t("updates.error.gatekeeper")) }
}

/// Every entry of the downloaded zip, checked before `ditto` unpacks any of it. The checksums
/// come from the same release, so they can't vouch for what is inside.
/// - **Names:** inside `Buntini.app/`, with no absolute path, `..`, `.`, empty part, backslash
///   or control character, and each once, ignoring case as the Mac's disk does.
/// - **Kinds:** from a Unix system, as ditto writes them, and only folders, files and links,
///   with no setuid, setgid or sticky bit.
/// - **Links:** stored as they are, relative, leading inside the app, never climbing back out
///   of another link, and nothing unpacked through one.
/// - **Layout:** one index, closing the file, whose entries lie back to back from the start,
///   each named as its own header names it: an unzipper reading from the front finds the same
///   entries as one reading the index.
pub(crate) fn check_archive(path: &Path) -> Result<(), String> {
    check_entries(path).map_err(|_| t("updates.error.unsafeArchive"))
}

/// An entry as its local header and the index describe it.
struct Entry {
    parts: Vec<String>,
    link: Option<usize>,
    start: u64,
    end: u64,
    descriptor: bool,
    crc: u32,
}

const LOCAL: [u8; 4] = *b"PK\x03\x04";
const CENTRAL: [u8; 4] = *b"PK\x01\x02";
const DESCRIPTOR: [u8; 4] = *b"PK\x07\x08";
const END: [u8; 4] = *b"PK\x05\x06";

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_at(file: &mut std::fs::File, at: u64, into: &mut [u8]) -> Result<(), ()> {
    file.seek(SeekFrom::Start(at)).map_err(drop)?;
    file.read_exact(into).map_err(drop)
}

fn check_entries(path: &Path) -> Result<(), ()> {
    let mut file = std::fs::File::open(path).map_err(drop)?;
    let length = file.metadata().map_err(drop)?.len();
    // The end record closes the file, with no comment to hide another one in.
    let mut end = [0u8; 22];
    read_at(&mut file, length.checked_sub(22).ok_or(())?, &mut end)?;
    if end[..4] != END || le16(&end, 20) != 0 {
        return Err(());
    }
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path).map_err(drop)?)).map_err(drop)?;
    if archive.offset() != 0 || archive.is_empty() || archive.len() > MAX_ENTRIES {
        return Err(());
    }
    let mut entries = Vec::with_capacity(archive.len());
    let mut names = HashSet::new();
    let mut unpacked = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(drop)?;
        let name = std::str::from_utf8(entry.name_raw()).map_err(drop)?;
        let parts = entry_parts(name)?;
        // The index's own record, read as written: the zip crate drops setuid bits, and makes
        // up a mode for entries from other systems.
        let mut central = [0u8; 46];
        read_at(&mut file, entry.central_header_start(), &mut central)?;
        if central[..4] != CENTRAL || central[5] != 3 {
            return Err(());
        }
        let mode = le32(&central, 38) >> 16;
        let folder = name.ends_with('/');
        let fits = match mode & 0o170000 {
            0o040000 => folder,
            0o100000 | 0o120000 => !folder,
            _ => false,
        };
        if !fits || mode & 0o7000 != 0 || entry.encrypted() || !names.insert(parts.join("/").to_lowercase()) {
            return Err(());
        }
        unpacked = unpacked.checked_add(entry.size()).filter(|total| *total <= MAX_UNPACKED).ok_or(())?;
        // The entry's local header says what the index says: the same name, method and flags.
        let mut local = [0u8; 30];
        read_at(&mut file, entry.header_start(), &mut local)?;
        if local[..4] != LOCAL {
            return Err(());
        }
        let (flags, method) = (le16(&local, 6), le16(&local, 8));
        if flags & 0b1001 != le16(&central, 8) & 0b1001 || method != le16(&central, 10) || flags & 1 != 0 {
            return Err(());
        }
        let (name_length, extra_length) = (u64::from(le16(&local, 26)), u64::from(le16(&local, 28)));
        let mut local_name = vec![0u8; name_length as usize];
        read_at(&mut file, entry.header_start() + 30, &mut local_name)?;
        if local_name != entry.name_raw() {
            return Err(());
        }
        let link = (mode & 0o170000 == 0o120000).then_some(index);
        let start = entry.header_start();
        let end = start.checked_add(30 + name_length + extra_length).and_then(|data| data.checked_add(entry.compressed_size())).ok_or(())?;
        entries.push(Entry { parts, link, start, end, descriptor: flags & 0b1000 != 0, crc: entry.crc32() });
    }
    // Back to back from the first byte to the index, each followed only by its data descriptor.
    entries.sort_by_key(|entry| entry.start);
    let mut at = 0u64;
    for (position, entry) in entries.iter().enumerate() {
        if entry.start != at {
            return Err(());
        }
        let next = entries.get(position + 1).map_or(archive.central_directory_start(), |next| next.start);
        let gap = next.checked_sub(entry.end).ok_or(())?;
        if entry.descriptor {
            let mut descriptor = [0u8; 8];
            read_at(&mut file, entry.end, &mut descriptor)?;
            let signed = descriptor[..4] == DESCRIPTOR;
            let crc = le32(&descriptor, if signed { 4 } else { 0 });
            let sizes = gap.checked_sub(if signed { 8 } else { 4 }).ok_or(())?;
            if crc != entry.crc || !(sizes == 8 || sizes == 16) {
                return Err(());
            }
        } else if gap != 0 {
            return Err(());
        }
        at = next;
    }
    if at != archive.central_directory_start() {
        return Err(());
    }
    // Links lead inside the app, and nothing is unpacked through one.
    let links: HashSet<String> = entries.iter().filter(|entry| entry.link.is_some()).map(|entry| entry.parts.join("/").to_lowercase()).collect();
    for entry in &entries {
        if (1..entry.parts.len()).any(|depth| links.contains(&entry.parts[..depth].join("/").to_lowercase())) {
            return Err(());
        }
        let Some(index) = entry.link else { continue };
        let mut stored = archive.by_index(index).map_err(drop)?;
        if stored.compression() != zip::CompressionMethod::Stored || stored.size() > 1024 {
            return Err(());
        }
        let mut target = String::new();
        stored.read_to_string(&mut target).map_err(drop)?;
        if !link_inside(&entry.parts, &target, &links) {
            return Err(());
        }
    }
    Ok(())
}

/// An entry's path, split, when it lies inside `Buntini.app/`.
fn entry_parts(name: &str) -> Result<Vec<String>, ()> {
    let path = name.strip_suffix('/').unwrap_or(name);
    if name.starts_with('/') || name.chars().any(|c| c == '\\' || c.is_control()) {
        return Err(());
    }
    let parts: Vec<String> = path.split('/').map(str::to_string).collect();
    if parts[0] != "Buntini.app" || parts.iter().any(|part| matches!(part.as_str(), "" | "." | "..")) {
        return Err(());
    }
    Ok(parts)
}

/// Whether a link at `link` to `target` stays inside the app. Followed by name alone, which is
/// only sound until the path goes through another link: after that, `..` is refused.
pub(crate) fn link_inside(link: &[String], target: &str, links: &HashSet<String>) -> bool {
    if target.is_empty() || target.starts_with('/') || target.chars().any(|c| c == '\\' || c.is_control()) {
        return false;
    }
    let mut at: Vec<&str> = link[..link.len() - 1].iter().map(String::as_str).collect();
    let mut through = false;
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // Never above the app's own folder.
                if through || at.len() <= 1 {
                    return false;
                }
                at.pop();
            }
            part => {
                at.push(part);
                through |= links.contains(&at.join("/").to_lowercase());
            }
        }
    }
    at.first() == Some(&"Buntini.app")
}

/// A folder only this user can open, made if needed, and never a link.
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    if !std::fs::symlink_metadata(path)?.is_dir() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// This copy of Buntini, when it runs as an app it can replace.
fn this_app() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| t("updates.error.findSelf"))?;
    let app = exe.ancestors().nth(3).filter(|path| path.extension().is_some_and(|ext| ext == "app")).map(Path::to_path_buf).ok_or_else(|| t("updates.error.notApp"))?;
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Err(t("updates.error.translocated"));
    }
    let parent = app.parent().ok_or_else(|| t("updates.error.findSelf"))?;
    let probe = parent.join(".buntini-update");
    std::fs::write(&probe, b"").map_err(|_| tf("updates.error.cannotWrite", &[("folder", &parent.display())]))?;
    let _ = std::fs::remove_file(probe);
    Ok(app)
}

fn updates_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_cache_dir().map(|dir| dir.join("updates")).map_err(|_| t("updates.error.noCache"))
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
        return Err(t("updates.error.copyGone"));
    }
    let dir = updates_dir(app)?;
    private_dir(&dir).map_err(|_| t("updates.error.prepareRestart"))?;
    // The copy may have waited in the cache for hours: it is checked again just before it
    // takes this one's place.
    verify(new_app, &release.version)?;
    let keep = dir.join("Buntini-previous.app");
    let _ = std::fs::remove_dir_all(&keep);
    let script = dir.join("relaunch.sh");
    std::fs::write(&script, RELAUNCH).map_err(|_| t("updates.error.prepareRestart"))?;
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
        .map_err(|_| t("updates.error.startRestart"))?;
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
            // Checking again takes a few seconds, so not on the island's thread.
            show(app, Stage::Checking(release.clone()));
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(why) = restart(&app, &release, &new_app) {
                    show(&app, Stage::Failed(Some(release), why));
                }
            });
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
        let calculator = Path::new("/System/Applications/Calculator.app");
        let why = verify(calculator, "1.0").unwrap_err();
        assert!(why.contains("isn't signed by Buntini's developer"), "{why}");
        assert!(signed_by_buntini(calculator, false).is_err());
        assert!(verify(Path::new("/nowhere/Buntini.app"), "1.0").is_err());
    }

    /// The copy in Applications, signed by Buntini's team, meets the requirement codesign
    /// enforces. Notarization is asked of updates only, since a local build isn't notarized.
    #[test]
    fn accepts_the_app_buntinis_developer_signed() {
        let installed = Path::new("/Applications/Buntini.app");
        if !installed.join("Contents/Info.plist").is_file() {
            return;
        }
        signed_by_buntini(installed, false).expect("signed by team UU6JNBQ2T8");
        assert_eq!(plist_value(&installed.join("Contents/Info.plist"), "CFBundleIdentifier").as_deref(), Some(BUNDLE));
        assert!(requirement(true).starts_with("=notarized and anchor apple generic"));
    }

    /// A bundle's author names its executable, and codesign prints that name first, so a name
    /// carrying "TeamIdentifier=UU6JNBQ2T8" once looked like Buntini's team. The requirement
    /// doesn't read names.
    #[test]
    fn a_crafted_executable_name_does_not_pass_as_buntinis_team() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Buntini.app");
        let name = "x\nIdentifier=app.buntini.companion\nTeamIdentifier=UU6JNBQ2T8";
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS").join(name)).unwrap();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>{}</string><key>CFBundleIdentifier</key><string>{BUNDLE}</string><key>CFBundleShortVersionString</key><string>9.9.9</string></dict></plist>\n",
            name.replace('\n', "&#10;")
        );
        std::fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        let signed = Command::new("/usr/bin/codesign").args(["--force", "--sign", "-", "--identifier", BUNDLE]).arg(&app).output().unwrap();
        if !signed.status.success() {
            // This Mac can't sign ad hoc here; nothing to compare.
            return;
        }
        let printed = Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2"]).arg(&app).output().unwrap();
        let printed = String::from_utf8_lossy(&printed.stderr);
        assert!(printed.lines().any(|line| line == "TeamIdentifier=UU6JNBQ2T8"), "the old text check would have passed: {printed}");
        assert!(signed_by_buntini(&app, false).is_err());
        assert!(verify(&app, "9.9.9").is_err());
    }

    #[test]
    fn gatekeeper_switched_off_is_not_a_pass() {
        assert!(gatekeeper_verdict(true, "/x/Buntini.app: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: Sagar Das (UU6JNBQ2T8)\n").is_ok());
        let off = gatekeeper_verdict(true, "/x/Buntini.app: accepted\noverride=security disabled\n").unwrap_err();
        assert!(off.contains("Gatekeeper is turned off"), "{off}");
        assert!(gatekeeper_verdict(false, "/x/Buntini.app: rejected\nsource=Unnotarized Developer ID\n").is_err());
    }

    #[test]
    fn the_daily_check_counts_days_across_launches() {
        let now = wall_ms();
        assert!(now > 1_700_000_000_000, "wall-clock milliseconds");
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - 60_000), now));
        assert!(check_due(Some(now - DAY_MS), now));
        // What was saved as time since launch, and a time from the future, count as long ago.
        assert!(check_due(Some(5 * 60 * 1000), now));
        assert!(check_due(Some(now + DAY_MS), now));
    }

    #[test]
    fn the_staging_folders_are_only_this_users() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("updates/0.3.0/app");
        private_dir(&folder).unwrap();
        for path in [dir.path().join("updates"), dir.path().join("updates/0.3.0"), folder.clone()] {
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700, "{}", path.display());
        }
        // An existing folder is closed up too, and a link is refused.
        let open = dir.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&open).unwrap();
        assert_eq!(std::fs::metadata(&open).unwrap().permissions().mode() & 0o777, 0o700);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&open, &link).unwrap();
        assert!(private_dir(&link).is_err());
    }

    // MARK: The zip, before it is unpacked

    /// A zip written by hand, so the tests can make what ditto never would.
    struct Raw {
        name: &'static str,
        /// What the entry's own header calls it, when that differs from the index.
        local: Option<&'static str>,
        mode: u32,
        data: &'static [u8],
        /// In the file, but left out of the index.
        hidden: bool,
    }

    fn raw(name: &'static str, mode: u32, data: &'static [u8]) -> Raw {
        Raw { name, local: None, mode, data, hidden: false }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn zip_of(entries: &[Raw], before: &[u8], comment: &[u8]) -> Vec<u8> {
        let mut out = before.to_vec();
        let mut index = Vec::new();
        let mut count = 0u16;
        for entry in entries {
            let start = (out.len() - before.len()) as u32;
            let local = entry.local.unwrap_or(entry.name);
            let crc = crc32(entry.data);
            let size = entry.data.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&[0; 4]); // flags, stored
            out.extend_from_slice(&[0; 4]); // time, date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(local.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(local.as_bytes());
            out.extend_from_slice(entry.data);
            if entry.hidden {
                continue;
            }
            count += 1;
            index.extend_from_slice(b"PK\x01\x02");
            index.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes());
            index.extend_from_slice(&20u16.to_le_bytes());
            index.extend_from_slice(&[0; 8]); // flags, stored, time, date
            index.extend_from_slice(&crc.to_le_bytes());
            index.extend_from_slice(&size.to_le_bytes());
            index.extend_from_slice(&size.to_le_bytes());
            index.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            index.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
            index.extend_from_slice(&(entry.mode << 16).to_le_bytes());
            index.extend_from_slice(&start.to_le_bytes());
            index.extend_from_slice(entry.name.as_bytes());
        }
        let at = (out.len() - before.len()) as u32;
        out.extend_from_slice(&index);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(index.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    const DIR: u32 = 0o040755;
    const FILE: u32 = 0o100644;
    const LINK: u32 = 0o120755;

    fn bundle() -> Vec<Raw> {
        vec![
            raw("Buntini.app/", DIR, b""),
            raw("Buntini.app/Contents/", DIR, b""),
            raw("Buntini.app/Contents/Info.plist", FILE, b"<plist/>"),
            raw("Buntini.app/Contents/Frameworks/", DIR, b""),
            raw("Buntini.app/Contents/Frameworks/X.framework/", DIR, b""),
            raw("Buntini.app/Contents/Frameworks/X.framework/Versions/", DIR, b""),
            raw("Buntini.app/Contents/Frameworks/X.framework/Versions/A/", DIR, b""),
            raw("Buntini.app/Contents/Frameworks/X.framework/Versions/A/X", FILE, b"x"),
            raw("Buntini.app/Contents/Frameworks/X.framework/Versions/Current", LINK, b"A"),
            raw("Buntini.app/Contents/Frameworks/X.framework/X", LINK, b"Versions/Current/X"),
        ]
    }

    fn checked(entries: Vec<Raw>, before: &[u8], comment: &[u8]) -> Result<(), String> {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join(ZIP);
        std::fs::write(&zip, zip_of(&entries, before, comment)).unwrap();
        check_archive(&zip)
    }

    fn with(extra: Raw) -> Vec<Raw> {
        let mut entries = bundle();
        entries.push(extra);
        entries
    }

    #[test]
    fn an_app_with_links_inside_itself_may_be_unpacked() {
        checked(bundle(), b"", b"").expect("a bundle with framework links");
    }

    #[test]
    fn nothing_outside_the_app_is_ever_unpacked() {
        for name in ["../evil", "/tmp/evil", "Other.app/x", "Buntini.app/../evil", "Buntini.app/./x", "Buntini.app//x", "Buntini.app\\..\\evil", "Buntini.app/a\u{0}b", "buntini.app/x"] {
            let why = checked(with(raw(name, FILE, b"x")), b"", b"").unwrap_err();
            assert!(why.contains("something other than Buntini"), "{name:?}: {why}");
        }
    }

    #[test]
    fn links_must_stay_inside_the_app() {
        for target in [&b"/Library/LaunchAgents"[..], b"../../..", b"../../../../evil", b"../../Contents/../../evil", b"Frameworks/X.framework/Versions/Current/../../../../../evil", b""] {
            let mut entries = bundle();
            entries.push(raw("Buntini.app/Contents/escape", LINK, target));
            assert!(checked(entries, b"", b"").is_err(), "{:?}", String::from_utf8_lossy(target));
        }
        // Inside the app, even by `..`.
        assert!(checked(with(raw("Buntini.app/Contents/Resources", LINK, b"../Contents/Frameworks")), b"", b"").is_ok());
        // Nothing is unpacked through a link, wherever it leads.
        assert!(checked(with(raw("Buntini.app/Contents/Frameworks/X.framework/Versions/Current/evil", FILE, b"x")), b"", b"").is_err());
        assert!(checked(with(raw("Buntini.app/Contents/frameworks/x.framework/versions/current/evil", FILE, b"x")), b"", b"").is_err());
        let links = HashSet::from(["buntini.app/contents/a".to_string()]);
        let at = |path: &str| path.split('/').map(str::to_string).collect::<Vec<_>>();
        assert!(link_inside(&at("Buntini.app/Contents/b"), "a/x", &links));
        assert!(!link_inside(&at("Buntini.app/Contents/b"), "a/../x", &links), "after a link, `..` isn't followed by name");
        assert!(!link_inside(&at("Buntini.app"), "x", &links));
    }

    #[test]
    fn odd_entries_are_refused() {
        // setuid, a device, a second entry with the same name in another case.
        assert!(checked(with(raw("Buntini.app/Contents/MacOS", 0o104755, b"x")), b"", b"").is_err());
        assert!(checked(with(raw("Buntini.app/Contents/tty", 0o020644, b"")), b"", b"").is_err());
        assert!(checked(with(raw("Buntini.app/contents/info.plist", FILE, b"<plist/>")), b"", b"").is_err());
        // A folder without its slash, and a file with one.
        assert!(checked(with(raw("Buntini.app/Contents/Plain", DIR, b"")), b"", b"").is_err());
        assert!(checked(with(raw("Buntini.app/Contents/Odd/", FILE, b"")), b"", b"").is_err());
    }

    #[test]
    fn what_an_unzipper_reads_from_the_front_matches_the_index() {
        // A header naming another path than the index does.
        let mut renamed = bundle();
        renamed[2].local = Some("Buntini.app/../../evil.plist");
        assert!(checked(renamed, b"", b"").is_err());
        // An entry in the file that the index leaves out.
        let mut hidden = bundle();
        hidden.insert(3, Raw { name: "Buntini.app/../../Library/LaunchAgents/evil.plist", local: None, mode: FILE, data: b"x", hidden: true });
        assert!(checked(hidden, b"", b"").is_err());
        // Bytes before the first entry, and a comment after the end record.
        assert!(checked(bundle(), b"MZ prepended", b"").is_err());
        assert!(checked(bundle(), b"", b"PK\x05\x06").is_err());
        // Not a zip at all.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.zip"), b"hello").unwrap();
        assert!(check_archive(&dir.path().join("x.zip")).is_err());
    }

    /// What CI makes: `ditto -c -k --keepParent`, with data descriptors and the `._` files that
    /// carry extended attributes.
    #[test]
    fn a_zip_made_the_way_releases_are_made_may_be_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Buntini.app");
        let versions = app.join("Contents/Frameworks/X.framework/Versions");
        std::fs::create_dir_all(versions.join("A")).unwrap();
        std::fs::write(versions.join("A/X"), b"x".repeat(5000)).unwrap();
        std::os::unix::fs::symlink("A", versions.join("Current")).unwrap();
        std::os::unix::fs::symlink("Versions/Current/X", app.join("Contents/Frameworks/X.framework/X")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();
        let _ = Command::new("/usr/bin/xattr").args(["-w", "com.example.note", "kept"]).arg(app.join("Contents/Info.plist")).status();
        let zip = dir.path().join(ZIP);
        run("/usr/bin/ditto", &["-c".as_ref(), "-k".as_ref(), "--keepParent".as_ref(), app.as_os_str(), zip.as_os_str()]).expect("ditto zips");
        check_archive(&zip).expect("ditto's zip of an app");
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
            check_archive(&zip).expect("only Buntini.app inside");
            run("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), unpacked.as_os_str()]).expect("unzips");
            let checked = verify(&unpacked.join("Buntini.app"), &release.version);
            let _ = std::fs::remove_dir_all(&folder);
            checked.expect("signed by Buntini's developer and allowed by Gatekeeper");
            println!("verified");
        });
    }
}
