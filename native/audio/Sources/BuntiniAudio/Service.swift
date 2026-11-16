// `buntini-audio --service --models <folder>`: one long-lived helper for dictation, call
// notes, the wake word and muting, so Parakeet loads once. Commands arrive on stdin as JSON
// lines with an "op"; events go out on stdout with an "event" (protocol 3):
//
//   load                                   → loaded {parakeet}
//   dictate {id, languages, input?}        → listening {id, engine}, bands {id, values},
//                                            partial {id, text}, then final or cancelled or error
//   stop {id} / cancel {id}
//   mute                                   → muted {devices}
//   unmute {devices?}                      → unmuted
//   notes {id, folder}                     → notes.recording {id}, notes.live {…}, then on
//   notesStop {id}                            notes.done {id, seconds, segments} or notes.error
//   notesFile {id, mic?, system}           → notes.done (a recorded call, for checks)
//   wake {on, languages}                   → wake.heard {text}
//   transcribe {id, path, engine?, languages} → transcribed {id, text, confidence, engine, ms}
//   quit
//
// `input` plays a recorded file instead of the microphone, for checks without a voice.
//
// When stdin closes (Buntini quit or crashed), anything muted is unmuted and an unfinished
// call's audio is deleted before the helper exits.
import FluidAudio
import Foundation

final class Service: @unchecked Sendable {
    static let protocolVersion = 3

    private let queue = DispatchQueue(label: "buntini.audio.service")
    private let models: SpeechModels
    private let microphone = Microphone()
    private let mute = MuteState()
    private var dictations: [Int: Dictation] = [:]
    private var call: AnyObject?
    private var callID: String?
    private var wake: WakeListener?
    private var signals: [DispatchSourceSignal] = []

    private init(models: URL) {
        self.models = SpeechModels(folder: models)
    }

    static func main() -> Never {
        let arguments = CommandLine.arguments
        guard let index = arguments.firstIndex(of: "--models"), arguments.indices.contains(index + 1), arguments[index + 1].hasPrefix("/") else {
            emit("error", ["code": "invalid_arguments", "message": "--service needs --models /absolute/folder"])
            exit(EX_USAGE)
        }
        // Nothing FluidAudio runs may fetch a file; Rust downloads and checks them.
        if #available(macOS 14.0, *) { ModelHub.offlineMode = true }
        Service(models: URL(fileURLWithPath: arguments[index + 1], isDirectory: true)).run()
    }

