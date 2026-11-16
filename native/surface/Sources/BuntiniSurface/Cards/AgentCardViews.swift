// Views for coding-agent cards (stream B).
import SwiftUI
import SurfaceKit

struct ApprovalCardView: View {
    let card: Card
    let approval: Approval
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: from(approval.agent, approval.project), title: approval.title)
            if let detail = approval.detail {
                Text(detail)
                    .font(approval.code ? .system(.caption, design: .monospaced) : .caption)
                    .lineLimit(4)
                    .opacity(0.85)
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: "In terminal") { send(Action(source: .agents, card: card.id, name: "terminal")) }
                IslandButton(label: approval.plan ? "Keep planning" : "Deny") { send(Action(source: .agents, card: card.id, name: "deny")) }
                if approval.always {
                    IslandButton(label: approval.plan ? "Accept edits" : "Always") { send(Action(source: .agents, card: card.id, name: "always")) }
                }
                IslandButton(label: approval.plan ? "Start" : "Allow", prominent: true) { send(Action(source: .agents, card: card.id, name: "allow")) }
            }
        }
    }
}

struct QuestionCardView: View {
    let card: Card
    let question: QuestionCard
    let send: (Action) -> Void
    @State private var step = 0
    @State private var answers: [String: JSONValue] = [:]
    @State private var picked: [String] = []

    var body: some View {
        let current = question.questions[min(step, question.questions.count - 1)]
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: [from(question.agent, question.project), question.questions.count > 1 ? (current.header ?? "\(step + 1) / \(question.questions.count)") : nil].compactMap { $0 }.joined(separator: " · "), title: current.question)
            FlowRow(spacing: 6) {
                ForEach(current.options, id: \.label) { option in
                    IslandButton(label: option.label, prominent: picked.contains(option.label)) {
                        if current.multi {
                            if let index = picked.firstIndex(of: option.label) { picked.remove(at: index) } else { picked.append(option.label) }
                        } else {
                            choose(current, .string(option.label))
                        }
                    }
                    .help(option.description ?? "")
                }
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: "In terminal") { send(Action(source: .agents, card: card.id, name: "terminal")) }
                if current.multi {
                    IslandButton(label: step + 1 < question.questions.count ? "Next" : "Send", prominent: true) {
                        if !picked.isEmpty { choose(current, .array(picked.map(JSONValue.string))) }
                    }
                }
            }
        }
        .onChange(of: card.id) { _ in
            step = 0
            answers = [:]
            picked = []
        }
    }

    private func choose(_ current: AgentQuestion, _ value: JSONValue) {
        answers[current.question] = value
        picked = []
        if step + 1 < question.questions.count {
            step += 1
        } else {
            send(Action(source: .agents, card: card.id, name: "answers", value: .object(answers)))
        }
    }
}

/// Lays its children out in rows, wrapping when a row is full.
struct FlowRow: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 400
        var x: CGFloat = 0, y: CGFloat = 0, row: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0 && x + size.width > width {
                x = 0
                y += row + spacing
                row = 0
            }
            x += size.width + spacing
            row = max(row, size.height)
        }
        return CGSize(width: width, height: y + row)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, row: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > bounds.minX && x + size.width > bounds.maxX {
                x = bounds.minX
                y += row + spacing
                row = 0
            }
            view.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            row = max(row, size.height)
        }
    }
}

struct TickerCardView: View {
    let card: Card
    let ticker: Ticker
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(from(ticker.agent, ticker.project)).font(.caption2.weight(.semibold)).opacity(0.6)
            let shown = Array(ticker.steps.enumerated()).suffix(4)
            ForEach(shown, id: \.offset) { index, step in
                let changed = step.added != nil || step.removed != nil
                Button {
                    if changed { send(Action(source: .agents, card: card.id, name: "diff", value: .number(Double(index)))) }
                } label: {
                    HStack(spacing: 6) {
                        Text(step.label).font(.caption).lineLimit(1).opacity(index == ticker.steps.count - 1 ? 1 : 0.7)
                        if let added = step.added { Text("+\(added)").font(.caption2.monospacedDigit()).foregroundStyle(Color(hex: "#7EE787")) }
                        if let removed = step.removed { Text("−\(removed)").font(.caption2.monospacedDigit()).foregroundStyle(Color(hex: "#FF7B72")) }
                        if step.failed { Image(systemName: "exclamationmark.triangle.fill").font(.caption2).foregroundStyle(Color(hex: "#F2B33D")) }
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!changed)
            }
        }
    }
}

struct DiffCardView: View {
    let diff: DiffCard

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(diff.file).font(.callout.weight(.semibold)).lineLimit(1).truncationMode(.head)
                Text("+\(diff.added)").font(.caption.monospacedDigit()).foregroundStyle(Color(hex: "#7EE787"))
                Text("−\(diff.removed)").font(.caption.monospacedDigit()).foregroundStyle(Color(hex: "#FF7B72"))
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(diff.lines.enumerated()), id: \.offset) { _, line in
                        Text(line.kind == "gap" ? "⋯" : (line.kind == "add" ? "+ " : line.kind == "remove" ? "− " : "  ") + line.text)
                            .font(.system(size: 11, design: .monospaced))
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 4)
                            .background(line.kind == "add" ? Color(hex: "#2EA043").opacity(0.22) : line.kind == "remove" ? Color(hex: "#F85149").opacity(0.22) : .clear)
                            .opacity(line.kind == "gap" ? 0.5 : 1)
                    }
                }
            }
            .frame(maxHeight: 170)
        }
    }
}

struct FinishedCardView: View {
    let finished: Finished

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill").font(.title3).foregroundStyle(Color(hex: "#7EE787"))
            CardHeading(from: from(finished.agent, finished.project), title: finished.message ?? "Done")
        }
    }
}

struct RecapCardView: View {
    let card: Card
    let recap: Recap
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(recap.week).font(.caption2.weight(.semibold)).opacity(0.6)
            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], alignment: .leading, spacing: 6) {
                ForEach(recap.stats.prefix(6), id: \.label) { stat in
                    VStack(alignment: .leading, spacing: 1) {
                        Text(stat.value).font(.callout.weight(.semibold)).lineLimit(1).minimumScaleFactor(0.7)
                        Text(stat.label).font(.caption2).opacity(0.6)
                    }
                }
            }
            HStack {
                Spacer()
                IslandButton(label: "Share") { send(Action(source: .agents, card: card.id, name: "shareRecap")) }
            }
        }
    }
}

struct UsageCardView: View {
    let usage: Usage

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(usage.agent).font(.caption2.weight(.semibold)).opacity(0.6)
            ForEach(usage.windows, id: \.label) { window in
                HStack(spacing: 8) {
                    Text(window.label).font(.caption).frame(width: 64, alignment: .leading)
                    GeometryReader { geometry in
                        ZStack(alignment: .leading) {
                            Capsule().fill(.white.opacity(0.12))
                            Capsule().fill(Color(hex: window.percent >= 90 ? "#FF7B72" : window.percent >= 70 ? "#F2B33D" : "#7EE787")).frame(width: geometry.size.width * min(1, max(0, window.percent / 100)))
                        }
                    }
                    .frame(height: 6)
                    Text("\(Int(window.percent.rounded()))%").font(.caption.monospacedDigit()).frame(width: 40, alignment: .trailing)
                }
            }
        }
    }
}
