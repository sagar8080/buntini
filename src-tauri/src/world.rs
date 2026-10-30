//! The pet's view of the desktop and its body in it.
//!
//! `start` samples a few local signals so the companion can behave like a pet:
//! seconds since the last input, the frontmost app's name and category, the front
//! window's frame (for perching on it), and coarse Mac health. Nothing here is
//! stored, logged or sent online, and window titles and contents are never read. A
//! three-hour, in-memory app timeline lets the on-device model answer "what am I doing?";
//! privacy.rs decides which models may see it. Motion commands glide the companion window along walk, jump and fall
//! paths, and a perch keeps it riding on top of a window when that window moves.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager};

static ROAM: AtomicBool = AtomicBool::new(false);
static AWARE: AtomicBool = AtomicBool::new(false);
static MOTION: AtomicU64 = AtomicU64::new(0);
static MOVING: AtomicBool = AtomicBool::new(false);
static PERCH: Mutex<Option<Perch>> = Mutex::new(None);
static HEALTH: Mutex<Option<Health>> = Mutex::new(None);
static TIMELINE: Mutex<Vec<Visit>> = Mutex::new(Vec::new());
static ACTIVE_SINCE: Mutex<Option<chrono::DateTime<chrono::Local>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontApp {
    pub name: String,
    pub category: &'static str,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontWindow {
    pub number: u32,
    pub rect: Rect,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub cpu_percent: f32,
    pub disk_free_percent: f32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
    pub idle_seconds: Option<f64>,
    pub app: Option<FrontApp>,
    pub front_window: Option<FrontWindow>,
    pub work_area: Rect,
    pub pet: Rect,
    pub fullscreen: bool,
    pub meeting: bool,
    pub health: Option<Health>,
    pub perched: bool,
}
/// One stretch of time in an app. Names and times only, kept in memory for three hours.
#[derive(Debug, Clone, PartialEq)]
pub struct Visit {
    pub name: String,
    pub category: &'static str,
    pub since: chrono::DateTime<chrono::Local>,
    pub until: chrono::DateTime<chrono::Local>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppTime {
    pub name: String,
    pub category: &'static str,
    pub minutes: i64,
}
/// What the local model may see when the user asks about their own activity.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub local_time: String,
    pub current_app: Option<String>,
    pub current_category: Option<&'static str>,
    pub minutes_in_current_app: i64,
    pub seconds_since_last_input: Option<i64>,
    pub minutes_active: Option<i64>,
    pub apps_last_two_hours: Vec<AppTime>,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
}
#[derive(Debug, Clone, Copy)]
struct Perch {
    number: u32,
    dx: f64,
    dy: f64,
    last: Rect,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MotionEvent {
    id: u64,
    phase: &'static str,
    kind: String,
    direction: i8,
}

/// Groups apps by what the user is probably doing; the bundle id never leaves Rust.
pub fn category(bundle: &str) -> &'static str {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| bundle.starts_with(prefix));
    if starts(&[
        "us.zoom.",
        "com.microsoft.teams",
        "com.apple.FaceTime",
        "com.cisco.webex",
        "com.skype.",
        "com.hnc.Discord",
    ]) {
        "meeting"
    } else if starts(&[
        "com.microsoft.VSCode",
        "com.apple.dt.Xcode",
        "com.jetbrains.",
        "dev.zed.",
        "com.sublimetext.",
        "com.googlecode.iterm2",
        "com.apple.Terminal",
        "dev.warp.",
        "com.todesktop.",
        "com.github.GitHubClient",
    ]) {
        "code"
    } else if starts(&[
        "com.apple.Safari",
        "com.google.Chrome",
        "org.mozilla.firefox",
        "company.thebrowser.",
        "com.brave.Browser",
        "com.microsoft.edgemac",
        "com.operasoftware.",
    ]) {
        "browser"
    } else if starts(&[
        "com.apple.iWork.",
        "com.microsoft.Word",
        "com.microsoft.Excel",
        "com.microsoft.Powerpoint",
        "notion.id",
        "md.obsidian",
        "com.apple.Notes",
        "com.ulyssesapp.",
        "net.shinyfrog.bear",
    ]) {
        "writing"
    } else if starts(&["com.apple.finder"]) {
        "files"
    } else if starts(&[
        "com.apple.Music",
        "com.spotify.client",
        "com.apple.TV",
        "org.videolan.vlc",
        "com.apple.podcasts",
    ]) {
        "media"
    } else if starts(&[
        "com.figma.",
        "com.adobe.",
        "com.bohemiancoding.sketch3",
        "com.pixelmatorteam.",
    ]) {
        "design"
    } else if starts(&[
        "com.tinyspeck.slackmacgap",
        "com.apple.MobileSMS",
        "com.apple.mail",
        "ru.keepcoder.Telegram",
        "net.whatsapp.",
    ]) {
        "chat"
    } else {
        "other"
    }
}

