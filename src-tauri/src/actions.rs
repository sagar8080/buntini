//! Things Buntini can do on this Mac, each only after the user approves the exact
//! details: add a reminder or calendar event, write a note, open a Mail draft (never
//! sent), run one of the user's Shortcuts, open an app or an https link.
//!
//! The model proposes; Rust validates and normalises the arguments, the app shows them
//! in an editable sheet, and only an approved, re-validated request runs. AppleScript
//! receives every value as a separate argument, never as script text.
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command;

pub const TOOLS: [&str; 7] = ["add_reminder", "add_calendar_event", "create_note", "draft_email", "run_shortcut", "open_app", "open_link"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    /// text, long or when (YYYY-MM-DD HH:MM), so the sheet can offer the right input.
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub tool: String,
    pub title: String,
    pub fields: Vec<Field>,
    pub note: String,
}

fn text(args: &Map<String, Value>, key: &str, max: usize, required: bool) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) if !required => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() && !required => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() || value.chars().count() > max || value.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                Err(format!("{key} must be 1–{max} characters."))
            } else {
                Ok(Some(value.to_string()))
            }
        }
        _ => Err(format!("{key} is required.")),
    }
}

/// A local date and time in the forms models write ("2026-10-22 18:00",
/// "2026-10-22T18:00:00", or a bare date meaning 9:00), as "YYYY-MM-DD HH:MM".
pub fn normalize_when(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('Z');
    let parsed = ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(value, format).ok())
        .or_else(|| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok().and_then(|day| day.and_hms_opt(9, 0, 0)))
        .ok_or("Use a date and time like 2026-10-22 18:00.")?;
    Ok(parsed.format("%Y-%m-%d %H:%M").to_string())
}

/// "YYYY-MM-DD HH:MM" in local time, as (year, month, day, hour, minute).
pub fn when(value: &str) -> Result<(i32, u32, u32, u32, u32), String> {
    let parsed = chrono::NaiveDateTime::parse_from_str(&normalize_when(value)?, "%Y-%m-%d %H:%M").map_err(|_| "Use a date and time like 2026-10-22 18:00.".to_string())?;
    use chrono::{Datelike, Timelike};
    Ok((parsed.year(), parsed.month(), parsed.day(), parsed.hour(), parsed.minute()))
}

fn https(value: &str) -> Result<String, String> {
    let candidate = if value.contains("://") { value.to_string() } else { format!("https://{value}") };
    let url = reqwest::Url::parse(&candidate).map_err(|_| "That isn’t a web address.".to_string())?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err("Only https web addresses can be opened.".into());
    }
    Ok(url.to_string())
}

/// Checks and normalises a proposed action. Unknown keys are refused.
pub fn validate(tool: &str, args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    let allowed: &[&str] = match tool {
        "add_reminder" => &["title", "due"],
        "add_calendar_event" => &["title", "start", "minutes", "location"],
        "create_note" => &["title", "body"],
        "draft_email" => &["to", "subject", "body"],
        "run_shortcut" | "open_app" => &["name"],
        "open_link" => &["url"],
        _ => return Err(format!("“{tool}” isn’t something Buntini can do.")),
    };
    if let Some(extra) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{tool} doesn’t take “{extra}”."));
    }
    let mut out = Map::new();
    let mut put = |key: &str, value: Option<String>| {
        if let Some(value) = value {
            out.insert(key.into(), Value::String(value));
        }
    };
    match tool {
        "add_reminder" => {
            put("title", text(args, "title", 200, true)?);
            let due = text(args, "due", 25, false)?.map(|due| normalize_when(&due)).transpose()?;
            put("due", due);
        }
        "add_calendar_event" => {
            put("title", text(args, "title", 200, true)?);
            let start = normalize_when(&text(args, "start", 25, true)?.unwrap())?;
            put("start", Some(start));
            put("location", text(args, "location", 200, false)?);
            let minutes = match args.get("minutes") {
                None | Some(Value::Null) => 60,
                Some(value) => value.as_u64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok())).filter(|m| (5..=1440).contains(m)).ok_or("Length must be 5–1440 minutes.")?,
            };
            out.insert("minutes".into(), json!(minutes));
        }
        "create_note" => {
            put("title", text(args, "title", 200, true)?);
            put("body", text(args, "body", 4000, true)?);
        }
        "draft_email" => {
            // A made-up or placeholder address is left blank for the user to fill in Mail.
            let to = text(args, "to", 254, false)?.filter(|address| {
                address.split_once('@').is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.')) && !address.contains(char::is_whitespace) && !address.contains([',', ';', '<', '>', '[', ']'])
            });
            put("to", to);
            put("subject", text(args, "subject", 200, true)?);
            put("body", text(args, "body", 4000, true)?);
        }
        "run_shortcut" | "open_app" => put("name", text(args, "name", 100, true)?),
        _ => put("url", Some(https(&text(args, "url", 2000, true)?.unwrap())?)),
    }
    Ok(out)
}

