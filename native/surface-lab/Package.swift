// swift-tools-version:5.10
// Tools for working on native/surface without building Buntini:
//   swift run SurfaceLab ../surface/Fixtures/contract.json [index] [--seconds N]
//   swift run SurfaceChecks ../surface/Fixtures/contract.json
// Kept out of native/surface so the app's build only compiles the library.
import PackageDescription

let package = Package(
    name: "SurfaceLab",
    platforms: [.macOS(.v13)],
    dependencies: [.package(path: "../surface")],
    targets: [
        .executableTarget(name: "SurfaceLab", dependencies: [.product(name: "BuntiniSurface", package: "surface")]),
        .executableTarget(name: "SurfaceChecks", dependencies: [.product(name: "SurfaceKit", package: "surface"), .product(name: "BuntiniSurface", package: "surface")]),
    ],
    swiftLanguageVersions: [.v5]
)
