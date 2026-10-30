//! Things Buntini can do on this Mac, each only after the user approves the exact
//! details: add a reminder or calendar event, write a note, open a Mail draft (never
//! sent) or a threaded reply to an email (never sent), run one of the user's Shortcuts,
//! open an app or an https link, and change an item the user already has: mark a
//! reminder done, move or rename an event, or add text to the end of a note.
//!
//! The model proposes; Rust validates and normalises the arguments, the app shows them
//! in an editable sheet, and only an approved, re-validated request runs. AppleScript
//! receives every value as a separate argument, never as script text.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

pub const TOOLS: [&str; 12] = [
    "add_reminder",
    "add_calendar_event",
    "create_note",
    "draft_email",
    "reply_email",
    "run_shortcut",
    "open_app",
    "open_link",
    "complete_reminder",
    "reschedule_reminder",
    "update_calendar_event",
    "append_to_note",
];

/// Writes that change an existing item. Its `id` is never the model's: it is bound from
/// a read in the same task (`source_fields::edit_target`) or picked by the user from
/// those records in the approval sheet, and the receipt reads the item back.
pub const EDIT_TOOLS: [&str; 4] = ["complete_reminder", "reschedule_reminder", "update_calendar_event", "append_to_note"];

/// A write is successful only when its requested result was read back. An
/// unknown result may already exist: callers must never replay it automatically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionReceipt {
    pub outcome: ActionOutcome,
    pub tool: String,
    pub object_id: Option<String>,
    pub destination: Option<String>,
    pub message: String,
    pub evidence: Value,
    pub error: Option<String>,
}

impl ActionReceipt {
    fn failure(tool: &str, outcome: ActionOutcome, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let message = if outcome == ActionOutcome::Unknown {
            format!("I couldn’t confirm the result. {detail} Check the destination before trying again; this action will not be repeated automatically.")
        } else {
            detail.clone()
        };
        Self {
            outcome,
            tool: tool.into(),
            object_id: None,
            destination: None,
            message,
            evidence: json!({}),
            error: Some(detail),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    /// text, long or when (YYYY-MM-DD HH:MM), so the sheet can offer the right input;
    /// hidden for a value the sheet sends back without showing (an item's id), and fixed
    /// for one it shows but never sends (what an item is like now).
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
    /// Values the user can pick for a field, such as the addresses of several people in
    /// Contacts who match the name they gave. Picking one is an edit like any other.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub field: String,
    pub label: String,
    pub value: String,
    /// Other fields the pick sets with it: picking one of two matching events also sets
    /// that event's own title, time and what it is like now.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub also: std::collections::BTreeMap<String, String>,
}

fn text(
    args: &Map<String, Value>,
    key: &str,
    max: usize,
    required: bool,
) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) if !required => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() && !required => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty()
                || value.chars().count() > max
                || value
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
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
/// An explicit UTC offset denotes an instant, so convert it before showing the
/// local approval field. AppleScript accepts wall-clock components only: reject
/// missing or repeated daylight-saving times instead of silently choosing one.
pub fn normalize_when(value: &str) -> Result<String, String> {
    normalize_when_in(value, &chrono::Local)
}

fn normalize_when_in<Tz: chrono::TimeZone>(value: &str, timezone: &Tz) -> Result<String, String> {
    let value = value.trim();
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(timezone).naive_local())
        .or_else(|| {
            [
                "%Y-%m-%d %H:%M",
                "%Y-%m-%dT%H:%M",
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%dT%H:%M:%S",
            ]
            .iter()
            .find_map(|format| chrono::NaiveDateTime::parse_from_str(value, format).ok())
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|day| day.and_hms_opt(9, 0, 0))
        })
        .ok_or("Use a date and time like 2026-10-22 18:00.")?;
    match timezone.from_local_datetime(&parsed) {
        chrono::LocalResult::Single(_) => {}
        chrono::LocalResult::Ambiguous(_, _) => return Err("That local time occurs twice when the clocks change. Choose a time outside the repeated hour.".into()),
        chrono::LocalResult::None => return Err("That local time does not exist when the clocks change. Choose another time.".into()),
    }
    Ok(parsed.format("%Y-%m-%d %H:%M").to_string())
}

/// "YYYY-MM-DD HH:MM" in local time, as (year, month, day, hour, minute).
pub fn when(value: &str) -> Result<(i32, u32, u32, u32, u32), String> {
    let parsed = chrono::NaiveDateTime::parse_from_str(&normalize_when(value)?, "%Y-%m-%d %H:%M")
        .map_err(|_| "Use a date and time like 2026-10-22 18:00.".to_string())?;
    use chrono::{Datelike, Timelike};
    Ok((
        parsed.year(),
        parsed.month(),
        parsed.day(),
        parsed.hour(),
        parsed.minute(),
    ))
}

/// Validate destinations opened outside Buntini as strictly as research hosts.
/// This is a syntax/IP-literal check; the user's browser still owns DNS and any
/// subsequent redirects. Preserve the original scheme and fragment for display.
pub(crate) fn public_web_url(value: &str) -> Result<reqwest::Url, String> {
    if value.is_empty()
        || value.len() > 4096
        || value != value.trim()
        || value.chars().any(char::is_control)
    {
        return Err("Enter a valid public web address of up to 4,096 characters.".into());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "That isn’t a web address.")?;
    let expected_port = match url.scheme() {
        "https" => 443,
        "http" => 80,
        _ => return Err("Only public HTTP or HTTPS web addresses can be opened.".into()),
    };
    if url.port_or_known_default() != Some(expected_port) {
        return Err("Public web addresses must use their standard HTTP or HTTPS port.".into());
    }
    // Reuse one authoritative host/IP policy for both schemes. Validation does
    // not send anything; the actual browser destination remains `url` below.
    let mut secure = url.clone();
    secure
        .set_scheme("https")
        .map_err(|_| "That isn’t a web address.")?;
    secure
        .set_port(None)
        .map_err(|_| "That isn’t a web address.")?;
    crate::research::validate_public_url(secure.as_str())?;
    Ok(url)
}

fn https(value: &str) -> Result<String, String> {
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("https://{value}")
    };
    let url = public_web_url(&candidate)?;
    if url.scheme() != "https" {
        return Err("Only public HTTPS web addresses can be opened.".into());
    }
    Ok(url.to_string())
}

/// An event's length in minutes, 60 unless given.
fn minutes(args: &Map<String, Value>) -> Result<u64, String> {
    match args.get("minutes") {
        None | Some(Value::Null) => Ok(60),
        Some(value) => Ok(value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
            .filter(|m| (5..=1440).contains(m))
            .ok_or("Length must be 5–1440 minutes.")?),
    }
}

/// The app's own identifier for an existing item. Missing only while the user still has
/// to pick one of several matches in the sheet; `execute` refuses to run without it.
fn item_id(args: &Map<String, Value>) -> Result<Option<String>, String> {
    let id = text(args, "id", 300, false)?;
    if id
        .as_deref()
        .is_some_and(|id| id.starts_with('-') || id.contains(char::is_whitespace))
    {
        return Err("That isn’t an item Buntini read.".into());
    }
    Ok(id)
}

/// Checks and normalises a proposed action. Unknown keys are refused.
pub fn validate(tool: &str, args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    let allowed: &[&str] = match tool {
        "add_reminder" => &["title", "due"],
        "add_calendar_event" => &["title", "start", "minutes", "location"],
        "create_note" => &["title", "body"],
        "draft_email" => &["to", "subject", "body"],
        "reply_email" => &["message", "message_id", "to", "subject", "body", "answers"],
        "run_shortcut" | "open_app" => &["name"],
        "open_link" => &["url"],
        "complete_reminder" => &["id", "title"],
        "reschedule_reminder" => &["id", "title", "was_due", "due"],
        "update_calendar_event" => &[
            "id",
            "occurrence",
            "was_title",
            "title",
            "start",
            "minutes",
            "location",
        ],
        "append_to_note" => &["id", "title", "text"],
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
            let due = text(args, "due", 40, false)?
                .map(|due| normalize_when(&due))
                .transpose()?;
            put("due", due);
        }
        "add_calendar_event" => {
            put("title", text(args, "title", 200, true)?);
            let start = normalize_when(&text(args, "start", 40, true)?.unwrap())?;
            put("start", Some(start));
            put("location", text(args, "location", 200, false)?);
            out.insert("minutes".into(), json!(minutes(args)?));
        }
        "create_note" => {
            put("title", text(args, "title", 200, true)?);
            put("body", text(args, "body", 4000, true)?);
        }
        "draft_email" => {
            // A made-up or placeholder address is left blank for the user to fill in Mail.
            let to = text(args, "to", 254, false)?.filter(|address| plain_address(address));
            put("to", to);
            put("subject", text(args, "subject", 200, true)?);
            put("body", text(args, "body", 4000, true)?);
        }
        "reply_email" => {
            // Which email this answers, and its sender, come from Mail, Buntini's own list
            // or its inbox search (recipes.rs), never from a model, so they must be exact
            // and complete. Before the user picks one of several found, all are missing
            // together; `execute` refuses to run then.
            if ["message", "message_id", "to"].iter().all(|key| args.get(*key).is_none()) {
                put("subject", text(args, "subject", 200, false)?);
                put("body", text(args, "body", 4000, true)?);
                return Ok(out);
            }
            let message = text(args, "message", 20, true)?.unwrap();
            if !message.chars().all(|c| c.is_ascii_digit()) {
                return Err("The email to answer must be Mail's own message number.".into());
            }
            put("message", Some(message));
            let message_id = text(args, "message_id", 998, true)?.unwrap();
            if message_id.contains(char::is_whitespace) {
                return Err("The email to answer has an invalid Message-ID.".into());
            }
            put("message_id", Some(message_id));
            let to = text(args, "to", 254, true)?.unwrap();
            if !plain_address(&to) {
                return Err("A reply goes to the sender's email address.".into());
            }
            put("to", Some(to));
            put("subject", text(args, "subject", 200, true)?);
            put("body", text(args, "body", 4000, true)?);
            put("answers", text(args, "answers", 300, false)?);
        }
        "run_shortcut" | "open_app" => put("name", text(args, "name", 100, true)?),
        "complete_reminder" => {
            put("id", item_id(args)?);
            put("title", text(args, "title", 200, true)?);
        }
        "reschedule_reminder" => {
            put("id", item_id(args)?);
            put("title", text(args, "title", 200, true)?);
            // As read: a time, a date without one, or nothing. Kept as written so the
            // helper can check the reminder still has it.
            let was = text(args, "was_due", 25, false)?.unwrap_or_default();
            if !was.is_empty() && reminder_due(&was).is_none() {
                return Err("The reminder's current due date isn’t one Buntini read.".into());
            }
            out.insert("was_due".into(), Value::String(was.clone()));
            let due = text(args, "due", 40, true)?.unwrap();
            let due = match reminder_due(due.trim()) {
                Some(day) => day,
                None => normalize_when(&due)?,
            };
            if due == was {
                return Err("That reminder is already due then.".into());
            }
            out.insert("due".into(), Value::String(due));
        }
        "update_calendar_event" => {
            put("id", item_id(args)?);
            let occurrence = normalize_when(&text(args, "occurrence", 40, true)?.unwrap())?;
            put("occurrence", Some(occurrence));
            put("was_title", text(args, "was_title", 200, true)?);
            put("title", text(args, "title", 200, true)?);
            let start = normalize_when(&text(args, "start", 40, true)?.unwrap())?;
            put("start", Some(start));
            put("location", text(args, "location", 200, false)?);
            out.insert("minutes".into(), json!(minutes(args)?));
        }
        "append_to_note" => {
            put("id", item_id(args)?);
            put("title", text(args, "title", 200, true)?);
            put("text", text(args, "text", 2000, true)?);
        }
        _ => put(
            "url",
            Some(https(&text(args, "url", 2000, true)?.unwrap())?),
        ),
    }
    Ok(out)
}

