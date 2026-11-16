// The island's few sounds: short, soft tones made here rather than recorded, so no sound
// files ship with Buntini. A file named like one of them (ask.wav, done.mp3, hello.m4a,
// drop.aiff) in Buntini's Sounds folder plays instead. Rust decides when a sound may play
// (the Mac's UI sounds on, the microphone free, no Focus).
import AVFoundation
import Foundation

final class SoundPlayer {
    private var players: [String: AVAudioPlayer] = [:]
    private var last: UInt64 = 0

    /// Plays `sound` once per new sequence number.
    func play(_ name: String, sequence: UInt64) {
        guard sequence > last else { return }
        last = sequence
        guard let player = player(name) else { return }
        player.currentTime = 0
        player.play()
    }

    private func player(_ name: String) -> AVAudioPlayer? {
        if let player = players[name] { return player }
        let player = own(name).flatMap { try? AVAudioPlayer(contentsOf: $0) } ?? SoundPlayer.tone(name).flatMap { try? AVAudioPlayer(data: $0) }
        player?.volume = 0.35
        player?.prepareToPlay()
        players[name] = player
        return player
    }

    /// A sound of the person's own, if they put one in Buntini's Sounds folder.
    private func own(_ name: String) -> URL? {
        guard let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        let folder = support.appendingPathComponent("app.buntini.companion/Sounds", isDirectory: true)
        return ["wav", "mp3", "m4a", "aiff", "caf"].map { folder.appendingPathComponent("\(name).\($0)") }.first { FileManager.default.fileExists(atPath: $0.path) }
    }

    /// Notes as (frequency in Hz, seconds); 0 Hz is a rest.
    private static func notes(_ name: String) -> [(Double, Double)]? {
        switch name {
        case "ask": return [(659.25, 0.09), (0, 0.03), (880.0, 0.14)]
        case "done": return [(523.25, 0.08), (659.25, 0.08), (783.99, 0.16)]
        case "hello": return [(587.33, 0.07), (880.0, 0.12)]
        case "drop": return [(392.0, 0.06), (261.63, 0.09)]
        default: return nil
        }
    }

    /// A 16-bit mono WAV of soft bell-like tones: a sine with a quick attack and a gentle fall.
    static func tone(_ name: String) -> Data? {
        guard let notes = notes(name) else { return nil }
        let rate = 44_100.0
        var samples: [Int16] = []
        for (frequency, seconds) in notes {
            let count = Int(seconds * rate)
            for index in 0..<count {
                let t = Double(index) / rate
                guard frequency > 0 else { samples.append(0); continue }
                let attack = min(1, t / 0.006)
                let fall = exp(-t * 18)
                let value = (sin(2 * .pi * frequency * t) + 0.25 * sin(4 * .pi * frequency * t)) * attack * fall * 0.6
                samples.append(Int16(max(-1, min(1, value)) * Double(Int16.max)))
            }
        }
        var data = Data()
        func append<T: FixedWidthInteger>(_ value: T) { withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) } }
        let bytes = samples.count * 2
        data.append(contentsOf: Array("RIFF".utf8)); append(UInt32(36 + bytes))
        data.append(contentsOf: Array("WAVE".utf8)); data.append(contentsOf: Array("fmt ".utf8))
        append(UInt32(16)); append(UInt16(1)); append(UInt16(1)); append(UInt32(rate)); append(UInt32(rate) * 2); append(UInt16(2)); append(UInt16(16))
        data.append(contentsOf: Array("data".utf8)); append(UInt32(bytes))
        for sample in samples { append(sample) }
        return data
    }
}

/// Makes every built-in sound, for SurfaceChecks. Nil when they all play.
public enum SurfaceSounds {
    public static func problem() -> String? {
        for name in ["ask", "done", "hello", "drop"] {
            guard let data = SoundPlayer.tone(name), let player = try? AVAudioPlayer(data: data), player.duration > 0.1 else {
                return "the \(name) sound doesn't play"
            }
        }
        return nil
    }
}
