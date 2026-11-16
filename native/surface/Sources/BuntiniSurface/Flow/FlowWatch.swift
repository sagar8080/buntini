// Watches the dictation key anywhere, ⌃⌘V, and whether the Mac is asleep, locked or on Low
// Power Mode. Reports only the kind of key (the dictation key, Esc, another key), never which
// key was typed, and nothing at all unless the dictation key was used in the last minutes.
import AppKit
import Carbon.HIToolbox

final class FlowWatch {
    static let shared = FlowWatch()

    private var emit: (([String: Any]) -> Void)?
    private var monitors: [Any] = []
    private var tap: CFMachPort?
    private var tapSource: CFRunLoopSource?
    private var hotKey: EventHotKeyRef?
    private var hotKeyHandler: EventHandlerRef?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var config: WatchConfig?
    /// The dictation key is down.
    private var down = false
    /// Other keys are reported for a while after the dictation key, for Esc and shortcuts.
    private var armedUntil: TimeInterval = 0
    private var asleep = false
    private var locked = false
    /// Device-dependent modifier bits (IOKit's NX_DEVICE…KEYMASK).
    private static let rightOption: UInt = 0x40
    private static let rightCommand: UInt = 0x10
    private static let rightControl: UInt = 0x2000
    /// Hands-free can last ten minutes.
    private static let armedFor: TimeInterval = 11 * 60

    func start(config: WatchConfig, keys: Bool, emit: @escaping ([String: Any]) -> Void) {
        stopKeys()
        self.emit = emit
        self.config = config
        if observers.isEmpty { watchSystem() }
        if keys {
            if config.key == "chord", let chord = config.chord {
                installTap(chord)
            } else {
                installMonitors(config.key)
            }
            installPasteLast()
        }
        reportSystem()
    }

    func stop() {
        stopKeys()
        for (center, observer) in observers { center.removeObserver(observer) }
        observers = []
        emit = nil
    }

    private func stopKeys() {
        monitors.forEach(NSEvent.removeMonitor)
        monitors = []
        if let tap {
            CGEvent.tapEnable(tap: tap, enable: false)
            if let tapSource { CFRunLoopRemoveSource(CFRunLoopGetMain(), tapSource, .commonModes) }
        }
        tap = nil
        tapSource = nil
        if let hotKey { UnregisterEventHotKey(hotKey) }
        hotKey = nil
        if let hotKeyHandler { RemoveEventHandler(hotKeyHandler) }
        hotKeyHandler = nil
        down = false
    }

    private func send(key: String) {
        emit?(["kind": "key", "key": key])
    }

    // MARK: Preset keys

