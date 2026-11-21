// Views for the island's own cards (stream A).
import SwiftUI
import SurfaceKit

struct DropCardView: View {
    let card: Card
    let drop: DropCard
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: drop.files.count > 1 ? "doc.on.doc.fill" : "doc.fill").font(.title3).opacity(0.85)
                let names = drop.files.map { ($0 as NSString).lastPathComponent }.joined(separator: ", ")
                Text(names).font(.callout.weight(.semibold)).lineLimit(1).truncationMode(.middle).help(names)
            }
            FlowRow(spacing: 6) {
                // Into the chat, attached to the next message.
                IslandButton(label: L(.chat)) {
                    send(Action(source: .island, card: card.id, name: "dropChat", value: .object(["paths": .array(drop.files.map(JSONValue.string))])))
                }
                ForEach(drop.targets, id: \.pill) { target in
                    IslandButton(label: target.label) {
                        send(Action(source: .island, card: card.id, name: "dropInto", value: .object(["pill": .string(target.pill), "paths": .array(drop.files.map(JSONValue.string))])))
                    }
                }
            }
        }
    }
}

/// The answer to what was typed in the island: the question, the answer in up to three
/// lines, and the chat for more.
struct AnswerCardView: View {
    let card: Card
    let answer: Answer
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: answer.question, title: answer.answer, titleLines: 3)
                .textSelection(.enabled)
            HStack {
                Spacer()
                IslandButton(label: L(.chat)) { send(Action(source: .island, card: card.id, name: "openChat")) }
            }
        }
    }
}

/// What's next: a meeting, how long until it and where, with Join for its call; or a reminder
/// past due, when it was due and its list, with Done.
struct UpcomingCardView: View {
    let card: Card
    let upcoming: Upcoming
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: heading, title: upcoming.title)
            HStack(spacing: 6) {
                Spacer()
                if upcoming.item == "meeting", let link = upcoming.link {
                    IslandButton(label: L(.join), prominent: true) { send(Action(source: .island, card: card.id, name: "join", value: .string(link))) }
                }
                if upcoming.item == "reminders", let id = upcoming.id {
                    IslandButton(label: L(.done), prominent: true) { send(Action(source: .island, card: card.id, name: "done", value: .string(id))) }
                }
            }
        }
    }

    /// "in 8 min · Zoom", or "Sat 09:00 · Home · +2", in the person's language.
    private var heading: String {
        var parts: [String] = []
        if let starts = upcoming.starts {
            let relative = RelativeDateTimeFormatter()
            relative.locale = Lexicon.locale
            relative.unitsStyle = .short
            parts.append(relative.localizedString(for: Date(timeIntervalSince1970: TimeInterval(starts)), relativeTo: Date()))
        }
        if let due = upcoming.due.flatMap(Self.due) { parts.append(due) }
        if let detail = upcoming.detail, !detail.isEmpty { parts.append(detail) }
        if upcoming.more > 0 { parts.append("+\(upcoming.more)") }
        return parts.joined(separator: " · ")
    }

    private static func due(_ stamp: String) -> String? {
        let parse = DateFormatter()
        parse.locale = Locale(identifier: "en_US_POSIX")
        parse.dateFormat = stamp.count > 10 ? "yyyy-MM-dd HH:mm" : "yyyy-MM-dd"
        guard let date = parse.date(from: stamp) else { return nil }
        let show = DateFormatter()
        show.locale = Lexicon.locale
        show.setLocalizedDateFormatFromTemplate(stamp.count > 10 ? (Calendar.current.isDateInToday(date) ? "HHmm" : "EEEHHmm") : "EEEdMMM")
        return show.string(from: date)
    }
}

struct GreetingCardView: View {
    let greeting: Greeting

    var body: some View {
        Text(greeting.name.map { L(.hiName, ["name": $0]) } ?? L(.hi)).font(.callout.weight(.semibold))
    }
}
