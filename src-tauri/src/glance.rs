//! Mac at a glance (docs/mac-at-a-glance.md): "How's my Mac?" as a picture, with no model.
//! `read` takes the live readings, `verdict` applies the fixed rules, and `build` makes the
//! card a turn keeps. The sampler adds one point a minute to `mac_history`, numbers only.
//! App names stay in memory, and reach the database only on a card the user asked for.
use crate::{store::Store, types::now};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

pub const QUIT_TOOL: &str = "quit_app";
/// What the database keeps; the card shows the last hour of it.
const HISTORY_SECONDS: i64 = 24 * 60 * 60;
const CARD_SECONDS: i64 = 60 * 60;
const SAMPLE_SECONDS: u64 = 60;
const SIZES_KEY: &str = "mac-folder-sizes";
const SIZES_SECONDS: i64 = 24 * 60 * 60;
const HEALTH_SECONDS: u64 = 24 * 60 * 60;
/// The disk is nearly full under either of these. Decimal, as macOS counts storage.
const LOW_DISK_BYTES: u64 = 10_000_000_000;
const LOW_DISK_SHARE: f64 = 0.05;
const BUSY_PERCENT: f32 = 80.0;
const BUSY_SECONDS: i64 = 330;
const BUSY_POINTS: usize = 5;
const LOW_BATTERY: u8 = 15;
const AGED_BATTERY: u8 = 80;
/// Under this, no app is really using the CPU, so "using the most" means memory instead.
const IDLE_APP_PERCENT: f32 = 2.0;
/// Below this, an app isn't what's making the Mac slow.
const WORTH_QUITTING_PERCENT: f32 = 10.0;
const MOST_APPS: usize = 3;
/// How long an app gets to quit before Buntini says it's still open.
const QUIT_WAIT: Duration = Duration::from_secs(10);
/// An app still open after this may be asking about unsaved work, so it comes to the front.
const QUIT_SHOW_PROMPT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Focus {
    Overview,
    Slow,
    Storage,
    Battery,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pressure {
    Easy,
    Tight,
    Struggling,
}
impl Pressure {
    /// `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4 critical.
    pub fn from_level(level: i32) -> Self {
        match level {
            2 => Self::Tight,
            4 => Self::Struggling,
            _ => Self::Easy,
        }
    }
    fn word(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Tight => "tight",
            Self::Struggling => "struggling",
        }
    }
    fn level(self) -> i32 {
        match self {
            Self::Easy => 1,
            Self::Tight => 2,
            Self::Struggling => 4,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    Cpu,
    Memory,
    Energy,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes_left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycles: Option<u32>,
}
/// An app with its helpers, or everything that belongs to no app.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUse {
    pub name: String,
    /// Share of the whole Mac, 0 to 100.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<f32>,
    /// Offered a Quit button. Never true for macOS, background tasks or Buntini.
    pub quittable: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub key: String,
    pub label: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub parts: Vec<Part>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub state: Pressure,
    pub total_bytes: u64,
    pub swap_bytes: u64,
}
/// One minute on the Mac. Numbers only: this is what the database keeps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Seconds since 1970.
    pub at: i64,
    pub cpu: f32,
    pub pressure: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// "quit" or "settings".
    pub kind: String,
    /// The app to quit, or the settings pane to open.
    pub target: String,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    /// fine, disk, memory, hot, busy, plug or aged.
    pub kind: String,
    pub title: String,
    pub actions: Vec<Action>,
}
/// What the card shows, kept with the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Glance {
    pub focus: Focus,
    /// The card in words, for Read aloud, Copy and a brain allowed Mac health.
    pub summary: String,
    pub verdict: Verdict,
    pub storage: Storage,
    pub memory: Memory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<Battery>,
    pub cpu_percent: f32,
    pub hot: bool,
    pub history: Vec<Sample>,
    pub metric: Metric,
    pub apps: Vec<AppUse>,
    pub captured_at: String,
}
/// The live readings the verdict rules and the card are made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub cpu_percent: f32,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub memory_total: u64,
    pub swap_bytes: u64,
    pub pressure: Pressure,
    /// macOS's thermal state: 0 nominal, 1 fair, 2 serious, 3 critical.
    pub thermal: i64,
    pub battery: Option<Battery>,
    pub apps: Vec<AppUse>,
}
/// How much of the disk Buntini could measure, kept for a day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderSizes {
    pub measured_at: i64,
    pub roots: Vec<String>,
    pub apps: u64,
    pub caches: u64,
    pub folders: u64,
}

fn quit_action(app: &AppUse) -> Action {
    Action { kind: "quit".into(), target: app.name.clone(), label: format!("Quit {}", app.name) }
}
fn settings_action(pane: &str, label: &str) -> Action {
    Action { kind: "settings".into(), target: pane.into(), label: label.into() }
}
fn most(apps: &[AppUse], by: impl Fn(&AppUse) -> f32) -> Option<&AppUse> {
    apps.iter().max_by(|a, b| by(a).total_cmp(&by(b)))
}
fn most_quittable(apps: &[AppUse], by: impl Fn(&AppUse) -> f32) -> Option<&AppUse> {
    most(&apps.iter().filter(|app| app.quittable).cloned().collect::<Vec<_>>(), by)
        .and_then(|found| apps.iter().find(|app| app.name == found.name))
}
fn cpu_of(app: &AppUse) -> f32 {
    app.cpu_percent
}
fn memory_of(app: &AppUse) -> f32 {
    app.memory_bytes as f32
}
/// Energy where `top` gave it, otherwise the CPU, which is what drives it.
fn energy_of(app: &AppUse) -> f32 {
    app.energy.unwrap_or(app.cpu_percent)
}

/// Above the line for the last five minutes, with no minute missing.
pub fn stayed_busy(samples: &[Sample], now: i64) -> bool {
    let recent: Vec<&Sample> = samples.iter().filter(|sample| sample.at > now - BUSY_SECONDS).collect();
    recent.len() >= BUSY_POINTS && recent.iter().all(|sample| sample.cpu > BUSY_PERCENT)
}

