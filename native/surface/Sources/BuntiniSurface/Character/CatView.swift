// The living cat face: it blinks, follows the pointer and shows its mood, redrawn at the
// display's own rate (120 Hz on ProMotion) and not at all while paused.
import AppKit
import SwiftUI
import SurfaceKit

/// The display's frame rate, read at most every two seconds.
enum FrameClock {
    private static var cached: (fps: Double, at: Date)?

    static var fps: Double {
        if let cached, Date().timeIntervalSince(cached.at) < 2 { return cached.fps }
        let fps = Double(NSScreen.main?.maximumFramesPerSecond ?? 60)
        cached = (max(30, fps), Date())
        return max(30, fps)
    }

    /// A little under one frame: at exactly 1/fps, timing jitter skips every other frame.
    static var interval: Double { 0.9 / fps }

    /// Frame-to-frame times, for SurfaceLab's report. Kept only when switched on.
    static var recording = false
    private(set) static var intervals: [Double] = []

    static func record(_ dt: Double) {
        guard recording else { return }
        intervals.append(dt)
        if intervals.count > 4_000 { intervals.removeFirst(intervals.count - 4_000) }
    }

    static func reset() { intervals.removeAll() }
}

/// Set while the island is drawn into a picture, where AppKit views can't be.
private struct PictureKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var inPicture: Bool {
        get { self[PictureKey.self] }
        set { self[PictureKey.self] = newValue }
    }
}

/// Keeps the view's place on screen, for aiming the eyes at the pointer.
final class ScreenSpot {
    var center: CGPoint?
}

private struct ScreenSpotReader: NSViewRepresentable {
    let spot: ScreenSpot

    final class Probe: NSView {
        var spot: ScreenSpot?

        override func layout() {
            super.layout()
            update()
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            update()
        }

        func update() {
            guard let window else { return }
            let rect = window.convertToScreen(convert(bounds, to: nil))
            spot?.center = CGPoint(x: rect.midX, y: rect.midY)
        }
    }

    func makeNSView(context: Context) -> Probe {
        let probe = Probe()
        probe.spot = spot
        return probe
    }

    func updateNSView(_ probe: Probe, context: Context) {
        probe.spot = spot
        probe.update()
    }
}

/// Blinks, gaze and mood over time.
final class CatLife {
    /// Something on the face is moving, so draw every frame; otherwise only glance at the
    /// pointer now and then.
    private(set) var busy = true
    private var gaze = CGPoint.zero
    private var last: Double?
    private var nextBlink: Double = 0
    private var blinkAt: Double?
    private var doubleBlink = false
    private var lid: Double = 0

    /// Whether a resting cat should wake: the pointer moved, a blink is due, or its mood moves.
    func wants(now: Double, mood: Mood, center: CGPoint?, reduceMotion: Bool) -> Bool {
        if (mood == .dancing || mood == .alert) && !reduceMotion { return true }
        if nextBlink - now < 0.12 { return true }
        guard let center, !reduceMotion else { return false }
        let mouse = NSEvent.mouseLocation
        let target = CGPoint(x: tanh((mouse.x - center.x) / 260), y: -tanh((mouse.y - center.y) / 200))
        return abs(target.x - gaze.x) > 0.004 || abs(target.y - gaze.y) > 0.004
    }

