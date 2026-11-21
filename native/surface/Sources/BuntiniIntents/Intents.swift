// Buntini's actions for Shortcuts and Spotlight (App Intents). They live in the app's own binary,
// so the system runs them in Buntini itself, and each is answered in Rust through the one handler
// Rust gives at launch (src-tauri/src/intents.rs). The system reads what they are from
// Metadata.appintents, made from this file by native/intents/metadata.sh: run it after changing
// an intent. Every intent stays in this one file, which Rust's call to `buntini_intents_start`
// links in, intents and all.
import AppIntents
import Foundation

public typealias BuntiniIntentHandler = @convention(c) (UnsafePointer<UInt8>?, Int, UnsafeMutablePointer<UInt8>?, Int) -> Int

private var handler: BuntiniIntentHandler?

/// Rust's handler for every intent: a JSON request in, `{"text": …}` or `{"error": …}` out.
@_cdecl("buntini_intents_start")
public func buntiniIntentsStart(_ given: BuntiniIntentHandler?) {
    handler = given
}

struct BuntiniIntentError: Error, CustomLocalizedStringResourceConvertible {
    let message: String
    var localizedStringResource: LocalizedStringResource { LocalizedStringResource(stringLiteral: message) }
}

enum IntentBridge {
    /// Asks Rust, off the caller's thread, and waits for its answer.
    static func call(_ request: [String: Any]) async throws -> String {
        guard let handler else { throw BuntiniIntentError(message: "Buntini is still starting.") }
        let body = try JSONSerialization.data(withJSONObject: request)
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                var out = [UInt8](repeating: 0, count: 64 * 1024)
                let written = body.withUnsafeBytes { raw in
                    out.withUnsafeMutableBufferPointer { buffer in
                        handler(raw.bindMemory(to: UInt8.self).baseAddress, raw.count, buffer.baseAddress, buffer.count)
                    }
                }
                guard written >= 0, written <= out.count,
                      let reply = try? JSONSerialization.jsonObject(with: Data(out.prefix(written))) as? [String: Any] else {
                    continuation.resume(throwing: BuntiniIntentError(message: "Buntini didn't answer."))
                    return
                }
                if let error = reply["error"] as? String {
                    continuation.resume(throwing: BuntiniIntentError(message: error))
                } else {
                    continuation.resume(returning: reply["text"] as? String ?? "")
                }
            }
        }
    }
}

struct AskBuntini: AppIntent {
    static let title: LocalizedStringResource = "Ask Buntini"
    static let openAppWhenRun = true

    @Parameter(title: "Question")
    var question: String

    func perform() async throws -> some IntentResult {
        _ = try await IntentBridge.call(["intent": "ask", "question": question])
        return .result()
    }
}

struct TodayOnMac: AppIntent {
    static let title: LocalizedStringResource = "Today on the Mac"

    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let text = try await IntentBridge.call(["intent": "today"])
        return .result(value: text, dialog: "\(text)")
    }
}

struct StartFocus: AppIntent {
    static let title: LocalizedStringResource = "Start Focus Timer"

    @Parameter(title: "Minutes", inclusiveRange: (1, 240))
    var minutes: Int?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let text = try await IntentBridge.call(["intent": "focus", "minutes": minutes.map { $0 as Any } ?? NSNull()])
        return .result(dialog: "\(text)")
    }
}

struct KeepMacAwake: AppIntent {
    static let title: LocalizedStringResource = "Keep Mac Awake"

    @Parameter(title: "Minutes", inclusiveRange: (1, 1440))
    var minutes: Int?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let text = try await IntentBridge.call(["intent": "awake", "minutes": minutes.map { $0 as Any } ?? NSNull()])
        return .result(dialog: "\(text)")
    }
}

struct BuntiniShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: AskBuntini(), phrases: ["Ask \(.applicationName)"], shortTitle: "Ask Buntini", systemImageName: "bubble.left")
        AppShortcut(intent: TodayOnMac(), phrases: ["What's on today in \(.applicationName)"], shortTitle: "Today on the Mac", systemImageName: "calendar")
        AppShortcut(intent: StartFocus(), phrases: ["Start a focus timer in \(.applicationName)"], shortTitle: "Start Focus Timer", systemImageName: "timer")
        AppShortcut(intent: KeepMacAwake(), phrases: ["Keep my Mac awake with \(.applicationName)"], shortTitle: "Keep Mac Awake", systemImageName: "cup.and.saucer")
    }
}
