// The moving parts of the now-playing card. The progress bar and the level bars are moved by
// Core Animation in the render server, at the display's own rate, so the app does no work per
// frame; they exist only while the card shows, and stop with it. The times tick once a second
// while playing and not at all while paused.
import AppKit
import SwiftUI
import SurfaceKit

/// Where the track is: a position read at a moment, running on while it plays.
struct PlaybackClock: Equatable {
    var position: Double
    var at: Date
    var playing: Bool
    var duration: Double?

    init(position: Double, at: Date, playing: Bool, duration: Double?) {
        self.position = position
        self.at = at
        self.playing = playing
        self.duration = duration
    }

    init?(_ card: NowPlaying, now: Date = Date()) {
        guard let position = card.position else { return nil }
        self.init(position: position, at: card.positionAt.map(Date.init(timeIntervalSince1970:)) ?? now, playing: card.playing, duration: card.duration)
    }

    func position(at now: Date) -> Double {
        let moved = playing ? max(0, now.timeIntervalSince(at)) : 0
        guard let duration, duration > 0 else { return position + moved }
        return min(position + moved, duration)
    }

    /// 0…1, when the length is known.
    func fraction(at now: Date) -> Double? {
        guard let duration, duration > 0 else { return nil }
        return min(1, max(0, position(at: now) / duration))
    }

    /// The share of the track that passes each second, while it plays.
    var rate: Double {
        guard playing, let duration, duration > 0 else { return 0 }
        return 1 / duration
    }

    /// The next moment the whole second changes, so the times tick on time.
    var tickStart: Date {
        at.addingTimeInterval(1 - position.truncatingRemainder(dividingBy: 1) + 0.02)
    }
}

/// "3:07", "1:02:09".
func playbackTime(_ seconds: Double) -> String {
    let whole = max(0, Int(seconds.rounded(.down)))
    let hours = whole / 3600, minutes = (whole % 3600) / 60, rest = whole % 60
    return hours > 0 ? String(format: "%d:%02d:%02d", hours, minutes, rest) : String(format: "%d:%02d", minutes, rest)
}

// MARK: - The progress bar

/// The track's progress, dragged to seek. It grows a little under the pointer.
struct ScrubBar: View {
    let clock: PlaybackClock
    let tint: RGB
    let seek: (Double) -> Void
    @State private var hovering = false
    /// 0…1 while dragged, and just after, until the player confirms.
    @State private var scrub: Double?
    @State private var held: Double?
    @Environment(\.inPicture) private var inPicture

    private var thick: Bool { hovering || scrub != nil }

    var body: some View {
        HStack(spacing: 8) {
            TimeText(clock: clock, shown: shownFraction, remaining: false)
            GeometryReader { proxy in
                let width = max(1, proxy.size.width)
                ZStack(alignment: .leading) {
                    if inPicture || shownFraction != nil {
                        StillBar(fraction: shownFraction ?? clock.fraction(at: Date()) ?? 0, tint: tint, thick: thick)
                    }
                    if !inPicture {
                        RunningBar(clock: clock, tint: tint, thick: thick)
                            .opacity(shownFraction == nil ? 1 : 0)
                    }
                    if let fraction = scrub {
                        Circle()
                            .fill(Color.white)
                            .frame(width: 11, height: 11)
                            .shadow(color: .black.opacity(0.35), radius: 2, y: 1)
                            .offset(x: fraction * width - 5.5)
                    }
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in scrub = min(1, max(0, value.location.x / width)) }
                        .onEnded { value in
                            let fraction = min(1, max(0, value.location.x / width))
                            scrub = nil
                            held = fraction
                            if let duration = clock.duration { seek(fraction * duration) }
                            // If the player never confirms, the bar goes back to its own clock.
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { if held == fraction { held = nil } }
                        }
                )
            }
            .frame(height: 12)
            .onHover { hovering = $0 }
            .animation(.spring(response: 0.25, dampingFraction: 0.85), value: thick)
            TimeText(clock: clock, shown: shownFraction, remaining: true)
        }
        .onChange(of: clock) { _ in held = nil }
        .accessibilityElement()
        .accessibilityLabel(L(.position))
        .accessibilityValue(playbackTime(clock.position(at: Date())))
    }

    private var shownFraction: Double? { scrub ?? held }
}

/// Elapsed or remaining time, ticking once a second only while it plays.
private struct TimeText: View {
    let clock: PlaybackClock
    let shown: Double?
    let remaining: Bool

