//! "Recommended for this Mac": picks the Qwen3.5 or Gemma 4 MLX model that suits this
//! Mac's chip and memory, gives the commands to set it up, and notices when the user
//! has finished.
//! Buntini never installs packages or downloads models itself.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Qwen3.5 and Gemma 4 MLX conversions on Hugging Face (mlx-community), with download
/// sizes checked in October 2026. Buntini's worker reads both families' tool calls.
pub struct Model {
    pub repo: &'static str,
    pub label: &'static str,
    pub download_gb: f64,
    pub note: &'static str,
}
pub const MODELS: [Model; 8] = [
    Model {
        repo: "Qwen3.5-0.8B-MLX-4bit",
        label: "Qwen3.5 0.8B",
        download_gb: 0.65,
        note: "Very quick, but simple answers.",
    },
    Model {
        repo: "Qwen3.5-4B-MLX-4bit",
        label: "Qwen3.5 4B",
        download_gb: 3.1,
        note: "Good everyday answers with room for your apps.",
    },
    Model {
        repo: "gemma-4-e2b-it-4bit",
        label: "Gemma 4 E2B",
        download_gb: 3.6,
        note: "Google’s small on-device model; quick on any Mac.",
    },
    Model {
        repo: "gemma-4-e4b-it-4bit",
        label: "Gemma 4 E4B",
        download_gb: 5.2,
        note: "Built to run fast on laptops; answers well for its size.",
    },
    Model {
        repo: "Qwen3.5-9B-4bit",
        label: "Qwen3.5 9B",
        download_gb: 6.0,
        note: "Noticeably smarter; the model Buntini is tested with.",
    },
    Model {
        repo: "Qwen3.5-35B-A3B-4bit",
        label: "Qwen3.5 35B-A3B",
        download_gb: 20.4,
        note: "Much stronger, and quick because it uses about 3B parameters per word.",
    },
    Model {
        repo: "gemma-4-31b-it-4bit",
        label: "Gemma 4 31B",
        download_gb: 18.4,
        note: "Google’s strongest open model; best on Max and Ultra chips.",
    },
    Model {
        repo: "Qwen3.5-27B-4bit",
        label: "Qwen3.5 27B",
        download_gb: 16.1,
        note: "Strong reasoning, slower than 35B-A3B.",
    },
];

fn model(repo: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.repo == repo)
}

/// How fast the chip can feed a model: memory bandwidth roughly doubles from base to
/// Pro to Max to Ultra, and speed follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Base,
    Pro,
    Max,
    Ultra,
}
pub fn tier(chip: &str) -> Tier {
    if chip.contains("Ultra") {
        Tier::Ultra
    } else if chip.contains("Max") {
        Tier::Max
    } else if chip.contains("Pro") {
        Tier::Pro
    } else {
        Tier::Base
    }
}

