// Renders the island to a picture without a screen, for checking how cards and the cat look
// (SurfaceLab --render). Metal and other AppKit views are left out of such pictures.
import AppKit
import SwiftUI
import SurfaceKit

public enum SurfaceSnapshot {
    /// A PNG of the island showing `scene` on a screen shaped like `screen`. `live` draws the
    /// views as they are on screen, scroll views included, in a window nobody sees, so no
    /// screen recording is needed.
    @MainActor
    public static func png(scene: SurfaceScene, screen: ScreenShape, hover: Bool = false, expanded: Bool = false, scale: CGFloat = 2, live: Bool = false) -> Data? {
        Lexicon.learn(scene)
        let model = IslandModel(screen: screen)
        model.inPicture = !live
        model.scene = scene
        model.hover = hover
        model.expanded = expanded
        let size = model.size
        let height = model.inCorner ? IslandGeometry.panel.height : max(size.height + 50, 120)
        let view = IslandRoot(model: model, send: { _ in })
            .frame(width: IslandGeometry.panel.width, height: height, alignment: .top)
            .clipped()
            .background(Color(white: 0.55))
            .environment(\.inPicture, !live)
        if live {
            let frame = NSRect(x: 0, y: 0, width: IslandGeometry.panel.width, height: height)
            let host = NSHostingView(rootView: view)
            let window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
            window.contentView = host
            host.frame = frame
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.3))
            guard let rep = host.bitmapImageRepForCachingDisplay(in: frame) else { return nil }
            host.cacheDisplay(in: frame, to: rep)
            return rep.representation(using: .png, properties: [:])
        }
        let renderer = ImageRenderer(content: view)
        renderer.scale = scale
        guard let image = renderer.cgImage else { return nil }
        return NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
    }

    /// A 14-inch MacBook Pro screen, notch included, for pictures made without one.
    public static let notchedScreen = ScreenShape(
        frame: CGRect(x: 0, y: 0, width: 1512, height: 982), visibleFrame: CGRect(x: 0, y: 0, width: 1512, height: 944),
        safeTop: 32, auxLeft: 664, auxRight: 664, statusBar: 24)

    /// A screen without a notch.
    public static let plainScreen = ScreenShape(
        frame: CGRect(x: 0, y: 0, width: 1920, height: 1080), visibleFrame: CGRect(x: 0, y: 0, width: 1920, height: 1055),
        safeTop: 0, auxLeft: nil, auxRight: nil, statusBar: 24)
}

/// Frame timing, for SurfaceLab's report.
public enum SurfaceFrames {
    public static func start() {
        FrameClock.reset()
        FrameClock.recording = true
    }

    /// Frames drawn, and the median and 95th-percentile time between them, in milliseconds.
    public static func report() -> (count: Int, median: Double, p95: Double, display: Double) {
        let sorted = FrameClock.intervals.map { $0 * 1000 }.sorted()
        guard !sorted.isEmpty else { return (0, 0, 0, FrameClock.fps) }
        return (sorted.count, sorted[sorted.count / 2], sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.95))], FrameClock.fps)
    }
}
