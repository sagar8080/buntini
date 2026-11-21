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
    /// Where Send to… can open the answer: `whatsapp`, `x`, `threads`, `bluesky`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub send_to: Vec<String>,
}

/// The shortcuts in the person's "Buntini" folder in Shortcuts, as buttons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcuts {
    pub title: String,
    pub buttons: Vec<ShortcutButton>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutButton {
    /// The shortcut's id in Shortcuts.
    pub id: String,
    pub name: String,
}

/// Something another assistant asks of the person, with Deny and Allow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// The assistant's name.
    pub from: String,
    /// What it asks, in Buntini's words.
    pub what: String,
}

/// Unread messages in a chat app, as its Dock icon counts them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unread {
    pub name: String,
    /// "3 unread", or that there are new messages.
    pub line: String,
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

/// A call in another app: Zoom, Teams, FaceTime, a meeting in a browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    /// The app's name, as macOS shows it.
    pub name: String,
    /// When the call began, in Unix seconds; the island counts on from there.
    pub since: i64,
    /// Whether the microphone is muted. Left out when it can't be from here.
    pub muted: Option<bool>,
    /// Whether the call is being recorded for notes. Left out while "Record calls" is off.
    pub recording: Option<bool>,
}

/// Hello on launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Greeting {
    pub name: Option<String>,
}
