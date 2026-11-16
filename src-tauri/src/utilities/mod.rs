//! Focus timer, screenshots and captures, and shortcuts you can change (stream F). Settings
//! search lives in the UI (src/utilities).
//!
//! - `focus`: the cycle of focus and rest, anchored to deadlines.
//! - `capture`: screenshots to the clipboard, areas to the island, mark-up and Ask.
//! - `shortcuts`: what each shortcut does and which combinations can't be used.
//! - `native`: the hot keys and pictures, in Buntini's own process.
//!
//! The shortcuts, the focus timer and muting change outside the settings sheet, so they're
//! kept as preferences of their own rather than in `settings.utilities`.
pub mod capture;
pub mod focus;
mod native;
pub mod shortcuts;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::surface::cards::utilities::{TimerCard, Vitals};
use crate::surface::{self, Action, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency};
use focus::{Phase, Plan, Session, Step};
use shortcuts::{Overrides, Shortcut};

/// Saved under `settings.utilities`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

const SHORTCUTS: &str = "utilities.shortcuts";
const FOCUS: &str = "utilities.focus";
const MUTED: &str = "utilities.muted";
pub(crate) const ASKED_RECORDING: &str = "utilities.askedRecording";
/// The macOS alert for the end of the current phase; one at a time.
const FOCUS_ALERT: &str = "buntini-focus";
const FOCUS_ID: &str = "utilities.focus";
const NOTICE_ID: &str = "utilities.notice";
/// How long a phase change shows on the island.
const TURN_MS: u64 = 6_000;
const NOTICE_MS: u64 = 1_400;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
    /// Every action and its shortcut.
    Shortcuts,
    /// A new shortcut for an action; none turns it off. Answers the list, or why not.
    #[serde(rename_all = "camelCase")]
    SetShortcut { id: String, shortcut: Option<Shortcut> },
    ResetShortcuts,
    /// While a shortcut is being recorded, Buntini's own don't fire.
    Recording { on: bool },
    /// Starts a focus set, from the desktop cat's offer.
    Focus { minutes: Option<u64> },
}

#[tauri::command]
pub async fn utilities_call(op: Op) -> Result<Value, String> {
    match op {
        Op::Ping => Ok(Value::Bool(true)),
        Op::Shortcuts => Ok(listed()),
        Op::SetShortcut { id, shortcut } => set_shortcut(&id, shortcut).map(|_| listed()),
        Op::Focus { minutes } => {
            start_focus(minutes.filter(|minutes| (1..=180).contains(minutes)));
            Ok(Value::Null)
        }
        Op::ResetShortcuts => {
            let mut keys = keys();
            keys.overrides = Overrides::new();
            apply(&mut keys);
            save_overrides(&keys.overrides);
            drop(keys);
            Ok(listed())
        }
        Op::Recording { on } => {
            recording(on);
            Ok(Value::Null)
        }
    }
}

/// Starts the shortcuts and picks up a focus timer left running.
pub fn start(app: AppHandle) {
    let _ = APP.set(app.clone());
    let handler = app.clone();
    surface::on_action(Source::Utilities, move |action| {
        if let Err(problem) = handle(&handler, &action) {
            eprintln!("A utilities action didn't go through: {problem}");
        }
    });
    surface::set_muted(preference(MUTED).as_deref() == Some("true"));
    {
        let mut keys = keys();
        keys.overrides = preference(SHORTCUTS).and_then(|raw| serde_json::from_str(&raw).ok()).map(shortcuts::tidy).unwrap_or_default();
        apply(&mut keys);
        if !keys.native {
            fallback(&app, &keys.overrides);
        }
    }
    {
        let mut shown = shown();
        shown.focus = preference(FOCUS).and_then(|raw| serde_json::from_str::<Session>(&raw).ok());
        if let Some(session) = shown.focus.as_mut() {
            if session.catch_up(clock()) == Step::Over {
                shown.focus = None;
            }
        }
        shown.focus_since = surface::now_ms();
        save_focus(shown.focus.as_ref());
        alert_at_turn(shown.focus.as_ref());
    }
    publish();
    std::thread::Builder::new().name("focus-timer".into()).spawn(ticker).ok();
    std::thread::spawn(capture::prune_old);
}

