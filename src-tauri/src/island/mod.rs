//! The island at the notch and the cat that lives in it (stream A): the cat's look and mood,
//! a hello on launch, files dropped on it, and the way to the chat. The cat that roams the
//! desktop stays as it is; the island shows the same cat.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::surface::cards::island as cards;
use crate::surface::{self, Action, Card, CardBody, Cat, Contribution, Home, IslandState, Mood, Offer, Placement, Source, Urgency};

/// How long the hello on launch shows.
const GREETING_MS: u64 = 2_600;
/// How long dropped files wait for where to go.
const DROP_MS: u64 = 15_000;
/// A stretch of agent work this long makes the cat sleepy.
const LONG_STRETCH: Duration = Duration::from_secs(2 * 3600);
/// No agent activity this long is a break.
const BREAK: Duration = Duration::from_secs(20 * 60);

/// Saved under `settings.island`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
}

#[tauri::command]
pub async fn island_call(op: Op) -> Result<serde_json::Value, String> {
    match op {
        Op::Ping => Ok(serde_json::Value::Bool(true)),
    }
}

/// Agent work without a break, for the cat's sleepiness. Only times are kept, in memory.
#[derive(Default)]
struct Streak {
    started: Option<Instant>,
    last: Option<Instant>,
}

impl Streak {
    fn activity(&mut self, now: Instant) {
        if self.last.is_none_or(|last| now.duration_since(last) >= BREAK) {
            self.started = Some(now);
        }
        self.last = Some(now);
    }

    fn sleepy(&self, now: Instant) -> bool {
        match (self.started, self.last) {
            (Some(started), Some(last)) => now.duration_since(last) < BREAK && last.duration_since(started) >= LONG_STRETCH,
            _ => false,
        }
    }
}

static STREAK: Mutex<Streak> = Mutex::new(Streak { started: None, last: None });
/// Music plays in Music or Spotify (stream F).
static MEDIA_PLAYING: AtomicBool = AtomicBool::new(false);
/// The thread that keeps the cat's look, woken when the music starts or stops.
static LOOK: OnceLock<std::thread::Thread> = OnceLock::new();

/// Music started or stopped: the cat dances while it plays, unless sleepiness or something
/// on the island sets its mood.
pub fn media_playing(playing: bool) {
    if MEDIA_PLAYING.swap(playing, Ordering::Relaxed) != playing {
        if let Some(thread) = LOOK.get() {
            thread.unpark();
        }
    }
}

/// A coding agent did something: the cat notices long stretches of work.
pub fn agent_activity() {
    STREAK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).activity(Instant::now());
}

/// Puts the cat in the island, says hello, and follows its look in Settings.
pub fn start(app: AppHandle) {
    let handler = app.clone();
    surface::on_action(Source::Island, move |action| {
        if let Err(problem) = handle(&handler, &action) {
            eprintln!("An island action didn't go through: {problem}");
        }
    });
    let cat = look(&app);
    let name = settings(&app).map(|settings| settings.user_name.trim().to_string()).unwrap_or_default();
    surface::set_island(IslandState { home: Home::Desktop, placement: Placement::Auto, cat: cat.clone() });
    publish(vec![Offer {
        card: Card { id: "greeting".into(), source: Source::Island, body: CardBody::Greeting(cards::Greeting { name: (!name.is_empty()).then(|| first_word(&name)) }) },
        urgency: Urgency::Alert,
        at_ms: surface::now_ms(),
        ttl_ms: Some(GREETING_MS),
    }]);
    std::thread::spawn(move || {
        let _ = LOOK.set(std::thread::current());
        let mut shown = cat;
        loop {
            std::thread::park_timeout(Duration::from_secs(2));
            let mut next = look(&app);
            if STREAK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).sleepy(Instant::now()) {
                next.mood = Mood::Sleepy;
            } else if MEDIA_PLAYING.load(Ordering::Relaxed) {
                next.mood = Mood::Dancing;
            }
            if next != shown {
                surface::set_island(IslandState { home: Home::Desktop, placement: Placement::Auto, cat: next.clone() });
                shown = next;
            }
        }
    });
}

