import AVFoundation
import Darwin
import Foundation
#if compiler(>=6.2) && canImport(Speech)
import Speech
#endif

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
    /// Where a WAV recording goes. Live transcription never writes audio, so with
    /// --transcribe this is only the fallback for Macs that cannot transcribe live.
    var output: URL?
    var maxSeconds: Double = 30
    var selfTest = false
    var transcribe = false
    var selfTestLive = false
    /// A recorded file the live self-test feeds through recognition instead of a tone.
    var input: URL?

    static func parse() throws -> Options {
        var output: String?
        var duration: Double = 30
        var selfTest = false
        var transcribe = false
        var selfTestLive = false
        var input: String?
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
            case "--transcribe": transcribe = true
            case "--self-test-live": selfTestLive = true
            case "--input":
                guard !args.isEmpty else { throw invalid("--input requires an absolute audio file path") }
                input = args.removeFirst()
            default: throw invalid("Unknown argument: \(arg)")
            }
        }
        if selfTestLive {
            guard input.map({ $0.hasPrefix("/") && !$0.contains("\0") }) ?? true else { throw invalid("--input requires an absolute audio file path") }
            return Options(output: nil, maxSeconds: duration, transcribe: true, selfTestLive: true, input: input.map { URL(fileURLWithPath: $0) })
        }
        guard let path = output else {
            if transcribe && !selfTest { return Options(output: nil, maxSeconds: duration, transcribe: true) }
            throw invalid("--output requires an absolute .wav path")
        }
        guard path.hasPrefix("/"), !path.contains("\0"),
            URL(fileURLWithPath: path).pathExtension.lowercased() == "wav" else {
            throw invalid("--output requires an absolute .wav path")
        }
        return Options(output: URL(fileURLWithPath: path).standardizedFileURL, maxSeconds: duration, selfTest: selfTest, transcribe: transcribe)
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

/// Live, on-device speech recognition fed straight from the microphone. Audio stays in
/// memory: nothing is written to disk, and Apple's on-device model does the recognition.
private protocol LiveText: AnyObject {
    var format: AVAudioFormat { get }
    func append(_ buffer: AVAudioPCMBuffer)
    func finish() async -> String
    func cancel() async
}

/// Collapses the recognizer's spacing so partial and final text compare cleanly.
private func tidy(_ text: String) -> String {
    text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
}

#if compiler(>=6.2) && canImport(Speech)
@available(macOS 26.0, *)
private final class LiveTranscriber: LiveText, @unchecked Sendable {
    let format: AVAudioFormat
    private let analyzer: SpeechAnalyzer
    private let transcriber: SpeechTranscriber
    private let input: AsyncStream<AnalyzerInput>.Continuation
    private let lock = NSLock()
    private var finalized = ""
    private var volatile = ""
    private var results: Task<Void, Never>?

    private init(format: AVAudioFormat, analyzer: SpeechAnalyzer, transcriber: SpeechTranscriber, input: AsyncStream<AnalyzerInput>.Continuation) {
        self.format = format
        self.analyzer = analyzer
        self.transcriber = transcriber
        self.input = input
    }

    /// Nil with a reason when this Mac cannot transcribe live: an unsupported language,
    /// a speech model that is not installed (and may not be downloaded), or a failed start.
    static func prepare(download: Bool, onPreparing: @escaping () -> Void, onText: @escaping @Sendable (String) -> Void) async -> (LiveTranscriber?, String) {
        guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current) else { return (nil, "unsupported_language") }
        let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
        do {
            switch await AssetInventory.status(forModules: [transcriber]) {
            case .installed: break
            case .unsupported: return (nil, "unsupported_language")
            default:
                // macOS downloads Apple's speech model once; no audio is involved.
                guard download else { return (nil, "not_installed") }
                onPreparing()
                if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
                    try await request.downloadAndInstall()
                }
            }
            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else { return (nil, "no_format") }
            let (stream, continuation) = AsyncStream.makeStream(of: AnalyzerInput.self)
            let analyzer = SpeechAnalyzer(modules: [transcriber])
            try await analyzer.prepareToAnalyze(in: format)
            let live = LiveTranscriber(format: format, analyzer: analyzer, transcriber: transcriber, input: continuation)
            live.results = Task { await live.collect(onText) }
            try await analyzer.start(inputSequence: stream)
            return (live, "")
        } catch {
            return (nil, "transcriber_failed")
        }
    }

    private func collect(_ onText: @Sendable (String) -> Void) async {
        do {
            for try await result in transcriber.results {
                let text = String(result.text.characters)
                let current = update(text, final: result.isFinal)
                onText(current)
            }
        } catch {}
    }

    /// Volatile text is the recognizer's current guess for the words after the last
    /// finalized range; it is replaced, never appended.
    private func update(_ text: String, final: Bool) -> String {
        lock.lock()
        defer { lock.unlock() }
        if final { finalized += " " + text; volatile = "" } else { volatile = text }
        return tidy(finalized + " " + volatile)
    }

    private func current() -> String {
        lock.lock()
        defer { lock.unlock() }
        return tidy(finalized + " " + volatile)
    }

    func append(_ buffer: AVAudioPCMBuffer) { input.yield(AnalyzerInput(buffer: buffer)) }

    func finish() async -> String {
        input.finish()
        try? await analyzer.finalizeAndFinishThroughEndOfInput()
        await results?.value
        return current()
    }

    func cancel() async {
        input.finish()
        await analyzer.cancelAndFinishNow()
        results?.cancel()
    }
}
#endif

