//! Screenshots and captures. The whole screen goes to the clipboard with a thumbnail in the
//! corner; part of the screen goes to the island, which offers to paste, copy, mark up or ask
//! about it. Pictures are taken by /usr/sbin/screencapture into Buntini's cache folder, kept a
//! day for dragging out and pasting, and never sent anywhere unless you choose Ask. Dictation
//! pastes a capture only while its card is in front on the island and the dictation started
//! over it; never a whole-screen screenshot.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::surface::cards::utilities::{Capture, Overlay};
use crate::surface::{self, Card, CardBody, Offer, Source, Urgency};

/// How long a capture waits on the island for what to do with it.
const CARD_MS: u64 = 20_000;
/// Pictures older than this are removed.
const KEEP: Duration = Duration::from_secs(24 * 3600);
/// At most this many are kept.
const KEEP_COUNT: usize = 40;
pub const CARD_ID: &str = "utilities.capture";

/// A picture taken.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub path: PathBuf,
    pub width: i64,
    pub height: i64,
}

#[derive(Default)]
struct State {
    /// The thumbnail in the corner after a screenshot.
    thumbnail: Option<Shot>,
    /// A capture on the island, and when it was offered (board clock).
    card: Option<(Shot, u64)>,
    /// Screen Recording isn't allowed: the island asks for it.
    blocked: Option<u64>,
    /// The dimmer is up for choosing an area.
    selecting: bool,
    /// A picture is being taken.
    busy: bool,
}

static STATE: Mutex<State> = Mutex::new(State { thumbnail: None, card: None, blocked: None, selecting: false, busy: false });

fn state() -> std::sync::MutexGuard<'static, State> {
    let mut state = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A capture whose time on the island is up is gone, not merely hidden.
    let now = surface::now_ms();
    if state.card.as_ref().is_some_and(|(_, at_ms)| !live(*at_ms, now)) {
        state.card = None;
    }
    state
}

/// Whether a capture offered at `at_ms` is still on the island.
fn live(at_ms: u64, now_ms: u64) -> bool {
    now_ms < at_ms.saturating_add(CARD_MS)
}

/// The island's capture cards.
pub(super) fn offers() -> Vec<Offer> {
    let state = state();
    let mut offers = Vec::new();
    if let Some((shot, at_ms)) = &state.card {
        offers.push(Offer {
            card: Card { id: CARD_ID.into(), source: Source::Utilities, body: CardBody::Capture(Capture { path: shot.path.to_string_lossy().into(), width: shot.width, height: shot.height }) },
            urgency: Urgency::Alert,
            at_ms: *at_ms,
            ttl_ms: Some(CARD_MS),
        });
    }
    if let Some(at_ms) = state.blocked {
        // An empty path: Screen Recording isn't allowed yet.
        offers.push(Offer {
            card: Card { id: format!("{CARD_ID}.allow"), source: Source::Utilities, body: CardBody::Capture(Capture { path: String::new(), width: 0, height: 0 }) },
            urgency: Urgency::Alert,
            at_ms,
            ttl_ms: Some(CARD_MS),
        });
    }
    offers
}

fn show_overlay(state: &State) {
    let overlay = if state.selecting {
        Some(Overlay { kind: "select".into(), path: String::new(), width: 0, height: 0 })
    } else {
        state.thumbnail.as_ref().map(|shot| Overlay { kind: "thumbnail".into(), path: shot.path.to_string_lossy().into(), width: shot.width, height: shot.height })
    };
    surface::set_overlay(overlay);
}

fn changed(state: &mut State) {
    show_overlay(state);
}

/// Whether Buntini may record the screen. macOS asks once, at the first capture; after that a
/// card on the island leads to System Settings, where macOS offers to reopen Buntini once it's
/// allowed. Without it, screencapture would only see the desktop picture.
fn allowed() -> bool {
    if super::native::capture_allowed(false) {
        return true;
    }
    if super::preference(super::ASKED_RECORDING).is_none() {
        super::set_preference(super::ASKED_RECORDING, "true");
        super::native::capture_allowed(true);
    }
    false
}

