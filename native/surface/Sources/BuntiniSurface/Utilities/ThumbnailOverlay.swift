// A screenshot's thumbnail in the bottom-right corner, as macOS shows its own: it springs in,
// leaves after a few seconds unless the pointer rests on it, opens on a click, goes away with
// a swipe to the right, and drags out as the file into Finder, Mail or a chat. Core Animation
// moves it; nothing runs while it sits still.
import AppKit
import QuartzCore
import SurfaceKit

final class ThumbnailOverlay {
    private let send: (Action) -> Void
    private var panel: ThumbnailPanel?
    private var path: String?

    init(send: @escaping (Action) -> Void) {
        self.send = send
    }

    func show(_ overlay: Overlay) {
        guard overlay.path != path else { return }
        panel?.leave(animated: false)
        panel = nil
        path = overlay.path
        let url = URL(fileURLWithPath: overlay.path)
        guard let image = Pictures.thumbnail(url, pixels: 720) else {
            gone()
            return
        }
        let panel = ThumbnailPanel(image: image, url: url)
        panel.onGone = { [weak self, weak panel] in
            guard let self, let panel, self.panel === panel else { return }
            self.panel = nil
            self.gone()
        }
        self.panel = panel
        panel.arrive()
    }

    /// Rust took it away.
    func hide() {
        path = nil
        panel?.leave(animated: true)
        panel = nil
    }

    private func gone() {
        path = nil
        send(Action(source: .utilities, card: nil, name: "thumbnailGone"))
    }
}

final class ThumbnailPanel: NSPanel {
    /// Room around the picture for its shadow.
    static let margin: CGFloat = 28
    /// The picture's longest sides.
    static let most = CGSize(width: 236, height: 152)
    /// The glass edge around the picture.
    static let bezel: CGFloat = 5

    var onGone: (() -> Void)?
    private let thumbnail: ThumbnailView
    private var timer: Timer?
    private var leaving = false

    init(image: CGImage, url: URL) {
        let aspect = CGFloat(image.width) / max(1, CGFloat(image.height))
        var size = aspect >= Self.most.width / Self.most.height
            ? CGSize(width: Self.most.width, height: (Self.most.width / aspect).rounded())
            : CGSize(width: (Self.most.height * aspect).rounded(), height: Self.most.height)
        size.width += 2 * Self.bezel
        size.height += 2 * Self.bezel
        let frame = CGSize(width: size.width + 2 * Self.margin, height: size.height + 2 * Self.margin)
        thumbnail = ThumbnailView(frame: NSRect(origin: .zero, size: frame), card: size, image: image, url: url)
        super.init(contentRect: NSRect(origin: .zero, size: frame), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .statusBar
        isFloatingPanel = true
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        isMovable = false
        sharingType = .none
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        contentView = thumbnail
        thumbnail.panel = self
        place()
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// The bottom-right corner of the screen with the pointer.
    private func place() {
        let pointer = NSEvent.mouseLocation
        guard let screen = NSScreen.screens.first(where: { NSMouseInRect(pointer, $0.frame, false) }) ?? NSScreen.main else { return }
        let visible = screen.visibleFrame
        setFrameOrigin(NSPoint(x: visible.maxX - frame.width + Self.margin - 18, y: visible.minY - Self.margin + 18))
    }

    func arrive() {
        orderFrontRegardless()
        thumbnail.slideIn()
        wait(5)
    }

    /// Leaves after `seconds`, unless the pointer comes to rest on it.
    func wait(_ seconds: TimeInterval) {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
            self?.leave(animated: true)
            self?.onGone?()
        }
    }

    func hold() {
        timer?.invalidate()
        timer = nil
    }

    func leave(animated: Bool) {
        guard !leaving else { return }
        leaving = true
        hold()
        guard animated else {
            orderOut(nil)
            return
        }
        thumbnail.slideOut { [weak self] in self?.orderOut(nil) }
    }

    /// Opened, dragged out or swiped away.
    func done() {
        leave(animated: false)
        onGone?()
    }
}

final class ThumbnailView: NSView, NSDraggingSource {
    weak var panel: ThumbnailPanel?
    private let card: NSView
    private let picture = NSView()
    private let image: CGImage
    private let url: URL
    private enum Gesture { case none, undecided, swiping, dragging }
    private var gesture = Gesture.none
    private var downAt: CGPoint = .zero
    private var swiped: CGFloat = 0

