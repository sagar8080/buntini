//! Cards for music and video players (stream F).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    /// `music`, `spotify`, `tv`, `browser` or `other`.
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
    /// Whether going back and skipping work right now. `None` for a player that always takes
    /// them; a browser says, as a YouTube video only skips in a playlist.
    #[serde(default)]
    pub can_previous: Option<bool>,
    #[serde(default)]
    pub can_next: Option<bool>,
    /// The bundle id of the app that plays, for one of the other players, whose card shows
    /// the app's own icon. Left out for the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Where the Mac's sound can go, the one in use marked. Left out until read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Output>,
}

/// One of the Mac's sound outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    /// Core Audio's id for it, which stays the same across restarts.
    pub uid: String,
    pub name: String,
    /// `builtIn`, `airpods`, `headphones`, `display`, `airplay`, `usb` or `other`.
    pub kind: String,
    pub current: bool,
}
