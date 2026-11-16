// Cards for coding agents (stream B). Mirror: src-tauri/src/surface/cards/agents.rs.
import Foundation

/// A tool call waiting for Allow or Deny.
public struct Approval: Codable, Equatable, Sendable {
    public let agent: String
    public let project: String?
    public let title: String
    public let detail: String?
    /// The detail is code or a command, shown monospaced.
    public let code: Bool
    /// "Always" can be offered.
    public let always: Bool
    /// The agent's plan, waiting to be started.
    public let plan: Bool
}

/// One to four questions an agent asks.
public struct QuestionCard: Codable, Equatable, Sendable {
    public let agent: String
    public let project: String?
    public let questions: [AgentQuestion]
}

public struct AgentQuestion: Codable, Equatable, Sendable {
    public let header: String?
    public let question: String
    public let options: [AgentChoice]
    public let multi: Bool
}

public struct AgentChoice: Codable, Equatable, Sendable {
    public let label: String
    public let description: String?
}

/// What a session is doing, step by step.
public struct Ticker: Codable, Equatable, Sendable {
    public let agent: String
    public let project: String?
    public let steps: [TickerStep]
}

public struct TickerStep: Codable, Equatable, Sendable {
    public let label: String
    public let detail: String?
    public let added: Int?
    public let removed: Int?
    public let failed: Bool
}

/// One file's changes.
public struct DiffCard: Codable, Equatable, Sendable {
    public let file: String
    public let added: Int
    public let removed: Int
    public let lines: [DiffLine]
}

public struct DiffLine: Codable, Equatable, Sendable {
    /// `add`, `remove`, `same` or `gap`.
    public let kind: String
    public let text: String
}

/// A turn finished.
public struct Finished: Codable, Equatable, Sendable {
    public let agent: String
    public let project: String?
    public let message: String?
}

/// The week with the agents.
public struct Recap: Codable, Equatable, Sendable {
    public let week: String
    public let stats: [RecapStat]
}

public struct RecapStat: Codable, Equatable, Sendable {
    public let label: String
    public let value: String
}

/// Plan limits of an agent's subscription.
public struct Usage: Codable, Equatable, Sendable {
    public let agent: String
    public let windows: [UsageWindow]
}

public struct UsageWindow: Codable, Equatable, Sendable {
    public let label: String
    /// 0…100.
    public let percent: Double
    /// Unix seconds.
    public let resetsAt: Int64?
}
