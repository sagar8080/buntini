//! Apps on this Mac that Buntini can work with, each only after the user allows it in
//! Settings → Apps. No accounts and nothing online: Buntini talks to the apps already
//! here, through AppleScript (values always passed as arguments, never as script text)
//! and, for Calendar, Reminders and Contacts, the `buntini-fm` helper (EventKit, so
//! repeating events are included, and CNContactStore). macOS still asks once per app
//! before the first use.
//!
//! Reading (email, the day's events, open reminders, notes, people in Contacts, what's
//! playing, open browser tabs and the Safari page) plus playback controls and switching
//! tabs run straight away
//! for an allowed app; anything that adds or writes goes through the approval sheet in
//! `actions.rs`. Private and incognito browser windows are skipped wherever the browser
//! says which they are.
use crate::{privacy::Share, types::Settings};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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
    /// Something the user should know before allowing it.
    pub note: Option<&'static str>,
}

pub const APPS: [App; 13] = [
    App { id: "mail", name: "Mail", can: &["Read and search your recent email", "Draft replies for you to send"], tools: &["read_mail", "draft_email"], share: Some(Share::Mail), note: None },
    App { id: "calendar", name: "Calendar", can: &["See what’s on any day", "Add events you approve"], tools: &["read_calendar", "add_calendar_event"], share: Some(Share::Calendar), note: None },
    App { id: "reminders", name: "Reminders", can: &["See your open reminders", "Add reminders you approve"], tools: &["read_reminders", "add_reminder"], share: Some(Share::Calendar), note: None },
    App { id: "notes", name: "Notes", can: &["Find your notes", "Write notes you approve"], tools: &["find_notes", "create_note"], share: Some(Share::Notes), note: None },
    App { id: "contacts", name: "Contacts", can: &["Find the people you name, with their email addresses and phone numbers"], tools: &["find_contact"], share: Some(Share::Contacts), note: Some("A draft is addressed from Contacts only when exactly one person and one address match. Otherwise you pick the address when you approve it.") },
    App { id: "music", name: "Music", can: &["See what’s playing", "Play, pause and skip", "Play songs and playlists from your library"], tools: &["control_music"], share: Some(Share::Music), note: None },
    App { id: "spotify", name: "Spotify", can: &["See what’s playing", "Play, pause and skip", "Play a song you name"], tools: &["control_music"], share: Some(Share::Music), note: Some("To play a song you name, Buntini opens Spotify’s search and presses play on the best match, so macOS asks you to allow Buntini under Accessibility.") },
    App { id: "shortcuts", name: "Shortcuts", can: &["Run your shortcuts after you approve"], tools: &["run_shortcut"], share: None, note: None },
    App { id: "safari", name: "Safari", can: &["List and find your open tabs", "Switch to a tab", "Read the page you’re on"], tools: &["browser_tabs", "switch_tab", "read_page"], share: Some(Share::Browser), note: Some("Safari doesn’t say which windows are private, so Buntini can see those too. Close private windows before asking.") },
    App { id: "chrome", name: "Google Chrome", can: &["List and find your open tabs", "Switch to a tab"], tools: &["browser_tabs", "switch_tab"], share: Some(Share::Browser), note: None },
    App { id: "brave", name: "Brave Browser", can: &["List and find your open tabs", "Switch to a tab"], tools: &["browser_tabs", "switch_tab"], share: Some(Share::Browser), note: None },
    App { id: "edge", name: "Microsoft Edge", can: &["List and find your open tabs", "Switch to a tab"], tools: &["browser_tabs", "switch_tab"], share: Some(Share::Browser), note: None },
    App { id: "arc", name: "Arc", can: &["List and find your open tabs", "Switch to a tab"], tools: &["browser_tabs", "switch_tab"], share: Some(Share::Browser), note: None },
];

pub const BROWSERS: [&str; 5] = ["safari", "chrome", "brave", "edge", "arc"];

/// App read/control schemas. The workflow controller requires durable approval
/// for playback mutations and tab switching; now_playing remains a read.
pub const READ_TOOLS: [&str; 9] = [
    "read_mail",
    "read_calendar",
    "read_reminders",
    "find_notes",
    "find_contact",
    "control_music",
    "browser_tabs",
    "switch_tab",
    "read_page",
];

pub fn app(id: &str) -> Option<&'static App> {
    APPS.iter().find(|app| app.id == id)
}

pub fn installed(app: &App) -> bool {
    crate::actions::find_app(app.name).is_some()
}

pub fn allowed(settings: &Settings, id: &str) -> bool {
    settings.apps.iter().any(|allowed| allowed == id)
}

/// The app a call uses: music and browser calls may name theirs; every other tool
/// belongs to one app. Tab calls that name no browser use every allowed browser.
pub fn for_call(tool: &str, args: &Map<String, Value>) -> Option<&'static App> {
    match tool {
        "control_music" => app(args.get("app").and_then(Value::as_str).unwrap_or("music")),
        "browser_tabs" | "switch_tab" => app(args
            .get("browser")
            .and_then(Value::as_str)
            .unwrap_or("safari")),
        _ => APPS.iter().find(|app| app.tools.contains(&tool)),
    }
}

/// Which music app a playback call means. Saying "Spotify" or "Apple Music" decides it;
/// otherwise the model's choice if it made one; otherwise the allowed app that's open,
/// preferring Spotify. Small models often leave the app out or pick the wrong one.
pub fn player(settings: &Settings, prompt: &str, chosen: Option<&str>) -> &'static str {
    let said = prompt.to_lowercase();
    if said.contains("spotify") {
        return "spotify";
    }
    if said.contains("apple music") || said.contains("music app") || said.contains("itunes") {
        return "music";
    }
    let usable = |id: &str| app(id).is_some_and(|app| allowed(settings, id) && installed(app));
    match chosen {
        Some("spotify") if usable("spotify") => return "spotify",
        Some("music") if usable("music") && !usable("spotify") => return "music",
        _ => {}
    }
    let running = |bundle: &str| {
        std::process::Command::new("/usr/bin/pgrep")
            .args(["-xq", bundle])
            .status()
            .is_ok_and(|status| status.success())
    };
    match (usable("spotify"), usable("music")) {
        (true, true) if running("Music") && !running("Spotify") => "music",
        (false, true) => "music",
        _ => "spotify",
    }
}

/// Allowed, installed browsers a tab call may look in.
fn browsers_for(settings: &Settings, args: &Map<String, Value>) -> Vec<&'static App> {
    let named = args.get("browser").and_then(Value::as_str);
    usable(settings)
        .into_iter()
        .filter(|app| BROWSERS.contains(&app.id) && named.is_none_or(|id| id == app.id))
        .collect()
}

/// Whether the user allowed the app behind this call, and it is on this Mac.
pub fn permits(settings: &Settings, tool: &str, args: &Map<String, Value>) -> bool {
    match tool {
        "open_app" | "open_link" => true,
        "browser_tabs" | "switch_tab" => !browsers_for(settings, args).is_empty(),
        _ => for_call(tool, args).is_some_and(|app| allowed(settings, app.id) && installed(app)),
    }
}

/// Allowed, installed apps whose tools a model may be offered, with the data each reads.
pub fn usable(settings: &Settings) -> Vec<&'static App> {
    APPS.iter()
        .filter(|app| allowed(settings, app.id) && installed(app))
        .collect()
}