    func pose(now: Double, mood: Mood, center: CGPoint?, reduceMotion: Bool) -> FacePose {
        let dt = min(1.0 / 30, max(0, now - (last ?? now)))
        // Smoothness counts only while moving; at rest the cat is drawn rarely on purpose.
        if let last, busy { FrameClock.record(now - last) }
        last = now

        // Look toward the pointer, smoothed the same at any frame rate.
        var target = CGPoint.zero
        if let center, !reduceMotion {
            let mouse = NSEvent.mouseLocation
            target = CGPoint(x: tanh((mouse.x - center.x) / 260), y: -tanh((mouse.y - center.y) / 200))
        }
        let k = 1 - pow(0.0025, dt)
        let moving = abs(target.x - gaze.x) > 0.004 || abs(target.y - gaze.y) > 0.004
        gaze.x += (target.x - gaze.x) * k
        gaze.y += (target.y - gaze.y) * k

        // Blink every 2.2–5.4 s, sometimes twice.
        if nextBlink == 0 { nextBlink = now + Double.random(in: 1...3) }
        if blinkAt == nil, now >= nextBlink, mood != .sleepy {
            blinkAt = now
            doubleBlink = Double.random(in: 0...1) < 0.22
        }
        var blink: Double = 0
        if let start = blinkAt {
            let length = 0.16, t = now - start
            if t < length {
                blink = sin(.pi * t / length)
            } else if doubleBlink, t < length * 2.4, t > length * 1.4 {
                blink = sin(.pi * (t - length * 1.4) / length)
            } else if t >= (doubleBlink ? length * 2.4 : length) {
                blinkAt = nil
                nextBlink = now + Double.random(in: 2.2...5.4)
            }
        }

        var pose = FacePose()
        pose.gaze = gaze
        let base: Double
        switch mood {
        case .sleepy: base = 0.55
        case .grumpy: base = 0.3
        default: base = 0
        }
        let lidTarget = max(base, blink)
        let lidMoving = abs(lidTarget - lid) > 0.002
        lid += (lidTarget - lid) * min(1, dt * 30)
        pose.lid = lid
        // Busy while the eyes travel, a blink runs or is about to, or the mood moves.
        let blinkSoon = nextBlink - now < 0.12
        busy = moving || lidMoving || blinkAt != nil || blinkSoon || ((mood == .dancing || mood == .alert) && !reduceMotion)
        switch mood {
        case .happy:
            pose.eyes = .happy
            pose.mouth = .smile
        case .dancing:
            pose.eyes = .happy
            pose.mouth = .smile
            pose.tilt = reduceMotion ? 0 : sin(now * 2 * .pi * 1.6) * 7
        case .curious:
            pose.pupil = 1.14
            pose.tilt = 6
        case .alert:
            pose.pupil = 1.14
            pose.ear = reduceMotion ? 0 : max(0, sin(now * 9)) * 6
        case .grumpy:
            pose.mouth = .frown
        case .sleepy, .calm:
            break
        }
        return pose
    }
}

struct CatView: View {
    let cat: SurfaceKit.Cat
    var active = true
    @State private var life = CatLife()
    @State private var spot = ScreenSpot()
    /// Drawing every frame; off while the cat rests, so a still cat costs nothing.
    @State private var awake = true
    /// Ten looks a second at whether to wake, without drawing.
    private let look = Timer.publish(every: 0.1, tolerance: 0.02, on: .main, in: .common).autoconnect()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        if active { living } else { still }
    }

    /// Drawn once, while the island hides or the screen sleeps.
    private var still: some View {
        Canvas { context, size in
            draw(&context, size: size, pose: FacePose())
        }
    }

    private func draw(_ context: inout GraphicsContext, size: CGSize, pose: FacePose) {
        let scale = min(size.width / faceBox.width, size.height / faceBox.height)
        context.translateBy(x: (size.width - faceBox.width * scale) / 2 - faceBox.minX * scale,
                            y: (size.height - faceBox.height * scale) / 2 - faceBox.minY * scale)
        context.scaleBy(x: scale, y: scale)
        CatFace(look: cat).draw(&context, pose: pose)
    }

    private var living: some View {
        TimelineView(.animation(minimumInterval: FrameClock.interval, paused: !awake)) { timeline in
            Canvas { context, size in
                // Reading the date makes SwiftUI redraw on every tick.
                let now = timeline.date.timeIntervalSinceReferenceDate
                let pose = life.pose(now: now, mood: cat.mood, center: spot.center, reduceMotion: reduceMotion)
                draw(&context, size: size, pose: pose)
                if !life.busy {
                    DispatchQueue.main.async { awake = false }
                }
            }
        }
        .background { if !inPicture { ScreenSpotReader(spot: spot) } }
        .onReceive(look) { date in
            if !awake && life.wants(now: date.timeIntervalSinceReferenceDate, mood: cat.mood, center: spot.center, reduceMotion: reduceMotion) {
                awake = true
            }
        }
        .accessibilityElement()
        .accessibilityLabel(cat.character == "orb" ? "Orbit" : "Cat")
    }
}
