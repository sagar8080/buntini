//! Nearby places from OpenStreetMap via Nominatim (keyless; at most one request per second).
use super::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ATTRIBUTION: &str = "© OpenStreetMap contributors (ODbL), via Nominatim";
const SEARCH: &str = "https://nominatim.openstreetmap.org/search";

/// (argument id, Nominatim special phrase, label)
pub const CATEGORIES: [(&str, &str, &str); 19] = [
    ("cafe", "cafe", "Café"),
    ("restaurant", "restaurant", "Restaurant"),
    ("bar", "bar", "Bar"),
    ("pharmacy", "pharmacy", "Pharmacy"),
    ("hospital", "hospital", "Hospital"),
    ("atm", "atm", "ATM"),
    ("bank", "bank", "Bank"),
    ("supermarket", "supermarket", "Supermarket"),
    ("bakery", "bakery", "Bakery"),
    ("fuel", "fuel", "Fuel station"),
    ("charging_station", "charging station", "EV charging"),
    ("parking", "parking", "Parking"),
    ("hotel", "hotel", "Hotel"),
    ("museum", "museum", "Museum"),
    ("park", "park", "Park"),
    ("library", "library", "Library"),
    ("post_office", "post office", "Post office"),
    ("gym", "fitness centre", "Gym"),
    ("toilets", "toilets", "Public toilets"),
];

static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

pub fn validate(args: &Value) -> Result<Value, String> {
    let map = object(args, &["category", "near", "radius_m"])?;
    let ids = CATEGORIES.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    Ok(json!({
        "category": choice(map, "category", &ids)?.ok_or("'category' is required.")?,
        "near": required(map, "near", 160)?,
        "radius_m": integer(map, "radius_m", 100, 5000)?.unwrap_or(1000),
    }))
}

fn label(category: &str) -> &'static str {
    CATEGORIES.iter().find(|(id, _, _)| *id == category).map(|(_, _, label)| *label).unwrap_or("Place")
}

pub fn preview(args: &Value) -> Preview {
    let category = text_of(args, "category");
    Preview {
        provider: "OpenStreetMap (Nominatim)".into(),
        host: "nominatim.openstreetmap.org".into(),
        summary: format!("{} near {}", label(&category), text_of(args, "near")),
        sends: vec![CardField::new("Looking for", label(&category)), CardField::new("Near", text_of(args, "near")), CardField::new("Within", format!("{} m", text_of(args, "radius_m")))],
        uses_key: false,
    }
}

/// Waits until a second has passed since this app last called Nominatim.
async fn throttle(cancel: &Arc<AtomicBool>) -> Result<(), String> {
    loop {
        let wait = {
            let mut last = LAST_REQUEST.lock().unwrap();
            let wait = last.map(|at| Duration::from_millis(1100).saturating_sub(at.elapsed())).unwrap_or_default();
            if wait.is_zero() {
                *last = Some(Instant::now());
                return Ok(());
            }
            wait
        };
        tokio::time::sleep(wait.min(Duration::from_millis(100))).await;
        if cancel.load(Ordering::Relaxed) {
            return Err("Task cancelled.".into());
        }
    }
}

pub async fn search(args: &Value, cancel: Arc<AtomicBool>) -> Result<Vec<ResultCard>, String> {
    let category = text_of(args, "category");
    let phrase = CATEGORIES.iter().find(|(id, _, _)| *id == category).map(|(_, phrase, _)| *phrase).ok_or("Unsupported place category.")?;
    let near = text_of(args, "near");
    let radius = args.get("radius_m").and_then(Value::as_f64).unwrap_or(1000.0);
    let client = client()?;
    throttle(&cancel).await?;
    let found = json_response(client.get(SEARCH).query(&[("q", near.as_str()), ("format", "jsonv2"), ("limit", "1")]).send().await, "OpenStreetMap", 262_144).await?;
    let center = found.as_array().and_then(|items| items.first()).ok_or_else(|| format!("OpenStreetMap couldn't find “{near}”. Try a street, district or city."))?;
    let latitude = coordinate(center, "lat").ok_or("OpenStreetMap returned an invalid location.")?;
    let longitude = coordinate(center, "lon").ok_or("OpenStreetMap returned an invalid location.")?;
    let (dlat, dlon) = (radius / 111_320.0, radius / (111_320.0 * latitude.to_radians().cos().max(0.01)));
    let viewbox = format!("{},{},{},{}", longitude - dlon, latitude + dlat, longitude + dlon, latitude - dlat);
    throttle(&cancel).await?;
    let places = json_response(
        client
            .get(SEARCH)
            .query(&[("amenity", phrase), ("format", "jsonv2"), ("limit", "25"), ("viewbox", viewbox.as_str()), ("bounded", "1"), ("extratags", "1"), ("addressdetails", "1")])
            .send()
            .await,
        "OpenStreetMap",
        1_048_576,
    )
    .await?;
    let cards = parse_places(&places, latitude, longitude, radius, label(&category));
    if cards.is_empty() {
        return Err(format!("OpenStreetMap lists no {} within {radius:.0} m of {}. Try a larger radius.", label(&category).to_lowercase(), center.get("display_name").and_then(Value::as_str).unwrap_or(&near)));
    }
    Ok(cards)
}