    var body: some View {
        // As wide as the longest time this track can show, so the bar never shifts.
        ZStack(alignment: remaining ? .trailing : .leading) {
            Text((remaining ? "-" : "") + playbackTime(clock.duration ?? 0)).hidden()
            if clock.playing && shown == nil {
                TimelineView(.periodic(from: clock.tickStart, by: 1)) { context in label(clock.position(at: context.date)) }
            } else {
                label(shown.map { $0 * (clock.duration ?? 0) } ?? clock.position(at: Date()))
            }
        }
        .font(.system(size: 10, weight: .medium).monospacedDigit())
        .foregroundStyle(Color.white.opacity(0.5))
        .lineLimit(1)
        .fixedSize()
    }

    private func label(_ position: Double) -> Text {
        guard remaining else { return Text(playbackTime(position)) }
        guard let duration = clock.duration else { return Text("") }
        return Text("-" + playbackTime(duration - position))
    }
}

/// The bar drawn in place: while dragged, and in pictures.
private struct StillBar: View {
    let fraction: Double
    let tint: RGB
    let thick: Bool

    var body: some View {
        GeometryReader { proxy in
            let height: CGFloat = thick ? 7 : 4
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.16))
                Capsule().fill(tint.color).frame(width: max(height, fraction * proxy.size.width))
            }
            .frame(height: height)
            .frame(maxHeight: .infinity)
        }
    }
}

/// The bar while it runs: Core Animation slides the fill to the end over the time left.
private struct RunningBar: NSViewRepresentable {
    let clock: PlaybackClock
    let tint: RGB
    let thick: Bool

    func makeNSView(context: Context) -> RunningBarView { RunningBarView() }

    func updateNSView(_ view: RunningBarView, context: Context) {
        view.show(.init(clock: clock, tint: tint, thick: thick))
    }
}

final class RunningBarView: NSView {
    struct State: Equatable {
        var clock: PlaybackClock
        var tint: RGB
        var thick: Bool
    }

    private let track = CALayer()
    private let fill = CALayer()
    private var state: State?
    private var drawn: (state: State, size: CGSize)?

    override init(frame: NSRect) {
        super.init(frame: frame)
        layer = CALayer()
        wantsLayer = true
        track.masksToBounds = true
        track.backgroundColor = NSColor.white.withAlphaComponent(0.16).cgColor
        fill.anchorPoint = .zero
        track.addSublayer(fill)
        layer?.addSublayer(track)
    }

    required init?(coder: NSCoder) { fatalError("Not used") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func show(_ next: State) {
        let thicker = state.map { $0.thick != next.thick } ?? false
        state = next
        draw(animatingThickness: thicker)
    }

    override func layout() {
        super.layout()
        draw(animatingThickness: false)
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        let scale = window?.backingScaleFactor ?? 2
        track.contentsScale = scale
        fill.contentsScale = scale
    }

    private func draw(animatingThickness: Bool) {
        let size = bounds.size
        guard let state, size.width > 0, size.height > 0 else { return }
        // Asked again with nothing new (the times ticking redraw the card around it): leave the
        // run as it is.
        if let drawn, drawn.state == state, drawn.size == size { return }
        drawn = (state, size)
        let height: CGFloat = state.thick ? 7 : 4
        CATransaction.begin()
        if animatingThickness {
            CATransaction.setAnimationDuration(0.22)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeOut))
        } else {
            CATransaction.setDisableActions(true)
        }
        track.frame = CGRect(x: 0, y: (size.height - height) / 2, width: size.width, height: height)
        track.cornerRadius = height / 2
        fill.bounds = CGRect(x: 0, y: 0, width: size.width, height: height)
        fill.cornerRadius = height / 2
        fill.backgroundColor = NSColor(srgbRed: state.tint.r / 255, green: state.tint.g / 255, blue: state.tint.b / 255, alpha: 1).cgColor
        CATransaction.commit()

        // The fill is as long as the track and slides in from the left, so its rounded end is
        // where the track has got to.
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let fraction = state.clock.fraction(at: Date()) ?? 0
        let start = (fraction - 1) * size.width
        fill.removeAnimation(forKey: "run")
        let rate = state.clock.rate
        if rate > 0, fraction < 1 {
            fill.position = CGPoint(x: 0, y: 0)
            let run = CABasicAnimation(keyPath: "position.x")
            run.fromValue = start
            run.toValue = 0
            run.duration = (1 - fraction) / rate
            run.timingFunction = CAMediaTimingFunction(name: .linear)
            fill.add(run, forKey: "run")
        } else {
            fill.position = CGPoint(x: start, y: 0)
        }
        CATransaction.commit()
    }
}