/// One line for the model: which apps it can use, and which the user could still allow.
pub fn prompt_line(settings: &Settings) -> String {
    let (here, missing): (Vec<&App>, Vec<&App>) = APPS.iter().partition(|app| installed(app));
    let (yes, no): (Vec<&App>, Vec<&App>) =
        here.into_iter().partition(|app| allowed(settings, app.id));
    let mut line = String::new();
    if !yes.is_empty() {
        line += &format!(
            " Apps you may use for the user: {}.",
            yes.iter()
                .map(|app| app.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !no.is_empty() {
        line += &format!(" Not allowed yet: {}. If they ask for one of those, say they can allow it in Settings → Apps.", no.iter().map(|app| app.name).collect::<Vec<_>>().join(", "));
    }
    if !missing.is_empty() {
        line += &format!(
            " Not installed on this Mac: {}. If they ask for one of those, say it isn’t installed.",
            missing
                .iter()
                .map(|app| app.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    line
}

fn text(args: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value))
            if value.trim().chars().count() <= max && !value.chars().any(char::is_control) =>
        {
            Ok(Some(value.trim().to_string()))
        }
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
        "find_contact" => &["name"],
        "control_music" => &["app", "action", "query"],
        "browser_tabs" => &["browser", "query"],
        "switch_tab" => &["browser", "query"],
        "read_page" => &[],
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
            let unread = match args.get("unread") {
                None => false,
                Some(Value::Bool(unread)) => *unread,
                _ => return Err("unread must be true or false.".into()),
            };
            out.insert("unread".into(), json!(unread));
            let limit = match args.get("limit") {
                None => 5,
                Some(limit) => limit
                    .as_u64()
                    .ok_or("limit must be a positive whole number.")?
                    .clamp(1, 8),
            };
            out.insert("limit".into(), json!(limit));
        }
        "read_calendar" => {
            let today = chrono::Local::now().date_naive();
            let day = match text(args, "day", 10)? {
                Some(day) => chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                    .map_err(|_| "Use a day like 2026-10-23.")?,
                None => today,
            };
            if (day - today).num_days().abs() > 366 {
                return Err("Pick a day within a year of today.".into());
            }
            out.insert("day".into(), day.format("%Y-%m-%d").to_string().into());
        }
        "find_notes" => {
            out.insert(
                "query".into(),
                text(args, "query", 100)?
                    .ok_or("Say what to look for in your notes.")?
                    .into(),
            );
        }
        "find_contact" => {
            let name = text(args, "name", 100)?.ok_or("Say whose contact to find.")?;
            // The helper takes the name as its last argument; it is never an option.
            if name.starts_with('-') {
                return Err("Say whose contact to find, by name or email address.".into());
            }
            out.insert("name".into(), name.into());
        }
        "control_music" => {
            let selected = text(args, "app", 10)?;
            let app = selected.as_deref().unwrap_or("music");
            if !["music", "spotify"].contains(&app) {
                return Err("Music controls work with Music or Spotify.".into());
            }
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("Say what to do: now_playing, play, pause, next or previous.")?;
            if !["now_playing", "play", "pause", "next", "previous"].contains(&action) {
                return Err(
                    "Music controls are now_playing, play, pause, next and previous.".into(),
                );
            }
            out.insert("app".into(), app.into());
            out.insert("action".into(), action.into());
            if action == "play" {
                if let Some(query) = text(args, "query", 100)? {
                    out.insert("query".into(), query.into());
                }
            }
        }
        "browser_tabs" | "switch_tab" => {
            if let Some(browser) = text(args, "browser", 10)? {
                if !BROWSERS.contains(&browser.as_str()) {
                    return Err("Tabs work with Safari, Chrome, Brave, Edge and Arc.".into());
                }
                out.insert("browser".into(), browser.into());
            }
            match text(args, "query", 100)? {
                Some(query) => {
                    out.insert("query".into(), query.into());
                }
                None if tool == "switch_tab" => {
                    return Err(
                        "Say which tab to switch to, by words from its title or address.".into(),
                    )
                }
                None => {}
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
// time, message ID, read, sender, subject, Mail-local numeric ID. Mail filters natively, so this stays quick.
const MAIL_LIST: &str = r#"on run argv
  set q to item 1 of argv
  set cutoff to (current date) - ((item 3 of argv) as integer) * days
  set out to {}
  tell application "Mail"
    if q is not "" and item 2 of argv is "1" then
      set found to (messages of inbox whose date received > cutoff and read status is false and (subject contains q or sender contains q))
    else if q is not "" then
      set found to (messages of inbox whose date received > cutoff and (subject contains q or sender contains q))
    else if item 2 of argv is "1" then
      set found to (messages of inbox whose date received > cutoff and read status is false)
    else
      set found to (messages of inbox whose date received > cutoff)
    end if
    repeat with m in found
      set end of out to my stamp(date received of m) & tab & my clean(message id of m) & tab & (read status of m as text) & tab & my clean(sender of m) & tab & my clean(subject of m) & tab & my clean(id of m)
      if (count of out) ≥ 300 then exit repeat
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (count of found as text) & tab & (count of out as text) & tab & "true"
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// The opening of each message, by its Mail-local numeric ID (message headers can duplicate).
const MAIL_TEXT: &str = r#"on run argv
  set out to {}
  tell application "Mail"
    repeat with mid in argv
      try
        set c to content of (first message of inbox whose id is (mid as integer))
        set originalLength to length of c
        if originalLength > 500 then set c to text 1 thru 500 of c
        set end of out to my clean(mid) & tab & my clean(c) & tab & (originalLength as text)
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
    set found to (notes whose name contains q or plaintext contains q)
    repeat with n in found
      set t to plaintext of n
      set originalLength to length of t
      if originalLength > 400 then set t to text 1 thru 400 of t
      set end of out to my stamp(modification date of n) & tab & my clean(id of n) & tab & my clean(name of n) & tab & my clean(t) & tab & (originalLength as text)
      if (count of out) ≥ 6 then exit repeat
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (count of found as text) & tab & (count of out as text) & tab & "true"
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// Notes changed in the last N days, for the memory index: modified time, ID, title and
// the first 1,500 characters of plain text (at most 300 notes).
const NOTES_RECENT: &str = r#"on run argv
  set cutoff to (current date) - ((item 1 of argv) as integer) * days
  set out to {}
  tell application "Notes"
    set found to (notes whose modification date > cutoff)
    repeat with n in found
      set t to plaintext of n
      if t is missing value then set t to ""
      if length of t > 1500 then set t to text 1 thru 1500 of t
      set end of out to my stamp(modification date of n) & tab & my clean(id of n) & tab & my clean(name of n) & tab & my clean(t)
      if (count of out) ≥ 300 then exit repeat
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (count of found as text) & tab & (count of out as text) & tab & "true"
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// argv: action, query. Asking what's playing never launches the app.
const MUSIC: &str = r#"on run argv
  set act to item 1 of argv
  set q to item 2 of argv
  if (act is "now_playing" or act is "read_evidence") and application "Music" is not running then return "closed"
  tell application "Music"
    if act is "now_playing" or act is "read_evidence" then
      if player state is stopped then return "stopped"
      set trackId to ""
      if act is "read_evidence" then
        try
          set trackId to persistent ID of current track
        end try
      end if
      set result to (player state as text) & tab & my clean(name of current track) & tab & my clean(artist of current track) & tab & my clean(album of current track)
      if act is "read_evidence" then return result & tab & my clean(trackId) & tab & "__buntini_end__"
      return result
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
// AppleScript, so a request to play something opens its search; `skills::music::press_play`
// then presses play on the best match.
const SPOTIFY: &str = r#"on run argv
  set act to item 1 of argv
  set q to item 2 of argv
  if (act is "now_playing" or act is "read_evidence") and application "Spotify" is not running then return "closed"
  tell application "Spotify"
    if act is "now_playing" or act is "read_evidence" then
      if player state is stopped then return "stopped"
      set trackId to ""
      if act is "read_evidence" then
        try
          set trackId to id of current track
        end try
      end if
      set result to (player state as text) & tab & my clean(name of current track) & tab & my clean(artist of current track) & tab & my clean(album of current track)
      if act is "read_evidence" then return result & tab & my clean(trackId) & tab & "__buntini_end__"
      return result
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

// Browser tabs. "APP" is replaced with the browser's name from `APPS`, never user text;
// the search words arrive in argv. Nothing launches a browser that isn't open. Each line:
// window number, whether the tab is the one showing, title, address.
const CHROMIUM_TABS: &str = r#"on run argv
  set q to item 1 of argv
  set out to {}
  set total to 0
  set completeRead to true
  if application "APP" is not running then return "__buntini_summary__" & tab & "0" & tab & "0" & tab & "true"
  tell application "APP"
    set wi to 0
    repeat with w in windows
      set wi to wi + 1
      if mode of w is not "incognito" then
        set showing to active tab index of w
        set ti to 0
        repeat with t in tabs of w
          set ti to ti + 1
          set n to my clean(title of t)
          set u to my clean(URL of t)
          if q is "" or n contains q or u contains q then
            set total to total + 1
            if (count of out) < 80 then
              set windowId to ""
              set tabId to ""
              try
              set windowId to my clean(id of w)
              set tabId to my clean(id of t)
              end try
              set end of out to (wi as text) & tab & (ti as text) & tab & ((ti = showing) as text) & tab & n & tab & u & tab & windowId & tab & tabId
            end if
          end if
        end repeat
      end if
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (total as text) & tab & (count of out as text) & tab & (completeRead as text)
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
const CHROMIUM_SWITCH: &str = r#"on run argv
  set q to item 1 of argv
  if application "APP" is not running then return ""
  tell application "APP"
    repeat with w in windows
      if mode of w is not "incognito" then
        set ti to 0
        repeat with t in tabs of w
          set ti to ti + 1
          if (title of t contains q) or (URL of t contains q) then
            set active tab index of w to ti
            set index of w to 1
            activate
            return my clean(title of t)
          end if
        end repeat
      end if
    end repeat
  end tell
  return ""
end run
"#;
const ARC_TABS: &str = r#"on run argv
  set q to item 1 of argv
  set out to {}
  set total to 0
  set completeRead to true
  if application "Arc" is not running then return "__buntini_summary__" & tab & "0" & tab & "0" & tab & "true"
  tell application "Arc"
    set wi to 0
    repeat with w in windows
      set wi to wi + 1
      if not (incognito of w) then
        set ti to 0
        repeat with t in tabs of w
          set ti to ti + 1
          set n to my clean(title of t)
          set u to my clean(URL of t)
          if q is "" or n contains q or u contains q then
            set total to total + 1
            if (count of out) < 80 then
              set windowId to ""
              set tabId to ""
              try
                set tabId to ""
              end try
              set end of out to (wi as text) & tab & (ti as text) & tab & "unknown" & tab & n & tab & u & tab & windowId & tab & tabId
            end if
          end if
        end repeat
      end if
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (total as text) & tab & (count of out as text) & tab & (completeRead as text)
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
const ARC_SWITCH: &str = r#"on run argv
  set q to item 1 of argv
  if application "Arc" is not running then return ""
  tell application "Arc"
    repeat with w in windows
      if not (incognito of w) then
        repeat with t in tabs of w
          if (title of t contains q) or (URL of t contains q) then
            tell t to select
            activate
            return my clean(title of t)
          end if
        end repeat
      end if
    end repeat
  end tell
  return ""
end run
"#;
// Safari can't say which windows are private (Settings says so before it's allowed).
const SAFARI_TABS: &str = r#"on run argv
  set q to item 1 of argv
  set out to {}
  set total to 0
  set completeRead to true
  if application "Safari" is not running then return "__buntini_summary__" & tab & "0" & tab & "0" & tab & "true"
  tell application "Safari"
    set wi to 0
    repeat with w in windows
      set wi to wi + 1
      try
        set showing to index of current tab of w
        set ti to 0
        repeat with t in tabs of w
          set ti to ti + 1
          set n to my clean(name of t)
          set u to my clean(URL of t)
          if q is "" or n contains q or u contains q then
            set total to total + 1
            if (count of out) < 80 then
              set windowId to ""
              set tabId to ""
              try
              set windowId to my clean(id of w)
              end try
              set end of out to (wi as text) & tab & (ti as text) & tab & ((ti = showing) as text) & tab & n & tab & u & tab & windowId & tab & tabId
            end if
          end if
        end repeat
      on error
        set completeRead to false
      end try
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (total as text) & tab & (count of out as text) & tab & (completeRead as text)
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
const SAFARI_SWITCH: &str = r#"on run argv
  set q to item 1 of argv
  if application "Safari" is not running then return ""
  tell application "Safari"
    repeat with w in windows
      try
        repeat with t in tabs of w
          if (name of t contains q) or (URL of t contains q) then
            set current tab of w to t
            set index of w to 1
            activate
            return my clean(name of t)
          end if
        end repeat
      end try
    end repeat
  end tell
  return ""
end run
"#;
// The page showing in Safari's front window: title and address, then its text.
const SAFARI_PAGE: &str = r#"on run argv
  if application "Safari" is not running then return "closed"
  tell application "Safari"
    if (count of documents) is 0 then return "none"
    set d to front document
    set n to my clean(name of d)
    set u to my clean(URL of d)
    set t to text of d
  end tell
  if t is missing value then set t to ""
  set originalLength to length of t
  if originalLength > 8000 then set t to text 1 thru 8000 of t
  return n & tab & u & tab & (originalLength as text) & linefeed & t
end run
"#;

// Unreplied inbox messages from the last N days, newest 60 first by Mail's order: time,
// Mail-local numeric ID, read, flagged, whether a To recipient is one of the user's own
// account addresses, how many To recipients, sender, subject. Then a count summary.
const MAIL_TRIAGE: &str = r#"on run argv
  set cutoff to (current date) - ((item 1 of argv) as integer) * days
  set out to {}
  tell application "Mail"
    set mine to {}
    repeat with a in accounts
      try
        set mine to mine & (email addresses of a)
      end try
    end repeat
    set found to (messages of inbox whose date received > cutoff and was replied to is false)
    set total to count of found
    repeat with m in found
      set toMe to "unknown"
      set toCount to 0
      try
        set recipientsTo to to recipients of m
        set toCount to count of recipientsTo
        if (count of mine) > 0 then
          set toMe to "false"
          repeat with r in recipientsTo
            if (address of r) is in mine then set toMe to "true"
          end repeat
        end if
      end try
      set end of out to my stamp(date received of m) & tab & my clean(id of m) & tab & (read status of m as text) & tab & (flagged status of m as text) & tab & toMe & tab & (toCount as text) & tab & my clean(sender of m) & tab & my clean(subject of m)
      if (count of out) ≥ 60 then exit repeat
    end repeat
  end tell
  set end of out to "__buntini_summary__" & tab & (total as text) & tab & (count of out as text) & tab & "true"
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;
// The message selected in Mail's front viewer: time, message ID, sender, subject, Mail-local
// numeric ID, body length and how many are selected, then the opening of its body.
const MAIL_SELECTED: &str = r#"on run argv
  if application "Mail" is not running then return "closed"
  tell application "Mail"
    set picked to selection
    if (count of picked) is 0 then return "none"
    set m to item 1 of picked
    set c to content of m
    if c is missing value then set c to ""
    set originalLength to length of c
    if originalLength > 4000 then set c to text 1 thru 4000 of c
    return my stamp(date received of m) & tab & my clean(message id of m) & tab & my clean(sender of m) & tab & my clean(subject of m) & tab & my clean(id of m) & tab & (originalLength as text) & tab & ((count of picked) as text) & linefeed & c
  end tell
end run
"#;
// The items selected in Finder's front window, as POSIX paths, one per line (at most 20).
const FINDER_SELECTED: &str = r#"on run argv
  if application "Finder" is not running then return ""
  tell application "Finder" to set picked to selection
  set out to {}
  repeat with f in picked
    set end of out to POSIX path of (f as alias)
    if (count of out) ≥ 20 then exit repeat
  end repeat
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;

async fn osascript(source: &str, argv: &[String], seconds: u64) -> Result<String, String> {
    let mut command =
        crate::actions::osascript_command("AppleScript", &format!("{HELPERS}{source}"));
    command.args(argv);
    crate::actions::output(command, seconds).await
}

fn rows(output: &str) -> Vec<Vec<String>> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split('\t')
                .map(|part| part.trim().to_string())
                .collect()
        })
        .collect()
}

pub(crate) fn percent(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// "2026-10-22 09:15" as "Thu 22 Oct 09:15".
fn when(stamp: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M")
        .map(|at| at.format("%a %-d %b %H:%M").to_string())
        .unwrap_or_else(|_| stamp.to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "calendarId")]
    pub calendar_id: Option<String>,
    #[serde(default)]
    pub calendar: String,
    #[serde(default, rename = "modifiedAt")]
    pub modified_at: Option<String>,
    #[serde(default, rename = "startAt")]
    pub start_at: Option<String>,
    #[serde(default, rename = "endAt")]
    pub end_at: Option<String>,
    pub start: String,
    pub end: String,
    #[serde(rename = "allDay")]
    pub all_day: bool,
    pub title: String,
    pub location: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Reminder {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "calendarId")]
    pub calendar_id: Option<String>,
    #[serde(default, rename = "modifiedAt")]
    pub modified_at: Option<String>,
    #[serde(default, rename = "dueAt")]
    pub due_at: Option<String>,
    #[serde(default, rename = "dueDateOnly")]
    pub due_date_only: Option<bool>,
    pub title: String,
    pub due: String,
    pub list: String,
}

/// A bounded agenda read, with the original result count retained through display.
#[derive(Debug, Clone, PartialEq)]
pub struct Agenda<T> {
    pub items: Vec<T>,
    pub total: usize,
}

impl<T> Agenda<T> {
    pub fn complete(&self) -> bool {
        self.items.len() == self.total
    }
}

async fn helper<T: DeserializeOwned>(helper: &Path, args: &[&str]) -> Result<Agenda<T>, String> {
    if !helper.is_file() {
        return Err(
            "Reading Calendar and Reminders needs Buntini’s Apple helper, which isn’t on this Mac."
                .into(),
        );
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
    parse_helper_rows(&output)
}

fn parse_helper_rows<T: DeserializeOwned>(output: &str) -> Result<Agenda<T>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Summary {
        #[serde(rename = "type")]
        kind: String,
        total: usize,
        returned: usize,
        complete: bool,
    }
    let invalid = || {
        "Calendar or Reminders returned incomplete data. Please try again with the current Apple helper.".to_string()
    };
    let mut items = Vec::new();
    let mut summary: Option<Summary> = None;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        if summary.is_some() {
            return Err(invalid());
        }
        let value: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        if value.get("type").is_some() {
            let counts: Summary = serde_json::from_value(value).map_err(|_| invalid())?;
            if counts.kind != "agenda_summary"
                || counts.returned != items.len()
                || counts.total < counts.returned
                || counts.complete != (counts.total == counts.returned)
            {
                return Err(invalid());
            }
            summary = Some(counts);
        } else {
            items.push(serde_json::from_value(value).map_err(|_| invalid())?);
        }
    }
    let summary = summary.ok_or_else(invalid)?;
    Ok(Agenda {
        items,
        total: summary.total,
    })
}

/// The day's events (YYYY-MM-DD), repeating ones included, in start order.
pub async fn events(helper_path: &Path, day: &str) -> Result<Agenda<Event>, String> {
    helper(helper_path, &["--events", day]).await
}

/// Open reminders, soonest due first; undated ones last.
pub async fn reminders(helper_path: &Path) -> Result<Agenda<Reminder>, String> {
    helper(helper_path, &["--reminders"]).await
}

/// At most this many people per lookup, each with at most this many addresses and numbers.
const CONTACT_LIMIT: usize = 5;
const CONTACT_DETAILS: usize = 5;

const CONTACTS_DENIED: &str = "macOS hasn’t allowed Buntini to read Contacts. Allow it in System Settings → Privacy & Security → Contacts.";
const CONTACTS_HELPER: &str =
    "Finding people in Contacts needs Buntini’s Apple helper, which isn’t on this Mac.";

/// One person from Contacts, as the helper read them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub id: Option<String>,
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

/// One plain address a draft could go to: no display name, list or placeholder.
pub fn plain_address(address: &str) -> bool {
    address.split_once('@').is_some_and(|(user, domain)| {
        !user.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    }) && address.chars().count() <= 254
        && !address.contains(char::is_whitespace)
        && !address.contains([',', ';', '<', '>', '[', ']', '"'])
}

/// The helper's answer, checked for a dropped or inconsistent count. Addresses that
/// could not address a draft are dropped here, so records and display text agree.
fn parse_contacts(output: &str) -> Result<Agenda<Contact>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Found {
        contacts: Vec<Contact>,
        total: usize,
        returned: usize,
        complete: bool,
    }
    let invalid = || {
        "Contacts returned incomplete data. Please try again with the current Apple helper."
            .to_string()
    };
    let value: Value = serde_json::from_str(output.trim()).map_err(|_| invalid())?;
    if value == json!({"error":"denied"}) {
        return Err(CONTACTS_DENIED.into());
    }
    let found: Found = serde_json::from_value(value).map_err(|_| invalid())?;
    if found.returned != found.contacts.len()
        || found.returned > CONTACT_LIMIT
        || found.total < found.returned
        || found.complete != (found.total == found.returned)
        || found.contacts.iter().any(|contact| {
            contact.emails.len() > CONTACT_DETAILS || contact.phones.len() > CONTACT_DETAILS
        })
    {
        return Err(invalid());
    }
    let items = found
        .contacts
        .into_iter()
        .map(|mut contact| {
            contact.emails.retain(|address| plain_address(address));
            contact.phones.retain(|phone| !phone.trim().is_empty());
            contact
        })
        .collect();
    Ok(Agenda {
        items,
        total: found.total,
    })
}

