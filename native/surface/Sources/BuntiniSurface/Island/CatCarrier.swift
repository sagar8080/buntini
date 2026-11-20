import AppKit
import SwiftUI
import SurfaceKit

/// The cat while it is carried: a small window of its own that follows the pointer, then
/// flies to where it lands.
final class CatCarrier {
    private static let size = NSSize(width: 64, height: 64)
    private let panel: NSPanel

    init(cat: SurfaceKit.Cat) {
        panel = NSPanel(contentRect: NSRect(origin: .zero, size: CatCarrier.size), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 4)
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.ignoresMouseEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        let held = CatView(cat: cat)
            .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
            .padding(6)
        panel.contentView = NSHostingView(rootView: held)
    }

    func move(to point: CGPoint) {
        panel.setFrameOrigin(NSPoint(x: point.x - CatCarrier.size.width / 2, y: point.y - CatCarrier.size.height / 2))
        if !panel.isVisible { panel.orderFrontRegardless() }
    }

    /// Flies to `point` and goes, handing back to the island.
    func land(at point: CGPoint, done: @escaping () -> Void) {
        let target = NSRect(x: point.x - CatCarrier.size.width / 2, y: point.y - CatCarrier.size.height / 2, width: CatCarrier.size.width, height: CatCarrier.size.height)
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = reduceMotion ? 0.01 : 0.32
            context.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1.0)
            panel.animator().setFrame(target, display: true)
        }, completionHandler: { [panel] in
            done()
            panel.orderOut(nil)
            panel.close()
        })
    }
}
