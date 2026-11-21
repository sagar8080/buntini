//! Music, Spotify, TV, browsers and any other player in the island (stream F). What's playing
//! is read only from a player that is open, allowed in Settings → Apps, and that macOS already
//! lets Buntini ask, so a read never opens a player or puts a permission prompt on screen.
//! What a browser or another player (Podcasts, VLC, IINA, Books) plays comes from macOS itself,
//! which shows it in Control Center too, and asks nothing of the app (system.rs); it shows
//! only for a browser allowed in Settings → Apps, or while "Other players" is. The card's
//! controls are the person's own clicks and act at once. The cat dances while music plays.
mod artwork;
mod native;
mod player;
mod system;

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, Notify};

use crate::surface::cards::media::NowPlaying;
use crate::surface::{self, Action, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency};
use player::{unix_now, Player, Read, Reading};

/// While the card is open and something plays.
const WATCHED: Duration = Duration::from_secs(1);
/// While something plays and the card is closed: the pill's progress, and anything the
/// players' own notifications don't say.
const PLAYING: Duration = Duration::from_secs(10);
/// A paused or closed player, or one Buntini may not ask yet.
const IDLE: Duration = Duration::from_secs(45);
/// While the screen sleeps nothing is read; waking says so.
const ASLEEP: Duration = Duration::from_secs(3600);
/// The pill stays this long after a pause, so playing on is a click away.
const PAUSED_PILL: Duration = Duration::from_secs(5 * 60);
/// A read this many seconds from where the track should be means it was moved.
const DRIFT: f64 = 1.5;
/// The pill's progress moves on in steps this big.
const PILL_STEP: f64 = 0.02;
/// Reads that fail in a row before a player counts as showing nothing.
const FAILURES: u8 = 3;
/// After a control, how long a browser or another player has to say it took it before the
/// card goes back to what macOS last said.
const ANSWER: Duration = Duration::from_secs(2);
const CARD: &str = "now-playing";

/// Saved under `settings.media`.
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
pub async fn media_call(op: Op) -> Result<serde_json::Value, String> {
    match op {
        Op::Ping => Ok(serde_json::Value::Bool(true)),
    }
}

/// One player as last seen.
#[derive(Default)]
struct Slot {
    read: Option<Read>,
    /// When it was last asked.
    at: Option<Instant>,
    /// Ask at the next look.
    force: bool,
    failures: u8,
}

/// The track on the card. `reading` holds the position and when it was read; the island
/// runs the clock on from there, so a read that agrees with it changes nothing.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    reading: Reading,
    /// When this track came to the card, on the board's clock.
    since_ms: u64,
    artwork: Option<String>,
    cover_asked: bool,
    /// When it was paused after playing on the card.
    paused_at: Option<Instant>,
}

/// What may show from what macOS says plays, as Settings → Apps has it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Watched {
    /// The browsers switched on, by their id there.
    browsers: Vec<&'static str>,
    /// "Other players" is switched on.
    others: bool,
}

impl Watched {
    fn any(&self) -> bool {
        self.others || !self.browsers.is_empty()
    }
}

#[derive(Default)]
struct Hub {
    app: Option<AppHandle>,
    covers: Option<PathBuf>,
    slots: [Slot; 4],
    /// What the watcher last said, before any control changed the card.
    heard: Option<Read>,
    /// Counts the controls sent to what macOS says plays, so only the last one's answer is
    /// waited for.
    reported_controls: u64,
    watched: Watched,
    shown: Option<Shown>,
    /// The card is on screen.
    visible: bool,
    asleep: bool,
    pill_progress: Option<f64>,
    published: Option<Contribution>,
    dancing: bool,
}

static HUB: LazyLock<Mutex<Hub>> = LazyLock::new(Default::default);
static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);
/// The screen slept or woke, or what may show changed: the watcher stops or starts.
static WATCH_WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);
static CONTROLS: OnceLock<mpsc::UnboundedSender<Action>> = OnceLock::new();

fn hub() -> MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The display sleeps or the session is locked, so the island isn't on screen.
pub fn screen_away() -> bool {
    hub().asleep
}

/// An app was switched on or off in Settings → Apps: the card follows at once.
pub fn apps_changed() {
    hub().slots.iter_mut().for_each(|slot| slot.force = true);
    WAKE.notify_one();
}

