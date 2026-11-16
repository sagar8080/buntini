//! What the services put on the island: one tinted pill each, an Info card with what the
//! service shows, and an Alert card for a CI failure or review request that is new since the
//! last poll. Pure, so the rules are tested without windows.
use std::collections::BTreeSet;

use super::{Kind, Row, Snapshot};
use crate::i18n::{t, tf};
use crate::surface::cards::services::{ServiceCard, ServiceRow};
use crate::surface::{Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency, ERROR_MS};

/// How one service's card sits on the board between polls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shown {
    /// Board clock when what the card says last changed: news shows newest first.
    pub changed_ms: u64,
    /// Alert rows at the last poll; `None` before the first, so launching alerts nothing.
    seen: Option<BTreeSet<String>>,
    /// Alert rows that are new, and when they came.
    fresh: Vec<String>,
    fresh_ms: u64,
}

impl Shown {
    pub fn absorb(&mut self, before: Option<&Snapshot>, after: &Snapshot, now_ms: u64) {
        let alerts: BTreeSet<String> = after.rows.iter().filter(|row| row.alert.is_some()).map(|row| row.id.clone()).collect();
        if let Some(seen) = &self.seen {
            let fresh: Vec<String> = after.rows.iter().filter(|row| alerts.contains(&row.id) && !seen.contains(&row.id)).map(|row| row.id.clone()).collect();
            if !fresh.is_empty() {
                self.fresh = fresh;
                self.fresh_ms = now_ms;
            }
        }
        // A failure fixed or a review done stops alerting.
        self.fresh.retain(|id| alerts.contains(id));
        self.seen = Some(alerts);
        if before.map(fingerprint) != Some(fingerprint(after)) {
            self.changed_ms = now_ms;
        }
    }

    /// The person asked to see this service's card.
    pub fn bring_forward(&mut self, now_ms: u64) {
        self.changed_ms = now_ms;
    }
}

/// What counts as news: the title and each row's id, title and state, not times.
fn fingerprint(snapshot: &Snapshot) -> Vec<(String, String, Option<PillState>)> {
    std::iter::once((String::new(), snapshot.title.clone(), None)).chain(snapshot.rows.iter().map(|row| (row.id.clone(), row.title.clone(), row.state))).collect()
}

pub struct View<'a> {
    pub kind: Kind,
    pub snapshot: Option<&'a Snapshot>,
    /// The key was refused: the pill shows it, with no card.
    pub rejected: bool,
    pub shown: &'a Shown,
}

pub fn card_id(kind: Kind) -> String {
    format!("services.{}", kind.id())
}

/// The service a pill or card id belongs to.
pub fn kind_of(id: &str) -> Option<Kind> {
    id.strip_prefix("services.")?.split('.').next().and_then(Kind::from_id)
}

fn card(kind: Kind, id: String, title: String, rows: Vec<&Row>, graph: Option<Vec<u32>>) -> Card {
    let rows = rows
        .into_iter()
        .take(super::snapshot::MAX_ROWS)
        .map(|row| ServiceRow { id: row.id.clone(), title: row.title.clone(), detail: row.detail.clone(), state: row.state, url: row.url.clone() })
        .collect();
    Card { id, source: Source::Services, body: CardBody::Service(ServiceCard { service: kind.id().into(), title, rows, graph }) }
}

pub fn contribution(views: &[View]) -> Contribution {
    let mut contribution = Contribution::default();
    for view in views {
        let id = card_id(view.kind);
        let pill = |state, badge| Pill { id: id.clone(), source: Source::Services, label: view.kind.name().into(), tint: view.kind.tint().into(), state, badge, progress: None };
        let Some(snapshot) = view.snapshot else {
            if view.rejected {
                contribution.pills.push(pill(PillState::Error, None));
            }
            continue;
        };
        contribution.pills.push(pill(snapshot.pill_state(), snapshot.badge.clone()));
        contribution.offers.push(Offer {
            card: card(view.kind, id.clone(), snapshot.title.clone(), snapshot.rows.iter().collect(), snapshot.graph.clone()),
            urgency: Urgency::Info,
            at_ms: view.shown.changed_ms,
            ttl_ms: None,
        });
        let fresh: Vec<&Row> = view.shown.fresh.iter().filter_map(|fresh| snapshot.rows.iter().find(|row| &row.id == fresh)).collect();
        if let Some(first) = fresh.first() {
            let alert = first.alert.clone().unwrap_or_else(|| t("services.alert.new"));
            let title = match fresh.len() {
                1 => tf("services.alert.title", &[("alert", &alert), ("title", &first.title)]),
                count => tf("services.alert.titleMore", &[("alert", &alert), ("title", &first.title), ("more", &(count - 1))]),
            };
            let rows: Vec<&Row> = fresh.iter().copied().chain(snapshot.rows.iter().filter(|row| !view.shown.fresh.contains(&row.id))).collect();
            contribution.offers.push(Offer {
                card: card(view.kind, format!("{id}.alert"), title, rows, None),
                urgency: Urgency::Alert,
                at_ms: view.shown.fresh_ms,
                ttl_ms: Some(ERROR_MS),
            });
        }
    }
    contribution
}

