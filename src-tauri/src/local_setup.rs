//! One-click local model: fetches a pinned Python, its packages and the model that suits this
//! Mac into <app data>/local-brain, tests one answer through the real worker, then hands over.
//!
//! Pressing the button is the consent to download. Every file is checked against the hash in
//! `worker/local-brain.json` or the package lock before it is unpacked or run. Nothing but
//! these exact bytes is fetched, nothing personal is sent, and once set up the model works
//! offline. A dropped connection waits and resumes, including after quitting Buntini.
use crate::controller::AppState;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::io::AsyncWriteExt;

const PINS: &str = include_str!("../../worker/local-brain.json");
const LOCK: &str = include_str!("../../worker/requirements-local.lock");
/// Set while a setup is unfinished, so it picks up again after a restart.
const IN_PROGRESS: &str = "local-setup-model";
/// Room left over once everything is on disk, with unpacked packages counted.
const MARGIN_BYTES: u64 = 3_000_000_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pins {
    python: PythonPin,
    package_bytes: u64,
    minimum_macos: String,
    models: HashMap<String, ModelPin>,
}
#[derive(Debug, Deserialize)]
struct PythonPin {
    url: String,
    bytes: u64,
    sha256: String,
}
#[derive(Debug, Deserialize)]
struct ModelPin {
    repo: String,
    revision: String,
    license: String,
    bytes: u64,
    files: Vec<FilePin>,
}
#[derive(Debug, Deserialize)]
struct FilePin {
    path: String,
    bytes: u64,
    sha256: String,
}

fn pins() -> &'static Pins {
    static LOADED: OnceLock<Pins> = OnceLock::new();
    LOADED.get_or_init(|| serde_json::from_str(PINS).expect("worker/local-brain.json must be valid"))
}

/// The one model setup would fetch here, and whether this Mac can run it.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub repo: String,
    pub label: String,
    pub download_bytes: u64,
    /// Why this Mac can't, in words to show.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// idle, downloading, waiting, installing, testing, ready or failed.
    pub phase: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub message: String,
    pub python: Option<String>,
    pub model: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// The model for one click: the advisor's pick, under a licence that needs no agreement from
/// the user, since Buntini is the one fetching it. Other models stay available by hand.
fn pick(memory_gb: u64, chip: &str) -> &'static crate::advisor::Model {
    let (best, other) = crate::advisor::recommend(memory_gb, crate::advisor::tier(chip));
    let open = |model: &&crate::advisor::Model| pins().models.get(model.repo).is_some_and(|pin| pin.license == "apache-2.0");
    [Some(best), other]
        .into_iter()
        .flatten()
        .find(|model| open(model))
        .or_else(|| crate::advisor::MODELS.iter().find(|model| open(model)))
        .expect("at least one pinned model has an open licence")
}

fn needed_bytes(model: &ModelPin) -> u64 {
    model.bytes + pins().python.bytes + pins().package_bytes + MARGIN_BYTES
}

fn plan_for(memory_gb: u64, chip: &str, apple_silicon: bool, macos: &str, free_bytes: u64) -> Plan {
    let chosen = pick(memory_gb, chip);
    let model = &pins().models[chosen.repo];
    let major = |version: &str| version.split('.').next().and_then(|part| part.parse::<u32>().ok()).unwrap_or(0);
    let problem = if !apple_silicon {
        Some("A model on this Mac needs Apple silicon.".to_string())
    } else if major(macos) < major(&pins().minimum_macos) {
        Some(format!("A model on this Mac needs macOS {} or newer.", major(&pins().minimum_macos)))
    } else if free_bytes < needed_bytes(model) {
        Some(format!("This needs {:.0} GB free, and this Mac has {:.0} GB.", needed_bytes(model) as f64 / 1e9, free_bytes as f64 / 1e9))
    } else {
        None
    };
    Plan {
        repo: chosen.repo.into(),
        label: chosen.label.into(),
        download_bytes: model.bytes + pins().python.bytes + pins().package_bytes,
        problem,
    }
}

fn base(state: &AppState) -> PathBuf {
    state.data_dir.join("local-brain")
}

fn python_of(base: &Path) -> PathBuf {
    base.join("python/bin/python3")
}

fn model_of(base: &Path, repo: &str) -> PathBuf {
    base.join("models").join(repo)
}

