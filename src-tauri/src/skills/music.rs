//! Music: play a song, artist or playlist you name, pause, resume, skip, go back and say
//! what's playing, in Spotify or the Music app (whichever you allowed in Settings → Apps).
//! Spotify can't search from AppleScript, so Buntini opens its search and presses play on
//! the best match through the macOS accessibility API, then reports what Spotify is
//! actually playing.
use super::{from_verb, Context, Outcome};
use crate::privacy::Share;

#[derive(Debug, PartialEq)]
pub struct Command {
    /// now_playing, play, pause, next or previous.
    pub action: &'static str,
    /// What to play, in the user's own words.
    pub query: Option<String>,
    /// "spotify" or "music" when the user named one.
    pub app: Option<&'static str>,
}

/// `text` and `previous` come from `skills::words`.
pub fn recognise(text: &str, previous: Option<&str>) -> Option<Command> {
    let app = if text.contains("spotify") { Some("spotify") } else if text.contains("apple music") || text.contains("music app") { Some("music") } else { None };
    let command = |action: &'static str, query: Option<String>| Some(Command { action, query, app });
    if ["what's playing", "whats playing", "what is playing", "what song is this", "what's this song", "which song is this", "what song is playing", "what am i listening to"].iter().any(|phrase| text.contains(phrase)) {
        return command("now_playing", None);
    }
    if let Some((verb, rest)) = from_verb(text, &["play", "pause", "stop", "skip", "next", "previous", "resume", "unpause"]) {
        let rest_words = rest.split(' ').filter(|word| !word.is_empty()).count();
        return match verb {
            "pause" if rest_words <= 4 => command("pause", None),
            "stop" if rest_words <= 3 && (rest_words == 0 || ["music", "playing", "the", "it", "song", "spotify"].iter().any(|word| rest.contains(word))) => command("pause", None),
            "skip" if rest_words <= 3 => command("next", None),
            "next" if rest_words <= 2 && (rest_words == 0 || rest.contains("song") || rest.contains("track")) => command("next", None),
            "previous" if rest_words <= 2 => command("previous", None),
            "resume" | "unpause" if rest_words <= 3 => command("play", None),
            "play" => play(rest, previous, app),
            _ => None,
        };
    }
    // "yes", "go ahead", "please do" right after a play request plays that request.
    let agreed = !text.is_empty() && text.split(' ').count() <= 6 && text.split(' ').all(|word| ["yes", "yeah", "yep", "sure", "ok", "okay", "go", "ahead", "please", "do", "it", "that", "play", "this", "correct", "song", "right"].contains(&word));
    if agreed {
        if let Some(earlier) = previous.and_then(|before| recognise(before, None)).filter(|earlier| earlier.action == "play" && earlier.query.is_some()) {
            return Some(Command { app: app.or(earlier.app), ..earlier });
        }
    }
    None
}

fn play(rest: &str, previous: Option<&str>, app: Option<&'static str>) -> Option<Command> {
    let mut query = rest.to_string();
    for tail in [" for me", " please", " on spotify", " in spotify", " on apple music", " in apple music", " in the music app", " on the music app", " now", " again", " right now"] {
        while query.ends_with(tail) {
            query.truncate(query.len() - tail.len());
        }
    }
    for lead in ["the song ", "song ", "some ", "me "] {
        query = query.strip_prefix(lead).unwrap_or(&query).to_string();
    }
    let query = query.trim().to_string();
    if ["video", "game", "youtube", "movie", "film", "podcast episode"].iter().any(|thing| query.split(' ').any(|word| word.starts_with(thing))) {
        return None;
    }
    match query.as_str() {
        "it" | "that" | "this" => match previous.and_then(|before| recognise(before, None)) {
            Some(earlier) if earlier.query.is_some() => Some(Command { app: app.or(earlier.app), ..earlier }),
            _ => Some(Command { action: "play", query: None, app }),
        },
        "" | "music" | "spotify" | "something" | "the music" => Some(Command { action: "play", query: None, app }),
        _ => Some(Command { action: "play", query: Some(query), app }),
    }
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    let player = crate::apps::player(context.settings, context.prompt, command.app);
    let usable = crate::apps::app(player).is_some_and(|app| crate::apps::allowed(context.settings, player) && crate::apps::installed(app));
    if !usable {
        let name = if player == "spotify" { "Spotify" } else { "Music" };
        return Err(format!("{name} isn’t allowed yet. You can allow it in Settings → Apps."));
    }
    let answer = crate::apps::control_music(player, command.action, command.query.as_deref().unwrap_or("")).await?;
    Ok(Outcome::Done { answer, share: Some(Share::Music) })
}

/// Words in a label, ignoring case and one-letter fragments ("T's").
fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| word.len() > 1).map(str::to_string).collect()
}