static OFFERS: Mutex<Vec<Offer>> = Mutex::new(Vec::new());

/// Replaces the island's own cards, keeping those still showing.
fn publish(mut offers: Vec<Offer>) {
    let mut kept = OFFERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = surface::now_ms();
    kept.retain(|offer| offer.ttl_ms.is_none_or(|ttl| now < offer.at_ms + ttl) && !offers.iter().any(|new| new.card.id == offer.card.id));
    kept.append(&mut offers);
    surface::publish(Source::Island, Contribution { pills: vec![], offers: kept.clone() });
}

fn forget(id: &str) {
    let mut kept = OFFERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    kept.retain(|offer| offer.card.id != id);
    surface::publish(Source::Island, Contribution { pills: vec![], offers: kept.clone() });
}

fn first_word(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).chars().take(24).collect()
}

fn settings(app: &AppHandle) -> Option<crate::types::Settings> {
    app.try_state::<crate::controller::AppState>()?.store.lock().ok()?.settings().ok()
}

/// The cat's look from Settings, with the season's outfit.
fn look(app: &AppHandle) -> Cat {
    let Some(settings) = settings(app) else { return Cat::default() };
    Cat {
        character: settings.avatar.clone(),
        coat: settings.cat_coat.clone(),
        eyes: settings.cat_eyes.clone(),
        accessory: settings.cat_accessory.clone(),
        accent: accent(&settings),
        mood: Mood::Calm,
        outfit: season(Local::now().date_naive()).map(str::to_string),
    }
}

/// The accent colour as Settings shows it on a dark panel (src/themes.ts `accentHex`).
fn accent(settings: &crate::types::Settings) -> String {
    let hex = |text: &str| text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit());
    match settings.accent.as_str() {
        "custom" if hex(&settings.accent_color) => settings.accent_color.clone(),
        "ember" => "#e9985d".into(),
        "ocean" => "#71bacb".into(),
        "violet" => "#bc92df".into(),
        "sage" => "#9db98b".into(),
        _ => match settings.theme_preset.as_str() {
            "midnight" => "#8c9bff",
            "paper" => "#9db4d6",
            "sakura" => "#f08aa7",
            "forest" => "#8fc286",
            "ocean" => "#6cc3dc",
            _ => "#ff9e62",
        }
        .into(),
    }
}

/// The outfit the cat wears on its own for the season.
pub fn season(date: NaiveDate) -> Option<&'static str> {
    match (date.month(), date.day()) {
        (10, 24..=31) => Some("witch"),
        (12, 1..=26) => Some("santa"),
        (12, 31) | (1, 1) => Some("party"),
        (2, 14) => Some("hearts"),
        _ => None,
    }
}

fn show_chat(app: &AppHandle) {
    crate::show_companion(app);
}