// MARK: - Level bars

/// Four small bars that dance while it plays and rest when it doesn't.
struct LevelBars: View {
    let playing: Bool
    let tint: RGB
    @Environment(\.inPicture) private var inPicture
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if inPicture || reduceMotion {
                HStack(alignment: .bottom, spacing: 2) {
                    ForEach(Array([0.55, 1.0, 0.7, 0.4].enumerated()), id: \.offset) { item in
                        RoundedRectangle(cornerRadius: 1).fill(tint.color).frame(width: 2.5, height: 12 * (playing ? item.element : 0.25))
                    }
                }
            } else {
                LevelBarsLayer(playing: playing, tint: tint)
            }
        }
        .frame(width: 16, height: 12, alignment: .bottom)
        .accessibilityHidden(true)
    }
}

private struct LevelBarsLayer: NSViewRepresentable {
    let playing: Bool
    let tint: RGB

    func makeNSView(context: Context) -> LevelBarsView { LevelBarsView() }

    func updateNSView(_ view: LevelBarsView, context: Context) {
        view.show(playing: playing, tint: tint)
    }
}

final class LevelBarsView: NSView {
    private var bars: [CALayer] = []
    private var shown: (playing: Bool, tint: RGB)?
    /// Each bar's own loop, so they never move together.
    private static let loops: [(values: [Double], duration: Double)] = [
        ([0.35, 0.9, 0.5, 1.0, 0.3, 0.7, 0.35], 1.05),
        ([0.8, 0.4, 1.0, 0.55, 0.9, 0.3, 0.8], 0.86),
        ([0.5, 1.0, 0.35, 0.75, 0.45, 1.0, 0.5], 1.18),
        ([0.3, 0.65, 0.9, 0.35, 0.8, 0.5, 0.3], 0.94),
    ]

    override init(frame: NSRect) {
        super.init(frame: frame)
        layer = CALayer()
        wantsLayer = true
        for _ in Self.loops {
            let bar = CALayer()
            bar.anchorPoint = CGPoint(x: 0.5, y: 0)
            bar.cornerRadius = 1
            layer?.addSublayer(bar)
            bars.append(bar)
        }
    }

    required init?(coder: NSCoder) { fatalError("Not used") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let width: CGFloat = 2.5, gap: CGFloat = 1.8
        for (index, bar) in bars.enumerated() {
            bar.bounds = CGRect(x: 0, y: 0, width: width, height: bounds.height)
            bar.position = CGPoint(x: CGFloat(index) * (width + gap) + width / 2, y: 0)
        }
        CATransaction.commit()
    }

    func show(playing: Bool, tint: RGB) {
        if let shown, shown.playing == playing, shown.tint == tint { return }
        shown = (playing, tint)
        let color = NSColor(srgbRed: tint.r / 255, green: tint.g / 255, blue: tint.b / 255, alpha: 1).cgColor
        for (index, bar) in bars.enumerated() {
            bar.backgroundColor = color
            let current = (bar.presentation()?.value(forKeyPath: "transform.scale.y") as? Double) ?? 0.25
            bar.removeAnimation(forKey: "dance")
            if playing {
                let loop = Self.loops[index]
                let dance = CAKeyframeAnimation(keyPath: "transform.scale.y")
                dance.values = loop.values
                dance.duration = loop.duration
                dance.calculationMode = .cubic
                dance.repeatCount = .infinity
                bar.setValue(loop.values[0], forKeyPath: "transform.scale.y")
                bar.add(dance, forKey: "dance")
            } else {
                // Settle down from wherever it was.
                let rest = CABasicAnimation(keyPath: "transform.scale.y")
                rest.fromValue = current
                rest.toValue = 0.25
                rest.duration = 0.3
                bar.setValue(0.25, forKeyPath: "transform.scale.y")
                bar.add(rest, forKey: "rest")
            }
        }
    }
}

// MARK: - Buttons

