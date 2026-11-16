// Cards for developer services (stream D). Mirror: src-tauri/src/surface/cards/services.rs.
import Foundation

public struct ServiceCard: Codable, Equatable, Sendable {
    /// `github`, `gitlab`, `gitea`, `vercel`, `stripe`, `n8n`, `notion`, `calcom` or `resend`.
    public let service: String
    public let title: String
    public let rows: [ServiceRow]
}

public struct ServiceRow: Codable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let detail: String?
    public let state: PillState?
    public let url: String?
}
