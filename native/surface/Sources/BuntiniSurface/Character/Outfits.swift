// What the cat wears on its own for the season (island::season in Rust): a Santa hat in
// December, a witch's hat at Halloween, a party hat at New Year and hearts on Valentine's Day.
import SwiftUI

extension CatFace {
    func drawOutfit(_ context: inout GraphicsContext, _ outfit: String) {
        switch outfit {
        case "santa":
            var hat = context
            hat.rotate(around: CGPoint(x: 90, y: 60), by: .degrees(-12))
            hat.fill(SVGPath.path("M66 64C70 46 84 32 102 30C112 30 118 36 121 44C112 42 104 46 100 56L112 64Z"), with: .linearGradient(Gradient(colors: [RGB(hex: "#E5484D").color, RGB(hex: "#B5262B").color]), startPoint: CGPoint(x: 80, y: 30), endPoint: CGPoint(x: 100, y: 64)))
            hat.fill(Path(roundedRect: CGRect(x: 60, y: 58, width: 58, height: 11), cornerRadius: 5.5), with: .color(.white))
            hat.fill(Path(ellipseIn: CGRect(x: 116, y: 39, width: 11, height: 11)), with: .color(.white))
        case "witch":
            var hat = context
            hat.rotate(around: CGPoint(x: 90, y: 60), by: .degrees(-8))
            let felt = RGB(hex: "#3B2A4F").color
            hat.fill(Path(ellipseIn: CGRect(x: 56, y: 56, width: 70, height: 12)), with: .color(felt))
            hat.fill(SVGPath.path("M74 61L92 22C94 18 98 18 99 23L108 61Z"), with: .color(felt))
            hat.fill(Path(CGRect(x: 75, y: 54, width: 33, height: 4)), with: .color(RGB(hex: "#F28C28").color))
        case "party":
            var hat = context
            hat.rotate(around: CGPoint(x: 104, y: 52), by: .degrees(16))
            let cone = SVGPath.path("M92 64L104 26L116 64Z")
            hat.fill(cone, with: .color(RGB(hex: "#6DB6E8").color))
            hat.clip(to: cone)
            for y in stride(from: 30.0, to: 66.0, by: 9.0) {
                hat.fill(Path(CGRect(x: 88, y: y, width: 32, height: 3.5)), with: .color(RGB(hex: "#FFD45C").color))
            }
            context.fill(Path(ellipseIn: CGRect(x: 108, y: 20, width: 8, height: 8)).applying(CGAffineTransform(translationX: 0, y: 0)), with: .color(RGB(hex: "#F08AA7").color))
        case "hearts":
            let heart = SVGPath.path("M0 3C-1.5 0-6-.5-6 3C-6 6.5-1.5 8.5 0 10C1.5 8.5 6 6.5 6 3C6-.5 1.5 0 0 3Z")
            for (x, y, scale) in [(62.0, 50.0, 0.9), (118.0, 46.0, 1.1), (132.0, 70.0, 0.75)] {
                var one = context
                one.translateBy(x: x, y: y)
                one.scaleBy(x: scale, y: scale)
                one.fill(heart, with: .linearGradient(Gradient(colors: [RGB(hex: "#FF9BB0").color, RGB(hex: "#F0577A").color]), startPoint: CGPoint(x: 0, y: 0), endPoint: CGPoint(x: 0, y: 10)))
            }
        default:
            break
        }
    }
}
