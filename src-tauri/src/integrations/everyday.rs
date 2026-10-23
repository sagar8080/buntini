//! Keyless everyday lookups: Open-Meteo weather, ECB rates via Frankfurter, Wikipedia.
use super::*;

const WEATHER_ATTRIBUTION: &str = "Weather data by Open-Meteo.com (CC BY 4.0)";
const RATES_ATTRIBUTION: &str = "European Central Bank reference rates via Frankfurter";
const WIKI_ATTRIBUTION: &str = "Wikipedia (CC BY-SA 4.0)";
const ECB_PAGE: &str = "https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.en.html";

/// Currencies with ECB reference rates (Frankfurter rejects anything else).
pub const CURRENCIES: [&str; 31] = [
    "AUD", "BGN", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD", "HUF", "IDR",
    "ILS", "INR", "ISK", "JPY", "KRW", "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "RON", "SEK",
    "SGD", "THB", "TRY", "USD", "ZAR",
];
pub const WIKI_LANGUAGES: [&str; 18] = [
    "en", "de", "fr", "es", "it", "pt", "nl", "sv", "pl", "ru", "uk", "ja", "zh", "ko", "ar", "hi",
    "tr", "id",
];

pub fn validate_weather(args: &Value) -> Result<Value, String> {
    // Always a full week: the model is unreliable at turning "this weekend" into a
    // day count, and the answer picks out the days that were asked about.
    let map = object(args, &["place", "units"])?;
    Ok(json!({
        "place": required(map, "place", 120)?,
        "units": choice(map, "units", &["celsius", "fahrenheit"])?.unwrap_or_else(|| "celsius".into()),
    }))
}

pub fn preview_weather(args: &Value) -> Preview {
    Preview {
        provider: "Open-Meteo".into(),
        host: "api.open-meteo.com".into(),
        summary: format!("Get the 7-day forecast for {}", text_of(args, "place")),
        sends: vec![
            CardField::new("Place", text_of(args, "place")),
            CardField::new("Units", text_of(args, "units")),
        ],
        uses_key: false,
    }
}

pub async fn weather(args: &Value) -> Result<Vec<ResultCard>, String> {
    let place = text_of(args, "place");
    let (name, hint) = match place.split_once(',') {
        Some((name, hint)) => (name.trim().to_string(), hint.trim().to_lowercase()),
        None => (place.clone(), String::new()),
    };
    let client = client()?;
    let found = json_response(
        client
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[
                ("name", name.as_str()),
                ("count", "5"),
                ("language", "en"),
                ("format", "json"),
            ])
            .send()
            .await,
        "Open-Meteo",
        262_144,
    )
    .await?;
    let (latitude, longitude, label) = pick_place(&found, &hint).ok_or_else(|| {
        format!("Open-Meteo couldn't find a place called “{place}”. Try a nearby city name.")
    })?;
    let fahrenheit = text_of(args, "units") == "fahrenheit";
    let mut query = vec![
        ("latitude", latitude.to_string()),
        ("longitude", longitude.to_string()),
        (
            "current",
            "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m"
                .into(),
        ),
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max"
                .into(),
        ),
        ("timezone", "auto".into()),
        ("forecast_days", "7".into()),
    ];
    if fahrenheit {
        query.push(("temperature_unit", "fahrenheit".into()));
        query.push(("wind_speed_unit", "mph".into()));
    }
    let forecast = json_response(
        client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&query)
            .send()
            .await,
        "Open-Meteo",
        262_144,
    )
    .await?;
    parse_weather(&label, &forecast)
}

fn pick_place(found: &Value, hint: &str) -> Option<(f64, f64, String)> {
    let results = found.get("results")?.as_array()?;
    let matches_hint = |place: &&Value| {
        hint.is_empty()
            || ["country", "admin1", "country_code"].iter().any(|key| {
                place
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        value.to_lowercase().contains(hint) || hint.contains(&value.to_lowercase())
                    })
            })
    };
    let place = results
        .iter()
        .find(matches_hint)
        .or_else(|| results.first())?;
    let label = [place.get("name"), place.get("admin1"), place.get("country")]
        .iter()
        .filter_map(|value| value.and_then(Value::as_str))
        .fold(Vec::<&str>::new(), |mut parts, part| {
            if !parts.contains(&part) {
                parts.push(part);
            }
            parts
        })
        .join(", ");
    Some((
        place.get("latitude")?.as_f64()?,
        place.get("longitude")?.as_f64()?,
        label,
    ))
}

