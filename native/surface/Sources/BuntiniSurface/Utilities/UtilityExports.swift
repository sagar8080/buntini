// The C entry points of the utilities (stream F), called from src-tauri/src/utilities/native.rs.
// Kept apart from Exports.swift so the parts of the parity work don't edit the same file.
import AppKit
import CoreGraphics

/// Replaces every hot key with `[{id, keyCode, modifiers}]`. Writes the ids macOS refused into
/// `out` as JSON and returns its length, or -1.
@_cdecl("buntini_hotkeys_set")
public func buntiniHotkeysSet(_ bytes: UnsafePointer<UInt8>?, _ length: Int, _ out: UnsafeMutablePointer<UInt8>?, _ capacity: Int) -> Int {
    guard let bytes, length > 0, length <= 16 << 10, let out, capacity > 0,
          let keys = try? JSONDecoder().decode([HotKeys.Key].self, from: Data(bytes: bytes, count: length)) else { return -1 }
    let refused = onMainSync { HotKeys.shared.set(keys) }
    guard let data = try? JSONEncoder().encode(refused), data.count <= capacity else { return -1 }
    data.copyBytes(to: out, count: data.count)
    return data.count
}

@_cdecl("buntini_hotkeys_pause")
public func buntiniHotkeysPause(_ paused: Bool) {
    onMain { HotKeys.shared.pause(paused) }
}

/// Whether macOS has the combination switched on for itself.
@_cdecl("buntini_hotkeys_system")
public func buntiniHotkeysSystem(_ keyCode: UInt16, _ modifiers: UInt32) -> Bool {
    onMainSync { HotKeys.systemUses(keyCode: UInt32(keyCode), modifiers: modifiers) }
}

/// Whether Buntini may record the screen. `ask` shows macOS's request, which it only ever
/// shows once.
@_cdecl("buntini_capture_allowed")
public func buntiniCaptureAllowed(_ ask: Bool) -> Bool {
    if CGPreflightScreenCaptureAccess() { return true }
    if ask { onMain { _ = CGRequestScreenCaptureAccess() } }
    return false
}

/// The screen with the pointer as four doubles: x, y, width, height from the main screen's
/// top left, in points.
@_cdecl("buntini_capture_screen")
public func buntiniCaptureScreen(_ out: UnsafeMutablePointer<Double>?) -> Bool {
    guard let out, let rect = onMainSync({ Pictures.pointerScreen() }) else { return false }
    out[0] = rect.minX
    out[1] = rect.minY
    out[2] = rect.width
    out[3] = rect.height
    return true
}

@_cdecl("buntini_capture_copy")
public func buntiniCaptureCopy(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let url = fileURL(bytes, length) else { return false }
    return onMainSync { Pictures.copy(url) != nil }
}

/// Blocks for about a second; Rust calls it off the main thread.
@_cdecl("buntini_capture_paste")
public func buntiniCapturePaste(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let url = fileURL(bytes, length), !Thread.isMainThread else { return false }
    return Pictures.paste(url)
}

/// Opens the mark-up editor. Done writes `output` and sends the `annotated` action.
@_cdecl("buntini_capture_annotate")
public func buntiniCaptureAnnotate(_ input: UnsafePointer<UInt8>?, _ inputLength: Int, _ output: UnsafePointer<UInt8>?, _ outputLength: Int) -> Bool {
    guard let source = fileURL(input, inputLength), let target = fileURL(output, outputLength),
          FileManager.default.fileExists(atPath: source.path) else { return false }
    onMain { MarkupEditor.open(source, saveTo: target) }
    return true
}

private func fileURL(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> URL? {
    guard let bytes, length > 0, length <= 4096,
          let path = String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8), path.hasPrefix("/") else { return nil }
    return URL(fileURLWithPath: path)
}
