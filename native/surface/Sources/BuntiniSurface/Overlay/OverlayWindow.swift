// What's shown over the desktop (stream F): a screenshot's thumbnail in the corner, or the
// dimmer for choosing an area to capture.
import AppKit
import SurfaceKit

final class OverlayWindow {
    private let thumbnail: ThumbnailOverlay
    private let selection: SelectionOverlay
    /// The dimmer was answered; scenes still asking for it until Rust hears back are ignored.
    private var answered = false

    init(send: @escaping (Action) -> Void) {
        thumbnail = ThumbnailOverlay(send: send)
        selection = SelectionOverlay(send: send)
        selection.onFinish = { [weak self] in self?.answered = true }
    }

    func apply(_ overlay: Overlay?) {
        switch overlay?.kind {
        case "select":
            thumbnail.hide()
            if !selection.active && !answered { selection.show() }
        case "thumbnail":
            answered = false
            selection.hide()
            if let overlay { thumbnail.show(overlay) }
        default:
            answered = false
            selection.hide()
            thumbnail.hide()
        }
    }

    func close() {
        selection.hide()
        thumbnail.hide()
    }
}
