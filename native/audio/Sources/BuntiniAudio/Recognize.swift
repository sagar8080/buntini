// Turning speech into words: Parakeet (FluidAudio) when its model was downloaded, Apple's
// on-device recognizer otherwise or for languages Parakeet doesn't know. With several
// languages each one runs, and the most confident answer wins.
import AVFoundation
import CoreML
import FluidAudio
import Foundation

struct Heard {
    let text: String
    let confidence: Double
    let language: String
    let engine: String
}

/// The folder Rust downloads into: parakeet/, diarizer/ and vad/.
final class SpeechModels: @unchecked Sendable {
    let folder: URL
    private let lock = NSLock()
    private var loading = false
    private var asrStorage: AnyObject?
    private var vadStorage: AnyObject?

    init(folder: URL) {
        self.folder = folder
    }

    var parakeetFolder: URL { folder.appendingPathComponent("parakeet", isDirectory: true) }
    var diarizerFolder: URL { folder.appendingPathComponent("diarizer", isDirectory: true) }
    var vadFolder: URL { folder.appendingPathComponent("vad", isDirectory: true) }

    /// The files are there (Rust only names a file once its hash matched).
    var parakeetPresent: Bool {
        guard #available(macOS 14.0, *) else { return false }
        let needed = ["Preprocessor.mlmodelc", "Encoder_v2.mlmodelc", "Decoder.mlmodelc", "JointDecisionv3.mlmodelc", "parakeet_vocab.json"]
        return needed.allSatisfy { FileManager.default.fileExists(atPath: parakeetFolder.appendingPathComponent($0).path) }
    }

    var diarizerPresent: Bool {
        guard #available(macOS 15.0, *) else { return false } // macOS 14's BNNS crashes in it (FluidAudio #661).
        let needed = ["Segmentation.mlmodelc", "FBank.mlmodelc", "Embedding.mlmodelc", "PldaRho.mlmodelc", "plda-parameters.json"]
        return needed.allSatisfy { FileManager.default.fileExists(atPath: diarizerFolder.appendingPathComponent($0).path) }
    }

    var vadPresent: Bool {
        FileManager.default.fileExists(atPath: vadFolder.appendingPathComponent("silero-vad-unified-256ms-v6.2.1.mlmodelc").path)
    }

    @available(macOS 14.0, *)
    var asr: AsrManager? {
        lock.lock()
        defer { lock.unlock() }
        return asrStorage as? AsrManager
    }

    var parakeetLoaded: Bool {
        guard #available(macOS 14.0, *) else { return false }
        return asr != nil
    }

    /// Loads Parakeet in the background, once. FluidAudio is in offline mode: anything that
    /// would fetch a file fails instead.
    func load(then: @escaping (Bool) -> Void = { _ in }) {
        guard #available(macOS 14.0, *), parakeetPresent else { return then(false) }
        lock.lock()
        if asrStorage != nil || loading {
            let loaded = asrStorage != nil
            lock.unlock()
            return then(loaded)
        }
        loading = true
        lock.unlock()
        let folder = parakeetFolder
        Task {
            var manager: AsrManager?
            do {
                let models = try AsrModels.loadLocal(from: folder, version: .v3, encoderPrecision: .int8V2)
                let asr = AsrManager(config: .default)
                try await asr.loadModels(models)
                manager = asr
            } catch {
                emit("log", ["message": "Parakeet didn't load"])
            }
            self.lock.withLock {
                self.asrStorage = manager
                self.loading = false
            }
            then(manager != nil)
        }
    }

    @available(macOS 14.0, *)
    func vad() -> VadManager? {
        lock.lock()
        defer { lock.unlock() }
        if let manager = vadStorage as? VadManager { return manager }
        guard vadPresent else { return nil }
        let configuration = MLModelConfiguration()
        configuration.computeUnits = .cpuAndNeuralEngine
        guard let model = try? MLModel(contentsOf: vadFolder.appendingPathComponent("silero-vad-unified-256ms-v6.2.1.mlmodelc"), configuration: configuration) else { return nil }
        let manager = VadManager(config: VadConfig(), vadModel: model)
        vadStorage = manager
        return manager
    }