/// Parses `pmset -g batt`, e.g. "-InternalBattery-0 (id=…)\t85%; discharging; 7:50 remaining".
pub fn parse_battery(output: &str) -> Option<(u8, bool)> {
    let line = output
        .lines()
        .find(|line| line.contains("InternalBattery"))?;
    let percent = line
        .split('%')
        .next()?
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse::<u8>()
        .ok()?;
    let state = line.split(';').nth(1).unwrap_or("").trim();
    Some((
        percent.min(100),
        state == "charging"
            || state == "charged"
            || state.starts_with("finishing")
            || output.contains("'AC Power'"),
    ))
}

const MINUTE: i64 = 60;
const DETOUR_SECONDS: i64 = 20;

/// Extends the current visit or starts a new one; a short detour keeps one visit going.
pub fn record_visit(
    timeline: &mut Vec<Visit>,
    name: &str,
    category: &'static str,
    now: chrono::DateTime<chrono::Local>,
) {
    match timeline.last_mut() {
        Some(last) if last.name == name && (now - last.until).num_seconds() < 5 * MINUTE => {
            last.until = now
        }
        _ => timeline.push(Visit {
            name: name.into(),
            category,
            since: now,
            until: now,
        }),
    }
    timeline.retain(|visit| (now - visit.until).num_hours() < 3);
    let excess = timeline.len().saturating_sub(200);
    timeline.drain(..excess);
}

fn minutes(seconds: i64) -> i64 {
    (seconds + 30) / MINUTE
}

/// Summarizes the timeline for the model: the latest app (Buntini itself is never
/// recorded, so this is what the user was doing before asking) and time per app.
pub fn summarize(
    timeline: &[Visit],
    now: chrono::DateTime<chrono::Local>,
    idle: Option<f64>,
    active_since: Option<chrono::DateTime<chrono::Local>>,
    health: Option<Health>,
) -> Activity {
    let window = now - chrono::Duration::hours(2);
    let mut totals: Vec<AppTime> = Vec::new();
    for visit in timeline.iter().filter(|visit| visit.until > window) {
        let seconds = (visit.until - visit.since.max(window)).num_seconds().max(0);
        match totals.iter_mut().find(|total| total.name == visit.name) {
            Some(total) => total.minutes += seconds,
            None => totals.push(AppTime {
                name: visit.name.clone(),
                category: visit.category,
                minutes: seconds,
            }),
        }
    }
    totals
        .iter_mut()
        .for_each(|total| total.minutes = minutes(total.minutes));
    totals.retain(|total| total.minutes > 0);
    totals.sort_by(|a, b| b.minutes.cmp(&a.minutes));
    totals.truncate(8);
    // The app the user was really in: brief detours on the way to Buntini (clicking the
    // desktop brings Finder forward for a moment) don't count.
    let last = timeline
        .iter()
        .rev()
        .find(|visit| (visit.until - visit.since).num_seconds() >= DETOUR_SECONDS)
        .or(timeline.last());
    Activity {
        local_time: now.format("%A %-d %B, %H:%M").to_string(),
        current_app: last.map(|visit| visit.name.clone()),
        current_category: last.map(|visit| visit.category),
        minutes_in_current_app: last
            .map(|visit| minutes((visit.until - visit.since).num_seconds()))
            .unwrap_or(0),
        seconds_since_last_input: idle.map(|idle| idle as i64),
        minutes_active: active_since.map(|since| minutes((now - since).num_seconds())),
        apps_last_two_hours: totals,
        battery_percent: health.and_then(|health| health.battery_percent),
        charging: health.map(|health| health.charging),
    }
}

