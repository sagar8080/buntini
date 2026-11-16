// Nine voice bands drawn by Metal: rounded bars as signed distances, one quad, one draw.
// The shader is compiled from source when first needed (no offline Metal compiler is
// required to build Buntini). Drawing stops while the pill is hidden.
import AppKit
import MetalKit
import SwiftUI

struct BandsView: NSViewRepresentable {
    let levels: Levels
    let active: Bool

    func makeNSView(context: Context) -> NSView {
        if let view = BandsMetalView(levels: levels) { return view }
        return BandsFallbackView(levels: levels)
    }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? BandsDrawing)?.setActive(active)
    }
}

protocol BandsDrawing: AnyObject {
    func setActive(_ active: Bool)
}

/// Smooths the levels: up fast, down slowly, so speech reads as movement, not flicker.
struct BandSmoother {
    private(set) var values = [Float](repeating: 0, count: Levels.count)

    mutating func step(toward target: [Float], seconds: Float) {
        for index in values.indices {
            let goal = index < target.count ? target[index] : 0
            let rate: Float = goal > values[index] ? 28 : 7
            values[index] += (goal - values[index]) * min(1, rate * seconds)
        }
    }
}

final class BandsMetalView: MTKView, MTKViewDelegate, BandsDrawing {
    private static let source = """
    #include <metal_stdlib>
    using namespace metal;

    struct Uniforms {
        float4 levels[3];   // nine used
        float4 size;        // width, height (points), scale, time
        float4 color;
    };

    struct Out {
        float4 position [[position]];
        float2 point;
    };

    vertex Out bands_vertex(uint id [[vertex_id]], constant Uniforms &u [[buffer(0)]]) {
        float2 corners[4] = { float2(-1, -1), float2(1, -1), float2(-1, 1), float2(1, 1) };
        Out out;
        out.position = float4(corners[id], 0, 1);
        out.point = (corners[id] * 0.5 + 0.5) * u.size.xy;
        return out;
    }

    // A vertical capsule of half-width r around a segment of half-length h.
    float capsule(float2 p, float h, float r) {
        p = float2(fabs(p.x), fabs(p.y) - h);
        return length(max(p, 0.0)) + min(max(p.x, p.y), 0.0) - r;
    }

    fragment float4 bands_fragment(Out in [[stage_in]], constant Uniforms &u [[buffer(0)]]) {
        float2 size = u.size.xy;
        float slot = size.x / 9.0;
        float radius = min(slot * 0.28, size.y * 0.5);
        float distance = 1e5;
        for (int i = 0; i < 9; i++) {
            float level = u.levels[i / 4][i % 4];
            // The middle bands sit a little taller, as a voice does.
            float shape = 0.55 + 0.45 * (1.0 - abs(float(i) - 4.0) / 4.0);
            float half_height = max(0.0, level * shape * (size.y * 0.5 - radius));
            float2 centre = float2(slot * (float(i) + 0.5), size.y * 0.5);
            distance = min(distance, capsule(in.point - centre, half_height, radius));
        }
        float edge = 0.5 / max(u.size.z, 1.0);
        float alpha = clamp(0.5 - distance / (2.0 * edge), 0.0, 1.0);
        return float4(u.color.rgb * u.color.a * alpha, u.color.a * alpha);
    }
    """

    private let levels: Levels
    private let queue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private var smoother = BandSmoother()
    private var last = CACurrentMediaTime()
    private var active = false

    init?(levels: Levels) {
        guard let device = MTLCreateSystemDefaultDevice(),
              let queue = device.makeCommandQueue(),
              let library = try? device.makeLibrary(source: Self.source, options: nil),
              let vertex = library.makeFunction(name: "bands_vertex"),
              let fragment = library.makeFunction(name: "bands_fragment") else { return nil }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = vertex
        descriptor.fragmentFunction = fragment
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        guard let pipeline = try? device.makeRenderPipelineState(descriptor: descriptor) else { return nil }
        self.levels = levels
        self.queue = queue
        self.pipeline = pipeline
        super.init(frame: .zero, device: device)
        colorPixelFormat = .bgra8Unorm
        clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        layer?.isOpaque = false
        framebufferOnly = true
        preferredFramesPerSecond = 120
        enableSetNeedsDisplay = false
        isPaused = true
        delegate = self
    }

    required init(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func setActive(_ active: Bool) {
        guard active != self.active else { return }
        self.active = active
        last = CACurrentMediaTime()
        isPaused = !active
        if !active { smoother = BandSmoother() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Off screen, nothing is drawn.
        isPaused = window == nil || !active
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        let now = CACurrentMediaTime()
        smoother.step(toward: levels.get(), seconds: Float(min(0.1, now - last)))
        last = now
        guard let pass = currentRenderPassDescriptor, let drawable = currentDrawable,
              let buffer = queue.makeCommandBuffer(), let encoder = buffer.makeRenderCommandEncoder(descriptor: pass) else { return }
        var uniforms = [Float](repeating: 0, count: 20)
        for (index, value) in smoother.values.enumerated() { uniforms[index] = max(0.06, value) }
        let scale = Float(window?.backingScaleFactor ?? 2)
        uniforms[12] = Float(bounds.width)
        uniforms[13] = Float(bounds.height)
        uniforms[14] = scale
        uniforms[15] = Float(now.truncatingRemainder(dividingBy: 1000))
        uniforms[16] = 1
        uniforms[17] = 1
        uniforms[18] = 1
        uniforms[19] = 0.95
        encoder.setRenderPipelineState(pipeline)
        uniforms.withUnsafeBytes { raw in
            encoder.setVertexBytes(raw.baseAddress!, length: raw.count, index: 0)
            encoder.setFragmentBytes(raw.baseAddress!, length: raw.count, index: 0)
        }
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
        encoder.endEncoding()
        buffer.present(drawable)
        buffer.commit()
    }
}

/// Without Metal: the same bars, drawn by Core Graphics at 30 frames a second.
final class BandsFallbackView: NSView, BandsDrawing {
    private let levels: Levels
    private var smoother = BandSmoother()
    private var timer: Timer?

    init(levels: Levels) {
        self.levels = levels
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { nil }

    func setActive(_ active: Bool) {
        timer?.invalidate()
        timer = nil
        guard active else { return }
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.smoother.step(toward: self.levels.get(), seconds: 1.0 / 30)
            self.needsDisplay = true
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        let slot = bounds.width / CGFloat(Levels.count)
        let width = slot * 0.56
        NSColor.white.withAlphaComponent(0.95).setFill()
        for (index, value) in smoother.values.enumerated() {
            let height = max(width, CGFloat(max(0.06, value)) * bounds.height)
            let rect = NSRect(x: slot * (CGFloat(index) + 0.5) - width / 2, y: (bounds.height - height) / 2, width: width, height: height)
            NSBezierPath(roundedRect: rect, xRadius: width / 2, yRadius: width / 2).fill()
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { setActive(false) }
    }
}
