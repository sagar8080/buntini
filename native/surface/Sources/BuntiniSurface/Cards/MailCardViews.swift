// Views for Mail cards (stream E).
import SwiftUI
import SurfaceKit

struct MailCardView: View {
    let card: Card
    let mail: MailCard
    let send: (Action) -> Void

    private func act(_ name: String, _ value: JSONValue? = nil) {
        send(Action(source: .mail, card: card.id, name: name, value: value))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let pending = mail.pending {
                MailPendingView(pending: pending, act: act)
            } else if let plan = mail.plan {
                MailPlanView(plan: plan, act: act)
            } else if let open = mail.open {
                MailOpenView(open: open, act: act).id(open.id)
            } else {
                MailListView(mail: mail, act: act)
            }
            if mail.closed {
                HStack {
                    Text(L(.mailClosed)).font(.caption).opacity(0.7)
                    Spacer()
                    IslandButton(label: L(.openMail)) { act("openMail") }
                }
            } else if let status = mail.status {
                Text(status).font(.caption).foregroundStyle(Color.orange).lineLimit(2)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L(.mail))
    }
}

private enum MailTime {
    static let hour: DateFormatter = { let f = DateFormatter(); f.dateFormat = "HH:mm"; return f }()
    /// Weekday and day names in the words' language, made again when it changes.
    private static var named: (language: String, weekday: DateFormatter, day: DateFormatter)?

    private static func formatters() -> (weekday: DateFormatter, day: DateFormatter) {
        let locale = Lexicon.locale
        if let named, named.language == locale.identifier { return (named.weekday, named.day) }
        let make = { (template: String) -> DateFormatter in
            let f = DateFormatter()
            f.locale = locale
            f.setLocalizedDateFormatFromTemplate(template)
            return f
        }
        let made = (language: locale.identifier, weekday: make("EEE"), day: make("d MMM"))
        named = made
        return (made.weekday, made.day)
    }

    static func short(_ seconds: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(seconds))
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return hour.string(from: date) }
        if let days = calendar.dateComponents([.day], from: date, to: Date()).day, days < 7 { return formatters().weekday.string(from: date) }
        return formatters().day.string(from: date)
    }
}

/// As tall as its content up to `maxHeight`, scrolling beyond it. A picture can't hold a
/// scroll view, so it gets the top of the content.
private struct MailFit<Content: View>: View {
    let maxHeight: CGFloat
    @ViewBuilder let content: () -> Content
    @State private var height: CGFloat = 0
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        if inPicture {
            content().frame(maxHeight: maxHeight, alignment: .top).clipped()
        } else {
            ScrollView {
                content()
                    .background(GeometryReader { proxy in
                        Color.clear.preference(key: MailFitHeight.self, value: proxy.size.height)
                    })
            }
            .frame(height: min(max(height, 1), maxHeight))
            .onPreferenceChange(MailFitHeight.self) { height = $0 }
        }
    }
}

private struct MailFitHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

private struct FilterChip: View {
    let label: String
    let on: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.caption.weight(on ? .semibold : .regular))
                .padding(.horizontal, 9)
                .frame(height: 22)
                .background(Capsule().fill(Color.white.opacity(on ? 0.22 : 0.07)))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

private struct MailListView: View {
    let mail: MailCard
    let act: (String, JSONValue?) -> Void
    @State private var query = ""

