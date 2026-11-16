// Opens the real island with scenes from a fixture and prints every action.
// With no index it steps through all scenes; Ctrl-C or --seconds ends it.
import AppKit
import BuntiniSurface

let arguments = Array(CommandLine.arguments.dropFirst())

// --recap <out.png>: the week's share picture, from a sample week.
if let flag = arguments.firstIndex(of: "--recap"), arguments.indices.contains(flag + 1) {
    let sample = #"{"week":"9–15 Nov","stats":[{"label":"Time","value":"6 h 40 min"},{"label":"Sessions","value":"14"},{"label":"Turns","value":"61"},{"label":"Lines","value":"+2,310 −480"},{"label":"Files","value":"37"},{"label":"Commands","value":"212"},{"label":"Top agent","value":"Claude Code"},{"label":"Busiest day","value":"Tuesday"},{"label":"Longest session","value":"1 h 52 min"}]}"#
    let data = Array(sample.utf8)
    let made = data.withUnsafeBufferPointer { bytes in
        arguments[flag + 1].withCString { path in buntiniAgentsRecapPNG(bytes.baseAddress, bytes.count, path) }
    }
    print(made ? "written" : "failed")
    exit(made ? 0 : 1)
}
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
