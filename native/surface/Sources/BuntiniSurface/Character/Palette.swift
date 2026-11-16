// The cat's colours, the same as src/Cat.tsx `palettes` and src/petOptions.ts `eyeColors`.
import SwiftUI

struct RGB: Equatable {
    var r: Double, g: Double, b: Double

    init(_ r: Double, _ g: Double, _ b: Double) {
        self.r = r
        self.g = g
        self.b = b
    }

    /// `#rrggbb`; grey when it doesn't read.
    init(hex: String) {
        let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard digits.count == 6, let value = UInt32(digits, radix: 16) else {
            self.init(128, 128, 128)
            return
        }
        self.init(Double((value >> 16) & 0xFF), Double((value >> 8) & 0xFF), Double(value & 0xFF))
    }

    func mix(_ other: RGB, _ amount: Double) -> RGB {
        RGB(r + (other.r - r) * amount, g + (other.g - g) * amount, b + (other.b - b) * amount)
    }

    func mix(_ hex: String, _ amount: Double) -> RGB { mix(RGB(hex: hex), amount) }

    var luminance: Double {
        func linear(_ value: Double) -> Double {
            let c = value / 255
            return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    var color: Color { Color(.sRGB, red: r / 255, green: g / 255, blue: b / 255, opacity: 1) }

    func opacity(_ alpha: Double) -> Color { Color(.sRGB, red: r / 255, green: g / 255, blue: b / 255, opacity: alpha) }
}

struct Coat {
    let base: RGB, shade: RGB, outline: RGB, belly: RGB, ear: RGB, nose: RGB, whisker: RGB
    var marks: RGB? = nil, patchA: RGB? = nil, patchB: RGB? = nil, point: RGB? = nil
    let blush: Double

    static func named(_ name: String) -> Coat {
        switch name {
        case "gray": return Coat(base: RGB(hex: "#A9B1BE"), shade: RGB(hex: "#8A93A2"), outline: RGB(hex: "#4E5563"), belly: RGB(hex: "#F2F3F6"), ear: RGB(hex: "#F2B8B5"), nose: RGB(hex: "#E58C9A"), whisker: RGB(hex: "#4E5563"), marks: RGB(hex: "#6E7787"), blush: 0.38)
        case "black": return Coat(base: RGB(hex: "#3E3B45"), shade: RGB(hex: "#2B2930"), outline: RGB(hex: "#141217"), belly: RGB(hex: "#57515F"), ear: RGB(hex: "#C98A93"), nose: RGB(hex: "#7A505D"), whisker: RGB(hex: "#CFCAD6"), blush: 0.24)
        case "white": return Coat(base: RGB(hex: "#FFFDF8"), shade: RGB(hex: "#ECE5DB"), outline: RGB(hex: "#A99E92"), belly: RGB(hex: "#FFFFFF"), ear: RGB(hex: "#F7B9B6"), nose: RGB(hex: "#F08A98"), whisker: RGB(hex: "#B3A79B"), blush: 0.42)
        case "tuxedo": return Coat(base: RGB(hex: "#37343D"), shade: RGB(hex: "#27252C"), outline: RGB(hex: "#141217"), belly: RGB(hex: "#FFFFFF"), ear: RGB(hex: "#C98A93"), nose: RGB(hex: "#F09AA6"), whisker: RGB(hex: "#D8D3DE"), blush: 0.3)
        case "calico": return Coat(base: RGB(hex: "#FFFBF4"), shade: RGB(hex: "#EDE4D8"), outline: RGB(hex: "#9C8D7E"), belly: RGB(hex: "#FFFFFF"), ear: RGB(hex: "#F7B9B6"), nose: RGB(hex: "#F08A98"), whisker: RGB(hex: "#A8998A"), patchA: RGB(hex: "#EF9B4E"), patchB: RGB(hex: "#433D44"), blush: 0.4)
        case "siamese": return Coat(base: RGB(hex: "#F6ECDC"), shade: RGB(hex: "#E4D3BB"), outline: RGB(hex: "#7A5A45"), belly: RGB(hex: "#FBF5EC"), ear: RGB(hex: "#8A6352"), nose: RGB(hex: "#5E4032"), whisker: RGB(hex: "#7A5A45"), point: RGB(hex: "#5E4032"), blush: 0.3)
        default: return Coat(base: RGB(hex: "#F6AA5E"), shade: RGB(hex: "#E2843D"), outline: RGB(hex: "#9A5126"), belly: RGB(hex: "#FFF1DC"), ear: RGB(hex: "#F7B3A6"), nose: RGB(hex: "#F08A98"), whisker: RGB(hex: "#9A5126"), marks: RGB(hex: "#D46F2D"), blush: 0.38)
        }
    }

    static func eye(_ name: String) -> RGB {
        switch name {
        case "green": return RGB(hex: "#86C46B")
        case "blue": return RGB(hex: "#6DB6E8")
        case "copper": return RGB(hex: "#D9772B")
        default: return RGB(hex: "#F2B33D")
        }
    }
}

let ink = RGB(hex: "#2A2230")
let white = RGB(255, 255, 255)
