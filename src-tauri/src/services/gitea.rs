//! Gitea (and Forgejo, which answers the same API) on the person's own host: your open pull
//! requests, reviews requested from you, CI on your newest three, and your activity heatmap.
use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, flag, graph, list, number, plural, short, text, time, when, Row, Snapshot};
use super::Memo;
use crate::surface::PillState;

pub const CAP: usize = 512 * 1024;
const SMALL_CAP: usize = 64 * 1024;
const PULL_CAP: usize = 256 * 1024;
const HEATMAP_CAP: usize = 256 * 1024;
const GRAPH_EVERY: i64 = 6 * 3600;
const USER_EVERY: i64 = 6 * 3600;
/// Pull requests whose CI is looked up each poll: two requests each at most.
const CI_LOOKUPS: usize = 3;

fn search(kind: &str) -> Vec<(&'static str, String)> {
    vec![("type", "pulls".into()), ("state", "open".into()), (if kind == "mine" { "created" } else { "review_requested" }, "true".into()), ("limit", "15".into())]
}

pub async fn poll(net: &Net, memo: &mut Memo, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let stamp = now.timestamp();
    if memo.user.is_none() || memo.due("user", USER_EVERY, stamp) {
        let user = net.get(&["api", "v1", "user"], &[], SMALL_CAP).await?;
        memo.user = text(&user, "/login");
        memo.mark("user", stamp);
    }
    let mine = net.get(&["api", "v1", "repos", "issues", "search"], &search("mine"), CAP).await?;
    let reviews = net.get(&["api", "v1", "repos", "issues", "search"], &search("reviews"), CAP).await?;

    let mut ci = HashMap::new();
    let mut current = vec![];
    for pr in list(&mine, "").iter().take(CI_LOOKUPS) {
        let (Some(owner), Some(repo), Some(index), Some(updated)) = (text(pr, "/repository/owner"), text(pr, "/repository/name"), text(pr, "/number"), text(pr, "/updated_at")) else { continue };
        let key = format!("{owner}/{repo}#{index}");
        current.push(key.clone());
        // The head commit changes only when the pull request does.
        let sha = match memo.heads.get(&key) {
            Some((at, sha)) if *at == updated => sha.clone(),
            _ => {
                let pull = net.get(&["api", "v1", "repos", &owner, &repo, "pulls", &index], &[], PULL_CAP).await?;
                let Some(sha) = text(&pull, "/head/sha") else { continue };
                memo.heads.insert(key.clone(), (updated, sha.clone()));
                sha
            }
        };
        let status = net.get(&["api", "v1", "repos", &owner, &repo, "commits", &sha, "status"], &[], PULL_CAP).await?;
        if number(&status, "/total_count").unwrap_or(1) > 0 {
            if let Some(state) = text(&status, "/state") {
                ci.insert(key, state);
            }
        }
    }
    memo.heads.retain(|key, _| current.contains(key));

    if let Some(user) = memo.user.clone().filter(|_| memo.due("graph", GRAPH_EVERY, stamp)) {
        match net.get(&["api", "v1", "users", &user, "heatmap"], &[], HEATMAP_CAP).await {
            Ok(heatmap) => memo.graph = Some(heatmap_graph(&heatmap, now)),
            Err(failure @ (Failure::Cancelled | Failure::Budget(_) | Failure::Limited { .. })) => return Err(failure),
            Err(_) => {}
        }
        memo.mark("graph", stamp);
    }
    Ok(parse(&mine, &reviews, &ci, now, memo.graph.clone()))
}

/// `[{"timestamp": …, "contributions": n}]`, summed per day in the Mac's time zone.
pub fn heatmap_graph(value: &Value, now: DateTime<FixedOffset>) -> Vec<u32> {
    let mut days: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    for entry in list(value, "") {
        if let (Some(at), Some(count)) = (time(entry, "/timestamp"), number(entry, "/contributions")) {
            let day = at.with_timezone(now.offset()).date_naive();
            *days.entry(day).or_default() += count.clamp(0, 10_000) as u32;
        }
    }
    graph(&days, now.date_naive())
}

fn ci_state(state: Option<&str>) -> (PillState, Option<&'static str>) {
    match state {
        Some("failure" | "error") => (PillState::Error, Some("CI failed")),
        Some("pending") => (PillState::Working, Some("CI running")),
        Some("success" | "warning") => (PillState::Done, Some("CI passed")),
        _ => (PillState::Idle, None),
    }
}

