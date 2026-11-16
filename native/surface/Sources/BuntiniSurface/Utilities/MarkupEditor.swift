// Marking up a capture: a pen, an arrow and a box in three colours, with undo. Marks are kept
// in the picture's own coordinates, so the saved copy is drawn at full resolution exactly as
// it looked. Done saves the copy and hands it back to the island as the `annotated` action.
import AppKit
import ImageIO
import SwiftUI
import SurfaceKit

/// A mark on a capture. Points run 0…1 across the picture, from its top left.
struct Mark: Equatable {
    enum Tool: String, CaseIterable {
        case pen, arrow, box

        var symbol: String {
            switch self {
            case .pen: return "pencil.tip"
            case .arrow: return "arrow.up.right"
            case .box: return "rectangle"
            }
        }

        var name: String {
            switch self {
            case .pen: return "Pen"
            case .arrow: return "Arrow"
            case .box: return "Box"
            }
        }
    }

    var tool: Tool
    var color: Int
    var points: [CGPoint]

    static let colors: [RGB] = [RGB(hex: "#FF453A"), RGB(hex: "#FFD60A"), RGB(hex: "#0A84FF")]
    /// Line width, as a share of the picture's long side.
    static let weight: CGFloat = 0.0042

    /// The mark's outline and its filled part (an arrow's head), drawn where `place` puts each
    /// point, with lines `width` wide.
    func paths(width: CGFloat, place: (CGPoint) -> CGPoint) -> (stroke: CGPath, fill: CGPath?) {
        let stroke = CGMutablePath()
        let placed = points.map(place)
        guard let first = placed.first, let last = placed.last else { return (stroke, nil) }
        switch tool {
        case .pen:
            stroke.move(to: first)
            guard placed.count > 2 else {
                stroke.addLine(to: last)
                break
            }
            // Through the midpoints, so a quick scribble comes out smooth.
            for index in 1..<placed.count - 1 {
                let middle = CGPoint(x: (placed[index].x + placed[index + 1].x) / 2, y: (placed[index].y + placed[index + 1].y) / 2)
                stroke.addQuadCurve(to: middle, control: placed[index])
            }
            stroke.addLine(to: last)
            return (stroke, nil)
        case .arrow:
            let dx = last.x - first.x, dy = last.y - first.y
            let length = hypot(dx, dy)
            guard length > 0.5 else { return (stroke, nil) }
            let head = min(length * 0.6, max(12, width * 4.4))
            let angle = atan2(dy, dx)
            let base = CGPoint(x: last.x - cos(angle) * head * 0.8, y: last.y - sin(angle) * head * 0.8)
            stroke.move(to: first)
            stroke.addLine(to: base)
            let fill = CGMutablePath()
            fill.move(to: last)
            fill.addLine(to: CGPoint(x: last.x - head * cos(angle - 0.45), y: last.y - head * sin(angle - 0.45)))
            fill.addLine(to: CGPoint(x: last.x - head * cos(angle + 0.45), y: last.y - head * sin(angle + 0.45)))
            fill.closeSubpath()
            return (stroke, fill)
        case .box:
            let rect = CGRect(x: min(first.x, last.x), y: min(first.y, last.y), width: abs(last.x - first.x), height: abs(last.y - first.y))
            let radius = min(width * 1.6, rect.width / 2, rect.height / 2)
            stroke.addRoundedRect(in: rect, cornerWidth: radius, cornerHeight: radius)
        }
        return (stroke, nil)
    }
}

final class MarkupModel: ObservableObject {
    @Published var marks: [Mark] = []
    @Published var current: Mark?
    @Published var tool: Mark.Tool = .arrow
    @Published var color = 0
}

struct MarkupView: View {
    @ObservedObject var model: MarkupModel
    let image: NSImage
    let pixels: CGSize
    let done: () -> Void
    let cancel: () -> Void

    /// Where the picture sits in `size`, leaving room for the tools below.
    private func frame(in size: CGSize) -> CGRect {
        let room = CGRect(x: 20, y: 34, width: max(1, size.width - 40), height: max(1, size.height - 34 - 78))
        let scale = min(room.width / pixels.width, room.height / pixels.height)
        let fitted = CGSize(width: pixels.width * scale, height: pixels.height * scale)
        return CGRect(x: room.midX - fitted.width / 2, y: room.midY - fitted.height / 2, width: fitted.width, height: fitted.height)
    }