/// Starts reading the players and handling the card. Call once the island is up.
pub fn start(app: AppHandle) {
    let covers = app.path().app_cache_dir().ok().map(|dir| dir.join("covers"));
    if let Some(dir) = &covers {
        artwork::reset(dir);
    }
    {
        let mut hub = hub();
        hub.app = Some(app);
        hub.covers = covers;
        hub.slots.iter_mut().for_each(|slot| slot.force = true);
    }
    let (sender, receiver) = mpsc::unbounded_channel();
    let _ = CONTROLS.set(sender);
    surface::on_action(Source::Media, received);
    if !native::watch(noticed) {
        eprintln!("The players' notifications aren't watched; reading on a timer only");
    }
    tauri::async_runtime::spawn(controls(receiver));
    tauri::async_runtime::spawn(watch());
    tauri::async_runtime::spawn(async {
        // Launch is busy enough; the card can wait a moment.
        tokio::time::sleep(Duration::from_secs(3)).await;
        loop {
            let wait = look().await;
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
}

/// The watcher, on the main thread: note it and get out of the way.
extern "C" fn noticed(bytes: *const u8, length: usize) {
    // SAFETY: Swift passes a buffer it keeps alive until this returns.
    let Some(event) = (unsafe { native::event(bytes, length) }) else { return };
    let mut hub = hub();
    match event {
        native::Event::Player { player } => {
            if let Some(player) = Player::from_id(&player) {
                hub.slots[player.index()].force = true;
            }
        }
        native::Event::Asleep { value } => {
            hub.asleep = value;
            if !value {
                hub.slots.iter_mut().for_each(|slot| slot.force = true);
            }
            WATCH_WAKE.notify_one();
        }
    }
    drop(hub);
    WAKE.notify_one();
}

/// From the card, on the surface's action thread.
fn received(action: Action) {
    if action.name == "shown" {
        let visible = action.value.as_ref().and_then(Value::as_bool).unwrap_or(false);
        let mut hub = hub();
        hub.visible = visible;
        if let Some(player) = hub.shown.as_ref().map(|shown| shown.reading.player).filter(|_| visible) {
            hub.slots[player.index()].force = true;
        }
        drop(hub);
        WAKE.notify_one();
        return;
    }
    if let Some(controls) = CONTROLS.get() {
        let _ = controls.send(action);
    }
}

// ----- Reading -----

/// Reads whichever players are due, settles what the card shows, and says how long until
/// the next look.
async fn look() -> Duration {
    let (app, asleep) = {
        let hub = hub();
        (hub.app.clone(), hub.asleep)
    };
    let Some(app) = app else { return IDLE };
    if asleep {
        return ASLEEP;
    }
    let (allowed, watched) = allowed_apps(&app);
    {
        let mut hub = hub();
        if hub.watched != watched {
            hub.watched = watched;
            WATCH_WAKE.notify_one();
        }
    }
    for player in Player::SCRIPTED {
        let due = {
            let hub = hub();
            let slot = &hub.slots[player.index()];
            slot.force || slot.at.is_none_or(|at| at.elapsed() >= cadence(&hub, player))
        };
        if !due {
            continue;
        }
        let read = ask(player, allowed.contains(&player)).await;
        let mut hub = hub();
        let slot = &mut hub.slots[player.index()];
        slot.at = Some(Instant::now());
        slot.force = false;
        match read {
            Some(read) => {
                slot.read = Some(read);
                slot.failures = 0;
            }
            // A player that's slow to answer keeps what it last said, for a while.
            None => {
                slot.failures = slot.failures.saturating_add(1);
                if slot.failures >= FAILURES {
                    slot.read = None;
                }
            }
        }
    }
    let mut hub = hub();
    settle(&mut hub);
    // macOS says when what it reports changes; while it plays the pill's progress still moves on.
    let reported = matches!(&hub.slots[Player::Browser.index()].read, Some(Read::Track(reading)) if reading.playing).then_some(PLAYING);
    Player::SCRIPTED
        .into_iter()
        .map(|player| {
            let slot = &hub.slots[player.index()];
            if slot.force {
                return Duration::ZERO;
            }
            slot.at.map_or(Duration::ZERO, |at| cadence(&hub, player).saturating_sub(at.elapsed()))
        })
        .chain(reported)
        .min()
        .unwrap_or(IDLE)
        .max(Duration::from_millis(200))
}

/// Keeps the watcher of what macOS says plays running while the screen is on and a browser or
/// "Other players" is switched on, and starts it again if it stops.
async fn watch() {
    let idle = || {
        let hub = hub();
        hub.asleep || !hub.watched.any()
    };
    let mut failures: u32 = 0;
    loop {
        while idle() {
            WATCH_WAKE.notified().await;
        }
        let started = Instant::now();
        let mut watcher = match system::Watcher::start() {
            Ok(watcher) => Some(watcher),
            Err(problem) => {
                eprintln!("{problem}");
                None
            }
        };
        if let Some(watcher) = watcher.as_mut() {
            loop {
                tokio::select! {
                    said = watcher.next() => match said {
                        Some(system::Heard::Read(read)) => heard(Some(read)),
                        Some(system::Heard::Unavailable) => {
                            eprintln!("macOS doesn't say what plays on this Mac");
                            heard(None);
                            return;
                        }
                        None => break,
                    },
                    _ = WATCH_WAKE.notified() => if idle() { break },
                }
            }
        }
        drop(watcher);
        if hub().asleep {
            // It's asked again on waking; until then the card is out of sight anyway.
            continue;
        }
        if idle() {
            // Every browser and "Other players" were switched off.
            heard(None);
            continue;
        }
        // It stopped by itself, so what it said may no longer hold.
        heard(None);
        failures = if started.elapsed() > Duration::from_secs(60) { 0 } else { failures + 1 };
        tokio::time::sleep(Duration::from_secs(1 << failures.min(6))).await;
    }
}

/// What the watcher said, on the card.
fn heard(read: Option<Read>) {
    let mut hub = hub();
    hub.heard = read.clone();
    let slot = &mut hub.slots[Player::Browser.index()];
    slot.read = read;
    slot.at = Some(Instant::now());
    settle(&mut hub);
}

/// How often a player is worth asking.
fn cadence(hub: &Hub, player: Player) -> Duration {
    let playing = matches!(&hub.slots[player.index()].read, Some(Read::Track(reading)) if reading.playing);
    let on_card = hub.shown.as_ref().is_some_and(|shown| shown.reading.player == player);
    match (playing, on_card && hub.visible) {
        (true, true) => WATCHED,
        (true, false) => PLAYING,
        _ => IDLE,
    }
}

/// One player, if it's allowed, open, and macOS won't have to ask. `None` when it didn't
/// answer.
async fn ask(player: Player, allowed: bool) -> Option<Read> {
    if !allowed || !player.running() || !crate::apps::may_script_silently(player.app()).await {
        return Some(Read::Closed);
    }
    match player::read(player).await {
        Ok(read) => Some(read),
        Err(problem) => {
            eprintln!("{} didn't say what's playing: {problem}", player.app());
            None
        }
    }
}

/// The players switched on in Settings → Apps, and what may show of what macOS says plays.
fn allowed_apps(app: &AppHandle) -> (Vec<Player>, Watched) {
    let settings = app.try_state::<crate::controller::AppState>().and_then(|state| state.store.lock().ok()?.settings().ok());
    let Some(settings) = settings else { return (Vec::new(), Watched::default()) };
    let players = Player::SCRIPTED.into_iter().filter(|player| crate::apps::allowed(&settings, player.id())).collect();
    let watched = Watched {
        browsers: crate::apps::BROWSERS.into_iter().filter(|id| crate::apps::allowed(&settings, id)).collect(),
        others: crate::apps::allowed(&settings, crate::apps::OTHER_PLAYERS),
    };
    (players, watched)
}

/// What macOS says plays, unless its browser, or "Other players", is switched off in
/// Settings → Apps.
fn shown_reported<'a>(read: Option<&'a Read>, watched: &Watched) -> Option<&'a Read> {
    read.filter(|read| match read {
        Read::Track(reading) if reading.player == Player::Other => watched.others,
        Read::Track(reading) => reading.app.as_deref().and_then(system::switch).is_some_and(|id| watched.browsers.contains(&id)),
        _ => true,
    })
}

/// Which track the card shows: one that plays, keeping the one shown; else the one shown,
/// paused; else any paused one.
fn choose(reads: [Option<&Read>; 4], current: Option<Player>) -> Option<Reading> {
    let tracks: Vec<&Reading> = reads
        .into_iter()
        .flatten()
        .filter_map(|read| match read {
            Read::Track(reading) => Some(reading),
            _ => None,
        })
        .collect();
    tracks
        .iter()
        .filter(|reading| reading.playing)
        .min_by_key(|reading| (Some(reading.player) != current, reading.player.index()))
        .or_else(|| tracks.iter().find(|reading| Some(reading.player) == current))
        .or_else(|| tracks.first())
        .map(|reading| (*reading).clone())
}

/// The card after a read: the same track keeps its place, cover and clock unless the read
/// says otherwise.
fn follow(shown: Option<Shown>, pick: Option<Reading>, now_ms: u64, now: Instant) -> Option<Shown> {
    let reading = pick?;
    let Some(old) = shown.filter(|old| old.reading.player == reading.player) else {
        return Some(Shown { reading, since_ms: now_ms, artwork: None, cover_asked: false, paused_at: None });
    };
    let paused_at = match (old.reading.playing, reading.playing) {
        (_, true) => None,
        (true, false) => Some(now),
        (false, false) => old.paused_at,
    };
    if old.reading.track != reading.track {
        // Another video that comes paused is another tab's, found paused: a closed tab hands
        // the card back to whatever played before it. The same goes for a player macOS goes
        // back to when another quits.
        let paused_at = paused_at.filter(|_| !reading.player.reported() || reading.playing);
        return Some(Shown { reading, since_ms: now_ms, artwork: None, cover_asked: false, paused_at });
    }
    let expected = old.reading.position_now(reading.at);
    let moved = match (expected, reading.position) {
        (Some(expected), Some(position)) => (expected - position).abs() > DRIFT,
        (None, None) => false,
        _ => true,
    };
    let same = |a: &Reading, b: &Reading| {
        (&a.title, &a.artist, &a.album, a.playing, a.duration, a.shuffle, &a.repeat, a.volume, &a.artwork_url, (&a.app, a.previous, a.next))
            == (&b.title, &b.artist, &b.album, b.playing, b.duration, b.shuffle, &b.repeat, b.volume, &b.artwork_url, (&b.app, b.previous, b.next))
    };
    let reading = if moved || !same(&old.reading, &reading) { reading } else { old.reading.clone() };
    Some(Shown { reading, paused_at, ..old })
}

/// Whether the pill shows: while it plays, and a while after a pause.
fn pill_shows(shown: &Shown) -> bool {
    shown.reading.playing || shown.paused_at.is_some_and(|at| at.elapsed() < PAUSED_PILL)
}

fn settle(hub: &mut Hub) {
    let current = hub.shown.as_ref().map(|shown| shown.reading.player);
    let reported = shown_reported(hub.slots[3].read.as_ref(), &hub.watched);
    let reads = [hub.slots[0].read.as_ref(), hub.slots[1].read.as_ref(), hub.slots[2].read.as_ref(), reported];
    let pick = choose(reads, current);
    hub.shown = follow(hub.shown.take(), pick, surface::now_ms(), Instant::now());
    if let (Some(shown), Some(dir)) = (hub.shown.as_mut(), hub.covers.clone()) {
        if !shown.cover_asked {
            shown.cover_asked = true;
            let reading = shown.reading.clone();
            tauri::async_runtime::spawn(async move {
                let path = artwork::cover(&dir, &reading).await;
                let mut hub = self::hub();
                if let Some(shown) = hub.shown.as_mut().filter(|shown| shown.reading.player == reading.player && shown.reading.track == reading.track) {
                    shown.artwork = path.map(|path| path.to_string_lossy().into_owned());
                }
                publish(&mut hub);
            });
        }
    }
    publish(hub);
}

// ----- The island -----

fn now_playing(shown: &Shown) -> NowPlaying {
    let reading = &shown.reading;
    NowPlaying {
        player: reading.player.id().into(),
        title: reading.title.clone(),
        artist: reading.artist.clone(),
        album: reading.album.clone(),
        artwork: shown.artwork.clone(),
        playing: reading.playing,
        position: reading.position,
        duration: reading.duration,
        shuffle: reading.shuffle,
        repeat_mode: reading.repeat.clone(),
        volume: reading.volume,
        position_at: reading.position.map(|_| reading.at),
        can_previous: reading.previous,
        can_next: reading.next,
        app: reading.app.clone().filter(|_| reading.player == Player::Other),
    }
}

/// Whether the cat dances to it: to music, not to a film, a video in a browser, a podcast
/// or a book.
fn dances(reading: &Reading) -> bool {
    reading.playing
        && match reading.player {
            Player::Spotify | Player::Music => true,
            Player::Tv | Player::Browser => false,
            Player::Other => reading.app.as_deref().is_some_and(system::plays_music),
        }
}

/// What the island gets: the card, shown when the island is opened, and while it plays the
/// pill in the player's colour.
fn contribution(shown: &Shown, pill: bool, progress: Option<f64>) -> Contribution {
    let reading = &shown.reading;
    let pills = pill
        .then(|| Pill {
            id: CARD.into(),
            source: Source::Media,
            label: reading.title.clone(),
            tint: reading.player.tint().into(),
            state: if reading.playing { PillState::Working } else { PillState::Idle },
            badge: reading.artist.clone(),
            progress,
            card: None,
        })
        .into_iter()
        .collect();
    Contribution {
        pills,
        offers: vec![Offer {
            card: Card { id: CARD.into(), source: Source::Media, body: CardBody::NowPlaying(now_playing(shown)) },
            urgency: Urgency::Info,
            at_ms: shown.since_ms,
            ttl_ms: None,
        }],
    }
}

/// Publishes what changed, if anything did, and lets the cat dance to it.
fn publish(hub: &mut Hub) {
    let next = match hub.shown.as_ref() {
        Some(shown) => {
            let progress = shown.reading.progress(unix_now());
            // The pill's progress moves in steps, so the island isn't redrawn every second.
            let progress = match (hub.pill_progress, progress) {
                (Some(last), Some(now)) if (now - last).abs() < PILL_STEP && shown.reading.playing => Some(last),
                (_, now) => now,
            };
            hub.pill_progress = progress;
            contribution(shown, pill_shows(shown), progress)
        }
        None => {
            hub.pill_progress = None;
            Contribution::default()
        }
    };
    if hub.published.as_ref() != Some(&next) {
        surface::publish(Source::Media, next.clone());
        hub.published = Some(next);
    }
    let dancing = hub.shown.as_ref().is_some_and(|shown| dances(&shown.reading));
    if dancing != hub.dancing {
        hub.dancing = dancing;
        crate::island::media_playing(dancing);
    }
}

// ----- The card's controls -----

/// Runs the card's controls one at a time. Of several seeks or volume changes in a row, only
/// the last is sent.
async fn controls(mut receiver: mpsc::UnboundedReceiver<Action>) {
    while let Some(first) = receiver.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = receiver.try_recv() {
            batch.push(more);
        }
        for action in coalesce(batch) {
            if let Err(problem) = act(&action).await {
                eprintln!("A music control didn't go through: {problem}");
            }
        }
    }
}

