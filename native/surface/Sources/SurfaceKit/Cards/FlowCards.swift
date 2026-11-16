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

/// How words come out in an app.
public enum FlowStyle: String, Codable, Sendable, CaseIterable {
    case asSaid, natural, polished
}

public enum FlowMode: String, Codable, Sendable, CaseIterable {
    case dictate, prompt, translate
}

/// The pill at the bottom of the screen.
public struct FlowPill: Codable, Equatable, Sendable {
    public let state: FlowState
    public let text: String?
    /// The app the words go to.
    public let app: String?
    /// Hands-free.
    public let locked: Bool
    /// How words come out in that app, for the right-click menu.
    public let style: FlowStyle?
    public let mode: FlowMode?
    /// Since when a call has been recording, in seconds since 1970.
    public let recording: Double?

    public init(state: FlowState, text: String? = nil, app: String? = nil, locked: Bool = false, style: FlowStyle? = nil, mode: FlowMode? = nil, recording: Double? = nil) {
        self.state = state
        self.text = text
        self.app = app
        self.locked = locked
        self.style = style
        self.mode = mode
        self.recording = recording
    }
}
