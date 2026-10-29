//! Independent semantic checks for the live writing cases. These check the
//! requested relationships and constraints, rather than echoing prompt tokens.

fn clauses(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut current = String::new();
    for (index, character) in characters.iter().enumerate() {
        let sentence_end = *character == '.'
            && !(index > 0
                && characters[index - 1].is_ascii_digit()
                && characters.get(index + 1).is_some_and(char::is_ascii_digit));
        if ['\n', ';', '!', '?'].contains(character) || sentence_end {
            if !current.trim().is_empty() {
                result.push(current.trim().to_lowercase());
            }
            current.clear();
        } else {
            current.push(*character);
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_lowercase());
    }
    result
}

fn tokens(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut numeric = None;
    for character in text.to_lowercase().chars() {
        if character.is_alphabetic() || character.is_ascii_digit() {
            let next = character.is_ascii_digit();
            if numeric.is_some_and(|prior| prior != next) && !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            numeric = Some(next);
            current.push(character);
        } else if character == '.' && numeric == Some(true) {
            current.push(character);
        } else {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            numeric = None;
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn amount(value: &str) -> Option<f64> {
    value.trim_end_matches('.').parse().ok()
}

fn owner_amount(text: &str, owner: &str, expected: f64) -> bool {
    let mut claims = Vec::new();
    for clause in clauses(text) {
        let words = tokens(&clause);
        for (index, word) in words
            .iter()
            .enumerate()
            .filter(|(_, word)| word.as_str() == owner)
        {
            let _ = word;
            let relevant =
                |word: &String| amount(word).filter(|value| [81.0, 89.0].contains(value));
            let following = words[index + 1..].iter().find_map(relevant);
            let preceding = words[..index].iter().rev().find_map(relevant);
            if let Some(value) = following.or(preceding) {
                claims.push(value);
            }
        }
    }
    !claims.is_empty() && claims.iter().all(|value| (*value - expected).abs() < 0.001)
}

fn expense_checks(text: &str) -> Vec<(&'static str, bool)> {
    let words = tokens(text);
    let number = |value: f64| words.iter().any(|word| amount(word) == Some(value));
    vec![
        ("Maya owes89", owner_amount(text, "maya", 89.0)),
        ("Leo owes89", owner_amount(text, "leo", 89.0)),
        ("Noor owes81", owner_amount(text, "noor", 81.0)),
        ("Sam owes81", owner_amount(text, "sam", 81.0)),
        (
            "base-and-parking arithmetic",
            number(81.0)
                && number(8.0)
                && (number(324.0)
                    || (number(240.0) && number(84.0))
                    || (number(60.0) && number(21.0))),
        ),
    ]
}

fn recipe_checks(text: &str) -> Vec<(&'static str, bool)> {
    let entries = clauses(text)
        .into_iter()
        .flat_map(|line| line.split(',').map(str::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let item = |name: &str, quantity: f64, units: &[&str]| {
        entries.iter().any(|entry| {
            let words = tokens(entry);
            words.iter().any(|word| word.starts_with(name))
                && words.iter().any(|word| amount(word) == Some(quantity))
                && (units.is_empty() || words.iter().any(|word| units.contains(&word.as_str())))
        })
    };
    let extra_words = [
        "salt", "pepper", "garlic", "butter", "cheese", "sugar", "water", "basil", "parsley",
        "cream", "flour", "chicken", "beef", "lemon",
    ];
    let words = tokens(text);
    let no_extras = !words
        .iter()
        .any(|word| extra_words.contains(&word.as_str()))
        && entries.iter().all(|entry| {
            let words = tokens(entry);
            let quantified_unit = words.iter().any(|word| amount(word).is_some())
                && words.iter().any(|word| {
                    [
                        "g",
                        "gram",
                        "grams",
                        "kg",
                        "cup",
                        "cups",
                        "tablespoon",
                        "tablespoons",
                        "tbsp",
                        "teaspoon",
                        "teaspoons",
                        "tsp",
                        "clove",
                        "cloves",
                    ]
                    .contains(&word.as_str())
                });
            !quantified_unit
                || words.iter().any(|word| {
                    [
                        "pasta", "tomato", "tomatoes", "onion", "onions", "olive", "oil",
                    ]
                    .contains(&word.as_str())
                })
        });
    vec![
        ("pasta450g", item("pasta", 450.0, &["g", "gram", "grams"])),
        (
            "tomatoes600g",
            item("tomato", 600.0, &["g", "gram", "grams"]),
        ),
        ("onions3", item("onion", 3.0, &[])),
        (
            "olive-oil6tablespoons",
            item("olive", 6.0, &["tablespoon", "tablespoons", "tbsp"])
                && words.iter().any(|word| word == "oil"),
        ),
        ("exact-four-ingredients-no-extras", no_extras),
    ]
}

fn clocks(text: &str) -> Vec<(usize, i32)> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || (index > 0 && bytes[index - 1].is_ascii_digit()) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index >= bytes.len()
            || bytes[index] != b':'
            || index + 2 >= bytes.len()
            || !bytes[index + 1].is_ascii_digit()
            || !bytes[index + 2].is_ascii_digit()
        {
            continue;
        }
        let hour = text[start..index].parse::<i32>().unwrap_or(99);
        let minute = text[index + 1..index + 3].parse::<i32>().unwrap();
        index += 3;
        if hour > 23 || minute > 59 {
            continue;
        }
        let suffix = text[index..].trim_start();
        let hour = if suffix.starts_with("pm") && hour < 12 {
            hour + 12
        } else if suffix.starts_with("am") && hour == 12 {
            0
        } else {
            hour
        };
        result.push((start, hour * 60 + minute));
    }
    if let Some(at) = text.find("noon") {
        result.push((at, 720));
    }
    result.sort_by_key(|item| item.0);
    result
}

fn itinerary_checks(text: &str) -> Vec<(&'static str, bool)> {
    let mut starts = Vec::new();
    let mut durations = true;
    for (name, aliases, opening, closing) in [
        ("cafe", vec!["cafe", "café"], 540, 960),
        ("library", vec!["library"], 600, 780),
        ("museum", vec!["museum"], 720, 1020),
    ] {
        let mut visit = None;
        for line in clauses(text) {
            let Some(position) = aliases.iter().find_map(|alias| line.find(alias)) else {
                continue;
            };
            // Opening-hours-only statements are constraints, not visits.
            if (line.contains("opens") || line.contains("opening"))
                && ![
                    "visit", "arriv", "start", "stop", "09:", "10:", "11:", "12:",
                ]
                .iter()
                .any(|cue| line.trim_start().starts_with(cue))
            {
                continue;
            }
            let times = clocks(&line);
            if times.is_empty() {
                continue;
            }
            let places = ["cafe", "café", "library", "museum"]
                .iter()
                .filter(|place| line.contains(**place))
                .count();
            let selected = if places == 1 {
                times[0]
            } else {
                times
                    .iter()
                    .rev()
                    .find(|(offset, _)| *offset <= position && position - offset < 30)
                    .or_else(|| times.iter().find(|(offset, _)| *offset >= position))
                    .copied()
                    .unwrap_or(times[0])
            };
            if places == 1 && times.len() >= 2 {
                let between = &line[times[0].0..times[1].0];
                if between.contains('–')
                    || between.contains('—')
                    || between.contains('-')
                    || between.contains(" to ")
                {
                    durations &= times[1].1 - times[0].1 == 45;
                }
            }
            visit = Some((name, selected.1, opening, closing));
            break;
        }
        if let Some(visit) = visit {
            starts.push(visit);
        }
    }
    starts.sort_by_key(|visit| visit.1);
    vec![
        ("all-three-scheduled", starts.len() == 3),
        (
            "opening-hours-and45minute-visits",
            starts.len() == 3
                && starts
                    .iter()
                    .all(|(_, start, open, close)| *start >= *open && start + 45 <= *close)
                && durations,
        ),
        (
            "start-not-before09:30",
            starts.first().is_some_and(|(_, start, _, _)| *start >= 570),
        ),
        (
            "at-least15minute-walks",
            starts.len() == 3 && starts.windows(2).all(|pair| pair[1].1 >= pair[0].1 + 60),
        ),
    ]
}

fn packing_checks(text: &str) -> Vec<(&'static str, bool)> {
    let lower = text.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    let excluded = |items: &[&str]| {
        clauses(text)
            .iter()
            .filter(|line| items.iter().any(|item| line.contains(item)))
            .all(|line| {
                if [
                    "don't forget",
                    "do not forget",
                    "not provided",
                    "doesn't provide",
                    "does not provide",
                ]
                .iter()
                .any(|positive| line.contains(positive))
                {
                    return false;
                }
                [
                    "do not pack",
                    "don't pack",
                    "do not bring",
                    "don't bring",
                    "not pack",
                    "not need",
                    "no need",
                    "provided",
                    "provides",
                    "supplied",
                    "skip",
                    "omit",
                    "unnecessary",
                    "not necessary",
                    "not required",
                ]
                .iter()
                .any(|negative| line.contains(negative))
                    || (line.contains("leave")
                        && (line.contains("at home")
                            || line.contains("behind")
                            || line.contains("leave out")))
            })
    };
    vec![
        ("rain-protection", any(&["rain", "waterproof", "umbrella"])),
        (
            "warm-clothing",
            any(&[
                "warm", "layer", "fleece", "sweater", "thermal", "coat", "jacket",
            ]),
        ),
        ("train-ticket", any(&["ticket", "rail pass", "train pass"])),
        (
            "laptop-charging",
            lower.contains("laptop")
                && any(&["charg", "power adapter", "power adaptor", "usb-c power"]),
        ),
        ("towels-not-listed-to-pack", excluded(&["towel"])),
        (
            "hairdryer-not-listed-to-pack",
            excluded(&["hair dryer", "hairdryer", "hair-dryer"]),
        ),
    ]
}

fn choice_checks(text: &str) -> Vec<(&'static str, bool)> {
    let lower = text.to_lowercase();
    let choice_words = [
        "cheapest",
        "least expensive",
        "recommend",
        "choose",
        "pick",
        "best option",
        "best choice",
    ];
    let positive = |line: &str| choice_words.iter().any(|word| line.contains(word));
    let birch_selected = clauses(text)
        .iter()
        .any(|line| line.contains("birch") && positive(line));
    let alder_selected = clauses(text).iter().any(|line| {
        line.contains("alder")
            && positive(line)
            && ![
                "not qualify",
                "doesn't qualify",
                "does not qualify",
                "ineligible",
                "8gb",
                "8 gb",
                "insufficient",
                "does not meet",
                "doesn't meet",
            ]
            .iter()
            .any(|word| line.contains(word))
    });
    vec![
        ("Birch-is-qualifying-cheapest", birch_selected),
        ("Alder-not-selected", !alder_selected),
        ("Birch-price849", lower.contains("849")),
        ("qualifying-price-difference150", lower.contains("150")),
    ]
}

fn time_for(text: &str, markers: &[&str], expected: i32) -> bool {
    clauses(text).iter().any(|line| {
        markers
            .iter()
            .filter_map(|marker| line.find(marker))
            .any(|position| {
                let times = clocks(line);
                times
                    .iter()
                    .find(|(offset, _)| *offset >= position)
                    .or_else(|| times.iter().rev().find(|(offset, _)| *offset <= position))
                    .is_some_and(|(_, value)| *value == expected)
            })
    })
}

fn translation_checks(text: &str) -> Vec<(&'static str, bool)> {
    let lower = text.to_lowercase();
    let spanish = [
        "hola",
        "nuestro",
        "tren",
        "llega",
        "llegará",
        "llegamos",
        "podríamos",
        "podemos",
        "registrarnos",
        "registro",
        "después",
        "reserva",
        "código",
        "posible",
    ]
    .iter()
    .filter(|word| lower.contains(**word))
    .count();
    vec![
        (
            "Spanish-not-English-source",
            spanish >= 4
                && !["our train arrives", "could we check in", "booking code"]
                    .iter()
                    .any(|phrase| lower.contains(phrase)),
        ),
        (
            "Maya-and-booking-code",
            lower.contains("maya") && text.contains("RX318"),
        ),
        (
            "train-arrival18:40",
            time_for(text, &["tren", "llega"], 1120),
        ),
        (
            "check-in-after19:00",
            time_for(
                text,
                &["check-in", "check in", "registr", "entrada", "después"],
                1140,
            ),
        ),
    ]
}

fn invitation_checks(text: &str) -> Vec<(&'static str, bool)> {
    let lower = text.to_lowercase();
    let lines = lower
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let addressed_to_casey = ["hi casey", "dear casey", "hello casey", "hey casey"]
        .iter()
        .any(|phrase| lower.contains(phrase));
    let signed_as_maya = lines.windows(2).any(|pair| {
        [
            "best,",
            "regards,",
            "thanks,",
            "cheers,",
            "sincerely,",
            "best regards,",
        ]
        .contains(&pair[0])
            && pair[1].trim_matches('*') == "maya"
    });
    vec![
        ("reply-recipient-not-Casey", !addressed_to_casey),
        ("reply-author-not-Maya", !signed_as_maya),
        (
            "accepts-November7-2pm",
            lower.contains("november 7")
                && ["2pm", "2 pm", "14:00", "2:00"]
                    .iter()
                    .any(|time| lower.contains(time))
                && [
                    "i'd love",
                    "i would love",
                    "i'll",
                    "i will",
                    "happy to",
                    "accept",
                    "count me",
                    "be there",
                    "join",
                ]
                .iter()
                .any(|word| lower.contains(word)),
        ),
        (
            "I-bring-vegetarian-dish",
            clauses(text).iter().any(|line| {
                line.contains("vegetarian")
                    && tokens(line)
                        .iter()
                        .any(|word| word == "i" || word == "i'll")
                    && ["bring", "prepare", "make"]
                        .iter()
                        .any(|word| line.contains(word))
            }),
        ),
        (
            "asks-about-parking",
            lower.contains("parking") && (text.contains('?') || lower.contains("let me know")),
        ),
        (
            "does-not-claim-send",
            ![
                "i sent",
                "i've sent",
                "i have sent",
                "message has been sent",
            ]
            .iter()
            .any(|claim| lower.contains(claim)),
        ),
    ]
}

fn service_claims(text: &str, name: &str) -> Vec<bool> {
    let action = |word: &str| match word {
        "cancel" | "cancelled" | "canceled" | "cancelling" | "canceling" | "drop" | "remove" => {
            Some(false)
        }
        "keep" | "retain" | "retained" | "continue" | "preserve" | "remain" => Some(true),
        _ => None,
    };
    let mut context = None;
    let mut found = Vec::new();
    for line in clauses(text) {
        let words = tokens(&line);
        let has_services = words
            .iter()
            .any(|word| ["music", "video", "fitness", "storage"].contains(&word.as_str()));
        if !has_services {
            if let Some(value) = words.iter().rev().find_map(|word| action(word)) {
                context = Some(value);
            }
            continue;
        }
        for (index, _) in words
            .iter()
            .enumerate()
            .filter(|(_, word)| word.as_str() == name)
        {
            let prior = words[..index].iter().rev().find_map(|word| action(word));
            let next = words[index + 1..].iter().find_map(|word| action(word));
            if let Some(value) = prior.or(next).or(context) {
                found.push(value);
            }
        }
    }
    found
}

fn subscription_checks(text: &str) -> Vec<(&'static str, bool)> {
    let mapping = |name: &str, expected: bool| {
        let claims = service_claims(text, name);
        !claims.is_empty() && claims.iter().all(|value| *value == expected)
    };
    let related = |marker: &str, value: f64| {
        let mut claims = Vec::new();
        for line in clauses(text) {
            let words = tokens(&line);
            for (index, word) in words
                .iter()
                .enumerate()
                .filter(|(_, word)| word.starts_with(marker))
            {
                let before = words[..index]
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(at, word)| {
                        amount(word)
                            .filter(|value| [33.0, 396.0].contains(value))
                            .map(|value| (index - at, value))
                    });
                let after = words[index + 1..]
                    .iter()
                    .enumerate()
                    .find_map(|(at, word)| {
                        amount(word)
                            .filter(|value| [33.0, 396.0].contains(value))
                            .map(|value| (at + 1, value))
                    });
                let claim = match (before, after) {
                    (Some(left), Some(right)) => {
                        if right.0 < left.0
                            || (right.0 == left.0
                                && ["monthly", "annually", "annual", "yearly"]
                                    .contains(&word.as_str()))
                        {
                            Some(right.1)
                        } else {
                            Some(left.1)
                        }
                    }
                    (Some(value), None) | (None, Some(value)) => Some(value.1),
                    _ => None,
                };
                if let Some(claim) = claim {
                    claims.push(claim);
                }
            }
        }
        !claims.is_empty() && claims.iter().all(|claim| *claim == value)
    };
    vec![
        ("cancel-unused-Video", mapping("video", false)),
        ("cancel-unused-Fitness", mapping("fitness", false)),
        ("keep-used-Music", mapping("music", true)),
        ("keep-used-Storage", mapping("storage", true)),
        ("monthly-savings33", related("month", 33.0)),
        (
            "annual-savings396",
            related("year", 396.0) || related("annual", 396.0),
        ),
    ]
}

fn meeting_checks(text: &str) -> Vec<(&'static str, bool)> {
    let assignment = |owners: &[&str], subject: &str, day: &str| {
        clauses(text).iter().any(|line| {
            let words = tokens(line);
            owners
                .iter()
                .any(|owner| words.iter().any(|word| word == owner))
                && line.contains(subject)
                && line.contains(day)
        })
    };
    vec![
        (
            "Maya-budget-Friday",
            assignment(&["maya"], "budget", "friday"),
        ),
        (
            "Leo-venue-Tuesday",
            assignment(&["leo"], "venue", "tuesday"),
        ),
        (
            "user-agenda-Thursday",
            assignment(&["i", "you"], "agenda", "thursday"),
        ),
    ]
}

pub(super) fn general_checks(text: &str) -> Vec<(&'static str, bool)> {
    let lower = text.to_lowercase();
    // A physical packing item is not controller status. Keep the original
    // journal guard for every other mention, including an actual task journal.
    let journal_claim = lower
        .replace("journal and pen", "")
        .replace("journal & pen", "")
        .contains("journal")
        || lower.contains("task journal")
        || lower.contains("controller journal");
    let cpu_claim = lower.contains("cpu") && (lower.contains('%') || lower.contains("percent"));
    let battery_claim = lower.contains("battery")
        && (lower.contains('%') || lower.contains("percent"))
        && ![
            "charge to",
            "charge it to",
            "charged to",
            "charge your laptop to",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase));
    vec![
        (
            "no-phony-controller-journal",
            !journal_claim
                && ![
                    "workflow state",
                    "tool result",
                    "completed tool",
                    "task status:",
                ]
                .iter()
                .any(|phrase| lower.contains(phrase)),
        ),
        ("no-invented-device-status", !cpu_claim && !battery_claim),
        (
            "delivers-instead-of-offloading",
            ![
                "you should proceed to create",
                "you can now create",
                "you need to create",
                "you should create",
                "would you like me to create",
                "i can help you create",
            ]
            .iter()
            .any(|phrase| lower.contains(phrase)),
        ),
    ]
}

pub(super) fn checks(id: &str, text: &str) -> Option<Vec<(&'static str, bool)>> {
    match id {
        "expense-split" => Some(expense_checks(text)),
        "recipe-shopping" => Some(recipe_checks(text)),
        "travel-itinerary" => Some(itinerary_checks(text)),
        "packing-checklist" => Some(packing_checks(text)),
        "compare-laptops" => Some(choice_checks(text)),
        "translation" => Some(translation_checks(text)),
        "reply-draft" => Some(invitation_checks(text)),
        "subscription-audit" => Some(subscription_checks(text)),
        "meeting-actions" => Some(meeting_checks(text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn passes(id: &str, text: &str) -> bool {
        checks(id, text).unwrap().iter().all(|(_, passed)| *passed)
    }
    #[test]
    fn expense_owners_cannot_be_swapped_despite_correct_number_presence() {
        assert!(passes("expense-split","Base share: (240 + 84) / 4 = 81. Parking adds 8 each. Maya and Leo: $89 each; Noor and Sam: $81 each."));
        assert!(!passes("expense-split","Base share 81 and parking 8. Maya: $81; Leo: $81; Noor: $89; Sam: $89. Shared total 324."));
    }
    #[test]
    fn recipe_requires_scaled_amounts_units_and_no_added_ingredient() {
        let valid = "450g pasta\n600 grams tomatoes\n3 onions\n6 tablespoons olive oil";
        assert!(passes("recipe-shopping", valid));
        assert!(!passes(
            "recipe-shopping",
            &valid.replace("600 grams", "600 cups")
        ));
        assert!(!passes(
            "recipe-shopping",
            &format!("{valid}\n2 cloves garlic")
        ));
        assert!(!passes(
            "recipe-shopping",
            &valid.replace("6 tablespoons", "6 teaspoons")
        ));
    }
    #[test]
    fn itinerary_accepts_alternatives_but_rejects_opening_and_walking_conflicts() {
        assert!(passes(
            "travel-itinerary",
            "09:30–10:15 Cafe\n10:30–11:15 Library\n12:00–12:45 Museum"
        ));
        assert!(passes(
            "travel-itinerary",
            "Cafe: 09:45-10:30\nLibrary: 10:45-11:30\nMuseum: 12:15-13:00"
        ));
        assert!(!passes(
            "travel-itinerary",
            "Cafe: 09:30-10:15\nLibrary: 10:20-11:05\nMuseum: 12:00-12:45"
        ));
        assert!(!passes(
            "travel-itinerary",
            "Cafe: 09:30-10:15\nMuseum: 10:30-11:15\nLibrary: 12:00-12:45"
        ));
        assert!(!passes(
            "travel-itinerary",
            "Cafe: 09:30-10:00\nLibrary: 10:30-11:15\nMuseum: 12:00-12:45"
        ));
    }
    #[test]
    fn packing_distinguishes_items_from_explicit_exclusions() {
        let base = "Waterproof jacket and warm layers, train ticket, laptop and power adapter.";
        assert!(passes("packing-checklist", base));
        assert!(passes(
            "packing-checklist",
            &format!("{base}\nDo not pack towels or a hair dryer; the hotel provides them.")
        ));
        assert!(!passes(
            "packing-checklist",
            &format!("{base}\n- towels\n- hair dryer")
        ));
        assert!(!passes(
            "packing-checklist",
            &format!("{base}\nDon't forget towels and a hair dryer.")
        ));
    }

    #[test]
    fn selection_translation_invitation_and_subscriptions_preserve_relationships() {
        assert!(passes("compare-laptops","Birch is the cheapest qualifying option at $849, $150 less than Cedar. Alder has only 8GB and does not qualify."));
        assert!(!passes(
            "compare-laptops",
            "Alder is the cheapest and best choice. Birch costs849 and Cedar costs150 more."
        ));
        assert!(passes("translation","Hola Maya, nuestro tren llega a las18:40. ¿Podríamos registrarnos después de las19:00? Código de reserva RX318."));
        assert!(!passes("translation","Hello Maya, our train arrives at18:40. Could we check in after19:00? Booking code RX318."));
        assert!(passes("reply-draft","Hi Maya,\nI'd love to join you on Saturday November7 at2pm. I will bring a vegetarian dish. Is parking available?\nBest,\nCasey".replace("November7","November 7").as_str()));
        assert!(!passes("reply-draft","Hi Casey, I will join November 7 at2pm and bring a vegetarian dish. Is parking available?\nBest,\nMaya"));
        assert!(passes("subscription-audit","Cancel Video and Fitness. Keep Music and Storage. Save $33 per month and $396 per year."));
        assert!(!passes("subscription-audit","Cancel Music and Storage. Keep Video and Fitness. Save $33 per month and $396 per year."));
        assert!(!passes("subscription-audit","Cancel Video and Fitness. Keep Music and Storage. Save $396 per month and $33 per year."));
        assert!(passes("meeting-actions","- Maya sends revised budget by Friday\n- Leo books venue by Tuesday\n- You approve final agenda by Thursday"));
        assert!(!passes("meeting-actions","- Leo sends revised budget by Friday\n- Maya books venue by Tuesday\n- You approve final agenda by Thursday"));
    }

    #[test]
    fn quoted_facts_and_offloading_do_not_masquerade_as_a_finished_answer() {
        assert!(general_checks("Maya sends the revised budget by Friday.")
            .iter()
            .all(|(_, passed)| *passed));
        assert!(!general_checks(
            "Journal status: CPU90%, battery70%. You should proceed to create a checklist."
        )
        .iter()
        .all(|(_, passed)| *passed));
        assert!(general_checks(
            "Pack a warm jacket, journal and pen, train ticket and laptop charger."
        )
        .iter()
        .all(|(_, passed)| *passed));
        assert!(!general_checks("Task journal: the reminder is completed.")
            .iter()
            .all(|(_, passed)| *passed));
        assert!(!general_checks("Journal: no pending approvals.")
            .iter()
            .all(|(_, passed)| *passed));
    }
}
