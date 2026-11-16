// Writing up a recorded call: the speakers' sound is removed from the microphone (echo),
// your side is cut where you speak, the other side is split by speaker (diarization), and
// each stretch is recognized. Rust merges the stretches into lines speaker by speaker.
import Accelerate
import FluidAudio
import Foundation

enum CallNotes {
    static let frame = 320 // 20 ms at 16 kHz
    /// Stretches are recognized at most this long.
    static let longest = 20.0

    struct Stretch {
        var start: Double
        var end: Double
        var speaker: Int
    }

    static func process(mic: [Float], system: [Float], languages: [String], models: SpeechModels) async -> [[String: Any]] {
        let clean = removeEcho(mic: mic, system: system)
        var segments: [[String: Any]] = []
        for stretch in await speech(clean, models: models) {
            if let text = await words(clean, stretch, languages: languages, models: models) {
                segments.append(["side": "mic", "speaker": 0, "start": stretch.start, "end": stretch.end, "text": text])
            }
        }
        for stretch in await speakers(system, models: models) {
            if let text = await words(system, stretch, languages: languages, models: models) {
                segments.append(["side": "system", "speaker": stretch.speaker, "start": stretch.start, "end": stretch.end, "text": text])
            }
        }
        return segments.sorted { ($0["start"] as? Double ?? 0) < ($1["start"] as? Double ?? 0) }
    }

    private static func words(_ samples: [Float], _ stretch: Stretch, languages: [String], models: SpeechModels) async -> String? {
        // A little either side, so a first or last word cut by the detector isn't lost.
        let from = max(0, Int((stretch.start - 0.25) * Microphone.rate))
        let to = min(samples.count, Int((stretch.end + 0.25) * Microphone.rate))
        guard to - from >= 4_800 else { return nil }
        let heard = await Recognizer.best(Array(samples[from..<to]), languages: languages, models: models)
        return heard.map(\.text).flatMap { $0.isEmpty ? nil : $0 }
    }

    // MARK: Echo

    private static func energies(_ samples: [Float]) -> [Float] {
        stride(from: 0, to: samples.count, by: frame).map { start in
            let slice = samples[start..<min(samples.count, start + frame)]
            return slice.withUnsafeBufferPointer { vDSP.sumOfSquares($0) } / Float(max(1, slice.count))
        }
    }

    /// Where the microphone only carries the speakers (a little later and quieter), it is
    /// silenced; where you talk over them it is kept. With headphones nothing changes.
    static func removeEcho(mic: [Float], system: [Float]) -> [Float] {
        let micEnergy = energies(mic)
        let systemEnergy = energies(system)
        let count = min(micEnergy.count, systemEnergy.count)
        guard count > 50 else { return mic }
        let loud = max(1e-6, (systemEnergy.max() ?? 0) * 0.01)
        let active = (0..<count).filter { systemEnergy[$0] > loud }
        guard active.count > 25 else { return mic }
        // How late the speakers reach the microphone, up to 300 ms.
        let delay = (0...15).max { a, b in correlation(micEnergy, systemEnergy, lag: a, frames: active) < correlation(micEnergy, systemEnergy, lag: b, frames: active) } ?? 0
        // How loud they arrive: the typical ratio while only the far side talks.
        let ratios = active.compactMap { index -> Float? in
            let source = index - delay
            guard source >= 0, systemEnergy[source] > loud else { return nil }
            return micEnergy[index] / systemEnergy[source]
        }.sorted()
        guard !ratios.isEmpty else { return mic }
        let coupling = ratios[ratios.count / 3]
        var out = mic
        for index in 0..<count {
            let source = index - delay
            guard source >= 0 else { continue }
            let near = (max(0, source - 1)...min(count - 1, source + 1)).map { systemEnergy[$0] }.max() ?? 0
            let echo = coupling * near
            // Your voice is well above what the speakers would explain.
            if near > loud, micEnergy[index] < echo * 4 {
                for sample in index * frame..<min(out.count, (index + 1) * frame) { out[sample] = 0 }
            }
        }
        return out
    }

    /// How well the microphone's loudness follows the speakers' `lag` frames later.
    private static func correlation(_ mic: [Float], _ system: [Float], lag: Int, frames: [Int]) -> Float {
        let pairs = frames.compactMap { index -> (Float, Float)? in
            index + lag < mic.count ? (log10(mic[index + lag] + 1e-9), log10(system[index] + 1e-9)) : nil
        }
        guard pairs.count > 2 else { return -1 }
        let meanMic = pairs.map(\.0).reduce(0, +) / Float(pairs.count)
        let meanSystem = pairs.map(\.1).reduce(0, +) / Float(pairs.count)
        var both: Float = 0, micOnly: Float = 0, systemOnly: Float = 0
        for (a, b) in pairs {
            both += (a - meanMic) * (b - meanSystem)
            micOnly += (a - meanMic) * (a - meanMic)
            systemOnly += (b - meanSystem) * (b - meanSystem)
        }
        return both / sqrt(micOnly * systemOnly + 1e-12)
    }

