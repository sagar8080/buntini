//! A call in another app, beside the notch: "Zoom · 12 min" while it goes on, with the
//! microphone's mute and the call's notes on its card. macOS says which apps are using the
//! microphone and the speakers (Core Audio's process list, what the menu bar's orange dot
//! comes from) to any app, without a permission and without asking the apps anything. Only
//! apps made for calls count, and a browser only while it both listens and plays, as a meeting
//! does; Siri listening for its name, dictation and Buntini's own microphone never do.
//!
//! Mute is the microphone's own, so it holds whatever app the call is in. Buntini lifts a mute
//! it set when the call ends, when it quits, and on its next start if it couldn't before.
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::i18n::{t, tf};
use crate::surface::cards::island::Call as CallCard;
use crate::surface::{Card, CardBody, Offer, Pill, PillState, Source, Urgency};

pub const CALL: &str = "call";
/// The pill while the microphone is muted, so it shows a crossed-out microphone.
const MUTED_PILL: &str = "call.muted";
const TINT: &str = "#30D158";
/// How often who uses the microphone is read.
const EVERY: Duration = Duration::from_secs(3);
/// A call whose app neither listens nor plays for this long is over.
const QUIET: f64 = 10.0;
/// An app that stopped listening this long ago, though it still plays, is no longer in a call.
const DEAF: f64 = 30.0 * 60.0;
/// A browser that listens and plays shows as a call only once it has for this long: a word
/// said to a page while music plays isn't a meeting.
const SETTLE: f64 = 6.0;

/// Apps made for calls, by the start of their bundle id, lower case.
const CALL_APPS: [&str; 19] = [
    "us.zoom.",
    "com.microsoft.teams",
    "com.apple.facetime",
    "com.cisco.webex",
    "cisco-systems.spark",
    "com.webex.",
    "com.tinyspeck.slackmacgap",
    "com.hnc.discord",
    "net.whatsapp.",
    "ru.keepcoder.telegram",
    "org.telegram.",
    "org.whispersystems.signal-desktop",
    "com.skype.",
    "com.ringcentral.",
    "com.logmein.gotomeeting",
    "com.bluejeans.",
    "com.around.",
    "app.tuple.",
    "com.amazon.chime",
];

/// Browsers, where Meet, Teams and Zoom run as pages.
const BROWSERS: [&str; 9] = [
    "com.google.chrome",
    "com.apple.safari",
    "com.brave.browser",
    "com.microsoft.edgemac",
    "company.thebrowser.",
    "org.mozilla.",
    "com.operasoftware.",
    "com.vivaldi.",
    "app.zen-browser.",
];

/// An app using the microphone or the speakers, as Core Audio says.
#[derive(Debug, Clone, PartialEq)]
pub struct Use {
    pub bundle: String,
    pub pid: i32,
    pub input: bool,
    pub output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Call,
    Browser,
}

/// The call going on. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Ongoing {
    /// The app's bundle id, its helpers folded into it.
    pub app: String,
    pub name: String,
    pub since: f64,
    /// When it last listened or played.
    pub heard: f64,
    /// When it last listened.
    pub listened: f64,
}

/// The app a process plays for: a browser's or Electron app's helper for the app, WebKit's
/// for Safari, and FaceTime's call service for FaceTime.
pub fn owner(bundle: &str) -> String {
    let bundle = bundle.trim();
    let lower = bundle.to_ascii_lowercase();
    if lower == "com.apple.avconferenced" {
        return "com.apple.FaceTime".into();
    }
    if lower.starts_with("com.apple.webkit.") {
        return "com.apple.Safari".into();
    }
    match lower.find(".helper") {
        Some(at) => bundle[..at].to_string(),
        None => bundle.to_string(),
    }
}

pub fn kind(app: &str) -> Option<Kind> {
    let lower = app.to_ascii_lowercase();
    if CALL_APPS.iter().any(|prefix| lower.starts_with(prefix)) {
        Some(Kind::Call)
    } else if BROWSERS.iter().any(|prefix| lower.starts_with(prefix)) {
        Some(Kind::Browser)
    } else {
        None
    }
}

