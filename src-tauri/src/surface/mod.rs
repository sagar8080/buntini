//! The native surfaces: the island, the dictation pill and overlays, drawn by
//! native/surface (SwiftUI and Metal) inside Buntini's own process.
//!
//! Rust owns the state. Each part of Buntini publishes its pills and cards here; the board
//! picks what shows; a scene goes to Swift at most once a frame. What the person does comes
//! back as an `Action`, handed to whoever registered for its source.
pub mod board;
pub mod cards;
#[cfg(target_os = "macos")]
mod ffi;
pub mod scene;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub use board::{Contribution, Offer, Urgency, ERROR_MS, FINISH_MS, HOLD_MS};
pub use scene::{Action, Card, CardBody, Cat, Home, Mood, Pill, PillState, Placement, Scene, Source, CONTRACT};

/// How the character lives on screen. Owned by the island (stream A).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IslandState {
    pub home: Home,
    pub placement: Placement,
    pub cat: Cat,
}

type Handler = Arc<dyn Fn(Action) + Send + Sync>;

#[derive(Default)]
struct Hub {
    board: board::Board,
    island: IslandState,
    flow: Option<cards::flow::FlowPill>,
    overlay: Option<cards::utilities::Overlay>,
    sound: Option<scene::Sound>,
    dirty: bool,
}

static HUB: LazyLock<(Mutex<Hub>, Condvar)> = LazyLock::new(Default::default);
static HANDLERS: LazyLock<Mutex<HashMap<Source, Handler>>> = LazyLock::new(Default::default);
static ACTIONS: Mutex<Option<mpsc::Sender<Vec<u8>>>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
static SOUNDS: AtomicU64 = AtomicU64::new(0);
static CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Milliseconds on the board's clock; use it for `Offer::at_ms`.
pub fn now_ms() -> u64 {
    CLOCK.elapsed().as_millis() as u64
}

/// Replaces everything `source` shows. An empty contribution removes it.
pub fn publish(source: Source, contribution: Contribution) {
    change(|hub| hub.board.set(source, contribution));
}

pub fn set_island(state: IslandState) {
    change(|hub| hub.island = state);
}

pub fn set_flow(pill: Option<cards::flow::FlowPill>) {
    change(|hub| hub.flow = pill);
}

pub fn set_overlay(overlay: Option<cards::utilities::Overlay>) {
    change(|hub| hub.overlay = overlay);
}

/// Plays a sound once.
pub fn play(name: &str) {
    let sequence = SOUNDS.fetch_add(1, Ordering::Relaxed) + 1;
    change(|hub| hub.sound = Some(scene::Sound { name: name.into(), sequence }));
}

