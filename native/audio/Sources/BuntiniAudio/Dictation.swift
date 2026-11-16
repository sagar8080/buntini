// One dictation: the microphone's samples kept in memory (never on disk), the bands sent to
// the pill, Apple's live words where it is the recognizer, and the words when it stops.
import AVFoundation
import Foundation

final class Dictation: @unchecked Sendable {
    /// Ten minutes of 16 kHz audio, the longest hands-free dictation.
    static let limit = Int(Microphone.rate) * 600

    let id: Int
    private let microphone: Microphone
    private let models: SpeechModels
    private let languages: [String]
    private let started = Date()
    private var sink: UUID?
    private var samples: [Float] = []
    private var peak: Float = 0
    private let bands = BandAnalyzer()
    private var lastBands = Date.distantPast
    /// Apple's live recognizers, one per language Parakeet doesn't cover.
    private var live: [(language: String, text: LiveText, converter: AVAudioConverter)] = []
    private var lastPartial = ""
    private var finished = false
    /// A recorded file played in place of the microphone, for checks without a voice.
    private let input: URL?
    private var player: DispatchSourceTimer?

    init(id: Int, languages: [String], microphone: Microphone, models: SpeechModels, input: URL? = nil) {
        self.id = id
        self.languages = Languages.wanted(languages)
        self.microphone = microphone
        self.models = models
        self.input = input
    }

    /// Whether Parakeet does the words, so Apple only covers what it doesn't.
    private var parakeet: Bool {
        guard #available(macOS 14.0, *) else { return false }
        return models.parakeetLoaded || models.parakeetPresent
    }

    func start(on queue: DispatchQueue) {
        let open: (@escaping (CaptureError?) -> Void) -> Void = input == nil ? { Microphone.authorize(on: queue, $0) } : { done in queue.async { done(nil) } }
        open { [self] problem in
            if let problem { return fail(problem) }
            guard !finished else { return }
            if let input {
                do { try play(input) } catch { return fail(error) }
            } else {
                do {
                    sink = try microphone.add { [weak self] buffer in self?.heard(buffer) }
                } catch { return fail(error) }
            }
            emit("listening", ["id": id, "engine": parakeet ? "parakeet" : "apple"])
            // Parakeet may still be loading: that only matters once the words are wanted.
            if parakeet { models.load() }
            let appleLanguages = languages.filter { language in
                guard parakeet, #available(macOS 14.0, *) else { return true }
                return Languages.parakeet(language) == nil
            }
            for (index, language) in appleLanguages.enumerated() {
                let first = index == 0 && !parakeet
                prepareLive(queue: microphone.queue, download: true, locale: Locale(identifier: language), onPreparing: {}, onText: { [weak self] text in
                    guard first else { return }
                    self?.microphone.queue.async { self?.partial(text) }
                }) { [weak self] text, _ in
                    guard let self, let text else { return }
                    guard !self.finished, let converter = AVAudioConverter(from: Microphone.format, to: text.format) else {
                        Task { await text.cancel() }
                        return
                    }
                    // What was said before Apple's recognizer was ready goes in first.
                    var offset = 0
                    while offset < self.samples.count {
                        let end = min(self.samples.count, offset + 16_000)
                        if let buffer = Microphone.buffer(self.samples[offset..<end]), let converted = Microphone.convert(buffer, with: converter) { text.append(converted) }
                        offset = end
                    }
                    self.live.append((language, text, converter))
                }
            }
        }
    }

    /// Feeds a file in real time, as the microphone would.
    private func play(_ url: URL) throws {
        let samples = try Microphone.read(url, limitSeconds: 60)
        var offset = 0
        let timer = DispatchSource.makeTimerSource(queue: microphone.queue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(20))
        timer.setEventHandler { [weak self] in
            guard let self, offset < samples.count else { return }
            let end = min(samples.count, offset + 320)
            if let buffer = Microphone.buffer(samples[offset..<end]) { self.heard(buffer) }
            offset = end
        }
        timer.resume()
        player = timer
    }

    /// On the microphone's queue.
    private func heard(_ buffer: AVAudioPCMBuffer) {
        guard !finished else { return }
        let chunk = Microphone.samples(buffer)
        if samples.count < Self.limit { samples.append(contentsOf: chunk.prefix(Self.limit - samples.count)) }
        for value in chunk { peak = max(peak, abs(value)) }
        for recognizer in live {
            if let converted = Microphone.convert(buffer, with: recognizer.converter) { recognizer.text.append(converted) }
        }
        if let values = bands.push(chunk), Date().timeIntervalSince(lastBands) >= 1.0 / 60 {
            lastBands = Date()
            emit("bands", ["id": id, "values": values.map { Double(($0 * 1000).rounded() / 1000) }])
        }
    }

    private func partial(_ text: String) {
        guard !finished, text != lastPartial else { return }
        lastPartial = text
        emit("partial", ["id": id, "text": text])
    }

    /// Closes the microphone and sends the words.
    func stop(then done: @escaping () -> Void) {
        microphone.queue.async { [self] in
            guard !finished else { return done() }
            finished = true
            player?.cancel()
            player = nil
            microphone.remove(sink)
            sink = nil
            let recorded = samples
            let apple = live
            let languages = languages
            let models = models
            let peak = peak
            let ms = Int(Date().timeIntervalSince(started) * 1000)
            Task {
                var candidates: [Heard] = []
                for recognizer in apple {
                    let text = await recognizer.text.finish()
                    candidates.append(Heard(text: text, confidence: recognizer.text.confidence, language: recognizer.language, engine: "apple"))
                }
                if #available(macOS 14.0, *), models.parakeetPresent {
                    // Waits for a load still under way.
                    if !models.parakeetLoaded { await withCheckedContinuation { continuation in models.load { _ in continuation.resume() } } }
                    if let heard = await Recognizer.parakeet(recorded, languages: languages, models: models) { candidates.append(heard) }
                }
                if candidates.isEmpty && apple.isEmpty && peak > 0.0015 {
                    emit("error", ["id": self.id, "code": "no_recognizer", "message": "Download the speech model in Settings → Voice to dictate on this Mac."])
                    return done()
                }
                let best = candidates.filter { !$0.text.isEmpty }.max { $0.confidence < $1.confidence }
                var fields: [String: Any] = ["id": self.id, "text": best?.text ?? "", "ms": ms, "peak": Double(peak)]
                if let best {
                    fields["confidence"] = best.confidence
                    fields["language"] = best.language
                    fields["engine"] = best.engine
                }
                emit("final", fields)
                done()
            }
        }
    }

    func cancel() {
        microphone.queue.async { [self] in
            guard !finished else { return }
            finished = true
            player?.cancel()
            player = nil
            microphone.remove(sink)
            sink = nil
            samples = []
            for recognizer in live { Task { await recognizer.text.cancel() } }
            live = []
            emit("cancelled", ["id": id])
        }
    }

    private func fail(_ error: Error) {
        guard !finished else { return }
        finished = true
        microphone.remove(sink)
        sink = nil
        let problem = error as? CaptureError
        emit("error", ["id": id, "code": problem?.code ?? "audio_failed", "message": problem?.message ?? "The microphone didn't open."])
    }
}