/// How well a Spotify result ("Hey There Delilah by Plain White T's") matches the request:
/// the song title counts most, and covers, karaoke, live and other versions lose unless
/// asked for. Zero means no match.
pub fn score(label: &str, wanted: &str) -> i32 {
    let wanted = word_set(wanted);
    if wanted.is_empty() {
        return 0;
    }
    let title = label.split(" by ").next().unwrap_or(label);
    let shared = word_set(label).intersection(&wanted).count();
    if shared * 2 < wanted.len() {
        return 0;
    }
    let mut score = (word_set(title).intersection(&wanted).count() * 2 + shared) as i32;
    let lower = label.to_lowercase();
    for other in ["karaoke", "cover", "tribute", "instrumental", "originally performed", "made famous", "remix", "live", "acoustic", "sped up", "slowed", "lofi", "lo-fi", "8d", "version", "piano"] {
        if lower.contains(other) && !wanted.contains(other) {
            score -= 4;
        }
    }
    score.max(0)
}

/// Presses play on the result in Spotify's open search that best matches `wanted`, the way
/// a person would, and returns its label. Spotify must already be showing the search.
pub async fn press_play(wanted: &str) -> Result<String, String> {
    let wanted = wanted.to_string();
    tokio::task::spawn_blocking(move || {
        if !crate::ax::trusted(true) {
            return Err("To press play for you, allow Buntini under System Settings → Privacy & Security → Accessibility, then ask again.".to_string());
        }
        let pid = crate::ax::pid("com.spotify.client").ok_or("Spotify isn’t open.")?;
        // The results take a moment to appear; keep looking for a few seconds.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        std::thread::sleep(std::time::Duration::from_millis(700));
        while std::time::Instant::now() < deadline {
            let buttons = crate::ax::buttons(pid, "Play ");
            let best = buttons.iter().map(|(_, label)| score(label, &wanted)).max().unwrap_or(0);
            if best > 0 {
                if let Some((button, label)) = buttons.iter().find(|(_, label)| score(label, &wanted) == best) {
                    if button.press() {
                        return Ok(label.clone());
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        Err("I couldn’t find a matching song in Spotify’s results.".into())
    })
    .await
    .map_err(|_| "Spotify couldn’t be reached.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::words;

    fn ask(prompt: &str, previous: Option<&str>) -> Option<Command> {
        recognise(&words(prompt), previous.map(words).as_deref())
    }

    #[test]
    fn everyday_music_requests_are_recognised() {
        let play = |query: &str, app: Option<&'static str>| Some(Command { action: "play", query: Some(query.into()), app });
        assert_eq!(ask("Hey Buntini, can you play Hey There Delilah on Spotify?", None), play("hey there delilah", Some("spotify")));
        assert_eq!(ask("Hey Bantani, can you play Hey The Delilah?", None), play("hey the delilah", None));
        assert_eq!(ask("Could you please play the song Here Without You for me", None), play("here without you", None));
        assert_eq!(ask("play some Coldplay", None), play("coldplay", None));
        assert_eq!(ask("pause the music", None).unwrap().action, "pause");
        assert_eq!(ask("stop the music", None).unwrap().action, "pause");
        assert_eq!(ask("skip this song", None).unwrap().action, "next");
        assert_eq!(ask("next song please", None).unwrap().action, "next");
        assert_eq!(ask("resume", None).unwrap(), Command { action: "play", query: None, app: None });
        assert_eq!(ask("What's playing right now?", None).unwrap().action, "now_playing");
        assert_eq!(ask("Yes, please go ahead.", Some("Can you play Hey There Delilah?")), play("hey there delilah", None));
        assert_eq!(ask("play it", Some("play Delilah on Spotify")), play("delilah", Some("spotify")));
        assert_eq!(ask("Yes, please go ahead.", Some("What's on my calendar?")), None, "a go-ahead for something else goes to the model");
        assert_eq!(ask("play a video of cats", None), None);
        assert_eq!(ask("what's on my calendar tomorrow?", None), None);
        assert_eq!(ask("stop reminding me about the bins every single day", None), None);
        assert_eq!(ask("next week I have a meeting", None), None);
    }

    #[test]
    fn the_original_song_beats_covers() {
        let wanted = "hey there delilah";
        let original = score("Hey There Delilah by Plain White T's", wanted);
        assert!(original > score("Hey There Delilah (Karaoke Version) by Sing Along Hits", wanted));
        assert!(original > score("Hey There Delilah - Acoustic Cover by Some Band", wanted));
        assert!(original > score("Delilah by Florence + The Machine", wanted));
        assert_eq!(score("Bohemian Rhapsody by Queen", wanted), 0);
    }
}
