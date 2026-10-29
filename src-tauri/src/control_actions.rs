//! Model-proposed controls require an approved durable step. These receipts use
//! actual app state, not the human-readable success strings of direct skills.
use crate::{
    actions::{ActionOutcome, ActionReceipt, Field},
    controller::AppState,
    store::Store,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const TOOLS: [&str; 4] = ["control_music", "switch_tab", "remember", "set_timer"];

pub fn validate(tool: &str, args: &Value) -> Result<Map<String, Value>, String> {
    if matches!(tool, "control_music" | "switch_tab") {
        let out = crate::apps::validate(tool, args)?;
        if tool == "control_music"
            && out.get("action").and_then(Value::as_str) == Some("now_playing")
        {
            return Err("What's playing is a read, not an action to approve.".into());
        }
        if tool == "switch_tab" {
            let browser = out
                .get("browser")
                .and_then(Value::as_str)
                .ok_or("Choose the browser for this tab switch before approval.")?;
            if browser == "arc" {
                return Err(
                    "Verified tab switching in Arc is not available in a multi-step task yet."
                        .into(),
                );
            }
        }
        return Ok(out);
    }
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    let key = match tool {
        "remember" => "fact",
        "set_timer" => "seconds",
        _ => return Err("Unknown control action.".into()),
    };
    if args.len() != 1 || !args.contains_key(key) {
        return Err(format!("{tool} takes only {key}."));
    }
    if tool == "remember" {
        let fact = args["fact"].as_str().ok_or("The fact must be text.")?;
        if fact.chars().any(|c| c.is_control() && !c.is_whitespace()) {
            return Err("The fact contains unsupported control characters.".into());
        }
        let fact = fact.split_whitespace().collect::<Vec<_>>().join(" ");
        if !(3..=300).contains(&fact.chars().count()) {
            return Err("Memories are 3–300 characters.".into());
        }
        Ok(Map::from_iter([("fact".into(), json!(fact))]))
    } else {
        let seconds = args["seconds"]
            .as_u64()
            .or_else(|| {
                args["seconds"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
            })
            .filter(|seconds| (1..=86400).contains(seconds))
            .ok_or("Use a whole number of seconds between 1 and 86,400.")?;
        Ok(Map::from_iter([("seconds".into(), json!(seconds))]))
    }
}
pub fn describe(tool: &str, args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let field = |key: &str, label: &str| Field {
        key: key.into(),
        label: label.into(),
        kind: "text",
        value: args
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default(),
    };
    match tool {
        "remember" => ("Remember this?".into(), vec![field("fact", "Fact or preference")], "Saved on your Mac for future conversations. Remove it anytime in Settings → Privacy.".into()),
        "set_timer" => ("Set this timer?".into(), vec![field("seconds", "Duration in seconds")], "The timer is saved across restarts. Alerts require Buntini running and your Mac awake; overdue timers appear when it returns. Desktop alerts need notification permission.".into()),
        "control_music" => ("Change music playback?".into(), vec![field("app", "Player (music or spotify)"), field("action", "Action (play, pause, next or previous)"), field("query", "Song or playlist (optional for play)")], "Changes playback in the selected app and reads its resulting state. The player must be allowed in Settings → Apps. Named Spotify search is not supported by this approved workflow.".into()),
        _ => ("Switch to this browser tab?".into(), vec![field("browser", "Browser"), field("query", "Words in the tab title or address")], "Switches only if one tab matches in the selected browser, then verifies the front window and selected tab. Private Chromium windows are skipped. Safari does not identify private windows.".into()),
    }
}
fn failure(
    tool: &str,
    outcome: ActionOutcome,
    detail: impl Into<String>,
    evidence: Value,
) -> ActionReceipt {
    let detail = detail.into();
    let message = if outcome == ActionOutcome::Unknown {
        format!("I could not verify the result. {detail} Check the app before repeating this action; it will not be retried automatically.")
    } else {
        detail.clone()
    };
    ActionReceipt {
        outcome,
        tool: tool.into(),
        object_id: None,
        destination: None,
        message,
        evidence,
        error: Some(detail),
    }
}
fn success(
    tool: &str,
    id: String,
    destination: &str,
    message: String,
    evidence: Value,
) -> ActionReceipt {
    ActionReceipt {
        outcome: ActionOutcome::Succeeded,
        tool: tool.into(),
        object_id: Some(id),
        destination: Some(destination.into()),
        message,
        evidence,
        error: None,
    }
}
async fn cancellable<T>(
    cancel: &Arc<AtomicBool>,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled before the result could be confirmed.".into());
        }
        tokio::select! { result = &mut future => return result, _ = tokio::time::sleep(std::time::Duration::from_millis(40)) => {} }
    }
}

