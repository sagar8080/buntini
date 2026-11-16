// Cards for Mail (stream E). Mirror: src-tauri/src/surface/cards/mail.rs.
import Foundation

public struct MailCard: Codable, Equatable, Sendable {
    /// `all`, `unread`, `important` or `today`.
    public let filter: String
    public let messages: [MailRow]
}

public struct MailRow: Codable, Equatable, Sendable {
    public let id: String
    public let from: String
    public let subject: String
    public let preview: String?
    public let unread: Bool
    public let flagged: Bool
    /// Unix seconds.
    public let at: Int64
}
