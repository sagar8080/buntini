// Records a call: the microphone and everything the Mac plays, through a Core Audio process
// tap. Both go into one private aggregate device, so they share one clock and one time
// origin: in each buffer list the microphone comes first and the tap's buffers last. Each side
// is written as 16 kHz mono 16-bit samples to a private file in Buntini's cache, which Rust
// deletes as soon as the transcript is saved.
import AVFoundation
import CoreAudio
import Foundation

@available(macOS 14.2, *)
final class CallRecorder: @unchecked Sendable {
    let id: String
    let micURL: URL
    let systemURL: URL
    /// Live words every so often, so the call can be followed while it runs.
    static let liveChunk = Int(Microphone.rate) * 20

    private let models: SpeechModels
    private let languages: [String]
    private let io = DispatchQueue(label: "buntini.audio.call-io", qos: .userInteractive)
    private let work = DispatchQueue(label: "buntini.audio.call-work")
    private var tap = AudioObjectID(kAudioObjectUnknown)
    private var aggregate = AudioObjectID(kAudioObjectUnknown)
    private var proc: AudioDeviceIOProcID?
    private var micFile: FileHandle?
    private var systemFile: FileHandle?
    private var micSide: Side?
    private var systemSide: Side?
    private var frames = 0
    private var stopped = false

