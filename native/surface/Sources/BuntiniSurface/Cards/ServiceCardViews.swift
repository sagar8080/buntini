// Views for developer-service cards (stream D).
import SwiftUI
import SurfaceKit

struct ServiceCardView: View {
    let card: Card
    let service: ServiceCard
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            CardHeading(from: serviceName(service.service), title: service.title)
            ForEach(service.rows.prefix(4), id: \.id) { row in
                ServiceRowLine(row: row) { url in
                    send(Action(source: .services, card: card.id, name: "open", value: .string(url)))
                }
            }
            if let graph = service.graph, !graph.isEmpty {
                ServiceActivityGraph(days: graph)
            }
        }
    }
}

/// One pull request, deployment, payment or run: a state dot, its title and detail.
struct ServiceRowLine: View {
    let row: ServiceRow
    let open: (String) -> Void

    var body: some View {
        if let url = row.url {
            Button { open(url) } label: { line.contentShape(Rectangle()) }
                .buttonStyle(.plain)
                .accessibilityAddTraits(.isLink)
        } else {
            line
        }
    }

    private var line: some View {
        HStack(spacing: 8) {
            Circle().fill(serviceStateColor(row.state)).frame(width: 6, height: 6)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.title).font(.caption.weight(.medium)).lineLimit(1)
                if let detail = row.detail {
                    Text(detail).font(.caption2).opacity(0.6).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            if row.url != nil {
                Image(systemName: "arrow.up.right").font(.caption2).opacity(0.45)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text([row.title, row.detail].compactMap { $0 }.joined(separator: ", ")))
    }
}

/// The year as weeks of days, Sunday at the top, like the services draw it themselves.
struct ServiceActivityGraph: View {
    let days: [UInt32]

    private static let cell: CGFloat = 6
    private static let gap: CGFloat = 2

    var body: some View {
        let weeks = (days.count + 6) / 7
        let peak = max(days.max() ?? 0, 1)
        Canvas { context, _ in
            for (index, count) in days.enumerated() {
                let x = CGFloat(index / 7) * (Self.cell + Self.gap)
                let y = CGFloat(index % 7) * (Self.cell + Self.gap)
                let rect = CGRect(x: x, y: y, width: Self.cell, height: Self.cell)
                context.fill(Path(roundedRect: rect, cornerRadius: 1.5), with: .color(shade(count, peak: peak)))
            }
        }
        .frame(width: CGFloat(weeks) * (Self.cell + Self.gap) - Self.gap, height: 7 * (Self.cell + Self.gap) - Self.gap)
        .accessibilityElement()
        .accessibilityLabel(Text("Activity"))
        .accessibilityValue(Text("\(days.reduce(0) { $0 + Int($1) })"))
    }

    /// Four steps of green, by share of the busiest day; empty days stay faint.
    private func shade(_ count: UInt32, peak: UInt32) -> Color {
        guard count > 0 else { return Color.white.opacity(0.08) }
        let share = Double(count) / Double(peak)
        let level = share > 0.75 ? 1.0 : share > 0.5 ? 0.75 : share > 0.25 ? 0.5 : 0.3
        return Color(red: 0.25, green: 0.8, blue: 0.4).opacity(level)
    }
}

func serviceName(_ id: String) -> String {
    switch id {
    case "github": return "GitHub"
    case "gitlab": return "GitLab"
    case "gitea": return "Gitea"
    case "vercel": return "Vercel"
    case "stripe": return "Stripe"
    case "n8n": return "n8n"
    case "notion": return "Notion"
    case "calcom": return "Cal.com"
    case "resend": return "Resend"
    default: return id
    }
}

func serviceStateColor(_ state: PillState?) -> Color {
    switch state {
    case .waiting: return Color(red: 1.0, green: 0.75, blue: 0.25)
    case .error: return Color(red: 1.0, green: 0.36, blue: 0.36)
    case .working: return Color(red: 0.36, green: 0.62, blue: 1.0)
    case .done: return Color(red: 0.3, green: 0.82, blue: 0.45)
    case .idle, .none: return Color.white.opacity(0.35)
    }
}
