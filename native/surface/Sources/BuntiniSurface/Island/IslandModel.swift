import AppKit
import SwiftUI
import SurfaceKit

/// What the island shows and how it is held, on the main thread.
final class IslandModel: ObservableObject {
    @Published var scene: SurfaceScene? {
        didSet {
            // A chip chosen stays chosen while the island is open; only a diff asked for from
            // a step comes forward in its place.
            if let card = scene?.card, card.id != oldValue?.card?.id, case .diff = card.body { chosen = nil }
            // News seen stays seen until that session has news again.
            let news = Set(scene?.pills.filter(\.isNews).map(\.id) ?? [])
            seenNews.formIntersection(news)
            if expanded { seenNews.formUnion(news) }
            measureAhead()
        }
    }
    @Published var screen: ScreenShape {
        didSet { measureAhead() }
    }
    /// The pointer is on the island (or on the notch while it hides).
    @Published var hover = false
    /// Opened by a click, with no card.
    @Published var expanded = false {
        didSet {
            if !expanded {
                chosen = nil
                asking = false
            } else {
                // Opened: what was news has been seen.
                seenNews.formUnion(scene?.pills.filter(\.isNews).map(\.id) ?? [])
                changedAt = Date()
            }
            measureAhead()
        }
    }
    /// Ask was chosen: the question field shows above what's running until it's left empty.
    @Published var asking = false
    /// The card picked from the chips in the opened island, until it closes.
    @Published var chosen: String? {
        didSet { if chosen != oldValue { changedAt = Date() } }
    }
    /// When the island opened or another card was chosen: Return waits out the arming delay.
    private(set) var changedAt = Date.distantPast
    /// What was typed on the opened island to start Ask, for the field to take.
    var askPrefill = ""
    /// Files are being dragged over it.
    @Published var dragging = false
    /// The screen is asleep or locked.
    @Published var asleep = false
    /// The cat is being carried to a new place.
    @Published var carrying = false
    /// Someone is typing in the island.
    @Published var typing = false
    /// Set by the window: the cat was picked up, moved, or let go.
    var carry: (CarryPhase) -> Void = { _ in }
    /// Drawn into a picture rather than on screen, where a card shows the top of what scrolls.
    var inPicture = false
    /// Each card's height as last measured, by card id.
    private var measured: [String: Measured] = [:]
    /// The tallest the opened island has been while the pointer stayed on it.
    private var held: CGFloat?
    /// Agent pills whose finished or failed turn was seen by opening the island.
    private var seenNews: Set<String> = []

    private struct Measured {
        let card: Card
        let width: CGFloat
        let words: String
        let height: CGFloat
    }

    enum CarryPhase { case moved, ended }

    init(screen: ScreenShape) {
        self.screen = screen
    }

    var hasNotch: Bool { IslandGeometry.notch(screen) != nil }

    var placement: Placement { scene?.placement ?? .auto }
    var inCorner: Bool { placement != .auto }

    var mode: IslandMode {
        guard let scene, !asleep else { return .hidden }
        // Waiting requests and alerts open it; other cards wait until it's opened.
        if (scene.card != nil && scene.opens) || expanded || dragging || typing { return .open }
        if hover || !scene.pills.isEmpty { return .peek }
        return .hidden
    }

    /// Where the island is inside the panel (top-left origin).
    var localRect: CGRect { IslandGeometry.localRect(size: size, placement: placement) }

    /// The top band: the notch, or the menu bar without one.
    var band: CGFloat { IslandGeometry.notch(screen)?.height ?? IslandGeometry.menuBar(screen) }

    var size: CGSize {
        let mode = mode
        // Only an opened island has a card to measure.
        let open = mode == .open ? openHeight : 0
        return inCorner
            ? IslandGeometry.cornerSize(mode, openHeight: open, pills: scene?.pills.count ?? 0, glance: glance?.width ?? 0)
            : IslandGeometry.size(mode, screen: screen, openHeight: open, wings: glance?.wings ?? IslandGeometry.peekWings)
    }

    /// Beside the notch: the most pressing thing running.
    var glance: Glance? {
        scene.flatMap { Glance.of(pills, overflow: $0.overflow.count) }
    }

