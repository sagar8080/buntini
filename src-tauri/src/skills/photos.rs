//! Photos: "show me photos from last weekend", "my favourite photos", "screenshots from
//! today", "photos from my Goa trip album", "selfies from March". The Apple helper reads
//! the Photos library on this Mac (PhotoKit) and saves small thumbnails in the cache; the
//! reply shows them, newest first, and opens one in Photos when clicked. Nothing is
//! downloaded from iCloud for this, and no model sees the pictures.
use super::{Context, Outcome};
use crate::{privacy::Share, types::*};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, PartialEq)]
pub struct Command {
    /// The first day, and the day after the last, in local time.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// "from last weekend", for the reply.
    pub when: String,
    pub album: Option<String>,
    pub favorites: bool,
    /// photo, video, screenshot or selfie; any kind when none.
    pub kind: Option<&'static str>,
}

const NOUNS: [(&str, Option<&str>); 9] = [
    ("photos", None),
    ("photo", None),
    ("pictures", None),
    ("pics", None),
    ("screenshots", Some("screenshot")),
    ("selfies", Some("selfie")),
    ("videos", Some("video")),
    ("favourites", None),
    ("favorites", None),
];
const VERBS: [&str; 9] = ["show", "find", "open", "display", "pull", "get", "see", "view", "any"];
/// Things to do to photos that aren't looking at them.
const OTHER: [&str; 12] = [
    "delete", "remove", "edit", "share", "send", "upload", "print", "export", "move", "rename", "crop", "email",
];
const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december",
];

/// `text` comes from `skills::words`.
pub fn recognise(text: &str) -> Option<Command> {
    recognise_on(text, Local::now().date_naive())
}

fn recognise_on(text: &str, today: NaiveDate) -> Option<Command> {
    let words: Vec<&str> = text.split(' ').collect();
    // A kind of photo, or an album by name ("open the album called Summer").
    let noun = words
        .iter()
        .position(|word| NOUNS.iter().any(|(noun, _)| noun == word))
        .or_else(|| words.iter().position(|word| *word == "album"))?;
    if words.iter().any(|word| OTHER.contains(word)) {
        return None;
    }
    // "show me photos …", "my photos from …", "photos from …", "any selfies from …".
    let asks = noun == 0
        || VERBS.contains(&words[0])
        || (words[0] == "my" && noun <= 3)
        || (["what", "which"].contains(&words[0]) && words.get(1) == Some(&"photos"));
    // "videos" alone is too often YouTube.
    if !asks || (words[noun] == "videos" && !words.contains(&"my") && !words.contains(&"i")) {
        return None;
    }
    let kind = NOUNS.iter().find(|(name, _)| *name == words[noun]).and_then(|(_, kind)| *kind);
    let favorites = words.iter().any(|word| word.starts_with("favourite") || word.starts_with("favorite"));
    let (from, to, when) = period(&words, today).unwrap_or((None, None, String::new()));
    Some(Command { from, to, when, album: album(&words), favorites, kind })
}

/// "photos from my goa trip album", "the album called goa".
fn album(words: &[&str]) -> Option<String> {
    let at = words.iter().position(|word| *word == "album")?;
    let name = if matches!(words.get(at + 1), Some(&"called") | Some(&"named")) {
        words[at + 2..].iter().take_while(|word| !["from", "in", "on"].contains(*word)).copied().collect::<Vec<_>>()
    } else {
        let start = words[..at].iter().rposition(|word| ["my", "the", "from", "in"].contains(word))? + 1;
        words[start..at].to_vec()
    };
    let name = name.join(" ");
    (!name.is_empty() && name.chars().count() <= 100).then_some(name)
}

type Period = (Option<NaiveDate>, Option<NaiveDate>, String);

fn month_range(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let end = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(year, month + 1, 1)? };
    Some((start, end))
}

