// A soft, breathing light around the island while something waits on you. Metal draws the
// light once for the island's size (the shader is compiled from source at first use: there is
// no offline Metal compiler in the build); Core Animation makes it breathe in the render
// server, so a waiting island costs the app nothing per frame.
import AppKit
import MetalKit
import SwiftUI

struct GlowView: NSViewRepresentable {
    /// The island inside the panel, top-left origin.
    let rect: CGRect
    /// It hangs from the top edge (the notch), so the light stays off that edge.
    let hangs: Bool
    let held: Bool
    let tint: String

    func makeNSView(context: Context) -> GlowMetalView { GlowMetalView() }

    func updateNSView(_ view: GlowMetalView, context: Context) {
        view.update(rect: rect, hangs: hangs, held: held, tint: RGB(hex: tint))
    }
}

/// Compiles every shader the surfaces use, for SurfaceChecks: a mistake in one only shows at
/// run time otherwise. Nil when they compile or there is no GPU to ask.
public enum SurfaceShaders {
    public static func problem() -> String? {
        guard let device = MTLCreateSystemDefaultDevice() else { return nil }
        do {
            _ = try device.makeLibrary(source: GlowMetalView.source, options: nil)
            return nil
        } catch {
            return "\(error)"
        }
    }
}

final class GlowMetalView: MTKView, MTKViewDelegate {
    private struct Uniforms {
        var viewSize: SIMD2<Float>
        var origin: SIMD2<Float>
        var island: SIMD2<Float>
        var radius: Float
        var lift: Float
        var strength: Float
        var tint: SIMD3<Float>
    }

    static let source = """
    #include <metal_stdlib>
    using namespace metal;
    struct Out { float4 position [[position]]; float2 uv; };
    struct Uniforms { float2 viewSize; float2 origin; float2 island; float radius; float lift; float strength; float3 tint; };

    vertex Out glow_vertex(uint id [[vertex_id]]) {
        float2 p = float2((id << 1) & 2, id & 2);
        Out out;
        out.position = float4(p * 2.0 - 1.0, 0.0, 1.0);
        out.uv = float2(p.x, 1.0 - p.y);
        return out;
    }

    static float roundedBox(float2 p, float2 extent, float r) {
        float2 q = abs(p) - extent + r;
        return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    }

    fragment float4 glow_fragment(Out in [[stage_in]], constant Uniforms &u [[buffer(0)]]) {
        float2 p = in.uv * u.viewSize;
        // An island hanging from the top edge runs past it (lift), so that edge stays dark.
        float2 center = float2(u.origin.x + u.island.x * 0.5, u.origin.y + (u.island.y - u.lift) * 0.5);
        float d = roundedBox(p - center, float2(u.island.x * 0.5, (u.island.y + u.lift) * 0.5), u.radius);
        if (d <= 0.0) { return float4(0.0); }
        // Two falloffs: a tight rim and a wide haze.
        float glow = (0.65 * exp(-d / 9.0) + 0.35 * exp(-d / 28.0)) * u.strength;
        return float4(u.tint * glow, glow);
    }
    """

    private static var shared: (device: MTLDevice, pipeline: MTLRenderPipelineState?)?
    private var queue: MTLCommandQueue?
    private var pipeline: MTLRenderPipelineState?
    private var island = CGRect.zero
    private var hangs = true
    /// What the island wants, and what the light shows: they differ until the shader is ready.
    private var held = false
    private var shown = false
    private var tint = SIMD3<Float>(1, 0.7, 0.24)

    init() {
        let device = MTLCreateSystemDefaultDevice()
        super.init(frame: .zero, device: device)
        delegate = self
        colorPixelFormat = .bgra8Unorm
        clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        // Drawn only when asked: the breathing is Core Animation's.
        isPaused = true
        enableSetNeedsDisplay = true
        wantsLayer = true
        layer?.isOpaque = false
        layer?.opacity = 0
        queue = device?.makeCommandQueue()
        compile()
    }

    required init(coder: NSCoder) { fatalError("Not used") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func update(rect: CGRect, hangs: Bool, held: Bool, tint: RGB) {
        let tint = SIMD3(Float(tint.r / 255), Float(tint.g / 255), Float(tint.b / 255))
        let redraw = rect != island || hangs != self.hangs || tint != self.tint
        island = rect
        self.hangs = hangs
        self.tint = tint
        if redraw { needsDisplay = true }
        self.held = held
        show()
    }

    private func show() {
        guard pipeline != nil, held != shown else { return }
        shown = held
        breathe(held)
    }

    /// Fades the light in and starts its slow breath, or fades it out.
    private func breathe(_ on: Bool) {
        guard let layer else { return }
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        let from = layer.presentation()?.opacity ?? layer.opacity
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = from
        fade.toValue = on ? 0.9 : 0
        fade.duration = on ? 0.45 : 0.3
        fade.beginTime = on ? CACurrentMediaTime() + 0.2 : 0
        fade.fillMode = .backwards
        fade.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer.removeAnimation(forKey: "breath")
        layer.opacity = on ? 0.9 : 0
        layer.add(fade, forKey: "fade")
        guard on, !reduceMotion else { return }
        let breath = CABasicAnimation(keyPath: "opacity")
        breath.fromValue = 0.9
        breath.toValue = 0.45
        breath.duration = 1.4
        breath.autoreverses = true
        breath.repeatCount = .infinity
        breath.beginTime = CACurrentMediaTime() + 0.65
        breath.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer.add(breath, forKey: "breath")
    }

    private func compile() {
        guard let device else { return }
        if let shared = GlowMetalView.shared, shared.device === device {
            pipeline = shared.pipeline
            return
        }
        device.makeLibrary(source: GlowMetalView.source, options: nil) { [weak self] library, _ in
            guard let library else { return }
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.vertexFunction = library.makeFunction(name: "glow_vertex")
            descriptor.fragmentFunction = library.makeFunction(name: "glow_fragment")
            descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
            let pipeline = try? device.makeRenderPipelineState(descriptor: descriptor)
            DispatchQueue.main.async {
                GlowMetalView.shared = (device, pipeline)
                guard let self else { return }
                self.pipeline = pipeline
                self.needsDisplay = true
                self.show()
            }
        }
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        needsDisplay = true
    }

    func draw(in view: MTKView) {
        guard let pipeline, let queue, let pass = currentRenderPassDescriptor, let drawable = currentDrawable,
              let buffer = queue.makeCommandBuffer(), let encoder = buffer.makeRenderCommandEncoder(descriptor: pass) else { return }
        var uniforms = Uniforms(
            viewSize: SIMD2(Float(bounds.width), Float(bounds.height)),
            origin: SIMD2(Float(island.minX), Float(island.minY)),
            island: SIMD2(Float(island.width), Float(island.height)),
            radius: hangs ? 26 : 22, lift: hangs ? 60 : 0, strength: 1, tint: tint)
        encoder.setRenderPipelineState(pipeline)
        encoder.setFragmentBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        encoder.endEncoding()
        buffer.present(drawable)
        buffer.commit()
    }
}