/// The rules of docs/mac-at-a-glance.md, in order. The first match wins.
pub fn verdict(reading: &Reading, samples: &[Sample], now: i64) -> Verdict {
    let made = |kind: &str, title: String, actions: Vec<Action>| Verdict { kind: kind.into(), title, actions };
    let quit = |found: Option<&AppUse>| found.map(quit_action).into_iter().collect::<Vec<_>>();
    let short_of_disk = reading.total_bytes > 0
        && (reading.free_bytes < LOW_DISK_BYTES
            || (reading.free_bytes as f64) < reading.total_bytes as f64 * LOW_DISK_SHARE);
    if short_of_disk {
        return made("disk", "Your disk is nearly full".into(), vec![settings_action("storage", "Storage settings")]);
    }
    if reading.pressure == Pressure::Struggling {
        return made("memory", "Your Mac is short on memory".into(), quit(most_quittable(&reading.apps, memory_of)));
    }
    if reading.thermal >= 2 {
        return made("hot", "Your Mac is running hot".into(), quit(most_quittable(&reading.apps, cpu_of)));
    }
    if stayed_busy(samples, now) {
        if let Some(busiest) = most(&reading.apps, cpu_of) {
            let actions = if busiest.quittable { vec![quit_action(busiest)] } else { vec![] };
            return made("busy", format!("{} is keeping your Mac busy", busiest.name), actions);
        }
    }
    if let Some(battery) = &reading.battery {
        if battery.percent < LOW_BATTERY && !battery.charging {
            return made("plug", "Plug in soon".into(), quit(most_quittable(&reading.apps, energy_of)));
        }
        if battery.health_percent.is_some_and(|health| health < AGED_BATTERY) {
            return made("aged", "Your battery has aged".into(), vec![settings_action("battery", "Battery settings")]);
        }
    }
    made("fine", "Your Mac is fine".into(), vec![])
}

/// The question's own next step, beside what the verdict offers. At most two buttons.
fn with_question(mut verdict: Verdict, focus: Focus, reading: &Reading, metric: Metric) -> Verdict {
    let extra = match focus {
        Focus::Storage => Some(settings_action("storage", "Storage settings")),
        Focus::Battery if reading.battery.is_some() => Some(settings_action("battery", "Battery settings")),
        Focus::Slow => {
            let by_memory = metric == Metric::Memory;
            most_quittable(&reading.apps, if by_memory { memory_of } else { cpu_of })
                .filter(|app| by_memory || app.cpu_percent >= WORTH_QUITTING_PERCENT)
                .map(quit_action)
        }
        _ => None,
    };
    if let Some(extra) = extra {
        if !verdict.actions.contains(&extra) {
            verdict.actions.push(extra);
        }
    }
    verdict.actions.truncate(2);
    verdict
}

fn metric_for(focus: Focus, reading: &Reading) -> Metric {
    if focus == Focus::Battery && reading.apps.iter().any(|app| app.energy.is_some()) {
        return Metric::Energy;
    }
    if reading.pressure != Pressure::Easy {
        return Metric::Memory;
    }
    if reading.apps.iter().all(|app| app.cpu_percent < IDLE_APP_PERCENT) {
        return Metric::Memory;
    }
    Metric::Cpu
}

/// Up to three apps by what the card ranks them on, biggest first.
fn top_apps(apps: &[AppUse], metric: Metric) -> Vec<AppUse> {
    let value = |app: &AppUse| match metric {
        Metric::Cpu => cpu_of(app),
        Metric::Memory => memory_of(app),
        Metric::Energy => energy_of(app),
    };
    let mut ranked: Vec<&AppUse> = apps.iter().filter(|app| value(app) > 0.0).collect();
    ranked.sort_by(|a, b| value(b).total_cmp(&value(a)));
    ranked.into_iter().take(MOST_APPS).cloned().collect()
}

/// What's measured, then the rest of what's used. Never more than is used.
pub fn storage_parts(total: u64, free: u64, sizes: Option<&FolderSizes>) -> Vec<Part> {
    let mut left = total.saturating_sub(free);
    let mut parts = Vec::new();
    if let Some(sizes) = sizes {
        for (key, label, bytes) in [("apps", "Apps", sizes.apps), ("caches", "Caches", sizes.caches), ("folders", "Your folders", sizes.folders)] {
            let bytes = bytes.min(left);
            if bytes > 0 {
                left -= bytes;
                parts.push(Part { key: key.into(), label: label.into(), bytes });
            }
        }
    }
    parts.push(Part { key: "other".into(), label: "Everything else".into(), bytes: left });
    parts
}

