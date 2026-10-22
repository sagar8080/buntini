import AVFoundation
import Darwin
import Foundation

// A deliberately small, local-only subprocess. Only JSONL is written to stdout.
private func emit(_ event: String, _ fields: [String: Any] = [:]) {
    var payload = fields
    payload["event"] = event
    guard let data = try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys]) else { return }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0a]))
}

private struct CaptureError: LocalizedError {
    let code: String
    let message: String
    var errorDescription: String? { message }
}

private struct Options {
    var output: URL
    var maxSeconds: Double = 30
    var selfTest = false

    static func parse() throws -> Options {
        var output: String?
        var duration: Double = 30
        var selfTest = false
        var args = Array(CommandLine.arguments.dropFirst())
        while !args.isEmpty {
            let arg = args.removeFirst()
            switch arg {
            case "--output":
                guard !args.isEmpty else { throw invalid("--output requires an absolute .wav path") }
                output = args.removeFirst()
            case "--max-seconds":
                guard !args.isEmpty, let value = Double(args.removeFirst()), value.isFinite,
                    value >= 1, value <= 120 else { throw invalid("--max-seconds must be between 1 and 120") }
                duration = value
            case "--self-test": selfTest = true
            default: throw invalid("Unknown argument: \(arg)")
            }
        }
        guard let path = output, path.hasPrefix("/"), !path.contains("\0"),
            URL(fileURLWithPath: path).pathExtension.lowercased() == "wav" else {
            throw invalid("--output requires an absolute .wav path")
        }
        return Options(output: URL(fileURLWithPath: path).standardizedFileURL, maxSeconds: duration, selfTest: selfTest)
    }

    private static func invalid(_ message: String) -> CaptureError {
        CaptureError(code: "invalid_arguments", message: message)
    }
}

// The file stores 16 kHz, mono, signed 16-bit PCM. At most 3.84 MB of audio can
// be produced because the CLI caps duration at 120 seconds.
private final class WaveWriter {
    static let sampleRate: Double = 16_000
    let output: URL
    let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1)!
    private var file: AVAudioFile?
    private let maxFrames: AVAudioFramePosition
    private(set) var frames: AVAudioFramePosition = 0
    private var ownsFile = false

    init(output: URL, seconds: Double) throws {
        self.output = output
        self.maxFrames = AVAudioFramePosition(seconds * Self.sampleRate)
        // O_EXCL rejects an existing file, including a symlink. Permissions are
        // restrictive from creation, before AVAudioFile opens the same path.
        let fd = open(output.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard fd >= 0 else {
            throw CaptureError(code: "output_unavailable", message: "Cannot create private recording file: \(String(cString: strerror(errno)))")
        }
        close(fd)
        ownsFile = true
        do {
            file = try AVAudioFile(forWriting: output, settings: [
                AVFormatIDKey: kAudioFormatLinearPCM,
                AVSampleRateKey: Self.sampleRate,
                AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false,
                AVLinearPCMIsBigEndianKey: false,
                AVLinearPCMIsNonInterleaved: false,
            ])
        } catch {
            discard()
            throw error
        }
    }

    var full: Bool { frames >= maxFrames }
    var durationMs: Int { Int(Double(frames) * 1_000 / Self.sampleRate) }

    func append(_ buffer: AVAudioPCMBuffer) throws {
        guard let file, !full else { return }
        let remaining = maxFrames - frames
        if AVAudioFramePosition(buffer.frameLength) > remaining {
            buffer.frameLength = AVAudioFrameCount(remaining)
        }
        guard buffer.frameLength > 0 else { return }
        try file.write(from: buffer)
        frames += AVAudioFramePosition(buffer.frameLength)
    }

    func finish() throws -> UInt64 {
        // Closing AVAudioFile finalizes the WAV header before the stopped event.
        file = nil
        let attributes = try FileManager.default.attributesOfItem(atPath: output.path)
        return (attributes[.size] as? NSNumber)?.uint64Value ?? 0
    }

    func discard() {
        file = nil
        if ownsFile { try? FileManager.default.removeItem(at: output) }
        ownsFile = false
    }
}

