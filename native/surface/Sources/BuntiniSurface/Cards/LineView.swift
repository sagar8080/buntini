// A card that has one thing to say, as a single line under the band for its moment: a finished
// turn, a hello, how the Mac is doing, being up to date. Its full card is a chip away once the
// island is opened, and the session's mark stays beside the notch until then.
import SwiftUI
import SurfaceKit

struct LineView: View {
    let card: Card

    /// The room below the band.
    static let height = CardSteps.line

    var body: some View {
        HStack(spacing: 8) {
            if let (symbol, tint) = sign {
                Image(systemName: symbol).font(.system(size: 13, weight: .semibold)).foregroundStyle(tint)
            }
            Text(headline).font(.system(size: 13, weight: .semibold)).lineLimit(1).layoutPriority(1)
            if let detail {
                Text(detail).font(.system(size: 12)).opacity(0.6).lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .frame(height: Self.height)
        .accessibilityElement(children: .combine)
    }

    private var sign: (String, Color)? {
        switch card.body {
        case .finished: return ("checkmark.circle.fill", Color(hex: "#7EE787"))
        case .vitals(let vitals): return (VitalsCardView.symbol(vitals.line), Color.white.opacity(0.85))
        case .update: return ("checkmark.circle.fill", Color(red: 0.36, green: 0.86, blue: 0.6))
        default: return nil
        }
    }

    private var headline: String {
        switch card.body {
        case .finished(let finished): return finished.message ?? L(.done)
        case .greeting(let greeting): return greeting.name.map { L(.hiName, ["name": $0]) } ?? L(.hi)
        case .vitals(let vitals): return vitals.line
        case .update: return L(.updateCurrent)
        default: return ""
        }
    }

    private var detail: String? {
        switch card.body {
        case .finished(let finished): return from(finished.agent, finished.project)
        case .update(let update): return "Buntini \(update.version)"
        default: return nil
        }
    }
}
