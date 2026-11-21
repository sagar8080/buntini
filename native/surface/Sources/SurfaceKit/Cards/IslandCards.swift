// Cards the island shows for itself (stream A). Mirror: src-tauri/src/surface/cards/island.rs.
import Foundation

/// Files dropped on the island, and where they can go.
public struct DropCard: Codable, Equatable, Sendable {
    public let files: [String]
    /// Agent sessions the paths can be pasted into.
    public let targets: [DropTarget]
}

public struct DropTarget: Codable, Equatable, Sendable {
    /// The session's pill.
    public let pill: String
    public let label: String
}

/// A short answer to what was typed in the island, shown there rather than opening the chat.
public struct Answer: Codable, Equatable, Sendable {
    public let question: String
    public let answer: String
    /// Where Send to… can open the answer: `whatsapp`, `x`, `threads`, `bluesky`.
    public let sendTo: [String]?
}

/// The shortcuts in the person's "Buntini" folder, as buttons. Mirror: `Shortcuts` in
/// src-tauri/src/surface/cards/island.rs.
public struct Shortcuts: Codable, Equatable, Sendable {
    public let title: String
    public let buttons: [ShortcutButton]
}

public struct ShortcutButton: Codable, Equatable, Sendable {
    public let id: String
    public let name: String
}

/// Something another assistant asks of the person, with Deny and Allow. Mirror: `Request` in
/// src-tauri/src/surface/cards/island.rs.
public struct Request: Codable, Equatable, Sendable {
    public let from: String
    public let what: String
}

/// Unread messages in a chat app, as its Dock icon counts them. Mirror: `Unread` in
/// src-tauri/src/surface/cards/island.rs.
public struct Unread: Codable, Equatable, Sendable {
    public let name: String
    /// "3 unread", or that there are new messages.
    public let line: String
}

/// What's next: a meeting about to start (`item` is `meeting`) or reminders past due
/// (`reminders`).
public struct Upcoming: Codable, Equatable, Sendable {
    public let item: String
    public let title: String
    /// When the meeting starts, in Unix seconds.
    public let starts: Int64?
    /// When the reminder was due, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    public let due: String?
    /// Where, or the reminder's list.
    public let detail: String?
    /// The meeting's call link.
    public let link: String?
    /// The reminder Done marks done.
    public let id: String?
    /// More reminders past due besides this one.
    public let more: UInt32
}

/// Hello on launch.
public struct Greeting: Codable, Equatable, Sendable {
    public let name: String?
}

/// A call in another app: Zoom, Teams, FaceTime, a meeting in a browser. Mirror: `Call` in
/// src-tauri/src/surface/cards/island.rs.
public struct Call: Codable, Equatable, Sendable {
    /// The app's name, as macOS shows it.
    public let name: String
    /// When the call began, in Unix seconds.
    public let since: Int64
    /// Whether the microphone is muted; `nil` when it can't be from here.
    public let muted: Bool?
    /// Whether the call is being recorded for notes; `nil` while "Record calls" is off.
    public let recording: Bool?
}
