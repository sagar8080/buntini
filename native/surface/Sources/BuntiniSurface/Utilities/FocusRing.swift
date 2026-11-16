// The focus timer's ring: what's left of the phase as an arc in the phase's colours, with a
// soft glow and a bright head. Drawn once per second at most, and not at all while paused.
import SwiftUI

struct FocusRing: View {
    /// 0…1 of the phase left.
    let left: Double
    let colors: [Color]
    var line: CGFloat = 6
    var dimmed = false

    var body: some View {
        Canvas { context, size in
            let side = min(size.width, size.height)
            let center = CGPoint(x: size.width / 2, y: size.height / 2)
            let radius = side / 2 - line / 2 - 3
            var track = Path()
            track.addArc(center: center, radius: radius, startAngle: .zero, endAngle: .degrees(360), clockwise: false)
            context.stroke(track, with: .color(.white.opacity(0.1)), lineWidth: line)

            let left = min(max(left, 0), 1)
            guard left > 0.001, let first = colors.first, let last = colors.last else { return }
            let start = Angle.degrees(-90)
            let end = Angle.degrees(-90 + 360 * left)
            var arc = Path()
            arc.addArc(center: center, radius: radius, startAngle: start, endAngle: end, clockwise: false)
            let gradient = Gradient(stops: [.init(color: first, location: 0), .init(color: last, location: max(left, 0.01))])
            let point = { (angle: Angle) in CGPoint(x: center.x + radius * cos(angle.radians), y: center.y + radius * sin(angle.radians)) }
            context.drawLayer { layer in
                layer.opacity = dimmed ? 0.45 : 1
                layer.addFilter(.shadow(color: last.opacity(dimmed ? 0 : 0.55), radius: line * 0.9))
                layer.stroke(arc, with: .conicGradient(gradient, center: center, angle: start), style: StrokeStyle(lineWidth: line, lineCap: .round))
                // The start's round cap, in the first colour where the gradient wraps.
                let cap = point(start)
                layer.fill(Path(ellipseIn: CGRect(x: cap.x - line / 2, y: cap.y - line / 2, width: line, height: line)), with: .color(first))
            }
            guard left < 0.999 else { return }
            let head = point(end)
            let knob = line * 0.36
            context.fill(Path(ellipseIn: CGRect(x: head.x - knob, y: head.y - knob, width: 2 * knob, height: 2 * knob)), with: .color(.white.opacity(dimmed ? 0.5 : 0.95)))
        }
    }

    static func colors(_ phase: String?) -> [Color] {
        switch phase {
        case "rest": return [RGB(hex: "#A6F4D6").color, RGB(hex: "#2FBF95").color]
        case "longRest": return [RGB(hex: "#B5DBFF").color, RGB(hex: "#5B8CFF").color]
        case "done": return [RGB(hex: "#FFE49A").color, RGB(hex: "#F2B33D").color]
        default: return [RGB(hex: "#FFC07A").color, RGB(hex: "#FF5F57").color]
        }
    }
}

/// "24:59", or "1:04:59" past an hour.
func clockText(_ seconds: Double) -> String {
    let total = max(0, Int((seconds - 0.05).rounded(.up)))
    let (hours, minutes, rest) = (total / 3600, total / 60 % 60, total % 60)
    return hours > 0 ? String(format: "%d:%02d:%02d", hours, minutes, rest) : String(format: "%d:%02d", minutes, rest)
}
