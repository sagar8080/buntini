// The cat's head, drawn natively from the same shapes and colours as src/Cat.tsx, so the cat
// in the island is the cat on the desktop. Coordinates are the SVG's 180 × 180 space.
import SwiftUI
import SurfaceKit

/// How the face looks this frame.
struct FacePose {
    /// Where the cat looks, −1…1 on each axis.
    var gaze = CGPoint.zero
    /// 0 open … 1 shut.
    var lid: Double = 0
    var eyes: Eyes = .open
    var mouth: Mouth = .rest
    /// Pupils grow when something has the cat's attention.
    var pupil: Double = 1
    /// Head tilt, degrees.
    var tilt: Double = 0
    /// Ear flick, degrees, for the right ear.
    var ear: Double = 0

    enum Eyes { case open, happy, closed, squeeze, wink }
    enum Mouth { case rest, smile, frown, speaking, yawn }
}

/// The region of the 180 × 180 drawing the head and its accessories cover.
let faceBox = CGRect(x: 38, y: 30, width: 104, height: 112)

struct CatFace {
    let look: SurfaceKit.Cat

    func draw(_ context: inout GraphicsContext, pose: FacePose) {
        if look.character == "orb" {
            drawOrb(&context, pose: pose)
            return
        }
        let coat = Coat.named(look.coat)
        let accent = RGB(hex: look.accent)
        let tabby = look.coat == "ginger" || look.coat == "gray"
        let deep = coat.shade.mix(coat.outline, 0.3)
        let tuft = coat.belly.mix(white, 0.5)
        let leftEar = coat.point ?? coat.patchA ?? coat.base
        let rightEar = coat.point ?? coat.patchB ?? coat.base
        let lids: RGB = look.coat == "black" || look.coat == "tuxedo" ? RGB(hex: "#F4E9DA") : look.coat == "siamese" ? RGB(hex: "#2E1F18") : coat.outline
        let mouthLine = look.coat == "siamese" ? RGB(hex: "#3A2A22") : coat.outline
        let edgeLift = coat.base.luminance > 0.7 ? 0.12 : 0.32
        let edge = GraphicsContext.Shading.linearGradient(
            Gradient(stops: [.init(color: coat.outline.mix(coat.base, edgeLift).color, location: 0), .init(color: coat.outline.color, location: 0.55)]),
            startPoint: CGPoint(x: 62, y: 52), endPoint: CGPoint(x: 124, y: 160))
        let line = StrokeStyle(lineWidth: 1.8, lineCap: .round, lineJoin: .round)
        let headFill = GraphicsContext.Shading.radialGradient(
            Gradient(colors: [coat.base.mix(white, 0.26).color, coat.base.color]), center: CGPoint(x: 80, y: 72), startRadius: 0, endRadius: 44)

        var head = context
        // The head turns toward the cursor, as `.cat-head-gaze` does in styles.css.
        head.translateBy(x: pose.gaze.x * 1.6, y: pose.gaze.y * 1.1)
        head.rotate(around: CGPoint(x: 90, y: 96), by: .degrees(pose.gaze.x * 2.5 + pose.tilt))

        // Ears, behind the head.
        drawEar(&head, fill: leftEar, coat: coat, tuft: tuft, edge: edge, line: line, flick: 0)
        var mirrored = head
        mirrored.translateBy(x: 180, y: 0)
        mirrored.scaleBy(x: -1, y: 1)
        drawEar(&mirrored, fill: rightEar, coat: coat, tuft: tuft, edge: edge, line: line, flick: pose.ear)

        let headPath = SVGPath.path("M52 98C49 76 66 58 90 58C114 58 131 76 128 98L134 103L127 105L131 111L123 110C117 122 105 128 90 128C75 128 63 122 57 110L49 111L53 105L46 103Z")
        head.fill(headPath, with: headFill)
        if look.coat == "calico", let a = coat.patchA, let b = coat.patchB {
            head.fill(SVGPath.path("M53 92C51 76 60 64 74 61C78 68 77 80 70 88C65 92 58 94 53 92Z"), with: .color(a.color))
            head.fill(SVGPath.path("M100 60C115 62 127 74 128 88C120 88 110 82 104 74C101 70 100 65 100 60Z"), with: .color(b.color))
        }
        if tabby, let marks = coat.marks {
            head.stroke(SVGPath.path("M83 64L85 72M90 61V71M97 64L95 72M54 96L63 98M53 102L62 102M126 96L117 98M127 102L118 102"), with: .color(marks.color), style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
        }
        head.fill(headPath, with: .radialGradient(
            Gradient(stops: [.init(color: deep.opacity(0), location: 0.6), .init(color: deep.opacity(0.68), location: 1)]),
            center: CGPoint(x: 86, y: 86), startRadius: 0, endRadius: 46))
        head.stroke(headPath, with: edge, style: line)

        // Muzzle, or the tuxedo's blaze, or the Siamese mask.
        switch look.coat {
        case "tuxedo":
            head.fill(SVGPath.path("M86 102C87 92 88 84 90 76C92 84 93 92 94 102C100 99 106 101 106 108C106 118 98 124 90 124C82 124 74 118 74 108C74 101 80 99 86 102Z"), with: .color(coat.belly.color))
        case "siamese":
            let point = coat.point ?? coat.outline
            ellipse(&head, center: CGPoint(x: 90, y: 106), rx: 27, ry: 21, stops: [(point, 0.95, 0), (point, 0.82, 0.4), (point, 0.38, 0.75), (point, 0, 1)], focusY: 0.56)
        default:
            head.fill(SVGPath.path("M77 109C77 102 83 100 90 104C97 100 103 102 103 109C103 116 97 120 90 120C83 120 77 116 77 109Z"), with: .radialGradient(
                Gradient(stops: [.init(color: coat.belly.color, location: 0), .init(color: coat.belly.color, location: 0.74), .init(color: coat.belly.opacity(0), location: 1)]),
                center: CGPoint(x: 90, y: 110), startRadius: 0, endRadius: 15))
        }

        // Blush.
        let blush = RGB(hex: "#FF8DA1")
        for x in [63.0, 117.0] {
            ellipse(&head, center: CGPoint(x: x, y: 108), rx: 7.5, ry: 4.4, stops: [(blush, min(1, coat.blush * 1.6), 0), (blush, 0, 1)])
        }

        // Eyes.
        switch pose.eyes {
        case .open, .wink:
            drawEye(&head, cx: 74, iris: look.eyes == "odd" ? Coat.eye("blue") : Coat.eye(look.eyes), pose: pose, headFill: headFill)
            if pose.eyes == .open {
                drawEye(&head, cx: 106, iris: look.eyes == "odd" ? Coat.eye("amber") : Coat.eye(look.eyes), pose: pose, headFill: headFill)
            } else {
                head.stroke(SVGPath.path("M97 95Q106 100.5 115 95"), with: .color(lids.color), style: StrokeStyle(lineWidth: 2.6, lineCap: .round))
            }
        case .happy:
            head.stroke(SVGPath.path("M66 97Q74 88 82 97M98 97Q106 88 114 97"), with: .color(lids.color), style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
        case .closed:
            head.stroke(SVGPath.path("M65 95Q74 101 83 95M97 95Q106 101 115 95"), with: .color(lids.color), style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
        case .squeeze:
            head.stroke(SVGPath.path("M67.5 89.5L79 94.5L67.5 99.5M112.5 89.5L101 94.5L112.5 99.5"), with: .color(lids.color), style: StrokeStyle(lineWidth: 2.6, lineCap: .round, lineJoin: .round))
        }

        // Nose.
        head.fill(SVGPath.path("M86.5 106.5C86.5 104.8 93.5 104.8 93.5 106.5C93.5 108.3 91.2 110 90 110C88.8 110 86.5 108.3 86.5 106.5Z"), with: .linearGradient(
            Gradient(colors: [coat.nose.mix(white, 0.3).color, coat.nose.color]), startPoint: CGPoint(x: 90, y: 104.8), endPoint: CGPoint(x: 90, y: 110)))
        head.fill(Path(ellipseIn: CGRect(x: 88.6 - 1.3, y: 106.2 - 0.75, width: 2.6, height: 1.5)), with: .color(white.opacity(0.55)))

        // Mouth.
        let mouthStyle = StrokeStyle(lineWidth: 1.6, lineCap: .round, lineJoin: .round)
        let inside = RGB(hex: "#7A3B4A")
        switch pose.mouth {
        case .rest:
            head.stroke(SVGPath.path("M90 110V112.5M85 112.5C86.5 115.5 89 115.5 90 112.5C91 115.5 93.5 115.5 95 112.5"), with: .color(mouthLine.color), style: mouthStyle)
        case .smile:
            let smile = SVGPath.path("M83.5 112C85 119.5 95 119.5 96.5 112C93 113.6 87 113.6 83.5 112Z")
            head.fill(smile, with: .color(inside.color))
            head.stroke(smile, with: .color(mouthLine.color), style: StrokeStyle(lineWidth: 1.4, lineJoin: .round))
        case .frown:
            head.stroke(SVGPath.path("M85 116Q90 112 95 116"), with: .color(mouthLine.color), style: StrokeStyle(lineWidth: 1.7, lineCap: .round))
        case .speaking:
            head.fill(SVGPath.path("M86 113C88 112 92 112 94 113C94 118 92 120 90 120C88 120 86 118 86 113Z"), with: .color(inside.color))
        case .yawn:
            head.fill(Path(ellipseIn: CGRect(x: 90 - 5.2, y: 117 - 6.8, width: 10.4, height: 13.6)), with: .color(inside.color))
            head.fill(Path(ellipseIn: CGRect(x: 90 - 3.2, y: 120.6 - 2.4, width: 6.4, height: 4.8)), with: .color(RGB(hex: "#F08A98").color))
        }

        // Whiskers.
        head.stroke(SVGPath.path("M58 106Q50 103.4 42 103M58 110Q50 110 41 111.4M59 114Q52 115.4 45 118.4M122 106Q130 103.4 138 103M122 110Q130 110 139 111.4M121 114Q128 115.4 135 118.4"), with: .color(coat.whisker.opacity(0.72)), style: StrokeStyle(lineWidth: 1, lineCap: .round))

        drawAccessory(&head, accent: accent, coat: coat)
        if let outfit = look.outfit { drawOutfit(&head, outfit) }
    }

    private func drawEar(_ context: inout GraphicsContext, fill: RGB, coat: Coat, tuft: RGB, edge: GraphicsContext.Shading, line: StrokeStyle, flick: Double) {
        var ear = context
        if flick != 0 { ear.rotate(around: CGPoint(x: 70, y: 70), by: .degrees(flick)) }
        let outer = SVGPath.path("M57 82C53 68 52 54 55 45C56 41 60 40 63 42C71 48 78 55 84 63Z")
        ear.fill(outer, with: .color(fill.color))
        ear.stroke(outer, with: edge, style: line)
        ear.fill(outer, with: .linearGradient(Gradient(colors: [coat.outline.opacity(0.28), coat.outline.opacity(0)]), startPoint: CGPoint(x: 58, y: 44), endPoint: CGPoint(x: 70, y: 72)))
        ear.fill(SVGPath.path("M61 73C59 63 59 55 60 49C66 53 71 58 76 63Z"), with: .linearGradient(
            Gradient(colors: [coat.ear.mix(coat.outline, 0.3).color, coat.ear.color]), startPoint: CGPoint(x: 0, y: 49), endPoint: CGPoint(x: 0, y: 73)))
        ear.stroke(SVGPath.path("M64.5 70Q63 64 63.6 58M68.5 67Q66.6 62 66.4 56.6M72.6 64.6Q70 61 69.4 58"), with: .color(tuft.opacity(0.6)), style: StrokeStyle(lineWidth: 1, lineCap: .round))
    }

    private func drawEye(_ context: inout GraphicsContext, cx: CGFloat, iris: RGB, pose: FacePose, headFill: GraphicsContext.Shading) {
        let eyeRect = CGRect(x: cx - 9, y: 84, width: 18, height: 20)
        let eye = Path(ellipseIn: eyeRect)
        context.fill(eye, with: .radialGradient(
            Gradient(stops: [.init(color: iris.mix(white, 0.55).color, location: 0), .init(color: iris.color, location: 0.45), .init(color: iris.mix(ink, 0.35).color, location: 0.86), .init(color: iris.mix(ink, 0.6).color, location: 1)]),
            center: CGPoint(x: cx, y: 84 + 20 * 0.66), startRadius: 0, endRadius: 11.8))
        context.fill(SVGPath.path("M\(cx - 8.9) 92.6A9 10 0 0 1 \(cx + 8.9) 92.6Q\(cx) 83.4 \(cx - 8.9) 92.6Z"), with: .color(RGB(hex: "#1E1724").opacity(0.24)))
        var inner = context
        inner.clip(to: eye)
        var look = inner
        look.translateBy(x: pose.gaze.x * 2.3, y: pose.gaze.y * 2.1)
        look.translateBy(x: cx, y: 94.5)
        look.scaleBy(x: pose.pupil, y: pose.pupil)
        look.translateBy(x: -cx, y: -94.5)
        look.fill(Path(ellipseIn: CGRect(x: cx - 5.1, y: 94.5 - 7.3, width: 10.2, height: 14.6)), with: .color(RGB(hex: "#1E1724").color))
        look.fill(Path(ellipseIn: CGRect(x: cx - 2.9 - 2.9, y: 89.8 - 3.2, width: 5.8, height: 6.4)), with: .color(.white))
        look.fill(Path(ellipseIn: CGRect(x: cx + 3.3 - 1.4, y: 98.8 - 1.4, width: 2.8, height: 2.8)), with: .color(white.opacity(0.85)))
        if pose.lid > 0.01 {
            // The upper lid slides down over the eye, as `.cat-lid` does.
            var lid = inner
            lid.translateBy(x: 0, y: -24 + 30 * min(1, pose.lid))
            lid.fill(SVGPath.path("M\(cx - 11) 70H\(cx + 11)V92Q\(cx) 99 \(cx - 11) 92Z"), with: headFill)
            lid.stroke(SVGPath.path("M\(cx + 11) 92Q\(cx) 99 \(cx - 11) 92"), with: .color(ink.color), style: StrokeStyle(lineWidth: 2.2, lineCap: .round))
        }
        context.stroke(eye, with: .color(ink.color), style: StrokeStyle(lineWidth: 1.2))
        context.stroke(SVGPath.path("M\(cx - 8.5) 90.6A9 10 0 0 1 \(cx + 8.5) 90.6"), with: .color(ink.color), style: StrokeStyle(lineWidth: 2.2, lineCap: .round))
    }

    private func drawAccessory(_ context: inout GraphicsContext, accent: RGB, coat: Coat) {
        switch look.accessory {
        case "bow":
            var bow = context
            bow.translateBy(x: 117, y: 58)
            bow.rotate(by: .degrees(18))
            bow.scaleBy(x: 1.25, y: 1.25)
            bow.translateBy(x: -117, y: -58)
            let outline = StrokeStyle(lineWidth: 1.1, lineJoin: .round)
            for wing in ["M117 58C111 51 105 53 106 59C107 65 113 63 117 58Z", "M117 58C123 51 129 53 128 59C127 65 121 63 117 58Z"] {
                bow.fill(SVGPath.path(wing), with: .color(accent.color))
                bow.stroke(SVGPath.path(wing), with: .color(.black.opacity(0.2)), style: outline)
            }
            bow.stroke(SVGPath.path("M113.5 56.4C111 54.6 108.6 55 108.2 57.4M120.5 56.4C123 54.6 125.4 55 125.8 57.4"), with: .color(.white.opacity(0.4)), style: StrokeStyle(lineWidth: 1, lineCap: .round))
            bow.stroke(SVGPath.path("M115 60L112 68M119 60L122 68"), with: .color(accent.color), style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
            bow.fill(Path(ellipseIn: CGRect(x: 117 - 3.2, y: 58 - 3.2, width: 6.4, height: 6.4)), with: .color(accent.mix("#1D1720", 0.12).color))
        case "hat":
            var hat = context
            hat.translateBy(x: 90, y: 61)
            hat.rotate(by: .degrees(-10))
            hat.scaleBy(x: 1.35, y: 1.35)
            hat.translateBy(x: -90, y: -61)
            let felt = RGB(hex: "#3A3245").color
            hat.fill(SVGPath.path("M80 60V44C80 42.5 81.5 41 83 41H97C98.5 41 100 42.5 100 44V60Z"), with: .color(felt))
            hat.fill(Path(CGRect(x: 80, y: 53, width: 20, height: 4)), with: .color(accent.color))
            hat.fill(Path(ellipseIn: CGRect(x: 74, y: 56.9, width: 32, height: 7.2)), with: .color(felt))
            hat.stroke(SVGPath.path("M83.5 45V51"), with: .color(.white.opacity(0.25)), style: StrokeStyle(lineWidth: 1.6, lineCap: .round))
        case "flowers":
            context.stroke(SVGPath.path("M71 65L73 63M83 59L85 59M96 59L98 59M108 63L110 65"), with: .color(RGB(hex: "#7FB069").color), style: StrokeStyle(lineWidth: 3, lineCap: .round))
            let petals = [accent, RGB(hex: "#FF9FB5"), white]
            for (index, center) in [(66.0, 69.0), (77.0, 62.0), (90.0, 59.0), (103.0, 62.0), (114.0, 69.0)].enumerated() {
                for angle in stride(from: 0.0, to: 360.0, by: 72.0) {
                    let x = center.0 + 2.9 * cos(angle * .pi / 180), y = center.1 + 2.9 * sin(angle * .pi / 180)
                    context.fill(Path(ellipseIn: CGRect(x: x - 2.6, y: y - 2.6, width: 5.2, height: 5.2)), with: .color(petals[index % 3].color))
                }
                context.fill(Path(ellipseIn: CGRect(x: center.0 - 1.8, y: center.1 - 1.8, width: 3.6, height: 3.6)), with: .color(RGB(hex: "#FFD45C").color))
            }
        case "bell":
            let band = SVGPath.path("M67 121Q90 133 113 121")
            context.stroke(band, with: .color(coat.outline.opacity(0.7)), style: StrokeStyle(lineWidth: 7, lineCap: .round))
            context.stroke(band, with: .color(accent.color), style: StrokeStyle(lineWidth: 4.6, lineCap: .round))
            context.stroke(SVGPath.path("M70 120.6Q90 130.6 110 120.6"), with: .color(.white.opacity(0.3)), style: StrokeStyle(lineWidth: 1.2, lineCap: .round))
            let bell = Path(ellipseIn: CGRect(x: 85, y: 125.5, width: 10, height: 10))
            context.fill(bell, with: .radialGradient(Gradient(stops: [.init(color: RGB(hex: "#FFF1B0").color, location: 0), .init(color: RGB(hex: "#F6C945").color, location: 0.45), .init(color: RGB(hex: "#C9921F").color, location: 1)]), center: CGPoint(x: 88.6, y: 128.5), startRadius: 0, endRadius: 7.5))
            context.stroke(bell, with: .color(RGB(hex: "#B5891F").color), style: StrokeStyle(lineWidth: 1.2))
            context.stroke(SVGPath.path("M87.5 132H92.5"), with: .color(RGB(hex: "#8A6614").color), style: StrokeStyle(lineWidth: 1, lineCap: .round))
        case "bandana":
            let cloth = SVGPath.path("M66 120Q90 132 114 120L108 128Q99 146 90 149Q81 146 72 128Z")
            context.fill(cloth, with: .color(accent.color))
            context.stroke(cloth, with: .color(.black.opacity(0.18)), style: StrokeStyle(lineWidth: 1.2, lineJoin: .round))
            for dot in [(84.0, 133.0), (96.0, 133.0), (90.0, 140.0)] {
                context.fill(Path(ellipseIn: CGRect(x: dot.0 - 1.4, y: dot.1 - 1.4, width: 2.8, height: 2.8)), with: .color(.white.opacity(0.8)))
            }
        default:
            break
        }
    }

    /// The orb character, from src/Pet.tsx.
    private func drawOrb(_ context: inout GraphicsContext, pose: FacePose) {
        let ring = Path(ellipseIn: CGRect(x: 90 - 67, y: 93 - 21, width: 134, height: 42))
            .applying(CGAffineTransform(translationX: 90, y: 93).rotated(by: -28 * .pi / 180).translatedBy(x: -90, y: -93))
        context.stroke(ring, with: .color(RGB(hex: "#B98A5E").color), style: StrokeStyle(lineWidth: 2))
        let ball = Path(ellipseIn: CGRect(x: 37, y: 40, width: 106, height: 106))
        context.fill(ball, with: .radialGradient(Gradient(stops: [.init(color: RGB(hex: "#FFF4D5").color, location: 0), .init(color: RGB(hex: "#E6AF6D").color, location: 0.45), .init(color: RGB(hex: "#8D573B").color, location: 1)]), center: CGPoint(x: 37 + 106 * 0.35, y: 40 + 106 * 0.25), startRadius: 0, endRadius: 106 * 0.8))
        context.stroke(ball, with: .color(RGB(hex: "#F6D2A2").color), style: StrokeStyle(lineWidth: 1.5))
        context.stroke(SVGPath.path("M56 70C61 58 74 51 88 52"), with: .color(RGB(hex: "#FFF8E2").opacity(0.65)), style: StrokeStyle(lineWidth: 5, lineCap: .round))
        var face = context
        face.translateBy(x: pose.gaze.x * 2.3, y: pose.gaze.y * 2.1)
        let eye = RGB(hex: "#563A2F")
        let open = max(0.12, 1 - pose.lid)
        for x in [71.0, 103.0] {
            face.fill(Path(roundedRect: CGRect(x: x, y: 92 - 6 * open, width: 6, height: 12 * open), cornerRadius: 3), with: .color(eye.color))
        }
        let mouth = pose.mouth == .speaking ? Path(ellipseIn: CGRect(x: 86, y: 106, width: 8, height: 10)) : SVGPath.path("M84 109Q90 114 96 109")
        if pose.mouth == .speaking { face.fill(mouth, with: .color(eye.color)) } else { face.stroke(mouth, with: .color(eye.color), style: StrokeStyle(lineWidth: 2.5, lineCap: .round)) }
        var front = context
        front.clip(to: Path(CGRect(x: -110, y: 93, width: 400, height: 200)).applying(CGAffineTransform(translationX: 90, y: 93).rotated(by: -28 * .pi / 180).translatedBy(x: -90, y: -93)))
        front.stroke(ring, with: .color(RGB(hex: "#D4A576").color), style: StrokeStyle(lineWidth: 2))
    }

    /// An ellipse filled with a radial gradient stretched to its shape, as SVG's bounding-box
    /// gradients are. Stops are (colour, opacity, location).
    private func ellipse(_ context: inout GraphicsContext, center: CGPoint, rx: CGFloat, ry: CGFloat, stops: [(RGB, Double, Double)], focusY: CGFloat = 0.5) {
        var layer = context
        layer.translateBy(x: center.x, y: center.y)
        layer.scaleBy(x: 1, y: ry / rx)
        let gradient = Gradient(stops: stops.map { .init(color: $0.0.opacity($0.1), location: $0.2) })
        layer.fill(Path(ellipseIn: CGRect(x: -rx, y: -rx, width: 2 * rx, height: 2 * rx)), with: .radialGradient(gradient, center: CGPoint(x: 0, y: (focusY - 0.5) * 2 * rx), startRadius: 0, endRadius: rx))
    }
}

extension GraphicsContext {
    /// Rotates about a point rather than the origin.
    mutating func rotate(around point: CGPoint, by angle: Angle) {
        translateBy(x: point.x, y: point.y)
        rotate(by: angle)
        translateBy(x: -point.x, y: -point.y)
    }
}
