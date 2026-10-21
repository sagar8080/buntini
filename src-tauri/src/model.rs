//! The model hub: which brain answers, whether it is ready, what it may see and how
//! much context it can take. Every model call in the app goes through here.
//!
//! Backends: a local MLX model run by the Python worker, Apple Intelligence through
//! the `buntini-fm` helper, or a cloud model with the user's key. On-device backends
//! see local data freely; cloud models only what `privacy.rs` allows.
use crate::{
    privacy::Locality,
    types::Settings,
    worker::{Launch, Turn, WorkerManager},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Mlx,
    Apple,
    Cloud,
}

pub const BACKENDS: [&str; 3] = ["apple", "mlx", "cloud"];

/// Context sizes per backend: how much earlier conversation fits, and the reply length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub history_tokens: usize,
    pub max_tokens: u32,
}

pub struct ModelHub {
    /// Python MLX worker: chat with a local MLX model, and Whisper transcription.
    pub mlx: WorkerManager,
    /// Apple Intelligence helper (Foundation Models and on-device speech).
    pub apple: WorkerManager,
    mlx_script: PathBuf,
    apple_helper: PathBuf,
    /// Apple Intelligence availability from the helper's last ping: (available, reason).
    apple_state: std::sync::Mutex<Option<(bool, String)>>,
}

pub fn backend(settings: &Settings) -> Backend {
    match settings.model_backend.as_str() {
        "apple" => Backend::Apple,
        "cloud" => Backend::Cloud,
        _ => Backend::Mlx,
    }
}

/// A local MLX model is usable when its folder holds a `config.json`.
pub fn mlx_model_present(path: &str) -> bool {
    !path.is_empty() && Path::new(path).join("config.json").is_file()
}

impl ModelHub {
    pub fn new(mlx_script: PathBuf, apple_helper: PathBuf) -> Self {
        Self { mlx: WorkerManager::new("local MLX model"), apple: WorkerManager::new("Apple Intelligence helper"), mlx_script, apple_helper, apple_state: std::sync::Mutex::new(None) }
    }

    pub fn locality(&self, settings: &Settings) -> Locality {
        match backend(settings) {
            Backend::Cloud => Locality::External,
            Backend::Mlx | Backend::Apple => Locality::OnDevice,
        }
    }

    /// Whether the chosen backend can answer right now, with advice when it cannot.
    pub fn ready(&self, settings: &Settings) -> Result<(), String> {
        match backend(settings) {
            Backend::Mlx => {
                if settings.python_path.trim().is_empty() {
                    return Err("Choose the Python environment that has MLX installed in Settings → Brain to chat with a local model.".into());
                }
                if !mlx_model_present(&settings.model_path) {
                    return Err("Choose your local MLX model folder in Settings → Brain to chat. File search and system health already work offline.".into());
                }
                Ok(())
            }
            Backend::Apple => match self.apple_status() {
                Some((true, _)) => Ok(()),
                Some((false, reason)) => Err(apple_advice(&reason)),
                None if self.apple_helper.is_file() => Ok(()),
                None => Err(apple_advice("missing")),
            },
            Backend::Cloud => Err("Cloud models aren’t available in this version yet. Choose a local model in Settings → Brain.".into()),
        }
    }

    pub fn budget(&self, settings: &Settings) -> Budget {
        match backend(settings) {
            Backend::Apple => Budget { history_tokens: 1200, max_tokens: 512 },
            Backend::Mlx => Budget { history_tokens: 6000, max_tokens: 1024 },
            Backend::Cloud => Budget { history_tokens: 12000, max_tokens: 1024 },
        }
    }

