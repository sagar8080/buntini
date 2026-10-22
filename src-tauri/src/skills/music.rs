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
    /// A retry ("that's the wrong song"): skip whatever is playing now.
    pub retry: bool,
}

/// The latest play request with something named, among `recent` messages (newest first).
fn last_request(recent: &[String]) -> Option<Command> {
    recent.iter().find_map(|before| recognise(before, &[]).filter(|earlier| earlier.action == "play" && earlier.query.is_some() && !earlier.retry))
}

/// `text` and `recent` (the user's last few messages, newest first) come from
/// `skills::words`.
pub fn recognise(text: &str, recent: &[String]) -> Option<Command> {
    let app = if text.contains("spotify") { Some("spotify") } else if text.contains("apple music") || text.contains("music app") { Some("music") } else { None };
    let command = |action: &'static str, query: Option<String>| Some(Command { action, query, app, retry: false });
    // Follow-ups about the last song: "try again", "it didn't play", "that's the wrong
    // song", "play the original version".
    let complaint = ["try again", "try it again", "one more time", "didn't play", "didnt play", "not playing", "isn't playing", "wrong song", "wrong track", "wrong version", "not the right", "not the correct", "the correct song", "the right song", "original version", "the original", "not that one", "not this one", "that's not it", "different version"];
    if complaint.iter().any(|phrase| text.contains(phrase)) {
        if let Some(earlier) = last_request(recent) {
            let retry = !["try again", "try it again", "one more time"].iter().any(|phrase| text.contains(phrase)) || text.contains("wrong") || text.contains("original");
            return Some(Command { app: app.or(earlier.app), retry, ..earlier });
        }
    }
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
            "play" => play(rest, recent, app),
            _ => None,
        };
    }
    // "yes", "go ahead", "please do" right after a play request plays that request.
    let agreed = !text.is_empty() && text.split(' ').count() <= 6 && text.split(' ').all(|word| ["yes", "yeah", "yep", "sure", "ok", "okay", "go", "ahead", "please", "do", "it", "that", "play", "this", "correct", "song", "right"].contains(&word));
    if agreed {
        if let Some(earlier) = recent.first().and_then(|before| recognise(before, &[])).filter(|earlier| earlier.action == "play" && earlier.query.is_some()) {
            return Some(Command { app: app.or(earlier.app), ..earlier });
        }
    }
    None
}

fn play(rest: &str, recent: &[String], app: Option<&'static str>) -> Option<Command> {
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
        "it" | "that" | "this" | "it again" | "that again" => match last_request(recent) {
            Some(earlier) => Some(Command { app: app.or(earlier.app), ..earlier }),
            None => Some(Command { action: "play", query: None, app, retry: false }),
        },
        "" | "music" | "spotify" | "something" | "the music" => Some(Command { action: "play", query: None, app, retry: false }),
        _ => Some(Command { action: "play", query: Some(query), app, retry: false }),
    }
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    let player = crate::apps::player(context.settings, context.prompt, command.app);
    let usable = crate::apps::app(player).is_some_and(|app| crate::apps::allowed(context.settings, player) && crate::apps::installed(app));
    if !usable {
        let name = if player == "spotify" { "Spotify" } else { "Music" };
        return Err(format!("{name} isn’t allowed yet. You can allow it in Settings → Apps."));
    }
    let answer = crate::apps::control_music(player, command.action, command.query.as_deref().unwrap_or(""), command.retry).await?;
    Ok(Outcome::Done { answer, share: Some(Share::Music) })
}

/// Words in a label, ignoring case and one-letter fragments ("T's").
fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| word.len() > 1).map(str::to_string).collect()
}

/// How well a Spotify result ("Hey There Delilah by Plain White T's") matches the request.
/// Every requested word found counts, every unrequested word counts against (so a label
/// padded with lyrics or "Alternate Lyrics" loses), and covers, karaoke, live and other
/// versions lose unless asked for. Zero means no match.
pub fn score(label: &str, wanted: &str) -> i32 {
    let wanted = word_set(wanted);
    if wanted.is_empty() {
        return 0;
    }
    let found = word_set(label);
    let shared = found.intersection(&wanted).count();
    if shared * 2 < wanted.len() {
        return 0;
    }
    let extra = found.iter().filter(|word| !wanted.contains(*word) && word.as_str() != "by").count();
    let mut score = (shared * 4) as i32 - extra as i32;
    let lower = label.to_lowercase();
    for other in ["karaoke", "cover", "tribute", "instrumental", "originally performed", "made famous", "remix", "live", "acoustic", "sped up", "slowed", "lofi", "lo-fi", "8d", "version", "piano", "alternate", "lyrics", "edit", "demo", "remaster", "re-recorded", "extended", " mix"] {
        if lower.contains(other) && !wanted.iter().any(|word| other.trim().starts_with(word.as_str())) {
            score -= 6;
        }
    }
    score.max(1)
}