fn record(world: &World) {
    let now = chrono::Local::now();
    if let Some(app) = &world.app {
        record_visit(&mut TIMELINE.lock().unwrap(), &app.name, app.category, now);
    }
    let mut active = ACTIVE_SINCE.lock().unwrap();
    match world.idle_seconds {
        Some(idle) if idle >= 180.0 => *active = None,
        Some(idle) if active.is_none() => {
            *active = Some(now - chrono::Duration::milliseconds((idle * 1000.0) as i64))
        }
        _ => {}
    }
}

/// The user's recent activity for the local model. Only available while awareness is on.
pub fn activity() -> Result<Activity, String> {
    if !AWARE.load(Ordering::Relaxed) {
        return Err("Activity awareness is off. Turn on “Notice what I’m doing” in Settings → Make it yours.".into());
    }
    #[cfg(target_os = "macos")]
    let idle = Some(mac::idle_seconds());
    #[cfg(not(target_os = "macos"))]
    let idle = None;
    Ok(summarize(
        &TIMELINE.lock().unwrap(),
        chrono::Local::now(),
        idle,
        *ACTIVE_SINCE.lock().unwrap(),
        *HEALTH.lock().unwrap(),
    ))
}

/// The latest battery, CPU and disk reading, refreshed once a minute while aware.
pub fn health_now() -> Option<Health> {
    *HEALTH.lock().unwrap()
}

fn same(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 2.0
        && (a.y - b.y).abs() < 2.0
        && (a.width - b.width).abs() < 2.0
        && (a.height - b.height).abs() < 2.0
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Rect;
    use objc2::{rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut AnyObject;
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }
    const ON_SCREEN_ONLY: u32 = 1;
    const EXCLUDE_DESKTOP: u32 = 1 << 4;
    const COMBINED_SESSION: i32 = 0;
    const ANY_INPUT: u32 = u32::MAX;

    /// Seconds since any keyboard, mouse or trackpad input. Needs no permission and
    /// reveals no content.
    pub fn idle_seconds() -> f64 {
        unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) }
    }

    pub struct Window {
        pub number: u32,
        pub pid: i32,
        pub layer: i64,
        pub rect: Rect,
    }

    fn number(dict: &NSDictionary<NSString, AnyObject>, key: &str) -> Option<f64> {
        let value = dict.objectForKey(&NSString::from_str(key))?;
        value
            .downcast::<NSNumber>()
            .ok()
            .map(|number| number.doubleValue())
    }

    /// On-screen windows front to back: owner pid, layer and frame only. Titles are
    /// withheld by macOS without Screen Recording permission and are never requested.
    pub fn windows() -> Vec<Window> {
        let raw = unsafe { CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0) };
        let Some(list) = (unsafe { Retained::<NSArray<AnyObject>>::from_raw(raw.cast()) }) else {
            return vec![];
        };
        (0..list.count())
            .filter_map(|index| {
                // CoreGraphics documents string keys, so the checked dictionaries can be
                // viewed with NSString keys.
                let dict = list.objectAtIndex(index).downcast::<NSDictionary>().ok()?;
                let dict =
                    unsafe { Retained::cast_unchecked::<NSDictionary<NSString, AnyObject>>(dict) };
                let bounds = dict
                    .objectForKey(&NSString::from_str("kCGWindowBounds"))?
                    .downcast::<NSDictionary>()
                    .ok()?;
                let bounds = unsafe {
                    Retained::cast_unchecked::<NSDictionary<NSString, AnyObject>>(bounds)
                };
                Some(Window {
                    number: number(&dict, "kCGWindowNumber")? as u32,
                    pid: number(&dict, "kCGWindowOwnerPID")? as i32,
                    layer: number(&dict, "kCGWindowLayer").unwrap_or(0.0) as i64,
                    rect: Rect {
                        x: number(&bounds, "X")?,
                        y: number(&bounds, "Y")?,
                        width: number(&bounds, "Width")?,
                        height: number(&bounds, "Height")?,
                    },
                })
            })
            .collect()
    }

    /// The frontmost app's process id, display name and bundle id.
    pub fn frontmost() -> Option<(i32, String, String)> {
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let name = app
            .localizedName()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let bundle = app
            .bundleIdentifier()
            .map(|bundle| bundle.to_string())
            .unwrap_or_default();
        Some((app.processIdentifier(), name, bundle))
    }
}

