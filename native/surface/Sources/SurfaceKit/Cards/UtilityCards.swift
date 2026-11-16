// Cards for timers, captures, vitals and updates (stream F, Phase 3).
// Mirror: src-tauri/src/surface/cards/utilities.rs.
import Foundation

public struct TimerCard: Codable, Equatable, Sendable {
    public let label: String
    /// `focus` or `rest` for focus cycles.
    public let phase: String?
    /// Unix seconds; absent while paused.
    public let endsAt: Int64?
    /// Seconds left.
    public let remaining: Double
    public let total: Double
    public let paused: Bool
}

public struct Capture: Codable, Equatable, Sendable {
    public let path: String
    public let width: Int
    public let height: Int
}

/// One line about how the Mac is doing.
public struct Vitals: Codable, Equatable, Sendable {
    public let line: String
}

public struct Update: Codable, Equatable, Sendable {
    public let version: String
    public let notes: [String]
}

public struct WhatsNew: Codable, Equatable, Sendable {
    public let version: String
    public let lines: [String]
}

/// A picture shown over the desktop, such as a screenshot's thumbnail.
public struct Overlay: Codable, Equatable, Sendable {
    /// `thumbnail`.
    public let kind: String
    public let path: String
    public let width: Int
    public let height: Int
}
