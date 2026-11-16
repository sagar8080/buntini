// The week with the coding agents as a 1080 × 1920 picture to share (stream B). Rust calls
// `buntini_agents_recap_png` with the recap card's JSON and where to write the PNG.
import AppKit
import SwiftUI
import SurfaceKit

struct RecapPicture: View {
    let recap: Recap

    var body: some View {
        ZStack {
            LinearGradient(colors: [Color(hex: "#1B1530"), Color(hex: "#0B0A10")], startPoint: .top, endPoint: .bottom)
            RadialGradient(colors: [Color(hex: "#D97757").opacity(0.35), .clear], center: UnitPoint(x: 0.8, y: 0.12), startRadius: 0, endRadius: 760)
            VStack(alignment: .leading, spacing: 56) {
                VStack(alignment: .leading, spacing: 18) {
                    Text(recap.week).font(.system(size: 54, weight: .semibold)).foregroundStyle(.white.opacity(0.6))
                    Text("My week with coding agents").font(.system(size: 88, weight: .bold)).foregroundStyle(.white).lineLimit(2)
                }
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 36), GridItem(.flexible(), spacing: 36)], alignment: .leading, spacing: 48) {
                    ForEach(recap.stats, id: \.label) { stat in
                        VStack(alignment: .leading, spacing: 10) {
                            Text(stat.value).font(.system(size: 64, weight: .bold, design: .rounded)).foregroundStyle(.white).lineLimit(1).minimumScaleFactor(0.5)
                            Text(stat.label).font(.system(size: 34, weight: .medium)).foregroundStyle(.white.opacity(0.55))
                        }
                    }
                }
                Spacer()
                Text("Buntini").font(.system(size: 40, weight: .semibold, design: .rounded)).foregroundStyle(.white.opacity(0.5))
            }
            .padding(.horizontal, 96)
            .padding(.vertical, 140)
        }
        .frame(width: 1080, height: 1920)
    }
}

/// Writes the picture of `json` (a recap card's body) to `path`. False when it can't.
@_cdecl("buntini_agents_recap_png")
public func buntiniAgentsRecapPNG(_ bytes: UnsafePointer<UInt8>?, _ length: Int, _ path: UnsafePointer<CChar>?) -> Bool {
    guard let bytes, length > 0, length < 1 << 20, let path,
          let recap = try? JSONDecoder().decode(Recap.self, from: Data(bytes: bytes, count: length)) else { return false }
    let url = URL(fileURLWithPath: String(cString: path))
    let render = { MainActor.assumeIsolated { renderRecap(recap, to: url) } }
    if Thread.isMainThread { return render() }
    return DispatchQueue.main.sync(execute: render)
}

@MainActor
private func renderRecap(_ recap: Recap, to url: URL) -> Bool {
    let renderer = ImageRenderer(content: RecapPicture(recap: recap))
    renderer.scale = 1
    guard let image = renderer.cgImage,
          let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { return false }
    return (try? png.write(to: url, options: .withoutOverwriting)) != nil
}