/// Without the native surfaces, the companion keeps its shortcut through Tauri.
fn fallback(app: &AppHandle, overrides: &Overrides) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let companion = shortcuts::bindings(overrides).into_iter().find(|(id, _)| *id == "companion").and_then(|(_, shortcut)| shortcut);
    if companion.is_some_and(|shortcut| shortcut.key_code == 49 && shortcut.carbon() == 0x300) {
        if let Err(error) = app.global_shortcut().register("CommandOrControl+Shift+Space") {
            eprintln!("Global shortcut unavailable: {error}");
        }
    }
}

// MARK: Preferences

fn store<T>(use_store: impl FnOnce(&crate::store::Store) -> T) -> Option<T> {
    let app = APP.get()?;
    let state = app.try_state::<crate::controller::AppState>()?;
    let store = state.store.lock().ok()?;
    Some(use_store(&store))
}

pub(crate) fn preference(key: &str) -> Option<String> {
    store(|store| store.preference(key).ok().flatten()).flatten().filter(|value| !value.is_empty())
}

pub(crate) fn set_preference(key: &str, value: &str) {
    store(|store| store.set_preference(key, value));
}

fn cache_dir() -> PathBuf {
    APP.get().and_then(|app| app.path().app_cache_dir().ok()).unwrap_or_else(|| std::env::temp_dir().join("buntini"))
}

fn clock() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// MARK: Shortcuts

#[derive(Default)]
struct Keys {
    overrides: Overrides,
    /// Shortcuts that couldn't be registered, and why: macOS uses the combination, or another
    /// app holds it.
    refused: Vec<(String, &'static str)>,
    /// The native hot keys are in.
    native: bool,
}

const SYSTEM_USES: &str = "macOS uses it";
const ANOTHER_APP: &str = "Another app uses it";

static KEYS: Mutex<Keys> = Mutex::new(Keys { overrides: Overrides::new(), refused: Vec::new(), native: false });
static RECORDING: AtomicU64 = AtomicU64::new(0);

fn keys() -> MutexGuard<'static, Keys> {
    KEYS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registers every shortcut that's set, except one macOS has taken for itself since (a
/// default such as ⌃⌥Space switches input sources on some Macs). Presses come back through
/// the native surfaces, so without them there are no hot keys (the companion's falls back to
/// Tauri at start).
fn apply(keys: &mut Keys) {
    if !surface::running() {
        keys.native = false;
        return;
    }
    let mut refused = Vec::new();
    let hot_keys: Vec<native::HotKey> = shortcuts::bindings(&keys.overrides)
        .into_iter()
        .filter_map(|(id, shortcut)| shortcut.map(|shortcut| (id, shortcut)))
        .filter(|(id, shortcut)| {
            let system = native::system_uses(shortcut.key_code, shortcut.carbon());
            if system {
                refused.push((id.to_string(), SYSTEM_USES));
            }
            !system
        })
        .map(|(id, shortcut)| native::HotKey { id: id.into(), key_code: shortcut.key_code, modifiers: shortcut.carbon() })
        .collect();
    match native::set_hot_keys(&hot_keys) {
        Some(taken) => {
            keys.native = true;
            refused.extend(taken.into_iter().map(|id| (id, ANOTHER_APP)));
            keys.refused = refused;
        }
        None => keys.native = false,
    }
}

fn refused(keys: &Keys, id: &str) -> Option<&'static str> {
    keys.refused.iter().find(|(other, _)| other == id).map(|(_, why)| *why)
}

fn save_overrides(overrides: &Overrides) {
    set_preference(SHORTCUTS, &serde_json::to_string(overrides).unwrap_or_default());
}

/// The dictation key, when it's a combination of your own.
fn dictation_chord() -> Option<Shortcut> {
    let settings = store(|store| store.settings().ok()).flatten()?;
    let chord = settings.flow.chord.filter(|_| settings.flow.enabled && settings.flow.key == crate::flow::DictationKey::Chord)?;
    Some(Shortcut { key_code: chord.key_code, modifiers: chord.modifiers, label: chord.label })
}

