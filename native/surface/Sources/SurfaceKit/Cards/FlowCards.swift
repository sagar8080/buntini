// Dictation and notes (stream C). Mirror: src-tauri/src/surface/cards/flow.rs.
import Foundation

/// A recorded call or meeting.
public struct Note: Codable, Equatable, Sendable {
    public let id: String
    public let title: String
    /// `recording`, `processing`, `ready` or `failed`.
    public let state: String
    public let seconds: Double?
    public let lines: [NoteLine]
}

public struct NoteLine: Codable, Equatable, Sendable {
    public let speaker: String?
    public let text: String
}

public enum FlowState: String, Codable, Sendable {
    case idle, listening, working, done, copied, error
}

/// The pill at the bottom of the screen.
public struct FlowPill: Codable, Equatable, Sendable {
    public let state: FlowState
    public let text: String?
    /// The app the words go to.
    public let app: String?
    /// Hands-free.
    public let locked: Bool
}
