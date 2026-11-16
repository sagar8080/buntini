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

pub mod demo;
pub mod updates;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {
    /// Where the island sits: `auto` (the notch, or the top of the screen) or the corner the
    /// cat was dragged to. Set by dragging, never in a settings page.
    pub placement: String,
    /// Look once a day for a newer Buntini (updates.rs). Off until you agree.
    pub updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { placement: "auto".into(), updates: false }
    }
}

const PLACEMENTS: [&str; 5] = ["auto", "topLeft", "topRight", "bottomLeft", "bottomRight"];

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !PLACEMENTS.contains(&self.placement.as_str()) {
            return Err("The island can sit at the top or in a corner.".into());
        }
        Ok(())
    }
}

fn placement(name: &str) -> Placement {
    match name {
        "topLeft" => Placement::TopLeft,
        "topRight" => Placement::TopRight,
        "bottomLeft" => Placement::BottomLeft,
        "bottomRight" => Placement::BottomRight,
        _ => Placement::Auto,
    }
}

/// The island as it should be now: the cat's look and mood, where it sits, the line about
/// the Mac.
fn state(app: &AppHandle) -> IslandState {
    let mut cat = look(app);
    if STREAK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).sleepy(Instant::now()) {
        cat.mood = Mood::Sleepy;
    } else if MEDIA_PLAYING.load(Ordering::Relaxed) {
        cat.mood = Mood::Dancing;
    }
    if let Some(mood) = *demo::MOOD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
        cat.mood = mood;
    }
    let placement = settings(app).map(|settings| placement(&settings.island.placement)).unwrap_or_default();
    let line = LINE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    IslandState { home: Home::Desktop, placement, cat, line }
}

static LINE: Mutex<Option<String>> = Mutex::new(None);

/// What the Mac needs a look at, in a few words: busy (with the busiest apps), hot, or low on
/// battery. None when it's fine.
pub fn health_line(cpu: f32, thermal: i64, battery: Option<(u8, bool)>, busiest: &[String]) -> Option<String> {
    if thermal >= 2 {
        return Some("Your Mac is hot".into());
    }
    if let Some((percent, false)) = battery.filter(|(percent, _)| *percent <= 15) {
        return Some(format!("Battery {percent}%"));
    }
    if cpu >= 85.0 {
        let apps = busiest.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
        return Some(if apps.is_empty() { format!("CPU {}%", cpu.round()) } else { format!("CPU {}% · {apps}", cpu.round()) });
    }
    None
}

