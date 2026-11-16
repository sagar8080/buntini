// Listens for "Hey Buntini". Recognition runs only while someone is speaking: an energy
// voice-activity check opens a stretch (with the half second before it), and when the
// speech stops the stretch is recognized and its words go to Rust, which looks for the
// phrase. No audio is kept.
import AVFoundation
import Foundation

final class WakeListener: @unchecked Sendable {
    static let frame = 320 // 20 ms
    static let preroll = 8_000 // 0.5 s
    static let longest = 48_000 // 3 s
    static let quietFrames = 20 // 0.4 s

    private let microphone: Microphone
    private let models: SpeechModels
    private let languages: [String]
    private var sink: UUID?
    private var pending: [Float] = []
    private var before: [Float] = []
    private var stretch: [Float]?
    private var quiet = 0
    private var floor: Float = 0.004
    private var busy = false

    init(languages: [String], microphone: Microphone, models: SpeechModels) {
        self.languages = languages
        self.microphone = microphone
        self.models = models
    }

    func start(on queue: DispatchQueue) {
        Microphone.authorize(on: queue) { [self] problem in
            if let problem {
                emit("wake.error", ["code": problem.code, "message": problem.message])
                return
            }
            if #available(macOS 14.0, *), models.parakeetPresent { models.load() }
            sink = try? microphone.add { [weak self] buffer in self?.heard(buffer) }
        }
    }

    func stop() {
        microphone.remove(sink)
        sink = nil
        microphone.queue.async { [self] in
            stretch = nil
            before = []
            pending = []
        }
    }

    /// On the microphone's queue.
    private func heard(_ buffer: AVAudioPCMBuffer) {
        pending.append(contentsOf: Microphone.samples(buffer))
        while pending.count >= Self.frame {
            let frame = Array(pending.prefix(Self.frame))
            pending.removeFirst(Self.frame)
            step(frame)
        }
    }

    private func step(_ frame: [Float]) {
        let level = BandAnalyzer.rms(frame[...])
        let speech = level > max(floor * 3, 0.006)
        // The floor follows quiet rooms down quickly and noisy ones up slowly.
        floor = speech ? floor : (level < floor ? level * 0.5 + floor * 0.5 : floor * 0.995 + level * 0.005)
        if var current = stretch {
            current.append(contentsOf: frame)
            quiet = speech ? 0 : quiet + 1
            if quiet >= Self.quietFrames || current.count >= Self.longest {
                stretch = nil
                recognize(current)
            } else {
                stretch = current
            }
        } else if speech {
            stretch = before + frame
            quiet = 0
            before = []
        } else {
            before.append(contentsOf: frame)
            if before.count > Self.preroll { before.removeFirst(before.count - Self.preroll) }
        }
    }

    private func recognize(_ samples: [Float]) {
        // Too short to be the phrase, or still busy with the last one.
        guard samples.count >= 9_600, !busy else { return }
        busy = true
        let languages = languages
        let models = models
        Task {
            let heard = await Recognizer.best(samples, languages: languages, models: models)
            self.microphone.queue.async { self.busy = false }
            if let text = heard?.text, !text.isEmpty { emit("wake.heard", ["text": text]) }
        }
    }
}
