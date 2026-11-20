// What has the caret in the app in front, read the way screen readers do. Rust decides from
// this whether to paste (src-tauri/src/flow/native.rs, `should_paste`).
import AppKit
import ApplicationServices
import Carbon.HIToolbox

enum FocusedField {
    /// Chromium and Electron apps build their accessibility tree only when asked.
    private static var asked: [pid_t: TimeInterval] = [:]
    private static let lock = NSLock()

    static func describe() -> [String: Any] {
        // Something is taking a password (any app's secure field, or a terminal's Secure
        // Keyboard Entry): dictated words are dropped.
        var out: [String: Any] = ["trusted": AXIsProcessTrusted(), "secureInput": IsSecureEventInputEnabled()]
        guard let app = NSWorkspace.shared.frontmostApplication else { return out }
        out["app"] = app.localizedName ?? ""
        out["bundle"] = app.bundleIdentifier ?? ""
        out["pid"] = Int(app.processIdentifier)
        guard AXIsProcessTrusted() else { return out }
        let element = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(element, 0.5)
        var focused = copy(element, kAXFocusedUIElementAttribute)
        if focused == nil, askForTree(app, element) {
            // The tree was just switched on; it takes a moment to appear. Dictation also asks
            // when it starts, so by the time the words are ready the tree is there.
            for _ in 0..<12 where focused == nil {
                Thread.sleep(forTimeInterval: 0.08)
                focused = copy(element, kAXFocusedUIElementAttribute)
            }
        }
        if focused == nil {
            let system = AXUIElementCreateSystemWide()
            AXUIElementSetMessagingTimeout(system, 0.5)
            focused = copy(system, kAXFocusedUIElementAttribute)
        }
        guard let field = focused, CFGetTypeID(field) == AXUIElementGetTypeID() else { return out }
        let target = field as! AXUIElement
        // Which element it is, to tell whether the caret moved to another field meanwhile.
        out["field"] = UInt64(CFHash(target))
        out["role"] = copy(target, kAXRoleAttribute) as? String ?? ""
        out["subrole"] = copy(target, kAXSubroleAttribute) as? String ?? ""
        var settable: DarwinBoolean = false
        AXUIElementIsAttributeSettable(target, kAXValueAttribute as CFString, &settable)
        out["settable"] = settable.boolValue
        out["selection"] = copy(target, kAXSelectedTextRangeAttribute) != nil
        out["secure"] = (out["subrole"] as? String) == kAXSecureTextFieldSubrole
        return out
    }

    /// Whether a password is being typed now: the field in front is secure, or secure input
    /// is on. Checked again right before ⌘V.
    static func secureNow() -> Bool {
        if IsSecureEventInputEnabled() { return true }
        guard AXIsProcessTrusted(), let app = NSWorkspace.shared.frontmostApplication else { return false }
        let element = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(element, 0.25)
        guard let focused = copy(element, kAXFocusedUIElementAttribute), CFGetTypeID(focused) == AXUIElementGetTypeID() else { return false }
        return copy(focused as! AXUIElement, kAXSubroleAttribute) as? String == kAXSecureTextFieldSubrole
    }

    /// What the focused field holds, as a fingerprint that changes when text arrives: its
    /// length, where the caret is, and a hash of its text (in memory only, never kept). Nil
    /// when the field says none of these.
    static func fingerprint() -> String? {
        guard AXIsProcessTrusted(), let app = NSWorkspace.shared.frontmostApplication else { return nil }
        let element = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(element, 0.25)
        guard let focused = copy(element, kAXFocusedUIElementAttribute), CFGetTypeID(focused) == AXUIElementGetTypeID() else { return nil }
        let field = focused as! AXUIElement
        var parts: [String] = []
        let count = (copy(field, kAXNumberOfCharactersAttribute) as? NSNumber)?.intValue
        if let count { parts.append("n\(count)") }
        if let range = copy(field, kAXSelectedTextRangeAttribute), CFGetTypeID(range) == AXValueGetTypeID() {
            var selected = CFRange()
            if AXValueGetValue(range as! AXValue, .cfRange, &selected) { parts.append("c\(selected.location)+\(selected.length)") }
        }
        // A terminal pads its lines, so typing there can leave the length as it was.
        if (count ?? 0) <= 200_000, let value = copy(field, kAXValueAttribute) as? String { parts.append("v\(value.hashValue)") }
        return parts.isEmpty ? nil : parts.joined(separator: " ")
    }

    private static func copy(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
        var value: CFTypeRef?
        return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
    }

    /// Sets AXManualAccessibility for Chromium and Electron apps. True when it was set now.
    private static func askForTree(_ app: NSRunningApplication, _ element: AXUIElement) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        // Chromium turns its tree off again when nobody has asked for a while, so this asks
        // again after a quiet minute.
        let now = ProcessInfo.processInfo.systemUptime
        guard now - (asked[app.processIdentifier] ?? -.infinity) > 60, builtOnChromium(app) else { return false }
        asked[app.processIdentifier] = now
        AXUIElementSetAttributeValue(element, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        // Browsers built on Chromium answer the attribute screen readers set (as ax.rs does).
        if let id = app.bundleIdentifier, browsers.contains(id) {
            AXUIElementSetAttributeValue(element, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
        }
        return true
    }

    private static let browsers: Set<String> = ["com.google.Chrome", "com.google.Chrome.canary", "company.thebrowser.Browser", "com.brave.Browser", "com.microsoft.edgemac", "com.vivaldi.Vivaldi", "com.operasoftware.Opera", "org.chromium.Chromium"]

    private static func builtOnChromium(_ app: NSRunningApplication) -> Bool {
        if let id = app.bundleIdentifier, browsers.contains(id) { return true }
        guard let bundle = app.bundleURL else { return false }
        let frameworks = bundle.appendingPathComponent("Contents/Frameworks")
        let names = (try? FileManager.default.contentsOfDirectory(atPath: frameworks.path)) ?? []
        return names.contains { name in
            name == "Electron Framework.framework" || name.hasSuffix("Chromium Framework.framework") || name == "Google Chrome Framework.framework"
                || name == "Microsoft Edge Framework.framework" || name == "Brave Browser Framework.framework" || name == "Chromium Embedded Framework.framework"
                || name.hasSuffix("Browser Framework.framework") || name == "Vivaldi Framework.framework"
        }
    }
}