fn set_shortcut(id: &str, shortcut: Option<Shortcut>) -> Result<(), String> {
    if !shortcuts::known(id) {
        return Err("There’s no such shortcut.".into());
    }
    let mut keys = keys();
    if let Some(chosen) = &shortcut {
        let current = shortcuts::bindings(&keys.overrides);
        if let Some(problem) = shortcuts::problem(id, chosen, &current, dictation_chord().as_ref()) {
            return Err(problem);
        }
        if native::system_uses(chosen.key_code, chosen.carbon()) {
            return Err(SYSTEM_USES.into());
        }
    }
    let before = keys.overrides.clone();
    let mut next = before.clone();
    next.insert(id.into(), shortcut);
    keys.overrides = shortcuts::tidy(next);
    apply(&mut keys);
    if let Some(why) = refused(&keys, id) {
        keys.overrides = before;
        apply(&mut keys);
        return Err(why.into());
    }
    save_overrides(&keys.overrides);
    Ok(())
}

fn listed() -> Value {
    let keys = keys();
    let defaults = shortcuts::bindings(&Overrides::new());
    let rows: Vec<Value> = shortcuts::bindings(&keys.overrides)
        .into_iter()
        .zip(defaults)
        .map(|((id, shortcut), (_, default))| {
            json!({
                "id": id,
                "label": shortcuts::label(id),
                "shortcut": shortcut,
                "default": default,
                "problem": refused(&keys, id),
            })
        })
        .collect();
    Value::Array(rows)
}

/// Pauses the hot keys while one is recorded; they come back by themselves after a minute.
fn recording(on: bool) {
    let generation = RECORDING.fetch_add(1, Ordering::SeqCst) + 1;
    native::pause_hot_keys(on);
    if on {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(60));
            if RECORDING.load(Ordering::SeqCst) == generation {
                native::pause_hot_keys(false);
            }
        });
    }
}

// MARK: What the island and the shortcuts ask

fn handle(app: &AppHandle, action: &Action) -> Result<(), String> {
    let card = action.card.as_deref();
    match action.name.as_str() {
        "shortcut" => {
            let id = action.value.as_ref().and_then(Value::as_str).unwrap_or_default();
            pressed(app, id);
            Ok(())
        }
        // The focus button on the island's home, and the focus card's buttons.
        "focus" => {
            start_focus(None);
            Ok(())
        }
        "pause" => focus_change(|session, now| session.pause(now)),
        "resume" => focus_change(|session, now| session.resume(now)),
        "skip" => skip_focus(),
        "stop" => {
            stop_focus();
            Ok(())
        }
        "pill" => {
            show_focus();
            Ok(())
        }
        "selected" => {
            let rect = action.value.as_ref().and_then(Value::as_array).filter(|values| values.len() == 4).ok_or("No area came with it.")?;
            let rect: Vec<f64> = rect.iter().filter_map(Value::as_f64).collect();
            capture::selected(rect.try_into().map_err(|_| "The area didn't read.")?);
            Ok(())
        }
        "selectCancelled" => {
            capture::cancelled();
            Ok(())
        }
        "thumbnailGone" => {
            capture::thumbnail_gone();
            Ok(())
        }
        name => capture::act(name, card, action.value.as_ref()),
    }
}

fn pressed(app: &AppHandle, id: &str) {
    match id {
        "companion" => {
            if let Some(window) = app.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
                    let _ = window.hide();
                    return;
                }
            }
            crate::show_companion(app);
        }
        "chat" => crate::show_companion(app),
        "waiting" => match surface::pills().into_iter().find(|pill| pill.state == PillState::Waiting) {
            Some(pill) => open_pill(pill),
            None => notice("Nothing waiting"),
        },
        "nextPill" => next_pill(),
        "mute" => toggle_mute(),
        "screenshot" => capture::screenshot(),
        "area" => capture::select(),
        "focus" => {
            let running = shown().focus.as_ref().map(Session::paused);
            match running {
                None => {
                    start_focus(None);
                }
                Some(true) => {
                    let _ = focus_change(|session, now| session.resume(now));
                }
                Some(false) => {
                    let _ = focus_change(|session, now| session.pause(now));
                }
            }
        }
        _ => {}
    }
}