fn period(words: &[&str], today: NaiveDate) -> Option<Period> {
    let text = words.join(" ");
    let has = |phrase: &str| format!(" {text} ").contains(&format!(" {phrase} "));
    let tomorrow = today + Duration::days(1);
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    // The weekend going on now, or the one just past.
    let saturday = match today.weekday() {
        Weekday::Sat => today,
        Weekday::Sun => today - Duration::days(1),
        _ => monday - Duration::days(2),
    };
    let span = |from: NaiveDate, to: NaiveDate, when: &str| Some((Some(from), Some(to), when.to_string()));
    if has("today") {
        return span(today, tomorrow, "from today");
    }
    if has("yesterday") {
        return span(today - Duration::days(1), today, "from yesterday");
    }
    if has("last weekend") {
        let start = if today.weekday() == Weekday::Sat || today.weekday() == Weekday::Sun { saturday - Duration::days(7) } else { saturday };
        return span(start, start + Duration::days(2), "from last weekend");
    }
    if has("this weekend") || has("the weekend") {
        return span(saturday, saturday + Duration::days(2), "from this weekend");
    }
    if has("this week") {
        return span(monday, tomorrow, "from this week");
    }
    if has("last week") {
        return span(monday - Duration::days(7), monday, "from last week");
    }
    if has("this month") {
        return span(today.with_day(1)?, tomorrow, "from this month");
    }
    if has("last month") {
        let (year, month) = if today.month() == 1 { (today.year() - 1, 12) } else { (today.year(), today.month() - 1) };
        let (start, end) = month_range(year, month)?;
        return span(start, end, "from last month");
    }
    if has("this year") {
        return span(NaiveDate::from_ymd_opt(today.year(), 1, 1)?, tomorrow, "from this year");
    }
    if has("last year") {
        let year = today.year() - 1;
        return span(NaiveDate::from_ymd_opt(year, 1, 1)?, NaiveDate::from_ymd_opt(year + 1, 1, 1)?, &format!("from {year}"));
    }
    if let Some(at) = words.iter().position(|word| *word == "last") {
        if let (Some(count), Some(&"days")) = (words.get(at + 1).and_then(|n| n.parse::<i64>().ok()), words.get(at + 2)) {
            if (1..=366).contains(&count) {
                return span(today - Duration::days(count - 1), tomorrow, &format!("from the last {count} days"));
            }
        }
    }
    let year_after = |at: usize| words.get(at + 1).and_then(|word| word.parse::<i32>().ok()).filter(|year| (1900..=2100).contains(year));
    if let Some((at, month)) = words.iter().enumerate().find_map(|(at, word)| MONTHS.iter().position(|month| month == word).map(|month| (at, month as u32 + 1))) {
        // "may" is also a verb: only a month after "in", "from" or "of", or before a year.
        if month == 5 && !(at > 0 && ["in", "from", "of"].contains(&words[at - 1])) && year_after(at).is_none() {
            return None;
        }
        let year = year_after(at).unwrap_or(if month > today.month() { today.year() - 1 } else { today.year() });
        let (start, end) = month_range(year, month)?;
        let name = MONTHS[month as usize - 1];
        let name = name[..1].to_uppercase() + &name[1..];
        return span(start, end, &format!("from {name} {year}"));
    }
    words.iter().enumerate().find_map(|(at, word)| {
        let year = word.parse::<i32>().ok().filter(|year| (1900..=2100).contains(year))?;
        (at > 0 && ["in", "from"].contains(&words[at - 1])).then_some(())?;
        span(NaiveDate::from_ymd_opt(year, 1, 1)?, NaiveDate::from_ymd_opt(year + 1, 1, 1)?, &format!("from {year}"))
    })
}

static THUMBS: OnceLock<PathBuf> = OnceLock::new();
/// Thumbnails kept; the oldest go as new ones are saved.
const THUMBS_KEPT: usize = 240;

/// Where thumbnails go, set once at startup.
pub fn set_thumbs(folder: &Path) {
    let _ = THUMBS.set(folder.to_path_buf());
}

fn ready_folder(folder: &Path) -> Option<()> {
    std::fs::create_dir_all(folder).ok()?;
    let mut saved = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect::<Vec<_>>();
    saved.sort();
    let keep = THUMBS_KEPT - LIMIT;
    for (_, old) in saved.iter().rev().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
    Some(())
}

/// The most photos one reply shows; the helper's limit.
pub const LIMIT: usize = 12;

fn thumb_name(name: &str) -> bool {
    name.len() == 36 && name.ends_with(".jpg") && name[..32].bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// A saved thumbnail as a data URL, for the chat to show.
#[tauri::command]
pub fn photo_thumbnail(id: String) -> Result<String, String> {
    thumbnail_in(THUMBS.get().ok_or("That photo isn’t here.")?, &id)
}

fn thumbnail_in(folder: &Path, id: &str) -> Result<String, String> {
    if !thumb_name(id) {
        return Err("That photo isn’t here.".into());
    }
    let bytes = std::fs::read(folder.join(id)).map_err(|_| "That photo isn’t here any more.".to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
}

/// A Photos library identifier ("3B5A…/L0/001"), never anything a script could misread.
fn photo_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '/')
}

const SHOW: &str = r#"on run argv
  tell application "Photos"
    activate
    spotlight media item id (item 1 of argv)
  end tell
