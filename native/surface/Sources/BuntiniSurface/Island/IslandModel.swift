import AppKit
import SwiftUI
import SurfaceKit

/// What the island shows and how it is held, on the main thread.
final class IslandModel: ObservableObject {
    @Published var scene: SurfaceScene?
    @Published var screen: ScreenShape
    /// The pointer is on the island (or on the notch while it hides).
    @Published var hover = false
    /// Opened by a click, with no card.
    @Published var expanded = false
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
            ? IslandGeometry.cornerSize(mode, openHeight: openHeight, pills: scene?.pills.count ?? 0)
            : IslandGeometry.size(mode, screen: screen, openHeight: openHeight)
    }

    /// The room a card needs below the top band.
    var openHeight: CGFloat {
        if dragging && scene?.card == nil { return 110 }
        guard let card = scene?.card else { return max(130, 86 + CGFloat(scene?.pills.count ?? 0) * 30) }
        switch card.body {
        case .approval: return 138
        case .question(let q): return 120 + CGFloat(min(q.questions.first?.options.count ?? 2, 4) > 3 ? 40 : 0)
        case .ticker: return 118
        case .diff: return 230
        case .finished: return 92
        case .recap: return 150
        case .usage: return 110
        case .service: return 150
        case .mail: return 200
        case .nowPlaying: return 112
        case .timer: return 100
        case .capture: return 150
        case .vitals: return 92
        case .update: return 112
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
