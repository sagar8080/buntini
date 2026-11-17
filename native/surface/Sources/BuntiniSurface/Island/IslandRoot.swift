import SwiftUI
import SurfaceKit

/// Square where it meets the top of the screen and rounded below. At the notch its top
/// corners flare out into the edge of the screen, the way the hardware's own corners do.
struct IslandShape: Shape {
    var bottomRadius: CGFloat
    var shoulder: CGFloat = 0

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(bottomRadius, shoulder) }
        set {
            bottomRadius = newValue.first
            shoulder = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        let flare = max(0, min(shoulder, rect.height / 2))
        let radius = max(0, min(bottomRadius, rect.width / 2, rect.height - flare))
        var path = Path()
        path.move(to: CGPoint(x: rect.minX - flare, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX + flare, y: rect.minY))
        if flare > 0 { path.addQuadCurve(to: CGPoint(x: rect.maxX, y: rect.minY + flare), control: CGPoint(x: rect.maxX, y: rect.minY)) }
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY - radius))
        path.addArc(center: CGPoint(x: rect.maxX - radius, y: rect.maxY - radius), radius: radius, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        path.addLine(to: CGPoint(x: rect.minX + radius, y: rect.maxY))
        path.addArc(center: CGPoint(x: rect.minX + radius, y: rect.maxY - radius), radius: radius, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false)
        path.addLine(to: CGPoint(x: rect.minX, y: rect.minY + flare))
        if flare > 0 { path.addQuadCurve(to: CGPoint(x: rect.minX - flare, y: rect.minY), control: CGPoint(x: rect.minX, y: rect.minY)) }
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
        let rect = model.localRect
        let corner = model.inCorner
        let notch = model.hasNotch && !corner
        let radius: CGFloat = corner ? (mode == .open ? 22 : 18) : mode == .open ? 24 : mode == .peek ? model.band / 2 : 8
        let flare: CGFloat = notch ? (mode == .open ? 10 : mode == .peek ? 7 : 0) : 0
        let shape = corner ? AnyShape(RoundedRectangle(cornerRadius: radius, style: .continuous)) : AnyShape(IslandShape(bottomRadius: radius, shoulder: flare))
        ZStack(alignment: .topLeading) {
            if !inPicture {
                GlowView(rect: rect, hangs: !corner, held: model.scene?.held == true && mode == .open, tint: model.scene?.cat.accent ?? "#F2B33D")
                    .frame(width: IslandGeometry.panel.width, height: IslandGeometry.panel.height)
                    .allowsHitTesting(false)
            }
            island(mode: mode)
                .frame(width: rect.width, height: rect.height, alignment: .top)
                .background(background(shape: shape))
                .clipShape(shape)
                .shadow(color: .black.opacity(corner ? 0.35 : 0), radius: 14, y: 6)
                .opacity(mode == .hidden && !corner ? 0 : 1)
                .offset(x: rect.minX, y: rect.minY)
            // Resting on the island: what it shows, in words, or what the Mac needs.
            if model.hover, mode == .peek, let text = model.scene?.line ?? model.glance?.caption {
                HealthLine(text: text)
                    .offset(x: rect.minX, y: model.placement == .bottomLeft || model.placement == .bottomRight ? rect.minY - 30 : rect.maxY + 8)
                    .frame(width: rect.width, alignment: .center)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(width: IslandGeometry.panel.width, height: IslandGeometry.panel.height, alignment: .topLeading)
        .animation(reduceMotion ? .easeOut(duration: 0.15) : mode == .open ? .spring(response: 0.5, dampingFraction: 0.72) : .timingCurve(0.45, 0, 0.2, 1, duration: 0.34), value: mode)
        .animation(reduceMotion ? .easeOut(duration: 0.15) : .spring(response: 0.4, dampingFraction: 0.8), value: rect)
        .animation(.easeOut(duration: 0.2), value: model.hover)
    }

    /// Black at the notch, so it blends with the hardware; glass elsewhere on macOS 26.
    @ViewBuilder
    private func background(shape: AnyShape) -> some View {
        if model.hasNotch && !model.inCorner {
            shape.fill(Color.black)
        } else if #available(macOS 26.0, *) {
            Color.black.opacity(0.35).glassEffect(.regular, in: shape)
        } else {
            Color.black.opacity(0.85).background(.ultraThinMaterial)
        }
    }

    /// The cat, which can be picked up and carried to a corner or back to the top.
    private func cat(_ cat: SurfaceKit.Cat) -> some View {
        CatView(cat: cat, active: !model.asleep)
            .contextMenu { Wardrobe(cat: cat, send: send) }
            .matchedGeometryEffect(id: "cat", in: space)
            .opacity(model.carrying ? 0 : 1)
            .gesture(
                DragGesture(minimumDistance: 6, coordinateSpace: .global)
                    .onChanged { _ in
                        if !model.carrying { model.carrying = true }
                        model.carry(.moved)
                    }
                    .onEnded { _ in model.carry(.ended) }
            )
            .accessibilityAddTraits(.isButton)
    }

    @ViewBuilder
    private func island(mode: IslandMode) -> some View {
        let scene = model.scene
        let corner = model.inCorner
        let trailing = model.placement == .topRight || model.placement == .bottomRight
        // The cat steps out of its wing only to sit beside a hello, a finished turn or files.
        let beside = mode == .open && model.interrupting.map(IslandModel.catBeside) == true
        let catSide: CGFloat = corner ? (mode == .open ? 26 : IslandGeometry.tile.width - 10) : model.band - 2
        VStack(spacing: 0) {
            // The top band: the cat in one wing, what's running in the other. In a corner
            // the cat rests in its tile, nearest the corner.
            HStack(spacing: corner ? 10 : 0) {
                if trailing {
                    glance(mode: mode).padding(.leading, 9)
                    Spacer(minLength: 0)
                }
                if !beside, let cat = scene?.cat {
                    self.cat(cat)
                        .frame(width: catSide, height: catSide)
                        .padding(.leading, corner ? (trailing ? 0 : (mode == .open ? 14 : 5)) : 16)
                        .padding(.trailing, corner && trailing ? (mode == .open ? 14 : 5) : 0)
                }
                if !trailing {
                    Spacer(minLength: 0)
                    glance(mode: mode).padding(.trailing, corner ? 9 : 14)
                }
            }
            .frame(height: corner ? (mode == .open ? 40 : IslandGeometry.tile.height) : model.band)
            .contentShape(Rectangle())
            .onTapGesture {
                if model.interrupting == nil { model.expanded.toggle() }
            }
            if mode == .open {
                opened(beside: beside)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .inLanguage()
                    .padding(.horizontal, 18)
                    .padding(.top, corner ? 0 : 10)
                    .padding(.bottom, 16)
                    .transition(.opacity.combined(with: .scale(scale: 0.97, anchor: .top)))
            }
        }
        .foregroundStyle(.white)
    }

    /// The wing's readout while peeking; nothing once opened, where the chips say it.
    @ViewBuilder
    private func glance(mode: IslandMode) -> some View {
        if mode == .peek, let glance = model.glance {
            GlanceView(glance: glance).transition(.opacity)
        }
    }

    @ViewBuilder
    private func opened(beside: Bool) -> some View {
        if model.dragging, model.scene?.card == nil {
            DropTarget()
        } else if let card = model.interrupting {
            if beside, let cat = model.scene?.cat {
                HStack(alignment: .top, spacing: 14) {
                    self.cat(cat).frame(width: 64, height: 68)
                    CardView(card: card, send: send).frame(maxWidth: .infinity, alignment: .leading)
                }
                .id(card.id)
            } else {
                CardView(card: card, send: send)
                    .id(card.id)
                    .transition(.opacity.combined(with: .scale(scale: 0.97, anchor: .top)))
            }
        } else if model.scene != nil {
            Home(model: model, width: model.size.width - 36, send: send)
        }
    }
}

