// The surfaces' words in the person's language. Rust reads them from locales/<language>.json
// and sends them with the scene (`words`); every key below is in locales/en.json, which the
// Rust tests check, and in the fixture, which SurfaceChecks checks.
import SwiftUI
import SurfaceKit

/// Every word the surfaces draw, by its key in the catalogs.
public enum Word: String, CaseIterable, Sendable {
    // Shared
    case allow = "surface.allow"
    case deny = "surface.deny"
    case cancel = "surface.cancel"
    case close = "surface.close"
    case done = "surface.done"
    case next = "surface.next"
    case send = "surface.send"
    case open = "surface.open"
    case copy = "surface.copy"
    case undo = "surface.undo"
    case clear = "surface.clear"
    case back = "surface.back"
    case dismiss = "surface.dismiss"
    case later = "surface.later"
    case notNow = "surface.notNow"
    case tryAgain = "surface.tryAgain"
    case pause = "surface.pause"
    case resume = "surface.resume"
    case skip = "surface.skip"
    case stop = "surface.stop"
    case share = "surface.share"
    // The island
    case chat = "surface.island.chat"
    case askBuntini = "surface.island.askBuntini"
    case dropFiles = "surface.island.dropFiles"
    case hi = "surface.island.hi"
    case hiName = "surface.island.hiName"
    case focus = "surface.island.focus"
    case cat = "surface.character.cat"
    case orb = "surface.character.orb"
    case coat = "surface.wardrobe.coat"
    case eyes = "surface.wardrobe.eyes"
    case accessory = "surface.wardrobe.accessory"
    case coatGinger = "surface.coat.ginger"
    case coatGray = "surface.coat.gray"
    case coatBlack = "surface.coat.black"
    case coatWhite = "surface.coat.white"
    case coatTuxedo = "surface.coat.tuxedo"
    case coatCalico = "surface.coat.calico"
    case coatSiamese = "surface.coat.siamese"
    case eyesAmber = "surface.eyes.amber"
    case eyesGreen = "surface.eyes.green"
    case eyesBlue = "surface.eyes.blue"
    case eyesCopper = "surface.eyes.copper"
    case eyesOdd = "surface.eyes.odd"
    case accessoryNone = "surface.accessory.none"
    case accessoryBell = "surface.accessory.bell"
    case accessoryBow = "surface.accessory.bow"
    case accessoryBandana = "surface.accessory.bandana"
    case accessoryHat = "surface.accessory.hat"
    case accessoryFlowers = "surface.accessory.flowers"
    // Coding agents
    case inTerminal = "surface.agents.inTerminal"
    case keepPlanning = "surface.agents.keepPlanning"
    case acceptEdits = "surface.agents.acceptEdits"
    case always = "surface.agents.always"
    case start = "surface.agents.start"
    case recapTitle = "surface.agents.recapTitle"
    // Notes
    case notes = "surface.note.notes"
    // Timer, captures, the Mac, updates
    case timerAgain = "surface.timer.again"
    case timerDone = "surface.timer.done"
    case timerLeft = "surface.timer.left"
    case timerRound = "surface.timer.round"
    case screenRecording = "surface.capture.screenRecording"
    case paste = "surface.capture.paste"
    case markUp = "surface.capture.markUp"
    case ask = "surface.capture.ask"
    case capture = "surface.capture.capture"
    case soundsOff = "surface.vitals.soundsOff"
    case soundsOn = "surface.vitals.soundsOn"
    case nothingWaiting = "surface.vitals.nothingWaiting"
    case updateAsk = "surface.update.ask"
    case updateCurrent = "surface.update.current"
    case updateDownloading = "surface.update.downloading"
    case updateChecking = "surface.update.checking"
    case updateReady = "surface.update.ready"
    case updateFailed = "surface.update.failed"
    case updateAvailable = "surface.update.available"
    case checkDaily = "surface.update.checkDaily"
    case install = "surface.update.install"
    case releaseNotes = "surface.update.notes"
    case restart = "surface.update.restart"
    case download = "surface.update.download"
    case whatsNew = "surface.update.whatsNew"
    // Services
    case activity = "surface.services.activity"
    // Mail
    case mail = "surface.mail.mail"
    case mailClosed = "surface.mail.closed"
    case openMail = "surface.mail.openMail"
    case mailAll = "surface.mail.all"
    case mailUnread = "surface.mail.unread"
    case mailImportant = "surface.mail.important"
    case mailToday = "surface.mail.today"
    case mailSort = "surface.mail.sort"
    case mailSearch = "surface.mail.search"
    case noMail = "surface.mail.noMail"
    case noMatch = "surface.mail.noMatch"
    case more = "surface.mail.more"
    case markRead = "surface.mail.markRead"
    case markUnread = "surface.mail.markUnread"
    case flag = "surface.mail.flag"
    case unflag = "surface.mail.unflag"
    case junk = "surface.mail.junk"
    case isUnread = "surface.mail.isUnread"
    case isFlagged = "surface.mail.isFlagged"
    case reply = "surface.mail.reply"
    case saveDraft = "surface.mail.saveDraft"
    case mailPlan = "surface.mail.plan"
    case plan = "surface.mail.planTitle"
    case discard = "surface.mail.discard"
    case run = "surface.mail.run"
    // Media
    case nowPlaying = "surface.media.nowPlaying"
    case shuffle = "surface.media.shuffle"
    case previous = "surface.media.previous"
    case nextTrack = "surface.media.next"
    case repeatMode = "surface.media.repeat"
    case play = "surface.media.play"
    case pauseMedia = "surface.media.pause"
    case position = "surface.media.position"
    case volume = "surface.media.volume"
    case output = "surface.media.output"
    // The dictation pill
    case listening = "surface.pill.listening"
    case handsFree = "surface.pill.handsFree"
    case finish = "surface.pill.finish"
    case working = "surface.pill.working"
    case copied = "surface.pill.copied"
    case dictate = "surface.pill.dictate"
    case recordCall = "surface.pill.recordCall"
    case stopRecording = "surface.pill.stopRecording"
    case openBuntini = "surface.pill.openBuntini"
    case automatic = "surface.pill.automatic"
    case asSaid = "surface.pill.asSaid"
    case natural = "surface.pill.natural"
    case polished = "surface.pill.polished"
    case modeDictate = "surface.pill.modeDictate"
    case modePrompt = "surface.pill.modePrompt"
    case modeTranslate = "surface.pill.modeTranslate"
    case library = "surface.pill.library"
    case recording = "surface.pill.recording"
    // Mark-up
    case pen = "surface.markup.pen"
    case arrow = "surface.markup.arrow"
    case box = "surface.markup.box"
    case red = "surface.markup.red"
    case yellow = "surface.markup.yellow"
    case blue = "surface.markup.blue"
}

