// Choosing an area to capture: every screen dims, the area you drag comes clear with its size
// beside it, and Esc or a right click lets go. The panels never take focus from the app you
// were in, so the island can paste back into it. Core Animation draws it; nothing runs while
// the pointer is still. The chosen area goes back as `selected`, in points from the main
// screen's top left.
import AppKit
import QuartzCore
import SurfaceKit

final class SelectionOverlay {
    private let send: (Action) -> Void
    private var panels: [SelectionPanel] = []
    private var monitors: [Any] = []
    /// Called once the person chose or let go.
    var onFinish: (() -> Void)?

    var active: Bool { !panels.isEmpty }

    init(send: @escaping (Action) -> Void) {
        self.send = send
    }

    func show() {
        guard panels.isEmpty, let main = NSScreen.screens.first else { return }
        for screen in NSScreen.screens {
            let panel = SelectionPanel(screen: screen)
            panel.selection.onChosen = { [weak self] rect in
                self?.finish(rect.map { Pictures.topLeft($0, main: main.frame) })
            }
            panel.selection.onCancel = { [weak self] in self?.finish(nil) }
            panel.alphaValue = 0
            panel.orderFrontRegardless()
            panels.append(panel)
        }
        let pointer = NSEvent.mouseLocation
        (panels.first { NSMouseInRect(pointer, $0.frame, false) } ?? panels.first)?.makeKey()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.14
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            for panel in panels { panel.animator().alphaValue = 1 }
        }
        NSCursor.crosshair.push()
        // Esc, whether or not a panel got the keys.
        let escape: (NSEvent) -> Bool = { [weak self] event in
            guard event.keyCode == 53 else { return false }
            self?.finish(nil)
            return true
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: .keyDown, handler: { escape($0) ? nil : $0 }) { monitors.append(local) }
        if let global = NSEvent.addGlobalMonitorForEvents(matching: .keyDown, handler: { _ = escape($0) }) { monitors.append(global) }
    }

    /// Takes the dimmer down at once, so it isn't in the picture.
    func hide() {
        guard !panels.isEmpty else { return }
        for panel in panels {
            panel.alphaValue = 0
            panel.orderOut(nil)
        }
        panels = []
        monitors.forEach(NSEvent.removeMonitor)
        monitors = []
        NSCursor.pop()
    }

    private func finish(_ rect: CGRect?) {
        guard active else { return }
        hide()
        onFinish?()
        if let rect {
            send(Action(source: .utilities, card: nil, name: "selected", value: .array([rect.minX, rect.minY, rect.width, rect.height].map { .number(Double($0)) })))
        } else {
            send(Action(source: .utilities, card: nil, name: "selectCancelled"))
        }
    }
}

final class SelectionPanel: NSPanel {
    let selection: SelectionView