pub fn describe_contacts(name: &str, found: &Agenda<Contact>) -> String {
    if found.total == 0 {
        return format!("No one in Contacts matches “{name}”.");
    }
    let lines = found.items.iter().map(|contact| {
        let who = if contact.name.is_empty() {
            "(no name)"
        } else {
            &contact.name
        };
        let details: Vec<&str> = contact
            .emails
            .iter()
            .chain(&contact.phones)
            .map(String::as_str)
            .collect();
        if details.is_empty() {
            format!("- {who}: no email address or phone number")
        } else {
            format!("- {who}: {}", details.join(", "))
        }
    });
    let coverage = if found.complete() {
        String::new()
    } else {
        format!(
            "\nShowing the first {} of {} people who match. A fuller name narrows it down.",
            found.items.len(),
            found.total
        )
    };
    format!(
        "People in Contacts matching “{name}”:\n{}{coverage}",
        lines.collect::<Vec<_>>().join("\n")
    )
}

/// Contacts' own identifiers and details beside the display text. Several matches or a
/// partial list stay as they are: choosing one is the user's call, never the model's.
pub fn contact_evidence(name: &str, found: &Agenda<Contact>) -> Value {
    let records = found
        .items
        .iter()
        .enumerate()
        .map(|(index, contact)| {
            json!({"id":contact.id.as_deref().and_then(nonempty),
                "reference":source_reference("contacts", contact.id.as_deref(), index),
                "name":contact.name, "emails":contact.emails, "phones":contact.phones})
        })
        .collect();
    evidence(
        "contacts",
        json!({"app":"contacts","name":name,"matches":"name_or_email","limit":CONTACT_LIMIT}),
        records,
        Some(found.total),
        found.complete(),
        describe_contacts(name, found),
    )
}

/// People in Contacts matching a name or address, read with the Apple helper.
pub async fn find_contact(helper_path: &Path, name: &str) -> Result<Value, String> {
    if !helper_path.is_file() {
        return Err(CONTACTS_HELPER.into());
    }
    let mut command = Command::new(helper_path);
    command.args(["--find-contact", name]);
    // The first use waits for the macOS permission prompt, so allow time to answer it.
    let output = crate::actions::output(command, 90).await?;
    Ok(contact_evidence(name, &parse_contacts(&output)?))
}

/// Asks macOS for Contacts right after the user allows it, so the prompt shows in Settings.
async fn contacts_access(helper_path: &Path) -> Result<String, String> {
    if !helper_path.is_file() {
        return Err(CONTACTS_HELPER.into());
    }
    let mut command = Command::new(helper_path);
    command.arg("--contacts-access");
    let output = crate::actions::output(command, 90).await?;
    match serde_json::from_str::<Value>(output.trim())
        .ok()
        .and_then(|value| value["granted"].as_bool())
    {
        Some(true) => Ok("Contacts is ready.".into()),
        Some(false) => Err(CONTACTS_DENIED.into()),
        None => Err(
            "Contacts returned incomplete data. Please try again with the current Apple helper."
                .into(),
        ),
    }
}

pub fn describe_events(day: &str, events: &Agenda<Event>) -> String {
    let label = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| date.format("%A %-d %B").to_string())
        .unwrap_or_else(|_| day.to_string());
    if events.total == 0 {
        return format!("Nothing on the calendar for {label}.");
    }
    let lines = events.items.iter().map(|event| {
        let time = if event.all_day {
            "all day".to_string()
        } else {
            format!(
                "{}–{}",
                event.start.get(11..).unwrap_or(""),
                event.end.get(11..).unwrap_or("")
            )
        };
        let place = if event.location.is_empty() {
            String::new()
        } else {
            format!(" at {}", event.location)
        };
        format!("- {time}: {}{place}", event.title)
    });
    let coverage = if events.complete() {
        String::new()
    } else {
        format!("\nPartial calendar: showing the first {} of {} events. Later events are not included; do not treat unlisted time as free.", events.items.len(), events.total)
    };
    format!(
        "Calendar for {label}:\n{}{coverage}",
        lines.collect::<Vec<_>>().join("\n")
    )
}

/// Offset-bearing source timestamps survive a restart or a change of local zone.
fn source_timestamp(exact: Option<&str>, wall: &str) -> Option<String> {
    use chrono::TimeZone;
    if let Some(exact) = exact {
        return chrono::DateTime::parse_from_rfc3339(exact)
            .ok()
            .map(|date| date.to_rfc3339());
    }
    chrono::NaiveDateTime::parse_from_str(wall, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|at| chrono::Local.from_local_datetime(&at).single())
        .map(|at| at.to_rfc3339())
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// Identifiers belong to a source app. Even a source ID must be re-read before a
/// later mutation: EventKit/Notes IDs can change after synchronization or deletion.
fn source_reference(app: &str, id: Option<&str>, index: usize) -> Value {
    match id.filter(|id| !id.is_empty()) {
        Some(id) => json!({"kind":"app_local", "app":app, "id":id, "mustRevalidate":true}),
        None => json!({"kind":"snapshot", "app":app, "index":index, "mustRevalidate":true}),
    }
}

fn evidence(
    kind: &str,
    scope: Value,
    records: Vec<Value>,
    total: Option<usize>,
    complete: bool,
    display: String,
) -> Value {
    json!({"kind":kind, "schemaVersion":1, "observedAt":chrono::Utc::now().to_rfc3339(),
        "scope":scope, "count":records.len(), "total":total, "complete":complete,
        "records":records, "text":display})
}

/// Keep source fields alongside display text. A partial read never establishes
/// that absent records do not exist, and missing IDs/timestamps remain null.
pub fn calendar_evidence(day: &str, events: &Agenda<Event>) -> Value {
    let records: Vec<_> = events.items.iter().enumerate().map(|(index, event)| json!({
        "id":event.id.as_deref().and_then(nonempty),
        "reference":source_reference("calendar", event.id.as_deref(), index),
        "calendarId":event.calendar_id, "calendar":nonempty(&event.calendar),
        "modifiedAt":event.modified_at.as_deref().and_then(|at| source_timestamp(Some(at), "")),
        "start":source_timestamp(event.start_at.as_deref(), &event.start),
        "end":source_timestamp(event.end_at.as_deref(), &event.end), "all_day":event.all_day,
        "title":event.title, "location":event.location
    })).collect();
    let mut result = evidence(
        "calendar",
        json!({"app":"calendar","day":day,"calendars":"all_accessible","limit":40}),
        records.clone(),
        Some(events.total),
        events.complete(),
        describe_events(day, events),
    );
    // Retain the established calendar planning protocol while all reads share records.
    result["day"] = json!(day);
    result["events"] = json!(records);
    result
}

pub fn reminder_evidence(reminders: &Agenda<Reminder>) -> Value {
    let records = reminders.items.iter().enumerate().map(|(index, reminder)| {
        let date_only = reminder.due_date_only.or_else(|| (!reminder.due.is_empty()).then_some(reminder.due.len() == 10));
        json!({"id":reminder.id.as_deref().and_then(nonempty),
            "reference":source_reference("reminders", reminder.id.as_deref(), index),
            "title":reminder.title, "list":reminder.list, "listId":reminder.calendar_id,
            "modifiedAt":reminder.modified_at.as_deref().and_then(|at| source_timestamp(Some(at), "")),
            "completed":false, "dueLocal":nonempty(&reminder.due), "dueDateOnly":date_only,
            "due":if date_only == Some(true) { None } else { source_timestamp(reminder.due_at.as_deref(), &reminder.due) }})
    }).collect();
    evidence(
        "reminders",
        json!({"app":"reminders","completed":false,"lists":"all_accessible","limit":50}),
        records,
        Some(reminders.total),
        reminders.complete(),
        describe_reminders(reminders),
    )
}

pub async fn run_evidence(
    tool: &str,
    args: &Map<String, Value>,
    helper_path: &Path,
    settings: &Settings,
) -> Result<Value, String> {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    match tool {
        "read_calendar" => {
            let day = args
                .get("day")
                .and_then(Value::as_str)
                .ok_or("A calendar read needs a date.")?;
            Ok(calendar_evidence(day, &events(helper_path, day).await?))
        }
        "read_reminders" => Ok(reminder_evidence(&reminders(helper_path).await?)),
        "read_mail" => {
            read_mail_evidence(
                get("query"),
                args.get("unread").and_then(Value::as_bool).unwrap_or(false),
                args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize,
            )
            .await
        }
        "find_notes" => find_notes_evidence(get("query")).await,
        "find_contact" => find_contact(helper_path, get("name")).await,
        "control_music" if get("action") == "now_playing" => {
            let selected = if get("app").is_empty() {
                "music"
            } else {
                get("app")
            };
            let output = osascript(
                if selected == "spotify" {
                    SPOTIFY
                } else {
                    MUSIC
                },
                &["read_evidence".into(), String::new()],
                20,
            )
            .await?;
            music_evidence(selected, &output)
        }
        "browser_tabs" => browser_tabs_evidence(&browsers_for(settings, args), get("query")).await,
        "read_page" => page_evidence(&osascript(SAFARI_PAGE, &[], 20).await?),
        // Mutating controls retain their action receipt flow. They are not reads.
        _ => run(tool, args, helper_path, settings).await.map(|text| {
            evidence(
                "app_control",
                json!({"tool":tool,"arguments":args}),
                vec![],
                None,
                false,
                text,
            )
        }),
    }
}

/// Model context needs one copy of each source record. The durable journal
/// keeps the complete envelope for display, validation and provenance.
pub fn model_evidence(value: &Value) -> Value {
    let mut projected = value.clone();
    if value["schemaVersion"] == 1
        && value["records"]
            .as_array()
            .is_some_and(|records| records.iter().all(Value::is_object))
        && value["kind"].as_str().is_some_and(|kind| {
            [
                "calendar",
                "reminders",
                "mail",
                "notes",
                "browser_tabs",
                "browser_page",
                "music",
                "contacts",
            ]
            .contains(&kind)
        })
    {
        let fields = projected.as_object_mut().unwrap();
        fields.remove("events");
        fields.remove("text");
    }
    projected
}

/// Validate saved observations before they satisfy a task outcome. This checks
/// source identity, scope and coverage; it does not establish semantic truth of
/// source text. Legacy untyped observations must be read again, not promoted.
pub fn validate_evidence(
    tool: &str,
    args: &Value,
    value: &Value,
    require_complete: bool,
) -> Result<(), String> {
    let invalid = || {
        format!("The saved {tool} observation is incomplete or does not match its requested source. Read it again before continuing.")
    };
    let kind = match tool {
        "read_calendar" => "calendar",
        "read_reminders" => "reminders",
        "read_mail" => "mail",
        "find_notes" => "notes",
        "find_contact" => "contacts",
        "browser_tabs" => "browser_tabs",
        "read_page" => "browser_page",
        "control_music" if args["action"] == "now_playing" => "music",
        _ => return Err(invalid()),
    };
    let valid_stamp = |stamp: &Value| {
        stamp
            .as_str()
            .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
    };
    let nullable_text =
        |field: &Value| field.is_null() || field.as_str().is_some_and(|s| !s.is_empty());
    if value["kind"] != kind
        || value["schemaVersion"] != 1
        || !valid_stamp(&value["observedAt"])
        || !value["text"].is_string()
        || !value["scope"].is_object()
    {
        return Err(invalid());
    }
    let records = value["records"].as_array().ok_or_else(invalid)?;
    let count = value["count"].as_u64().ok_or_else(invalid)?;
    let total = value["total"].as_u64();
    let complete = value["complete"].as_bool().ok_or_else(invalid)?;
    if count != records.len() as u64
        || total.is_some_and(|total| total < count)
        || complete && total != Some(count)
        || !value
            .get("total")
            .is_some_and(|v| v.is_null() || v.is_u64())
        || kind != "browser_tabs" && (total.is_none() || complete != (total == Some(count)))
    {
        return Err(invalid());
    }
    if require_complete && !complete {
        return Err(format!("The {tool} observation contains only {count} of {} matching records. Buntini cannot verify this task from partial source coverage.", total.map(|n|n.to_string()).unwrap_or_else(||"an unknown number of".into())));
    }
    let scope = &value["scope"];
    let query = args["query"].as_str().unwrap_or("");
    let app = match kind {
        "calendar" => "calendar",
        "reminders" => "reminders",
        "mail" => "mail",
        "notes" => "notes",
        "contacts" => "contacts",
        "browser_page" => "safari",
        "music" => args["app"].as_str().unwrap_or("music"),
        _ => "",
    };
    if kind != "browser_tabs" && scope["app"] != app {
        return Err(invalid());
    }
    match kind {
        "calendar" => {
            let day = args["day"].as_str().ok_or_else(invalid)?;
            if chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err()
                || value["day"] != day
                || scope["day"] != day
                || value["events"] != value["records"]
                || scope["calendars"] != "all_accessible"
                || scope["limit"] != 40
                || count > 40
            {
                return Err(invalid());
            }
        }
        "reminders"
            if scope["completed"] != false
                || scope["lists"] != "all_accessible"
                || scope["limit"] != 50
                || count > 50 =>
        {
            return Err(invalid())
        }
        "mail" => {
            let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, 8);
            let scanned = scope["metadataScanned"].as_u64().ok_or_else(invalid)?;
            if scope["query"] != query
                || scope["unreadOnly"] != args["unread"].as_bool().unwrap_or(false)
                || scope["mailbox"] != "inbox"
                || scope["lookbackDays"] != if query.is_empty() { 14 } else { 120 }
                || scope["limit"] != limit
                || scope["metadataScanLimit"] != 300
                || scope["excerptLimit"] != 500
                || count > limit
                || scanned < count
                || scanned > 300
                || total.is_some_and(|t| t < scanned)
            {
                return Err(invalid());
            }
        }
        "notes"
            if scope["query"] != query
                || scope["matches"] != "title_or_plaintext"
                || scope["limit"] != 6
                || scope["excerptLimit"] != 400
                || count > 6 =>
        {
            return Err(invalid())
        }
        "browser_tabs" => {
            let browsers = scope["browsers"].as_array().ok_or_else(invalid)?;
            if scope["query"] != query
                || scope["limit"] != 40
                || scope["perBrowserLimit"] != 80
                || count > 40
                || browsers
                    .iter()
                    .any(|b| b.as_str().is_none_or(|b| !BROWSERS.contains(&b)))
                || args["browser"]
                    .as_str()
                    .is_some_and(|b| browsers != &vec![json!(b)])
            {
                return Err(invalid());
            }
        }
        "browser_page"
            if scope["surface"] != "front_document" || scope["textLimit"] != 2500 || count > 1 =>
        {
            return Err(invalid())
        }
        "contacts"
            if args["name"].as_str().is_none_or(str::is_empty)
                || scope["name"] != args["name"]
                || scope["matches"] != "name_or_email"
                || scope["limit"] != CONTACT_LIMIT as u64
                || count > CONTACT_LIMIT as u64 =>
        {
            return Err(invalid())
        }
        "music" => {
            let state = value["state"].as_str().ok_or_else(invalid)?;
            if !["music", "spotify"].contains(&app)
                || scope["action"] != "now_playing"
                || !["playing", "paused", "closed", "stopped"].contains(&state)
                || count != u64::from(["playing", "paused"].contains(&state))
            {
                return Err(invalid());
            }
        }
        _ => {}
    }
    let mut ids = std::collections::HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if !record.is_object() || record.get("id").is_none() || !nullable_text(&record["id"]) {
            return Err(invalid());
        }
        let reference = &record["reference"];
        let record_app = if kind == "browser_tabs" {
            record["browser"].as_str().ok_or_else(invalid)?
        } else {
            app
        };
        if reference["app"] != record_app || reference["mustRevalidate"] != true {
            return Err(invalid());
        }
        match record["id"].as_str() {
            Some(id) => {
                let expected_kind = if kind == "browser_tabs" {
                    "browser_session"
                } else {
                    "app_local"
                };
                if reference["kind"] != expected_kind
                    || reference["id"] != id
                    || !ids.insert((
                        record_app,
                        id,
                        if kind == "calendar" {
                            record["start"].as_str().unwrap_or("")
                        } else {
                            ""
                        },
                    ))
                {
                    return Err(invalid());
                }
            }
            None => {
                if reference["kind"] != "snapshot" {
                    return Err(invalid());
                }
                if !["browser_tabs", "browser_page"].contains(&kind)
                    && reference["index"] != index as u64
                {
                    return Err(invalid());
                }
            }
        }
        match kind {
            "calendar" => {
                if !record["title"].is_string()
                    || !record["location"].is_string()
                    || !record["all_day"].is_boolean()
                    || !valid_stamp(&record["start"])
                    || !valid_stamp(&record["end"])
                {
                    return Err(invalid());
                }
                let start = chrono::DateTime::parse_from_rfc3339(record["start"].as_str().unwrap())
                    .map_err(|_| invalid())?;
                let end = chrono::DateTime::parse_from_rfc3339(record["end"].as_str().unwrap())
                    .map_err(|_| invalid())?;
                if end <= start {
                    return Err(invalid());
                }
            }
            "reminders" => {
                if !record["title"].is_string()
                    || !record["list"].is_string()
                    || record["completed"] != false
                    || !nullable_text(&record["dueLocal"])
                    || !(record["dueDateOnly"].is_null() || record["dueDateOnly"].is_boolean())
                    || !(record["due"].is_null() || valid_stamp(&record["due"]))
                    || record["dueDateOnly"] == true && !record["due"].is_null()
                {
                    return Err(invalid());
                }
            }
            "mail" | "notes" => {
                let limit = if kind == "mail" { 500 } else { 400 };
                let length = record["sourceCharacterCount"].as_u64();
                let excerpt = record["excerpt"].as_str();
                if !(record["excerpt"].is_null() || excerpt.is_some())
                    || excerpt.is_some_and(|s| s.chars().count() > limit)
                    || record["excerptLimit"] != limit as u64
                    || !record["contentComplete"].is_boolean()
                    || excerpt.is_some() != length.is_some()
                    || excerpt.is_some()
                        && (record["contentComplete"] != (length.unwrap() <= limit as u64)
                            || record["excerptTruncated"] != (length.unwrap() > limit as u64))
                    || excerpt.is_none()
                        && (record["contentComplete"] != false
                            || !record["excerptTruncated"].is_null())
                {
                    return Err(invalid());
                }
                if kind == "mail" {
                    if !record["sender"].is_string()
                        || !record["subject"].is_string()
                        || !record["read"].is_boolean()
                        || scope["unreadOnly"] == true && record["read"] != false
                        || record["mailbox"] != "inbox"
                    {
                        return Err(invalid());
                    }
                } else if !record["title"].is_string() || excerpt.is_none() {
                    return Err(invalid());
                }
            }
            "browser_tabs" => {
                if !scope["browsers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b == record_app)
                    || !record["title"].is_string()
                    || !nullable_text(&record["url"])
                    || record["windowIndex"].as_u64().is_none_or(|n| n == 0)
                    || record["tabIndex"].as_u64().is_none_or(|n| n == 0)
                    || !(record["showing"].is_null() || record["showing"].is_boolean())
                    || (record_app == "safari" && !record["private"].is_null())
                    || (record_app != "safari" && record["private"] != false)
                {
                    return Err(invalid());
                }
            }
            "browser_page" => {
                let length = record["sourceCharacterCount"]
                    .as_u64()
                    .ok_or_else(invalid)?;
                if !record["title"].is_string()
                    || !nullable_text(&record["url"])
                    || record["text"]
                        .as_str()
                        .is_none_or(|s| s.chars().count() > 2500)
                    || record["textLimit"] != 2500
                    || record["textTruncated"] != (length > 2500)
                    || record["contentComplete"] != (length <= 2500)
                    || !record["private"].is_null()
                    || reference["surface"] != "front_document"
                {
                    return Err(invalid());
                }
            }
            "contacts" => {
                let listed = |key: &str, valid: fn(&str) -> bool| {
                    record[key].as_array().is_some_and(|items| {
                        items.len() <= CONTACT_DETAILS
                            && items.iter().all(|item| item.as_str().is_some_and(valid))
                    })
                };
                if !record["name"].is_string()
                    || !listed("emails", plain_address)
                    || !listed("phones", |phone| !phone.trim().is_empty())
                {
                    return Err(invalid());
                }
            }
            "music"
                if !record["title"].is_string()
                    || !record["artist"].is_string()
                    || !record["album"].is_string()
                    || record["state"] != value["state"] =>
            {
                return Err(invalid())
            }
            _ => {}
        }
    }
    Ok(())
}

