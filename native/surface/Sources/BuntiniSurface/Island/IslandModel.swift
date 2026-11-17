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
        }
    }
    @Published var screen: ScreenShape
    /// The pointer is on the island (or on the notch while it hides).
    @Published var hover = false
    /// Opened by a click, with no card.
    @Published var expanded = false {
        didSet { if !expanded { chosen = nil } }
    }
    /// The card picked from the chips in the opened island, until it closes.
    @Published var chosen: String?
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
        inCorner
            ? IslandGeometry.cornerSize(mode, openHeight: openHeight, pills: scene?.pills.count ?? 0, glance: glance?.width ?? 0)
            : IslandGeometry.size(mode, screen: screen, openHeight: openHeight, wings: glance?.wings ?? IslandGeometry.peekWings)
    }

    /// Beside the notch: the most pressing thing running.
    var glance: Glance? {
        scene.flatMap { Glance.of($0.pills, overflow: $0.overflow.count) }
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

    /// The room below the top band.
    var openHeight: CGFloat {
        if dragging && scene?.card == nil { return 110 }
        if let card = interrupting { return Self.height(card) }
        // Opened by a click: the question field, the chips and the chosen card, 12 apart.
        var height: CGFloat = 10 + 30 + 16
        if !chips.isEmpty { height += 12 + 28 }
        if let card = selected { height += 12 + Self.height(card) - 24 }
        return height
    }

    /// What a card needs below the top band, with room around it.
    static func height(_ card: Card) -> CGFloat {
        switch card.body {
        // The whole command and every changed line, scrolling past a point, and the rules
        // Always keeps.
        case .approval(let approval):
            return 100 + (approval.title.count > 52 ? 18 : 0) + (ApprovalCardView.hasBody(approval) ? ApprovalCardView.bodyHeight(approval) + 8 : 0) + ApprovalCardView.rulesHeight(approval)
        case .question(let q): return 120 + CGFloat(min(q.questions.first?.options.count ?? 2, 4) > 3 ? 40 : 0)
        // The heading, then the last four steps.
        case .ticker(let ticker): return 24 + 16 + CGFloat(min(max(ticker.steps.count, 1), 4)) * 21
        case .diff: return 230
        case .finished: return 92
        case .recap: return 150
        case .usage: return 110
        case .service: return 150
        // A change or a plan waiting for Allow or Run lists every line it carries out.
        case .mail(let mail): return mail.pending != nil || mail.plan != nil ? 290 : 200
        case .nowPlaying: return 112
        case .timer: return 100
        case .capture: return 150
        case .vitals: return 92
        // A failure's reason may take two lines in longer languages.
        case .update(let update): return update.stage == "failed" ? 128 : 112
        case .whatsNew: return 130
        case .drop: return 112
        case .greeting: return 84
        case .note: return 160
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