    /// The pills beside the notch, most urgent first: an agent's finished or failed turn is
    /// news until the island is opened, then it reads as any other session.
    var pills: [Pill] {
        let shown = (scene?.pills ?? []).map { $0.isNews && seenNews.contains($0.id) ? $0.seen : $0 }
        return shown.enumerated()
            .sorted { (Glance.rank($0.element.state), $0.offset) < (Glance.rank($1.element.state), $1.offset) }
            .map(\.element)
    }

    /// The cards the chips switch between, in order: what ←, → and a swipe move through.
    private var switchable: [Card] { chips.compactMap(\.card) }

    /// Shows the next (`delta` 1) or previous (-1) chip's card, stopping at either end. Only
    /// on the island opened by a click.
    func step(by delta: Int) {
        let cards = switchable
        guard mode == .open, interrupting == nil, !cards.isEmpty else { return }
        let index = selected.flatMap { current in cards.firstIndex(where: { $0.id == current.id }) } ?? 0
        let next = min(max(index + delta, 0), cards.count - 1)
        if next != index { chosen = cards[next].id }
    }

    /// What Return does on the island opened by a click: the chosen card's one button, unless
    /// it's a decision; nothing in the moment after it comes forward.
    func primary(now: Date = Date()) -> Action? {
        guard mode == .open, interrupting == nil, !asking, let card = selected, Arming.ready(since: changedAt, now: now) else { return nil }
        return Self.primary(card)
    }

    static func primary(_ card: Card) -> Action? {
        switch card.body {
        case .nowPlaying(let playing): return Action(source: .media, card: card.id, name: playing.playing ? "pause" : "play")
        case .timer(let timer): return Action(source: .utilities, card: card.id, name: timer.phase == "done" ? "focus" : timer.paused ? "resume" : "pause")
        case .capture(let capture) where !capture.path.isEmpty: return Action(source: .utilities, card: card.id, name: "paste")
        case .recap: return Action(source: .agents, card: card.id, name: "shareRecap")
        case .note: return Action(source: .flow, card: card.id, name: "library")
        case .upcoming(let upcoming) where upcoming.item == "meeting" && upcoming.link != nil:
            return Action(source: .island, card: card.id, name: "join", value: .string(upcoming.link ?? ""))
        case .call(let call) where call.muted != nil: return Action(source: .island, card: card.id, name: "mute")
        case .awake: return Action(source: .utilities, card: card.id, name: "stop")
        default: return nil
        }
    }

    /// Typing on the opened island starts Ask with what was typed.
    func ask(with text: String) {
        askPrefill += text
        asking = true
        typing = true
    }

    /// A card that opened the island to say one thing (a finished turn, a hello, how the Mac
    /// is doing, being up to date) shows as a line under the band for its moment.
    var line: Card? {
        guard !inCorner, let card = interrupting, Self.saysOneLine(card) else { return nil }
        return card
    }

    static func saysOneLine(_ card: Card) -> Bool {
        switch card.body {
        case .finished, .greeting, .vitals: return true
        case .update(let update): return update.stage == "current"
        default: return false
        }
    }

    /// The card that opened the island by itself, to be answered or read.
    var interrupting: Card? {
        guard let scene, scene.opens else { return nil }
        return scene.card
    }

    /// Opened with nothing asking: a chip for each thing running, and the chosen one's card.
    var chips: [Chip] { scene.map(Chips.of) ?? [] }
    var selected: Card? { scene.flatMap { Chips.selected($0, chosen: chosen) } }

    /// The cat steps out of the wing to sit beside these: a hello, a finished turn, files to
    /// place.
    static func catBeside(_ card: Card) -> Bool {
        switch card.body {
        case .greeting, .finished, .drop: return true
        default: return false
        }
    }

    /// The room below the top band: the card as tall as it measures, rounded up to a step, and
    /// never shorter than it was while the pointer stays on the island.
    var openHeight: CGFloat {
        let natural = naturalOpenHeight
        guard mode == .open else {
            held = nil
            return natural
        }
        let height = CardSteps.hold(natural, held: held, pointerIn: hover)
        held = height
        return height
    }

