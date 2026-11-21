// Keys and swipes on the island opened by a click. The island takes the keyboard only then, and
// only while the pointer is on it (IslandWindow), so keystrokes meant for the app you're in
// still go there. Pure, so SurfaceChecks checks the rules without a screen.
import CoreGraphics

/// What a key does on the opened island.
public enum IslandKey: Equatable {
    /// ← and →: the chip before or after, in the order they read.
    case previous, next
    /// Return: the card's one button, unless it's a decision.
    case primary
    /// Esc.
    case close
    /// Anything else typed starts Ask with it.
    case type(String)

    public static func of(keyCode: UInt16, characters: String?, command: Bool, control: Bool, rightToLeft: Bool) -> IslandKey? {
        switch keyCode {
        case 123: return rightToLeft ? .next : .previous
        case 124: return rightToLeft ? .previous : .next
        case 36, 76: return .primary
        case 53: return .close
        default: break
        }
        // Shortcuts, and keys that type nothing (arrows, function keys), aren't the island's.
        guard !command, !control, let text = characters, !text.isEmpty,
              text.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x7F && !(0xF700...0xF8FF).contains($0.value) }) else { return nil }
        return .type(text)
    }
}

/// A two-finger swipe sideways: one step for each swipe, once the fingers have moved far
/// enough, and none for a mostly vertical one, which scrolls the card.
public struct IslandSwipe {
    public static let distance: CGFloat = 36
    private var moved: CGFloat = 0
    private var stepped = false

    public init() {}

    /// `dx` and `dy` as the fingers moved, not as the content scrolls. `began` starts a
    /// swipe, `ended` finishes it. Returns 1 for the next item (the fingers moving left, as
    /// Safari goes forward) or -1 for the one before.
    public mutating func add(dx: CGFloat, dy: CGFloat, began: Bool, ended: Bool) -> Int? {
        if began {
            moved = 0
            stepped = false
        }
        defer {
            if ended {
                moved = 0
                stepped = false
            }
        }
        guard !stepped, abs(dx) > abs(dy) else { return nil }
        moved += dx
        guard abs(moved) >= Self.distance else { return nil }
        stepped = true
        return moved < 0 ? 1 : -1
    }
}
