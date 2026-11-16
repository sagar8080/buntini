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

/// Hello on launch.
public struct Greeting: Codable, Equatable, Sendable {
    public let name: String?
}
