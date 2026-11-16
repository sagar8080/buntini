//! Cards the island shows for itself (stream A).
use serde::{Deserialize, Serialize};

/// Files dropped on the island.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropCard {
    pub files: Vec<String>,
}

/// Hello on launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Greeting {
    pub name: Option<String>,
}
