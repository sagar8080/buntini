//! Debug-only, opt-in real-app acceptance helpers. No synthetic success receipts.
//! Every write goes through the production validator/executor; cleanup requires
//! the exact returned object ID and an unchanged, namespaced record payload.
use crate::actions::{self, ActionOutcome, ActionReceipt};
use serde_json::{json, Map, Value};

fn scoped(namespace: &str, tool: &str, raw: &Value) -> Result<Map<String, Value>, String> {
    if !namespace.starts_with("BuntiniQA-")
        || !(25..=75).contains(&namespace.len())
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("Acceptance requires a unique BuntiniQA-<uuid> namespace.".into());
    }
    if !["create_note", "add_reminder", "add_calendar_event"].contains(&tool) {
        return Err(
            "Real-app acceptance permits only Notes, Reminders and Calendar test records.".into(),
        );
    }
    let args = actions::validate(tool, raw)?;
    let title = args["title"]
        .as_str()
        .ok_or("The test record needs a title.")?;
    if !title
        .strip_prefix(namespace)
        .is_some_and(|rest| rest.starts_with([' ', ':']))
    {
        return Err("The exact title must begin with the unique acceptance namespace.".into());
    }
    Ok(args)
}

fn rejected(tool: &str, error: String) -> Value {
    json!({"outcome":"failed","tool":tool,"objectId":null,"destination":null,
        "message":error,"error":error,"evidence":{"acceptanceGuard":true,"executorStarted":false}})
}

/// The aggregate harness must review model output against `expected` before it
/// calls this boundary. This function never fills missing fields or fakes an app.
pub async fn execute_exact(
    namespace: &str,
    tool: &str,
    proposed: &Value,
    expected: &Value,
) -> Value {
    let checked = (|| {
        let approved = scoped(namespace, tool, expected)?;
        let proposed = scoped(namespace, tool, proposed)?;
        if approved != proposed {
            return Err(
                "The generated app action differs from the exact approved test fields.".into(),
            );
        }
        for key in ["due", "start"] {
            if let Some(value) = approved.get(key).and_then(Value::as_str) {
                let date = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M")
                    .map_err(|_| "Invalid test date.")?;
                if date <= chrono::Local::now().naive_local() + chrono::Duration::minutes(30) {
                    return Err("Acceptance dates must be at least 30 minutes in the future to avoid test alerts.".into());
                }
            }
        }
        Ok(approved)
    })();
    let args = match checked {
        Ok(args) => args,
        Err(error) => return rejected(tool, error),
    };
    let receipt = actions::execute(tool, &args).await;
    let mut value = serde_json::to_value(receipt).expect("Action receipts are JSON serializable");
    value["acceptance"] = json!({"source":"production_actions_execute","namespace":namespace,"synthetic":false,"approvedArguments":args});
    value
}

/// Reads only a nonce-bearing test query through the real production Notes
/// integration. Useful for independently verifying retrieval fixes before AI.
pub async fn read_notes_exact(namespace: &str, query: &str) -> Value {
    if !query.to_lowercase().contains(&namespace.to_lowercase())
        || scoped(
            namespace,
            "create_note",
            &json!({"title":format!("{namespace} Scope guard"),"body":"guard"}),
        )
        .is_err()
    {
        return json!({"error":"The Notes read does not contain a valid acceptance namespace.","executorStarted":false});
    }
    let args = match crate::apps::validate("find_notes", &json!({"query":query})) {
        Ok(args) => args,
        Err(error) => return json!({"error":error,"executorStarted":false}),
    };
    let settings = crate::types::Settings {
        apps: vec!["notes".into()],
        ..Default::default()
    };
    match crate::apps::run_evidence("find_notes", &args, std::path::Path::new(""), &settings).await
    {
        Ok(value) => value,
        Err(error) => json!({"error":error}),
    }
}

const DATE_TEXT: &str = r#"on dateText(t)
  if t is missing value then return ""
  set m to text -2 thru -1 of ("0" & ((month of t) as integer))
  set d to text -2 thru -1 of ("0" & (day of t))
  set h to text -2 thru -1 of ("0" & (hours of t))
  set mi to text -2 thru -1 of ("0" & (minutes of t))
  return ((year of t) as text) & "-" & m & "-" & d & " " & h & ":" & mi
end dateText
"#;
const CLEAN_NOTE: &str = r#"on run argv
  tell application "Notes"
    set found to every note whose id is item 1 of argv
    if (count of found) is 0 then return "already_absent"
    if (count of found) is not 1 then error "Test note ID is not unique."
    set targetNote to item 1 of found
    if name of targetNote is not item 2 of argv then error "Test note title changed; cleanup stopped."
    if plaintext of targetNote is not item 3 of argv then error "Test note body changed; cleanup stopped."
    set originalFolder to folder id (item 4 of argv)
    set currentFolder to container of targetNote
    -- Never delete again after Notes moved the record to Recently Deleted.
    if (id of currentFolder) is not item 4 of argv then return "already_removed_from_original_folder"
    delete targetNote
    repeat 20 times
      if (count of (every note of originalFolder whose id is item 1 of argv)) is 0 then return "removed_from_original_folder"
      delay 0.1
    end repeat
    error "Test note is still in its original folder after deletion."
  end tell
