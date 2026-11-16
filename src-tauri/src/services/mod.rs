//! Developer services: GitHub, GitLab, Gitea, Vercel, Stripe, n8n, Notion, Cal.com and Resend (stream D).
use serde::{Deserialize, Serialize};

/// Saved under `settings.services`.
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
pub async fn services_call(op: Op) -> Result<serde_json::Value, String> {
    match op {
        Op::Ping => Ok(serde_json::Value::Bool(true)),
    }
}

/// Polls the services that are due. Called from the scheduler's tick, which already pauses
/// while a task runs and catches up after the Mac wakes.
pub async fn run_due(_app: &tauri::AppHandle) {}