fn open_pill(pill: Pill) {
    surface::act(Action { source: pill.source, card: None, name: "pill".into(), value: Some(Value::String(pill.id)) });
}

static LAST_PILL: Mutex<Option<String>> = Mutex::new(None);

/// Each press opens the next pill in the island's order.
fn next_pill() {
    let pills = surface::pills();
    if pills.is_empty() {
        return;
    }
    let mut last = LAST_PILL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = last.as_ref().and_then(|id| pills.iter().position(|pill| &pill.id == id)).map_or(0, |index| (index + 1) % pills.len());
    let pill = pills[at].clone();
    *last = Some(pill.id.clone());
    drop(last);
    open_pill(pill);
}

fn toggle_mute() {
    let muted = !surface::muted();
    surface::set_muted(muted);
    set_preference(MUTED, if muted { "true" } else { "false" });
    notice(if muted { "Sounds off" } else { "Sounds on" });
}

/// A word on the island for a moment.
fn notice(line: &str) {
    shown().notice = Some((line.into(), surface::now_ms()));
    publish();
}

/// Ask about a capture: the chat takes the picture from here.
fn ask(path: &Path) {
    if let Some(app) = APP.get() {
        crate::show_companion(app);
        let _ = app.emit("capture-ask", json!({ "path": path }));
    }
}

// MARK: The focus timer

#[derive(Default)]
struct Shown {
    focus: Option<Session>,
    /// When the timer started, on the board's clock.
    focus_since: u64,
    /// The focus card is shown as news since then: a start, a turn or a click on its pill.
    focus_alert: Option<u64>,
    /// The set just ended.
    focus_done: Option<(Session, u64)>,
    /// A word on the island for a moment, such as "Sounds off".
    notice: Option<(String, u64)>,
}

static SHOWN: Mutex<Shown> = Mutex::new(Shown { focus: None, focus_since: 0, focus_alert: None, focus_done: None, notice: None });
static TICK: Condvar = Condvar::new();

fn shown() -> MutexGuard<'static, Shown> {
    SHOWN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts a set, replacing one running.
pub fn start_focus(minutes: Option<u64>) -> Session {
    let plan = minutes.map_or_else(Plan::default, |minutes| Plan::for_focus(minutes as i64 * 60_000));
    let session = Session::start(plan, clock());
    {
        let mut shown = shown();
        shown.focus = Some(session.clone());
        shown.focus_since = surface::now_ms();
        shown.focus_alert = Some(surface::now_ms());
        shown.focus_done = None;
    }
    changed_focus();
    session
}

/// Changes the running timer; false when none runs.
fn focus_change(change: impl FnOnce(&mut Session, i64)) -> Result<(), String> {
    {
        let mut shown = shown();
        let session = shown.focus.as_mut().ok_or("There’s no focus timer running.")?;
        change(session, clock());
    }
    changed_focus();
    Ok(())
}

fn skip_focus() -> Result<(), String> {
    let step = {
        let mut shown = shown();
        let session = shown.focus.as_mut().ok_or("There’s no focus timer running.")?;
        session.skip(clock())
    };
    if step == Step::Over {
        finish_focus();
    } else {
        shown().focus_alert = Some(surface::now_ms());
        changed_focus();
    }
    Ok(())
}

pub fn stop_focus() -> bool {
    let had = shown().focus.take().is_some();
    changed_focus();
    had
}

fn show_focus() {
    let mut shown = shown();
    if shown.focus.is_some() {
        shown.focus_alert = Some(surface::now_ms());
        drop(shown);
        publish();
    }
}

/// The set is over: say so for a moment.
fn finish_focus() {
    {
        let mut shown = shown();
        if let Some(session) = shown.focus.take() {
            shown.focus_done = Some((session, surface::now_ms()));
        }
    }
    changed_focus();
}

