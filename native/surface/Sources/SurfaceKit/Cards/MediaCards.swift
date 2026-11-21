// Cards for music and video players (stream F). Mirror: src-tauri/src/surface/cards/media.rs.
import Foundation

public struct NowPlaying: Codable, Equatable, Sendable {
    /// `music`, `spotify`, `tv`, `browser` or `other`.
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
    /// When `position` was read, in Unix seconds: the island runs the clock on from there.
    public let positionAt: Double?
    /// Whether going back and skipping work right now. `nil` for a player that always takes
    /// them; a browser says, as a YouTube video only skips in a playlist.
    public let canPrevious: Bool?
    public let canNext: Bool?
    /// The bundle id of the app that plays, for one of the other players (Podcasts, VLC):
    /// its card shows the app's own icon.
    public let app: String?

    /// The player's own colour, until a cover gives one. Mirror: `Player::tint` in
    /// src-tauri/src/media/player.rs.
    public var playerTint: String {
        switch player {
        case "spotify": return "#1ED760"
        case "tv": return "#8EA2FF"
        case "browser": return "#5E9EFF"
        case "other": return "#BF5AF2"
        default: return "#FA2D48"
        }
    }
}
