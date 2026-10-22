//! Apps on this Mac that Buntini can work with, each only after the user allows it in
//! Settings → Apps. No accounts and nothing online: Buntini talks to the apps already
//! here, through AppleScript (values always passed as arguments, never as script text)
//! and, for Calendar and Reminders, EventKit in the `buntini-fm` helper so repeating
//! events are included. macOS still asks once per app before the first use.
//!
//! Reading (email, the day's events, open reminders, notes, what's playing) and playback
//! controls run straight away for an allowed app; anything that adds or writes goes
//! through the approval sheet in `actions.rs`.
use crate::{privacy::Share, types::Settings};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    /// What Buntini can do with it, in plain words for Settings.
    pub can: &'static [&'static str],
    /// The model tools this app unlocks (reads here, writes in `actions.rs`).
    pub tools: &'static [&'static str],
    /// The kind of data it reads, for what external models may see.
    pub share: Option<Share>,
}

pub const APPS: [App; 7] = [
    App { id: "mail", name: "Mail", can: &["Read and search your recent email", "Draft replies for you to send"], tools: &["read_mail", "draft_email"], share: Some(Share::Mail) },
    App { id: "calendar", name: "Calendar", can: &["See what’s on any day", "Add events you approve"], tools: &["read_calendar", "add_calendar_event"], share: Some(Share::Calendar) },
    App { id: "reminders", name: "Reminders", can: &["See your open reminders", "Add reminders you approve"], tools: &["read_reminders", "add_reminder"], share: Some(Share::Calendar) },
    App { id: "notes", name: "Notes", can: &["Find your notes", "Write notes you approve"], tools: &["find_notes", "create_note"], share: Some(Share::Notes) },
    App { id: "music", name: "Music", can: &["See what’s playing", "Play, pause and skip", "Play songs and playlists from your library"], tools: &["control_music"], share: Some(Share::Music) },
    App { id: "spotify", name: "Spotify", can: &["See what’s playing", "Play, pause and skip", "Open a search in Spotify"], tools: &["control_music"], share: Some(Share::Music) },
    App { id: "shortcuts", name: "Shortcuts", can: &["Run your shortcuts after you approve"], tools: &["run_shortcut"], share: None },
];

/// Tools that read or control an app directly, without an approval sheet.
pub const READ_TOOLS: [&str; 5] = ["read_mail", "read_calendar", "read_reminders", "find_notes", "control_music"];

pub fn app(id: &str) -> Option<&'static App> {
    APPS.iter().find(|app| app.id == id)
}

pub fn installed(app: &App) -> bool {
    crate::actions::find_app(app.name).is_some()
}

pub fn allowed(settings: &Settings, id: &str) -> bool {
    settings.apps.iter().any(|allowed| allowed == id)
}

/// The app a call uses: music controls name theirs; every other tool belongs to one app.
pub fn for_call(tool: &str, args: &Map<String, Value>) -> Option<&'static App> {
    if tool == "control_music" {
        return app(args.get("app").and_then(Value::as_str).unwrap_or("music"));
    }
    APPS.iter().find(|app| app.tools.contains(&tool))
}

/// Whether the user allowed the app behind this call, and it is on this Mac.
pub fn permits(settings: &Settings, tool: &str, args: &Map<String, Value>) -> bool {
    match tool {
        "open_app" | "open_link" => true,
        _ => for_call(tool, args).is_some_and(|app| allowed(settings, app.id) && installed(app)),
    }
}

/// Allowed, installed apps whose tools a model may be offered, with the data each reads.
pub fn usable(settings: &Settings) -> Vec<&'static App> {
    APPS.iter().filter(|app| allowed(settings, app.id) && installed(app)).collect()
}

/// One line for the model: which apps it can use, and which the user could still allow.
pub fn prompt_line(settings: &Settings) -> String {
    let (yes, no): (Vec<&App>, Vec<&App>) = APPS.iter().filter(|app| installed(app)).partition(|app| allowed(settings, app.id));
    let mut line = String::new();
    if !yes.is_empty() {
        line += &format!(" Apps you may use for the user: {}.", yes.iter().map(|app| app.name).collect::<Vec<_>>().join(", "));
    }
    if !no.is_empty() {
        line += &format!(" Not allowed yet: {}. If they ask for one of those, say they can allow it in Settings → Apps.", no.iter().map(|app| app.name).collect::<Vec<_>>().join(", "));
    }
    line
}