    private var naturalOpenHeight: CGFloat {
        if dragging && scene?.card == nil { return 110 }
        if line != nil { return LineView.height }
        if let card = interrupting {
            let beside = Self.catBeside(card)
            // Beside the cat a card is at least as tall as the cat.
            let content = beside ? max(68, contentHeight(card, beside: true)) : contentHeight(card, beside: false)
            return 10 + CardSteps.snap(content) + 16
        }
        // Opened by a click: the question field, the chips and the chosen card, 12 apart, as
        // many of them as show.
        var rows: [CGFloat] = []
        if showsField { rows.append(30) }
        if showsChips { rows.append(28) }
        if let card = selected { rows.append(CardSteps.snap(contentHeight(card, beside: false))) }
        return 10 + rows.reduce(0, +) + CGFloat(max(rows.count - 1, 0)) * 12 + 16
    }

    /// Opened by a click with nothing asking: Ask, Focus and Chat sit in the band.
    var showsControls: Bool {
        mode == .open && interrupting == nil && !(dragging && scene?.card == nil)
    }

    /// The question field, once Ask is chosen, or when there's nothing else to show.
    var showsField: Bool {
        asking || (selected == nil && !showsChips)
    }

    /// Chips once there's more than one thing to switch between, or one with no card of its
    /// own that only a chip reaches.
    var showsChips: Bool {
        let chips = chips
        return chips.count > 1 || chips.first.map { $0.card == nil } == true
    }

    /// The width cards are laid out at once opened: the island less its 18 pt sides.
    var cardWidth: CGFloat {
        let width = inCorner
            ? IslandGeometry.openWidth
            : IslandGeometry.size(.open, screen: screen, openHeight: 0, wings: glance?.wings ?? IslandGeometry.peekWings).width
        return width - 36
    }

    /// How tall `card` sets, measured as the island lays it out in the person's language. Beside
    /// the cat it's narrower by the cat and the space after it.
    func contentHeight(_ card: Card, beside: Bool) -> CGFloat {
        let width = cardWidth - (beside ? 78 : 0)
        let words = Lexicon.locale.identifier + (Lexicon.direction == .rightToLeft ? "/rtl" : "")
        let key = (beside ? "beside:" : "") + card.id
        if let known = measured[key], known.card == card, known.width == width, known.words == words {
            return known.height
        }
        let height = CardMeasure.height(card, width: width, inPicture: inPicture)
        measured[key] = Measured(card: card, width: width, words: words, height: height)
        return height
    }

    /// Whether `card` fits the largest step as it is. One that doesn't is laid out in what the
    /// island has, and its scrolling part gives way.
    func fits(_ card: Card, beside: Bool) -> Bool {
        contentHeight(card, beside: beside) <= CardSteps.largest + 0.5
    }

    /// Measures the cards that may show next, while nothing is being drawn: the one opening the
    /// island, and once it's open, the ones its chips switch to. Cards no longer around are
    /// forgotten.
    private func measureAhead() {
        guard let scene, !asleep else {
            measured.removeAll()
            return
        }
        let cards = Chips.cards(scene) + [scene.card].compactMap { $0 }
        measured = measured.filter { entry in cards.contains { $0.id == entry.value.card.id } }
        if let card = interrupting { _ = contentHeight(card, beside: Self.catBeside(card)) }
        if expanded || typing {
            for card in Chips.cards(scene) { _ = contentHeight(card, beside: false) }
        }
    }
}

extension ScreenShape {
    init(_ screen: NSScreen) {
        var left: CGFloat?, right: CGFloat?
        if #available(macOS 12.0, *) {
            left = screen.auxiliaryTopLeftArea?.width
            right = screen.auxiliaryTopRightArea?.width
        }
        let top: CGFloat
        if #available(macOS 12.0, *) { top = screen.safeAreaInsets.top } else { top = 0 }
        self.init(frame: screen.frame, visibleFrame: screen.visibleFrame, safeTop: top, auxLeft: left, auxRight: right, statusBar: NSStatusBar.system.thickness)
    }
}