fn local_execute(
    store: &Store,
    intent_id: &str,
    tool: &str,
    args: &Map<String, Value>,
    cancel: &AtomicBool,
) -> ActionReceipt {
    if intent_id.is_empty()
        || intent_id.len() > 200
        || !intent_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return failure(
            tool,
            ActionOutcome::Failed,
            "The control has no valid durable intent identifier.",
            json!({}),
        );
    }
    let transaction = match store.db.unchecked_transaction() {
        Ok(tx) => tx,
        Err(error) => return failure(tool, ActionOutcome::Failed, error.to_string(), json!({})),
    };
    let result = (|| -> Result<ActionReceipt, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled before saving.".into());
        }
        store
            .db
            .execute("UPDATE control_intents SET tool=tool WHERE 0", [])
            .map_err(|error| error.to_string())?;
        let previous: Option<(String, String, String)> = store
            .db
            .query_row(
                "SELECT tool,args,object_id FROM control_intents WHERE id=?1",
                [intent_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let serialized = serde_json::to_string(args).map_err(|error| error.to_string())?;
        if previous
            .as_ref()
            .is_some_and(|(old_tool, old_args, _)| old_tool != tool || old_args != &serialized)
        {
            return Err(
                "This approval was already used with different details. Start a new request."
                    .into(),
            );
        }
        let receipt = if tool == "remember" {
            let fact = args["fact"].as_str().unwrap();
            let previous_id = previous
                .as_ref()
                .map(|(_, _, id)| {
                    id.parse::<i64>()
                        .map_err(|_| "Invalid saved memory receipt.".to_string())
                })
                .transpose()?;
            let mut found = store.memories()?.into_iter().find(|memory| {
                previous_id
                    .map(|id| memory.id == id)
                    .unwrap_or(memory.fact == fact)
            });
            if previous.is_some() && found.is_none() {
                return Err("This memory was already saved and later forgotten. The old approval cannot recreate it.".into());
            }
            if found.is_none() {
                store.remember(fact)?;
                found = store
                    .memories()?
                    .into_iter()
                    .find(|memory| memory.fact == fact);
            }
            let memory = found.ok_or("The saved memory could not be read back.")?;
            if memory.fact != fact {
                return Err("The saved memory does not match the approved fact.".into());
            }
            success(
                tool,
                memory.id.to_string(),
                "Buntini memory",
                "Saved and verified that memory.".into(),
                json!({"verified":true,"fact":memory.fact,"createdAt":memory.created_at}),
            )
        } else {
            let seconds = args["seconds"].as_u64().unwrap();
            let timer = if let Some((_, _, id)) = &previous {
                crate::delivery::timer(store, id)?.ok_or("This timer was already created and later removed. The old approval cannot recreate it.")?
            } else {
                crate::delivery::schedule(store, seconds, chrono::Utc::now())?
            };
            if timer.seconds != seconds {
                return Err("The saved timer does not match the approved duration.".into());
            }
            if previous.is_some() && timer.state != "pending" {
                return Err(format!(
                    "This timer already has status {}. The old approval cannot start it again.",
                    timer.state
                ));
            }
            success(tool, timer.id.clone(), "Buntini · History → Scheduled work", format!("Saved and verified a {} timer, due at {}. Alerts require Buntini running and your Mac awake.", crate::skills::mac::spoken(seconds), timer.due_at), json!({"verified":true,"timer":timer}))
        };
        store
            .db
            .execute(
                "INSERT OR IGNORE INTO control_intents(id,tool,args,object_id) VALUES(?1,?2,?3,?4)",
                params![intent_id, tool, serialized, receipt.object_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(receipt)
    })();
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => return failure(tool, ActionOutcome::Failed, error, json!({})),
    };
    match transaction.commit() {
        Ok(()) => receipt,
        Err(error) => failure(
            tool,
            ActionOutcome::Unknown,
            format!("The local write could not be committed: {error}"),
            json!({}),
        ),
    }
}

