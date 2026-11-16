// Pictures and the clipboard: a capture copied for pasting by hand, or pasted where the caret
// is with ⌘V and the clipboard put back after, as dictation does with words (Flow/Paster).
import AppKit
import ImageIO

enum Pictures {
    /// Puts a PNG on the clipboard. TIFF, which older apps ask for, is made only if one does.
    /// Main thread. Returns the clipboard's change count.
    @discardableResult
    static func copy(_ url: URL, pasteboard: NSPasteboard = .general, transient: Bool = false) -> Int? {
        guard let png = try? Data(contentsOf: url), NSImage(data: png) != nil else { return nil }
        pasteboard.clearContents()
        let item = NSPasteboardItem()
        item.setData(png, forType: .png)
        let tiff = TIFFProvider(png: png)
        item.setDataProvider(tiff, forTypes: [.tiff])
        TIFFProvider.current = tiff
        if transient { item.setData(Data(), forType: Paster.transient) }
        pasteboard.writeObjects([item])
        return pasteboard.changeCount
    }

    /// Pastes the picture with ⌘V, waits for the app to take it, and puts the clipboard back.
    /// Blocks the calling thread for about a second; call it off the main thread. False when
    /// ⌘V couldn't be sent (no Accessibility).
    static func paste(_ url: URL) -> Bool {
        let started: (saved: [[NSPasteboard.PasteboardType: Data]], ours: Int)? = onMainSync {
            let pasteboard = NSPasteboard.general
            let saved = Paster.save(pasteboard)
            guard let ours = copy(url, pasteboard: pasteboard, transient: true) else { return nil }
            guard Paster.postCommandV() else {
                Paster.restore(saved, to: pasteboard)
                return nil
            }
            return (saved, ours)
        }
        guard let started else { return false }
        // A picture takes an app longer to read than words.
        Thread.sleep(forTimeInterval: Paster.restoreAfter + 0.3)
        onMainSync {
            let pasteboard = NSPasteboard.general
            if pasteboard.changeCount == started.ours { Paster.restore(started.saved, to: pasteboard) }
        }
        return true
    }

    /// The screen with the pointer, in points from the main screen's top left (screencapture's
    /// `-R`).
    static func pointerScreen() -> CGRect? {
        let pointer = NSEvent.mouseLocation
        guard let main = NSScreen.screens.first,
              let screen = NSScreen.screens.first(where: { NSMouseInRect(pointer, $0.frame, false) }) ?? NSScreen.main else { return nil }
        return topLeft(screen.frame, main: main.frame)
    }

    /// AppKit's coordinates (from the main screen's bottom left) to the window server's.
    static func topLeft(_ rect: CGRect, main: CGRect) -> CGRect {
        CGRect(x: rect.minX, y: main.maxY - rect.maxY, width: rect.width, height: rect.height)
    }

    /// A small copy of a picture for showing, at most `pixels` on its long side.
    static func thumbnail(_ url: URL, pixels: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: pixels,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}

/// Makes TIFF from the PNG only when an app asks for it: a 5K screenshot is 60 MB as TIFF.
final class TIFFProvider: NSObject, NSPasteboardItemDataProvider {
    /// Kept alive while its item is on the clipboard.
    static var current: TIFFProvider?
    private let png: Data

    init(png: Data) {
        self.png = png
    }

    func pasteboard(_ pasteboard: NSPasteboard?, item: NSPasteboardItem, provideDataForType type: NSPasteboard.PasteboardType) {
        guard type == .tiff, let tiff = NSBitmapImageRep(data: png)?.tiffRepresentation else { return }
        item.setData(tiff, forType: .tiff)
    }

    func pasteboardFinishedWithDataProvider(_ pasteboard: NSPasteboard) {
        if TIFFProvider.current === self { TIFFProvider.current = nil }
    }
}

/// Small pictures of captures for the island, read once.
enum CaptureImages {
    private static var cache: [String: NSImage] = [:]
    private static var order: [String] = []

    static func image(_ path: String) -> NSImage? {
        if let image = cache[path] { return image }
        guard FileManager.default.fileExists(atPath: path),
              let cg = Pictures.thumbnail(URL(fileURLWithPath: path), pixels: 480) else { return nil }
        let image = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
        cache[path] = image
        order.append(path)
        if order.count > 6 { cache[order.removeFirst()] = nil }
        return image
    }
}