end run"#;

/// Opens Photos on one of the photos a reply showed.
#[tauri::command]
pub async fn show_photo(id: String) -> Result<(), String> {
    if !photo_id(&id) {
        return Err("That isn’t a photo Buntini showed.".into());
    }
    super::osascript(SHOW, &[&id], 15).await.map(|_| ()).map_err(|error| {
        if error.contains("-1743") {
            "macOS hasn’t allowed Buntini to open Photos. Allow it in System Settings → Privacy & Security → Automation.".into()
        } else {
            error
        }
    })
}

fn summary(command: &Command, total: u64, shown: usize) -> String {
    let what = match (command.kind, command.favorites) {
        (Some("screenshot"), _) => "screenshots",
        (Some("selfie"), _) => "selfies",
        (Some("video"), _) => "videos",
        (_, true) => "favourite photos",
        _ => "photos",
    };
    let mut place = String::new();
    if let Some(album) = &command.album {
        place = format!(" in “{album}”");
    }
    let when = if command.when.is_empty() { String::new() } else { format!(" {}", command.when) };
    match total {
        0 => format!("No {what}{place}{when}."),
        1 => format!("One {}{place}{when}.", what.trim_end_matches('s')),
        total if total as usize == shown => format!("{total} {what}{place}{when}."),
        total => format!("The newest {shown} of {total} {what}{place}{when}."),
    }
}