/// What the approval sheet shows: a question and the editable details.
pub fn describe(tool: &str, args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let get = |key: &str| args.get(key).map(|value| value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())).unwrap_or_default();
    let field = |key: &str, label: &str, kind: &'static str| Field { key: key.into(), label: label.into(), value: get(key), kind };
    match tool {
        "add_reminder" => ("Add this reminder?".into(), vec![field("title", "Reminder", "text"), field("due", "When", "when")], "Added to your default list in Reminders.".into()),
        "add_calendar_event" => ("Add this to your calendar?".into(), vec![field("title", "Event", "text"), field("start", "Starts", "when"), field("minutes", "Minutes", "text"), field("location", "Where", "text")], "Added to your first writable calendar.".into()),
        "create_note" => ("Save this note?".into(), vec![field("title", "Title", "text"), field("body", "Note", "long")], "Saved in Notes.".into()),
        "draft_email" => ("Open this email draft?".into(), vec![field("to", "To", "text"), field("subject", "Subject", "text"), field("body", "Message", "long")], "Opens in Mail for you to review. Buntini never sends email.".into()),
        "run_shortcut" => ("Run this shortcut?".into(), vec![field("name", "Shortcut", "text")], "Shortcuts can do anything you built them to do.".into()),
        "open_app" => ("Open this app?".into(), vec![field("name", "App", "text")], String::new()),
        _ => ("Open this page?".into(), vec![field("url", "Address", "text")], "Opens in your default browser.".into()),
    }
}

/// Notes bodies are HTML: show the text as written.
fn html(text: &str) -> String {
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    escaped.split('\n').map(|line| format!("<div>{}</div>", if line.is_empty() { "<br>" } else { line })).collect()
}

// Each script reads its values from argv, so user text is never part of the script.
const DATE_HELPER: &str = r#"on makeDate(y, m, d, h, mi)
  set t to current date
  set day of t to 1
  set year of t to y as integer
  set month of t to m as integer
  set day of t to d as integer
  set time of t to ((h as integer) * 3600 + (mi as integer) * 60)
  return t
end makeDate
"#;
const REMINDER: &str = r#"on run argv
  tell application "Reminders"
    if (count of argv) > 1 then
      make new reminder with properties {name:item 1 of argv, remind me date:my makeDate(item 2 of argv, item 3 of argv, item 4 of argv, item 5 of argv, item 6 of argv)}
    else
      make new reminder with properties {name:item 1 of argv}
    end if
  end tell
end run
"#;
const EVENT: &str = r#"on run argv
  set startDate to my makeDate(item 2 of argv, item 3 of argv, item 4 of argv, item 5 of argv, item 6 of argv)
  set endDate to startDate + ((item 7 of argv) as integer) * 60
  tell application "Calendar"
    set target to first calendar whose writable is true
    tell target to make new event with properties {summary:item 1 of argv, start date:startDate, end date:endDate, location:item 8 of argv}
  end tell
end run
"#;
const NOTE: &str = r#"on run argv
  tell application "Notes" to make new note with properties {name:item 1 of argv, body:item 2 of argv}