/// One line about the Mac, under the island while the pointer is on it.
struct HealthLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .foregroundStyle(.white)
            .padding(.horizontal, 10)
            .frame(height: 22)
            .background(Capsule().fill(.black.opacity(0.78)))
            .fixedSize()
            .accessibilityLabel(text)
    }
}

/// The cat's wardrobe, on a right-click: the same choices as Settings.
struct Wardrobe: View {
    let cat: SurfaceKit.Cat
    let send: (Action) -> Void

    private static var coats: [(String, String)] { [("ginger", L(.coatGinger)), ("gray", L(.coatGray)), ("black", L(.coatBlack)), ("white", L(.coatWhite)), ("tuxedo", L(.coatTuxedo)), ("calico", L(.coatCalico)), ("siamese", L(.coatSiamese))] }
    private static var eyes: [(String, String)] { [("amber", L(.eyesAmber)), ("green", L(.eyesGreen)), ("blue", L(.eyesBlue)), ("copper", L(.eyesCopper)), ("odd", L(.eyesOdd))] }
    private static var accessories: [(String, String)] { [("none", L(.accessoryNone)), ("bell", L(.accessoryBell)), ("bow", L(.accessoryBow)), ("bandana", L(.accessoryBandana)), ("hat", L(.accessoryHat)), ("flowers", L(.accessoryFlowers))] }

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
        menu(L(.coat), "catCoat", Self.coats, cat.coat)
        menu(L(.eyes), "catEyes", Self.eyes, cat.eyes)
        menu(L(.accessory), "catAccessory", Self.accessories, cat.accessory)
    }
}

