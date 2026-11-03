//! Apple Maps through the `buntini-fm` helper (MapKit): places around somewhere and how
//! long a trip takes. Keyless and unthrottled; Maps sees only the approved places and
//! category, as when the user searches in the Maps app. Without the helper, places fall
//! back to OpenStreetMap and travel times are unavailable.
use super::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ATTRIBUTION: &str = "Apple Maps";
pub const HOST: &str = "Apple Maps (via macOS)";
pub const MODES: [&str; 3] = ["driving", "walking", "transit"];

static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// Where the helper is, set once at startup.
pub fn set_helper(path: &Path) {
    let _ = HELPER.set(path.to_path_buf());
}

/// The helper when it is on this Mac.
pub fn helper() -> Option<&'static Path> {
    HELPER.get().map(PathBuf::as_path).filter(|path| path.is_file())
}

pub fn validate_route(args: &Value) -> Result<Value, String> {
    let map = object(args, &["from", "to", "mode"])?;
    Ok(json!({
        "from": required(map, "from", 160)?,
        "to": required(map, "to", 160)?,
        "mode": choice(map, "mode", &MODES)?.unwrap_or_else(|| "driving".into()),
    }))
}

fn by(mode: &str) -> &'static str {
    match mode {
        "walking" => "on foot",
        "transit" => "by public transport",
        _ => "by car",
    }
}

pub fn preview_route(args: &Value) -> Preview {
    Preview {
        provider: ATTRIBUTION.into(),
        host: HOST.into(),
        summary: format!(
            "{} to {} {}",
            text_of(args, "from"),
            text_of(args, "to"),
            by(&text_of(args, "mode"))
        ),
        sends: vec![
            CardField::new("From", text_of(args, "from")),
            CardField::new("To", text_of(args, "to")),
            CardField::new("Travelling", by(&text_of(args, "mode"))),
        ],
        uses_key: false,
    }
}

pub fn preview_places(args: &Value, label: &str) -> Preview {
    Preview {
        provider: ATTRIBUTION.into(),
        host: HOST.into(),
        summary: format!("{label} near {}", text_of(args, "near")),
        sends: vec![
            CardField::new("Looking for", label),
            CardField::new("Near", text_of(args, "near")),
            CardField::new("Within", format!("{} m", text_of(args, "radius_m"))),
        ],
        uses_key: false,
    }
}

async fn ask(helper: &Path, args: &[&str]) -> Result<Value, String> {
    let mut command = tokio::process::Command::new(helper);
    command.args(args);
    let output = crate::actions::output(command, 30).await.map_err(|error| {
        error
            .split_once(": ")
            .filter(|(code, _)| ["lookup-failed", "invalid-query"].contains(code))
            .map(|(_, message)| message.trim().to_string())
            .unwrap_or(error)
    })?;
    serde_json::from_str(output.trim()).map_err(|_| "Apple Maps returned an unreadable answer.".into())
}

/// Maps' multi-line addresses on one line.
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn missing(answer: &Value, words: &[(&str, &str)]) -> Option<String> {
    answer["error"].as_str().filter(|error| *error == "not_found")?;
    let place = answer["which"].as_str().unwrap_or("");
    let said = words.iter().find(|(key, _)| *key == place).map(|(_, said)| *said).unwrap_or("that place");
    Some(format!("Apple Maps couldn’t find “{said}”. Try a street, area or city."))
}

fn away(distance: f64) -> String {
    if distance < 1000.0 {
        format!("{:.0} m away", (distance / 10.0).round() * 10.0)
    } else {
        format!("{:.1} km away", distance / 1000.0)
    }
}

/// A link that opens Maps at a place, or on a route.
fn maps_link(query: &[(&str, String)]) -> String {
    reqwest::Url::parse_with_params("https://maps.apple.com/", query)
        .map(String::from)
        .unwrap_or_default()
}

fn spot(place: &Value) -> String {
    format!(
        "{},{}",
        place["latitude"].as_f64().unwrap_or_default(),
        place["longitude"].as_f64().unwrap_or_default()
    )
}

