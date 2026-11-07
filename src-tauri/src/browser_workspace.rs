//! A visible, separately profiled browser for approved HTTPS navigation and reads.
//! No model-supplied script, clicks, form submission, purchases or sending. Remote
//! pages have no Tauri capability; extracted text is untrusted source material.
//!
//! macOS 14+ is required for persistent WKWebsiteDataStore identifiers and proxy
//! configuration. Native symbols are resolved at runtime to preserve the rest of
//! Buntini's macOS 13 support. Before any remote load, verify the exact data-store
//! identifier, required proxy (with direct failover disabled), and disabled peer
//! connections/media capture. Failure at any boundary leaves a blank view.
use crate::actions::{ActionOutcome, ActionReceipt, Field};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Semaphore},
};

pub const TOOLS: [&str; 1] = ["browse_page"];
const TEXT_LIMIT: usize = 6000;
const RESPONSE_LIMIT: usize = 192_000;
const LINK_LIMIT: usize = 32;
const HEADER_LIMIT: usize = 8192;
static APP: OnceLock<AppHandle> = OnceLock::new();
static WORKSPACE: tokio::sync::Mutex<Option<Workspace>> = tokio::sync::Mutex::const_new(None);
static DNS_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static PRICE_READS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

pub fn validate(tool: &str, raw: &Value) -> Result<Map<String, Value>, String> {
    if tool != "browse_page" {
        return Err("Unknown browser workspace action.".into());
    }
    let args = raw
        .as_object()
        .ok_or("Browser arguments must be an object.")?;
    if args.len() != 1 || !args.contains_key("url") {
        return Err("browse_page accepts only the approved HTTPS url.".into());
    }
    let raw = args["url"].as_str().ok_or("The page URL must be text.")?;
    crate::research::validate_public_url(raw)?;
    let url = reqwest::Url::parse(raw).map_err(|_| "The page URL is invalid.")?;
    Ok(json!({"url":url.to_string()}).as_object().unwrap().clone())
}

pub fn describe(_: &str, args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    ("Open and read this page in Buntini’s browser?".into(), vec![Field { key: "url".into(), label: "Page".into(), value: args["url"].as_str().unwrap_or_default().into(), kind: "text" }],
        "Opens a separate browser profile for this site and reads its visible text. Sign-ins persist in that profile; you may sign in manually. Navigation to another origin, popups and downloads are blocked. Buntini cannot submit forms, book, buy or send through this browser.".into())
}

fn failed(outcome: ActionOutcome, detail: impl Into<String>) -> ActionReceipt {
    let detail = detail.into();
    ActionReceipt {
        outcome,
        tool: "browse_page".into(),
        object_id: None,
        destination: Some("Buntini browser workspace".into()),
        message: detail.clone(),
        evidence: json!({}),
        error: Some(detail),
    }
}

fn origin(url: &reqwest::Url) -> String {
    url.origin().ascii_serialization()
}
fn navigation_allowed(url: &reqwest::Url, approved_origin: &str) -> bool {
    url.as_str() == "about:blank"
        || (crate::research::validate_public_url(url.as_str()).is_ok()
            && origin(url) == approved_origin)
}

fn public_addresses(addresses: &[SocketAddr]) -> Result<(), String> {
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses
            .iter()
            .any(|a| a.port() != 443 || !crate::research::is_public_ip(a.ip()))
    {
        Err("This host does not resolve exclusively to public HTTPS addresses.".into())
    } else {
        Ok(())
    }
}

async fn resolve_public(host: String) -> Result<Vec<SocketAddr>, String> {
    let permits = DNS_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(4)))
        .clone();
    let permit = permits
        .try_acquire_owned()
        .map_err(|_| "Browser DNS is busy.")?;
    // System DNS may outlive a timeout. Keep its permit inside the blocking job
    // so repeated malicious hosts cannot accumulate unlimited resolver jobs.
    let lookup = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        let _permit = permit;
        (host.as_str(), 443)
            .to_socket_addrs()
            .map(|addresses| addresses.take(33).collect::<Vec<_>>())
            .map_err(|_| "DNS lookup failed.".to_string())
    });
    let addresses = tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .map_err(|_| "DNS lookup timed out.")?
        .map_err(|_| "DNS lookup ended unexpectedly.")??;
    public_addresses(&addresses)?;
    Ok(addresses)
}

/// Parse only HTTP CONNECT authority form. Plain HTTP, alternate ports, URL
/// credentials, ambiguous targets and incomplete headers never reach a socket.
fn connect_host(header: &[u8]) -> Result<String, String> {
    if header.len() > HEADER_LIMIT || !header.ends_with(b"\r\n\r\n") {
        return Err("Invalid proxy header.".into());
    }
    let header = std::str::from_utf8(header).map_err(|_| "Invalid proxy header text.")?;
    let line = header
        .split("\r\n")
        .next()
        .ok_or("Missing proxy request.")?;
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 3 || parts[0] != "CONNECT" || !["HTTP/1.1", "HTTP/1.0"].contains(&parts[2]) {
        return Err("Only HTTPS CONNECT is allowed.".into());
    }
    let target = parts[1];
    if target.contains(['/', '?', '#', '@', '\\']) || !target.ends_with(":443") {
        return Err("Only public HTTPS on port 443 is allowed.".into());
    }
    let url = crate::research::validate_public_url(&format!("https://{target}/"))?;
    Ok(url
        .host_str()
        .ok_or("Missing proxy host.")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .into())
}

async fn read_header(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Result<Vec<u8>, String> {
    let mut header = Vec::new();
    loop {
        if header.len() >= HEADER_LIMIT {
            return Err("Proxy headers are too large.".into());
        }
        let byte = stream
            .read_u8()
            .await
            .map_err(|_| "Incomplete proxy request.")?;
        header.push(byte);
        if header.ends_with(b"\r\n\r\n") {
            return Ok(header);
        }
    }
}

async fn tunnel(mut incoming: TcpStream, stopped: Arc<AtomicBool>) {
    let established: Result<TcpStream, String> = async {
        let header = tokio::time::timeout(Duration::from_secs(5), read_header(&mut incoming))
            .await
            .map_err(|_| "Proxy handshake timed out.")??;
        let host = connect_host(&header)?;
        let addresses = resolve_public(host).await?;
        if stopped.load(Ordering::Acquire) {
            return Err("Browser workspace closed.".into());
        }
        // Connect to the exact validated IP rather than resolving the host again.
        // Every connection repeats validation, so DNS rebinding cannot bypass it.
        let mut connected = None;
        for address in addresses.iter().take(8) {
            if stopped.load(Ordering::Acquire) {
                return Err("Browser workspace closed.".into());
            }
            if let Ok(Ok(stream)) =
                tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(address)).await
            {
                let peer = stream
                    .peer_addr()
                    .map_err(|_| "Peer address unavailable.")?;
                if peer != *address || !crate::research::is_public_ip(peer.ip()) {
                    return Err("Unverified proxy peer.".into());
                }
                connected = Some(stream);
                break;
            }
        }
        connected.ok_or_else(|| "Public HTTPS connection failed.".into())
    }
    .await;
    let Ok(mut outgoing) = established else {
        let _ = incoming
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };
    if stopped.load(Ordering::Acquire) {
        return;
    }
    if incoming
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    let stop = async {
        while !stopped.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        _ = stop => {},
        _ = tokio::time::timeout(Duration::from_secs(120), tokio::io::copy_bidirectional(&mut incoming, &mut outgoing)) => {},
    }
}