fn coalesce(batch: Vec<Action>) -> Vec<Action> {
    let mut kept: Vec<Action> = Vec::new();
    for action in batch {
        if matches!(action.name.as_str(), "seek" | "volume") {
            kept.retain(|earlier| earlier.name != action.name);
        }
        kept.push(action);
    }
    kept
}

async fn act(action: &Action) -> Result<(), String> {
    let Some(player) = hub().shown.as_ref().map(|shown| shown.reading.player) else { return Ok(()) };
    if player.reported() {
        return act_reported(player, action).await;
    }
    if !player.running() {
        refresh(player);
        return Ok(());
    }
    let number = action.value.as_ref().and_then(Value::as_f64).filter(|value| value.is_finite());
    match action.name.as_str() {
        name @ ("play" | "pause") => {
            let playing = name == "play";
            adjust(player, |reading| reading.playing = playing);
            player::control(player, name).await?;
        }
        name @ ("next" | "previous") => player::control(player, name).await?,
        "seek" => {
            let duration = hub().shown.as_ref().and_then(|shown| shown.reading.duration).unwrap_or(f64::MAX);
            let to = number.ok_or("No position came with it.")?.clamp(0.0, duration);
            adjust(player, |reading| reading.position = Some(to));
            player::set(player, "seek", &((to * 1000.0).round() as i64).to_string()).await?;
        }
        "shuffle" if player != Player::Tv => {
            let on = action.value.as_ref().and_then(Value::as_bool).ok_or("No choice came with it.")?;
            adjust(player, |reading| reading.shuffle = Some(on));
            player::set(player, "shuffle", if on { "true" } else { "false" }).await?;
        }
        "repeat" if player != Player::Tv => {
            let mode = match (player, action.value.as_ref().and_then(Value::as_str)) {
                (Player::Spotify, Some("one" | "all")) => "all",
                (_, Some(mode @ ("off" | "one" | "all"))) => mode,
                _ => return Err("That isn't a repeat mode.".into()),
            };
            adjust(player, |reading| reading.repeat = Some(mode.into()));
            player::set(player, "repeat", mode).await?;
        }
        "volume" => {
            let level = number.ok_or("No volume came with it.")?.clamp(0.0, 1.0);
            adjust(player, |reading| reading.volume = Some(level));
            player::set(player, "volume", &((level * 100.0).round() as i64).to_string()).await?;
        }
        // The artwork, the title or the pill: the player itself, in front.
        "open" | "pill" => {
            if let Some(bundle) = player.bundle() {
                let _ = tokio::process::Command::new("/usr/bin/open").args(["-b", bundle]).status().await;
            }
            return Ok(());
        }
        _ => return Ok(()),
    }
    refresh(player);
    Ok(())
}