fn blocked() {
    state().blocked = Some(surface::now_ms());
    super::publish();
}

/// ⌘⇧2: the screen with the pointer goes to the clipboard, with a thumbnail in the corner.
pub fn screenshot() {
    if !begin() {
        return;
    }
    std::thread::spawn(|| {
        let result = (|| -> Result<Shot, Option<String>> {
            if !allowed() {
                return Err(None);
            }
            // A thumbnail still showing would be in the picture.
            let had = {
                let mut state = state();
                let had = state.thumbnail.take().is_some();
                changed(&mut state);
                had
            };
            if had {
                std::thread::sleep(Duration::from_millis(120));
            }
            let rect = super::native::pointer_screen().ok_or(Some("No screen".to_string()))?;
            let shot = take(&rect, "Screenshot").map_err(Some)?;
            super::native::copy_image(&shot.path);
            Ok(shot)
        })();
        let mut state = state();
        state.busy = false;
        match result {
            Ok(shot) => {
                state.thumbnail = Some(shot);
                changed(&mut state);
            }
            Err(None) => {
                drop(state);
                blocked();
            }
            Err(Some(problem)) => eprintln!("A screenshot didn't work: {problem}"),
        }
    });
}

/// ⌘⇧S: dims the screen to choose an area.
pub fn select() {
    {
        let mut state = state();
        if state.selecting || state.busy {
            return;
        }
        if !allowed() {
            drop(state);
            blocked();
            return;
        }
        state.selecting = true;
        state.thumbnail = None;
        changed(&mut state);
    }
}

/// The area chosen, in points from the top left of the main screen.
pub fn selected(rect: [f64; 4]) {
    {
        let mut state = state();
        if !state.selecting {
            return;
        }
        state.selecting = false;
        state.busy = true;
        changed(&mut state);
    }
    std::thread::spawn(move || {
        // The dimmer is already gone; give the window server a frame to agree.
        std::thread::sleep(Duration::from_millis(60));
        let result = take(&rect, "Capture");
        let mut state = state();
        state.busy = false;
        match result {
            Ok(shot) => {
                state.card = Some((shot, surface::now_ms()));
                drop(state);
                super::publish();
            }
            Err(problem) => eprintln!("A capture didn't work: {problem}"),
        }
    });
}

pub fn cancelled() {
    let mut state = state();
    state.selecting = false;
    changed(&mut state);
}

fn begin() -> bool {
    let mut state = state();
    if state.busy || state.selecting {
        return false;
    }
    state.busy = true;
    true
}

/// The thumbnail was opened, dragged out, swiped away or left by itself.
pub fn thumbnail_gone() {
    let mut state = state();
    state.thumbnail = None;
    changed(&mut state);
}

fn card_path(card: Option<&str>) -> Option<PathBuf> {
    let state = state();
    let (shot, _) = state.card.as_ref()?;
    (card.is_none_or(|card| card == CARD_ID)).then(|| shot.path.clone())
}

fn forget_card() {
    state().card = None;
    super::publish();
}

