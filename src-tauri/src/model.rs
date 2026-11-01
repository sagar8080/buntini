//! The model hub: which brain answers, whether it is ready, what it may see and how
//! much context it can take. Every model call in the app goes through here.
//!
//! Backends: a local MLX model run by the Python worker, Apple Intelligence through
//! the `buntini-fm` helper, or a cloud model with the user's key. On-device backends
//! see local data freely; cloud models only what `privacy.rs` allows.
use crate::{
    cloud,
    privacy::Locality,
    store::Store,
    types::Settings,
    worker::{Launch, Turn, WorkerManager},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
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
    /// Where cloud spending is reserved and settled.
    ledger: OnceLock<Arc<Mutex<Store>>>,
    /// Shows the 80% spending notice.
    notice: OnceLock<Box<dyn Fn(String) + Send + Sync>>,
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
        Self {
            mlx: WorkerManager::new("local MLX model"),
            apple: WorkerManager::new("Apple Intelligence helper"),
            mlx_script,
            apple_helper,
            apple_state: std::sync::Mutex::new(None),
            ledger: OnceLock::new(),
            notice: OnceLock::new(),
        }
    }

    /// The Swift helper, which also reads Calendar and Reminders (`apps.rs`).
    pub fn apple_helper(&self) -> &Path {
        &self.apple_helper
    }

    pub fn set_ledger(&self, store: Arc<Mutex<Store>>) {
        let _ = self.ledger.set(store);
    }

    pub fn on_notice(&self, notify: impl Fn(String) + Send + Sync + 'static) {
        let _ = self.notice.set(Box::new(notify));
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
            Backend::Cloud => {
                let provider = cloud::provider(&settings.cloud_provider)
                    .ok_or("Choose a cloud model in Settings → Brain.")?;
                if provider.id == "chatgpt" {
                    return match crate::chatgpt::load() {
                        Some(session) if session.plan_allowed() => Ok(()),
                        Some(_) => Err("Your ChatGPT sign-in doesn’t allow Buntini to use your plan. Sign in again and allow plan usage.".into()),
                        None => Err("Sign in with ChatGPT in Settings → Brain to use your ChatGPT plan.".into()),
                    };
                }
                if crate::web::credential(provider.id).is_err() {
                    return Err(format!(
                        "Add your {} API key in Settings → Brain to use the cloud model.",
                        provider.name
                    ));
                }
                if settings.cloud_budget_usd <= 0.0 {
                    return Err("Set a monthly spending cap above $0 in Settings → Brain to use the cloud model.".into());
                }
                Ok(())
            }
        }
    }

    pub fn cloud_model(settings: &Settings) -> String {
        match settings.cloud_model.trim() {
            "" => cloud::provider(&settings.cloud_provider)
                .map(|provider| provider.model)
                .unwrap_or_default()
                .to_string(),
            model => model.to_string(),
        }
    }

    pub fn budget(&self, settings: &Settings) -> Budget {
        match backend(settings) {
            Backend::Apple => Budget {
                history_tokens: 1200,
                max_tokens: 512,
            },
            Backend::Mlx => Budget {
                history_tokens: 6000,
                max_tokens: 1024,
            },
            // Thinking counts toward the reply allowance on current cloud models.
            Backend::Cloud => Budget {
                history_tokens: 12000,
                max_tokens: 4096,
            },
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
            Backend::Cloud => match self.ready(settings) {
                Ok(()) => format!(
                    "Ready · {} {}",
                    cloud::provider(&settings.cloud_provider)
                        .map(|provider| provider.name)
                        .unwrap_or("Cloud"),
                    Self::cloud_model(settings)
                ),
                Err(advice) => advice,
            },
        }
    }

    fn mlx_launch(&self, settings: &Settings) -> Result<Launch, String> {
        if settings.python_path.trim().is_empty() {
            return Err(
                "Choose the Python environment that has MLX installed in Settings → Brain.".into(),
            );
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
            args: vec![
                "--tools".into(),
                self.mlx_script
                    .with_file_name("tools.json")
                    .to_string_lossy()
                    .into_owned(),
            ],
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
            Ok(launch) => match self
                .apple
                .request(
                    &launch,
                    serde_json::json!({"type":"ping"}),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .await
            {
                Ok(turn) => {
                    let pong: Value = serde_json::from_str(&turn.text).unwrap_or_default();
                    (
                        pong["available"].as_bool().unwrap_or(false),
                        pong["reason"].as_str().unwrap_or("unavailable").to_string(),
                    )
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
            Some((false, reason)) if reason == "unsupported" || reason == "missing" => Err("Voice needs macOS 26, which has Apple’s speech recognition.".into()),
            _ if !self.apple_helper.is_file() => Err("Voice needs Buntini’s Apple helper, which this copy of Buntini doesn’t include. Reinstall Buntini to use voice.".into()),
            _ => Ok(()),
        }
    }

    pub async fn transcribe(
        &self,
        settings: &Settings,
        audio_path: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<String, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err("Task cancelled.".into());
        }
        let request = if self.apple_speech(settings) {
            self.apple
                .request(
                    &self.apple_launch()?,
                    serde_json::json!({"type":"transcribe","audio_path":audio_path}),
                    cancel,
                    |_| {},
                )
                .await
        } else {
            self.mlx_request(settings, serde_json::json!({"type":"transcribe","audio_path":audio_path,"model_path":settings.whisper_model,"vad_model_path":settings.vad_model_path}), cancel).await
        };
        request.map(|turn| turn.text)
    }

    /// Generates with the chosen backend. Callers send the generic payload
    /// (`messages`, `tools`, `max_tokens`); backend-specific fields are added here.
    pub async fn generate<F>(
        &self,
        settings: &Settings,
        mut payload: Value,
        cancel: Arc<AtomicBool>,
        on_token: F,
    ) -> Result<Turn, String>
    where
        F: FnMut(String) + Send,
    {
        if cancel.load(Ordering::SeqCst) {
            return Err("Task cancelled.".into());
        }
        self.ready(settings)?;
        payload["type"] = "generate".into();
        match backend(settings) {
            Backend::Mlx => {
                payload["model_path"] = settings.model_path.clone().into();
                self.mlx
                    .request(&self.mlx_launch(settings)?, payload, cancel, on_token)
                    .await
            }
            Backend::Apple => {
                self.apple
                    .request(&self.apple_launch()?, payload, cancel, on_token)
                    .await
            }
            Backend::Cloud => {
                self.cloud_generate(settings, payload, cancel, on_token)
                    .await
            }
        }
    }

    /// One cloud call: reserve the estimated cost, stream, then settle with real usage.
    /// Rejected requests cost nothing; anything accepted keeps at least its reservation
    /// until the provider reports usage.
    async fn cloud_generate<F>(
        &self,
        settings: &Settings,
        payload: Value,
        cancel: Arc<AtomicBool>,
        on_token: F,
    ) -> Result<Turn, String>
    where
        F: FnMut(String) + Send,
    {
        let provider = cloud::provider(&settings.cloud_provider)
            .ok_or("Choose a cloud provider in Settings → Brain.")?;
        // A ChatGPT plan isn't billed per token, so there's nothing to reserve.
        if provider.id == "chatgpt" {
            let token = crate::chatgpt::access_token().await?;
            let model = match settings.cloud_model.trim() {
                "" => crate::chatgpt::models()
                    .await?
                    .into_iter()
                    .next()
                    .map(|model| model.slug)
                    .ok_or("Your ChatGPT plan didn’t list any models.")?,
                chosen => chosen.to_string(),
            };
            return match cloud::generate(provider, &model, &token, &payload, cancel, on_token).await
            {
                Ok((turn, _)) => Ok(turn),
                Err(cloud::Failure::Rejected(message) | cloud::Failure::Failed(message)) => {
                    Err(message)
                }
            };
        }
        let ledger = self
            .ledger
            .get()
            .ok_or("Cloud spending can’t be tracked right now, so the cloud model is paused.")?;
        let key = crate::web::credential(provider.id)?;
        let (input_price, output_price) = settings.cloud_prices();
        let estimate = cloud::price(cloud::estimate(&payload), input_price, output_price);
        let account = cloud::ledger_key(provider.id);
        let reservation = ledger
            .lock()
            .unwrap()
            .reserve(&account, crate::budget::cloud_limits(settings, estimate))?;
        if let (Some(warning), Some(notify)) = (reservation.warning, self.notice.get()) {
            notify(warning);
        }
        let settle = |actual: f64| {
            let _ = ledger.lock().unwrap().settle(&account, estimate, actual);
        };
        match cloud::generate(
            provider,
            &Self::cloud_model(settings),
            &key,
            &payload,
            cancel,
            on_token,
        )
        .await
        {
            Ok((turn, usage)) => {
                if let Some(usage) = usage {
                    settle(cloud::price(usage, input_price, output_price));
                }
                Ok(turn)
            }
            Err(cloud::Failure::Rejected(message)) => {
                settle(0.0);
                Err(message)
            }
            Err(cloud::Failure::Failed(message)) => Err(message),
        }
    }

    /// Sends a raw protocol request (ping, transcribe) to the Python worker.
    pub async fn mlx_request(
        &self,
        settings: &Settings,
        payload: Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Turn, String> {
        self.mlx
            .request(&self.mlx_launch(settings)?, payload, cancel, |_| {})
            .await
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
        Settings {
            model_backend: backend.into(),
            python_path: String::new(),
            model_path: String::new(),
            whisper_model: String::new(),
            ..Settings::default()
        }
    }

    #[tokio::test]
    async fn cancelled_generation_stops_before_credentials_or_model_preflight() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::new());
        for backend in ["apple", "mlx", "cloud"] {
            for provider in ["anthropic", "openai", "gemini", "chatgpt"] {
                let mut settings = settings(backend);
                settings.cloud_provider = provider.into();
                let error = hub
                    .generate(
                        &settings,
                        serde_json::json!({"messages":[]}),
                        Arc::new(AtomicBool::new(true)),
                        |_| panic!("cancelled work cannot emit tokens"),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(error, "Task cancelled.", "{backend}/{provider}");
            }
        }
        assert!(hub.apple.status.lock().unwrap().starts_with("Not started"));
        assert!(hub.mlx.status.lock().unwrap().starts_with("Not started"));
    }

    #[tokio::test]
    async fn cancelled_transcription_never_opens_a_model_or_audio_file() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::new());
        for backend in ["apple", "mlx"] {
            let error = hub
                .transcribe(
                    &settings(backend),
                    Path::new("/nonexistent/private-audio.wav"),
                    Arc::new(AtomicBool::new(true)),
                )
                .await
                .unwrap_err();
            assert_eq!(error, "Task cancelled.");
        }
        assert!(hub.apple.status.lock().unwrap().starts_with("Not started"));
        assert!(hub.mlx.status.lock().unwrap().starts_with("Not started"));
    }

    #[test]
    fn locality_follows_the_backend() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::new());
        assert_eq!(hub.locality(&settings("mlx")), Locality::OnDevice);
        assert_eq!(hub.locality(&settings("apple")), Locality::OnDevice);
        assert_eq!(hub.locality(&settings("cloud")), Locality::External);
        assert!(hub.system_preamble(&settings("mlx")).contains("private"));
        assert!(!hub.system_preamble(&settings("cloud")).contains("private"));
        assert!(
            hub.budget(&settings("apple")).history_tokens
                < hub.budget(&settings("mlx")).history_tokens
        );
    }

    #[test]
    fn apple_readiness_follows_the_helper_report() {
        let hub = ModelHub::new(PathBuf::new(), PathBuf::from("/nonexistent/buntini-fm"));
        let apple = settings("apple");
        assert!(hub.ready(&apple).unwrap_err().contains("missing"));
        *hub.apple_state.lock().unwrap() = Some((false, "appleIntelligenceNotEnabled".into()));
        assert!(hub
            .ready(&apple)
            .unwrap_err()
            .contains("Turn on Apple Intelligence"));
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
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let hub = ModelHub::new(root.join("worker/main.py"), root.join("native/buntini-fm"));
        let (available, reason) = hub.probe_apple().await;
        if !available {
            eprintln!("skipping: Apple Intelligence unavailable ({reason})");
            return;
        }
        let apple = settings("apple");
        let ask = |prompt: &str, tools: Value| serde_json::json!({"messages":[{"role":"system","content":"You are a concise desktop companion."},{"role":"user","content":prompt}],"tools":tools,"max_tokens":120});
        let turn = hub
            .generate(
                &apple,
                ask(
                    "how much memory is my mac using right now?",
                    crate::tools::offer(&["search_files", "system_info"]),
                ),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(
            turn.tool.as_ref().and_then(|call| call["name"].as_str()),
            Some("system_info")
        );
        let mut streamed = String::new();
        let turn = hub
            .generate(
                &apple,
                ask("Say hello in five words.", Value::Array(vec![])),
                Arc::new(AtomicBool::new(false)),
                |token| streamed.push_str(&token),
            )
            .await
            .unwrap();
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
