//! Parakeet (recognition), the speaker diarizer and the voice detector, fetched only when you
//! ask, into <app data>/speech-models. Every file is pinned to a revision and checked against
//! its SHA-256 before it gets its name, as for the local brain. The audio helper only ever
//! loads from that folder; nothing it runs downloads anything.
use crate::i18n::{t, tf};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const PINS: &str = include_str!("speech-models.json");
/// Room left over once everything is on disk.
const MARGIN_BYTES: u64 = 1_000_000_000;

#[derive(Debug, Deserialize)]
pub struct Pin {
    pub repo: String,
    pub revision: String,
    pub bytes: u64,
    pub files: Vec<FilePin>,
}

#[derive(Debug, Deserialize)]
pub struct FilePin {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// `parakeet`, `diarizer` and `vad`, by folder name.
pub fn pins() -> &'static BTreeMap<String, Pin> {
    static LOADED: OnceLock<BTreeMap<String, Pin>> = OnceLock::new();
    LOADED.get_or_init(|| serde_json::from_str(PINS).expect("flow/speech-models.json must be valid"))
}

pub fn total_bytes() -> u64 {
    pins().values().map(|pin| pin.bytes).sum()
}

pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("speech-models")
}

/// Whether every pinned file is in place. A file only gets its name once its hash matched,
/// so its size is enough here.
pub fn ready(base: &Path) -> bool {
    pins().iter().all(|(name, pin)| {
        pin.files.iter().all(|file| std::fs::metadata(base.join(name).join(&file.path)).is_ok_and(|meta| meta.len() == file.bytes))
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// idle, downloading, waiting, ready or failed.
    pub phase: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub message: String,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

pub fn status(base: &Path) -> Status {
    if let Some(status) = STATUS.lock().unwrap().clone() {
        if status.phase != "ready" {
            return status;
        }
    }
    let phase = if ready(base) { "ready" } else { "idle" };
    Status { phase: phase.into(), total_bytes: total_bytes(), ..Status::default() }
}

fn set(status: Status, tell: &(dyn Fn(Status) + Send + Sync)) {
    *STATUS.lock().unwrap() = Some(status.clone());
    tell(status);
}

/// Fetches everything missing. Pressing Download is the consent; nothing personal is sent.
pub async fn download(base: PathBuf, free_bytes: u64, tell: Arc<dyn Fn(Status) + Send + Sync>) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = CANCEL.lock().unwrap();
        if slot.is_some() {
            return Err("The speech model is already downloading.".into());
        }
        *slot = Some(cancel.clone());
    }
    let result = fetch_all(&base, free_bytes, &cancel, tell.as_ref()).await;
    *CANCEL.lock().unwrap() = None;
    let total = total_bytes();
    match &result {
        Ok(()) => set(Status { phase: "ready".into(), done_bytes: total, total_bytes: total, message: String::new() }, tell.as_ref()),
        Err(message) if cancel.load(Ordering::SeqCst) => {
            let _ = message;
            set(Status { phase: "idle".into(), total_bytes: total, ..Status::default() }, tell.as_ref());
        }
        Err(message) => set(Status { phase: "failed".into(), total_bytes: total, message: message.clone(), ..Status::default() }, tell.as_ref()),
    }
    result
}

pub fn cancel() {
    if let Some(cancel) = CANCEL.lock().unwrap().as_ref() {
        cancel.store(true, Ordering::SeqCst);
    }
}

/// Removes the models; recognition goes back to Apple's.
pub fn remove(base: &Path) -> Result<(), String> {
    cancel();
    *STATUS.lock().unwrap() = None;
    match std::fs::remove_dir_all(base) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Buntini couldn’t remove the speech model.".into()),
    }
}

async fn fetch_all(base: &Path, free_bytes: u64, cancel: &AtomicBool, tell: &(dyn Fn(Status) + Send + Sync)) -> Result<(), String> {
    let total = total_bytes();
    if free_bytes < total + MARGIN_BYTES {
        let size = format!("{:.1}", (total + MARGIN_BYTES) as f64 / 1e9);
        return Err(tf("flow.models.needsSpace", &[("size", &size)]));
    }
    tokio::fs::create_dir_all(base).await.map_err(|_| t("flow.models.noFolder"))?;
    // Gigabytes that can be fetched again have no place in a backup.
    let _ = tokio::process::Command::new("/usr/bin/tmutil").arg("addexclusion").arg(base).output().await;
    let client = crate::local_setup::download_client()?;
    let told = Mutex::new(Instant::now() - Duration::from_secs(1));
    let mut finished = 0;
    set(Status { phase: "downloading".into(), total_bytes: total, ..Status::default() }, tell);
    for (name, pin) in pins() {
        for file in &pin.files {
            if !crate::local_setup::safe_relative(&file.path) {
                return Err(t("flow.models.unexpectedFile"));
            }
            let url = format!("https://huggingface.co/{}/resolve/{}/{}", pin.repo, pin.revision, file.path);
            let so_far = finished;
            let mut waiting = |message: &str| set(Status { phase: "waiting".into(), done_bytes: so_far, total_bytes: total, message: message.into() }, tell);
            let mut progress = |done: u64| {
                let mut last = told.lock().unwrap();
                if last.elapsed() >= Duration::from_millis(250) {
                    *last = Instant::now();
                    set(Status { phase: "downloading".into(), done_bytes: so_far + done, total_bytes: total, message: String::new() }, tell);
                }
            };
            crate::local_setup::fetch(&client, &url, &file.sha256, file.bytes, &base.join(name).join(&file.path), cancel, &mut waiting, &mut progress).await?;
            finished += file.bytes;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_pinned_with_a_hash_and_a_safe_path() {
        assert_eq!(pins().keys().collect::<Vec<_>>(), ["diarizer", "parakeet", "vad"]);
        for (name, pin) in pins() {
            assert_eq!(pin.revision.len(), 40, "{name} is pinned to a commit");
            assert!(!pin.files.is_empty());
            assert_eq!(pin.files.iter().map(|file| file.bytes).sum::<u64>(), pin.bytes, "{name}");
            for file in &pin.files {
                assert!(crate::local_setup::safe_relative(&file.path), "{}", file.path);
                assert!(file.sha256.len() == 64 && file.sha256.chars().all(|ch| ch.is_ascii_hexdigit()), "{}", file.path);
            }
        }
        // Parakeet uses the corrected int8 encoder, and is about 650 MB with the rest.
        assert!(pins()["parakeet"].files.iter().any(|file| file.path.starts_with("Encoder_v2.mlmodelc/")));
        assert!((550_000_000..750_000_000).contains(&total_bytes()));
    }

    #[test]
    fn ready_only_when_every_file_is_in_place() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!ready(dir.path()));
        for (name, pin) in pins() {
            for file in &pin.files {
                let path = dir.path().join(name).join(&file.path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::File::create(&path).unwrap().set_len(file.bytes).unwrap();
            }
        }
        assert!(ready(dir.path()));
        let first = &pins()["vad"].files[0];
        std::fs::File::options().write(true).open(dir.path().join("vad").join(&first.path)).unwrap().set_len(first.bytes - 1).unwrap();
        assert!(!ready(dir.path()));
        remove(dir.path()).unwrap();
        assert!(!dir.path().exists());
    }

    #[tokio::test]
    async fn too_little_room_stops_before_anything_is_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("speech-models");
        let error = download(base.clone(), 10, Arc::new(|_| {})).await.unwrap_err();
        assert!(error.contains("GB free"));
        assert!(!base.exists());
        assert_eq!(STATUS.lock().unwrap().as_ref().unwrap().phase, "failed");
    }
}
