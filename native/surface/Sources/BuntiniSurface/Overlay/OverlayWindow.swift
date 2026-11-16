// Pictures shown over the desktop, such as a screenshot's thumbnail (stream F).
import AppKit
import SurfaceKit

final class OverlayWindow {
    private let send: (Action) -> Void

    init(send: @escaping (Action) -> Void) {
        self.send = send
    }

    func apply(_ overlay: Overlay?) {}

    func close() {}
}
