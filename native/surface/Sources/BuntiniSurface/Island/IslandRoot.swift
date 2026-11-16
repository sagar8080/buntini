import SwiftUI
import SurfaceKit

/// Square where it meets the top of the screen; rounded below.
struct IslandShape: Shape {
    var bottomRadius: CGFloat

    var animatableData: CGFloat {
        get { bottomRadius }
        set { bottomRadius = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let radius = min(bottomRadius, rect.width / 2, rect.height)
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY - radius))
        path.addArc(center: CGPoint(x: rect.maxX - radius, y: rect.maxY - radius), radius: radius, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        path.addLine(to: CGPoint(x: rect.minX + radius, y: rect.maxY))
        path.addArc(center: CGPoint(x: rect.minX + radius, y: rect.maxY - radius), radius: radius, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false)
        path.closeSubpath()
        return path
    }
}

struct IslandRoot: View {
    @ObservedObject var model: IslandModel
    let send: (Action) -> Void
    @Namespace private var space
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        let mode = model.mode
        let size = model.size
        let radius: CGFloat = mode == .open ? 26 : mode == .peek ? model.band / 2 : 8
        ZStack(alignment: .top) {
            if !inPicture {
                GlowView(island: size, held: model.scene?.held == true && mode == .open, tint: model.scene?.cat.accent ?? "#F2B33D")
                    .frame(width: IslandGeometry.panel.width, height: IslandGeometry.panel.height)
                    .allowsHitTesting(false)
            }
            island(mode: mode)
                .frame(width: size.width, height: size.height, alignment: .top)
                .background(background(mode: mode, radius: radius))
                .clipShape(IslandShape(bottomRadius: radius))
                .opacity(mode == .hidden ? 0 : 1)
        }
        .frame(width: IslandGeometry.panel.width, height: IslandGeometry.panel.height, alignment: .top)
        .animation(reduceMotion ? .easeOut(duration: 0.15) : mode == .open ? .spring(response: 0.5, dampingFraction: 0.72) : .timingCurve(0.45, 0, 0.2, 1, duration: 0.34), value: mode)
        .animation(.spring(response: 0.4, dampingFraction: 0.8), value: size)
    }

    /// Black at the notch, so it blends with the hardware; glass elsewhere on macOS 26.
    @ViewBuilder
    private func background(mode: IslandMode, radius: CGFloat) -> some View {
        if model.hasNotch {
            Color.black
        } else if #available(macOS 26.0, *) {
            Color.black.opacity(0.35).glassEffect(.regular, in: IslandShape(bottomRadius: radius))
        } else {
            Color.black.opacity(0.85).background(.ultraThinMaterial)
        }
    }

    @ViewBuilder
    private func island(mode: IslandMode) -> some View {
        let scene = model.scene
        VStack(spacing: 0) {
            // The top band: the cat on the left wing, pills on the right.
            HStack(spacing: 0) {
                if mode == .peek, let cat = scene?.cat {
                    CatView(cat: cat, active: !model.asleep)
                        .contextMenu { Wardrobe(cat: cat, send: send) }
                        .matchedGeometryEffect(id: "cat", in: space)
                        .frame(width: model.band - 2, height: model.band - 2)
                        .padding(.leading, 16)
                }
                Spacer(minLength: 0)
                if let scene, mode != .hidden {
                    PillDots(pills: scene.pills, overflow: scene.overflow.count)
                        .padding(.trailing, 16)
                }
            }
            .frame(height: model.band)
            .contentShape(Rectangle())
            .onTapGesture {
                if scene?.opens != true { model.expanded.toggle() }
            }
            if mode == .open {
                HStack(alignment: .top, spacing: 14) {
                    if let cat = scene?.cat {
                        CatView(cat: cat, active: !model.asleep)
                            .contextMenu { Wardrobe(cat: cat, send: send) }
                            .matchedGeometryEffect(id: "cat", in: space)
                            .frame(width: 64, height: 68)
                    }
                    content(scene)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 18)
                .padding(.top, 8)
                .padding(.bottom, 16)
                .transition(.opacity.combined(with: .scale(scale: 0.97, anchor: .top)))
            }
        }
        .foregroundStyle(.white)
    }

    @ViewBuilder
    private func content(_ scene: SurfaceScene?) -> some View {
        if model.dragging, scene?.card == nil {
            DropTarget()
        } else if let card = scene?.card {
            CardView(card: card, send: send)
                .id(card.id)
                .transition(.opacity.combined(with: .scale(scale: 0.97, anchor: .top)))
        } else if let scene {
            Home(pills: scene.pills, send: send)
        }
    }
}

/// The cat's wardrobe, on a right-click: the same choices as Settings.
struct Wardrobe: View {
    let cat: SurfaceKit.Cat
    let send: (Action) -> Void