/// A reminder's due date as Buntini reads and writes it: "2026-11-06 09:00", or
/// "2026-11-06" for a date without a time.
fn reminder_due(text: &str) -> Option<String> {
    let exact = |format: &str| {
        chrono::NaiveDateTime::parse_from_str(text, format)
            .map(|at| at.format(format).to_string())
            .ok()
    };
    exact("%Y-%m-%d %H:%M")
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map(|day| day.format("%Y-%m-%d").to_string())
                .ok()
        })
        .filter(|normal| normal == text)
}

/// One plain address ("ana@example.com"), not a list, a display name or a placeholder.
fn plain_address(address: &str) -> bool {
    address
        .split_once('@')
        .is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.'))
        && !address.contains(char::is_whitespace)
        && !address.contains([',', ';', '<', '>', '[', ']'])
}

/// Details the approval sheet shows but the user can't change: which email a reply
/// answers and who it goes to belong to that email, so an edit can never redirect it.
pub fn fixed_fields(tool: &str) -> &'static [&'static str] {
    match tool {
        "reply_email" => &["message", "message_id", "to", "answers"],
        _ => &[],
    }
}

/// The approved details with every fixed field taken from the proposal, whatever the
/// sheet sent back. A field the proposal didn't have stays absent.
pub fn keep_fixed(tool: &str, proposed: &Value, approved: &Value) -> Value {
    // A call to a tool the user added runs exactly as shown; its sheet edits nothing.
    if crate::mcp::is_tool(tool) {
        return proposed.clone();
    }
    let mut kept = approved.clone();
    if let Some(fields) = kept.as_object_mut() {
        for key in fixed_fields(tool) {
            match proposed.get(*key) {
                Some(value) => fields.insert(key.to_string(), value.clone()),
                None => fields.remove(*key),
            };
        }
    }
    kept
}

/// What the approval sheet shows: a question and the editable details.
pub fn describe(tool: &str, args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let get = |key: &str| {
        args.get(key)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default()
    };
    let field = |key: &str, label: &str, kind: &'static str| Field {
        key: key.into(),
        label: label.into(),
        value: get(key),
        kind,
    };
    match tool {
        "add_reminder" => (
            "Add this reminder?".into(),
            vec![
                field("title", "Reminder", "text"),
                field("due", "When", "when"),
            ],
            "Added to your default list in Reminders.".into(),
        ),
        "add_calendar_event" => (
            "Add this to your calendar?".into(),
            vec![
                field("title", "Event", "text"),
                field("start", "Starts", "when"),
                field("minutes", "Minutes", "text"),
                field("location", "Where", "text"),
            ],
            "Added to your first writable calendar.".into(),
        ),
        "create_note" => (
            "Save this note?".into(),
            vec![
                field("title", "Title", "text"),
                field("body", "Note", "long"),
            ],
            "Saved in Notes.".into(),
        ),
        "draft_email" => (
            "Open this email draft?".into(),
            vec![
                field("to", "To", "text"),
                field("subject", "Subject", "text"),
                field("body", "Message", "long"),
            ],
            "Opens in Mail for you to review. Buntini never sends email.".into(),
        ),
        "reply_email" => (
            "Open this reply in Mail?".into(),
            [
                // Picked in the sheet when the inbox search found several.
                field("message", "Which email", "hidden"),
                field("answers", "Answers", "fixed"),
                field("to", "To", "fixed"),
                field("subject", "Subject", "text"),
                field("body", "Message", "long"),
            ]
            .into_iter()
            // Which email shows only as the picker, while there is one to pick.
            .filter(|field| match field.kind {
                "fixed" => !field.value.is_empty(),
                "hidden" => field.value.is_empty(),
                _ => true,
            })
            .collect(),
            "Opens in Mail as a reply in the same thread, for you to review. Buntini never sends email.".into(),
        ),
        "run_shortcut" => (
            "Run this shortcut?".into(),
            vec![field("name", "Shortcut", "text")],
            "Shortcuts can do anything you built them to do.".into(),
        ),
        "open_app" => (
            "Open this app?".into(),
            vec![field("name", "App", "text")],
            String::new(),
        ),
        "complete_reminder" => (
            "Mark this reminder done?".into(),
            vec![
                field("id", "Which reminder", "hidden"),
                field("title", "Reminder", "hidden"),
                Field {
                    key: "after".into(),
                    label: "After".into(),
                    value: "Done".into(),
                    kind: "fixed",
                },
            ],
            "Marked as done in Reminders. Nothing else about it changes.".into(),
        ),
        "reschedule_reminder" => (
            "Move this reminder?".into(),
            vec![
                field("id", "Which reminder", "hidden"),
                field("title", "Reminder", "hidden"),
                field("was_due", "Was due", "hidden"),
                // A date without a time stays one: the time picker can't show it.
                field("due", "Due", if get("due").len() == 10 { "text" } else { "when" }),
            ],
            "Only its due date changes in Reminders. An alarm at its due time moves with it.".into(),
        ),
        "update_calendar_event" => (
            "Change this event?".into(),
            vec![
                field("id", "Which event", "hidden"),
                field("occurrence", "Occurrence", "hidden"),
                field("was_title", "Event", "hidden"),
                field("title", "Event", "text"),
                field("start", "Starts", "when"),
                field("minutes", "Minutes", "text"),
                field("location", "Where", "text"),
            ],
            "Only this event changes. For a repeating event, only this one occurrence changes."
                .into(),
        ),
        "append_to_note" => (
            "Add this to your note?".into(),
            vec![
                field("id", "Which note", "hidden"),
                field("title", "Note", "hidden"),
                field("text", "Add", "long"),
            ],
            "Added to the end of the note in Notes. The rest of the note stays as it is.".into(),
        ),
        _ => (
            "Open this page?".into(),
            vec![field("url", "Address", "text")],
            "Opens in your default browser.".into(),
        ),
    }
}

/// Notes bodies are HTML: show the text as written.
fn html(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    escaped
        .split('\n')
        .map(|line| format!("<div>{}</div>", if line.is_empty() { "<br>" } else { line }))
        .collect()
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
// Foundation serializes arbitrary user text safely; stdout is exactly one
// structured receipt. Track the mutation boundary so a post-write failure can
// never look like a safe-to-repeat rejection.
const RECEIPT_HELPER: &str = r#"use framework "Foundation"
use scripting additions
on putField(fields, fieldName, fieldValue)
  if fieldValue is missing value then set fieldValue to ""
  fields's setObject:fieldValue forKey:fieldName
end putField
on dateText(t)
  if t is missing value then return ""
  set m to text -2 thru -1 of ("0" & ((month of t) as integer))
  set d to text -2 thru -1 of ("0" & (day of t))
  set h to text -2 thru -1 of ("0" & (hours of t))
  set mi to text -2 thru -1 of ("0" & (minutes of t))
  return ((year of t) as text) & "-" & m & "-" & d & " " & h & ":" & mi
end dateText
on dateSecond(t)
  if t is missing value then return ""
  return seconds of t
end dateSecond
on receiptJSON(resultOutcome, resultID, destinationName, fields, failureText)
  set resultData to current application's NSMutableDictionary's dictionary()
  my putField(resultData, "outcome", resultOutcome)
  my putField(resultData, "objectId", resultID)
  my putField(resultData, "destination", destinationName)
  my putField(resultData, "evidence", fields)
  my putField(resultData, "error", failureText)
  set encoded to current application's NSJSONSerialization's dataWithJSONObject:resultData options:0 |error|:(missing value)
  if encoded is missing value then error "Could not encode action verification."
  return (current application's NSString's alloc()'s initWithData:encoded encoding:(current application's NSUTF8StringEncoding)) as text
end receiptJSON
"#;
const RECEIPT_START: &str = r#"on run argv
  set mutationStarted to false
  set resultID to ""
  set destinationName to ""
  set observed to current application's NSMutableDictionary's dictionary()
  try
"#;
const RECEIPT_END: &str = r#"    return my receiptJSON("succeeded", resultID, destinationName, observed, "")
  on error failureText number failureNumber
    set resultOutcome to "failed"
    if mutationStarted then set resultOutcome to "unknown"
    return my receiptJSON(resultOutcome, resultID, destinationName, observed, failureText & " (" & failureNumber & ")")
  end try
end run
"#;
const REMINDER: &str = r#"    tell application "Reminders"
      set targetList to default list
      set destinationName to name of targetList
      my putField(observed, "destinationId", id of targetList)
      if (count of argv) > 1 then
        set requestedDate to my makeDate(item 2 of argv, item 3 of argv, item 4 of argv, item 5 of argv, item 6 of argv)
        set mutationStarted to true
        set created to make new reminder at end of reminders of targetList with properties {name:item 1 of argv, due date:requestedDate, remind me date:requestedDate}
      else
        set mutationStarted to true
        set created to make new reminder at end of reminders of targetList with properties {name:item 1 of argv}
      end if
      set resultID to id of created
      set stored to first reminder of targetList whose id is resultID
      my putField(observed, "title", name of stored)
      my putField(observed, "due", my dateText(due date of stored))
      my putField(observed, "remindAt", my dateText(remind me date of stored))
      my putField(observed, "dueSecond", my dateSecond(due date of stored))
      my putField(observed, "remindSecond", my dateSecond(remind me date of stored))
      my putField(observed, "completed", completed of stored)
      my putField(observed, "identifierScope", "persistent")
    end tell
