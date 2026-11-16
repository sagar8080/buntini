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
                IslandButton(label: "Deny") { send(Action(source: .agents, card: card.id, name: "deny")) }
                if approval.always {
                    IslandButton(label: "Always") { send(Action(source: .agents, card: card.id, name: "always")) }
                }
                IslandButton(label: "Allow", prominent: true) { send(Action(source: .agents, card: card.id, name: "allow")) }
            }
        }
    }
}

struct QuestionCardView: View {
    let card: Card
    let question: QuestionCard
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: from(question.agent, question.project), title: question.questions.first?.question ?? "")
            HStack(spacing: 6) {
                ForEach(question.questions.first?.options ?? [], id: \.label) { option in
                    IslandButton(label: option.label) {
                        send(Action(source: .agents, card: card.id, name: "answer", value: .string(option.label)))
                    }
                }
            }
        }
    }
}

struct TickerCardView: View {
    let ticker: Ticker

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(from(ticker.agent, ticker.project)).font(.caption2.weight(.semibold)).opacity(0.6)
            ForEach(Array(ticker.steps.suffix(3).enumerated()), id: \.offset) { _, step in
                Text(step.label).font(.caption).lineLimit(1)
            }
        }
    }
}

struct DiffCardView: View {
    let diff: DiffCard

    var body: some View {
        CardHeading(from: "+\(diff.added) −\(diff.removed)", title: diff.file)
    }
}

struct FinishedCardView: View {
    let finished: Finished

    var body: some View {
        CardHeading(from: from(finished.agent, finished.project), title: finished.message ?? "Done")
    }
}

struct RecapCardView: View {
    let recap: Recap

    var body: some View {
        CardHeading(from: recap.week, title: recap.stats.map { "\($0.label) \($0.value)" }.joined(separator: " · "))
    }
}

struct UsageCardView: View {
    let usage: Usage

    var body: some View {
        CardHeading(from: usage.agent, title: usage.windows.map { "\($0.label) \(Int($0.percent))%" }.joined(separator: " · "))
    }
}
