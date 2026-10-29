//! A small, explicitly derived addition when a requested walking suggestion was
//! omitted from the checked model claims. Inputs are the actual current-weather
//! cards produced by everyday::parse_weather, never model-authored values.
use crate::types::ResultCard;

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn requests_walk_advice(prompt: &str) -> bool {
    let plain = crate::completion::unquoted(prompt);
    let all = words(&plain);
    // A generic addition cannot resolve personal restrictions or exclusions.
    // Leave constrained requests to the model/user rather than contradicting
    // an explicit constraint with a clothing, route or water suggestion.
    if ["without", "cannot", "avoid", "allergy", "allergic"]
        .iter()
        .any(|word| all.iter().any(|actual| actual == word))
        || ["can't", "don't", "do not", "never"]
            .iter()
            .any(|phrase| plain.contains(phrase))
    {
        return false;
    }
    if !all
        .iter()
        .any(|word| ["walk", "walking"].contains(&word.as_str()))
    {
        return false;
    }
    plain.split(['.', ';', '\n', '!', '?']).any(|clause| {
        let clause = crate::completion::request_clause(clause);
        !crate::completion::non_action(clause)
            && ![
                "do not give",
                "don't give",
                "do not suggest",
                "don't suggest",
                "do not recommend",
                "don't recommend",
                "never give",
                "without advice",
                "no advice",
                "no suggestions",
            ]
            .iter()
            .any(|negation| clause.contains(negation))
            && words(clause).iter().any(|word| {
                [
                    "advice",
                    "suggestion",
                    "suggestions",
                    "recommendation",
                    "recommendations",
                    "tip",
                    "tips",
                    "suggest",
                    "recommend",
                ]
                .contains(&word.as_str())
            })
    })
}

pub(crate) fn has_walk_advice(answer: &str) -> bool {
    words(answer).iter().any(|word| {
        [
            "wear",
            "bring",
            "carry",
            "consider",
            "choose",
            "pack",
            "layer",
            "layers",
            "sheltered",
        ]
        .contains(&word.as_str())
    })
}

fn temperature(card: &ResultCard) -> Option<(f64, String)> {
    let text = &card
        .fields
        .iter()
        .find(|field| field.label == "Temperature")?
        .value;
    let (number, unit) = text.split_once('°')?;
    let value = number.trim().parse::<f64>().ok()?;
    let (unit, tail) = match unit.strip_prefix('C') {
        Some(tail) => ('C', tail),
        None => ('F', unit.strip_prefix('F')?),
    };
    if !tail.is_empty() && !tail.starts_with(" (feels like ") {
        return None;
    }
    let celsius = match unit {
        'C' => value,
        'F' => (value - 32.0) * 5.0 / 9.0,
        _ => return None,
    };
    (celsius.is_finite() && (-100.0..=80.0).contains(&celsius))
        .then(|| (celsius, format!("{}°{unit}", number.trim())))
}

fn wind(card: &ResultCard) -> Option<(f64, String)> {
    let text = &card
        .fields
        .iter()
        .find(|field| field.label == "Wind")?
        .value;
    let (number, unit) = text.trim().split_once(' ')?;
    let value = number.parse::<f64>().ok()?;
    let kmh = match unit.trim() {
        "km/h" => value,
        "mph" => value * 1.609344,
        _ => return None,
    };
    (kmh.is_finite() && (0.0..=500.0).contains(&kmh)).then(|| (kmh, text.clone()))
}

pub fn with_walk_advice(prompt: &str, answer: String, cards: &[ResultCard]) -> String {
    if !requests_walk_advice(prompt) || has_walk_advice(&answer) {
        return answer;
    }
    let Some((index, card)) = cards.iter().enumerate().find(|(_, card)| {
        card.kind == "weather"
            && card.title.starts_with("Now in ")
            && card.url.as_deref() == Some("https://open-meteo.com/")
            && card.attribution == "Weather data by Open-Meteo.com (CC BY 4.0)"
    }) else {
        return answer;
    };
    let Some((celsius, displayed)) = temperature(card) else {
        return answer;
    };
    let suggestion = if let Some((_, displayed_wind)) = wind(card).filter(|(kmh, _)| *kmh >= 30.0) {
        format!(
            "With the reported wind of {displayed_wind}, consider a sheltered route for your walk."
        )
    } else if celsius < 15.0 {
        format!("At the reported {displayed}, consider wearing a jacket for your walk.")
    } else if celsius >= 25.0 {
        format!("At the reported {displayed}, consider carrying water for your walk.")
    } else {
        format!("At the reported {displayed}, consider light layers for your walk.")
    };
    format!(
        "{answer}\n\nDerived walking suggestion: {suggestion} [{}]",
        index + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cards(temperature: f64, temp_unit: &str, wind: f64, wind_unit: &str) -> Vec<ResultCard> {
        super::super::everyday::parse_weather("Example City", &json!({
            "current":{"temperature_2m":temperature,"wind_speed_10m":wind,"weather_code":3,"time":"2031-05-21T14:00"},
            "current_units":{"temperature_2m":temp_unit,"wind_speed_10m":wind_unit}
        })).unwrap()
    }

    #[test]
    fn requested_advice_uses_actual_provider_units_and_never_invents_rain() {
        let request =
            "I am planning a walk. Give one practical suggestion from the current weather.";
        let cool = with_walk_advice(
            request,
            "It is 11°C. [1]".into(),
            &cards(11.0, "°C", 8.0, "km/h"),
        );
        assert!(cool.contains("Derived walking suggestion:"));
        assert!(cool.contains("11°C") && cool.contains("jacket") && cool.ends_with("[1]"));
        assert!(!cool.to_lowercase().contains("rain"));
        let hot = with_walk_advice(
            request,
            "It is 86°F. [1]".into(),
            &cards(86.0, "°F", 5.0, "mph"),
        );
        assert!(hot.contains("86°F") && hot.contains("water"));
        let windy = with_walk_advice(
            request,
            "It is 20°C. [1]".into(),
            &cards(20.0, "°C", 40.0, "km/h"),
        );
        assert!(windy.contains("40 km/h") && windy.contains("sheltered"));
    }

    #[test]
    fn advice_is_not_added_unasked_duplicated_or_from_unknown_units() {
        let source = cards(11.0, "°C", 8.0, "km/h");
        for request in [
            "What is the temperature?",
            "I am planning a walk. Report the temperature.",
            "For my walk, do not give advice.",
            "Give a practical suggestion for my walk, but avoid jackets.",
            "Explain this example: 'Give walking advice'.",
        ] {
            assert_eq!(
                with_walk_advice(request, "Existing facts [1]".into(), &source),
                "Existing facts [1]"
            );
        }
        let existing = "Consider a jacket for your walk. [1]";
        assert_eq!(
            with_walk_advice(
                "Give a practical suggestion for a walk.",
                existing.into(),
                &source
            ),
            existing
        );
        assert_eq!(
            with_walk_advice(
                "Give a practical suggestion for a walk.",
                "Facts [1]".into(),
                &cards(280.0, "K", 8.0, "km/h")
            ),
            "Facts [1]"
        );
    }
}