"#;
const EVENT: &str = r#"    set startDate to my makeDate(item 2 of argv, item 3 of argv, item 4 of argv, item 5 of argv, item 6 of argv)
    set endDate to startDate + ((item 7 of argv) as integer) * 60
    -- Calendar's calendarIdentifier getter fails on some macOS releases even
    -- though its returned object specifier carries the real persistent ID.
    -- Read that formal descriptor; never substitute a name or invented ID.
    set lookup to current application's NSAppleScript's alloc()'s initWithSource:"tell application \"Calendar\" to get first calendar whose writable is true"
    set calendarDescriptor to lookup's executeAndReturnError:(missing value)
    if calendarDescriptor is missing value then error "Calendar did not return a writable calendar reference."
    -- Require an object specifier for the Calendar class, selected by unique ID
    -- (obj / wres / ID), rather than accepting a display-name selection.
    if (calendarDescriptor's descriptorType()) as integer is not 1868720672 then error "Calendar returned an invalid object reference."
    set wantedClass to calendarDescriptor's descriptorForKeyword:2002873972
    set selectionForm to calendarDescriptor's descriptorForKeyword:1718579821
    if wantedClass is missing value or selectionForm is missing value then error "Calendar returned an incomplete object reference."
    if (wantedClass's typeCodeValue()) as integer is not 2003985779 then error "The selected object is not a calendar."
    if (selectionForm's enumCodeValue()) as integer is not 1229201440 then error "The selected calendar reference is not identified by a persistent ID."
    set idDescriptor to calendarDescriptor's descriptorForKeyword:1936026724
    if idDescriptor is missing value then error "The selected calendar has no persistent identifier descriptor."
    set selectedCalendarID to idDescriptor's stringValue() as text
    if selectedCalendarID is "" then error "The selected calendar has no persistent identifier."
    tell application "Calendar"
      set targetCalendar to calendar id selectedCalendarID
      if writable of targetCalendar is not true then error "The selected calendar is no longer writable."
      set destinationName to name of targetCalendar
      my putField(observed, "destinationId", selectedCalendarID)
      set mutationStarted to true
      tell targetCalendar to set created to make new event with properties {summary:item 1 of argv, start date:startDate, end date:endDate, location:item 8 of argv}
      set resultID to uid of created
      set stored to first event of targetCalendar whose uid is resultID
      my putField(observed, "title", summary of stored)
      my putField(observed, "start", my dateText(start date of stored))
      my putField(observed, "end", my dateText(end date of stored))
      my putField(observed, "startSecond", my dateSecond(start date of stored))
      my putField(observed, "endSecond", my dateSecond(end date of stored))
      my putField(observed, "durationSeconds", (end date of stored) - (start date of stored))
      my putField(observed, "location", location of stored)
      my putField(observed, "identifierScope", "persistent")
    end tell
"#;
const NOTE: &str = r#"    tell application "Notes"
      set mutationStarted to true
      set created to make new note with properties {body:item 2 of argv}
      set resultID to id of created
      set stored to first note whose id is resultID
      set targetFolder to my folderOf(resultID)
      set destinationName to name of targetFolder
      my putField(observed, "destinationId", id of targetFolder)
      my putField(observed, "title", name of stored)
      my putField(observed, "plaintext", plaintext of stored)
      my putField(observed, "identifierScope", "persistent")
    end tell
"#;
const MAIL: &str = r#"    tell application "Mail"
      set destinationName to "Mail draft"
      set requestedSubject to item 2 of argv
      set knownDraftIDs to id of every message of drafts mailbox whose subject is requestedSubject
      set mutationStarted to true
      set created to make new outgoing message with properties {subject:item 2 of argv, content:item 3 of argv, visible:true}
      set compositionID to id of created
      set resultID to "mail-composition:" & compositionID
      my putField(observed, "compositionId", compositionID)
      set message signature of created to missing value
      if item 1 of argv is not "" then
        tell created to make new to recipient at end of to recipients with properties {address:item 1 of argv}
      end if
      save created
      set stored to first outgoing message whose id is compositionID
      my putField(observed, "subject", subject of stored)
      my putField(observed, "body", (content of stored) as text)
      set recipientAddresses to address of every to recipient of stored
      my putField(observed, "to", current application's NSArray's arrayWithArray:recipientAddresses)
      my putField(observed, "ccCount", count of cc recipients of stored)
      my putField(observed, "bccCount", count of bcc recipients of stored)
      my putField(observed, "visible", visible of stored)
      my putField(observed, "saveAccepted", true)
      my putField(observed, "identifierScope", "open_composition")
      set newDraftIDs to {}
      repeat 20 times
        set matchingDrafts to every message of drafts mailbox whose subject is requestedSubject
        set newDraftIDs to {}
        repeat with candidate in matchingDrafts
          set candidateID to id of candidate
          if candidateID is not in knownDraftIDs then set end of newDraftIDs to candidateID
        end repeat
        if (count of newDraftIDs) > 0 then exit repeat
        delay 0.25
      end repeat
      if (count of newDraftIDs) is not 1 then error "The saved draft could not be identified uniquely. Review the open draft before retrying."
      set savedID to item 1 of newDraftIDs
      if savedID < 1 then error "Mail did not return a persistent saved draft identifier."
      set savedDraft to first message of drafts mailbox whose id is savedID
      set resultID to "mail-message:" & savedID
      set destinationName to name of mailbox of savedDraft
      my putField(observed, "savedSubject", subject of savedDraft)
      my putField(observed, "savedBody", (content of savedDraft) as text)
      set savedAddresses to address of every to recipient of savedDraft
      my putField(observed, "savedTo", current application's NSArray's arrayWithArray:savedAddresses)
      my putField(observed, "savedCcCount", count of cc recipients of savedDraft)
      my putField(observed, "savedBccCount", count of bcc recipients of savedDraft)
      my putField(observed, "identifierScope", "mail_library")
      activate
    end tell
"#;

// argv: Mail's message number, its Message-ID, the sender's address, subject, body.
// The email is found again and checked against all three before Mail's own reply
// command runs, so a changed inbox fails safely instead of answering someone else.
const REPLY: &str = r#"    tell application "Mail"
      set destinationName to "Mail draft"
      set originalNumber to (item 1 of argv) as integer
      set found to (every message of inbox whose id is originalNumber)
      if (count of found) is 0 then
        repeat with a in accounts
          repeat with b in mailboxes of a
            set found to found & (every message of b whose id is originalNumber)
          end repeat
        end repeat
      end if
      if (count of found) is not 1 then error "The email to answer is no longer in Mail."
      set original to item 1 of found
      set originalMessageID to message id of original
      if originalMessageID is not (item 2 of argv) then error "The email to answer has changed since you asked. Ask again."
      if (extract address from (sender of original)) is not (item 3 of argv) then error "The email to answer is from someone else now. Ask again."
      my putField(observed, "originalMessageId", originalMessageID)
      set requestedSubject to item 4 of argv
      set knownDraftIDs to id of every message of drafts mailbox whose subject is requestedSubject
      set mutationStarted to true
      set created to reply original with opening window
      set compositionID to id of created
      set resultID to "mail-composition:" & compositionID
      my putField(observed, "compositionId", compositionID)
      set subject of created to requestedSubject
      set content of created to item 5 of argv
      save created
      set stored to first outgoing message whose id is compositionID
      my putField(observed, "subject", subject of stored)
      my putField(observed, "body", (content of stored) as text)
      set recipientAddresses to address of every to recipient of stored
      my putField(observed, "to", current application's NSArray's arrayWithArray:recipientAddresses)
      my putField(observed, "ccCount", count of cc recipients of stored)
      my putField(observed, "bccCount", count of bcc recipients of stored)
      my putField(observed, "visible", visible of stored)
      my putField(observed, "saveAccepted", true)
      my putField(observed, "identifierScope", "open_composition")
      set newDraftIDs to {}
      repeat 20 times
        set matchingDrafts to every message of drafts mailbox whose subject is requestedSubject
        set newDraftIDs to {}
        repeat with candidate in matchingDrafts
          set candidateID to id of candidate
          if candidateID is not in knownDraftIDs then set end of newDraftIDs to candidateID
        end repeat
        if (count of newDraftIDs) > 0 then exit repeat
        delay 0.25
      end repeat
      if (count of newDraftIDs) is not 1 then error "The saved reply could not be identified uniquely. Review the open reply before retrying."
      set savedID to item 1 of newDraftIDs
      if savedID < 1 then error "Mail did not return a persistent saved draft identifier."
      set savedDraft to first message of drafts mailbox whose id is savedID
      set resultID to "mail-message:" & savedID
      set destinationName to name of mailbox of savedDraft
      my putField(observed, "savedSubject", subject of savedDraft)
      my putField(observed, "savedBody", (content of savedDraft) as text)
      set savedAddresses to address of every to recipient of savedDraft
      my putField(observed, "savedTo", current application's NSArray's arrayWithArray:savedAddresses)
      my putField(observed, "savedCcCount", count of cc recipients of savedDraft)
      my putField(observed, "savedBccCount", count of bcc recipients of savedDraft)
      my putField(observed, "savedHeaders", all headers of savedDraft)
      my putField(observed, "identifierScope", "mail_library")
      activate
    end tell
"#;

// argv: note id, its title as read, the HTML to add. The title is compared with case
// considered, and a locked note or one with attachments is refused before anything is
// written: setting a body through scripting can drop attachments.
const NOTE_APPEND: &str = r#"    tell application "Notes"
      set stored to note id (item 1 of argv)
      considering case
        if my trimmed(name of stored) is not my trimmed(item 2 of argv) then error "The note’s title has changed since Buntini read it. Ask again so it reads the note afresh."
      end considering
      if password protected of stored then error "That note is locked, so Buntini can’t add to it."
      if (count of attachments of stored) > 0 then error "That note has attachments, which Notes could drop when Buntini adds text. Add the text in Notes yourself."
      set targetFolder to my folderOf(id of stored)
      set destinationName to name of targetFolder
      if destinationName is "Recently Deleted" then error "That note is in Recently Deleted, so Buntini won’t add to it."
      -- Setting a body through scripting rewrites it whole, and Notes' checklists (with
      -- what's ticked) and tables don't survive that; plain text does.
      set storedBody to body of stored
      if storedBody contains "<ul" or storedBody contains "<ol" or storedBody contains "<table" then error "That note has a list, checklist or table, which Notes could change when Buntini adds text. Add the text in Notes yourself."
      my putField(observed, "destinationId", id of targetFolder)
      my putField(observed, "beforeTitle", name of stored)
      my putField(observed, "beforePlaintext", plaintext of stored)
      set mutationStarted to true
      set body of stored to storedBody & item 3 of argv
      set resultID to id of stored
      set stored to note id resultID
      my putField(observed, "title", name of stored)
      my putField(observed, "plaintext", plaintext of stored)
      my putField(observed, "folderId", id of my folderOf(resultID))
      my putField(observed, "attachmentCount", count of attachments of stored)
      my putField(observed, "identifierScope", "persistent")
    end tell
"#;

/// The folder holding a note. Notes sometimes refuses `container of` a note it just
/// returned (error -1728), so the folders' own note lists answer when it does.
const FOLDER_HELPER: &str = r#"on folderOf(noteID)
  tell application "Notes"
    try
      return container of note id noteID
    end try
    repeat with f in folders
      if (id of notes of f) contains noteID then return contents of f
    end repeat
  end tell
  error "Notes didn’t say which folder the note is in."
end folderOf
"#;

/// Text without spaces, tabs or line breaks around it, as Buntini reads titles.
const TRIM_HELPER: &str = r#"on trimmed(s)
  set blanks to {space, tab, return, linefeed, character id 160}
  repeat while s is not "" and blanks contains (character 1 of s)
    if length of s is 1 then return ""
    set s to text 2 thru -1 of s
  end repeat
  repeat while s is not "" and blanks contains (character -1 of s)
    if length of s is 1 then return ""
    set s to text 1 thru -2 of s
  end repeat
  return s
end trimmed
"#;

fn verified_script(body: &str) -> String {
    format!("{RECEIPT_HELPER}{DATE_HELPER}{RECEIPT_START}{body}{RECEIPT_END}")
}

/// The script and its arguments for an AppleScript-backed action.
pub fn script(tool: &str, args: &Map<String, Value>) -> Option<(String, Vec<String>)> {
    let get = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let parts = |value: &str| {
        when(value)
            .map(|(y, m, d, h, mi)| {
                vec![
                    y.to_string(),
                    m.to_string(),
                    d.to_string(),
                    h.to_string(),
                    mi.to_string(),
                ]
            })
            .unwrap_or_default()
    };
    match tool {
        "add_reminder" => {
            let mut argv = vec![get("title")];
            argv.extend(parts(&get("due")));
            Some((verified_script(REMINDER), argv))
        }
        "add_calendar_event" => {
            let mut argv = vec![get("title")];
            argv.extend(parts(&get("start")));
            argv.push(
                args.get("minutes")
                    .and_then(Value::as_u64)
                    .unwrap_or(60)
                    .to_string(),
            );
            argv.push(get("location"));
            Some((verified_script(EVENT), argv))
        }
        "create_note" => Some((
            format!("{RECEIPT_HELPER}{FOLDER_HELPER}{DATE_HELPER}{RECEIPT_START}{NOTE}{RECEIPT_END}"),
            vec![
                get("title"),
                format!(
                    "<h1>{}</h1>{}",
                    html(&get("title"))
                        .trim_start_matches("<div>")
                        .trim_end_matches("</div>"),
                    html(&get("body"))
                ),
            ],
        )),
        "draft_email" => Some((
            verified_script(MAIL),
            vec![get("to"), get("subject"), get("body")],
        )),
        "reply_email" => Some((
            verified_script(REPLY),
            vec![
                get("message"),
                get("message_id"),
                get("to"),
                get("subject"),
                get("body"),
            ],
        )),
        "append_to_note" => Some((
            format!("{RECEIPT_HELPER}{TRIM_HELPER}{FOLDER_HELPER}{DATE_HELPER}{RECEIPT_START}{NOTE_APPEND}{RECEIPT_END}"),
            vec![get("id"), get("title"), html(&get("text"))],
        )),
        _ => None,
    }
}

/// The Apple helper's arguments for a Reminders or Calendar change (EventKit, which
/// owns the identifiers `read_reminders` and `read_calendar` return). The helper checks
/// the item still is what was read before writing, and reads it back afterwards.
pub fn helper_arguments(tool: &str, args: &Map<String, Value>) -> Option<Vec<String>> {
    let get = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match tool {
        "complete_reminder" => Some(vec!["--complete-reminder".into(), get("id"), get("title")]),
        "reschedule_reminder" => Some(vec![
            "--reschedule-reminder".into(),
            get("id"),
            get("title"),
            get("was_due"),
            get("due"),
        ]),
        "update_calendar_event" => Some(vec![
            "--update-event".into(),
            json!({"id":get("id"), "occurrence":get("occurrence"), "wasTitle":get("was_title"),
                "title":get("title"), "start":get("start"), "location":get("location"),
                "minutes":args.get("minutes").and_then(Value::as_u64).unwrap_or(60)})
            .to_string(),
        ]),
        _ => None,
    }
}

const OUTPUT_LIMIT: u64 = 1024 * 1024;

async fn read_output(stream: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    stream
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Couldn’t read the helper’s response.".to_string())?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err("The helper returned too much data. Narrow the request and try again.".into());
    }
    Ok(bytes)
}

/// Return complete helper output or an error. Truncating successful output can
/// turn a partial JSON row into missing calendar events, reminders or shortcuts.
#[derive(Debug)]
struct HelperFailure {
    started: bool,
    message: String,
}

async fn command_output(mut command: Command, seconds: u64) -> Result<String, HelperFailure> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| HelperFailure {
        started: false,
        message: "Couldn’t start the helper for that.".into(),
    })?;
    let result = async {
        let stdout = child.stdout.take().ok_or("Couldn’t read the helper’s response.")?;
        let stderr = child.stderr.take().ok_or("Couldn’t read the helper’s response.")?;
        let (stdout, stderr, status) = tokio::time::timeout(Duration::from_secs(seconds), async {
            tokio::try_join!(read_output(stdout), read_output(stderr), async {
                child.wait().await.map_err(|_| "Couldn’t wait for the helper to finish.".to_string())
            })
        }).await.map_err(|_| "That took too long to confirm. Check the app before repeating an action.".to_string())??;
        if status.success() {
            String::from_utf8(stdout).map_err(|_| "The helper returned invalid text.".into())
        } else {
            let detail = String::from_utf8_lossy(&stderr).to_string();
            Err(if detail.contains("-1743") || detail.contains("Not authorized") {
                "macOS didn’t allow Buntini to use that app. Allow it in System Settings → Privacy & Security → Automation.".into()
            } else {
                format!("That didn’t work: {}", detail.trim().chars().take(200).collect::<String>())
            })
        }
    }.await;
    result.map_err(|message| HelperFailure {
        started: true,
        message,
    })
}