const JSON_HELPER: &str = r#"use framework "Foundation"
use scripting additions
on putField(fields, fieldName, fieldValue)
  if fieldValue is missing value then set fieldValue to ""
  fields's setObject:fieldValue forKey:fieldName
end putField
on encode(fields)
  set encoded to current application's NSJSONSerialization's dataWithJSONObject:fields options:0 |error|:(missing value)
  if encoded is missing value then error "Could not encode observed app state."
  return (current application's NSString's alloc()'s initWithData:encoded encoding:(current application's NSUTF8StringEncoding)) as text
end encode
"#;
const MUSIC_STATE: &str = r#"on run argv
  set fields to current application's NSMutableDictionary's dictionary()
  my putField(fields, "state", "closed")
  my putField(fields, "trackId", "")
  my putField(fields, "title", "")
  my putField(fields, "artist", "")
  my putField(fields, "playlist", "")
  my putField(fields, "position", 0)
  if application "APP" is not running then return my encode(fields)
  tell application "APP"
    set currentState to player state as text
    my putField(fields, "state", currentState)
    if currentState is not "stopped" then
      my putField(fields, "trackId", TRACK_ID of current track as text)
      my putField(fields, "title", name of current track as text)
      my putField(fields, "artist", artist of current track as text)
      my putField(fields, "position", player position)
      PLAYLIST_READ
    end if
  end tell
  return my encode(fields)
end run
"#;
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MusicState {
    state: String,
    track_id: String,
    title: String,
    artist: String,
    playlist: String,
    position: f64,
}
fn parse_music(raw: &str) -> Result<MusicState, String> {
    let state: MusicState = serde_json::from_str(raw)
        .map_err(|_| "The player returned incomplete state.".to_string())?;
    if !["closed", "stopped", "paused", "playing"].contains(&state.state.as_str())
        || !state.position.is_finite()
        || state.position < 0.0
        || (["paused", "playing"].contains(&state.state.as_str())
            && (state.track_id.is_empty() || state.title.is_empty()))
    {
        return Err("The player returned an invalid state.".into());
    }
    Ok(state)
}
fn normalized(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}
fn music_verified(action: &str, query: &str, before: &MusicState, after: &MusicState) -> bool {
    match action {
        "pause" => ["paused", "stopped", "closed"].contains(&after.state.as_str()),
        "play" if after.state == "playing" => {
            query.is_empty()
                || [
                    after.title.as_str(),
                    after.playlist.as_str(),
                    format!("{} by {}", after.title, after.artist).as_str(),
                ]
                .iter()
                .any(|candidate| {
                    !candidate.is_empty() && normalized(candidate) == normalized(query)
                })
        }
        "next" | "previous" => {
            after.state == "playing"
                && !before.track_id.is_empty()
                && after.track_id != before.track_id
        }
        _ => false,
    }
}
async fn script(source: &str, args: &[String], cancel: &Arc<AtomicBool>) -> Result<String, String> {
    let mut command =
        crate::actions::osascript_command("AppleScript", &format!("{JSON_HELPER}{source}"));
    command.args(args);
    cancellable(cancel, crate::actions::output(command, 20)).await
}
async fn music_state(app: &str, cancel: &Arc<AtomicBool>) -> Result<MusicState, String> {
    let source = MUSIC_STATE
        .replace("APP", if app == "spotify" { "Spotify" } else { "Music" })
        .replace(
            "TRACK_ID",
            if app == "spotify" {
                "id"
            } else {
                "persistent ID"
            },
        )
        .replace(
            "PLAYLIST_READ",
            if app == "music" {
                "my putField(fields, \"playlist\", name of current playlist as text)"
            } else {
                ""
            },
        );
    parse_music(&script(&source, &[], cancel).await?)
}