    private static var filters: [(String, String)] { [("all", L(.mailAll)), ("unread", L(.mailUnread)), ("important", L(.mailImportant)), ("today", L(.mailToday))] }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                ForEach(Self.filters, id: \.0) { filter in
                    FilterChip(label: filter.1, on: mail.filter == filter.0) { act("filter", .string(filter.0)) }
                }
                Spacer(minLength: 4)
                if mail.sortable > 0 {
                    IslandButton(label: L(.mailSort, ["count": String(mail.sortable)])) { act("sort", nil) }
                }
            }
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").font(.caption).opacity(0.6)
                TextField(L(.mailSearch), text: $query)
                    .textFieldStyle(.plain)
                    .font(.caption)
                    .onSubmit { act("search", .string(query)) }
                if !query.isEmpty {
                    Button { query = ""; act("search", .string("")) } label: { Image(systemName: "xmark.circle.fill").font(.caption) }
                        .buttonStyle(.plain)
                        .opacity(0.6)
                        .accessibilityLabel(L(.clear))
                }
            }
            .padding(.horizontal, 8)
            .frame(height: 24)
            .background(RoundedRectangle(cornerRadius: 7).fill(Color.white.opacity(0.08)))
            .onAppear { query = mail.query }

            if mail.messages.isEmpty {
                Text(mail.query.isEmpty ? L(.noMail) : L(.noMatch)).font(.caption).opacity(0.5).padding(.vertical, 6)
            } else {
                MailFit(maxHeight: 210) {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(mail.messages, id: \.id) { row in
                            MailRowView(row: row, act: act)
                        }
                        if mail.more {
                            Button { act("more", nil) } label: { Text(L(.more)).font(.caption).opacity(0.7) }
                                .buttonStyle(.plain)
                                .padding(.vertical, 4)
                        }
                    }
                }
            }
            if !mail.log.isEmpty {
                Divider().overlay(Color.white.opacity(0.15))
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(mail.log.prefix(3), id: \.id) { entry in
                        HStack(spacing: 6) {
                            Text(entry.label).font(.caption2).opacity(0.65).lineLimit(1)
                            Spacer(minLength: 4)
                            if entry.undoable {
                                Button { act("undo", .string(entry.id)) } label: {
                                    Label(L(.undo), systemImage: "arrow.uturn.backward").font(.caption2.weight(.semibold))
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }
        }
    }
}

private struct MailRowView: View {
    let row: MailRow
    let act: (String, JSONValue?) -> Void
    @State private var hovering = false

    var body: some View {
        Button { act("open", .string(row.id)) } label: {
            HStack(spacing: 7) {
                Circle().fill(row.unread ? Color(hex: "#3D8BFD") : Color.clear).frame(width: 6, height: 6)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(row.from).font(.caption.weight(row.unread ? .semibold : .regular)).lineLimit(1)
                        if row.flagged { Image(systemName: "flag.fill").font(.system(size: 8)).foregroundStyle(Color.orange) }
                        Spacer(minLength: 4)
                        Text(MailTime.short(row.at)).font(.caption2).opacity(0.5)
                    }
                    Text(row.subject).font(.caption2).opacity(0.7).lineLimit(1)
                }
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 7).fill(Color.white.opacity(hovering ? 0.08 : 0)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .contextMenu {
            Button(row.unread ? L(.markRead) : L(.markUnread)) { act(row.unread ? "markRead" : "markUnread", .string(row.id)) }
            Button(row.flagged ? L(.unflag) : L(.flag)) { act(row.flagged ? "unflag" : "flag", .string(row.id)) }
            Button(L(.junk)) { act("junk", .string(row.id)) }
        }
        .accessibilityLabel([row.from, row.subject, row.unread ? L(.isUnread) : nil, row.flagged ? L(.isFlagged) : nil].compactMap { $0 }.joined(separator: ", "))
    }
}

private struct MailOpenView: View {
    let open: MailOpen
    let act: (String, JSONValue?) -> Void
    @State private var replying = false
    @State private var reply = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Button { act("close", nil) } label: { Image(systemName: "chevron.left").font(.caption.weight(.semibold)).flipsForRightToLeftLayoutDirection(true) }
                    .buttonStyle(.plain)
                    .accessibilityLabel(L(.back))
                Text(open.from).font(.caption.weight(.semibold)).lineLimit(1)
                Spacer()
                Text(MailTime.short(open.at)).font(.caption2).opacity(0.5)
            }
            Text(open.subject).font(.callout.weight(.semibold)).lineLimit(2)
            MailFit(maxHeight: replying ? 90 : 170) {
                Text(open.body)
                    .font(.caption)
                    .opacity(0.85)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if !open.attachments.isEmpty {
                Label(open.attachments.joined(separator: ", "), systemImage: "paperclip").font(.caption2).opacity(0.7).lineLimit(1)
            }
            if replying {
                TextField(L(.reply), text: $reply, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.caption)
                    .lineLimit(3...6)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.white.opacity(0.08)))
                HStack(spacing: 6) {
                    Spacer()
                    IslandButton(label: L(.cancel)) { replying = false }
                    IslandButton(label: L(.saveDraft), prominent: true) {
                        act("reply", .object(["id": .string(open.id), "body": .string(reply)]))
                    }
                    .disabled(reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            } else {
                HStack(spacing: 6) {
                    IslandButton(label: L(.reply), prominent: true) { replying = true }
                    IslandButton(label: open.unread ? L(.markRead) : L(.markUnread)) { act(open.unread ? "markRead" : "markUnread", .string(open.id)) }
                    IslandButton(label: open.flagged ? L(.unflag) : L(.flag)) { act(open.flagged ? "unflag" : "flag", .string(open.id)) }
                    IslandButton(label: L(.junk)) { act("junk", .string(open.id)) }
                }
            }
        }
    }
}

private struct MailPendingView: View {
    let pending: MailPending
    let act: (String, JSONValue?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: L(.mail), title: pending.title)
            // Every line Allow carries out, whole; past the room there is, it scrolls.
            MailFit(maxHeight: 170) {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(pending.lines.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.caption).opacity(0.85).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: L(.deny)) { act("deny", .string(pending.id)) }
                IslandButton(label: L(.allow), prominent: true) { act("allow", .string(pending.id)) }
            }
        }
        // A change that takes another's place starts the arming delay again.
        .arms(on: pending)
    }
}

private struct MailPlanView: View {
    let plan: MailPlan
    let act: (String, JSONValue?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: L(.mailPlan), title: plan.summary.isEmpty ? L(.plan) : plan.summary)
            // Every line Run carries out, whole and wrapped; past the room there is, it scrolls.
            MailFit(maxHeight: 170) {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(plan.lines.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.caption).opacity(0.85).fixedSize(horizontal: false, vertical: true)
                    }
                    ForEach(Array(plan.problems.enumerated()), id: \.offset) { _, problem in
                        Text(problem).font(.caption2).foregroundStyle(Color.orange.opacity(0.85)).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: L(.discard)) { act("discard", .string(plan.id)) }
                IslandButton(label: L(.run), prominent: true) { act("run", .string(plan.id)) }
            }
        }
        // A plan that takes another's place starts the arming delay again.
        .arms(on: plan)
    }
}
