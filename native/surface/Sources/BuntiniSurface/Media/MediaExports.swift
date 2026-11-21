// The C entry points of the media part (stream F), called from src-tauri/src/media/native.rs.
// Kept apart from Exports.swift so the parts of the parity work don't edit the same file.
import AppKit

public typealias MediaEventCallback = @convention(c) (UnsafePointer<UInt8>?, Int) -> Void

/// Starts watching the players and the screen. Events go to `callback` as JSON on the main
/// thread: `{"event":"player","player":"spotify"}` and `{"event":"asleep","value":true}`.
@_cdecl("buntini_media_watch")
public func buntiniMediaWatch(_ callback: MediaEventCallback?) -> Bool {
    guard let callback else { return false }
    let emit: ([String: Any]) -> Void = { payload in
        guard let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
        data.withUnsafeBytes { raw in callback(raw.bindMemory(to: UInt8.self).baseAddress, raw.count) }
    }
    onMain { MediaWatch.shared.start(emit: emit) }
    return true
}

/// Notices when a player says its track or state changed, when one opens or quits, and when
/// the screen sleeps or locks. Only that something happened is passed on, never what: the
/// players' own notifications carry the track, and Rust reads it only where it's allowed to.
final class MediaWatch: NSObject {
    static let shared = MediaWatch()

    private static let players: [(id: String, bundle: String, notification: String)] = [
        ("spotify", "com.spotify.client", "com.spotify.client.PlaybackStateChanged"),
        ("music", "com.apple.Music", "com.apple.Music.playerInfo"),
        ("tv", "com.apple.TV", "com.apple.TV.playerInfo"),
    ]

    private var emit: (([String: Any]) -> Void)?
    private var started = false
    /// The display sleeps; the session is locked. Either keeps the island away.
    private var sleeping = false
    private var locked = false

