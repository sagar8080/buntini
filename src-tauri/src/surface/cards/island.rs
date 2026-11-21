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

/// What's next: a meeting about to start (`item` is `meeting`) or reminders past due
/// (`reminders`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upcoming {
    pub item: String,
    pub title: String,
    /// When the meeting starts, in Unix seconds; the island says how long until then.
    pub starts: Option<i64>,
    /// When the reminder was due, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`, as Reminders has it.
    pub due: Option<String>,
    /// Where, or the reminder's list.
    pub detail: Option<String>,
    /// The meeting's call link (https), to join from the island.
    pub link: Option<String>,
    /// The reminder Done marks done.
    pub id: Option<String>,
    /// More reminders past due besides this one.
    pub more: u32,
}

/// Hello on launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Greeting {
    pub name: Option<String>,
}
