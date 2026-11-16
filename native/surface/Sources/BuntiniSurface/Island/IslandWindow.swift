// The island at the top of the screen. The panel keeps one size and never moves while shown;
// only the shape inside it springs. Clicks pass through everywhere except the island itself,
// decided by watching the pointer (more often the closer it is), so the menu bar and the app
// below stay usable. Design after Coucou's IslandWindowController (MIT).
import AppKit
import SwiftUI
import SurfaceKit

final class IslandWindow {
    let model: IslandModel
    private let panel = IslandPanel()
    private let send: (Action) -> Void
    private var timer: Timer?
    private var interval: Double = 0
    private var leaving: DispatchWorkItem?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var currentScreen: NSScreen?

    init(send: @escaping (Action) -> Void) {
        self.send = send
        let screen = IslandWindow.preferredScreen(pointer: NSEvent.mouseLocation)
        currentScreen = screen
        model = IslandModel(screen: screen.map(ScreenShape.init) ?? ScreenShape(frame: .zero, visibleFrame: .zero, safeTop: 0, auxLeft: nil, auxRight: nil, statusBar: 24))

        let container = NSView(frame: NSRect(origin: .zero, size: IslandGeometryPanel.size))
        let host = NSHostingView(rootView: IslandRoot(model: model, send: send))
        host.frame = container.bounds
        host.autoresizingMask = [.width, .height]
        container.addSubview(host)
        let drop = FileDropView(frame: container.bounds)
        drop.autoresizingMask = [.width, .height]
        drop.onHover = { [weak self] on in self?.model.dragging = on }
        drop.onDrop = { [weak self] paths in
            self?.send(Action(source: .island, card: nil, name: "drop", value: .array(paths.map(JSONValue.string))))
        }
        container.addSubview(drop)
        panel.contentView = container
        place()

        observe(NotificationCenter.default, NSApplication.didChangeScreenParametersNotification) { [weak self] in self?.place() }
        let workspace = NSWorkspace.shared.notificationCenter
        observe(workspace, NSWorkspace.screensDidSleepNotification) { [weak self] in self?.setAsleep(true) }
        observe(workspace, NSWorkspace.screensDidWakeNotification) { [weak self] in self?.setAsleep(false) }
        let distributed = DistributedNotificationCenter.default()
        observe(distributed, Notification.Name("com.apple.screenIsLocked")) { [weak self] in self?.setAsleep(true) }
        observe(distributed, Notification.Name("com.apple.screenIsUnlocked")) { [weak self] in self?.setAsleep(false) }
        schedule(1.0 / 8)
    }

    func apply(_ scene: SurfaceScene) {
        model.scene = scene
        if !panel.isVisible { panel.orderFrontRegardless() }
    }

    func close() {
        timer?.invalidate()
        timer = nil
        for (center, token) in observers { center.removeObserver(token) }
        observers.removeAll()
        panel.orderOut(nil)
        panel.close()
    }

    private func observe(_ center: NotificationCenter, _ name: Notification.Name, _ run: @escaping () -> Void) {
        observers.append((center, center.addObserver(forName: name, object: nil, queue: .main) { _ in run() }))
    }

    private func setAsleep(_ asleep: Bool) {
        model.asleep = asleep
        if asleep {
            timer?.invalidate()
            timer = nil
            interval = 0
        } else {
            schedule(1.0 / 8)
        }
    }

    /// The screen with the notch, or else the one the pointer is on.
    private static func preferredScreen(pointer: CGPoint) -> NSScreen? {
        if #available(macOS 12.0, *), let notched = NSScreen.screens.first(where: { $0.safeAreaInsets.top > 0 }) {
            return notched
        }
        return NSScreen.screens.first(where: { $0.frame.contains(pointer) }) ?? NSScreen.main
    }

    private func place() {
        guard let screen = IslandWindow.preferredScreen(pointer: NSEvent.mouseLocation) else { return }
        currentScreen = screen
        let shape = ScreenShape(screen)
        if shape != model.screen { model.screen = shape }
        panel.setFrame(IslandGeometry.panelFrame(shape), display: false)
    }

    private func schedule(_ seconds: Double) {
        guard seconds != interval || timer == nil else { return }
        interval = seconds
        timer?.invalidate()
        let timer = Timer(timeInterval: seconds, repeats: true) { [weak self] _ in self?.poll() }
        timer.tolerance = seconds * 0.2
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func poll() {
        let pointer = NSEvent.mouseLocation
        let dragging = NSEvent.pressedMouseButtons & 1 != 0
        // Follow the pointer to another screen, but never while the island is open.
        if model.mode != .open, let screen = currentScreen, !screen.frame.contains(pointer),
           IslandWindow.preferredScreen(pointer: pointer) != screen {
            place()
        }
        let frame = panel.frame
        let rect = model.mode == .hidden
            ? IslandGeometry.wakeRect(model.screen, panel: frame)
            : IslandGeometry.islandRect(panel: frame, size: model.size).insetBy(dx: -6, dy: -6)
        let inside = rect.contains(pointer)
        panel.ignoresMouseEvents = !inside
        if inside {
            leaving?.cancel()
            leaving = nil
            if !model.hover { model.hover = true }
        } else if model.hover, leaving == nil {
            // A short grace, so crossing the edge doesn't close it.
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.model.hover = false
                self.model.expanded = false
                self.leaving = nil
            }
            leaving = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: work)
        }
        schedule(IslandGeometry.pollInterval(distance: IslandGeometry.distance(pointer, to: rect), dragging: dragging))
    }
}
