//! Cal.com: upcoming bookings, soonest first, from api.cal.com.
use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, is_today, short, text, time, when, Row, Snapshot};
use super::Kind;
use crate::i18n::t;
use crate::surface::PillState;

pub const HOST: &str = "api.cal.com";
pub const CAP: usize = 512 * 1024;

pub async fn poll(net: &Net, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let query = [("status", "upcoming".to_string()), ("take", "10".to_string()), ("sortStart", "asc".to_string())];
    let value = net.get(&["v2", "bookings"], &query, CAP).await?;
    parse(&value, now)
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    if text(value, "/status").as_deref() == Some("error") {
        return Err(Failure::Other(format!("Cal.com: {}", text(value, "/error/message").unwrap_or_default())));
    }
    let bookings = value.pointer("/data").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::Calcom))?;
    let mut rows = vec![];
    let mut today = 0;
    for booking in bookings {
        let status = text(booking, "/status").unwrap_or_default().to_ascii_lowercase();
        if ["cancelled", "canceled", "rejected"].contains(&status.as_str()) {
            continue;
        }
        let Some(start) = time(booking, "/start").or_else(|| time(booking, "/startTime")) else { continue };
        if is_today(start, now) {
            today += 1;
        }
        // A booking waiting to be confirmed waits on the person; no interruption, though.
        let state = if ["pending", "unconfirmed"].contains(&status.as_str()) { PillState::Waiting } else { PillState::Idle };
        let who = text(booking, "/attendees/0/name");
        let uid = text(booking, "/uid").unwrap_or_default();
        rows.push(
            Row::new(format!("booking-{uid}"), text(booking, "/title").unwrap_or_else(|| t("services.calcom.booking")))
                .detail([Some(when(start, now)), who, (state == PillState::Waiting).then(|| t("services.status.unconfirmed"))])
                .state(Some(state))
                .url((!uid.is_empty()).then(|| format!("https://app.cal.com/booking/{uid}"))),
        );
    }
    let title = match rows.first() {
        Some(next) => format!("{} · {}", next.title, next.detail.as_deref().and_then(|detail| detail.split(" · ").next()).unwrap_or_default()),
        None => t("services.calcom.noBookings"),
    };
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge: (today > 0).then(|| short(today)), rows, graph: None, busy: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, refusals, replay};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn upcoming_bookings_read_from_a_recorded_answer() {
        let fixtures = fixture("calcom");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, _) = replay(Kind::Calcom, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now).unwrap();
        assert_eq!(snapshot.title, "Intro call · 16:30");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.rows[0].detail.as_deref(), Some("16:30 · Ana Silva"));
        assert_eq!(snapshot.rows[1].state, Some(PillState::Waiting));
        assert_eq!(snapshot.rows[1].detail.as_deref(), Some("Wed 10:00 · Ben · Unconfirmed"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://app.cal.com/booking/bk_1"));
        assert_eq!(snapshot.pill_state(), PillState::Idle);
    }

    #[tokio::test]
    async fn refusals_and_errors() {
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, _) = replay(Kind::Calcom, &refusals()["calcom_badkey"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        assert!(parse(&serde_json::json!({"status": "error", "error": {"message": "x"}}), now).is_err());
        assert_eq!(parse(&serde_json::json!({"status": "success", "data": []}), now).unwrap().title, "No upcoming bookings");
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused() {
        assert_capped(Kind::Calcom, CAP).await;
    }
}