/// Play, pause, skipping and seeking reach a browser or another player the way a media key
/// does; the watcher says what came of it.
async fn act_reported(player: Player, action: &Action) -> Result<(), String> {
    let (app, duration) = {
        let hub = hub();
        let reading = hub.shown.as_ref().map(|shown| &shown.reading);
        (reading.and_then(|reading| reading.app.clone()), reading.and_then(|reading| reading.duration))
    };
    let Some(app) = app.filter(|app| system::kind(app) == Some(player)) else { return Ok(()) };
    let number = action.value.as_ref().and_then(Value::as_f64).filter(|value| value.is_finite());
    let sent = match action.name.as_str() {
        name @ ("play" | "pause") => {
            let playing = name == "play";
            adjust(player, |reading| reading.playing = playing);
            system::control(name, None, &app).await
        }
        name @ ("next" | "previous") => system::control(name, None, &app).await,
        "seek" => {
            let to = number.ok_or("No position came with it.")?.clamp(0.0, duration.unwrap_or(f64::MAX));
            adjust(player, |reading| reading.position = Some(to));
            system::control("seek", Some(to), &app).await
        }
        // The app, in front. Which tab plays isn't known without asking the browser.
        "open" | "pill" => {
            let _ = tokio::process::Command::new("/usr/bin/open").args(["-b", &app]).status().await;
            return Ok(());
        }
        _ => return Ok(()),
    };
    await_answer();
    sent
}