/// The call after a read at `now`: the one going on while its app still listens or plays,
/// else a new one in an app that started listening, a call app before a browser.
pub fn step(current: Option<Ongoing>, uses: &[Use], now: f64, name_of: impl Fn(&str, i32) -> String) -> Option<Ongoing> {
    // Each app's processes together: Chrome listens in one helper and plays in another.
    let mut apps: Vec<(String, bool, bool, i32)> = Vec::new();
    for found in uses {
        let app = owner(&found.bundle);
        if kind(&app).is_none() {
            continue;
        }
        match apps.iter_mut().find(|(known, ..)| known.eq_ignore_ascii_case(&app)) {
            Some((_, input, output, pid)) => {
                *input |= found.input;
                *output |= found.output;
                if found.input {
                    *pid = found.pid;
                }
            }
            None => apps.push((app, found.input, found.output, found.pid)),
        }
    }
    if let Some(mut call) = current {
        if let Some((_, input, output, _)) = apps.iter().find(|(app, ..)| app.eq_ignore_ascii_case(&call.app)) {
            if *input {
                call.listened = now;
            }
            if *input || *output {
                call.heard = now;
            }
        }
        if now - call.heard <= QUIET && now - call.listened <= DEAF {
            return Some(call);
        }
    }
    apps.into_iter()
        .filter(|(app, input, output, _)| *input && (kind(app) == Some(Kind::Call) || *output))
        .min_by_key(|(app, ..)| (kind(app) == Some(Kind::Browser), app.to_ascii_lowercase()))
        .map(|(app, _, _, pid)| Ongoing { name: name_of(&app, pid), app, since: now, heard: now, listened: now })
}

/// The pill and card for `call` at `now`: nothing yet for a browser that only just started.
pub fn view(call: &Ongoing, muted: Option<bool>, recording: Option<bool>, now: f64) -> Option<(Pill, Card)> {
    if kind(&call.app) == Some(Kind::Browser) && now - call.since < SETTLE {
        return None;
    }
    let minutes = ((now - call.since).max(0.0) / 60.0).floor() as i64;
    let badge = match (muted, minutes) {
        (Some(true), _) => t("surface.call.muted"),
        (_, 0) => t("utilities.focus.underMinute"),
        (_, minutes) => tf("utilities.focus.minutes", &[("minutes", &minutes)]),
    };
    let pill = Pill {
        id: if muted == Some(true) { MUTED_PILL } else { CALL }.into(),
        source: Source::Island,
        label: call.name.clone(),
        tint: TINT.into(),
        state: PillState::Working,
        badge: Some(badge),
        progress: None,
        card: (muted == Some(true)).then(|| CALL.to_string()),
    };
    let card = Card {
        id: CALL.into(),
        source: Source::Island,
        body: CardBody::Call(CallCard { name: call.name.clone(), since: call.since.floor() as i64, muted, recording }),
    };
    Some((pill, card))
}

#[derive(Default)]
struct State {
    app: Option<AppHandle>,
    call: Option<Ongoing>,
    /// The microphone's mute as last read: `None` when it can't be muted from here.
    muted: Option<bool>,
    /// The microphones Buntini muted, by their UID, so the mute can be lifted.
    muted_here: Vec<String>,
    /// A call is being recorded for notes (flow).
    recording: bool,
    /// "Record calls" is on in Settings.
    record_calls: bool,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
/// Where the microphones Buntini muted are written down, in case it quits without lifting it.
static MARKER: OnceLock<PathBuf> = OnceLock::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_secs_f64()).unwrap_or(0.0)
}

/// Lifts a mute left from before, then reads who uses the microphone every few seconds.
pub fn start(app: &AppHandle) {
    if let Some(path) = app.path().app_cache_dir().ok().map(|dir| dir.join("muted-microphones.json")) {
        let left: Vec<String> = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        for uid in &left {
            if let Some(device) = audio::device(uid) {
                audio::set_muted(device, false);
            }
        }
        let _ = std::fs::remove_file(&path);
        let _ = MARKER.set(path);
    }
    state().app = Some(app.clone());
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(EVERY).await;
            let uses = tokio::task::spawn_blocking(audio::uses).await.unwrap_or_default();
            if update(&uses) {
                super::refresh();
            }
        }
    });
}