end run
"#;
const MAIL: &str = r#"on run argv
  tell application "Mail"
    set draft to make new outgoing message with properties {subject:item 2 of argv, content:item 3 of argv, visible:true}
    if item 1 of argv is not "" then
      tell draft to make new to recipient at end of to recipients with properties {address:item 1 of argv}
    end if
    activate
  end tell
end run
"#;

/// The script and its arguments for an AppleScript-backed action.
pub fn script(tool: &str, args: &Map<String, Value>) -> Option<(String, Vec<String>)> {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let parts = |value: &str| when(value).map(|(y, m, d, h, mi)| vec![y.to_string(), m.to_string(), d.to_string(), h.to_string(), mi.to_string()]).unwrap_or_default();
    match tool {
        "add_reminder" => {
            let mut argv = vec![get("title")];
            argv.extend(parts(&get("due")));
            Some((format!("{DATE_HELPER}{REMINDER}"), argv))
        }
        "add_calendar_event" => {
            let mut argv = vec![get("title")];
            argv.extend(parts(&get("start")));
            argv.push(args.get("minutes").and_then(Value::as_u64).unwrap_or(60).to_string());
            argv.push(get("location"));
            Some((format!("{DATE_HELPER}{EVENT}"), argv))
        }
        "create_note" => Some((NOTE.into(), vec![get("title"), format!("<h1>{}</h1>{}", html(&get("title")).trim_start_matches("<div>").trim_end_matches("</div>"), html(&get("body")))])),
        "draft_email" => Some((MAIL.into(), vec![get("to"), get("subject"), get("body")])),
        _ => None,
    }
}

async fn output(mut command: Command, seconds: u64) -> Result<String, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(seconds), command.output()).await.map_err(|_| "That took too long and was stopped.".to_string())?.map_err(|_| "Couldn’t start the helper for that.".to_string())?;
    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).chars().take(4000).collect())
    } else {
        let detail = String::from_utf8_lossy(&result.stderr).to_string();
        Err(if detail.contains("-1743") || detail.contains("Not authorized") {
            "macOS didn’t allow Buntini to use that app. Allow it in System Settings → Privacy & Security → Automation.".into()
        } else {
            format!("That didn’t work: {}", detail.trim().chars().take(200).collect::<String>())
        })
    }
}

fn find_app(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let bundle = if name.to_lowercase().ends_with(".app") { name.to_string() } else { format!("{name}.app") };
    let home = std::env::var("HOME").map(|home| PathBuf::from(home).join("Applications")).ok();
    ["/Applications", "/System/Applications", "/System/Applications/Utilities", "/Applications/Utilities"].iter().map(PathBuf::from).chain(home).map(|dir| dir.join(&bundle)).find(|path| path.is_dir())
}