/// What the person chose on a capture card.
pub fn act(name: &str, card: Option<&str>, value: Option<&serde_json::Value>) -> Result<(), String> {
    match name {
        "allowRecording" => {
            state().blocked = None;
            super::publish();
            super::native::open_recording_settings();
            Ok(())
        }
        "paste" | "copy" | "annotate" | "ask" | "dismiss" => {
            let path = card_path(card).ok_or("That capture is gone.")?;
            match name {
                "paste" => {
                    forget_card();
                    std::thread::spawn(move || {
                        if !super::native::paste_image(&path) {
                            super::native::copy_image(&path);
                        }
                    });
                }
                "copy" => {
                    forget_card();
                    super::native::copy_image(&path);
                }
                "annotate" => {
                    forget_card();
                    let out = marked_path(&path);
                    if !super::native::annotate(&path, &out) {
                        return Err("The editor didn’t open.".into());
                    }
                }
                "ask" => {
                    forget_card();
                    super::ask(&path);
                }
                _ => forget_card(),
            }
            Ok(())
        }
        // The editor saved a marked-up copy: offer it as the capture.
        "annotated" => {
            let path = value.and_then(serde_json::Value::as_str).map(PathBuf::from).ok_or("No picture came back.")?;
            if !path.starts_with(captures()) {
                return Err("That picture isn't Buntini's.".into());
            }
            let (width, height) = png_size(&path).ok_or("The marked-up picture didn't read.")?;
            state().card = Some((Shot { path, width, height }, surface::now_ms()));
            super::publish();
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The capture card's picture, when the card is in front on the island now.
fn in_front(card: Option<&(Shot, u64)>, front: Option<&str>, now_ms: u64) -> Option<PathBuf> {
    let (shot, at_ms) = card?;
    (front == Some(CARD_ID) && live(*at_ms, now_ms)).then(|| shot.path.clone())
}

/// The capture showing on the island, for a dictation starting now. A whole-screen
/// screenshot (the thumbnail) never counts.
pub fn showing() -> Option<PathBuf> {
    if !surface::running() {
        return None;
    }
    let front = surface::front_card();
    in_front(state().card.as_ref(), front.as_deref(), surface::now_ms())
}

/// The capture a dictation started over, if its card is still in front: it is pasted with the
/// words, and taken off the island.
pub fn for_dictation(started_over: &Path) -> Option<PathBuf> {
    let path = showing().filter(|path| path == started_over && path.exists())?;
    forget_card();
    Some(path)
}

/// Delete everything: no capture on the island or in the corner, and none kept.
pub fn erase(cache_dir: &Path) -> Result<(), String> {
    {
        let mut state = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = State::default();
        show_overlay(&state);
    }
    super::publish();
    match std::fs::remove_dir_all(cache_dir.join("captures")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err("Couldn’t remove the captures.".into()),
        _ => Ok(()),
    }
}

/// Pastes a capture where the caret is, keeping the clipboard as it was.
pub fn paste(path: &Path) -> bool {
    super::native::paste_image(path)
}

/// Where captures are kept: Buntini's cache folder.
pub fn captures() -> PathBuf {
    super::cache_dir().join("captures")
}

fn marked_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().trim_end_matches(" (marked up)").to_string()).unwrap_or_default();
    path.with_file_name(format!("{stem} (marked up).png"))
}

/// A new file name in the macOS style, "Screenshot 2026-11-16 at 10.22.31.png".
fn new_path(folder: &Path, kind: &str, now: chrono::DateTime<chrono::Local>) -> PathBuf {
    let base = format!("{kind} {}", now.format("%Y-%m-%d at %H.%M.%S"));
    let mut path = folder.join(format!("{base}.png"));
    let mut number = 2;
    while path.exists() {
        path = folder.join(format!("{base} ({number}).png"));
        number += 1;
    }
    path
}

/// Takes the rectangle `[x, y, width, height]` (points from the main screen's top left).
fn take(rect: &[f64; 4], kind: &str) -> Result<Shot, String> {
    let [x, y, width, height] = *rect;
    if !(width >= 2.0 && height >= 2.0 && x.is_finite() && y.is_finite() && width < 100_000.0 && height < 100_000.0) {
        return Err("That area is too small.".into());
    }
    let folder = captures();
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    prune(&folder, SystemTime::now());
    let path = new_path(&folder, kind, chrono::Local::now());
    let area = format!("-R{},{},{},{}", x.round() as i64, y.round() as i64, width.round() as i64, height.round() as i64);
    // No -x: the Mac's own shutter sound, as with ⇧⌘3, when its sound effects are on.
    let status = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-tpng", &area])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() || !path.exists() {
        return Err(format!("screencapture ended with {status}"));
    }
    let (width, height) = png_size(&path).ok_or("The picture didn't read.")?;
    Ok(Shot { path, width, height })
}

/// A PNG's size in pixels, from its header.
pub fn png_size(path: &Path) -> Option<(i64, i64)> {
    use std::io::Read;
    let mut header = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?) as i64;
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?) as i64;
    (width > 0 && height > 0).then_some((width, height))
}

