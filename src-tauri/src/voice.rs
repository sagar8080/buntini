use crate::{model::ModelHub, types::Settings};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};

struct Recording {
    child: Child,
    input: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let pending = tokio::spawn(async move { start.start().await });
        while voice.recording_generation.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        voice.cancel().await.unwrap();
        drop(speech_lock);
        let error = pending.await.unwrap().unwrap_err();
        assert_eq!(error, "Recording cancelled.");
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
}
impl Voice {
    pub fn new(helper: PathBuf, cache: PathBuf) -> Self {
        // Only remove this app's own prior recordings after an interrupted session.
        if let Ok(entries) = std::fs::read_dir(&cache) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("buntini-recording-") && name.ends_with(".wav") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Self {
            recording: Mutex::new(None),
            speech: Mutex::new(None),
            speech_id: AtomicU64::new(0),
            recording_generation: AtomicU64::new(0),
            current_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
            closed: AtomicBool::new(false),
            helper,
            cache,
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
    pub async fn start(&self) -> Result<(), String> {
        let generation = self.recording_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.closed.load(Ordering::Relaxed) {
            return Err("Buntini is closing.".into());
        }
        self.stop_speaking().await?;
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
        let mut child = Command::new(&self.helper)
            .arg("--output")
            .arg(&path)
            .arg("--max-seconds")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "The local audio helper could not start.".to_string())?;
        let input = child.stdin.take().ok_or("Cannot open audio input.")?;
        let mut lines =
            BufReader::new(child.stdout.take().ok_or("Cannot read audio helper.")?).lines();
        let ready=timeout(Duration::from_secs(30),async {
            loop {
                if cancel.load(Ordering::Relaxed)||self.closed.load(Ordering::Relaxed)||self.recording_generation.load(Ordering::SeqCst)!=generation{return Err("Recording cancelled.".into());}
                let line=tokio::select!{result=lines.next_line()=>result.map_err(|_|"Audio helper connection failed.".to_string())?,_=tokio::time::sleep(Duration::from_millis(40))=>continue};
                let Some(line)=line else{return Err("The audio helper exited before recording began.".into());};
                if line.len()>8192{return Err("Invalid audio helper response.".into());}
                let message:Value=serde_json::from_str(&line).map_err(|_|"Invalid audio helper response.".to_string())?;
                match message.get("event").and_then(Value::as_str){Some("recording")=>return Ok(()),Some("error")=>return Err(message.get("message").and_then(Value::as_str).unwrap_or("Microphone could not start.").to_string()),Some("cancelled")=>return Err("Recording cancelled.".into()),_=>{}}
            }
        }).await.unwrap_or_else(|_|Err("Microphone permission or startup timed out. Enable Microphone access in macOS Settings, then try again.".into()));
        let ready = ready.and_then(|()| {
            if self.recording_generation.load(Ordering::SeqCst) == generation
                && !cancel.load(Ordering::Relaxed)
            {
                Ok(())
            } else {
                Err("Recording cancelled.".into())
            }
        });
        if let Err(e) = ready {
            let _ = child.kill().await;
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        *slot = Some(Recording {
            child,
            input,
            lines,
            path,
            cancel,
        });
        Ok(())
    }
    pub async fn stop(
        &self,
        models: &ModelHub,
        settings: &Settings,
    ) -> Result<String, String> {
        let mut rec = self
            .recording
            .lock()
            .await
            .take()
            .ok_or("No recording is active.")?;
        let _ = rec.input.write_all(b"stop\n").await;
        let stopped = timeout(Duration::from_secs(8), async {
            loop {
                if rec.cancel.load(Ordering::Relaxed){return Err("Recording cancelled.".into());}
                let line=tokio::select!{result=rec.lines.next_line()=>result.map_err(|_|"Audio helper connection failed.".to_string())?,_=tokio::time::sleep(Duration::from_millis(40))=>continue};
                let Some(line)=line else{return Err("The recording ended without audio.".into());};
                if line.len() > 8192 {
                    return Err("Invalid audio helper response.".into());
                }
                let message: Value = serde_json::from_str(&line)
                    .map_err(|_| "Invalid audio response.".to_string())?;
                match message.get("event").and_then(Value::as_str) {
                    Some("stopped") => {
                        if message.get("path").and_then(Value::as_str) != rec.path.to_str() {
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
            Ok(()) => {
                models.transcribe(settings, &rec.path, rec.cancel.clone()).await
            }
        };
        let _ = std::fs::remove_file(&rec.path);
        result
    }
}
