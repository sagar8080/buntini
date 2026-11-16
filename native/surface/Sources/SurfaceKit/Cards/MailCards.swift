// Cards for Mail (stream E). Mirror: src-tauri/src/surface/cards/mail.rs.
import Foundation

public struct MailCard: Codable, Equatable, Sendable {
    /// `all`, `unread`, `important` or `today`.
    public let filter: String
    public let messages: [MailRow]
    public let query: String
    /// Unread across every inbox.
    public let unread: Int
    /// Emails sorting would change now.
    public let sortable: Int
    /// More rows match than are shown.
    public let more: Bool
    /// The email being read.
    public let open: MailOpen?
    /// A change waiting for Allow.
    public let pending: MailPending?
    /// A plan waiting for Run.
    public let plan: MailPlan?
    /// The latest changes, newest first.
    public let log: [MailLogRow]
    /// What went wrong last, if anything.
    public let status: String?
    /// Mail is closed.
    public let closed: Bool

    private enum CodingKeys: String, CodingKey {
        case filter, messages, query, unread, sortable, more, open, pending, plan, log, status, closed
    }

    public init(filter: String, messages: [MailRow], query: String = "", unread: Int = 0, sortable: Int = 0, more: Bool = false,
                open: MailOpen? = nil, pending: MailPending? = nil, plan: MailPlan? = nil, log: [MailLogRow] = [],
                status: String? = nil, closed: Bool = false) {
        self.filter = filter
        self.messages = messages
        self.query = query
        self.unread = unread
        self.sortable = sortable
        self.more = more
        self.open = open
        self.pending = pending
        self.plan = plan
        self.log = log
        self.status = status
        self.closed = closed
    }

    /// Fields added after the first contract default when an older Buntini leaves them out.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        filter = try c.decode(String.self, forKey: .filter)
        messages = try c.decode([MailRow].self, forKey: .messages)
        query = try c.decodeIfPresent(String.self, forKey: .query) ?? ""
        unread = try c.decodeIfPresent(Int.self, forKey: .unread) ?? 0
        sortable = try c.decodeIfPresent(Int.self, forKey: .sortable) ?? 0
        more = try c.decodeIfPresent(Bool.self, forKey: .more) ?? false
        open = try c.decodeIfPresent(MailOpen.self, forKey: .open)
        pending = try c.decodeIfPresent(MailPending.self, forKey: .pending)
        plan = try c.decodeIfPresent(MailPlan.self, forKey: .plan)
        log = try c.decodeIfPresent([MailLogRow].self, forKey: .log) ?? []
        status = try c.decodeIfPresent(String.self, forKey: .status)
        closed = try c.decodeIfPresent(Bool.self, forKey: .closed) ?? false
    }
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

public struct MailOpen: Codable, Equatable, Sendable {
    public let id: String
    public let from: String
    public let subject: String
    /// Unix seconds.
    public let at: Int64
    public let body: String
    public let attachments: [String]
    public let flagged: Bool
    public let unread: Bool
}

public struct MailPending: Codable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let lines: [String]
}

public struct MailPlan: Codable, Equatable, Sendable {
    public let id: String
    public let summary: String
    public let lines: [String]
    /// Steps left out, and why.
    public let problems: [String]
}

public struct MailLogRow: Codable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let undoable: Bool
}