fn window_rect(window: &tauri::WebviewWindow) -> Option<Rect> {
    let scale = window.scale_factor().ok()?;
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(Rect {
        x: f64::from(position.x) / scale,
        y: f64::from(position.y) / scale,
        width: f64::from(size.width) / scale,
        height: f64::from(size.height) / scale,
    })
}

fn work_area(window: &tauri::WebviewWindow) -> Option<(Rect, Rect)> {
    let monitor = window.current_monitor().ok()??;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (position, size) = (monitor.position(), monitor.size());
    Some((
        Rect {
            x: f64::from(area.position.x) / scale,
            y: f64::from(area.position.y) / scale,
            width: f64::from(area.size.width) / scale,
            height: f64::from(area.size.height) / scale,
        },
        Rect {
            x: f64::from(position.x) / scale,
            y: f64::from(position.y) / scale,
            width: f64::from(size.width) / scale,
            height: f64::from(size.height) / scale,
        },
    ))
}

fn sample(window: &tauri::WebviewWindow) -> Option<World> {
    let pet = window_rect(window)?;
    let (area, screen) = work_area(window)?;
    #[cfg(target_os = "macos")]
    {
        let aware = AWARE.load(Ordering::Relaxed);
        let front = mac::frontmost();
        let own = std::process::id() as i32;
        let windows = mac::windows();
        let front_window =
            front
                .as_ref()
                .filter(|(pid, _, _)| *pid != own)
                .and_then(|(pid, _, _)| {
                    windows.iter().find(|w| {
                        w.pid == *pid
                            && w.layer == 0
                            && w.rect.width >= 160.0
                            && w.rect.height >= 100.0
                    })
                });
        let fullscreen = front_window.is_some_and(|w| same(w.rect, screen));
        let category = front
            .as_ref()
            .map(|(_, _, bundle)| category(bundle))
            .unwrap_or("other");
        Some(World {
            idle_seconds: aware.then(mac::idle_seconds),
            app: front
                .filter(|(pid, _, _)| aware && *pid != own)
                .map(|(_, name, _)| FrontApp { name, category }),
            front_window: front_window.map(|w| FrontWindow {
                number: w.number,
                rect: w.rect,
            }),
            work_area: area,
            pet,
            fullscreen,
            meeting: aware && category == "meeting",
            health: if aware { *HEALTH.lock().unwrap() } else { None },
            perched: PERCH.lock().unwrap().is_some(),
        })
    }
    #[cfg(not(target_os = "macos"))]
    Some(World {
        idle_seconds: None,
        app: None,
        front_window: None,
        work_area: area,
        pet,
        fullscreen: false,
        meeting: false,
        health: None,
        perched: false,
    })
}

