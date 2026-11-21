// What the island says at a glance. Beside the notch: the most pressing thing running, in a
// few characters. Opened: a chip for everything running, to switch between their cards.
// Pure, so SurfaceChecks checks it without a screen.
import CoreGraphics

/// The wing beside the notch while the island peeks.
public struct Glance: Equatable, Sendable {
    /// The most pressing pill: the board sends them most urgent first.
    public let lead: Pill
    /// A dot for each of the next few.
    public let others: [Pill]
    /// Running beyond those.
    public let more: Int

    /// Dots shown beside the lead.
    public static let dots = 3
    /// Characters of a badge shown beside the notch.
    public static let longest = 9

    /// Most urgent first, as the board ranks them (`board::rank` in Rust).
    public static func rank(_ state: PillState) -> Int {
        switch state {
        case .waiting: return 0
        case .error: return 1
        case .working: return 2
        case .done: return 3
        case .idle: return 4
        }
    }

    public static func of(_ pills: [Pill], overflow: Int = 0) -> Glance? {
        guard let lead = pills.first else { return nil }
        let rest = Array(pills.dropFirst())
        return Glance(lead: lead, others: Array(rest.prefix(dots)), more: max(0, rest.count - dots) + overflow)
    }

    /// The lead's few characters: time left, a count, a project. Music shows its bars instead;
    /// its title is too long for the wing.
    public var text: String? {
        guard lead.source != .media, let badge = lead.badge?.trimmingCharacters(in: .whitespacesAndNewlines), !badge.isEmpty else { return nil }
        return badge.count > Self.longest ? String(badge.prefix(Self.longest - 1)) + "…" : badge
    }

    /// What the pointer resting on the island reads: the lead in words.
    public var caption: String {
        guard let badge = lead.badge, !badge.isEmpty, badge != lead.label else { return lead.label }
        return "\(lead.label) · \(badge)"
    }

    /// About how wide the readout is: the lead's mark, its text, the dots and "+N".
    public var width: CGFloat {
        var width: CGFloat = 16
        if let text { width += 5 + CGFloat(text.count) * 7.4 }
        if !others.isEmpty { width += 8 + CGFloat(others.count) * 7 + CGFloat(others.count - 1) * 5 }
        if more > 0 { width += 6 + CGFloat(String(more).count + 1) * 7 }
        return width
    }

    /// Each wing beside the notch: the cat on one side, the readout on the other, the same
    /// width so the island stays centred on the notch.
    public var wings: CGFloat {
        min(max(IslandGeometry.peekWings, width + 30), IslandGeometry.widestWing)
    }
}

/// One thing running, as a chip in the opened island.
public struct Chip: Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public let badge: String?
    public let tint: String
    /// The pill it stands for. None for a card with no pill, such as a plan window.
    public let pill: Pill?
    /// The card it opens, when there is one to show.
    public let card: Card?
    /// A sign for a chip with no pill to take its mark from.
    public var symbol: String? = nil
}

public enum Chips {
    /// The most chips in a row.
    public static let most = 5
    /// The widest a chip grows, its badge included.
    public static let widest: CGFloat = 150

    /// The cards that can be looked at: the card in front when it is news, then the shelf.
    public static func cards(_ scene: SurfaceScene) -> [Card] {
        var cards: [Card] = []
        if let card = scene.card, !scene.opens { cards.append(card) }
        for card in scene.shelf ?? [] where !cards.contains(where: { $0.id == card.id }) { cards.append(card) }
        return cards
    }

    /// A chip for every pill, with its card when it has one, then one for each card no pill
    /// stands for (a plan window, music on pause). A diff opens from its step, not a chip.
    public static func of(_ scene: SurfaceScene) -> [Chip] {
        let cards = cards(scene)
        var chips = scene.pills.map { pill in
            Chip(id: pill.id, label: pill.label, badge: pill.badge, tint: pill.tint, pill: pill, card: cards.first(where: { $0.id == pill.cardID }))
        }
        for card in cards where !chips.contains(where: { $0.card?.id == card.id }) {
            guard let (label, badge, tint, symbol) = name(card) else { continue }
            chips.append(Chip(id: card.id, label: label, badge: badge, tint: tint, pill: nil, card: card, symbol: symbol))
        }
        return Array(chips.prefix(most))
    }

    /// What a chip for a card with no pill says, and its sign.
    static func name(_ card: Card) -> (String, String?, String, String)? {
        switch card.body {
        case .usage(let usage):
            let most = usage.windows.map(\.percent).max()
            return (usage.agent, most.map { "\(Int($0.rounded()))%" }, usage.agent.lowercased().contains("codex") ? "#10A37F" : "#D97757", "speedometer")
        case .nowPlaying(let playing):
            return (playing.title, playing.artist, playing.player == "spotify" ? "#1ED760" : "#FA2D48", "pause.fill")
        case .service(let service):
            return (service.title, nil, "#8B949E", "circle.fill")
        case .awake(let awake):
            return (awake.label, nil, "#FFD60A", "cup.and.saucer.fill")
        case .timer(let timer):
            return (timer.label, nil, "#FF7A59", "timer")
        case .answer(let answer):
            return (answer.question, nil, "#F2B33D", "text.bubble")
        case .upcoming(let upcoming):
            return (upcoming.title, nil, upcoming.item == "meeting" ? "#FF453A" : "#FF9F0A", upcoming.item == "meeting" ? "calendar" : "checklist")
        case .shortcuts(let shortcuts):
            return (shortcuts.title, nil, "#FF375F", "square.stack.3d.up.fill")
        case .request(let request):
            return (request.from, nil, "#BF5AF2", "hand.raised.fill")
        case .unread(let unread):
            return (unread.name, nil, "#34C759", "bubble.left.fill")
        case .call(let call):
            return (call.name, nil, "#30D158", call.muted == true ? "mic.slash.fill" : "phone.fill")
        default:
            return nil
        }
    }

    /// Widths that fit `room`: each as wide as it wants while that fits, the widest cut to
    /// an equal share of what the others leave.
    public static func share(_ wanted: [CGFloat], room: CGFloat) -> [CGFloat] {
        guard wanted.reduce(0, +) > room, !wanted.isEmpty else { return wanted }
        var left = room
        var open = wanted.indices.sorted { wanted[$0] < wanted[$1] }
        var given = wanted
        while let first = open.first {
            let fair = left / CGFloat(open.count)
            if wanted[first] <= fair {
                left -= wanted[first]
                open.removeFirst()
            } else {
                for index in open { given[index] = max(0, fair) }
                break
            }
        }
        return given
    }

    /// The card to show: the one chosen, or else the newest news, or else the first chip's.
    public static func selected(_ scene: SurfaceScene, chosen: String?) -> Card? {
        let cards = cards(scene)
        if let chosen, let card = cards.first(where: { $0.id == chosen }) { return card }
        if let card = scene.card, !scene.opens { return card }
        return of(scene).first(where: { $0.card != nil })?.card
    }
}
