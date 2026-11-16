import Foundation

/// The card in front of the island. Each part of Buntini owns its card kinds; their payloads
/// live in Cards/<Part>Cards.swift and src-tauri/src/surface/cards/<part>.rs.
public struct Card: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let source: Source
    public let body: CardBody
}

public enum CardBody: Equatable, Sendable {
    // Coding agents
    case approval(Approval)
    case question(QuestionCard)
    case ticker(Ticker)
    case diff(DiffCard)
    case finished(Finished)
    case recap(Recap)
    case usage(Usage)
    // Developer services
    case service(ServiceCard)
    // Mail
    case mail(MailCard)
    // Media
    case nowPlaying(NowPlaying)
    // Utilities
    case timer(TimerCard)
    case capture(Capture)
    case vitals(Vitals)
    case update(Update)
    case whatsNew(WhatsNew)
    // The island itself
    case drop(DropCard)
    case greeting(Greeting)
    // Voice
    case note(Note)

    /// The `kind` each card is sent with.
    public enum Kind: String, Codable, CaseIterable, Sendable {
        case approval, question, ticker, diff, finished, recap, usage
        case service, mail, nowPlaying, timer, capture, vitals, update, whatsNew
        case drop, greeting, note
    }

    public var kind: Kind {
        switch self {
        case .approval: return .approval
        case .question: return .question
        case .ticker: return .ticker
        case .diff: return .diff
        case .finished: return .finished
        case .recap: return .recap
        case .usage: return .usage
        case .service: return .service
        case .mail: return .mail
        case .nowPlaying: return .nowPlaying
        case .timer: return .timer
        case .capture: return .capture
        case .vitals: return .vitals
        case .update: return .update
        case .whatsNew: return .whatsNew
        case .drop: return .drop
        case .greeting: return .greeting
        case .note: return .note
        }
    }
}

extension CardBody: Codable {
    private enum Key: String, CodingKey { case kind }

    public init(from decoder: Decoder) throws {
        switch try decoder.container(keyedBy: Key.self).decode(Kind.self, forKey: .kind) {
        case .approval: self = .approval(try Approval(from: decoder))
        case .question: self = .question(try QuestionCard(from: decoder))
        case .ticker: self = .ticker(try Ticker(from: decoder))
        case .diff: self = .diff(try DiffCard(from: decoder))
        case .finished: self = .finished(try Finished(from: decoder))
        case .recap: self = .recap(try Recap(from: decoder))
        case .usage: self = .usage(try Usage(from: decoder))
        case .service: self = .service(try ServiceCard(from: decoder))
        case .mail: self = .mail(try MailCard(from: decoder))
        case .nowPlaying: self = .nowPlaying(try NowPlaying(from: decoder))
        case .timer: self = .timer(try TimerCard(from: decoder))
        case .capture: self = .capture(try Capture(from: decoder))
        case .vitals: self = .vitals(try Vitals(from: decoder))
        case .update: self = .update(try Update(from: decoder))
        case .whatsNew: self = .whatsNew(try WhatsNew(from: decoder))
        case .drop: self = .drop(try DropCard(from: decoder))
        case .greeting: self = .greeting(try Greeting(from: decoder))
        case .note: self = .note(try Note(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Key.self)
        try container.encode(kind, forKey: .kind)
        switch self {
        case .approval(let card): try card.encode(to: encoder)
        case .question(let card): try card.encode(to: encoder)
        case .ticker(let card): try card.encode(to: encoder)
        case .diff(let card): try card.encode(to: encoder)
        case .finished(let card): try card.encode(to: encoder)
        case .recap(let card): try card.encode(to: encoder)
        case .usage(let card): try card.encode(to: encoder)
        case .service(let card): try card.encode(to: encoder)
        case .mail(let card): try card.encode(to: encoder)
        case .nowPlaying(let card): try card.encode(to: encoder)
        case .timer(let card): try card.encode(to: encoder)
        case .capture(let card): try card.encode(to: encoder)
        case .vitals(let card): try card.encode(to: encoder)
        case .update(let card): try card.encode(to: encoder)
        case .whatsNew(let card): try card.encode(to: encoder)
        case .drop(let card): try card.encode(to: encoder)
        case .greeting(let card): try card.encode(to: encoder)
        case .note(let card): try card.encode(to: encoder)
        }
    }
}
