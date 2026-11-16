// Views for notes (stream C).
import SwiftUI
import SurfaceKit

struct NoteCardView: View {
    let card: Card
    let note: Note
    let send: (Action) -> Void

    var body: some View {
        CardHeading(from: note.state, title: note.title)
    }
}
