//! Cards for developer services (stream D).
use serde::{Deserialize, Serialize};

use crate::surface::PillState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCard {
    /// `github`, `gitlab`, `gitea`, `vercel`, `stripe`, `n8n`, `notion`, `calcom` or `resend`.
    pub service: String,
    pub title: String,
    pub rows: Vec<ServiceRow>,
    /// The activity graph: daily counts, oldest first, starting on a Sunday and ending today.
    /// GitHub, GitLab and Gitea only.
    #[serde(default)]
    pub graph: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRow {
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    pub state: Option<PillState>,
    /// Opened through the `open` action, with this URL as its value.
    pub url: Option<String>,
}