pub(crate) fn card(command: &Command, answer: &Value) -> Result<(String, Option<ResultCard>), String> {
    match answer["error"].as_str() {
        Some("denied") => return Err("macOS hasn’t allowed Buntini to see your photos. Allow it in System Settings → Privacy & Security → Photos.".into()),
        Some("no_album") => return Err(format!("There’s no album called “{}” in Photos.", command.album.as_deref().unwrap_or_default())),
        Some(_) => return Err("Photos didn’t answer. Please try again.".into()),
        None => {}
    }
    let photos: Vec<CardPhoto> = answer["photos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|photo| {
            let id = photo["id"].as_str().filter(|id| photo_id(id))?;
            Some(CardPhoto {
                id: id.into(),
                thumb: photo["thumb"].as_str().filter(|name| thumb_name(name)).unwrap_or_default().into(),
                taken: photo["taken"].as_str().unwrap_or_default().into(),
                video: photo["video"] == true,
            })
        })
        .take(LIMIT)
        .collect();
    let total = answer["total"].as_u64().unwrap_or(photos.len() as u64);
    let text = summary(command, total, photos.len());
    if photos.is_empty() {
        return Ok((text, None));
    }
    let mut shown = crate::integrations::card("photos", text.trim_end_matches('.'), "", vec![], None, "Photos");
    shown.photos = photos;
    Ok((text, Some(shown)))
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    if !crate::apps::allowed(context.settings, "photos") {
        return Err("Photos isn’t allowed yet. You can allow it in Settings → Apps.".into());
    }
    if !context.helper.is_file() {
        return Err("Finding photos needs Buntini’s Apple helper, which isn’t on this Mac.".into());
    }
    let folder = THUMBS.get().ok_or("Photos isn’t ready yet.")?;
    ready_folder(folder).ok_or("Buntini couldn’t make room for the thumbnails.")?;
    let day = |date: Option<NaiveDate>| date.map(|date| date.format("%Y-%m-%d").to_string());
    let query = json!({
        "from": day(command.from), "to": day(command.to), "album": command.album,
        "favorites": command.favorites, "kind": command.kind, "thumbs": folder,
    });
    let mut helper = tokio::process::Command::new(context.helper);
    helper.args(["--find-photos", &query.to_string()]);
    // The first use waits for the macOS permission prompt, so allow time to answer it.
    let output = crate::actions::output(helper, 90).await?;
    let answer: Value = serde_json::from_str(output.trim()).map_err(|_| "Photos returned an unreadable answer.".to_string())?;
    let (answer, card) = card(&command, &answer)?;
    Ok(Outcome::Shown { answer, shares: vec![Share::Photos], cards: card.into_iter().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(text: &str) -> Option<Command> {
        // A Wednesday.
        recognise_on(&crate::skills::words(text), NaiveDate::from_ymd_opt(2026, 11, 4).unwrap())
    }
    fn days(command: &Command) -> (String, String) {
        (command.from.unwrap().to_string(), command.to.unwrap().to_string())
    }

    #[test]
    fn asking_to_see_photos_is_recognised_with_its_dates() {
        let weekend = ask("Show me photos from last weekend").unwrap();
        assert_eq!(days(&weekend), ("2026-10-31".into(), "2026-11-02".into()));
        assert_eq!(weekend.when, "from last weekend");
        assert_eq!(days(&ask("my photos from today").unwrap()), ("2026-11-04".into(), "2026-11-05".into()));
        assert_eq!(days(&ask("photos from yesterday").unwrap()), ("2026-11-03".into(), "2026-11-04".into()));
        assert_eq!(days(&ask("show pictures from last week").unwrap()), ("2026-10-26".into(), "2026-11-02".into()));
        assert_eq!(days(&ask("find photos from last month").unwrap()), ("2026-10-01".into(), "2026-11-01".into()));
        let march = ask("show selfies from March").unwrap();
        assert_eq!((days(&march), march.kind, march.when.as_str()), (("2026-03-01".into(), "2026-04-01".into()), Some("selfie"), "from March 2026"));
        assert_eq!(days(&ask("photos from December").unwrap()).0, "2025-12-01", "a month still to come means last year's");
        assert_eq!(days(&ask("show photos from 2024").unwrap()), ("2024-01-01".into(), "2025-01-01".into()));
        assert_eq!(days(&ask("photos from the last 3 days").unwrap()), ("2026-11-02".into(), "2026-11-05".into()));
        assert_eq!(ask("screenshots from today").unwrap().kind, Some("screenshot"));
        assert!(ask("show my favourite photos").unwrap().favorites);
        let album = ask("show photos from my Goa trip album").unwrap();
        assert_eq!((album.album.as_deref(), album.from), (Some("goa trip"), None));
        assert_eq!(ask("open the album called summer 2025").unwrap().album.as_deref(), Some("summer 2025"));
        assert_eq!(ask("show my photos").unwrap(), Command { from: None, to: None, when: String::new(), album: None, favorites: false, kind: None });
    }

    #[test]
    fn other_things_about_photos_go_to_the_model() {
        for text in ["delete the photos from yesterday", "send Ana the photos", "edit this photo", "how do I take a screenshot", "play videos on youtube", "show me how to crop photos", "the photo you sent"] {
            assert_eq!(ask(text), None, "{text}");
        }
    }

    #[test]
    fn photos_read_as_a_card_of_thumbnails() {
        let command = ask("show me photos from last weekend").unwrap();
        let thumb = format!("{}.jpg", "a".repeat(32));
        let answer = json!({"total":30,"photos":[
            {"id":"3B5A-11/L0/001","taken":"2026-11-01T10:00:00Z","video":false,"favorite":false,"thumb":thumb},
            {"id":"bad id; do shell script","taken":"","video":false,"thumb":""},
            {"id":"9C2D/L0/001","taken":"2026-10-31T18:00:00Z","video":true,"thumb":"../secret.jpg"}
        ]});
        let (text, shown) = card(&command, &answer).unwrap();
        let shown = shown.unwrap();
        assert_eq!(text, "The newest 2 of 30 photos from last weekend.");
        assert_eq!(shown.photos.iter().map(|photo| photo.id.as_str()).collect::<Vec<_>>(), ["3B5A-11/L0/001", "9C2D/L0/001"]);
        assert_eq!(shown.photos[1].thumb, "", "a thumbnail outside the cache is dropped");
        assert!(shown.photos[1].video);
        assert_eq!(card(&command, &json!({"total":0,"photos":[]})).unwrap(), ("No photos from last weekend.".into(), None));
        assert!(card(&command, &json!({"error":"denied"})).unwrap_err().contains("Privacy & Security → Photos"));
        let album = ask("show photos from my Goa album").unwrap();
        assert_eq!(card(&album, &json!({"error":"no_album"})).unwrap_err(), "There’s no album called “goa” in Photos.");
        assert_eq!(summary(&ask("my favourite photos").unwrap(), 1, 1), "One favourite photo.");
    }

    #[test]
    fn thumbnails_are_read_back_only_from_the_cache_by_name() {
        let folder = tempfile::tempdir().unwrap();
        let name = format!("{}.jpg", "b".repeat(32));
        std::fs::write(folder.path().join(&name), b"jpg").unwrap();
        assert_eq!(thumbnail_in(folder.path(), &name).unwrap(), "data:image/jpeg;base64,anBn");
        for bad in ["../x.jpg", "", &name.to_uppercase(), "b.jpg"] {
            assert!(thumbnail_in(folder.path(), bad).is_err(), "{bad}");
        }
        assert!(photo_id("3B5A-11/L0/001") && !photo_id("x\" & do shell script \"") && !photo_id(""));
    }
}