/// The summary makes a dropped last line distinguishable from an empty app.
/// Source rows contain flattened tabs/newlines and are treated solely as data.
struct CountedRows {
    rows: Vec<Vec<String>>,
    total: usize,
    source_complete: bool,
}
fn counted_rows(output: &str, columns: usize, source: &str) -> Result<CountedRows, String> {
    let mut parsed = rows(output);
    let invalid = || format!("{source} returned incomplete data. Please try again.");
    let summary = parsed.pop().ok_or_else(invalid)?;
    if summary.len() != 4 || summary[0] != "__buntini_summary__" {
        return Err(invalid());
    }
    let total = summary[1].parse::<usize>().map_err(|_| invalid())?;
    let returned = summary[2].parse::<usize>().map_err(|_| invalid())?;
    let source_complete = match summary[3].as_str() {
        "true" => true,
        "false" => false,
        _ => return Err(invalid()),
    };
    if returned != parsed.len() || returned > total || parsed.iter().any(|row| row.len() != columns)
    {
        return Err(invalid());
    }
    Ok(CountedRows {
        rows: parsed,
        total,
        source_complete,
    })
}

pub fn describe_reminders(reminders: &Agenda<Reminder>) -> String {
    if reminders.total == 0 {
        return "No open reminders.".into();
    }
    let lines = reminders.items.iter().map(|reminder| {
        let due = if reminder.due.is_empty() {
            String::new()
        } else {
            format!(" (due {})", when(&reminder.due))
        };
        format!("- {}{due} [{}]", reminder.title, reminder.list)
    });
    let coverage = if reminders.complete() {
        String::new()
    } else {
        format!("\nPartial reminders: showing the first {} of {} open reminders, soonest due first. Other reminders are not included.", reminders.items.len(), reminders.total)
    };
    format!(
        "Open reminders:\n{}{coverage}",
        lines.collect::<Vec<_>>().join("\n")
    )
}

/// Recent inbox metadata, with explicitly bounded excerpts and search coverage.
async fn read_mail_evidence(query: &str, unread: bool, limit: usize) -> Result<Value, String> {
    let days = if query.is_empty() { "14" } else { "120" };
    let output = osascript(
        MAIL_LIST,
        &[
            query.into(),
            if unread { "1" } else { "0" }.into(),
            days.into(),
        ],
        40,
    )
    .await?;
    let found = counted_rows(&output, 6, "Mail")?;
    let selected = select_mail_rows(&found, unread, limit)?;
    let ids: Vec<String> = selected
        .iter()
        .filter_map(|row| nonempty(&row[5]).map(str::to_string))
        .collect();
    let openings = if ids.is_empty() {
        Ok(String::new())
    } else {
        osascript(MAIL_TEXT, &ids, 40).await
    };
    mail_evidence(query, unread, limit, found, openings.as_deref().ok())
}

fn select_mail_rows(
    found: &CountedRows,
    unread: bool,
    limit: usize,
) -> Result<Vec<Vec<String>>, String> {
    if found
        .rows
        .iter()
        .any(|row| !["true", "false"].contains(&row[2].as_str()) || unread && row[2] != "false")
    {
        return Err("Mail returned an invalid read status. Please try again.".into());
    }
    let mut selected = found.rows.clone();
    selected.sort_by(|a, b| b[0].cmp(&a[0]));
    selected.truncate(limit.clamp(1, 8));
    Ok(selected)
}

