use crate::{model::ModelHub, types::Settings};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, Mutex},
    time::{timeout, Duration},
};

struct Recording {
    child: Child,
    input: ChildStdin,
    /// Helper events other than live partials and loudness, which go straight to the UI.
    events: mpsc::UnboundedReceiver<Value>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    /// Set when this session listened while Buntini was talking, so its own words are
    /// removed from what was heard.
    echo_since: Option<Instant>,
}

/// How a listening session starts. `barge_in` keeps Buntini's reply playing so the
/// user can talk over it; otherwise starting to listen silences Buntini first.
#[derive(Clone, Copy, Default)]
pub struct Listen {
    pub barge_in: bool,
}

/// What the UI learns when the microphone is open: the session its live events carry,
/// and whether words arrive while the user talks (Apple's live recognition) or only
/// after they stop (a recording transcribed afterwards).
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listening {
    pub session: u64,
    pub live: bool,
}

/// Sends a live voice event (`voice-partial`, `voice-level`, `voice-ended`) to the UI.
pub type VoiceEvents = Arc<dyn Fn(&'static str, Value) + Send + Sync>;

/// Live voice events go to the companion window only, never to browser workspaces.
pub fn events(app: tauri::AppHandle) -> VoiceEvents {
    use tauri::Emitter;
    Arc::new(move |name, payload| {
        let _ = app.emit_to("main", name, payload);
    })
}

/// A push-to-talk turn is capped at 30 seconds, as before. A session that starts while
/// Buntini is still answering also covers the rest of the reply; the UI still ends the
/// user's own turn 30 seconds after Buntini goes quiet.
const TURN_SECONDS: u32 = 30;
const BARGE_IN_SECONDS: u32 = 120;

/// What Buntini said aloud recently, kept in memory only, so the microphone picking up
/// its own voice is not mistaken for the user. Nothing here is saved.
#[derive(Default)]
struct SpokenLog {
    next: u64,
    entries: Vec<Spoken>,
}
struct Spoken {
    id: u64,
    text: String,
    ended: Option<Instant>,
}
impl SpokenLog {
    fn begin(&mut self, text: &str) -> u64 {
        let now = Instant::now();
        self.entries.retain(|entry| {
            entry
                .ended
                .is_none_or(|ended| now.duration_since(ended) < Duration::from_secs(60))
        });
        self.next += 1;
        self.entries.push(Spoken {
            id: self.next,
            text: text.to_string(),
            ended: None,
        });
        self.next
    }
    fn end(&mut self, id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.ended.get_or_insert_with(Instant::now);
        }
    }
    /// Words spoken while a session listened. Room echo and the audio pipeline lag the
    /// speaker by a moment, so speech that ended just before the session still counts.
    fn words_since(&self, since: Instant) -> HashSet<String> {
        let window = since.checked_sub(Duration::from_secs(2)).unwrap_or(since);
        self.entries
            .iter()
            .filter(|entry| entry.ended.is_none_or(|ended| ended >= window))
            .flat_map(|entry| words(&entry.text))
            .collect()
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '’')
                .flat_map(char::to_lowercase)
                .map(|c| if c == '’' { '\'' } else { c })
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Removes Buntini's own words from the start of what the microphone heard while it was
/// talking. The transcript is the reply's echo followed by whatever the user said, so it
/// is cut where the fewest words disagree: non-echo words before the cut plus echo words
/// after it. Ties keep more of the user's words.
fn strip_echo(heard: &str, spoken: &HashSet<String>) -> String {
    if spoken.is_empty() {
        return heard.trim().to_string();
    }
    let raw: Vec<&str> = heard.split_whitespace().collect();
    let echo: Vec<bool> = raw
        .iter()
        .map(|word| {
            words(word)
                .first()
                .is_some_and(|word| spoken.contains(word))
        })
        .collect();
    let mut best = (usize::MAX, 0);
    let mut misses_before = 0;
    let mut hits_after = echo.iter().filter(|hit| **hit).count();
    for cut in 0..=raw.len() {
        let cost = misses_before + hits_after;
        if cost < best.0 {
            best = (cost, cut);
        }
        match echo.get(cut) {
            Some(true) => hits_after -= 1,
            Some(false) => misses_before += 1,
            None => {}
        }
    }
    raw[best.1..].join(" ")
}

/// Reads helper output for the whole session. Partials and loudness go to the UI as
/// they arrive (with Buntini's own words removed); everything else waits for stop().
fn spawn_reader(
    mut lines: Lines<BufReader<ChildStdout>>,
    sender: mpsc::UnboundedSender<Value>,
    emit: VoiceEvents,
    session: u64,
    spoken: Arc<std::sync::Mutex<SpokenLog>>,
    echo_since: Option<Instant>,
) {
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let message = (line.len() <= 65_536)
                .then(|| serde_json::from_str::<Value>(&line).ok())
                .flatten();
            let Some(message) = message else {
                let _ = sender
                    .send(json!({"event": "error", "message": "Invalid audio helper response."}));
                break;
            };
            match message.get("event").and_then(Value::as_str) {
                Some("partial") => {
                    let heard = message
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let text = match echo_since {
                        Some(since) => {
                            strip_echo(heard, &spoken.lock().unwrap().words_since(since))
                        }
                        None => heard.trim().to_string(),
                    };
                    emit("voice-partial", json!({"session": session, "text": text}));
                }
                Some("level") => {
                    let level = message.get("rms").and_then(Value::as_f64).unwrap_or(0.0);
                    emit("voice-level", json!({"session": session, "level": level}));
                }
                event => {
                    let done = matches!(event, Some("stopped" | "error" | "cancelled"));
                    if sender.send(message).is_err() {
                        break;
                    }
                    // The helper ended on its own (time limit or failure); the UI collects it.
                    if done {
                        emit("voice-ended", json!({"session": session}));
                    }
                }
            }
        }
    });
}