/// Follows a read. True when what shows changed.
fn update(uses: &[Use]) -> bool {
    let mut state = state();
    let before = (state.call.as_ref().map(|call| (call.app.clone(), call.since)), state.muted, state.record_calls);
    state.call = step(state.call.take(), uses, now(), app_name);
    if state.call.is_some() {
        state.record_calls = state.app.as_ref().and_then(super::settings).is_some_and(|settings| settings.flow.record_calls);
        let device = audio::default_input();
        // A microphone plugged in while muted here is muted too.
        if let Some(device) = device.filter(|_| !state.muted_here.is_empty()) {
            if let Some(uid) = audio::uid(device).filter(|uid| !state.muted_here.contains(uid)) {
                if audio::muted(device) == Some(false) && audio::set_muted(device, true) {
                    state.muted_here.push(uid);
                    write_down(&state.muted_here);
                }
            }
        }
        state.muted = device.and_then(audio::muted);
    } else {
        lift_all(&mut state);
        state.muted = None;
    }
    before != (state.call.as_ref().map(|call| (call.app.clone(), call.since)), state.muted, state.record_calls)
}

/// The call's pill and card, while there's one.
pub fn shown() -> (Vec<Pill>, Vec<Offer>) {
    let state = state();
    let Some((pill, card)) = state.call.as_ref().and_then(|call| view(call, state.muted, state.record_calls.then_some(state.recording), now())) else {
        return (Vec::new(), Vec::new());
    };
    (vec![pill], vec![Offer { card, urgency: Urgency::Info, at_ms: 0, ttl_ms: None }])
}

/// The card's Mute and Unmute: the microphone's own mute.
pub fn toggle_mute() -> Result<(), String> {
    let device = audio::default_input().ok_or("There's no microphone.")?;
    let muted = audio::muted(device).ok_or("This microphone can't be muted from here.")?;
    if !audio::set_muted(device, !muted) {
        return Err("The microphone didn't take it.".into());
    }
    let mut state = state();
    if let Some(uid) = audio::uid(device) {
        state.muted_here.retain(|known| *known != uid);
        if !muted {
            state.muted_here.push(uid);
        }
        write_down(&state.muted_here);
    }
    state.muted = Some(!muted);
    drop(state);
    super::refresh();
    Ok(())
}

/// The call's app, in front.
pub fn open() {
    if let Some(app) = state().call.as_ref().map(|call| call.app.clone()) {
        let _ = std::process::Command::new("/usr/bin/open").args(["-b", &app]).spawn();
    }
}

/// Flow started or stopped recording a call.
pub fn set_recording(recording: bool) {
    let mut state = state();
    if state.recording != recording {
        state.recording = recording;
        let shown = state.call.is_some();
        drop(state);
        if shown {
            super::refresh();
        }
    }
}

/// Lifts every mute Buntini set, as it quits.
pub fn lift() {
    lift_all(&mut state());
}

fn lift_all(state: &mut State) {
    if state.muted_here.is_empty() {
        return;
    }
    for uid in state.muted_here.drain(..) {
        if let Some(device) = audio::device(&uid) {
            audio::set_muted(device, false);
        }
    }
    write_down(&[]);
}

fn write_down(uids: &[String]) {
    let Some(path) = MARKER.get() else { return };
    if uids.is_empty() {
        let _ = std::fs::remove_file(path);
    } else if let Ok(bytes) = serde_json::to_vec(uids) {
        let _ = std::fs::write(path, bytes);
    }
}

/// The app's name as macOS shows it: the running app with that bundle id, else the process.
fn app_name(app: &str, pid: i32) -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSRunningApplication;
        use objc2_foundation::NSString;
        let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(app));
        let name = running
            .firstObject()
            .and_then(|found| found.localizedName())
            .or_else(|| NSRunningApplication::runningApplicationWithProcessIdentifier(pid).and_then(|found| found.localizedName()));
        if let Some(name) = name.map(|name| name.to_string()).filter(|name| !name.trim().is_empty()) {
            return name;
        }
    }
    let _ = pid;
    app.rsplit('.').next().unwrap_or(app).to_string()
}