    /// One side's conversion to 16 kHz and its stretch waiting for live words.
    private final class Side {
        let converter: AVAudioConverter
        let input: AVAudioFormat
        var live: [Float] = []
        var liveStart = 0
        init?(rate: Double) {
            guard let input = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false),
                  let converter = AVAudioConverter(from: input, to: Microphone.format) else { return nil }
            self.input = input
            self.converter = converter
        }
    }

    init(id: String, folder: URL, languages: [String], models: SpeechModels) {
        self.id = id
        self.micURL = folder.appendingPathComponent("\(id)-mic.pcm")
        self.systemURL = folder.appendingPathComponent("\(id)-system.pcm")
        self.languages = languages
        self.models = models
    }

    private static func address(_ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }

    private static func defaultInput() -> AudioObjectID? {
        var where_ = address(kAudioHardwarePropertyDefaultInputDevice)
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &where_, 0, nil, &size, &device) == noErr, device != kAudioObjectUnknown else { return nil }
        return device
    }

    private static func rate(_ device: AudioObjectID) -> Double {
        var where_ = address(kAudioDevicePropertyNominalSampleRate)
        var rate: Float64 = 0
        var size = UInt32(MemoryLayout<Float64>.size)
        AudioObjectGetPropertyData(device, &where_, 0, nil, &size, &rate)
        return rate > 0 ? rate : 48_000
    }

    private static func privateFile(_ url: URL) throws -> FileHandle {
        let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { throw CaptureError(code: "output_unavailable", message: "The recording couldn't be saved.") }
        return FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    }

    func start() throws {
        guard let input = Self.defaultInput(), let inputUID = OutputMute.uid(input) else {
            throw CaptureError(code: "microphone_unavailable", message: "No microphone is available.")
        }
        let description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
        description.uuid = UUID()
        description.name = "Buntini call notes"
        description.isPrivate = true
        description.muteBehavior = .unmuted
        guard AudioHardwareCreateProcessTap(description, &tap) == noErr, tap != kAudioObjectUnknown else {
            throw CaptureError(code: "system_audio_denied", message: "Allow Buntini to record the Mac's sound in System Settings → Privacy & Security → Screen & System Audio Recording.")
        }
        let settings: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Buntini call",
            kAudioAggregateDeviceUIDKey: "app.buntini.call.\(UUID().uuidString)",
            kAudioAggregateDeviceMainSubDeviceKey: inputUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: inputUID]],
            kAudioAggregateDeviceTapListKey: [[kAudioSubTapDriftCompensationKey: true, kAudioSubTapUIDKey: description.uuid.uuidString]],
        ]
        guard AudioHardwareCreateAggregateDevice(settings as CFDictionary, &aggregate) == noErr else {
            tearDown()
            throw CaptureError(code: "call_audio_failed", message: "The call's sound couldn't be opened.")
        }
        let rate = Self.rate(aggregate)
        guard let mic = Side(rate: rate), let system = Side(rate: rate) else {
            tearDown()
            throw CaptureError(code: "conversion_unavailable", message: "The call's sound can't be converted.")
        }
        micSide = mic
        systemSide = system
        micFile = try Self.privateFile(micURL)
        systemFile = try Self.privateFile(systemURL)
        let status = AudioDeviceCreateIOProcIDWithBlock(&proc, aggregate, io) { [weak self] _, input, _, _, _ in
            self?.received(input)
        }
        guard status == noErr, let proc, AudioDeviceStart(aggregate, proc) == noErr else {
            tearDown()
            throw CaptureError(code: "call_audio_failed", message: "The call's sound couldn't be opened.")
        }
    }

    /// The microphone's buffers come first and the tap's last.
    private func received(_ list: UnsafePointer<AudioBufferList>) {
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: list))
        guard buffers.count >= 2 else { return }
        let mic = Self.mono(buffers[0])
        let system = Self.mono(buffers[buffers.count - 1])
        work.async { [self] in
            guard !stopped, let micSide, let systemSide else { return }
            let micOut = Self.convert(mic, side: micSide)
            var systemOut = Self.convert(system, side: systemSide)
            // Keep both sides the same length, so they keep one time origin.
            if systemOut.count < micOut.count { systemOut += [Float](repeating: 0, count: micOut.count - systemOut.count) }
            systemOut = Array(systemOut.prefix(micOut.count))
            write(micOut, to: micFile)
            write(systemOut, to: systemFile)
            live(micOut, side: micSide, name: "mic")
            live(systemOut, side: systemSide, name: "system")
            frames += micOut.count
        }
    }

    private static func mono(_ buffer: AudioBuffer) -> [Float] {
        guard let data = buffer.mData, buffer.mNumberChannels > 0 else { return [] }
        let channels = Int(buffer.mNumberChannels)
        let count = Int(buffer.mDataByteSize) / MemoryLayout<Float>.size / channels
        let samples = data.assumingMemoryBound(to: Float.self)
        var out = [Float](repeating: 0, count: count)
        for frame in 0..<count {
            var sum: Float = 0
            for channel in 0..<channels { sum += samples[frame * channels + channel] }
            out[frame] = sum / Float(channels)
        }
        return out
    }

    private static func convert(_ samples: [Float], side: Side) -> [Float] {
        guard !samples.isEmpty, let buffer = AVAudioPCMBuffer(pcmFormat: side.input, frameCapacity: AVAudioFrameCount(samples.count)) else { return [] }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { buffer.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count) }
        return Microphone.convert(buffer, with: side.converter).map(Microphone.samples) ?? []
    }

    private func write(_ samples: [Float], to file: FileHandle?) {
        guard let file, !samples.isEmpty else { return }
        var pcm = [Int16](repeating: 0, count: samples.count)
        for (index, value) in samples.enumerated() { pcm[index] = Int16(max(-1, min(1, value)) * 32_767) }
        pcm.withUnsafeBytes { file.write(Data($0)) }
    }

    /// Every 20 seconds, the words of that stretch on each side.
    private func live(_ samples: [Float], side: Side, name: String) {
        if side.live.isEmpty { side.liveStart = frames }
        side.live.append(contentsOf: samples)
        guard side.live.count >= Self.liveChunk else { return }
        let chunk = side.live
        let start = Double(side.liveStart) / Microphone.rate
        side.live = []
        guard BandAnalyzer.rms(chunk[...]) > 0.004 else { return }
        let id = id, languages = languages, models = models
        Task {
            guard let heard = await Recognizer.best(chunk, languages: languages, models: models, engine: "parakeet"), !heard.text.isEmpty else { return }
            emit("notes.live", ["id": id, "side": name, "start": start, "end": start + Double(chunk.count) / Microphone.rate, "text": heard.text])
        }
    }

    /// Stops recording; returns the seconds recorded once the files are complete.
    func stop() -> Double {
        if let proc { AudioDeviceStop(aggregate, proc) }
        tearDown()
        return work.sync {
            stopped = true
            try? micFile?.close()
            try? systemFile?.close()
            micFile = nil
            systemFile = nil
            return Double(frames) / Microphone.rate
        }
    }

    /// Stops and deletes what was recorded.
    func discard() {
        _ = stop()
        try? FileManager.default.removeItem(at: micURL)
        try? FileManager.default.removeItem(at: systemURL)
    }

    private func tearDown() {
        if let proc, aggregate != kAudioObjectUnknown { AudioDeviceDestroyIOProcID(aggregate, proc) }
        proc = nil
        if aggregate != kAudioObjectUnknown { AudioHardwareDestroyAggregateDevice(aggregate) }
        aggregate = AudioObjectID(kAudioObjectUnknown)
        if tap != kAudioObjectUnknown { AudioHardwareDestroyProcessTap(tap) }
        tap = AudioObjectID(kAudioObjectUnknown)
    }

    /// A side's file as samples.
    static func read(_ url: URL) -> [Float] {
        guard let data = try? Data(contentsOf: url) else { return [] }
        return data.withUnsafeBytes { raw in raw.bindMemory(to: Int16.self).map { Float($0) / 32_767 } }
    }
}
