//! Research in the user's existing desktop browser. Only an owned, newly created
//! tab is navigated. Browser preferences, existing tabs and private windows are
//! never changed. Source text inherits Browser privacy, including search results.
use crate::{research::ResearchContent, types::Source};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub const SEARCH_HOST: &str = "www.google.com";
const MAX_SOURCES: usize = 10;
const MAX_TEXT: usize = 8000;
const MAX_LINKS: usize = 64;
static OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTarget {
    pub name: String,
    pub bundle_id: String,
    pub app_id: Option<String>,
    pub readable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserError {
    pub message: String,
    pub did_read: bool,
    pub navigated: bool,
}
impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for BrowserError {}
fn failure(message: impl Into<String>, did_read: bool, navigated: bool) -> BrowserError {
    BrowserError {
        message: message.into(),
        did_read,
        navigated,
    }
}

#[derive(Debug)]
pub struct ResearchSession {
    target: BrowserTarget,
    window_id: String,
    tab_id: String,
    last_url: String,
    usable: bool,
}

#[derive(Debug)]
pub struct BrowserSearch {
    pub query: String,
    pub search_url: String,
    pub retrieved_at: String,
    pub sources: Vec<Source>,
    pub truncated: bool,
    pub session: ResearchSession,
}

fn target(bundle: &str, name: &str) -> BrowserTarget {
    let canonical = match bundle.to_ascii_lowercase().as_str() {
        "com.google.chrome" => "com.google.Chrome",
        "com.brave.browser" => "com.brave.Browser",
        "com.apple.safari" => "com.apple.Safari",
        "company.thebrowser.browser" => "company.thebrowser.Browser",
        _ => bundle,
    };
    let app_id = match canonical {
        "com.google.Chrome" => Some("chrome"),
        "com.brave.Browser" => Some("brave"),
        "com.microsoft.edgemac" => Some("edge"),
        "com.apple.Safari" => Some("safari"),
        "company.thebrowser.Browser" => Some("arc"),
        _ => None,
    };
    BrowserTarget {
        name: name.into(),
        bundle_id: canonical.into(),
        app_id: app_id.map(str::to_string),
        readable: matches!(app_id, Some("chrome" | "brave" | "edge")),
    }
}

#[cfg(target_os = "macos")]
fn default_browser_bundle() -> Option<String> {
    use objc2_core_foundation::{CFRetained, CFString};
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultHandlerForURLScheme(scheme: *const CFString) -> *const CFString;
    }
    let scheme = CFString::from_str("https");
    let raw = unsafe { LSCopyDefaultHandlerForURLScheme(&*scheme) };
    let retained = unsafe { CFRetained::from_raw(std::ptr::NonNull::new(raw.cast_mut())?) };
    Some(retained.to_string())
}

/// Metadata only: no browser tabs or page content are inspected during approval
/// preparation. The chosen app is held fixed for the approved operation.
pub fn selected_browser() -> Result<BrowserTarget, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSBundle, NSString, NSURL};
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        if let Some(app) = workspace.frontmostApplication() {
            if let Some(bundle) = app.bundleIdentifier() {
                let bundle = bundle.to_string();
                let name = app
                    .localizedName()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| bundle.clone());
                let choice = target(&bundle, &name);
                if choice.app_id.is_some() {
                    return Ok(choice);
                }
            }
        }
        let url = NSURL::URLWithString(&NSString::from_str("https://www.google.com/"))
            .ok_or("Could not identify the default web browser.")?;
        let application = workspace.URLForApplicationToOpenURL(&url);
        let bundle = application
            .as_ref()
            .and_then(|url| NSBundle::bundleWithURL(url))
            .and_then(|bundle| bundle.bundleIdentifier())
            .map(|value| value.to_string())
            .or_else(default_browser_bundle)
            .ok_or("Choose a default web browser in macOS Settings first.")?;
        let choice = target(&bundle, &bundle);
        let application = application.or_else(|| {
            workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(&choice.bundle_id))
        });
        let name = application
            .and_then(|url| url.lastPathComponent())
            .map(|value| value.to_string())
            .unwrap_or_else(|| match choice.app_id.as_deref() {
                Some("chrome") => "Google Chrome".into(),
                Some("brave") => "Brave Browser".into(),
                Some("edge") => "Microsoft Edge".into(),
                Some("safari") => "Safari".into(),
                Some("arc") => "Arc".into(),
                _ => bundle.clone(),
            });
        Ok(target(&bundle, name.trim_end_matches(".app")))
    }
    #[cfg(not(target_os = "macos"))]
    Err("Desktop browser research currently requires macOS.".into())
}

