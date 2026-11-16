//! Mail in the island: filters, actions with Undo, sorting rules and mail plans (stream E).
use serde::{Deserialize, Serialize};

/// Saved under `settings.mail`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
}

#[tauri::command]
pub async fn mail_call(op: Op) -> Result<serde_json::Value, String> {
    match op {
        Op::Ping => Ok(serde_json::Value::Bool(true)),
    }
}
