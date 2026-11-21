// Where the island sits and how big it is, from a screen's measurements alone, so the rules are
// checked without a display (SurfaceChecks). Coordinates are AppKit's: origin bottom left.
import CoreGraphics

/// What the island needs to know about a screen.
public struct ScreenShape: Equatable, Sendable {
    public var frame: CGRect
    public var visibleFrame: CGRect
    /// `safeAreaInsets.top`: the notch's height, 0 without one.
    public var safeTop: CGFloat
    /// Widths of `auxiliaryTopLeftArea` and `auxiliaryTopRightArea`, when known.
    public var auxLeft: CGFloat?
    public var auxRight: CGFloat?
    /// The menu bar's height when it is hidden or the screen is in full screen.
    public var statusBar: CGFloat

    public init(frame: CGRect, visibleFrame: CGRect, safeTop: CGFloat, auxLeft: CGFloat?, auxRight: CGFloat?, statusBar: CGFloat) {
        self.frame = frame
        self.visibleFrame = visibleFrame
        self.safeTop = safeTop
        self.auxLeft = auxLeft
        self.auxRight = auxRight
        self.statusBar = statusBar
    }
}

public enum IslandMode: Equatable, Sendable {
    case hidden, peek, open
}

public enum IslandGeometry {
    /// The panel's fixed size. Only the shape inside it moves.
    public static let panel = CGSize(width: 720, height: 560)
    /// Room each side of the notch for the cat and the pills when peeking.
    public static let peekWings: CGFloat = 80
    public static let openWidth: CGFloat = 560

    /// The hardware notch, when the screen has one.
    public static func notch(_ screen: ScreenShape) -> CGSize? {
        guard screen.safeTop > 0 else { return nil }
        var width: CGFloat = 184
        if let left = screen.auxLeft, let right = screen.auxRight, left > 0, right > 0 {
            width = screen.frame.width - left - right
        }
        return CGSize(width: max(80, width), height: screen.safeTop)
    }

    /// The menu bar's height, or the status bar's when the menu bar is hidden.
    public static func menuBar(_ screen: ScreenShape) -> CGFloat {
        let shown = screen.frame.maxY - screen.visibleFrame.maxY
        return shown > 0 ? min(shown, 40) : screen.statusBar
    }

    /// Widest a wing grows to hold what it shows.
    public static let widestWing: CGFloat = 150

    /// The island's size in each mode. `openHeight` is the card's height below the top edge;
    /// `wings` the room each side of the notch, wider while there is more to show.
    public static func size(_ mode: IslandMode, screen: ScreenShape, openHeight: CGFloat, wings: CGFloat = peekWings) -> CGSize {
        let wings = min(max(wings, peekWings), widestWing)
        if let notch = notch(screen) {
            switch mode {
            case .hidden: return notch
            case .peek: return CGSize(width: notch.width + 2 * wings, height: notch.height)
            case .open: return CGSize(width: max(openWidth, notch.width + 2 * wings), height: notch.height + openHeight)
            }
        }
        let bar = menuBar(screen)
        switch mode {
        case .hidden: return CGSize(width: 0, height: 0)
        case .peek: return CGSize(width: 40 + 2 * wings, height: bar)
        case .open: return CGSize(width: openWidth, height: bar + openHeight)
        }
    }

    /// The panel's frame: centred at the top edge of the screen.
    public static func panelFrame(_ screen: ScreenShape) -> CGRect {
        CGRect(x: screen.frame.midX - panel.width / 2, y: screen.frame.maxY - panel.height, width: panel.width, height: panel.height)
    }

    /// The island's rectangle on screen for a size, hanging from the top edge.
    public static func islandRect(panel: CGRect, size: CGSize) -> CGRect {
        CGRect(x: panel.midX - size.width / 2, y: panel.maxY - size.height, width: size.width, height: size.height)
    }

    /// Where hovering wakes the island while it is hidden: the notch, or a thin strip at the top
    /// centre of a screen without one.
    public static func wakeRect(_ screen: ScreenShape, panel: CGRect) -> CGRect {
        let size = notch(screen) ?? CGSize(width: 200, height: max(4, min(8, menuBar(screen))))
        return islandRect(panel: panel, size: size)
    }

    /// How often to check the pointer, by its distance from the island: often near it, rarely
    /// far away, so an idle island costs next to nothing.
    public static func pollInterval(distance: CGFloat, dragging: Bool) -> Double {
        if dragging { return 1.0 / 120 }
        if distance <= 24 { return 1.0 / 60 }
        if distance <= 200 { return 1.0 / 20 }
        return 1.0 / 8
    }

    /// Distance from a point to a rectangle, 0 inside.
    public static func distance(_ point: CGPoint, to rect: CGRect) -> CGFloat {
        let dx = max(rect.minX - point.x, 0, point.x - rect.maxX)
        let dy = max(rect.minY - point.y, 0, point.y - rect.maxY)
        return hypot(dx, dy)
    }
}

// MARK: - Corners

