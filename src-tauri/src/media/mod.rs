//! Music, Spotify and TV in the island (stream F). What's playing is read only from a player
//! that is open, allowed in Settings → Apps, and that macOS already lets Buntini ask, so a
//! read never opens a player or puts a permission prompt on screen. The card's controls are
//! the person's own clicks and act at once. The cat dances while music plays.
mod artwork;
mod native;
mod player;

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

#[derive(Default)]
struct Hub {
    app: Option<AppHandle>,
    covers: Option<PathBuf>,
    slots: [Slot; 3],
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
static CONTROLS: OnceLock<mpsc::UnboundedSender<Action>> = OnceLock::new();

fn hub() -> MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What plays now, for other assistants: the app, the title, the artist and whether it plays.
pub fn playing_now() -> Option<(String, String, Option<String>, bool)> {
    let hub = hub();
    let reading = &hub.shown.as_ref()?.reading;
    Some((reading.player.app().to_string(), reading.title.clone(), reading.artist.clone(), reading.playing))
}

/// The display sleeps or the session is locked, so the island isn't on screen.
pub fn screen_away() -> bool {
    hub().asleep
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
    let allowed = allowed_players(&app);
    for player in Player::ALL {
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
    Player::ALL
        .into_iter()
        .map(|player| {
            let slot = &hub.slots[player.index()];
            if slot.force {
                return Duration::ZERO;
            }
            slot.at.map_or(Duration::ZERO, |at| cadence(&hub, player).saturating_sub(at.elapsed()))
        })
        .min()
        .unwrap_or(IDLE)
        .max(Duration::from_millis(200))
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

fn allowed_players(app: &AppHandle) -> Vec<Player> {
    let settings = app.try_state::<crate::controller::AppState>().and_then(|state| state.store.lock().ok()?.settings().ok());
    let Some(settings) = settings else { return Vec::new() };
    Player::ALL.into_iter().filter(|player| crate::apps::allowed(&settings, player.id())).collect()
}

/// Which track the card shows: one that plays, keeping the one shown; else the one shown,
/// paused; else any paused one.
fn choose(reads: [Option<&Read>; 3], current: Option<Player>) -> Option<Reading> {
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
        return Some(Shown { reading, since_ms: now_ms, artwork: None, cover_asked: false, paused_at });
    }
    let expected = old.reading.position_now(reading.at);
    let moved = match (expected, reading.position) {
        (Some(expected), Some(position)) => (expected - position).abs() > DRIFT,
        (None, None) => false,
        _ => true,
    };
    let same = |a: &Reading, b: &Reading| {
        (&a.title, &a.artist, &a.album, a.playing, a.duration, a.shuffle, &a.repeat, a.volume, &a.artwork_url)
            == (&b.title, &b.artist, &b.album, b.playing, b.duration, b.shuffle, &b.repeat, b.volume, &b.artwork_url)
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
    let reads = [hub.slots[0].read.as_ref(), hub.slots[1].read.as_ref(), hub.slots[2].read.as_ref()];
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
    // The cat dances to music, not to a film.
    let dancing = hub.shown.as_ref().is_some_and(|shown| shown.reading.playing && shown.reading.player != Player::Tv);
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
            let _ = tokio::process::Command::new("/usr/bin/open").args(["-b", player.bundle()]).status().await;
            return Ok(());
        }
        _ => return Ok(()),
    }
    refresh(player);
    Ok(())
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
        }
    }

    fn shown(reading: Reading) -> Shown {
        Shown { reading, since_ms: 10, artwork: Some("/covers/a.jpg".into()), cover_asked: true, paused_at: None }
    }

    #[test]
    fn the_player_that_plays_is_shown_and_the_shown_one_wins_a_tie() {
        let spotify = Read::Track(reading(Player::Spotify, "a", false, 0.0, 0.0));
        let music = Read::Track(reading(Player::Music, "b", true, 0.0, 0.0));
        let picked = choose([Some(&spotify), Some(&music), None], None).unwrap();
        assert_eq!(picked.player, Player::Music);
        let both = Read::Track(reading(Player::Spotify, "a", true, 0.0, 0.0));
        assert_eq!(choose([Some(&both), Some(&music), None], Some(Player::Music)).unwrap().player, Player::Music);
        assert_eq!(choose([Some(&both), Some(&music), None], None).unwrap().player, Player::Spotify);
        // Nothing plays: the one shown stays.
        let paused = Read::Track(reading(Player::Music, "b", false, 0.0, 0.0));
        assert_eq!(choose([Some(&spotify), Some(&paused), None], Some(Player::Music)).unwrap().player, Player::Music);
        assert_eq!(choose([Some(&Read::Closed), Some(&Read::Stopped), None], Some(Player::Music)), None);
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
        for player in Player::ALL {
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

    #[test]
    fn only_the_last_of_several_seeks_is_sent() {
        let action = |name: &str, value: f64| Action { source: Source::Media, card: Some(CARD.into()), name: name.into(), value: Some(value.into()) };
        let batch = vec![action("seek", 10.0), action("volume", 0.2), action("seek", 20.0), action("next", 0.0), action("volume", 0.4)];
        let kept: Vec<(String, f64)> = coalesce(batch).into_iter().map(|a| (a.name, a.value.unwrap().as_f64().unwrap())).collect();
        assert_eq!(kept, [("seek".into(), 20.0), ("next".into(), 0.0), ("volume".into(), 0.4)]);
    }
}
