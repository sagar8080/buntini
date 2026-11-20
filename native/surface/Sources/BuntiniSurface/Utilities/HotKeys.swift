// Buntini's shortcuts you can change, as Carbon hot keys: they need no permission and the
// keys never reach the app in front. A press goes back to Rust as the `shortcut` action.
// Main thread only.
import AppKit
import Carbon.HIToolbox
import SurfaceKit

final class HotKeys {
    static let shared = HotKeys()
    /// "BunU", so presses meant for other handlers (dictation's ⌃⌘V) pass by.
    static let signature: OSType = 0x4275_6E55

    struct Key: Decodable, Equatable {
        let id: String
        let keyCode: UInt32
        /// Carbon's modifier mask.
        let modifiers: UInt32
    }

    private var keys: [Key] = []
    private var refs: [EventHotKeyRef] = []
    private var handler: EventHandlerRef?
    private var paused = false

    /// Replaces every hot key. Returns the ids macOS refused because another app holds them.
    func set(_ keys: [Key]) -> [String] {
        self.keys = keys
        guard !paused else { return [] }
        return register()
    }

    /// While a shortcut is being recorded in Settings, none of Buntini's fire.
    func pause(_ on: Bool) {
        guard on != paused else { return }
        paused = on
        if on { unregister() } else { _ = register() }
    }

    private func register() -> [String] {
        unregister()
        installHandler()
        var refused: [String] = []
        for (index, key) in keys.enumerated() {
            var ref: EventHotKeyRef?
            let id = EventHotKeyID(signature: Self.signature, id: UInt32(index + 1))
            let status = RegisterEventHotKey(key.keyCode, key.modifiers, id, GetEventDispatcherTarget(), 0, &ref)
            if status == noErr, let ref { refs.append(ref) } else { refused.append(key.id) }
        }
        return refused
    }

    private func unregister() {
        refs.forEach { UnregisterEventHotKey($0) }
        refs = []
    }

    private func installHandler() {
        guard handler == nil else { return }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetEventDispatcherTarget(), { _, event, _ in
            guard let event, let id = HotKeys.id(of: event), id.signature == HotKeys.signature else { return OSStatus(eventNotHandledErr) }
            HotKeys.shared.pressed(Int(id.id) - 1)
            return noErr
        }, 1, &spec, nil, &handler)
    }

    /// The hot key a Carbon event is about.
    static func id(of event: EventRef) -> EventHotKeyID? {
        var id = EventHotKeyID()
        let status = GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
        return status == noErr ? id : nil
    }

    private func pressed(_ index: Int) {
        guard keys.indices.contains(index) else { return }
        SurfaceRuntime.shared.send(Action(source: .utilities, card: nil, name: "shortcut", value: .string(keys[index].id)))
    }

    /// Whether macOS has this combination switched on for itself: Spotlight, Mission Control,
    /// input sources, screenshots and the rest of System Settings → Keyboard Shortcuts.
    static func systemUses(keyCode: UInt32, modifiers: UInt32) -> Bool {
        var list: Unmanaged<CFArray>?
        guard CopySymbolicHotKeys(&list) == noErr, let entries = list?.takeRetainedValue() as? [[String: Any]] else { return false }
        let relevant = UInt32(cmdKey | shiftKey | optionKey | controlKey)
        return entries.contains { entry in
            guard (entry["kHISymbolicHotKeyEnabled"] as? Bool) == true,
                  let code = (entry["kHISymbolicHotKeyCode"] as? NSNumber)?.uint32Value,
                  let mask = (entry["kHISymbolicHotKeyModifiers"] as? NSNumber)?.uint32Value else { return false }
            return code == keyCode && mask & relevant == modifiers & relevant
        }
    }
}