fn cancelled(cancel: &AtomicBool, did_read: bool, navigated: bool) -> Result<(), BrowserError> {
    if cancel.load(Ordering::Acquire) {
        Err(failure("Task cancelled.", did_read, navigated))
    } else {
        Ok(())
    }
}

pub fn search_url(query: &str) -> Result<reqwest::Url, String> {
    if query.trim().is_empty() || query.chars().count() > 2000 || query.chars().any(char::is_control) {
        return Err("Enter a web query of up to 2,000 characters.".into());
    }
    let mut url = reqwest::Url::parse("https://www.google.com/search").unwrap();
    url.query_pairs_mut().append_pair("q", query);
    Ok(url)
}

/// Preflight only. An ordinary browser owns its networking and redirect policy;
/// this does not claim the embedded workspace's pinned-proxy guarantees.
async fn public_destination(raw: &str) -> Result<(), String> {
    let url = crate::research::validate_public_url(raw)?;
    let host = url.host_str().ok_or("The page needs a public host.")?;
    let addresses =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| "Browser destination DNS timed out.")?
            .map_err(|_| "The browser destination could not be resolved.")?;
    let addresses: Vec<_> = addresses.take(33).collect();
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses
            .iter()
            .any(|a| !crate::research::is_public_ip(a.ip()))
    {
        return Err(
            "The browser destination does not resolve exclusively to public addresses.".into(),
        );
    }
    Ok(())
}

async fn command(
    mut command: tokio::process::Command,
    cancel: Arc<AtomicBool>,
    seconds: u64,
) -> Result<String, String> {
    command.kill_on_drop(true);
    let pending = crate::actions::output(command, seconds);
    tokio::pin!(pending);
    loop {
        tokio::select! {
            result=&mut pending=>return result,
            _=tokio::time::sleep(Duration::from_millis(40))=>if cancel.load(Ordering::Acquire){return Err("Task cancelled.".into());}
        }
    }
}

async fn open_only(
    target: &BrowserTarget,
    url: &str,
    cancel: Arc<AtomicBool>,
) -> Result<(), BrowserError> {
    cancelled(&cancel, false, false)?;
    public_destination(url)
        .await
        .map_err(|e| failure(e, false, false))?;
    cancelled(&cancel, false, false)?;
    let mut open = tokio::process::Command::new("/usr/bin/open");
    open.arg("-b").arg(&target.bundle_id).arg(url);
    command(open, cancel, 12)
        .await
        .map_err(|e| failure(e, false, true))?;
    Ok(())
}

