// swift-tools-version:6.2
// buntini-audio: the microphone, recognition, call notes and the wake word, in one helper.
// FluidAudio is pinned to an exact release with its prebuilt text engine left out (traits: []),
// so the only code that ships is source in Package.resolved.
import PackageDescription

let package = Package(
    name: "BuntiniAudio",
    // FluidAudio declares macOS 14; build.sh compiles everything for macOS 13 so the helper
    // still starts there, and Parakeet is used only behind an availability check.
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.17.7", traits: []),
    ],
    targets: [
        .executableTarget(
            name: "buntini-audio",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")],
            path: "Sources/BuntiniAudio",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.linkedFramework("AVFoundation"), .linkedFramework("CoreAudio"), .linkedFramework("Accelerate")]
        ),
    ]
)