fn free_bytes(path: &Path) -> u64 {
    let Ok(disks) = sysinfo::Disks::new_with_refreshed_list() else {
        return 0;
    };
    disks
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
        .unwrap_or(0)
}

#[tauri::command]
pub fn local_setup_plan(state: tauri::State<'_, AppState>) -> Plan {
    let chip = crate::advisor::sysctl("machdep.cpu.brand_string");
    let memory = crate::advisor::sysctl("hw.memsize").parse::<u64>().unwrap_or(0);
    plan_for(
        (memory as f64 / 1_073_741_824.0).round() as u64,
        &chip,
        std::env::consts::ARCH == "aarch64",
        &sysinfo::System::os_version().unwrap_or_default(),
        free_bytes(&state.data_dir),
    )
}

fn publish(app: &tauri::AppHandle, status: Status) {
    *STATUS.lock().unwrap() = Some(status.clone());
    let _ = app.emit("local-setup", status);
}

/// What's happening now. A model set up earlier reads as ready.
#[tauri::command]
pub fn local_setup_status(state: tauri::State<'_, AppState>) -> Status {
    if let Some(status) = STATUS.lock().unwrap().clone() {
        return status;
    }
    let settings = state.store.lock().unwrap().settings().unwrap_or_default();
    let ours = base(&state);
    if crate::model::local_ready(&settings) && Path::new(&settings.python_path).starts_with(&ours) {
        return Status { phase: "ready".into(), python: Some(settings.python_path), model: Some(settings.model_path), ..Status::default() };
    }
    Status { phase: "idle".into(), ..Status::default() }
}