    private func installMonitors(_ key: String) {
        let flags: (NSEvent) -> Void = { [weak self] event in self?.flagsChanged(event, key: key) }
        let keyDown: (NSEvent) -> Void = { [weak self] event in self?.keyDown(event) }
        if let monitor = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged, handler: flags) { monitors.append(monitor) }
        if let monitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown, handler: keyDown) { monitors.append(monitor) }
        // While Buntini itself is in front, global monitors see nothing.
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged, handler: { flags($0); return $0 }) { monitors.append(monitor) }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown, handler: { keyDown($0); return $0 }) { monitors.append(monitor) }
    }

    private func flagsChanged(_ event: NSEvent, key: String) {
        let raw = event.modifierFlags.rawValue
        let isDown: Bool
        switch key {
        case "rightCommand": isDown = raw & Self.rightCommand != 0
        case "rightControl": isDown = raw & Self.rightControl != 0
        case "fn": isDown = event.keyCode == UInt16(kVK_Function) ? event.modifierFlags.contains(.function) : down
        default: isDown = raw & Self.rightOption != 0
        }
        guard isDown != down else { return }
        down = isDown
        if isDown { armedUntil = ProcessInfo.processInfo.systemUptime + Self.armedFor }
        send(key: isDown ? "down" : "up")
    }

    private func keyDown(_ event: NSEvent) {
        guard ProcessInfo.processInfo.systemUptime < armedUntil, !event.isARepeat else { return }
        send(key: event.keyCode == UInt16(kVK_Escape) ? "escape" : "other")
    }

    // MARK: A chord of your own

    private func installTap(_ chord: WatchConfig.Chord) {
        let mask = (1 << CGEventType.keyDown.rawValue) | (1 << CGEventType.keyUp.rawValue) | (1 << CGEventType.flagsChanged.rawValue)
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
            eventsOfInterest: CGEventMask(mask),
            callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                let watch = Unmanaged<FlowWatch>.fromOpaque(context).takeUnretainedValue()
                return watch.tapped(type: type, event: event)
            },
            userInfo: context
        ) else { return }
        self.tap = tap
        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        tapSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    private static func flags(_ names: [String]) -> CGEventFlags {
        var flags: CGEventFlags = []
        for name in names {
            switch name {
            case "command": flags.insert(.maskCommand)
            case "option": flags.insert(.maskAlternate)
            case "control": flags.insert(.maskControl)
            case "shift": flags.insert(.maskShift)
            default: break
            }
        }
        return flags
    }

    /// Runs on the main thread. The chord's own key presses are swallowed so they don't type.
    private func tapped(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            // macOS switched the tap off; keys may have gone up meanwhile.
            if let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            if down { down = false; emit?(["kind": "reset"]) }
            return Unmanaged.passUnretained(event)
        }
        guard let chord = config?.chord else { return Unmanaged.passUnretained(event) }
        let wanted = Self.flags(chord.modifiers)
        let relevant: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl, .maskShift]
        let held = event.flags.intersection(relevant)
        let code = UInt16(event.getIntegerValueField(.keyboardEventKeycode))
        switch type {
        case .keyDown where code == chord.keyCode && held == wanted:
            if !down {
                down = true
                armedUntil = ProcessInfo.processInfo.systemUptime + Self.armedFor
                send(key: "down")
            }
            return nil
        case .keyUp where code == chord.keyCode && down:
            down = false
            send(key: "up")
            return nil
        case .flagsChanged where down && !held.isSuperset(of: wanted):
            down = false
            send(key: "up")
        case .keyDown:
            let repeated = event.getIntegerValueField(.keyboardEventAutorepeat) != 0
            if ProcessInfo.processInfo.systemUptime < armedUntil, !repeated {
                send(key: code == UInt16(kVK_Escape) ? "escape" : "other")
            }
        default: break
        }
        return Unmanaged.passUnretained(event)
    }

    // MARK: ⌃⌘V pastes the last dictation again

    private func installPasteLast() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let context = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetEventDispatcherTarget(), { _, _, context in
            guard let context else { return noErr }
            Unmanaged<FlowWatch>.fromOpaque(context).takeUnretainedValue().emit?(["kind": "pasteLast"])
            return noErr
        }, 1, &spec, context, &hotKeyHandler)
        let id = EventHotKeyID(signature: OSType(0x42_75_6E_46), id: 1) // "BunF"
        RegisterEventHotKey(UInt32(KeyLayout.code(for: "v")), UInt32(cmdKey | controlKey), id, GetEventDispatcherTarget(), 0, &hotKey)
    }

    // MARK: Sleep, lock and Low Power Mode

    private func watchSystem() {
        let workspace = NSWorkspace.shared.notificationCenter
        let distributed = DistributedNotificationCenter.default()
        let watch: (NotificationCenter, Notification.Name, @escaping () -> Void) -> Void = { center, name, change in
            let observer = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                change()
                self?.reportSystem()
            }
            self.observers.append((center, observer))
        }
        watch(workspace, NSWorkspace.willSleepNotification) { [weak self] in self?.asleep = true }
        watch(workspace, NSWorkspace.didWakeNotification) { [weak self] in self?.asleep = false }
        watch(workspace, NSWorkspace.screensDidSleepNotification) { [weak self] in self?.asleep = true }
        watch(workspace, NSWorkspace.screensDidWakeNotification) { [weak self] in self?.asleep = false }
        watch(distributed, Notification.Name("com.apple.screenIsLocked")) { [weak self] in self?.locked = true }
        watch(distributed, Notification.Name("com.apple.screenIsUnlocked")) { [weak self] in self?.locked = false }
        watch(NotificationCenter.default, Notification.Name.NSProcessInfoPowerStateDidChange) {}
    }

    private func reportSystem() {
        emit?(["kind": "system", "asleep": asleep, "locked": locked, "lowPower": ProcessInfo.processInfo.isLowPowerModeEnabled])
    }
}

/// Key codes for the keyboard layout in use, so ⌘V is V on AZERTY and Dvorak too.
enum KeyLayout {
    static func code(for character: String) -> CGKeyCode {
        for source in [TISCopyCurrentKeyboardLayoutInputSource(), TISCopyCurrentASCIICapableKeyboardLayoutInputSource()] {
            guard let source = source?.takeRetainedValue(),
                  let pointer = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else { continue }
            let data = Unmanaged<CFData>.fromOpaque(pointer).takeUnretainedValue() as Data
            let found: CGKeyCode? = data.withUnsafeBytes { raw in
                guard let layout = raw.baseAddress?.assumingMemoryBound(to: UCKeyboardLayout.self) else { return nil }
                for code in 0..<CGKeyCode(128) {
                    var dead: UInt32 = 0
                    var length = 0
                    var chars = [UniChar](repeating: 0, count: 4)
                    let status = UCKeyTranslate(layout, code, UInt16(kUCKeyActionDisplay), 0, UInt32(LMGetKbdType()), OptionBits(kUCKeyTranslateNoDeadKeysBit), &dead, chars.count, &length, &chars)
                    if status == noErr, length > 0, String(utf16CodeUnits: chars, count: length) == character { return code }
                }
                return nil
            }
            if let found { return found }
        }
        return CGKeyCode(kVK_ANSI_V)
    }
}