/// Saves, re-arms the macOS alert, wakes the ticker and shows the change.
fn changed_focus() {
    let session = shown().focus.clone();
    save_focus(session.as_ref());
    alert_at_turn(session.as_ref());
    TICK.notify_all();
    publish();
}

fn save_focus(session: Option<&Session>) {
    set_preference(FOCUS, &session.and_then(|session| serde_json::to_string(session).ok()).unwrap_or_default());
}

fn minutes(ms: i64) -> i64 {
    (ms + 59_999) / 60_000
}

/// What the end of this phase brings, for the macOS alert.
fn turn_words(session: &Session) -> String {
    let plan = session.plan;
    match session.phase {
        Phase::Focus if session.round >= focus::ROUNDS => format!("{} rounds done. Rest for {} minutes.", focus::ROUNDS, minutes(plan.long_rest)),
        Phase::Focus => format!("Time for a {}-minute rest.", minutes(plan.rest)),
        Phase::Rest => format!("Back to focus, round {} of {}.", session.round + 1, focus::ROUNDS),
        Phase::LongRest => "Your focus set is done.".into(),
    }
}

/// Hands macOS the alert for the end of this phase, so it shows on time even while Buntini
/// is busy or asleep. Only with desktop notifications on.
fn alert_at_turn(session: Option<&Session>) {
    let os = crate::delivery::os();
    os.remove_pending(&[FOCUS_ALERT.to_string()]);
    let enabled = store(|store| store.settings().map(|settings| settings.notifications_enabled).unwrap_or(false)).unwrap_or(false);
    if let Some(session) = session.filter(|session| enabled && !session.paused()) {
        let seconds = (session.remaining(clock()) + 999) / 1000;
        if seconds > 0 {
            os.schedule(FOCUS_ALERT, seconds as u64, session.phase.label(), &turn_words(session));
        }
    }
}

fn timer_card(session: &Session, now: i64) -> TimerCard {
    TimerCard {
        label: session.phase.label().into(),
        phase: Some(session.phase.id().into()),
        ends_at: session.ends_at.map(|ms| (ms + 500) / 1000),
        remaining: session.remaining(now) as f64 / 1000.0,
        total: session.length() as f64 / 1000.0,
        paused: session.paused(),
        round: Some(session.round),
        rounds: Some(focus::ROUNDS),
    }
}

/// Wakes at each phase's end and whenever the pill's minutes change.
fn ticker() {
    let mut guard = shown();
    loop {
        let now = clock();
        let step = guard.focus.as_mut().map(|session| session.catch_up(now));
        match step {
            Some(Step::Turned) => {
                guard.focus_alert = Some(surface::now_ms());
                drop(guard);
                changed_focus();
                guard = shown();
                continue;
            }
            Some(Step::Over) => {
                drop(guard);
                finish_focus();
                guard = shown();
                continue;
            }
            _ => {}
        }
        let wait = guard.focus.as_ref().filter(|session| !session.paused()).map(|session| {
            let left = session.remaining(now);
            Duration::from_millis(focus::next_badge_change(left).clamp(20, 30_000) as u64)
        });
        guard = match wait {
            Some(wait) => {
                let (next, timeout) = TICK.wait_timeout(guard, wait).unwrap_or_else(|poisoned| poisoned.into_inner());
                if timeout.timed_out() {
                    drop(next);
                    publish();
                    shown()
                } else {
                    next
                }
            }
            None => TICK.wait(guard).unwrap_or_else(|poisoned| poisoned.into_inner()),
        };
    }
}

