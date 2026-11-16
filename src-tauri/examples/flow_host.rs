//! Dictation on this Mac without building Buntini or speaking, debug only:
//! `cargo run --example flow_host -- <buntini-audio> <models folder> <recorded speech> [rounds] [settings json]`.
//! Holding right ⌥ dictates the recording into whatever has the caret, through the real key
//! watcher, helper, text steps and paste, with the pill on screen. Prints a JSON line per
//! dictation and exits after `rounds` (default 1).
#[cfg(debug_assertions)]
fn main() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let main = objc2::MainThreadMarker::new().expect("Run on the main thread");
    let app = NSApplication::sharedApplication(main);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: flow_host <buntini-audio> <models folder> <recorded speech> [rounds] [settings json]");
        std::process::exit(64);
    }
    let rounds = args.get(4).and_then(|value| value.parse().ok()).unwrap_or(1);
    let settings = args.get(5).map(|raw| serde_json::from_str(raw).expect("settings JSON")).unwrap_or_else(|| serde_json::json!({}));
    if !buntini_lib::surface::start() {
        eprintln!("The native surfaces did not start");
    }
    buntini_lib::run_flow_smoke(args[1].clone().into(), args[2].clone().into(), args[3].clone().into(), settings, rounds);
    app.run();
}

#[cfg(not(debug_assertions))]
fn main() {}
