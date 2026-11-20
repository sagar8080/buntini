//! GitHub: your open pull requests with their CI, reviews requested from you, and your
//! contribution graph, from one read-only GraphQL query to api.github.com.
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::{json, Value};

use super::fetch::{Failure, Net};
use super::snapshot::{self, flag, graph, list, number, short, text, time, when, Row, Snapshot};
use super::{Kind, Memo};
use crate::i18n::{t, tf, tn};
use crate::surface::PillState;

pub const HOST: &str = "api.github.com";
pub const CAP: usize = 768 * 1024;
/// The contribution graph changes slowly.
const GRAPH_EVERY: i64 = 6 * 3600;

/// Reads only. The contribution calendar is included when it is due.
const QUERY: &str = "query($graph: Boolean!) { \
viewer { login \
contributionsCollection @include(if: $graph) { contributionCalendar { weeks { contributionDays { date contributionCount } } } } \
pullRequests(first: 15, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount nodes { \
number title url isDraft updatedAt repository { nameWithOwner } \
commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } } } } \
reviews: search(query: \"is:open is:pr archived:false review-requested:@me\", type: ISSUE, first: 15) { issueCount nodes { \
... on PullRequest { number title url updatedAt repository { nameWithOwner } author { login } } } } }";

pub async fn poll(net: &Net, memo: &mut Memo, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let with_graph = memo.due("graph", GRAPH_EVERY, now.timestamp());
    let value = net.post(&["graphql"], &json!({ "query": QUERY, "variables": { "graph": with_graph } }), CAP).await?;
    let snapshot = parse(&value, now, memo.graph.clone())?;
    if with_graph {
        memo.graph = snapshot.graph.clone();
        memo.mark("graph", now.timestamp());
    }
    Ok(snapshot)
}

/// GraphQL answers 200 with `errors` when it refuses; data that came with them is still used.
pub fn graphql_errors(kind: Kind, value: &Value, data: &str) -> Result<(), Failure> {
    let errors = list(value, "/errors");
    if errors.is_empty() || value.pointer(data).is_some_and(|data| !data.is_null()) {
        return Ok(());
    }
    let message = errors.iter().find_map(|error| text(error, "/message")).unwrap_or_else(|| format!("{} refused the request.", kind.name()));
    let kinds: Vec<String> = errors.iter().filter_map(|error| text(error, "/type").or_else(|| text(error, "/extensions/code"))).collect();
    if kinds.iter().any(|kind| kind == "RATE_LIMITED" || kind == "RATE_LIMIT") {
        return Err(Failure::Limited { until: 0, message: format!("{} is limiting requests right now.", kind.name()) });
    }
    if kinds.iter().any(|kind| kind == "FORBIDDEN" || kind == "UNAUTHENTICATED" || kind == "INSUFFICIENT_SCOPES") {
        return Err(Failure::Rejected(format!("{} rejected the saved credentials. Check them in Settings.", kind.name())));
    }
    Err(Failure::Other(format!("{}: {}", kind.name(), snapshot::clip(&message, 160))))
}

