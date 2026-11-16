// Opens the real island with scenes from a fixture and prints every action.
// With no index it steps through all scenes; Ctrl-C or --seconds ends it.
import AppKit
import BuntiniSurface

let arguments = Array(CommandLine.arguments.dropFirst())
guard let path = arguments.first else {
    FileHandle.standardError.write("Usage: SurfaceLab <fixture.json> [index] [--seconds N]\n".data(using: .utf8)!)
    exit(64)
}
let seconds = arguments.firstIndex(of: "--seconds").flatMap { arguments.indices.contains($0 + 1) ? Double(arguments[$0 + 1]) : nil }
let index = arguments.dropFirst().first.flatMap(Int.init)

guard let data = FileManager.default.contents(atPath: path),
      let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let raw = root["scenes"] as? [Any] else {
    FileHandle.standardError.write("Could not read scenes from \(path)\n".data(using: .utf8)!)
    exit(65)
}
let scenes = raw.compactMap { try? JSONSerialization.data(withJSONObject: $0) }.compactMap(SurfaceRuntime.decode)
guard scenes.count == raw.count else {
    FileHandle.standardError.write("\(raw.count - scenes.count) scenes did not decode\n".data(using: .utf8)!)
    exit(65)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let runtime = SurfaceRuntime.shared
runtime.start { action in
    print("action", String(decoding: action, as: UTF8.self))
    fflush(stdout)
}

if let index {
    guard scenes.indices.contains(index) else { exit(64) }
    runtime.show(scenes[index])
} else {
    var next = 0
    func step() {
        print("scene", next, scenes[next].card.map { "\($0.body.kind)" } ?? "-")
        fflush(stdout)
        runtime.show(scenes[next])
        next = (next + 1) % scenes.count
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.5, execute: step)
    }
    step()
}
if let seconds {
    DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { exit(0) }
}
app.run()
