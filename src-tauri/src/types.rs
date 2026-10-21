use serde::{Deserialize, Serialize};

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {
    pub model_backend: String,
    pub python_path: String,
    pub model_path: String,
    pub whisper_model: String,
    pub search_provider: String,
    pub monthly_search_limit: u32,
    pub daily_search_limit: u32,
    /// Stay inside the search provider's free monthly allowance.
    pub free_tier_only: bool,
    pub speak_responses: bool,
    pub theme: String,
    pub reduced_motion: bool,
    pub assistant_name: String,
    pub personality: String,
    pub avatar: String,
    pub accent: String,
    pub companion_scale: f64,
    pub cat_coat: String,
    pub cat_eyes: String,
    pub cat_accessory: String,
    pub pet_motion: String,
    pub pet_roam: bool,
    pub pet_awareness: bool,
    pub pet_nudges: bool,
    pub voice_id: String,
    pub voice_rate: u32,
    pub notifications_enabled: bool,
    pub monthly_budget_usd: f64,
    pub cost_per_search_usd: f64,
    pub vad_model_path: String,
    pub ebay_marketplace: String,
    /// Local data categories an external model backend may receive. On-device models see everything.
    pub external_shares: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let voice_python = project.join(".venv-voice/bin/python");
        let local_whisper = project.join("models/whisper-tiny.en");
        let local_vad = project.join("models/silero_vad.onnx");
        let candidate = std::env::var("HOME")
            .map(|h| format!("{h}/projects/pulse-venv/bin/python"))
            .unwrap_or_default();
        Self {
            model_backend: "apple".into(),
            // Never guess `python3`: on a fresh Mac that launches the Command Line Tools installer.
            python_path: std::env::var("BUNTINI_PYTHON").unwrap_or_else(|_| {
                if voice_python.is_file() {
                    voice_python.to_string_lossy().into_owned()
                } else if std::path::Path::new(&candidate).is_file() {
                    candidate
                } else {
                    String::new()
                }
            }),
            model_path: std::env::var("BUNTINI_MODEL_PATH").unwrap_or_else(|_| {
                let local = std::env::var("HOME")
                    .map(|home| format!("{home}/projects/pulse-venv/models/Qwen3.5-9B-4bit"))
                    .unwrap_or_default();
                if std::path::Path::new(&local).join("config.json").is_file() {
                    local
                } else {
                    String::new()
                }
            }),
            whisper_model: std::env::var("BUNTINI_WHISPER_MODEL_PATH").unwrap_or_else(|_| {
                if local_whisper.join("config.json").is_file() {
                    local_whisper.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            }),
            search_provider: "brave".into(),
            monthly_search_limit: 1000,
            daily_search_limit: 30,
            free_tier_only: true,
            speak_responses: false,
            theme: "system".into(),
            reduced_motion: false,
            assistant_name: "Buntini".into(),
            personality: "Warm, concise, practical, and honest.".into(),
            avatar: "cat".into(),
            accent: "ember".into(),
            companion_scale: 1.0,
            cat_coat: "ginger".into(),
            cat_eyes: "green".into(),
            cat_accessory: "bell".into(),
            pet_motion: "playful".into(),
            pet_roam: true,
            pet_awareness: true,
            pet_nudges: true,
            voice_id: String::new(),
            voice_rate: 180,
            notifications_enabled: false,
            monthly_budget_usd: 5.0,
            cost_per_search_usd: 0.005,
            ebay_marketplace: "EBAY_US".into(),
            external_shares: Vec::new(),
            vad_model_path: std::env::var("BUNTINI_VAD_MODEL_PATH").unwrap_or_else(|_| {
                if local_vad.is_file() {
                    local_vad.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            }),
        }
    }
}

const CAT_COATS: [&str; 7] = ["ginger", "gray", "black", "white", "tuxedo", "calico", "siamese"];
const CAT_EYES: [&str; 5] = ["amber", "green", "blue", "copper", "odd"];
const CAT_ACCESSORIES: [&str; 6] = ["none", "bell", "bow", "bandana", "hat", "flowers"];
const PET_MOTIONS: [&str; 3] = ["calm", "playful", "sleepy"];

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.assistant_name.trim().is_empty()
            || self.assistant_name.chars().count() > 40
            || self.assistant_name.chars().any(char::is_control)
        {
            return Err("Assistant names must contain 1–40 visible characters.".into());
        }
        if self.personality.chars().count() > 1200 {
            return Err("Personality must be no longer than 1,200 characters.".into());
        }
        if !["kyuubi", "fox", "cat", "orb"].contains(&self.avatar.as_str())
            || !["ember", "ocean", "violet", "sage"].contains(&self.accent.as_str())
        {
            return Err("Choose a supported avatar and accent.".into());
        }
        crate::privacy::validate_shares(&self.external_shares)?;
        if !crate::integrations::ebay::MARKETPLACES
            .iter()
            .any(|(id, _)| *id == self.ebay_marketplace)
        {
            return Err("Choose a supported eBay marketplace.".into());
        }
        if !CAT_COATS.contains(&self.cat_coat.as_str())
            || !CAT_EYES.contains(&self.cat_eyes.as_str())
            || !CAT_ACCESSORIES.contains(&self.cat_accessory.as_str())
            || !PET_MOTIONS.contains(&self.pet_motion.as_str())
        {
            return Err("Choose a supported coat, eye color, accessory, and motion.".into());
        }
        if ![0.8, 1.0, 1.2]
            .iter()
            .any(|v| (self.companion_scale - v).abs() < 0.000001)
        {
            return Err("Companion size must be 0.8, 1.0, or 1.2.".into());
        }
        if !(100..=300).contains(&self.voice_rate)
            || self.voice_id.len() > 128
            || self.voice_id.chars().any(char::is_control)
        {
            return Err("Choose a valid macOS voice and rate between 100 and 300.".into());
        }
        if !self.monthly_budget_usd.is_finite()
            || !(0.0..=100000.0).contains(&self.monthly_budget_usd)
            || !self.cost_per_search_usd.is_finite()
            || !(0.0..=10.0).contains(&self.cost_per_search_usd)
        {
            return Err("Set a finite nonnegative monthly budget and per-search cost estimate (up to $10 per search).".into());
        }
        if !self.vad_model_path.is_empty()
            && (!std::path::Path::new(&self.vad_model_path).is_absolute()
                || !std::path::Path::new(&self.vad_model_path).is_file())
        {
            return Err("Silero VAD must point to an existing local ONNX file.".into());
        }
        if !["brave", "tavily"].contains(&self.search_provider.as_str()) {
            return Err("Choose Brave or Tavily.".into());
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            return Err("Unknown theme.".into());
        }
        if self.monthly_search_limit > 10_000 {
            return Err("The monthly search limit cannot exceed 10,000.".into());
        }
        if !(1..=2_000).contains(&self.daily_search_limit) {
            return Err("The daily lookup limit must be between 1 and 2,000.".into());
        }
        if !crate::model::BACKENDS.contains(&self.model_backend.as_str()) {
            return Err("Choose Apple Intelligence, a local MLX model, or a cloud model.".into());
        }
        if self.python_path.len() > 4096 || self.python_path.chars().any(char::is_control) {
            return Err("The Python path is not valid.".into());
        }
        if self.model_path.len() > 4096 || self.whisper_model.len() > 4096 {
            return Err("Model path is too long.".into());
        }
        for path in [&self.model_path, &self.whisper_model] {
            if !path.is_empty() && !std::path::Path::new(path).is_dir() {
                return Err("Model paths must point to existing local model directories. Buntini does not download models.".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub retrieved_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub name: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySnapshot {
    pub file_vault: String,
    pub gatekeeper: String,
    pub system_integrity: String,
    pub os_update_status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub uptime_seconds: u64,
    pub os: String,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub captured_at: String,
    #[serde(default)]
    pub security: SecuritySnapshot,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub prompt: String,
    pub answer: String,
    pub state: String,
    pub created_at: String,
    pub sources: Vec<Source>,
    pub files: Vec<FileHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemSnapshot>,
    #[serde(default)]
    pub research: Vec<crate::research::ResearchContent>,
    #[serde(default)]
    pub prices: Vec<crate::pricing::PriceOffer>,
    #[serde(default)]
    pub cards: Vec<ResultCard>,
}
/// One structured result from an approved integration (a forecast, rate, place, listing or flight).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResultCard {
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub fields: Vec<CardField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub attribution: String,
    pub retrieved_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardField {
    pub label: String,
    pub value: String,
}
impl CardField {
    pub fn new(label: &str, value: impl Into<String>) -> Self {
        Self { label: label.into(), value: value.into() }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub detail: String,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub id: String,
    pub query: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub created_at: String,
    pub next_run_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_result_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricePoint {
    pub price: String,
    pub currency: String,
    pub observed_at: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceWatch {
    pub id: String,
    pub url: String,
    pub label: String,
    pub currency: String,
    pub threshold: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub created_at: String,
    pub next_run_at: String,
    pub last_checked_at: Option<String>,
    pub last_price: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_availability: Option<String>,
    pub history: Vec<PricePoint>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub model_ready: bool,
    /// "available", a reason it isn't, or "checking".
    pub apple_intelligence: String,
    pub worker_status: String,
    pub brave_configured: bool,
    pub tavily_configured: bool,
    pub ebay_configured: bool,
    pub duffel_configured: bool,
    pub searches_used: u32,
    pub searches_today: u32,
    pub search_limit_month: u32,
    pub search_limit_day: u32,
    pub estimated_spend_usd: f64,
    pub remaining_budget_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub prompt: String,
    pub provider: String,
    pub expires_at: String,
    pub approved: bool,
    /// The validated tool call this approval authorizes, exactly as shown to the user.
    pub tool: String,
    pub arguments: serde_json::Value,
    pub service: String,
    pub summary: String,
    pub sends: Vec<CardField>,
    pub host: String,
    pub uses_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceChoice {
    pub id: String,
    pub name: String,
    pub language: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsUpdateCheck {
    pub state: String,
    pub checked_at: String,
    pub summary: String,
    pub updates: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub files: usize,
    pub truncated: bool,
    pub indexed_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub status: IndexStatus,
    pub roots: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub settings: Settings,
    pub tasks: Vec<TaskRecord>,
    pub roots: Vec<String>,
    pub diagnostics: Diagnostics,
    pub audit: Vec<AuditEntry>,
    pub saved_searches: Vec<SavedSearch>,
    pub price_watches: Vec<PriceWatch>,
    pub os_update_check: Option<OsUpdateCheck>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub r#type: String,
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskRecord>,
}
impl TaskEvent {
    pub fn new(kind: &str, id: &str, text: Option<String>) -> Self {
        Self {
            r#type: kind.into(),
            task_id: id.into(),
            state: None,
            text,
            tool: None,
            task: None,
        }
    }
}

/// Requests used by one online service today and this month (local dates).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub today: u32,
    pub month: u32,
    pub spend: f64,
}
/// A reserved request; `warning` is set the first time this month usage crosses 80%.
#[derive(Debug, Clone, PartialEq)]
pub struct Reservation {
    pub warning: Option<String>,
}
