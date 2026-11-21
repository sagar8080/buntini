// Views for coding-agent cards (stream B).
import SwiftUI
import SurfaceKit

/// What an agent asks to do, shown whole: every line of the command and every line an edit
/// changes, scrolling past the room there is. Deny and Allow are on the card; In terminal and
/// Always, with every rule Always keeps and where, are behind ⋯ and a right-click. When Rust
/// couldn't show all of it (`allow` off), only Deny and In terminal are offered, both on the
/// card.
struct ApprovalCardView: View {
    let card: Card
    let approval: Approval
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: from(approval.agent, approval.project), title: approval.title)
            if Self.hasBody(approval) {
                CardScroll(maxHeight: Self.bodyCap) {
                    VStack(alignment: .leading, spacing: 6) {
                        if let detail = approval.detail {
                            Text(detail)
                                .font(approval.code ? .system(.caption, design: .monospaced) : .caption)
                                .opacity(0.85)
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                        if !approval.changes.isEmpty {
                            ChangeLines(lines: approval.changes)
                        }
                    }
                }
                // Code reads left to right in every language.
                .environment(\.layoutDirection, approval.code ? .leftToRight : Lexicon.direction)
            }
            HStack(spacing: 6) {
                Spacer()
                if approval.allow {
                    MoreButton { more }
                    IslandButton(label: approval.plan ? L(.keepPlanning) : L(.deny)) { act("deny") }
                    IslandButton(label: approval.plan ? L(.start) : L(.allow), prominent: true) { act("allow") }
                } else {
                    IslandButton(label: approval.plan ? L(.keepPlanning) : L(.deny)) { act("deny") }
                    IslandButton(label: L(.inTerminal), prominent: true) { act("terminal") }
                }
            }
        }
        .contextMenu { if approval.allow { more } }
    }

    private func act(_ name: String) {
        send(Action(source: .agents, card: card.id, name: name))
    }

    /// In terminal, and Always with each rule it would keep and where.
    @ViewBuilder private var more: some View {
        Button(L(.inTerminal)) { act("terminal") }
        if Self.offersAlways(approval) {
            let always = approval.plan ? L(.acceptEdits) : L(.always)
            if approval.rules.isEmpty {
                Button(always) { act("always") }
            } else {
                Button(([always] + approval.rules.map { "\($0.rule) · \($0.place)" }).joined(separator: " — ")) { act("always") }
            }
        }
    }

    static func offersAlways(_ approval: Approval) -> Bool {
        approval.allow && approval.always
    }

    static func hasBody(_ approval: Approval) -> Bool {
        approval.detail != nil || !approval.changes.isEmpty
    }

    /// The most of the command and the changes shown before they scroll. With less room in the
    /// island they scroll sooner, and the buttons stay.
    static let bodyCap: CGFloat = 220
}

/// An edit's lines, each whole and wrapped, marked as added, removed or kept.
struct ChangeLines: View {
    let lines: [DiffLine]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                Text(line.kind == "gap" ? "⋯" : (line.kind == "add" ? "+ " : line.kind == "remove" ? "− " : "  ") + line.text)
                    .font(.system(size: 11, design: .monospaced))
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 4)
                    .background(line.kind == "add" ? Color(hex: "#2EA043").opacity(0.22) : line.kind == "remove" ? Color(hex: "#F85149").opacity(0.22) : .clear)
                    .opacity(line.kind == "gap" ? 0.5 : 1)
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
            CardHeading(from: [from(question.agent, question.project), question.questions.count > 1 ? (current.header ?? "\(step + 1) / \(question.questions.count)") : nil].compactMap { $0 }.joined(separator: " · "), title: current.question, titleLines: 3)
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
                // With nothing to send, ⋯ ends the row of choices rather than taking a row.
                if !current.multi { MoreButton { inTerminal } }
            }
            if current.multi {
                HStack(spacing: 6) {
                    Spacer()
                    MoreButton { inTerminal }
                    IslandButton(label: step + 1 < question.questions.count ? L(.next) : L(.send), prominent: true) {
                        if !picked.isEmpty { choose(current, .array(picked.map(JSONValue.string))) }
                    }
                }
            }
        }
        .contextMenu { inTerminal }
        // The next question's choices take the place of the last one's.
        .arms(on: step)
        .onChange(of: card.id) { _ in
            step = 0
            answers = [:]
            picked = []
        }
    }

    private var inTerminal: some View {
        Button(L(.inTerminal)) { send(Action(source: .agents, card: card.id, name: "terminal")) }
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
            let shown = Array(ticker.steps.enumerated()).suffix(3)
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
                Text(diff.file).font(.callout.weight(.semibold)).lineLimit(1).truncationMode(.middle).help(diff.file)
                Text("+\(diff.added)").font(.caption.monospacedDigit()).foregroundStyle(Color(hex: "#7EE787"))
                Text("−\(diff.removed)").font(.caption.monospacedDigit()).foregroundStyle(Color(hex: "#FF7B72"))
            }
            CardScroll(maxHeight: 170) {
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
        }
    }
}

struct FinishedCardView: View {
    let finished: Finished

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill").font(.title3).foregroundStyle(Color(hex: "#7EE787"))
            CardHeading(from: from(finished.agent, finished.project), title: finished.message ?? L(.done))
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
                IslandButton(label: L(.share)) { send(Action(source: .agents, card: card.id, name: "shareRecap")) }
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
                    Text(window.label).font(.caption).lineLimit(1).fixedSize().frame(minWidth: 64, alignment: .leading)
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