/// Prepares live recognition, then calls back on `queue`. Older macOS gets nil, so the
/// caller records a WAV for the existing file transcription instead.
private func prepareLive(queue: DispatchQueue, download: Bool, onPreparing: @escaping () -> Void, onText: @escaping @Sendable (String) -> Void, then: @escaping (LiveText?, String) -> Void) {
    #if compiler(>=6.2) && canImport(Speech)
    if #available(macOS 26.0, *) {
        Task {
            let (live, reason) = await LiveTranscriber.prepare(download: download, onPreparing: onPreparing, onText: onText)
            queue.async { then(live, reason) }
        }
        return
    }
    #endif
    queue.async { then(nil, "unsupported") }
}

/// All mutable state is confined to `queue`, which is what makes the Sendable claim hold.
private final class Capture: @unchecked Sendable {
    private let options: Options
    private let queue = DispatchQueue(label: "buntini.audio.capture")
    private let pendingBuffers = DispatchSemaphore(value: 8)
    private let engine = AVAudioEngine()
    private var writer: WaveWriter?
    private var live: LiveText?
    private var converter: AVAudioConverter?
    private var timer: DispatchSourceTimer?
    private var signals: [DispatchSourceSignal] = []
    private var tapInstalled = false
    private var finished = false
    private var stopping = false
    private var recording = false
    /// The loudest sample so far (0…1), so a silent microphone can be told apart from quiet speech.
    private var peak: Float = 0
    private var inputFrames: Double = 0
    private var inputRate: Double = 0
    private var levelSum: Float = 0
    private var levelFrames = 0
    private var lastPartial = ""

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
        emit("ready", ["protocolVersion": 2])
        if options.selfTestLive {
            queue.async { self.liveSelfTest() }
        } else if options.selfTest {
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
        case .authorized: prepare()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .audio) { allowed in
                self.queue.async {
                    guard !self.finished else { return }
                    if allowed { self.prepare() }
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

    private func prepare() {
        guard options.transcribe else { start(live: nil, reason: ""); return }
        let queue = self.queue
        prepareLive(queue: queue, download: true,
            onPreparing: { queue.async { emit("preparing") } },
            onText: { [weak self] text in queue.async { self?.partial(text) } },
            then: { [weak self] live, reason in self?.start(live: live, reason: reason) })
    }

    private func partial(_ text: String) {
        guard !finished, recording || stopping, text != lastPartial else { return }
        lastPartial = text
        emit("partial", ["text": text])
    }

    private func start(live: LiveText?, reason: String) {
        guard !finished else {
            if let live { Task { await live.cancel() } }
            return
        }
        do {
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.sampleRate > 0, format.channelCount > 0 else {
                throw CaptureError(code: "microphone_unavailable", message: "No microphone input is available")
            }
            inputRate = format.sampleRate
            let target: AVAudioFormat
            if let live {
                self.live = live
                target = live.format
            } else {
                guard let output = options.output else {
                    throw CaptureError(code: "live_unavailable", message: "Live speech recognition isn’t available on this Mac (\(reason.isEmpty ? "unsupported" : reason)).")
                }
                let writer = try WaveWriter(output: output, seconds: options.maxSeconds)
                self.writer = writer
                target = writer.format
            }
            guard let converter = AVAudioConverter(from: format, to: target) else {
                throw CaptureError(code: "conversion_unavailable", message: "Cannot convert the microphone input for speech recognition")
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
            var fields: [String: Any] = ["sampleRate": Int(target.sampleRate), "channels": 1, "maxSeconds": options.maxSeconds, "live": live != nil]
            if live == nil && options.transcribe { fields["liveUnavailable"] = reason.isEmpty ? "unsupported" : reason }
            emit("recording", fields)
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

    /// Loudness every ~100 ms lets Buntini tell when you stop talking without a
    /// transcript (the Whisper fallback) and spot a microphone that only sends silence.
    private func measure(_ input: AVAudioPCMBuffer) {
        let count = Int(input.frameLength)
        inputFrames += Double(count)
        guard let samples = input.floatChannelData?[0] else { return }
        for index in 0..<count {
            let value = samples[index]
            peak = max(peak, abs(value))
            levelSum += value * value
        }
        levelFrames += count
        if Double(levelFrames) >= input.format.sampleRate / 10 {
            let rms = (levelSum / Float(levelFrames)).squareRoot()
            emit("level", ["rms": (Double(rms) * 10_000).rounded() / 10_000])
            levelSum = 0
            levelFrames = 0
        }
    }

    private var durationMs: Int { inputRate > 0 ? Int(inputFrames * 1_000 / inputRate) : 0 }

    private func consume(_ input: AVAudioPCMBuffer) {
        guard let converter else { return }
        measure(input)
        do {
            let target = converter.outputFormat
            let capacity = AVAudioFrameCount(ceil(Double(input.frameLength) * target.sampleRate / input.format.sampleRate) + 64)
            guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else {
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
            if let live {
                if output.frameLength > 0 { live.append(output) }
            } else if let writer {
                try writer.append(output)
                if writer.full { stop() }
            }
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
        guard !finished, !stopping else { return }
        guard recording else { cancel(); return }
        if let live {
            // Finalizing waits for the recognizer's last words; cancel still wins meanwhile.
            stopping = true
            stopEngine()
            let queue = self.queue
            Task {
                let text = await live.finish()
                queue.async { self.stopped(transcript: text) }
            }
            return
        }
        guard let writer, let output = options.output else { cancel(); return }
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
            emit("stopped", ["path": output.path, "durationMs": writer.durationMs, "bytes": bytes, "peak": Double(peak)])
            exit(EXIT_SUCCESS)
        } catch { fail(error) }
    }

    private func stopped(transcript: String) {
        guard !finished else { return }
        finished = true
        emit("stopped", ["transcript": transcript, "durationMs": durationMs, "peak": Double(peak)])
        exit(EXIT_SUCCESS)
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

    /// A 48 kHz stereo tone: the same conversion a microphone goes through.
    private static func syntheticTone(frames: AVAudioFrameCount) -> AVAudioPCMBuffer {
        let sourceFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
        let buffer = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: frames)!
        buffer.frameLength = frames
        for frame in 0..<Int(frames) {
            let value = Float(sin(2 * Double.pi * 440 * Double(frame) / 48_000) * 0.1)
            buffer.floatChannelData![0][frame] = value
            buffer.floatChannelData![1][frame] = value
        }
        return buffer
    }

    /// A whole (small) audio file in 100 ms buffers, as a microphone would deliver it.
    private static func read(_ url: URL) throws -> [AVAudioPCMBuffer] {
        let file = try AVAudioFile(forReading: url)
        let chunk = AVAudioFrameCount(max(1, file.processingFormat.sampleRate / 10))
        var buffers: [AVAudioPCMBuffer] = []
        while file.framePosition < min(file.length, AVAudioFramePosition(file.processingFormat.sampleRate * 30)) {
            guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk) else { break }
            try file.read(into: buffer, frameCount: chunk)
            if buffer.frameLength == 0 { break }
            buffers.append(buffer)
        }
        return buffers
    }

    private func syntheticTest() {
        do {
            guard let output = options.output else { throw CaptureError(code: "invalid_arguments", message: "--self-test needs --output") }
            let writer = try WaveWriter(output: output, seconds: 1)
            self.writer = writer
            // Exercise resampling and stereo-to-mono mixing, using exactly the
            // conversion path used by a microphone without opening an input.
            let buffer = Self.syntheticTone(frames: 4_800)
            converter = AVAudioConverter(from: buffer.format, to: writer.format)
            inputRate = buffer.format.sampleRate
            recording = true
            consume(buffer)
            stop()
        } catch { fail(error) }
    }

    /// Runs synthetic audio through live recognition without a microphone. A Mac without
    /// macOS 26 or without the speech model installed reports why and exits cleanly; the
    /// test never downloads a model.
    private func liveSelfTest() {
        let queue = self.queue
        prepareLive(queue: queue, download: false, onPreparing: {}, onText: { [weak self] text in queue.async { self?.partial(text) } }) { live, reason in
            guard let live else {
                emit("live", ["available": false, "reason": reason])
                exit(EXIT_SUCCESS)
            }
            emit("live", ["available": true, "sampleRate": live.format.sampleRate])
            var buffers = [Self.syntheticTone(frames: 24_000)]
            if let url = self.options.input {
                do { buffers = try Self.read(url) } catch { return self.fail(CaptureError(code: "input_unavailable", message: "Cannot read the test audio file")) }
            }
            guard let first = buffers.first, let converter = AVAudioConverter(from: first.format, to: live.format) else {
                return self.fail(CaptureError(code: "conversion_unavailable", message: "Cannot convert audio for speech recognition"))
            }
            self.converter = converter
            self.live = live
            self.inputRate = first.format.sampleRate
            self.recording = true
            for buffer in buffers { self.consume(buffer) }
            self.stop()
        }
    }
}

if CommandLine.arguments.contains("--help") {
    print("Usage: buntini-audio [--transcribe] [--output /absolute/file.wav] [--max-seconds 30] [--self-test | --self-test-live [--input /absolute/audio]]\nCapture stdin: stop\\n or cancel\\n. Output: JSONL events (ready, preparing, recording, level, partial, stopped, cancelled, error).\n--transcribe recognizes speech live on this Mac and writes no audio; --output is then only used where live recognition is unavailable.\n--self-test and --self-test-live use synthetic audio only.")
    exit(EXIT_SUCCESS)
}

do {
    let capture = Capture(options: try Options.parse())
    capture.run()
} catch {
    emit("error", ["code": (error as? CaptureError)?.code ?? "invalid_arguments", "message": error.localizedDescription])
    exit(EX_USAGE)
}