fn text(args: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) if value.trim().chars().count() <= max && !value.chars().any(char::is_control) => Ok(Some(value.trim().to_string())),
        _ => Err(format!("{key} must be text of up to {max} characters.")),
    }
}

/// Checks and normalises a read or playback call. Unknown keys are refused.
pub fn validate(tool: &str, args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    let allowed: &[&str] = match tool {
        "read_mail" => &["query", "unread", "limit"],
        "read_calendar" => &["day"],
        "read_reminders" => &[],
        "find_notes" => &["query"],
        "control_music" => &["app", "action", "query"],
        _ => return Err(format!("“{tool}” isn’t an app Buntini can use.")),
    };
    if let Some(extra) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{tool} doesn’t take “{extra}”."));
    }
    let mut out = Map::new();
    match tool {
        "read_mail" => {
            if let Some(query) = text(args, "query", 100)? {
                out.insert("query".into(), query.into());
            }
            out.insert("unread".into(), json!(args.get("unread").and_then(Value::as_bool).unwrap_or(false)));
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5).clamp(1, 8);
            out.insert("limit".into(), json!(limit));
        }
        "read_calendar" => {
            let today = chrono::Local::now().date_naive();
            let day = match text(args, "day", 10)? {
                Some(day) => chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").map_err(|_| "Use a day like 2026-10-23.")?,
                None => today,
            };
            if (day - today).num_days().abs() > 366 {
                return Err("Pick a day within a year of today.".into());
            }
            out.insert("day".into(), day.format("%Y-%m-%d").to_string().into());
        }
        "find_notes" => {
            out.insert("query".into(), text(args, "query", 100)?.ok_or("Say what to look for in your notes.")?.into());
        }
        "control_music" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or("music");
            if !["music", "spotify"].contains(&app) {
                return Err("Music controls work with Music or Spotify.".into());
            }
            let action = args.get("action").and_then(Value::as_str).ok_or("Say what to do: now_playing, play, pause, next or previous.")?;
            if !["now_playing", "play", "pause", "next", "previous"].contains(&action) {
                return Err("Music controls are now_playing, play, pause, next and previous.".into());
            }
            out.insert("app".into(), app.into());
            out.insert("action".into(), action.into());
            if action == "play" {
                if let Some(query) = text(args, "query", 100)? {
                    out.insert("query".into(), query.into());
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

// Shared AppleScript helpers: a sortable timestamp, and text with tabs and line breaks
// flattened so each result stays on one tab-separated line.
const HELPERS: &str = r#"on pad(n)
  return text -2 thru -1 of ("0" & n)
end pad
on stamp(d)
  return (year of d as text) & "-" & my pad((month of d) as integer) & "-" & my pad(day of d) & " " & my pad(hours of d) & ":" & my pad(minutes of d)
end stamp
on clean(t)
  if t is missing value then return ""
  set AppleScript's text item delimiters to {tab, return, linefeed}
  set parts to text items of (t as text)
  set AppleScript's text item delimiters to " "
  set t to parts as text
  set AppleScript's text item delimiters to ""
  return t
end clean
"#;
// Inbox messages from the last N days (optionally unread, or matching sender/subject):
// time, message id, read, sender, subject. Mail filters natively, so this stays quick.
const MAIL_LIST: &str = r#"on run argv
  set q to item 1 of argv
  set cutoff to (current date) - ((item 3 of argv) as integer) * days
  set out to {}
  tell application "Mail"
    if q is not "" then
      set found to (messages of inbox whose date received > cutoff and (subject contains q or sender contains q))
    else if item 2 of argv is "1" then
      set found to (messages of inbox whose date received > cutoff and read status is false)
    else
      set found to (messages of inbox whose date received > cutoff)
    end if
    repeat with m in found
      set end of out to my stamp(date received of m) & tab & my clean(message id of m) & tab & (read status of m as text) & tab & my clean(sender of m) & tab & my clean(subject of m)
      if (count of out) ≥ 300 then exit repeat
    end repeat
  end tell
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// The opening of each message, by message id.
const MAIL_TEXT: &str = r#"on run argv
  set out to {}
  tell application "Mail"
    repeat with mid in argv
      try
        set c to content of (first message of inbox whose message id is (mid as text))
        if (length of c) > 500 then set c to text 1 thru 500 of c
        set end of out to (mid as text) & tab & my clean(c)
      end try
    end repeat
  end tell
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
const NOTES_FIND: &str = r#"on run argv
  set q to item 1 of argv
  set out to {}
  tell application "Notes"
    repeat with n in (notes whose name contains q or plaintext contains q)
      set t to plaintext of n
      if (length of t) > 400 then set t to text 1 thru 400 of t
      set end of out to my stamp(modification date of n) & tab & my clean(name of n) & tab & my clean(t)
      if (count of out) ≥ 6 then exit repeat
    end repeat
  end tell
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// argv: action, query. Asking what's playing never launches the app.
const MUSIC: &str = r#"on run argv
  set act to item 1 of argv
  set q to item 2 of argv
  if act is "now_playing" and application "Music" is not running then return "closed"
  tell application "Music"
    if act is "now_playing" then
      if player state is stopped then return "stopped"
      return (player state as text) & tab & my clean(name of current track) & tab & my clean(artist of current track) & tab & my clean(album of current track)
    else if act is "pause" then
      pause
    else if act is "next" then
      next track
    else if act is "previous" then
      previous track
    else if q is "" then
      play
    else
      try
        play playlist q
        return "playlist" & tab & q
      end try
      set found to (search library playlist 1 for q)
      if found is {} then return "notfound"
      play item 1 of found
      return "track" & tab & my clean(name of item 1 of found) & tab & my clean(artist of item 1 of found)
    end if
  end tell
  return "ok"
end run
"#;
// argv: action, query (already a spotify:search: address). Spotify can't search from
// AppleScript, so a request to play something opens its search for the user to pick.
const SPOTIFY: &str = r#"on run argv
  set act to item 1 of argv
  set q to item 2 of argv
  if act is "now_playing" and application "Spotify" is not running then return "closed"
  tell application "Spotify"
    if act is "now_playing" then
      if player state is stopped then return "stopped"
      return (player state as text) & tab & my clean(name of current track) & tab & my clean(artist of current track) & tab & my clean(album of current track)
    else if act is "pause" then
      pause
    else if act is "next" then
      next track
    else if act is "previous" then
      previous track
    else if q is "" then
      play
    else
      open location q
      activate
      return "search"
    end if
  end tell
  return "ok"
end run
"#;

async fn osascript(source: &str, argv: &[String], seconds: u64) -> Result<String, String> {
    let mut command = Command::new("/usr/bin/osascript");
    command.arg("-e").arg(format!("{HELPERS}{source}")).args(argv);
    crate::actions::output(command, seconds).await
}

fn rows(output: &str) -> Vec<Vec<String>> {
    output.lines().filter(|line| !line.trim().is_empty()).map(|line| line.split('\t').map(|part| part.trim().to_string()).collect()).collect()
}

fn percent(text: &str) -> String {
    text.bytes().map(|byte| if byte.is_ascii_alphanumeric() { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

/// "2026-10-22 09:15" as "Thu 22 Oct 09:15".
fn when(stamp: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M").map(|at| at.format("%a %-d %b %H:%M").to_string()).unwrap_or_else(|_| stamp.to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub title: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub title: String,
    pub due: String,
    pub list: String,
}

async fn helper(helper: &Path, args: &[&str]) -> Result<Vec<Value>, String> {
    if !helper.is_file() {
        return Err("Reading Calendar and Reminders needs Buntini’s Apple helper, which isn’t on this Mac.".into());
    }
    let mut command = Command::new(helper);
    command.args(args);
    // The first use waits for the macOS permission prompt, so allow time to answer it.
    let output = crate::actions::output(command, 90).await.map_err(|error| {
        if error.contains("not-authorized") {
            "macOS hasn’t allowed Buntini to read that. Allow it in System Settings → Privacy & Security → Calendars or Reminders.".to_string()
        } else {
            error
        }
    })?;
    Ok(output.lines().filter_map(|line| serde_json::from_str(line).ok()).collect())
}

fn field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The day's events (YYYY-MM-DD), repeating ones included, in start order.
pub async fn events(helper_path: &Path, day: &str) -> Result<Vec<Event>, String> {
    Ok(helper(helper_path, &["--events", day]).await?.iter().map(|event| Event { start: field(event, "start"), end: field(event, "end"), all_day: event.get("allDay").and_then(Value::as_bool).unwrap_or(false), title: field(event, "title"), location: field(event, "location") }).collect())
}

/// Open reminders, soonest due first; undated ones last.
pub async fn reminders(helper_path: &Path) -> Result<Vec<Reminder>, String> {
    Ok(helper(helper_path, &["--reminders"]).await?.iter().map(|reminder| Reminder { title: field(reminder, "title"), due: field(reminder, "due"), list: field(reminder, "list") }).collect())
}

pub fn describe_events(day: &str, events: &[Event]) -> String {
    let label = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map(|date| date.format("%A %-d %B").to_string()).unwrap_or_else(|_| day.to_string());
    if events.is_empty() {
        return format!("Nothing on the calendar for {label}.");
    }
    let lines = events.iter().map(|event| {
        let time = if event.all_day { "all day".to_string() } else { format!("{}–{}", event.start.get(11..).unwrap_or(""), event.end.get(11..).unwrap_or("")) };
        let place = if event.location.is_empty() { String::new() } else { format!(" at {}", event.location) };
        format!("- {time}: {}{place}", event.title)
    });
    format!("Calendar for {label}:\n{}", lines.collect::<Vec<_>>().join("\n"))
}

pub fn describe_reminders(reminders: &[Reminder]) -> String {
    if reminders.is_empty() {
        return "No open reminders.".into();
    }
    let lines = reminders.iter().take(25).map(|reminder| {
        let due = if reminder.due.is_empty() { String::new() } else { format!(" (due {})", when(&reminder.due)) };
        format!("- {}{due} [{}]", reminder.title, reminder.list)
    });
    format!("Open reminders:\n{}", lines.collect::<Vec<_>>().join("\n"))
}

/// Recent mail as plain lines, newest first, with the opening of each message.
async fn read_mail(query: &str, unread: bool, limit: usize) -> Result<String, String> {
    let days = if query.is_empty() { "14" } else { "120" };
    let mut found = rows(&osascript(MAIL_LIST, &[query.into(), if unread { "1" } else { "0" }.into(), days.into()], 40).await?);
    found.retain(|row| row.len() >= 5);
    found.sort_by(|a, b| b[0].cmp(&a[0]));
    found.truncate(limit);
    if found.is_empty() {
        return Ok(match (query.is_empty(), unread) {
            (false, _) => format!("No email in the inbox from the last four months matches “{query}”."),
            (true, true) => "No unread email in the inbox from the last two weeks.".into(),
            _ => "No email in the inbox from the last two weeks.".into(),
        });
    }
    let ids: Vec<String> = found.iter().map(|row| row[1].clone()).collect();
    let openings: std::collections::HashMap<String, String> = rows(&osascript(MAIL_TEXT, &ids, 40).await.unwrap_or_default()).into_iter().filter(|row| row.len() >= 2).map(|row| (row[0].clone(), row[1].clone())).collect();
    let lines = found.iter().enumerate().map(|(index, row)| {
        let unread = if row[2] == "false" { " · unread" } else { "" };
        let opening = openings.get(&row[1]).map(|text| format!("\n   {text}")).unwrap_or_default();
        format!("{}. From {} · {}{unread}\n   Subject: {}{opening}", index + 1, row[3], when(&row[0]), row[4])
    });
    Ok(format!("Email in the inbox, newest first:\n{}", lines.collect::<Vec<_>>().join("\n")))
}

async fn find_notes(query: &str) -> Result<String, String> {
    let found = rows(&osascript(NOTES_FIND, &[query.into()], 40).await?);
    if found.is_empty() {
        return Ok(format!("No notes mention “{query}”."));
    }
    let lines = found.iter().filter(|row| row.len() >= 3).map(|row| format!("- {} (edited {})\n  {}", row[1], when(&row[0]), row[2]));
    Ok(format!("Notes mentioning “{query}”:\n{}", lines.collect::<Vec<_>>().join("\n")))
}

async fn control_music(app: &str, action: &str, query: &str) -> Result<String, String> {
    let name = if app == "spotify" { "Spotify" } else { "Music" };
    let query = if app == "spotify" && !query.is_empty() { format!("spotify:search:{}", percent(query)) } else { query.to_string() };
    let result = osascript(if app == "spotify" { SPOTIFY } else { MUSIC }, &[action.into(), query.clone()], 20).await?;
    let parts: Vec<&str> = result.trim().split('\t').collect();
    Ok(match (action, parts.as_slice()) {
        (_, ["closed"]) => format!("{name} isn’t open, so nothing is playing."),
        (_, ["stopped"]) => format!("Nothing is playing in {name}."),
        ("now_playing", [state, track, artist, album]) => format!("{name} is {}: “{track}” by {artist}, from {album}.", if *state == "paused" { "paused on" } else { "playing" }),
        (_, ["playlist", playlist]) => format!("Playing the “{playlist}” playlist in {name}."),
        (_, ["track", track, artist]) => format!("Playing “{track}” by {artist} in {name}."),
        (_, ["notfound"]) => format!("Nothing in your {name} library matches that."),
        (_, ["search"]) => format!("Opened Spotify’s search for that. Pick what to play there; Spotify doesn’t let other apps start a search result."),
        ("pause", _) => format!("Paused {name}."),
        ("next", _) => format!("Skipped to the next track in {name}."),
        ("previous", _) => format!("Went back a track in {name}."),
        _ => format!("{name} is playing."),
    })
}

/// Runs a read or playback call (already validated, for an allowed app) and returns
/// plain text for the model.
pub async fn run(tool: &str, args: &Map<String, Value>, helper_path: &Path) -> Result<String, String> {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    match tool {
        "read_mail" => read_mail(&get("query"), args.get("unread").and_then(Value::as_bool).unwrap_or(false), args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize).await,
        "read_calendar" => Ok(describe_events(&get("day"), &events(helper_path, &get("day")).await?)),
        "read_reminders" => Ok(describe_reminders(&reminders(helper_path).await?)),
        "find_notes" => find_notes(&get("query")).await,
        "control_music" => control_music(&get("app"), &get("action"), &get("query")).await,
        _ => Err("Unknown app tool.".into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    id: &'static str,
    name: &'static str,
    can: &'static [&'static str],
    installed: bool,
    allowed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppList {
    apps: Vec<AppStatus>,
    /// Every other app in the Applications folders, which Buntini can open when asked.
    others: Vec<String>,
}

fn app_names() -> Vec<String> {
    let home = std::env::var("HOME").map(|home| PathBuf::from(home).join("Applications")).ok();
    let mut names: Vec<String> = ["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities"]
        .iter()
        .map(PathBuf::from)
        .chain(home)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_suffix(".app")).map(str::to_string))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The apps Buntini can work with and whether each is allowed, plus the rest it can open.
#[tauri::command]
pub fn app_list(state: tauri::State<'_, crate::controller::AppState>) -> Result<AppList, String> {
    let settings = state.store.lock().unwrap().settings()?;
    let apps: Vec<AppStatus> = APPS.iter().map(|app| AppStatus { id: app.id, name: app.name, can: app.can, installed: installed(app), allowed: allowed(&settings, app.id) }).collect();
    let others = app_names().into_iter().filter(|name| !APPS.iter().any(|app| app.name == name)).collect();
    Ok(AppList { apps, others })
}

/// Asks macOS for permission right after the user allows Calendar or Reminders, so the
/// prompt appears in Settings rather than in the middle of a chat. Other apps are asked
/// the first time Buntini uses them.
#[tauri::command]
pub async fn check_app(id: String, state: tauri::State<'_, crate::controller::AppState>) -> Result<String, String> {
    let helper_path = state.models.apple_helper().to_path_buf();
    match id.as_str() {
        "calendar" => events(&helper_path, &chrono::Local::now().format("%Y-%m-%d").to_string()).await.map(|_| "Calendar is ready.".into()),
        "reminders" => reminders(&helper_path).await.map(|_| "Reminders is ready.".into()),
        _ => Ok(app(&id).map(|app| format!("macOS will ask you once, the first time Buntini uses {}.", app.name)).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_checked_and_normalised() {
        let mail = validate("read_mail", &json!({"query":" invoice ","limit":40})).unwrap();
        assert_eq!(mail["query"], "invoice");
        assert_eq!(mail["limit"], 8, "at most eight messages");
        assert!(validate("read_mail", &json!({"folder":"Sent"})).is_err(), "unknown keys are refused");
        assert!(validate("read_calendar", &json!({"day":"tomorrow"})).is_err());
        assert!(validate("read_calendar", &json!({})).unwrap()["day"].as_str().unwrap().len() == 10, "today by default");
        assert!(validate("find_notes", &json!({})).is_err());
        assert!(validate("control_music", &json!({"app":"vlc","action":"play"})).is_err());
        assert!(validate("control_music", &json!({"action":"shuffle"})).is_err());
        let play = validate("control_music", &json!({"app":"spotify","action":"pause","query":"jazz"})).unwrap();
        assert!(play.get("query").is_none(), "only play takes a query");
        assert!(validate("send_mail", &json!({})).is_err());
    }

    #[test]
    fn only_allowed_apps_are_usable() {
        let mut settings = Settings::default();
        let args = Map::new();
        assert!(!permits(&settings, "draft_email", &args), "nothing is allowed until the user says so");
        assert!(permits(&settings, "open_app", &args), "opening an app is asked each time instead");
        settings.apps = vec!["mail".into()];
        let mail = APPS.iter().find(|app| app.id == "mail").unwrap();
        assert_eq!(permits(&settings, "read_mail", &args), installed(mail));
        let spotify = validate("control_music", &json!({"app":"spotify","action":"next"})).unwrap();
        assert!(!permits(&settings, "control_music", &spotify), "Music being allowed says nothing about Spotify");
        assert_eq!(for_call("add_reminder", &args).unwrap().id, "reminders");
    }

    #[test]
    fn results_read_plainly() {
        let events = [Event { start: "2026-10-22 09:00".into(), end: "2026-10-22 09:15".into(), all_day: false, title: "Standup".into(), location: String::new() }, Event { start: "2026-10-22 00:00".into(), end: "2026-10-23 00:00".into(), all_day: true, title: "Ana’s birthday".into(), location: "Porto".into() }];
        assert_eq!(describe_events("2026-10-22", &events), "Calendar for Thursday 22 October:\n- 09:00–09:15: Standup\n- all day: Ana’s birthday at Porto");
        assert_eq!(describe_events("2026-10-22", &[]), "Nothing on the calendar for Thursday 22 October.");
        let reminders = [Reminder { title: "Bins".into(), due: "2026-10-22 19:00".into(), list: "Home".into() }];
        assert_eq!(describe_reminders(&reminders), "Open reminders:\n- Bins (due Thu 22 Oct 19:00) [Home]");
        assert_eq!(percent("lo-fi beats"), "lo%2Dfi%20beats");
    }

    #[test]
    fn values_are_arguments_not_script() {
        for source in [MAIL_LIST, MAIL_TEXT, NOTES_FIND, MUSIC, SPOTIFY] {
            assert!(source.contains("on run argv"), "every script reads its values from argv");
        }
    }
}