/// The words last sent with a scene. Read and written on the main thread.
public enum Lexicon {
    private static var words: Words?

    /// Keeps the words a scene brings; a scene without them keeps the last.
    public static func learn(_ scene: SurfaceScene) {
        if let words = scene.words { self.words = words }
    }

    /// The words' language in the Mac's region, for dates and numbers.
    public static var locale: Locale {
        guard let language = words?.language else { return .current }
        return Locale(identifier: Locale.current.region.map { "\(language)_\($0.identifier)" } ?? language)
    }

    public static var direction: LayoutDirection { words?.rtl == true ? .rightToLeft : .leftToRight }

    /// The text for `label`, its `{name}` placeholders filled. Before any words arrive (a
    /// fixture without them), the key shows, so a gap is plain to see.
    public static func text(_ label: Word, _ values: [String: String] = [:]) -> String {
        fill(words?.labels[label.rawValue] ?? label.rawValue, values)
    }

    /// `template` with each `{name}` it has replaced by its value, in one pass, as Rust and
    /// TypeScript do: a value that itself says `{other}` stays as it is. Unknown names stay.
    public static func fill(_ template: String, _ values: [String: String]) -> String {
        guard !values.isEmpty else { return template }
        var out = ""
        var rest = template[...]
        while let open = rest.firstIndex(of: "{") {
            out += rest[..<open]
            let after = rest.index(after: open)
            if let close = rest[after...].firstIndex(of: "}"), let value = values[String(rest[after..<close])] {
                out += value
                rest = rest[rest.index(after: close)...]
            } else {
                out += "{"
                rest = rest[after...]
            }
        }
        return out + rest
    }

    /// The keys a set of words lacks, for the checks.
    public static func missing(in words: Words) -> [String] {
        Word.allCases.map(\.rawValue).filter { words.labels[$0] == nil }
    }
}

/// `L(.chat)`: the text for a label now.
func L(_ label: Word, _ values: [String: String] = [:]) -> String {
    Lexicon.text(label, values)
}

extension View {
    /// Lays the view out in the words' direction, with their locale for dates and numbers.
    func inLanguage() -> some View {
        environment(\.layoutDirection, Lexicon.direction).environment(\.locale, Lexicon.locale)
    }
}
