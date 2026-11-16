// The dictation pill at the bottom of the screen (stream C): above the Dock, at status-bar
// level, never taking focus from the app the words go to.
import AppKit
import SwiftUI
import SurfaceKit

/// What the pill view watches.
final class PillModel: ObservableObject {
    @Published var pill: FlowPill?
    /// The panel is on screen; drawing stops when it isn't.
    @Published var visible = false
}

final class PillPanel: NSPanel {
    static let size = NSSize(width: 360, height: 96)

    init() {
        super.init(contentRect: NSRect(origin: .zero, size: PillPanel.size), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        isFloatingPanel = true
        level = .statusBar
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        // Clicks on its clear parts reach the window below.
        ignoresMouseEvents = false
        acceptsMouseMovedEvents = true
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class FlowPillWindow {
    private let levels: Levels
    private let send: (Action) -> Void
    private let model = PillModel()
    private var panel: PillPanel?
    private var screenObserver: NSObjectProtocol?

    init(levels: Levels, send: @escaping (Action) -> Void) {
        self.levels = levels
        self.send = send
    }

    func apply(_ pill: FlowPill?) {
        model.pill = pill
        guard pill != nil else {
            panel?.orderOut(nil)
            model.visible = false
            return
        }
        let panel = self.panel ?? makePanel()
        place(panel)
        if !panel.isVisible { panel.orderFrontRegardless() }
        model.visible = true
    }

    func close() {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        screenObserver = nil
        model.visible = false
        panel?.orderOut(nil)
        panel?.close()
        panel = nil
    }

    private func makePanel() -> PillPanel {
        let panel = PillPanel()
        panel.contentView = NSHostingView(rootView: PillView(model: model, levels: levels, send: send))
        self.panel = panel
        screenObserver = NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self, weak panel] _ in
            if let panel { self?.place(panel) }
        }
        return panel
    }

    /// Bottom centre of the screen with the pointer, just above the Dock.
    private func place(_ panel: PillPanel) {
        let pointer = NSEvent.mouseLocation
        guard let screen = NSScreen.screens.first(where: { NSMouseInRect(pointer, $0.frame, false) }) ?? NSScreen.main else { return }
        let size = PillPanel.size
        let frame = NSRect(x: screen.visibleFrame.midX - size.width / 2, y: screen.visibleFrame.minY + 6, width: size.width, height: size.height)
        if panel.frame != frame { panel.setFrame(frame, display: false) }
    }
}