struct Proxy {
    port: u16,
    stopped: Arc<AtomicBool>,
    listener: tokio::task::JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.listener.abort();
    }
}
async fn proxy() -> Result<Proxy, String> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "The browser’s private network gate could not start.")?;
    let port = listener
        .local_addr()
        .map_err(|_| "The browser network gate has no address.")?
        .port();
    let stopped = Arc::new(AtomicBool::new(false));
    let running = stopped.clone();
    let listener = tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(32));
        loop {
            if running.load(Ordering::Acquire) {
                break;
            }
            let Ok((stream, peer)) = listener.accept().await else {
                break;
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                continue;
            };
            let stopped = running.clone();
            tokio::spawn(async move {
                let _permit = permit;
                tunnel(stream, stopped).await;
            });
        }
        running.store(true, Ordering::Release);
    });
    Ok(Proxy {
        port,
        stopped,
        listener,
    })
}

/// Each approved origin has its own persistent profile, shared with neither
/// Safari/Chrome nor Buntini's trusted UI. Never silently substitute the default.
fn profile_id(directory: &Path, approved_origin: &str) -> Result<[u8; 16], String> {
    use std::io::Write;
    let directory = directory.join("browser-profiles");
    if directory
        .symlink_metadata()
        .is_ok_and(|metadata| !metadata.file_type().is_dir())
    {
        return Err("The browser profile directory must be a regular local directory.".into());
    }
    std::fs::create_dir_all(&directory)
        .map_err(|_| "Could not create the browser profile directory.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not protect the browser profile directory.")?;
    }
    let digest = Sha256::digest(approved_origin.as_bytes());
    let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = directory.join(name);
    if path.exists() {
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| "Could not inspect browser profile.")?;
        if !metadata.file_type().is_file() || metadata.len() != 16 {
            return Err(
                "The browser profile identifier must be a regular 16-byte local file.".into(),
            );
        }
        let bytes =
            std::fs::read(&path).map_err(|_| "Could not read the browser profile identifier.")?;
        let id: [u8; 16] = bytes
            .try_into()
            .map_err(|_| "The browser profile identifier is damaged; it was not replaced.")?;
        if id == [0; 16] {
            return Err("The browser profile identifier is invalid.".into());
        }
        return Ok(id);
    }
    let id = *uuid::Uuid::new_v4().as_bytes();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "Could not reserve the browser profile identifier.")?;
    file.write_all(&id)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save the browser profile identifier.")?;
    Ok(id)
}

fn verify_store_identity(
    expected: Option<[u8; 16]>,
    persistent: bool,
    identifier: Option<&str>,
) -> Result<(), String> {
    match expected {
        Some(profile)
            if persistent
                && identifier.is_some_and(|value| {
                    value.eq_ignore_ascii_case(&uuid::Uuid::from_bytes(profile).to_string())
                }) => Ok(()),
        Some(_) => Err("The browser profile did not match its isolated persistent store.".into()),
        None if !persistent && identifier.is_none() => Ok(()),
        None => Err("Public research requires a fresh nonpersistent browser store; remote navigation was refused.".into()),
    }
}

fn saved_profiles(directory: &Path) -> Result<Vec<(PathBuf, [u8; 16])>, String> {
    let directory = directory.join("browser-profiles");
    let metadata = match directory.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Could not inspect the browser profile directory.".into()),
    };
    if !metadata.file_type().is_dir() {
        return Err("The browser profile directory is not a regular local directory.".into());
    }
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|_| "Could not list browser profiles.")? {
        let entry = entry.map_err(|_| "Could not inspect a browser profile.")?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_str().ok_or("Invalid browser profile filename.")?;
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| "Could not inspect a browser profile.")?;
        if name.len() != 64
            || !name.bytes().all(|b| b.is_ascii_hexdigit())
            || !metadata.file_type().is_file()
            || metadata.len() != 16
        {
            return Err("A browser profile identifier is damaged. Browser data deletion could not be confirmed.".into());
        }
        let id: [u8; 16] = std::fs::read(&path)
            .map_err(|_| "Could not read a browser profile identifier.")?
            .try_into()
            .map_err(|_| "Invalid browser profile identifier.")?;
        if id == [0; 16] {
            return Err("A browser profile identifier is invalid.".into());
        }
        profiles.push((path, id));
    }
    Ok(profiles)
}