/// None when macOS doesn't report system information.
fn health() -> Option<Health> {
    use sysinfo::{Disks, System};
    let mut system = System::new().ok()?;
    system.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_usage();
    let disks = Disks::new_with_refreshed_list().ok()?;
    let root = disks
        .iter()
        .find(|disk| disk.mount_point() == std::path::Path::new("/"))
        .or_else(|| disks.first());
    let disk_free_percent = root
        .filter(|disk| disk.total_space() > 0)
        .map(|disk| disk.available_space() as f32 / disk.total_space() as f32 * 100.0)
        .unwrap_or(100.0);
    let battery = std::process::Command::new("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .ok()
        .and_then(|output| parse_battery(&String::from_utf8_lossy(&output.stdout)));
    Some(Health {
        battery_percent: battery.map(|(percent, _)| percent),
        charging: battery.is_none_or(|(_, charging)| charging),
        cpu_percent: system.global_cpu_usage(),
        disk_free_percent,
    })
}

/// Samples the world once a second while roaming or awareness is on, glues the pet to
/// its perch ten times a second, and refreshes health once a minute.
pub fn start(app: tauri::AppHandle) {
    let glue = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut ticks = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            ticks += 1;
            let roam = ROAM.load(Ordering::Relaxed);
            let aware = AWARE.load(Ordering::Relaxed);
            if !roam && !aware {
                continue;
            }
            let Some(window) = glue.get_webview_window("main") else {
                continue;
            };
            if roam && !MOVING.load(Ordering::Relaxed) && !crate::overlay::is_expanded() {
                follow_perch(&glue, &window);
            }
            if aware && (ticks == 1 || ticks % 600 == 0) {
                tauri::async_runtime::spawn_blocking(|| *HEALTH.lock().unwrap() = health());
            }
            if ticks % 10 == 0 {
                if let Some(world) = sample(&window) {
                    if aware {
                        record(&world);
                    }
                    let _ = glue.emit_to("main", "pet-world", world);
                }
            }
        }
    });
}