    /// The speaker diarizer, built from the files by hand: FluidAudio's own loader would
    /// download.
    @available(macOS 14.0, *)
    func diarizer() -> OfflineDiarizerManager? {
        guard diarizerPresent else { return nil }
        func model(_ name: String, _ units: MLComputeUnits) throws -> MLModel {
            let configuration = MLModelConfiguration()
            configuration.computeUnits = units
            configuration.allowLowPrecisionAccumulationOnGPU = true
            return try MLModel(contentsOf: diarizerFolder.appendingPathComponent(name), configuration: configuration)
        }
        do {
            let started = Date()
            let segmentation = try model("Segmentation.mlmodelc", .all)
            let embedding = try model("Embedding.mlmodelc", .all)
            let rho = try model("PldaRho.mlmodelc", .all)
            let fbank = try model("FBank.mlmodelc", .cpuOnly)
            let data = try Data(contentsOf: diarizerFolder.appendingPathComponent("plda-parameters.json"))
            guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let tensors = root["tensors"] as? [String: Any], let psi = tensors["psi"] as? [String: Any],
                  let base64 = psi["data_base64"] as? String,
                  let raw = Data(base64Encoded: base64, options: .ignoreUnknownCharacters), raw.count >= 4 else { return nil }
            var values = [Float](repeating: 0, count: raw.count / 4)
            _ = values.withUnsafeMutableBytes { raw.copyBytes(to: $0) }
            let models = OfflineDiarizerModels(segmentationModel: segmentation, fbankModel: fbank, embeddingModel: embedding, pldaRhoModel: rho, pldaPsi: values.map(Double.init), compilationDuration: Date().timeIntervalSince(started))
            let manager = OfflineDiarizerManager(config: .default)
            manager.initialize(models: models)
            return manager
        } catch {
            return nil
        }
    }
}

/// Which languages to try: yours, or the Mac's.
enum Languages {
    static func wanted(_ tags: [String]) -> [String] {
        let tags = tags.filter { !$0.isEmpty }.prefix(3)
        if !tags.isEmpty { return Array(tags) }
        return [Locale.current.identifier.replacingOccurrences(of: "_", with: "-")]
    }

    static func code(_ tag: String) -> String {
        String(tag.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? "")
    }

    @available(macOS 14.0, *)
    static func parakeet(_ tag: String) -> Language? {
        Language(rawValue: code(tag))
    }
}

enum Recognizer {
    /// Parakeet on these samples, once per writing system among the languages.
    @available(macOS 14.0, *)
    static func parakeet(_ samples: [Float], languages: [String], models: SpeechModels) async -> Heard? {
        guard let asr = models.asr, samples.count >= 4_800 else { return nil }
        let known = languages.compactMap(Languages.parakeet)
        guard !known.isEmpty else { return nil }
        var tries: [Language] = []
        for language in known where !tries.contains(where: { $0.script == language.script }) { tries.append(language) }
        var best: Heard?
        for language in tries {
            var state = TdtDecoderState.make(decoderLayers: await asr.decoderLayerCount)
            guard let result = try? await asr.transcribe(samples, decoderState: &state, language: language) else { continue }
            let heard = Heard(text: tidy(result.text), confidence: Double(result.confidence), language: language.rawValue, engine: "parakeet")
            if best.map({ heard.confidence > $0.confidence }) ?? true { best = heard }
        }
        return best
    }

    /// Apple's recognizer on these samples, for one language (macOS 26).
    static func apple(_ samples: [Float], language: String) async -> Heard? {
        let queue = DispatchQueue(label: "buntini.audio.apple-once")
        let live: LiveText? = await withCheckedContinuation { continuation in
            prepareLive(queue: queue, download: false, locale: Locale(identifier: language), onPreparing: {}, onText: { _ in }) { live, _ in
                continuation.resume(returning: live)
            }
        }
        guard let live, let converter = AVAudioConverter(from: Microphone.format, to: live.format) else { return nil }
        var offset = 0
        while offset < samples.count {
            let end = min(samples.count, offset + 16_000)
            if let buffer = Microphone.buffer(samples[offset..<end]), let converted = Microphone.convert(buffer, with: converter) {
                live.append(converted)
            }
            offset = end
        }
        let text = await live.finish()
        return Heard(text: text, confidence: live.confidence, language: language, engine: "apple")
    }

    /// Recognizes recorded samples with whatever this Mac has, the most confident winning.
    static func best(_ samples: [Float], languages: [String], models: SpeechModels, engine: String? = nil) async -> Heard? {
        let wanted = Languages.wanted(languages)
        var candidates: [Heard] = []
        if engine != "apple", #available(macOS 14.0, *), models.parakeetLoaded {
            if let heard = await parakeet(samples, languages: wanted, models: models) { candidates.append(heard) }
        }
        let parakeetCovers: (String) -> Bool = { tag in
            guard #available(macOS 14.0, *), models.parakeetLoaded, engine != "apple" else { return false }
            return Languages.parakeet(tag) != nil
        }
        if engine != "parakeet" {
            for language in wanted where !parakeetCovers(language) || engine == "apple" {
                if let heard = await apple(samples, language: language) { candidates.append(heard) }
            }
        }
        return candidates.filter { !$0.text.isEmpty }.max { $0.confidence < $1.confidence } ?? candidates.first
    }
}
