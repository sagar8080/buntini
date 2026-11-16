// The surface contract. Rust owns the state and sends a whole SurfaceScene; Swift draws it and sends
// Actions back. src-tauri/src/surface/ holds the same types, and Fixtures/contract.json is
// checked on both sides, so a change here must be made there too.
import Foundation

/// Bumped whenever a change on one side would break the other.
public let surfaceContract: UInt32 = 1

/// Which part of Buntini a pill, card or action belongs to.
public enum Source: String, Codable, CaseIterable, Sendable {
    case agents, services, mail, media, utilities, flow, island
}

/// Where the character lives.
public enum Home: String, Codable, Sendable {
    case island, desktop
}

/// The notch or menu bar (`auto`), or a corner the character was dragged to.
public enum Placement: String, Codable, Sendable {
    case auto, topLeft, topRight, bottomLeft, bottomRight
}

public enum PillState: String, Codable, Sendable {
    case idle, working, waiting, done, error
}

public struct Pill: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let source: Source
    public let label: String
    /// `#rrggbb`.
    public let tint: String
    public let state: PillState
    public let badge: String?
    /// 0…1 when the pill shows progress.
    public let progress: Double?
}

public enum Mood: String, Codable, Sendable {
    case calm, happy, curious, sleepy, alert, grumpy, dancing
}

/// The cat's look, the same choices as Settings (src/petOptions.ts).
public struct Cat: Codable, Equatable, Sendable {
    /// `cat` or `orb`.
    public let character: String
    public let coat: String
    public let eyes: String
    public let accessory: String
    /// `#rrggbb`.
    public let accent: String
    public let mood: Mood
    /// A seasonal outfit, when one applies today.
    public let outfit: String?
}

/// A sound to play once. A new sequence number plays it again.
public struct Sound: Codable, Equatable, Sendable {
    public let name: String
    public let sequence: UInt64
}

public struct SurfaceScene: Codable, Equatable, Sendable {
    public let contract: UInt32
    public let home: Home
    public let placement: Placement
    public let pills: [Pill]
    /// Ids of waiting pills that didn't fit, shown as "+N".
    public let overflow: [String]
    /// The card in front, if any.
    public let card: Card?
    /// Something waits on the person: the island stays open.
    public let held: Bool
    public let cat: Cat
    public let flow: FlowPill?
    public let overlay: Overlay?
    public let sound: Sound?
}

/// What the person did, sent back to Rust.
public struct Action: Codable, Equatable, Sendable {
    public let source: Source
    /// The card it came from.
    public let card: String?
    public let name: String
    public let value: JSONValue?

    public init(source: Source, card: String?, name: String, value: JSONValue? = nil) {
        self.source = source
        self.card = card
        self.name = name
        self.value = value
    }
}

/// Any JSON value, for action payloads.
public indirect enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([JSONValue].self) { self = .array(value) }
        else { self = .object(try container.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }
}
