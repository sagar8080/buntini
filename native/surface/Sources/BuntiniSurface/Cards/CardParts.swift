import SwiftUI
import SurfaceKit

/// The heading every card starts with: who it is from, and its title.
struct CardHeading: View {
    let from: String
    let title: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(from).font(.caption2.weight(.semibold)).opacity(0.6).lineLimit(1)
            Text(title).font(.callout.weight(.semibold)).lineLimit(2)
        }
    }
}

/// A click in the moment after a card comes forward (or a part of it that can be clicked
/// changes) was aimed at whatever was there before, so it is ignored. The chat's agent card
/// waits as long (src/AgentRequests.tsx), and Rust checks it again for agent answers.
public enum Arming {
    public static let delay: TimeInterval = 0.7

    public static func ready(since: Date, now: Date = Date()) -> Bool {
        now.timeIntervalSince(since) >= delay
    }
}

private struct ArmedSinceKey: EnvironmentKey {
    static let defaultValue = Date.distantPast
}

extension EnvironmentValues {
    /// When the clickable part around this view last changed.
    var armedSince: Date {
        get { self[ArmedSinceKey.self] }
        set { self[ArmedSinceKey.self] = newValue }
    }
}

/// Starts the arming delay again whenever `value` changes, for everything inside.
private struct ArmsOnChange<Value: Equatable>: ViewModifier {
    let value: Value
    @State private var since = Date()
    @Environment(\.armedSince) private var outer

    func body(content: Content) -> some View {
        content
            .environment(\.armedSince, max(outer, since))
            .onChange(of: value) { _ in since = Date() }
    }
}

extension View {
    /// Ignores clicks on the island buttons inside for a moment whenever `value` changes.
    func arms<Value: Equatable>(on value: Value) -> some View {
        modifier(ArmsOnChange(value: value))
    }
}

/// A plain island button. Clicks while its card is arming do nothing.
struct IslandButton: View {
    let label: String
    var prominent = false
    let action: () -> Void
    @Environment(\.armedSince) private var armedSince

    var body: some View {
        Button {
            if Arming.ready(since: armedSince) { action() }
        } label: {
            Text(label)
                .font(.caption.weight(.semibold))
                .padding(.horizontal, 12)
                .frame(height: 26)
                .background(Capsule().fill(prominent ? Color.white : Color.white.opacity(0.12)))
                .foregroundStyle(prominent ? Color.black : Color.white)
        }
        .buttonStyle(.plain)
    }
}

/// Scrolls what doesn't fit in `height`. A picture can't hold a scroll view, so it gets
/// the top of it.
struct CardScroll<Content: View>: View {
    let height: CGFloat
    @ViewBuilder let content: () -> Content
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        if inPicture {
            content()
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .frame(height: height, alignment: .top)
                .clipped()
        } else {
            ScrollView {
                content().frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: height)
        }
    }
}

/// About how tall `text` sets in a column `perLine` characters wide, `lineHeight` a line.
func textHeight(_ text: String, perLine: Int, lineHeight: CGFloat) -> CGFloat {
    let lines = text.split(separator: "\n", omittingEmptySubsequences: false).reduce(0) { total, line in
        total + max(1, Int((Double(line.count) / Double(perLine)).rounded(.up)))
    }
    return CGFloat(lines) * lineHeight
}

func from(_ agent: String, _ project: String?) -> String {
    project.map { "\(agent) · \($0)" } ?? agent
}
