import SwiftUI
import SurfaceKit

struct IslandRoot: View {
    @ObservedObject var store: SceneStore
    let send: (Action) -> Void

    var body: some View {
        VStack(spacing: 0) {
            if let scene = store.scene, (scene.card != nil && scene.opens) || !scene.pills.isEmpty {
                IslandContent(scene: scene, send: send)
            }
            Spacer(minLength: 0)
        }
        .frame(width: IslandPanel.size.width, height: IslandPanel.size.height, alignment: .top)
    }
}

struct IslandContent: View {
    let scene: SurfaceScene
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !scene.pills.isEmpty {
                HStack(spacing: 6) {
                    ForEach(scene.pills) { pill in PillChip(pill: pill) }
                    if !scene.overflow.isEmpty { Text("+\(scene.overflow.count)").font(.caption2.weight(.semibold)) }
                }
            }
            if let card = scene.card, scene.opens { CardView(card: card, send: send) }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 18)
        .padding(.top, 14)
        .padding(.bottom, 16)
        .frame(width: 520, alignment: .leading)
        .background(IslandShape(bottomRadius: 24).fill(.black))
    }
}

struct PillChip: View {
    let pill: Pill

    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(Color(hex: pill.tint)).frame(width: 8, height: 8)
            Text(pill.label).font(.caption.weight(.medium)).lineLimit(1)
            if let badge = pill.badge { Text(badge).font(.caption2).opacity(0.7) }
        }
        .padding(.horizontal, 8)
        .frame(height: 24)
        .background(Capsule().fill(.white.opacity(0.1)))
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

/// Square at the top, where it meets the screen edge; rounded below.
struct IslandShape: Shape {
    var bottomRadius: CGFloat

    var animatableData: CGFloat {
        get { bottomRadius }
        set { bottomRadius = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let radius = min(bottomRadius, rect.width / 2, rect.height / 2)
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY - radius))
        path.addQuadCurve(to: CGPoint(x: rect.maxX - radius, y: rect.maxY), control: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX + radius, y: rect.maxY))
        path.addQuadCurve(to: CGPoint(x: rect.minX, y: rect.maxY - radius), control: CGPoint(x: rect.minX, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}

extension Color {
    /// `#rrggbb`; grey when it doesn't parse.
    init(hex: String) {
        let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard digits.count == 6, let value = UInt32(digits, radix: 16) else {
            self = .gray
            return
        }
        self = Color(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }
}
