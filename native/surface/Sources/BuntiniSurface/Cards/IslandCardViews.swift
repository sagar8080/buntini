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

struct GreetingCardView: View {
    let greeting: Greeting

    var body: some View {
        Text(greeting.name.map { L(.hiName, ["name": $0]) } ?? L(.hi)).font(.callout.weight(.semibold))
    }
}
