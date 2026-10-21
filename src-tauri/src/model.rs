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
    #[allow(dead_code)] // Used once the Apple Intelligence helper is wired in.
    apple_helper: PathBuf,
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
        Self { mlx: WorkerManager::new("local MLX model"), apple: WorkerManager::new("Apple Intelligence helper"), mlx_script, apple_helper }
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
            Backend::Apple => Err("Apple Intelligence isn’t available in this version yet. Choose a local model in Settings → Brain.".into()),
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
            Backend::Apple => self.apple.status.lock().unwrap().clone(),
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
            Backend::Apple | Backend::Cloud => Err(self.ready(settings).unwrap_err()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(backend: &str) -> Settings {
        Settings { model_backend: backend.into(), python_path: String::new(), model_path: String::new(), ..Settings::default() }
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
