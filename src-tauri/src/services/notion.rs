//! Notion: pages changed lately, from a read-only search on api.notion.com. The integration sees
//! only the pages the person shared with it.
use chrono::{DateTime, FixedOffset};
use serde_json::{json, Value};

use super::fetch::{Failure, Net};
use super::snapshot::{self, flag, is_today, list, short, text, time, when, Row, Snapshot};
use super::Kind;
use crate::i18n::{t, tn};

pub const HOST: &str = "api.notion.com";
pub const CAP: usize = 1024 * 1024;

pub async fn poll(net: &Net, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    let search = json!({
        "filter": { "property": "object", "value": "page" },
        "sort": { "direction": "descending", "timestamp": "last_edited_time" },
        "page_size": 10
    });
    let value = net.post(&["v1", "search"], &search, CAP).await?;
    parse(&value, now)
}

/// A page's title is whichever property has the type `title`.
fn title(page: &Value) -> String {
    let properties = page.get("properties").and_then(Value::as_object);
    let title = properties
        .into_iter()
        .flatten()
        .find(|(_, property)| text(property, "/type").as_deref() == Some("title"))
        .map(|(_, property)| list(property, "/title").iter().filter_map(|part| text(part, "/plain_text")).collect::<String>())
        .unwrap_or_default();
    if title.trim().is_empty() {
        t("services.notion.untitled")
    } else {
        title
    }
}

pub fn parse(value: &Value, now: DateTime<FixedOffset>) -> Result<Snapshot, Failure> {
    if text(value, "/object").as_deref() == Some("error") {
        return Err(Failure::Other(format!("Notion: {}", text(value, "/message").unwrap_or_default())));
    }
    let pages = value.pointer("/results").and_then(Value::as_array).ok_or_else(|| Failure::unreadable(Kind::Notion))?;
    let mut rows = vec![];
    let mut today = 0;
    for page in pages.iter().filter(|page| text(page, "/object").as_deref() == Some("page") && !flag(page, "/archived") && !flag(page, "/in_trash")) {
        let edited = time(page, "/last_edited_time");
        if edited.is_some_and(|at| is_today(at, now)) {
            today += 1;
        }
        rows.push(Row::new(text(page, "/id").unwrap_or_default(), title(page)).detail([edited.map(|at| when(at, now))]).url(text(page, "/url")));
    }
    let title = if today > 0 { tn("services.notion.pagesChangedToday", today as u64, &[]) } else { t("services.notion.noPages") };
    rows.truncate(snapshot::MAX_ROWS);
    Ok(Snapshot { title, badge: (today > 0).then(|| short(today)), rows, graph: None, busy: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recorded::{assert_capped, fixture, refusals, replay, NOW};
    use crate::services::snapshot::at;

    #[tokio::test]
    async fn pages_changed_read_from_a_recorded_answer() {
        let fixtures = fixture("notion");
        let now = at("2026-11-16T15:00:00+01:00");
        let (value, _) = replay(Kind::Notion, &fixtures["ok"], CAP).await;
        let snapshot = parse(&value.unwrap(), now).unwrap();
        assert_eq!(snapshot.title, "2 pages changed today");
        let titles: Vec<&str> = snapshot.rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, ["Roadmap", "Untitled", "Meeting notes"]);
        assert_eq!(snapshot.rows[0].detail.as_deref(), Some("14:12"));
        assert_eq!(snapshot.rows[0].url.as_deref(), Some("https://www.notion.so/Roadmap-1a2b3c"));
        assert!(snapshot.rows.iter().all(|row| row.state.is_none() && row.alert.is_none()));
    }

    #[tokio::test]
    async fn refusals_and_limits() {
        let (value, _) = replay(Kind::Notion, &refusals()["notion_badkey"], CAP).await;
        assert!(matches!(value, Err(Failure::Rejected(_))));
        let fixtures = fixture("notion");
        let (value, limit) = replay(Kind::Notion, &fixtures["rate_limited"], CAP).await;
        assert!(matches!(value, Err(Failure::Limited { .. })));
        assert_eq!(limit.retry_at, Some(NOW + 2));
        assert!(parse(&serde_json::json!({"object": "error", "message": "x"}), at("2026-11-16T15:00:00+01:00")).is_err());
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused() {
        assert_capped(Kind::Notion, CAP).await;
    }
}