    var body: some View {
        GeometryReader { geometry in
            let rect = frame(in: geometry.size)
            ZStack(alignment: .topLeading) {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: rect.width, height: rect.height)
                    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    .shadow(color: .black.opacity(0.45), radius: 18, y: 8)
                    .offset(x: rect.minX, y: rect.minY)
                Canvas { context, _ in
                    let width = max(2, Mark.weight * max(rect.width, rect.height))
                    context.addFilter(.shadow(color: .black.opacity(0.28), radius: 1.5, y: 0.5))
                    for mark in model.marks + [model.current].compactMap({ $0 }) {
                        let color = Mark.colors[mark.color % Mark.colors.count].color
                        let (stroke, fill) = mark.paths(width: width) { CGPoint(x: rect.minX + $0.x * rect.width, y: rect.minY + $0.y * rect.height) }
                        context.stroke(Path(stroke), with: .color(color), style: StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round))
                        if let fill { context.fill(Path(fill), with: .color(color)) }
                    }
                }
                .allowsHitTesting(false)
            }
            .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { drag in
                let at = { (point: CGPoint) in CGPoint(x: min(max((point.x - rect.minX) / rect.width, 0), 1), y: min(max((point.y - rect.minY) / rect.height, 0), 1)) }
                if model.current == nil {
                    guard rect.insetBy(dx: -12, dy: -12).contains(drag.startLocation) else { return }
                    model.current = Mark(tool: model.tool, color: model.color, points: [at(drag.startLocation)])
                }
                if model.tool == .pen { model.current?.points.append(at(drag.location)) } else if let start = model.current?.points.first { model.current?.points = [start, at(drag.location)] }
            }.onEnded { _ in
                if let mark = model.current, mark.points.count > 1 { model.marks.append(mark) }
                model.current = nil
            })
            .overlay(alignment: .bottom) { tools.padding(.bottom, 18) }
        }
        .background(Color(white: 0.11))
        .preferredColorScheme(.dark)
    }

    private var tools: some View {
        HStack(spacing: 4) {
            ForEach(Mark.Tool.allCases, id: \.self) { tool in
                ToolButton(symbol: tool.symbol, name: tool.name, on: model.tool == tool) { model.tool = tool }
            }
            Divider().frame(height: 18).padding(.horizontal, 4)
            ForEach(Mark.colors.indices, id: \.self) { index in
                Button { model.color = index } label: {
                    Circle()
                        .fill(Mark.colors[index].color)
                        .frame(width: 16, height: 16)
                        .overlay(Circle().stroke(.white, lineWidth: model.color == index ? 2 : 0).frame(width: 22, height: 22))
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(["Red", "Yellow", "Blue"][index])
            }
            Divider().frame(height: 18).padding(.horizontal, 4)
            ToolButton(symbol: "arrow.uturn.backward", name: "Undo", on: false) { _ = model.marks.popLast() }
                .keyboardShortcut("z", modifiers: .command)
                .disabled(model.marks.isEmpty)
            Button("Done", action: done)
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.plain)
                .font(.callout.weight(.semibold))
                .padding(.horizontal, 14)
                .frame(height: 28)
                .background(Capsule().fill(Color.white))
                .foregroundStyle(Color.black)
                .padding(.leading, 6)
            Button("", action: cancel).keyboardShortcut(.cancelAction).opacity(0).frame(width: 0, height: 0)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .modifier(GlassCapsule())
    }
}

private struct ToolButton: View {
    let symbol: String
    let name: String
    let on: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .frame(width: 32, height: 28)
                .background(Capsule().fill(Color.white.opacity(on ? 0.2 : 0)))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(name)
        .accessibilityLabel(name)
    }
}

/// Liquid Glass on macOS 26; a frosted capsule before it.
struct GlassCapsule: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 26.0, *) {
            content.glassEffect(.regular, in: Capsule())
        } else {
            content.background(.regularMaterial, in: Capsule())
        }
    }
}

/// Draws a box onto a blank picture and reads it back, for SurfaceChecks: the saved copy must
/// have each mark where it was drawn, the right way up. Nil when it does.
public enum MarkupCheck {
    public static func problem() -> String? {
        let width = 1000, height = 500
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let blank = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return "no picture to mark up" }
        blank.setFillColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 1))
        blank.fill(CGRect(x: 0, y: 0, width: width, height: height))
        guard let picture = blank.makeImage(),
              let marked = MarkupEditor.render(picture, marks: [Mark(tool: .box, color: 0, points: [CGPoint(x: 0.1, y: 0.1), CGPoint(x: 0.4, y: 0.5)])]) else { return "mark-up didn't render" }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let reader = CGContext(data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return "no reader" }
        reader.draw(marked, in: CGRect(x: 0, y: 0, width: width, height: height))
        // Rows run from the top.
        func pixel(_ x: Int, _ y: Int) -> (r: UInt8, g: UInt8, b: UInt8) {
            let at = (y * width + x) * 4
            return (pixels[at], pixels[at + 1], pixels[at + 2])
        }
        let edge = pixel(250, 50), mirror = pixel(250, 450), inside = pixel(250, 150)
        guard edge.r > 200, edge.g < 120, edge.b < 120 else { return "the box's top edge isn't red at 10% down: \(edge)" }
        guard mirror.g > 240, inside.g > 240 else { return "the mark landed upside down or filled in" }
        return nil
    }
}