/// Called by Delete everything before its database is erased. The native store
/// owns cookies/cache; removing only our identifier files would leave that data
/// behind and make it impossible to address. Retain each ID until WebKit confirms
/// removal, including on timeout, so a later attempt can finish the deletion.
pub async fn clear_profiles() -> Result<(), String> {
    let Some(app) = APP.get() else {
        return Err("The browser workspace is not initialized.".into());
    };
    let mut slot = tokio::time::timeout(Duration::from_secs(2), WORKSPACE.lock())
        .await
        .map_err(|_| "Stop the browser operation before deleting its data.")?;
    if let Some(workspace) = slot.take() {
        let was_closed = workspace.proxy.stopped.swap(true, Ordering::AcqRel);
        let (send, receive) = oneshot::channel();
        let sender = Mutex::new(Some(send));
        workspace.window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(sender) = sender.lock().unwrap().take() {
                    let _ = sender.send(());
                }
            }
        });
        let _ = workspace.window.destroy();
        drop(workspace);
        if !was_closed {
            tokio::time::timeout(Duration::from_secs(5), receive)
                .await
                .map_err(|_| "The browser did not close; its stored data was not erased.")?
                .map_err(|_| "The browser closed without confirmation.")?;
        }
    }
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| "The browser profile directory is unavailable.")?;
    let profiles = saved_profiles(&directory)?;
    #[cfg(target_os = "macos")]
    for (path, id) in profiles {
        // WKWebView destruction can finish on a later main-run-loop turn. The
        // operation is idempotent deletion of this exact profile; retry only an
        // explicit native error, never its timeout/unknown completion.
        native::remove_profile(app, id).await?;
        std::fs::remove_file(path).map_err(|_| {
            "Browser cookies were removed, but its profile identifier could not be deleted."
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    if !profiles.is_empty() {
        return Err("Browser profiles require macOS to remove their stored website data.".into());
    }
    Ok(())
}

struct Workspace {
    window: WebviewWindow,
    proxy: Proxy,
    profile: Option<[u8; 16]>,
    approved_origin: String,
}
impl Drop for Workspace {
    fn drop(&mut self) {
        self.proxy.stopped.store(true, Ordering::Release);
        // The companion intercepts CloseRequested to hide its own window.
        // Destroy this isolated view directly so a research store cannot linger
        // hidden in memory after its operation has completed.
        let _ = self.window.destroy();
    }
}

// What a shop page shows about its price. Fixed like the script below: it reads, and nothing
// clicks or writes. No inputs or editable fields are inspected.
const READ_PRICE: &str = r#"(() => {
  try {
    if (document.readyState === 'loading' || !document.body) return {ready:false};
    const PRICE = /^(?:[A-Z]{0,3}\s?[$€£¥₹]\s?\d[\d.,]*|[A-Z]{3}\s\d[\d.,]*|\d[\d.,]*\s?(?:[$€£¥₹]|[A-Z]{3}))$/;
    const NOISE = 'nav,footer,aside,[class*="recommend" i],[class*="carousel" i],[class*="related" i],[class*="similar" i],[class*="upsell" i],[class*="cross-sell" i],[class*="crosssell" i],[class*="sponsor" i],[id*="recommend" i],[class*="financ" i],[class*="installment" i],[class*="monthly" i],[class*="footer" i],[class*="compare" i],[class*="bundle" i],[class*="minicart" i],[class*="mini-cart" i]';
    const clean = text => (text || '').replace(/\s+/g, ' ').trim();
    const reading = element => {
      // "$3,199" and a raised "95" read as one price.
      const sup = element.querySelector('sup');
      if (sup && /^\d{2}$/.test(clean(sup.textContent)) && !clean(element.textContent).includes('.')) {
        const whole = clean(element.textContent);
        return whole.slice(0, whole.length - 2) + '.' + whole.slice(-2);
      }
      return clean(element.textContent).replace(/\s(?=\d)/g, '');
    };
    const candidates = [];
    const elements = document.body.querySelectorAll('li,span,div,p,strong,b,bdi,ins,h1,h2,h3,h4,h5,h6,dd,td,a,data,em,label');
    let seen = 0;
    for (const element of elements) {
      if (++seen > 40000 || candidates.length >= 80) break;
      if (element.closest('input,textarea,select,[contenteditable],script,style,noscript,svg')) continue;
      const raw = element.textContent || '';
      if (raw.length > 40 || !/\d/.test(raw)) continue;
      const text = reading(element);
      if (!PRICE.test(text)) continue;
      if ([...element.children].some(child => PRICE.test(reading(child)))) continue;
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect();
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0 || box.width < 2 || box.height < 2) continue;
      let struck = !!element.closest('s,del,strike');
      for (let up = element, level = 0; up && level < 4 && !struck; up = up.parentElement, level++) {
        struck = getComputedStyle(up).textDecorationLine.includes('line-through');
      }
      const parent = element.parentElement;
      const around = clean(parent && parent.innerText);
      const hint = /price/i.test(`${element.className} ${element.id} ${element.getAttribute('itemprop') || ''} ${element.getAttribute('data-testid') || ''} ${parent ? parent.className : ''} ${parent ? parent.getAttribute('data-testid') || '' : ''}`);
      candidates.push({text, size: parseFloat(style.fontSize) || 0, top: Math.round(box.top + scrollY), struck, noise: !!element.closest(NOISE), hint, near: around.length <= 60 ? around : ''});
    }
    const heading = document.querySelector('h1');
    const headingBox = heading ? heading.getBoundingClientRect() : null;
    const scripts = [];
    let total = 0;
    for (const script of document.querySelectorAll('script[type="application/ld+json"]')) {
      const text = script.textContent || '';
      if (scripts.length >= 32 || total + text.length > 120000) break;
      scripts.push(text); total += text.length;
    }
    const meta = name => { const tag = document.querySelector(`meta[property="${name}"],meta[name="${name}"],meta[itemprop="${name}"]`); return tag ? clean(tag.getAttribute('content')) : null; };
    const buttons = [...document.querySelectorAll('button,a[role="button"],[class*="add-to-cart" i],[class*="addtocart" i]')].slice(0, 400).filter(el => { const box = el.getBoundingClientRect(); return box.width > 1 && box.height > 1; }).map(el => clean(el.textContent).slice(0, 60));
    return {
      ready: true, url: location.href, title: clean(document.title).slice(0, 300),
      heading: heading ? clean(heading.textContent).slice(0, 300) : '', headingTop: headingBox ? Math.round(headingBox.top + scrollY) : null,
      jsonLd: scripts, metaAmount: meta('product:price:amount') || meta('og:price:amount') || meta('price'), metaCurrency: meta('product:price:currency') || meta('og:price:currency') || meta('priceCurrency'),
      candidates, buy: buttons.some(text => /add to (cart|bag|basket)|buy now|pre-?order/i.test(text)), soldOut: buttons.some(text => /sold out|out of stock|unavailable|notify me/i.test(text))
    };
  } catch (_) { return {ready:false}; }
})()"#;

