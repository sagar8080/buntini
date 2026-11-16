import AppKit
import SurfaceKit

/// Voice bands. Kept out of the published scene so 60 updates a second don't re-render views.
public final class Levels: @unchecked Sendable {
    public static let count = 9
    private let lock = NSLock()
    private var values = [Float](repeating: 0, count: Levels.count)

    public func set(_ new: [Float]) {
        lock.lock()
        values = new.map { min(max($0, 0), 1) }
        lock.unlock()
    }

    public func get() -> [Float] {
        lock.lock()
        defer { lock.unlock() }
        return values
    }
}

/// Owns the windows. Used on the main thread only.
public final class SurfaceRuntime {
    public static let shared = SurfaceRuntime()
    /// A scene is a few kilobytes; anything near this is a bug on the Rust side.
    public static let sceneLimit = 4 << 20

    public let levels = Levels()
    private var sink: ((Data) -> Void)?
    private var island: IslandWindow?
    private var flowPill: FlowPillWindow?
    private var overlay: OverlayWindow?
    private let sounds = SoundPlayer()

    public static func decode(_ data: Data) -> SurfaceScene? {
        guard let scene = try? JSONDecoder().decode(SurfaceScene.self, from: data),
              scene.contract == surfaceContract else { return nil }
        return scene
    }

    public func start(onAction: @escaping (Data) -> Void) {
        sink = onAction
        if island == nil {
            island = IslandWindow { [weak self] action in self?.send(action) }
            flowPill = FlowPillWindow(levels: levels) { [weak self] action in self?.send(action) }
            overlay = OverlayWindow { [weak self] action in self?.send(action) }
        }
    }

    public func show(_ scene: SurfaceScene) {
        island?.apply(scene)
        flowPill?.apply(scene.flow)
        overlay?.apply(scene.overlay)
        if let sound = scene.sound { sounds.play(sound.name, sequence: sound.sequence) }
    }

    public func stop() {
        island?.close()
        flowPill?.close()
        overlay?.close()
        island = nil
        flowPill = nil
        overlay = nil
        sink = nil
    }

    func send(_ action: Action) {
        guard let data = try? JSONEncoder().encode(action) else { return }
        sink?(data)
    }
}
