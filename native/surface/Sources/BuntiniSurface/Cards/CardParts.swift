import SwiftUI
import SurfaceKit

/// The heading every card starts with: who it is from, and its title on one line, the whole of
/// it on hover. A question, which is what's being asked, takes up to three.
struct CardHeading: View {
    let from: String
    let title: String
    var titleLines = 1

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(from).font(.caption2.weight(.semibold)).opacity(0.6).lineLimit(1)
            Text(title)
                .font(.callout.weight(.semibold))
                .lineLimit(titleLines)
                .truncationMode(.tail)
                .fixedSize(horizontal: false, vertical: true)
                .help(title)
        }
    }
}

/// A card's other actions, behind ⋯ beside its buttons; the card offers the same on a
/// right-click (`.contextMenu`). A picture can't hold a menu, so it gets the button's look.
struct MoreButton<Items: View>: View {
    @ViewBuilder let items: () -> Items
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        Group {
            if inPicture {
                mark
            } else {
                Menu(content: items) { mark }
                    .menuStyle(.button)
                    .buttonStyle(.plain)
                    .menuIndicator(.hidden)
            }
        }
        .fixedSize()
        .accessibilityLabel(L(.more))
    }

    private var mark: some View {
        Image(systemName: "ellipsis")
            .font(.system(size: 11, weight: .bold))
            .frame(width: 26, height: 26)
            .background(Circle().fill(Color.white.opacity(0.12)))
            .contentShape(Circle())
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

/// A button in the band beside the notch, small enough to fit a menu bar. With `iconOnly` it
/// shows its symbol alone and says its label to VoiceOver and on hover.
struct BandButton: View {
    let label: String
    var symbol: String? = nil
    var iconOnly = false
    var prominent = false
    var spoken: String? = nil
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if let symbol { Image(systemName: symbol).font(.system(size: 10, weight: .semibold)) }
                if !iconOnly { Text(label).lineLimit(1) }
            }
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, iconOnly ? 0 : 10)
            .frame(width: iconOnly ? 22 : nil, height: 22)
            .background(Capsule().fill(prominent ? Color.white : Color.white.opacity(0.12)))
            .foregroundStyle(prominent ? Color.black : Color.white)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(iconOnly ? label : "")
        .accessibilityLabel(spoken ?? label)
    }
}

/// As tall as its content up to `maxHeight`, scrolling past it, and shorter still when the
/// island has less room, so what comes after it stays in view. A picture can't hold a scroll
/// view, so it gets the top of the content, sized the same way.
struct CardScroll<Content: View>: View {
    let maxHeight: CGFloat
    @ViewBuilder let content: () -> Content
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        if inPicture {
            content()
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .frame(minHeight: 0, maxHeight: maxHeight, alignment: .top)
                .clipped()
        } else {
            ScrollView {
                content().frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: maxHeight)
        }
    }
}

func from(_ agent: String, _ project: String?) -> String {
    project.map { "\(agent) · \($0)" } ?? agent
}