/// Everything this part shows on the island: the focus pill and card, captures, a notice.
fn publish() {
    let board = surface::now_ms();
    let now = clock();
    let mut pills = Vec::new();
    let mut offers = Vec::new();
    {
        let mut shown = shown();
        shown.focus_alert = shown.focus_alert.filter(|at| board < at + TURN_MS);
        shown.notice = shown.notice.take().filter(|(_, at)| board < at + NOTICE_MS);
        shown.focus_done = shown.focus_done.take().filter(|(_, at)| board < at + TURN_MS);
        if let Some(session) = &shown.focus {
            let card = timer_card(session, now);
            let left = session.remaining(now);
            pills.push(Pill {
                id: FOCUS_ID.into(),
                source: Source::Utilities,
                label: session.phase.label().into(),
                tint: tint(session.phase).into(),
                state: if session.paused() { PillState::Idle } else if session.phase == Phase::Focus { PillState::Working } else { PillState::Done },
                badge: Some(if session.paused() { "Paused".into() } else { focus::badge(left) }),
                progress: Some(1.0 - left as f64 / session.length().max(1) as f64),
            });
            offers.push(Offer { card: Card { id: FOCUS_ID.into(), source: Source::Utilities, body: CardBody::Timer(card.clone()) }, urgency: Urgency::Info, at_ms: shown.focus_since, ttl_ms: None });
            if let Some(at) = shown.focus_alert {
                offers.push(Offer { card: Card { id: format!("{FOCUS_ID}.turn"), source: Source::Utilities, body: CardBody::Timer(card) }, urgency: Urgency::Alert, at_ms: at, ttl_ms: Some(TURN_MS) });
            }
        }
        if let Some((session, at)) = &shown.focus_done {
            let card = TimerCard { label: "Focus".into(), phase: Some("done".into()), ends_at: None, remaining: 0.0, total: session.length() as f64 / 1000.0, paused: false, round: Some(focus::ROUNDS), rounds: Some(focus::ROUNDS) };
            offers.push(Offer { card: Card { id: format!("{FOCUS_ID}.done"), source: Source::Utilities, body: CardBody::Timer(card) }, urgency: Urgency::Alert, at_ms: *at, ttl_ms: Some(TURN_MS) });
        }
        if let Some((line, at)) = &shown.notice {
            offers.push(Offer { card: Card { id: NOTICE_ID.into(), source: Source::Utilities, body: CardBody::Vitals(Vitals { line: line.clone() }) }, urgency: Urgency::Alert, at_ms: *at, ttl_ms: Some(NOTICE_MS) });
        }
    }
    offers.extend(capture::offers());
    surface::publish(Source::Utilities, Contribution { pills, offers });
}

pub fn tint(phase: Phase) -> &'static str {
    match phase {
        Phase::Focus => "#FF7A59",
        Phase::Rest => "#4FD1A5",
        Phase::LongRest => "#7CB8FF",
    }
}

