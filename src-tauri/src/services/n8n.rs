//! n8n on the person's own host: the latest workflow runs and how they went.
use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, flag, is_today, list, plural, short, text, time, when, Row, Snapshot};
use super::{Kind, Memo};
use crate::surface::PillState;

pub const CAP: usize = 512 * 1024;
/// Workflows come with all their nodes, so the list can be large.
const WORKFLOWS_CAP: usize = 8 * 1024 * 1024;
const NAMES_EVERY: i64 = 6 * 3600;
/// Sooner when a run names a workflow not seen yet.
const UNKNOWN_EVERY: i64 = 1800;

pub async fn poll(net: &Net, memo: &mut Memo, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let value = net.get(&["api", "v1", "executions"], &[("limit", "20".into())], CAP).await?;
    let unknown = list(&value, "/data").iter().filter_map(|run| text(run, "/workflowId")).any(|id| !memo.names.contains_key(&id));
    if memo.due("names", if unknown { UNKNOWN_EVERY } else { NAMES_EVERY }, now.timestamp()) {
        match net.get(&["api", "v1", "workflows"], &[("limit", "100".into()), ("excludePinnedData", "true".into())], WORKFLOWS_CAP).await {
            Ok(workflows) => memo.names = names(&workflows),
            Err(failure @ (Failure::Cancelled | Failure::Budget(_) | Failure::Limited { .. } | Failure::Rejected(_))) => return Err(failure),
            // Runs still show, by workflow number.
            Err(_) => {}
        }
        memo.mark("names", now.timestamp());
    }
    parse(&value, &memo.names, net.url(&[]).as_str().trim_end_matches('/'), now)
}

pub fn names(value: &Value) -> HashMap<String, String> {
    list(value, "/data").iter().filter_map(|workflow| Some((text(workflow, "/id")?, text(workflow, "/name")?))).collect()
}

fn state(run: &Value) -> (PillState, &'static str) {
    match text(run, "/status").as_deref() {
        Some("success") => (PillState::Done, "Succeeded"),
        Some("error" | "crashed") => (PillState::Error, "Failed"),
        Some("running" | "new") => (PillState::Working, "Running"),
        Some("waiting") => (PillState::Idle, "Waiting"),
        Some("canceled") => (PillState::Idle, "Canceled"),
        // Versions before `status`: finished, stopped without finishing, or still going.
        _ if flag(run, "/finished") => (PillState::Done, "Succeeded"),
        _ if run.get("stoppedAt").is_some_and(|at| !at.is_null()) => (PillState::Error, "Failed"),
        _ => (PillState::Working, "Running"),
    }
}

/// `base` is `https://host[:port]`, where the editor's pages live.
pub fn parse(value: &Value, names: &HashMap<String, String>, base: &str, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let runs = value.pointer("/data").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::N8n))?;
    let mut rows = vec![];
    let (mut failed_today, mut ran_today, mut running) = (0, 0, None);
    for run in runs.iter().filter(|run| run.get("id").is_some()) {
        let (pill, label) = state(run);
        let workflow = text(run, "/workflowId").unwrap_or_default();
        let name = names.get(&workflow).cloned().unwrap_or_else(|| format!("Workflow {workflow}"));
        let started = time(run, "/startedAt");
        if started.is_some_and(|at| is_today(at, now)) {
            ran_today += 1;
            if pill == PillState::Error {
                failed_today += 1;
            }
        }
        if pill == PillState::Working && running.is_none() {
            running = Some(name.clone());
        }
        let id = text(run, "/id").unwrap_or_default();
        rows.push(
            Row::new(format!("run-{id}"), name)
                .detail([Some(label.into()), started.map(|at| when(at, now))])
                .state(Some(pill))
                .url(Some(format!("{base}/workflow/{workflow}/executions/{id}"))),
        );
    }
    let title = if failed_today > 0 {
        format!("{} today", plural(failed_today, "failed run", "failed runs"))
    } else if let Some(name) = &running {
        format!("Running {name}")
    } else if ran_today > 0 {
        format!("{} today", plural(ran_today, "run", "runs"))
    } else {
        "No runs today".into()
    };
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge: (failed_today > 0).then(|| short(failed_today)), rows, graph: None, busy: running.is_some() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, replay};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn runs_and_workflow_names_read_from_recorded_answers() {
        let fixtures = fixture("n8n");
        let now = at("2026-11-16T15:00:00+01:00");
        let (workflows, _) = replay(Kind::N8n, &fixtures["workflows"], WORKFLOWS_CAP).await;
        let names = names(&workflows.unwrap());
        assert_eq!(names.get("wf1").map(String::as_str), Some("Daily report"));
        let (value, _) = replay(Kind::N8n, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), &names, "https://n8n.example.com", now).unwrap();
        assert_eq!(snapshot.title, "1 failed run today");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        assert!(snapshot.busy);
        let states: Vec<Option<PillState>> = snapshot.rows.iter().map(|row| row.state).collect();
        assert_eq!(states, [Some(PillState::Working), Some(PillState::Error), Some(PillState::Done), Some(PillState::Done), Some(PillState::Error)]);
        assert_eq!(snapshot.rows[1].title, "Sync invoices");
        assert_eq!(snapshot.rows[1].url.as_deref(), Some("https://n8n.example.com/workflow/wf2/executions/1041"));
        assert_eq!(snapshot.rows[3].title, "Workflow wf9");
        assert_eq!(snapshot.rows[2].detail.as_deref(), Some("Succeeded · 10:00"));
        // Nothing interrupts for n8n: failures show on the pill and card only.
        assert!(snapshot.rows.iter().all(|row| row.alert.is_none()));
    }

    #[tokio::test]
    async fn quiet_days_and_odd_answers() {
        let now = at("2026-11-16T15:00:00+01:00");
        let snapshot = parse(&serde_json::json!({"data": [], "nextCursor": null}), &HashMap::new(), "https://n8n.example.com", now).unwrap();
        assert_eq!(snapshot.title, "No runs today");
        assert!(parse(&serde_json::json!({"message": "Unauthorized"}), &HashMap::new(), "https://n8n.example.com", now).is_err());
        let fixtures = fixture("n8n");
        let (value, _) = replay(Kind::N8n, &fixtures["unauthorized"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
    }

    #[tokio::test]
    async fn answers_over_the_cap_are_refused() {
        assert_capped(Kind::N8n, CAP).await;
    }
}
