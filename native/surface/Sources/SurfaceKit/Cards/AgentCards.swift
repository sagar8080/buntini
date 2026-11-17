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
    /// "Allow" can be offered: everything it approves is shown. Off when the detail had to be
    /// cut or the request carries something the card can't show; then only Deny and In
    /// terminal are. A Buntini that leaves it out offers no Allow.
    public let allow: Bool
    /// What an edit changes, line by line.
    public let changes: [DiffLine]
    /// What "Always" keeps, and where.
    public let rules: [AgentRule]

    private enum CodingKeys: String, CodingKey {
        case agent, project, title, detail, code, always, plan, allow, changes, rules
    }

    public init(agent: String, project: String?, title: String, detail: String?, code: Bool, always: Bool, plan: Bool, allow: Bool, changes: [DiffLine] = [], rules: [AgentRule] = []) {
        self.agent = agent
        self.project = project
        self.title = title
        self.detail = detail
        self.code = code
        self.always = always
        self.plan = plan
        self.allow = allow
        self.changes = changes
        self.rules = rules
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        agent = try c.decode(String.self, forKey: .agent)
        project = try c.decodeIfPresent(String.self, forKey: .project)
        title = try c.decode(String.self, forKey: .title)
        detail = try c.decodeIfPresent(String.self, forKey: .detail)
        code = try c.decode(Bool.self, forKey: .code)
        always = try c.decode(Bool.self, forKey: .always)
        plan = try c.decodeIfPresent(Bool.self, forKey: .plan) ?? false
        allow = try c.decodeIfPresent(Bool.self, forKey: .allow) ?? false
        changes = try c.decodeIfPresent([DiffLine].self, forKey: .changes) ?? []
        rules = try c.decodeIfPresent([AgentRule].self, forKey: .rules) ?? []
    }

    /// As Rust writes it: empty lists left out.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(agent, forKey: .agent)
        try c.encodeIfPresent(project, forKey: .project)
        try c.encode(title, forKey: .title)
        try c.encodeIfPresent(detail, forKey: .detail)
        try c.encode(code, forKey: .code)
        try c.encode(always, forKey: .always)
        try c.encode(plan, forKey: .plan)
        try c.encode(allow, forKey: .allow)
        if !changes.isEmpty { try c.encode(changes, forKey: .changes) }
        if !rules.isEmpty { try c.encode(rules, forKey: .rules) }
    }
}

/// A rule "Always" keeps, as the agent writes it, and where it's kept.
public struct AgentRule: Codable, Equatable, Sendable {
    public let rule: String
    /// This session or this project, in words.
    public let place: String
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
