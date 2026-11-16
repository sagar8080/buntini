// Cards for music and video players (stream F). Mirror: src-tauri/src/surface/cards/media.rs.
import Foundation

public struct NowPlaying: Codable, Equatable, Sendable {
    /// `music`, `spotify` or `tv`.
    public let player: String
    public let title: String
    public let artist: String?
    public let album: String?
    /// A file path to the cover, when the player gives one.
    public let artwork: String?
    public let playing: Bool
    /// Seconds.
    public let position: Double?
    public let duration: Double?
    public let shuffle: Bool?
    /// `off`, `one` or `all`.
    public let repeatMode: String?
    /// 0…1.
    public let volume: Double?
}
