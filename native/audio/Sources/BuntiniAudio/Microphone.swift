// One microphone shared by dictation and the wake word: open while anyone listens, closed
// (and the orange dot gone) otherwise. Hands out 16 kHz mono Float32 buffers on its queue.
import AVFoundation
import Foundation

final class Microphone: @unchecked Sendable {
    typealias Sink = (AVAudioPCMBuffer) -> Void

    static let rate: Double = 16_000
    static let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false)!

    /// Where buffers are delivered, in order.
    let queue = DispatchQueue(label: "buntini.audio.microphone")
    private let lock = NSLock()
    private var engine: AVAudioEngine?
    private var converter: AVAudioConverter?
    private var sinks: [UUID: Sink] = [:]
    /// Buffers waiting for the queue; past this the oldest kind of work is dropped, never the
    /// real-time thread blocked.
    private let pending = DispatchSemaphore(value: 32)

    /// Asks for the microphone when needed; calls back on the service's queue.
    static func authorize(on queue: DispatchQueue, _ done: @escaping (CaptureError?) -> Void) {
        guard Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") != nil else {
            return queue.async { done(CaptureError(code: "missing_usage_description", message: "The audio helper is missing its microphone description; rebuild it with native/build.sh.")) }
        }
        let denied = CaptureError(code: "microphone_denied", message: "Microphone access is off. Allow Buntini in System Settings → Privacy & Security → Microphone.")
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: queue.async { done(nil) }
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .audio) { allowed in queue.async { done(allowed ? nil : denied) } }
        default: queue.async { done(denied) }
        }
    }

    /// Adds a listener, opening the microphone for the first one.
    func add(_ sink: @escaping Sink) throws -> UUID {
        let id = UUID()
        lock.lock()
        sinks[id] = sink
        let first = engine == nil
        lock.unlock()
        if first {
            do { try openEngine() } catch {
                lock.lock()
                sinks[id] = nil
                lock.unlock()
                throw error
            }
        }
        return id
    }

    /// Removes a listener, closing the microphone after the last one.
    func remove(_ id: UUID?) {
        guard let id else { return }
        lock.lock()
        sinks[id] = nil
        let last = sinks.isEmpty
        let running = engine
        if last {
            engine = nil
            converter = nil
        }
        lock.unlock()
        if last, let running {
            running.stop()
            running.inputNode.removeTap(onBus: 0)
        }
    }

    private func openEngine() throws {
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw CaptureError(code: "microphone_unavailable", message: "No microphone is available.")
        }
        guard let converter = AVAudioConverter(from: format, to: Self.format) else {
            throw CaptureError(code: "conversion_unavailable", message: "The microphone's sound can't be converted for recognition.")
        }
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            guard let self, self.pending.wait(timeout: .now()) == .success else { return }
            guard let copy = Self.copy(buffer) else { self.pending.signal(); return }
            self.queue.async {
                defer { self.pending.signal() }
                self.deliver(copy)
            }
        }
        lock.lock()
        self.engine = engine
        self.converter = converter
        lock.unlock()
        engine.prepare()
        do { try engine.start() } catch {
            input.removeTap(onBus: 0)
            lock.lock()
            self.engine = nil
            self.converter = nil
            lock.unlock()
            throw CaptureError(code: "microphone_failed", message: "The microphone didn't open.")
        }
    }

    private func deliver(_ input: AVAudioPCMBuffer) {
        lock.lock()
        let converter = self.converter
        let listeners = Array(sinks.values)
        lock.unlock()
        guard let converter, !listeners.isEmpty, let output = Self.convert(input, with: converter) else { return }
        for listener in listeners { listener(output) }
    }

    static func convert(_ input: AVAudioPCMBuffer, with converter: AVAudioConverter) -> AVAudioPCMBuffer? {
        let target = converter.outputFormat
        let capacity = AVAudioFrameCount(ceil(Double(input.frameLength) * target.sampleRate / input.format.sampleRate) + 64)
        guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return nil }
        var provided = false
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, state in
            if provided { state.pointee = .noDataNow; return nil }
            provided = true
            state.pointee = .haveData
            return input
        }
        return status == .error || error != nil ? nil : output
    }

    static func copy(_ input: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        guard let output = AVAudioPCMBuffer(pcmFormat: input.format, frameCapacity: input.frameLength) else { return nil }
        output.frameLength = input.frameLength
        let source = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input.audioBufferList))
        let destination = UnsafeMutableAudioBufferListPointer(output.mutableAudioBufferList)
        for index in 0..<source.count {
            guard let src = source[index].mData, let dst = destination[index].mData else { return nil }
            memcpy(dst, src, Int(source[index].mDataByteSize))
        }
        return output
    }

    /// The samples of a 16 kHz mono buffer.
    static func samples(_ buffer: AVAudioPCMBuffer) -> [Float] {
        guard let data = buffer.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: data, count: Int(buffer.frameLength)))
    }

    /// A 16 kHz mono buffer holding `samples`.
    static func buffer(_ samples: ArraySlice<Float>) -> AVAudioPCMBuffer? {
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(max(1, samples.count))) else { return nil }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            if let base = source.baseAddress { buffer.floatChannelData![0].update(from: base, count: samples.count) }
        }
        return buffer
    }

    /// A whole audio file as 16 kHz mono samples (for checks on recorded files).
    static func read(_ url: URL, limitSeconds: Double = 4 * 3600) throws -> [Float] {
        let file = try AVAudioFile(forReading: url)
        guard let converter = AVAudioConverter(from: file.processingFormat, to: format) else {
            throw CaptureError(code: "conversion_unavailable", message: "That file can't be read.")
        }
        var out: [Float] = []
        let chunk = AVAudioFrameCount(file.processingFormat.sampleRate)
        let limit = AVAudioFramePosition(file.processingFormat.sampleRate * limitSeconds)
        while file.framePosition < min(file.length, limit) {
            guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk) else { break }
            try file.read(into: buffer, frameCount: chunk)
            if buffer.frameLength == 0 { break }
            guard let converted = convert(buffer, with: converter) else { break }
            out.append(contentsOf: samples(converted))
        }
        return out
    }
}
