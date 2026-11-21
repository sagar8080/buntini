//! Cards the island shows for itself (stream A).
use serde::{Deserialize, Serialize};

/// Files dropped on the island, and where they can go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropCard {
    pub files: Vec<String>,
    /// Agent sessions the paths can be pasted into.
    #[serde(default)]
    pub targets: Vec<DropTarget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropTarget {
    /// The session's pill.
    pub pill: String,
    pub label: String,
}

/// A short answer to what was typed in the island, shown there rather than opening the chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub question: String,
    pub answer: String,
}

/// Hello on launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Greeting {
    pub name: Option<String>,
}