/// Opened by a click: a question for Buntini and the way to the chat, then a chip for each
/// thing running and the chosen one's card.
struct Home: View {
    @ObservedObject var model: IslandModel
    let width: CGFloat
    let send: (Action) -> Void
    @State private var question = ""
    @FocusState private var focused: Bool
    @Environment(\.inPicture) private var inPicture

    var body: some View {
        let chips = model.chips
        let selected = model.selected
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                field
                    .font(.callout)
                    .padding(.horizontal, 12)
                    .frame(height: 30)
                    .background(Capsule().fill(.white.opacity(0.1)))
                UtilityHomeButtons(pills: model.scene?.pills ?? [], send: send)
                IslandButton(label: L(.chat), prominent: question.isEmpty) { send(Action(source: .island, card: nil, name: "openChat")) }
            }
            if !chips.isEmpty {
                ChipRow(chips: chips, selected: selected?.id, width: width, choose: choose)
            }
            if let card = selected {
                CardView(card: card, send: send)
                    .id(card.id)
                    .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 0.18), value: selected?.id)
        .onChange(of: focused) { now in model.typing = now }
        .onDisappear { model.typing = false }
    }

    private func choose(_ chip: Chip) {
        if let card = chip.card, model.selected?.id != card.id {
            model.chosen = card.id
        } else if let pill = chip.pill {
            send(Action(source: pill.source, card: nil, name: "pill", value: .string(pill.id)))
        }
    }

    /// A picture can't hold a live text field, so it gets the field's look.
    @ViewBuilder private var field: some View {
        if inPicture {
            Text(L(.askBuntini)).foregroundStyle(.white.opacity(0.4)).frame(maxWidth: .infinity, alignment: .leading)
        } else {
            TextField(L(.askBuntini), text: $question)
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit(ask)
                .onExitCommand {
                    focused = false
                    model.expanded = false
                }
        }
    }

    private func ask() {
        let text = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        send(Action(source: .island, card: nil, name: "ask", value: .string(text)))
        question = ""
        focused = false
        model.expanded = false
    }
}

struct DropTarget: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 14)
            .strokeBorder(.white.opacity(0.5), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5]))
            .overlay(Image(systemName: "tray.and.arrow.down").font(.title2).opacity(0.8))
            .frame(height: 70)
            .accessibilityLabel(L(.dropFiles))
    }
}

/// Picks the view for a card. Each part of Buntini owns its views in Cards/. Nothing a card
/// sends counts in the moment after it comes forward (`Arming`): that click was aimed at
/// whatever was there before.
struct CardView: View {
    let card: Card
    let send: (Action) -> Void
    @State private var shown = Date()

    var body: some View {
        views(send: { action in
            if Arming.ready(since: shown) { send(action) }
        })
        .environment(\.armedSince, shown)
        .onChange(of: card.id) { _ in shown = Date() }
    }

    @ViewBuilder
    private func views(send: @escaping (Action) -> Void) -> some View {
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
