//! Duffel flight offer search with the user's access token. Search only: Buntini never books.
use super::*;

const API: &str = "https://api.duffel.com";
const CABINS: [&str; 4] = ["economy", "premium_economy", "business", "first"];

pub fn validate(args: &Value, today: chrono::NaiveDate) -> Result<Value, String> {
    let map = object(
        args,
        &[
            "origin",
            "destination",
            "departure_date",
            "return_date",
            "adults",
            "cabin",
        ],
    )?;
    let place = |key: &str| -> Result<String, String> {
        let value = required(map, key, 60)?;
        Ok(
            if value.len() == 3 && value.chars().all(|c| c.is_ascii_alphabetic()) {
                value.to_uppercase()
            } else {
                value
            },
        )
    };
    let date = |key: &str| -> Result<Option<chrono::NaiveDate>, String> {
        text(map, key, 10)?
            .map(|value| {
                let date = chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .map_err(|_| format!("'{key}' must be a date like 2026-11-30."))?;
                if date < today || date > today + chrono::Duration::days(360) {
                    return Err(format!(
                        "'{key}' must be between today and 360 days from now."
                    ));
                }
                Ok(date)
            })
            .transpose()
    };
    let (origin, destination) = (place("origin")?, place("destination")?);
    if origin.eq_ignore_ascii_case(&destination) {
        return Err("Origin and destination must differ.".into());
    }
    let departure = date("departure_date")?.ok_or("'departure_date' is required.")?;
    let back = date("return_date")?;
    if back.is_some_and(|back| back < departure) {
        return Err("The return date must be on or after the departure date.".into());
    }
    Ok(json!({
        "origin": origin,
        "destination": destination,
        "departure_date": departure.to_string(),
        "return_date": back.map(|date| date.to_string()),
        "adults": integer(map, "adults", 1, 9)?.unwrap_or(1),
        "cabin": choice(map, "cabin", &CABINS)?.unwrap_or_else(|| "economy".into()),
    }))
}

pub fn preview(args: &Value) -> Preview {
    let back = text_of(args, "return_date");
    let mut sends = vec![
        CardField::new("From", text_of(args, "origin")),
        CardField::new("To", text_of(args, "destination")),
        CardField::new("Depart", text_of(args, "departure_date")),
    ];
    if !back.is_empty() {
        sends.push(CardField::new("Return", back.clone()));
    }
    sends.push(CardField::new("Adults", text_of(args, "adults")));
    sends.push(CardField::new(
        "Cabin",
        text_of(args, "cabin").replace('_', " "),
    ));
    Preview {
        provider: "Duffel".into(),
        host: "api.duffel.com".into(),
        summary: format!(
            "Search {} flights {} → {}",
            if back.is_empty() { "one-way" } else { "return" },
            text_of(args, "origin"),
            text_of(args, "destination")
        ),
        sends,
        uses_key: true,
    }
}

fn request(
    client: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
    token: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, format!("{API}{path}"))
        .bearer_auth(token)
        .header("Duffel-Version", "v2")
        .header("Accept", "application/json")
}

/// Turns a city or airport name into an IATA code with Duffel's place suggestions.
async fn resolve(client: &reqwest::Client, token: &str, place: &str) -> Result<String, String> {
    if place.len() == 3 && place.chars().all(|c| c.is_ascii_uppercase()) {
        return Ok(place.into());
    }
    let found = json_response(
        request(client, reqwest::Method::GET, "/places/suggestions", token)
            .query(&[("query", place)])
            .send()
            .await,
        "Duffel",
        524_288,
    )
    .await?;
    found
        .get("data")
        .and_then(Value::as_array)
        .and_then(|places| {
            places.iter().find_map(|place| {
                place
                    .get("iata_code")
                    .and_then(Value::as_str)
                    .filter(|code| code.len() == 3)
            })
        })
        .map(str::to_string)
        .ok_or_else(|| {
            format!("Duffel couldn't match “{place}” to an airport. Try its IATA code, like LIS.")
        })
}