fn follow_perch(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    let Some(perch) = *PERCH.lock().unwrap() else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let Some(pet) = window_rect(window) else {
            return;
        };
        let target = mac::windows()
            .into_iter()
            .find(|w| w.number == perch.number)
            .map(|w| w.rect);
        let center = pet.x + pet.width / 2.0;
        // A window that leaps (a Space switch, a resize to another screen) drops the pet
        // instead of teleporting it.
        let leapt = target.is_some_and(|rect| {
            (rect.x - perch.last.x).abs() > 240.0 || (rect.y - perch.last.y).abs() > 240.0
        });
        match target {
            Some(rect) if !leapt && center > rect.x + 8.0 && center < rect.x + rect.width - 8.0 => {
                if let Some(slot) = PERCH.lock().unwrap().as_mut() {
                    slot.last = rect;
                }
                let (x, y) = (rect.x + perch.dx, rect.y + perch.dy);
                if (x - pet.x).abs() > 0.5 || (y - pet.y).abs() > 0.5 {
                    place(window, x, y);
                }
            }
            // The window closed, minimized, leapt, or slid out from under the pet: let it fall.
            _ => {
                *PERCH.lock().unwrap() = None;
                let _ = app.emit_to("main", "pet-perch-lost", ());
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window, perch);
}

/// Keeps a pet position on the current screen: at most a quarter of the pet may hang
/// off a side, and it never sinks below the usable area.
pub fn clamp_to_area(x: f64, y: f64, pet: Rect, area: Rect) -> (f64, f64) {
    (
        x.clamp(
            area.x - pet.width * 0.25,
            (area.x + area.width - pet.width * 0.75).max(area.x),
        ),
        y.clamp(
            area.y - pet.height * 0.5,
            (area.y + area.height - pet.height * 0.8).max(area.y),
        ),
    )
}

/// The only way this module moves the window.
fn place(window: &tauri::WebviewWindow, x: f64, y: f64) {
    let (Some(pet), Some((area, _))) = (window_rect(window), work_area(window)) else {
        return;
    };
    let (x, y) = clamp_to_area(x, y, pet, area);
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

/// A fresh look at the world for planning a move, instead of the last once-a-second sample.
#[tauri::command]
pub fn pet_sample(app: tauri::AppHandle) -> Option<World> {
    sample(&app.get_webview_window("main")?)
}

#[tauri::command]
pub fn pet_world(roam: bool, aware: bool) {
    ROAM.store(roam, Ordering::Relaxed);
    AWARE.store(aware, Ordering::Relaxed);
    if !roam {
        stop();
    }
    if !aware {
        // Turning awareness off forgets everything it noticed.
        *HEALTH.lock().unwrap() = None;
        TIMELINE.lock().unwrap().clear();
        *ACTIVE_SINCE.lock().unwrap() = None;
    }
}

fn stop() {
    MOTION.fetch_add(1, Ordering::SeqCst);
    MOVING.store(false, Ordering::Relaxed);
    *PERCH.lock().unwrap() = None;
}

#[tauri::command]
pub fn pet_stop() {
    stop();
}

/// Position along a motion at progress `t` (0–1) or elapsed `seconds` for falls.
pub fn path(kind: &str, from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    let t = t.clamp(0.0, 1.0);
    match kind {
        "jump" => {
            // Quadratic curve through an apex above the higher end.
            let control = (
                (from.0 + to.0) / 2.0,
                from.1.min(to.1) - 70.0 - (from.1 - to.1).abs() * 0.15,
            );
            let u = 1.0 - t;
            (
                u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
            )
        }
        "fall" => (
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t * t,
        ),
        _ => {
            let eased = t * t * (3.0 - 2.0 * t) * 0.25 + t * 0.75;
            (
                from.0 + (to.0 - from.0) * eased,
                from.1 + (to.1 - from.1) * eased,
            )
        }
    }
}

/// Seconds for a motion. Jumps and falls are timed by height but never cross the screen
/// faster than a cat could, so a long sideways move can't become a slide.
pub fn duration(kind: &str, from: (f64, f64), to: (f64, f64), speed: f64) -> f64 {
    let (dx, dy) = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
    match kind {
        "jump" => (0.45 + dy / 1200.0).max(dx / 450.0),
        "fall" => (2.0 * dy.max(1.0) / 2600.0)
            .sqrt()
            .max(dx / 400.0)
            .max(0.15),
        _ => (dx.hypot(dy) / speed.clamp(20.0, 400.0)).max(0.05),
    }
}

/// Moves the companion window to (x, y) in logical screen points. Ends any perch;
/// the frontend perches again after landing on a window.
#[tauri::command]
pub async fn pet_move(
    app: tauri::AppHandle,
    x: f64,
    y: f64,
    kind: String,
    speed: f64,
) -> Result<u64, String> {
    if !ROAM.load(Ordering::Relaxed) {
        return Err("Roaming is off.".into());
    }
    if !x.is_finite()
        || !y.is_finite()
        || !speed.is_finite()
        || !["walk", "jump", "fall"].contains(&kind.as_str())
    {
        return Err("Invalid pet motion.".into());
    }
    let window = app
        .get_webview_window("main")
        .ok_or("The companion window is unavailable.")?;
    let start = window_rect(&window).ok_or("The companion position is unavailable.")?;
    let (area, _) = work_area(&window).ok_or("The screen is unavailable.")?;
    let to = clamp_to_area(x, y, start, area);
    let from = (start.x, start.y);
    *PERCH.lock().unwrap() = None;
    let id = MOTION.fetch_add(1, Ordering::SeqCst) + 1;
    MOVING.store(true, Ordering::Relaxed);
    let direction = if to.0 > from.0 + 0.5 {
        1
    } else if to.0 < from.0 - 0.5 {
        -1
    } else {
        0
    };
    let event = |phase: &'static str| MotionEvent {
        id,
        phase,
        kind: kind.clone(),
        direction,
    };
    let _ = app.emit_to("main", "pet-motion", event("start"));
    let seconds = duration(&kind, from, to, speed);
    let began = std::time::Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        if MOTION.load(Ordering::SeqCst) != id || crate::overlay::is_expanded() {
            let _ = app.emit_to("main", "pet-motion", event("cancelled"));
            return Ok(id);
        }
        let t = (began.elapsed().as_secs_f64() / seconds).min(1.0);
        let (px, py) = path(&kind, from, to, t);
        place(&window, px, py);
        if t >= 1.0 {
            break;
        }
    }
    MOVING.store(false, Ordering::Relaxed);
    let _ = app.emit_to("main", "pet-motion", event("end"));
    Ok(id)
}

