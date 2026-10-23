#[cfg(debug_assertions)]
fn main() {
    buntini_lib::run_public_desktop_browser_smoke();
}

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("The public desktop-browser smoke is available in debug builds only.");
}
