// A cover ready to draw: the picture at the size the card needs, a soft light made from its
// own colours, and the one colour that suits it on the island's black. Made once per file,
// off the main thread, and kept; nothing about it is redrawn while the card shows.
import AppKit
import CoreImage
import ImageIO
import SwiftUI

final class CoverArt {
    let image: CGImage
    /// The cover blurred past its own edges, more vivid than the cover itself.
    let glow: CGImage?
    let tint: RGB

    init(image: CGImage, glow: CGImage?, tint: RGB) {
        self.image = image
        self.glow = glow
        self.tint = tint
    }

    private static let cache: NSCache<NSString, CoverArt> = {
        let cache = NSCache<NSString, CoverArt>()
        cache.countLimit = 12
        return cache
    }()
    private static let queue = DispatchQueue(label: "app.buntini.surface.cover", qos: .userInitiated)
    private static let context = CIContext(options: [.cacheIntermediates: false])

    static func cached(_ path: String) -> CoverArt? {
        cache.object(forKey: path as NSString)
    }

    /// Makes it in the background and answers on the main thread.
    static func load(_ path: String, done: @escaping (CoverArt?) -> Void) {
        if let art = cached(path) { return done(art) }
        queue.async {
            let art = make(path)
            DispatchQueue.main.async {
                if let art { cache.setObject(art, forKey: path as NSString) }
                done(art)
            }
        }
    }

    /// Made on the spot, for pictures drawn without a screen.
    static func now(_ path: String) -> CoverArt? {
        if let art = cached(path) { return art }
        let art = make(path)
        if let art { cache.setObject(art, forKey: path as NSString) }
        return art
    }

    static func make(_ path: String) -> CoverArt? {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else { return nil }
        // Twice the card's size for a Retina screen, decoded once.
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: 240,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return CoverArt(image: image, glow: glow(of: image), tint: tint(of: image))
    }

    /// How far the light reaches past the cover, as a share of the cover's side.
    static let glowReach: CGFloat = 0.32

    private static func glow(of image: CGImage) -> CGImage? {
        let side: CGFloat = 64
        let source = CIImage(cgImage: image)
        let scaled = source.transformed(by: CGAffineTransform(scaleX: side / source.extent.width, y: side / source.extent.height))
        let pad = side * glowReach
        let canvas = CGRect(x: -pad, y: -pad, width: side + 2 * pad, height: side + 2 * pad)
        let light = scaled
            .applyingFilter("CIColorControls", parameters: [kCIInputSaturationKey: 1.6, kCIInputBrightnessKey: 0.04])
            .applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: 9])
            .cropped(to: canvas)
        return context.createCGImage(light, from: canvas)
    }

    /// The most vivid colour that covers a fair part of the cover, made bright enough to read
    /// on black; a soft white for a cover without one.
    static func tint(of image: CGImage) -> RGB {
        let side = 24
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let drawn: Bool = pixels.withUnsafeMutableBytes { raw in
            guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                  let context = CGContext(data: raw.baseAddress, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.interpolationQuality = .medium
            context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
            return true
        }
        let soft = RGB(236, 236, 242)
        guard drawn else { return soft }
        var weights = [Double](repeating: 0, count: 12)
        var sums = [RGB](repeating: RGB(0, 0, 0), count: 12)
        for index in stride(from: 0, to: pixels.count, by: 4) {
            let r = Double(pixels[index]) / 255, g = Double(pixels[index + 1]) / 255, b = Double(pixels[index + 2]) / 255
            let high = max(r, g, b), low = min(r, g, b)
            let saturation = high == 0 ? 0 : (high - low) / high
            guard saturation > 0.2, high > 0.15 else { continue }
            let hue = NSColor(srgbRed: r, green: g, blue: b, alpha: 1).hueComponent
            let bucket = min(11, Int(hue * 12))
            let weight = saturation * saturation * (0.3 + high)
            weights[bucket] += weight
            sums[bucket] = RGB(sums[bucket].r + r * weight, sums[bucket].g + g * weight, sums[bucket].b + b * weight)
        }
        guard let best = weights.indices.max(by: { weights[$0] < weights[$1] }), weights[best] > 1.2 else { return soft }
        let total = weights[best]
        let color = NSColor(srgbRed: sums[best].r / total, green: sums[best].g / total, blue: sums[best].b / total, alpha: 1)
        let lifted = NSColor(hue: color.hueComponent, saturation: min(0.82, max(0.38, color.saturationComponent)), brightness: max(0.86, color.brightnessComponent), alpha: 1)
            .usingColorSpace(.sRGB) ?? color
        return RGB(lifted.redComponent * 255, lifted.greenComponent * 255, lifted.blueComponent * 255)
    }
}

/// The cover with its light, or the player's own colour and a note while there is none.
struct CoverView: View {
    let art: CoverArt?
    let player: String
    let tint: RGB
    let size: CGFloat

    var body: some View {
        ZStack {
            if let glow = art?.glow {
                Image(decorative: glow, scale: 1)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: size * (1 + 2 * CoverArt.glowReach), height: size * (1 + 2 * CoverArt.glowReach))
                    .opacity(0.8)
                    .blendMode(.plusLighter)
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
            cover
                .frame(width: size, height: size)
                .clipShape(RoundedRectangle(cornerRadius: size * 0.12, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: size * 0.12, style: .continuous).strokeBorder(Color.white.opacity(0.12), lineWidth: 0.5))
                .shadow(color: .black.opacity(0.45), radius: 5, y: 2)
        }
        .frame(width: size, height: size)
        .animation(.easeOut(duration: 0.35), value: art.map(ObjectIdentifier.init))
    }

    @ViewBuilder
    private var cover: some View {
        if let art {
            Image(decorative: art.image, scale: 1)
                .resizable()
                .interpolation(.high)
                .aspectRatio(contentMode: .fill)
                .transition(.opacity)
        } else {
            ZStack {
                LinearGradient(colors: [tint.mix(RGB(0, 0, 0), 0.35).color, tint.mix(RGB(0, 0, 0), 0.75).color], startPoint: .topLeading, endPoint: .bottomTrailing)
                Image(systemName: player == "tv" ? "tv" : "music.note")
                    .font(.system(size: size * 0.34, weight: .medium))
                    .foregroundStyle(Color.white.opacity(0.85))
            }
        }
    }
}