/// For chat while a download runs: "The model on this Mac is 40% ready."
pub fn progress_line() -> Option<String> {
    let status = STATUS.lock().unwrap().clone()?;
    match status.phase.as_str() {
        "downloading" | "waiting" if status.total_bytes > 0 => Some(format!("The model on this Mac is {}% ready.", status.done_bytes * 100 / status.total_bytes)),
        "installing" | "testing" => Some("The model on this Mac is nearly ready.".into()),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
enum Fetch {
    /// Worth trying again by itself: the connection dropped, or the server is having a moment.
    Retry(String),
    Stop(String),
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        // No bytes for this long counts as a dropped connection. There's no total limit.
        .read_timeout(Duration::from_secs(60))
        .user_agent("Buntini")
        .build()
        .map_err(|_| "Buntini couldn’t prepare a secure connection.".to_string())
}

/// One attempt at the rest of a file, appending to `part` from where it stopped.
async fn fetch_once(client: &reqwest::Client, url: &str, part: &Path, cancel: &AtomicBool, progress: &mut (impl FnMut(u64) + Send)) -> Result<(), Fetch> {
    let have = tokio::fs::metadata(part).await.map(|meta| meta.len()).unwrap_or(0);
    let mut request = client.get(url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let response = request.send().await.map_err(|_| Fetch::Retry("Waiting for the connection".into()))?;
    let status = response.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(part).await;
        return Err(Fetch::Retry("Starting this file again".into()));
    }
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Fetch::Retry("Waiting for the download to come back".into()));
    }
    if !status.is_success() {
        return Err(Fetch::Stop(format!("The download was refused ({}).", status.as_u16())));
    }
    // A server that ignored the range sends the whole file again.
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .await
        .map_err(|_| Fetch::Stop("Buntini couldn’t save the download.".into()))?;
    if !resumed {
        progress(0);
    }
    let mut done = if resumed { have } else { 0 };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err(Fetch::Stop("Cancelled.".into()));
        }
        let chunk = chunk.map_err(|_| Fetch::Retry("Waiting for the connection".into()))?;
        file.write_all(&chunk).await.map_err(|_| Fetch::Stop("Buntini couldn’t save the download. Check there’s free disk space.".into()))?;
        done += chunk.len() as u64;
        progress(done);
    }
    file.flush().await.map_err(|_| Fetch::Stop("Buntini couldn’t save the download.".into()))?;
    Ok(())
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        match file.read(&mut buffer)? {
            0 => break,
            read => hasher.update(&buffer[..read]),
        }
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Fetches one file to `dest`. It only gets that name once its hash matches, so a file there
/// is a checked one. Connection trouble waits and resumes for as long as it takes.
async fn fetch(client: &reqwest::Client, url: &str, sha256: &str, bytes: u64, dest: &Path, cancel: &AtomicBool, waiting: &mut (impl FnMut(&str) + Send), progress: &mut (impl FnMut(u64) + Send)) -> Result<(), String> {
    if tokio::fs::metadata(dest).await.is_ok_and(|meta| meta.len() == bytes) {
        progress(bytes);
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|_| "Buntini couldn’t make a folder for the download.".to_string())?;
    }
    let part = dest.with_extension(format!("{}part", dest.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
    let mut pause = 1;
    loop {
        let have = tokio::fs::metadata(&part).await.map(|meta| meta.len()).unwrap_or(0);
        if have > bytes {
            let _ = tokio::fs::remove_file(&part).await;
            continue;
        }
        if have == bytes {
            break;
        }
        progress(have);
        match fetch_once(client, url, &part, cancel, progress).await {
            Ok(()) => pause = 1,
            Err(Fetch::Stop(message)) => return Err(message),
            Err(Fetch::Retry(message)) => {
                waiting(&message);
                for _ in 0..pause * 10 {
                    if cancel.load(Ordering::SeqCst) {
                        return Err("Cancelled.".into());
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                pause = (pause * 2).min(30);
            }
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled.".into());
        }
    }
    let checked = part.clone();
    let found = tokio::task::spawn_blocking(move || sha256_of(&checked)).await.map_err(|_| "Buntini couldn’t check the download.".to_string())?;
    if found.ok().as_deref() != Some(sha256) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("A download didn’t match what was expected, so it was deleted. Try again.".into());
    }
    tokio::fs::rename(&part, dest).await.map_err(|_| "Buntini couldn’t save the download.".to_string())
}

/// Stops a helper program as soon as setup is cancelled.
async fn run_cancellable(mut command: tokio::process::Command, cancel: &AtomicBool) -> Result<std::process::Output, String> {
    command.kill_on_drop(true).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let child = command.spawn().map_err(|_| "A setup step couldn’t start.".to_string())?;
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    loop {
        tokio::select! {
            finished = &mut wait => return finished.map_err(|_| "A setup step stopped unexpectedly.".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Cancelled.".into());
                }
            }
        }
    }
}

/// `tar` refuses paths that leave the folder unless told otherwise, and this archive's hash
/// was checked. It unpacks to one side and moves in whole, so `python/` is only ever complete.
async fn unpack_python(base: &Path, archive: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let unpacking = base.join("unpacking");
    let _ = tokio::fs::remove_dir_all(&unpacking).await;
    tokio::fs::create_dir_all(&unpacking).await.map_err(|_| "Buntini couldn’t unpack Python.".to_string())?;
    let mut tar = tokio::process::Command::new("/usr/bin/tar");
    tar.arg("-xzf").arg(archive).arg("-C").arg(&unpacking);
    let output = run_cancellable(tar, cancel).await?;
    if !output.status.success() || !unpacking.join("python/bin/python3").is_file() {
        return Err("Buntini couldn’t unpack Python.".into());
    }
    let _ = tokio::fs::remove_dir_all(base.join("python")).await;
    tokio::fs::rename(unpacking.join("python"), base.join("python")).await.map_err(|_| "Buntini couldn’t unpack Python.".to_string())?;
    let _ = tokio::fs::remove_dir_all(&unpacking).await;
    Ok(())
}

/// The packages, each checked by pip against the lock. Nothing but wheels: no package gets to
/// run its own setup code.
async fn install_packages(base: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let marker = base.join("packages-installed");
    let wanted: String = Sha256::digest(LOCK).iter().map(|byte| format!("{byte:02x}")).collect();
    if tokio::fs::read_to_string(&marker).await.is_ok_and(|found| found == wanted) {
        return Ok(());
    }
    let lock = base.join("requirements.lock");
    tokio::fs::write(&lock, LOCK).await.map_err(|_| "Buntini couldn’t prepare the packages.".to_string())?;
    let temp = base.join("tmp");
    tokio::fs::create_dir_all(&temp).await.map_err(|_| "Buntini couldn’t prepare the packages.".to_string())?;
    let mut pip = tokio::process::Command::new(python_of(base));
    pip.args(["-I", "-m", "pip", "install", "--isolated", "--disable-pip-version-check", "--no-input", "--no-cache-dir", "--only-binary", ":all:", "--no-deps", "--require-hashes", "--retries", "5", "--timeout", "60", "-r"])
        .arg(&lock)
        .env_clear()
        .env("HOME", base)
        .env("TMPDIR", &temp)
        .env("PATH", "/usr/bin:/bin");
    let output = run_cancellable(pip, cancel).await?;
    let _ = tokio::fs::remove_dir_all(&temp).await;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr).to_uppercase();
        return Err(if why.contains("HASH") {
            "A package didn’t match what was expected, so nothing was installed. Try again.".into()
        } else {
            "The packages didn’t install. Check the internet connection and try again.".into()
        });
    }
    tokio::fs::write(&marker, &wanted).await.map_err(|_| "Buntini couldn’t finish installing the packages.".to_string())
}