// All operations are fixed code. The bundle, IDs, expected URL, new URL and
// extraction program travel as argv, never interpolated into executable source.
const CONTROL: &str = r#"function run(argv) {
  function fail(code,message) { throw {buntiniCode:code,message:message}; }
  let mutated=false;
  try {
    const mode=argv[0], bundle=argv[1];
    if (['com.google.Chrome','com.brave.Browser','com.microsoft.edgemac'].indexOf(bundle)<0) fail('unsupported','This browser cannot expose a stable research tab.');
    const app=Application(bundle);
    if(mode==='preflight') {
      // Metadata only. This branch must never launch, focus, navigate or create
      // anything, so a permission timeout can safely use OS-open exactly once.
      if(app.running()) {const count=app.windows.length;}
      return JSON.stringify({ok:true});
    }
    if (!app.running()) {
      if (mode!=='create') fail('closed','The research browser was closed.');
      app.launch();
      const deadline=Date.now()+6000;
      while(!app.running() && Date.now()<deadline) delay(0.1);
      if(!app.running()) fail('closed','The default browser could not finish launching.');
    }
    let windows=app.windows();
    let w, t;
    if (mode==='create') {
      w=windows.find(window=>window.mode()==='normal');
      let freshWindow=false;
      if (!w) {
        const beforeWindows=windows.map(window=>String(window.id()));
        mutated=true;
        app.windows.push(app.Window({mode:'normal'}));
        windows=app.windows();
        const added=windows.filter(window=>beforeWindows.indexOf(String(window.id()))<0 && window.mode()==='normal');
        if(added.length!==1) fail('identity','A new normal window could not be identified. Existing windows were not changed.');
        w=added[0];freshWindow=true;
      }
      const beforeTabs=w.tabs();
      const before=beforeTabs.map(tab=>String(tab.id()));
      if(freshWindow && beforeTabs.length===1 && ['','about:blank','chrome://newtab/','chrome://new-tab-page/','brave://newtab/','edge://newtab/'].indexOf(beforeTabs[0].url())>=0){
        t=beforeTabs[0];mutated=true;t.url=argv[5];
      }else{
        mutated=true;
        w.tabs.push(app.Tab({url:argv[5]}));
      }
      const after=w.tabs();
      if(!t){
        const added=after.filter(tab=>before.indexOf(String(tab.id()))<0);
        if (added.length!==1) fail('identity','The new research tab could not be identified without touching existing tabs.');
        t=added[0];
      }
      w.activeTabIndex=after.findIndex(tab=>String(tab.id())===String(t.id()))+1;
      w.index=1;
      app.activate();
    } else {
      w=windows.find(window=>String(window.id())===argv[2]);
      if (!w || w.mode()!=='normal') fail('changed','The research window was closed or is no longer a normal window.');
      if (!windows.length || String(windows[0].id())!==argv[2]) fail('changed','The user changed browser windows; research stopped.');
      t=w.tabs().find(tab=>String(tab.id())===argv[3]);
      if (!t || String(w.activeTab().id())!==argv[3]) fail('changed','The research tab was closed, moved, or is no longer selected.');
      if (argv[4] && t.url()!==argv[4]) fail('changed','The research tab navigated outside its expected page.');
      if (mode==='navigate') {mutated=true;t.url=argv[5];}
    }
    const result={ok:true,windowId:String(w.id()),tabId:String(t.id()),url:t.url(),loading:t.loading()};
    if (mode==='snapshot') {
      try { result.snapshot=JSON.parse(t.execute({javascript:argv[6]})); }
      catch(error) { fail('javascript_unavailable','The browser did not allow page JavaScript through Apple Events. Buntini has not changed that setting.'); }
      if (t.url()!==result.url || String(w.activeTab().id())!==String(t.id()) || String(app.windows()[0].id())!==String(w.id())) fail('changed','The browser changed while the page was being read.');
    }
    return JSON.stringify(result);
  } catch(error) {
    const message=String(error.message || error);
    const code=error.buntiniCode || (error.number===-1743 || error.errorNumber===-1743 || message.indexOf('-1743')>=0 || message.indexOf('Not authorized')>=0 ?'automation_denied':'browser_error');
    return JSON.stringify({ok:false,code:code,message:message.slice(0,400),mutated:mutated});
  }
}"#;