fn coordinate(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_str()?.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    6_371_000.0 * 2.0 * a.sqrt().asin()
}

pub(crate) fn parse_places(data: &Value, latitude: f64, longitude: f64, radius: f64, label: &str) -> Vec<ResultCard> {
    let mut places = data
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(Value::as_str).filter(|name| !name.trim().is_empty())?;
                    let distance = distance_m(latitude, longitude, coordinate(item, "lat")?, coordinate(item, "lon")?);
                    (distance <= radius).then_some((distance, name, item))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    places.sort_by(|a, b| a.0.total_cmp(&b.0));
    places
        .into_iter()
        .take(10)
        .map(|(distance, name, item)| {
            let address = item.get("address");
            let part = |key: &str| address.and_then(|a| a.get(key)).and_then(Value::as_str).unwrap_or("");
            let street = [part("road"), part("house_number")].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
            let area = [part("suburb"), part("city"), part("town"), part("village")].into_iter().find(|p| !p.is_empty()).unwrap_or("");
            let extra = |keys: &[&str]| keys.iter().find_map(|key| item.pointer(&format!("/extratags/{key}")).and_then(Value::as_str)).unwrap_or("").to_string();
            let away = if distance < 1000.0 { format!("{:.0} m away", (distance / 10.0).round() * 10.0) } else { format!("{:.1} km away", distance / 1000.0) };
            let link = match (item.get("osm_type").and_then(Value::as_str), item.get("osm_id").and_then(Value::as_u64)) {
                (Some(kind @ ("node" | "way" | "relation")), Some(id)) => Some(format!("https://www.openstreetmap.org/{kind}/{id}")),
                _ => None,
            };
            card(
                "place",
                name,
                format!("{label} · {away}"),
                vec![
                    CardField::new("Address", [street, area.to_string()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ")),
                    CardField::new("Hours", extra(&["opening_hours"])),
                    CardField::new("Phone", extra(&["phone", "contact:phone"])),
                    CardField::new("Website", extra(&["website", "contact:website"])),
                ],
                link,
                ATTRIBUTION,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_radius_are_bounded() {
        assert_eq!(validate(&json!({"category":"Charging Station","near":"Baixa, Lisbon"})).unwrap(), json!({"category":"charging_station","near":"Baixa, Lisbon","radius_m":1000}));
        assert!(validate(&json!({"category":"casino","near":"Lisbon"})).is_err());
        assert!(validate(&json!({"category":"cafe","near":"Lisbon","radius_m":"50000"})).is_err());
        assert!(validate(&json!({"category":"cafe"})).is_err());
    }

    #[test]
    fn places_are_named_sorted_by_distance_and_inside_the_radius() {
        let data = json!([
            {"name":"Far Café","lat":"38.7300","lon":"-9.1366","osm_type":"node","osm_id":1},
            {"name":"","lat":"38.7111","lon":"-9.1366"},
            {"name":"The Coffee","lat":"38.7113696","lon":"-9.1365815","osm_type":"node","osm_id":42,"address":{"road":"Rua dos Douradores","house_number":"10","city":"Lisboa"},"extratags":{"opening_hours":"Mo-Su 08:00-18:00","contact:website":"https://coffee.example"}},
            {"name":"Starbucks","lat":"38.7145606","lon":"-9.1408850","osm_type":"way","osm_id":7}
        ]);
        let cards = parse_places(&data, 38.7110, -9.1366, 1000.0, "Café");
        assert_eq!(cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["The Coffee", "Starbucks"]);
        assert_eq!(cards[0].subtitle, "Café · 40 m away");
        assert_eq!(cards[0].fields, vec![CardField::new("Address", "Rua dos Douradores 10, Lisboa"), CardField::new("Hours", "Mo-Su 08:00-18:00"), CardField::new("Website", "https://coffee.example")]);
        assert_eq!(cards[1].url.as_deref(), Some("https://www.openstreetmap.org/way/7"));
        assert!(parse_places(&json!({"error":"x"}), 0.0, 0.0, 1000.0, "Café").is_empty());
    }

    #[tokio::test]
    async fn nominatim_calls_are_spaced_a_second_apart() {
        let cancel = Arc::new(AtomicBool::new(false));
        throttle(&cancel).await.unwrap();
        let start = Instant::now();
        throttle(&cancel).await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(1000));
    }
}