/// Core Audio, read and set from Buntini's own process.
#[cfg(target_os = "macos")]
mod audio {
    use std::ffi::c_void;
    use std::ptr::null;

    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    use super::Use;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn code(text: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*text)
    }

    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = code(b"glob");
    const INPUT: u32 = code(b"inpt");
    const MAIN: u32 = 0;
    const PROCESSES: u32 = code(b"prs#");
    const PID: u32 = code(b"ppid");
    const BUNDLE: u32 = code(b"pbid");
    const LISTENS: u32 = code(b"piri");
    const PLAYS: u32 = code(b"piro");
    const DEFAULT_INPUT: u32 = code(b"dIn ");
    const MUTE: u32 = code(b"mute");
    const UID: u32 = code(b"uid ");
    const FROM_UID: u32 = code(b"uidd");
    /// More processes than this isn't a list Core Audio gave.
    const MOST: usize = 4096;

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectHasProperty(object: u32, address: *const Address) -> u8;
        fn AudioObjectIsPropertySettable(object: u32, address: *const Address, settable: *mut u8) -> i32;
        fn AudioObjectGetPropertyDataSize(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
        fn AudioObjectSetPropertyData(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: u32, data: *const c_void) -> i32;
    }

    fn get<T: Copy + Default>(object: u32, selector: u32, scope: u32) -> Option<T> {
        let address = Address { selector, scope, element: MAIN };
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        // SAFETY: `value` has room for `size` bytes, which Core Audio never writes past.
        let status = unsafe { AudioObjectGetPropertyData(object, &address, 0, null(), &mut size, &mut value as *mut T as *mut c_void) };
        (status == 0 && size as usize == std::mem::size_of::<T>()).then_some(value)
    }

    fn string(object: u32, selector: u32) -> Option<String> {
        let address = Address { selector, scope: GLOBAL, element: MAIN };
        let mut value: *mut NSString = std::ptr::null_mut();
        let mut size = std::mem::size_of::<*mut NSString>() as u32;
        // SAFETY: Core Audio writes one CFString reference, which the caller then owns.
        let status = unsafe { AudioObjectGetPropertyData(object, &address, 0, null(), &mut size, &mut value as *mut *mut NSString as *mut c_void) };
        if status != 0 || value.is_null() {
            return None;
        }
        // SAFETY: a CFString is an NSString; taking it over releases it once dropped.
        let owned = unsafe { Retained::from_raw(value) }?;
        Some(owned.to_string())
    }

    pub fn uses() -> Vec<Use> {
        let address = Address { selector: PROCESSES, scope: GLOBAL, element: MAIN };
        let mut size = 0u32;
        // SAFETY: only the size is written.
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &address, 0, null(), &mut size) } != 0 {
            return Vec::new();
        }
        let count = size as usize / std::mem::size_of::<u32>();
        if count == 0 || count > MOST {
            return Vec::new();
        }
        let mut ids = vec![0u32; count];
        // SAFETY: `ids` has room for `size` bytes.
        if unsafe { AudioObjectGetPropertyData(SYSTEM, &address, 0, null(), &mut size, ids.as_mut_ptr() as *mut c_void) } != 0 {
            return Vec::new();
        }
        ids.truncate(size as usize / std::mem::size_of::<u32>());
        ids.into_iter()
            .filter_map(|id| {
                let input = get::<u32>(id, LISTENS, GLOBAL).unwrap_or(0) != 0;
                let output = get::<u32>(id, PLAYS, GLOBAL).unwrap_or(0) != 0;
                (input || output).then(|| Use { bundle: string(id, BUNDLE).unwrap_or_default(), pid: get::<i32>(id, PID, GLOBAL).unwrap_or(0), input, output })
            })
            .collect()
    }

    pub fn default_input() -> Option<u32> {
        get::<u32>(SYSTEM, DEFAULT_INPUT, GLOBAL).filter(|device| *device != 0)
    }

    /// Whether `device` is muted, when its own mute can be set.
    pub fn muted(device: u32) -> Option<bool> {
        let address = Address { selector: MUTE, scope: INPUT, element: MAIN };
        // SAFETY: plain reads of the address.
        if unsafe { AudioObjectHasProperty(device, &address) } == 0 {
            return None;
        }
        let mut settable = 0u8;
        // SAFETY: only `settable` is written.
        if unsafe { AudioObjectIsPropertySettable(device, &address, &mut settable) } != 0 || settable == 0 {
            return None;
        }
        get::<u32>(device, MUTE, INPUT).map(|value| value != 0)
    }

    pub fn set_muted(device: u32, muted: bool) -> bool {
        let address = Address { selector: MUTE, scope: INPUT, element: MAIN };
        let value = u32::from(muted);
        // SAFETY: Core Audio reads the four bytes of `value`.
        unsafe { AudioObjectSetPropertyData(device, &address, 0, null(), 4, &value as *const u32 as *const c_void) == 0 }
    }

    pub fn uid(device: u32) -> Option<String> {
        string(device, UID)
    }

    /// The device with `uid`, if it's connected.
    pub fn device(uid: &str) -> Option<u32> {
        let address = Address { selector: FROM_UID, scope: GLOBAL, element: MAIN };
        let text = NSString::from_str(uid);
        let reference: *const NSString = Retained::as_ptr(&text);
        let mut device = 0u32;
        let mut size = 4u32;
        // SAFETY: the qualifier is one CFString reference, kept alive by `text`.
        let status = unsafe {
            AudioObjectGetPropertyData(SYSTEM, &address, std::mem::size_of::<*const NSString>() as u32, &reference as *const *const NSString as *const c_void, &mut size, &mut device as *mut u32 as *mut c_void)
        };
        (status == 0 && device != 0).then_some(device)
    }
}