/// A plain symbol button: shuffle, repeat, skip. `on` marks a mode that's switched on.
struct GlyphButton: View {
    let symbol: String
    var size: CGFloat = 13
    var on: Bool?
    let tint: RGB
    let label: String
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size, weight: .semibold))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(on == true ? tint.color : Color.white.opacity(on == false ? 0.5 : 0.92))
                .frame(width: 26, height: 26)
                .background(Circle().fill(Color.white.opacity(hovering ? 0.1 : 0)))
                .overlay(alignment: .bottom) {
                    if on == true { Circle().fill(tint.color).frame(width: 3, height: 3).offset(y: 2) }
                }
                .contentShape(Circle())
        }
        .buttonStyle(PressStyle())
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.15), value: hovering)
        .animation(.spring(response: 0.3, dampingFraction: 0.7), value: on)
        .accessibilityLabel(label)
        .accessibilityAddTraits(on == true ? .isSelected : [])
    }
}

/// Play and pause, on glass where the Mac has it.
struct PlayButton: View {
    let playing: Bool
    let tint: RGB
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: playing ? "pause.fill" : "play.fill")
                .font(.system(size: 15, weight: .bold))
                .foregroundStyle(Color.white)
                .modifier(SymbolSwap())
                .frame(width: 30, height: 30)
                .modifier(PlayBackground(tint: tint))
                .contentShape(Circle())
        }
        .buttonStyle(PressStyle())
        .accessibilityLabel(playing ? L(.pauseMedia) : L(.play))
    }
}

private struct SymbolSwap: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 14.0, *) {
            content.contentTransition(.symbolEffect(.replace))
        } else {
            content
        }
    }
}

private struct PlayBackground: ViewModifier {
    let tint: RGB
    @Environment(\.inPicture) private var inPicture

    func body(content: Content) -> some View {
        if #available(macOS 26.0, *), !inPicture {
            content.glassEffect(.regular.tint(tint.opacity(0.5)).interactive(), in: Circle())
        } else {
            content.background(
                Circle()
                    .fill(LinearGradient(colors: [tint.opacity(0.95), tint.mix(RGB(0, 0, 0), 0.3).opacity(0.95)], startPoint: .top, endPoint: .bottom))
                    .overlay(Circle().strokeBorder(Color.white.opacity(0.2), lineWidth: 0.5))
            )
        }
    }
}

private struct PressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.86 : 1)
            .animation(.spring(response: 0.2, dampingFraction: 0.6), value: configuration.isPressed)
    }
}

// MARK: - Volume

/// The player's own volume: a speaker that shows the level and a short bar to drag.
struct VolumeBar: View {
    let volume: Double
    let tint: RGB
    let change: (Double) -> Void
    @State private var dragged: Double?
    @State private var dragging = false
    @State private var sentAt = Date.distantPast
    @State private var hovering = false

    private var level: Double { dragged ?? volume }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(Color.white.opacity(0.7))
                .frame(width: 16, alignment: .trailing)
            GeometryReader { proxy in
                let width = max(1, proxy.size.width)
                let height: CGFloat = hovering || dragged != nil ? 6 : 4
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.white.opacity(0.16))
                    Capsule().fill(Color.white.opacity(0.85)).frame(width: max(height, level * width))
                }
                .frame(height: height)
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            let next = min(1, max(0, value.location.x / width))
                            dragging = true
                            dragged = next
                            // Heard while dragging, without flooding the player.
                            if Date().timeIntervalSince(sentAt) > 0.12 {
                                sentAt = Date()
                                change(next)
                            }
                        }
                        .onEnded { value in
                            let next = min(1, max(0, value.location.x / width))
                            dragging = false
                            dragged = next
                            change(next)
                            // If the player never confirms, the bar goes back to what it said.
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) { if !dragging { dragged = nil } }
                        }
                )
            }
            .frame(width: 62, height: 12)
            .onHover { hovering = $0 }
            .animation(.spring(response: 0.25, dampingFraction: 0.85), value: hovering || dragged != nil)
        }
        .onChange(of: volume) { _ in if !dragging { dragged = nil } }
        .accessibilityElement()
        .accessibilityLabel(L(.volume))
        .accessibilityValue("\(Int((level * 100).rounded()))%")
    }

    private var symbol: String {
        switch level {
        case ..<0.01: return "speaker.slash.fill"
        case ..<0.34: return "speaker.wave.1.fill"
        case ..<0.67: return "speaker.wave.2.fill"
        default: return "speaker.wave.3.fill"
        }
    }
}