    private static let coats = [("ginger", "Ginger tabby"), ("gray", "Silver tabby"), ("black", "Midnight"), ("white", "Snowball"), ("tuxedo", "Tuxedo"), ("calico", "Calico"), ("siamese", "Siamese")]
    private static let eyes = [("amber", "Amber"), ("green", "Green"), ("blue", "Blue"), ("copper", "Copper"), ("odd", "Odd-eyed")]
    private static let accessories = [("none", "None"), ("bell", "Bell collar"), ("bow", "Bow"), ("bandana", "Bandana"), ("hat", "Top hat"), ("flowers", "Flowers")]

    private func choose(_ key: String, _ value: String) {
        send(Action(source: .island, card: nil, name: "look", value: .object(["key": .string(key), "value": .string(value)])))
    }

    private func menu(_ title: String, _ key: String, _ items: [(String, String)], _ current: String) -> some View {
        Menu(title) {
            ForEach(items, id: \.0) { item in
                Button { choose(key, item.0) } label: {
                    if item.0 == current { Label(item.1, systemImage: "checkmark") } else { Text(item.1) }
                }
            }
        }
    }

    var body: some View {
        menu("Coat", "catCoat", Self.coats, cat.coat)
        menu("Eyes", "catEyes", Self.eyes, cat.eyes)
        menu("Accessory", "catAccessory", Self.accessories, cat.accessory)
    }
}

/// Up to four tinted dots, then "+N".
struct PillDots: View {
    let pills: [Pill]
    let overflow: Int

    var body: some View {
        HStack(spacing: 5) {
            ForEach(pills.prefix(4)) { pill in
                Circle()
                    .fill(Color(hex: pill.tint))
                    .frame(width: 7, height: 7)
                    .overlay(Circle().stroke(.white.opacity(pill.state == .waiting ? 0.9 : 0), lineWidth: 1.5).frame(width: 11, height: 11))
                    .accessibilityLabel(pill.label)
            }
            if overflow > 0 { Text("+\(overflow)").font(.system(size: 10, weight: .semibold)).opacity(0.75) }
        }
    }
}

/// Opened with nothing to answer: what is running, and the way to the chat.
struct Home: View {
    let pills: [Pill]
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(pills) { pill in
                Button {
                    send(Action(source: pill.source, card: nil, name: "pill", value: .string(pill.id)))
                } label: {
                    HStack(spacing: 8) {
                        Circle().fill(Color(hex: pill.tint)).frame(width: 8, height: 8)
                        Text(pill.label).font(.callout.weight(.medium)).lineLimit(1)
                        if let badge = pill.badge { Text(badge).font(.caption).opacity(0.6).lineLimit(1) }
                        Spacer(minLength: 0)
                    }
                    .frame(height: 24)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            HStack {
                Spacer()
                IslandButton(label: "Chat", prominent: true) { send(Action(source: .island, card: nil, name: "openChat")) }
            }
        }
    }
}

struct DropTarget: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 14)
            .strokeBorder(.white.opacity(0.5), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5]))
            .overlay(Image(systemName: "tray.and.arrow.down").font(.title2).opacity(0.8))
            .frame(height: 70)
            .accessibilityLabel("Drop files")
    }
}

/// Picks the view for a card. Each part of Buntini owns its views in Cards/.
struct CardView: View {
    let card: Card
    let send: (Action) -> Void

    var body: some View {
        switch card.body {
        case .approval(let value): ApprovalCardView(card: card, approval: value, send: send)
        case .question(let value): QuestionCardView(card: card, question: value, send: send)
        case .ticker(let value): TickerCardView(card: card, ticker: value, send: send)
        case .diff(let value): DiffCardView(diff: value)
        case .finished(let value): FinishedCardView(finished: value)
        case .recap(let value): RecapCardView(card: card, recap: value, send: send)
        case .usage(let value): UsageCardView(usage: value)
        case .service(let value): ServiceCardView(card: card, service: value, send: send)
        case .mail(let value): MailCardView(card: card, mail: value, send: send)
        case .nowPlaying(let value): NowPlayingCardView(card: card, playing: value, send: send)
        case .timer(let value): TimerCardView(card: card, timer: value, send: send)
        case .capture(let value): CaptureCardView(card: card, capture: value, send: send)
        case .vitals(let value): VitalsCardView(vitals: value)
        case .update(let value): UpdateCardView(card: card, update: value, send: send)
        case .whatsNew(let value): WhatsNewCardView(whatsNew: value)
        case .drop(let value): DropCardView(card: card, drop: value, send: send)
        case .greeting(let value): GreetingCardView(greeting: value)
        case .note(let value): NoteCardView(card: card, note: value, send: send)
        }
    }
}

extension Color {
    /// `#rrggbb`; grey when it doesn't parse.
    init(hex: String) {
        self = RGB(hex: hex).color
    }
}
