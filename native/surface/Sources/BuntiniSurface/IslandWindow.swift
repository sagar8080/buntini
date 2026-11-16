import AppKit
import SwiftUI
import SurfaceKit

/// A borderless panel that never takes focus from the app you're in, above the menu bar.
final class IslandPanel: NSPanel {
    static let size = NSSize(width: 720, height: 560)

    init() {
        super.init(
            contentRect: NSRect(origin: .zero, size: IslandPanel.size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        isFloatingPanel = true
        level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        ignoresMouseEvents = true
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
    // AppKit would push the panel below the menu bar.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// The island at the top of the screen. The panel keeps one size; only the shape inside moves.
final class IslandWindow {
    private let panel = IslandPanel()
    private var screenObserver: NSObjectProtocol?

    init(store: SceneStore, send: @escaping (Action) -> Void) {
        panel.contentView = NSHostingView(rootView: IslandRoot(store: store, send: send))
        place()
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in self?.place() }
    }

    func apply(_ scene: SurfaceScene) {
        let visible = (scene.card != nil && scene.opens) || !scene.pills.isEmpty
        panel.ignoresMouseEvents = !(scene.card != nil && scene.opens)
        if visible {
            place()
            panel.orderFrontRegardless()
        } else {
            panel.orderOut(nil)
        }
    }

    func close() {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        panel.orderOut(nil)
        panel.close()
    }

    /// Top centre of the screen with the notch, or of the main screen.
    private func place() {
        guard let screen = NSScreen.screens.first(where: { $0.safeAreaInsets.top > 0 }) ?? NSScreen.main else { return }
        let size = IslandPanel.size
        panel.setFrame(
            NSRect(x: screen.frame.midX - size.width / 2, y: screen.frame.maxY - size.height, width: size.width, height: size.height),
            display: false
        )
    }
}