// A fixed extraction script is the only code Buntini runs in a page. No inputs,
// passwords, textareas or editable fields are inspected. Nothing clicks or writes.
const READ_PAGE: &str = r#"(() => {
  try {
    if (document.readyState !== 'complete' || !document.body) return {ready:false};
    const visibleText = (root, limit) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      const parts = []; let length = 0; let nodes = 0;
      while (walker.nextNode() && ++nodes <= 300) {
        const node = walker.currentNode; const element = node.parentElement;
        if (!element || element.closest('script,style,noscript,form,input,textarea,[contenteditable],svg')) continue;
        const style = getComputedStyle(element);
        if (style.display === 'none' || style.visibility === 'hidden' || !element.getClientRects().length) continue;
        const text = (node.nodeValue || '').slice(0,limit).replace(/\s+/g,' ').trim();
        if (!text) continue;
        parts.push(text); length += text.length + 1;
        if (length >= limit) break;
      }
      return parts.join(' ').slice(0,limit);
    };
    const contentRoot = document.querySelector('main,[role="main"]') || document.querySelector('article') || document.body;
    const walker = document.createTreeWalker(contentRoot, NodeFilter.SHOW_TEXT);
    const parts = []; let length = 0; let nodes = 0; let clipped = false;
    while (walker.nextNode()) {
      if (++nodes > 10000) { clipped = true; break; }
      const node = walker.currentNode;
      const element = node.parentElement;
      if (!element || element.closest('script,style,noscript,form,input,textarea,[contenteditable],svg')) continue;
      const style = getComputedStyle(element);
      if (style.display === 'none' || style.visibility === 'hidden' || !element.getClientRects().length) continue;
      const raw = node.nodeValue || '';
      if (raw.length > 24000) clipped = true;
      const piece = raw.slice(0,24000).replace(/\s+/g, ' ').trim();
      if (!piece) continue;
      parts.push(piece); length += piece.length + 1;
      if (length > 6000) { clipped = true; break; }
    }
    // Links are observations, never commands. Inspect only visible anchors with
    // a result heading; navigation/footer links are not search evidence.
    const links = []; const seen = new Set(); let anchors = 0; let linksClipped = false;
    for (const anchor of document.querySelectorAll('a[href]')) {
      if (++anchors > 3000) { linksClipped = true; break; }
      if (anchor.closest('nav,header,footer,form,[contenteditable]')) continue;
      const style = getComputedStyle(anchor);
      if (style.display === 'none' || style.visibility === 'hidden' || !anchor.getClientRects().length) continue;
      const heading = anchor.querySelector('h1,h2,h3,h4,[role="heading"],.snippet-title,.search-snippet-title') || anchor.closest('h1,h2,h3,h4,[role="heading"]');
      const namedResult = anchor.matches('.heading-serpresult,.result-header,.snippet-title') || !!anchor.closest('.snippet-title,.heading-serpresult');
      if (!heading && !namedResult) continue;
      const url = anchor.href;
      if (!url || url.length > 4096 || seen.has(url)) continue;
      const title = visibleText(heading || anchor, 300);
      if (!title) continue;
      const container = anchor.closest('.snippet,article,li,[data-type="web"]');
      // Never collect form controls or editable content as a result description.
      const summary = container && container.querySelector('.snippet-description,.snippet-content,.description,p');
      let description = '';
      if (summary && !summary.closest('form,[contenteditable]') && getComputedStyle(summary).display !== 'none' && summary.getClientRects().length) {
        description = visibleText(summary, 700);
      }
      links.push({url,title,description}); seen.add(url);
      if (links.length >= 32) { linksClipped = true; break; }
    }
    return {ready:true, url:location.href, title:(document.title || '').slice(0,300), text:parts.join('\n').slice(0,6000), truncated:clipped,links,linksTruncated:linksClipped};
  } catch (_) { return {ready:false}; }
})()"#;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservedLink {
    url: String,
    title: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct Observation {
    ready: bool,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    links: Vec<ObservedLink>,
    #[serde(default)]
    links_truncated: bool,
}
fn observation(
    raw: &str,
    native_url: &reqwest::Url,
    approved_origin: &str,
) -> Result<Option<Observation>, String> {
    if raw.len() > RESPONSE_LIMIT {
        return Err("The browser returned an oversized page snapshot.".into());
    }
    let value: Observation =
        serde_json::from_str(raw).map_err(|_| "The browser returned an invalid page snapshot.")?;
    // WKWebView can expose the destination native URL while the previous blank
    // document is still complete. Wait for the new document, never accept it.
    if !value.ready || value.url == "about:blank" {
        return Ok(None);
    }
    let page_url =
        reqwest::Url::parse(&value.url).map_err(|_| "The page snapshot has no valid URL.")?;
    if !navigation_allowed(native_url, approved_origin)
        || native_url.scheme() != "https"
        || origin(&page_url) != approved_origin
        || page_url != *native_url
    {
        return Err("The page changed during readback or left the approved origin. Its contents were not accepted.".into());
    }
    if value.url.len() > 4096
        || value.title.chars().count() > 300
        || value.text.chars().count() > TEXT_LIMIT
        || value.links.len() > LINK_LIMIT
        || value.links.iter().any(|link| {
            link.url.len() > 4096
                || link.title.chars().count() > 300
                || link.description.chars().count() > 700
        })
        || (value.title.trim().is_empty() && value.text.trim().is_empty())
    {
        return Err("The page did not return a useful bounded snapshot.".into());
    }
    Ok(Some(value))
}

async fn snapshot(window: &WebviewWindow) -> Result<String, String> {
    run_script(window, READ_PAGE).await
}

