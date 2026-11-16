// Views for timers, captures, vitals and updates (stream F).
import SwiftUI
import SurfaceKit

/// The focus timer: a ring counting down, the rounds, and pause, skip and stop.
struct TimerCardView: View {
    let card: Card
    let timer: TimerCard
    let send: (Action) -> Void
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        if let endsAt = timer.endsAt, !timer.paused, !inPicture {
            // Ticks on the deadline's whole seconds, so the digits turn exactly on time.
            TimelineView(.periodic(from: Date(timeIntervalSince1970: Double(endsAt) - timer.total - 2), by: 1)) { context in
                content(left: Double(endsAt) - context.date.timeIntervalSince1970)
            }
        } else {
            content(left: timer.remaining)
        }
    }

    private var done: Bool { timer.phase == "done" }

    private func act(_ name: String) {
        send(Action(source: .utilities, card: card.id, name: name))
    }

    private func content(left: Double) -> some View {
        let colors = FocusRing.colors(timer.phase)
        return HStack(spacing: 14) {
            ZStack {
                FocusRing(left: done ? 1 : left / max(timer.total, 1), colors: colors, dimmed: timer.paused)
                if done {
                    Image(systemName: "checkmark").font(.system(size: 17, weight: .bold))
                } else {
                    Text(clockText(left))
                        .font(.system(size: 14, weight: .semibold, design: .rounded))
                        .monospacedDigit()
                        .opacity(timer.paused ? 0.6 : 1)
                }
            }
            .frame(width: 66, height: 66)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(done ? "\(timer.label) done" : "\(timer.label), \(clockText(left)) left")
            VStack(alignment: .leading, spacing: 9) {
                HStack(spacing: 8) {
                    Text(timer.label).font(.callout.weight(.semibold))
                    if let round = timer.round, let rounds = timer.rounds, rounds > 1 {
                        Rounds(round: round, rounds: rounds, resting: timer.phase != "focus", tint: colors.last ?? .white)
                    }
                }
                HStack(spacing: 6) {
                    if done {
                        IslandButton(label: "Again", prominent: true) { act("focus") }
                    } else {
                        IslandButton(label: timer.paused ? "Resume" : "Pause", prominent: true) { act(timer.paused ? "resume" : "pause") }
                        IslandButton(label: "Skip") { act("skip") }
                        IslandButton(label: "Stop") { act("stop") }
                    }
                }
            }
            Spacer(minLength: 0)
        }
    }
}

/// One mark per focus round: done, under way, to come.
private struct Rounds: View {
    let round: Int
    let rounds: Int
    /// A rest follows `round`, so it's done.
    let resting: Bool
    let tint: Color

    var body: some View {
        HStack(spacing: 4) {
            ForEach(1...rounds, id: \.self) { index in
                Capsule()
                    .fill(index < round || (index == round && resting) ? tint : index == round ? tint.opacity(0.45) : Color.white.opacity(0.16))
                    .frame(width: 12, height: 4)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Round \(round) of \(rounds)")
    }
}

/// An area just captured: paste it where you were typing, copy, mark it up, or ask about it.
struct CaptureCardView: View {
    let card: Card
    let capture: Capture
    let send: (Action) -> Void

    private func act(_ name: String) {
        send(Action(source: .utilities, card: card.id, name: name))
    }

    var body: some View {
        if capture.path.isEmpty {
            HStack(spacing: 10) {
                Image(systemName: "rectangle.dashed.badge.record").font(.title2).opacity(0.9)
                Text("Screen Recording").font(.callout.weight(.semibold))
                Spacer(minLength: 0)
                IslandButton(label: "Allow", prominent: true) { act("allowRecording") }
            }
            .frame(maxHeight: .infinity)
        } else {
            HStack(alignment: .center, spacing: 14) {
                CaptureThumb(path: capture.path, width: capture.width, height: capture.height)
                    .onDrag { NSItemProvider(contentsOf: URL(fileURLWithPath: capture.path)) ?? NSItemProvider() }
                VStack(alignment: .leading, spacing: 9) {
                    HStack {
                        Text(verbatim: "\(String(capture.width)) × \(String(capture.height))").font(.caption2.weight(.semibold)).monospacedDigit().opacity(0.6)
                        Spacer(minLength: 0)
                        Button { act("dismiss") } label: {
                            Image(systemName: "xmark").font(.system(size: 10, weight: .bold)).frame(width: 20, height: 20).background(Circle().fill(Color.white.opacity(0.12)))
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Dismiss")
                    }
                    FlowRow(spacing: 6) {
                        IslandButton(label: "Paste", prominent: true) { act("paste") }
                        IslandButton(label: "Copy") { act("copy") }
                        IslandButton(label: "Mark up") { act("annotate") }
                        IslandButton(label: "Ask") { act("ask") }
                    }
                }
            }
            .frame(maxHeight: .infinity)
        }
    }
}

/// A capture's picture on the island, kept to its shape.
struct CaptureThumb: View {
    let path: String
    let width: Int
    let height: Int

    var body: some View {
        let aspect = CGFloat(max(width, 1)) / CGFloat(max(height, 1))
        let size = aspect >= 140.0 / 96.0 ? CGSize(width: 140, height: max(36, 140 / aspect)) : CGSize(width: max(36, 96 * aspect), height: 96)
        Group {
            if let image = CaptureImages.image(path) {
                Image(nsImage: image).resizable().interpolation(.high).aspectRatio(contentMode: .fill)
            } else {
                LinearGradient(colors: [Color(white: 0.32), Color(white: 0.18)], startPoint: .topLeading, endPoint: .bottomTrailing)
                    .overlay(Image(systemName: "photo").font(.title3).opacity(0.5))
            }
        }
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).stroke(Color.white.opacity(0.2), lineWidth: 0.5))
        .shadow(color: .black.opacity(0.45), radius: 8, y: 3)
        .accessibilityLabel("Capture")
    }
}

/// One line: how the Mac is doing, or a word such as "Sounds off".
struct VitalsCardView: View {
    let vitals: Vitals

    private var symbol: String {
        switch vitals.line {
        case "Sounds off": return "speaker.slash.fill"
        case "Sounds on": return "speaker.wave.2.fill"
        case "Nothing waiting": return "checkmark.circle"
        default: return "gauge.with.dots.needle.67percent"
        }
    }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).font(.callout).opacity(0.85)
            Text(vitals.line).font(.callout.weight(.semibold)).lineLimit(1)
        }
        .frame(maxHeight: .infinity)
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

/// On the island's home: start focusing, unless a focus timer already runs.
struct UtilityHomeButtons: View {
    let pills: [Pill]
    let send: (Action) -> Void

    var body: some View {
        if !pills.contains(where: { $0.source == .utilities && $0.id == "utilities.focus" }) {
            IslandButton(label: "Focus") { send(Action(source: .utilities, card: nil, name: "focus")) }
        }
    }
}
