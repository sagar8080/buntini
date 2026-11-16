//! Dictation and notes (stream C).
use serde::{Deserialize, Serialize};

/// A recorded call or meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    /// `recording`, `processing`, `ready` or `failed`.
    pub state: String,
    pub seconds: Option<f64>,
    pub lines: Vec<NoteLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLine {
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowState {
    Idle,
    Listening,
    Working,
    Done,
    Copied,
    Error,
}

/// The pill at the bottom of the screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowPill {
    pub state: FlowState,
    pub text: Option<String>,
    /// The app the words go to.
    pub app: Option<String>,
    /// Hands-free.
    pub locked: bool,
}