/// A control the app ignores leaves nothing for the watcher to say, so after a moment the
/// card goes back to what macOS last said. A newer control waits on its own.
fn await_answer() {
    let control = {
        let mut hub = hub();
        hub.reported_controls += 1;
        hub.reported_controls
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(ANSWER).await;
        let mut hub = hub();
        if hub.reported_controls != control {
            return;
        }
        let heard = hub.heard.clone();
        hub.slots[Player::Browser.index()].read = heard;
        settle(&mut hub);
    });
}

/// Shows a control's effect at once, before the player confirms it.
fn adjust(player: Player, change: impl Fn(&mut Reading)) {
    let mut hub = hub();
    let now = unix_now();
    let update = |reading: &mut Reading| {
        reading.position = reading.position_now(now);
        reading.at = now;
        change(reading);
    };
    if let Some(shown) = hub.shown.as_mut().filter(|shown| shown.reading.player == player) {
        let was_playing = shown.reading.playing;
        update(&mut shown.reading);
        match (was_playing, shown.reading.playing) {
            (true, false) => shown.paused_at = Some(Instant::now()),
            (_, true) => shown.paused_at = None,
            _ => {}
        }
    }
    if let Some(Read::Track(reading)) = hub.slots[player.index()].read.as_mut() {
        update(reading);
    }
    publish(&mut hub);
}

