//! GitLab, on gitlab.com or the person's own host: your open merge requests with their
//! pipelines and reviews requested from you, from one read-only GraphQL query, and your
//! activity calendar.
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::{json, Value};

use super::fetch::{Failure, Net};
use super::github::graphql_errors;
use super::snapshot::{self, flag, graph, list, number, plural, short, text, time, when, Row, Snapshot};
use super::{Kind, Memo};
use crate::surface::PillState;

pub const CAP: usize = 512 * 1024;
const CALENDAR_CAP: usize = 64 * 1024;
const GRAPH_EVERY: i64 = 6 * 3600;

const QUERY: &str = "query { currentUser { username \
authoredMergeRequests(state: opened, first: 15, sort: UPDATED_DESC) { count nodes { \
iid title webUrl draft updatedAt project { fullPath } headPipeline { status } } } \
reviewRequestedMergeRequests(state: opened, first: 15, sort: UPDATED_DESC) { count nodes { \
iid title webUrl updatedAt project { fullPath } author { username } } } } }";

pub async fn poll(net: &Net, memo: &mut Memo, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let value = net.post(&["api", "graphql"], &json!({ "query": QUERY }), CAP).await?;
    let username = text(&value, "/data/currentUser/username");
    if let Some(username) = username.filter(|_| memo.due("graph", GRAPH_EVERY, now.timestamp())) {
        // The calendar is a public page of the profile, so it goes without the key. A private
        // profile has none; the card simply has no graph then.
        match net.get_public(&["users", &username, "calendar.json"], CALENDAR_CAP).await {
            Ok(days) => memo.graph = Some(calendar(&days, now)),
            Err(failure @ (Failure::Cancelled | Failure::Budget(_) | Failure::Limited { .. })) => return Err(failure),
            Err(_) => {}
        }
        memo.mark("graph", now.timestamp());
    }
    parse(&value, now, memo.graph.clone())
}

/// `{"2026-11-16": 3, …}` laid out as the activity graph.
pub fn calendar(value: &Value, now: DateTime<FixedOffset>) -> Vec<u32> {
    let days: BTreeMap<NaiveDate, u32> = value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(date, count)| Some((NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?, count.as_u64()?.min(u32::MAX as u64) as u32)))
        .collect();
    graph(&days, now.date_naive())
}