pub(crate) fn parse_weather(label: &str, data: &Value) -> Result<Vec<ResultCard>, String> {
    let current = data
        .get("current")
        .ok_or("Open-Meteo returned no current conditions.")?;
    let units = data.get("current_units");
    let unit = |key: &str| {
        units
            .and_then(|units| units.get(key))
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let value = |key: &str| current.get(key).and_then(Value::as_f64);
    let mut cards = vec![card(
        "weather",
        format!("Now in {label}"),
        condition(value("weather_code")),
        vec![
            CardField::new(
                "Temperature",
                match (value("temperature_2m"), value("apparent_temperature")) {
                    (Some(now), Some(feels)) => format!(
                        "{now:.0}{} (feels like {feels:.0}{})",
                        unit("temperature_2m"),
                        unit("apparent_temperature")
                    ),
                    (Some(now), None) => format!("{now:.0}{}", unit("temperature_2m")),
                    _ => String::new(),
                },
            ),
            CardField::new(
                "Humidity",
                value("relative_humidity_2m")
                    .map(|h| format!("{h:.0}%"))
                    .unwrap_or_default(),
            ),
            CardField::new(
                "Wind",
                value("wind_speed_10m")
                    .map(|w| format!("{w:.0} {}", unit("wind_speed_10m")))
                    .unwrap_or_default(),
            ),
            CardField::new(
                "Local time",
                current
                    .get("time")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .replace('T', " "),
            ),
        ],
        Some("https://open-meteo.com/".into()),
        WEATHER_ATTRIBUTION,
    )];
    if let Some(daily) = data.get("daily") {
        let column = |key: &str| {
            daily
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let (dates, codes, highs, lows, rain) = (
            column("time"),
            column("weather_code"),
            column("temperature_2m_max"),
            column("temperature_2m_min"),
            column("precipitation_probability_max"),
        );
        let degree = data
            .get("daily_units")
            .and_then(|units| units.get("temperature_2m_max"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let fields = dates
            .iter()
            .enumerate()
            .filter_map(|(index, date)| {
                let date = chrono::NaiveDate::parse_from_str(date.as_str()?, "%Y-%m-%d").ok()?;
                let low = lows.get(index).and_then(Value::as_f64)?;
                let high = highs.get(index).and_then(Value::as_f64)?;
                let chance = rain
                    .get(index)
                    .and_then(Value::as_f64)
                    .map(|p| format!(" · {p:.0}% chance of rain"))
                    .unwrap_or_default();
                Some(CardField::new(
                    &date.format("%a %-d %b").to_string(),
                    format!(
                        "{} · {low:.0}–{high:.0}{degree}{chance}",
                        condition(codes.get(index).and_then(Value::as_f64))
                    ),
                ))
            })
            .collect::<Vec<_>>();
        if !fields.is_empty() {
            cards.push(card(
                "weather",
                format!("{}-day forecast", fields.len()),
                label,
                fields,
                Some("https://open-meteo.com/".into()),
                WEATHER_ATTRIBUTION,
            ));
        }
    }
    Ok(cards)
}

/// WMO weather interpretation codes used by Open-Meteo.
fn condition(code: Option<f64>) -> String {
    match code.map(|code| code as i64) {
        Some(0) => "Clear sky",
        Some(1) => "Mainly clear",
        Some(2) => "Partly cloudy",
        Some(3) => "Overcast",
        Some(45 | 48) => "Fog",
        Some(51 | 53 | 55) => "Drizzle",
        Some(56 | 57) => "Freezing drizzle",
        Some(61) => "Light rain",
        Some(63) => "Rain",
        Some(65) => "Heavy rain",
        Some(66 | 67) => "Freezing rain",
        Some(71 | 73 | 75) => "Snow",
        Some(77) => "Snow grains",
        Some(80..=82) => "Rain showers",
        Some(85 | 86) => "Snow showers",
        Some(95) => "Thunderstorm",
        Some(96 | 99) => "Thunderstorm with hail",
        _ => "Conditions unavailable",
    }
    .into()
}

pub fn validate_currency(args: &Value) -> Result<Value, String> {
    let map = object(args, &["amount", "from", "to"])?;
    let code = |value: String| {
        let value = value.trim().to_uppercase();
        if CURRENCIES.contains(&value.as_str()) {
            Ok(value)
        } else {
            Err(format!(
                "{value} has no ECB reference rate. Supported: {}.",
                CURRENCIES.join(", ")
            ))
        }
    };
    let from = code(required(map, "from", 3)?)?;
    let mut to = Vec::new();
    for part in required(map, "to", 40)?
        .split([',', ' '])
        .filter(|part| !part.is_empty())
    {
        let part = code(part.into())?;
        if part != from && !to.contains(&part) {
            to.push(part);
        }
    }
    if to.is_empty() || to.len() > 5 {
        return Err("Convert to between one and five other currencies.".into());
    }
    Ok(
        json!({ "amount": number(map, "amount", 0.000001, 1e12)?.unwrap_or(1.0), "from": from, "to": to.join(",") }),
    )
}

pub fn preview_currency(args: &Value) -> Preview {
    Preview {
        provider: "Frankfurter (ECB rates)".into(),
        host: "api.frankfurter.dev".into(),
        summary: format!(
            "Convert {} {} to {}",
            money(args.get("amount").and_then(Value::as_f64).unwrap_or(1.0)),
            text_of(args, "from"),
            text_of(args, "to").replace(',', ", ")
        ),
        sends: vec![
            CardField::new("Amount", text_of(args, "amount")),
            CardField::new("From", text_of(args, "from")),
            CardField::new("To", text_of(args, "to")),
        ],
        uses_key: false,
    }
}

pub async fn currency(args: &Value) -> Result<Vec<ResultCard>, String> {
    let amount = args
        .get("amount")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .to_string();
    let data = json_response(
        client()?
            .get("https://api.frankfurter.dev/v1/latest")
            .query(&[
                ("base", text_of(args, "from")),
                ("symbols", text_of(args, "to")),
                ("amount", amount),
            ])
            .send()
            .await,
        "Frankfurter",
        65_536,
    )
    .await?;
    parse_rates(&data)
}

pub(crate) fn parse_rates(data: &Value) -> Result<Vec<ResultCard>, String> {
    let amount = data
        .get("amount")
        .and_then(Value::as_f64)
        .ok_or("Frankfurter returned no amount.")?;
    let base = data
        .get("base")
        .and_then(Value::as_str)
        .ok_or("Frankfurter returned no base currency.")?;
    let date = data.get("date").and_then(Value::as_str).unwrap_or("latest");
    let rates = data
        .get("rates")
        .and_then(Value::as_object)
        .ok_or("Frankfurter returned no rates.")?;
    let cards = rates
        .iter()
        .filter_map(|(code, total)| {
            let total = total.as_f64()?;
            let rate = total / amount;
            Some(card(
                "currency",
                format!("{} {base} = {} {code}", money(amount), money(total)),
                format!("ECB reference rate for {date}"),
                vec![
                    CardField::new("Rate", format!("1 {base} = {} {code}", amount_text(rate))),
                    CardField::new(
                        "Inverse",
                        format!("1 {code} = {} {base}", amount_text(1.0 / rate)),
                    ),
                ],
                Some(ECB_PAGE.into()),
                RATES_ATTRIBUTION,
            ))
        })
        .collect::<Vec<_>>();
    if cards.is_empty() {
        return Err("Frankfurter returned no usable rates.".into());
    }
    Ok(cards)
}

/// Totals read like money: cents for normal amounts, more precision for tiny ones.
fn money(value: f64) -> String {
    trim_zeros(if value.abs() >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    })
}

fn amount_text(value: f64) -> String {
    trim_zeros(if value.abs() >= 100.0 {
        format!("{value:.2}")
    } else if value.abs() >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.6}")
    })
}

fn trim_zeros(text: String) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

pub fn validate_wikipedia(args: &Value) -> Result<Value, String> {
    let map = object(args, &["query", "language"])?;
    Ok(json!({
        "query": required(map, "query", 200)?,
        "language": choice(map, "language", &WIKI_LANGUAGES)?.unwrap_or_else(|| "en".into()),
    }))
}

pub fn preview_wikipedia(args: &Value) -> Preview {
    let language = text_of(args, "language");
    Preview {
        provider: "Wikipedia".into(),
        host: format!("{language}.wikipedia.org"),
        summary: format!("Look up “{}” on Wikipedia", text_of(args, "query")),
        sends: vec![
            CardField::new("Search", text_of(args, "query")),
            CardField::new("Language", language),
        ],
        uses_key: false,
    }
}

pub async fn wikipedia(args: &Value) -> Result<Vec<ResultCard>, String> {
    let language = text_of(args, "language");
    if !WIKI_LANGUAGES.contains(&language.as_str()) {
        return Err("Unsupported Wikipedia language.".into());
    }
    let client = client()?;
    let base = format!("https://{language}.wikipedia.org");
    let found = json_response(
        client
            .get(format!("{base}/w/rest.php/v1/search/page"))
            .query(&[("q", text_of(args, "query")), ("limit", "4".into())])
            .send()
            .await,
        "Wikipedia",
        262_144,
    )
    .await?;
    let pages = found
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first = pages
        .first()
        .and_then(|page| page.get("key"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "Wikipedia has no article matching “{}”.",
                text_of(args, "query")
            )
        })?;
    let mut summary_url = reqwest::Url::parse(&format!("{base}/api/rest_v1/page/summary/"))
        .map_err(|e| e.to_string())?;
    summary_url
        .path_segments_mut()
        .map_err(|_| "Invalid Wikipedia address.")?
        .pop_if_empty()
        .push(first);
    let summary = json_response(client.get(summary_url).send().await, "Wikipedia", 262_144).await?;
    Ok(parse_wikipedia(&base, &summary, &pages))
}

