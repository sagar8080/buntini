fn main() {
    #[cfg(debug_assertions)]
    buntini_lib::run_public_browser_smoke();
    #[cfg(not(debug_assertions))]
    panic!("The public browser smoke host is available only in debug builds.");
}