    init(frame: NSRect, card size: CGSize, image: CGImage, url: URL) {
        self.image = image
        self.url = url
        card = NSView(frame: NSRect(x: ThumbnailPanel.margin, y: ThumbnailPanel.margin, width: size.width, height: size.height))
        super.init(frame: frame)
        wantsLayer = true
        card.wantsLayer = true
        card.layerContentsRedrawPolicy = .never
        let radius: CGFloat = 14
        if let shadow = card.layer {
            shadow.shadowColor = NSColor.black.cgColor
            shadow.shadowOpacity = 0.32
            shadow.shadowRadius = 16
            shadow.shadowOffset = CGSize(width: 0, height: -6)
            shadow.shadowPath = CGPath(roundedRect: card.bounds, cornerWidth: radius, cornerHeight: radius, transform: nil)
        }

        let bezel = Self.glass(frame: card.bounds, radius: radius)
        picture.frame = card.bounds.insetBy(dx: ThumbnailPanel.bezel, dy: ThumbnailPanel.bezel)
        picture.wantsLayer = true
        if let layer = picture.layer {
            layer.contents = image
            layer.contentsGravity = .resizeAspectFill
            layer.cornerRadius = radius - ThumbnailPanel.bezel
            layer.cornerCurve = .continuous
            layer.masksToBounds = true
            layer.borderWidth = 0.5
            layer.borderColor = NSColor.white.withAlphaComponent(0.22).cgColor
        }
        bezel.addSubview(picture)
        card.addSubview(bezel)
        addSubview(card)
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        setAccessibilityLabel(url.deletingPathExtension().lastPathComponent)
    }

    required init?(coder: NSCoder) { nil }