pub async fn search(args: &Value, token: &str) -> Result<Vec<ResultCard>, String> {
    let token = token.trim();
    if !token.starts_with("duffel_") {
        return Err("Re-enter your Duffel access token in Settings.".into());
    }
    let client = client()?;
    let origin = resolve(&client, token, &text_of(args, "origin")).await?;
    let destination = resolve(&client, token, &text_of(args, "destination")).await?;
    let mut slices = vec![
        json!({"origin": origin, "destination": destination, "departure_date": text_of(args, "departure_date")}),
    ];
    let back = text_of(args, "return_date");
    if !back.is_empty() {
        slices.push(json!({"origin": destination, "destination": origin, "departure_date": back}));
    }
    let adults = args
        .get("adults")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 9);
    let body = json!({"data": {"slices": slices, "passengers": vec![json!({"type": "adult"}); adults as usize], "cabin_class": text_of(args, "cabin")}});
    let created = json_response(
        request(&client, reqwest::Method::POST, "/air/offer_requests", token)
            .query(&[("return_offers", "false"), ("supplier_timeout", "20000")])
            .json(&body)
            .send()
            .await,
        "Duffel",
        1_048_576,
    )
    .await?;
    let id = created
        .pointer("/data/id")
        .and_then(Value::as_str)
        .ok_or("Duffel did not start the search.")?;
    let offers = json_response(
        request(&client, reqwest::Method::GET, "/air/offers", token)
            .query(&[
                ("offer_request_id", id),
                ("sort", "total_amount"),
                ("limit", "8"),
            ])
            .send()
            .await,
        "Duffel",
        4_194_304,
    )
    .await?;
    let cards = parse_offers(&offers, token.starts_with("duffel_test_"));
    if cards.is_empty() {
        return Err(format!(
            "Duffel found no {} offers for {origin} → {destination} on these dates.",
            text_of(args, "cabin").replace('_', " ")
        ));
    }
    Ok(cards)
}