end run
"#;
const CLEAN_REMINDER: &str = r#"on run argv
  tell application "Reminders"
    set containers to every list whose id is item 2 of argv
    if (count of containers) is not 1 then error "Exact test reminder list is unavailable."
    set targetList to item 1 of containers
    set found to every reminder of targetList whose id is item 1 of argv
    if (count of found) is 0 then return "already_absent"
    if (count of found) is not 1 then error "Test reminder ID is not unique."
    set targetReminder to item 1 of found
    if name of targetReminder is not item 3 of argv then error "Test reminder title changed; cleanup stopped."
    if my dateText(due date of targetReminder) is not item 4 of argv then error "Test reminder due date changed; cleanup stopped."
    if my dateText(remind me date of targetReminder) is not item 5 of argv then error "Test reminder alert date changed; cleanup stopped."
    if completed of targetReminder then error "Test reminder was completed; cleanup stopped."
    delete targetReminder
    if (count of (every reminder of targetList whose id is item 1 of argv)) is not 0 then error "Test reminder remains after deletion."
    return "removed"
  end tell
end run
"#;
const CLEAN_EVENT: &str = r#"on run argv
  tell application "Calendar"
    if not (exists calendar id (item 2 of argv)) then error "Exact test calendar is unavailable."
    set targetCalendar to calendar id (item 2 of argv)
    set found to every event of targetCalendar whose uid is item 1 of argv
    if (count of found) is 0 then return "already_absent"
    if (count of found) is not 1 then error "Test event ID is not unique."
    set targetEvent to item 1 of found
    if summary of targetEvent is not item 3 of argv then error "Test event title changed; cleanup stopped."
    if my dateText(start date of targetEvent) is not item 4 of argv then error "Test event start changed; cleanup stopped."
    if my dateText(end date of targetEvent) is not item 5 of argv then error "Test event end changed; cleanup stopped."
    if location of targetEvent is not item 6 of argv then error "Test event location changed; cleanup stopped."
    delete targetEvent
    if (count of (every event of targetCalendar whose uid is item 1 of argv)) is not 0 then error "Test event remains after deletion."
    return "removed"
  end tell
end run
"#;

pub async fn cleanup_receipt(namespace: &str, raw: &Value, expected: &Value) -> Value {
    let prepared = (|| -> Result<_, String> {
        let receipt: ActionReceipt =
            serde_json::from_value(raw.clone()).map_err(|_| "Invalid real-app receipt.")?;
        let args = scoped(namespace, &receipt.tool, expected)?;
        if !matches!(
            receipt.outcome,
            ActionOutcome::Succeeded | ActionOutcome::Unknown
        ) {
            return Err("No confirmed write boundary to clean up.".into());
        }
        let id = receipt
            .object_id
            .as_ref()
            .filter(|id| !id.is_empty() && id.len() <= 2048 && !id.chars().any(char::is_control))
            .ok_or("No exact object ID was captured; manual reconciliation is required.")?
            .clone();
        let field = |key: &str| {
            receipt.evidence[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("Missing real-app readback field {key}; cleanup stopped."))
        };
        let title = args["title"].as_str().unwrap().to_string();
        let (body, argv) = match receipt.tool.as_str() {
            "create_note" => (
                CLEAN_NOTE,
                vec![
                    id.clone(),
                    title,
                    field("plaintext")?,
                    field("destinationId")?,
                ],
            ),
            "add_reminder" => (
                CLEAN_REMINDER,
                vec![
                    id.clone(),
                    field("destinationId")?,
                    title,
                    field("due")?,
                    field("remindAt")?,
                ],
            ),
            "add_calendar_event" => (
                CLEAN_EVENT,
                vec![
                    id.clone(),
                    field("destinationId")?,
                    title,
                    field("start")?,
                    field("end")?,
                    field("location")?,
                ],
            ),
            _ => return Err("No cleanup executor for this tool.".into()),
        };
        Ok((receipt.tool, id, body, argv))
    })();
    let (tool, id, body, argv) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            return json!({"passed":false,"manualReconciliationRequired":true,"error":error})
        }
    };
    let mut command = actions::osascript_command("AppleScript", &format!("{DATE_TEXT}{body}"));
    command.args(argv);
    match actions::output(command, 30).await {
        Ok(status)
            if [
                "removed",
                "already_absent",
                "removed_from_original_folder",
                "already_removed_from_original_folder",
            ]
            .contains(&status.trim()) =>
        {
            json!({"passed":true,"tool":tool,"objectId":id,"status":status.trim(),"exactIdAndUnchangedPayloadVerified":true,"notesDeletion":"Notes may retain its own Recently Deleted copy; no trash purge is performed."})
        }
        Ok(status) => {
            json!({"passed":false,"tool":tool,"objectId":id,"manualReconciliationRequired":true,"error":format!("Unexpected cleanup response: {status}")})
        }
        Err(error) => {
            json!({"passed":false,"tool":tool,"objectId":id,"manualReconciliationRequired":true,"error":error})
        }
    }
}

