//! Cards for timers, captures, vitals and updates (stream F, Phase 3).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerCard {
    pub label: String,
    /// `focus`, `rest`, `longRest`, or `done` when a set of focus cycles ends.
    pub phase: Option<String>,
    /// Unix seconds; absent while paused.
    pub ends_at: Option<i64>,
    /// Seconds left.
    pub remaining: f64,
    pub total: f64,
    pub paused: bool,
    /// The focus round, 1…`rounds`.
    #[serde(default)]
    pub round: Option<u32>,
    #[serde(default)]
    pub rounds: Option<u32>,
}

/// A capture waiting on the island. An empty path: Screen Recording isn't allowed yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub path: String,
    pub width: i64,
    pub height: i64,
}

/// The Mac kept awake as asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Awake {
    pub label: String,
    /// When it ends, in Unix seconds; left out until stopped.
    pub until: Option<i64>,
}

/// One line about how the Mac is doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vitals {
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    pub notes: Vec<String>,
    /// `ask`, `current`, `available`, `downloading`, `checking`, `ready` or `failed` (the
    /// note says why). None reads as `available`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// While downloading, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    pub lines: Vec<String>,
}

/// Shown over the desktop: a screenshot's thumbnail in the corner, or the dimmer for choosing
/// an area (which has no picture).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlay {
    /// `thumbnail` or `select`.
    pub kind: String,
    pub path: String,
    pub width: i64,
    pub height: i64,
}
