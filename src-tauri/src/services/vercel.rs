//! Vercel: your latest deployments and how they went, from api.vercel.com.
use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, short, text, time, when, Row, Snapshot};
use super::{Kind, Memo};
use crate::surface::PillState;

pub const HOST: &str = "api.vercel.com";
pub const CAP: usize = 512 * 1024;
const USER_CAP: usize = 64 * 1024;
const USER_EVERY: i64 = 6 * 3600;

pub async fn poll(net: &Net, memo: &mut Memo, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let stamp = now.timestamp();
    if memo.due("user", USER_EVERY, stamp) {
        // Accounts live in a default team; deployments are listed under it.
        let user = net.get(&["v2", "user"], &[], USER_CAP).await?;
        memo.user = text(&user, "/user/defaultTeamId");
        memo.mark("user", stamp);
    }
    let mut query = vec![("limit", "8".to_string())];
    if let Some(team) = &memo.user {
        query.push(("teamId", team.clone()));
    }
    let value = net.get(&["v6", "deployments"], &query, CAP).await?;
    parse(&value, now)
}

fn state(deployment: &Value) -> (PillState, &'static str) {
    match text(deployment, "/state").or_else(|| text(deployment, "/readyState")).as_deref() {
        Some("READY") => (PillState::Done, "Ready"),
        Some("ERROR") => (PillState::Error, "Failed"),
        Some("BUILDING") => (PillState::Working, "Building"),
        Some("INITIALIZING" | "QUEUED") => (PillState::Working, "Queued"),
        Some("CANCELED") => (PillState::Idle, "Canceled"),
        _ => (PillState::Idle, "Unknown"),
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let deployments = value.pointer("/deployments").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::Vercel))?;
    let mut rows = vec![];
    let mut building = 0;
    for deployment in deployments.iter().filter(|deployment| deployment.get("uid").is_some()) {
        let (pill, label) = state(deployment);
        if pill == PillState::Working {
            building += 1;
        }
        let target = match text(deployment, "/target").as_deref() {
            Some("production") => "Production",
            _ => "Preview",
        };
        let message = text(deployment, "/meta/githubCommitMessage").or_else(|| text(deployment, "/meta/gitlabCommitMessage")).map(|message| snapshot::clip(message.lines().next().unwrap_or_default(), 60));
        let url = text(deployment, "/inspectorUrl").or_else(|| text(deployment, "/url").map(|host| format!("https://{host}")));
        rows.push(
            Row::new(text(deployment, "/uid").unwrap_or_default(), text(deployment, "/name").unwrap_or_else(|| "Deployment".into()))
                .detail([Some(target.into()), Some(label.into()), message, time(deployment, "/created").map(|at| when(at, now))])
                .state(Some(pill))
                .url(url)
                .alert("Deploy failed", pill == PillState::Error),
        );
    }
    let title = match rows.first() {
        None => "No deployments yet".into(),
        Some(latest) => match latest.state {
            Some(PillState::Working) => format!("Deploying {}", latest.title),
            Some(PillState::Error) => format!("{} failed to deploy", latest.title),
            Some(PillState::Done) => format!("{} is live", latest.title),
            _ => latest.title.clone(),
        },
    };
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge: (building > 0).then(|| short(building)), rows, graph: None, busy: building > 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, replay, NOW};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn deployments_read_from_a_recorded_answer() {
        let fixtures = fixture("vercel");
        let now = at("2026-11-16T15:00:00+01:00");
        let (user, _) = replay(Kind::Vercel, &fixtures["user"], USER_CAP).await;
        assert_eq!(text(&user.unwrap(), "/user/defaultTeamId").as_deref(), Some("team_8x2Kq"));
        let (value, limit) = replay(Kind::Vercel, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now).unwrap();
        assert_eq!(limit.remaining, Some(97));
        assert_eq!(snapshot.title, "Deploying buntini-site");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        assert!(snapshot.busy);
        let states: Vec<Option<PillState>> = snapshot.rows.iter().map(|row| row.state).collect();
        assert_eq!(states, [Some(PillState::Working), Some(PillState::Error), Some(PillState::Done)]);
        assert_eq!(snapshot.rows[1].alert, Some("Deploy failed"));
        assert_eq!(snapshot.rows[1].detail.as_deref(), Some("Preview · Failed · Try the new hero · 13:50"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://vercel.com/sagar/buntini-site/9Gq1"));
        assert_eq!(snapshot.rows[2].url.as_deref(), Some("https://buntini-site-a1b2.vercel.app"));
    }

    #[tokio::test]
    async fn an_answer_without_deployments_is_unreadable_and_limits_wait() {
        let fixtures = fixture("vercel");
        let now = at("2026-11-16T15:00:00+01:00");
        assert!(matches!(parse(&serde_json::json!({"error": "x"}), now), Err(Failure::Other(_))));
        let snapshot = parse(&serde_json::json!({"deployments": []}), now).unwrap();
        assert_eq!(snapshot.title, "No deployments yet");
        let (value, limit) = replay(Kind::Vercel, &fixtures["rate_limited"], CAP).await;
        assert!(matches!(value, Err(Failure::Limited { .. })));
        assert_eq!(limit.wait_until(NOW), Some(NOW + 45));
    }

    #[tokio::test]
    async fn answers_over_the_cap_are_refused() {
        assert_capped(Kind::Vercel, CAP).await;
    }
}