const READ_DOM: &str = r#"JSON.stringify((()=>{
  if(document.readyState!=='complete'||!document.body)return {ready:false};
  function text(root,cap,nodes){
    const walker=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);const parts=[];let length=0,count=0,clipped=false;
    while(walker.nextNode()){
      if(++count>nodes){clipped=true;break;}
      const node=walker.currentNode,e=node.parentElement;
      if(!e||e.closest('script,style,noscript,form,input,textarea,[contenteditable],svg,nav,header,footer'))continue;
      const style=getComputedStyle(e);if(style.display==='none'||style.visibility==='hidden'||!e.getClientRects().length)continue;
      const raw=node.nodeValue||'';const value=raw.slice(0,cap).replace(/\s+/g,' ').trim();if(!value)continue;
      parts.push(value);length+=value.length+1;if(length>cap||raw.length>cap){clipped=true;break;}
    }
    return {value:parts.join('\n').slice(0,cap),clipped};
  }
  const body=text(document.querySelector('main,[role="main"]')||document.querySelector('article')||document.body,8000,12000);
  const links=[];let examined=0,clipped=false;
  for(const a of document.querySelectorAll('a[href]')){
    if(++examined>3000){clipped=true;break;}
    if(a.closest('nav,header,footer,form,[contenteditable]'))continue;
    const style=getComputedStyle(a);if(style.display==='none'||style.visibility==='hidden'||!a.getClientRects().length)continue;
    const heading=a.querySelector('h1,h2,h3,h4,[role="heading"]')||a.closest('h1,h2,h3,h4,[role="heading"]');
    if(!heading)continue;const url=a.href;const label=text(heading,300,300).value.replace(/\s+/g,' ');
    if(!label||!url||url.length>4096)continue;
    links.push({url,label});if(links.length>=64){clipped=true;break;}
  }
  return {ready:true,url:location.href,title:document.title.slice(0,300),text:body.value,links,truncated:body.clipped||clipped};
})())"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlReply {
    ok: bool,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    mutated: bool,
    #[serde(default)]
    window_id: String,
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    loading: bool,
    #[serde(default)]
    snapshot: Option<Snapshot>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    url: String,
    label: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    ready: bool,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    links: Vec<Link>,
    #[serde(default)]
    truncated: bool,
}

async fn control(
    mode: &str,
    target: &BrowserTarget,
    window: &str,
    tab: &str,
    expected: &str,
    destination: &str,
    cancel: Arc<AtomicBool>,
) -> Result<ControlReply, String> {
    let mut cmd = crate::actions::osascript_command("JavaScript", CONTROL);
    cmd.args([
        mode,
        &target.bundle_id,
        window,
        tab,
        expected,
        destination,
        if mode == "snapshot" { READ_DOM } else { "" },
    ]);
    // A cold browser launch or the initial macOS Automation prompt needs more
    // time than an already-owned tab operation. Cancellation still kills the
    // helper, and an unknown creation outcome is never retried automatically.
    let seconds = match mode {
        "preflight" => 8,
        "create" => 45,
        _ => 12,
    };
    let raw = command(cmd, cancel, seconds).await?;
    if raw.len() > 192_000 {
        return Err("The browser returned too much page data.".into());
    }
    serde_json::from_str(&raw)
        .map_err(|_| "The browser returned an invalid research-tab response.".into())
}

fn check_identity(session: &ResearchSession, reply: &ControlReply) -> Result<(), String> {
    if !reply.ok {
        return Err(match reply.code.as_str() {
            "automation_denied" => {
                "macOS did not allow browser Automation. Buntini has not changed that permission."
                    .into()
            }
            _ => reply.message.clone(),
        });
    }
    if reply.window_id != session.window_id || reply.tab_id != session.tab_id {
        return Err("The research tab identity changed; the page was not read.".into());
    }
    Ok(())
}

fn preflight_reason(
    reply: Result<ControlReply, String>,
    cancel: &AtomicBool,
) -> Result<Option<String>, BrowserError> {
    cancelled(cancel, false, false)?;
    Ok(match reply {
        Ok(reply) if reply.ok => None,
        Ok(reply) if reply.code == "automation_denied" => {
            Some("macOS did not allow browser Automation".into())
        }
        Ok(_) => Some("the browser could not confirm Automation availability".into()),
        Err(_) => Some("the browser did not answer the read-only Automation check".into()),
    })
}

