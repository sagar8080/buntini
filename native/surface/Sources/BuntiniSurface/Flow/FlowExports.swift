// The C entry points of dictation (stream C), called from src-tauri/src/flow/native.rs. Kept
// apart from Exports.swift so the parts of the parity work don't edit the same file.
import AppKit
import ApplicationServices

public typealias FlowEventCallback = @convention(c) (UnsafePointer<UInt8>?, Int) -> Void

/// What Rust asks the watcher to watch.
struct WatchConfig: Decodable {
    /// Watch the dictation key; otherwise only sleep, lock and Low Power Mode.
    let keys: Bool
    /// `rightOption`, `rightCommand`, `rightControl`, `fn` or `chord`.
    let key: String
    let chord: Chord?

    struct Chord: Decodable {
        let keyCode: UInt16
        let modifiers: [String]
    }
}

/// Starts or reconfigures the watcher. Events go to `callback` as JSON on the main thread.
/// False when the keys were asked for and Accessibility isn't allowed yet; sleep, lock and
/// Low Power Mode are watched either way.
@_cdecl("buntini_flow_watch")
public func buntiniFlowWatch(_ callback: FlowEventCallback?, _ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let callback, let bytes, length > 0, length <= 4096,
          let config = try? JSONDecoder().decode(WatchConfig.self, from: Data(bytes: bytes, count: length)) else { return false }
    let emit: ([String: Any]) -> Void = { payload in
        guard let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
        data.withUnsafeBytes { raw in callback(raw.bindMemory(to: UInt8.self).baseAddress, raw.count) }
    }
    let keys = config.keys && AXIsProcessTrusted()
    onMain {
        FlowWatch.shared.start(config: config, keys: keys, emit: emit)
    }
    return keys || !config.keys
}

@_cdecl("buntini_flow_unwatch")
public func buntiniFlowUnwatch() {
    onMain { FlowWatch.shared.stop() }
}

/// The focused element of the app in front, as JSON into `out`. Returns the length, or -1.
@_cdecl("buntini_flow_focus")
public func buntiniFlowFocus(_ out: UnsafeMutablePointer<UInt8>?, _ capacity: Int) -> Int {
    guard let out, capacity > 0, let data = try? JSONSerialization.data(withJSONObject: FocusedField.describe()), data.count <= capacity else { return -1 }
    data.copyBytes(to: out, count: data.count)
    return data.count
}

/// Pastes with ⌘V, checks the words arrived, and puts the clipboard back. Blocks the calling
/// thread for up to 0.7 s (call it off the main thread). False: the words are on the clipboard
/// to paste by hand.
@_cdecl("buntini_flow_paste")
public func buntiniFlowPaste(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let text = flowText(bytes, length) else { return false }
    let before = FocusedField.fingerprint()
    guard let started = onMainSync({ Paster.begin(text) }) else { return false }
    // An app busy building its accessibility tree (Chromium, after a while unused) can drop the
    // key: wait for the field to change. A field that says nothing about itself is trusted.
    var arrived = before == nil
    let deadline = Date().addingTimeInterval(Paster.restoreAfter)
    while !arrived && Date() < deadline {
        Thread.sleep(forTimeInterval: 0.05)
        if let now = FocusedField.fingerprint(), let before, now != before { arrived = true }
    }
    if arrived {
        let wait = deadline.timeIntervalSinceNow
        if wait > 0 { Thread.sleep(forTimeInterval: wait) }
    }
    onMainSync { Paster.end(text, saved: started.saved, ours: started.ours, arrived: arrived) }
    return arrived
}

/// Puts text on the clipboard to paste by hand.
@_cdecl("buntini_flow_copy")
public func buntiniFlowCopy(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let text = flowText(bytes, length) else { return false }
    return onMainSync { Paster.copy(text) }
}

private func flowText(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> String? {
    guard let bytes, length > 0, length <= 1 << 20 else { return nil }
    return String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8)
}

func onMainSync<T>(_ work: () -> T) -> T {
    Thread.isMainThread ? work() : DispatchQueue.main.sync(execute: work)
}