    /// Liquid Glass on macOS 26; a frosted edge before it.
    private static func glass(frame: CGRect, radius: CGFloat) -> NSView {
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView(frame: frame)
            glass.cornerRadius = radius
            return glass
        }
        let blur = NSVisualEffectView(frame: frame)
        blur.material = .hudWindow
        blur.blendingMode = .behindWindow
        blur.state = .active
        blur.wantsLayer = true
        blur.layer?.cornerRadius = radius
        blur.layer?.cornerCurve = .continuous
        blur.layer?.masksToBounds = true
        return blur
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func hitTest(_ point: NSPoint) -> NSView? {
        card.frame.contains(convert(point, from: superview)) ? self : nil
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: card.frame, options: [.activeAlways, .mouseEnteredAndExited], owner: self))
    }

    // MARK: Motion

    private func spring(_ keyPath: String, from: CGFloat, to: CGFloat, damping: CGFloat = 17, stiffness: CGFloat = 210) {
        guard let layer = card.layer else { return }
        let animation = CASpringAnimation(keyPath: keyPath)
        animation.fromValue = from
        animation.toValue = to
        animation.damping = damping
        animation.stiffness = stiffness
        animation.mass = 1
        animation.duration = animation.settlingDuration
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.setValue(to, forKeyPath: keyPath)
        CATransaction.commit()
        layer.add(animation, forKey: keyPath)
    }

    private func set(_ keyPath: String, _ value: CGFloat) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        card.layer?.setValue(value, forKeyPath: keyPath)
        CATransaction.commit()
    }

    func slideIn() {
        spring("transform.translation.x", from: card.frame.width + ThumbnailPanel.margin + 24, to: 0)
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0
        fade.toValue = 1
        fade.duration = 0.18
        card.layer?.add(fade, forKey: "fade")
    }

    func slideOut(then: @escaping () -> Void) {
        guard let layer = card.layer else { return then() }
        let now = layer.presentation()?.value(forKeyPath: "transform.translation.x") as? CGFloat ?? 0
        CATransaction.begin()
        CATransaction.setCompletionBlock(then)
        let move = CABasicAnimation(keyPath: "transform.translation.x")
        move.fromValue = now
        move.toValue = card.frame.width + ThumbnailPanel.margin + 24
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 1
        fade.toValue = 0
        let group = CAAnimationGroup()
        group.animations = [move, fade]
        group.duration = 0.26
        group.timingFunction = CAMediaTimingFunction(controlPoints: 0.4, 0, 0.9, 0.6)
        group.fillMode = .forwards
        group.isRemovedOnCompletion = false
        layer.add(group, forKey: "leave")
        CATransaction.commit()
    }

    override func mouseEntered(with event: NSEvent) {
        panel?.hold()
        spring("transform.scale", from: card.layer?.presentation()?.value(forKeyPath: "transform.scale") as? CGFloat ?? 1, to: 1.03, damping: 20, stiffness: 300)
    }

    override func mouseExited(with event: NSEvent) {
        guard gesture == .none || gesture == .undecided else { return }
        panel?.wait(3)
        spring("transform.scale", from: card.layer?.presentation()?.value(forKeyPath: "transform.scale") as? CGFloat ?? 1.03, to: 1, damping: 20, stiffness: 300)
    }

    // MARK: Click, swipe, drag

    override func mouseDown(with event: NSEvent) {
        gesture = .undecided
        downAt = event.locationInWindow
        panel?.hold()
    }

    override func mouseDragged(with event: NSEvent) {
        let dx = event.locationInWindow.x - downAt.x
        let dy = event.locationInWindow.y - downAt.y
        switch gesture {
        case .undecided where dx > 6 && abs(dy) < dx:
            gesture = .swiping
        case .undecided where hypot(dx, dy) > 4:
            gesture = .dragging
            beginFileDrag(event)
        case .swiping:
            // Follows the pointer to the right, and resists to the left.
            set("transform.translation.x", dx > 0 ? dx : dx * 0.2)
        default:
            break
        }
    }

    override func mouseUp(with event: NSEvent) {
        defer { if gesture != .dragging { gesture = .none } }
        switch gesture {
        case .undecided:
            NSWorkspace.shared.open(url)
            panel?.done()
        case .swiping:
            settle(event.locationInWindow.x - downAt.x)
        default:
            break
        }
    }

    override func scrollWheel(with event: NSEvent) {
        guard event.hasPreciseScrollingDeltas, event.momentumPhase.isEmpty else { return }
        if event.phase.contains(.began) {
            swiped = 0
            panel?.hold()
        }
        if event.phase.contains(.began) || event.phase.contains(.changed) {
            swiped += event.isDirectionInvertedFromDevice ? event.scrollingDeltaX : -event.scrollingDeltaX
            set("transform.translation.x", swiped > 0 ? swiped : swiped * 0.2)
        }
        if event.phase.contains(.ended) || event.phase.contains(.cancelled) {
            settle(swiped)
        }
    }

    /// Far enough right, it goes; otherwise it springs back.
    private func settle(_ distance: CGFloat) {
        if distance > 56 {
            panel?.leave(animated: true)
            panel?.onGone?()
        } else {
            spring("transform.translation.x", from: distance > 0 ? distance : distance * 0.2, to: 0, damping: 22, stiffness: 320)
            panel?.wait(3)
        }
    }

    private func beginFileDrag(_ event: NSEvent) {
        let item = NSDraggingItem(pasteboardWriter: url as NSURL)
        let preview = NSImage(cgImage: image, size: picture.bounds.size)
        item.setDraggingFrame(convert(picture.frame, from: card), contents: preview)
        let session = beginDraggingSession(with: [item], event: event, source: self)
        session.animatesToStartingPositionsOnCancelOrFail = true
    }

    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        context == .outsideApplication ? .copy : []
    }

    func draggingSession(_ session: NSDraggingSession, willBeginAt screenPoint: NSPoint) {
        window?.alphaValue = 0
    }

    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        gesture = .none
        if operation.isEmpty {
            window?.alphaValue = 1
            panel?.wait(3)
        } else {
            panel?.done()
        }
    }
}
