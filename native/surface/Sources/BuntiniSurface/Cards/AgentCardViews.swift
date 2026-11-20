// Views for coding-agent cards (stream B).
import SwiftUI
import SurfaceKit

/// What an agent asks to do, shown whole: every line of the command, every line an edit
/// changes, and every rule Always keeps with where it's kept. When Rust couldn't show all of
/// it (`allow` off), only Deny and In terminal are offered.
struct ApprovalCardView: View {
    let card: Card
    let approval: Approval
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: from(approval.agent, approval.project), title: approval.title)
            if Self.hasBody(approval) {
                CardScroll(height: Self.bodyHeight(approval)) {
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
            if Self.offersAlways(approval) && !approval.rules.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(approval.rules.enumerated()), id: \.offset) { _, rule in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(rule.rule)
                                .font(.system(.caption2, design: .monospaced))
                                .fixedSize(horizontal: false, vertical: true)
                                .environment(\.layoutDirection, .leftToRight)
                            Text(rule.place).font(.caption2).opacity(0.6).fixedSize()
                            Spacer(minLength: 0)
                        }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(RoundedRectangle(cornerRadius: 7).fill(Color.white.opacity(0.07)))
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: L(.inTerminal)) { send(Action(source: .agents, card: card.id, name: "terminal")) }
                IslandButton(label: approval.plan ? L(.keepPlanning) : L(.deny)) { send(Action(source: .agents, card: card.id, name: "deny")) }
                if Self.offersAlways(approval) {
                    IslandButton(label: approval.plan ? L(.acceptEdits) : L(.always)) { send(Action(source: .agents, card: card.id, name: "always")) }
                }
                if approval.allow {
                    IslandButton(label: approval.plan ? L(.start) : L(.allow), prominent: true) { send(Action(source: .agents, card: card.id, name: "allow")) }
                }
            }
        }
    }

    static func offersAlways(_ approval: Approval) -> Bool {
        approval.allow && approval.always
    }

    static func hasBody(_ approval: Approval) -> Bool {
        approval.detail != nil || !approval.changes.isEmpty
    }

    /// The room the command and the changes take, up to where they scroll.
    static func bodyHeight(_ approval: Approval) -> CGFloat {
        var height: CGFloat = 0
        if let detail = approval.detail {
            height += textHeight(detail, perLine: approval.code ? 66 : 76, lineHeight: 13)
        }
        if !approval.changes.isEmpty {
            height += (approval.detail == nil ? 0 : 6) + approval.changes.reduce(0) { $0 + textHeight($1.text, perLine: 60, lineHeight: 14) }
        }
        return min(max(height, 14), 220)
    }

    /// The room the rules Always keeps take.
    static func rulesHeight(_ approval: Approval) -> CGFloat {
        guard offersAlways(approval), !approval.rules.isEmpty else { return 0 }
        return 18 + approval.rules.reduce(0) { $0 + textHeight($1.rule, perLine: 52, lineHeight: 15) }
    }
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
                IslandButton(label: L(.inTerminal)) { send(Action(source: .agents, card: card.id, name: "terminal")) }
                if current.multi {
                    IslandButton(label: step + 1 < question.questions.count ? L(.next) : L(.send), prominent: true) {
                        if !picked.isEmpty { choose(current, .array(picked.map(JSONValue.string))) }
                    }
                }
            }
        }
        // The next question's choices take the place of the last one's.
        .arms(on: step)
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