/// The nine voice bands, 0…1. Sent straight through, outside the scene.
pub fn levels(values: [f32; 9]) {
    #[cfg(target_os = "macos")]
    if STARTED.load(Ordering::SeqCst) {
        ffi::levels(&values);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = values;
}

/// Receives what the person does on `source`'s cards and pills. Called off the main thread.
pub fn on_action(source: Source, handler: impl Fn(Action) + Send + Sync + 'static) {
    HANDLERS.lock().unwrap().insert(source, Arc::new(handler));
}

/// Opens the native surfaces. False when the native library speaks another contract.
pub fn start() -> bool {
    #[cfg(target_os = "macos")]
    {
        if STARTED.load(Ordering::SeqCst) {
            return true;
        }
        if ffi::contract() != CONTRACT {
            eprintln!("Native surfaces speak contract {}, Buntini {}", ffi::contract(), CONTRACT);
            return false;
        }
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        *ACTIONS.lock().unwrap() = Some(sender);
        std::thread::Builder::new()
            .name("surface-actions".into())
            .spawn(move || {
                for bytes in receiver {
                    route(&bytes);
                }
            })
            .ok();
        if !ffi::start(received) {
            return false;
        }
        STARTED.store(true, Ordering::SeqCst);
        std::thread::Builder::new().name("surface-scenes".into()).spawn(send_scenes).ok();
        change(|_| {});
        true
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Closes the native surfaces.
pub fn stop() {
    if STARTED.swap(false, Ordering::SeqCst) {
        #[cfg(target_os = "macos")]
        ffi::stop();
        *ACTIONS.lock().unwrap() = None;
        HUB.1.notify_all();
    }
}

fn change(edit: impl FnOnce(&mut Hub)) {
    let (lock, wake) = &*HUB;
    let mut hub = lock.lock().unwrap();
    edit(&mut hub);
    hub.dirty = true;
    wake.notify_all();
}

fn scene(hub: &mut Hub, now_ms: u64) -> Scene {
    let picked = hub.board.pick(now_ms);
    Scene {
        contract: CONTRACT,
        home: hub.island.home,
        placement: hub.island.placement,
        pills: picked.pills,
        overflow: picked.overflow,
        card: picked.card,
        held: picked.held,
        cat: hub.island.cat.clone(),
        flow: hub.flow.clone(),
        overlay: hub.overlay.clone(),
        sound: hub.sound.clone(),
    }
}

/// Sends a scene when something changed or a card's time is up, at most once a frame.
#[cfg(target_os = "macos")]
fn send_scenes() {
    const FRAME: Duration = Duration::from_millis(16);
    let mut last: Option<Vec<u8>> = None;
    while STARTED.load(Ordering::SeqCst) {
        let json = {
            let (lock, wake) = &*HUB;
            let mut hub = lock.lock().unwrap();
            loop {
                if !STARTED.load(Ordering::SeqCst) {
                    return;
                }
                let now = now_ms();
                let due = hub.board.next_change_ms(now).is_some_and(|at| at <= now);
                if hub.dirty || due {
                    break;
                }
                let wait = hub.board.next_change_ms(now).map_or(Duration::from_secs(60), |at| Duration::from_millis(at - now));
                hub = wake.wait_timeout(hub, wait).unwrap().0;
            }
            hub.dirty = false;
            serde_json::to_vec(&scene(&mut hub, now_ms())).ok()
        };
        if let Some(json) = json {
            if last.as_ref() != Some(&json) {
                if !ffi::update(&json) {
                    eprintln!("The native surfaces refused a scene");
                }
                last = Some(json);
            }
        }
        std::thread::sleep(FRAME);
    }
}

/// Called by Swift on the main thread: copy and hand over, never block.
#[cfg(target_os = "macos")]
extern "C" fn received(bytes: *const u8, length: usize) {
    // SAFETY: Swift passes a buffer it keeps alive until this returns.
    let Some(bytes) = (unsafe { ffi::copy(bytes, length) }) else { return };
    if let Some(sender) = ACTIONS.lock().unwrap().as_ref() {
        let _ = sender.send(bytes);
    }
}

fn route(bytes: &[u8]) {
    let Ok(action) = serde_json::from_slice::<Action>(bytes) else {
        eprintln!("A surface action didn't read");
        return;
    };
    let handler = HANDLERS.lock().unwrap().get(&action.source).cloned();
    if let Some(handler) = handler {
        handler(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FIXTURE: &str = include_str!("../../../native/surface/Fixtures/contract.json");

    #[test]
    fn the_shared_fixture_reads_and_writes_back_unchanged() {
        let raw: Value = serde_json::from_str(FIXTURE).unwrap();
        let scenes: Vec<Scene> = serde_json::from_value(raw["scenes"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&scenes).unwrap(), raw["scenes"]);
        let actions: Vec<Action> = serde_json::from_value(raw["actions"].clone()).unwrap();
        assert!(actions.iter().all(|action| !action.name.is_empty()));
        assert!(scenes.iter().all(|scene| scene.contract == CONTRACT));
    }

    #[test]
    fn the_fixture_has_every_card_kind() {
        let raw: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut kinds: Vec<String> = raw["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|scene| scene["card"]["body"]["kind"].as_str().map(str::to_string))
            .collect();
        kinds.sort();
        kinds.dedup();
        let mut every = [
            "approval", "question", "ticker", "diff", "finished", "recap", "usage", "service", "mail", "nowPlaying", "timer",
            "capture", "vitals", "update", "whatsNew", "drop", "greeting", "note",
        ]
        .map(String::from)
        .to_vec();
        every.sort();
        assert_eq!(kinds, every);
        // A kind added to CardBody but not to the list above fails here.
        for scene in serde_json::from_value::<Vec<Scene>>(raw["scenes"].clone()).unwrap() {
            if let Some(card) = scene.card {
                let kind = serde_json::to_value(&card.body).unwrap()["kind"].as_str().unwrap().to_string();
                assert!(every.contains(&kind), "{kind}");
            }
        }
    }

    #[test]
    fn an_action_with_unknown_fields_is_refused() {
        assert!(serde_json::from_str::<Action>(r#"{"source":"agents","name":"allow","extra":1}"#).is_err());
        let action: Action = serde_json::from_str(r#"{"source":"agents","card":"a1","name":"allow"}"#).unwrap();
        assert_eq!(action.value, None);
    }

    #[test]
    fn an_empty_hub_draws_nothing() {
        let scene = scene(&mut Hub::default(), 0);
        assert!(scene.card.is_none() && scene.pills.is_empty() && !scene.held);
        assert_eq!(scene.home, Home::Desktop);
    }
}
