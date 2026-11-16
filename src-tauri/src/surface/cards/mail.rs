//! Cards for Mail (stream E).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailCard {
    /// `all`, `unread`, `important` or `today`.
    pub filter: String,
    pub messages: Vec<MailRow>,
    pub query: String,
    /// Unread across every inbox.
    pub unread: u32,
    /// Emails sorting would change now.
    pub sortable: u32,
    /// More rows match than are shown.
    pub more: bool,
    /// The email being read.
    pub open: Option<MailOpen>,
    /// A change waiting for Allow.
    pub pending: Option<MailPending>,
    /// A plan waiting for Run.
    pub plan: Option<MailPlan>,
    /// The latest changes, newest first.
    pub log: Vec<MailLogRow>,
    /// What went wrong last, if anything.
    pub status: Option<String>,
    /// Mail is closed.
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailRow {
    pub id: String,
    pub from: String,
    pub subject: String,
    pub preview: Option<String>,
    pub unread: bool,
    pub flagged: bool,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailOpen {
    pub id: String,
    pub from: String,
    pub subject: String,
    /// Unix seconds.
    pub at: i64,
    pub body: String,
    pub attachments: Vec<String>,
    pub flagged: bool,
    pub unread: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailPending {
    pub id: String,
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailPlan {
    pub id: String,
    pub summary: String,
    pub lines: Vec<String>,
    /// Steps left out, and why.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailLogRow {
    pub id: String,
    pub label: String,
    pub undoable: bool,
}