/// A model's file, only ever inside its own folder.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn status(phase: &str, done: u64, total: u64, message: &str) -> Status {
    Status { phase: phase.into(), done_bytes: done, total_bytes: total, message: message.into(), ..Status::default() }
}

async fn run(state: &AppState, repo: &str, cancel: &AtomicBool, tell: &(dyn Fn(Status) + Send + Sync)) -> Result<(String, String), String> {
    let base = base(state);
    let model = pins().models.get(repo).ok_or("That model isn’t one Buntini can set up.")?;
    tokio::fs::create_dir_all(&base).await.map_err(|_| "Buntini couldn’t make its folder.".to_string())?;
    // Gigabytes that can be fetched again have no place in a backup.
    let mut exclude = tokio::process::Command::new("/usr/bin/tmutil");
    exclude.arg("addexclusion").arg(&base);
    let _ = run_cancellable(exclude, cancel).await;

    let python = &pins().python;
    let total = python.bytes + pins().package_bytes + model.bytes;
    let client = download_client()?;
    let told = Mutex::new(Instant::now() - Duration::from_secs(1));
    let report = |done: u64| {
        let mut last = told.lock().unwrap();
        if last.elapsed() >= Duration::from_millis(250) {
            *last = Instant::now();
            tell(status("downloading", done, total, ""));
        }
    };
    let archive = base.join("downloads/python.tar.gz");
    if !python_of(&base).is_file() {
        let mut waiting = |message: &str| tell(status("waiting", 0, total, message));
        fetch(&client, &python.url, &python.sha256, python.bytes, &archive, cancel, &mut waiting, &mut |done| report(done)).await?;
        unpack_python(&base, &archive, cancel).await?;
        let _ = tokio::fs::remove_file(&archive).await;
    }
    let mut finished = python.bytes;
    tell(status("installing", finished, total, ""));
    install_packages(&base, cancel).await?;
    finished += pins().package_bytes;

    let folder = model_of(&base, repo);
    for file in &model.files {
        if !safe_relative(&file.path) {
            return Err("That model has an unexpected file.".into());
        }
        let url = format!("https://huggingface.co/{}/resolve/{}/{}", model.repo, model.revision, file.path);
        let so_far = finished;
        let mut waiting = |message: &str| tell(status("waiting", so_far, total, message));
        fetch(&client, &url, &file.sha256, file.bytes, &folder.join(&file.path), cancel, &mut waiting, &mut |done| report(so_far + done)).await?;
        finished += file.bytes;
    }

    tell(status("testing", total, total, ""));
    let python_path = python_of(&base).to_string_lossy().into_owned();
    let model_path = folder.to_string_lossy().into_owned();
    let settings = state.store.lock().unwrap().settings()?;
    let trial = crate::types::Settings { model_backend: "mlx".into(), python_path: python_path.clone(), model_path: model_path.clone(), ..settings };
    let quiet = Arc::new(AtomicBool::new(false));
    let turn = state
        .models
        .generate(&trial, json!({"messages":[{"role":"user","content":"Say hello in one word."}],"tools":[],"max_tokens":16}), quiet, |_| {})
        .await
        .map_err(|_| "The model didn’t answer its test. Try again.".to_string())?;
    if turn.text.trim().is_empty() {
        return Err("The model didn’t answer its test. Try again.".into());
    }
    Ok((python_path, model_path))
}

/// Saves where the model is, waiting out any task that holds the settings.
async fn hand_over(app: &tauri::AppHandle, python: &str, model: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut tries = 0;
    loop {
        let mut settings = state.store.lock().unwrap().settings()?;
        settings.python_path = python.into();
        settings.model_path = model.into();
        match crate::controller::save_settings_in(&state, settings).await {
            Ok(_) => break,
            Err(_) if tries < 300 => {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error),
        }
    }
    let _ = state.store.lock().unwrap().set_preference(IN_PROGRESS, "");
    crate::brain::settle_and_tell(app).await;
    Ok(())
}

