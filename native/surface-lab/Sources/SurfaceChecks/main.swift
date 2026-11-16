// Checks the Swift side of the surface contract against the fixture Rust also checks
// (src-tauri/src/surface/mod.rs): everything decodes, writes back the same, and every card
// kind is covered. Exits 1 on the first failure.
import BuntiniSurface
import Foundation
import SurfaceKit

struct Fixture: Codable {
    let scenes: [SurfaceScene]
    let actions: [Action]
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write("FAIL: \(message)\n".data(using: .utf8)!)
    exit(1)
}

/// Swift leaves out nil optionals where Rust writes null, so compare without nulls.
func withoutNulls(_ value: Any) -> Any {
    if let object = value as? [String: Any] {
        return object.filter { !($0.value is NSNull) }.mapValues(withoutNulls)
    }
    if let array = value as? [Any] { return array.map(withoutNulls) }
    return value
}

let path = CommandLine.arguments.dropFirst().first ?? "../surface/Fixtures/contract.json"
guard let data = FileManager.default.contents(atPath: path) else { fail("no fixture at \(path)") }

let fixture: Fixture
do {
    fixture = try JSONDecoder().decode(Fixture.self, from: data)
} catch {
    fail("the fixture doesn't decode: \(error)")
}

guard fixture.scenes.allSatisfy({ $0.contract == surfaceContract }) else { fail("a scene speaks another contract") }

let kinds = Set(fixture.scenes.compactMap { $0.card?.body.kind })
let missing = Set(CardBody.Kind.allCases).subtracting(kinds)
guard missing.isEmpty else { fail("no scene shows \(missing.map(\.rawValue).sorted())") }

let original = withoutNulls(try! JSONSerialization.jsonObject(with: data)) as! NSDictionary
let written = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(fixture)) as! NSDictionary
guard original.isEqual(written) else {
    for key in ["scenes", "actions"] {
        let before = original[key] as! [Any], after = written[key] as! [Any]
        for (offset, pair) in zip(before, after).enumerated() where !(pair.0 as AnyObject).isEqual(pair.1) {
            fail("\(key)[\(offset)] writes back differently:\n\(pair.0)\n\(pair.1)")
        }
    }
    fail("the fixture writes back differently")
}

if let problem = SurfaceShaders.problem() { fail("a shader doesn't compile: \(problem)") }
if let problem = SurfaceSounds.problem() { fail(problem) }

print("Surface contract \(surfaceContract): \(fixture.scenes.count) scenes, \(kinds.count) card kinds, \(fixture.actions.count) actions; shaders compile; sounds play")
