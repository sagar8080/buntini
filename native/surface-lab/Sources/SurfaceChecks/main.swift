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

// Opened heights: a card is measured and rounded up to one of three steps; past the largest
// only a card with a part that scrolls goes, and it scrolls; with the pointer on it the island
// only grows. In English, German (long words) and Arabic (right to left).
guard CardSteps.snap(10) == 86, CardSteps.snap(86) == 86, CardSteps.snap(87) == 142, CardSteps.snap(222) == 222, CardSteps.snap(900) == 222,
      CardSteps.hold(144, held: 200, pointerIn: true) == 200, CardSteps.hold(144, held: 200, pointerIn: false) == 144 else { fail("opened heights don't snap or hold") }
let notched = ScreenShape(frame: CGRect(x: 0, y: 0, width: 1512, height: 982), visibleFrame: CGRect(x: 0, y: 0, width: 1512, height: 944), safeTop: 32, auxLeft: 664, auxRight: 664, statusBar: 24)
let band = IslandGeometry.notch(notched)!.height
let scrolls: Set<CardBody.Kind> = [.approval, .mail, .diff]
let steps = CardSteps.all
// What an opened island may measure: a card that opened it, or whichever of the question
// field, the chips and the chosen card show, 12 apart.
var opened = Set<CGFloat>()
for field: CGFloat in [0, 30] {
    for chipRow: CGFloat in [0, 28] {
        for card in [0] + steps {
            let rows = [field, chipRow, card].filter { $0 > 0 }
            guard !rows.isEmpty else { continue }
            opened.insert(band + 10 + rows.reduce(0, +) + CGFloat(rows.count - 1) * 12 + 16)
        }
    }
}
// Opened by a click, the band holds Ask, Focus and Chat; the field shows only once asked for
// or when nothing runs, and the chips once there's more than one thing to switch between.
let home = MainActor.assumeIsolated { IslandSizing.home(busy, screen: notched) }
guard home == (false, true, true) else { fail("opened with several things running, the home shows \(home)") }
if let empty = fixture.scenes.first(where: { $0.card == nil && $0.pills.isEmpty && $0.placement == .auto }) {
    guard MainActor.assumeIsolated({ IslandSizing.home(empty, screen: notched) }) == (true, false, true) else { fail("opened with nothing running, the question field doesn't show") }
}
if let single = fixture.scenes.first(where: { $0.pills.count == 1 && $0.card != nil && !$0.opens && $0.pills[0].cardID == $0.card?.id && $0.placement == .auto }) {
    guard MainActor.assumeIsolated({ IslandSizing.home(single, screen: notched) }).chips == false else { fail("one thing running gets a chip row of one") }
}
let root = URL(fileURLWithPath: path).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let raw = try! JSONSerialization.jsonObject(with: data) as! [String: Any]
// A chip is never wider than the widest, however long its name.
guard MainActor.assumeIsolated({ IslandSizing.chipWidths(busy, screen: notched) }).allSatisfy({ $0 <= Chips.widest }) else { fail("a chip is wider than \(Chips.widest) pt") }
var longRaw = (raw["scenes"] as! [[String: Any]]).first(where: { ($0["pills"] as? [Any])?.count == 4 && $0["shelf"] != nil })!
var longPills = longRaw["pills"] as! [[String: Any]]
longPills[0]["label"] = String(repeating: "A long name for a check run ", count: 4)
longRaw["pills"] = [longPills[0]]
let lone = try! JSONDecoder().decode(SurfaceScene.self, from: JSONSerialization.data(withJSONObject: longRaw))
guard let loneWidth = MainActor.assumeIsolated({ IslandSizing.chipWidths(lone, screen: notched) }).first, loneWidth <= Chips.widest else { fail("a lone chip with a long name grows past \(Chips.widest) pt") }
var heights: [String: [String]] = [:]
for code in ["en", "de", "ar"] {
    guard let catalog = (try? Data(contentsOf: root.appendingPathComponent("locales/\(code).json"))).flatMap({ try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }) else { fail("no catalog for \(code)") }
    var first = (raw["scenes"] as! [[String: Any]])[0]
    first["words"] = ["language": code, "rtl": code == "ar", "labels": Dictionary(uniqueKeysWithValues: words.labels.map { ($0.key, catalog[$0.key] as? String ?? $0.value) })]
    let worded = try! JSONDecoder().decode(SurfaceScene.self, from: JSONSerialization.data(withJSONObject: first))
    let width = MainActor.assumeIsolated { IslandSizing.cardWidth(worded, screen: notched) }
    guard width == IslandGeometry.openWidth - 36 else { fail("cards are laid out \(width) pt wide") }
    for scene in fixture.scenes {
        for card in [scene.card].compactMap({ $0 }) + (scene.shelf ?? []) {
            let height = MainActor.assumeIsolated { IslandSizing.cardHeight(card, width: width) }
            heights[card.body.kind.rawValue, default: []].append("\(code) \(Int(height))→\(Int(CardSteps.snap(height)))")
            if height > CardSteps.largest + 0.5 && !scrolls.contains(card.body.kind) {
                fail("a \(card.body.kind.rawValue) card is \(Int(height)) pt in \(code), past the largest step with nothing that scrolls")
            }
        }
        for expanded in [false, true] where scene.placement == .auto {
            let size = MainActor.assumeIsolated { IslandSizing.size(scene, screen: notched, expanded: expanded) }
            guard size.height > band else { continue }
            guard size.width == IslandGeometry.openWidth, opened.contains(size.height) else { fail("a \(scene.card?.body.kind.rawValue ?? "home") scene opens at \(size) in \(code), not on a step") }
        }
    }
    // Switching chips with the pointer on the island: it holds the taller card's height, then
    // settles once the pointer leaves.
    let cards = Chips.cards(busy).filter { $0.body.kind != .diff }
    let measured = cards.map { card in (card.id, CardSteps.snap(MainActor.assumeIsolated { IslandSizing.cardHeight(card, width: width) })) }
    if let tallest = measured.max(by: { $0.1 < $1.1 }), let shortest = measured.min(by: { $0.1 < $1.1 }), tallest.1 > shortest.1 {
        let held = MainActor.assumeIsolated { IslandSizing.held(busy, screen: notched, tallest: tallest.0, shortest: shortest.0) }
        guard held.short == held.tall, held.left < held.tall else { fail("the island shrank under the pointer, or stayed tall once it left: \(held)") }
    }
}
if CommandLine.arguments.contains("--heights") {
    for (kind, values) in heights.sorted(by: { $0.key < $1.key }) { print(kind, values.joined(separator: ", ")) }
}

print("Surface contract \(surfaceContract): \(fixture.scenes.count) scenes, \(kinds.count) card kinds, \(fixture.actions.count) actions, \(words.labels.count) words; shaders compile; sounds play; mark-up draws; dictation's clipboard checks pass; fields take the keyboard and buttons don't; the wing and the chips say what runs; cards open at three heights in English, German and Arabic")
