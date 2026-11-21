//! Send to…: an answer in the island, opened in another app's own compose window with the text
//! filled in, where the person picks who it goes to and presses send. Nothing is sent from
//! here. WhatsApp when it's on this Mac; X, Threads and Bluesky in the browser, through their
//! published compose links.

/// Where an answer can go, in the order the menu lists them.
pub fn places() -> Vec<String> {
    let whatsapp = crate::actions::find_app("WhatsApp").is_some();
    whatsapp.then_some("whatsapp").into_iter().chain(["x", "threads", "bluesky"]).map(str::to_string).collect()
}

/// The longest text a compose link carries.
const LONGEST: usize = 4_000;

/// The compose link for `text` in `place`.
pub fn link(place: &str, text: &str) -> Option<String> {
    let text: String = text.trim().chars().take(LONGEST).collect();
    if text.is_empty() {
        return None;
    }
    let text = encode(&text);
    Some(match place {
        "whatsapp" => format!("whatsapp://send?text={text}"),
        "x" => format!("https://x.com/intent/post?text={text}"),
        "threads" => format!("https://www.threads.com/intent/post?text={text}"),
        "bluesky" => format!("https://bsky.app/intent/compose?text={text}"),
        _ => return None,
    })
}

/// Every byte but letters, digits and `-._~` as `%XX`, so spaces stay spaces in any app.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_place_gets_its_own_compose_link_with_the_text_encoded() {
        let text = "Lunch at 1? Café & tea";
        assert_eq!(link("x", text).as_deref(), Some("https://x.com/intent/post?text=Lunch%20at%201%3F%20Caf%C3%A9%20%26%20tea"));
        assert_eq!(link("whatsapp", "hi").as_deref(), Some("whatsapp://send?text=hi"));
        assert!(link("threads", "hi").unwrap().starts_with("https://www.threads.com/intent/post?text="));
        assert!(link("bluesky", "hi").unwrap().starts_with("https://bsky.app/intent/compose?text="));
        assert_eq!(link("fax", "hi"), None);
        assert_eq!(link("x", "  "), None);
        // Line breaks and a link inside stay in the text, not in the address.
        assert!(link("x", "a\nb https://example.com/?q=1&r=2").unwrap().ends_with("a%0Ab%20https%3A%2F%2Fexample.com%2F%3Fq%3D1%26r%3D2"));
    }

    #[test]
    fn the_browser_places_are_always_there() {
        let places = places();
        assert!(places.ends_with(&["x".to_string(), "threads".to_string(), "bluesky".to_string()]));
    }
}