pub(crate) fn parse_wikipedia(base: &str, summary: &Value, pages: &[Value]) -> Vec<ResultCard> {
    let page_url = |key: &str| {
        let mut url = reqwest::Url::parse(&format!("{base}/wiki/")).ok()?;
        url.path_segments_mut().ok()?.pop_if_empty().push(key);
        Some(url.to_string())
    };
    let get = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut cards = vec![];
    let main_title = get(summary, "title");
    if !main_title.is_empty() {
        let mut subtitle = get(summary, "description");
        if get(summary, "type") == "disambiguation" {
            subtitle = "Disambiguation page: several topics share this name".into();
        }
        let url = summary
            .pointer("/content_urls/desktop/page")
            .and_then(Value::as_str)
            .map(str::to_string);
        cards.push(card(
            "article",
            main_title.clone(),
            subtitle,
            vec![
                CardField::new("Summary", get(summary, "extract")),
                CardField::new(
                    "Last edited",
                    get(summary, "timestamp")
                        .replace('T', " ")
                        .trim_end_matches('Z')
                        .to_string(),
                ),
            ],
            url,
            WIKI_ATTRIBUTION,
        ));
    }
    for page in pages {
        let title = get(page, "title");
        if title.is_empty() || title == main_title {
            continue;
        }
        cards.push(card(
            "article",
            title,
            get(page, "description"),
            vec![],
            page_url(&get(page, "key")),
            WIKI_ATTRIBUTION,
        ));
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_arguments_get_defaults_and_bounds() {
        assert_eq!(
            validate_weather(&json!({"place":" Lisbon "})).unwrap(),
            json!({"place":"Lisbon","units":"celsius"})
        );
        assert_eq!(
            validate_weather(&json!({"place":"Austin, Texas","units":"Fahrenheit"})).unwrap()
                ["units"],
            "fahrenheit"
        );
        assert!(validate_weather(&json!({"place":"Lisbon","days":2})).is_err());
        assert!(validate_weather(&json!({"days":2})).is_err());
    }

    #[test]
    fn weather_prefers_the_hinted_country_and_formats_the_forecast() {
        let found = json!({"results":[{"name":"Paris","latitude":33.6,"longitude":-95.5,"admin1":"Texas","country":"United States"},{"name":"Paris","latitude":48.85,"longitude":2.35,"admin1":"Île-de-France","country":"France"}]});
        let (latitude, _, label) = pick_place(&found, "france").unwrap();
        assert_eq!(
            (latitude, label.as_str()),
            (48.85, "Paris, Île-de-France, France")
        );
        assert_eq!(
            pick_place(&found, "").unwrap().2,
            "Paris, Texas, United States"
        );
        let forecast = json!({"current_units":{"temperature_2m":"°C","apparent_temperature":"°C","wind_speed_10m":"km/h"},"current":{"time":"2026-10-21T22:00","temperature_2m":13.3,"apparent_temperature":11.8,"relative_humidity_2m":74,"weather_code":0,"wind_speed_10m":8.4},"daily_units":{"temperature_2m_max":"°C"},"daily":{"time":["2026-10-21","2026-10-22"],"weather_code":[3,61],"temperature_2m_max":[20.8,20.0],"temperature_2m_min":[11.3,9.2],"precipitation_probability_max":[0,40]}});
        let cards = parse_weather("Lisbon, Portugal", &forecast).unwrap();
        assert_eq!(cards[0].title, "Now in Lisbon, Portugal");
        assert_eq!(cards[0].subtitle, "Clear sky");
        assert_eq!(cards[0].fields[0].value, "13°C (feels like 12°C)");
        assert_eq!(
            cards[1].fields[1],
            CardField::new("Thu 22 Oct", "Light rain · 9–20°C · 40% chance of rain")
        );
        assert!(parse_weather("x", &json!({})).is_err());
    }

    #[test]
    fn currency_codes_are_checked_and_deduplicated() {
        assert_eq!(
            validate_currency(&json!({"from":"usd","to":"eur, GBP,eur,USD","amount":"1,250"}))
                .unwrap(),
            json!({"amount":1250.0,"from":"USD","to":"EUR,GBP"})
        );
        assert!(validate_currency(&json!({"from":"USD","to":"BTC"})).is_err());
        assert!(validate_currency(&json!({"from":"USD","to":"USD"})).is_err());
        assert!(validate_currency(&json!({"from":"USD","to":"EUR","amount":-5})).is_err());
        let cards = parse_rates(
            &json!({"amount":100.0,"base":"USD","date":"2026-10-21","rates":{"EUR":89.015}}),
        )
        .unwrap();
        assert_eq!(cards[0].title, "100 USD = 89.02 EUR");
        assert_eq!(cards[0].fields[0].value, "1 USD = 0.89015 EUR");
        assert!(parse_rates(&json!({"amount":1.0,"base":"USD","rates":{}})).is_err());
    }

    #[test]
    fn wikipedia_cards_link_to_encoded_article_paths() {
        assert!(validate_wikipedia(&json!({"query":"Turing","language":"xx"})).is_err());
        let summary = json!({"type":"standard","title":"Alan Turing","description":"English computer scientist","extract":"Alan Mathison Turing was…","timestamp":"2026-10-21T18:09:45Z","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Alan_Turing"}}});
        let pages = vec![
            json!({"key":"Alan_Turing","title":"Alan Turing"}),
            json!({"key":"Turing_test","title":"Turing test","description":"Test of machine intelligence"}),
            json!({"key":"A/B?c","title":"Odd"}),
        ];
        let cards = parse_wikipedia("https://en.wikipedia.org", &summary, &pages);
        assert_eq!(cards.len(), 3);
        assert_eq!(
            cards[0].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Alan_Turing")
        );
        assert_eq!(
            cards[1].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Turing_test")
        );
        assert_eq!(
            cards[2].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/A%2FB%3Fc")
        );
    }
}
