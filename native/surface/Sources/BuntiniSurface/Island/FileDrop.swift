import AppKit
import SurfaceKit

/// Receives files dragged onto the island. It never takes clicks: `hitTest` passes them on.
final class FileDropView: NSView {
    var onHover: (Bool) -> Void = { _ in }
    var onDrop: ([String]) -> Void = { _ in }

    override init(frame: NSRect) {
        super.init(frame: frame)
        registerForDraggedTypes([.fileURL])
    }

    required init?(coder: NSCoder) { nil }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard files(sender).isEmpty == false else { return [] }
        onHover(true)
        return .copy
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        files(sender).isEmpty ? [] : .copy
    }

    override func draggingExited(_ sender: NSDraggingInfo?) { onHover(false) }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let paths = files(sender)
        onHover(false)
        guard !paths.isEmpty else { return false }
        onDrop(paths)
        return true
    }

    private func files(_ sender: NSDraggingInfo) -> [String] {
        let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        return urls.prefix(20).map(\.path)
    }
}
