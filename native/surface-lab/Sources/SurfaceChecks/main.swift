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

// The fixture's words are locales/en.json's (the Rust tests check that), so a word the views
// draw that isn't among them is missing from the catalogs.
guard let words = fixture.scenes.first?.words else { fail("the first scene brings no words") }
let unknown = Lexicon.missing(in: words)
guard unknown.isEmpty else { fail("no words for \(unknown.sorted())") }

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

// Where a dropped cat sends the island.
let screen = ScreenShape(frame: CGRect(x: 0, y: 0, width: 1512, height: 982), visibleFrame: CGRect(x: 0, y: 70, width: 1512, height: 880), safeTop: 32, auxLeft: 664, auxRight: 664, statusBar: 24)
guard IslandGeometry.placement(dropAt: CGPoint(x: 40, y: 900), screen: screen) == .topLeft,
      IslandGeometry.placement(dropAt: CGPoint(x: 1480, y: 100), screen: screen) == .bottomRight,
      IslandGeometry.placement(dropAt: CGPoint(x: 756, y: 960), screen: screen) == .auto,
      IslandGeometry.placement(dropAt: CGPoint(x: 756, y: 500), screen: screen) == nil else { fail("a dropped cat lands in the wrong place") }
let corner = IslandGeometry.panelFrame(screen, placement: .bottomRight)
let tile = IslandGeometry.screenRect(panel: corner, size: IslandGeometry.tile, placement: .bottomRight)
guard tile.maxX == screen.visibleFrame.maxX - IslandGeometry.cornerMargin, tile.minY == screen.visibleFrame.minY + IslandGeometry.cornerMargin else { fail("the corner tile isn't in its corner: \(tile)") }

if let problem = SurfaceShaders.problem() { fail("a shader doesn't compile: \(problem)") }
if let problem = SurfaceSounds.problem() { fail(problem) }
if let problem = MarkupCheck.problem() { fail(problem) }

print("Surface contract \(surfaceContract): \(fixture.scenes.count) scenes, \(kinds.count) card kinds, \(fixture.actions.count) actions, \(words.labels.count) words; shaders compile; sounds play; mark-up draws")
