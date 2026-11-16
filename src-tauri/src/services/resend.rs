//! Resend: mail sent lately and what became of it (delivered, bounced), from api.resend.com.
use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, is_today, plural, short, text, time, when, Row, Snapshot};
use super::Kind;
use crate::surface::PillState;

pub const HOST: &str = "api.resend.com";
pub const CAP: usize = 512 * 1024;

pub async fn poll(net: &Net, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let value = net.get(&["emails"], &[("limit", "20".into())], CAP).await?;
    parse(&value, now)
}

fn state(event: Option<&str>) -> (PillState, &'static str) {
    match event {
        Some("delivered") => (PillState::Done, "Delivered"),
        Some("opened") => (PillState::Done, "Opened"),
        Some("clicked") => (PillState::Done, "Clicked"),
        Some("bounced") => (PillState::Error, "Bounced"),
        Some("complained") => (PillState::Error, "Marked as spam"),
        Some("failed") => (PillState::Error, "Failed"),
        Some("delivery_delayed") => (PillState::Working, "Delayed"),
        Some("sent") => (PillState::Working, "Sent"),
        Some("queued") => (PillState::Working, "Queued"),
        Some("scheduled") => (PillState::Idle, "Scheduled"),
        Some("canceled") => (PillState::Idle, "Canceled"),
        _ => (PillState::Idle, "Sent"),
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let mails = value.pointer("/data").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::Resend))?;
    let mut rows = vec![];
    let (mut sent_today, mut bounced_today) = (0, 0);
    for mail in mails.iter().filter(|mail| mail.get("id").is_some()) {
        let (pill, label) = state(text(mail, "/last_event").as_deref());
        let created = time(mail, "/created_at");
        if created.is_some_and(|at| is_today(at, now)) {
            sent_today += 1;
            if pill == PillState::Error {
                bounced_today += 1;
            }
        }
        let to = text(mail, "/to/0").or_else(|| text(mail, "/to"));
        rows.push(
            Row::new(text(mail, "/id").unwrap_or_default(), text(mail, "/subject").unwrap_or_else(|| "(no subject)".into()))
                .detail([to, Some(label.into()), created.map(|at| when(at, now))])
                .state(Some(pill))
                .url(text(mail, "/id").map(|id| format!("https://resend.com/emails/{id}"))),
        );
    }
    let title = if bounced_today > 0 {
        format!("{} today", plural(bounced_today, "mail bounced", "mails bounced"))
    } else if sent_today > 0 {
        format!("{} today", plural(sent_today, "mail sent", "mails sent"))
    } else {
        "No mail sent today".into()
    };
    let badge = if bounced_today > 0 { Some(short(bounced_today)) } else { (sent_today > 0).then(|| short(sent_today)) };
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge, rows, graph: None, busy: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, refusals, replay, NOW};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn sent_and_bounced_mail_read_from_a_recorded_answer() {
        let fixtures = fixture("resend");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, limit) = replay(Kind::Resend, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now).unwrap();
        assert_eq!(limit.remaining, Some(1));
        assert_eq!(limit.wait_until(NOW), Some(NOW + 1));
        assert_eq!(snapshot.title, "1 mail bounced today");
        assert_eq!(snapshot.badge.as_deref(), Some("1"));
        let states: Vec<Option<PillState>> = snapshot.rows.iter().map(|row| row.state).collect();
        assert_eq!(states, [Some(PillState::Done), Some(PillState::Error), Some(PillState::Working), Some(PillState::Done)]);
        assert_eq!(snapshot.rows[1].detail.as_deref(), Some("old@example.com · Bounced · 12:30"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://resend.com/emails/4ef9a417"));
        assert!(snapshot.rows.iter().all(|row| row.alert.is_none()));
    }

    #[tokio::test]
    async fn refusals_quiet_days_and_odd_answers() {
        let now = at("2026-11-16T15:00:00+01:00");
        // Recorded from api.resend.com: an unknown key is answered with 400.
        let (value, _) = replay(Kind::Resend, &refusals()["resend_badkey"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        let fixtures = fixture("resend");
        let (value, _) = replay(Kind::Resend, &fixtures["sending_only"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        assert_eq!(parse(&serde_json::json!({"object": "list", "data": []}), now).unwrap().title, "No mail sent today");
        assert!(parse(&serde_json::json!({"name": "x"}), now).is_err());
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused() {
        assert_capped(Kind::Resend, CAP).await;
    }
}