    init(screen: NSScreen) {
        selection = SelectionView(frame: NSRect(origin: .zero, size: screen.frame.size))
        super.init(contentRect: screen.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .screenSaver
        isFloatingPanel = true
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        ignoresMouseEvents = false
        acceptsMouseMovedEvents = true
        sharingType = .none
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        contentView = selection
        setFrame(screen.frame, display: false)
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

final class SelectionView: NSView {
    var onChosen: ((CGRect?) -> Void)?
    var onCancel: (() -> Void)?
    private let dim = CAShapeLayer()
    private let edge = CAShapeLayer()
    private let guides = CAShapeLayer()
    private let badge = CALayer()
    private let badgeText = CATextLayer()
    private var start: CGPoint?
    private var area: CGRect?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        let scale = NSScreen.main?.backingScaleFactor ?? 2
        dim.fillRule = .evenOdd
        dim.fillColor = NSColor.black.withAlphaComponent(0.34).cgColor
        edge.fillColor = nil
        edge.strokeColor = NSColor.white.withAlphaComponent(0.95).cgColor
        edge.lineWidth = 1
        edge.shadowColor = NSColor.black.cgColor
        edge.shadowOpacity = 0.45
        edge.shadowRadius = 3
        edge.shadowOffset = .zero
        guides.strokeColor = NSColor.white.withAlphaComponent(0.28).cgColor
        guides.lineWidth = 1
        guides.lineDashPattern = [4, 4]
        badge.backgroundColor = NSColor.black.withAlphaComponent(0.62).cgColor
        badge.cornerRadius = 7
        badge.cornerCurve = .continuous
        badge.isHidden = true
        badgeText.font = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .semibold)
        badgeText.fontSize = 11
        badgeText.foregroundColor = NSColor.white.cgColor
        badgeText.alignmentMode = .center
        badgeText.contentsScale = scale
        badge.addSublayer(badgeText)
        for sublayer in [dim, guides, edge, badge] {
            sublayer.contentsScale = scale
            layer?.addSublayer(sublayer)
        }
        show(nil)
    }

    required init?(coder: NSCoder) { nil }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.activeAlways, .mouseMoved, .mouseEnteredAndExited, .cursorUpdate, .inVisibleRect], owner: self))
    }

    override func cursorUpdate(with event: NSEvent) {
        NSCursor.crosshair.set()
    }

    override func mouseMoved(with event: NSEvent) {
        guard start == nil else { return }
        NSCursor.crosshair.set()
        let point = convert(event.locationInWindow, from: nil)
        let path = CGMutablePath()
        path.move(to: CGPoint(x: 0, y: point.y.rounded() + 0.5))
        path.addLine(to: CGPoint(x: bounds.maxX, y: point.y.rounded() + 0.5))
        path.move(to: CGPoint(x: point.x.rounded() + 0.5, y: 0))
        path.addLine(to: CGPoint(x: point.x.rounded() + 0.5, y: bounds.maxY))
        quietly { guides.path = path }
    }

    override func mouseExited(with event: NSEvent) {
        quietly { guides.path = nil }
    }

    override func mouseDown(with event: NSEvent) {
        start = convert(event.locationInWindow, from: nil)
        quietly { guides.path = nil }
    }

    override func mouseDragged(with event: NSEvent) {
        guard let start else { return }
        let point = convert(event.locationInWindow, from: nil)
        let rect = CGRect(x: min(start.x, point.x), y: min(start.y, point.y), width: abs(point.x - start.x), height: abs(point.y - start.y)).integral
        area = rect
        show(rect)
    }

    override func mouseUp(with event: NSEvent) {
        defer { start = nil }
        guard let area, area.width >= 4, area.height >= 4, let window else {
            // A click without a drag: keep choosing.
            self.area = nil
            show(nil)
            return
        }
        onChosen?(window.convertToScreen(convert(area, to: nil)))
    }

    override func rightMouseDown(with event: NSEvent) {
        onCancel?()
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { onCancel?() } else { super.keyDown(with: event) }
    }

    /// An area as if dragged, for pictures (OverlaySnapshot).
    func preview(_ rect: CGRect?) {
        show(rect)
    }

    private func quietly(_ change: () -> Void) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        change()
        CATransaction.commit()
    }

    private func show(_ rect: CGRect?) {
        quietly {
            let path = CGMutablePath()
            path.addRect(bounds)
            if let rect { path.addRect(rect) }
            dim.path = path
            dim.frame = bounds
            edge.path = rect.map { CGPath(rect: $0.insetBy(dx: -0.5, dy: -0.5), transform: nil) }
            guard let rect else {
                badge.isHidden = true
                return
            }
            let scale = window?.backingScaleFactor ?? 2
            badgeText.string = "\(Int(rect.width * scale)) × \(Int(rect.height * scale))"
            let size = CGSize(width: 18 + 7.2 * CGFloat((badgeText.string as? String)?.count ?? 9), height: 22)
            // Below the area's right edge, or inside it when there's no room.
            var origin = CGPoint(x: rect.maxX - size.width, y: rect.minY - size.height - 8)
            if origin.y < 6 { origin.y = rect.minY + 8 }
            origin.x = max(6, min(origin.x, bounds.maxX - size.width - 6))
            badge.frame = CGRect(origin: origin, size: size)
            badgeText.frame = CGRect(x: 0, y: 4, width: size.width, height: 15)
            badge.isHidden = false
        }
    }
}
