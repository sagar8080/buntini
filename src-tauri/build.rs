fn main() {
    // The island, the dictation pill and overlays (native/surface) are linked into the app
    // itself, so they share its Accessibility grant and add no executable to the bundle.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        swift_rs::SwiftLinker::new("13.0")
            .with_package("BuntiniSurface", "../native/surface")
            .link();
    }
    // The explicit controller allowlist in permissions/companion.json activates
    // Tauri's app-command ACL. Only the main capability grants these commands.
    tauri_build::build()
}