final class MarkupEditor: NSObject, NSWindowDelegate {
    private static var editors: [MarkupEditor] = []
    private let window: NSWindow
    private let model = MarkupModel()
    private let picture: CGImage
    private let source: URL
    private let target: URL
    private let previous: NSRunningApplication?

    static func open(_ source: URL, saveTo target: URL) {
        if let editor = editors.first(where: { $0.source == source }) {
            editor.show()
            return
        }
        guard let editor = MarkupEditor(source: source, target: target) else { return }
        editors.append(editor)
        editor.show()
    }

    private init?(source: URL, target: URL) {
        guard let file = CGImageSourceCreateWithURL(source as CFURL, nil),
              let picture = CGImageSourceCreateImageAtIndex(file, 0, nil) else { return nil }
        self.picture = picture
        self.source = source
        self.target = target
        previous = NSWorkspace.shared.frontmostApplication.flatMap { $0.processIdentifier == ProcessInfo.processInfo.processIdentifier ? nil : $0 }
        let pixels = CGSize(width: picture.width, height: picture.height)
        let pointer = NSEvent.mouseLocation
        let screen = NSScreen.screens.first(where: { NSMouseInRect(pointer, $0.frame, false) }) ?? NSScreen.main
        let visible = screen?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1440, height: 900)
        let scale = screen?.backingScaleFactor ?? 2
        let fit = min(1, visible.width * 0.78 / (pixels.width / scale), (visible.height * 0.78 - 112) / (pixels.height / scale))
        let size = CGSize(width: max(460, pixels.width / scale * fit + 40), height: max(340, pixels.height / scale * fit + 112))
        window = NSWindow(contentRect: CGRect(x: visible.midX - size.width / 2, y: visible.midY - size.height / 2, width: size.width, height: size.height), styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        super.init()
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.title = source.deletingPathExtension().lastPathComponent
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.backgroundColor = NSColor(white: 0.11, alpha: 1)
        window.level = .floating
        window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
        window.minSize = CGSize(width: 460, height: 340)
        window.delegate = self
        let image = NSImage(cgImage: picture, size: pixels)
        window.contentView = NSHostingView(rootView: MarkupView(model: model, image: image, pixels: pixels, done: { [weak self] in self?.save() }, cancel: { [weak self] in self?.window.close() }))
    }

    private func show() {
        if #available(macOS 14.0, *) { NSApp.activate() } else { NSApp.activate(ignoringOtherApps: true) }
        window.makeKeyAndOrderFront(nil)
    }

    /// Draws the marks onto the full picture and saves it.
    private func save() {
        guard let marked = Self.render(picture, marks: model.marks), Self.write(marked, to: target) else {
            NSSound.beep()
            return
        }
        SurfaceRuntime.shared.send(Action(source: .utilities, card: nil, name: "annotated", value: .string(target.path)))
        window.close()
    }

    static func render(_ picture: CGImage, marks: [Mark]) -> CGImage? {
        let width = picture.width, height = picture.height
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        context.draw(picture, in: CGRect(x: 0, y: 0, width: width, height: height))
        let line = Mark.weight * CGFloat(max(width, height))
        context.setLineCap(.round)
        context.setLineJoin(.round)
        context.setLineWidth(line)
        context.setShadow(offset: CGSize(width: 0, height: -line * 0.12), blur: line * 0.35, color: NSColor.black.withAlphaComponent(0.28).cgColor)
        for mark in marks {
            let color = Mark.colors[mark.color % Mark.colors.count]
            let cg = CGColor(srgbRed: color.r / 255, green: color.g / 255, blue: color.b / 255, alpha: 1)
            // Points run from the top left; Core Graphics from the bottom left.
            let (stroke, fill) = mark.paths(width: line) { CGPoint(x: $0.x * CGFloat(width), y: (1 - $0.y) * CGFloat(height)) }
            context.setStrokeColor(cg)
            context.addPath(stroke)
            context.strokePath()
            if let fill {
                context.setFillColor(cg)
                context.addPath(fill)
                context.fillPath()
            }
        }
        return context.makeImage()
    }

    static func write(_ image: CGImage, to url: URL) -> Bool {
        guard let destination = CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil) else { return false }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination)
    }

    func windowWillClose(_ notification: Notification) {
        Self.editors.removeAll { $0 === self }
        // Back to the app you were in, so Paste lands there.
        if let previous, !previous.isTerminated {
            if #available(macOS 14.0, *) {
                NSApp.yieldActivation(to: previous)
                previous.activate()
            } else {
                previous.activate(options: [])
            }
        }
    }
}
