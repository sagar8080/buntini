// Checks the Swift side of the surface contract against the fixture Rust also checks
// (src-tauri/src/surface/mod.rs): everything decodes, writes back the same, and every card
// kind is covered. Exits 1 on the first failure.
import AppKit
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

// Dictation's clipboard, on a private pasteboard: a password manager's item is never written
// back, the clipboard otherwise goes back as it was, and words not seen to arrive stay on it.
let board = NSPasteboard(name: NSPasteboard.Name("app.buntini.checks.\(UUID().uuidString)"))
board.clearContents()
let secret = NSPasteboardItem()
secret.setString("hunter2", forType: .string)
secret.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
board.writeObjects([secret])
let concealed = Paster.save(board)
Paster.end("dictated", saved: concealed, ours: Paster.place("dictated", on: board, transient: true), arrived: true, pasteboard: board)
guard board.string(forType: .string) == nil, (board.pasteboardItems ?? []).isEmpty else { fail("a concealed clipboard item was written back") }
board.clearContents()
board.setString("mine", forType: .string)
let plain = Paster.save(board)
Paster.end("dictated", saved: plain, ours: Paster.place("dictated", on: board, transient: true), arrived: true, pasteboard: board)
guard board.string(forType: .string) == "mine" else { fail("the clipboard wasn't put back after the words arrived") }
Paster.end("dictated", saved: Paster.save(board), ours: Paster.place("dictated", on: board, transient: true), arrived: false, pasteboard: board)
guard board.string(forType: .string) == "dictated", board.pasteboardItems?.first?.types.contains(NSPasteboard.PasteboardType("org.nspasteboard.TransientType")) == false else { fail("words not seen to arrive aren't left on the clipboard") }
guard !Paster.arrived(before: nil, after: "n4"), !Paster.arrived(before: "n4", after: nil), !Paster.arrived(before: "n4", after: "n4"), Paster.arrived(before: "n4", after: "n9") else { fail("a field that says nothing counts as the words arriving") }
board.releaseGlobally()

if let problem = SurfaceShaders.problem() { fail("a shader doesn't compile: \(problem)") }
if let problem = SurfaceSounds.problem() { fail(problem) }
if let problem = MarkupCheck.problem() { fail(problem) }
if let problem = MainActor.assumeIsolated({ IslandKeyCheck.problem() }) { fail(problem) }

// Placeholders fill in one pass: a value is never filled again.
guard Lexicon.fill("{a} and {b}", ["a": "{b}", "b": "x"]) == "{b} and x",
      Lexicon.fill("{missing} {a", ["a": "y"]) == "{missing} {a",
      Lexicon.fill("{a}{a}", ["a": "é"]) == "éé" else { fail("placeholders are filled more than once") }

// A click in the moment after a card comes forward doesn't count.
let shown = Date()
guard !Arming.ready(since: shown, now: shown.addingTimeInterval(0.5)), Arming.ready(since: shown, now: shown.addingTimeInterval(0.71)) else { fail("cards arm at the wrong time") }

// An approval from a Buntini that doesn't say Allow fits offers none.
let older = #"{"agent":"Codex","project":null,"title":"Run a command","detail":"ls","code":true,"always":true,"plan":false}"#
guard let approval = try? JSONDecoder().decode(Approval.self, from: Data(older.utf8)), !approval.allow, approval.changes.isEmpty, approval.rules.isEmpty else { fail("an approval without allow offers Allow") }

// Beside the notch: the most pressing pill in a few characters, a dot for each of the next
// three, then "+N"; music shows its bars, not its title.
guard let busy = fixture.scenes.first(where: { $0.pills.count == 4 && $0.shelf != nil }) else { fail("no scene has several things running") }
guard let glance = Glance.of(busy.pills, overflow: 2), glance.lead.id == busy.pills[0].id, glance.text == "CI", glance.others.count == 3, glance.more == 2,
      glance.caption == "GitHub · CI", glance.wings > IslandGeometry.peekWings, glance.wings <= IslandGeometry.widestWing else { fail("the wing doesn't say what leads") }
let music = busy.pills.first(where: { $0.source == .media })!
guard Glance.of([music])?.text == nil, Glance.of([music])?.wings == IslandGeometry.peekWings, Glance.of([]) == nil else { fail("music's title went in the wing") }
let long = try! JSONDecoder().decode(Pill.self, from: Data(##"{"id":"p","source":"services","label":"Vercel","tint":"#000000","state":"working","badge":"a-long-project-name","progress":null}"##.utf8))
guard Glance.of([long])?.text == "a-long-p…", long.cardID == "p" else { fail("a long badge isn't cut") }
guard IslandGeometry.size(.peek, screen: screen, openHeight: 0, wings: 400).width == IslandGeometry.notch(screen)!.width + 2 * IslandGeometry.widestWing else { fail("a wing grew past the widest") }

// Opened: a chip for every pill, with its card, then the cards no pill stands for, but not
// a diff; the newest news shows until another chip is chosen.
let chips = Chips.of(busy)
guard chips.prefix(4).map(\.id) == busy.pills.map(\.id), chips.count <= Chips.most else { fail("the chips aren't the pills, in order") }
guard let codex = chips.first(where: { $0.pill?.source == .agents }), codex.card?.id == codex.pill?.card,
      chips.first(where: { $0.pill?.source == .media })?.card?.id == music.id else { fail("a chip doesn't open its own card") }
guard chips.contains(where: { $0.pill == nil && $0.symbol == "speedometer" }) else { fail("a plan window has no chip") }
guard Chips.selected(busy, chosen: nil)?.id == busy.card?.id, Chips.selected(busy, chosen: music.id)?.id == music.id,
      Chips.selected(busy, chosen: "gone")?.id == busy.card?.id else { fail("the wrong card shows when opened") }
if let asking = fixture.scenes.first(where: { $0.opens && $0.card?.body.kind == .approval }) {
    guard Chips.cards(asking).allSatisfy({ $0.id != asking.card?.id }) else { fail("a card that asks is offered as a chip as well") }
}
// The row fits: each as wide as it wants while they fit, the widest cut to share the rest.
guard Chips.share([50, 60], room: 200) == [50, 60], Chips.share([50, 300, 400], room: 350) == [50, 150, 150] else { fail("chips don't share the row") }

print("Surface contract \(surfaceContract): \(fixture.scenes.count) scenes, \(kinds.count) card kinds, \(fixture.actions.count) actions, \(words.labels.count) words; shaders compile; sounds play; mark-up draws; dictation's clipboard checks pass; fields take the keyboard and buttons don't; the wing and the chips say what runs")
