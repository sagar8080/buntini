// The dictation pill at the bottom of the screen (stream C).
import AppKit
import SurfaceKit

final class FlowPillWindow {
    private let levels: Levels
    private let send: (Action) -> Void

    init(levels: Levels, send: @escaping (Action) -> Void) {
        self.levels = levels
        self.send = send
    }

    func apply(_ pill: FlowPill?) {}

    func close() {}
}