pub fn parse(mine: &Value, reviews: &Value, ci: &HashMap<String, String>, now: DateTime<FixedOffset>, graph: Option<Vec<u32>>) -> Snapshot {
    let key = |pr: &Value| format!("{}#{}", text(pr, "/repository/full_name").unwrap_or_default(), text(pr, "/number").unwrap_or_default());
    let updated = |pr: &Value| time(pr, "/updated_at").map(|at| when(at, now));
    let pulls = |value: &Value| -> Vec<Value> { list(value, "").iter().filter(|pr| pr.get("number").is_some()).cloned().collect() };

    let reviews = pulls(reviews);
    let mut rows: Vec<Row> = reviews
        .iter()
        .map(|pr| {
            Row::new(format!("review-{}", key(pr)), text(pr, "/title").unwrap_or_default())
                .detail([Some(key(pr)), text(pr, "/user/login"), updated(pr)])
                .state(Some(PillState::Waiting))
                .url(text(pr, "/html_url"))
                .alert("Review requested", true)
        })
        .collect();
    let mine = pulls(mine);
    let mut failing = vec![];
    let mut others = vec![];
    let mut busy = false;
    for pr in &mine {
        let (state, label) = ci_state(ci.get(&key(pr)).map(String::as_str));
        busy |= state == PillState::Working;
        let row = Row::new(format!("pr-{}", key(pr)), text(pr, "/title").unwrap_or_default())
            .detail([Some(key(pr)), flag(pr, "/pull_request/draft").then(|| "Draft".into()), label.map(str::to_string), updated(pr)])
            .state(Some(state))
            .url(text(pr, "/html_url"))
            .alert("CI failed", state == PillState::Error);
        if state == PillState::Error {
            failing.push(row);
        } else {
            others.push(row);
        }
    }
    let title = if !reviews.is_empty() {
        plural(reviews.len(), "review requested", "reviews requested")
    } else if let Some(first) = failing.first() {
        format!("CI failed on {}", first.title)
    } else {
        plural(mine.len(), "open pull request", "open pull requests")
    };
    let badge = if !reviews.is_empty() { Some(short(reviews.len())) } else { (!mine.is_empty()).then(|| short(mine.len())) };
    rows.extend(failing);
    rows.extend(others);
    rows.truncate(snapshot::MAX_ROWS);
    Snapshot { title, badge, rows, graph, busy }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, replay};
    use crate::services::snapshot::at;
    use crate::services::Kind;

    #[tokio::test]
    async fn pull_requests_reviews_and_ci_read_from_recorded_answers() {
        let fixtures = fixture("gitea");
        let now = at("2026-11-16T15:00:00+01:00");
        let (mine, _) = replay(Kind::Gitea, &fixtures["mine"], CAP).await;
        let (reviews, _) = replay(Kind::Gitea, &fixtures["reviews"], CAP).await;
        let (pull, _) = replay(Kind::Gitea, &fixtures["pull"], PULL_CAP).await;
        let (status, _) = replay(Kind::Gitea, &fixtures["status"], PULL_CAP).await;
        assert_eq!(text(&pull.unwrap(), "/head/sha").as_deref(), Some("4f1c2e9"));
        let mut ci = HashMap::new();
        ci.insert("sagar/buntini#5".to_string(), text(&status.unwrap(), "/state").unwrap());
        ci.insert("sagar/buntini#6".to_string(), "pending".to_string());
        let snapshot = parse(&mine.unwrap(), &reviews.unwrap(), &ci, now, None);
        assert_eq!(snapshot.title, "1 review requested");
        let ids: Vec<&str> = snapshot.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["review-team/site#40", "pr-sagar/buntini#5", "pr-sagar/buntini#6"]);
        assert_eq!(snapshot.rows[1].alert, Some("CI failed"));
        assert_eq!(snapshot.rows[2].detail.as_deref(), Some("sagar/buntini#6 · Draft · CI running · 08:30"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://git.example.com/team/site/pulls/40"));
        assert!(snapshot.busy);
    }

    #[tokio::test]
    async fn the_heatmap_becomes_the_graph_in_local_days() {
        let fixtures = fixture("gitea");
        let (value, limit) = replay(Kind::Gitea, &fixtures["heatmap"], HEATMAP_CAP).await;
        // 2026-11-15 23:30 UTC is already the 16th at UTC+1.
        let cells = heatmap_graph(&value.unwrap(), at("2026-11-16T15:00:00+01:00"));
        assert_eq!(cells.last(), Some(&5));
        assert_eq!(cells.iter().sum::<u32>(), 5 + 1);
        assert_eq!(limit.remaining, Some(1990));
    }

    #[tokio::test]
    async fn nothing_open_reads_as_none() {
        let now = at("2026-11-16T15:00:00+01:00");
        let snapshot = parse(&serde_json::json!([]), &serde_json::json!({"message": "odd"}), &HashMap::new(), now, None);
        assert_eq!(snapshot.title, "0 open pull requests");
        assert!(snapshot.rows.is_empty() && snapshot.badge.is_none());
    }

    #[tokio::test]
    async fn answers_over_the_cap_are_refused() {
        assert_capped(Kind::Gitea, CAP).await;
        assert_capped(Kind::Gitea, PULL_CAP).await;
    }
}