/// Keep all runtime values after the option terminator. A separate argv value
/// is insufficient for osascript: a leading `-e` would otherwise become another
/// source fragment, and `-l` or `-s` would change interpreter/output behavior.
pub(crate) fn osascript_command(language: &str, source: &str) -> Command {
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-l", language, "-e", source, "--"]);
    command
}

pub async fn output(command: Command, seconds: u64) -> Result<String, String> {
    command_output(command, seconds)
        .await
        .map_err(|failure| failure.message)
}

pub fn find_app(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let bundle = if name.to_lowercase().ends_with(".app") {
        name.to_string()
    } else {
        format!("{name}.app")
    };
    let home = std::env::var("HOME")
        .map(|home| PathBuf::from(home).join("Applications"))
        .ok();
    [
        "/Applications",
        "/System/Applications",
        "/System/Applications/Utilities",
        "/Applications/Utilities",
    ]
    .iter()
    .map(PathBuf::from)
    .chain(home)
    .map(|dir| dir.join(&bundle))
    .find(|path| path.is_dir())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperReceipt {
    outcome: ActionOutcome,
    object_id: String,
    destination: String,
    evidence: Map<String, Value>,
    error: String,
}

fn normalized_plaintext(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end_matches('\n')
        .to_string()
}

fn check_evidence(
    tool: &str,
    args: &Map<String, Value>,
    receipt: &HelperReceipt,
) -> Result<(), String> {
    let approved = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let observed = |key: &str| receipt.evidence.get(key).and_then(Value::as_str);
    let matches = |key: &str, expected: &str| -> Result<(), String> {
        if observed(key) == Some(expected) {
            Ok(())
        } else {
            Err(format!(
                "The saved {key} did not match the approved details."
            ))
        }
    };
    if receipt.object_id.trim().is_empty() || receipt.destination.trim().is_empty() {
        return Err("The app did not return an identifiable destination object.".into());
    }
    if !["draft_email", "reply_email"].contains(&tool) {
        if observed("identifierScope") != Some("persistent")
            || observed("destinationId").is_none_or(str::is_empty)
        {
            return Err(
                "The app did not return persistent object and destination identifiers.".into(),
            );
        }
        // An existing item keeps its own title, which Buntini read trimmed.
        if EDIT_TOOLS.contains(&tool) {
            if observed("title").map(str::trim) != Some(approved("title")) {
                return Err("The saved title did not match the approved details.".into());
            }
        } else {
            matches("title", approved("title"))?;
        }
    }
    match tool {
        "add_reminder" => {
            matches("due", approved("due"))?;
            matches("remindAt", approved("due"))?;
            let seconds = if approved("due").is_empty() {
                json!("")
            } else {
                json!(0)
            };
            if receipt.evidence.get("dueSecond") != Some(&seconds)
                || receipt.evidence.get("remindSecond") != Some(&seconds)
            {
                return Err(
                    "The saved reminder time did not exactly match the approved minute.".into(),
                );
            }
            if receipt.evidence.get("completed").and_then(Value::as_bool) != Some(false) {
                return Err("The reminder could not be verified as incomplete.".into());
            }
        }
        "add_calendar_event" => {
            matches("start", approved("start"))?;
            matches("location", approved("location"))?;
            if receipt.evidence.get("startSecond").and_then(Value::as_u64) != Some(0)
                || receipt.evidence.get("endSecond").and_then(Value::as_u64) != Some(0)
            {
                return Err(
                    "The saved event time did not exactly match the approved minute.".into(),
                );
            }
            if receipt
                .evidence
                .get("durationSeconds")
                .and_then(Value::as_u64)
                != args
                    .get("minutes")
                    .and_then(Value::as_u64)
                    .and_then(|minutes| minutes.checked_mul(60))
            {
                return Err("The saved event length did not match the approved details.".into());
            }
        }
        "create_note" => {
            let expected = format!("{}\n{}", approved("title"), approved("body"));
            if observed("plaintext").map(normalized_plaintext)
                != Some(normalized_plaintext(&expected))
            {
                return Err(
                    "The saved note text did not match the approved title and body.".into(),
                );
            }
        }
        "draft_email" => {
            matches("subject", approved("subject"))?;
            matches("savedSubject", approved("subject"))?;
            if observed("body").map(normalized_plaintext)
                != Some(normalized_plaintext(approved("body")))
                || observed("savedBody").map(normalized_plaintext)
                    != Some(normalized_plaintext(approved("body")))
            {
                return Err("The draft message did not match the approved body.".into());
            }
            let recipients = if approved("to").is_empty() {
                json!([])
            } else {
                json!([approved("to")])
            };
            if receipt.evidence.get("to") != Some(&recipients)
                || receipt.evidence.get("ccCount").and_then(Value::as_u64) != Some(0)
                || receipt.evidence.get("bccCount").and_then(Value::as_u64) != Some(0)
                || receipt.evidence.get("savedTo") != Some(&recipients)
                || receipt.evidence.get("savedCcCount").and_then(Value::as_u64) != Some(0)
                || receipt
                    .evidence
                    .get("savedBccCount")
                    .and_then(Value::as_u64)
                    != Some(0)
            {
                return Err("The draft recipients did not match the approved details.".into());
            }
            if receipt.evidence.get("visible").and_then(Value::as_bool) != Some(true)
                || receipt
                    .evidence
                    .get("saveAccepted")
                    .and_then(Value::as_bool)
                    != Some(true)
                || observed("identifierScope") != Some("mail_library")
            {
                return Err("The draft could not be verified as saved and open for review.".into());
            }
        }
        "reply_email" => {
            matches("subject", approved("subject"))?;
            matches("savedSubject", approved("subject"))?;
            // Mail keeps the quoted original and the user's signature around the reply,
            // so the approved text must be there, not be all there is.
            let body = normalized_plaintext(approved("body"));
            if !["body", "savedBody"].iter().all(|key| {
                observed(key).is_some_and(|text| normalized_plaintext(text).contains(&body))
            }) {
                return Err("The reply did not contain the approved message.".into());
            }
            // Mail picks the recipient of its own reply. It must be the sender who was
            // approved, alone; anyone else (a Reply-To address, say) is a mismatch.
            let only_sender = |key: &str| {
                receipt.evidence.get(key).and_then(Value::as_array).is_some_and(|to| {
                    to.len() == 1
                        && to[0]
                            .as_str()
                            .is_some_and(|address| address.eq_ignore_ascii_case(approved("to")))
                })
            };
            if !only_sender("to")
                || !only_sender("savedTo")
                || ["ccCount", "bccCount", "savedCcCount", "savedBccCount"]
                    .iter()
                    .any(|key| receipt.evidence.get(*key).and_then(Value::as_u64) != Some(0))
            {
                return Err(
                    "Mail addressed the reply to someone other than the sender of the email.".into(),
                );
            }
            let original = bare_message_id(approved("message_id"));
            if observed("originalMessageId").map(bare_message_id) != Some(original) {
                return Err("Mail answered a different email than the one approved.".into());
            }
            if !observed("savedInReplyTo").is_some_and(|ids| {
                ids.split_whitespace().any(|id| bare_message_id(id) == original)
            }) {
                return Err("The saved reply is not threaded to the original email.".into());
            }
            if receipt.evidence.get("visible").and_then(Value::as_bool) != Some(true)
                || receipt
                    .evidence
                    .get("saveAccepted")
                    .and_then(Value::as_bool)
                    != Some(true)
                || observed("identifierScope") != Some("mail_library")
            {
                return Err("The reply could not be verified as saved and open for review.".into());
            }
        }
        "complete_reminder" | "reschedule_reminder" | "update_calendar_event" => check_change(tool, args, receipt)?,
        "append_to_note" => {
            let before = observed("beforePlaintext").map(normalized_plaintext);
            let after = observed("plaintext").map(normalized_plaintext);
            let added = normalized_plaintext(approved("text"));
            // Everything that was there stays first, unchanged; the new text follows on
            // its own line and is the only addition.
            let appended = before.as_deref().zip(after.as_deref()).is_some_and(|(before, after)| {
                after.strip_prefix(before).is_some_and(|rest| {
                    rest.starts_with('\n') && rest.trim_start_matches('\n') == added
                })
            });
            if !appended {
                return Err("The note did not read back as the approved text added to the end of what was there.".into());
            }
            if observed("beforeTitle").map(str::trim) != Some(approved("title")) {
                return Err("The saved beforeTitle did not match the approved details.".into());
            }
            if observed("folderId") != observed("destinationId")
                || receipt.evidence.get("attachmentCount").and_then(Value::as_u64) != Some(0)
            {
                return Err("The note read back in a different folder or with different attachments.".into());
            }
        }
        _ => return Err("This action does not have an app-record verifier.".into()),
    }
    Ok(())
}

/// "<a@b>" and "a@b" are the same Message-ID.
fn bare_message_id(id: &str) -> &str {
    id.trim().trim_start_matches('<').trim_end_matches('>')
}

/// The Message-IDs a saved draft answers, from its In-Reply-To and References headers
/// (continuation lines included). Only these are kept; the other headers are dropped.
fn answered_ids(headers: &str) -> String {
    let mut unfolded: Vec<String> = Vec::new();
    for line in headers.replace("\r\n", "\n").replace('\r', "\n").lines() {
        match unfolded.last_mut() {
            Some(previous) if line.starts_with([' ', '\t']) => previous.push_str(line),
            _ => unfolded.push(line.to_string()),
        }
    }
    unfolded
        .iter()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| {
            ["in-reply-to", "references"].contains(&name.trim().to_ascii_lowercase().as_str())
        })
        .flat_map(|(_, value)| value.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fields of a reminder or event that a change must leave exactly as they were.
const KEPT: [(&str, &[&str]); 3] = [
    ("complete_reminder", &["title", "due", "listId", "notes", "priority", "url", "alarms"]),
    ("reschedule_reminder", &["title", "listId", "notes", "priority", "url", "alarms", "completed"]),
    ("update_calendar_event", &["calendarId", "allDay", "notes", "url", "alarms"]),
];

/// The helper reports the item as read just before the change and again afterwards.
/// Only the approved change may differ, and the item must have been what was approved.
fn check_change(
    tool: &str,
    args: &Map<String, Value>,
    receipt: &HelperReceipt,
) -> Result<(), String> {
    let approved = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let side = |name: &str| receipt.evidence.get(name).and_then(Value::as_object);
    let (Some(before), Some(after)) = (side("before"), side("after")) else {
        return Err("The app did not report the item before and after the change.".into());
    };
    // Buntini trims what it reads and what the sheet sends, so the app's side is trimmed too.
    let text = |fields: &Map<String, Value>, key: &str| fields.get(key).and_then(Value::as_str).map(|value| value.trim().to_owned());
    let kept = KEPT.iter().find(|(name, _)| *name == tool).map(|(_, keys)| *keys).unwrap_or(&[]);
    for key in kept {
        if before.get(*key).is_none() || before.get(*key) != after.get(*key) {
            return Err(format!("Something else about it changed too ({key}). Check it in the app."));
        }
    }
    if tool == "complete_reminder" {
        if text(before, "title").as_deref() != Some(approved("title"))
            || before.get("completed") != Some(&json!(false))
        {
            return Err("The reminder was not the open one you approved.".into());
        }
        if after.get("completed") != Some(&json!(true)) {
            return Err("The reminder did not read back as done.".into());
        }
        return Ok(());
    }
    if tool == "reschedule_reminder" {
        if text(before, "title").as_deref() != Some(approved("title"))
            || before.get("completed") != Some(&json!(false))
            || text(before, "due").as_deref() != Some(approved("was_due"))
        {
            return Err("The reminder was not the open one you approved.".into());
        }
        if text(after, "due").as_deref() != Some(approved("due")) {
            return Err("The reminder did not read back with the approved due date.".into());
        }
        // An alarm at a set time moves with the due date; one set relative to it
        // follows on its own. Either way each alarm keeps its place relative to the due.
        if before.get("alarmOffsets").is_none_or(|offsets| !offsets.is_array())
            || before.get("alarmOffsets") != after.get("alarmOffsets")
        {
            return Err("The reminder's alarms did not move with its due date. Check them in Reminders.".into());
        }
        return Ok(());
    }
    if text(before, "title").as_deref() != Some(approved("was_title"))
        || text(before, "start").as_deref() != Some(approved("occurrence"))
        || before.get("allDay") != Some(&json!(false))
    {
        return Err("The event was not the one you approved.".into());
    }
    let minutes = args.get("minutes").and_then(Value::as_u64).unwrap_or(0);
    if text(after, "title").as_deref() != Some(approved("title"))
        || text(after, "start").as_deref() != Some(approved("start"))
        || after.get("startSecond") != Some(&json!(0))
        || after.get("durationSeconds").and_then(Value::as_u64) != minutes.checked_mul(60)
        || text(after, "location").as_deref() != Some(approved("location"))
    {
        return Err("The event did not read back with the approved title, time, length and place.".into());
    }
    if text(after, "calendarId").as_deref() != receipt.evidence.get("destinationId").and_then(Value::as_str) {
        return Err("The event read back in a different calendar.".into());
    }
    if approved("start") != approved("occurrence")
        && receipt.evidence.get("oldSlotVacated") != Some(&json!(true))
    {
        return Err("The event still shows at its old time as well.".into());
    }
    // A repeating event changes one occurrence only: the next one keeps its time.
    if before.get("recurring") == Some(&json!(true))
        && (receipt.evidence.get("span").and_then(Value::as_str) != Some("this_occurrence")
            || receipt.evidence.get("nextBefore").is_none_or(|next| !next.is_string())
            || receipt.evidence.get("nextBefore") != receipt.evidence.get("nextAfter"))
    {
        return Err("Other occurrences of this repeating event may have changed. Check them in Calendar.".into());
    }
    Ok(())
}

fn parse_receipt(tool: &str, args: &Map<String, Value>, response: &str) -> ActionReceipt {
    let mut helper: HelperReceipt = match serde_json::from_str(response) {
        Ok(receipt) => receipt,
        Err(_) => {
            return ActionReceipt::failure(
                tool,
                ActionOutcome::Unknown,
                "The helper returned no valid verification receipt.",
            )
        }
    };
    if tool == "reply_email" {
        if let Some(headers) = helper.evidence.remove("savedHeaders") {
            let ids = answered_ids(headers.as_str().unwrap_or_default());
            helper.evidence.insert("savedInReplyTo".into(), Value::String(ids));
        }
    }
    let detail = if helper.outcome == ActionOutcome::Succeeded {
        if !helper.error.is_empty() {
            Some("The helper reported an error alongside its success receipt.".into())
        } else {
            check_evidence(tool, args, &helper).err()
        }
    } else {
        Some(if helper.error.is_empty() {
            "The app did not confirm the action.".into()
        } else {
            helper.error.clone()
        })
    };
    let outcome = if (helper.outcome == ActionOutcome::Succeeded && detail.is_some())
        || (helper.outcome == ActionOutcome::Failed && !helper.object_id.is_empty())
    {
        ActionOutcome::Unknown
    } else {
        helper.outcome.clone()
    };
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let mut receipt = if let Some(detail) = detail {
        ActionReceipt::failure(tool, outcome, detail)
    } else {
        let message = match tool {
            "add_reminder" if !get("due").is_empty() => format!("Verified “{}” in Reminders for {}.", get("title"), get("due")),
            "add_reminder" => format!("Verified “{}” in Reminders.", get("title")),
            "add_calendar_event" => format!("Verified “{}” in {} at {}.", get("title"), helper.destination, get("start")),
            "create_note" => format!("Verified “{}” in Notes.", get("title")),
            "reply_email" => format!("Your reply to {} is saved and open in Mail, in the same thread as their email. I checked its recipient, subject and message; review it before sending.", get("to")),
            "complete_reminder" => format!("Marked “{}” as done in Reminders. I checked it’s done and nothing else about it changed.", get("title")),
            "reschedule_reminder" => format!("Moved “{}” in Reminders: it’s now due {}. I read it back and nothing else about it changed.", get("title"), get("due")),
            "update_calendar_event" => format!("Changed “{}” in {}: it’s now “{}” at {}{}. I read it back and nothing else about it changed.", get("was_title"), helper.destination, get("title"), get("start"), if get("location").is_empty() { String::new() } else { format!(", {}", get("location")) }),
            "append_to_note" => format!("Added your text to the end of “{}” in Notes. I read the note back and the text that was there is unchanged.", get("title")),
            _ => "Your draft is saved and open in Mail. I checked its recipients, subject and message; review it before sending.".into(),
        };
        ActionReceipt {
            outcome,
            tool: tool.into(),
            object_id: None,
            destination: None,
            message,
            evidence: json!({}),
            error: None,
        }
    };
    receipt.object_id = (!helper.object_id.is_empty()).then_some(helper.object_id);
    receipt.destination = (!helper.destination.is_empty()).then_some(helper.destination);
    receipt.evidence = Value::Object(kept_evidence(tool, helper.evidence));
    receipt
}

/// A note's whole text is needed to check an append, not to keep: the journal keeps its
/// lengths and what follows the old text, so a long note isn't copied into every task.
fn kept_evidence(tool: &str, mut evidence: Map<String, Value>) -> Map<String, Value> {
    if tool == "append_to_note" {
        let text = |value: Option<Value>| value.and_then(|v| v.as_str().map(normalized_plaintext)).unwrap_or_default();
        let before = text(evidence.remove("beforePlaintext"));
        let after = text(evidence.remove("plaintext"));
        let added = after.strip_prefix(before.as_str()).map(str::to_owned);
        evidence.insert("beforeCharacters".into(), json!(before.chars().count()));
        evidence.insert("afterCharacters".into(), json!(after.chars().count()));
        evidence.insert("addedAtEnd".into(), json!(added));
    }
    evidence
}

async fn dispatched_receipt(
    tool: &str,
    command: Command,
    seconds: u64,
    message: String,
    evidence: Value,
) -> ActionReceipt {
    match command_output(command, seconds).await {
        Ok(_) => ActionReceipt {
            outcome: ActionOutcome::Succeeded,
            tool: tool.into(),
            object_id: None,
            destination: None,
            message,
            evidence,
            error: None,
        },
        Err(failure) => ActionReceipt::failure(
            tool,
            if failure.started {
                ActionOutcome::Unknown
            } else {
                ActionOutcome::Failed
            },
            failure.message,
        ),
    }
}

/// Execute exactly one approved intent. The caller journals its idempotency key
/// before calling this, and must preserve Unknown receipts across restarts.
pub async fn execute(tool: &str, args: &Map<String, Value>) -> ActionReceipt {
    execute_with(tool, args, Path::new("")).await
}

/// `execute`, with the Apple helper that changes Reminders and Calendar items.
pub async fn execute_with(tool: &str, args: &Map<String, Value>, helper: &Path) -> ActionReceipt {
    let args = match validate(tool, &Value::Object(args.clone())) {
        Ok(args) => args,
        Err(error) => return ActionReceipt::failure(tool, ActionOutcome::Failed, error),
    };
    if EDIT_TOOLS.contains(&tool) && !args.contains_key("id") {
        return ActionReceipt::failure(tool, ActionOutcome::Failed, "Pick which one to change first.");
    }
    if tool == "reply_email" && !args.contains_key("message") {
        return ActionReceipt::failure(tool, ActionOutcome::Failed, "Pick which email to answer first.");
    }
    if let Some(argv) = helper_arguments(tool, &args) {
        if !helper.is_file() {
            return ActionReceipt::failure(tool, ActionOutcome::Failed, "Changing Reminders and Calendar needs Buntini’s Apple helper, which isn’t on this Mac.");
        }
        let mut command = Command::new(helper);
        command.args(argv);
        // The first use may wait for the macOS permission prompt.
        return match command_output(command, 90).await {
            Ok(response) => parse_receipt(tool, &args, &response),
            Err(failure) => ActionReceipt::failure(
                tool,
                if failure.started { ActionOutcome::Unknown } else { ActionOutcome::Failed },
                failure.message,
            ),
        };
    }
    let get = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    if let Some((source, argv)) = script(tool, &args) {
        let mut command = osascript_command("AppleScript", &source);
        command.args(argv);
        return match command_output(command, 30).await {
            Ok(response) => parse_receipt(tool, &args, &response),
            Err(failure) => ActionReceipt::failure(
                tool,
                if failure.started {
                    ActionOutcome::Unknown
                } else {
                    ActionOutcome::Failed
                },
                failure.message,
            ),
        };
    }
    match tool {
        "run_shortcut" => {
            let name = get("name");
            let mut listing = Command::new("/usr/bin/shortcuts");
            listing.arg("list");
            let list = match output(listing, 15).await {
                Ok(list) => list,
                Err(error) => return ActionReceipt::failure(tool, ActionOutcome::Failed, error),
            };
            let Some(exact) = list
                .lines()
                .map(str::trim)
                .find(|line| line.eq_ignore_ascii_case(&name))
            else {
                return ActionReceipt::failure(
                    tool,
                    ActionOutcome::Failed,
                    format!("I couldn’t find a shortcut called “{name}”."),
                );
            };
            let mut command = Command::new("/usr/bin/shortcuts");
            // A shortcut name may itself begin with a CLI option such as
            // --help; it must remain the approved positional name.
            command.arg("run").arg("--").arg(exact);
            dispatched_receipt(tool, command, 120, format!("The “{exact}” shortcut finished. Its individual app changes have not been verified."), json!({"verification":"shortcut_process_completed", "name":exact})).await
        }
        "open_app" => {
            let name = get("name");
            let Some(path) = find_app(&name) else {
                return ActionReceipt::failure(
                    tool,
                    ActionOutcome::Failed,
                    format!("I couldn’t find an app called “{name}” in Applications."),
                );
            };
            let mut command = Command::new("/usr/bin/open");
            command.arg(&path);
            dispatched_receipt(
                tool,
                command,
                15,
                format!("macOS accepted the request to open {name}."),
                json!({"verification":"launch_services_accepted", "path":path}),
            )
            .await
        }
        "open_link" => {
            let url = get("url");
            let mut command = Command::new("/usr/bin/open");
            command.arg(&url);
            dispatched_receipt(tool, command, 15, format!("macOS accepted the request to open {url}. The page contents have not been verified."), json!({"verification":"launch_services_accepted", "url":url})).await
        }
        _ => ActionReceipt::failure(tool, ActionOutcome::Failed, "Unknown action."),
    }
}

/// Compatibility for direct callers that only display text. Durable task runners
/// should retain the complete receipt from execute instead.
pub async fn run(tool: &str, args: &Map<String, Value>) -> Result<String, String> {
    let receipt = execute(tool, args).await;
    if receipt.outcome == ActionOutcome::Succeeded {
        Ok(receipt.message)
    } else {
        Err(receipt.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposals_are_checked_and_normalised() {
        let reminder = validate(
            "add_reminder",
            &json!({"title":" Call mum ","due":"2026-10-22 18:00"}),
        )
        .unwrap();
        assert_eq!(reminder["title"], "Call mum");
        assert!(validate("add_reminder", &json!({"title":"x","due":"tomorrow"})).is_err());
        assert!(
            validate("add_reminder", &json!({"title":"x","list":"Work"})).is_err(),
            "unknown keys are refused"
        );
        let event = validate(
            "add_calendar_event",
            &json!({"title":"Lunch","start":"2026-10-24 13:00"}),
        )
        .unwrap();
        assert_eq!(event["minutes"], 60);
        assert!(validate(
            "add_calendar_event",
            &json!({"title":"Lunch","start":"2026-10-24 13:00","minutes":2})
        )
        .is_err());
        assert_eq!(
            validate("open_link", &json!({"url":"github.com"})).unwrap()["url"],
            "https://github.com/"
        );
        assert!(validate("open_link", &json!({"url":"file:///etc/passwd"})).is_err());
        assert!(validate("open_link", &json!({"url":"http://example.com"})).is_err());
        assert!(
            validate(
                "draft_email",
                &json!({"to":"a@b.com, c@d.com","subject":"s","body":"b"})
            )
            .unwrap()
            .get("to")
            .is_none(),
            "one recipient, no lists"
        );
        assert!(
            validate(
                "draft_email",
                &json!({"to":"[Landlord's Email]","subject":"s","body":"b"})
            )
            .unwrap()
            .get("to")
            .is_none(),
            "placeholders are left for the user"
        );
        assert_eq!(
            validate(
                "add_calendar_event",
                &json!({"title":"Lunch","start":"2026-10-23T13:00:00"})
            )
            .unwrap()["start"],
            "2026-10-23 13:00"
        );
        assert_eq!(
            validate("add_reminder", &json!({"title":"Bins","due":"2026-10-23"})).unwrap()["due"],
            "2026-10-23 09:00"
        );
        assert!(validate("send_email", &json!({})).is_err());
        assert!(find_app("../../bin/sh").is_none());
    }

    #[test]
    fn external_links_reject_private_hosts_and_preserve_public_http_and_fragments() {
        for value in [
            "https://127.0.0.1/admin",
            "http://2130706433/admin",
            "https://[::1]/",
            "https://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://localhost/",
            "https://tauri.localhost/",
            "https://office.internal/",
            "https://office.local/",
            "https://example.com:8443/",
            "http://example.com:443/",
            "https://name:password@example.com/",
            "https://example.com/\nadmin",
            " https://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert!(public_web_url(value).is_err(), "{value}");
        }
        for value in [
            "http://example.com/article#section",
            "https://example.com/article#section",
        ] {
            assert_eq!(public_web_url(value).unwrap().as_str(), value);
        }
        for value in [
            "127.0.0.1",
            "192.168.1.1",
            "office.local",
            "example.com:8443",
        ] {
            assert!(
                validate("open_link", &json!({"url":value})).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn script_arguments_are_separated_from_interpreter_options() {
        let mut command = osascript_command("AppleScript", "on run argv\nreturn argv\nend run");
        command.args(["-e", "error number 123", "-lJavaScript", "--"]);
        let arguments: Vec<_> = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "-l",
                "AppleScript",
                "-e",
                "on run argv\nreturn argv\nend run",
                "--",
                "-e",
                "error number 123",
                "-lJavaScript",
                "--"
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn script_option_prefixes_remain_literal_values_at_runtime() {
        // These fixed scripts only echo argv; no app automation or shell execution.
        // Without the terminator, osascript parses the first value as more source
        // and rejects the invocation instead of delivering the original values.
        for (language, source) in [
            (
                "AppleScript",
                "on run argv\nset text item delimiters to \"|\"\nreturn argv as text\nend run",
            ),
            (
                "JavaScript",
                "function run(argv) { return argv.join('|'); }",
            ),
        ] {
            for values in [
                vec!["-e", "error number 123"],
                vec!["-eerror number 123", "-lJavaScript", "--"],
                vec!["-s", "s", "-e"],
            ] {
                let mut command = osascript_command(language, source);
                command.args(&values);
                assert_eq!(
                    output(command, 5).await.unwrap().trim_end(),
                    values.join("|")
                );
            }
        }
    }

    #[test]
    fn scripts_take_values_as_arguments_not_code() {
        let args = validate(
            "add_reminder",
            &json!({"title":"\" & do shell script \"rm -rf ~\" & \"","due":"2026-10-22 18:05"}),
        )
        .unwrap();
        let (source, argv) = script("add_reminder", &args).unwrap();
        assert!(!source.contains("rm -rf"), "user text never becomes script");
        assert_eq!(
            argv,
            [
                "\" & do shell script \"rm -rf ~\" & \"",
                "2026",
                "10",
                "22",
                "18",
                "5"
            ]
        );
        let note = validate(
            "create_note",
            &json!({"title":"Ideas","body":"<b>tea</b>\nbiscuits"}),
        )
        .unwrap();
        let (_, argv) = script("create_note", &note).unwrap();
        assert!(
            argv[1].contains("&lt;b&gt;tea&lt;/b&gt;") && argv[1].contains("<div>biscuits</div>")
        );
        let (_, mail) = script(
            "draft_email",
            &validate("draft_email", &json!({"subject":"Leak","body":"Hello"})).unwrap(),
        )
        .unwrap();
        assert_eq!(mail, ["", "Leak", "Hello"]);
    }

    fn helper_receipt(evidence: Value) -> Value {
        json!({"outcome":"succeeded", "objectId":"test-object-123", "destination":"Test destination", "evidence":evidence, "error":""})
    }

    #[test]
    fn success_requires_the_destination_identifier_and_exact_fields() {
        let cases = [
            (
                "add_reminder",
                json!({"title":"Pay rent", "due":"2026-11-01 09:00"}),
                json!({"destinationId":"list-1", "identifierScope":"persistent", "title":"Pay rent", "due":"2026-11-01 09:00", "remindAt":"2026-11-01 09:00", "dueSecond":0, "remindSecond":0, "completed":false}),
            ),
            (
                "add_calendar_event",
                json!({"title":"Dentist", "start":"2026-11-02 10:00", "minutes":30, "location":"Clinic"}),
                json!({"destinationId":"calendar-1", "identifierScope":"persistent", "title":"Dentist", "start":"2026-11-02 10:00", "startSecond":0, "endSecond":0, "durationSeconds":1800, "location":"Clinic"}),
            ),
            (
                "create_note",
                json!({"title":"Trip", "body":"Pack tea\nRemember tickets"}),
                json!({"destinationId":"folder-1", "identifierScope":"persistent", "title":"Trip", "plaintext":"Trip\nPack tea\nRemember tickets\n"}),
            ),
            (
                "draft_email",
                json!({"to":"test@example.com", "subject":"Appointment", "body":"Please confirm.\nThank you."}),
                json!({"identifierScope":"mail_library", "subject":"Appointment", "body":"Please confirm.\r\nThank you.\r\n", "savedSubject":"Appointment", "savedBody":"Please confirm.\nThank you.\n", "savedTo":["test@example.com"], "savedCcCount":0, "savedBccCount":0, "to":["test@example.com"], "ccCount":0, "bccCount":0, "visible":true, "saveAccepted":true}),
            ),
        ];
        for (tool, approved, evidence) in cases {
            let args = validate(tool, &approved).unwrap();
            let valid = helper_receipt(evidence.clone());
            let receipt = parse_receipt(tool, &args, &valid.to_string());
            assert_eq!(
                receipt.outcome,
                ActionOutcome::Succeeded,
                "{tool}: {}",
                receipt.message
            );
            assert_eq!(receipt.object_id.as_deref(), Some("test-object-123"));
            assert_eq!(
                serde_json::from_str::<ActionReceipt>(&serde_json::to_string(&receipt).unwrap())
                    .unwrap(),
                receipt
            );
            for field in evidence.as_object().unwrap().keys() {
                let mut incomplete = valid.clone();
                incomplete["evidence"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert_eq!(
                    parse_receipt(tool, &args, &incomplete.to_string()).outcome,
                    ActionOutcome::Unknown,
                    "{tool}: missing {field} must not pass"
                );
            }
            let mut no_id = valid.clone();
            no_id["objectId"] = json!("");
            assert_eq!(
                parse_receipt(tool, &args, &no_id.to_string()).outcome,
                ActionOutcome::Unknown
            );
            let mut contradiction = valid;
            contradiction["error"] = json!("Writing stopped unexpectedly");
            assert_eq!(
                parse_receipt(tool, &args, &contradiction.to_string()).outcome,
                ActionOutcome::Unknown
            );
        }
    }

    #[test]
    fn incomplete_or_mismatched_receipts_preserve_unknown_outcomes() {
        let args = validate("add_reminder", &json!({"title":"Call mum"})).unwrap();
        for response in [
            "",
            "ok",
            "{}",
            "{\"outcome\":\"succeeded\"}",
            "{\"outcome\":\"succeeded\"} garbage",
        ] {
            let receipt = parse_receipt("add_reminder", &args, response);
            assert_eq!(receipt.outcome, ActionOutcome::Unknown);
            assert!(receipt
                .message
                .contains("will not be repeated automatically"));
        }
        let mut mismatched = helper_receipt(
            json!({"destinationId":"list-1", "identifierScope":"persistent", "title":"Call someone else", "due":"", "remindAt":"", "dueSecond":"", "remindSecond":"", "completed":false}),
        );
        let receipt = parse_receipt("add_reminder", &args, &mismatched.to_string());
        assert_eq!(receipt.outcome, ActionOutcome::Unknown);
        assert_eq!(
            receipt.object_id.as_deref(),
            Some("test-object-123"),
            "retain the existing object for manual recovery"
        );
        mismatched["outcome"] = json!("failed");
        mismatched["error"] = json!("Verification failed after creation");
        assert_eq!(
            parse_receipt("add_reminder", &args, &mismatched.to_string()).outcome,
            ActionOutcome::Unknown,
            "a reported failure with an object ID cannot be replayed safely"
        );
        mismatched["objectId"] = json!("");
        assert_eq!(
            parse_receipt("add_reminder", &args, &mismatched.to_string()).outcome,
            ActionOutcome::Failed
        );
    }

    #[test]
    fn note_and_draft_verifiers_do_not_drop_body_or_recipient_differences() {
        let args = validate(
            "draft_email",
            &json!({"to":"test@example.com", "subject":"Hi", "body":"One\n\nTwo"}),
        )
        .unwrap();
        let mut fixture = helper_receipt(
            json!({"identifierScope":"mail_library", "subject":"Hi", "body":"One\n\nTwo", "savedSubject":"Hi", "savedBody":"One\n\nTwo", "savedTo":["test@example.com"], "savedCcCount":0, "savedBccCount":0, "to":["test@example.com"], "ccCount":0, "bccCount":0, "visible":true, "saveAccepted":true}),
        );
        for (key, value) in [
            ("body", json!("One\nTwo")),
            ("to", json!(["other@example.com"])),
            ("bccCount", json!(1)),
            ("visible", json!(false)),
            ("saveAccepted", json!(false)),
            ("identifierScope", json!("persistent")),
        ] {
            let mut wrong = fixture.clone();
            wrong["evidence"][key] = value;
            assert_eq!(
                parse_receipt("draft_email", &args, &wrong.to_string()).outcome,
                ActionOutcome::Unknown,
                "{key}"
            );
        }
        fixture["evidence"]["to"] = json!([]);
        fixture["evidence"]["savedTo"] = json!([]);
        let blank = validate("draft_email", &json!({"subject":"Hi", "body":"One\n\nTwo"})).unwrap();
        assert_eq!(
            parse_receipt("draft_email", &blank, &fixture.to_string()).outcome,
            ActionOutcome::Succeeded
        );
    }

    fn reply_args() -> Map<String, Value> {
        validate(
            "reply_email",
            &json!({"message":"4821", "message_id":"m-1@example.com", "to":"ana@example.com", "subject":"Re: Friday", "body":"Yes, I'll be there.", "answers":"Ana Silva · “Friday” · 2026-10-29 09:15"}),
        )
        .unwrap()
    }

    fn reply_receipt() -> Value {
        helper_receipt(json!({
            "identifierScope":"mail_library", "originalMessageId":"m-1@example.com",
            "subject":"Re: Friday", "body":"Yes, I'll be there.\n\nOn 29 Oct, Ana wrote:\n> Are you free?",
            "savedSubject":"Re: Friday", "savedBody":"Yes, I'll be there.\n\nOn 29 Oct, Ana wrote:\n> Are you free?",
            "to":["Ana@Example.com"], "ccCount":0, "bccCount":0,
            "savedTo":["ana@example.com"], "savedCcCount":0, "savedBccCount":0,
            "savedHeaders":"From: me@example.com\r\nIn-Reply-To: <m-1@example.com>\r\nReferences: <m-0@example.com>\r\n <m-1@example.com>\r\nSubject: Re: Friday",
            "visible":true, "saveAccepted":true
        }))
    }

    #[test]
    fn replies_need_the_email_they_answer_and_its_sender() {
        let args = reply_args();
        assert_eq!(args["to"], "ana@example.com");
        for (key, value) in [
            ("message", json!("12; delete")),
            ("message", json!("")),
            ("message_id", json!("a b@example.com")),
            ("to", json!("Ana Silva")),
            ("to", json!("")),
            ("body", json!("")),
        ] {
            let mut wrong = Value::Object(args.clone());
            wrong[key] = value;
            assert!(validate("reply_email", &wrong).is_err(), "{key}");
        }
        let mut missing = Value::Object(args.clone());
        missing.as_object_mut().unwrap().remove("message_id");
        assert!(validate("reply_email", &missing).is_err());
        // The script gets every value as an argument, in a fixed order.
        let (source, argv) = script("reply_email", &args).unwrap();
        assert!(source.contains("reply original with opening window"));
        assert!(!source.lines().any(|line| line.trim_start().starts_with("send ")), "never sent");
        assert_eq!(argv, ["4821", "m-1@example.com", "ana@example.com", "Re: Friday", "Yes, I'll be there."]);
        // The sheet shows which email it answers and who it goes to, and can't change them.
        let (title, fields, note) = describe("reply_email", &args);
        assert_eq!(title, "Open this reply in Mail?");
        assert_eq!(
            fields.iter().map(|field| (field.key.as_str(), field.kind)).collect::<Vec<_>>(),
            [("answers", "fixed"), ("to", "fixed"), ("subject", "text"), ("body", "long")]
        );
        assert!(note.contains("never sends"));
        let edited = json!({"to":"eve@example.com", "subject":"Re: Friday!", "body":"Edited", "message":"1"});
        let kept = keep_fixed("reply_email", &Value::Object(args.clone()), &edited);
        assert_eq!(kept["to"], "ana@example.com");
        assert_eq!(kept["message"], "4821");
        assert_eq!(kept["message_id"], "m-1@example.com");
        assert_eq!(kept["subject"], "Re: Friday!");
        assert_eq!(kept["body"], "Edited");
        assert_eq!(keep_fixed("draft_email", &json!({"to":"a@b.co"}), &edited), edited);
    }

    #[test]
    fn a_reply_is_verified_against_its_sender_subject_text_and_thread() {
        let args = reply_args();
        let receipt = parse_receipt("reply_email", &args, &reply_receipt().to_string());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        assert!(receipt.message.contains("ana@example.com") && receipt.message.contains("same thread"));
        // Only the threading headers are kept from the saved draft.
        assert!(receipt.evidence.get("savedHeaders").is_none());
        assert_eq!(receipt.evidence["savedInReplyTo"], "<m-1@example.com> <m-0@example.com> <m-1@example.com>");
        for (key, value) in [
            ("to", json!(["reply-to@example.com"])),
            ("savedTo", json!(["ana@example.com", "sam@example.com"])),
            ("savedCcCount", json!(1)),
            ("savedSubject", json!("Re: Something else")),
            ("savedBody", json!("On 29 Oct, Ana wrote:")),
            ("originalMessageId", json!("other@example.com")),
            ("savedHeaders", json!("In-Reply-To: <other@example.com>")),
            ("visible", json!(false)),
            ("identifierScope", json!("open_composition")),
        ] {
            let mut wrong = reply_receipt();
            wrong["evidence"][key] = value;
            let receipt = parse_receipt("reply_email", &args, &wrong.to_string());
            assert_eq!(receipt.outcome, ActionOutcome::Unknown, "{key}");
            assert!(receipt.message.starts_with("I couldn’t confirm the result."), "{key}");
        }
        let mut unthreaded = reply_receipt();
        unthreaded["evidence"].as_object_mut().unwrap().remove("savedHeaders");
        assert_eq!(parse_receipt("reply_email", &args, &unthreaded.to_string()).outcome, ActionOutcome::Unknown);
        let mismatch = {
            let mut wrong = reply_receipt();
            wrong["evidence"]["to"] = json!(["reply-to@example.com"]);
            parse_receipt("reply_email", &args, &wrong.to_string())
        };
        assert!(mismatch.message.contains("someone other than the sender"), "{}", mismatch.message);
        // Failing before Mail's reply command ran is a safe failure, not an unknown.
        let before = json!({"outcome":"failed", "objectId":"", "destination":"", "evidence":{}, "error":"The email to answer is no longer in Mail. (-2700)"});
        assert_eq!(parse_receipt("reply_email", &args, &before.to_string()).outcome, ActionOutcome::Failed);
    }

    #[tokio::test]
    async fn invalid_actions_do_not_start_and_uncertain_helpers_are_not_retried() {
        let receipt = execute(
            "add_calendar_event",
            &json!({"title":"Lunch", "start":"tomorrow"})
                .as_object()
                .unwrap(),
        )
        .await;
        assert_eq!(receipt.outcome, ActionOutcome::Failed);
        let missing = dispatched_receipt(
            "open_app",
            Command::new("/definitely/not/a/buntini-helper"),
            1,
            "wrong".into(),
            json!({}),
        )
        .await;
        assert_eq!(missing.outcome, ActionOutcome::Failed);
        let dir = tempfile::tempdir().unwrap();
        let attempts = dir.path().join("attempts");
        let mut command = Command::new("/bin/sh");
        // A test-only helper records one mutation, then fails before a receipt.
        command
            .arg("-c")
            .arg("printf x >> \"$1\"; exit 1")
            .arg("fixture")
            .arg(&attempts);
        let result = dispatched_receipt("create_note", command, 2, "wrong".into(), json!({})).await;
        assert_eq!(result.outcome, ActionOutcome::Unknown);
        assert_eq!(std::fs::read_to_string(attempts).unwrap(), "x");
        let mut stalled = Command::new("/bin/sleep");
        stalled.arg("5");
        let result = dispatched_receipt("create_note", stalled, 0, "wrong".into(), json!({})).await;
        assert_eq!(result.outcome, ActionOutcome::Unknown);
    }

    #[test]
    fn sheets_show_every_value() {
        let args = validate(
            "add_calendar_event",
            &json!({"title":"Lunch","start":"2026-10-24 13:00","location":"Café"}),
        )
        .unwrap();
        let (title, fields, _) = describe("add_calendar_event", &args);
        assert_eq!(title, "Add this to your calendar?");
        assert_eq!(
            fields
                .iter()
                .map(|field| field.value.as_str())
                .collect::<Vec<_>>(),
            ["Lunch", "2026-10-24 13:00", "60", "Café"]
        );
    }

    #[test]
    fn zoned_action_times_preserve_the_instant_in_the_local_approval() {
        let eastern = chrono::FixedOffset::west_opt(4 * 60 * 60).unwrap();
        assert_eq!(
            normalize_when_in("2026-10-22T18:00:00Z", &eastern).unwrap(),
            "2026-10-22 14:00"
        );
        assert_eq!(
            normalize_when_in("2026-10-22T00:30:00+02:00", &eastern).unwrap(),
            "2026-10-21 18:30"
        );
        assert_eq!(
            normalize_when_in("2026-10-22 18:00", &eastern).unwrap(),
            "2026-10-22 18:00"
        );
        assert!(normalize_when_in("2026-10-22ZZ", &eastern).is_err());
        assert!(normalize_when_in("2026-02-30 18:00", &eastern).is_err());
    }

    #[tokio::test]
    async fn helper_output_is_complete_and_bounded() {
        let content = "a calendar title with unicode ☕\n".repeat(300);
        let mut command = Command::new("/usr/bin/printf");
        command.arg("%s").arg(&content);
        assert_eq!(output(command, 5).await.unwrap(), content);
        assert_eq!(read_output(&b"complete"[..]).await.unwrap(), b"complete");
        let too_large = vec![b'x'; OUTPUT_LIMIT as usize + 1];
        assert!(read_output(too_large.as_slice())
            .await
            .unwrap_err()
            .contains("too much data"));
        let command = Command::new("/usr/bin/yes");
        assert!(
            output(command, 5)
                .await
                .unwrap_err()
                .contains("too much data"),
            "an unbounded helper must be stopped instead of exhausting memory"
        );
        let mut invalid = Command::new("/usr/bin/printf");
        invalid.arg("\\377");
        assert!(output(invalid, 5)
            .await
            .unwrap_err()
            .contains("invalid text"));
    }

    #[test]
    fn changes_to_existing_items_are_checked_and_passed_as_arguments() {
        let done = validate("complete_reminder", &json!({"id":"r-1","title":" Buy milk "})).unwrap();
        assert_eq!(done["title"], "Buy milk");
        assert!(validate("complete_reminder", &json!({"title":"Buy milk"})).unwrap().get("id").is_none(), "still to be picked");
        for invalid in [json!({"id":"-x","title":"a"}), json!({"id":"a b","title":"a"}), json!({"id":"r-1"}), json!({"id":"r-1","title":"a","list":"Work"})] {
            assert!(validate("complete_reminder", &invalid).is_err(), "{invalid}");
        }
        let moved = validate("update_calendar_event", &json!({"id":"e-1","occurrence":"2026-11-03T15:00","was_title":"Review","title":"Review","start":"2026-11-03 16:00","minutes":"45"})).unwrap();
        assert_eq!((moved["occurrence"].as_str(), moved["minutes"].as_u64()), (Some("2026-11-03 15:00"), Some(45)));
        assert!(validate("update_calendar_event", &json!({"id":"e-1","occurrence":"2026-11-03 15:00","was_title":"Review","title":"Review","start":"Friday","minutes":45})).is_err());
        let (_, argv) = script("append_to_note", &validate("append_to_note", &json!({"id":"n-1","title":"Shopping","text":"<b>milk</b>\neggs"})).unwrap()).unwrap();
        assert_eq!(argv[..2], ["n-1", "Shopping"]);
        assert!(argv[2].contains("&lt;b&gt;milk") && argv[2].contains("<div>eggs</div>"), "note text is shown as written, never as markup");
        assert_eq!(helper_arguments("complete_reminder", &done).unwrap(), ["--complete-reminder", "r-1", "Buy milk"]);
        let change: Value = serde_json::from_str(&helper_arguments("update_calendar_event", &moved).unwrap()[1]).unwrap();
        assert_eq!(change, json!({"id":"e-1","occurrence":"2026-11-03 15:00","wasTitle":"Review","title":"Review","start":"2026-11-03 16:00","minutes":45,"location":""}));
        let (title, fields, _) = describe("update_calendar_event", &moved);
        assert_eq!(title, "Change this event?");
        assert!(fields.iter().filter(|f| ["id", "occurrence", "was_title"].contains(&f.key.as_str())).all(|f| f.kind == "hidden"));
    }

    #[tokio::test]
    async fn a_change_without_its_item_never_starts() {
        let receipt = execute_with("complete_reminder", json!({"title":"Buy milk"}).as_object().unwrap(), Path::new("/definitely/not/a/helper")).await;
        assert_eq!((receipt.outcome, receipt.message.as_str()), (ActionOutcome::Failed, "Pick which one to change first."));
        let receipt = execute_with("complete_reminder", json!({"id":"r-1","title":"Buy milk"}).as_object().unwrap(), Path::new("/definitely/not/a/helper")).await;
        assert_eq!(receipt.outcome, ActionOutcome::Failed);
    }

    fn change_receipt(tool: &str, args: &Map<String, Value>, evidence: Value) -> ActionReceipt {
        parse_receipt(tool, args, &helper_receipt(evidence).to_string())
    }

    #[test]
    fn a_completed_reminder_must_read_back_done_and_otherwise_unchanged() {
        let args = validate("complete_reminder", &json!({"id":"r-1","title":"Buy milk"})).unwrap();
        let before = json!({"title":"Buy milk","due":"2026-11-03 09:00","listId":"list-1","notes":"","priority":0,"url":"","alarms":1,"completed":false});
        let mut after = before.clone();
        after["completed"] = json!(true);
        let valid = json!({"destinationId":"list-1","identifierScope":"persistent","title":"Buy milk","before":before,"after":after});
        let receipt = change_receipt("complete_reminder", &args, valid.clone());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        assert!(receipt.message.contains("Marked “Buy milk” as done"));
        for (side, key, value) in [("after", "completed", json!(false)), ("after", "due", json!("2026-11-04 09:00")), ("after", "notes", json!("changed")),
            ("before", "completed", json!(true)), ("before", "title", json!("Buy milk powder")), ("after", "alarms", json!(0))] {
            let mut wrong = valid.clone();
            wrong[side][key] = value;
            assert_eq!(change_receipt("complete_reminder", &args, wrong).outcome, ActionOutcome::Unknown, "{side}.{key}");
        }
        let mut missing = valid;
        missing["before"].as_object_mut().unwrap().remove("listId");
        missing["after"].as_object_mut().unwrap().remove("listId");
        assert_eq!(change_receipt("complete_reminder", &args, missing).outcome, ActionOutcome::Unknown, "an unreported field is not proof it was kept");
        // A dictated title can end in a space; Buntini read it trimmed.
        let spaced = change_receipt("complete_reminder", &args, valid_spaced());
        assert_eq!(spaced.outcome, ActionOutcome::Succeeded, "{}", spaced.message);
    }

    #[test]
    fn a_moved_reminder_must_read_back_with_the_new_due_and_its_alarms_moved() {
        let args = validate("reschedule_reminder", &json!({"id":"r-1","title":"Dentist","was_due":"2026-11-03 09:00","due":"2026-11-06 09:00"})).unwrap();
        assert_eq!(helper_arguments("reschedule_reminder", &args).unwrap(), ["--reschedule-reminder", "r-1", "Dentist", "2026-11-03 09:00", "2026-11-06 09:00"]);
        // A date without a time stays one; a stray or repeated due date is refused.
        let day = validate("reschedule_reminder", &json!({"id":"r-1","title":"Rent","was_due":"","due":"2026-11-02"})).unwrap();
        assert_eq!((day["was_due"].as_str(), day["due"].as_str()), (Some(""), Some("2026-11-02")));
        assert_eq!(validate("reschedule_reminder", &json!({"id":"r-1","title":"Rent","due":"2026-11-02T10:00"})).unwrap()["due"], "2026-11-02 10:00");
        for invalid in [
            json!({"id":"r-1","title":"Dentist","was_due":"Tuesday","due":"2026-11-06 09:00"}),
            json!({"id":"r-1","title":"Dentist","was_due":"2026-11-03 09:00","due":"2026-11-03 09:00"}),
            json!({"id":"r-1","title":"Dentist","due":"Friday"}),
            json!({"id":"r-1","title":"Dentist","due":"2026-11-06 09:00","list":"Work"}),
        ] {
            assert!(validate("reschedule_reminder", &invalid).is_err(), "{invalid}");
        }
        let (_, fields, _) = describe("reschedule_reminder", &args);
        assert_eq!(fields.iter().find(|f| f.key == "due").map(|f| f.kind), Some("when"));
        assert_eq!(describe("reschedule_reminder", &day).1.iter().find(|f| f.key == "due").map(|f| f.kind), Some("text"));
        let before = json!({"title":"Dentist","due":"2026-11-03 09:00","listId":"list-1","notes":"","priority":0,"url":"","alarms":1,"completed":false,"alarmOffsets":["at:-3600"]});
        let mut after = before.clone();
        after["due"] = json!("2026-11-06 09:00");
        let valid = json!({"destinationId":"list-1","identifierScope":"persistent","title":"Dentist","before":before,"after":after});
        let receipt = change_receipt("reschedule_reminder", &args, valid.clone());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        assert!(receipt.message.contains("now due 2026-11-06 09:00"));
        for (side, key, value) in [
            ("after", "due", json!("2026-11-03 09:00")),
            ("after", "completed", json!(true)),
            ("after", "notes", json!("changed")),
            ("after", "alarmOffsets", json!(["at:-262800"])),
            ("before", "due", json!("2026-11-02 09:00")),
            ("before", "completed", json!(true)),
            ("before", "title", json!("Dentist visit")),
        ] {
            let mut wrong = valid.clone();
            wrong[side][key] = value;
            assert_eq!(change_receipt("reschedule_reminder", &args, wrong).outcome, ActionOutcome::Unknown, "{side}.{key}");
        }
        let mut unreported = valid;
        unreported["before"].as_object_mut().unwrap().remove("alarmOffsets");
        unreported["after"].as_object_mut().unwrap().remove("alarmOffsets");
        assert_eq!(change_receipt("reschedule_reminder", &args, unreported).outcome, ActionOutcome::Unknown, "an older helper can't show its alarms moved");
    }

    fn valid_spaced() -> Value {
        let before = json!({"title":"Buy milk ","due":"","listId":"list-1","notes":"","priority":0,"url":"","alarms":0,"completed":false});
        let mut after = before.clone();
        after["completed"] = json!(true);
        json!({"destinationId":"list-1","identifierScope":"persistent","title":"Buy milk ","before":before,"after":after})
    }

    #[test]
    fn a_note_append_refuses_what_a_rewrite_could_lose() {
        let (source, argv) = script("append_to_note", &validate("append_to_note", &json!({"id":"n-1","title":"Shopping ","text":"milk"})).unwrap()).unwrap();
        assert_eq!(argv[1], "Shopping");
        for guard in ["Recently Deleted", "contains \"<ul\"", "contains \"<table\"", "my trimmed(name of stored)"] {
            assert!(source.contains(guard), "{guard}");
        }
        assert!(source.find("use framework").unwrap() < source.find("on trimmed").unwrap(), "use statements come first");
    }

    #[test]
    fn a_moved_event_must_read_back_exactly_as_approved() {
        let args = validate("update_calendar_event", &json!({"id":"e-1","occurrence":"2026-11-03 15:00","was_title":"Review","title":"Review","start":"2026-11-03 16:00","minutes":45,"location":"Room 4"})).unwrap();
        let before = json!({"title":"Review","start":"2026-11-03 15:00","end":"2026-11-03 15:45","startSecond":0,"durationSeconds":2700,"location":"Room 4","allDay":false,"calendarId":"cal-1","notes":"Agenda","url":"","alarms":1,"recurring":false});
        let mut after = before.clone();
        after["start"] = json!("2026-11-03 16:00");
        after["end"] = json!("2026-11-03 16:45");
        let valid = json!({"destinationId":"cal-1","identifierScope":"persistent","title":"Review","before":before,"after":after,"oldSlotVacated":true,"span":"this_event"});
        let receipt = change_receipt("update_calendar_event", &args, valid.clone());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        for (path, value) in [("/after/start", json!("2026-11-03 15:00")), ("/after/durationSeconds", json!(3600)), ("/after/location", json!("")),
            ("/after/notes", json!("")), ("/after/calendarId", json!("cal-2")), ("/before/title", json!("Standup")), ("/oldSlotVacated", json!(false)),
            ("/after/startSecond", json!(30))] {
            let mut wrong = valid.clone();
            *wrong.pointer_mut(path).unwrap() = value;
            assert_eq!(change_receipt("update_calendar_event", &args, wrong).outcome, ActionOutcome::Unknown, "{path}");
        }
        // A repeating event: only this occurrence, and the next one keeps its time.
        let mut repeating = valid;
        repeating["before"]["recurring"] = json!(true);
        repeating["after"]["recurring"] = json!(true);
        repeating["span"] = json!("this_occurrence");
        repeating["nextBefore"] = json!("2026-11-10 15:00|Review");
        repeating["nextAfter"] = json!("2026-11-10 15:00|Review");
        assert_eq!(change_receipt("update_calendar_event", &args, repeating.clone()).outcome, ActionOutcome::Succeeded);
        for (key, value) in [("nextAfter", json!("2026-11-10 16:00|Review")), ("span", json!("this_event"))] {
            let mut wrong = repeating.clone();
            wrong[key] = value;
            assert_eq!(change_receipt("update_calendar_event", &args, wrong).outcome, ActionOutcome::Unknown, "{key}");
        }
        repeating.as_object_mut().unwrap().remove("nextBefore");
        assert_eq!(change_receipt("update_calendar_event", &args, repeating).outcome, ActionOutcome::Unknown);
    }

    #[test]
    fn text_added_to_a_note_must_follow_everything_that_was_there() {
        let args = validate("append_to_note", &json!({"id":"n-1","title":"Shopping","text":"milk\neggs"})).unwrap();
        let valid = json!({"destinationId":"folder-1","identifierScope":"persistent","beforeTitle":"Shopping","beforePlaintext":"Shopping\nBread\n",
            "title":"Shopping","plaintext":"Shopping\nBread\nmilk\neggs\n","folderId":"folder-1","attachmentCount":0});
        let receipt = change_receipt("append_to_note", &args, valid.clone());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{}", receipt.message);
        for (key, value) in [("plaintext", json!("Shopping\nmilk\neggs")), ("plaintext", json!("Shopping\nBread\nmilk\neggs\nextra")), ("plaintext", json!("Shopping\nBreadmilk\neggs")),
            ("beforeTitle", json!("Shopping list")), ("folderId", json!("folder-2")), ("attachmentCount", json!(1))] {
            let mut wrong = valid.clone();
            wrong[key] = value;
            assert_eq!(change_receipt("append_to_note", &args, wrong).outcome, ActionOutcome::Unknown, "{key}");
        }
    }
}
