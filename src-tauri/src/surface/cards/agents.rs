//! Cards for coding agents (stream B).
use serde::{Deserialize, Serialize};

/// A tool call waiting for Allow or Deny.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub agent: String,
    pub project: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    /// The detail is code or a command, shown monospaced.
    pub code: bool,
    /// "Always" can be offered.
    pub always: bool,
    /// The agent's plan, waiting to be started.
    #[serde(default)]
    pub plan: bool,
}

/// One to four questions an agent asks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionCard {
    pub agent: String,
    pub project: Option<String>,
    pub questions: Vec<AgentQuestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuestion {
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<AgentChoice>,
    pub multi: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChoice {
    pub label: String,
    pub description: Option<String>,
}

/// What a session is doing, step by step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticker {
    pub agent: String,
    pub project: Option<String>,
    pub steps: Vec<TickerStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickerStep {
    pub label: String,
    pub detail: Option<String>,
    pub added: Option<i64>,
    pub removed: Option<i64>,
    pub failed: bool,
}

/// One file's changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffCard {
    pub file: String,
    pub added: i64,
    pub removed: i64,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    /// `add`, `remove`, `same` or `gap`.
    pub kind: String,
    pub text: String,
}

/// A turn finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub agent: String,
    pub project: Option<String>,
    pub message: Option<String>,
}

/// The week with the agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recap {
    pub week: String,
    pub stats: Vec<RecapStat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapStat {
    pub label: String,
    pub value: String,
}

/// Plan limits of an agent's subscription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub agent: String,
    pub windows: Vec<UsageWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub label: String,
    /// 0…100.
    pub percent: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}