    func start(emit: @escaping ([String: Any]) -> Void) {
        self.emit = emit
        guard !started else { return }
        started = true
        // AppKit may hold distributed notifications back while an app is in the background;
        // these come at once.
        let distributed = DistributedNotificationCenter.default()
        for player in Self.players {
            distributed.addObserver(self, selector: #selector(playerChanged(_:)), name: Notification.Name(player.notification), object: nil, suspensionBehavior: .deliverImmediately)
        }
        distributed.addObserver(self, selector: #selector(sessionLocked), name: Notification.Name("com.apple.screenIsLocked"), object: nil, suspensionBehavior: .deliverImmediately)
        distributed.addObserver(self, selector: #selector(sessionUnlocked), name: Notification.Name("com.apple.screenIsUnlocked"), object: nil, suspensionBehavior: .deliverImmediately)
        let workspace = NSWorkspace.shared.notificationCenter
        workspace.addObserver(self, selector: #selector(appChanged(_:)), name: NSWorkspace.didLaunchApplicationNotification, object: nil)
        workspace.addObserver(self, selector: #selector(appChanged(_:)), name: NSWorkspace.didTerminateApplicationNotification, object: nil)
        workspace.addObserver(self, selector: #selector(displaySlept), name: NSWorkspace.screensDidSleepNotification, object: nil)
        workspace.addObserver(self, selector: #selector(displayWoke), name: NSWorkspace.screensDidWakeNotification, object: nil)
        // Started behind the lock screen: nothing to read until it's unlocked.
        if let session = CGSessionCopyCurrentDictionary() as? [String: Any], session["CGSSessionScreenIsLocked"] as? Bool == true {
            set(sleeping: sleeping, locked: true)
        }
    }

    @objc private func playerChanged(_ note: Notification) {
        guard let player = Self.players.first(where: { $0.notification == note.name.rawValue }) else { return }
        emit?(["event": "player", "player": player.id])
    }

    @objc private func appChanged(_ note: Notification) {
        guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              let player = Self.players.first(where: { $0.bundle == app.bundleIdentifier }) else { return }
        emit?(["event": "player", "player": player.id])
    }

    @objc private func displaySlept() { set(sleeping: true, locked: locked) }
    @objc private func displayWoke() { set(sleeping: false, locked: locked) }
    @objc private func sessionLocked() { set(sleeping: sleeping, locked: true) }
    @objc private func sessionUnlocked() { set(sleeping: sleeping, locked: false) }

    private func set(sleeping: Bool, locked: Bool) {
        let before = self.sleeping || self.locked
        self.sleeping = sleeping
        self.locked = locked
        if before != (sleeping || locked) { emit?(["event": "asleep", "value": sleeping || locked]) }
    }
}

/// Sends play (0), pause (1), next (4), previous (5) or a seek (24) to `position` seconds to
/// whatever macOS says plays, as a media key would, but only while `bundle` is what plays.
/// Waits up to two seconds for the answer: 1 when the player took it, 0 when it didn't, -1
/// when another app plays now. Call off the main thread.
@_cdecl("buntini_media_command")
public func buntiniMediaCommand(_ command: UInt32, _ position: Double, _ bundle: UnsafePointer<CChar>?) -> Int32 {
    guard let bundle else { return 0 }
    let wanted = String(cString: bundle)
    return autoreleasepool { MediaCommand.send(command, position: position, to: wanted) }
}

/// MediaRemote's own request, which macOS lets any app use to send a command and to ask which
/// app plays, though not what it plays: that's read in src-tauri/src/media/system.rs.
enum MediaCommand {
    static let seek: UInt32 = 24
    private static let loaded = dlopen("/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote", RTLD_NOW) != nil

    private final class Answer {
        var taken = false
    }

    static func send(_ command: UInt32, position: Double, to bundle: String) -> Int32 {
        guard loaded, let request = NSClassFromString("MRNowPlayingRequest") as? NSObject.Type else { return 0 }
        guard playing(request).contains(where: { $0.caseInsensitiveCompare(bundle) == .orderedSame }) else { return -1 }
        let selector = NSSelectorFromString("sendCommand:options:queue:completion:")
        let sender = request.init()
        guard sender.responds(to: selector) else { return 0 }
        typealias Send = @convention(c) (AnyObject, Selector, UInt32, NSDictionary, DispatchQueue, @escaping @convention(block) (AnyObject?) -> Void) -> Void
        let send = unsafeBitCast(sender.method(for: selector), to: Send.self)
        let options: NSDictionary = command == seek ? ["kMRMediaRemoteOptionPlaybackPosition": position] : [:]
        let answer = Answer()
        let done = DispatchSemaphore(value: 0)
        send(sender, selector, command, options, DispatchQueue.global(qos: .userInitiated)) { result in
            let result = result as? NSObject
            let failed = (value(result, "sendError") as? NSNumber)?.intValue ?? 0
            answer.taken = result != nil && value(result, "error") == nil && failed == 0
            done.signal()
        }
        guard done.wait(timeout: .now() + 2) == .success else { return 0 }
        return answer.taken ? 1 : 0
    }

    /// The app that plays, and the app it belongs to: Safari plays through WebKit.
    private static func playing(_ request: NSObject.Type) -> [String] {
        typealias Get = @convention(c) (AnyObject, Selector) -> Unmanaged<AnyObject>?
        let selector = NSSelectorFromString("localNowPlayingPlayerPath")
        guard let method = class_getClassMethod(request, selector) else { return [] }
        let path = unsafeBitCast(method_getImplementation(method), to: Get.self)(request, selector)?.takeUnretainedValue() as? NSObject
        let client = value(path, "client") as? NSObject
        return ["bundleIdentifier", "parentApplicationBundleIdentifier"].compactMap { value(client, $0) as? String }
    }

    /// A property of a private object, or nothing if this macOS doesn't have it, rather than
    /// the exception asking for an unknown key would throw.
    private static func value(_ object: NSObject?, _ key: String) -> Any? {
        guard let object, object.responds(to: NSSelectorFromString(key)) else { return nil }
        return object.value(forKey: key)
    }
}