/// Keeps only links a card may open: HTTPS, on the service's own web host, no user or odd port.
pub fn safe_links(kind: Kind, host: &str, mut snapshot: Snapshot) -> Snapshot {
    // A self-hosted service links to the host it was given; the rest to their known sites.
    let own = kind.api_host().is_none().then_some(host);
    for row in &mut snapshot.rows {
        row.url = row.url.take().filter(|url| allowed(kind, own, url));
    }
    snapshot
}

fn allowed(kind: Kind, own: Option<&str>, raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else { return false };
    if raw.len() > 2048 || url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(name) = url.host_str() else { return false };
    let at = match url.port() {
        Some(port) => format!("{name}:{port}"),
        None => name.to_string(),
    };
    if let Some(own) = own {
        return at == own;
    }
    url.port().is_none() && kind.web_hosts().iter().any(|host| if host.starts_with('.') { name.ends_with(host) } else { name == *host })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, state: PillState, alert: Option<&'static str>) -> Row {
        let mut row = Row::new(id, format!("Title {id}")).state(Some(state));
        row.alert = alert.map(String::from);
        row
    }

    fn snapshot(rows: Vec<Row>) -> Snapshot {
        Snapshot { title: "t".into(), badge: Some("2".into()), rows, graph: None, busy: false }
    }

    fn offers(shown: &Shown, snapshot: &Snapshot) -> Vec<(String, Urgency, u64, Option<u64>)> {
        let view = View { kind: Kind::Github, snapshot: Some(snapshot), rejected: false, shown };
        contribution(&[view]).offers.into_iter().map(|offer| (offer.card.id, offer.urgency, offer.at_ms, offer.ttl_ms)).collect()
    }

    #[test]
    fn the_first_poll_alerts_nothing_and_a_new_failure_alerts_once() {
        let mut shown = Shown::default();
        let first = snapshot(vec![row("review-a", PillState::Waiting, Some("Review requested"))]);
        shown.absorb(None, &first, 1_000);
        assert_eq!(offers(&shown, &first), vec![("services.github".into(), Urgency::Info, 1_000, None)]);

        let second = snapshot(vec![row("review-a", PillState::Waiting, Some("Review requested")), row("pr-b", PillState::Error, Some("CI failed"))]);
        shown.absorb(Some(&first), &second, 5_000);
        let all = offers(&shown, &second);
        assert_eq!(all[1], ("services.github.alert".into(), Urgency::Alert, 5_000, Some(ERROR_MS)));
        let view = View { kind: Kind::Github, snapshot: Some(&second), rejected: false, shown: &shown };
        let alert = contribution(&[view]).offers.remove(1).card;
        let CardBody::Service(card) = alert.body else { panic!() };
        assert_eq!(card.title, "CI failed: Title pr-b");
        assert_eq!(card.rows[0].id, "pr-b");

        // The same failure at the next poll doesn't alert again: same moment, long expired.
        shown.absorb(Some(&second), &second, 200_000);
        assert_eq!(offers(&shown, &second)[1].2, 5_000);
        // Fixed: no alert left.
        let fixed = snapshot(vec![row("review-a", PillState::Waiting, Some("Review requested")), row("pr-b", PillState::Done, None)]);
        shown.absorb(Some(&second), &fixed, 300_000);
        assert_eq!(offers(&shown, &fixed).len(), 1);
        // Failing again later alerts again.
        shown.absorb(Some(&fixed), &second, 400_000);
        assert_eq!(offers(&shown, &second)[1].2, 400_000);
    }

    #[test]
    fn news_moves_the_card_forward_but_a_quiet_poll_does_not() {
        let mut shown = Shown::default();
        let first = snapshot(vec![row("a", PillState::Done, None)]);
        shown.absorb(None, &first, 1_000);
        let mut same = first.clone();
        same.rows[0].detail = Some("14:03".into());
        shown.absorb(Some(&first), &same, 2_000);
        assert_eq!(shown.changed_ms, 1_000);
        let changed = snapshot(vec![row("a", PillState::Working, None)]);
        shown.absorb(Some(&same), &changed, 3_000);
        assert_eq!(shown.changed_ms, 3_000);
        shown.bring_forward(9_000);
        assert_eq!(shown.changed_ms, 9_000);
    }

    #[test]
    fn each_service_has_a_tinted_pill_and_a_refused_key_shows_only_a_pill() {
        let shown = Shown::default();
        let ok = snapshot(vec![row("a", PillState::Error, Some("CI failed"))]);
        let views = [
            View { kind: Kind::Github, snapshot: Some(&ok), rejected: false, shown: &shown },
            View { kind: Kind::Stripe, snapshot: None, rejected: true, shown: &shown },
            View { kind: Kind::Notion, snapshot: None, rejected: false, shown: &shown },
        ];
        let contribution = contribution(&views);
        assert_eq!(contribution.pills.len(), 2);
        let github = &contribution.pills[0];
        assert_eq!((github.id.as_str(), github.label.as_str(), github.tint.as_str(), github.state), ("services.github", "GitHub", Kind::Github.tint(), PillState::Error));
        assert_eq!(github.badge.as_deref(), Some("2"));
        assert_eq!(contribution.pills[1].state, PillState::Error);
        assert_eq!(contribution.offers.len(), 1);
        assert_eq!(kind_of("services.github.alert"), Some(Kind::Github));
        assert_eq!(kind_of("services.calcom"), Some(Kind::Calcom));
        assert_eq!(kind_of("agents.x"), None);
    }

    /// The whole way from a recorded GitHub answer to the scene Swift draws. Set
    /// SERVICES_SCENE_OUT to a path to write it as a SurfaceLab fixture.
    #[tokio::test]
    async fn a_recorded_answer_reaches_the_island_as_a_scene() {
        use crate::services::recorded::{fixture, replay};
        use crate::surface::{board::Board, Cat, Home, Placement, Scene, CONTRACT};
        let (value, _) = replay(Kind::Github, &fixture("github")["ok"], super::super::github::CAP).await;
        let parsed = super::super::github::parse(&value.unwrap(), crate::services::snapshot::at("2026-11-16T15:00:00+01:00"), None).unwrap();
        let snapshot = safe_links(Kind::Github, "api.github.com", parsed);
        let mut shown = Shown::default();
        shown.absorb(None, &snapshot, 0);
        let mut board = Board::default();
        board.set(Source::Services, contribution(&[View { kind: Kind::Github, snapshot: Some(&snapshot), rejected: false, shown: &shown }]));
        let picked = board.pick(0);
        let scene = Scene {
            contract: CONTRACT,
            home: Home::Island,
            placement: Placement::Auto,
            pills: picked.pills,
            overflow: picked.overflow,
            card: picked.card,
            held: picked.held,
            interrupt: picked.interrupt,
            cat: Cat::default(),
            flow: None,
            overlay: None,
            sound: None,
            line: None,
            words: None,
        };
        assert!(!scene.held);
        assert_eq!(scene.pills[0].state, PillState::Waiting);
        let json = serde_json::to_value(&scene).unwrap();
        assert_eq!(json["card"]["body"]["kind"], "service");
        assert_eq!(json["card"]["body"]["graph"].as_array().unwrap().len(), 7 * 52 + 2);
        assert_eq!(serde_json::from_value::<Scene>(json.clone()).unwrap(), scene);
        if let Ok(path) = std::env::var("SERVICES_SCENE_OUT") {
            std::fs::write(path, serde_json::json!({ "scenes": [json], "actions": [] }).to_string()).unwrap();
        }
    }

    #[test]
    fn cards_open_only_links_to_the_services_own_sites() {
        let links = |kind, host: &str, urls: &[&str]| -> Vec<bool> {
            let rows = urls.iter().map(|url| Row::new("r", "r").url(Some(url.to_string()))).collect();
            safe_links(kind, host, snapshot(rows)).rows.iter().map(|row| row.url.is_some()).collect()
        };
        assert_eq!(
            links(Kind::Github, "api.github.com", &["https://github.com/a/b/pull/1", "http://github.com/a", "https://github.com.evil.example/", "https://evil.example/", "https://user@github.com/", "https://github.com:8443/", "javascript:alert(1)"]),
            [true, false, false, false, false, false, false]
        );
        assert_eq!(links(Kind::Vercel, "api.vercel.com", &["https://vercel.com/team/app/abc", "https://app-abc.vercel.app", "https://vercel.app.evil.example"]), [true, true, false]);
        assert_eq!(links(Kind::Gitea, "git.example.com:3000", &["https://git.example.com:3000/me/repo/pulls/4", "https://git.example.com/me", "https://github.com/x"]), [true, false, false]);
        assert_eq!(links(Kind::Gitlab, "gitlab.com", &["https://gitlab.com/g/p/-/merge_requests/3", "https://gitlab.example.com/"]), [true, false]);
        assert_eq!(links(Kind::Notion, "api.notion.com", &["https://www.notion.so/Roadmap-abc", "https://team.notion.site/x"]), [true, true]);
    }
}
