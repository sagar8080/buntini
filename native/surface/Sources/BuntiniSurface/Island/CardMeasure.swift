// How tall a card sets, measured rather than guessed, so the island opens at the step that
// holds it (CardSteps). Scrolling parts measure as their content up to their cap and shrink
// when the island has less room, so a card held to the largest step keeps its buttons.
import AppKit
import SwiftUI
import SurfaceKit

enum CardMeasure {
    /// The card's height at `width`, laid out as the island lays it out, in the person's
    /// language. Call on the main thread.
    static func height(_ card: Card, width: CGFloat, inPicture: Bool) -> CGFloat {
        MainActor.assumeIsolated {
            let view = CardView(card: card, send: { _ in })
                .frame(width: width, alignment: .topLeading)
                .inLanguage()
                .environment(\.inPicture, inPicture)
                .foregroundStyle(.white)
            return ceil(NSHostingView(rootView: view).fittingSize.height)
        }
    }
}

/// For SurfaceChecks: what a card measures and how big the island opens.
public enum IslandSizing {
    /// The width a card is laid out at in an island opened at the top of `screen`.
    @MainActor
    public static func cardWidth(_ scene: SurfaceScene, screen: ScreenShape) -> CGFloat {
        model(scene, screen: screen).cardWidth
    }

    /// How tall `card` sets at `width`, in the words last learnt.
    @MainActor
    public static func cardHeight(_ card: Card, width: CGFloat) -> CGFloat {
        CardMeasure.height(card, width: width, inPicture: false)
    }

    /// The island's size for `scene`: opened by a click when `expanded`, showing the chip
    /// `chosen`, with the pointer on it when `hover`.
    @MainActor
    public static func size(_ scene: SurfaceScene, screen: ScreenShape, expanded: Bool, chosen: String? = nil, hover: Bool = false) -> CGSize {
        let model = model(scene, screen: screen)
        model.expanded = expanded
        model.chosen = chosen
        model.hover = hover
        return model.size
    }

    /// Opened with the pointer on it, the island moves from the tallest of `cards` to the
    /// shortest: the heights it takes, then its height once the pointer leaves.
    @MainActor
    public static func held(_ scene: SurfaceScene, screen: ScreenShape, tallest: String, shortest: String) -> (tall: CGFloat, short: CGFloat, left: CGFloat) {
        let model = model(scene, screen: screen)
        model.expanded = true
        model.hover = true
        model.chosen = tallest
        let tall = model.size.height
        model.chosen = shortest
        let short = model.size.height
        model.hover = false
        return (tall, short, model.size.height)
    }

    /// Opened by a click: whether the question field and the chip row show, before Ask is
    /// chosen.
    @MainActor
    public static func home(_ scene: SurfaceScene, screen: ScreenShape) -> (field: Bool, chips: Bool, controls: Bool) {
        let model = model(scene, screen: screen)
        model.expanded = true
        return (model.showsField, model.showsChips, model.showsControls)
    }

    @MainActor
    private static func model(_ scene: SurfaceScene, screen: ScreenShape) -> IslandModel {
        Lexicon.learn(scene)
        let model = IslandModel(screen: screen)
        model.scene = scene
        return model
    }
}
