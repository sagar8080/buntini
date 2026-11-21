//! Recording the screen. "Record my screen" starts it, with a red mark and the minutes beside
//! the notch and Stop on its card; stopping keeps the film in Movies, and the card offers to
//! open it, show it in Finder or drag it out. Apple's `screencapture` records, as it takes
//! Buntini's screenshots, under the same Screen Recording permission, asked for once. Nothing
//! is sent anywhere.
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::surface::cards::utilities::Recording as RecordingCard;
use crate::surface::{self, Card, CardBody, Offer, Pill, PillState, Source, Urgency};

pub const ID: &str = "utilities.recording";
const TINT: &str = "#FF453A";
/// How long the finished film's card stays.
const DONE_MS: u64 = 2 * 60 * 1000;
/// How long `screencapture` gets to finish the file after Stop.
const FINISHING: Duration = Duration::from_secs(15);

#[derive(Debug, PartialEq)]
pub enum Command {
    Start,
    Stop,
}

/// "Record my screen", "start a screen recording"; "stop recording", "stop the screen recording".
pub fn recognise(text: &str) -> Option<Command> {
    let line = text.split_whitespace().filter(|word| !["please", "can", "you", "could", "buntini", "hey", "now"].contains(word)).collect::<Vec<_>>().join(" ");
    let start = ["record my screen", "record the screen", "record screen", "start a screen recording", "start screen recording", "start recording my screen", "start recording the screen", "screen record", "make a screen recording"];
    let stop = ["stop recording", "stop the recording", "stop the screen recording", "stop screen recording", "stop recording my screen", "stop recording the screen", "end the screen recording"];
    if start.contains(&line.as_str()) {
        Some(Command::Start)
    } else if stop.contains(&line.as_str()) {
        Some(Command::Stop)
    } else {
        None
    }
}

struct Running {
    child: Child,
    path: PathBuf,
    /// When it started, in Unix seconds, and on the board's clock.
    since: i64,
    since_board: u64,
    started: Instant,
}

struct Done {
    path: PathBuf,
    seconds: f64,
    at: u64,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    done: Option<Done>,
}

static STATE: Mutex<State> = Mutex::new(State { running: None, done: None });

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where a film started at `at` goes: Movies, named for when, never over another.
pub fn film_path(movies: &Path, at: chrono::DateTime<chrono::Local>) -> PathBuf {
    let stem = format!("Screen Recording {}", at.format("%Y-%m-%d at %H.%M.%S"));
    let mut path = movies.join(format!("{stem}.mov"));
    let mut n = 2;
    while path.exists() {
        path = movies.join(format!("{stem} {n}.mov"));
        n += 1;
    }
    path
}

/// Starts recording the screen. False when Screen Recording isn't allowed (the island says so).
pub fn start() -> Result<bool, String> {
    if state().running.is_some() {
        return Ok(true);
    }
    if !super::capture::allowed() {
        super::capture::blocked();
        return Ok(false);
    }
    let movies = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Movies")).ok_or("There's no home folder.")?;
    std::fs::create_dir_all(&movies).map_err(|error| error.to_string())?;
    let path = film_path(&movies, chrono::Local::now());
    let child = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-v", "-x"])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "The recording didn't start.".to_string())?;
    state().running = Some(Running { child, path, since: chrono::Utc::now().timestamp(), since_board: surface::now_ms(), started: Instant::now() });
    super::publish();
    // The pill's minutes move on.
    std::thread::spawn(|| {
        while state().running.is_some() {
            std::thread::sleep(Duration::from_secs(30));
            super::publish();
        }
    });
    Ok(true)
}

/// Stops recording; the film is finished in the background and its card shows. False when
/// nothing was being recorded.
pub fn stop() -> bool {
    let Some(mut running) = state().running.take() else { return false };
    super::publish();
    std::thread::spawn(move || {
        // SAFETY: the pid is our own child's, which hasn't been waited for yet.
        unsafe { libc::kill(running.child.id() as i32, libc::SIGINT) };
        let deadline = Instant::now() + FINISHING;
        let finished = loop {
            match running.child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                _ => {
                    let _ = running.child.kill();
                    let _ = running.child.wait();
                    break false;
                }
            }
        };
        if finished && running.path.is_file() {
            state().done = Some(Done { path: running.path, seconds: running.started.elapsed().as_secs_f64(), at: surface::now_ms() });
            super::publish();
        } else {
            eprintln!("The screen recording didn't finish its file.");
        }
    });
    true
}

