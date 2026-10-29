fn main() {
    // The explicit controller allowlist in permissions/companion.json activates
    // Tauri's app-command ACL. Only the main capability grants these commands.
    tauri_build::build()
}