/// Runs an approved action and says what happened.
pub async fn run(tool: &str, args: &Map<String, Value>) -> Result<String, String> {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    if let Some((source, argv)) = script(tool, args) {
        let mut command = Command::new("/usr/bin/osascript");
        command.arg("-e").arg(source).args(argv);
        output(command, 30).await?;
        return Ok(match tool {
            "add_reminder" if !get("due").is_empty() => format!("Added “{}” to Reminders for {}.", get("title"), get("due")),
            "add_reminder" => format!("Added “{}” to Reminders.", get("title")),
            "add_calendar_event" => format!("Added “{}” to your calendar at {}.", get("title"), get("start")),
            "create_note" => format!("Saved “{}” in Notes.", get("title")),
            _ => "Your draft is open in Mail. Review it and send it when you’re ready.".into(),
        });
    }
    match tool {
        "run_shortcut" => {
            let name = get("name");
            let mut listing = Command::new("/usr/bin/shortcuts");
            listing.arg("list");
            let list = output(listing, 15).await?;
            let exact = list.lines().map(str::trim).find(|line| line.eq_ignore_ascii_case(&name)).map(str::to_string).ok_or_else(|| format!("I couldn’t find a shortcut called “{name}”."))?;
            let mut command = Command::new("/usr/bin/shortcuts");
            command.arg("run").arg(&exact);
            output(command, 120).await?;
            Ok(format!("Ran your “{exact}” shortcut."))
        }
        "open_app" => {
            let name = get("name");
            let path = find_app(&name).ok_or_else(|| format!("I couldn’t find an app called “{name}” in Applications."))?;
            let mut command = Command::new("/usr/bin/open");
            command.arg(&path);
            output(command, 15).await?;
            Ok(format!("Opened {}.", path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or(name)))
        }
        "open_link" => {
            let url = https(&get("url"))?;
            let mut command = Command::new("/usr/bin/open");
            command.arg(&url);
            output(command, 15).await?;
            Ok(format!("Opened {url}."))
        }
        _ => Err("Unknown action.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposals_are_checked_and_normalised() {
        let reminder = validate("add_reminder", &json!({"title":" Call mum ","due":"2026-10-22 18:00"})).unwrap();
        assert_eq!(reminder["title"], "Call mum");
        assert!(validate("add_reminder", &json!({"title":"x","due":"tomorrow"})).is_err());
        assert!(validate("add_reminder", &json!({"title":"x","list":"Work"})).is_err(), "unknown keys are refused");
        let event = validate("add_calendar_event", &json!({"title":"Lunch","start":"2026-10-24 13:00"})).unwrap();
        assert_eq!(event["minutes"], 60);
        assert!(validate("add_calendar_event", &json!({"title":"Lunch","start":"2026-10-24 13:00","minutes":2})).is_err());
        assert_eq!(validate("open_link", &json!({"url":"github.com"})).unwrap()["url"], "https://github.com/");
        assert!(validate("open_link", &json!({"url":"file:///etc/passwd"})).is_err());
        assert!(validate("open_link", &json!({"url":"http://example.com"})).is_err());
        assert!(validate("draft_email", &json!({"to":"a@b.com, c@d.com","subject":"s","body":"b"})).unwrap().get("to").is_none(), "one recipient, no lists");
        assert!(validate("draft_email", &json!({"to":"[Landlord's Email]","subject":"s","body":"b"})).unwrap().get("to").is_none(), "placeholders are left for the user");
        assert_eq!(validate("add_calendar_event", &json!({"title":"Lunch","start":"2026-10-23T13:00:00"})).unwrap()["start"], "2026-10-23 13:00");
        assert_eq!(validate("add_reminder", &json!({"title":"Bins","due":"2026-10-23"})).unwrap()["due"], "2026-10-23 09:00");
        assert!(validate("send_email", &json!({})).is_err());
        assert!(find_app("../../bin/sh").is_none());
    }

    #[test]
    fn scripts_take_values_as_arguments_not_code() {
        let args = validate("add_reminder", &json!({"title":"\" & do shell script \"rm -rf ~\" & \"","due":"2026-10-22 18:05"})).unwrap();
        let (source, argv) = script("add_reminder", &args).unwrap();
        assert!(!source.contains("rm -rf"), "user text never becomes script");
        assert_eq!(argv, ["\" & do shell script \"rm -rf ~\" & \"", "2026", "10", "22", "18", "5"]);
        let note = validate("create_note", &json!({"title":"Ideas","body":"<b>tea</b>\nbiscuits"})).unwrap();
        let (_, argv) = script("create_note", &note).unwrap();
        assert!(argv[1].contains("&lt;b&gt;tea&lt;/b&gt;") && argv[1].contains("<div>biscuits</div>"));
        let (_, mail) = script("draft_email", &validate("draft_email", &json!({"subject":"Leak","body":"Hello"})).unwrap()).unwrap();
        assert_eq!(mail, ["", "Leak", "Hello"]);
    }

    #[test]
    fn sheets_show_every_value() {
        let args = validate("add_calendar_event", &json!({"title":"Lunch","start":"2026-10-24 13:00","location":"Café"})).unwrap();
        let (title, fields, _) = describe("add_calendar_event", &args);
        assert_eq!(title, "Add this to your calendar?");
        assert_eq!(fields.iter().map(|field| field.value.as_str()).collect::<Vec<_>>(), ["Lunch", "2026-10-24 13:00", "60", "Café"]);
    }
}
