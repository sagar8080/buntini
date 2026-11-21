// What the island shows at a glance: beside the notch, the most pressing thing running; when
// opened, a chip for each thing running. Anything that moves here is a Core Animation layer,
// so it runs in the window server and costs the app nothing between changes.
import AppKit
import QuartzCore
import SwiftUI
import SurfaceKit

private let waitingTint = "#F2B33D"
private let errorTint = "#FF6B6B"
private let doneTint = "#4FD1A5"

/// A pill's mark: what it is and how it's doing, in one small sign.
struct PillMark: View {
    let pill: Pill?
    let tint: String
    var size: CGFloat = 12
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        mark
            .font(.system(size: size, weight: .semibold))
            .frame(width: size + 2, height: size + 2)
            .accessibilityHidden(true)
    }

    private var moving: Bool { !reduceMotion && !inPicture }

    @ViewBuilder private var mark: some View {
        let color = Color(hex: tint)
        switch (pill?.source, pill?.state) {
        case (.flow?, _):
            Image(systemName: "record.circle").foregroundStyle(Color(hex: errorTint))
        case (_, .waiting?):
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Color(hex: waitingTint))
        case (_, .error?):
            Image(systemName: "xmark.circle.fill").foregroundStyle(Color(hex: errorTint))
        case (.agents?, .working?):
            if moving { Spinner(tint: tint).frame(width: size, height: size) } else { Ring(progress: 0.7, tint: tint, width: size / 6) }
        case (.agents?, .done?):
            Image(systemName: "checkmark.circle.fill").foregroundStyle(Color(hex: doneTint))
        case (.media?, .working?):
            if moving { Bars(tint: tint).frame(width: size, height: size) } else { StillBars(tint: tint) }
        case (.media?, _):
            Image(systemName: "pause.fill").foregroundStyle(color.opacity(0.85))
        case (.mail?, _):
            Image(systemName: "envelope.fill").foregroundStyle(color)
        case (.island?, _) where pill?.id == "upcoming.reminders":
            Image(systemName: "checklist").foregroundStyle(color)
        case (.island?, _) where pill?.id == "call":
            Image(systemName: "phone.fill").foregroundStyle(color)
        case (.island?, _) where pill?.id == "call.muted":
            Image(systemName: "mic.slash.fill").foregroundStyle(Color(hex: errorTint))
        case (.island?, .working?) where pill?.id.hasPrefix("assistant.") == true && pill?.progress == nil:
            if moving { Spinner(tint: tint).frame(width: size, height: size) } else { Ring(progress: 0.7, tint: tint, width: size / 6) }
        case (.island?, _) where pill?.id.hasPrefix("command.") == true:
            if moving { Spinner(tint: tint).frame(width: size, height: size) } else { Ring(progress: 0.7, tint: tint, width: size / 6) }
        case (.island?, _) where pill?.id.hasPrefix("unread.") == true:
            Image(systemName: "bubble.left.fill").foregroundStyle(color)
        case (.utilities?, _) where pill?.id == "utilities.awake":
            Image(systemName: "cup.and.saucer.fill").foregroundStyle(color)
        case (_, _) where pill?.progress != nil:
            Ring(progress: pill?.progress ?? 0, tint: tint, width: size / 6)
        default:
            Circle().fill(color).frame(width: size * 0.62, height: size * 0.62)
        }
    }
}

/// Part of a circle, filled to `progress`.
struct Ring: View {
    let progress: Double
    let tint: String
    var width: CGFloat = 2