/// Runs setup for `repo` to the end. Cancelling leaves nothing behind.
async fn drive(app: tauri::AppHandle, repo: String) {
    let cancel = Arc::new(AtomicBool::new(false));
    *CANCEL.lock().unwrap() = Some(cancel.clone());
    let state = app.state::<AppState>();
    let _ = state.store.lock().unwrap().set_preference(IN_PROGRESS, &repo);
    let tell = |status: Status| publish(&app, status);
    let outcome = match run(&state, &repo, &cancel, &tell).await {
        Ok((python, model)) => hand_over(&app, &python, &model).await.map(|()| (python, model)),
        Err(error) => Err(error),
    };
    *CANCEL.lock().unwrap() = None;
    if cancel.load(Ordering::SeqCst) {
        let _ = state.store.lock().unwrap().set_preference(IN_PROGRESS, "");
        let _ = tokio::fs::remove_dir_all(base(&state)).await;
        publish(&app, Status { phase: "idle".into(), ..Status::default() });
        return;
    }
    match outcome {
        Ok((python, model)) => publish(&app, Status { phase: "ready".into(), python: Some(python), model: Some(model), ..Status::default() }),
        Err(message) => publish(&app, Status { phase: "failed".into(), message, ..Status::default() }),
    }
}

fn running() -> bool {
    CANCEL.lock().unwrap().is_some()
}

/// Starts or retries setup with the model `local_setup_plan` names.
#[tauri::command]
pub fn local_setup_start(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    if running() {
        return Ok(());
    }
    let plan = local_setup_plan(state);
    if let Some(problem) = plan.problem {
        return Err(problem);
    }
    publish(&app, status("downloading", 0, plan.download_bytes, ""));
    tauri::async_runtime::spawn(drive(app, plan.repo));
    Ok(())
}

#[tauri::command]
pub fn local_setup_cancel(state: tauri::State<'_, AppState>, app: tauri::AppHandle) {
    match CANCEL.lock().unwrap().as_ref() {
        Some(cancel) => cancel.store(true, Ordering::SeqCst),
        None => {
            // A failed setup has stopped already, with its partial files still there.
            let _ = state.store.lock().unwrap().set_preference(IN_PROGRESS, "");
            let _ = std::fs::remove_dir_all(base(&state));
            publish(&app, Status { phase: "idle".into(), ..Status::default() });
        }
    }
}

/// Frees the disk. A brain that was using the model moves on when it can.
#[tauri::command]
pub async fn local_setup_remove(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    if running() {
        return Err("Cancel the setup first.".into());
    }
    let mut settings = state.store.lock().unwrap().settings()?;
    let ours = base(&state);
    if Path::new(&settings.python_path).starts_with(&ours) {
        settings.python_path.clear();
        settings.model_path.clear();
        crate::controller::save_settings_in(&state, settings).await?;
    }
    state.models.reset().await;
    match tokio::fs::remove_dir_all(&ours).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err("Buntini couldn’t remove the model.".into()),
        _ => {}
    }
    publish(&app, Status { phase: "idle".into(), ..Status::default() });
    crate::brain::settle_and_tell(&app).await;
    Ok(())
}

/// Picks an unfinished setup up again after a restart.
pub async fn resume(app: tauri::AppHandle) {
    let repo = app.state::<AppState>().store.lock().unwrap().preference(IN_PROGRESS).ok().flatten().filter(|repo| !repo.is_empty());
    if let Some(repo) = repo {
        drive(app, repo).await;
    }
}

