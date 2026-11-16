// Views for timers, captures, vitals and updates (stream F, Phase 3).
import SwiftUI
import SurfaceKit

struct TimerCardView: View {
    let card: Card
    let timer: TimerCard
    let send: (Action) -> Void

    var body: some View {
        CardHeading(from: timer.phase ?? "Timer", title: timer.label)
    }
}

struct CaptureCardView: View {
    let card: Card
    let capture: Capture
    let send: (Action) -> Void

    var body: some View {
        CardHeading(from: "\(capture.width) × \(capture.height)", title: (capture.path as NSString).lastPathComponent)
    }
}

struct VitalsCardView: View {
    let vitals: Vitals

    var body: some View {
        Text(vitals.line).font(.caption)
    }
}

struct UpdateCardView: View {
    let card: Card
    let update: Update
    let send: (Action) -> Void

    var body: some View {
        CardHeading(from: "Buntini \(update.version)", title: update.notes.first ?? "")
    }
}

struct WhatsNewCardView: View {
    let whatsNew: WhatsNew

    var body: some View {
        CardHeading(from: "Buntini \(whatsNew.version)", title: whatsNew.lines.first ?? "")
    }
}
