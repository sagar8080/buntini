// Cards the island shows for itself (stream A). Mirror: src-tauri/src/surface/cards/island.rs.
import Foundation

/// Files dropped on the island.
public struct DropCard: Codable, Equatable, Sendable {
    public let files: [String]
}

/// Hello on launch.
public struct Greeting: Codable, Equatable, Sendable {
    public let name: String?
}
