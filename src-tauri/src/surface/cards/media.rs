//! Cards for music and video players (stream F).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    /// `music`, `spotify` or `tv`.
    pub player: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// A file path to the cover, when the player gives one.
    pub artwork: Option<String>,
    pub playing: bool,
    /// Seconds.
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub shuffle: Option<bool>,
    /// `off`, `one` or `all`.
    pub repeat_mode: Option<String>,
    /// 0…1.
    pub volume: Option<f64>,
    /// When `position` was read, in Unix seconds: the island runs the clock on from there.
    #[serde(default)]
    pub position_at: Option<f64>,
}
