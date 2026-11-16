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
                    Text("Mail is closed").font(.caption).opacity(0.7)
                    Spacer()
                    IslandButton(label: "Open Mail") { act("openMail") }
                }
            } else if let status = mail.status {
                Text(status).font(.caption).foregroundStyle(Color.orange).lineLimit(2)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Mail")
    }
}

private enum MailTime {
    static let hour: DateFormatter = { let f = DateFormatter(); f.dateFormat = "HH:mm"; return f }()
    static let weekday: DateFormatter = { let f = DateFormatter(); f.setLocalizedDateFormatFromTemplate("EEE"); return f }()
    static let day: DateFormatter = { let f = DateFormatter(); f.setLocalizedDateFormatFromTemplate("d MMM"); return f }()

    static func short(_ seconds: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(seconds))
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return hour.string(from: date) }
        if let days = calendar.dateComponents([.day], from: date, to: Date()).day, days < 7 { return weekday.string(from: date) }
        return day.string(from: date)
    }
}

/// As tall as its content up to `maxHeight`, scrolling beyond it.
private struct MailFit<Content: View>: View {
    let maxHeight: CGFloat
    @ViewBuilder let content: () -> Content
    @State private var height: CGFloat = 0

    var body: some View {
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

    private static let filters = [("all", "All"), ("unread", "Unread"), ("important", "Important"), ("today", "Today")]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                ForEach(Self.filters, id: \.0) { filter in
                    FilterChip(label: filter.1, on: mail.filter == filter.0) { act("filter", .string(filter.0)) }
                }
                Spacer(minLength: 4)
                if mail.sortable > 0 {
                    IslandButton(label: "Sort \(mail.sortable)") { act("sort", nil) }
                }
            }
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").font(.caption).opacity(0.6)
                TextField("Search", text: $query)
                    .textFieldStyle(.plain)
                    .font(.caption)
                    .onSubmit { act("search", .string(query)) }
                if !query.isEmpty {
                    Button { query = ""; act("search", .string("")) } label: { Image(systemName: "xmark.circle.fill").font(.caption) }
                        .buttonStyle(.plain)
                        .opacity(0.6)
                        .accessibilityLabel("Clear")
                }
            }
            .padding(.horizontal, 8)
            .frame(height: 24)
            .background(RoundedRectangle(cornerRadius: 7).fill(Color.white.opacity(0.08)))
            .onAppear { query = mail.query }

            if mail.messages.isEmpty {
                Text(mail.query.isEmpty ? "No mail" : "No match").font(.caption).opacity(0.5).padding(.vertical, 6)
            } else {
                MailFit(maxHeight: 210) {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(mail.messages, id: \.id) { row in
                            MailRowView(row: row, act: act)
                        }
                        if mail.more {
                            Button { act("more", nil) } label: { Text("More").font(.caption).opacity(0.7) }
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
                                    Label("Undo", systemImage: "arrow.uturn.backward").font(.caption2.weight(.semibold))
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
            Button(row.unread ? "Mark Read" : "Mark Unread") { act(row.unread ? "markRead" : "markUnread", .string(row.id)) }
            Button(row.flagged ? "Unflag" : "Flag") { act(row.flagged ? "unflag" : "flag", .string(row.id)) }
            Button("Junk") { act("junk", .string(row.id)) }
        }
        .accessibilityLabel("\(row.from), \(row.subject)\(row.unread ? ", unread" : "")\(row.flagged ? ", flagged" : "")")
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
                Button { act("close", nil) } label: { Image(systemName: "chevron.left").font(.caption.weight(.semibold)) }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Back")
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
                TextField("Reply", text: $reply, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.caption)
                    .lineLimit(3...6)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.white.opacity(0.08)))
                HStack(spacing: 6) {
                    Spacer()
                    IslandButton(label: "Cancel") { replying = false }
                    IslandButton(label: "Save draft", prominent: true) {
                        act("reply", .object(["id": .string(open.id), "body": .string(reply)]))
                    }
                    .disabled(reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            } else {
                HStack(spacing: 6) {
                    IslandButton(label: "Reply", prominent: true) { replying = true }
                    IslandButton(label: open.unread ? "Mark Read" : "Mark Unread") { act(open.unread ? "markRead" : "markUnread", .string(open.id)) }
                    IslandButton(label: open.flagged ? "Unflag" : "Flag") { act(open.flagged ? "unflag" : "flag", .string(open.id)) }
                    IslandButton(label: "Junk") { act("junk", .string(open.id)) }
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
            CardHeading(from: "Mail", title: pending.title)
            VStack(alignment: .leading, spacing: 3) {
                ForEach(Array(pending.lines.prefix(8).enumerated()), id: \.offset) { _, line in
                    Text(line).font(.caption).opacity(0.85).lineLimit(3)
                }
                if pending.lines.count > 8 {
                    Text("+\(pending.lines.count - 8)").font(.caption2).opacity(0.6)
                }
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: "Deny") { act("deny", .string(pending.id)) }
                IslandButton(label: "Allow", prominent: true) { act("allow", .string(pending.id)) }
            }
        }
    }
}

private struct MailPlanView: View {
    let plan: MailPlan
    let act: (String, JSONValue?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: "Mail plan", title: plan.summary.isEmpty ? "Plan" : plan.summary)
            MailFit(maxHeight: 200) {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(plan.lines.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.caption).opacity(0.85).lineLimit(2)
                    }
                    ForEach(Array(plan.problems.enumerated()), id: \.offset) { _, problem in
                        Text(problem).font(.caption2).foregroundStyle(Color.orange.opacity(0.85)).lineLimit(2)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack(spacing: 6) {
                Spacer()
                IslandButton(label: "Discard") { act("discard", .string(plan.id)) }
                IslandButton(label: "Run", prominent: true) { act("run", .string(plan.id)) }
            }
        }
    }
}