// One unique target is resolved before mutation. The script records both that
// target and independently read current front-window/tab properties afterward.
const SWITCH: &str = r#"on run argv
  set q to item 1 of argv
  set fields to current application's NSMutableDictionary's dictionary()
  set mutationStarted to false
  try
    if application "APP" is not running then error "The browser is not open."
    tell application "APP"
      set matchCount to 0
      set chosenWindow to missing value
      set chosenTab to missing value
      repeat with w in windows
        if PUBLIC_WINDOW then
          set ti to 0
          repeat with t in tabs of w
            set ti to ti + 1
            if (TAB_TITLE of t contains q) or (URL of t contains q) then
              set matchCount to matchCount + 1
              set chosenWindow to contents of w
              set chosenTab to contents of t
              set chosenIndex to ti
            end if
          end repeat
        end if
      end repeat
      if matchCount is 0 then error "No open tab matches that request."
      if matchCount is not 1 then error "Several tabs match. Use more specific words from the title or address."
      set expectedWindow to id of chosenWindow as text
      set expectedTab to TARGET_TAB_ID
      set expectedTitle to TAB_TITLE of chosenTab as text
      set expectedUrl to URL of chosenTab as text
      my putField(fields, "expectedWindowId", expectedWindow)
      my putField(fields, "expectedTabId", expectedTab)
      my putField(fields, "expectedTitle", expectedTitle)
      my putField(fields, "expectedUrl", expectedUrl)
      set mutationStarted to true
      SELECT_TAB
      set index of chosenWindow to 1
      activate
      delay 0.1
      set observedWindow to front window
      set observedTab to CURRENT_TAB of observedWindow
      my putField(fields, "windowId", id of observedWindow as text)
      my putField(fields, "tabId", OBSERVED_TAB_ID)
      my putField(fields, "title", TAB_TITLE of observedTab as text)
      my putField(fields, "url", URL of observedTab as text)
      my putField(fields, "frontmost", frontmost)
      my putField(fields, "publicWindow", OBSERVED_PUBLIC)
    end tell
    my putField(fields, "error", "")
  on error failureText number failureNumber
    my putField(fields, "error", failureText & " (" & failureNumber & ")")
  end try
  my putField(fields, "mutationStarted", mutationStarted)
  return my encode(fields)