/// A microphone that delivers only silence (muted, the wrong input, or blocked) peaks far
/// below even a quiet room, which sits around 0.003 and up.
fn silent(peak: f64) -> bool {
    peak < 0.0015
}
const SILENT: &str = "I couldn’t hear anything: the microphone only sent silence. Check the input in System Settings → Sound, and that Buntini is allowed under Privacy & Security → Microphone.";

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> VoiceEvents {
        Arc::new(|_, _| {})
    }

    fn spoken(text: &str) -> HashSet<String> {
        words(text).into_iter().collect()
    }

    #[test]
    fn buntinis_own_words_are_removed_from_what_the_microphone_heard() {
        let reply = spoken("Your calendar is empty today. Anything else?");
        // Only the echo: nothing is left, so it can never interrupt or be sent.
        assert_eq!(strip_echo("your calendar is empty today", &reply), "");
        // One misheard word of the echo does not leak through as the user's speech.
        assert_eq!(strip_echo("Your calender is empty today", &reply), "");
        // The echo followed by the user talking over it keeps only the user's words.
        assert_eq!(
            strip_echo("your calendar is empty what about tomorrow", &reply),
            "what about tomorrow"
        );
        // The user's own words survive even when they share a common word with the reply.
        assert_eq!(
            strip_echo("Turn off the lights", &spoken("The timer is set.")),
            "Turn off the lights"
        );
        // Outside a barge-in there is nothing to remove.
        assert_eq!(strip_echo(" hello there ", &HashSet::new()), "hello there");
    }

    #[test]
    fn only_speech_heard_while_listening_counts_as_echo() {
        let mut log = SpokenLog::default();
        let old = log.begin("Old sentence long finished");
        log.entries[0].ended = Instant::now().checked_sub(Duration::from_secs(10));
        log.end(old);
        log.begin("Still talking now");
        let words = log.words_since(Instant::now());
        assert!(words.contains("talking") && !words.contains("finished"));
    }

    /// A stand-in helper: checks it was asked for live recognition, streams partials
    /// and loudness, and answers stop with a transcript, never a recording file.
    fn fake_helper(dir: &std::path::Path, partial: &str, transcript: &str) -> PathBuf {
        let path = dir.join("fake-audio-helper");
        let script = format!(
            r#"#!/usr/bin/env python3
import json, sys
def say(event, **fields):
    fields["event"] = event
    print(json.dumps(fields), flush=True)
if "--transcribe" not in sys.argv:
    say("error", message="live recognition was not requested")
    sys.exit(1)
say("ready", protocolVersion=2)
say("recording", live=True, sampleRate=16000, channels=1)
say("level", rms=0.05)
say("partial", text={partial:?})
for line in sys.stdin:
    if line.strip() == "stop":
        say("stopped", transcript={transcript:?}, durationMs=900, peak=0.4)
        break
"#
        );
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn collector() -> (VoiceEvents, Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        (
            Arc::new(move |name, payload| {
                sink.lock().unwrap().push((name.to_string(), payload));
            }),
            seen,
        )
    }

    async fn wait_for(seen: &std::sync::Mutex<Vec<(String, Value)>>, name: &str) -> Value {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some((_, payload)) = seen.lock().unwrap().iter().find(|(n, _)| n == name) {
                    return payload.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn live_words_stream_to_the_ui_and_no_audio_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let helper = fake_helper(dir.path(), "what is on", "What is on my calendar?");
        let voice = Voice::new(helper, dir.path().to_path_buf());
        let (emit, seen) = collector();
        let listening = voice.start(Listen::default(), emit).await.unwrap();
        assert!(listening.live);
        let partial = wait_for(&seen, "voice-partial").await;
        assert_eq!(
            partial,
            json!({"session": listening.session, "text": "what is on"})
        );
        assert_eq!(wait_for(&seen, "voice-level").await["level"], json!(0.05));
        let models = ModelHub::new(PathBuf::new(), PathBuf::new());
        let text = voice.stop(&models, &Settings::default()).await.unwrap();
        assert_eq!(text, "What is on my calendar?");
        assert_eq!(
            wait_for(&seen, "voice-ended").await["session"],
            json!(listening.session)
        );
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".wav")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn talking_over_a_reply_keeps_it_playing_and_hides_its_echo() {
        let dir = tempfile::tempdir().unwrap();
        let helper = fake_helper(
            dir.path(),
            "your calendar is empty",
            "your calendar is empty what about tomorrow",
        );
        let voice = Voice::new(helper, dir.path().to_path_buf());
        // Buntini is mid-sentence when the microphone opens.
        let _speaking = SpeakingNow::new(&voice.spoken, "Your calendar is empty today.");
        let epoch = voice.reserve_speech();
        let (emit, seen) = collector();
        let listening = voice.start(Listen { barge_in: true }, emit).await.unwrap();
        assert_eq!(
            voice.speech_id.load(Ordering::SeqCst),
            epoch,
            "listening for a barge-in must not silence the reply"
        );
        // The echo alone never reaches the composer as if the user had said it.
        assert_eq!(wait_for(&seen, "voice-partial").await["text"], json!(""));
        let models = ModelHub::new(PathBuf::new(), PathBuf::new());
        let text = voice.stop(&models, &Settings::default()).await.unwrap();
        assert_eq!(text, "what about tomorrow");
        assert!(listening.session > 0);
    }

    #[tokio::test]
    async fn a_normal_start_still_silences_buntini_first() {
        let dir = tempfile::tempdir().unwrap();
        let voice = Voice::new(dir.path().join("missing-helper"), dir.path().to_path_buf());
        let epoch = voice.reserve_speech();
        let error = voice.start(Listen::default(), quiet()).await.unwrap_err();
        assert!(error.contains("audio helper is missing"));
        assert_ne!(voice.speech_id.load(Ordering::SeqCst), epoch);
    }

    #[test]
    fn a_silent_microphone_is_told_apart_from_quiet_speech() {
        assert!(silent(0.0) && silent(0.0004));
        assert!(!silent(0.004) && !silent(0.3));
    }
    #[tokio::test]
    async fn stopping_during_preflight_invalidates_the_pending_speech() {
        let dir = tempfile::tempdir().unwrap();
        let voice = Voice::new(dir.path().join("unused-helper"), dir.path().to_path_buf());
        let epoch = voice.reserve_speech();
        voice.stop_speaking().await.unwrap();
        voice
            .speak_with_voice("This must never start".into(), "", 180, epoch)
            .await
            .unwrap();
        assert!(voice.speech.lock().await.is_none());
    }
    #[tokio::test]
    async fn microphone_cancel_before_startup_await_cannot_open_a_new_session() {
        let dir = tempfile::tempdir().unwrap();
        let voice = Arc::new(Voice::new(
            dir.path().join("must-not-launch"),
            dir.path().to_path_buf(),
        ));
        let speech_lock = voice.speech.lock().await;
        let start = voice.clone();
        let pending = tokio::spawn(async move { start.start(Listen::default(), quiet()).await });
        while voice.recording_generation.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        voice.cancel().await.unwrap();
        drop(speech_lock);
        let error = pending.await.unwrap().unwrap_err();
        assert_eq!(error, "Recording cancelled.");
        assert!(voice.recording.lock().await.is_none());
    }

    #[tokio::test]
    async fn cancellation_reaches_a_recording_already_owned_by_stop() {
        let dir = tempfile::tempdir().unwrap();
        let voice = Arc::new(Voice::new(
            dir.path().join("must-not-launch"),
            dir.path().to_path_buf(),
        ));
        let path = dir.path().join("buntini-recording-fixture.wav");
        std::fs::write(&path, b"private audio fixture").unwrap();
        // A pipe fixture exercises stop/cancel without a microphone, permission
        // dialog, speech process or transcription model.
        let mut child = Command::new("python3")
            .args([
                "-u",
                "-c",
                "import sys,time;sys.stdin.readline();time.sleep(30)",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        *voice.current_cancel.lock().unwrap() = cancel.clone();
        let (_sender, events) = mpsc::unbounded_channel();
        *voice.recording.lock().await = Some(Recording {
            input: child.stdin.take().unwrap(),
            events,
            child,
            path: path.clone(),
            cancel,
            echo_since: None,
        });
        let stopping_voice = voice.clone();
        let stopping = tokio::spawn(async move {
            stopping_voice
                .stop(
                    &ModelHub::new(PathBuf::new(), PathBuf::new()),
                    &Settings::default(),
                )
                .await
        });
        timeout(Duration::from_secs(2), async {
            while voice.recording.lock().await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        voice.cancel().await.unwrap();
        let result = timeout(Duration::from_secs(2), stopping)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "Recording cancelled.");
        assert!(
            !path.exists(),
            "cancelled audio must be removed before returning"
        );
        assert!(voice.recording.lock().await.is_none());
    }
}
pub struct Voice {
    recording: Mutex<Option<Recording>>,
    speech: Mutex<Option<Child>>,
    speech_id: AtomicU64,
    recording_generation: AtomicU64,
    current_cancel: std::sync::Mutex<Arc<AtomicBool>>,
    closed: AtomicBool,
    helper: PathBuf,
    cache: PathBuf,
    spoken: Arc<std::sync::Mutex<SpokenLog>>,
}
/// Marks text as being spoken for as long as `say` may be playing it, however playback ends.
struct SpeakingNow<'a> {
    log: &'a std::sync::Mutex<SpokenLog>,
    id: u64,
}
impl<'a> SpeakingNow<'a> {
    fn new(log: &'a std::sync::Mutex<SpokenLog>, text: &str) -> Self {
        let id = log.lock().unwrap().begin(text);
        Self { log, id }
    }
}
impl Drop for SpeakingNow<'_> {
    fn drop(&mut self) {
        self.log.lock().unwrap().end(self.id);
    }
}
/// Only removes this app's own recordings (left after an interrupted session, or on
/// "Delete everything").
fn remove_recordings(cache: &std::path::Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Couldn’t inspect the temporary recording cache.".into()),
    };
    let mut failed = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("buntini-recording-") && name.ends_with(".wav") {
            failed |= std::fs::remove_file(entry.path()).is_err();
        }
    }
    if failed {
        Err("Couldn’t remove all temporary recordings. Check the app cache permissions and try Delete everything again.".into())
    } else {
        Ok(())
    }
}
impl Voice {
    pub fn clear_cache(&self) -> Result<(), String> {
        self.spoken.lock().unwrap().entries.clear();
        remove_recordings(&self.cache)
    }
    pub fn new(helper: PathBuf, cache: PathBuf) -> Self {
        let _ = remove_recordings(&cache);
        Self {
            recording: Mutex::new(None),
            speech: Mutex::new(None),
            speech_id: AtomicU64::new(0),
            recording_generation: AtomicU64::new(0),
            current_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
            closed: AtomicBool::new(false),
            helper,
            cache,
            spoken: Arc::default(),
        }
    }
    pub async fn speak_with_voice(
        &self,
        text: String,
        voice_id: &str,
        rate: u32,
        id: u64,
    ) -> Result<(), String> {
        if self.closed.load(Ordering::Relaxed) {
            return Err("Buntini is closing.".into());
        }
        if text.chars().count() > 12000 {
            return Err("This answer is too long to speak. Choose a shorter passage.".into());
        }
        {
            let mut slot = self.speech.lock().await;
            if self.speech_id.load(Ordering::SeqCst) != id {
                return Ok(());
            }
            if let Some(mut previous) = slot.take() {
                let _ = previous.kill().await;
            }
        }
        if self.speech_id.load(Ordering::SeqCst) != id || self.closed.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut command = Command::new("/usr/bin/say");
        command.arg("-r").arg(rate.to_string());
        if !voice_id.is_empty() {
            command.arg("-v").arg(voice_id);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "macOS speech could not start.".to_string())?;
        let _spoken = SpeakingNow::new(&self.spoken, &text);
        if let Some(mut input) = child.stdin.take() {
            let write = input.write_all(text.as_bytes());
            tokio::pin!(write);
            loop {
                tokio::select! {result=&mut write=>{result.map_err(|_|"Speech input failed.".to_string())?;break;},_=tokio::time::sleep(Duration::from_millis(25))=>{if self.speech_id.load(Ordering::SeqCst)!=id||self.closed.load(Ordering::Relaxed){let _=child.kill().await;return Ok(());}}}
            }
        }
        let mut slot = self.speech.lock().await;
        if self.closed.load(Ordering::Relaxed) || self.speech_id.load(Ordering::SeqCst) != id {
            let _ = child.kill().await;
            return Err("Buntini is closing.".into());
        }
        *slot = Some(child);
        drop(slot);
        loop {
            if self.speech_id.load(Ordering::SeqCst) != id {
                return Ok(());
            }
            {
                let mut speech = self.speech.lock().await;
                match speech.as_mut() {
                    Some(child) => {
                        if let Some(status) = child
                            .try_wait()
                            .map_err(|_| "Speech process could not be inspected.".to_string())?
                        {
                            *speech = None;
                            return if status.success() {
                                Ok(())
                            } else {
                                Err("macOS speech could not complete. Check the selected installed voice.".into())
                            };
                        }
                    }
                    None => return Ok(()),
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub fn reserve_speech(&self) -> u64 {
        self.speech_id.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub async fn stop_speaking(&self) -> Result<(), String> {
        self.speech_id.fetch_add(1, Ordering::SeqCst);
        if let Some(mut child) = self.speech.lock().await.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
    pub async fn cancel(&self) -> Result<(), String> {
        self.recording_generation.fetch_add(1, Ordering::SeqCst);
        self.current_cancel
            .lock()
            .unwrap()
            .store(true, Ordering::SeqCst);
        if let Some(mut rec) = self.recording.lock().await.take() {
            let _ = rec.input.write_all(b"cancel\n").await;
            let _ = rec.child.kill().await;
            let _ = std::fs::remove_file(&rec.path);
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.cancel().await;
        let _ = self.stop_speaking().await;
    }
    /// Opens the microphone. Apple's live recognition is asked for first, so words appear
    /// while the user talks and no audio is written; where it is unavailable the helper
    /// records a private WAV that stop() transcribes with the configured recognizer.
    pub async fn start(&self, listen: Listen, emit: VoiceEvents) -> Result<Listening, String> {
        let generation = self.recording_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.closed.load(Ordering::Relaxed) {
            return Err("Buntini is closing.".into());
        }
        if !listen.barge_in {
            self.stop_speaking().await?;
        }
        let mut slot = self.recording.lock().await;
        if self.recording_generation.load(Ordering::SeqCst) != generation
            || self.closed.load(Ordering::Relaxed)
        {
            return Err("Recording cancelled.".into());
        }
        if slot.is_some() {
            return Err("A recording is already active.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut current = self.current_cancel.lock().unwrap();
            current.store(true, Ordering::SeqCst);
            *current = cancel.clone();
        }
        if !self.helper.is_file() {
            return Err(
                "The local audio helper is missing. Run native/build.sh, then restart Buntini."
                    .into(),
            );
        }
        let path = self
            .cache
            .join(format!("buntini-recording-{}.wav", uuid::Uuid::new_v4()));
        let seconds = if listen.barge_in {
            BARGE_IN_SECONDS
        } else {
            TURN_SECONDS
        };
        let echo_since = listen.barge_in.then(Instant::now);
        let mut child = Command::new(&self.helper)
            .arg("--transcribe")
            .arg("--output")
            .arg(&path)
            .arg("--max-seconds")
            .arg(seconds.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "The local audio helper could not start.".to_string())?;
        let input = child.stdin.take().ok_or("Cannot open audio input.")?;
        let mut lines =
            BufReader::new(child.stdout.take().ok_or("Cannot read audio helper.")?).lines();
        let ready = self
            .wait_until_recording(&mut lines, &cancel, generation)
            .await;
        let ready = ready.and_then(|live| {
            if self.recording_generation.load(Ordering::SeqCst) == generation
                && !cancel.load(Ordering::Relaxed)
            {
                Ok(live)
            } else {
                Err("Recording cancelled.".into())
            }
        });
        let live = match ready {
            Ok(live) => live,
            Err(e) => {
                let _ = child.kill().await;
                let _ = std::fs::remove_file(&path);
                return Err(e);
            }
        };
        let (sender, events) = mpsc::unbounded_channel();
        spawn_reader(
            lines,
            sender,
            emit,
            generation,
            self.spoken.clone(),
            echo_since,
        );
        *slot = Some(Recording {
            child,
            input,
            events,
            path,
            cancel,
            echo_since,
        });
        Ok(Listening {
            session: generation,
            live,
        })
    }

    /// Waits for the microphone to open. Permission prompts get 30 seconds; a first-time
    /// download of Apple's speech model (the helper says "preparing") gets three minutes.
    async fn wait_until_recording(
        &self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        cancel: &AtomicBool,
        generation: u64,
    ) -> Result<bool, String> {
        let mut deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if cancel.load(Ordering::Relaxed)
                || self.closed.load(Ordering::Relaxed)
                || self.recording_generation.load(Ordering::SeqCst) != generation
            {
                return Err("Recording cancelled.".into());
            }
            if Instant::now() >= deadline {
                return Err("Microphone permission or startup timed out. Enable Microphone access in macOS Settings, then try again.".into());
            }
            let line = tokio::select! {
                result = lines.next_line() => result.map_err(|_| "Audio helper connection failed.".to_string())?,
                _ = tokio::time::sleep(Duration::from_millis(40)) => continue,
            };
            let Some(line) = line else {
                return Err("The audio helper exited before recording began.".into());
            };
            if line.len() > 8192 {
                return Err("Invalid audio helper response.".into());
            }
            let message: Value = serde_json::from_str(&line)
                .map_err(|_| "Invalid audio helper response.".to_string())?;
            match message.get("event").and_then(Value::as_str) {
                Some("recording") => {
                    return Ok(message
                        .get("live")
                        .and_then(Value::as_bool)
                        .unwrap_or(false))
                }
                Some("preparing") => deadline = Instant::now() + Duration::from_secs(180),
                Some("error") => {
                    return Err(message
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Microphone could not start.")
                        .to_string())
                }
                Some("cancelled") => return Err("Recording cancelled.".into()),
                _ => {}
            }
        }
    }

    pub async fn stop(&self, models: &ModelHub, settings: &Settings) -> Result<String, String> {
        let mut rec = self
            .recording
            .lock()
            .await
            .take()
            .ok_or("No recording is active.")?;
        let _ = rec.input.write_all(b"stop\n").await;
        // How loud the recording got (0–1); None from older helpers that don't report it.
        let mut peak: Option<f64> = None;
        // Live recognition hands back its words; a recording still needs transcribing.
        let mut transcript: Option<String> = None;
        let stopped = timeout(Duration::from_secs(8), async {
            loop {
                if rec.cancel.load(Ordering::Relaxed) {
                    return Err("Recording cancelled.".into());
                }
                let message = tokio::select! {
                    message = rec.events.recv() => message,
                    _ = tokio::time::sleep(Duration::from_millis(40)) => continue,
                };
                let Some(message) = message else {
                    return Err("The recording ended without audio.".into());
                };
                match message.get("event").and_then(Value::as_str) {
                    Some("stopped") => {
                        peak = message.get("peak").and_then(Value::as_f64);
                        if let Some(text) = message.get("transcript").and_then(Value::as_str) {
                            transcript = Some(text.to_string());
                        } else if message.get("path").and_then(Value::as_str) != rec.path.to_str() {
                            return Err(
                                "Audio helper returned an unexpected recording path.".into()
                            );
                        }
                        return Ok(());
                    }
                    Some("error") => {
                        return Err(message
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Recording failed.")
                            .to_string())
                    }
                    Some("cancelled") => return Err("Recording cancelled.".into()),
                    _ => {}
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err("Stopping the microphone timed out.".into()));
        let _ = rec.child.kill().await;
        let result = match stopped {
            Err(e) => Err(e),
            Ok(()) if peak.is_some_and(silent) => Err(SILENT.into()),
            Ok(()) => match transcript.take() {
                Some(text) => Ok(text),
                None => {
                    models
                        .transcribe(settings, &rec.path, rec.cancel.clone())
                        .await
                }
            },
        };
        let _ = std::fs::remove_file(&rec.path);
        // stop() owns the Recording after taking it out of the shared slot.
        // Its original cancellation token still belongs to this recording,
        // including after erasure or a later start replaces current_cancel.
        if rec.cancel.load(Ordering::SeqCst) {
            return Err("Recording cancelled.".into());
        }
        result.map(|text| match rec.echo_since {
            Some(since) => strip_echo(&text, &self.spoken.lock().unwrap().words_since(since)),
            None => text,
        })
    }
}
