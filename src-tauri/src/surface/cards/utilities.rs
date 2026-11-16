//! Cards for timers, captures, vitals and updates (stream F, Phase 3).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerCard {
    pub label: String,
    /// `focus` or `rest` for focus cycles.
    pub phase: Option<String>,
    /// Unix seconds; absent while paused.
    pub ends_at: Option<i64>,
    /// Seconds left.
    pub remaining: f64,
    pub total: f64,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub path: String,
    pub width: i64,
    pub height: i64,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    pub lines: Vec<String>,
}

/// A picture shown over the desktop, such as a screenshot's thumbnail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlay {
    /// `thumbnail`.
    pub kind: String,
    pub path: String,
    pub width: i64,
    pub height: i64,
}
