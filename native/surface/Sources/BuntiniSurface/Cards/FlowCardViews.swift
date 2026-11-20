// Views for notes (stream C).
import SwiftUI
import SurfaceKit

struct NoteCardView: View {
    let card: Card
    let note: Note
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardHeading(from: heading, title: note.title)
            ForEach(Array(note.lines.prefix(3).enumerated()), id: \.offset) { _, line in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if let speaker = line.speaker {
                        Text(speaker).font(.caption.weight(.semibold)).opacity(0.7)
                    }
                    Text(line.text).font(.caption).lineLimit(2)
                }
            }
            HStack {
                Spacer()
                IslandButton(label: L(.open), prominent: true) {
                    send(Action(source: .flow, card: card.id, name: "library"))
                }
            }
        }
        .accessibilityElement(children: .contain)
    }

    private var heading: String {
        guard let seconds = note.seconds, seconds > 0 else { return L(.notes) }
        return "\(L(.notes)) · \(RecordingMark.elapsed(seconds))"
    }
}