fn gigabytes(bytes: u64) -> String {
    let value = bytes as f64 / 1e9;
    if value >= 10.0 { format!("{value:.0} GB") } else { format!("{value:.1} GB") }
}
fn time_left(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn summary(verdict: &Verdict, reading: &Reading, top: &[AppUse]) -> String {
    let mut lines = vec![
        format!("{}.", verdict.title),
        format!("{} free of {}.", gigabytes(reading.free_bytes), gigabytes(reading.total_bytes)),
        format!("Memory is {}.", reading.pressure.word()),
    ];
    if let Some(battery) = &reading.battery {
        lines.push(match (battery.charging, battery.minutes_left) {
            (true, _) => format!("Battery at {}%, plugged in.", battery.percent),
            (false, Some(minutes)) => format!("Battery at {}%, {} left.", battery.percent, time_left(minutes)),
            (false, None) => format!("Battery at {}%.", battery.percent),
        });
    }
    if let Some(app) = top.first() {
        lines.push(format!("{} is using the most.", app.name));
    }
    lines.join(" ")
}

/// The card, from the readings. A Mac with no battery has nothing to focus on there.
pub fn build(focus: Focus, reading: Reading, samples: Vec<Sample>, sizes: Option<&FolderSizes>, now_seconds: i64) -> Glance {
    let focus = if focus == Focus::Battery && reading.battery.is_none() { Focus::Overview } else { focus };
    let metric = metric_for(focus, &reading);
    let verdict = with_question(verdict(&reading, &samples, now_seconds), focus, &reading, metric);
    let apps = top_apps(&reading.apps, metric);
    let history = samples.into_iter().filter(|sample| sample.at > now_seconds - CARD_SECONDS).collect();
    Glance {
        focus,
        summary: summary(&verdict, &reading, &apps),
        storage: Storage {
            total_bytes: reading.total_bytes,
            free_bytes: reading.free_bytes,
            parts: storage_parts(reading.total_bytes, reading.free_bytes, sizes),
        },
        memory: Memory { state: reading.pressure, total_bytes: reading.memory_total, swap_bytes: reading.swap_bytes },
        cpu_percent: reading.cpu_percent,
        hot: reading.thermal >= 2,
        battery: reading.battery,
        verdict,
        history,
        metric,
        apps,
        captured_at: now(),
    }
}

// ---- Apps, not processes.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    pub exe: Option<String>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}
#[derive(Debug, PartialEq)]
enum Owner {
    App { name: String, path: String },
    System,
    Unknown,
}
/// The outermost app a path belongs to, so Chrome's nested helper apps read as Chrome.
fn owner_of(exe: &str) -> Owner {
    if exe.starts_with("/System/Library/") || exe.starts_with("/Library/Apple/") {
        return Owner::System;
    }
    let mut path = String::new();
    for part in exe.split('/').filter(|part| !part.is_empty()) {
        path.push('/');
        path.push_str(part);
        if let Some(name) = part.strip_suffix(".app") {
            return Owner::App { name: name.into(), path };
        }
    }
    if ["/usr/", "/bin/", "/sbin/"].iter().any(|root| exe.starts_with(root)) {
        return Owner::System;
    }
    Owner::Unknown
}

const SYSTEM_GROUP: &str = "macOS";
const OTHER_GROUP: &str = "Background tasks";

/// Rolls processes up to their apps. A process with no app of its own belongs to the app
/// that started it (a Terminal's `cargo`, Buntini's worker), or else to macOS or background
/// tasks. `quittable` holds the paths of apps macOS can ask to quit.
pub fn roll_up(processes: &[Proc], quittable: &[String], energy: &HashMap<u32, f32>) -> Vec<AppUse> {
    let by_pid: HashMap<u32, &Proc> = processes.iter().map(|process| (process.pid, process)).collect();
    let mut apps: HashMap<String, AppUse> = HashMap::new();
    for process in processes {
        let mut current = process;
        let mut group = None;
        for _ in 0..32 {
            match current.exe.as_deref().map(owner_of) {
                Some(Owner::App { name, path }) => {
                    group = Some((name, quittable.contains(&path)));
                    break;
                }
                Some(Owner::System) if current.parent.is_none_or(|parent| parent <= 1) => break,
                _ => {}
            }
            match current.parent.filter(|parent| *parent > 1).and_then(|parent| by_pid.get(&parent)) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        let (name, can_quit) = group.unwrap_or_else(|| {
            let system = process.exe.as_deref().is_none_or(|exe| owner_of(exe) == Owner::System);
            ((if system { SYSTEM_GROUP } else { OTHER_GROUP }).to_string(), false)
        });
        let app = apps.entry(name.clone()).or_insert_with(|| AppUse { name, ..AppUse::default() });
        app.cpu_percent += process.cpu_percent;
        app.memory_bytes += process.memory_bytes;
        app.quittable |= can_quit;
        if let Some(used) = energy.get(&process.pid) {
            *app.energy.get_or_insert(0.0) += *used;
        }
    }
    apps.into_values().collect()
}

// ---- Reading what macOS reports.

/// The minutes in `pmset -g batt`'s "7:50 remaining", when it has an estimate.
pub fn parse_minutes_left(output: &str) -> Option<u32> {
    let line = output.lines().find(|line| line.contains("InternalBattery"))?;
    let (before, _) = line.split_once(" remaining")?;
    let clock = before.rsplit(' ').next()?;
    let (hours, minutes) = clock.split_once(':')?;
    let total = hours.parse::<u32>().ok()? * 60 + minutes.parse::<u32>().ok()?;
    (total > 0).then_some(total)
}
/// Maximum capacity and cycle count from `system_profiler SPPowerDataType`.
pub fn parse_battery_health(output: &str) -> (Option<u8>, Option<u32>) {
    let value = |label: &str| {
        output.lines().find_map(|line| line.trim().strip_prefix(label).map(|rest| rest.trim().trim_end_matches('%').to_string()))
    };
    (value("Maximum Capacity:").and_then(|v| v.parse().ok()), value("Cycle Count:").and_then(|v| v.parse().ok()))
}
/// The last sample of `top -l 2 -stats pid,power`; the first has no usable power figures.
pub fn parse_top_power(output: &str) -> HashMap<u32, f32> {
    let lines: Vec<&str> = output.lines().collect();
    let Some(header) = lines.iter().rposition(|line| line.trim_start().starts_with("PID")) else {
        return HashMap::new();
    };
    lines[header + 1..]
        .iter()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect()
}

fn sysctl_level() -> Option<i32> {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and value is a writable int of the size given.
    let status = unsafe {
        libc::sysctlbyname(c"kern.memorystatus_vm_pressure_level".as_ptr(), (&raw mut value).cast(), &raw mut size, std::ptr::null_mut(), 0)
    };
    (status == 0).then_some(value)
}
fn swap_used() -> u64 {
    let mut usage: libc::xsw_usage = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::xsw_usage>();
    // SAFETY: the name is NUL-terminated and usage is a writable xsw_usage of the size given.
    let status = unsafe {
        libc::sysctlbyname(c"vm.swapusage".as_ptr(), (&raw mut usage).cast(), &raw mut size, std::ptr::null_mut(), 0)
    };
    if status == 0 { usage.xsu_used } else { 0 }
}
fn thermal_state() -> i64 {
    objc2_foundation::NSProcessInfo::processInfo().thermalState().0 as i64
}

/// Runs a fixed read-only command and gives up after `limit`, so a stuck tool can't hold a card.
fn output_within(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    use std::process::{Command, Stdio};
    let child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id() as libc::pid_t;
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sent.send(child.wait_with_output());
    });
    match received.recv_timeout(limit) {
        Ok(Ok(output)) if output.status.success() => Some(String::from_utf8_lossy(&output.stdout).into_owned()),
        Ok(_) => None,
        Err(_) => {
            // SAFETY: signals only the child this function started, which hasn't been reaped.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            None
        }
    }
}

fn battery_now() -> Option<Battery> {
    let output = output_within("/usr/bin/pmset", &["-g", "batt"], Duration::from_secs(3))?;
    let (percent, charging) = crate::world::parse_battery(&output)?;
    Some(Battery { percent, charging, minutes_left: parse_minutes_left(&output), health_percent: None, cycles: None })
}

/// Battery health is slow to read, so it's read once a day and kept in memory.
static HEALTH: Mutex<Option<(Instant, (Option<u8>, Option<u32>))>> = Mutex::new(None);
fn battery_health() -> (Option<u8>, Option<u32>) {
    let mut kept = HEALTH.lock().unwrap();
    match *kept {
        Some((at, health)) if at.elapsed() < Duration::from_secs(HEALTH_SECONDS) => health,
        _ => {
            let health = output_within("/usr/sbin/system_profiler", &["SPPowerDataType"], Duration::from_secs(20))
                .map(|output| parse_battery_health(&output))
                .unwrap_or((None, None));
            *kept = Some((Instant::now(), health));
            health
        }
    }
}

