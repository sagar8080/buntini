//! What Rust sends to the native surfaces and what comes back. Mirrors
//! native/surface/Sources/SurfaceKit/Scene.swift and Card.swift; both sides check
//! native/surface/Fixtures/contract.json.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cards::{agents, flow, island, mail, media, services, utilities};

/// Bumped whenever a change on one side would break the other.
pub const CONTRACT: u32 = 1;

/// Which part of Buntini a pill, card or action belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Agents,
    Services,
    Mail,
    Media,
    Utilities,
    Flow,
    Island,
}

/// Where the character lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Home {
    Island,
    #[default]
    Desktop,
}

/// The notch or menu bar (`Auto`), or a corner the character was dragged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    #[default]
    Auto,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PillState {
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pill {
    pub id: String,
    pub source: Source,
    pub label: String,
    /// `#rrggbb`.
    pub tint: String,
    pub state: PillState,
    pub badge: Option<String>,
    /// 0…1 when the pill shows progress.
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mood {
    #[default]
    Calm,
    Happy,
    Curious,
    Sleepy,
    Alert,
    Grumpy,
    Dancing,
}

/// The cat's look, the same choices as Settings (src/petOptions.ts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cat {
    /// `cat` or `orb`.
    pub character: String,
    pub coat: String,
    pub eyes: String,
    pub accessory: String,
    /// `#rrggbb`.
    pub accent: String,
    pub mood: Mood,
    /// A seasonal outfit, when one applies today.
    pub outfit: Option<String>,
}

impl Default for Cat {
    fn default() -> Self {
        Self {
            character: "cat".into(),
            coat: "ginger".into(),
            eyes: "amber".into(),
            accessory: "none".into(),
            accent: "#F2B33D".into(),
            mood: Mood::Calm,
            outfit: None,
        }
    }
}

/// A sound to play once. A new sequence number plays it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    pub name: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub contract: u32,
    pub home: Home,
    pub placement: Placement,
    pub pills: Vec<Pill>,
    /// Ids of waiting pills that didn't fit, shown as "+N".
    pub overflow: Vec<String>,
    /// The card in front, if any.
    pub card: Option<Card>,
    /// Something waits on the person: the island stays open.
    pub held: bool,
    /// The card in front asks for attention, so the island opens by itself. Otherwise it
    /// shows once the island is opened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupt: bool,
    pub cat: Cat,
    pub flow: Option<flow::FlowPill>,
    pub overlay: Option<utilities::Overlay>,
    pub sound: Option<Sound>,
}

/// The card in front of the island.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub source: Source,
    pub body: CardBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CardBody {
    // Coding agents
    Approval(agents::Approval),
    Question(agents::QuestionCard),
    Ticker(agents::Ticker),
    Diff(agents::DiffCard),
    Finished(agents::Finished),
    Recap(agents::Recap),
    Usage(agents::Usage),
    // Developer services
    Service(services::ServiceCard),
    // Mail
    Mail(mail::MailCard),
    // Media
    NowPlaying(media::NowPlaying),
    // Utilities
    Timer(utilities::TimerCard),
    Capture(utilities::Capture),
    Vitals(utilities::Vitals),
    Update(utilities::Update),
    WhatsNew(utilities::WhatsNew),
    // The island itself
    Drop(island::DropCard),
    Greeting(island::Greeting),
    // Voice
    Note(flow::Note),
}

/// What the person did on a surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub source: Source,
    /// The card it came from.
    #[serde(default)]
    pub card: Option<String>,
    pub name: String,
    #[serde(default)]
    pub value: Option<Value>,
}
