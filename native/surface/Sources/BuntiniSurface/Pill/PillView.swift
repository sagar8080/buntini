// What the dictation pill draws: a small mark when idle, a toolbar on hover, the voice bands
// while listening, then what happened to the words.
import SwiftUI
import SurfaceKit

struct PillView: View {
    @ObservedObject var model: PillModel
    let levels: Levels
    let send: (Action) -> Void
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            if let pill = model.pill {
                pillBody(pill).inLanguage()
                    .background(Capsule().fill(Color.black.opacity(0.88)))
                    .overlay(Capsule().strokeBorder(Color.white.opacity(0.14), lineWidth: 0.5))
                    .shadow(color: .black.opacity(0.25), radius: 6, y: 2)
                    .onHover { inside in hovering = inside }
                    .contextMenu { menu(pill) }
                    .animation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.78), value: shape(pill))
                    .accessibilityElement(children: .contain)
            }
        }
        .frame(width: PillPanel.size.width, height: PillPanel.size.height, alignment: .bottom)
        .padding(.bottom, 2)
    }

    /// What decides the pill's size, for its animation.
    private func shape(_ pill: FlowPill) -> String {
        "\(pill.state.rawValue)-\(hovering)-\(pill.locked)-\(pill.recording != nil)"
    }

    @ViewBuilder
    private func pillBody(_ pill: FlowPill) -> some View {
        HStack(spacing: 8) {
            if let since = pill.recording {
                RecordingMark(since: since)
            }
            switch pill.state {
            case .idle:
                if hovering { toolbar(pill) } else if pill.recording == nil { Color.clear.frame(width: 34, height: 4) }
            case .listening:
                BandsView(levels: levels, active: model.visible)
                    .frame(width: 84, height: 22)
                    .accessibilityLabel(L(.listening))
                if pill.locked {
                    Image(systemName: "lock.fill").font(.system(size: 10, weight: .semibold)).foregroundStyle(.white.opacity(0.7))
                        .accessibilityLabel(L(.handsFree))
                }
                if hovering || pill.locked {
                    iconButton("stop.fill", label: L(.finish)) { act("dictate") }
                    iconButton("xmark", label: L(.cancel)) { act("cancel") }
                }
            case .working:
                WorkingDots(still: reduceMotion).frame(width: 40, height: 18).accessibilityLabel(L(.working))
            case .done:
                result("checkmark", pill.app)
            case .copied:
                result("doc.on.clipboard", L(.copied))
            case .error:
                result("exclamationmark.triangle.fill", pill.text, tint: .orange)
            }
        }
        .padding(.horizontal, padding(pill))
        .frame(height: height(pill))
        .foregroundStyle(.white)
    }

    private func padding(_ pill: FlowPill) -> CGFloat {
        pill.state == .idle && !hovering && pill.recording == nil ? 3 : 12
    }

    private func height(_ pill: FlowPill) -> CGFloat {
        pill.state == .idle && !hovering && pill.recording == nil ? 10 : 34
    }

    private func toolbar(_ pill: FlowPill) -> some View {
        HStack(spacing: 4) {
            iconButton("mic.fill", label: L(.dictate)) { act("dictate") }
            iconButton(pill.recording == nil ? "record.circle" : "stop.circle", label: pill.recording == nil ? L(.recordCall) : L(.stopRecording), tint: pill.recording == nil ? .white : .red) { act("record") }
            iconButton("pawprint.fill", label: L(.openBuntini)) { act("open") }
        }
    }

    private func result(_ symbol: String, _ text: String?, tint: Color = .white) -> some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).font(.system(size: 12, weight: .semibold)).foregroundStyle(tint)
            if let text, !text.isEmpty {
                Text(text).font(.system(size: 12, weight: .medium)).lineLimit(1).truncationMode(.tail).frame(maxWidth: 240)
            }
        }
    }

    private func iconButton(_ symbol: String, label: String, tint: Color = .white, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(tint)
                .frame(width: 28, height: 26)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(label)
        .accessibilityLabel(label)
    }

    @ViewBuilder
    private func menu(_ pill: FlowPill) -> some View {
        if let app = pill.app, !app.isEmpty {
            Section(app) {
                ForEach(FlowStyle.allCases, id: \.self) { style in
                    Button { act("style", .string(style.rawValue)) } label: {
                        if pill.style == style { Label(styleName(style), systemImage: "checkmark") } else { Text(styleName(style)) }
                    }
                }
                Button(L(.automatic)) { act("style", .null) }
            }
        }
        Section {
            ForEach(FlowMode.allCases, id: \.self) { mode in
                Button { act("mode", .string(mode.rawValue)) } label: {
                    if pill.mode == mode { Label(modeName(mode), systemImage: "checkmark") } else { Text(modeName(mode)) }
                }
            }
        }
        Button(L(.library)) { act("library") }
    }

    private func styleName(_ style: FlowStyle) -> String {
        switch style {
        case .asSaid: L(.asSaid)
        case .natural: L(.natural)
        case .polished: L(.polished)
        }
    }

    private func modeName(_ mode: FlowMode) -> String {
        switch mode {
        case .dictate: L(.modeDictate)
        case .prompt: L(.modePrompt)
        case .translate: L(.modeTranslate)
        }
    }

    private func act(_ name: String, _ value: JSONValue? = nil) {
        send(Action(source: .flow, card: nil, name: name, value: value))
    }
}

/// A red dot and the time a call has been recording.
struct RecordingMark: View {
    let since: Double

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            HStack(spacing: 5) {
                Circle().fill(Color.red).frame(width: 7, height: 7)
                Text(Self.elapsed(context.date.timeIntervalSince1970 - since))
                    .font(.system(size: 11, weight: .semibold).monospacedDigit())
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(L(.recording))
    }

    static func elapsed(_ seconds: Double) -> String {
        let total = max(0, Int(seconds))
        return total >= 3600 ? String(format: "%d:%02d:%02d", total / 3600, total / 60 % 60, total % 60) : String(format: "%d:%02d", total / 60, total % 60)
    }
}

/// Three dots breathing while the words are worked on.
struct WorkingDots: View {
    let still: Bool

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: still)) { context in
            let time = context.date.timeIntervalSinceReferenceDate
            HStack(spacing: 5) {
                ForEach(0..<3, id: \.self) { index in
                    Circle()
                        .fill(Color.white)
                        .frame(width: 5, height: 5)
                        .opacity(still ? 0.8 : 0.35 + 0.65 * (0.5 + 0.5 * sin(time * 5 - Double(index) * 0.8)))
                }
            }
        }
    }
}