fn mail_evidence(
    query: &str,
    unread: bool,
    limit: usize,
    found: CountedRows,
    openings: Option<&str>,
) -> Result<Value, String> {
    let selected = select_mail_rows(&found, unread, limit)?;
    let bodies: std::collections::HashMap<_, _> = rows(openings.unwrap_or(""))
        .into_iter()
        .filter_map(|row| {
            if row.len() == 3 {
                row[2]
                    .parse::<usize>()
                    .ok()
                    .map(|length| (row[0].clone(), (row[1].clone(), length)))
            } else {
                None
            }
        })
        .collect();
    let records: Vec<_> = selected.iter().enumerate().map(|(index, row)| {
        let body = bodies.get(&row[5]);
        json!({"id":nonempty(&row[5]), "messageId":nonempty(&row[1]),
            "reference":source_reference("mail", nonempty(&row[5]), index), "mailbox":"inbox",
            "receivedAt":source_timestamp(None, &row[0]), "receivedLocal":row[0], "read":row[2] == "true",
            "sender":row[3], "subject":row[4], "excerpt":body.map(|(text, _)| text),
            "excerptLimit":500, "sourceCharacterCount":body.map(|(_, length)| length),
            "contentComplete":body.is_some_and(|(_, length)| *length <= 500),
            "excerptTruncated":body.map(|(_, length)| *length > 500),
            "contentUnavailableReason":if body.is_some() { None } else if openings.is_none() { Some("body_read_failed") } else { Some("body_not_returned") }})
    }).collect();
    let complete = found.source_complete && selected.len() == found.total;
    let days = if query.is_empty() { 14 } else { 120 };
    let mut display = if selected.is_empty() {
        match (query.is_empty(), unread) {
            (false, _) => {
                format!("No email in the inbox from the last four months matches “{query}”.")
            }
            (true, true) => "No unread email in the inbox from the last two weeks.".into(),
            _ => "No email in the inbox from the last two weeks.".into(),
        }
    } else {
        let lines = selected
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let status = if row[2] == "false" { " · unread" } else { "" };
                let opening = bodies
                    .get(&row[5])
                    .map(|(text, _)| format!("\n   {text}"))
                    .unwrap_or_default();
                format!(
                    "{}. From {} · {}{status}\n   Subject: {}{opening}",
                    index + 1,
                    row[3],
                    when(&row[0]),
                    row[4]
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Email in the inbox, returned messages newest first:\n{lines}")
    };
    if !complete {
        display += &format!("\nPartial inbox result: showing {} of {} matching messages. Search covers only the last {days} days.", selected.len(), found.total);
    }
    Ok(evidence(
        "mail",
        json!({"app":"mail","mailbox":"inbox","query":query,"unreadOnly":unread,
        "lookbackDays":days,"limit":limit.clamp(1,8),"metadataScanLimit":300,"metadataScanned":found.rows.len(),
        "order":if found.rows.len() == found.total { "newest_first" } else { "newest_in_scanned_prefix" },"excerptLimit":500}),
        records,
        Some(found.total),
        complete,
        display,
    ))
}

async fn read_mail(query: &str, unread: bool, limit: usize) -> Result<String, String> {
    Ok(read_mail_evidence(query, unread, limit).await?["text"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

#[cfg(test)]
fn mail_rows(output: &str, unread: bool, limit: usize) -> Vec<Vec<String>> {
    let mut found = rows(output);
    found.retain(|row| row.len() >= 5 && (!unread || row[2] == "false"));
    found.sort_by(|a, b| b[0].cmp(&a[0]));
    found.truncate(limit);
    found
}

async fn find_notes_evidence(query: &str) -> Result<Value, String> {
    let exact = notes_evidence(query, &osascript(NOTES_FIND, &[query.into()], 40).await?)?;
    if exact["count"] != 0 {
        return Ok(exact);
    }
    let Some(fallback) = notes_query_fallback(query) else {
        return Ok(exact);
    };
    let output = osascript(NOTES_FIND, &[fallback.clone()], 40).await?;
    notes_fallback_evidence(query, &fallback, &output)
}

/// Models sometimes include the generic noun in "my Groceries note". Preserve
/// every identifying word and try removing only that trailing noun, and only
/// after the exact phrase produced zero matches. Never shorten arbitrary text.
fn notes_query_fallback(query: &str) -> Option<String> {
    let query = query.trim();
    let lower = query.to_ascii_lowercase();
    for suffix in [" notes", " note"] {
        if lower.ends_with(suffix) {
            let candidate = query[..query.len() - suffix.len()].trim();
            return (candidate.chars().count() >= 3).then(|| candidate.to_string());
        }
    }
    None
}

fn notes_fallback_evidence(query: &str, effective: &str, output: &str) -> Result<Value, String> {
    let mut result = notes_evidence(effective, output)?;
    result["scope"]["query"] = json!(query);
    result["scope"]["effectiveQuery"] = json!(effective);
    result["scope"]["queryStrategy"] = json!("exact_phrase_then_generic_note_suffix");
    result["scope"]["exactPhraseMatches"] = json!(0);
    Ok(result)
}

fn notes_evidence(query: &str, output: &str) -> Result<Value, String> {
    let found = counted_rows(output, 5, "Notes")?;
    let mut records = Vec::new();
    let mut lines = Vec::new();
    for (index, row) in found.rows.iter().enumerate() {
        let length = row[4]
            .parse::<usize>()
            .map_err(|_| "Notes returned an invalid excerpt length.")?;
        records.push(json!({"id":nonempty(&row[1]), "reference":source_reference("notes", nonempty(&row[1]), index),
            "modifiedAt":source_timestamp(None, &row[0]), "modifiedLocal":row[0], "title":row[2], "excerpt":row[3],
            "excerptLimit":400,"sourceCharacterCount":length,"excerptTruncated":length > 400,"contentComplete":length <= 400}));
        lines.push(format!(
            "- {} (edited {})\n  {}",
            row[2],
            when(&row[0]),
            row[3]
        ));
    }
    let complete = found.source_complete && records.len() == found.total;
    let mut display = if lines.is_empty() {
        format!("No notes mention “{query}”.")
    } else {
        format!("Notes mentioning “{query}”:\n{}", lines.join("\n"))
    };
    if !complete {
        display += &format!(
            "\nPartial notes result: showing {} of {} matching notes.",
            records.len(),
            found.total
        );
    }
    Ok(evidence(
        "notes",
        json!({"app":"notes","query":query,"matches":"title_or_plaintext","limit":6,"excerptLimit":400}),
        records,
        Some(found.total),
        complete,
        display,
    ))
}

async fn find_notes(query: &str) -> Result<String, String> {
    Ok(find_notes_evidence(query).await?["text"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

fn music_evidence(app: &str, output: &str) -> Result<Value, String> {
    let name = if app == "spotify" { "Spotify" } else { "Music" };
    let parts: Vec<_> = output.trim_end_matches(['\r', '\n']).split('\t').collect();
    let (state, records, display) = match parts.as_slice() {
        ["closed"] => (
            "closed",
            vec![],
            format!("{name} isn’t open, so nothing is playing."),
        ),
        ["stopped"] => ("stopped", vec![], format!("Nothing is playing in {name}.")),
        [state @ ("playing" | "paused"), track, artist, album, id, "__buntini_end__"] => {
            let record = json!({"id":nonempty(id), "reference":source_reference(app, nonempty(id), 0),
                "title":track, "artist":artist, "album":album, "state":state});
            (
                *state,
                vec![record],
                format!(
                    "{name} is {}: “{track}” by {artist}, from {album}.",
                    if *state == "paused" {
                        "paused on"
                    } else {
                        "playing"
                    }
                ),
            )
        }
        _ => {
            return Err(format!(
                "{name} returned an incomplete playback state. Please try again."
            ))
        }
    };
    let mut result = evidence(
        "music",
        json!({"app":app,"action":"now_playing"}),
        records,
        Some(usize::from(!["closed", "stopped"].contains(&state))),
        true,
        display,
    );
    result["state"] = json!(state);
    Ok(result)
}

pub(crate) async fn control_music(
    app: &str,
    action: &str,
    query: &str,
    retry: bool,
) -> Result<String, String> {
    let name = if app == "spotify" { "Spotify" } else { "Music" };
    let words = query.to_string();
    // On a retry, the song playing now is the wrong one.
    let avoid = if retry && app == "spotify" {
        osascript(SPOTIFY, &["now_playing".into(), String::new()], 10)
            .await
            .ok()
            .and_then(|now| {
                let parts: Vec<_> = now.trim().split('\t').collect();
                match parts.as_slice() {
                    [_, title, artist, _] if !title.is_empty() && !artist.is_empty() => {
                        Some(format!("{title} by {artist}"))
                    }
                    _ => None,
                }
            })
    } else {
        None
    };
    let query = if app == "spotify" && !query.is_empty() {
        format!("spotify:search:{}", percent(query))
    } else {
        query.to_string()
    };
    let result = osascript(
        if app == "spotify" { SPOTIFY } else { MUSIC },
        &[action.into(), query.clone()],
        20,
    )
    .await?;
    if result.trim() == "search" {
        return Ok(
            match crate::skills::music::press_play(&words, avoid).await {
                // Say what Spotify is actually playing, not what was asked for.
                Ok(pressed) => {
                    // Spotify takes a moment to switch; wait until it reports the new song.
                    let mut now = String::new();
                    let mut confirmed = false;
                    for _ in 0..8 {
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        now = osascript(SPOTIFY, &["now_playing".into(), String::new()], 10)
                            .await
                            .unwrap_or_default();
                        if matches_pressed_track(&pressed, &now) {
                            confirmed = true;
                            break;
                        }
                    }
                    match now.trim().split('\t').collect::<Vec<_>>().as_slice() {
                    ["playing", track, artist, _] if confirmed => format!("Playing “{track}” by {artist} in Spotify."),
                    ["playing", track, artist, _] => format!("Spotify is playing “{track}” by {artist}, but I couldn’t confirm that the requested song started."),
                    _ => format!("Pressed play for {pressed} in Spotify, but it hasn’t confirmed playback yet."),
                }
                }
                Err(why) => format!("Opened Spotify’s search for that, but didn’t start it: {why}"),
            },
        );
    }
    let parts: Vec<&str> = result.trim().split('\t').collect();
    Ok(match (action, parts.as_slice()) {
        (_, ["closed"]) => format!("{name} isn’t open, so nothing is playing."),
        (_, ["stopped"]) => format!("Nothing is playing in {name}."),
        ("now_playing", [state, track, artist, album]) => format!(
            "{name} is {}: “{track}” by {artist}, from {album}.",
            if *state == "paused" {
                "paused on"
            } else {
                "playing"
            }
        ),
        (_, ["playlist", playlist]) => format!("Playing the “{playlist}” playlist in {name}."),
        (_, ["track", track, artist]) => format!("Playing “{track}” by {artist} in {name}."),
        (_, ["notfound"]) => format!("Nothing in your {name} library matches that."),
        ("pause", _) => format!("Paused {name}."),
        ("next", _) => format!("Skipped to the next track in {name}."),
        ("previous", _) => format!("Went back a track in {name}."),
        _ => format!("{name} is playing."),
    })
}

fn matches_pressed_track(label: &str, state: &str) -> bool {
    let parts: Vec<_> = state.trim().split('\t').collect();
    match parts.as_slice() {
        ["playing", track, artist, _] if !track.is_empty() && !artist.is_empty() => {
            let label = label.to_lowercase();
            label.starts_with(&track.to_lowercase()) && label.contains(&artist.to_lowercase())
        }
        _ => false,
    }
}

fn tab_scripts(app: &App) -> (String, String) {
    let (list, switch) = match app.id {
        "safari" => (SAFARI_TABS, SAFARI_SWITCH),
        "arc" => (ARC_TABS, ARC_SWITCH),
        _ => (CHROMIUM_TABS, CHROMIUM_SWITCH),
    };
    (
        list.replace("APP", app.name),
        switch.replace("APP", app.name),
    )
}

fn window_privacy_script(app: &App) -> Option<String> {
    let check = match app.id {
        "arc" => "incognito of front window",
        "chrome" | "brave" | "edge" => "mode of front window is \"incognito\"",
        _ => return None,
    };
    Some(format!(
        r#"on run argv
  if application "{name}" is not running then return "none"
  tell application "{name}"
    if (count of windows) is 0 then return "none"
    if {check} then return "private"
    return "public"
  end tell
end run
"#,
        name = app.name
    ))
}

/// AX exposes private Chromium pages too. Check the browser's own privacy flag
/// before asking AX for content or acting on its current tab.
pub(crate) async fn ensure_public_window(app: &App) -> Result<(), String> {
    let Some(script) = window_privacy_script(app) else {
        return Ok(());
    };
    match osascript(&script, &[], 10).await?.trim() {
        "public" => Ok(()),
        "private" => Err(format!(
            "The front window in {} is private. Switch to a regular window first.",
            app.name
        )),
        "none" => Err(format!("Open a page in {} first.", app.name)),
        _ => Err(format!(
            "I couldn’t check whether the window in {} is private.",
            app.name
        )),
    }
}

/// A tab is a browser-session reference, not a permanent page identifier.
/// Snapshot indices help explain results but must be refreshed before acting.
fn tab_records(app: &App, output: &str) -> Result<(Vec<Value>, CountedRows), String> {
    let found = counted_rows(output, 7, app.name)?;
    let mut records = Vec::new();
    for row in &found.rows {
        let window = row[0]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("A browser returned an invalid window index.")?;
        let tab = row[1]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("A browser returned an invalid tab index.")?;
        let showing = match row[2].as_str() {
            "true" => Some(true),
            "false" => Some(false),
            "unknown" => None,
            _ => return Err("A browser returned an invalid selected-tab state.".into()),
        };
        let reference = match nonempty(&row[6]) {
            Some(id) => {
                json!({"kind":"browser_session","app":app.id,"id":id,"windowId":nonempty(&row[5]),"mustRevalidate":true})
            }
            None => {
                json!({"kind":"snapshot","app":app.id,"windowIndex":window,"tabIndex":tab,"mustRevalidate":true})
            }
        };
        records.push(json!({"id":nonempty(&row[6]),"reference":reference,"browser":app.id,"browserName":app.name,
            "windowIndex":window,"tabIndex":tab,"windowId":nonempty(&row[5]),"showing":showing,"title":row[3],"url":nonempty(&row[4]),
            "private":if app.id == "safari" { None } else { Some(false) }}));
    }
    Ok((records, found))
}

async fn browser_tabs_evidence(browsers: &[&App], query: &str) -> Result<Value, String> {
    let mut results = Vec::new();
    for app in browsers {
        results.push((
            *app,
            osascript(&tab_scripts(app).0, &[query.into()], 20).await,
        ));
    }
    tabs_evidence(browsers, query, results)
}

fn tabs_evidence(
    browsers: &[&App],
    query: &str,
    results: Vec<(&App, Result<String, String>)>,
) -> Result<Value, String> {
    let mut records = Vec::new();
    let mut problems = Vec::new();
    let mut total = 0;
    let mut source_complete = true;
    for (app, result) in results {
        match result.and_then(|output| tab_records(app, &output)) {
            Ok((mut tabs, found)) => {
                records.append(&mut tabs);
                total += found.total;
                source_complete &= found.source_complete;
            }
            Err(error) => {
                problems.push(format!("{}: {error}", app.name));
                source_complete = false;
            }
        }
    }
    if records.is_empty() && !problems.is_empty() {
        return Err(problems.join(" "));
    }
    records.truncate(40);
    let complete = source_complete && records.len() == total;
    let names = browsers
        .iter()
        .map(|app| app.name)
        .collect::<Vec<_>>()
        .join(", ");
    let privacy = if browsers.iter().any(|app| app.id == "safari") {
        "private windows are skipped where supported; Safari does not identify private windows"
    } else {
        "private windows are skipped"
    };
    let mut display = if records.is_empty() {
        if source_complete {
            if query.is_empty() {
                format!("No open tabs in {names} ({privacy}, and closed browsers aren’t opened).")
            } else {
                format!("No open tab in {names} mentions “{query}”.")
            }
        } else {
            format!("No matching tabs were returned from the readable windows in {names}; some windows could not be read.")
        }
    } else {
        let lines = records
            .iter()
            .map(|tab| {
                format!(
                    "- {} window {}{}: {} — {}",
                    tab["browserName"].as_str().unwrap_or(""),
                    tab["windowIndex"],
                    if tab["showing"] == true {
                        " (showing)"
                    } else {
                        ""
                    },
                    tab["title"].as_str().unwrap_or(""),
                    tab["url"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Open browser tabs{}, {privacy}:\n{lines}",
            if query.is_empty() {
                String::new()
            } else {
                format!(" mentioning “{query}”")
            }
        )
    };
    if !complete {
        display += &format!("\nPartial tab result: {} tabs returned; other tabs or unreadable windows may be omitted.", records.len());
    }
    if !problems.is_empty() {
        display += &format!("\nCouldn’t read {}", problems.join(" "));
    }
    let mut result = evidence(
        "browser_tabs",
        json!({"browsers":browsers.iter().map(|app|app.id).collect::<Vec<_>>(),
        "query":query,"limit":40,"perBrowserLimit":80,"privacy":privacy}),
        records,
        source_complete.then_some(total),
        complete,
        display,
    );
    result["errors"] = json!(problems);
    Ok(result)
}

/// Compatibility display used by the browser skill; data collection is shared.
pub(crate) async fn browser_tabs(browsers: &[&App], query: &str) -> Result<String, String> {
    Ok(browser_tabs_evidence(browsers, query).await?["text"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Brings the first matching tab to the front, in the first browser that has one.
pub(crate) async fn switch_tab(browsers: &[&App], query: &str) -> Result<String, String> {
    for app in browsers {
        let title = osascript(&tab_scripts(app).1, &[query.into()], 20).await?;
        if !title.trim().is_empty() {
            return Ok(format!("Switched to “{}” in {}.", title.trim(), app.name));
        }
    }
    Ok(format!("No open tab mentions “{query}”."))
}

struct PageSnapshot {
    title: String,
    url: String,
    text: String,
    original_length: usize,
}
fn parse_page(output: &str) -> Result<Option<PageSnapshot>, String> {
    if ["closed", "none"].contains(&output.trim()) {
        return Ok(None);
    }
    let (head, body) = output
        .split_once('\n')
        .ok_or("Safari returned incomplete page data.")?;
    let parts: Vec<_> = head.split('\t').collect();
    if parts.len() != 3 {
        return Err("Safari returned incomplete page data.".into());
    }
    let original_length = parts[2]
        .trim()
        .parse::<usize>()
        .map_err(|_| "Safari returned an invalid page length.")?;
    let text = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(8000)
        .collect();
    Ok(Some(PageSnapshot {
        title: parts[0].trim().into(),
        url: parts[1].trim().into(),
        text,
        original_length,
    }))
}

/// The compatibility view retains the larger 8,000-character snapshot used by
/// the browser skill. The model-facing read below advertises its smaller bound.
pub(crate) async fn safari_page() -> Result<Option<(String, String, String)>, String> {
    Ok(parse_page(&osascript(SAFARI_PAGE, &[], 20).await?)?
        .map(|page| (page.title, page.url, page.text)))
}

/// The email selected in Mail, for "this". Mail must be allowed in Settings → Apps.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedMail {
    pub id: String,
    pub message_id: String,
    pub received: String,
    pub sender: String,
    pub subject: String,
    pub text: String,
    pub truncated: bool,
    pub selected: usize,
}

pub(crate) async fn selected_mail() -> Result<Option<SelectedMail>, String> {
    parse_selected_mail(&osascript(MAIL_SELECTED, &[], 10).await?)
}

fn parse_selected_mail(output: &str) -> Result<Option<SelectedMail>, String> {
    if ["closed", "none", ""].contains(&output.trim()) {
        return Ok(None);
    }
    let (head, body) = output.split_once('\n').unwrap_or((output, ""));
    let parts: Vec<&str> = head.split('\t').map(str::trim).collect();
    let [received, message_id, sender, subject, id, length, selected] = parts[..] else {
        return Err("Mail returned incomplete message data.".into());
    };
    let length: usize = length
        .parse()
        .map_err(|_| "Mail returned an invalid message length.")?;
    let text: String = body.trim().chars().take(4000).collect();
    Ok(Some(SelectedMail {
        id: id.into(),
        message_id: message_id.into(),
        received: received.into(),
        sender: sender.into(),
        subject: subject.into(),
        truncated: length > text.chars().count(),
        text,
        selected: selected.parse().unwrap_or(1),
    }))
}

/// An unreplied inbox message, for "what needs a reply?".
#[derive(Debug, Clone, PartialEq)]
pub struct MailHeader {
    pub received: String,
    pub id: String,
    pub read: bool,
    pub flagged: bool,
    /// Whether a To recipient is one of the user's account addresses; None if unknown.
    pub to_me: Option<bool>,
    pub to_count: usize,
    pub sender: String,
    pub subject: String,
}

/// Unreplied inbox messages from the last `days` days (at most 60), and how many there were.
pub(crate) async fn unreplied_mail(days: u32) -> Result<(Vec<MailHeader>, usize), String> {
    parse_unreplied(&osascript(MAIL_TRIAGE, &[days.to_string()], 60).await?)
}

fn parse_unreplied(output: &str) -> Result<(Vec<MailHeader>, usize), String> {
    let found = counted_rows(output, 8, "Mail")?;
    let flag = |value: &str| match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Mail returned an invalid message status. Please try again.".to_string()),
    };
    let mut headers = Vec::new();
    for row in &found.rows {
        headers.push(MailHeader {
            received: row[0].clone(),
            id: nonempty(&row[1]).ok_or("Mail returned a message without an ID.")?.into(),
            read: flag(&row[2])?,
            flagged: flag(&row[3])?,
            to_me: if row[4] == "unknown" { None } else { Some(flag(&row[4])?) },
            to_count: row[5].parse().unwrap_or(0),
            sender: row[6].clone(),
            subject: row[7].clone(),
        });
    }
    headers.sort_by(|a, b| b.received.cmp(&a.received));
    Ok((headers, found.total))
}

/// The opening (up to 500 characters) of each message by Mail-local ID.
pub(crate) async fn mail_openings(
    ids: &[String],
) -> Result<std::collections::HashMap<String, String>, String> {
    if ids.is_empty() {
        return Ok(Default::default());
    }
    Ok(rows(&osascript(MAIL_TEXT, ids, 40).await?)
        .into_iter()
        .filter(|row| row.len() >= 2)
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect())
}

/// A record for the memory index: (time, ID, title, text).
pub type Recent = (String, String, String, String);

/// Notes changed in the last `days` days (at most 300). Notes must be allowed.
pub(crate) async fn recent_notes(days: u32) -> Result<Vec<Recent>, String> {
    let found = counted_rows(&osascript(NOTES_RECENT, &[days.to_string()], 120).await?, 4, "Notes")?;
    Ok(found
        .rows
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone(), row[2].clone(), row[3].clone()))
        .collect())
}

/// Inbox messages from the last `days` days, newest `limit` with their openings, as
/// (received, Mail ID, "subject — from sender", opening). Mail must be allowed.
pub(crate) async fn recent_mail(days: u32, limit: usize) -> Result<Vec<Recent>, String> {
    let output = osascript(MAIL_LIST, &["".into(), "0".into(), days.to_string()], 120).await?;
    let mut rows = counted_rows(&output, 6, "Mail")?.rows;
    rows.sort_by(|a, b| b[0].cmp(&a[0]));
    rows.truncate(limit);
    let ids: Vec<String> = rows.iter().filter_map(|row| nonempty(&row[5]).map(str::to_string)).collect();
    let mut openings = std::collections::HashMap::new();
    for batch in ids.chunks(40) {
        openings.extend(mail_openings(batch).await?);
    }
    Ok(rows
        .into_iter()
        .filter(|row| !row[5].is_empty())
        .map(|row| {
            let opening = openings.get(&row[5]).cloned().unwrap_or_default();
            (row[0].clone(), row[5].clone(), format!("{} — from {}", row[4], row[3]), opening)
        })
        .collect())
}

/// The paths selected in Finder, for "this". macOS asks once before Buntini may see them.
pub(crate) async fn selected_finder_items() -> Result<Vec<String>, String> {
    Ok(osascript(FINDER_SELECTED, &[], 10)
        .await?
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(str::to_string)
        .collect())
}

fn page_evidence(output: &str) -> Result<Value, String> {
    let mut records = Vec::new();
    let display = match parse_page(output)? {
        None => "No page is open in Safari.".into(),
        Some(page) => {
            let text: String = page.text.chars().take(2500).collect();
            let truncated = page.original_length > 2500;
            let display = format!(
                "The page showing in Safari: “{}” ({})\n{}{}",
                page.title,
                page.url,
                if text.is_empty() {
                    "The page has no readable text."
                } else {
                    &text
                },
                if truncated {
                    "\nPage text is a bounded excerpt; remaining content is omitted."
                } else {
                    ""
                }
            );
            records.push(json!({"id":Value::Null,"reference":{"kind":"snapshot","app":"safari","surface":"front_document","mustRevalidate":true},
                "title":page.title,"url":nonempty(&page.url),"text":text,"textLimit":2500,
                "sourceCharacterCount":page.original_length,"textTruncated":truncated,"contentComplete":!truncated,
                "private":Value::Null}));
            display
        }
    };
    let count = records.len();
    Ok(evidence(
        "browser_page",
        json!({"app":"safari","surface":"front_document","textLimit":2500,
        "privacy":"Safari does not identify private windows"}),
        records,
        Some(count),
        true,
        display,
    ))
}

async fn read_page() -> Result<String, String> {
    Ok(
        page_evidence(&osascript(SAFARI_PAGE, &[], 20).await?)?["text"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    )
}

/// Runs a read or playback call (already validated, for an allowed app) and returns
/// plain text for the model.
pub async fn run(
    tool: &str,
    args: &Map<String, Value>,
    helper_path: &Path,
    settings: &Settings,
) -> Result<String, String> {
    let get = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match tool {
        "read_mail" => {
            read_mail(
                &get("query"),
                args.get("unread").and_then(Value::as_bool).unwrap_or(false),
                args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize,
            )
            .await
        }
        "read_calendar" => Ok(describe_events(
            &get("day"),
            &events(helper_path, &get("day")).await?,
        )),
        "read_reminders" => Ok(describe_reminders(&reminders(helper_path).await?)),
        "find_notes" => find_notes(&get("query")).await,
        "find_contact" => Ok(find_contact(helper_path, &get("name")).await?["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()),
        "control_music" => control_music(&get("app"), &get("action"), &get("query"), false).await,
        "browser_tabs" => browser_tabs(&browsers_for(settings, args), &get("query")).await,
        "switch_tab" => switch_tab(&browsers_for(settings, args), &get("query")).await,
        "read_page" => read_page().await,
        _ => Err("Unknown app tool.".into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    id: &'static str,
    name: &'static str,
    can: &'static [&'static str],
    note: Option<&'static str>,
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

pub(crate) fn app_names() -> Vec<String> {
    let home = std::env::var("HOME")
        .map(|home| PathBuf::from(home).join("Applications"))
        .ok();
    let mut names: Vec<String> = [
        "/Applications",
        "/Applications/Utilities",
        "/System/Applications",
        "/System/Applications/Utilities",
    ]
    .iter()
    .map(PathBuf::from)
    .chain(home)
    .filter_map(|dir| std::fs::read_dir(dir).ok())
    .flatten()
    .flatten()
    .filter_map(|entry| {
        entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".app"))
            .map(str::to_string)
    })
    .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The apps Buntini can work with and whether each is allowed, plus the rest it can open.
#[tauri::command]
pub fn app_list(state: tauri::State<'_, crate::controller::AppState>) -> Result<AppList, String> {
    let settings = state.store.lock().unwrap().settings()?;
    let apps: Vec<AppStatus> = APPS
        .iter()
        .map(|app| AppStatus {
            id: app.id,
            name: app.name,
            can: app.can,
            note: app.note,
            installed: installed(app),
            allowed: allowed(&settings, app.id),
        })
        .collect();
    let others = app_names()
        .into_iter()
        .filter(|name| !APPS.iter().any(|app| app.name == name))
        .collect();
    Ok(AppList { apps, others })
}

/// Whether Buntini may use Accessibility (pressing play in Spotify, reading pages in
/// Chrome, Brave, Edge and Arc, locking the screen). With `ask`, macOS shows its prompt.
#[tauri::command]
pub async fn accessibility(ask: bool) -> bool {
    tokio::task::spawn_blocking(move || crate::ax::trusted(ask))
        .await
        .unwrap_or(false)
}

/// Asks macOS for permission right after the user allows Calendar or Reminders, so the
/// prompt appears in Settings rather than in the middle of a chat. Other apps are asked
/// the first time Buntini uses them.
#[tauri::command]
pub async fn check_app(
    id: String,
    state: tauri::State<'_, crate::controller::AppState>,
) -> Result<String, String> {
    let helper_path = state.models.apple_helper().to_path_buf();
    match id.as_str() {
        "calendar" => events(
            &helper_path,
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
        )
        .await
        .map(|_| "Calendar is ready.".into()),
        "reminders" => reminders(&helper_path)
            .await
            .map(|_| "Reminders is ready.".into()),
        "contacts" => contacts_access(&helper_path).await,
        // Pressing play on a song you name uses Accessibility; ask for it now.
        "spotify" => Ok(
            if tokio::task::spawn_blocking(|| crate::ax::trusted(true))
                .await
                .unwrap_or(false)
            {
                "Spotify is ready, including playing songs you name.".into()
            } else {
                "To play songs you name, allow Buntini under System Settings → Privacy & Security → Accessibility. Play, pause and skip work without it.".into()
            },
        ),
        _ => Ok(app(&id)
            .map(|app| {
                format!(
                    "macOS will ask you once, the first time Buntini uses {}.",
                    app.name
                )
            })
            .unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreplied_mail_rows_are_typed_and_sorted_newest_first() {
        let output = "2026-10-27 08:00\t11\ttrue\tfalse\ttrue\t1\tAna <ana@x.com>\tLease\n2026-10-28 10:00\t12\tfalse\ttrue\tunknown\t3\tnoreply@shop.com\tYour order\n__buntini_summary__\t5\t2\ttrue";
        let (headers, total) = parse_unreplied(output).unwrap();
        assert_eq!(total, 5);
        assert_eq!(headers[0].id, "12");
        assert_eq!(headers[0].to_me, None);
        assert!(headers[0].flagged && !headers[0].read);
        assert_eq!(headers[1].to_me, Some(true));
        assert!(parse_unreplied("2026-10-27 08:00\t11\tmaybe\tfalse\ttrue\t1\tA\tB\n__buntini_summary__\t1\t1\ttrue").is_err());
        assert!(parse_unreplied("__buntini_summary__\t1\t0\ttrue").unwrap().0.is_empty());
    }

    #[test]
    fn the_selected_email_is_parsed_or_refused_never_guessed() {
        assert_eq!(parse_selected_mail("none").unwrap(), None);
        assert_eq!(parse_selected_mail("closed\n").unwrap(), None);
        let found = parse_selected_mail(
            "2026-10-29 09:15\t<a@b>\tAna Silva <ana@example.com>\tLease renewal\t42\t5000\t2\nCan you sign\nby Friday?",
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.subject, "Lease renewal");
        assert_eq!(found.id, "42");
        assert_eq!(found.selected, 2);
        assert_eq!(found.text, "Can you sign\nby Friday?");
        assert!(found.truncated);
        assert!(parse_selected_mail("2026-10-29 09:15\t<a@b>\tAna").is_err());
        assert!(parse_selected_mail("a\tb\tc\td\te\tlots\t1\nbody").is_err());
    }

    #[test]
    fn notes_suffix_fallback_preserves_identifiers_and_reports_its_actual_query() {
        let literal = "BuntiniQA-01234567-0123-4567-8901-012345678901 Handoff";
        let original = format!("{literal} note");
        assert_eq!(notes_query_fallback(&original).as_deref(), Some(literal));
        assert_eq!(
            notes_query_fallback("Project Notes").as_deref(),
            Some("Project")
        );
        for query in [
            "note",
            "notes",
            "Footnotes",
            "Notebook",
            "Do not shorten other words",
            "x note",
        ] {
            assert_eq!(notes_query_fallback(query), None, "{query}");
        }
        let output = format!("2026-10-29 15:00\towned-note-id\t{literal}\tAlex owns the draft.\t20\n__buntini_summary__\t1\t1\ttrue");
        let found = notes_fallback_evidence(&original, literal, &output).unwrap();
        assert_eq!(found["scope"]["query"], original);
        assert_eq!(found["scope"]["effectiveQuery"], literal);
        assert_eq!(found["records"][0]["id"], "owned-note-id");
        assert!(validate_evidence("find_notes", &json!({"query":original}), &found, true).is_ok());
    }

    #[test]
    fn typed_calendar_preserves_source_ids_and_exact_dst_instants() {
        let agenda: Agenda<Event> = parse_helper_rows(&format!("{}\n{}", json!({
            "id":"event-17","calendarId":"work-4","calendar":"Work","modifiedAt":"2026-10-22T12:00:00Z",
            "start":"2026-11-01 01:30","end":"2026-11-01 01:45","startAt":"2026-11-01T06:30:00Z","endAt":"2026-11-01T06:45:00Z",
            "allDay":false,"title":"Repeated hour","location":"Office"}), json!({"type":"agenda_summary","total":1,"returned":1,"complete":true}))).unwrap();
        let value = calendar_evidence("2026-11-01", &agenda);
        assert_eq!(value["schemaVersion"], 1);
        assert!(
            chrono::DateTime::parse_from_rfc3339(value["observedAt"].as_str().unwrap()).is_ok()
        );
        assert_eq!(value["records"], value["events"]);
        assert_eq!(value["records"][0]["id"], "event-17");
        assert_eq!(value["records"][0]["reference"]["kind"], "app_local");
        assert_eq!(value["records"][0]["start"], "2026-11-01T06:30:00+00:00");
        assert_eq!(
            value["records"][0]["modifiedAt"],
            "2026-10-22T12:00:00+00:00"
        );
        assert!(
            validate_evidence("read_calendar", &json!({"day":"2026-11-01"}), &value, true).is_ok()
        );
    }

    #[test]
    fn typed_reminders_distinguish_date_only_due_times_and_missing_identity() {
        let agenda = Agenda {
            items: vec![
                Reminder {
                    id: Some("reminder-7".into()),
                    calendar_id: Some("home-2".into()),
                    title: "Bins".into(),
                    due: "2026-10-23".into(),
                    due_date_only: Some(true),
                    list: "Home".into(),
                    ..Default::default()
                },
                Reminder {
                    title: "Call".into(),
                    due: "2026-10-23 00:00".into(),
                    due_at: Some("2026-10-23T04:00:00Z".into()),
                    due_date_only: Some(false),
                    ..Default::default()
                },
                Reminder {
                    title: "Someday".into(),
                    ..Default::default()
                },
            ],
            total: 4,
        };
        let value = reminder_evidence(&agenda);
        assert_eq!(value["complete"], false);
        assert_eq!(value["total"], 4);
        assert_eq!(value["records"][0]["dueLocal"], "2026-10-23");
        assert!(
            value["records"][0]["due"].is_null(),
            "date-only reminders must not invent midnight"
        );
        assert_eq!(value["records"][1]["due"], "2026-10-23T04:00:00+00:00");
        assert_eq!(value["records"][1]["reference"]["kind"], "snapshot");
        assert!(value["records"][1]["id"].is_null());
        assert!(value["records"][2]["dueDateOnly"].is_null());
        assert!(validate_evidence("read_reminders", &json!({}), &value, false).is_ok());
    }

    #[test]
    fn counted_app_protocol_rejects_dropped_rows_and_false_empty_results() {
        let valid = "a\tb\n__buntini_summary__\t3\t1\ttrue\n";
        assert_eq!(counted_rows(valid, 2, "Test").unwrap().total, 3);
        for invalid in [
            "",
            "a\tb",
            "__buntini_summary__\t3\t1\ttrue",
            "a\tb\n__buntini_summary__\t0\t1\ttrue",
            "a\n__buntini_summary__\t1\t1\ttrue",
            "a\tb\n__buntini_summary__\t1\t1\tmaybe",
        ] {
            assert!(
                counted_rows(invalid, 2, "Test").is_err(),
                "accepted {invalid:?}"
            );
        }
        assert!(counted_rows("__buntini_summary__\t0\t0\ttrue", 2, "Test")
            .unwrap()
            .rows
            .is_empty());
    }

    #[test]
    fn mail_evidence_reports_query_bounds_body_truncation_and_unavailable_content() {
        let output = "2026-10-22 09:00\tm1\tfalse\tAlice\tInvoice\t101\n2026-10-22 10:00\tm2\tfalse\tBob\tLease\t102\n__buntini_summary__\t8\t2\ttrue";
        let value = mail_evidence(
            "lease",
            true,
            2,
            counted_rows(output, 6, "Mail").unwrap(),
            Some("102\tOpening\t701"),
        )
        .unwrap();
        assert_eq!(value["records"][0]["id"], "102");
        assert_eq!(value["records"][0]["messageId"], "m2");
        assert_eq!(value["records"][0]["excerptTruncated"], true);
        assert_eq!(value["records"][0]["contentComplete"], false);
        assert!(value["records"][1]["excerpt"].is_null());
        assert_eq!(
            value["records"][1]["contentUnavailableReason"],
            "body_not_returned"
        );
        assert_eq!(value["total"], 8);
        assert_eq!(value["complete"], false);
        assert_eq!(value["scope"]["lookbackDays"], 120);
        assert_eq!(value["scope"]["order"], "newest_in_scanned_prefix");
        assert!(validate_evidence(
            "read_mail",
            &json!({"query":"lease","unread":true,"limit":2}),
            &value,
            false
        )
        .is_ok());
        assert!(validate_evidence(
            "read_mail",
            &json!({"query":"unrelated","unread":true,"limit":2}),
            &value,
            false
        )
        .is_err());
        assert!(mail_evidence(
            "lease",
            true,
            2,
            counted_rows(&output.replace("\tfalse\t", "\ttrue\t"), 6, "Mail").unwrap(),
            None
        )
        .is_err());
    }

    #[test]
    fn notes_evidence_keeps_real_ids_and_marks_excerpt_coverage() {
        let value = notes_evidence("Packing", "2026-10-22 12:30\tx-coredata://note/7\tPacking\tPassport and adapter\t900\n__buntini_summary__\t2\t1\ttrue").unwrap();
        assert_eq!(value["records"][0]["id"], "x-coredata://note/7");
        assert_eq!(value["records"][0]["excerpt"], "Passport and adapter");
        assert_eq!(value["records"][0]["excerptTruncated"], true);
        assert_eq!(value["complete"], false);
        assert!(
            validate_evidence("find_notes", &json!({"query":"Packing"}), &value, false).is_ok()
        );
        let mut invented_complete = value.clone();
        invented_complete["records"][0]["contentComplete"] = json!(true);
        assert!(validate_evidence(
            "find_notes",
            &json!({"query":"Packing"}),
            &invented_complete,
            false
        )
        .is_err());
        assert!(notes_evidence(
            "x",
            "2026-10-22 12:30\tid\tx\tx\tnot-a-length\n__buntini_summary__\t1\t1\ttrue"
        )
        .is_err());
    }

    #[test]
    fn browser_evidence_distinguishes_session_ids_and_incomplete_private_visibility() {
        let chrome = app("chrome").unwrap();
        let safari = app("safari").unwrap();
        let chrome_rows =
            "1\t2\ttrue\tExample\thttps://example.test\t41\t98\n__buntini_summary__\t1\t1\ttrue";
        let safari_rows = "1\t1\ttrue\tPersonal\thttps://example.test/private\t21\t\n__buntini_summary__\t1\t1\tfalse";
        let value = tabs_evidence(
            &[chrome, safari],
            "",
            vec![
                (chrome, Ok(chrome_rows.into())),
                (safari, Ok(safari_rows.into())),
            ],
        )
        .unwrap();
        assert_eq!(value["records"][0]["reference"]["kind"], "browser_session");
        assert_eq!(value["records"][0]["id"], "98");
        assert_eq!(value["records"][0]["private"], false);
        assert_eq!(value["records"][1]["reference"]["kind"], "snapshot");
        assert!(value["records"][1]["id"].is_null() && value["records"][1]["private"].is_null());
        assert!(value["total"].is_null());
        assert_eq!(value["complete"], false);
        assert!(validate_evidence("browser_tabs", &json!({}), &value, false).is_ok());
        assert!(
            tabs_evidence(&[chrome], "", vec![(chrome, Err("not authorized".into()))]).is_err()
        );
    }

    #[test]
    fn page_evidence_marks_bounded_text_and_never_manufactures_identity() {
        let value = page_evidence(&format!(
            "Long page\thttps://example.test\t9000\n{}",
            "x".repeat(8000)
        ))
        .unwrap();
        assert_eq!(value["records"][0]["text"].as_str().unwrap().len(), 2500);
        assert_eq!(value["records"][0]["contentComplete"], false);
        assert_eq!(value["records"][0]["sourceCharacterCount"], 9000);
        assert!(value["records"][0]["id"].is_null());
        assert!(validate_evidence("read_page", &json!({}), &value, false).is_ok());
        assert_eq!(page_evidence("closed").unwrap()["count"], 0);
        assert!(page_evidence("Title\tURL\ntruncated").is_err());
        assert!(page_evidence("Title\tURL\tunknown\ntext").is_err());
    }

    #[test]
    fn playback_evidence_requires_a_complete_state_and_preserves_track_identity() {
        let value = music_evidence(
            "spotify",
            "paused\tSong\tArtist\tAlbum\tspotify:track:17\t__buntini_end__\n",
        )
        .unwrap();
        assert_eq!(value["state"], "paused");
        assert_eq!(value["records"][0]["id"], "spotify:track:17");
        assert!(validate_evidence(
            "control_music",
            &json!({"action":"now_playing","app":"spotify"}),
            &value,
            false
        )
        .is_ok());
        assert_eq!(music_evidence("music", "closed").unwrap()["count"], 0);
        assert!(music_evidence("music", "playing\tSong\tArtist\tAlbum").is_err());
        let missing =
            music_evidence("music", "playing\tSong\tArtist\tAlbum\t\t__buntini_end__").unwrap();
        assert_eq!(missing["records"][0]["reference"]["kind"], "snapshot");
    }

    #[test]
    fn saved_evidence_cannot_fake_a_complete_calendar_or_an_empty_app() {
        let args = json!({"day":"2026-10-23"});
        let valid = calendar_evidence(
            "2026-10-23",
            &Agenda {
                items: vec![],
                total: 0,
            },
        );
        assert!(validate_evidence("read_calendar", &args, &valid, true).is_ok());
        for bad in [
            Value::Null,
            json!("No events"),
            json!({"kind":"calendar","day":"2026-10-23","events":[],"total":0,"complete":true}),
        ] {
            assert!(validate_evidence("read_calendar", &args, &bad, true).is_err());
        }
        for (field, replacement) in [
            ("day", json!("2026-10-24")),
            ("kind", json!("notes")),
            ("records", json!([{}])),
            ("count", json!(2)),
            ("complete", json!(false)),
            ("observedAt", json!("yesterday")),
        ] {
            let mut bad = valid.clone();
            bad[field] = replacement;
            assert!(
                validate_evidence("read_calendar", &args, &bad, true).is_err(),
                "accepted changed {field}"
            );
        }
        let partial = calendar_evidence(
            "2026-10-23",
            &Agenda {
                items: vec![],
                total: 1,
            },
        );
        assert!(validate_evidence("read_calendar", &args, &partial, true).is_err());
        assert!(validate_evidence("read_calendar", &args, &partial, false).is_ok(), "partial coverage is a valid observation, but cannot satisfy a complete-calendar dependency");
        let notes = notes_evidence("x", "__buntini_summary__\t0\t0\ttrue").unwrap();
        assert!(validate_evidence("find_notes", &json!({"query":"x"}), &notes, false).is_ok());
        let mut fake_empty = notes;
        fake_empty["total"] = json!(1);
        assert!(
            validate_evidence("find_notes", &json!({"query":"x"}), &fake_empty, false).is_err()
        );
    }

    #[test]
    fn model_projection_preserves_source_records_and_never_strips_receipts_or_errors() {
        let source = calendar_evidence(
            "2026-10-23",
            &Agenda {
                items: vec![Event {
                    id: Some("event-7".into()),
                    start: "2026-10-23 09:00".into(),
                    end: "2026-10-23 09:30".into(),
                    title: "Appointment".into(),
                    ..Default::default()
                }],
                total: 1,
            },
        );
        let projected = model_evidence(&source);
        assert!(projected.get("events").is_none() && projected.get("text").is_none());
        for field in [
            "records",
            "count",
            "total",
            "complete",
            "scope",
            "observedAt",
            "day",
            "schemaVersion",
        ] {
            assert_eq!(projected[field], source[field]);
        }
        assert!(
            source.get("events").is_some() && source.get("text").is_some(),
            "stored evidence is unchanged"
        );
        for value in [
            json!({"receipt":{"outcome":"unknown","objectId":"destination-7"},"text":"Preserve receipt text"}),
            json!({"error":"Permission denied","retryAllowed":true,"text":"Preserve error text"}),
            json!({"kind":"calendar","records":[],"text":"unversioned legacy evidence"}),
            json!("Plain source text"),
        ] {
            assert_eq!(model_evidence(&value), value);
        }
    }

    #[test]
    fn calendar_planning_evidence_keeps_offset_and_source_fields() {
        let agenda = Agenda {
            items: vec![Event {
                start: "2026-10-23 09:00".into(),
                end: "2026-10-23 09:30".into(),
                all_day: false,
                title: "Standup".into(),
                location: "Office".into(),
                ..Default::default()
            }],
            total: 1,
        };
        let evidence = calendar_evidence("2026-10-23", &agenda);
        let observed =
            chrono::DateTime::parse_from_rfc3339(evidence["events"][0]["start"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            observed
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            agenda.items[0].start
        );
        assert_eq!(evidence["events"][0]["title"], "Standup");
        assert_eq!(evidence["events"][0]["location"], "Office");
        assert_eq!(evidence["complete"], true);
        assert_eq!(evidence["total"], 1);
    }

    #[test]
    fn calls_are_checked_and_normalised() {
        let mail = validate("read_mail", &json!({"query":" invoice ","limit":40})).unwrap();
        assert_eq!(mail["query"], "invoice");
        assert_eq!(mail["limit"], 8, "at most eight messages");
        assert!(
            validate("read_mail", &json!({"folder":"Sent"})).is_err(),
            "unknown keys are refused"
        );
        assert!(validate("read_calendar", &json!({"day":"tomorrow"})).is_err());
        assert!(
            validate("read_calendar", &json!({})).unwrap()["day"]
                .as_str()
                .unwrap()
                .len()
                == 10,
            "today by default"
        );
        assert!(validate("find_notes", &json!({})).is_err());
        assert!(validate("control_music", &json!({"app":"vlc","action":"play"})).is_err());
        assert!(validate("control_music", &json!({"action":"shuffle"})).is_err());
        let play = validate(
            "control_music",
            &json!({"app":"spotify","action":"pause","query":"jazz"}),
        )
        .unwrap();
        assert!(play.get("query").is_none(), "only play takes a query");
        assert!(validate("send_mail", &json!({})).is_err());
    }

    #[test]
    fn contact_lookups_are_typed_bounded_and_checked() {
        assert_eq!(validate("find_contact", &json!({"name":" Ana Silva "})).unwrap()["name"], "Ana Silva");
        for invalid in [json!({}), json!({"name":""}), json!({"name":"--events"}), json!({"name":"a".repeat(101)}), json!({"name":"Ana","limit":3})] {
            assert!(validate("find_contact", &invalid).is_err(), "{invalid}");
        }
        assert_eq!(for_call("find_contact", &Map::new()).unwrap().id, "contacts");
        assert_eq!(app("contacts").unwrap().share, Some(Share::Contacts));
        // Addresses that could not address a draft are dropped, so records and text agree.
        let output = json!({"contacts":[
            {"id":"c-1","name":"Ana Silva","emails":["ana@example.com","Ana Silva <ana@x.com>","nobody"],"phones":["+1 555 0100",""]},
            {"id":null,"name":"Ana Costa","emails":[],"phones":[]}
        ],"total":2,"returned":2,"complete":true}).to_string();
        let found = parse_contacts(&output).unwrap();
        assert_eq!(found.items[0].emails, ["ana@example.com"]);
        assert_eq!(found.items[0].phones, ["+1 555 0100"]);
        let args = json!({"name":"Ana"});
        let evidence = contact_evidence("Ana", &found);
        assert!(validate_evidence("find_contact", &args, &evidence, true).is_ok());
        assert_eq!(evidence["records"][1]["reference"]["kind"], "snapshot");
        assert!(evidence["text"].as_str().unwrap().contains("Ana Silva: ana@example.com, +1 555 0100"));
        assert!(model_evidence(&evidence).get("text").is_none());
        // A read for another name, a smuggled address or a hidden extra person is refused.
        assert!(validate_evidence("find_contact", &json!({"name":"Eve"}), &evidence, false).is_err());
        let mut smuggled = evidence.clone();
        smuggled["records"][0]["emails"] = json!(["Ana <eve@example.com>"]);
        assert!(validate_evidence("find_contact", &args, &smuggled, false).is_err());
        let mut uncounted = evidence.clone();
        uncounted["count"] = json!(1);
        assert!(validate_evidence("find_contact", &args, &uncounted, false).is_err());
        // More people matched than were read: the read says so and is never complete.
        let five: Vec<Value> = (0..5).map(|i| json!({"id":format!("c-{i}"),"name":"Ana","emails":[],"phones":[]})).collect();
        let partial = parse_contacts(&json!({"contacts":five,"total":6,"returned":5,"complete":false}).to_string()).unwrap();
        assert!(!partial.complete());
        let read = contact_evidence("Ana", &partial);
        assert!(validate_evidence("find_contact", &args, &read, false).is_ok());
        assert!(validate_evidence("find_contact", &args, &read, true).is_err());
        assert!(read["text"].as_str().unwrap().contains("first 5 of 6 people"));
        assert!(parse_contacts(&json!({"contacts":five,"total":6,"returned":5,"complete":true}).to_string()).is_err(), "a missing person cannot be called complete");
        let six: Vec<Value> = (0..6).map(|i| json!({"id":format!("c-{i}"),"name":"Ana","emails":[],"phones":[]})).collect();
        assert!(parse_contacts(&json!({"contacts":six,"total":6,"returned":6,"complete":true}).to_string()).is_err(), "at most five people per read");
        for broken in ["", "[]", "{\"contacts\":[],\"total\":1,\"returned\":0,\"complete\":true}", "{\"contacts\":[],\"total\":0,\"returned\":0,\"complete\":true,\"extra\":1}"] {
            assert!(parse_contacts(broken).is_err(), "{broken}");
        }
        assert_eq!(parse_contacts("{\"error\":\"denied\"}").unwrap_err(), CONTACTS_DENIED);
        let nobody = parse_contacts("{\"contacts\":[],\"total\":0,\"returned\":0,\"complete\":true}").unwrap();
        assert_eq!(describe_contacts("Zed", &nobody), "No one in Contacts matches “Zed”.");
        assert!(validate_evidence("find_contact", &json!({"name":"Zed"}), &contact_evidence("Zed", &nobody), true).is_ok());
    }

    #[test]
    fn only_allowed_apps_are_usable() {
        let mut settings = Settings::default();
        let args = Map::new();
        assert!(
            !permits(&settings, "draft_email", &args),
            "nothing is allowed until the user says so"
        );
        assert!(
            permits(&settings, "open_app", &args),
            "opening an app is asked each time instead"
        );
        settings.apps = vec!["mail".into()];
        let mail = APPS.iter().find(|app| app.id == "mail").unwrap();
        assert_eq!(permits(&settings, "read_mail", &args), installed(mail));
        let spotify = validate("control_music", &json!({"app":"spotify","action":"next"})).unwrap();
        assert!(
            !permits(&settings, "control_music", &spotify),
            "Music being allowed says nothing about Spotify"
        );
        assert_eq!(for_call("add_reminder", &args).unwrap().id, "reminders");
    }

    #[test]
    fn tab_calls_use_only_allowed_browsers() {
        assert!(
            validate("switch_tab", &json!({})).is_err(),
            "switching needs words to look for"
        );
        assert!(validate("browser_tabs", &json!({"browser":"firefox"})).is_err());
        assert!(
            validate("read_page", &json!({"url":"https://example.com"})).is_err(),
            "only the page already showing"
        );
        let mut settings = Settings::default();
        let any = validate("browser_tabs", &json!({})).unwrap();
        assert!(!permits(&settings, "browser_tabs", &any));
        settings.apps = BROWSERS.iter().map(|id| id.to_string()).collect();
        let installed_browsers = APPS
            .iter()
            .filter(|app| BROWSERS.contains(&app.id) && installed(app))
            .count();
        assert_eq!(browsers_for(&settings, &any).len(), installed_browsers);
        let chrome = validate("browser_tabs", &json!({"browser":"chrome"})).unwrap();
        assert!(browsers_for(&settings, &chrome)
            .iter()
            .all(|app| app.id == "chrome"));
        for app in APPS.iter().filter(|app| BROWSERS.contains(&app.id)) {
            let (list, switch) = tab_scripts(app);
            assert!(
                list.contains(&format!("application \"{}\"", app.name))
                    && !list.contains("\"APP\"")
            );
            assert!(
                app.id == "safari" || list.contains("incognito") && switch.contains("incognito"),
                "{} skips private windows",
                app.name
            );
        }
    }

    #[test]
    fn results_read_plainly() {
        let events = Agenda {
            items: vec![
                Event {
                    start: "2026-10-22 09:00".into(),
                    end: "2026-10-22 09:15".into(),
                    all_day: false,
                    title: "Standup".into(),
                    location: String::new(),
                    ..Default::default()
                },
                Event {
                    start: "2026-10-22 00:00".into(),
                    end: "2026-10-23 00:00".into(),
                    all_day: true,
                    title: "Ana’s birthday".into(),
                    location: "Porto".into(),
                    ..Default::default()
                },
            ],
            total: 2,
        };
        assert_eq!(describe_events("2026-10-22", &events), "Calendar for Thursday 22 October:\n- 09:00–09:15: Standup\n- all day: Ana’s birthday at Porto");
        assert_eq!(
            describe_events(
                "2026-10-22",
                &Agenda {
                    items: vec![],
                    total: 0
                }
            ),
            "Nothing on the calendar for Thursday 22 October."
        );
        let reminders = Agenda {
            items: vec![Reminder {
                title: "Bins".into(),
                due: "2026-10-22 19:00".into(),
                list: "Home".into(),
                ..Default::default()
            }],
            total: 1,
        };
        assert_eq!(
            describe_reminders(&reminders),
            "Open reminders:\n- Bins (due Thu 22 Oct 19:00) [Home]"
        );
        assert_eq!(percent("lo-fi beats"), "lo%2Dfi%20beats");
    }

    #[test]
    fn app_reads_reject_corrupt_or_incomplete_helper_data() {
        let row = json!({"start":"2026-10-22 09:00", "end":"2026-10-22 10:00", "allDay":false, "title":"x".repeat(400), "location":"Room"});
        let rows = std::iter::repeat_n(row.to_string(), 30)
            .collect::<Vec<_>>()
            .join("\n");
        let output = format!(
            "{rows}\n{}",
            json!({"type":"agenda_summary","returned":30,"total":30,"complete":true})
        );
        assert!(output.len() > 4000);
        let events: Agenda<Event> = parse_helper_rows(&output).unwrap();
        assert_eq!(events.items.len(), 30);
        assert!(
            parse_helper_rows::<Event>(&output[..output.len() - 10]).is_err(),
            "truncation must not become missing events"
        );
        assert!(parse_helper_rows::<Event>(r#"{"title":"Meeting"}"#).is_err());
        assert!(
            parse_helper_rows::<Reminder>(r#"{"title":"Bins","due":false,"list":"Home"}"#).is_err()
        );
        assert!(parse_helper_rows::<Reminder>("warning: no access").is_err());
        assert!(
            parse_helper_rows::<Event>("").is_err(),
            "an absent completion record cannot prove an empty calendar"
        );
        assert!(
            parse_helper_rows::<Event>(&rows).is_err(),
            "a dropped final line cannot prove completeness"
        );
        let empty: Agenda<Event> = parse_helper_rows(
            r#"{"type":"agenda_summary","returned":0,"total":0,"complete":true}"#,
        )
        .unwrap();
        assert_eq!(empty.total, 0);
        for summary in [
            json!({"type":"agenda_summary","returned":29,"total":30,"complete":true}),
            json!({"type":"agenda_summary","returned":30,"total":29,"complete":false}),
            json!({"type":"agenda_summary","returned":30,"total":31,"complete":true}),
        ] {
            assert!(parse_helper_rows::<Event>(&format!("{rows}\n{summary}")).is_err());
        }
        let partial: Agenda<Event> = parse_helper_rows(&format!(
            "{rows}\n{}",
            json!({"type":"agenda_summary","returned":30,"total":41,"complete":false})
        ))
        .unwrap();
        assert!(describe_events("2026-10-22", &partial).contains("first 30 of 41 events"));
        assert!(
            parse_helper_rows::<Event>(&format!("{output}\n{row}")).is_err(),
            "a summary must be the final record"
        );
    }

    #[test]
    fn reminder_display_keeps_all_returned_items_and_reports_coverage() {
        let reminders = Agenda {
            items: (1..=50)
                .map(|n| Reminder {
                    title: format!("Task {n}"),
                    due: String::new(),
                    list: "Work".into(),
                    ..Default::default()
                })
                .collect(),
            total: 63,
        };
        let answer = describe_reminders(&reminders);
        assert!(answer.contains("Task 50") && answer.contains("first 50 of 63 open reminders"));
    }

    #[test]
    fn unread_mail_filters_before_sorting_and_limiting() {
        let output = "2026-10-22 11:00\tread\ttrue\tAlice\tInvoice\n2026-10-22 09:00\tearly\tfalse\tBob\tInvoice\n2026-10-22 10:00\tlatest\tfalse\tCleo\tInvoice";
        let unread = mail_rows(output, true, 1);
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0][1], "latest");
        assert_eq!(mail_rows(output, false, 1)[0][1], "read");
        assert!(validate("read_mail", &json!({"unread":"true"})).is_err());
        assert!(validate("read_mail", &json!({"limit":-1})).is_err());
        assert!(validate("control_music", &json!({"app":false,"action":"play"})).is_err());
    }

    #[test]
    fn spotify_confirmation_checks_both_the_track_and_artist() {
        let label = "Hey There Delilah by Plain White T's";
        assert!(matches_pressed_track(
            label,
            "playing\tHey There Delilah\tPlain White T's\tAlbum"
        ));
        assert!(!matches_pressed_track(
            label,
            "playing\tHey There Delilah\tKaraoke Band\tAlbum"
        ));
        assert!(!matches_pressed_track(
            label,
            "playing\tOther Song\tPlain White T's\tAlbum"
        ));
        assert!(!matches_pressed_track(
            label,
            "paused\tHey There Delilah\tPlain White T's\tAlbum"
        ));
        assert!(!matches_pressed_track(label, "playing\t\t\t"));
    }

    #[test]
    fn chromium_reads_check_the_front_windows_privacy_flag() {
        for id in ["chrome", "brave", "edge", "arc"] {
            let script = window_privacy_script(app(id).unwrap()).unwrap();
            assert!(script.contains("front window") && script.contains("incognito"));
            assert!(script.contains("return \"private\"") && script.contains("return \"none\""));
        }
        assert!(
            window_privacy_script(app("safari").unwrap()).is_none(),
            "Safari does not expose private mode"
        );
    }

    #[test]
    fn values_are_arguments_not_script() {
        for source in [
            MAIL_LIST,
            MAIL_TEXT,
            NOTES_FIND,
            MUSIC,
            SPOTIFY,
            CHROMIUM_TABS,
            CHROMIUM_SWITCH,
            ARC_TABS,
            ARC_SWITCH,
            SAFARI_TABS,
            SAFARI_SWITCH,
            SAFARI_PAGE,
        ] {
            assert!(
                source.contains("on run argv"),
                "every script reads its values from argv"
            );
        }
    }
}