/// A focus request from the chat.
pub fn run_focus(command: focus::Command) -> Result<crate::skills::Outcome, String> {
    let answer = match command {
        focus::Command::Start { minutes: asked } => {
            let session = start_focus(asked);
            let until = chrono::Local::now() + chrono::Duration::milliseconds(session.plan.focus);
            format!("Focusing for {} minutes, until {}. A {}-minute rest comes next.", minutes(session.plan.focus), until.format("%-H:%M"), minutes(session.plan.rest))
        }
        focus::Command::Pause => {
            focus_change(|session, now| session.pause(now))?;
            let left = shown().focus.as_ref().map_or(0, |session| session.remaining(clock()));
            format!("Paused, with {} left.", crate::skills::mac::spoken(((left + 999) / 1000) as u64))
        }
        focus::Command::Resume => {
            focus_change(|session, now| session.resume(now))?;
            let (phase, left) = shown().focus.as_ref().map_or((Phase::Focus, 0), |session| (session.phase, session.remaining(clock())));
            format!("{} again, {} left.", phase.label(), crate::skills::mac::spoken(((left + 999) / 1000) as u64))
        }
        focus::Command::Stop => {
            if stop_focus() {
                "Stopped the focus timer.".to_string()
            } else {
                return Err("There’s no focus timer running.".into());
            }
        }
    };
    Ok(crate::skills::Outcome::Done { answer, shares: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_end_names_what_comes_next() {
        let mut session = Session::start(Plan::default(), 0);
        assert_eq!(turn_words(&session), "Time for a 5-minute rest.");
        session.catch_up(25 * 60_000);
        assert_eq!(turn_words(&session), "Back to focus, round 2 of 4.");
        session.round = 4;
        session.phase = Phase::Focus;
        assert_eq!(turn_words(&session), "4 rounds done. Rest for 15 minutes.");
        session.phase = Phase::LongRest;
        assert_eq!(turn_words(&session), "Your focus set is done.");
    }

    #[test]
    fn the_timer_card_carries_its_deadline_in_seconds() {
        let session = Session::start(Plan::default(), 1_763_300_000_400);
        let card = timer_card(&session, 1_763_300_000_400 + 60_000);
        assert_eq!(card.ends_at, Some(1_763_301_500));
        assert_eq!((card.remaining, card.total, card.round, card.rounds), (1440.0, 1500.0, Some(1), Some(4)));
        assert_eq!(card.phase.as_deref(), Some("focus"));
    }

    /// The real ticker on the real clock, with phases of a few hundred milliseconds: every
    /// phase comes in order, the island's pill follows, and the set ends on its deadline.
    #[test]
    fn a_sped_up_set_turns_on_time_on_the_real_clock() {
        let plan = Plan { focus: 400, rest: 150, long_rest: 250 };
        let started = clock();
        shown().focus = Some(Session::start(plan, started));
        changed_focus();
        std::thread::spawn(ticker);
        let mut seen: Vec<(Phase, u32, i64)> = Vec::new();
        let mut pills = Vec::new();
        let give_up = std::time::Instant::now() + Duration::from_secs(10);
        let ended = loop {
            let (now, done) = {
                let shown = shown();
                (shown.focus.as_ref().map(|session| (session.phase, session.round)), shown.focus_done.is_some())
            };
            if let Some((phase, round)) = now {
                if seen.last().is_none_or(|last| (last.0, last.1) != (phase, round)) {
                    seen.push((phase, round, clock() - started));
                }
                if let Some(pill) = surface::pills().into_iter().find(|pill| pill.id == FOCUS_ID) {
                    if pills.last() != Some(&pill.label) {
                        pills.push(pill.label);
                    }
                }
            } else if done {
                break clock() - started;
            }
            assert!(std::time::Instant::now() < give_up, "the set never ended: {seen:?}");
            std::thread::sleep(Duration::from_millis(3));
        };
        let order: Vec<(Phase, u32)> = seen.iter().map(|(phase, round, _)| (*phase, *round)).collect();
        use Phase::*;
        assert_eq!(order, [(Focus, 1), (Rest, 1), (Focus, 2), (Rest, 2), (Focus, 3), (Rest, 3), (Focus, 4), (LongRest, 4)]);
        // Each turn lands on its deadline, give or take the scheduler.
        let due = [0, 400, 550, 950, 1100, 1500, 1650, 2050];
        for ((_, _, at), due) in seen.iter().zip(due) {
            assert!((*at - due).abs() < 120, "turned at {at} ms, due at {due} ms: {seen:?}");
        }
        assert!((2300..2450).contains(&ended), "ended at {ended} ms");
        assert_eq!(pills, ["Focus", "Rest", "Focus", "Rest", "Focus", "Rest", "Focus", "Long rest"]);
        assert!(surface::pills().iter().all(|pill| pill.id != FOCUS_ID), "the pill goes with the set");
    }

    #[test]
    fn ops_read_as_the_ui_sends_them() {
        let op: Op = serde_json::from_str(r#"{"op":"setShortcut","id":"focus","shortcut":{"keyCode":3,"modifiers":["control","option"],"label":"⌃⌥F"}}"#).unwrap();
        assert!(matches!(op, Op::SetShortcut { ref id, shortcut: Some(ref shortcut) } if id == "focus" && shortcut.carbon() == 0x1800));
        assert!(matches!(serde_json::from_str::<Op>(r#"{"op":"setShortcut","id":"chat","shortcut":null}"#).unwrap(), Op::SetShortcut { shortcut: None, .. }));
        assert!(serde_json::from_str::<Op>(r#"{"op":"recording","on":true,"x":1}"#).is_err());
    }
}