    /// The opening line of every system prompt. Only on-device models are told they are private.
    pub fn system_preamble(&self, settings: &Settings) -> &'static str {
        match self.locality(settings) {
            Locality::OnDevice => "You are a private local desktop companion running entirely on this Mac.",
            Locality::External => "You are a desktop companion. You are running on a cloud model the user chose, and you only see what the user allowed it to share.",
        }
    }

    pub fn status(&self, settings: &Settings) -> String {
        match backend(settings) {
            Backend::Mlx => self.mlx.status.lock().unwrap().clone(),
            Backend::Apple => match self.apple_status() {
                Some((true, _)) => "Ready · Apple Intelligence on this Mac".into(),
                Some((false, reason)) => apple_advice(&reason),
                None => "Checking Apple Intelligence…".into(),
            },
            Backend::Cloud => "Cloud model · not configured".into(),
        }
    }

    fn mlx_launch(&self, settings: &Settings) -> Result<Launch, String> {
        if settings.python_path.trim().is_empty() {
            return Err("Choose the Python environment that has MLX installed in Settings → Brain.".into());
        }
        if !self.mlx_script.is_file() {
            return Err("The MLX worker is missing from this installation.".into());
        }
        Ok(Launch {
            program: PathBuf::from(&settings.python_path),
            args: vec!["-u".into(), self.mlx_script.to_string_lossy().into_owned()],
            env: vec![("PYTHONUNBUFFERED", "1".into()), ("HF_HUB_OFFLINE", "1".into()), ("TRANSFORMERS_OFFLINE", "1".into())],
            key: format!("{}\u{0}{}", settings.python_path, settings.model_path),
            start_error: "Cannot start Python. Select your existing MLX virtual environment’s Python executable in Settings → Brain.".into(),
        })
    }

    fn apple_launch(&self) -> Result<Launch, String> {
        if !self.apple_helper.is_file() {
            return Err(apple_advice("missing"));
        }
        Ok(Launch {
            program: self.apple_helper.clone(),
            args: vec!["--tools".into(), self.mlx_script.with_file_name("tools.json").to_string_lossy().into_owned()],
            env: vec![],
            key: "apple".into(),
            start_error: apple_advice("missing"),
        })
    }

    pub fn apple_status(&self) -> Option<(bool, String)> {
        self.apple_state.lock().unwrap().clone()
    }

    /// Asks the helper whether Apple Intelligence can run here and remembers the answer,
    /// so the synchronous bootstrap and readiness checks can report it.
    pub async fn probe_apple(&self) -> (bool, String) {
        let status = match self.apple_launch() {
            Err(_) => (false, "missing".to_string()),
            Ok(launch) => match self.apple.request(&launch, serde_json::json!({"type":"ping"}), Arc::new(AtomicBool::new(false)), |_| {}).await {
                Ok(turn) => {
                    let pong: Value = serde_json::from_str(&turn.text).unwrap_or_default();
                    (pong["available"].as_bool().unwrap_or(false), pong["reason"].as_str().unwrap_or("unavailable").to_string())
                }
                Err(_) => (false, "unsupported".to_string()),
            },
        };
        *self.apple_state.lock().unwrap() = Some(status.clone());
        status
    }

    /// Voice uses Apple's on-device speech unless a local Whisper model is set up and
    /// the user's brain is the local MLX model.
    pub fn apple_speech(&self, settings: &Settings) -> bool {
        backend(settings) == Backend::Apple || !mlx_model_present(&settings.whisper_model)
    }

    pub fn voice_ready(&self, settings: &Settings) -> Result<(), String> {
        if !self.apple_speech(settings) {
            return Ok(());
        }
        match self.apple_status() {
            Some((false, reason)) if reason == "unsupported" || reason == "missing" => Err("Voice needs macOS 26 for Apple speech recognition, or a local MLX Whisper model folder in Settings.".into()),
            _ if !self.apple_helper.is_file() => Err("Voice needs a local MLX Whisper model folder in Settings.".into()),
            _ => Ok(()),
        }
    }

    pub async fn transcribe(&self, settings: &Settings, audio_path: &Path, cancel: Arc<AtomicBool>) -> Result<String, String> {
        let request = if self.apple_speech(settings) {
            self.apple.request(&self.apple_launch()?, serde_json::json!({"type":"transcribe","audio_path":audio_path}), cancel, |_| {}).await
        } else {
            self.mlx_request(settings, serde_json::json!({"type":"transcribe","audio_path":audio_path,"model_path":settings.whisper_model,"vad_model_path":settings.vad_model_path}), cancel).await
        };
        request.map(|turn| turn.text)
    }

    /// Generates with the chosen backend. Callers send the generic payload
    /// (`messages`, `tools`, `max_tokens`); backend-specific fields are added here.
    pub async fn generate<F>(&self, settings: &Settings, mut payload: Value, cancel: Arc<AtomicBool>, on_token: F) -> Result<Turn, String>
    where
        F: FnMut(String) + Send,
    {
        self.ready(settings)?;
        payload["type"] = "generate".into();
        match backend(settings) {
            Backend::Mlx => {
                payload["model_path"] = settings.model_path.clone().into();
                self.mlx.request(&self.mlx_launch(settings)?, payload, cancel, on_token).await
            }
            Backend::Apple => self.apple.request(&self.apple_launch()?, payload, cancel, on_token).await,
            Backend::Cloud => Err(self.ready(settings).unwrap_err()),
        }
    }

    /// Sends a raw protocol request (ping, transcribe) to the Python worker.
    pub async fn mlx_request(&self, settings: &Settings, payload: Value, cancel: Arc<AtomicBool>) -> Result<Turn, String> {
        self.mlx.request(&self.mlx_launch(settings)?, payload, cancel, |_| {}).await
    }

    pub async fn reset(&self) {
        self.mlx.reset().await;
        self.apple.reset().await;
    }

    pub async fn shutdown(&self) {
        self.mlx.shutdown().await;
        self.apple.shutdown().await;
    }
}