/// Looks at the Mac once a minute, from the readings the health card already keeps, and
/// reads which apps are busy only when the Mac is.
fn watch_health() {
    std::thread::spawn(|| {
        let mut busiest_at: Option<Instant> = None;
        let mut busiest: Vec<String> = Vec::new();
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let Some(cpu) = sysinfo::System::new().ok().map(|mut system| {
                system.refresh_cpu_usage();
                std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
                system.refresh_cpu_usage();
                system.global_cpu_usage()
            }) else {
                continue;
            };
            let battery = crate::glance::battery_now().map(|battery| (battery.percent, battery.charging));
            if cpu >= 85.0 && busiest_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(300)) {
                busiest = crate::glance::read(false, false).map(|reading| reading.apps.into_iter().filter(|app| app.cpu_percent >= 10.0).map(|app| app.name).collect()).unwrap_or_default();
                busiest_at = Some(Instant::now());
            }
            let line = health_line(cpu, crate::glance::thermal_state(), battery, &busiest);
            let mut current = LINE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if *current != line {
                *current = line;
                drop(current);
                CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
}

static CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    let shown = state(&app);
    let name = settings(&app).map(|settings| settings.user_name.trim().to_string()).unwrap_or_default();
    surface::set_island(shown.clone());
    watch_health();
    updates::start(app.clone());
    publish(vec![Offer {
        card: Card { id: "greeting".into(), source: Source::Island, body: CardBody::Greeting(cards::Greeting { name: (!name.is_empty()).then(|| first_word(&name)) }) },
        urgency: Urgency::Alert,
        at_ms: surface::now_ms(),
        ttl_ms: Some(GREETING_MS),
    }]);
    std::thread::spawn(move || {
        let _ = LOOK.set(std::thread::current());
        let mut shown = shown;
        loop {
            std::thread::park_timeout(Duration::from_secs(2));
            let next = state(&app);
            if next != shown || CHANGED.swap(false, Ordering::SeqCst) {
                surface::set_island(next.clone());
                shown = next;
            }
        }
    });
}

static OFFERS: Mutex<Vec<Offer>> = Mutex::new(Vec::new());

/// Takes one of the island's own cards away.
fn withdraw(id: &str) {
    OFFERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).retain(|offer| offer.card.id != id);
    publish(Vec::new());
}

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
    // A click on the tour only moves it along.
    if action.card.as_deref().is_some_and(|card| card.starts_with("demo:")) {
        demo::advance();
        return Ok(());
    }
    if action.card.as_deref() == Some("update") {
        return updates::act(app, &action.name);
    }
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
            // The chat always takes them (the card's Chat button), and so does an agent session.
            let targets: Vec<cards::DropTarget> = crate::agents::drop_targets().into_iter().take(3).map(|(pill, label)| cards::DropTarget { pill, label }).collect();
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
            let paths: Vec<String> = action.value.as_ref().and_then(|value| value.get("paths")).and_then(Value::as_array).map(|paths| paths.iter().filter_map(Value::as_str).filter(|path| path.starts_with('/')).take(crate::attach::MAX_ATTACHMENTS).map(str::to_string).collect()).unwrap_or_default();
            forget("drop");
            show_chat(app);
            let _ = app.emit("island-files", paths);
            Ok(())
        }
        // Typed in the island: it goes to the chat, which opens with the answer.
        "ask" => {
            let text = action.value.as_ref().and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if text.is_empty() || text.chars().count() > 8_000 {
                return Ok(());
            }
            show_chat(app);
            let _ = app.emit("island-ask", text);
            Ok(())
        }
        // The cat was dragged to a corner, or back to the top.
        "place" => {
            let chosen = action.value.as_ref().and_then(Value::as_str).unwrap_or_default().to_string();
            if !PLACEMENTS.contains(&chosen.as_str()) {
                return Err("The island can sit at the top or in a corner.".into());
            }
            let state = app.try_state::<crate::controller::AppState>().ok_or("Buntini is still starting.")?;
            let store = state.store.lock().map_err(|_| "Settings are busy.")?;
            let mut settings = store.settings()?;
            settings.island.placement = chosen;
            settings.validate()?;
            store.save_settings(&settings)?;
            drop(store);
            surface::set_island(self::state(app));
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
    fn the_line_names_what_needs_a_look_and_stays_quiet_otherwise() {
        assert_eq!(health_line(30.0, 0, Some((80, false)), &[]), None);
        assert_eq!(health_line(94.4, 0, None, &["node".into(), "Google Chrome".into(), "Mail".into()]).as_deref(), Some("CPU 94% · node, Google Chrome"));
        assert_eq!(health_line(90.0, 0, None, &[]).as_deref(), Some("CPU 90%"));
        assert_eq!(health_line(10.0, 0, Some((14, false)), &[]).as_deref(), Some("Battery 14%"));
        assert_eq!(health_line(10.0, 0, Some((14, true)), &[]), None, "charging is fine");
        assert_eq!(health_line(99.0, 2, Some((5, false)), &[]).as_deref(), Some("Your Mac is hot"));
    }

    #[test]
    fn the_island_sits_only_where_it_can() {
        assert!(Settings::default().validate().is_ok());
        assert!(Settings { placement: "bottomLeft".into(), ..Settings::default() }.validate().is_ok());
        assert!(Settings { placement: "middle".into(), ..Settings::default() }.validate().is_err());
        assert_eq!(placement("topRight"), Placement::TopRight);
        assert_eq!(placement("anything"), Placement::Auto);
    }

    #[test]
    fn a_greeting_uses_the_first_name_only() {
        assert_eq!(first_word("Sagar Das"), "Sagar");
        assert_eq!(first_word("  "), "  ");
    }
}