    private func run() -> Never {
        signal(SIGPIPE, SIG_IGN)
        for sig in [SIGTERM, SIGINT, SIGHUP] {
            signal(sig, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: sig, queue: queue)
            source.setEventHandler { [weak self] in self?.quit() }
            source.resume()
            signals.append(source)
        }
        let parakeet: Bool
        if #available(macOS 14.0, *) { parakeet = models.parakeetPresent } else { parakeet = false }
        emit("ready", ["protocolVersion": Self.protocolVersion, "parakeet": parakeet, "diarizer": models.diarizerPresent])
        DispatchQueue.global(qos: .userInitiated).async {
            while let line = readLine(strippingNewline: true) {
                guard line.utf8.count <= 1 << 16,
                      let data = line.data(using: .utf8),
                      let command = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    emit("error", ["code": "invalid_command", "message": "Expected a JSON command."])
                    continue
                }
                self.queue.async { self.handle(command) }
            }
            self.queue.async { self.quit() }
        }
        dispatchMain()
    }

    private func handle(_ command: [String: Any]) {
        let id = command["id"] as? Int
        let languages = (command["languages"] as? [String]) ?? []
        switch command["op"] as? String {
        case "load":
            models.load { loaded in emit("loaded", ["parakeet": loaded]) }
        case "dictate":
            guard let id else { return }
            dictations[id]?.cancel()
            let input = (command["input"] as? String).flatMap { $0.hasPrefix("/") ? URL(fileURLWithPath: $0) : nil }
            let dictation = Dictation(id: id, languages: languages, microphone: microphone, models: models, input: input)
            dictations[id] = dictation
            dictation.start(on: queue)
        case "stop":
            guard let id, let dictation = dictations[id] else { return }
            dictation.stop { [weak self] in self?.queue.async { self?.dictations[id] = nil } }
        case "cancel":
            guard let id, let dictation = dictations.removeValue(forKey: id) else { return }
            dictation.cancel()
        case "mute":
            emit("muted", ["devices": mute.mute()])
        case "unmute":
            mute.unmute(command["devices"] as? [String])
            emit("unmuted")
        case "notes":
            startCall(id: command["id"] as? String, folder: command["folder"] as? String, languages: languages)
        case "notesStop":
            stopCall(id: command["id"] as? String, languages: languages)
        case "notesFile":
            notesFromFiles(id: command["id"] as? String, mic: command["mic"] as? String, system: command["system"] as? String, languages: languages)
        case "wake":
            wake?.stop()
            wake = nil
            if command["on"] as? Bool == true {
                let listener = WakeListener(languages: languages, microphone: microphone, models: models)
                wake = listener
                listener.start(on: queue)
            }
        case "transcribe":
            transcribe(id: id, path: command["path"] as? String, engine: command["engine"] as? String, languages: languages)
        case "quit":
            quit()
        default:
            emit("error", ["code": "invalid_command", "message": "Unknown command."])
        }
    }

    // MARK: Calls

    private func startCall(id: String?, folder: String?, languages: [String]) {
        guard let id, let folder, folder.hasPrefix("/"), id.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" }) else { return }
        guard call == nil else {
            return emit("notes.error", ["id": id, "code": "busy", "message": "A call is already recording."])
        }
        guard #available(macOS 14.2, *) else {
            return emit("notes.error", ["id": id, "code": "unsupported", "message": "Recording calls needs macOS 14.2 or later."])
        }
        Microphone.authorize(on: queue) { [self] problem in
            if let problem { return emit("notes.error", ["id": id, "code": problem.code, "message": problem.message]) }
            if models.parakeetPresent { models.load() }
            let recorder = CallRecorder(id: id, folder: URL(fileURLWithPath: folder, isDirectory: true), languages: languages, models: models)
            do {
                try recorder.start()
                call = recorder
                callID = id
                emit("notes.recording", ["id": id])
            } catch {
                recorder.discard()
                let problem = error as? CaptureError
                emit("notes.error", ["id": id, "code": problem?.code ?? "call_audio_failed", "message": problem?.message ?? "The call couldn't be recorded."])
            }
        }
    }

    private func stopCall(id: String?, languages: [String]) {
        guard #available(macOS 14.2, *), let recorder = call as? CallRecorder, recorder.id == id else { return }
        call = nil
        callID = nil
        let seconds = recorder.stop()
        let models = models
        Task {
            let mic = CallRecorder.read(recorder.micURL)
            let system = CallRecorder.read(recorder.systemURL)
            let segments = await CallNotes.process(mic: mic, system: system, languages: languages, models: models)
            emit("notes.done", ["id": recorder.id, "seconds": seconds, "segments": segments])
        }
    }

    private func notesFromFiles(id: String?, mic: String?, system: String?, languages: [String]) {
        guard let id, let system, system.hasPrefix("/") else { return }
        let models = models
        Task {
            do {
                let other = try Microphone.read(URL(fileURLWithPath: system))
                var own = [Float](repeating: 0, count: other.count)
                if let mic, mic.hasPrefix("/") { own = try Microphone.read(URL(fileURLWithPath: mic)) }
                let length = max(own.count, other.count)
                own += [Float](repeating: 0, count: length - own.count)
                let padded = other + [Float](repeating: 0, count: length - other.count)
                let segments = await CallNotes.process(mic: own, system: padded, languages: languages, models: models)
                emit("notes.done", ["id": id, "seconds": Double(length) / Microphone.rate, "segments": segments])
            } catch {
                emit("notes.error", ["id": id, "code": "input_unavailable", "message": "That recording can't be read."])
            }
        }
    }

    // MARK: Checks on recorded files

    private func transcribe(id: Int?, path: String?, engine: String?, languages: [String]) {
        guard let path, path.hasPrefix("/") else { return }
        let models = models
        Task {
            guard let samples = try? Microphone.read(URL(fileURLWithPath: path)) else {
                return emit("error", ["id": id ?? 0, "code": "input_unavailable", "message": "That file can't be read."])
            }
            if engine != "apple", #available(macOS 14.0, *), models.parakeetPresent, !models.parakeetLoaded {
                await withCheckedContinuation { continuation in models.load { _ in continuation.resume() } }
            }
            let started = Date()
            let heard = await Recognizer.best(samples, languages: languages, models: models, engine: engine)
            emit("transcribed", ["id": id ?? 0, "text": heard?.text ?? "", "confidence": heard?.confidence ?? 0, "engine": heard?.engine ?? "none", "language": heard?.language ?? "", "ms": Int(Date().timeIntervalSince(started) * 1000)])
        }
    }

    // MARK: Leaving

    private func quit() -> Never {
        for dictation in dictations.values { dictation.cancel() }
        wake?.stop()
        if #available(macOS 14.2, *), let recorder = call as? CallRecorder { recorder.discard() }
        mute.unmute()
        // Let the last events out.
        Thread.sleep(forTimeInterval: 0.05)
        exit(EXIT_SUCCESS)
    }
}
