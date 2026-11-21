// Cards for timers, captures, vitals and updates (stream F, Phase 3).
// Mirror: src-tauri/src/surface/cards/utilities.rs.
import Foundation

public struct TimerCard: Codable, Equatable, Sendable {
    public let label: String
    /// `focus`, `rest`, `longRest`, or `done` when a set of focus cycles ends.
    public let phase: String?
    /// Unix seconds; absent while paused.
    public let endsAt: Int64?
    /// Seconds left.
    public let remaining: Double
    public let total: Double
    public let paused: Bool
    /// The focus round, 1…`rounds`.
    public let round: Int?
    public let rounds: Int?
}

/// A capture waiting on the island. An empty path: Screen Recording isn't allowed yet.
public struct Capture: Codable, Equatable, Sendable {
    public let path: String
    public let width: Int
    public let height: Int
}

/// One line about how the Mac is doing.
public struct Vitals: Codable, Equatable, Sendable {
    public let line: String
}

/// A screen recording: running since a time, or finished as a film. Mirror: `Recording` in
/// src-tauri/src/surface/cards/utilities.rs.
public struct Recording: Codable, Equatable, Sendable {
    public let since: Int64?
    public let path: String?
    public let name: String?
    public let seconds: Double?
}

/// The Mac kept awake as asked. Mirror: `Awake` in src-tauri/src/surface/cards/utilities.rs.
public struct Awake: Codable, Equatable, Sendable {
    public let label: String
    /// When it ends, in Unix seconds; `nil` until stopped.
    public let until: Int64?
}

public struct Update: Codable, Equatable, Sendable {
    public let version: String
    public let notes: [String]
    /// `ask`, `current`, `available`, `downloading`, `checking`, `ready` or `failed`.
    public let stage: String?
    /// While downloading, 0 to 1.
    public let progress: Double?
}

public struct WhatsNew: Codable, Equatable, Sendable {
    public let version: String
    public let lines: [String]
}

/// Shown over the desktop: a screenshot's thumbnail in the corner, or the dimmer for choosing
/// an area (which has no picture).
public struct Overlay: Codable, Equatable, Sendable {
    /// `thumbnail` or `select`.
    public let kind: String
    public let path: String
    public let width: Int
    public let height: Int
}
