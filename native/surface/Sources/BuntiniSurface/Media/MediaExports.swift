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