private final class Capture {
    private let options: Options
    private let queue = DispatchQueue(label: "buntini.audio.capture")
    private let pendingBuffers = DispatchSemaphore(value: 8)
    private let engine = AVAudioEngine()
    private var writer: WaveWriter?
    private var converter: AVAudioConverter?
    private var timer: DispatchSourceTimer?
    private var signals: [DispatchSourceSignal] = []
    private var tapInstalled = false
    private var finished = false
    private var recording = false

    init(options: Options) { self.options = options }

    func run() -> Never {
        // Ignore broken stdout pipes and clean up on termination from the parent.
        signal(SIGPIPE, SIG_IGN)
        for sig in [SIGTERM, SIGINT, SIGHUP] {
            signal(sig, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: sig, queue: queue)
            source.setEventHandler { [weak self] in self?.cancel() }
            source.resume()
            signals.append(source)
        }
        emit("ready", ["protocolVersion": 1])
        if options.selfTest {
            queue.async { self.syntheticTest() }
        } else {
            DispatchQueue.global(qos: .userInitiated).async {
                while let line = readLine(strippingNewline: true) {
                    self.queue.async {
                        switch line.trimmingCharacters(in: .whitespacesAndNewlines) {
                        case "stop": self.stop()
                        case "cancel": self.cancel()
                        default: self.fail(CaptureError(code: "invalid_command", message: "Expected stop or cancel on stdin"))
                        }
                    }
                }
                self.queue.async { self.cancel() }
            }
            queue.async { self.authorize() }
        }
        dispatchMain()
    }