fn ci(state: Option<&str>) -> (PillState, Option<String>) {
    match state {
        Some("FAILURE" | "ERROR") => (PillState::Error, Some(t("services.ci.failed"))),
        Some("PENDING" | "EXPECTED") => (PillState::Working, Some(t("services.ci.running"))),
        Some("SUCCESS") => (PillState::Done, Some(t("services.ci.passed"))),
        _ => (PillState::Idle, None),
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>, graph_before: Option<Vec<u32>>) -> Result<Snapshot, Failure> {
    graphql_errors(Kind::Github, value, "/data/viewer")?;
    let viewer = value.pointer("/data/viewer").filter(|viewer| viewer.is_object()).ok_or_else(|| Failure::unreadable(Kind::Github))?;
    let place = |pr: &Value| format!("{}#{}", text(pr, "/repository/nameWithOwner").unwrap_or_default(), number(pr, "/number").unwrap_or_default());
    let updated = |pr: &Value| time(pr, "/updatedAt").map(|at| when(at, now));

    let reviews: Vec<&Value> = list(value, "/data/reviews/nodes").iter().filter(|pr| pr.get("number").is_some()).collect();
    let mut rows: Vec<Row> = reviews
        .iter()
        .map(|pr| {
            Row::new(format!("review-{}", place(pr)), text(pr, "/title").unwrap_or_default())
                .detail([Some(place(pr)), text(pr, "/author/login"), updated(pr)])
                .state(Some(PillState::Waiting))
                .url(text(pr, "/url"))
                .alert(t("services.alert.reviewRequested"), true)
        })
        .collect();

    let mine = list(viewer, "/pullRequests/nodes");
    let mut failing: Vec<Row> = vec![];
    let mut others: Vec<Row> = vec![];
    let mut busy = false;
    for pr in mine {
        let (state, label) = ci(text(pr, "/commits/nodes/0/commit/statusCheckRollup/state").as_deref());
        busy |= state == PillState::Working;
        let draft = flag(pr, "/isDraft").then(|| t("services.draft"));
        let row = Row::new(format!("pr-{}", place(pr)), text(pr, "/title").unwrap_or_default())
            .detail([Some(place(pr)), draft, label, updated(pr)])
            .state(Some(state))
            .url(text(pr, "/url"))
            .alert(t("services.ci.failed"), state == PillState::Error);
        if state == PillState::Error {
            failing.push(row);
        } else {
            others.push(row);
        }
    }

    let review_count = number(value, "/data/reviews/issueCount").map_or(reviews.len(), |count| count.max(0) as usize);
    let open = number(viewer, "/pullRequests/totalCount").map_or(mine.len(), |count| count.max(0) as usize);
    let title = if review_count > 0 {
        tn("services.reviewsRequested", review_count as u64, &[])
    } else if let Some(first) = failing.first() {
        tf("services.ciFailedOn", &[("title", &first.title)])
    } else {
        tn("services.openPullRequests", open as u64, &[])
    };
    let badge = if review_count > 0 {
        Some(short(review_count))
    } else {
        (open > 0).then(|| short(open))
    };
    let calendar = list(viewer, "/contributionsCollection/contributionCalendar/weeks");
    let graph = if calendar.is_empty() {
        graph_before
    } else {
        let mut days = BTreeMap::new();
        for day in calendar.iter().flat_map(|week| list(week, "/contributionDays")) {
            if let (Some(date), Some(count)) = (text(day, "/date").and_then(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()), number(day, "/contributionCount")) {
                days.insert(date, count.clamp(0, u32::MAX as i64) as u32);
            }
        }
        Some(graph(&days, now.date_naive()))
    };
    rows.extend(failing);
    rows.extend(others);
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge, rows, graph, busy })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, replay};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn pull_requests_reviews_ci_and_the_graph_read_from_a_recorded_answer() {
        let fixtures = fixture("github");
        let (value, limit) = replay(Kind::Github, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), at("2026-11-16T15:00:00+01:00"), None).unwrap();
        assert_eq!(limit.remaining, Some(4987));
        assert_eq!(snapshot.title, "1 review requested");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        let ids: Vec<&str> = snapshot.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["review-acme/api#88", "pr-sagar/buntini#14", "pr-sagar/buntini#12", "pr-acme/web#7"]);
        let review = &snapshot.rows[0];
        assert_eq!(review.state, Some(PillState::Waiting));
        assert_eq!(review.alert.as_deref(), Some("Review requested"));
        assert_eq!(review.detail.as_deref(), Some("acme/api#88 · ana · 13:40"));
        let failing = &snapshot.rows[1];
        assert_eq!((failing.state, failing.alert.as_deref()), (Some(PillState::Error), Some("CI failed")));
        assert_eq!(failing.url.as_deref(), Some("https://github.com/sagar/buntini/pull/14"));
        assert_eq!(snapshot.rows[2].state, Some(PillState::Working));
        assert_eq!(snapshot.rows[2].detail.as_deref(), Some("sagar/buntini#12 · Draft · CI running · Sun 18:05"));
        assert_eq!(snapshot.rows[3].state, Some(PillState::Done));
        assert!(snapshot.busy);
        assert_eq!(snapshot.pill_state(), PillState::Waiting);
        let graph = snapshot.graph.unwrap();
        assert_eq!(graph.last(), Some(&3));
        assert_eq!(graph.iter().sum::<u32>(), 3 + 5 + 1);
    }

    #[tokio::test]
    async fn without_reviews_a_failure_names_the_title_and_the_graph_is_kept() {
        let fixtures = fixture("github");
        let (value, _) = replay(Kind::Github, &fixtures["quiet"], CAP).await;
        let snapshot = parse(&value.unwrap(), at("2026-11-16T15:00:00+01:00"), Some(vec![1, 2])).unwrap();
        assert_eq!(snapshot.title, "CI failed on Speed up the index");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        assert_eq!(snapshot.graph, Some(vec![1, 2]));
        assert!(!snapshot.busy);
        let (value, _) = replay(Kind::Github, &fixtures["empty"], CAP).await;
        let snapshot = parse(&value.unwrap(), at("2026-11-16T15:00:00+01:00"), None).unwrap();
        assert_eq!(snapshot.title, "0 open pull requests");
        assert_eq!(snapshot.badge, None);
        assert!(snapshot.rows.is_empty());
    }

    #[tokio::test]
    async fn graphql_refusals_wait_stop_or_report() {
        let fixtures = fixture("github");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, _) = replay(Kind::Github, &fixtures["rate_limited"], CAP).await;
        assert!(matches!(parse(&value.unwrap(), now, None), Err(Failure::Limited { .. })));
        let (value, _) = replay(Kind::Github, &fixtures["forbidden"], CAP).await;
        assert!(matches!(parse(&value.unwrap(), now, None), Err(Failure::Rejected(_))));
        let (value, _) = replay(Kind::Github, &fixtures["broken"], CAP).await;
        assert!(matches!(parse(&value.unwrap(), now, None), Err(Failure::Other(_))));
        assert!(matches!(parse(&json!({"data": {"viewer": null}}), now, None), Err(Failure::Other(_))));
        assert!(matches!(parse(&json!([]), now, None), Err(Failure::Other(_))));
        let (value, _) = replay(Kind::Github, &fixtures["secondary_limit"], CAP).await;
        assert!(matches!(value, Err(Failure::Limited { until, .. }) if until == crate::services::recorded::NOW + 60));
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused() {
        assert_capped(Kind::Github, CAP).await;
    }
}