/// What to tell the user when Apple Intelligence can't run.
pub fn apple_advice(reason: &str) -> String {
    match reason {
        "deviceNotEligible" => "This Mac doesn’t support Apple Intelligence. Choose a local or cloud model in Settings → Brain.",
        "appleIntelligenceNotEnabled" => "Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri, then try again.",
        "modelNotReady" => "Apple Intelligence is still downloading its model. Try again in a few minutes.",
        "unsupported" => "Apple Intelligence needs macOS 26 on an Apple silicon Mac. Choose a local or cloud model in Settings → Brain.",
        "missing" => "The Apple Intelligence helper is missing from this installation. Reinstall Buntini or choose another brain in Settings.",
        _ => "Apple Intelligence isn’t available right now. Choose a local or cloud model in Settings → Brain.",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(backend: &str) -> Settings {
        Settings { model_backend: backend.into(), python_path: String::new(), model_path: String::new(), whisper_model: String::new(), ..Settings::default() }
    }

    #[test]
    fn locality_follows_the_backend() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::new());
        assert_eq!(hub.locality(&settings("mlx")), Locality::OnDevice);
        assert_eq!(hub.locality(&settings("apple")), Locality::OnDevice);
        assert_eq!(hub.locality(&settings("cloud")), Locality::External);
        assert!(hub.system_preamble(&settings("mlx")).contains("private"));
        assert!(!hub.system_preamble(&settings("cloud")).contains("private"));
        assert!(hub.budget(&settings("apple")).history_tokens < hub.budget(&settings("mlx")).history_tokens);
    }

    #[test]
    fn apple_readiness_follows_the_helper_report() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::from("/nonexistent/buntini-fm"));
        let apple = settings("apple");
        assert!(hub.ready(&apple).unwrap_err().contains("missing"));
        *hub.apple_state.lock().unwrap() = Some((false, "appleIntelligenceNotEnabled".into()));
        assert!(hub.ready(&apple).unwrap_err().contains("Turn on Apple Intelligence"));
        *hub.apple_state.lock().unwrap() = Some((true, "available".into()));
        assert!(hub.ready(&apple).is_ok());
        assert!(hub.apple_speech(&apple));
        let mut mlx = settings("mlx");
        assert!(hub.apple_speech(&mlx), "no Whisper model: use Apple speech");
        let whisper = tempfile::tempdir().unwrap();
        std::fs::write(whisper.path().join("config.json"), "{}").unwrap();
        mlx.whisper_model = whisper.path().to_string_lossy().into_owned();
        assert!(!hub.apple_speech(&mlx));
    }

    #[tokio::test]
    #[ignore = "uses Apple Intelligence on this Mac; build native/buntini-fm first, run with --ignored live_apple"]
    async fn live_apple_routes_tools_and_streams_answers() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let hub = ModelHub::new(root.join("worker/main.py"), root.join("native/buntini-fm"));
        let (available, reason) = hub.probe_apple().await;
        if !available {
            eprintln!("skipping: Apple Intelligence unavailable ({reason})");
            return;
        }
        let apple = settings("apple");
        let ask = |prompt: &str, tools: Value| serde_json::json!({"messages":[{"role":"system","content":"You are a concise desktop companion."},{"role":"user","content":prompt}],"tools":tools,"max_tokens":120});
        let turn = hub.generate(&apple, ask("how much memory is my mac using right now?", crate::tools::offer(&["search_files", "system_info"])), Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();
        assert_eq!(turn.tool.as_ref().and_then(|call| call["name"].as_str()), Some("system_info"));
        let mut streamed = String::new();
        let turn = hub.generate(&apple, ask("Say hello in five words.", Value::Array(vec![])), Arc::new(AtomicBool::new(false)), |token| streamed.push_str(&token)).await.unwrap();
        assert!(turn.tool.is_none() && !turn.text.is_empty());
        assert_eq!(streamed, turn.text, "streamed tokens add up to the answer");
        hub.shutdown().await;
    }

    #[test]
    fn mlx_readiness_needs_python_and_a_model_folder() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::new());
        let mut mlx = settings("mlx");
        assert!(hub.ready(&mlx).unwrap_err().contains("Python"));
        mlx.python_path = "/usr/bin/python3".into();
        assert!(hub.ready(&mlx).unwrap_err().contains("model folder"));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), "{}").unwrap();
        mlx.model_path = dir.path().to_string_lossy().into_owned();
        assert!(hub.ready(&mlx).is_ok());
    }
}
