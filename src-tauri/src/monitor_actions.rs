//! Approved, idempotent creation of recurring searches and public price watches.
//! Creation is local SQLite work. Only the scheduler performs future network work.
use crate::{
    actions::{ActionOutcome, ActionReceipt, Field},
    store::Store,
    types::{now, PriceWatch, SavedSearch},
};
use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};

pub const TOOLS: [&str; 2] = ["create_saved_search", "create_price_watch"];

fn text(args: &Map<String, Value>, key: &str, max: usize) -> Result<String, String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be text."))?
        .trim();
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("{key} must contain 1–{max} visible characters."));
    }
    Ok(value.into())
}
pub fn validate(tool: &str, args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    let allowed = match tool {
        "create_saved_search" => &["query", "interval_minutes"][..],
        "create_price_watch" => &["url", "label", "currency", "threshold", "interval_minutes"][..],
        _ => return Err("Unknown monitoring action.".into()),
    };
    if let Some(extra) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{tool} does not accept {extra}."));
    }
    // Approval sheets return text inputs; accept integer text while preserving a
    // numeric schema and normalized integer in the durable approved step.
    let interval = args
        .get("interval_minutes")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .filter(|value| (15..=525600).contains(value))
        .ok_or("Check intervals must be whole minutes between 15 and 525,600.")?;
    let mut out = Map::new();
    out.insert("interval_minutes".into(), json!(interval));
    if tool == "create_saved_search" {
        let query = text(args, "query", 2000)?;
        crate::scheduler::validate(&query, interval as u32)?;
        out.insert("query".into(), json!(query));
    } else {
        let url = crate::research::validate_public_url(&text(args, "url", 2000)?)?.to_string();
        let label = text(args, "label", 120)?;
        let currency = text(args, "currency", 3)?.to_ascii_uppercase();
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err("Use a three-letter currency code such as USD.".into());
        }
        let threshold = crate::pricing::canonical_decimal(&text(args, "threshold", 80)?)
            .ok_or("Enter a positive decimal target price with up to six decimal places.")?;
        out.extend([
            (String::from("url"), json!(url)),
            (String::from("label"), json!(label)),
            (String::from("currency"), json!(currency)),
            (String::from("threshold"), json!(threshold)),
        ]);
    }
    Ok(out)
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
    if tool == "create_saved_search" {
        ("Run this search regularly?".into(), vec![field("query", "Search sent to your selected search provider"), field("interval_minutes", "Check every (minutes)")], "Allow authorizes repeated searches for this exact query, starting after the chosen interval. Each check can use your configured search allowance or budget. New or changed results appear in Buntini; desktop alerts need notification permission. Checks run while Buntini is open and your Mac is awake. You can pause or delete this search in Scheduled work.".into())
    } else {
        ("Track this product's price?".into(), vec![field("url", "Public product page"), field("label", "Watch name"), field("currency", "Currency code"), field("threshold", "Alert at or below"), field("interval_minutes", "Check every (minutes)")], "Allow authorizes repeated reads of this exact public HTTPS page, starting after the chosen interval. Buntini tracks a published exact structured price; pages requiring a login or JavaScript checkout are unsupported. This creates a watch; it does not verify the current price. Checks run while Buntini is open and your Mac is awake. You can pause or delete it in Scheduled work.".into())
    }
}
fn failure(tool: &str, outcome: ActionOutcome, message: String) -> ActionReceipt {
    ActionReceipt {
        outcome,
        tool: tool.into(),
        object_id: None,
        destination: Some("Buntini · Scheduled work".into()),
        message: message.clone(),
        evidence: json!({}),
        error: Some(message),
    }
}
fn matches_search(search: &SavedSearch, args: &Map<String, Value>) -> bool {
    search.enabled
        && search.query == args["query"].as_str().unwrap_or_default()
        && u64::from(search.interval_minutes) == args["interval_minutes"].as_u64().unwrap_or(0)
}
fn matches_watch(watch: &PriceWatch, args: &Map<String, Value>) -> bool {
    watch.enabled
        && watch.url == args["url"].as_str().unwrap_or_default()
        && watch.label == args["label"].as_str().unwrap_or_default()
        && watch.currency == args["currency"].as_str().unwrap_or_default()
        && watch.threshold == args["threshold"].as_str().unwrap_or_default()
        && u64::from(watch.interval_minutes) == args["interval_minutes"].as_u64().unwrap_or(0)
}
/// intent_id is supplied by the durable execution journal, never by the model.
/// Replaying the same approved intent verifies the existing object without
/// changing its schedule or creating another recurring job.
pub fn execute(
    store: &Store,
    intent_id: &str,
    tool: &str,
    raw: &Map<String, Value>,
) -> ActionReceipt {
    let args = match validate(tool, &Value::Object(raw.clone())) {
        Ok(args) => args,
        Err(error) => return failure(tool, ActionOutcome::Failed, error),
    };
    if intent_id.is_empty()
        || intent_id.len() > 200
        || !intent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return failure(
            tool,
            ActionOutcome::Failed,
            "This monitoring request has no valid durable intent identifier.".into(),
        );
    }
    let id = format!("action-{intent_id}");
    let transaction = match store.db.unchecked_transaction() {
        Ok(tx) => tx,
        Err(error) => return failure(tool, ActionOutcome::Failed, error.to_string()),
    };
    // Claim SQLite's write lock before checking the idempotency key or quota.
    if let Err(error) = store
        .db
        .execute("UPDATE saved_searches SET enabled=enabled WHERE 0", [])
    {
        return failure(tool, ActionOutcome::Failed, error.to_string());
    }
    let result = (|| -> Result<(String, Value), String> {
        let previous: Option<(String, String)> = store
            .db
            .query_row(
                "SELECT tool,args FROM monitor_intents WHERE id=?1",
                [intent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let serialized_args = serde_json::to_string(&args).map_err(|error| error.to_string())?;
        if let Some((previous_tool, previous_args)) = &previous {
            if previous_tool != tool || previous_args != &serialized_args {
                return Err("This approved intent has already been used with different settings. Start a new request to change the monitor.".into());
            }
            let still_exists = if tool == "create_saved_search" {
                store.saved_search(&id)?.is_some()
            } else {
                store.price_watch(&id)?.is_some()
            };
            if !still_exists {
                return Err("This monitor was already created and later deleted. The old approval cannot recreate it; make a new request if you want to track it again.".into());
            }
        }
        // This tombstone and the object commit together. Deleting the object
        // intentionally keeps the old approval consumed.
        store.db.execute("INSERT OR IGNORE INTO monitor_intents(id,tool,args,monitor_id) VALUES(?1,?2,?3,?4)", rusqlite::params![intent_id, tool, serialized_args, id]).map_err(|error| error.to_string())?;
        let interval_minutes = args["interval_minutes"].as_u64().unwrap() as u32;
        let next_run_at = (chrono::Utc::now()
            + chrono::Duration::minutes(i64::from(interval_minutes)))
        .to_rfc3339();
        if tool == "create_saved_search" {
            if store.price_watch(&id)?.is_some() {
                return Err("This intent already created a different type of monitor. Start a new request to change it.".into());
            }
            if let Some(existing) = store.saved_search(&id)? {
                if !matches_search(&existing, &args) {
                    return Err("This search already exists but its settings have changed. It was not duplicated or overwritten.".into());
                }
            } else {
                if store.saved_searches()?.len() >= 20 {
                    return Err(
                        "You can save up to 20 searches. Delete one before adding another.".into(),
                    );
                }
                store.save_search(&SavedSearch {
                    id: id.clone(),
                    query: args["query"].as_str().unwrap().into(),
                    interval_minutes,
                    enabled: true,
                    created_at: now(),
                    next_run_at,
                    last_result_task_id: None,
                    last_error: None,
                })?;
                store.audit(
                    "saved_search.allowed",
                    &format!("{id}: approved recurring search; every {interval_minutes} minutes"),
                )?;
            }
            let saved = store
                .saved_search(&id)?
                .ok_or("The saved search could not be read back.")?;
            if !matches_search(&saved, &args) {
                return Err(
                    "The saved search did not match the approved query and interval.".into(),
                );
            }
            Ok((format!("Saved the recurring search. Its next check is {}; checks require Buntini to be running and your Mac awake.", saved.next_run_at), json!({"verified":true,"record":saved,"networkRequestMade":false})))
        } else {
            if store.saved_search(&id)?.is_some() {
                return Err("This intent already created a different type of monitor. Start a new request to change it.".into());
            }
            if let Some(existing) = store.price_watch(&id)? {
                if !matches_watch(&existing, &args) {
                    return Err("This price watch already exists but its settings have changed. It was not duplicated or overwritten.".into());
                }
            } else {
                if store.price_watches()?.len() >= 20 {
                    return Err(
                        "You can save up to 20 price watches. Delete one before adding another."
                            .into(),
                    );
                }
                store.save_price_watch(&PriceWatch {
                    id: id.clone(),
                    url: args["url"].as_str().unwrap().into(),
                    label: args["label"].as_str().unwrap().into(),
                    currency: args["currency"].as_str().unwrap().into(),
                    threshold: args["threshold"].as_str().unwrap().into(),
                    interval_minutes,
                    enabled: true,
                    created_at: now(),
                    next_run_at,
                    last_checked_at: None,
                    last_price: None,
                    last_error: None,
                    last_availability: None,
                    history: vec![],
                })?;
                store.audit("price_watch.allowed", &format!("{id}: approved recurring public HTTPS fetch; every {interval_minutes} minutes"))?;
            }
            let saved = store
                .price_watch(&id)?
                .ok_or("The price watch could not be read back.")?;
            if !matches_watch(&saved, &args) {
                return Err("The price watch did not match the approved page, currency, threshold and interval.".into());
            }
            Ok((format!("Saved the price watch. Its next check is {}; the current price has not been verified by creating this watch.", saved.next_run_at), json!({"verified":true,"record":saved,"networkRequestMade":false})))
        }
    })();
    let (message, evidence) = match result {
        Ok(result) => result,
        Err(error) => return failure(tool, ActionOutcome::Failed, error),
    };
    if let Err(error) = transaction.commit() {
        return failure(tool, ActionOutcome::Unknown, format!("I could not confirm that the monitor was committed: {error}. Inspect Scheduled work before trying again; this intent will not be repeated automatically."));
    }
    ActionReceipt {
        outcome: ActionOutcome::Succeeded,
        tool: tool.into(),
        object_id: Some(id),
        destination: Some("Buntini · Scheduled work".into()),
        message,
        evidence,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn search() -> Map<String, Value> {
        validate(
            "create_saved_search",
            &json!({"query":"electric cargo bike availability","interval_minutes":60}),
        )
        .unwrap()
    }
    fn watch() -> Map<String, Value> {
        validate("create_price_watch", &json!({"url":"https://shop.example.com/item","label":"Cargo bike","currency":"usd","threshold":"1200.00","interval_minutes":60})).unwrap()
    }
    #[test]
    fn monitor_validation_rejects_hidden_scope_and_unsafe_inputs() {
        for invalid in [
            json!({"query":"q","interval_minutes":1}),
            json!({"query":"q","interval_minutes":15.5}),
            json!({"query":"q","interval_minutes":60,"run_now":true}),
            json!({"query":"","interval_minutes":60}),
        ] {
            assert!(validate("create_saved_search", &invalid).is_err());
        }
        for url in [
            "http://shop.example.com/item",
            "https://localhost/item",
            "https://127.0.0.1/item",
            "https://user:secret@shop.example.com/item",
        ] {
            let mut args = watch();
            args.insert("url".into(), json!(url));
            assert!(validate("create_price_watch", &Value::Object(args)).is_err());
        }
        assert_eq!(watch()["threshold"], "1200");
        assert_eq!(watch()["currency"], "USD");
        let (_, fields, note) = describe("create_saved_search", &search());
        assert_eq!(fields.len(), 2);
        assert!(note.contains("repeated searches"));
        assert!(note.contains("budget"));
    }
    #[test]
    fn approved_search_and_watch_read_back_and_replay_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("monitors.sqlite");
        let store = Store::open(&path).unwrap();
        let first = execute(&store, "intent-search", "create_saved_search", &search());
        assert_eq!(first.outcome, ActionOutcome::Succeeded);
        assert_eq!(first.evidence["networkRequestMade"], false);
        let next = first.evidence["record"]["nextRunAt"].clone();
        drop(store);
        let store = Store::open(&path).unwrap();
        let again = execute(&store, "intent-search", "create_saved_search", &search());
        assert_eq!(again.object_id, first.object_id);
        assert_eq!(again.evidence["record"]["nextRunAt"], next);
        assert_eq!(store.saved_searches().unwrap().len(), 1);
        let created = execute(&store, "intent-watch", "create_price_watch", &watch());
        assert_eq!(created.outcome, ActionOutcome::Succeeded);
        assert_eq!(created.evidence["record"]["lastCheckedAt"], Value::Null);
        assert_eq!(
            execute(&store, "intent-watch", "create_price_watch", &watch()).object_id,
            created.object_id
        );
        assert_eq!(store.price_watches().unwrap().len(), 1);
        let mut changed = search();
        changed.insert("query".into(), json!("different query"));
        assert_eq!(
            execute(&store, "intent-search", "create_saved_search", &changed).outcome,
            ActionOutcome::Failed
        );
        assert_eq!(
            execute(&store, "intent-search", "create_price_watch", &watch()).outcome,
            ActionOutcome::Failed
        );
        store
            .delete_search(first.object_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            execute(&store, "intent-search", "create_saved_search", &search()).outcome,
            ActionOutcome::Failed
        );
        assert!(store.saved_searches().unwrap().is_empty());
        store
            .delete_price_watch(created.object_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            execute(&store, "intent-watch", "create_price_watch", &watch()).outcome,
            ActionOutcome::Failed
        );
        assert!(store.price_watches().unwrap().is_empty());
    }
    #[test]
    fn failed_monitor_receipts_roll_back_saved_objects() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.db.execute_batch("CREATE TRIGGER fail_monitor_audit BEFORE INSERT ON audit BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert_eq!(
            execute(&store, "failed", "create_saved_search", &search()).outcome,
            ActionOutcome::Failed
        );
        assert!(store.saved_searches().unwrap().is_empty());
        assert_eq!(
            execute(&store, "failed-price", "create_price_watch", &watch()).outcome,
            ActionOutcome::Failed
        );
        assert!(store.price_watches().unwrap().is_empty());
    }
    #[test]
    fn export_and_erase_cover_approved_intents_timers_and_monitor_history() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let receipt = execute(&store, "export-intent", "create_saved_search", &search());
        assert_eq!(receipt.outcome, ActionOutcome::Succeeded);
        crate::delivery::schedule(&store, 60, chrono::Utc::now()).unwrap();
        let run = crate::delivery::monitor_started(
            &store,
            "saved-search-action-export-intent",
            None,
            chrono::Utc::now(),
        )
        .unwrap();
        crate::delivery::monitor_finished(&store, &run, "completed", None).unwrap();
        crate::delivery::monitor_delivery(
            &store,
            "saved-search-action-export-intent",
            Err("Notifications denied."),
        )
        .unwrap();
        let export = store.export_snapshot().unwrap();
        assert_eq!(
            export["monitorIntents"][0]["arguments"]["query"],
            search()["query"]
        );
        assert_eq!(
            export["monitorIntents"][0]["monitorId"],
            receipt.object_id.unwrap()
        );
        assert_eq!(export["timers"].as_array().unwrap().len(), 1);
        assert_eq!(
            export["monitorRuns"][0]["deliveryError"],
            "Notifications denied."
        );
        store.erase_everything().unwrap();
        let empty = store.export_snapshot().unwrap();
        for key in ["monitorIntents", "timers", "monitorRuns", "savedSearches"] {
            assert!(
                empty[key].as_array().unwrap().is_empty(),
                "{key} must be erased"
            );
        }
    }
}
