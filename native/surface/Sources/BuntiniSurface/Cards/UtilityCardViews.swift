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
            .accessibilityLabel(done ? L(.timerDone, ["label": timer.label]) : L(.timerLeft, ["label": timer.label, "time": clockText(left)]))
            VStack(alignment: .leading, spacing: 9) {
                HStack(spacing: 8) {
                    Text(timer.label).font(.callout.weight(.semibold))
                    if let round = timer.round, let rounds = timer.rounds, rounds > 1 {
                        Rounds(round: round, rounds: rounds, resting: timer.phase != "focus", tint: colors.last ?? .white)
                    }
                }
                HStack(spacing: 6) {
                    if done {
                        IslandButton(label: L(.timerAgain), prominent: true) { act("focus") }
                    } else {
                        IslandButton(label: timer.paused ? L(.resume) : L(.pause), prominent: true) { act(timer.paused ? "resume" : "pause") }
                        IslandButton(label: L(.skip)) { act("skip") }
                        IslandButton(label: L(.stop)) { act("stop") }
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
        .accessibilityLabel(L(.timerRound, ["round": String(round), "rounds": String(rounds)]))
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
                Text(L(.screenRecording)).font(.callout.weight(.semibold))
                Spacer(minLength: 0)
                IslandButton(label: L(.allow), prominent: true) { act("allowRecording") }
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
                        .accessibilityLabel(L(.dismiss))
                    }
                    FlowRow(spacing: 6) {
                        IslandButton(label: L(.paste), prominent: true) { act("paste") }
                        IslandButton(label: L(.copy)) { act("copy") }
                        IslandButton(label: L(.markUp)) { act("annotate") }
                        IslandButton(label: L(.ask)) { act("ask") }
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
        .accessibilityLabel(L(.capture))
    }
}

/// One line: how the Mac is doing, or a word such as "Sounds off" (Rust writes it from the
/// same words).
struct VitalsCardView: View {
    let vitals: Vitals

    private var symbol: String {
        switch vitals.line {
        case L(.soundsOff): return "speaker.slash.fill"
        case L(.soundsOn): return "speaker.wave.2.fill"
        case L(.nothingWaiting): return "checkmark.circle"
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

/// Buntini's own update: asked once, then found, downloaded, checked and ready to restart.
struct UpdateCardView: View {
    let card: Card
    let update: Update
    let send: (Action) -> Void

    private var stage: String { update.stage ?? "available" }

    private func act(_ name: String) {
        send(Action(source: card.source, card: card.id, name: name))
    }

    private var title: String {
        switch stage {
        case "ask": return L(.updateAsk)
        case "current": return L(.updateCurrent)
        case "downloading": return L(.updateDownloading)
        case "checking": return L(.updateChecking)
        case "ready": return L(.updateReady)
        case "failed": return L(.updateFailed)
        default: return L(.updateAvailable)
        }
    }

    var body: some View {
        HStack(spacing: 14) {
            UpdateBadge(stage: stage, progress: update.progress ?? 0)
                .frame(width: 58, height: 58)
            VStack(alignment: .leading, spacing: 7) {
                CardHeading(from: stage == "ask" ? "Buntini" : "Buntini \(update.version)", title: title)
                if stage == "downloading" {
                    Text("\(Int(((update.progress ?? 0) * 100).rounded()))%")
                        .font(.caption.weight(.medium)).monospacedDigit().opacity(0.6)
                } else if !update.notes.isEmpty, stage != "ask" {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(update.notes.prefix(stage == "failed" ? 1 : 2).enumerated()), id: \.offset) { _, note in
                            Text(stage == "failed" ? note : "· \(note)").font(.caption).opacity(0.7).lineLimit(stage == "failed" ? 2 : 1).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                HStack(spacing: 6) {
                    switch stage {
                    case "ask":
                        IslandButton(label: L(.checkDaily), prominent: true) { act("allow") }
                        IslandButton(label: L(.notNow)) { act("notNow") }
                    case "available":
                        IslandButton(label: L(.install), prominent: true) { act("install") }
                        IslandButton(label: L(.releaseNotes)) { act("open") }
                        IslandButton(label: L(.later)) { act("later") }
                    case "ready":
                        IslandButton(label: L(.restart), prominent: true) { act("restart") }
                        IslandButton(label: L(.later)) { act("later") }
                    case "failed":
                        IslandButton(label: L(.tryAgain), prominent: true) { act("install") }
                        IslandButton(label: L(.download)) { act("open") }
                        IslandButton(label: L(.close)) { act("later") }
                    default:
                        EmptyView()
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .contain)
    }
}

/// A ring that fills as the download runs, around a sign for the stage.
private struct UpdateBadge: View {
    let stage: String
    let progress: Double
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.inPicture) private var inPicture

    private var tint: [Color] {
        switch stage {
        case "failed": return [Color(red: 1, green: 0.45, blue: 0.4), Color(red: 1, green: 0.62, blue: 0.3)]
        case "current", "ready": return [Color(red: 0.36, green: 0.86, blue: 0.6), Color(red: 0.3, green: 0.75, blue: 0.95)]
        default: return [Color(red: 0.46, green: 0.62, blue: 1), Color(red: 0.72, green: 0.5, blue: 1)]
        }
    }

    private var symbol: String {
        switch stage {
        case "ask": return "arrow.triangle.2.circlepath"
        case "current", "ready": return "checkmark"
        case "failed": return "exclamationmark"
        case "checking": return "checkmark.shield"
        default: return "arrow.down"
        }
    }

    private var filled: Double {
        switch stage {
        case "downloading": return max(0.02, min(1, progress))
        case "checking", "current", "ready": return 1
        default: return 0
        }
    }

    var body: some View {
        ZStack {
            Circle().stroke(Color.white.opacity(0.12), lineWidth: 5)
            Circle()
                .trim(from: 0, to: filled)
                .stroke(AngularGradient(colors: tint + [tint[0]], center: .center), style: StrokeStyle(lineWidth: 5, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .shadow(color: tint[0].opacity(0.55), radius: 5)
                .animation(reduceMotion || inPicture ? nil : .easeOut(duration: 0.35), value: filled)
            Circle()
                .fill(RadialGradient(colors: [tint[0].opacity(0.35), .clear], center: .center, startRadius: 2, endRadius: 26))
                .padding(6)
            Image(systemName: symbol)
                .font(.system(size: 18, weight: .bold))
                .foregroundStyle(LinearGradient(colors: tint, startPoint: .topLeading, endPoint: .bottomTrailing))
        }
        .accessibilityHidden(true)
    }
}

/// After an update: the new version and what changed.
struct WhatsNewCardView: View {
    let whatsNew: WhatsNew

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: "Buntini \(whatsNew.version)", title: L(.whatsNew))
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(whatsNew.lines.prefix(4).enumerated()), id: \.offset) { _, line in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: "sparkle").font(.system(size: 9, weight: .bold)).foregroundStyle(Color(red: 0.72, green: 0.5, blue: 1))
                        Text(line).font(.caption).lineLimit(1).opacity(0.85)
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// On the island's home: start focusing, unless a focus timer already runs.
struct UtilityHomeButtons: View {
    let pills: [Pill]
    let send: (Action) -> Void

    var body: some View {
        if !pills.contains(where: { $0.source == .utilities && $0.id == "utilities.focus" }) {
            IslandButton(label: L(.focus)) { send(Action(source: .utilities, card: nil, name: "focus")) }
        }
    }
}
