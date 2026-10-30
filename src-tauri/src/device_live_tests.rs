//! Opt-in device checks for writes that change existing items. They run the real
//! Reminders, Calendar and Notes paths on this Mac against items they create themselves,
//! all named "Buntini device test", and print each receipt.
//!
//! cargo test --manifest-path src-tauri/Cargo.toml device_live -- --ignored --nocapture --test-threads=1
use crate::actions::{self, ActionOutcome};
use crate::types::Settings;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const NAME: &str = "Buntini device test";

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native/buntini-fm")
}

fn settings() -> Settings {
    Settings {
        apps: ["reminders", "calendar", "notes", "mail"].map(String::from).to_vec(),
        ..Settings::default()
    }
}

async fn write(tool: &str, args: Value) -> actions::ActionReceipt {
    let args = actions::validate(tool, &args).unwrap_or_else(|error| panic!("{tool}: {error}"));
    let receipt = actions::execute_with(tool, &args, &helper()).await;
    println!("{tool}: {:?} — {}{}", receipt.outcome, receipt.message, receipt.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default());
    receipt
}

async fn read(tool: &str, args: Value) -> Vec<Value> {
    let args: Map<String, Value> = args.as_object().unwrap().clone();
    let evidence = crate::apps::run_evidence(tool, &args, &helper(), &settings(), crate::apps::Launch::Allow)
        .await
        .unwrap_or_else(|error| panic!("{tool}: {error}"));
    evidence["records"].as_array().cloned().unwrap_or_default()
}

fn named<'a>(records: &'a [Value], title: &str) -> &'a Value {
    records
        .iter()
        .find(|record| record["title"] == title)
        .unwrap_or_else(|| panic!("“{title}” was not read back: {records:#?}"))
}

fn tomorrow() -> String {
    (chrono::Local::now().date_naive() + chrono::Days::new(1)).format("%Y-%m-%d").to_string()
}

#[tokio::test]
#[ignore]
async fn device_live_reminders_are_completed_and_rescheduled() {
    let day = tomorrow();
    let done = format!("{NAME} milk");
    let moved = format!("{NAME} dentist");
    assert_eq!(write("add_reminder", json!({"title":done, "due":format!("{day} 18:00")})).await.outcome, ActionOutcome::Succeeded);
    assert_eq!(write("add_reminder", json!({"title":moved, "due":format!("{day} 09:00")})).await.outcome, ActionOutcome::Succeeded);
    let records = read("read_reminders", json!({})).await;
    let first = named(&records, &done);
    println!("read back: {first}");
    let receipt = write("complete_reminder", json!({"id":first["id"], "title":done})).await;
    assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{receipt:#?}");
    let second = named(&records, &moved);
    let was = second["dueLocal"].as_str().unwrap_or_default().to_string();
    let receipt = write("reschedule_reminder", json!({"id":second["id"], "title":moved, "was_due":was, "due":format!("{day} 10:30")})).await;
    assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{receipt:#?}");
    let after = read("read_reminders", json!({})).await;
    assert!(after.iter().all(|record| record["title"] != done), "the completed reminder is still open");
    println!("after: {}", named(&after, &moved));
}

#[tokio::test]
#[ignore]
async fn device_live_an_event_is_moved_and_renamed() {
    let day = tomorrow();
    let title = format!("{NAME} planning");
    let renamed = format!("{NAME} planning (moved)");
    assert_eq!(
        write("add_calendar_event", json!({"title":title, "start":format!("{day} 15:00"), "minutes":45, "location":"Room 4"})).await.outcome,
        ActionOutcome::Succeeded
    );
    let records = read("read_calendar", json!({"day":day})).await;
    let event = named(&records, &title);
    println!("read back: {event}");
    let receipt = write(
        "update_calendar_event",
        json!({"id":event["id"], "occurrence":format!("{day} 15:00"), "was_title":title, "title":renamed,
            "start":format!("{day} 16:00"), "minutes":45, "location":event["location"]}),
    )
    .await;
    assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{receipt:#?}");
    let after = read("read_calendar", json!({"day":day})).await;
    println!("after: {}", named(&after, &renamed));
}

#[tokio::test]
#[ignore]
async fn device_live_text_is_added_to_the_end_of_a_note() {
    let title = format!("{NAME} shopping");
    assert_eq!(write("create_note", json!({"title":title, "body":"bread"})).await.outcome, ActionOutcome::Succeeded);
    let records = read("find_notes", json!({"query":title})).await;
    let note = named(&records, &title);
    println!("read back: {note}");
    let receipt = write("append_to_note", json!({"id":note["id"], "title":title, "text":"milk and eggs"})).await;
    assert_eq!(receipt.outcome, ActionOutcome::Succeeded, "{receipt:#?}");
    println!("after: {}", read("find_notes", json!({"query":title})).await.first().cloned().unwrap_or_default());
}