/// Reads this Mac now. Blocking: it waits for a CPU sample, and for `top` when it needs energy.
pub fn read(focus: Focus) -> Result<Reading, String> {
    use sysinfo::{Disks, ProcessesToUpdate, System};
    let unreadable = |_| "macOS didn't report this Mac's system information. Try again.".to_string();
    let mut system = System::new_all().map_err(unreadable)?;
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_usage();
    system.refresh_memory();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let cores = system.cpus().len().max(1) as f32;
    let processes: Vec<Proc> = system
        .processes()
        .iter()
        .map(|(pid, process)| Proc {
            pid: pid.as_u32(),
            parent: process.parent().map(|parent| parent.as_u32()),
            exe: process.exe().map(|exe| exe.to_string_lossy().into_owned()),
            cpu_percent: process.cpu_usage() / cores,
            memory_bytes: process.memory(),
        })
        .collect();
    let disks = Disks::new_with_refreshed_list().map_err(unreadable)?;
    let root = disks.iter().find(|disk| disk.mount_point() == Path::new("/")).or_else(|| disks.first());
    let (total_bytes, free_bytes) = root.map(|disk| (disk.total_space(), disk.available_space())).unwrap_or((0, 0));
    let mut battery = battery_now();
    if let Some(battery) = battery.as_mut() {
        (battery.health_percent, battery.cycles) = battery_health();
    }
    let low = battery.as_ref().is_some_and(|battery| battery.percent < LOW_BATTERY && !battery.charging);
    let energy = if battery.is_some() && (focus == Focus::Battery || low) {
        output_within("/usr/bin/top", &["-l", "2", "-s", "1", "-n", "60", "-o", "power", "-stats", "pid,power"], Duration::from_secs(6))
            .map(|output| parse_top_power(&output))
            .unwrap_or_default()
    } else {
        HashMap::new()
    };
    let paths: Vec<String> = running_apps().into_iter().map(|app| app.path).collect();
    Ok(Reading {
        cpu_percent: system.global_cpu_usage(),
        total_bytes,
        free_bytes,
        memory_total: system.total_memory(),
        swap_bytes: swap_used(),
        pressure: Pressure::from_level(sysctl_level().unwrap_or(1)),
        thermal: thermal_state(),
        battery,
        apps: roll_up(&processes, &paths, &energy),
    })
}

// ---- Asking an app to quit.

#[derive(Debug, Clone, PartialEq)]
pub struct RunningApp {
    /// The bundle's name, as the card shows it: "Google Chrome".
    pub name: String,
    /// What the app calls itself, which can differ: "Code".
    pub localized: String,
    pub path: String,
    pub pid: i32,
}
/// Only macOS's system apps stay out by path: Mail and Safari can be quit, Finder can't.
pub fn may_quit(path: &str) -> bool {
    !path.contains("/System/Library/")
}
/// Apps that can be asked to quit: not Buntini, not a system app, not a hidden background one.
fn running_apps() -> Vec<RunningApp> {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
    let own = std::process::id() as i32;
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| app.processIdentifier() != own && app.activationPolicy() != NSApplicationActivationPolicy::Prohibited)
        // macOS updates this list on the main thread, so it can lag behind an app that just quit.
        .filter(|app| !is_gone(app.processIdentifier()))
        .filter_map(|app| {
            let path = app.bundleURL()?.path()?.to_string();
            let name = Path::new(&path).file_stem()?.to_str()?.to_string();
            may_quit(&path).then(|| RunningApp {
                localized: app.localizedName().map(|name| name.to_string()).unwrap_or_else(|| name.clone()),
                name,
                path,
                pid: app.processIdentifier(),
            })
        })
        .collect()
}
/// The one app these words mean: its exact name, or else a single app with those words in
/// its name ("chrome" for Google Chrome). Two matches mean none, since the user has to say.
pub fn find_app<'a>(query: &str, apps: &'a [RunningApp]) -> Option<&'a RunningApp> {
    let query = crate::skills::words(query);
    if query.is_empty() {
        return None;
    }
    let exact = |text: &str| crate::skills::words(text) == query;
    if let Some(app) = apps.iter().find(|app| exact(&app.name) || exact(&app.localized)) {
        return Some(app);
    }
    let holds = |text: &str| format!(" {} ", crate::skills::words(text)).contains(&format!(" {query} "));
    let mut found = apps.iter().filter(|app| holds(&app.name) || holds(&app.localized));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}
/// The name of the running app a request means, if any.
pub fn running_app_named(query: &str) -> Option<String> {
    find_app(query, &running_apps()).map(|app| app.name.clone())
}
fn is_gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) }
}
/// Asks the app to quit, as its Quit menu does, so it can ask about unsaved work. Never
/// force-quits. It succeeded only if the process is actually gone.
pub fn quit_app(name: &str) -> Result<String, String> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let apps = running_apps();
    let app = find_app(name, &apps).ok_or_else(|| format!("{name} isn’t open."))?;
    let running = NSRunningApplication::runningApplicationWithProcessIdentifier(app.pid).ok_or_else(|| format!("{name} isn’t open."))?;
    if !running.terminate() {
        return Err(format!("{} didn’t accept the request to quit, so it’s still open.", app.name));
    }
    let started = Instant::now();
    let mut shown = false;
    while started.elapsed() < QUIT_WAIT {
        if is_gone(app.pid) {
            return Ok(app.name.clone());
        }
        if !shown && started.elapsed() >= QUIT_SHOW_PROMPT {
            // Any save prompt is the app's own, and it can't be answered from behind.
            shown = true;
            running.activateWithOptions(NSApplicationActivationOptions::empty());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{} is still open. It may be asking you something.", app.name))
}

// ---- History.

pub fn save_sample(store: &Store, sample: &Sample) -> Result<(), String> {
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO mac_history(at,cpu,pressure,battery) VALUES(?1,?2,?3,?4)",
            params![sample.at, sample.cpu, sample.pressure, sample.battery],
        )
        .map_err(|error| error.to_string())?;
    store
        .db
        .execute("DELETE FROM mac_history WHERE at<?1", [sample.at - HISTORY_SECONDS])
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub fn samples_since(store: &Store, since: i64) -> Result<Vec<Sample>, String> {
    let mut statement = store
        .db
        .prepare("SELECT at,cpu,pressure,battery FROM mac_history WHERE at>?1 ORDER BY at")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([since], |row| {
            Ok(Sample { at: row.get(0)?, cpu: row.get(1)?, pressure: row.get(2)?, battery: row.get(3)? })
        })
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}
/// What "Export my data" includes.
pub fn export(store: &Store) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "samples": samples_since(store, 0)?, "folderSizes": folder_sizes(store)? }))
}