/// Real notes are seeded through execute_exact, then read by production
/// find_notes with a unique query. These are source fixtures, never fake receipts.
pub fn corpus(namespace: &str) -> Value {
    let title = |suffix: &str| format!("{namespace} {suffix}");
    json!([
        {"id":"notes-grocery-summary","seeds":[{"tool":"create_note","arguments":{"title":title("Groceries"),"body":"Dinner for two: chickpeas, spinach, tomatoes. Pantry already has rice. Budget: $25. Buy only the missing ingredients."}}],"readQuery":title("Groceries"),"prompt":format!("Find my note mentioning {namespace} Groceries and tell me what I still need to buy and my budget."),"answerMustContain":["chickpeas","spinach","tomatoes","25"]},
        {"id":"notes-travel-checklist","seeds":[{"tool":"create_note","arguments":{"title":title("Travel"),"body":"Train leaves at 08:10. Arrive at the station by 07:40. Bring passport, charger and the printed ticket. Hotel check-in starts at 15:00."}}],"readQuery":title("Travel"),"prompt":format!("Read my note mentioning {namespace} Travel. What time should I reach the station, and what three things must I bring?"),"answerMustContain":["07:40","passport","charger","ticket"]},
        {"id":"notes-compare-quotes","seeds":[{"tool":"create_note","arguments":{"title":title("Quotes"),"body":"Bike repair quote A: $75 parts plus $40 labor, ready Friday. Quote B: $120 total, ready Wednesday. Prefer the lowest total cost unless the difference is $5 or less, then choose the earlier date."}}],"readQuery":title("Quotes"),"prompt":format!("Read my {namespace} Quotes note, calculate both total prices, and recommend one using my stated rule."),"answerMustContain":["115","120","Wednesday"]},
        {"id":"notes-handoff-summary","seeds":[{"tool":"create_note","arguments":{"title":title("Handoff"),"body":"Alex owns the landing-page draft, due Monday. Mina reviews the accessibility checklist, due Tuesday. I must send the finalized screenshots to Alex before Monday. Keep the reply as a checklist; do not send anything."}}],"readQuery":title("Handoff"),"prompt":format!("Find my {namespace} Handoff note and summarize the three owners and deadlines as a checklist. Do not send any messages."),"answerMustContain":["Alex","Mina","Monday","Tuesday","screenshots"]},
        {"id":"notes-literal-text","seeds":[{"tool":"create_note","arguments":{"title":title("Literal text"),"body":"Keep this literal example: <invoice total=\"42\"> & coffee ☕. This is plain text, not HTML. The amount is 42 dollars."}}],"readQuery":title("Literal text"),"prompt":format!("Read my {namespace} Literal text note. What amount does it mention, and which drink symbol is included?"),"answerMustContain":["42","coffee"]},
        {"id":"notes-project-next-step","seeds":[{"tool":"create_note","arguments":{"title":title("Project next step"),"body":"Website launch: design approved; privacy review complete; DNS migration still pending. Next step: update DNS after the backup is verified. Owner: me. Do not change DNS from this note."}}],"readQuery":title("Project next step"),"prompt":format!("Read my {namespace} Project next step note and identify the unfinished step, its prerequisite, and the owner. Do not execute the step."),"answerMustContain":["DNS","backup"]}
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acceptance_never_executes_an_unscoped_or_changed_proposal() {
        let namespace = "BuntiniQA-01234567-0123-4567-8901-012345678901";
        let approved =
            json!({"title":format!("{namespace} Checklist"),"body":"Known test payload"});
        for (scope, tool, proposed) in [
            ("BuntiniQA", "create_note", approved.clone()),
            (
                namespace,
                "create_note",
                json!({"title":"A personal note","body":"Known test payload"}),
            ),
            (
                namespace,
                "create_note",
                json!({"title":approved["title"],"body":"Changed payload"}),
            ),
            (
                namespace,
                "draft_email",
                json!({"subject":"Anything","body":"Must not execute"}),
            ),
        ] {
            let refused = execute_exact(scope, tool, &proposed, &approved).await;
            assert_eq!(refused["outcome"], "failed");
            assert_eq!(refused["evidence"]["executorStarted"], false);
        }
    }

    #[tokio::test]
    async fn cleanup_without_exact_identifier_or_readback_never_runs() {
        let namespace = "BuntiniQA-01234567-0123-4567-8901-012345678901";
        let approved =
            json!({"title":format!("{namespace} Checklist"),"body":"Known test payload"});
        let incomplete = json!({"outcome":"unknown","tool":"create_note","objectId":null,"destination":null,"message":"No receipt","evidence":{},"error":"Unknown"});
        let cleanup = cleanup_receipt(namespace, &incomplete, &approved).await;
        assert_eq!(cleanup["passed"], false);
        assert_eq!(cleanup["manualReconciliationRequired"], true);
    }
}