/// "PT7H35M" / "P1DT2H" → "7h 35m" / "26h"
fn duration(iso: &str) -> String {
    let mut total = 0u64;
    let mut number = String::new();
    let mut in_time = false;
    for c in iso.chars() {
        match c {
            '0'..='9' => number.push(c),
            'T' => in_time = true,
            'D' => {
                total += number
                    .drain(..)
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
                    * 1440
            }
            'H' if in_time => {
                total += number
                    .drain(..)
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
                    * 60
            }
            'M' if in_time => {
                total += number
                    .drain(..)
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            }
            _ => number.clear(),
        }
    }
    match (total / 60, total % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

fn slice_text(slice: &Value) -> Option<String> {
    let segments = slice.get("segments")?.as_array()?;
    let (first, last) = (segments.first()?, segments.last()?);
    let code = |segment: &Value, end: &str| {
        segment
            .pointer(&format!("/{end}/iata_code"))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let time = |segment: &Value, key: &str| {
        segment
            .get(key)
            .and_then(Value::as_str)
            .and_then(|t| t.get(11..16))
            .unwrap_or("")
            .to_string()
    };
    let date = |segment: &Value, key: &str| {
        segment
            .get(key)
            .and_then(Value::as_str)
            .and_then(|t| t.get(..10))
            .unwrap_or("")
            .to_string()
    };
    let next_day = if date(last, "arriving_at") > date(first, "departing_at") {
        " (+1)"
    } else {
        ""
    };
    let stops = match segments.len() {
        1 => "nonstop".to_string(),
        n => format!(
            "{} stop{} via {}",
            n - 1,
            if n == 2 { "" } else { "s" },
            segments[..n - 1]
                .iter()
                .map(|s| code(s, "destination"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    Some(format!(
        "{} {} {} → {} {}{next_day} · {} · {stops}",
        date(first, "departing_at"),
        code(first, "origin"),
        time(first, "departing_at"),
        code(last, "destination"),
        time(last, "arriving_at"),
        slice
            .get("duration")
            .and_then(Value::as_str)
            .map(duration)
            .unwrap_or_default(),
    ))
}

pub(crate) fn parse_offers(data: &Value, test_mode: bool) -> Vec<ResultCard> {
    let attribution = if test_mode {
        "Duffel test mode · sample airline data, not real fares"
    } else {
        "Live airline offer via Duffel · confirmed only at booking"
    };
    let Some(offers) = data.get("data").and_then(Value::as_array) else {
        return vec![];
    };
    offers
        .iter()
        .filter_map(|offer| {
            let amount = offer.get("total_amount")?.as_str()?;
            let currency = offer.get("total_currency")?.as_str()?;
            let airline = offer
                .pointer("/owner/name")
                .and_then(Value::as_str)
                .unwrap_or("Airline");
            let slices = offer.get("slices")?.as_array()?;
            let mut fields = slices
                .iter()
                .enumerate()
                .filter_map(|(index, slice)| {
                    Some(CardField::new(
                        if index == 0 { "Outbound" } else { "Return" },
                        slice_text(slice)?,
                    ))
                })
                .collect::<Vec<_>>();
            let flights = slices
                .iter()
                .flat_map(|slice| {
                    slice
                        .get("segments")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .filter_map(|segment| {
                    Some(format!(
                        "{}{}",
                        segment.pointer("/marketing_carrier/iata_code")?.as_str()?,
                        segment.get("marketing_carrier_flight_number")?.as_str()?
                    ))
                })
                .collect::<Vec<_>>();
            fields.push(CardField::new("Flights", flights.join(", ")));
            let cabin = offer
                .pointer("/slices/0/segments/0/passengers/0/cabin_class_marketing_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            fields.push(CardField::new("Cabin", cabin));
            fields.push(CardField::new(
                "Price held until",
                offer
                    .get("expires_at")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .replace('T', " ")
                    .chars()
                    .take(16)
                    .collect::<String>(),
            ));
            Some(card(
                "flight",
                format!("{airline} · {amount} {currency}"),
                format!(
                    "{} total for all passengers",
                    if slices.len() > 1 {
                        "Return"
                    } else {
                        "One-way"
                    }
                ),
                fields,
                None,
                attribution,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()
    }

    #[test]
    fn dates_routes_and_passengers_are_validated() {
        let ok = validate(&json!({"origin":"lis","destination":"New York","departure_date":"2026-11-10","return_date":"2026-11-17","adults":"2"}), today()).unwrap();
        assert_eq!(
            ok,
            json!({"origin":"LIS","destination":"New York","departure_date":"2026-11-10","return_date":"2026-11-17","adults":2,"cabin":"economy"})
        );
        assert!(validate(
            &json!({"origin":"LIS","destination":"lis","departure_date":"2026-11-10"}),
            today()
        )
        .is_err());
        assert!(validate(
            &json!({"origin":"LIS","destination":"JFK","departure_date":"2026-10-01"}),
            today()
        )
        .is_err());
        assert!(validate(
            &json!({"origin":"LIS","destination":"JFK","departure_date":"2028-01-01"}),
            today()
        )
        .is_err());
        assert!(validate(
            &json!({"origin":"LIS","destination":"JFK","departure_date":"next friday"}),
            today()
        )
        .is_err());
        assert!(validate(&json!({"origin":"LIS","destination":"JFK","departure_date":"2026-11-10","return_date":"2026-11-01"}), today()).is_err());
        assert!(validate(
            &json!({"origin":"LIS","destination":"JFK","departure_date":"2026-11-10","adults":12}),
            today()
        )
        .is_err());
        assert!(validate(
            &json!({"origin":"LIS","destination":"JFK","departure_date":"2026-11-10","book":true}),
            today()
        )
        .is_err());
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration("PT7H35M"), "7h 35m");
        assert_eq!(duration("P1DT2H"), "26h");
        assert_eq!(duration("PT45M"), "45m");
    }

    #[test]
    fn offers_show_route_times_stops_and_price() {
        let data = json!({"data":[{
            "total_amount":"412.30","total_currency":"EUR","expires_at":"2026-10-21T23:10:00Z","owner":{"name":"TAP Air Portugal"},
            "slices":[{"duration":"PT10H5M","segments":[
                {"origin":{"iata_code":"LIS"},"destination":{"iata_code":"OPO"},"departing_at":"2026-11-10T21:05:00","arriving_at":"2026-11-10T22:00:00","marketing_carrier":{"iata_code":"TP"},"marketing_carrier_flight_number":"1946","passengers":[{"cabin_class_marketing_name":"Economy"}]},
                {"origin":{"iata_code":"OPO"},"destination":{"iata_code":"JFK"},"departing_at":"2026-11-10T23:30:00","arriving_at":"2026-11-11T02:10:00","marketing_carrier":{"iata_code":"TP"},"marketing_carrier_flight_number":"209"}]}]
        },{"total_currency":"EUR"}]});
        let cards = parse_offers(&data, true);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].title, "TAP Air Portugal · 412.30 EUR");
        assert_eq!(
            cards[0].fields[0].value,
            "2026-11-10 LIS 21:05 → JFK 02:10 (+1) · 10h 5m · 1 stop via OPO"
        );
        assert_eq!(cards[0].fields[1].value, "TP1946, TP209");
        assert!(cards[0].attribution.starts_with("Duffel test mode"));
        assert_eq!(cards[0].url, None);
    }
}