fn same_url(left: &str, right: &str) -> bool {
    match (
        crate::research::validate_public_url(left),
        crate::research::validate_public_url(right),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
fn expected_page(expected: &str, actual: &str, query: Option<&str>) -> bool {
    if let Some(query) = query {
        let Ok(url) = crate::research::validate_public_url(actual) else {
            return false;
        };
        let values: Vec<_> = url.query_pairs().filter(|(key, _)| key == "q").collect();
        return url.host_str() == Some(SEARCH_HOST)
            && url.path() == "/search"
            && values.len() == 1
            && values[0].1 == query;
    }
    same_url(expected, actual)
}

fn validate_snapshot(snapshot: &Snapshot, url: &str) -> Result<(), String> {
    if !snapshot.ready {
        return Err("The browser page is still loading.".into());
    }
    if !same_url(&snapshot.url, url)
        || snapshot.url.len() > 4096
        || snapshot.title.chars().count() > 300
        || snapshot.text.chars().count() > MAX_TEXT
        || snapshot.links.len() > MAX_LINKS
        || snapshot
            .links
            .iter()
            .any(|link| link.url.len() > 4096 || link.label.chars().count() > 300)
    {
        return Err("The browser snapshot changed page or exceeded its evidence bounds.".into());
    }
    Ok(())
}

async fn read_tab(
    session: &mut ResearchSession,
    expected: &str,
    query: Option<&str>,
    cancel: Arc<AtomicBool>,
) -> Result<Snapshot, BrowserError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut did_read = false;
    loop {
        cancelled(&cancel, did_read, true)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(failure(
                "The browser did not provide a verified readable page within 25 seconds.",
                did_read,
                true,
            ));
        }
        let state = control(
            "inspect",
            &session.target,
            &session.window_id,
            &session.tab_id,
            "",
            "",
            cancel.clone(),
        )
        .await
        .map_err(|e| failure(e, did_read, true))?;
        check_identity(session, &state).map_err(|e| failure(e, did_read, true))?;
        if !expected_page(expected, &state.url, query) {
            if state.loading && (state.url == "about:blank" || state.url == session.last_url) {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            return Err(failure(
                "The research tab left the selected page; its contents were not read.",
                did_read,
                true,
            ));
        }
        if state.loading {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        did_read = true;
        let reply = control(
            "snapshot",
            &session.target,
            &session.window_id,
            &session.tab_id,
            &state.url,
            "",
            cancel.clone(),
        )
        .await
        .map_err(|e| failure(e, true, true))?;
        if reply.ok {
            check_identity(session, &reply).map_err(|e| failure(e, true, true))?;
            if let Some(snapshot) = reply.snapshot {
                if !snapshot.ready {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                validate_snapshot(&snapshot, &state.url).map_err(|e| failure(e, true, true))?;
                session.last_url = state.url;
                return Ok(snapshot);
            }
            return Err(failure(
                "The browser returned no page snapshot.",
                true,
                true,
            ));
        }
        if reply.code != "javascript_unavailable" {
            return Err(failure(reply.message, true, true));
        }
        // Accessibility is a read-only fallback. Never enable JavaScript from
        // Apple Events, alter browser preferences, or invoke a user's extension.
        let bundle = session.target.bundle_id.clone();
        let expected_url = state.url.clone();
        let ax=tokio::task::spawn_blocking(move||{
            if !crate::ax::trusted(false){return Err("Page scripting is disabled and Accessibility access is unavailable. Buntini has not changed browser settings.".into());}
            let pid=crate::ax::pid(&bundle).ok_or("The research browser closed.")?;
            crate::ax::research_page(pid,&expected_url,MAX_TEXT,MAX_LINKS)
        }).await.map_err(|_|failure("Browser accessibility read ended unexpectedly.",true,true))?
            .map_err(|e|failure(e,true,true))?;
        cancelled(&cancel, true, true)?;
        let after = control(
            "inspect",
            &session.target,
            &session.window_id,
            &session.tab_id,
            &state.url,
            "",
            cancel.clone(),
        )
        .await
        .map_err(|e| failure(e, true, true))?;
        check_identity(session, &after).map_err(|e| failure(e, true, true))?;
        let snapshot = Snapshot {
            ready: true,
            url: ax.url,
            title: ax.title.chars().take(300).collect(),
            text: ax.text,
            links: ax
                .links
                .into_iter()
                .filter(|link| query.is_none() || link.heading)
                .map(|l| Link {
                    url: l.url,
                    label: l.label.chars().take(300).collect(),
                })
                .collect(),
            truncated: ax.truncated,
        };
        validate_snapshot(&snapshot, &state.url).map_err(|e| failure(e, true, true))?;
        session.last_url = state.url;
        return Ok(snapshot);
    }
}

fn result_url(raw: &str) -> Option<String> {
    let url = crate::research::validate_public_url(raw).ok()?;
    let result = if url.host_str() == Some(SEARCH_HOST) && url.path() == "/url" {
        let targets: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| ["q", "url"].contains(&key.as_ref()))
            .collect();
        if targets.len() != 1 {
            return None;
        }
        crate::research::validate_public_url(&targets[0].1).ok()?
    } else {
        url
    };
    let host = result.host_str()?;
    if host == SEARCH_HOST
        || host == "accounts.google.com"
        || host == "support.google.com"
        || host == "policies.google.com"
    {
        return None;
    }
    Some(result.to_string())
}

fn sources(snapshot: &Snapshot) -> Result<Vec<Source>, String> {
    let lower = snapshot.text.to_lowercase();
    if [
        "unusual traffic from your computer network",
        "verify you are human",
        "verifying you're not a bot",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return Err("Google is asking for verification instead of providing readable search results. No challenge was submitted.".into());
    }
    let now = crate::types::now();
    let mut seen = HashSet::new();
    let result: Vec<_> = snapshot
        .links
        .iter()
        .filter_map(|link| {
            let url = result_url(&link.url)?;
            if link.label.trim().is_empty() || !seen.insert(url.clone()) {
                return None;
            }
            Some(Source {
                title: link.label.clone(),
                url,
                snippet: String::new(),
                retrieved_at: now.clone(),
            })
        })
        .take(MAX_SOURCES)
        .collect();
    if result.is_empty() {
        Err("The browser opened Google but returned no readable result links; opening a tab is not completed research.".into())
    } else {
        Ok(result)
    }
}

/// Online consent authorizes navigation. Reading additionally requires the
/// selected app's allowlist and Browser sharing permission, computed by root.
pub async fn search(
    target: &BrowserTarget,
    query: &str,
    allow_read: bool,
    cancel: Arc<AtomicBool>,
) -> Result<BrowserSearch, BrowserError> {
    let url = search_url(query).map_err(|e| failure(e, false, false))?;
    cancelled(&cancel, false, false)?;
    if !target.readable || !allow_read {
        open_only(target, url.as_str(), cancel).await?;
        return Err(failure(
            if !allow_read {
                format!("Opened Google in {}. Browser reading is not allowed for this request; continuing with the configured search API.",target.name)
            } else {
                format!("Opened Google in {}. This browser cannot expose a verified normal-window research tab; continuing with the configured search API.",target.name)
            },
            false,
            true,
        ));
    }
    let _lock = tokio::time::timeout(Duration::from_secs(2), OPERATIONS.lock())
        .await
        .map_err(|_| {
            failure(
                "Another browser research operation is running.",
                false,
                false,
            )
        })?;
    public_destination(url.as_str())
        .await
        .map_err(|e| failure(e, false, false))?;
    cancelled(&cancel, false, false)?;
    let preflight = control("preflight", target, "", "", "", "", cancel.clone()).await;
    if let Some(reason) = preflight_reason(preflight, &cancel)? {
        open_only(target, url.as_str(), cancel).await?;
        return Err(failure(format!("Opened Google in {}. Browser reading is unavailable because {reason}; continuing with the configured search API. Browser settings were not changed.",target.name),false,true));
    }
    let created = control("create", target, "", "", "", url.as_str(), cancel.clone())
        .await
        .map_err(|e| failure(e, false, true))?;
    if !created.ok {
        if created.code == "automation_denied" && !created.mutated {
            open_only(target, url.as_str(), cancel).await?;
            return Err(failure(format!("Opened Google in {}. macOS Automation permission prevented controlled browser reading; browser settings were not changed.",target.name),false,true));
        }
        return Err(failure(created.message, false, created.mutated));
    }
    if created.window_id.is_empty()
        || created.tab_id.is_empty()
        || created.window_id.len() > 100
        || created.tab_id.len() > 100
    {
        return Err(failure(
            "The new research tab did not return stable identity.",
            false,
            true,
        ));
    }
    let mut session = ResearchSession {
        target: target.clone(),
        window_id: created.window_id,
        tab_id: created.tab_id,
        last_url: created.url,
        usable: true,
    };
    let snapshot = read_tab(&mut session, url.as_str(), Some(query), cancel).await?;
    let found = sources(&snapshot).map_err(|e| failure(e, true, true))?;
    Ok(BrowserSearch {
        query: query.into(),
        search_url: snapshot.url,
        retrieved_at: crate::types::now(),
        sources: found,
        truncated: snapshot.truncated || snapshot.links.len() > MAX_SOURCES,
        session,
    })
}

pub async fn read_selected(
    session: &mut ResearchSession,
    selected_urls: &[String],
    url: &str,
    cancel: Arc<AtomicBool>,
) -> Result<ResearchContent, BrowserError> {
    if !session.usable {
        return Err(failure(
            "The browser research session stopped; its tab will not be reused.",
            false,
            false,
        ));
    }
    if !selected_urls.iter().any(|selected| selected == url) {
        return Err(failure(
            "This page is not one of the selected search results.",
            false,
            false,
        ));
    }
    cancelled(&cancel, false, false)?;
    let _lock = tokio::time::timeout(Duration::from_secs(2), OPERATIONS.lock())
        .await
        .map_err(|_| {
            failure(
                "Another browser research operation is running.",
                false,
                false,
            )
        })?;
    public_destination(url)
        .await
        .map_err(|e| failure(e, false, false))?;
    cancelled(&cancel, false, false)?;
    let navigated = control(
        "navigate",
        &session.target,
        &session.window_id,
        &session.tab_id,
        &session.last_url,
        url,
        cancel.clone(),
    )
    .await
    .map_err(|e| failure(e, false, true));
    let result = async {
        let navigated = navigated?;
        check_identity(session, &navigated).map_err(|e| failure(e, false, true))?;
        let snapshot = read_tab(session, url, None, cancel).await?;
        if snapshot.text.trim().is_empty() {
            return Err(failure(
                "The selected browser page contained no readable text.",
                true,
                true,
            ));
        }
        Ok(ResearchContent {
            url: snapshot.url,
            title: snapshot.title,
            text: snapshot.text,
            retrieved_at: crate::types::now(),
            truncated: snapshot.truncated,
        })
    }
    .await;
    if result.is_err() {
        session.usable = false;
    }
    result
}

/// Opt-in developer smoke: only a fixed public Google query and an observed
/// official Rust documentation result, using the same transport as production.
#[cfg(debug_assertions)]
pub fn run_public_smoke() {
    let report = match selected_browser() {
        Err(error) => serde_json::json!({"stage":"selection","error":error}),
        Ok(target) => tokio::runtime::Runtime::new().unwrap().block_on(async {
            let cancel=Arc::new(AtomicBool::new(false));
            let query="Rust programming language official documentation";
            let mut found=match search(&target,query,true,cancel.clone()).await {
                Ok(found)=>found,
                Err(error)=>return serde_json::json!({"stage":"search","browser":target,"query":query,"error":error}),
            };
            let selected=found.sources.iter().find(|source|reqwest::Url::parse(&source.url).ok().and_then(|url|url.host_str().map(str::to_owned)).as_deref()==Some("doc.rust-lang.org")).map(|source|source.url.clone());
            let Some(url)=selected else{return serde_json::json!({"stage":"selection","browser":target,"query":query,"sources":found.sources,"error":"No observed official Rust documentation result."});};
            let selected_urls=found.sources.iter().map(|source|source.url.clone()).collect::<Vec<_>>();
            match read_selected(&mut found.session,&selected_urls,&url,cancel).await {
                Ok(page)=>serde_json::json!({"stage":"complete","browser":target,"query":query,"searchUrl":found.search_url,"sources":found.sources,"page":{"url":page.url,"title":page.title,"characters":page.text.chars().count(),"excerpt":page.text.chars().take(300).collect::<String>(),"truncated":page.truncated}}),
                Err(error)=>serde_json::json!({"stage":"read","browser":target,"query":query,"sources":found.sources,"error":error}),
            }
        }),
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonmutating_preflight_failure_can_open_once_but_cancellation_is_terminal() {
        let cancel = AtomicBool::new(false);
        assert!(preflight_reason(Err("helper timed out".into()), &cancel)
            .unwrap()
            .is_some());
        let denied =
            serde_json::from_value(json!({"ok":false,"code":"automation_denied","mutated":false}))
                .unwrap();
        assert!(preflight_reason(Ok(denied), &cancel).unwrap().is_some());
        let available = serde_json::from_value(json!({"ok":true})).unwrap();
        assert!(preflight_reason(Ok(available), &cancel).unwrap().is_none());
        cancel.store(true, Ordering::Release);
        let error = preflight_reason(Err("helper timed out".into()), &cancel).unwrap_err();
        assert_eq!(error.message, "Task cancelled.");
        assert!(!error.navigated && !error.did_read);
    }
    #[test]
    fn query_is_encoded_without_becoming_script_or_extra_parameters() {
        let query = "museum hours & q=other ' \"";
        let url = search_url(query).unwrap();
        assert_eq!(url.host_str(), Some(SEARCH_HOST));
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![("q".into(), query.into())]
        );
        assert!(search_url("bad\nquery").is_err());
        assert!(search_url(&"x".repeat(2001)).is_err());
        // The limit is in characters, as the lookup's validation and approval count them.
        assert!(search_url(&"検索".repeat(450)).is_ok());
    }
    #[test]
    fn stable_identity_and_expected_selected_url_are_required() {
        let session = ResearchSession {
            target: target("com.google.Chrome", "Chrome"),
            window_id: "11".into(),
            tab_id: "22".into(),
            last_url: "https://museum.org/".into(),
            usable: true,
        };
        let mut reply:ControlReply=serde_json::from_value(json!({"ok":true,"windowId":"11","tabId":"22","url":"https://museum.org/","loading":false})).unwrap();
        assert!(check_identity(&session, &reply).is_ok());
        reply.tab_id = "23".into();
        assert!(check_identity(&session, &reply).is_err());
        assert!(expected_page(
            "https://museum.org/",
            "https://museum.org/#hours",
            None
        ));
        assert!(!expected_page(
            "https://museum.org/",
            "https://museum.org/private",
            None
        ));
        assert!(expected_page(
            "",
            "https://www.google.com/search?q=museum&source=browser",
            Some("museum")
        ));
        assert!(!expected_page(
            "",
            "https://www.google.com/search?q=museum&q=other",
            Some("museum")
        ));
    }
    #[test]
    fn source_links_must_be_observed_public_urls() {
        assert_eq!(
            result_url("https://www.google.com/url?q=https%3A%2F%2Fmuseum.org%2Fvisit"),
            Some("https://museum.org/visit".into())
        );
        for bad in [
            "https://127.0.0.1/private",
            "https://office.local/",
            "javascript:alert(1)",
            "https://www.google.com/url?q=https%3A%2F%2F127.0.0.1%2F",
            "https://www.google.com/url?q=https%3A%2F%2Fmuseum.org&url=https%3A%2F%2Fother.org",
        ] {
            assert!(result_url(bad).is_none(), "{bad}");
        }
        let snapshot = Snapshot {
            ready: true,
            url: "https://www.google.com/search?q=museum".into(),
            title: "Search".into(),
            text: "Results".into(),
            links: vec![],
            truncated: false,
        };
        assert!(sources(&snapshot).is_err());
    }
    #[test]
    fn unsupported_browser_capabilities_are_explicit() {
        assert!(target("com.google.Chrome", "Chrome").readable);
        assert!(!target("com.apple.Safari", "Safari").readable);
        assert!(!target("company.thebrowser.Browser", "Arc").readable);
        assert!(CONTROL.contains("w.tabs.push(app.Tab"));
        assert!(!CONTROL.contains(".close("));
        assert!(!CONTROL.contains("preferences"));
        assert!(CONTROL.contains("w.mode()!=='normal'"));
    }
    #[tokio::test]
    async fn cancellation_and_unselected_sources_do_not_touch_browsers() {
        let mut session = ResearchSession {
            target: target("com.google.Chrome", "Chrome"),
            window_id: "11".into(),
            tab_id: "22".into(),
            last_url: "https://museum.org/".into(),
            usable: true,
        };
        let cancelled = Arc::new(AtomicBool::new(true));
        let result = search(&session.target, "museum", true, cancelled.clone())
            .await
            .unwrap_err();
        assert!(!result.navigated && !result.did_read);
        let result = read_selected(&mut session, &[], "https://museum.org/", cancelled)
            .await
            .unwrap_err();
        assert!(!result.navigated && !result.did_read);
    }
}