fn pipeline(status: Option<&str>) -> (PillState, Option<&'static str>) {
    match status {
        Some("FAILED") => (PillState::Error, Some("Pipeline failed")),
        Some("SUCCESS") => (PillState::Done, Some("Pipeline passed")),
        Some("CREATED" | "WAITING_FOR_RESOURCE" | "PREPARING" | "WAITING_FOR_CALLBACK" | "PENDING" | "RUNNING" | "SCHEDULED") => (PillState::Working, Some("Pipeline running")),
        _ => (PillState::Idle, None),
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>, graph: Option<Vec<u32>>) -> Result<Snapshot, Failure> {
    graphql_errors(Kind::Gitlab, value, "/data/currentUser")?;
    // Without a key GitLab still answers, with no user.
    let user = value.pointer("/data/currentUser").ok_or_else(|| Failure::unreadable(Kind::Gitlab))?;
    if user.is_null() {
        return Err(Failure::Rejected("GitLab rejected the saved credentials. Check them in Settings.".into()));
    }
    let place = |mr: &Value| format!("{}!{}", text(mr, "/project/fullPath").unwrap_or_default(), text(mr, "/iid").unwrap_or_default());
    let updated = |mr: &Value| time(mr, "/updatedAt").map(|at| when(at, now));

    let reviews = list(user, "/reviewRequestedMergeRequests/nodes");
    let mut rows: Vec<Row> = reviews
        .iter()
        .filter(|mr| mr.get("iid").is_some())
        .map(|mr| {
            Row::new(format!("review-{}", place(mr)), text(mr, "/title").unwrap_or_default())
                .detail([Some(place(mr)), text(mr, "/author/username"), updated(mr)])
                .state(Some(PillState::Waiting))
                .url(text(mr, "/webUrl"))
                .alert("Review requested", true)
        })
        .collect();
    let mut failing = vec![];
    let mut others = vec![];
    let mut busy = false;
    for mr in list(user, "/authoredMergeRequests/nodes").iter().filter(|mr| mr.get("iid").is_some()) {
        let (state, label) = pipeline(text(mr, "/headPipeline/status").as_deref());
        busy |= state == PillState::Working;
        let row = Row::new(format!("mr-{}", place(mr)), text(mr, "/title").unwrap_or_default())
            .detail([Some(place(mr)), flag(mr, "/draft").then(|| "Draft".into()), label.map(str::to_string), updated(mr)])
            .state(Some(state))
            .url(text(mr, "/webUrl"))
            .alert("Pipeline failed", state == PillState::Error);
        if state == PillState::Error {
            failing.push(row);
        } else {
            others.push(row);
        }
    }
    let review_count = number(user, "/reviewRequestedMergeRequests/count").map_or(rows.len(), |count| count.max(0) as usize);
    let open = number(user, "/authoredMergeRequests/count").map_or(failing.len() + others.len(), |count| count.max(0) as usize);
    let title = if review_count > 0 {
        plural(review_count, "review requested", "reviews requested")
    } else if let Some(first) = failing.first() {
        format!("Pipeline failed on {}", first.title)
    } else {
        plural(open, "open merge request", "open merge requests")
    };
    let badge = if review_count > 0 { Some(short(review_count)) } else { (open > 0).then(|| short(open)) };
    rows.extend(failing);
    rows.extend(others);
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge, rows, graph, busy })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, refusals, replay};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn merge_requests_reviews_and_pipelines_read_from_a_recorded_answer() {
        let fixtures = fixture("gitlab");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, limit) = replay(Kind::Gitlab, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now, Some(vec![2])).unwrap();
        assert_eq!(limit.remaining, Some(1995));
        assert_eq!(snapshot.title, "2 reviews requested");
        let ids: Vec<&str> = snapshot.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["review-acme/api!31", "mr-sagar/buntini!9", "mr-sagar/buntini!8"]);
        assert_eq!(snapshot.rows[1].alert, Some("Pipeline failed"));
        assert_eq!(snapshot.rows[2].state, Some(PillState::Working));
        assert_eq!(snapshot.rows[2].detail.as_deref(), Some("sagar/buntini!8 · Draft · Pipeline running · 10:15"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://gitlab.com/acme/api/-/merge_requests/31"));
        assert!(snapshot.busy);
        assert_eq!(snapshot.graph, Some(vec![2]));
    }

    #[tokio::test]
    async fn the_calendar_becomes_the_graph() {
        let fixtures = fixture("gitlab");
        let (value, _) = replay(Kind::Gitlab, &fixtures["calendar"], CALENDAR_CAP).await;
        let cells = calendar(&value.unwrap(), at("2026-11-16T15:00:00+01:00"));
        assert_eq!(cells.last(), Some(&4));
        assert_eq!(cells.iter().sum::<u32>(), 4 + 2 + 7);
    }

    #[tokio::test]
    async fn no_user_means_the_key_was_refused() {
        // Recorded from gitlab.com: without a key the query answers 200 with no user.
        let (value, _) = replay(Kind::Gitlab, &refusals()["gitlab_graphql_noauth"], CAP).await;
        assert!(matches!(parse(&value.unwrap(), at("2026-11-16T15:00:00+01:00"), None), Err(Failure::Rejected(_))));
        let fixtures = fixture("gitlab");
        let (value, limit) = replay(Kind::Gitlab, &fixtures["throttled"], CAP).await;
        assert!(matches!(value, Err(Failure::Limited { .. })));
        assert_eq!(limit.retry_at, Some(crate::services::recorded::NOW + 30));
    }

    #[tokio::test]
    async fn answers_over_the_cap_are_refused() {
        assert_capped(Kind::Gitlab, CAP).await;
        assert_capped(Kind::Gitlab, CALENDAR_CAP).await;
    }
}
