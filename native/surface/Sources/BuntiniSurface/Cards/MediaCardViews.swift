// Views for now-playing cards (stream F): the cover in its own light, the track, a progress
// bar to drag, and the player's controls. The moving parts are in Media/.
import SwiftUI
import SurfaceKit

struct NowPlayingCardView: View {
    let card: Card
    let playing: NowPlaying
    let send: (Action) -> Void
    @State private var art: CoverArt?
    /// The file `art` was made from.
    @State private var artPath: String?
    @Environment(\.inPicture) private var inPicture

    private static let coverSide: CGFloat = 84

    private func act(_ name: String, _ value: JSONValue? = nil) {
        send(Action(source: .media, card: card.id, name: name, value: value))
    }

    /// The player's own colour, until the cover gives one.
    private var playerTint: RGB {
        RGB(hex: playing.player == "spotify" ? "#1ED760" : playing.player == "tv" ? "#8EA2FF" : "#FA2D48")
    }

    private var shownArt: CoverArt? {
        guard let path = playing.artwork else { return nil }
        if inPicture { return CoverArt.now(path) }
        return artPath == path ? art : CoverArt.cached(path)
    }

    var body: some View {
        let tint = shownArt?.tint ?? playerTint
        HStack(alignment: .center, spacing: 14) {
            CoverView(art: shownArt, player: playing.player, tint: tint, size: Self.coverSide)
                .onTapGesture { act("open") }
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel(playing.album ?? playing.title)
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .top, spacing: 8) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(playing.title)
                            .font(.system(size: 14, weight: .semibold))
                            .lineLimit(1)
                        if let subtitle {
                            Text(subtitle)
                                .font(.system(size: 12))
                                .foregroundStyle(Color.white.opacity(0.6))
                                .lineLimit(1)
                        }
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { act("open") }
                    Spacer(minLength: 0)
                    LevelBars(playing: playing.playing, tint: tint)
                        .padding(.top, 3)
                }
                Spacer(minLength: 4)
                if let clock = PlaybackClock(playing), playing.duration != nil {
                    ScrubBar(clock: clock, tint: tint) { seconds in act("seek", .number(seconds)) }
                    Spacer(minLength: 2)
                }
                controls(tint: tint)
            }
            .frame(height: Self.coverSide)
        }
        .animation(.easeOut(duration: 0.25), value: tint)
        .onAppear {
            act("shown", .bool(true))
            load()
        }
        .onDisappear { act("shown", .bool(false)) }
        .onChange(of: playing.artwork) { _ in load() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Now playing")
    }

    private var subtitle: String? {
        let parts = [playing.artist, playing.album].compactMap { $0?.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private func controls(tint: RGB) -> some View {
        HStack(spacing: 6) {
            if let shuffle = playing.shuffle {
                GlyphButton(symbol: "shuffle", size: 12, on: shuffle, tint: tint, label: "Shuffle") { act("shuffle", .bool(!shuffle)) }
            }
            GlyphButton(symbol: "backward.fill", size: 14, tint: tint, label: "Previous") { act("previous") }
            PlayButton(playing: playing.playing, tint: tint) { act(playing.playing ? "pause" : "play") }
            GlyphButton(symbol: "forward.fill", size: 14, tint: tint, label: "Next") { act("next") }
            if let mode = playing.repeatMode {
                GlyphButton(symbol: mode == "one" ? "repeat.1" : "repeat", size: 12, on: mode != "off", tint: tint, label: "Repeat") {
                    act("repeat", .string(Self.nextRepeat(mode, player: playing.player)))
                }
            }
            Spacer(minLength: 8)
            if let volume = playing.volume {
                VolumeBar(volume: volume, tint: tint) { level in act("volume", .number((level * 100).rounded() / 100)) }
            }
        }
        // The first symbol lines up with the title, not its button's edge.
        .padding(.leading, -5)
    }

    /// Off, all, one, and round again; Spotify has no repeat-one to script.
    static func nextRepeat(_ mode: String, player: String) -> String {
        switch mode {
        case "off": return "all"
        case "all": return player == "music" ? "one" : "off"
        default: return "off"
        }
    }

    private func load() {
        guard let path = playing.artwork, !inPicture else {
            art = nil
            artPath = nil
            return
        }
        artPath = path
        if let cached = CoverArt.cached(path) {
            art = cached
            return
        }
        CoverArt.load(path) { loaded in
            // A newer track may have come meanwhile.
            guard artPath == path else { return }
            withAnimation(.easeOut(duration: 0.35)) { art = loaded }
        }
    }
}
