// Cards the island shows for itself (stream A). Mirror: src-tauri/src/surface/cards/island.rs.
import Foundation

/// Files dropped on the island, and where they can go.
public struct DropCard: Codable, Equatable, Sendable {
    public let files: [String]
    /// Agent sessions the paths can be pasted into.
    public let targets: [DropTarget]
}

public struct DropTarget: Codable, Equatable, Sendable {
    /// The session's pill.
    public let pill: String
    public let label: String
}

/// A short answer to what was typed in the island, shown there rather than opening the chat.
public struct Answer: Codable, Equatable, Sendable {
    public let question: String
    public let answer: String
}

/// What's next: a meeting about to start (`item` is `meeting`) or reminders past due
/// (`reminders`).
public struct Upcoming: Codable, Equatable, Sendable {
    public let item: String
    public let title: String
    /// When the meeting starts, in Unix seconds.
    public let starts: Int64?
    /// When the reminder was due, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    public let due: String?
    /// Where, or the reminder's list.
    public let detail: String?
    /// The meeting's call link.
    public let link: String?
    /// The reminder Done marks done.
    public let id: String?
    /// More reminders past due besides this one.
    public let more: UInt32
}

/// Hello on launch.
public struct Greeting: Codable, Equatable, Sendable {
    public let name: String?
}