pub fn folder_sizes(store: &Store) -> Result<Option<FolderSizes>, String> {
    Ok(store.preference(SIZES_KEY)?.and_then(|raw| serde_json::from_str(&raw).ok()))
}
fn sizes_are_stale(sizes: Option<&FolderSizes>, roots: &[String], now_seconds: i64) -> bool {
    sizes.is_none_or(|sizes| sizes.roots != roots || now_seconds - sizes.measured_at > SIZES_SECONDS)
}
/// The space a folder's files take, without following links or leaving its volume.
fn folder_bytes(folder: &Path) -> u64 {
    let Ok(top) = std::fs::symlink_metadata(folder) else {
        return 0;
    };
    let device = top.dev();
    let mut total = 0u64;
    let mut seen = 0u32;
    let mut waiting = vec![folder.to_path_buf()];
    while let Some(next) = waiting.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.dev() != device {
                continue;
            }
            if metadata.is_dir() {
                waiting.push(entry.path());
            } else if metadata.is_file() {
                total += metadata.blocks() * 512;
            }
            seen += 1;
            if seen % 512 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    total
}
/// Shared folders with any folder inside another left out, so nothing counts twice.
fn outermost(roots: &[String]) -> Vec<PathBuf> {
    let paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    paths
        .iter()
        .filter(|path| !paths.iter().any(|other| other != *path && path.starts_with(other)))
        .cloned()
        .collect()
}
fn measure(home: &Path, roots: &[String], now_seconds: i64) -> FolderSizes {
    let apps = folder_bytes(Path::new("/Applications")) + folder_bytes(&home.join("Applications"));
    let caches = folder_bytes(&home.join("Library/Caches"));
    let folders = outermost(roots).iter().map(|root| folder_bytes(root)).sum();
    FolderSizes { measured_at: now_seconds, roots: roots.to_vec(), apps, caches, folders }
}

static MEASURING: AtomicBool = AtomicBool::new(false);
/// Measures folder sizes in the background when the last measure is old, on a thread of its
/// own at background priority, so it stays out of the chart it's for.
pub fn measure_if_stale(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<crate::controller::AppState>();
    let (roots, sizes) = {
        let store = state.store.lock().unwrap();
        (store.roots().unwrap_or_default(), folder_sizes(&store).ok().flatten())
    };
    let now_seconds = chrono::Utc::now().timestamp();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    if !sizes_are_stale(sizes.as_ref(), &roots, now_seconds) || MEASURING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // SAFETY: only sets this new thread's own priority class.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) };
        let sizes = measure(&home, &roots, now_seconds);
        let state = app.state::<crate::controller::AppState>();
        if let Ok(raw) = serde_json::to_string(&sizes) {
            let _ = state.store.lock().unwrap().set_preference(SIZES_KEY, &raw);
        }
        MEASURING.store(false, Ordering::SeqCst);
    });
}

/// Takes the minute's numbers. It keeps one `System` between samples, so the CPU figure
/// covers the whole minute rather than a moment of it, at the cost of a single refresh.
pub struct Sampler {
    system: sysinfo::System,
}
impl Sampler {
    pub fn new() -> Option<Self> {
        let mut system = sysinfo::System::new().ok()?;
        system.refresh_cpu_usage();
        Some(Self { system })
    }
    pub fn take(&mut self) -> Sample {
        self.system.refresh_cpu_usage();
        Sample {
            at: chrono::Utc::now().timestamp(),
            cpu: self.system.global_cpu_usage(),
            pressure: sysctl_level().unwrap_or(Pressure::Easy.level()),
            battery: battery_now().map(|battery| battery.percent),
        }
    }
}

/// Starts the sampler, and reads what's slow once while nobody is waiting for it.
pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let health = tauri::async_runtime::spawn_blocking(|| {
            if battery_now().is_some() {
                battery_health();
            }
        });
        let _ = health.await;
        let Ok(Some(mut sampler)) = tauri::async_runtime::spawn_blocking(Sampler::new).await else {
            return;
        };
        let mut minutes = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(SAMPLE_SECONDS)).await;
            let Ok((back, sample)) = tauri::async_runtime::spawn_blocking(move || {
                let sample = sampler.take();
                (sampler, sample)
            })
            .await
            else {
                return;
            };
            sampler = back;
            let _ = save_sample(&app.state::<crate::controller::AppState>().store.lock().unwrap(), &sample);
            // Checked hourly, starting a minute after launch.
            minutes += 1;
            if minutes % 60 == 1 {
                measure_if_stale(&app);
            }
        }
    });
}