#[cfg(not(target_os = "macos"))]
mod audio {
    use super::Use;
    pub fn uses() -> Vec<Use> {
        Vec::new()
    }
    pub fn default_input() -> Option<u32> {
        None
    }
    pub fn muted(_: u32) -> Option<bool> {
        None
    }
    pub fn set_muted(_: u32, _: bool) -> bool {
        false
    }
    pub fn uid(_: u32) -> Option<String> {
        None
    }
    pub fn device(_: &str) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn using(bundle: &str, input: bool, output: bool) -> Use {
        Use { bundle: bundle.into(), pid: 7, input, output }
    }

    fn named(app: &str, _: i32) -> String {
        app.rsplit('.').next().unwrap_or(app).to_string()
    }

    #[test]
    fn helpers_fold_into_their_app() {
        assert_eq!(owner("com.google.Chrome.helper"), "com.google.Chrome");
        assert_eq!(owner("com.google.Chrome.helper.renderer"), "com.google.Chrome");
        assert_eq!(owner("com.hnc.Discord.helper"), "com.hnc.Discord");
        assert_eq!(owner("com.apple.WebKit.GPU"), "com.apple.Safari");
        assert_eq!(owner("com.apple.avconferenced"), "com.apple.FaceTime");
        assert_eq!(owner("us.zoom.xos"), "us.zoom.xos");
    }

    #[test]
    fn only_call_apps_and_browsers_can_be_in_a_call() {
        for app in ["us.zoom.xos", "com.microsoft.teams2", "com.apple.FaceTime", "com.hnc.Discord", "net.whatsapp.WhatsApp", "com.tinyspeck.slackmacgap"] {
            assert_eq!(kind(app), Some(Kind::Call), "{app}");
        }
        for app in ["com.google.Chrome", "com.apple.Safari", "company.thebrowser.Browser", "org.mozilla.firefox"] {
            assert_eq!(kind(app), Some(Kind::Browser), "{app}");
        }
        // Siri listening for its name, dictation, Buntini, recorders and music: never.
        for app in ["com.apple.CoreSpeech", "com.apple.assistantd", "com.apple.SpeechRecognitionCore", "app.buntini.companion", "com.apple.VoiceMemos", "com.apple.QuickTimePlayerX", "com.spotify.client", ""] {
            assert_eq!(kind(app), None, "{app}");
        }
    }

    #[test]
    fn a_call_starts_when_a_call_app_listens_and_ends_once_it_goes_quiet() {
        let zoom = [using("us.zoom.xos", true, true), using("com.apple.CoreSpeech", true, false)];
        let call = step(None, &zoom, 100.0, named).unwrap();
        assert_eq!((call.app.as_str(), call.name.as_str(), call.since), ("us.zoom.xos", "xos", 100.0));
        // Muted in Zoom itself, it only plays: still in the call.
        let later = step(Some(call.clone()), &[using("us.zoom.xos", false, true)], 400.0, named).unwrap();
        assert_eq!((later.since, later.heard, later.listened), (100.0, 400.0, 100.0));
        // A moment of silence when a device changes isn't the end.
        assert!(step(Some(later.clone()), &[], 405.0, named).is_some());
        assert!(step(Some(later.clone()), &[], 411.0, named).is_none());
        // Playing for half an hour without listening isn't a call any more.
        assert!(step(Some(later), &[using("us.zoom.xos", false, true)], 100.0 + DEAF + 1.0, named).is_none());
        // Nothing listening, nothing started.
        assert!(step(None, &[using("us.zoom.xos", false, true), using("com.spotify.client", false, true)], 0.0, named).is_none());
    }