async fn run_script(window: &WebviewWindow, script: &str) -> Result<String, String> {
    let (send, receive) = oneshot::channel();
    let sender = Mutex::new(Some(send));
    window
        .eval_with_callback(script, move |value| {
            if let Some(send) = sender.lock().unwrap().take() {
                let _ = send.send(value);
            }
        })
        .map_err(|_| "The page could not be inspected.")?;
    tokio::time::timeout(Duration::from_secs(2), receive)
        .await
        .map_err(|_| "The page readback timed out.")?
        .map_err(|_| "The page closed before readback.".to_string())
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
        sel,
    };
    use objc2_foundation::{NSArray, NSProcessInfo, NSString};
    use std::ffi::{CStr, CString};

    type Endpoint =
        unsafe extern "C" fn(*const libc::c_char, *const libc::c_char) -> *mut AnyObject;
    type MakeProxy = unsafe extern "C" fn(*mut AnyObject, *mut AnyObject) -> *mut AnyObject;
    type SetFailover = unsafe extern "C" fn(*mut AnyObject, bool);
    type GetFailover = unsafe extern "C" fn(*mut AnyObject) -> bool;
    struct Symbols {
        endpoint: Endpoint,
        proxy: MakeProxy,
        set_failover: SetFailover,
        get_failover: GetFailover,
    }
    static SYMBOLS: OnceLock<Result<Symbols, String>> = OnceLock::new();

    pub fn supported() -> bool {
        NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 14
    }

    pub async fn remove_profile(app: &AppHandle, id: [u8; 16]) -> Result<(), String> {
        for attempt in 0..3 {
            match remove_profile_once(app, id).await? {
                true => return Ok(()),
                false if attempt < 2 => tokio::time::sleep(Duration::from_millis(250)).await,
                false => return Err("WebKit could not confirm browser profile deletion. Close its window and try Delete everything again.".into()),
            }
        }
        unreachable!()
    }

    async fn remove_profile_once(app: &AppHandle, id: [u8; 16]) -> Result<bool, String> {
        if !supported() {
            return Err("Removing isolated browser profiles requires macOS 14 or later.".into());
        }
        let (send, receive) = oneshot::channel();
        app.run_on_main_thread(move || unsafe {
            let sender = Arc::new(Mutex::new(Some(send)));
            let result = (|| -> Result<(), String> {
                let class = AnyClass::get(c"WKWebsiteDataStore").ok_or("The browser data-store API is unavailable.")?;
                let available: bool = msg_send![class, respondsToSelector: sel!(removeDataStoreForIdentifier:completionHandler:)];
                if !available { return Err("This macOS version cannot remove isolated browser profiles.".into()); }
                let uuid_class = AnyClass::get(c"NSUUID").ok_or("The native identifier API is unavailable.")?;
                let allocated: *mut AnyObject = msg_send![uuid_class, alloc];
                let identifier: *mut AnyObject = msg_send![allocated, initWithUUIDString: &*NSString::from_str(&uuid::Uuid::from_bytes(id).to_string())];
                let identifier = Retained::<AnyObject>::from_raw(identifier).ok_or("The native profile identifier could not be created.")?;
                let callback_sender = sender.clone();
                let callback = block2::RcBlock::new(move |error: *mut AnyObject| {
                    // Do not surface native descriptions that may contain local
                    // cookie/cache paths; preserve the ID for an explicit retry.
                    let result = Ok(error.is_null());
                    if let Some(send) = callback_sender.lock().unwrap().take() { let _ = send.send(result); }
                });
                let _: () = msg_send![class, removeDataStoreForIdentifier: &*identifier, completionHandler: &*callback];
                Ok(())
            })();
            if let Err(error) = result {
                if let Some(send) = sender.lock().unwrap().take() { let _ = send.send(Err(error)); }
            }
        }).map_err(|_| "Browser profile deletion could not start.")?;
        tokio::time::timeout(Duration::from_secs(10), receive).await.map_err(|_| "Browser profile deletion timed out. Its identifier was retained so deletion can be retried.")?.map_err(|_| "Browser profile deletion ended without confirmation.")?
    }

    fn symbols() -> Result<&'static Symbols, String> {
        SYMBOLS
            .get_or_init(|| unsafe {
                // Keep this framework loaded for process lifetime: retained native
                // configuration objects may outlive this call.
                let library = libc::dlopen(
                    c"/System/Library/Frameworks/Network.framework/Network".as_ptr(),
                    libc::RTLD_NOW | libc::RTLD_LOCAL,
                );
                if library.is_null() {
                    return Err("Network framework could not be loaded.".into());
                }
                let get = |name: &CStr| -> Result<*mut libc::c_void, String> {
                    let function = libc::dlsym(library, name.as_ptr());
                    if function.is_null() {
                        Err(
                            "Required proxy enforcement is unavailable on this macOS version."
                                .into(),
                        )
                    } else {
                        Ok(function)
                    }
                };
                Ok(Symbols {
                    endpoint: std::mem::transmute::<*mut libc::c_void, Endpoint>(get(
                        c"nw_endpoint_create_host",
                    )?),
                    proxy: std::mem::transmute::<*mut libc::c_void, MakeProxy>(get(
                        c"nw_proxy_config_create_http_connect",
                    )?),
                    set_failover: std::mem::transmute::<*mut libc::c_void, SetFailover>(get(
                        c"nw_proxy_config_set_failover_allowed",
                    )?),
                    get_failover: std::mem::transmute::<*mut libc::c_void, GetFailover>(get(
                        c"nw_proxy_config_get_failover_allowed",
                    )?),
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub async fn configure(
        window: &WebviewWindow,
        profile: Option<[u8; 16]>,
        port: u16,
    ) -> Result<(), String> {
        let symbols = symbols()?;
        let (send, receive) = oneshot::channel();
        window
            .with_webview(move |platform| {
                let result = unsafe { install(platform.inner().cast(), profile, port, symbols) };
                let _ = send.send(result);
            })
            .map_err(|_| "Could not configure the native browser isolation.")?;
        tokio::time::timeout(Duration::from_secs(5), receive)
            .await
            .map_err(|_| "Browser isolation configuration timed out.")?
            .map_err(|_| "Browser isolation configuration ended unexpectedly.")?
    }

    unsafe fn install(
        view: *mut AnyObject,
        profile: Option<[u8; 16]>,
        port: u16,
        symbols: &Symbols,
    ) -> Result<(), String> {
        let configuration: *mut AnyObject = msg_send![view, configuration];
        let store: *mut AnyObject = msg_send![configuration, websiteDataStore];
        for selector in [
            sel!(identifier),
            sel!(proxyConfigurations),
            sel!(setProxyConfigurations:),
        ] {
            let available: bool = msg_send![store, respondsToSelector: selector];
            if !available {
                return Err(
                    "Required persistent-profile or proxy isolation is unavailable.".into(),
                );
            }
        }
        let identifier: *mut AnyObject = msg_send![store, identifier];
        let persistent: bool = msg_send![store, isPersistent];
        let identifier_text = if !identifier.is_null() {
            let identifier_text: Retained<NSString> = msg_send![identifier, UUIDString];
            Some(identifier_text.to_string())
        } else {
            None
        };
        verify_store_identity(profile, persistent, identifier_text.as_deref())?;
        // HTTP proxies cannot carry WebRTC's direct peer sockets. Disable those
        // APIs natively, alongside media capture, before loading remote content.
        // WebKit source: WKPreferencesPrivate.h / WKPreferences.mm.
        let preferences: *mut AnyObject = msg_send![configuration, preferences];
        for selector in [
            sel!(_setPeerConnectionEnabled:),
            sel!(_peerConnectionEnabled),
            sel!(_setMediaDevicesEnabled:),
            sel!(_mediaDevicesEnabled),
            sel!(_setScreenCaptureEnabled:),
            sel!(_screenCaptureEnabled),
        ] {
            let available: bool = msg_send![preferences, respondsToSelector: selector];
            if !available {
                return Err("The browser cannot enforce direct-connection and capture restrictions on this macOS version.".into());
            }
        }
        let _: () = msg_send![preferences, _setPeerConnectionEnabled: false];
        let _: () = msg_send![preferences, _setMediaDevicesEnabled: false];
        let _: () = msg_send![preferences, _setScreenCaptureEnabled: false];
        let peers: bool = msg_send![preferences, _peerConnectionEnabled];
        let media: bool = msg_send![preferences, _mediaDevicesEnabled];
        let capture: bool = msg_send![preferences, _screenCaptureEnabled];
        if peers || media || capture {
            return Err("The browser did not apply its direct-connection restrictions.".into());
        }
        let port = CString::new(port.to_string()).map_err(|_| "Invalid proxy port.")?;
        let endpoint = Retained::<AnyObject>::from_raw((symbols.endpoint)(
            c"127.0.0.1".as_ptr(),
            port.as_ptr(),
        ))
        .ok_or("Proxy endpoint creation failed.")?;
        let proxy = Retained::<AnyObject>::from_raw((symbols.proxy)(
            Retained::as_ptr(&endpoint).cast_mut(),
            std::ptr::null_mut(),
        ))
        .ok_or("Proxy configuration creation failed.")?;
        (symbols.set_failover)(Retained::as_ptr(&proxy).cast_mut(), false);
        if (symbols.get_failover)(Retained::as_ptr(&proxy).cast_mut()) {
            return Err("The proxy allowed direct-network fallback.".into());
        }
        let proxies = NSArray::from_slice(&[&*proxy]);
        let _: () = msg_send![store, setProxyConfigurations: &*proxies];
        let installed: *mut AnyObject = msg_send![store, proxyConfigurations];
        let count: usize = msg_send![installed, count];
        if count != 1 {
            return Err("The browser did not retain the required network proxy.".into());
        }
        let installed_proxy: *mut AnyObject = msg_send![installed, objectAtIndex: 0usize];
        if installed_proxy != Retained::as_ptr(&proxy).cast_mut()
            || (symbols.get_failover)(installed_proxy)
        {
            return Err("The installed browser proxy could not be verified.".into());
        }
        Ok(())
    }
}

/// Called only after the durable journal consumes an approval. The workspace's
/// page snapshot is Browser data and must inherit that privacy category.
pub async fn execute(
    intent_id: &str,
    args: &Map<String, Value>,
    cancel: Arc<AtomicBool>,
) -> ActionReceipt {
    read_approved_page(intent_id, args, cancel).await
}

/// A blank isolated window behind the verified proxy, ready to navigate. `allow` decides each
/// navigation. Failure at any step leaves nothing loaded.
#[cfg(target_os = "macos")]
async fn open_isolated(
    app: &AppHandle,
    profile: Option<[u8; 16]>,
    approved_origin: String,
    allow: impl Fn(&reqwest::Url) -> bool + Send + 'static,
) -> Result<Workspace, String> {
    let proxy = proxy().await?;
    let stopped = proxy.stopped.clone();
    let builder = WebviewWindowBuilder::new(
        app,
        format!("buntini-browser-{}", uuid::Uuid::new_v4()),
        WebviewUrl::External(reqwest::Url::parse("about:blank").unwrap()),
    )
    .title("Buntini browser workspace")
    .inner_size(1100.0, 780.0)
    .visible(false)
    .on_navigation(move |url| !stopped.load(Ordering::Acquire) && allow(url))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_download(|_, _| false);
    let builder = if let Some(profile) = profile {
        builder.data_store_identifier(profile)
    } else {
        builder.incognito(true)
    };
    let window = builder.build().map_err(|_| "The isolated browser window could not be created.".to_string())?;
    let workspace = Workspace { window, proxy, profile, approved_origin };
    let stopped = workspace.proxy.stopped.clone();
    workspace.window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            stopped.store(true, Ordering::Release);
        }
    });
    native::configure(&workspace.window, profile, workspace.proxy.port).await?;
    Ok(workspace)
}

