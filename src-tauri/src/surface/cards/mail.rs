//! Cards for Mail (stream E).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailCard {
    /// `all`, `unread`, `important` or `today`.
    pub filter: String,
    pub messages: Vec<MailRow>,
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
