//! The cat's memory of you: its mood and bond, the day it joined you and its favourite
//! spot to be petted. Kept in the store's preferences, so export includes it and Delete
//! everything erases it. The webview owns the feelings; this only keeps them.

use crate::controller::AppState;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use tauri::State;

const KEY: &str = "pet_memory";
/// Where each cat most likes to be petted, chosen once when it joins you.
pub const FAVOURITES: &[&str] = &["cheeks", "chin", "ears"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PetMood {
    pub energy: f64,
    pub affection: f64,
    pub boredom: f64,
    pub annoyance: f64,
    pub excitement: f64,
    pub bond: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PetMemory {
    /// Which cat this is. A new one after Delete everything, so a save from before the
    /// erase can't bring the old bond back.
    pub id: String,
    pub adopted_on: String,
    pub favourite: String,
    pub mood: Option<PetMood>,
    /// When the mood was saved, in milliseconds since the epoch.
    pub saved_at: Option<i64>,
}

fn valid(memory: &PetMemory) -> bool {
    uuid::Uuid::parse_str(&memory.id).is_ok()
        && chrono::NaiveDate::parse_from_str(&memory.adopted_on, "%Y-%m-%d").is_ok()
        && FAVOURITES.contains(&memory.favourite.as_str())
}

fn read(store: &Store) -> Result<Option<PetMemory>, String> {
    Ok(store
        .preference(KEY)?
        .and_then(|raw| serde_json::from_str::<PetMemory>(&raw).ok())
        .filter(valid))
}

fn write(store: &Store, memory: &PetMemory) -> Result<(), String> {
    store.set_preference(KEY, &serde_json::to_string(memory).map_err(|e| e.to_string())?)
}

/// The cat's memory, or a new cat that joins you today.
pub fn load(store: &Store, today: chrono::NaiveDate) -> Result<PetMemory, String> {
    if let Some(memory) = read(store)? {
        return Ok(memory);
    }
    let id = uuid::Uuid::new_v4();
    let memory = PetMemory {
        id: id.to_string(),
        adopted_on: today.format("%Y-%m-%d").to_string(),
        favourite: FAVOURITES[usize::from(id.as_bytes()[0]) % FAVOURITES.len()].into(),
        mood: None,
        saved_at: None,
    };
    write(store, &memory)?;
    Ok(memory)
}

/// Saves the mood of the cat `id`. Feelings are clamped to 0–1; a save for a cat that is
/// no longer here (erased since) is ignored.
pub fn save(store: &Store, id: &str, mood: PetMood, saved_at: i64) -> Result<(), String> {
    let feelings = [mood.energy, mood.affection, mood.boredom, mood.annoyance, mood.excitement, mood.bond];
    if feelings.iter().any(|value| !value.is_finite()) || saved_at <= 0 {
        return Err("The cat's mood is malformed.".into());
    }
    let Some(mut memory) = read(store)? else { return Ok(()) };
    if memory.id != id {
        return Ok(());
    }
    let clamp = |value: f64| value.clamp(0.0, 1.0);
    memory.mood = Some(PetMood {
        energy: clamp(mood.energy),
        affection: clamp(mood.affection),
        boredom: clamp(mood.boredom),
        annoyance: clamp(mood.annoyance),
        excitement: clamp(mood.excitement),
        bond: clamp(mood.bond),
    });
    memory.saved_at = Some(saved_at);
    write(store, &memory)
}

/// What export keeps of the cat: who it is to you, not how it felt a minute ago.
pub fn export(store: &Store) -> Result<serde_json::Value, String> {
    Ok(read(store)?.map_or(serde_json::Value::Null, |memory| {
        serde_json::json!({
            "adoptedOn": memory.adopted_on,
            "favourite": memory.favourite,
            "bond": memory.mood.map(|mood| mood.bond),
        })
    }))
}

#[tauri::command]
pub fn pet_memory(state: State<'_, AppState>) -> Result<PetMemory, String> {
    load(&state.store.lock().unwrap(), chrono::Local::now().date_naive())
}

/// Waits for Delete everything, which holds the same gate, so a save can't slip in
/// between its erase and the cat starting over.
#[tauri::command]
pub async fn save_pet_memory(id: String, mood: PetMood, saved_at: i64, state: State<'_, AppState>) -> Result<(), String> {
    let _gate = state.config_gate.lock().await;
    save(&state.store.lock().unwrap(), &id, mood, saved_at)
}

/// Whether a purr may be heard: the Mac's "Play user interface sound effects" is on (it
/// is unless someone turned it off), no call or recording has the microphone, and no
/// Focus is on.
pub fn sound_allowed(ui_sounds: Option<bool>, microphone_in_use: bool, focus: bool) -> bool {
    ui_sounds != Some(false) && !microphone_in_use && !focus
}

#[tauri::command]
pub fn pet_sound_allowed() -> bool {
    sound_allowed(mac::ui_sounds(), mac::microphone_in_use(), crate::nudges::focus_on())
}

/// One soft tap on a Force Touch trackpad, and a second 120 ms later for a strong purr.
/// Taps closer than 700 ms apart are dropped, whatever the page asks for. Mice and
/// trackpads without haptics, and the Mac's own haptics setting, simply feel nothing.
#[tauri::command]
pub fn pet_purr_tap(double: bool, app: tauri::AppHandle) {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < Duration::from_millis(700)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let _ = app.run_on_main_thread(mac::tap);
    if double {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            let _ = app.run_on_main_thread(mac::tap);
        });
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    pub fn tap() {
        use objc2_app_kit::{NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer};
        NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(NSHapticFeedbackPattern::Alignment, NSHapticFeedbackPerformanceTime::Now);
    }

    /// The global `com.apple.sound.uiaudio.enabled`, or None when it was never changed.
    pub fn ui_sounds() -> Option<bool> {
        let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
        let key = objc2_foundation::ns_string!("com.apple.sound.uiaudio.enabled");
        defaults.objectForKey(key)?;
        Some(defaults.boolForKey(key))
    }

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }
    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    }
    const SYSTEM_OBJECT: u32 = 1;
    const fn code(name: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*name)
    }

    fn property(object: u32, selector: &[u8; 4]) -> Option<u32> {
        let address = Address { selector: code(selector), scope: code(b"glob"), element: 0 };
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: a global property whose value is one UInt32, read into a UInt32.
        let status = unsafe { AudioObjectGetPropertyData(object, &address, 0, std::ptr::null(), &mut size, (&mut value as *mut u32).cast()) };
        (status == 0).then_some(value)
    }

    /// Whether anything (a call, a recording, Buntini listening) is using the default
    /// input device. Reading this needs no microphone permission.
    pub fn microphone_in_use() -> bool {
        property(SYSTEM_OBJECT, b"dIn ")
            .filter(|device| *device != 0)
            .and_then(|device| property(device, b"gone"))
            .is_some_and(|running| running != 0)
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    pub fn tap() {}
    pub fn ui_sounds() -> Option<bool> {
        None
    }
    pub fn microphone_in_use() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_purr_is_heard_only_when_nothing_says_otherwise() {
        assert!(sound_allowed(None, false, false), "interface sounds are on unless turned off");
        assert!(sound_allowed(Some(true), false, false));
        assert!(!sound_allowed(Some(false), false, false));
        assert!(!sound_allowed(None, true, false), "not during a call or recording");
        assert!(!sound_allowed(None, false, true), "not in Focus");
    }

    #[test]
    fn the_macs_audio_and_defaults_can_be_read() {
        // Whatever this Mac's state, reading it never fails or hangs.
        let _ = (mac::ui_sounds(), mac::microphone_in_use());
    }

    fn day(text: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }
    fn mood(bond: f64) -> PetMood {
        PetMood { energy: 0.6, affection: 0.5, boredom: 0.0, annoyance: 0.1, excitement: 0.2, bond }
    }

    #[test]
    fn a_cat_joins_once_and_keeps_its_day_spot_and_mood() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let cat = load(&store, day("2026-11-03")).unwrap();
        assert_eq!(cat.adopted_on, "2026-11-03");
        assert!(FAVOURITES.contains(&cat.favourite.as_str()));
        assert_eq!(cat.mood, None);
        save(&store, &cat.id, mood(0.4), 1_000).unwrap();
        let later = load(&store, day("2026-12-25")).unwrap();
        assert_eq!((later.id.as_str(), later.adopted_on.as_str(), later.favourite.as_str()), (cat.id.as_str(), "2026-11-03", cat.favourite.as_str()));
        assert_eq!(later.mood, Some(mood(0.4)));
        assert_eq!(later.saved_at, Some(1_000));
    }

    #[test]
    fn feelings_are_clamped_and_malformed_moods_refused() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let cat = load(&store, day("2026-11-03")).unwrap();
        save(&store, &cat.id, PetMood { energy: 3.0, bond: -1.0, ..mood(0.0) }, 5).unwrap();
        let saved = load(&store, day("2026-11-03")).unwrap().mood.unwrap();
        assert_eq!((saved.energy, saved.bond), (1.0, 0.0));
        assert!(save(&store, &cat.id, PetMood { affection: f64::NAN, ..mood(0.1) }, 5).is_err());
        assert!(save(&store, &cat.id, mood(0.1), 0).is_err());
        assert!(serde_json::from_value::<PetMood>(serde_json::json!({"energy":0.1,"affection":0.1,"boredom":0.1,"annoyance":0.1,"excitement":0.1,"bond":0.1,"hunger":1})).is_err());
    }

    #[test]
    fn a_save_from_before_delete_everything_never_brings_the_bond_back() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let old = load(&store, day("2026-11-03")).unwrap();
        save(&store, &old.id, mood(0.9), 10).unwrap();
        store.erase_everything().unwrap();
        assert_eq!(store.preference(KEY).unwrap(), None, "erased with the rest");
        assert_eq!(export(&store).unwrap(), serde_json::Value::Null);
        save(&store, &old.id, mood(0.9), 20).unwrap();
        assert_eq!(store.preference(KEY).unwrap(), None, "a late save for the old cat is dropped");
        let new = load(&store, day("2026-11-04")).unwrap();
        save(&store, &old.id, mood(0.9), 30).unwrap();
        assert_eq!(load(&store, day("2026-11-04")).unwrap().mood, None);
        assert_ne!(new.id, old.id);
    }

    #[test]
    fn export_keeps_the_day_spot_and_bond() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let cat = load(&store, day("2026-11-03")).unwrap();
        save(&store, &cat.id, mood(0.35), 10).unwrap();
        let export = store.export_snapshot().unwrap();
        assert_eq!(export["cat"], serde_json::json!({"adoptedOn": "2026-11-03", "favourite": cat.favourite, "bond": 0.35}));
    }

    #[test]
    fn a_damaged_memory_gives_a_new_cat() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store.set_preference(KEY, r#"{"id":"x","adoptedOn":"soon","favourite":"tail","mood":null,"savedAt":null}"#).unwrap();
        let cat = load(&store, day("2026-11-03")).unwrap();
        assert!(valid(&cat));
    }
}
