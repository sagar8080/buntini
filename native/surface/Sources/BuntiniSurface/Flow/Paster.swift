// Types dictated words where the caret is: through the clipboard and ⌘V, putting back what
// the clipboard held. Main thread only.
import AppKit
import Carbon.HIToolbox

public enum Paster {
    /// Clipboard managers skip items marked this way (nspasteboard.org).
    static let transient = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")
    /// A password manager's copy (nspasteboard.org): never read back or written again.
    static let concealed = NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")
    /// Long enough for the app to read the clipboard after ⌘V.
    static let restoreAfter: TimeInterval = 0.7

    /// What the clipboard held: every item and every type, as data. A concealed item (a
    /// password) is not kept; the clipboard is cleared instead of putting it back.
    public struct Saved {
        let items: [[NSPasteboard.PasteboardType: Data]]
        let concealed: Bool
    }

    public static func save(_ pasteboard: NSPasteboard) -> Saved {
        let items = pasteboard.pasteboardItems ?? []
        if items.contains(where: { $0.types.contains(concealed) }) { return Saved(items: [], concealed: true) }
        return Saved(items: items.map { item in
            var types: [NSPasteboard.PasteboardType: Data] = [:]
            for type in item.types {
                if let data = item.data(forType: type) { types[type] = data }
            }
            return types
        }, concealed: false)
    }

    public static func restore(_ saved: Saved, to pasteboard: NSPasteboard) {
        pasteboard.clearContents()
        let items: [NSPasteboardItem] = saved.items.map { types in
            let item = NSPasteboardItem()
            for (type, data) in types { item.setData(data, forType: type) }
            return item
        }
        if !items.isEmpty { pasteboard.writeObjects(items) }
    }

    /// Puts `text` on the clipboard, marked transient. Returns the clipboard's change count.
    @discardableResult
    public static func place(_ text: String, on pasteboard: NSPasteboard, transient isTransient: Bool) -> Int {
        pasteboard.clearContents()
        let item = NSPasteboardItem()
        item.setString(text, forType: .string)
        if isTransient { item.setData(Data(), forType: transient) }
        pasteboard.writeObjects([item])
        return pasteboard.changeCount
    }

    /// Puts `text` on the clipboard and presses ⌘V. Call on the main thread; returns what
    /// the clipboard held before and the change count that is ours, or nil when ⌘V couldn't
    /// be sent (the clipboard is then already put back).
    static func begin(_ text: String, pasteboard: NSPasteboard = .general) -> (saved: Saved, ours: Int)? {
        let saved = save(pasteboard)
        let ours = place(text, on: pasteboard, transient: true)
        guard postCommandV() else {
            restore(saved, to: pasteboard)
            return nil
        }
        return (saved, ours)
    }

    /// After ⌘V: the clipboard goes back as it was, unless something else used it meanwhile.
    /// When the words weren't seen to arrive, they stay on the clipboard for pasting by hand.
    public static func end(_ text: String, saved: Saved, ours: Int, arrived: Bool, pasteboard: NSPasteboard = .general) {
        guard pasteboard.changeCount == ours else { return }
        if arrived { restore(saved, to: pasteboard) } else { place(text, on: pasteboard, transient: false) }
    }

    /// Whether the words were seen to arrive: the field's fingerprint changed. A field that
    /// says nothing about itself, before or after, is unknown, so not arrived.
    public static func arrived(before: String?, after: String?) -> Bool {
        guard let before, let after else { return false }
        return before != after
    }

    /// The words stay on the clipboard for pasting by hand, so they're not transient.
    public static func copy(_ text: String, pasteboard: NSPasteboard = .general) -> Bool {
        place(text, on: pasteboard, transient: false)
        return true
    }

    /// ⌘V with the V of the keyboard layout in use, and no other modifier the person may
    /// still be holding.
    static func postCommandV() -> Bool {
        guard AXIsProcessTrusted() else { return false }
        let source = CGEventSource(stateID: .combinedSessionState)
        let code = KeyLayout.code(for: "v")
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) else { return false }
        down.flags = .maskCommand
        up.flags = .maskCommand
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        return true
    }
}