pub(crate) fn place_cards(answer: &Value, near: &str, radius: f64, label: &str) -> Result<Vec<ResultCard>, String> {
    if let Some(message) = missing(answer, &[("near", near)]) {
        return Err(message);
    }
    let cards = answer["places"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|place| place["name"].as_str().is_some_and(|name| !name.trim().is_empty()))
        .take(10)
        .map(|place| {
            let name = place["name"].as_str().unwrap_or_default();
            card(
                "place",
                name,
                format!("{label} · {}", away(place["distance_m"].as_f64().unwrap_or_default())),
                vec![
                    CardField::new("Address", one_line(place["address"].as_str().unwrap_or_default())),
                    CardField::new("Phone", place["phone"].as_str().unwrap_or_default()),
                    CardField::new("Website", place["url"].as_str().unwrap_or_default()),
                ],
                Some(maps_link(&[("q", name.to_string()), ("ll", spot(place))])),
                ATTRIBUTION,
            )
        })
        .collect::<Vec<_>>();
    if cards.is_empty() {
        let center = answer["center"]["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(near);
        return Err(format!(
            "Apple Maps lists no {} within {radius:.0} m of {center}. Try a larger radius.",
            label.to_lowercase()
        ));
    }
    Ok(cards)
}

/// "45 min", "1 h 5 min".
fn duration(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round().max(1.0) as i64;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

pub(crate) fn route_card(answer: &Value, args: &Value) -> Result<ResultCard, String> {
    let (from, to, mode) = (text_of(args, "from"), text_of(args, "to"), text_of(args, "mode"));
    if let Some(message) = missing(answer, &[("from", &from), ("to", &to)]) {
        return Err(message);
    }
    let seconds = answer["seconds"].as_f64().filter(|seconds| *seconds > 0.0);
    let (Some(seconds), None) = (seconds, answer["error"].as_str()) else {
        return Err(format!("Apple Maps has no route {} from {from} to {to}.", by(&mode)));
    };
    let name = |place: &Value, said: &str| place["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(said).to_string();
    let distance = answer["distance_m"].as_f64().unwrap_or_default();
    let flag = match mode.as_str() {
        "walking" => "w",
        "transit" => "r",
        _ => "d",
    };
    Ok(card(
        "route",
        format!("{} {}", duration(seconds), by(&mode)),
        format!("{} → {}", name(&answer["from"], &from), name(&answer["to"], &to)),
        vec![
            CardField::new("Distance", if distance > 0.0 { format!("{:.1} km", distance / 1000.0) } else { String::new() }),
            CardField::new("Via", answer["via"].as_str().unwrap_or_default()),
            CardField::new("To", one_line(answer["to"]["address"].as_str().unwrap_or_default())),
        ],
        Some(maps_link(&[("saddr", spot(&answer["from"])), ("daddr", spot(&answer["to"])), ("dirflg", flag.into())])),
        ATTRIBUTION,
    ))
}

pub async fn places(helper: &Path, args: &Value, label: &str) -> Result<Vec<ResultCard>, String> {
    let (category, near, radius) = (text_of(args, "category"), text_of(args, "near"), text_of(args, "radius_m"));
    let answer = ask(helper, &["--maps-places", &category, &near, &radius]).await?;
    place_cards(&answer, &near, radius.parse().unwrap_or(1000.0), label)
}

pub async fn travel_time(args: &Value) -> Result<Vec<ResultCard>, String> {
    let helper = helper().ok_or("Travel times need Buntini’s Apple helper, which isn’t on this Mac.")?;
    let answer = ask(helper, &["--maps-route", &text_of(args, "from"), &text_of(args, "to"), &text_of(args, "mode")]).await?;
    Ok(vec![route_card(&answer, args)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_bounded_and_default_to_driving() {
        assert_eq!(
            validate_route(&json!({"from":"Indiranagar, Bengaluru","to":"the airport"})).unwrap(),
            json!({"from":"Indiranagar, Bengaluru","to":"the airport","mode":"driving"})
        );
        assert_eq!(validate_route(&json!({"from":"a","to":"b","mode":"Transit"})).unwrap()["mode"], "transit");
        assert!(validate_route(&json!({"from":"a","to":"b","mode":"flying"})).is_err());
        assert!(validate_route(&json!({"from":"a"})).is_err());
        assert!(validate_route(&json!({"from":"a","to":"b","via":"c"})).is_err());
    }

    #[test]
    fn places_read_as_cards_that_open_in_maps() {
        let answer = json!({"center":{"name":"Indiranagar"},"places":[
            {"name":"Apollo Pharmacy","address":"795, 1st Cross Road\nIndiranagar","phone":"+91 80-47476965","url":"https://www.apollopharmacy.in","distance_m":70,"latitude":12.97,"longitude":77.64},
            {"name":" ","distance_m":10,"latitude":0,"longitude":0},
            {"name":"SS Medicals","address":"962, 12th Main Road","phone":"","url":"","distance_m":1240,"latitude":12.96,"longitude":77.63}
        ],"total":3});
        let cards = place_cards(&answer, "Indiranagar", 2000.0, "Pharmacy").unwrap();
        assert_eq!(cards.len(), 2, "nameless places are skipped");
        assert_eq!(cards[0].subtitle, "Pharmacy · 70 m away");
        assert_eq!(cards[0].fields[0].value, "795, 1st Cross Road, Indiranagar");
        assert_eq!(cards[0].url.as_deref(), Some("https://maps.apple.com/?q=Apollo+Pharmacy&ll=12.97%2C77.64"));
        assert_eq!(cards[1].subtitle, "Pharmacy · 1.2 km away");
        assert!(place_cards(&json!({"error":"not_found","which":"near"}), "Nowhere", 500.0, "Café").unwrap_err().contains("couldn’t find “Nowhere”"));
        assert!(place_cards(&json!({"center":{"name":"Baixa"},"places":[]}), "Baixa", 500.0, "Café").unwrap_err().contains("no café within 500 m of Baixa"));
    }

    #[test]
    fn a_route_says_how_long_and_opens_the_route_in_maps() {
        let args = json!({"from":"Indiranagar","to":"the airport","mode":"driving"});
        let answer = json!({"from":{"name":"Indiranagar","latitude":12.97,"longitude":77.64},"to":{"name":"Kempegowda International Airport","address":"Devanahalli\nBengaluru","latitude":13.2,"longitude":77.7},"mode":"driving","seconds":2728,"distance_m":38907,"via":"Bellary Road"});
        let card = route_card(&answer, &args).unwrap();
        assert_eq!(card.title, "45 min by car");
        assert_eq!(card.subtitle, "Indiranagar → Kempegowda International Airport");
        assert_eq!(card.fields.iter().map(|field| field.value.as_str()).collect::<Vec<_>>(), ["38.9 km", "Bellary Road", "Devanahalli, Bengaluru"]);
        assert_eq!(card.url.as_deref(), Some("https://maps.apple.com/?saddr=12.97%2C77.64&daddr=13.2%2C77.7&dirflg=d"));
        assert_eq!(duration(3900.0), "1 h 5 min");
        assert_eq!(duration(7200.0), "2 h");
        let transit = json!({"from":"a","to":"b","mode":"transit"});
        assert!(route_card(&json!({"from":{},"to":{},"error":"no_route"}), &transit).unwrap_err().contains("no route by public transport"));
        assert!(route_card(&json!({"error":"not_found","which":"to"}), &transit).unwrap_err().contains("couldn’t find “b”"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "asks Apple Maps; run with --ignored live_apple_maps"]
    async fn live_apple_maps() {
        set_helper(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../native/buntini-fm"));
        let settings = Settings::default();
        let cancel = Arc::new(AtomicBool::new(false));
        for (tool, args) in [
            ("places", json!({"category":"pharmacy","near":"Indiranagar, Bengaluru","radius_m":800})),
            ("travel_time", json!({"from":"Indiranagar, Bengaluru","to":"Kempegowda International Airport"})),
            ("travel_time", json!({"from":"Union Square, San Francisco","to":"Golden Gate Park","mode":"transit"})),
        ] {
            let call = super::super::validate(tool, &args, "x").unwrap();
            let preview = super::super::preview(&call, &settings);
            assert_eq!(preview.provider, "Apple Maps", "{tool}");
            let outcome = super::super::execute(&call, &settings, cancel.clone()).await.unwrap_or_else(|error| panic!("{tool}: {error}"));
            println!("--- {tool}: {}\n{}", preview.summary, cards_text(&outcome.cards));
        }
        assert!(super::super::available().contains(&"travel_time"));
    }
}
