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

/// A plain island button.
struct IslandButton: View {
    let label: String
    var prominent = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
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

func from(_ agent: String, _ project: String?) -> String {
    project.map { "\(agent) · \($0)" } ?? agent
}