    #[test]
    fn a_browser_is_in_a_call_only_while_it_listens_and_plays() {
        assert!(step(None, &[using("com.google.Chrome.helper", true, false)], 0.0, named).is_none());
        // Chrome listens in one helper and plays in another.
        let meet = [using("com.google.Chrome.helper", true, false), using("com.google.Chrome.helper.renderer", false, true)];
        let call = step(None, &meet, 0.0, named).unwrap();
        assert_eq!(call.app, "com.google.Chrome");
        // A call app that listens comes first.
        let both = [using("com.google.Chrome.helper", true, true), using("com.microsoft.teams2", true, true)];
        assert_eq!(step(None, &both, 0.0, named).unwrap().app, "com.microsoft.teams2");
        // The call going on keeps its place when another starts.
        let ongoing = step(Some(call.clone()), &both, 5.0, named).unwrap();
        assert_eq!((ongoing.app.as_str(), ongoing.since), ("com.google.Chrome", 0.0));
    }

    #[test]
    fn the_pill_says_how_long_or_muted_and_a_browser_waits_a_moment() {
        let call = Ongoing { app: "us.zoom.xos".into(), name: "Zoom".into(), since: 1_000.0, heard: 1_000.0, listened: 1_000.0 };
        let (pill, card) = view(&call, Some(false), None, 1_000.0 + 12.5 * 60.0).unwrap();
        assert_eq!((pill.id.as_str(), pill.label.as_str(), pill.card.as_deref()), (CALL, "Zoom", None));
        assert_eq!(pill.badge, Some(tf("utilities.focus.minutes", &[("minutes", &12)])));
        let CardBody::Call(body) = card.body else { panic!() };
        assert_eq!((body.since, body.muted, body.recording), (1_000, Some(false), None));
        let (pill, _) = view(&call, Some(true), Some(true), 1_030.0).unwrap();
        assert_eq!((pill.id.as_str(), pill.card.as_deref(), pill.badge), (MUTED_PILL, Some(CALL), Some(t("surface.call.muted"))));
        let (pill, _) = view(&call, None, None, 1_030.0).unwrap();
        assert_eq!(pill.badge, Some(t("utilities.focus.underMinute")));
        let meet = Ongoing { app: "com.google.Chrome".into(), ..call };
        assert!(view(&meet, None, None, 1_003.0).is_none());
        assert!(view(&meet, None, None, 1_000.0 + SETTLE).is_some());
    }

    /// Who uses the microphone and the speakers on this Mac now, read the way the island
    /// reads it: `cargo test --lib island::calls::tests::live -- --ignored --nocapture`. Reads
    /// for `BUNTINI_LIVE_SECONDS` (10 unless set), printing each change and the call it makes.
    /// With `BUNTINI_LIVE_MUTE=1`, and only while no app listens, it first mutes the
    /// microphone, reads it back and puts it back as it was.
    #[test]
    #[ignore]
    fn live() {
        if std::env::var("BUNTINI_LIVE_MUTE").is_ok_and(|value| value == "1") {
            assert!(audio::uses().iter().all(|found| !found.input), "something is listening; the microphone is left alone");
            let device = audio::default_input().expect("a microphone");
            let before = audio::muted(device).expect("a mute that can be set");
            // Core Audio applies a change a moment after it's taken.
            let settle = || std::thread::sleep(Duration::from_millis(250));
            assert!(audio::set_muted(device, !before));
            settle();
            let changed = audio::muted(device);
            assert!(audio::set_muted(device, before));
            settle();
            assert_eq!((changed, audio::muted(device)), (Some(!before), Some(before)));
            let uid = audio::uid(device).expect("a uid");
            assert_eq!(audio::device(&uid), Some(device));
            println!("mute: {before} → {} → {before} on {uid}", !before);
        }
        let seconds: f64 = std::env::var("BUNTINI_LIVE_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(10.0);
        let start = now();
        let mut call: Option<Ongoing> = None;
        let mut last = String::new();
        while now() - start < seconds {
            let uses = audio::uses();
            call = step(call, &uses, now(), app_name);
            let mic = audio::default_input().map(|device| (device, audio::uid(device), audio::muted(device)));
            let line = format!("{uses:?} → {:?} · mic {mic:?}", call.as_ref().map(|call| (&call.name, &call.app)));
            if line != last {
                println!("{:5.1}s {line}", now() - start);
                last = line;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