/// The best fit and an alternative, keeping the model to roughly half of memory so
/// macOS and your apps stay comfortable. Base chips get Gemma 4's efficient E-models
/// where Qwen's denser 9B would feel slow; Max and Ultra chips can run Gemma 4 31B.
pub fn recommend(memory_gb: u64, tier: Tier) -> (&'static Model, Option<&'static Model>) {
    let get = |repo: &str| model(repo).unwrap();
    match (memory_gb, tier) {
        (0..=7, _) => (get("Qwen3.5-0.8B-MLX-4bit"), None),
        (8..=15, _) => (get("Qwen3.5-4B-MLX-4bit"), Some(get("gemma-4-e2b-it-4bit"))),
        (16..=35, Tier::Base) => (get("gemma-4-e4b-it-4bit"), Some(get("Qwen3.5-9B-4bit"))),
        (16..=35, _) => (get("Qwen3.5-9B-4bit"), Some(get("gemma-4-e4b-it-4bit"))),
        (_, Tier::Max | Tier::Ultra) => (
            get("Qwen3.5-35B-A3B-4bit"),
            Some(get("gemma-4-31b-it-4bit")),
        ),
        _ => (get("Qwen3.5-35B-A3B-4bit"), Some(get("Qwen3.5-9B-4bit"))),
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub repo: String,
    pub label: String,
    pub download_gb: f64,
    pub note: String,
}
impl From<&Model> for Choice {
    fn from(model: &Model) -> Self {
        Self {
            repo: model.repo.into(),
            label: model.label.into(),
            download_gb: model.download_gb,
            note: model.note.into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub title: String,
    pub command: String,
    pub done: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub chip: String,
    pub memory_gb: u64,
    pub apple_silicon: bool,
    pub recommended: Choice,
    pub alternative: Option<Choice>,
    /// The model the steps below are for.
    pub selected: Choice,
    pub steps: Vec<Step>,
    /// Absolute paths, once each part is ready to use.
    pub python: Option<String>,
    pub model_folder: Option<String>,
}

const ENV: &str = "buntini-mlx";
const MODELS_DIR: &str = "buntini-models";

/// The environment is usable once its Python and the mlx_lm package are both there.
fn python_ready(env: &Path) -> bool {
    let python = env.join("bin/python");
    python.is_file()
        && std::fs::read_dir(env.join("lib"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.path().join("site-packages/mlx_lm").is_dir())
}

/// A download is complete when its config is there, no partial files remain and every
/// weight shard listed in the index exists.
pub fn model_ready(folder: &Path) -> bool {
    if !folder.join("config.json").is_file() {
        return false;
    }
    let partial = std::fs::read_dir(folder.join(".cache/huggingface/download"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".incomplete"));
    if partial {
        return false;
    }
    match std::fs::read_to_string(folder.join("model.safetensors.index.json")) {
        Ok(index) => serde_json::from_str::<serde_json::Value>(&index)
            .ok()
            .and_then(|value| {
                value["weight_map"].as_object().map(|map| {
                    map.values()
                        .filter_map(|file| file.as_str())
                        .all(|file| folder.join(file).is_file())
                })
            })
            .unwrap_or(false),
        Err(_) => folder.join("model.safetensors").is_file(),
    }
}

fn sysctl(name: &str) -> String {
    std::process::Command::new("/usr/sbin/sysctl")
        .arg("-n")
        .arg(name)
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn profile(home: &Path, chip: String, memory_bytes: u64, wanted: Option<&str>) -> Profile {
    let memory_gb = (memory_bytes as f64 / 1_073_741_824.0).round() as u64;
    let (best, alternative) = recommend(memory_gb, tier(&chip));
    let selected = wanted.and_then(model).unwrap_or(best);
    let env = home.join(ENV);
    let folder: PathBuf = home.join(MODELS_DIR).join(selected.repo);
    let python = python_ready(&env).then(|| env.join("bin/python").to_string_lossy().into_owned());
    let model_folder = model_ready(&folder).then(|| folder.to_string_lossy().into_owned());
    let steps = vec![
        Step {
            title: "Create a Python environment".into(),
            command: format!("python3 -m venv ~/{ENV}"),
            done: env.join("bin/python").is_file(),
        },
        Step {
            title: "Install MLX".into(),
            command: format!("~/{ENV}/bin/pip install \"mlx-lm>=0.30,<0.33\""),
            done: python.is_some(),
        },
        Step {
            title: format!(
                "Download {} ({:.1} GB)",
                selected.label, selected.download_gb
            ),
            command: format!(
                "~/{ENV}/bin/hf download mlx-community/{repo} --local-dir ~/{MODELS_DIR}/{repo}",
                repo = selected.repo
            ),
            done: model_folder.is_some(),
        },
    ];
    Profile {
        chip,
        memory_gb,
        apple_silicon: std::env::consts::ARCH == "aarch64",
        recommended: best.into(),
        alternative: alternative.map(Choice::from),
        selected: selected.into(),
        steps,
        python,
        model_folder,
    }
}

/// This Mac's chip and memory, the model that suits it (or `model`, when the user picked
/// the alternative), and how far setup has got.
#[tauri::command]
pub fn mac_profile(model: Option<String>) -> Result<Profile, String> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Can’t find your home folder.".to_string())?;
    let chip = sysctl("machdep.cpu.brand_string");
    let memory = sysctl("hw.memsize").parse().unwrap_or(0);
    Ok(profile(
        &home,
        if chip.is_empty() { "Mac".into() } else { chip },
        memory,
        model.as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_follow_memory_and_chip() {
        let pick = |gb, chip: &str| {
            let (best, other) = recommend(gb, tier(chip));
            (best.repo, other.map(|model| model.repo))
        };
        assert_eq!(
            pick(8, "Apple M2"),
            ("Qwen3.5-4B-MLX-4bit", Some("gemma-4-e2b-it-4bit"))
        );
        assert_eq!(
            pick(16, "Apple M1"),
            ("gemma-4-e4b-it-4bit", Some("Qwen3.5-9B-4bit")),
            "base chips get the efficient Gemma"
        );
        assert_eq!(
            pick(24, "Apple M4 Pro"),
            ("Qwen3.5-9B-4bit", Some("gemma-4-e4b-it-4bit"))
        );
        assert_eq!(
            pick(36, "Apple M3 Max"),
            ("Qwen3.5-35B-A3B-4bit", Some("gemma-4-31b-it-4bit"))
        );
        assert_eq!(
            pick(48, "Apple M4 Pro"),
            ("Qwen3.5-35B-A3B-4bit", Some("Qwen3.5-9B-4bit")),
            "a dense 31B is slow on Pro bandwidth"
        );
        assert_eq!(tier("Apple M2 Ultra"), Tier::Ultra);
        for (gb, chip) in [
            (8, "Apple M1"),
            (16, "Apple M2"),
            (24, "Apple M3 Pro"),
            (36, "Apple M3 Max"),
            (64, "Apple M2 Ultra"),
        ] {
            let (best, other) = recommend(gb, tier(chip));
            for choice in std::iter::once(best).chain(other) {
                assert!(
                    choice.download_gb <= gb as f64 * 0.6,
                    "{} on {gb} GB leaves room for macOS and apps",
                    choice.repo
                );
            }
        }
    }

    #[test]
    fn setup_is_noticed_only_when_complete() {
        let home = tempfile::tempdir().unwrap();
        let fresh = profile(home.path(), "Apple M2".into(), 8 * 1_073_741_824, None);
        assert_eq!(fresh.memory_gb, 8);
        assert_eq!(fresh.selected.repo, "Qwen3.5-4B-MLX-4bit");
        assert!(
            fresh.steps.iter().all(|step| !step.done)
                && fresh.python.is_none()
                && fresh.model_folder.is_none()
        );
        assert!(fresh.steps[2]
            .command
            .ends_with("--local-dir ~/buntini-models/Qwen3.5-4B-MLX-4bit"));

        let env = home.path().join("buntini-mlx");
        std::fs::create_dir_all(env.join("bin")).unwrap();
        std::fs::write(env.join("bin/python"), "").unwrap();
        std::fs::create_dir_all(env.join("lib/python3.13/site-packages/mlx_lm")).unwrap();
        let folder = home.path().join("buntini-models/Qwen3.5-4B-MLX-4bit");
        std::fs::create_dir_all(folder.join(".cache/huggingface/download")).unwrap();
        std::fs::write(folder.join("config.json"), "{}").unwrap();
        std::fs::write(
            folder.join("model.safetensors.index.json"),
            r#"{"weight_map":{"a":"model-1.safetensors","b":"model-2.safetensors"}}"#,
        )
        .unwrap();
        std::fs::write(folder.join("model-1.safetensors"), "").unwrap();
        assert!(!model_ready(&folder), "a shard is still missing");
        std::fs::write(folder.join("model-2.safetensors"), "").unwrap();
        std::fs::write(folder.join(".cache/huggingface/download/x.incomplete"), "").unwrap();
        assert!(!model_ready(&folder), "still downloading");
        std::fs::remove_file(folder.join(".cache/huggingface/download/x.incomplete")).unwrap();

        let ready = profile(home.path(), "Apple M2".into(), 8 * 1_073_741_824, None);
        assert!(ready.steps.iter().all(|step| step.done));
        assert_eq!(
            ready.model_folder.as_deref(),
            Some(folder.to_str().unwrap())
        );
        assert!(ready.python.unwrap().ends_with("buntini-mlx/bin/python"));
        let other = profile(
            home.path(),
            "Apple M2".into(),
            8 * 1_073_741_824,
            Some("gemma-4-e2b-it-4bit"),
        );
        assert_eq!(other.selected.repo, "gemma-4-e2b-it-4bit");
        assert!(other.model_folder.is_none());
        assert!(other.steps[2]
            .command
            .contains("mlx-community/gemma-4-e2b-it-4bit"));
        assert_eq!(
            profile(
                home.path(),
                "Apple M2".into(),
                8 * 1_073_741_824,
                Some("not-a-model")
            )
            .selected
            .repo,
            "Qwen3.5-4B-MLX-4bit"
        );
    }
}