/// Starts riding on the given on-screen window at the pet's current offset.
#[tauri::command]
pub fn pet_perch(app: tauri::AppHandle, number: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("main")
            .ok_or("The companion window is unavailable.")?;
        let pet = window_rect(&window).ok_or("The companion position is unavailable.")?;
        let rect = mac::windows()
            .into_iter()
            .find(|w| w.number == number)
            .map(|w| w.rect)
            .ok_or("That window is no longer on screen.")?;
        *PERCH.lock().unwrap() = Some(Perch {
            number,
            dx: pet.x - rect.x,
            dy: pet.y - rect.y,
            last: rect,
        });
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, number);
        Err("Perching is only available on macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_map_to_coarse_categories() {
        assert_eq!(category("us.zoom.xos"), "meeting");
        assert_eq!(category("com.microsoft.teams2"), "meeting");
        assert_eq!(category("com.apple.dt.Xcode"), "code");
        assert_eq!(category("com.jetbrains.intellij"), "code");
        assert_eq!(category("com.apple.Safari"), "browser");
        assert_eq!(category("com.apple.finder"), "files");
        assert_eq!(category("com.spotify.client"), "media");
        assert_eq!(category("com.example.unknown"), "other");
    }

    #[test]
    fn battery_lines_parse_charge_and_power_source() {
        assert_eq!(parse_battery("Now drawing from 'Battery Power'\n -InternalBattery-0 (id=34799715)\t85%; discharging; 7:50 remaining present: true"), Some((85, false)));
        assert_eq!(parse_battery("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true"), Some((100, true)));
        assert_eq!(parse_battery("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t42%; charging; 1:10 remaining"), Some((42, true)));
        assert_eq!(parse_battery("Now drawing from 'AC Power'"), None);
    }

    #[test]
    fn motion_paths_start_and_end_where_asked() {
        for kind in ["walk", "jump", "fall"] {
            assert_eq!(
                path(kind, (10.0, 500.0), (300.0, 200.0), 0.0),
                (10.0, 500.0)
            );
            let end = path(kind, (10.0, 500.0), (300.0, 200.0), 1.0);
            assert!(
                (end.0 - 300.0).abs() < 1e-9 && (end.1 - 200.0).abs() < 1e-9,
                "{kind}"
            );
        }
        let apex = path("jump", (0.0, 500.0), (200.0, 500.0), 0.5);
        assert!(apex.1 < 500.0 - 30.0, "jumps arc upward");
        assert!(
            path("fall", (0.0, 0.0), (0.0, 400.0), 0.5).1 < 200.0,
            "falls accelerate"
        );
        assert!(duration("walk", (0.0, 0.0), (300.0, 0.0), 100.0) > 2.9);
        assert!(duration("jump", (0.0, 300.0), (40.0, 0.0), 100.0) < 1.0);
        // Sideways speed stays cat-like even when a jump or fall covers the whole screen.
        for kind in ["jump", "fall"] {
            let seconds = duration(kind, (1648.0, 951.0), (-32.0, 951.0), 95.0);
            assert!(
                1680.0 / seconds <= 450.0,
                "{kind} crossed at {:.0} pt/s",
                1680.0 / seconds
            );
        }
    }

    #[test]
    fn positions_stay_on_the_usable_screen() {
        let pet = Rect {
            x: 0.0,
            y: 0.0,
            width: 128.0,
            height: 136.0,
        };
        let area = Rect {
            x: 0.0,
            y: 33.0,
            width: 1800.0,
            height: 1038.0,
        };
        assert_eq!(clamp_to_area(-213.0, 951.0, pet, area), (-32.0, 951.0));
        assert_eq!(
            clamp_to_area(5000.0, 5000.0, pet, area),
            (1704.0, 33.0 + 1038.0 - 108.8)
        );
        assert_eq!(
            clamp_to_area(600.0, -400.0, pet, area),
            (600.0, 33.0 - 68.0)
        );
        assert_eq!(clamp_to_area(600.0, 500.0, pet, area), (600.0, 500.0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "reads this Mac's live window list; run with --ignored live_world"]
    fn live_world_signals() {
        let (pid, name, bundle) = mac::frontmost().expect("a frontmost app");
        let windows = mac::windows();
        let front = windows.iter().find(|w| w.pid == pid && w.layer == 0);
        println!(
            "front app: {name} ({}) idle {:.1}s, {} windows, front window {:?}",
            category(&bundle),
            mac::idle_seconds(),
            windows.len(),
            front.map(|w| w.rect)
        );
        println!("health: {:?}", health());
        assert!(mac::idle_seconds() >= 0.0);
        assert!(!windows.is_empty());
    }

    #[test]
    fn the_timeline_merges_visits_and_summarizes_recent_apps() {
        use chrono::TimeZone;
        let at = |minute: i64| {
            chrono::Local
                .with_ymd_and_hms(2026, 10, 21, 16, 0, 0)
                .unwrap()
                + chrono::Duration::minutes(minute)
        };
        let mut timeline = Vec::new();
        for minute in 0..=40 {
            record_visit(&mut timeline, "Xcode", "code", at(minute));
        }
        for minute in 41..=51 {
            record_visit(&mut timeline, "Safari", "browser", at(minute));
        }
        for minute in 52..=70 {
            record_visit(&mut timeline, "Xcode", "code", at(minute));
        }
        assert_eq!(
            timeline.len(),
            3,
            "a return to Xcode after a detour starts a new visit"
        );
        let health = Health {
            battery_percent: Some(64),
            charging: false,
            cpu_percent: 12.0,
            disk_free_percent: 30.0,
        };
        let summary = summarize(&timeline, at(70), Some(4.0), Some(at(0)), Some(health));
        assert_eq!(summary.current_app.as_deref(), Some("Xcode"));
        assert_eq!(summary.minutes_in_current_app, 18);
        assert_eq!(summary.minutes_active, Some(70));
        assert_eq!(
            summary.apps_last_two_hours,
            vec![
                AppTime {
                    name: "Xcode".into(),
                    category: "code",
                    minutes: 58
                },
                AppTime {
                    name: "Safari".into(),
                    category: "browser",
                    minutes: 10
                }
            ]
        );
        assert_eq!(summary.battery_percent, Some(64));
        assert!(summary
            .local_time
            .starts_with("Wednesday 21 October, 17:10"));
        // Clicking the desktop to reach Buntini brings Finder forward for a moment.
        let mut detour = timeline.clone();
        record_visit(
            &mut detour,
            "Finder",
            "files",
            at(70) + chrono::Duration::seconds(4),
        );
        assert_eq!(
            summarize(
                &detour,
                at(70) + chrono::Duration::seconds(6),
                None,
                None,
                None
            )
            .current_app
            .as_deref(),
            Some("Xcode"),
            "a few seconds in Finder is a detour"
        );
        // Only the last two hours count, and entries older than three hours are dropped.
        record_visit(&mut timeline, "Notes", "writing", at(250));
        assert_eq!(timeline.len(), 1);
        assert!(summarize(&[], at(0), None, None, None)
            .current_app
            .is_none());
    }

    #[test]
    fn rects_compare_with_tolerance() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: 982.0,
        };
        assert!(same(a, Rect { x: 1.0, ..a }));
        assert!(!same(a, Rect { height: 950.0, ..a }));
    }
}
