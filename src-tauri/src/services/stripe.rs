//! Stripe: today's payments and their total, from api.stripe.com with a restricted key that can
//! only read charges.
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use super::fetch::{Failure, Net};
use super::snapshot::{self, flag, money, number, start_of_today, text, time, when, Row, Snapshot};
use super::Kind;
use crate::surface::PillState;

pub const HOST: &str = "api.stripe.com";
pub const CAP: usize = 1024 * 1024;

pub async fn poll(net: &Net, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let query = [("limit", "100".to_string()), ("created[gte]", start_of_today(now).to_string())];
    let value = net.get(&["v1", "charges"], &query, CAP).await?;
    parse(&value, now)
}

fn state(charge: &Value) -> (PillState, &'static str) {
    if flag(charge, "/refunded") {
        return (PillState::Idle, "Refunded");
    }
    match text(charge, "/status").as_deref() {
        Some("succeeded") => (PillState::Done, "Paid"),
        Some("pending") => (PillState::Working, "Pending"),
        Some("failed") => (PillState::Error, "Failed"),
        _ => (PillState::Idle, "Unknown"),
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    if let Some(message) = text(value, "/error/message") {
        return Err(Failure::Other(format!("Stripe: {}", snapshot::clip(&message, 160))));
    }
    let charges = value.pointer("/data").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::Stripe))?;
    // Totals per currency: what was paid today, less what was refunded.
    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    let mut rows = vec![];
    for charge in charges.iter().filter(|charge| charge.get("id").is_some()) {
        let currency = text(charge, "/currency").unwrap_or_else(|| "usd".into()).to_ascii_lowercase();
        let amount = number(charge, "/amount").unwrap_or(0);
        let created = time(charge, "/created");
        if created.is_some_and(|at| snapshot::is_today(at, now)) {
            if text(charge, "/status").as_deref() == Some("succeeded") {
                *totals.entry(currency.clone()).or_default() += amount - number(charge, "/amount_refunded").unwrap_or(0);
            }
        }
        let (pill, label) = state(charge);
        let id = text(charge, "/payment_intent").or_else(|| text(charge, "/id")).unwrap_or_default();
        let mode = if flag(charge, "/livemode") { "" } else { "test/" };
        let who = text(charge, "/billing_details/name").or_else(|| text(charge, "/description"));
        rows.push(
            Row::new(text(charge, "/id").unwrap_or_default(), money(amount, &currency))
                .detail([who, Some(label.into()), created.map(|at| when(at, now))])
                .state(Some(pill))
                .url(Some(format!("https://dashboard.stripe.com/{mode}payments/{id}"))),
        );
    }
    let more = if flag(value, "/has_more") { "+" } else { "" };
    let mut ranked: Vec<(String, i64)> = totals.into_iter().collect();
    ranked.sort_by_key(|(_, total)| -total);
    let title = if ranked.is_empty() {
        "No payments today".to_string()
    } else {
        let sums: Vec<String> = ranked.iter().map(|(currency, total)| money(*total, currency)).collect();
        format!("{}{more} today", sums.join(" · "))
    };
    let badge = ranked.first().map(|(currency, total)| format!("{}{more}", money(*total, currency)));
    let busy = rows.iter().any(|row| row.state == Some(PillState::Working));
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge, rows, graph: None, busy })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, refusals, replay};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn todays_payments_and_total_read_from_a_recorded_answer() {
        let fixtures = fixture("stripe");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, _) = replay(Kind::Stripe, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now).unwrap();
        // €49 + €120 − €20 refunded; the failed one doesn't count; the dollar one stands apart.
        assert_eq!(snapshot.title, "€149.00 · $15.00 today");
        assert_eq!(snapshot.badge.as_deref(), Some("€149.00"));
        assert_eq!(snapshot.rows.len(), 5);
        assert_eq!(snapshot.rows[0].title, "€49.00");
        assert_eq!(snapshot.rows[0].detail.as_deref(), Some("Ana Silva · Paid · 14:02"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://dashboard.stripe.com/payments/pi_3Abc"));
        assert_eq!(snapshot.rows[2].state, Some(PillState::Error));
        assert_eq!(snapshot.rows[3].url.as_deref(), Some("https://dashboard.stripe.com/test/payments/ch_4"));
        assert_eq!(snapshot.rows[4].state, Some(PillState::Idle));
        assert!(snapshot.rows.iter().all(|row| row.alert.is_none()));
    }

    #[tokio::test]
    async fn a_quiet_day_more_pages_and_errors() {
        let fixtures = fixture("stripe");
        let now = at("2026-11-16T15:00:00+01:00");
        let snapshot = parse(&serde_json::json!({"object": "list", "data": [], "has_more": false}), now).unwrap();
        assert_eq!(snapshot.title, "No payments today");
        assert_eq!(snapshot.badge, None);
        let (value, _) = replay(Kind::Stripe, &fixtures["more"], CAP).await;
        assert_eq!(parse(&value.unwrap(), now).unwrap().title, "¥5,000+ today");
        let (value, _) = replay(Kind::Stripe, &fixtures["missing_permission"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        let (value, _) = replay(Kind::Stripe, &refusals()["stripe_badkey"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        let (value, limit) = replay(Kind::Stripe, &fixtures["rate_limited"], CAP).await;
        assert!(matches!(value, Err(Failure::Limited { until: 0, .. })));
        assert_eq!(limit, Default::default());
        assert!(matches!(parse(&serde_json::json!({"error": {"message": "Odd"}}), now), Err(Failure::Other(_))));
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused() {
        assert_capped(Kind::Stripe, CAP).await;
    }
}