/// The card's buttons.
pub fn act(name: &str) -> Result<(), String> {
    match name {
        "stop" => {
            stop();
            Ok(())
        }
        name @ ("open" | "show") => {
            let path = state().done.as_ref().map(|done| done.path.clone()).ok_or("The recording isn't here any more.")?;
            let mut command = std::process::Command::new("/usr/bin/open");
            if name == "show" {
                command.arg("-R");
            }
            command.arg(path).spawn().map(|_| ()).map_err(|error| error.to_string())
        }
        _ => Ok(()),
    }
}

/// The red mark while recording, and the card: Stop while recording, then the film.
pub fn shown() -> (Option<Pill>, Option<Offer>) {
    let mut state = state();
    let board = surface::now_ms();
    state.done = state.done.take().filter(|done| board < done.at + DONE_MS);
    if let Some(running) = &state.running {
        let pill = Pill {
            id: ID.into(),
            source: Source::Utilities,
            label: t("surface.pill.recording"),
            tint: TINT.into(),
            state: PillState::Working,
            badge: Some(super::focus::badge(running.started.elapsed().as_millis() as i64)),
            progress: None,
            card: None,
        };
        let card = Card { id: ID.into(), source: Source::Utilities, body: CardBody::Recording(RecordingCard { since: Some(running.since), path: None, name: None, seconds: None }) };
        return (Some(pill), Some(Offer { card, urgency: Urgency::Info, at_ms: running.since_board, ttl_ms: None }));
    }
    let offer = state.done.as_ref().map(|done| {
        let name = done.path.file_name().map(|name| name.to_string_lossy().into_owned());
        let card = Card { id: ID.into(), source: Source::Utilities, body: CardBody::Recording(RecordingCard { since: None, path: Some(done.path.to_string_lossy().into_owned()), name, seconds: Some(done.seconds) }) };
        Offer { card, urgency: Urgency::Alert, at_ms: done.at, ttl_ms: Some(DONE_MS) }
    });
    (None, offer)
}

/// A request from the chat.
pub fn run(command: Command) -> Result<crate::skills::Outcome, String> {
    let answer = match command {
        Command::Start => {
            if !start()? {
                return Err(t("utilities.recording.chat.allow"));
            }
            t("utilities.recording.chat.started")
        }
        Command::Stop => {
            if !stop() {
                return Err(t("utilities.recording.chat.none"));
            }
            t("utilities.recording.chat.stopped")
        }
    };
    Ok(crate::skills::Outcome::Done { answer, shares: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(text: &str) -> Option<Command> {
        recognise(&crate::skills::words(text))
    }

    #[test]
    fn recording_is_asked_for_in_plain_words() {
        for text in ["Record my screen", "Can you record the screen please?", "start a screen recording", "screen record"] {
            assert_eq!(ask(text), Some(Command::Start), "{text}");
        }
        for text in ["Stop recording", "stop the screen recording"] {
            assert_eq!(ask(text), Some(Command::Stop), "{text}");
        }
        for text in ["how do I record my screen", "record my screen and send it to Ana", "record a call", "recording"] {
            assert_eq!(ask(text), None, "{text}");
        }
    }

    #[test]
    fn a_film_is_named_for_when_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 21, 9, 15, 2).unwrap();
        let first = film_path(dir.path(), at);
        assert_eq!(first.file_name().unwrap(), "Screen Recording 2026-11-21 at 09.15.02.mov");
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(film_path(dir.path(), at).file_name().unwrap(), "Screen Recording 2026-11-21 at 09.15.02 2.mov");
    }

    use chrono::TimeZone;

    /// A real recording of a few seconds, through the same commands, kept nowhere: `cargo test
    /// --lib utilities::recording::tests::live -- --ignored --nocapture`. Needs Screen Recording
    /// already allowed for the process running it, so it never asks.
    #[test]
    #[ignore]
    fn live() {
        assert!(super::super::native::capture_allowed(false), "Screen Recording isn't allowed for this process");
        let dir = tempfile::tempdir().unwrap();
        let path = film_path(dir.path(), chrono::Local::now());
        let child = std::process::Command::new("/usr/sbin/screencapture").args(["-v", "-x"]).arg(&path).spawn().unwrap();
        state().running = Some(Running { child, path: path.clone(), since: 0, since_board: 0, started: Instant::now() });
        std::thread::sleep(Duration::from_secs(3));
        assert!(stop());
        let deadline = Instant::now() + Duration::from_secs(20);
        while state().done.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        let done = state().done.take().expect("the film finished");
        println!("{} bytes, {:.1} s", std::fs::metadata(&done.path).unwrap().len(), done.seconds);
        assert!(std::fs::metadata(&done.path).unwrap().len() > 10_000);
    }
}