    private func authorize() {
        guard !finished else { return }
        // A separately launched command-line binary inherits the responsible
        // application identity. The parent app must declare this usage string.
        guard Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") != nil else {
            fail(CaptureError(code: "missing_usage_description", message: "Audio helper is missing NSMicrophoneUsageDescription; rebuild with native/build.sh"))
            return
        }
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: start()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .audio) { allowed in
                self.queue.async {
                    guard !self.finished else { return }
                    if allowed { self.start() }
                    else { self.permissionDenied() }
                }
            }
        case .denied, .restricted: permissionDenied()
        @unknown default: permissionDenied()
        }
    }

    private func permissionDenied() {
        fail(CaptureError(code: "microphone_denied", message: "Microphone access is disabled. Enable Buntini in System Settings > Privacy & Security > Microphone."))
    }

    private func start() {
        guard !finished else { return }
        do {
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.sampleRate > 0, format.channelCount > 0 else {
                throw CaptureError(code: "microphone_unavailable", message: "No microphone input is available")
            }
            let writer = try WaveWriter(output: options.output, seconds: options.maxSeconds)
            self.writer = writer
            guard let converter = AVAudioConverter(from: format, to: writer.format) else {
                throw CaptureError(code: "conversion_unavailable", message: "Cannot convert the microphone input to 16 kHz mono audio")
            }
            self.converter = converter
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
                guard let self else { return }
                // Never block the real-time audio callback or accumulate unbounded
                // copies if writing to the local filesystem stalls.
                guard self.pendingBuffers.wait(timeout: .now()) == .success else {
                    self.queue.async { self.fail(CaptureError(code: "audio_backpressure", message: "Audio storage could not keep up with recording")) }
                    return
                }
                guard let copy = Self.copy(buffer) else {
                    self.pendingBuffers.signal()
                    self.queue.async { self.fail(CaptureError(code: "audio_buffer_failed", message: "Cannot allocate an audio buffer")) }
                    return
                }
                self.queue.async {
                    defer { self.pendingBuffers.signal() }
                    guard !self.finished, self.recording else { return }
                    self.consume(copy)
                }
            }
            tapInstalled = true
            engine.prepare()
            try engine.start()
            recording = true
            emit("recording", ["sampleRate": 16_000, "channels": 1, "maxSeconds": options.maxSeconds])
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + options.maxSeconds)
            timer.setEventHandler { [weak self] in self?.stop() }
            timer.resume()
            self.timer = timer
        } catch { fail(error) }
    }

    private static func copy(_ input: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
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

    private func consume(_ input: AVAudioPCMBuffer) {
        guard let writer, let converter else { return }
        do {
            let capacity = AVAudioFrameCount(ceil(Double(input.frameLength) * WaveWriter.sampleRate / input.format.sampleRate) + 64)
            guard let output = AVAudioPCMBuffer(pcmFormat: writer.format, frameCapacity: capacity) else {
                throw CaptureError(code: "audio_buffer_failed", message: "Cannot allocate a conversion buffer")
            }
            var provided = false
            var error: NSError?
            let status = converter.convert(to: output, error: &error) { _, state in
                if provided { state.pointee = .noDataNow; return nil }
                provided = true
                state.pointee = .haveData
                return input
            }
            if let error { throw error }
            guard status != .error else { throw CaptureError(code: "conversion_failed", message: "Audio conversion failed") }
            try writer.append(output)
            if writer.full { stop() }
        } catch { fail(error) }
    }

    private func stopEngine() {
        recording = false
        timer?.cancel()
        timer = nil
        engine.stop()
        if tapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            tapInstalled = false
        }
    }

    private func stop() {
        guard !finished else { return }
        guard recording, let writer else { cancel(); return }
        stopEngine()
        do {
            // Drain resampler latency after the last microphone buffer.
            if let converter, !writer.full, let tail = AVAudioPCMBuffer(pcmFormat: writer.format, frameCapacity: 4096) {
                var error: NSError?
                _ = converter.convert(to: tail, error: &error) { _, state in state.pointee = .endOfStream; return nil }
                if let error { throw error }
                try writer.append(tail)
            }
            let bytes = try writer.finish()
            finished = true
            emit("stopped", ["path": options.output.path, "durationMs": writer.durationMs, "bytes": bytes])
            exit(EXIT_SUCCESS)
        } catch { fail(error) }
    }

    private func cancel() {
        guard !finished else { return }
        finished = true
        stopEngine()
        writer?.discard()
        emit("cancelled")
        exit(EXIT_SUCCESS)
    }

    private func fail(_ error: Error) {
        guard !finished else { return }
        finished = true
        stopEngine()
        writer?.discard()
        let code = (error as? CaptureError)?.code ?? "audio_failed"
        emit("error", ["code": code, "message": error.localizedDescription])
        exit(EXIT_FAILURE)
    }

    private func syntheticTest() {
        do {
            let writer = try WaveWriter(output: options.output, seconds: 1)
            self.writer = writer
            // Exercise resampling and stereo-to-mono mixing, using exactly the
            // conversion path used by a microphone without opening an input.
            let sourceFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
            converter = AVAudioConverter(from: sourceFormat, to: writer.format)
            let buffer = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: 4_800)!
            buffer.frameLength = 4_800
            for frame in 0..<4_800 {
                let value = Float(sin(2 * Double.pi * 440 * Double(frame) / 48_000) * 0.1)
                buffer.floatChannelData![0][frame] = value
                buffer.floatChannelData![1][frame] = value
            }
            recording = true
            consume(buffer)
            stop()
        } catch { fail(error) }
    }
}

if CommandLine.arguments.contains("--help") {
    print("Usage: buntini-audio --output /absolute/file.wav [--max-seconds 30] [--self-test]\nCapture stdin: stop\\n or cancel\\n. Output: JSONL events. --self-test uses synthetic audio only.")
    exit(EXIT_SUCCESS)
}

do {
    let capture = Capture(options: try Options.parse())
    capture.run()
} catch {
    emit("error", ["code": (error as? CaptureError)?.code ?? "invalid_arguments", "message": error.localizedDescription])
    exit(EX_USAGE)
}
