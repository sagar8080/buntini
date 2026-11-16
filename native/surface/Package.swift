// swift-tools-version:5.10
// Buntini's native surfaces: the island, the dictation pill and the overlays. Linked into the
// app's own process (src-tauri/build.rs) so it shares Buntini's Accessibility grant.
import PackageDescription

let package = Package(
    name: "BuntiniSurface",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "BuntiniSurface", type: .static, targets: ["BuntiniSurface"]),
        .library(name: "SurfaceKit", targets: ["SurfaceKit"]),
    ],
    targets: [
        // What Rust sends and receives, with no AppKit, so it can be checked anywhere.
        .target(name: "SurfaceKit"),
        // Windows, drawing and the C entry points Rust calls.
        .target(name: "BuntiniSurface", dependencies: ["SurfaceKit"]),
    ],
    swiftLanguageVersions: [.v5]
)