end run
"#;
fn switch_script(browser: &str) -> Result<String, String> {
    let app = crate::apps::app(browser).ok_or("Unknown browser.")?;
    if browser == "arc" {
        return Err("Verified tab switching in Arc is not available yet.".into());
    }
    let safari = browser == "safari";
    Ok(SWITCH
        .replace("APP", app.name)
        .replace(
            "PUBLIC_WINDOW",
            if safari {
                "true"
            } else {
                "mode of w is not \"incognito\""
            },
        )
        .replace("TAB_TITLE", if safari { "name" } else { "title" })
        .replace(
            "TARGET_TAB_ID",
            if safari {
                "chosenIndex as text"
            } else {
                "id of chosenTab as text"
            },
        )
        .replace(
            "SELECT_TAB",
            if safari {
                "set current tab of chosenWindow to chosenTab"
            } else {
                "set active tab index of chosenWindow to chosenIndex"
            },
        )
        .replace(
            "CURRENT_TAB",
            if safari { "current tab" } else { "active tab" },
        )
        .replace(
            "OBSERVED_TAB_ID",
            if safari {
                "index of observedTab as text"
            } else {
                "id of observedTab as text"
            },
        )
        .replace(
            "OBSERVED_PUBLIC",
            if safari {
                "true"
            } else {
                "mode of observedWindow is not \"incognito\""
            },
        ))
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TabState {
    mutation_started: bool,
    error: String,
    expected_window_id: Option<String>,
    expected_tab_id: Option<String>,
    expected_title: Option<String>,
    expected_url: Option<String>,
    window_id: Option<String>,
    tab_id: Option<String>,
    title: Option<String>,
    url: Option<String>,
    frontmost: Option<bool>,
    public_window: Option<bool>,
}
fn tab_receipt(browser: &str, query: &str, raw: &str) -> ActionReceipt {
    let state: TabState = match serde_json::from_str(raw) {
        Ok(state) => state,
        Err(_) => {
            return failure(
                "switch_tab",
                ActionOutcome::Unknown,
                "The browser returned no valid state receipt.",
                json!({}),
            )
        }
    };
    let evidence: Value = serde_json::from_str(raw).unwrap_or(json!({}));
    if !state.error.is_empty() {
        return failure(
            "switch_tab",
            if state.mutation_started {
                ActionOutcome::Unknown
            } else {
                ActionOutcome::Failed
            },
            state.error,
            evidence,
        );
    }
    let target_matches = state
        .expected_title
        .as_ref()
        .is_some_and(|title| title.to_lowercase().contains(&query.to_lowercase()))
        || state
            .expected_url
            .as_ref()
            .is_some_and(|url| url.to_lowercase().contains(&query.to_lowercase()));
    let verified = state.mutation_started
        && state.frontmost == Some(true)
        && state.public_window == Some(true)
        && target_matches
        && state
            .expected_window_id
            .as_ref()
            .is_some_and(|id| !id.is_empty())
        && state.window_id == state.expected_window_id
        && state
            .expected_tab_id
            .as_ref()
            .is_some_and(|id| !id.is_empty())
        && state.tab_id == state.expected_tab_id
        && state
            .expected_url
            .as_ref()
            .is_some_and(|url| !url.is_empty())
        && state.url == state.expected_url
        && state.title == state.expected_title;
    if !verified {
        return failure(
            "switch_tab",
            ActionOutcome::Unknown,
            "The selected front window and tab did not match the approved target.",
            evidence,
        );
    }
    success(
        "switch_tab",
        format!(
            "{browser}:{}:{}",
            state.window_id.unwrap(),
            state.tab_id.unwrap()
        ),
        crate::apps::app(browser).unwrap().name,
        format!(
            "Switched to and verified “{}”.",
            state.title.unwrap_or_default()
        ),
        json!({"verified":true,"identifierScope":"browser_session","observed":evidence}),
    )
}

pub async fn execute(
    state: &AppState,
    intent_id: &str,
    tool: &str,
    raw: &Map<String, Value>,
    cancel: Arc<AtomicBool>,
) -> ActionReceipt {
    let args = match validate(tool, &Value::Object(raw.clone())) {
        Ok(args) => args,
        Err(error) => return failure(tool, ActionOutcome::Failed, error, json!({})),
    };
    if cancel.load(Ordering::SeqCst) {
        return failure(
            tool,
            ActionOutcome::Failed,
            "Cancelled before the action started.",
            json!({}),
        );
    }
    if matches!(tool, "remember" | "set_timer") {
        return local_execute(
            &state.store.lock().unwrap(),
            intent_id,
            tool,
            &args,
            &cancel,
        );
    }
    let settings = match state.store.lock().unwrap().settings() {
        Ok(settings) => settings,
        Err(error) => return failure(tool, ActionOutcome::Failed, error, json!({})),
    };
    if !crate::apps::permits(&settings, tool, &args) {
        return failure(
            tool,
            ActionOutcome::Failed,
            "Allow the selected app in Settings → Apps before using this control.",
            json!({}),
        );
    }
    if tool == "switch_tab" {
        let browser = args["browser"].as_str().unwrap();
        let query = args["query"].as_str().unwrap();
        let source = match switch_script(browser) {
            Ok(source) => source,
            Err(error) => return failure(tool, ActionOutcome::Failed, error, json!({})),
        };
        return match script(&source, &[query.into()], &cancel).await {
            Ok(raw) => tab_receipt(browser, query, &raw),
            Err(error) => failure(tool, ActionOutcome::Unknown, error, json!({})),
        };
    }
    let app = args["app"].as_str().unwrap();
    let action = args["action"].as_str().unwrap();
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    if app == "spotify" && !query.is_empty() {
        return failure(tool,ActionOutcome::Failed,"Named Spotify search is not supported by this approved workflow. Spotify resume, pause and track skipping are available.",json!({}));
    }
    let before = match music_state(app, &cancel).await {
        Ok(state) => state,
        Err(error) => return failure(tool, ActionOutcome::Failed, error, json!({})),
    };
    if cancel.load(Ordering::SeqCst) {
        return failure(
            tool,
            ActionOutcome::Failed,
            "Cancelled before changing playback.",
            json!({"before":before}),
        );
    }
    if before.state == "closed" && action != "play" {
        return if action == "pause" {
            success(
                tool,
                format!("{app}:player"),
                app,
                "The player is closed; nothing is playing.".into(),
                json!({"verified":true,"before":before,"after":before}),
            )
        } else {
            failure(
                tool,
                ActionOutcome::Failed,
                "Open the player and start a track before skipping.",
                json!({"before":before}),
            )
        };
    }
    if let Err(error) = cancellable(
        &cancel,
        crate::apps::control_music(app, action, query, false),
    )
    .await
    {
        return failure(
            tool,
            ActionOutcome::Unknown,
            error,
            json!({"before":before}),
        );
    }
    let after = match music_state(app, &cancel).await {
        Ok(state) => state,
        Err(error) => {
            return failure(
                tool,
                ActionOutcome::Unknown,
                error,
                json!({"before":before}),
            )
        }
    };
    if !music_verified(action, query, &before, &after) {
        return failure(
            tool,
            ActionOutcome::Unknown,
            "The player's observed state did not confirm the requested playback change.",
            json!({"before":before,"after":after}),
        );
    }
    let id = if after.track_id.is_empty() {
        format!("{app}:player")
    } else {
        after.track_id.clone()
    };
    success(
        tool,
        id,
        app,
        format!(
            "Verified {app} is {}{}.",
            after.state,
            if after.title.is_empty() {
                String::new()
            } else {
                format!(" on “{}” by {}", after.title, after.artist)
            }
        ),
        json!({"verified":true,"before":before,"after":after}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }
    fn music(state: &str, id: &str, title: &str) -> MusicState {
        MusicState {
            state: state.into(),
            track_id: id.into(),
            title: title.into(),
            artist: "Artist".into(),
            playlist: "Playlist".into(),
            position: 15.0,
        }
    }
    #[test]
    fn model_controls_validate_without_silent_scope_changes() {
        assert!(validate("control_music", &json!({"action":"now_playing"})).is_err());
        assert!(validate("switch_tab", &json!({"query":"mail"})).is_err());
        assert!(validate("switch_tab", &json!({"browser":"arc","query":"mail"})).is_err());
        for value in [json!(0), json!(-1), json!(1.5), json!(86401), json!(true)] {
            assert!(validate("set_timer", &json!({"seconds":value})).is_err());
        }
        assert_eq!(
            validate("set_timer", &json!({"seconds":"60"})).unwrap()["seconds"],
            60
        );
        assert!(validate("remember", &json!({"fact":"A preference","extra":"hidden"})).is_err());
        assert_eq!(
            validate("remember", &json!({"fact":"  Likes   tea "})).unwrap()["fact"],
            "Likes tea"
        );
    }
    #[test]
    fn playback_receipts_require_observed_state_and_track_changes() {
        let before = music("playing", "a", "First");
        assert!(!music_verified("pause", "", &before, &before));
        assert!(music_verified(
            "pause",
            "",
            &before,
            &music("paused", "a", "First")
        ));
        assert!(!music_verified("next", "", &before, &before));
        assert!(music_verified(
            "next",
            "",
            &before,
            &music("playing", "b", "Second")
        ));
        assert!(!music_verified("play", "Wrong title", &before, &before));
        assert!(music_verified("play", "First by Artist", &before, &before));
        assert!(music_verified("play", "Playlist", &before, &before));
        assert!(parse_music(r#"{"state":"playing","trackId":"","title":"First","artist":"Artist","playlist":"","position":1}"#).is_err());
        assert!(parse_music(r#"{"state":"playing","trackId":"a"}"#).is_err());
    }
    #[test]
    fn tab_receipts_verify_the_actual_front_window_and_exact_tab() {
        let raw = json!({"mutationStarted":true,"error":"","expectedWindowId":"10","expectedTabId":"2","expectedTitle":"Inbox","expectedUrl":"https://mail.example.com/","windowId":"10","tabId":"2","title":"Inbox","url":"https://mail.example.com/","frontmost":true,"publicWindow":true});
        assert_eq!(
            tab_receipt("chrome", "Inbox", &raw.to_string()).outcome,
            ActionOutcome::Succeeded
        );
        for key in ["windowId", "tabId", "url", "title"] {
            let mut wrong = raw.clone();
            wrong[key] = json!("wrong");
            assert_eq!(
                tab_receipt("chrome", "Inbox", &wrong.to_string()).outcome,
                ActionOutcome::Unknown
            );
        }
        let mut hidden = raw.clone();
        hidden["frontmost"] = json!(false);
        assert_eq!(
            tab_receipt("chrome", "Inbox", &hidden.to_string()).outcome,
            ActionOutcome::Unknown
        );
        assert_eq!(
            tab_receipt("chrome", "different", &raw.to_string()).outcome,
            ActionOutcome::Unknown
        );
        assert_eq!(
            tab_receipt(
                "chrome",
                "Inbox",
                r#"{"mutationStarted":false,"error":"Several tabs match."}"#
            )
            .outcome,
            ActionOutcome::Failed
        );
        assert_eq!(
            tab_receipt("chrome", "Inbox", "Switched!").outcome,
            ActionOutcome::Unknown
        );
    }
    #[test]
    fn local_controls_are_idempotent_and_do_not_recreate_deleted_objects() {
        let store = store();
        let cancel = AtomicBool::new(false);
        let args = validate("remember", &json!({"fact":"Likes green tea"})).unwrap();
        let first = local_execute(&store, "memory-intent", "remember", &args, &cancel);
        assert_eq!(first.outcome, ActionOutcome::Succeeded);
        let second = local_execute(&store, "memory-intent", "remember", &args, &cancel);
        assert_eq!(first.object_id, second.object_id);
        assert_eq!(store.memories().unwrap().len(), 1);
        store
            .forget(first.object_id.unwrap().parse().unwrap())
            .unwrap();
        assert_eq!(
            local_execute(&store, "memory-intent", "remember", &args, &cancel).outcome,
            ActionOutcome::Failed
        );
        let args = validate("set_timer", &json!({"seconds":60})).unwrap();
        let first = local_execute(&store, "timer-intent", "set_timer", &args, &cancel);
        assert_eq!(first.outcome, ActionOutcome::Succeeded);
        let second = local_execute(&store, "timer-intent", "set_timer", &args, &cancel);
        assert_eq!(first.object_id, second.object_id);
        assert_eq!(first.evidence, second.evidence);
        assert_eq!(crate::delivery::timers(&store).unwrap().len(), 1);
        crate::delivery::cancel(&store, first.object_id.as_deref()).unwrap();
        assert_eq!(
            local_execute(&store, "timer-intent", "set_timer", &args, &cancel).outcome,
            ActionOutcome::Failed
        );
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(
            local_execute(&store, "cancelled", "set_timer", &args, &cancel).outcome,
            ActionOutcome::Failed
        );
        assert_eq!(crate::delivery::timers(&store).unwrap().len(), 1);
    }
    #[test]
    fn native_verifiers_keep_untrusted_values_as_arguments() {
        for browser in ["safari", "chrome", "brave", "edge"] {
            let script = switch_script(browser).unwrap();
            assert!(script.contains("item 1 of argv"));
            assert!(script.contains("set mutationStarted to true"));
            assert!(script.contains("frontmost"));
            assert!(!script.contains("do shell script"));
        }
    }
    #[test]
    fn local_receipts_and_tombstones_roll_back_together() {
        let store = store();
        let cancel = AtomicBool::new(false);
        store.db.execute_batch("CREATE TRIGGER reject_control_intent BEFORE INSERT ON control_intents BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        let memory = validate("remember", &json!({"fact":"Prefers tea"})).unwrap();
        assert_eq!(
            local_execute(&store, "failed-memory", "remember", &memory, &cancel).outcome,
            ActionOutcome::Failed
        );
        assert!(store.memories().unwrap().is_empty());
        let timer = validate("set_timer", &json!({"seconds":60})).unwrap();
        assert_eq!(
            local_execute(&store, "failed-timer", "set_timer", &timer, &cancel).outcome,
            ActionOutcome::Failed
        );
        assert!(crate::delivery::timers(&store).unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancellation_never_starts_a_queued_control() {
        let cancel = Arc::new(AtomicBool::new(true));
        let polled = AtomicBool::new(false);
        let result = cancellable(&cancel, async {
            polled.store(true, Ordering::SeqCst);
            Ok::<_, String>(())
        })
        .await;
        assert!(result.is_err());
        assert!(!polled.load(Ordering::SeqCst));
        cancel.store(false, Ordering::SeqCst);
        let token = cancel.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            token.store(true, Ordering::SeqCst);
        });
        let result = cancellable(&cancel, std::future::pending::<Result<(), String>>()).await;
        assert!(result.is_err());
        cancel_task.await.unwrap();
    }
    #[test]
    fn control_intents_are_exported_and_erased_with_their_local_objects() {
        let store = store();
        let cancel = AtomicBool::new(false);
        let args = validate("remember", &json!({"fact":"Prefers green tea"})).unwrap();
        let receipt = local_execute(&store, "export-memory", "remember", &args, &cancel);
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded);
        let export = store.export_snapshot().unwrap();
        assert_eq!(
            export["controlIntents"][0]["arguments"]["fact"],
            "Prefers green tea"
        );
        assert_eq!(
            export["controlIntents"][0]["objectId"],
            receipt.object_id.unwrap()
        );
        store.erase_everything().unwrap();
        assert!(store.export_snapshot().unwrap()["controlIntents"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(store.memories().unwrap().is_empty());
    }
}
