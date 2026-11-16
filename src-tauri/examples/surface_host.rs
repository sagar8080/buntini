//! Opens the native island outside the app and prints what the person does, debug only:
//! `cargo run --example surface_host [card] [seconds]`, where card is `approval` (default),
//! `question`, `ticker`, `service`, `mail`, `nowPlaying` or `none`. Exits 0 on the first
//! action, 2 when the time runs out.
#[cfg(debug_assertions)]
fn main() {
    use buntini_lib::surface::{self, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let main = objc2::MainThreadMarker::new().expect("Run on the main thread");
    let app = NSApplication::sharedApplication(main);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let args: Vec<String> = std::env::args().collect();
    let kind = args.get(1).map(String::as_str).unwrap_or("approval");
    let seconds: u64 = args.get(2).and_then(|value| value.parse().ok()).unwrap_or(20);
    for source in [Source::Agents, Source::Services, Source::Mail, Source::Media, Source::Island] {
        surface::on_action(source, |action| {
            println!("action {}", serde_json::to_string(&action).unwrap());
            std::process::exit(0);
        });
    }
    if !surface::start() {
        eprintln!("The native surfaces did not start");
        std::process::exit(1);
    }
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../native/surface/Fixtures/contract.json")).unwrap();
    let card = fixture["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|scene| serde_json::from_value::<Card>(scene["card"].clone()).ok())
        .find(|card| serde_json::to_value(&card.body).unwrap()["kind"] == kind);
    let waiting = matches!(card.as_ref().map(|card| &card.body), Some(CardBody::Approval(_) | CardBody::Question(_)));
    let pill = Pill {
        id: "claude".into(),
        source: Source::Agents,
        label: "Claude Code".into(),
        tint: "#D97757".into(),
        state: if waiting { PillState::Waiting } else { PillState::Working },
        badge: Some("buntini".into()),
        progress: None,
    };
    let offers = card
        .map(|card| Offer { card, urgency: if waiting { Urgency::Waiting } else { Urgency::Info }, at_ms: surface::now_ms(), ttl_ms: None })
        .into_iter()
        .collect();
    surface::publish(Source::Agents, Contribution { pills: vec![pill], offers });
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        println!("timeout");
        std::process::exit(2);
    });
    app.run();
}

#[cfg(not(debug_assertions))]
fn main() {}