/// Presses play on the result in Spotify's open search that best matches `wanted`, the way
/// a person would, and returns its label. Spotify must already be showing the search.
pub async fn press_play(wanted: &str, avoid: Option<String>) -> Result<String, String> {
    let wanted = wanted.to_string();
    // A retry skips the song that's playing now (the wrong one).
    let avoided = move |label: &str| avoid.as_deref().is_some_and(|title| !title.is_empty() && label.to_lowercase().starts_with(&title.to_lowercase()));
    tokio::task::spawn_blocking(move || {
        if !crate::ax::trusted(true) {
            return Err("To press play for you, allow Buntini under System Settings → Privacy & Security → Accessibility, then ask again.".to_string());
        }
        let pid = crate::ax::pid("com.spotify.client").ok_or("Spotify isn’t open.")?;
        // The results take a moment to appear; keep looking for a few seconds.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        std::thread::sleep(std::time::Duration::from_millis(700));
        while std::time::Instant::now() < deadline {
            let buttons: Vec<_> = crate::ax::buttons(pid, "Play ").into_iter().filter(|(_, label)| !avoided(label)).collect();
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
        recognise(&words(prompt), &previous.map(words).into_iter().collect::<Vec<_>>())
    }

    #[test]
    fn everyday_music_requests_are_recognised() {
        let play = |query: &str, app: Option<&'static str>| Some(Command { action: "play", query: Some(query.into()), app, retry: false });
        assert_eq!(ask("Hey Buntini, can you play Hey There Delilah on Spotify?", None), play("hey there delilah", Some("spotify")));
        assert_eq!(ask("Hey Bantani, can you play Hey The Delilah?", None), play("hey the delilah", None));
        assert_eq!(ask("Could you please play the song Here Without You for me", None), play("here without you", None));
        assert_eq!(ask("play some Coldplay", None), play("coldplay", None));
        assert_eq!(ask("pause the music", None).unwrap().action, "pause");
        assert_eq!(ask("stop the music", None).unwrap().action, "pause");
        assert_eq!(ask("skip this song", None).unwrap().action, "next");
        assert_eq!(ask("next song please", None).unwrap().action, "next");
        assert_eq!(ask("resume", None).unwrap(), Command { action: "play", query: None, app: None, retry: false });
        assert_eq!(ask("What's playing right now?", None).unwrap().action, "now_playing");
        assert_eq!(ask("Yes, please go ahead.", Some("Can you play Hey There Delilah?")), play("hey there delilah", None));
        assert_eq!(ask("play it", Some("play Delilah on Spotify")), play("delilah", Some("spotify")));
        assert_eq!(ask("Yes, please go ahead.", Some("What's on my calendar?")), None, "a go-ahead for something else goes to the model");
        assert_eq!(ask("play a video of cats", None), None);
        assert_eq!(ask("what's on my calendar tomorrow?", None), None);
        assert_eq!(ask("stop reminding me about the bins every single day", None), None);
        assert_eq!(ask("next week I have a meeting", None), None);
        assert_eq!(ask("It still didn't play.", None), None, "a complaint with no song to retry goes to the model");
    }

    #[test]
    fn follow_ups_retry_the_last_song() {
        let recent = |messages: &[&str]| messages.iter().map(|message| words(message)).collect::<Vec<_>>();
        let retry = |text: &str, history: &[&str]| recognise(&words(text), &recent(history));
        let history = ["it still didn't play", "Can you play I Want It That Way by the Backstreet Boys?"];
        let wrong = retry("You're playing the alternate version. I want the original version.", &history[1..]).unwrap();
        assert_eq!((wrong.query.as_deref(), wrong.retry), (Some("i want it that way by the backstreet boys"), true));
        assert_eq!(retry("No, the correct song is not playing.", &history).unwrap().query.as_deref(), Some("i want it that way by the backstreet boys"), "looks past earlier complaints");
        let again = retry("Can you try it again?", &["Hey Bunchni, can you play Hey There Delilah by Plain White Ts?"]).unwrap();
        assert_eq!((again.query.as_deref(), again.retry), (Some("hey there delilah by plain white ts"), false));
    }

    #[test]
    fn the_original_song_beats_covers() {
        let wanted = "hey there delilah";
        let original = score("Hey There Delilah by Plain White T's", wanted);
        assert!(original > score("Hey There Delilah (Karaoke Version) by Sing Along Hits", wanted));
        assert!(original > score("Hey There Delilah - Acoustic Cover by Some Band", wanted));
        assert!(original > score("Delilah by Florence + The Machine", wanted));
        assert_eq!(score("Bohemian Rhapsody by Queen", wanted), 0);
        let wanted = "i want it that way by the backstreet boys";
        assert!(score("I Want It That Way by Backstreet Boys", wanted) > score("I Want It That Way - Alternate Lyrics - Backstreet Boys You are my fire The one desire Believe when", wanted));
    }
}
