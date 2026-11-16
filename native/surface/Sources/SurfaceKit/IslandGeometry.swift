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

    /// The island's size in each mode. `openHeight` is the card's height below the top edge.
    public static func size(_ mode: IslandMode, screen: ScreenShape, openHeight: CGFloat) -> CGSize {
        if let notch = notch(screen) {
            switch mode {
            case .hidden: return notch
            case .peek: return CGSize(width: notch.width + 2 * peekWings, height: notch.height)
            case .open: return CGSize(width: max(openWidth, notch.width + 2 * peekWings), height: notch.height + openHeight)
            }
        }
        let bar = menuBar(screen)
        switch mode {
        case .hidden: return CGSize(width: 0, height: 0)
        case .peek: return CGSize(width: 200, height: bar)
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