/// Asks the player again at once.
fn refresh(player: Player) {
    hub().slots[player.index()].force = true;
    WAKE.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(player: Player, track: &str, playing: bool, position: f64, at: f64) -> Reading {
        Reading {
            player,
            playing,
            title: track.into(),
            artist: Some("Daft Punk".into()),
            album: None,
            duration: Some(300.0),
            position: Some(position),
            shuffle: Some(false),
            repeat: Some("off".into()),
            volume: Some(0.5),
            artwork_url: None,
            track: track.into(),
            at,
            app: None,
            previous: None,
            next: None,
        }
    }

    fn shown(reading: Reading) -> Shown {
        Shown { reading, since_ms: 10, artwork: Some("/covers/a.jpg".into()), cover_asked: true, paused_at: None }
    }

    #[test]
    fn the_player_that_plays_is_shown_and_the_shown_one_wins_a_tie() {
        let spotify = Read::Track(reading(Player::Spotify, "a", false, 0.0, 0.0));
        let music = Read::Track(reading(Player::Music, "b", true, 0.0, 0.0));
        let picked = choose([Some(&spotify), Some(&music), None, None], None).unwrap();
        assert_eq!(picked.player, Player::Music);
        let both = Read::Track(reading(Player::Spotify, "a", true, 0.0, 0.0));
        assert_eq!(choose([Some(&both), Some(&music), None, None], Some(Player::Music)).unwrap().player, Player::Music);
        assert_eq!(choose([Some(&both), Some(&music), None, None], None).unwrap().player, Player::Spotify);
        // Nothing plays: the one shown stays.
        let paused = Read::Track(reading(Player::Music, "b", false, 0.0, 0.0));
        assert_eq!(choose([Some(&spotify), Some(&paused), None, None], Some(Player::Music)).unwrap().player, Player::Music);
        assert_eq!(choose([Some(&Read::Closed), Some(&Read::Stopped), None, Some(&Read::Closed)], Some(Player::Music)), None);
    }

    #[test]
    fn a_read_that_agrees_with_the_clock_changes_nothing() {
        let now = Instant::now();
        let before = shown(reading(Player::Spotify, "a", true, 60.0, 1_000.0));
        let after = follow(Some(before.clone()), Some(reading(Player::Spotify, "a", true, 65.3, 1_005.0)), 99, now).unwrap();
        assert_eq!(after, before);
        // Moved by hand in the player: drawn again from the new place.
        let moved = follow(Some(before.clone()), Some(reading(Player::Spotify, "a", true, 120.0, 1_005.0)), 99, now).unwrap();
        assert_eq!(moved.reading.position, Some(120.0));
        assert_eq!((moved.since_ms, moved.artwork.as_deref()), (10, Some("/covers/a.jpg")));
    }

    #[test]
    fn a_new_track_takes_a_new_place_and_asks_for_its_cover() {
        let before = shown(reading(Player::Spotify, "a", true, 60.0, 1_000.0));
        let after = follow(Some(before), Some(reading(Player::Spotify, "b", true, 0.0, 1_010.0)), 99, Instant::now()).unwrap();
        assert_eq!((after.since_ms, after.artwork, after.cover_asked), (99, None, false));
        assert_eq!(follow(None, None, 0, Instant::now()), None);
    }

    #[test]
    fn the_pill_stays_a_while_after_a_pause_but_not_for_a_player_found_paused() {
        let now = Instant::now();
        let playing = shown(reading(Player::Music, "a", true, 10.0, 0.0));
        assert!(pill_shows(&playing));
        let paused = follow(Some(playing), Some(reading(Player::Music, "a", false, 12.0, 2.0)), 0, now).unwrap();
        assert_eq!(paused.paused_at, Some(now));
        assert!(pill_shows(&paused));
        let long_ago = Shown { paused_at: now.checked_sub(PAUSED_PILL + Duration::from_secs(1)), ..paused };
        assert!(!pill_shows(&long_ago));
        let found = follow(None, Some(reading(Player::Music, "a", false, 12.0, 2.0)), 0, now).unwrap();
        assert!(!pill_shows(&found));
    }

    #[test]
    fn the_island_gets_a_card_and_a_tinted_pill() {
        let shown = shown(reading(Player::Spotify, "Instant Crush", true, 60.0, 1_000.0));
        let given = contribution(&shown, true, Some(0.2));
        let pill = &given.pills[0];
        assert_eq!((pill.tint.as_str(), pill.state, pill.progress), ("#1ED760", PillState::Working, Some(0.2)));
        let offer = &given.offers[0];
        assert_eq!((offer.urgency, offer.at_ms, offer.ttl_ms), (Urgency::Info, 10, None));
        let CardBody::NowPlaying(card) = &offer.card.body else { panic!() };
        assert_eq!((card.position, card.position_at), (Some(60.0), Some(1_000.0)));
        assert!(contribution(&shown, false, None).pills.is_empty());
    }

    #[test]
    fn a_browser_video_gets_its_own_card_and_keeps_it_while_it_plays() {
        let video = Reading {
            artist: Some("Blender".into()),
            shuffle: None,
            repeat: None,
            volume: None,
            app: Some("com.google.Chrome".into()),
            previous: Some(false),
            next: Some(true),
            ..reading(Player::Browser, "Big Buck Bunny", true, 100.0, 1_000.0)
        };
        let given = contribution(&shown(video.clone()), true, Some(0.16));
        assert_eq!(given.pills[0].tint, "#5E9EFF");
        let CardBody::NowPlaying(card) = &given.offers[0].card.body else { panic!() };
        assert_eq!((card.player.as_str(), card.can_previous, card.can_next), ("browser", Some(false), Some(true)));
        assert_eq!((card.shuffle, card.repeat_mode.as_deref(), card.volume), (None, None, None));
        // Music that starts playing while the video plays doesn't take its place, and the
        // video comes back to the card when Music is all that's paused.
        let browser = Read::Track(video.clone());
        let music = Read::Track(reading(Player::Music, "b", true, 0.0, 0.0));
        assert_eq!(choose([None, Some(&music), None, Some(&browser)], Some(Player::Browser)).unwrap().player, Player::Browser);
        let paused = Read::Track(reading(Player::Music, "b", false, 0.0, 0.0));
        assert_eq!(choose([None, Some(&paused), None, Some(&browser)], Some(Player::Music)).unwrap().player, Player::Browser);
        // A playlist that stops taking "next" is drawn again.
        let changed = follow(Some(shown(video.clone())), Some(Reading { next: Some(false), ..video.clone() }), 99, Instant::now()).unwrap();
        assert_eq!(changed.reading.next, Some(false));
        // Its tab closed and macOS went back to another tab's paused video: no pill for that,
        // though pausing the video itself keeps one.
        let other = Reading { playing: false, title: "Sintel".into(), track: "Sintel".into(), ..video.clone() };
        assert!(!pill_shows(&follow(Some(shown(video.clone())), Some(other), 99, Instant::now()).unwrap()));
        let paused = Reading { playing: false, ..video.clone() };
        assert!(pill_shows(&follow(Some(shown(video.clone())), Some(paused), 99, Instant::now()).unwrap()));
        // Only while its browser is switched on in Settings → Apps; "Other players" isn't it.
        assert!(shown_reported(Some(&browser), &watched(&["chrome"], false)).is_some());
        assert!(shown_reported(Some(&browser), &watched(&["safari", "arc"], true)).is_none());
        assert!(shown_reported(Some(&browser), &watched(&[], false)).is_none());
        let brave = Read::Track(Reading { app: Some("com.brave.Browser".into()), ..video });
        assert!(shown_reported(Some(&brave), &watched(&["brave"], false)).is_some());
    }

    fn watched(browsers: &[&'static str], others: bool) -> Watched {
        Watched { browsers: browsers.to_vec(), others }
    }

    fn podcast(playing: bool) -> Reading {
        Reading {
            artist: Some("The New York Times".into()),
            shuffle: None,
            repeat: None,
            volume: None,
            app: Some("com.apple.podcasts".into()),
            previous: Some(false),
            next: Some(false),
            ..reading(Player::Other, "The Daily", playing, 42.0, 1_000.0)
        }
    }

    #[test]
    fn another_player_shows_while_other_players_is_switched_on_with_its_app() {
        let episode = Read::Track(podcast(true));
        assert!(shown_reported(Some(&episode), &watched(&[], true)).is_some());
        assert!(shown_reported(Some(&episode), &watched(&["chrome", "safari"], false)).is_none());
        // Switching every browser and "Other players" off stops the watcher.
        assert!(!watched(&[], false).any());
        assert!(watched(&[], true).any() && watched(&["arc"], false).any());
        let given = contribution(&shown(podcast(true)), true, Some(0.02));
        assert_eq!(given.pills[0].tint, "#BF5AF2");
        let CardBody::NowPlaying(card) = &given.offers[0].card.body else { panic!() };
        assert_eq!((card.player.as_str(), card.app.as_deref()), ("other", Some("com.apple.podcasts")));
        // Only another player's card names its app; a browser's doesn't.
        let video = Reading { app: Some("com.google.Chrome".into()), ..reading(Player::Browser, "Sintel", true, 0.0, 0.0) };
        let CardBody::NowPlaying(card) = &contribution(&shown(video), true, None).offers[0].card.body else { panic!() };
        assert_eq!(card.app, None);
        // Music that plays doesn't give way to a podcast that starts later, and a podcast
        // found paused after another app quit gets no pill.
        let music = Read::Track(reading(Player::Music, "b", true, 0.0, 0.0));
        assert_eq!(choose([None, Some(&music), None, Some(&episode)], Some(Player::Music)).unwrap().player, Player::Music);
        let found = follow(Some(shown(reading(Player::Other, "Earlier", true, 0.0, 0.0))), Some(podcast(false)), 99, Instant::now()).unwrap();
        assert!(!pill_shows(&found));
    }

    #[test]
    fn the_cat_dances_to_music_only() {
        assert!(dances(&reading(Player::Spotify, "a", true, 0.0, 0.0)));
        assert!(!dances(&reading(Player::Music, "a", false, 0.0, 0.0)));
        assert!(!dances(&reading(Player::Tv, "a", true, 0.0, 0.0)));
        assert!(!dances(&podcast(true)));
        assert!(dances(&Reading { app: Some("com.tidal.desktop".into()), ..podcast(true) }));
        assert!(!dances(&Reading { app: Some("com.google.Chrome".into()), ..reading(Player::Browser, "a", true, 0.0, 0.0) }));
    }

    /// The real players, on this Mac: `cargo test --lib media::tests::live_players -- --ignored
    /// --nocapture`. Reads each open player, and on one that has a track, changes volume,
    /// position, shuffle, repeat and play/pause, checking each and putting it back. A player
    /// that's closed stays closed, and one macOS would have to ask about is left alone, so it
    /// never shows a prompt: run it from a terminal already allowed to control them.
    #[tokio::test]
    #[ignore]
    async fn live_players() {
        let pause = || tokio::time::sleep(Duration::from_millis(600));
        let now = |player: Player| async move {
            match player::read(player).await.expect("read") {
                Read::Track(reading) => reading,
                other => panic!("{other:?}"),
            }
        };
        for player in Player::SCRIPTED {
            if !player.running() {
                println!("{}: not open, left closed", player.app());
                assert!(!player.running());
                continue;
            }
            if !crate::apps::may_script_silently(player.app()).await {
                println!("{}: macOS would ask first, so left alone", player.app());
                continue;
            }
            let Read::Track(before) = player::read(player).await.expect("read") else {
                println!("{}: nothing loaded", player.app());
                continue;
            };
            println!("{}: {before:?}", player.app());
            let dir = tempfile::tempdir().unwrap();
            let cover = artwork::cover(dir.path(), &before).await;
            println!("cover: {cover:?} ({} bytes)", cover.as_ref().and_then(|path| std::fs::metadata(path).ok()).map_or(0, |meta| meta.len()));

            player::set(player, "volume", "12").await.unwrap();
            pause().await;
            assert_eq!(now(player).await.volume.map(|volume| (volume * 100.0).round()), Some(12.0));
            if let Some(volume) = before.volume {
                player::set(player, "volume", &((volume * 100.0).round() as i64).to_string()).await.unwrap();
            }
            if let (Some(position), Some(duration)) = (before.position, before.duration) {
                let to = (position + 20.0).min(duration - 5.0).max(0.0);
                player::set(player, "seek", &((to * 1000.0) as i64).to_string()).await.unwrap();
                pause().await;
                let moved = now(player).await.position.unwrap();
                assert!((moved - to).abs() < 2.0, "{moved} after seeking to {to}");
                player::set(player, "seek", &((position * 1000.0) as i64).to_string()).await.unwrap();
            }
            if let Some(shuffle) = before.shuffle {
                player::set(player, "shuffle", if shuffle { "false" } else { "true" }).await.unwrap();
                pause().await;
                assert_eq!(now(player).await.shuffle, Some(!shuffle));
                player::set(player, "shuffle", if shuffle { "true" } else { "false" }).await.unwrap();
            }
            if let Some(repeat) = before.repeat.clone() {
                let other = if repeat == "off" { "all" } else { "off" };
                player::set(player, "repeat", other).await.unwrap();
                pause().await;
                assert_eq!(now(player).await.repeat.as_deref(), Some(other));
                player::set(player, "repeat", &repeat).await.unwrap();
            }
            let (first, second) = if before.playing { ("pause", "play") } else { ("play", "pause") };
            player::control(player, first).await.unwrap();
            pause().await;
            assert_eq!(now(player).await.playing, !before.playing);
            player::control(player, second).await.unwrap();
            pause().await;
            let after = now(player).await;
            assert_eq!((after.playing, after.track.as_str()), (before.playing, before.track.as_str()));
            println!("{}: every control answered and was put back", player.app());
        }
    }

    /// What a browser or another player plays on this Mac, as the island gets it: `cargo test
    /// --lib media::tests::live_reported -- --ignored --nocapture`. Runs the real watcher for
    /// `BUNTINI_LIVE_SECONDS` (60 unless set) and prints the card and pill each time they
    /// change, with the seconds since it started and the Unix time: play, pause, change videos
    /// or close the tab meanwhile. With `BUNTINI_LIVE_CONTROLS=1` it also sends the card's
    /// pause, a seek 20 seconds back and play to the first thing that plays whose title has
    /// `BUNTINI_LIVE_TITLE` in it, timing each answer. Nothing here asks the app anything,
    /// so it never shows a prompt.
    #[tokio::test]
    #[ignore]
    async fn live_reported() {
        let seconds = std::env::var("BUNTINI_LIVE_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(60);
        let controls = std::env::var("BUNTINI_LIVE_CONTROLS").is_ok_and(|value| value == "1");
        let only = std::env::var("BUNTINI_LIVE_TITLE").unwrap_or_default();
        let start = Instant::now();
        let mut watcher = system::Watcher::start().expect("the watcher starts");
        let mut shown: Option<Shown> = None;
        let mut last = String::new();
        // Settles what was heard as the hub does, printing what changes.
        let mut follow_until = |shown: &mut Option<Shown>, heard: Read| {
            let pick = choose([None, None, None, Some(&heard)], shown.as_ref().map(|shown| shown.reading.player));
            *shown = follow(shown.take(), pick, surface::now_ms(), Instant::now());
            let given = shown.as_ref().map(|shown| contribution(shown, pill_shows(shown), shown.reading.progress(unix_now())));
            let given = given.map(|given| serde_json::json!({"pill": given.pills.first(), "card": given.offers.first().map(|offer| &offer.card)}));
            let line = serde_json::to_string(&given).unwrap();
            if line != last {
                println!("{:6.2}s {:.2} {line}", start.elapsed().as_secs_f64(), unix_now());
                last = line;
            }
        };
        let deadline = start + Duration::from_secs(seconds);
        let mut controlled = !controls;
        while let Ok(Some(heard)) = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), watcher.next()).await {
            let system::Heard::Read(read) = heard else { panic!("macOS doesn't say what plays here") };
            follow_until(&mut shown, read);
            let Some(reading) = shown.as_ref().map(|shown| shown.reading.clone()).filter(|reading| reading.playing && !controlled && reading.title.contains(&only)) else { continue };
            controlled = true;
            for (what, to, want) in [
                ("pause", None, Box::new(|reading: &Reading| !reading.playing) as Box<dyn Fn(&Reading) -> bool>),
                ("seek", reading.position_now(unix_now()).map(|at| (at - 20.0).max(0.0)), Box::new(|_: &Reading| true)),
                ("play", None, Box::new(|reading: &Reading| reading.playing)),
            ] {
                let sent = Instant::now();
                system::control(what, to, reading.app.as_deref().unwrap_or_default()).await.unwrap_or_else(|problem| panic!("{what}: {problem}"));
                loop {
                    let heard = tokio::time::timeout(Duration::from_secs(5), watcher.next()).await;
                    let Ok(Some(system::Heard::Read(read))) = heard else { panic!("{what}: no answer within 5 s") };
                    follow_until(&mut shown, read);
                    let now = shown.as_ref().map(|shown| shown.reading.clone()).expect("still shown");
                    let moved = to.is_none_or(|to| now.position.is_some_and(|at| (at - to).abs() < 2.0));
                    if want(&now) && moved {
                        println!("{what}: answered in {} ms, on the card at {:.2}", sent.elapsed().as_millis(), unix_now());
                        break;
                    }
                }
            }
        }
        assert!(controlled, "nothing played to try the controls on");
    }

    #[test]
    fn only_the_last_of_several_seeks_is_sent() {
        let action = |name: &str, value: f64| Action { source: Source::Media, card: Some(CARD.into()), name: name.into(), value: Some(value.into()) };
        let batch = vec![action("seek", 10.0), action("volume", 0.2), action("seek", 20.0), action("next", 0.0), action("volume", 0.4)];
        let kept: Vec<(String, f64)> = coalesce(batch).into_iter().map(|a| (a.name, a.value.unwrap().as_f64().unwrap())).collect();
        assert_eq!(kept, [("seek".into(), 20.0), ("next".into(), 0.0), ("volume".into(), 0.4)]);
    }
}
