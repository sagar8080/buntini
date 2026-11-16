// Pictures of the overlays without a screen, for checking how they look. Glass and blur are
// drawn by the window server, so they're left out; the rest is the real layers.
import AppKit

public enum OverlaySnapshot {
    /// The corner thumbnail for the picture at `path`, on a grey backdrop.
    @MainActor
    public static func thumbnail(path: String) -> Data? {
        guard let image = Pictures.thumbnail(URL(fileURLWithPath: path), pixels: 720) else { return nil }
        let panel = ThumbnailPanel(image: image, url: URL(fileURLWithPath: path))
        guard let view = panel.contentView as? ThumbnailView else { return nil }
        // Layers of views in a window that was never shown are built on the first display.
        panel.displayIfNeeded()
        view.layoutSubtreeIfNeeded()
        return png(view.layer, size: view.bounds.size, backdrop: NSColor(white: 0.55, alpha: 1))
    }

    /// The dimmer over a `size` screen, with `area` chosen (from the bottom left, in points).
    @MainActor
    public static func selection(size: CGSize, area: CGRect?) -> Data? {
        let view = SelectionView(frame: CGRect(origin: .zero, size: size))
        view.preview(area)
        return png(view.layer, size: size, backdrop: NSColor(calibratedRed: 0.35, green: 0.5, blue: 0.62, alpha: 1))
    }

    private static func png(_ layer: CALayer?, size: CGSize, backdrop: NSColor, scale: CGFloat = 2) -> Data? {
        guard let layer, let context = CGContext(data: nil, width: Int(size.width * scale), height: Int(size.height * scale), bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        context.scaleBy(x: scale, y: scale)
        context.setFillColor(backdrop.cgColor)
        context.fill(CGRect(origin: .zero, size: size))
        layer.layoutIfNeeded()
        layer.render(in: context)
        guard let image = context.makeImage() else { return nil }
        return NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
    }
}