/// Loads a page the user chose to watch in a fresh private store with no window on screen,
/// waits for its scripts to settle, and reads what it shows with a fixed script. Nothing is
/// clicked, and the model never sees it. Only the page's own site can be navigated to.
pub async fn read_rendered_price(raw_url: &str, cancel: Arc<AtomicBool>) -> Result<crate::pricing::RenderedPage, String> {
    let url = crate::research::validate_public_url(raw_url)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (url, cancel);
        return Err("Reading this shop needs macOS 14 or later.".into());
    }
    #[cfg(target_os = "macos")]
    {
        if !native::supported() {
            return Err("Reading this shop needs macOS 14 or later.".into());
        }
        let app = APP.get().ok_or("The browser workspace is not initialized.")?;
        // Its own window and proxy, one read at a time: a check in the background must never
        // disturb a page the user has open in the workspace.
        let _turn = PRICE_READS.lock().await;
        let site = url.host_str().unwrap_or_default().to_string();
        let workspace = open_isolated(app, None, origin(&url), move |target| {
            target.as_str() == "about:blank"
                || (crate::research::validate_public_url(target.as_str()).is_ok()
                    && crate::research::same_site(&site, target.host_str().unwrap_or_default()))
        })
        .await?;
        workspace.window.navigate(url).map_err(|_| "The shop page could not be opened.".to_string())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        let mut last: Option<crate::pricing::RenderedPage> = None;
        let mut settled_since = tokio::time::Instant::now();
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Price check cancelled.".into());
            }
            if workspace.proxy.stopped.load(Ordering::Acquire) {
                return Err("The browser closed before the page could be read.".into());
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            if let Ok(raw) = run_script(&workspace.window, READ_PRICE).await {
                if let Ok(page) = serde_json::from_str::<crate::pricing::RenderedPage>(&raw) {
                    if page.ready {
                        let same = last.as_ref().is_some_and(|before| before.signature() == page.signature());
                        if !same {
                            settled_since = now;
                        }
                        let has_something = !page.candidates.is_empty() || !page.json_ld.is_empty() || page.meta_amount.is_some();
                        last = Some(page);
                        // Prices hydrate in late, so what's read has to have stopped changing.
                        if has_something && same && now.duration_since(settled_since) >= Duration::from_millis(2500) {
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        last.ok_or_else(|| "The shop page didn’t finish loading.".to_string())
    }
}

async fn read_approved_page(
    intent_id: &str,
    args: &Map<String, Value>,
    cancel: Arc<AtomicBool>,
) -> ActionReceipt {
    let args = match validate("browse_page", &Value::Object(args.clone())) {
        Ok(args) => args,
        Err(error) => return failed(ActionOutcome::Failed, error),
    };
    if cancel.load(Ordering::Acquire) {
        return failed(
            ActionOutcome::Failed,
            "Browser navigation was cancelled before it started.",
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (intent_id, args);
        return failed(
            ActionOutcome::Failed,
            "The isolated browser currently requires macOS 14 or later.",
        );
    }
    #[cfg(target_os = "macos")]
    {
        if !native::supported() {
            return failed(
                ActionOutcome::Failed,
                "The isolated browser requires macOS 14 or later. Buntini did not open the page.",
            );
        }
        let Some(app) = APP.get() else {
            return failed(
                ActionOutcome::Failed,
                "The browser workspace is not initialized.",
            );
        };
        let mut slot = match tokio::time::timeout(Duration::from_secs(2), WORKSPACE.lock()).await {
            Ok(slot) => slot,
            Err(_) => {
                return failed(
                    ActionOutcome::Failed,
                    "Another browser operation is still running.",
                )
            }
        };
        if cancel.load(Ordering::Acquire) {
            return failed(
                ActionOutcome::Failed,
                "Browser navigation was cancelled before it started.",
            );
        }
        // Every operation starts from a blank view with freshly verified proxy
        // settings. Research has a fresh nonpersistent store for every page.
        slot.take();
        let url = reqwest::Url::parse(args["url"].as_str().unwrap()).unwrap();
        let approved_origin = origin(&url);
        let directory = match app.path().app_data_dir() {
            Ok(directory) => directory,
            Err(_) => {
                return failed(
                    ActionOutcome::Failed,
                    "The browser profile directory is unavailable.",
                )
            }
        };
        let profile = match profile_id(&directory, &approved_origin) {
            Ok(id) => Some(id),
            Err(error) => return failed(ActionOutcome::Failed, error),
        };
        let scoped_origin = approved_origin.clone();
        let denied = Arc::new(Mutex::new(None::<String>));
        let denied_navigation = denied.clone();
        let opened = open_isolated(app, profile, approved_origin, move |url| {
            let allowed = navigation_allowed(url, &scoped_origin);
            if !allowed {
                *denied_navigation.lock().unwrap() =
                    Some("A navigation outside the approved HTTPS origin was blocked.".into());
            }
            allowed
        })
        .await;
        let workspace = match opened {
            Ok(workspace) => workspace,
            Err(error) => return failed(ActionOutcome::Failed, error),
        };
        if cancel.load(Ordering::Acquire) {
            return failed(
                ActionOutcome::Failed,
                "Browser navigation was cancelled before it started.",
            );
        }
        if workspace.window.navigate(url).is_err() {
            return failed(ActionOutcome::Unknown, "The browser did not confirm navigation. No website action was performed by Buntini.");
        }
        if workspace.window.show().is_err() {
            return failed(ActionOutcome::Unknown, "The browser page may have loaded, but its window could not be shown. No website action was performed by Buntini.");
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        let result = loop {
            if cancel.load(Ordering::Acquire) {
                break Err(
                    "Browser readback was cancelled. The page was not confirmed.".to_string(),
                );
            }
            if workspace.proxy.stopped.load(Ordering::Acquire) {
                break Err(
                    "The browser workspace closed before the page could be verified.".to_string(),
                );
            }
            if let Some(error) = denied.lock().unwrap().take() {
                break Err(error);
            }
            if tokio::time::Instant::now() >= deadline {
                break Err("The page did not produce a verified snapshot within 25 seconds. It may need a manual sign-in or a new approved destination.".to_string());
            }
            if let Ok(native_url) = workspace.window.url() {
                if native_url.scheme() == "https" {
                    if let Ok(raw) = snapshot(&workspace.window).await {
                        let current_url = workspace.window.url().unwrap_or(native_url);
                        match observation(&raw, &current_url, &workspace.approved_origin) {
                            Ok(Some(observed)) => break Ok(observed),
                            Ok(None) => {}
                            Err(error)
                                if error.starts_with("The page changed during readback")
                                    && navigation_allowed(
                                        &current_url,
                                        &workspace.approved_origin,
                                    ) =>
                            {
                                // Same-origin SPA navigation may race readback.
                                // A fresh observation must agree exactly; no
                                // mismatched snapshot is accepted or reused.
                            }
                            Err(error) => break Err(error),
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        };
        let receipt = match result {
            Ok(observed) => ActionReceipt { outcome: ActionOutcome::Succeeded, tool: "browse_page".into(), object_id: workspace.profile.map(|profile| format!("browser-profile:{}", uuid::Uuid::from_bytes(profile))), destination: Some(observed.url.clone()),
                message: format!("Opened and read “{}” in Buntini’s separate browser. Sign-in status and website actions have not been verified.", observed.title),
                evidence: json!({"intentId":intent_id,"url":observed.url,"title":observed.title,"text":observed.text,"truncated":observed.truncated,"links":observed.links,"linksTruncated":observed.links_truncated,"sourceKind":"untrusted_web_page","verification":"native_url_and_bounded_page_snapshot","authentication":"not_verified","websiteActionsPerformed":false,"profileScope":workspace.approved_origin}), error: None },
            Err(error) => failed(ActionOutcome::Unknown, error),
        };
        if cancel.load(Ordering::Acquire) {
            drop(workspace);
        } else {
            *slot = Some(workspace);
        }
        receipt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_store_cannot_inherit_a_persistent_or_named_profile() {
        let id = *uuid::Uuid::new_v4().as_bytes();
        let name = uuid::Uuid::from_bytes(id).to_string();
        assert!(verify_store_identity(None, false, None).is_ok());
        assert!(verify_store_identity(None, true, None).is_err());
        assert!(verify_store_identity(None, true, Some(&name)).is_err());
        assert!(verify_store_identity(None, false, Some(&name)).is_err());
        assert!(verify_store_identity(Some(id), true, Some(&name)).is_ok());
        assert!(verify_store_identity(Some(id), false, Some(&name)).is_err());
        assert!(verify_store_identity(Some(id), true, None).is_err());
        assert!(verify_store_identity(Some(id), true, Some("another-profile")).is_err());
    }

    #[test]
    fn navigation_is_https_exact_origin_and_never_a_local_protocol() {
        let approved = "https://example.com";
        for raw in [
            "https://example.com/",
            "https://example.com/sign-in?next=%2Fhome",
            "https://example.com/#section",
        ] {
            assert!(
                navigation_allowed(&reqwest::Url::parse(raw).unwrap(), approved),
                "{raw}"
            );
        }
        for raw in [
            "http://example.com/",
            "https://login.example.com/",
            "https://example.com.evil.com/",
            "https://example.com:8443/",
            "https://name:password@example.com/",
            "tauri://localhost/",
            "https://tauri.localhost/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:test@example.com",
            "https://127.0.0.1/",
        ] {
            assert!(
                !navigation_allowed(&reqwest::Url::parse(raw).unwrap(), approved),
                "{raw}"
            );
        }
        assert!(validate(
            "browse_page",
            &json!({"url":"https://example.com/a#section"})
        )
        .unwrap()["url"]
            .as_str()
            .unwrap()
            .ends_with("#section"));
        assert!(validate(
            "browse_page",
            &json!({"url":"https://example.com", "script":"anything"})
        )
        .is_err());
    }

    #[test]
    fn proxy_only_accepts_bounded_public_https_connect_requests() {
        assert_eq!(
            connect_host(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n")
                .unwrap(),
            "example.com"
        );
        for target in [
            "127.0.0.1:443",
            "127.1:443",
            "2130706433:443",
            "[::1]:443",
            "[::ffff:127.0.0.1]:443",
            "10.0.0.1:443",
            "localhost:443",
            "office.local:443",
            "example.com:80",
            "example.com:8443",
            "user@example.com:443",
            "example.com/path:443",
            "example.com?query:443",
            "example.com#fragment:443",
        ] {
            assert!(
                connect_host(format!("CONNECT {target} HTTP/1.1\r\n\r\n").as_bytes()).is_err(),
                "{target}"
            );
        }
        for header in [
            "GET https://example.com HTTP/1.1\r\n\r\n",
            "CONNECT  example.com:443 HTTP/1.1\r\n\r\n",
            "CONNECT example.com:443 HTTP/2\r\n\r\n",
            "CONNECT example.com:443 HTTP/1.1\r\n",
        ] {
            assert!(connect_host(header.as_bytes()).is_err(), "{header}");
        }
        let too_long = format!(
            "CONNECT example.com:443 HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(HEADER_LIMIT)
        );
        assert!(connect_host(too_long.as_bytes()).is_err());
    }

    #[test]
    fn every_resolved_address_must_be_public_to_prevent_mixed_answer_rebinding() {
        let public: SocketAddr = "1.1.1.1:443".parse().unwrap();
        assert!(public_addresses(&[public]).is_ok());
        assert!(public_addresses(&[]).is_err());
        for private in [
            "127.0.0.1:443",
            "10.2.3.4:443",
            "192.168.1.5:443",
            "169.254.169.254:443",
            "[::1]:443",
            "[fd00::1]:443",
            "[::ffff:127.0.0.1]:443",
            "1.1.1.1:80",
        ] {
            assert!(
                public_addresses(&[public, private.parse().unwrap()]).is_err(),
                "{private}"
            );
        }
    }

    #[test]
    fn profiles_persist_per_origin_without_silent_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let first = profile_id(directory.path(), "https://example.com").unwrap();
        assert_eq!(
            profile_id(directory.path(), "https://example.com").unwrap(),
            first
        );
        assert_ne!(
            profile_id(directory.path(), "https://other.example.com").unwrap(),
            first
        );
        let digest = Sha256::digest("https://example.com".as_bytes());
        let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        std::fs::write(
            directory.path().join("browser-profiles").join(name),
            b"damaged",
        )
        .unwrap();
        assert!(
            profile_id(directory.path(), "https://example.com").is_err(),
            "corruption must not silently lose the isolated profile"
        );
    }

    #[test]
    fn deletion_enumeration_keeps_the_identifiers_until_native_cleanup_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        assert!(saved_profiles(directory.path()).unwrap().is_empty());
        let id = profile_id(directory.path(), "https://example.com").unwrap();
        let profiles = saved_profiles(directory.path()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].1, id);
        assert!(
            profiles[0].0.exists(),
            "listing must not erase recovery identifiers"
        );
        std::fs::write(&profiles[0].0, b"damaged").unwrap();
        assert!(saved_profiles(directory.path()).is_err());
        assert!(profiles[0].0.exists());
        #[cfg(unix)]
        {
            let isolated = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(directory.path(), isolated.path().join("browser-profiles"))
                .unwrap();
            assert!(saved_profiles(isolated.path()).is_err());
            assert!(profile_id(isolated.path(), "https://example.com").is_err());
        }
    }

    #[test]
    fn snapshots_require_native_url_agreement_bounded_content_and_strict_shape() {
        let url = reqwest::Url::parse("https://example.com/account").unwrap();
        let data = json!({"ready":true,"url":url.as_str(),"title":"Account","text":"Your visible page","truncated":false});
        assert!(observation(&data.to_string(), &url, "https://example.com")
            .unwrap()
            .is_some());
        assert!(
            observation(r#"{"ready":false}"#, &url, "https://example.com")
                .unwrap()
                .is_none()
        );
        assert!(observation(
            r#"{"ready":true,"url":"about:blank"}"#,
            &url,
            "https://example.com"
        )
        .unwrap()
        .is_none());
        assert!(observation(
            &data.to_string(),
            &reqwest::Url::parse("https://example.com/changed").unwrap(),
            "https://example.com"
        )
        .is_err());
        assert!(observation(&data.to_string(), &url, "https://elsewhere.com").is_err());
        let mut unbounded = data.clone();
        unbounded["text"] = json!("x".repeat(TEXT_LIMIT + 1));
        assert!(observation(&unbounded.to_string(), &url, "https://example.com").is_err());
        let mut injected = data;
        injected["tool"] = json!("send_email");
        assert!(observation(&injected.to_string(), &url, "https://example.com").is_err());
    }

    #[test]
    fn observed_links_are_bounded_and_cannot_carry_extra_commands() {
        let url = reqwest::Url::parse("https://search.brave.com/search?q=museum").unwrap();
        let link = json!({"url":"https://museum.org/visit","title":"Museum hours","description":"The museum is open"});
        let mut data = json!({"ready":true,"url":url,"title":"Museum search","text":"Search results","links":[link.clone()],"linksTruncated":false});
        assert!(
            observation(&data.to_string(), &url, "https://search.brave.com")
                .unwrap()
                .is_some()
        );
        data["links"][0]["click"] = json!(true);
        assert!(observation(&data.to_string(), &url, "https://search.brave.com").is_err());
        data["links"] = Value::Array(vec![link.clone(); LINK_LIMIT + 1]);
        assert!(observation(&data.to_string(), &url, "https://search.brave.com").is_err());
        data["links"] = json!([link]);
        data["links"][0]["description"] = json!("x".repeat(701));
        assert!(observation(&data.to_string(), &url, "https://search.brave.com").is_err());
    }

    #[tokio::test]
    async fn proxy_header_reader_stops_before_tunnel_data_and_caps_missing_terminator() {
        let request = b"CONNECT example.com:443 HTTP/1.1\r\n\r\nTLS BYTES";
        let mut stream = &request[..];
        assert_eq!(
            read_header(&mut stream).await.unwrap(),
            b"CONNECT example.com:443 HTTP/1.1\r\n\r\n"
        );
        assert_eq!(stream, b"TLS BYTES");
        let bytes = vec![b'x'; HEADER_LIMIT + 1];
        assert!(read_header(&mut bytes.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn cancellation_and_invalid_input_fail_before_browser_initialization() {
        let cancel = Arc::new(AtomicBool::new(true));
        let args = validate("browse_page", &json!({"url":"https://example.com"})).unwrap();
        let receipt = execute("test", &args, cancel).await;
        assert_eq!(receipt.outcome, ActionOutcome::Failed);
        let receipt = execute(
            "test",
            &json!({"url":"https://127.0.0.1"}).as_object().unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert_eq!(receipt.outcome, ActionOutcome::Failed);
    }

}