fn handle(app: &AppHandle, action: &Action) -> Result<(), String> {
    match action.name.as_str() {
        "openChat" => {
            show_chat(app);
            Ok(())
        }
        // Files dropped on the island: show where they can go.
        "drop" => {
            let paths: Vec<String> = match &action.value {
                Some(Value::Array(paths)) => paths.iter().filter_map(Value::as_str).map(str::to_string).take(20).collect(),
                _ => Vec::new(),
            };
            if paths.is_empty() {
                return Ok(());
            }
            let targets: Vec<cards::DropTarget> = crate::agents::drop_targets().into_iter().take(3).map(|(pill, label)| cards::DropTarget { pill, label }).collect();
            // Somewhere to put them: an agent session for now; the chat and Mail take them later.
            if targets.is_empty() {
                return Ok(());
            }
            surface::play("drop");
            publish(vec![Offer {
                card: Card { id: "drop".into(), source: Source::Island, body: CardBody::Drop(cards::DropCard { files: paths, targets }) },
                urgency: Urgency::Alert,
                at_ms: surface::now_ms(),
                ttl_ms: Some(DROP_MS),
            }]);
            Ok(())
        }
        // Into an agent session: `{ "pill": …, "paths": [...] }`.
        "dropInto" => {
            let value = action.value.as_ref().ok_or("Nothing was dropped.")?;
            let pill = value.get("pill").and_then(Value::as_str).ok_or("No session came with it.")?;
            let paths: Vec<String> = value.get("paths").and_then(Value::as_array).map(|paths| paths.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            forget("drop");
            crate::agents::terminal::drop_files(pill, &paths)
        }
        // To the chat, which takes them from here.
        "dropChat" => {
            let paths = action.value.as_ref().and_then(|value| value.get("paths")).cloned().unwrap_or(Value::Array(vec![]));
            forget("drop");
            show_chat(app);
            let _ = app.emit("island-files", paths);
            Ok(())
        }
        // The wardrobe: `{ "key": "catCoat" | "catEyes" | "catAccessory" | "avatar", "value": … }`.
        "look" => {
            let value = action.value.as_ref().ok_or("Nothing was chosen.")?;
            let key = value.get("key").and_then(Value::as_str).unwrap_or_default();
            let chosen = value.get("value").and_then(Value::as_str).unwrap_or_default().to_string();
            let state = app.try_state::<crate::controller::AppState>().ok_or("Buntini is still starting.")?;
            let store = state.store.lock().map_err(|_| "Settings are busy.")?;
            let mut settings = store.settings()?;
            match key {
                "catCoat" if ["ginger", "gray", "black", "white", "tuxedo", "calico", "siamese"].contains(&chosen.as_str()) => settings.cat_coat = chosen,
                "catEyes" if ["amber", "green", "blue", "copper", "odd"].contains(&chosen.as_str()) => settings.cat_eyes = chosen,
                "catAccessory" if ["none", "bell", "bow", "bandana", "hat", "flowers"].contains(&chosen.as_str()) => settings.cat_accessory = chosen,
                "avatar" if ["cat", "orb"].contains(&chosen.as_str()) => settings.avatar = chosen,
                _ => return Err("That isn't something the cat can wear.".into()),
            }
            settings.validate()?;
            store.save_settings(&settings)?;
            drop(store);
            let _ = app.emit("settings-changed", ());
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_cat_dresses_for_the_season_and_not_otherwise() {
        assert_eq!(season(day("2026-10-31")), Some("witch"));
        assert_eq!(season(day("2026-12-24")), Some("santa"));
        assert_eq!(season(day("2026-12-31")), Some("party"));
        assert_eq!(season(day("2027-01-01")), Some("party"));
        assert_eq!(season(day("2027-02-14")), Some("hearts"));
        assert_eq!(season(day("2026-11-16")), None);
    }

    #[test]
    fn two_hours_of_agent_work_without_a_break_is_sleepy() {
        let start = Instant::now();
        let mut streak = Streak::default();
        for minutes in (0..=125).step_by(10) {
            streak.activity(start + Duration::from_secs(minutes * 60));
        }
        assert!(streak.sleepy(start + Duration::from_secs(126 * 60)));
        // A break starts it over.
        streak.activity(start + Duration::from_secs(150 * 60));
        assert!(!streak.sleepy(start + Duration::from_secs(151 * 60)));
        // And long after the last activity, the cat is rested.
        assert!(!Streak { started: Some(start), last: Some(start + LONG_STRETCH) }.sleepy(start + LONG_STRETCH + BREAK));
    }

    #[test]
    fn a_greeting_uses_the_first_name_only() {
        assert_eq!(first_word("Sagar Das"), "Sagar");
        assert_eq!(first_word("  "), "  ");
    }
}