    var body: some View {
        ZStack {
            Circle().stroke(Color.white.opacity(0.18), lineWidth: width)
            Circle().trim(from: 0, to: max(0.02, min(1, progress)))
                .stroke(Color(hex: tint), style: StrokeStyle(lineWidth: width, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .padding(width / 2)
    }
}

/// Three still bars, for pictures and Reduce Motion.
private struct StillBars: View {
    let tint: String

    var body: some View {
        HStack(alignment: .bottom, spacing: 1.5) {
            ForEach([0.55, 1.0, 0.7], id: \.self) { height in
                Capsule().fill(Color(hex: tint)).frame(width: 2.5).frame(maxHeight: .infinity, alignment: .bottom).scaleEffect(y: height, anchor: .bottom)
            }
        }
    }
}

/// An arc turning round, for work under way.
struct Spinner: NSViewRepresentable {
    let tint: String

    func makeNSView(context: Context) -> LayerView { LayerView(kind: .spinner) }
    func updateNSView(_ view: LayerView, context: Context) { view.tint = RGB(hex: tint) }
}

/// Bars rising and falling, for music playing.
struct Bars: NSViewRepresentable {
    let tint: String

    func makeNSView(context: Context) -> LayerView { LayerView(kind: .bars) }
    func updateNSView(_ view: LayerView, context: Context) { view.tint = RGB(hex: tint) }
}

/// A small view drawn by Core Animation layers whose animations repeat on their own.
final class LayerView: NSView {
    enum Kind { case spinner, bars }

    private let kind: Kind
    private var parts: [CALayer] = []

    var tint = RGB(hex: "#FFFFFF") {
        didSet { if tint != oldValue { paint() } }
    }

    init(kind: Kind) {
        self.kind = kind
        super.init(frame: .zero)
        wantsLayer = true
    }

    required init?(coder: NSCoder) { nil }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        build()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        build()
    }

    private func paint() {
        let color = CGColor(srgbRed: tint.r / 255, green: tint.g / 255, blue: tint.b / 255, alpha: 1)
        for part in parts {
            if let shape = part as? CAShapeLayer { shape.strokeColor = color } else { part.backgroundColor = color }
        }
    }

    private func build() {
        guard let layer, bounds.width > 0 else { return }
        parts.forEach { $0.removeFromSuperlayer() }
        parts = []
        let side = min(bounds.width, bounds.height)
        switch kind {
        case .spinner:
            let arc = CAShapeLayer()
            let line = max(1.5, side / 6)
            arc.frame = CGRect(x: (bounds.width - side) / 2, y: (bounds.height - side) / 2, width: side, height: side)
            arc.path = CGPath(ellipseIn: arc.bounds.insetBy(dx: line / 2, dy: line / 2), transform: nil)
            arc.fillColor = nil
            arc.lineWidth = line
            arc.lineCap = .round
            arc.strokeStart = 0
            arc.strokeEnd = 0.72
            let turn = CABasicAnimation(keyPath: "transform.rotation.z")
            turn.fromValue = 0
            turn.toValue = -2 * Double.pi
            turn.duration = 1.1
            turn.repeatCount = .infinity
            turn.isRemovedOnCompletion = false
            arc.add(turn, forKey: "turn")
            layer.addSublayer(arc)
            parts = [arc]
        case .bars:
            let width = side / 5
            let gap = (side - 3 * width) / 2
            for index in 0..<3 {
                let bar = CALayer()
                bar.anchorPoint = CGPoint(x: 0.5, y: 0)
                bar.frame = CGRect(x: (bounds.width - side) / 2 + CGFloat(index) * (width + gap), y: (bounds.height - side) / 2, width: width, height: side)
                bar.cornerRadius = width / 2
                let rise = CABasicAnimation(keyPath: "transform.scale.y")
                rise.fromValue = 0.3
                rise.toValue = 1
                rise.duration = [0.42, 0.55, 0.36][index]
                rise.autoreverses = true
                rise.repeatCount = .infinity
                rise.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                rise.isRemovedOnCompletion = false
                bar.add(rise, forKey: "rise")
                layer.addSublayer(bar)
                parts.append(bar)
            }
        }
        paint()
    }
}

/// Beside the notch while the island peeks: the lead's mark and its few characters, then a
/// dot for each of the next few, then "+N".
struct GlanceView: View {
    let glance: Glance

    var body: some View {
        HStack(spacing: 5) {
            PillMark(pill: glance.lead, tint: glance.lead.tint, size: 12)
            if let text = glance.text {
                Text(text)
                    .font(.system(size: 12, weight: .semibold).monospacedDigit())
                    .lineLimit(1)
                    .fixedSize()
            }
            if !glance.others.isEmpty {
                HStack(spacing: 5) {
                    ForEach(glance.others) { pill in
                        Circle()
                            .fill(Color(hex: pill.state == .waiting ? waitingTint : pill.state == .error ? errorTint : pill.tint))
                            .frame(width: 7, height: 7)
                            .opacity(pill.state == .idle ? 0.55 : 1)
                    }
                }
                .padding(.leading, 3)
            }
            if glance.more > 0 {
                Text("+\(glance.more)").font(.system(size: 10, weight: .semibold)).opacity(0.7).fixedSize()
            }
        }
        .foregroundStyle(.white)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(glance.caption)
    }
}

/// The chips in the opened island: one for each thing running. A click shows its card; a
/// session's chip, clicked again or with nothing to show, goes to its terminal.
struct ChipRow: View {
    let chips: [Chip]
    let selected: String?
    let width: CGFloat
    let choose: (Chip) -> Void

    static let spacing: CGFloat = 6
    private static let font = NSFont.systemFont(ofSize: 11.5, weight: .semibold)

    var body: some View {
        let (widths, badges) = Self.layout(chips, selected: selected, width: width)
        HStack(spacing: Self.spacing) {
            ForEach(Array(chips.enumerated()), id: \.element.id) { index, chip in
                let on = Self.on(chip, selected)
                Button { choose(chip) } label: {
                    ChipLabel(chip: chip, on: on, badge: badges[index]).frame(width: widths[index])
                }
                .buttonStyle(.plain)
                .accessibilityLabel(chip.badge.map { "\(chip.label), \($0)" } ?? chip.label)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
            Spacer(minLength: 0)
        }
    }

    static func on(_ chip: Chip, _ selected: String?) -> Bool {
        chip.card != nil && chip.card?.id == selected
    }

    private static func measure(_ text: String) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    /// Each chip as wide as it says, up to `Chips.widest`, with its badge while there's room
    /// for it, in order; when even the names don't fit, the longest are cut to share what is
    /// left.
    static func layout(_ chips: [Chip], selected: String?, width: CGFloat) -> ([CGFloat], [Bool]) {
        let room = width - spacing * CGFloat(max(chips.count - 1, 0))
        var widths = chips.map { chip in
            min(Chips.widest, 20 + 12 + 6 + measure(chip.label) + (ChipLabel.jumps(chip, on: on(chip, selected)) ? 6 + 9 : 0) + 1)
        }
        var badges = chips.map { _ in false }
        guard widths.reduce(0, +) <= room else { return (Chips.share(widths, room: room), badges) }
        for (index, chip) in chips.enumerated() {
            guard let text = chip.badge, !text.isEmpty else { continue }
            let extra = 6 + measure(text)
            if widths.reduce(0, +) + extra <= room, widths[index] + extra <= Chips.widest {
                widths[index] += extra
                badges[index] = true
            }
        }
        return (widths, badges)
    }
}

private struct ChipLabel: View {
    let chip: Chip
    let on: Bool
    let badge: Bool

    /// A session's chip goes to its terminal once its card shows, or when it has none.
    static func jumps(_ chip: Chip, on: Bool) -> Bool { chip.pill?.source == .agents && (on || chip.card == nil) }

    var body: some View {
        HStack(spacing: 6) {
            if let symbol = chip.symbol {
                Image(systemName: symbol).font(.system(size: 10, weight: .semibold)).foregroundStyle(Color(hex: chip.tint)).frame(width: 12, height: 12)
            } else {
                PillMark(pill: chip.pill, tint: chip.tint, size: 10)
            }
            Text(chip.label).lineLimit(1).truncationMode(.tail)
            if badge, let text = chip.badge, !text.isEmpty {
                Text(text).lineLimit(1).opacity(0.55).fixedSize()
            }
            if Self.jumps(chip, on: on) {
                Image(systemName: "arrow.up.forward").font(.system(size: 8.5, weight: .bold)).opacity(0.65)
            }
        }
        .font(.system(size: 11.5, weight: .semibold))
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity)
        .frame(height: 28)
        .background(Capsule().fill(Color.white.opacity(on ? 0.16 : 0.07)))
        .overlay(Capsule().strokeBorder(Color(hex: chip.tint).opacity(on ? 0.6 : 0), lineWidth: 1))
        .contentShape(Capsule())
    }
}
