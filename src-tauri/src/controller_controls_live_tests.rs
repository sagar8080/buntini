//! Actual Apple-proposed, approved durable controls over a disposable profile.
use super::apple_live_tests::{approve_live_action, live_state, record_live_case, run_live_prompt};
use super::*;

pub(super) async fn control_workflows() -> Vec<Value> {
    let mut rows = Vec::new();
    {
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let first = run_live_prompt(
            &state,
            "Remember that my preferred quiet work location is Maple Library.",
        )
        .await;
        let proposed = first
            .workflow
            .as_ref()
            .and_then(|flow| flow.steps.last())
            .cloned();
        let valid = proposed.as_ref().is_some_and(|step| {
            let fact = step.arguments["fact"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase();
            step.tool == "remember"
                && fact.contains("maple library")
                && fact.contains("quiet")
                && fact.contains("work")
        });
        let approved = if valid {
            let step = proposed.as_ref().unwrap();
            Some(approve_live_action(&state, &step.id, step.arguments.clone()).await)
        } else {
            None
        };
        let memories = state.store.lock().unwrap().memories().unwrap();
        state.models.shutdown().await;
        drop(state);
        let restarted = live_state(dir.path());
        restarted.store.lock().unwrap().new_conversation().unwrap();
        let recalled =
            run_live_prompt(&restarted, "Where do I prefer to work when I need quiet?").await;
        let calls = first.apple_calls
            + approved
                .as_ref()
                .map(|turn| turn.apple_calls)
                .unwrap_or_default()
            + recalled.apple_calls;
        let checks = json!({"valid_ai_proposal":valid,"approved_write_completed":approved.as_ref().is_some_and(|turn|turn.task.state=="completed"),
            "exactly_one_real_memory":memories.len()==1 && memories[0].fact.to_lowercase().contains("maple library"),
            "recall_after_restart_and_new_thread":recalled.task.state=="completed" && recalled.task.answer.to_lowercase().contains("maple library")});
        rows.push(record_live_case(json!({"id":"control_remember_work_location","goal":"Ask the assistant to remember a work preference, approve the actual Store write, restart and recall it in a new conversation","passed":checks.as_object().unwrap().values().all(|value|value==true),"apple_calls":calls,"checks":checks,"initial":first,"approved":approved,"memories_read_back":memories,"after_restart":recalled,"source_kind":"synthetic user preference, real disposable persistent profile; no preseeded memory"})));
        restarted.models.shutdown().await;
    }
    {
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let initial = run_live_prompt(&state,"Set a 12-minute timer for the rice and set a 6-minute timer for the vegetables.").await;
        let mut remaining = vec![720u64, 360];
        let mut current = None;
        let mut approvals = Vec::new();
        let mut gate_errors = Vec::new();
        for _ in 0..2 {
            let turn = current.as_ref().unwrap_or(&initial);
            let Some(step) = turn
                .workflow
                .as_ref()
                .and_then(|flow| flow.steps.last())
                .cloned()
            else {
                break;
            };
            let seconds = step.arguments["seconds"].as_u64();
            let index = seconds
                .and_then(|seconds| remaining.iter().position(|expected| *expected == seconds));
            if step.tool != "set_timer" || index.is_none() || turn.task.state != "waiting_action" {
                gate_errors.push("The AI proposal did not match an outstanding requested timer; no write was approved.");
                break;
            }
            remaining.remove(index.unwrap());
            current = Some(approve_live_action(&state, &step.id, step.arguments.clone()).await);
            approvals.push(serde_json::to_value(current.as_ref().unwrap()).unwrap());
        }
        let timers = crate::delivery::timers(&state.store.lock().unwrap()).unwrap();
        let mut durations = timers.iter().map(|timer| timer.seconds).collect::<Vec<_>>();
        durations.sort_unstable();
        let calls = initial.apple_calls
            + approvals
                .iter()
                .filter_map(|row| row["apple_calls"].as_u64())
                .sum::<u64>() as usize;
        let checks = json!({"all_requested_proposals_approved":remaining.is_empty() && gate_errors.is_empty(),
            "two_real_distinct_timers":durations==vec![360u64,720] && timers[0].id!=timers[1].id,
            "completed":current.as_ref().is_some_and(|turn|turn.task.state=="completed"),
            "both_pending":timers.iter().all(|timer|timer.state=="pending")});
        rows.push(record_live_case(json!({"id":"control_parallel_cooking_timers","goal":"Create two independently verified timers from a natural language cooking request","passed":checks.as_object().unwrap().values().all(|value|value==true),"apple_calls":calls,"checks":checks,"initial":initial,"approvals":approvals,"gate_errors":gate_errors,"timers_read_back":timers,"source_kind":"real timer records in a disposable persistent profile; scheduler not launched, no alerts sent"})));
        state.models.shutdown().await;
    }
    rows
}