/// Removes pictures older than a day, and all but the newest few.
fn prune(folder: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "png"))
        .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    for (index, (path, modified)) in files.into_iter().enumerate() {
        if index >= KEEP_COUNT || now.duration_since(modified).is_ok_and(|age| age > KEEP) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Removes captures from earlier days.
pub fn prune_old() {
    prune(&captures(), SystemTime::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(path: &Path, width: u32, height: u32) {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_png_header_gives_its_size() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("a.png");
        png(&path, 3024, 1964);
        assert_eq!(png_size(&path), Some((3024, 1964)));
        std::fs::write(&path, b"not a picture at all, no").unwrap();
        assert_eq!(png_size(&path), None);
        assert_eq!(png_size(&folder.path().join("missing.png")), None);
    }

    #[test]
    fn names_read_like_the_macs_own_and_never_overwrite() {
        let folder = tempfile::tempdir().unwrap();
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 16, 10, 22, 31).unwrap();
        let first = new_path(folder.path(), "Screenshot", at);
        assert_eq!(first.file_name().unwrap(), "Screenshot 2026-11-16 at 10.22.31.png");
        std::fs::write(&first, b"").unwrap();
        assert_eq!(new_path(folder.path(), "Screenshot", at).file_name().unwrap(), "Screenshot 2026-11-16 at 10.22.31 (2).png");
        assert_eq!(marked_path(&first).file_name().unwrap(), "Screenshot 2026-11-16 at 10.22.31 (marked up).png");
        assert_eq!(marked_path(&marked_path(&first)), marked_path(&first), "marking up again replaces the copy");
    }

    #[test]
    fn old_pictures_are_removed() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..3 {
            png(&folder.path().join(format!("{index}.png")), 1, 1);
        }
        std::fs::write(folder.path().join("notes.txt"), b"kept").unwrap();
        prune(folder.path(), SystemTime::now() + Duration::from_secs(2 * 24 * 3600));
        let left: Vec<_> = std::fs::read_dir(folder.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, ["notes.txt"]);
    }

    #[test]
    fn dictation_takes_only_a_capture_card_in_front_and_in_time() {
        let shot = Shot { path: PathBuf::from("/captures/Capture.png"), width: 10, height: 10 };
        let card = (shot.clone(), 1_000);
        assert_eq!(in_front(Some(&card), Some(CARD_ID), 5_000), Some(shot.path.clone()));
        assert_eq!(in_front(Some(&card), Some("agent.ask"), 5_000), None, "another card covers it");
        assert_eq!(in_front(Some(&card), None, 5_000), None, "the island shows no card");
        assert_eq!(in_front(Some(&card), Some(CARD_ID), 1_000 + CARD_MS), None, "its time is up");
        assert_eq!(in_front(None, Some(CARD_ID), 5_000), None, "dismissed, copied or pasted");
    }

    #[test]
    fn a_capture_expires_and_erase_removes_every_capture() {
        let folder = tempfile::tempdir().unwrap();
        let captures = folder.path().join("captures");
        std::fs::create_dir_all(&captures).unwrap();
        png(&captures.join("a.png"), 1, 1);
        assert!(live(1_000, 1_000 + CARD_MS - 1) && !live(1_000, 1_000 + CARD_MS));
        STATE.lock().unwrap().card = Some((Shot { path: captures.join("a.png"), width: 1, height: 1 }, surface::now_ms()));
        STATE.lock().unwrap().thumbnail = Some(Shot { path: captures.join("a.png"), width: 1, height: 1 });
        erase(folder.path()).unwrap();
        assert!(state().card.is_none() && state().thumbnail.is_none());
        assert!(!captures.exists());
        erase(folder.path()).unwrap();
    }

    #[test]
    fn a_tiny_area_is_not_taken() {
        assert!(take(&[0.0, 0.0, 1.0, 400.0], "Capture").is_err());
        assert!(take(&[f64::NAN, 0.0, 100.0, 100.0], "Capture").is_err());
    }

    use chrono::TimeZone;
}