/// The card for a question, now.
pub async fn card(state: &crate::controller::AppState, focus: Focus) -> Result<Glance, String> {
    let reading = tauri::async_runtime::spawn_blocking(move || read(focus))
        .await
        .map_err(|_| "System information could not be collected.".to_string())??;
    let now_seconds = chrono::Utc::now().timestamp();
    let (samples, sizes) = {
        let store = state.store.lock().unwrap();
        (samples_since(&store, now_seconds - CARD_SECONDS)?, folder_sizes(&store)?)
    };
    Ok(build(focus, reading, samples, sizes.as_ref(), now_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;
    const NOW: i64 = 1_800_000_000;

    fn app(name: &str, cpu: f32, memory: u64, quittable: bool) -> AppUse {
        AppUse { name: name.into(), cpu_percent: cpu, memory_bytes: memory, energy: None, quittable }
    }
    fn fine() -> Reading {
        Reading {
            cpu_percent: 12.0,
            total_bytes: 500 * GB,
            free_bytes: 175 * GB,
            memory_total: 16 * GB,
            swap_bytes: 0,
            pressure: Pressure::Easy,
            thermal: 0,
            battery: Some(Battery { percent: 60, charging: false, minutes_left: Some(120), health_percent: Some(89), cycles: Some(359) }),
            apps: vec![
                app(SYSTEM_GROUP, 40.0, 9 * GB, false),
                app("Google Chrome", 20.0, 3 * GB, true),
                app("Photos", 5.0, 5 * GB, true),
            ],
        }
    }
    fn busy() -> Vec<Sample> {
        (0..5).map(|minute| Sample { at: NOW - 60 * minute, cpu: 90.0, pressure: 1, battery: None }).collect()
    }
    fn kind(reading: &Reading, samples: &[Sample]) -> String {
        verdict(reading, samples, NOW).kind
    }

    #[test]
    fn each_rule_fires_and_beats_the_ones_below_it() {
        // Every condition at once, then each removed from the top down.
        let mut reading = fine();
        reading.free_bytes = 5 * GB;
        reading.pressure = Pressure::Struggling;
        reading.thermal = 3;
        reading.battery = Some(Battery { percent: 5, charging: false, minutes_left: None, health_percent: Some(70), cycles: None });
        assert_eq!(kind(&reading, &busy()), "disk");
        reading.free_bytes = 175 * GB;
        assert_eq!(kind(&reading, &busy()), "memory");
        reading.pressure = Pressure::Tight;
        assert_eq!(kind(&reading, &busy()), "hot");
        reading.thermal = 1;
        assert_eq!(kind(&reading, &busy()), "busy");
        assert_eq!(kind(&reading, &[]), "plug");
        reading.battery.as_mut().unwrap().charging = true;
        assert_eq!(kind(&reading, &[]), "aged");
        reading.battery.as_mut().unwrap().health_percent = Some(89);
        assert_eq!(kind(&reading, &[]), "fine");
    }

    #[test]
    fn the_disk_rule_counts_gigabytes_and_share() {
        let mut reading = fine();
        reading.free_bytes = 10 * GB - 1;
        assert_eq!(kind(&reading, &[]), "disk");
        // 15 GB is plenty on a small disk, and under 5% on a big one.
        reading.free_bytes = 15 * GB;
        reading.total_bytes = 250 * GB;
        assert_eq!(kind(&reading, &[]), "fine");
        reading.total_bytes = 500 * GB;
        assert_eq!(kind(&reading, &[]), "disk");
    }

    #[test]
    fn verdicts_offer_what_fits_and_never_a_system_group() {
        let mut reading = fine();
        reading.pressure = Pressure::Struggling;
        let memory = verdict(&reading, &[], NOW);
        assert_eq!(memory.title, "Your Mac is short on memory");
        assert_eq!(memory.actions[0].target, "Photos", "the app using the most memory, and not macOS");
        reading.pressure = Pressure::Easy;
        reading.thermal = 2;
        assert_eq!(verdict(&reading, &[], NOW).actions[0].target, "Google Chrome");
        reading.thermal = 0;
        let names_macos = verdict(&reading, &busy(), NOW);
        assert_eq!(names_macos.title, "macOS is keeping your Mac busy");
        assert!(names_macos.actions.is_empty(), "macOS is never offered to quit");
        reading.apps[0].cpu_percent = 1.0;
        let names_app = verdict(&reading, &busy(), NOW);
        assert_eq!(names_app.title, "Google Chrome is keeping your Mac busy");
        assert_eq!(names_app.actions[0].label, "Quit Google Chrome");
        reading.battery.as_mut().unwrap().percent = 9;
        reading.apps[1].energy = Some(1.0);
        reading.apps[2].energy = Some(30.0);
        assert_eq!(verdict(&reading, &[], NOW).actions[0].target, "Photos", "the most energy");
    }

    #[test]
    fn five_busy_minutes_need_every_minute() {
        assert!(stayed_busy(&busy(), NOW));
        let mut dipped = busy();
        dipped[2].cpu = 40.0;
        assert!(!stayed_busy(&dipped, NOW));
        assert!(!stayed_busy(&busy()[..4], NOW), "a gap isn't five minutes");
        assert!(!stayed_busy(&busy(), NOW + 600), "an old stretch is over");
    }

    #[test]
    fn a_mac_with_no_battery_has_no_battery_card_or_verdict() {
        let mut reading = fine();
        reading.battery = None;
        let glance = build(Focus::Battery, reading, vec![], None, NOW);
        assert_eq!(glance.focus, Focus::Overview);
        assert!(glance.battery.is_none());
        assert_eq!(glance.verdict.kind, "fine");
        assert!(glance.verdict.actions.is_empty());
        assert!(!glance.summary.contains("Battery"));
        assert!(!serde_json::to_string(&glance).unwrap().contains("\"battery\":{"));
    }

    #[test]
    fn the_question_decides_the_emphasis() {
        let storage = build(Focus::Storage, fine(), vec![], None, NOW);
        assert_eq!(storage.verdict.kind, "fine");
        assert_eq!(storage.verdict.actions[0].target, "storage");
        let mut full = fine();
        full.free_bytes = GB;
        let both = build(Focus::Storage, full, vec![], None, NOW);
        assert_eq!(both.verdict.actions.len(), 1, "the same button once");
        let battery = build(Focus::Battery, fine(), vec![], None, NOW);
        assert_eq!(battery.verdict.actions[0].target, "battery");
        assert_eq!(build(Focus::Slow, fine(), vec![], None, NOW).verdict.actions[0].target, "Google Chrome");
        let mut quiet = fine();
        quiet.apps[1].cpu_percent = 3.0;
        assert!(build(Focus::Slow, quiet, vec![], None, NOW).verdict.actions.is_empty(), "nothing worth quitting");
        assert!(build(Focus::Overview, fine(), vec![], None, NOW).verdict.actions.is_empty());
        let mut crowded = fine();
        crowded.pressure = Pressure::Struggling;
        assert!(build(Focus::Battery, crowded, vec![], None, NOW).verdict.actions.len() <= 2);
    }

    #[test]
    fn apps_rank_by_what_the_question_is_about() {
        let by_cpu = build(Focus::Overview, fine(), vec![], None, NOW);
        assert_eq!(by_cpu.metric, Metric::Cpu);
        assert_eq!(by_cpu.apps.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(), [SYSTEM_GROUP, "Google Chrome", "Photos"]);
        let mut tight = fine();
        tight.pressure = Pressure::Tight;
        let by_memory = build(Focus::Slow, tight, vec![], None, NOW);
        assert_eq!(by_memory.metric, Metric::Memory);
        assert_eq!(by_memory.apps[1].name, "Photos");
        let mut idle = fine();
        for app in &mut idle.apps {
            app.cpu_percent = 0.5;
        }
        assert_eq!(build(Focus::Overview, idle, vec![], None, NOW).metric, Metric::Memory);
        let mut energy = fine();
        energy.apps[2].energy = Some(50.0);
        assert_eq!(build(Focus::Battery, energy, vec![], None, NOW).metric, Metric::Energy);
        assert_eq!(build(Focus::Battery, fine(), vec![], None, NOW).metric, Metric::Cpu, "top couldn't say");
        let mut many = fine();
        many.apps.push(app("Mail", 1.0, GB, true));
        assert_eq!(build(Focus::Overview, many, vec![], None, NOW).apps.len(), 3);
    }

    #[test]
    fn storage_shows_what_it_could_measure_and_the_rest() {
        let sizes = FolderSizes { apps: 40 * GB, caches: 10 * GB, folders: 100 * GB, ..FolderSizes::default() };
        let parts = storage_parts(500 * GB, 175 * GB, Some(&sizes));
        assert_eq!(parts.iter().map(|part| part.bytes).sum::<u64>(), 325 * GB, "adds up to what's used");
        assert_eq!(parts.last().unwrap().label, "Everything else");
        assert_eq!(parts.last().unwrap().bytes, 175 * GB);
        // Before anything's measured, and when a stale measure is more than is used.
        assert_eq!(storage_parts(500 * GB, 175 * GB, None).len(), 1);
        let over = storage_parts(100 * GB, 90 * GB, Some(&sizes));
        assert_eq!(over.iter().map(|part| part.bytes).sum::<u64>(), 10 * GB);
    }

    #[test]
    fn folder_sizes_are_remeasured_after_a_day_or_a_change_of_folders() {
        let roots = vec!["/Users/a/Documents".to_string()];
        let sizes = FolderSizes { measured_at: NOW, roots: roots.clone(), ..FolderSizes::default() };
        assert!(sizes_are_stale(None, &roots, NOW));
        assert!(!sizes_are_stale(Some(&sizes), &roots, NOW + 3600));
        assert!(sizes_are_stale(Some(&sizes), &roots, NOW + SIZES_SECONDS + 1));
        assert!(sizes_are_stale(Some(&sizes), &[], NOW));
        let nested = ["/a".to_string(), "/a/b".to_string(), "/c".to_string()];
        assert_eq!(outermost(&nested), [PathBuf::from("/a"), PathBuf::from("/c")]);
    }

    #[cfg(unix)]
    #[test]
    fn folder_sizes_count_files_and_never_follow_links() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("big"), vec![1u8; 1_000_000]).unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::write(dir.path().join("inner/file"), vec![1u8; 100_000]).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let bytes = folder_bytes(dir.path());
        assert!((100_000..500_000).contains(&bytes), "{bytes}");
        assert_eq!(folder_bytes(&dir.path().join("missing")), 0);
    }

    fn process(pid: u32, parent: u32, exe: Option<&str>, cpu: f32) -> Proc {
        Proc { pid, parent: Some(parent), exe: exe.map(str::to_string), cpu_percent: cpu, memory_bytes: 100 }
    }
    const CHROME: &str = "/Applications/Google Chrome.app";

    #[test]
    fn chrome_and_its_helpers_read_as_one_app() {
        let processes = [
            process(10, 1, Some("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"), 5.0),
            process(11, 10, Some("/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)"), 7.0),
        ];
        let apps = roll_up(&processes, &[CHROME.to_string()], &HashMap::new());
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Google Chrome");
        assert_eq!(apps[0].cpu_percent, 12.0);
        assert_eq!(apps[0].memory_bytes, 200);
        assert!(apps[0].quittable);
    }

    #[test]
    fn no_system_process_appears_as_an_app() {
        let processes = [
            process(2, 1, Some("/System/Library/CoreServices/WindowServer"), 1.0),
            process(3, 1, Some("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder"), 1.0),
            process(4, 1, Some("/System/Library/Frameworks/CoreAudio.framework/coreaudiod"), 1.0),
            process(5, 1, Some("/usr/libexec/logd"), 1.0),
            process(6, 1, None, 1.0),
        ];
        let apps = roll_up(&processes, &["/System/Library/CoreServices/Finder.app".to_string()], &HashMap::new());
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, SYSTEM_GROUP);
        assert_eq!(apps[0].cpu_percent, 5.0);
        assert!(!apps[0].quittable);
    }

    #[test]
    fn a_process_belongs_to_the_app_that_started_it() {
        let terminal = "/System/Applications/Utilities/Terminal.app";
        let processes = [
            process(20, 1, Some(&format!("{terminal}/Contents/MacOS/Terminal")), 1.0),
            process(21, 20, Some("/usr/bin/login"), 0.0),
            process(22, 21, Some("/bin/zsh"), 0.0),
            process(23, 22, Some("/Users/a/.cargo/bin/cargo"), 50.0),
            process(30, 1, Some("/opt/homebrew/sbin/some-daemon"), 3.0),
        ];
        let apps = roll_up(&processes, &[terminal.to_string()], &HashMap::new());
        let named = |name: &str| apps.iter().find(|app| app.name == name).unwrap();
        assert_eq!(named("Terminal").cpu_percent, 51.0);
        assert!(named("Terminal").quittable);
        assert_eq!(named(OTHER_GROUP).cpu_percent, 3.0);
        assert!(!named(OTHER_GROUP).quittable);
    }

    #[test]
    fn an_app_macos_cannot_quit_gets_no_button() {
        let processes = [process(1000, 1, Some("/Applications/Buntini.app/Contents/MacOS/buntini"), 1.0), process(1001, 1000, Some("/Users/a/python3"), 30.0)];
        let apps = roll_up(&processes, &[], &HashMap::new());
        assert_eq!(apps[0].name, "Buntini");
        assert_eq!(apps[0].cpu_percent, 31.0, "its worker counts as Buntini");
        assert!(!apps[0].quittable);
    }

    #[test]
    fn energy_adds_up_across_helpers() {
        let processes = [process(10, 1, Some("/Applications/A.app/Contents/MacOS/A"), 0.0), process(11, 10, Some("/Applications/A.app/Contents/B.app/Contents/MacOS/B"), 0.0)];
        let energy = HashMap::from([(10, 4.0), (11, 6.5), (99, 80.0)]);
        assert_eq!(roll_up(&processes, &[], &energy)[0].energy, Some(10.5));
        assert_eq!(roll_up(&processes, &[], &HashMap::new())[0].energy, None);
    }

    #[test]
    fn readings_parse_as_macos_prints_them() {
        let discharging = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=34799715)\t59%; discharging; 1:54 remaining present: true\n";
        assert_eq!(parse_minutes_left(discharging), Some(114));
        assert_eq!(parse_minutes_left(" -InternalBattery-0 (id=1)\t80%; discharging; (no estimate) present: true"), None);
        assert_eq!(parse_minutes_left(" -InternalBattery-0 (id=1)\t80%; charging; 0:00 remaining present: true"), None);
        assert_eq!(parse_minutes_left("Now drawing from 'AC Power'\n"), None);
        let profile = "Battery Information:\n      Health Information:\n          Cycle Count: 359\n          Condition: Normal\n          Maximum Capacity: 89%\n";
        assert_eq!(parse_battery_health(profile), (Some(89), Some(359)));
        assert_eq!(parse_battery_health("AC Charger Information:\n"), (None, None));
        let top = "Processes: 1\nPID    POWER\n1012   0.0\n\nProcesses: 1\nDisks: 8/15T read.\n\nPID    POWER\n1012   81.1 \n1196   69.8 \n";
        assert_eq!(parse_top_power(top), HashMap::from([(1012, 81.1), (1196, 69.8)]));
        assert!(parse_top_power("no table").is_empty());
        assert_eq!(Pressure::from_level(1), Pressure::Easy);
        assert_eq!(Pressure::from_level(2), Pressure::Tight);
        assert_eq!(Pressure::from_level(4), Pressure::Struggling);
    }

    fn running(name: &str, localized: &str) -> RunningApp {
        RunningApp { name: name.into(), localized: localized.into(), path: format!("/Applications/{name}.app"), pid: 1 }
    }

    #[test]
    fn a_request_names_one_running_app_or_none() {
        let apps = [running("Google Chrome", "Google Chrome"), running("Visual Studio Code", "Code"), running("Chrome Remote Desktop", "Chrome Remote Desktop")];
        assert_eq!(find_app("google chrome", &apps).unwrap().name, "Google Chrome");
        assert_eq!(find_app("Code", &apps).unwrap().name, "Visual Studio Code");
        assert_eq!(find_app("studio", &apps).unwrap().name, "Visual Studio Code");
        assert!(find_app("chrome", &apps).is_none(), "two apps have that word");
        assert!(find_app("smoking", &apps).is_none());
        assert!(find_app("", &apps).is_none());
        assert!(find_app("goog", &apps).is_none(), "a whole word, not part of one");
    }

    #[test]
    fn system_apps_are_never_quit_but_apples_own_apps_can_be() {
        assert!(!may_quit("/System/Library/CoreServices/Finder.app"));
        assert!(!may_quit("/System/Library/CoreServices/Dock.app"));
        assert!(may_quit("/System/Applications/Mail.app"));
        assert!(may_quit("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app"));
        assert!(may_quit("/Applications/Google Chrome.app"));
    }

    #[test]
    fn history_keeps_numbers_for_a_day_and_erase_removes_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        let sample = |at| Sample { at, cpu: 10.0, pressure: 1, battery: None };
        save_sample(&store, &sample(NOW - HISTORY_SECONDS - 10)).unwrap();
        save_sample(&store, &sample(NOW - 60)).unwrap();
        save_sample(&store, &Sample { at: NOW, cpu: 22.5, pressure: 2, battery: Some(40) }).unwrap();
        let kept = samples_since(&store, 0).unwrap();
        assert_eq!(kept.len(), 2, "older than a day is gone");
        assert_eq!(kept[1], Sample { at: NOW, cpu: 22.5, pressure: 2, battery: Some(40) });
        assert_eq!(samples_since(&store, NOW - 30).unwrap().len(), 1);
        // The table has no column an app's name could go in.
        let columns: Vec<String> = store.db.prepare("SELECT name FROM pragma_table_info('mac_history')").unwrap().query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(columns, ["at", "cpu", "pressure", "battery"]);
        assert_eq!(store.export_snapshot().unwrap()["macHistory"]["samples"].as_array().unwrap().len(), 2);
        store.set_preference(SIZES_KEY, "{}").unwrap();
        store.erase_everything().unwrap();
        assert!(samples_since(&store, 0).unwrap().is_empty());
        assert!(folder_sizes(&store).unwrap().is_none());
    }

    #[test]
    fn the_card_keeps_the_last_hour_and_says_itself_in_words() {
        let samples: Vec<Sample> = [NOW - 7200, NOW - 3000, NOW].iter().map(|at| Sample { at: *at, cpu: 10.0, pressure: 1, battery: None }).collect();
        let glance = build(Focus::Overview, fine(), samples, None, NOW);
        assert_eq!(glance.history.len(), 2);
        assert_eq!(glance.summary, "Your Mac is fine. 175 GB free of 500 GB. Memory is easy. Battery at 60%, 2 h left. macOS is using the most.");
        assert_eq!(time_left(45), "45 min");
        assert_eq!(time_left(114), "1 h 54 min");
    }

    /// `cargo test --lib glance -- --ignored --nocapture`. The target is 0.1% of one core:
    /// 60 ms of CPU for each minute's sample, its `pmset` included.
    #[test]
    #[ignore = "reads this Mac; prints what a sample costs"]
    fn measure_sampler_cost() {
        fn cpu_ms(who: libc::c_int) -> f64 {
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            unsafe { libc::getrusage(who, &raw mut usage) };
            let time = |t: libc::timeval| t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3;
            time(usage.ru_utime) + time(usage.ru_stime)
        }
        let mut sampler = Sampler::new().unwrap();
        let runs = 40;
        let before = cpu_ms(libc::RUSAGE_SELF) + cpu_ms(libc::RUSAGE_CHILDREN);
        for _ in 0..runs {
            sampler.take();
        }
        let each = (cpu_ms(libc::RUSAGE_SELF) + cpu_ms(libc::RUSAGE_CHILDREN) - before) / runs as f64;
        println!("sampler_cpu_ms_per_sample={each:.2} share_of_one_core_percent={:.4}", each / 60_000.0 * 100.0);
        assert!(each < 60.0, "{each} ms a sample is over the target");
    }

    /// Opens Chess, which has no work to lose, then asks it to quit for real.
    #[test]
    #[ignore = "opens and quits an app on this Mac"]
    fn quitting_a_real_app_leaves_it_gone() {
        assert!(std::process::Command::new("/usr/bin/open").args(["-g", "-j", "-a", "Chess"]).status().unwrap().success());
        let started = Instant::now();
        while running_app_named("Chess").is_none() {
            assert!(started.elapsed() < Duration::from_secs(10), "Chess didn't open");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(quit_app("chess").unwrap(), "Chess");
        assert!(running_app_named("Chess").is_none());
        assert_eq!(quit_app("Chess").unwrap_err(), "Chess isn’t open.");
        assert!(quit_app("Finder").is_err(), "Finder is never on the list");
    }

    #[test]
    #[ignore = "reads this Mac"]
    fn read_this_mac() {
        let glance = build(Focus::Battery, read(Focus::Battery).unwrap(), vec![], None, NOW);
        println!("{}", serde_json::to_string_pretty(&glance).unwrap());
    }
}
