// Views for now-playing cards (stream F).
import SwiftUI
import SurfaceKit

struct NowPlayingCardView: View {
    let card: Card
    let playing: NowPlaying
    let send: (Action) -> Void

    var body: some View {
        HStack {
            CardHeading(from: playing.artist ?? playing.player, title: playing.title)
            Spacer()
            IslandButton(label: playing.playing ? "Pause" : "Play") {
                send(Action(source: .media, card: card.id, name: playing.playing ? "pause" : "play"))
            }
        }
    }
}