extension IslandGeometry {
    /// The tile the cat rests in when the island sits in a corner.
    public static let tile = CGSize(width: 56, height: 56)
    /// Space between a corner island and the screen's edges.
    public static let cornerMargin: CGFloat = 12
    /// How close to a corner a dropped cat has to land to settle there.
    public static let cornerReach: CGFloat = 180

    /// The panel's frame for a placement: centred at the top, or with its corner on the
    /// matching corner of the visible screen (clear of the menu bar and the Dock).
    public static func panelFrame(_ screen: ScreenShape, placement: Placement) -> CGRect {
        let visible = screen.visibleFrame
        switch placement {
        case .auto: return panelFrame(screen)
        case .topLeft: return CGRect(x: visible.minX, y: visible.maxY - panel.height, width: panel.width, height: panel.height)
        case .topRight: return CGRect(x: visible.maxX - panel.width, y: visible.maxY - panel.height, width: panel.width, height: panel.height)
        case .bottomLeft: return CGRect(x: visible.minX, y: visible.minY, width: panel.width, height: panel.height)
        case .bottomRight: return CGRect(x: visible.maxX - panel.width, y: visible.minY, width: panel.width, height: panel.height)
        }
    }

    /// The island's size in a corner: the tile at rest, the tile and its pills when peeking,
    /// the card when open.
    public static func cornerSize(_ mode: IslandMode, openHeight: CGFloat, pills: Int, glance: CGFloat? = nil) -> CGSize {
        switch mode {
        case .hidden: return tile
        case .peek:
            if let glance { return CGSize(width: tile.width + (glance > 0 ? 14 + glance : 0), height: tile.height) }
            return CGSize(width: tile.width + (pills > 0 ? 18 + 13 * CGFloat(min(pills, 4)) : 0), height: tile.height)
        case .open: return CGSize(width: openWidth, height: 20 + openHeight)
        }
    }

    /// Where the island sits inside the panel, top-left origin (SwiftUI's), for a placement.
    public static func localRect(size: CGSize, placement: Placement) -> CGRect {
        let m = cornerMargin
        switch placement {
        case .auto: return CGRect(x: (panel.width - size.width) / 2, y: 0, width: size.width, height: size.height)
        case .topLeft: return CGRect(x: m, y: m, width: size.width, height: size.height)
        case .topRight: return CGRect(x: panel.width - m - size.width, y: m, width: size.width, height: size.height)
        case .bottomLeft: return CGRect(x: m, y: panel.height - m - size.height, width: size.width, height: size.height)
        case .bottomRight: return CGRect(x: panel.width - m - size.width, y: panel.height - m - size.height, width: size.width, height: size.height)
        }
    }

    /// The same rectangle on screen (AppKit's bottom-left origin).
    public static func screenRect(panel frame: CGRect, size: CGSize, placement: Placement) -> CGRect {
        let local = localRect(size: size, placement: placement)
        return CGRect(x: frame.minX + local.minX, y: frame.maxY - local.maxY, width: local.width, height: local.height)
    }

    /// Where a cat dropped at `point` asks the island to go: a corner it landed near, the top
    /// centre, or nowhere new.
    public static func placement(dropAt point: CGPoint, screen: ScreenShape) -> Placement? {
        let visible = screen.visibleFrame
        let corners: [(Placement, CGPoint)] = [
            (.topLeft, CGPoint(x: visible.minX, y: visible.maxY)), (.topRight, CGPoint(x: visible.maxX, y: visible.maxY)),
            (.bottomLeft, CGPoint(x: visible.minX, y: visible.minY)), (.bottomRight, CGPoint(x: visible.maxX, y: visible.minY)),
        ]
        if let near = corners.min(by: { hypot(point.x - $0.1.x, point.y - $0.1.y) < hypot(point.x - $1.1.x, point.y - $1.1.y) }),
           hypot(point.x - near.1.x, point.y - near.1.y) <= cornerReach {
            return near.0
        }
        if abs(point.x - screen.frame.midX) <= 200 && screen.frame.maxY - point.y <= 140 {
            return .auto
        }
        return nil
    }
}

// MARK: - Opened heights

/// The few heights an opened island's card takes. Each card is measured as laid out and
/// rounded up to the next step, so the island opens at one of three sizes rather than as tall
/// as whatever it holds.
public enum CardSteps {
    /// Room for a card below the band, not counting the 10 pt above it and 16 below: S, M and
    /// L. Under a 32 pt notch they make an island 144, 200 or 280 pt tall.
    public static let all: [CGFloat] = [86, 142, 222]
    /// A card that says one thing opens as a line this tall below the band, 62 pt in all
    /// under a 32 pt notch.
    public static let line: CGFloat = 30
    public static var largest: CGFloat { all[all.count - 1] }

    /// The smallest step that holds `content`. Past the largest it's the largest, and the card
    /// scrolls inside it.
    public static func snap(_ content: CGFloat) -> CGFloat {
        all.first(where: { content <= $0 + 0.5 }) ?? largest
    }

    /// While the pointer stays on the island it only grows, so what it's aiming at doesn't move
    /// away; it settles once the pointer leaves.
    public static func hold(_ height: CGFloat, held: CGFloat?, pointerIn: Bool) -> CGFloat {
        pointerIn ? max(height, held ?? 0) : height
    }
}