    // MARK: Where someone speaks

    /// Your side: where the voice detector hears speech (or loudness, without it).
    static func speech(_ samples: [Float], models: SpeechModels) async -> [Stretch] {
        var found: [Stretch] = []
        if #available(macOS 14.0, *), let vad = models.vad(), let segments = try? await vad.segmentSpeech(samples) {
            found = segments.map { Stretch(start: $0.startTime, end: $0.endTime, speaker: 0) }
        } else {
            found = loudness(samples)
        }
        return split(found)
    }

    /// Stretches louder than the room, joined across short pauses.
    static func loudness(_ samples: [Float]) -> [Stretch] {
        let levels = energies(samples).map { sqrt($0) }
        let sorted = levels.sorted()
        let room = sorted.isEmpty ? 0 : sorted[sorted.count / 5]
        let threshold = max(0.01, room * 3)
        var out: [Stretch] = []
        var start: Int?
        var quiet = 0
        for (index, level) in levels.enumerated() {
            if level > threshold {
                if start == nil { start = index }
                quiet = 0
            } else if let from = start {
                quiet += 1
                if quiet > 25 {
                    out.append(Stretch(start: Double(from * frame) / Microphone.rate, end: Double((index - quiet + 1) * frame) / Microphone.rate, speaker: 0))
                    start = nil
                    quiet = 0
                }
            }
        }
        if let from = start { out.append(Stretch(start: Double(from * frame) / Microphone.rate, end: Double(levels.count * frame) / Microphone.rate, speaker: 0)) }
        return out.filter { $0.end - $0.start >= 0.4 }
    }

    /// The other side, by speaker. Without the diarizer, everyone there is one speaker.
    static func speakers(_ samples: [Float], models: SpeechModels) async -> [Stretch] {
        guard BandAnalyzer.rms(samples[...]) > 0.0005 else { return [] }
        if #available(macOS 14.0, *), let diarizer = models.diarizer(), let result = try? await diarizer.process(audio: samples) {
            var ids: [String: Int] = [:]
            var stretches: [Stretch] = []
            for segment in result.segments.sorted(by: { $0.startTimeSeconds < $1.startTimeSeconds }) {
                let speaker = ids[segment.speakerId] ?? {
                    let next = ids.count + 1
                    ids[segment.speakerId] = next
                    return next
                }()
                let stretch = Stretch(start: Double(segment.startTimeSeconds), end: Double(segment.endTimeSeconds), speaker: speaker)
                // One speaker's pieces close together read as one stretch.
                if var last = stretches.last, last.speaker == speaker, stretch.start - last.end < 0.8, stretch.end - last.start <= longest {
                    last.end = stretch.end
                    stretches[stretches.count - 1] = last
                } else {
                    stretches.append(stretch)
                }
            }
            return split(widen(stretches, to: await speech(samples, models: models)))
        }
        return split(await speech(samples, models: models).map { Stretch(start: $0.start, end: $0.end, speaker: 1) })
    }

    /// The diarizer can start a speaker's stretch late (it needs a second of context): each
    /// stretch reaches back and forward to the speech the voice detector heard around it,
    /// never into its neighbours.
    static func widen(_ stretches: [Stretch], to speech: [Stretch]) -> [Stretch] {
        var out = stretches
        for index in out.indices {
            let previous = index > 0 ? out[index - 1].end : 0
            let next = index + 1 < out.count ? out[index + 1].start : .infinity
            for heard in speech where heard.start < out[index].end && heard.end > out[index].start {
                out[index].start = max(previous, min(out[index].start, heard.start))
                out[index].end = min(next, max(out[index].end, heard.end))
            }
        }
        return out
    }

    /// Long stretches cut into pieces the recognizer handles well.
    static func split(_ stretches: [Stretch]) -> [Stretch] {
        stretches.flatMap { stretch -> [Stretch] in
            guard stretch.end - stretch.start > longest else { return [stretch] }
            return stride(from: stretch.start, to: stretch.end, by: longest).map { start in
                Stretch(start: start, end: min(stretch.end, start + longest), speaker: stretch.speaker)
            }
        }
    }
}
