import AppKit
import SwiftUI

/// A borderless panel that never takes focus from the app you're in, above the menu bar.
/// It takes the keyboard only while one of its text fields is being typed in, so a click on
/// a card's button leaves your keystrokes going to your app. It asks to be left out of
/// screenshots and screen sharing, since its cards can show commands and mail; tools that
/// capture through ScreenCaptureKit may still show it on macOS 15.4 and later.
final class IslandPanel: NSPanel {
    /// Someone is typing in the island's own field (set by the window, from the model).
    var typing = false {
        didSet {
            if !typing {
                clickedField = false
                giveKeyBack()
            }
        }
    }
    /// The last click went into a text field, which needs the keyboard to be typed in.
    private var clickedField = false

    init() {
        super.init(contentRect: NSRect(origin: .zero, size: IslandGeometryPanel.size), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        isFloatingPanel = true
        level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        ignoresMouseEvents = true
        sharingType = .none
    }

    override var canBecomeKey: Bool { typing || clickedField }
    override var canBecomeMain: Bool { false }
    // AppKit would push the panel below the menu bar.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }

    override func sendEvent(_ event: NSEvent) {
        if event.type == .leftMouseDown {
            clickedField = IslandPanel.takesTyping(contentView?.hitTest(event.locationInWindow))
        }
        super.sendEvent(event)
        // A click on anything but a field: the keyboard goes back to the app you're in.
        if event.type == .leftMouseUp, !clickedField {
            giveKeyBack()
        }
    }

    /// Hands the keyboard back, unless the island's field is being typed in.
    func giveKeyBack() {
        if isKeyWindow && !typing { resignKey() }
    }

    /// A view that is typed into: an editable text field, or the editor typing into one.
    static func takesTyping(_ view: NSView?) -> Bool {
        var view = view
        while let current = view {
            if let field = current as? NSTextField { return field.isEditable }
            if let text = current as? NSTextView { return text.isEditable }
            view = current.superview
        }
        return false
    }
}

enum IslandGeometryPanel {
    static let size = NSSize(width: 720, height: 560)
}

/// A text field beside an island button, as the island's home row has them.
private enum NSHostingViewForCheck {
    @MainActor
    static func make() -> NSView {
        NSHostingView(rootView: HStack(spacing: 0) {
            TextField("Ask", text: .constant("")).textFieldStyle(.plain).frame(width: 150, height: 80)
            IslandButton(label: "Chat") {}.frame(width: 150, height: 80)
        })
    }
}

/// Checks, for SurfaceChecks, that a click on a SwiftUI text field reaches a view that
/// `IslandPanel` lets take the keyboard, and a click on a button doesn't.
public enum IslandKeyCheck {
    @MainActor
    public static func problem() -> String? {
        let host = NSHostingViewForCheck.make()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 80), styleMask: [.borderless], backing: .buffered, defer: true)
        window.contentView = host
        host.frame = NSRect(x: 0, y: 0, width: 300, height: 80)
        host.layoutSubtreeIfNeeded()
        // The field fills the left half, the button the right (see NSHostingViewForCheck).
        let field = host.hitTest(NSPoint(x: 60, y: 40))
        let button = host.hitTest(NSPoint(x: 240, y: 40))
        if !IslandPanel.takesTyping(field) { return "a click on the island's text field doesn't reach a field (\(field.map { "\(type(of: $0))" } ?? "nothing"))" }
        if IslandPanel.takesTyping(button) { return "a click on an island button would take the keyboard" }
        return nil
    }
}