/// Delete everything: stops a setup and removes what it fetched.
pub fn erase(state: &AppState) {
    if let Some(cancel) = CANCEL.lock().unwrap().as_ref() {
        cancel.store(true, Ordering::SeqCst);
    }
    let _ = std::fs::remove_dir_all(base(state));
    *STATUS.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn every_advisor_model_is_pinned_with_safe_paths_and_the_sizes_add_up() {
        assert_eq!(pins().models.len(), crate::advisor::MODELS.len());
        for model in crate::advisor::MODELS {
            let pin = &pins().models[model.repo];
            assert!(pin.repo.starts_with("mlx-community/") && pin.revision.len() == 40, "{}", model.repo);
            assert_eq!(pin.bytes, pin.files.iter().map(|file| file.bytes).sum::<u64>());
            assert!(pin.files.iter().any(|file| file.path == "config.json"), "{}", model.repo);
            for file in &pin.files {
                assert!(safe_relative(&file.path), "{}", file.path);
                assert!(file.sha256.len() == 64 && file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()), "{}", file.path);
            }
        }
        assert!(pins().python.url.starts_with("https://github.com/astral-sh/python-build-standalone/"));
        assert!(LOCK.lines().filter(|line| !line.starts_with('#')).all(|line| line.contains("==") && line.contains("--hash=sha256:")), "every package is pinned and hashed");
        assert!(!safe_relative("../secret") && !safe_relative("/etc/passwd") && !safe_relative(""));
    }

    #[test]
    fn one_click_only_fetches_openly_licensed_models_that_suit_the_mac() {
        for (memory, chip) in [(8, "Apple M1"), (16, "Apple M4"), (16, "Apple M3 Pro"), (36, "Apple M3 Max"), (64, "Apple M2 Ultra")] {
            let model = pick(memory, chip);
            assert_eq!(pins().models[model.repo].license, "apache-2.0", "{chip} {memory}");
            assert!(model.download_gb <= memory as f64 * 0.6, "{chip} {memory}: {}", model.repo);
        }
        assert_eq!(pick(8, "Apple M1").repo, "Qwen3.5-4B-MLX-4bit");
        assert_eq!(pick(16, "Apple M4").repo, "Qwen3.5-9B-4bit", "Gemma's own licence is skipped");
    }

    #[test]
    fn a_mac_that_cant_is_told_why_before_anything_downloads() {
        let plenty = 500_000_000_000;
        assert_eq!(plan_for(16, "Apple M4", true, "15.3", plenty).problem, None);
        assert!(plan_for(16, "Intel", false, "15.3", plenty).problem.unwrap().contains("Apple silicon"));
        assert!(plan_for(16, "Apple M1", true, "13.6", plenty).problem.unwrap().contains("macOS 14"));
        assert!(plan_for(16, "Apple M4", true, "15.3", 4_000_000_000).problem.unwrap().contains("GB free"));
        let plan = plan_for(16, "Apple M4", true, "26.0", plenty);
        assert!(plan.download_bytes > pins().models[&plan.repo].bytes, "the size includes Python and packages");
    }

    /// Serves `body`, honouring ranges, and drops the connection partway on the first request.
    async fn server(body: Vec<u8>, drop_after: usize) -> (String, Arc<Mutex<Vec<Option<u64>>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let ranges = Arc::new(Mutex::new(vec![]));
        let seen = ranges.clone();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let read = socket.read(&mut request).await.unwrap();
                let text = String::from_utf8_lossy(&request[..read]).to_lowercase();
                let start = text.split("range: bytes=").nth(1).and_then(|rest| rest.split('-').next()).and_then(|n| n.parse::<u64>().ok());
                seen.lock().unwrap().push(start);
                let from = start.unwrap_or(0) as usize;
                let rest = &body[from..];
                let (status, range) = if start.is_some() { ("206 Partial Content", format!("Content-Range: bytes {from}-{}/{}\r\n", body.len() - 1, body.len())) } else { ("200 OK", String::new()) };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}Connection: close\r\n\r\n", rest.len());
                socket.write_all(head.as_bytes()).await.unwrap();
                let sent = if first { &rest[..drop_after.min(rest.len())] } else { rest };
                first = false;
                socket.write_all(sent).await.unwrap();
                socket.flush().await.unwrap();
                drop(socket);
            }
        });
        (url, ranges)
    }

    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder().build().unwrap()
    }

    #[tokio::test]
    async fn a_dropped_download_resumes_and_is_checked_before_it_gets_its_name() {
        let body: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        let sha: String = Sha256::digest(&body).iter().map(|byte| format!("{byte:02x}")).collect();
        let (url, ranges) = server(body.clone(), 60_000).await;
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("deep/model.safetensors");
        let cancel = AtomicBool::new(false);
        let (mut waited, mut latest) = (vec![], 0);
        fetch(&plain_client(), &url, &sha, body.len() as u64, &dest, &cancel, &mut |message| waited.push(message.to_string()), &mut |done| latest = done).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(ranges.lock().unwrap()[0], None);
        assert_eq!(ranges.lock().unwrap().iter().flatten().next(), Some(&60_000), "picked up where it stopped");
        assert_eq!(latest, body.len() as u64);
        assert!(!waited.is_empty(), "the wait was shown");
        assert_eq!(std::fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1, "no partial file left");
    }

    #[tokio::test]
    async fn a_tampered_download_is_deleted_and_never_named() {
        let body = vec![7u8; 5_000];
        let (url, _) = server(body.clone(), usize::MAX).await;
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("python.tar.gz");
        let error = fetch(&plain_client(), &url, &"0".repeat(64), body.len() as u64, &dest, &AtomicBool::new(false), &mut |_| {}, &mut |_| {}).await.unwrap_err();
        assert!(error.contains("didn’t match"), "{error}");
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 0, "nothing left behind");
    }

    #[tokio::test]
    async fn cancelling_stops_a_download_that_cant_connect() {
        let folder = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            flag.store(true, Ordering::SeqCst);
        });
        let dest = folder.path().join("file");
        let error = fetch(&plain_client(), "http://127.0.0.1:9/nothing", &"0".repeat(64), 10, &dest, &cancel, &mut |_| {}, &mut |_| {}).await.unwrap_err();
        assert_eq!(error, "Cancelled.");
    }

    /// From an empty folder with this project's worker: downloads about 0.8 GB.
    #[tokio::test]
    #[ignore = "downloads Python, packages and a model; run with --ignored live_local_setup"]
    async fn live_local_setup() {
        let folder = tempfile::tempdir().unwrap();
        let state = AppState::new(folder.path().join("data"), folder.path().join("cache"), folder.path().into()).unwrap();
        let repo = "Qwen3.5-0.8B-MLX-4bit";
        let total = pins().python.bytes + pins().package_bytes + pins().models[repo].bytes;
        let seen = Arc::new(Mutex::new(Vec::<Status>::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        // Stop partway through the model, as quitting would.
        let (flag, watched) = (cancel.clone(), seen.clone());
        let tell = move |status: Status| {
            if status.done_bytes > total / 2 {
                flag.store(true, Ordering::SeqCst);
            }
            watched.lock().unwrap().push(status);
        };
        assert_eq!(run(&state, repo, &cancel, &tell).await.unwrap_err(), "Cancelled.");
        let stopped = seen.lock().unwrap().last().unwrap().done_bytes;
        println!("stopped at {} MB of {} MB", stopped / 1_000_000, total / 1_000_000);

        seen.lock().unwrap().clear();
        let watched = seen.clone();
        let started = Instant::now();
        let (python, model) = run(&state, repo, &AtomicBool::new(false), &move |status: Status| watched.lock().unwrap().push(status)).await.unwrap();
        let phases = seen.lock().unwrap().iter().map(|status| status.phase.clone()).collect::<std::collections::BTreeSet<_>>();
        // Python, packages and the small files come first. Past them, the big file carries on
        // from where it stopped, with nothing in between fetched again.
        let small = pins().python.bytes + pins().package_bytes + 1_000_000;
        let first = seen.lock().unwrap().iter().filter(|status| status.done_bytes > small + 50_000_000).map(|status| status.done_bytes).min().unwrap_or(0);
        println!("resumed at {} MB, finished in {:?}, phases {phases:?}", first / 1_000_000, started.elapsed());
        assert!(first >= stopped, "kept what it had: {first} after stopping at {stopped}");
        assert!(phases.contains("testing"));
        assert!(python.starts_with(&*folder.path().to_string_lossy()) && model.contains(repo));
        let on_disk = std::process::Command::new("/usr/bin/du").args(["-sh"]).arg(folder.path()).output().unwrap();
        println!("on disk: {}", String::from_utf8_lossy(&on_disk.stdout).trim());
        // The real answer came from the test step; ask once more with a real question.
        let settings = crate::types::Settings { model_backend: "mlx".into(), python_path: python, model_path: model, ..crate::types::Settings::default() };
        let turn = state.models.generate(&settings, json!({"messages":[{"role":"user","content":"What is the capital of France? One word."}],"tools":[],"max_tokens":24}), Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();
        println!("answer: {}", turn.text.trim());
        state.models.shutdown().await;
    }

    #[tokio::test]
    async fn a_file_already_checked_isnt_fetched_again() {
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("file");
        std::fs::write(&dest, b"12345").unwrap();
        let mut latest = 0;
        fetch(&plain_client(), "http://127.0.0.1:9/never", &"0".repeat(64), 5, &dest, &AtomicBool::new(false), &mut |_| {}, &mut |done| latest = done).await.unwrap();
        assert_eq!(latest, 5);
    }
}
