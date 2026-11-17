//! Decides what the island shows from what every part of Buntini offers. Pure, so the rules
//! are tested without windows: a card stays at least `HOLD_MS` unless something more urgent
//! comes, waiting cards queue oldest first, other news shows newest first, and waiting pills
//! that don't fit are counted as "+N". The timings follow Coucou's SessionBoard (MIT).
use std::collections::BTreeMap;

use super::scene::{Card, Pill, PillState, Source};

/// The shortest time a card stays before a card of the same urgency replaces it.
pub const HOLD_MS: u64 = 6_000;
/// How long a finished turn shows.
pub const FINISH_MS: u64 = 5_200;
/// How long an error shows.
pub const ERROR_MS: u64 = 30_000;
/// Pills shown at once.
pub const VISIBLE_PILLS: usize = 4;
/// Cards kept beside the one in front, to switch to once the island is opened.
pub const SHELF: usize = 8;

/// Lower is more urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    /// The person must answer: an approval or a question. Holds the island open.
    Waiting,
    /// Worth interrupting for: an error, a finished turn.
    Alert,
    /// Everything else.
    Info,
}

/// A card a part of Buntini would like to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub card: Card,
    pub urgency: Urgency,
    /// When it was offered, in milliseconds on the caller's clock.
    pub at_ms: u64,
    /// Gone after this long.
    pub ttl_ms: Option<u64>,
}

impl Offer {
    fn live(&self, now_ms: u64) -> bool {
        self.ttl_ms.is_none_or(|ttl| now_ms < self.at_ms.saturating_add(ttl))
    }

    /// Waiting cards queue oldest first; news shows newest first.
    fn order(&self) -> (Urgency, u64) {
        let age = if self.urgency == Urgency::Waiting { self.at_ms } else { u64::MAX - self.at_ms };
        (self.urgency, age)
    }
}

/// Everything one part of Buntini shows right now. Publishing replaces the previous one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contribution {
    pub pills: Vec<Pill>,
    pub offers: Vec<Offer>,
}

/// What the island shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picked {
    pub pills: Vec<Pill>,
    pub overflow: Vec<String>,
    pub card: Option<Card>,
    /// The other news worth a look, newest first.
    pub shelf: Vec<Card>,
    pub held: bool,
    /// The card is waiting or an alert, not news: the island opens for it.
    pub interrupt: bool,
    /// How urgent the card in front is.
    pub urgency: Option<Urgency>,
}

#[derive(Debug, Default)]
pub struct Board {
    sources: BTreeMap<Source, Contribution>,
    /// The card in front: its id, urgency and when it came to the front.
    shown: Option<(String, Urgency, u64)>,
}

impl Board {
    pub fn set(&mut self, source: Source, contribution: Contribution) {
        if contribution.pills.is_empty() && contribution.offers.is_empty() {
            self.sources.remove(&source);
        } else {
            self.sources.insert(source, contribution);
        }
    }

    pub fn pick(&mut self, now_ms: u64) -> Picked {
        let mut live: Vec<&Offer> = self.sources.values().flat_map(|c| &c.offers).filter(|o| o.live(now_ms)).collect();
        live.sort_by_key(|offer| (offer.order(), offer.card.id.clone()));
        let held = live.iter().any(|offer| offer.urgency == Urgency::Waiting);
        let kept = self.shown.as_ref().and_then(|(id, urgency, since)| {
            let offer = live.iter().find(|offer| &offer.card.id == id)?;
            let best = live.first()?;
            let young = now_ms.saturating_sub(*since) < HOLD_MS;
            (young && best.urgency >= *urgency).then_some(*offer)
        });
        let front = kept.or_else(|| live.first().copied());
        self.shown = front.map(|offer| match &self.shown {
            Some((id, _, since)) if *id == offer.card.id => (id.clone(), offer.urgency, *since),
            _ => (offer.card.id.clone(), offer.urgency, now_ms),
        });

        let mut shelf: Vec<Card> = Vec::new();
        for offer in live.iter().filter(|offer| offer.urgency == Urgency::Info) {
            if front.is_none_or(|front| front.card.id != offer.card.id) && !shelf.iter().any(|card| card.id == offer.card.id) {
                shelf.push(offer.card.clone());
            }
        }
        shelf.truncate(SHELF);

        let mut pills: Vec<&Pill> = self.sources.values().flat_map(|c| &c.pills).collect();
        pills.sort_by_key(|pill| rank(pill.state));
        let overflow = pills
            .iter()
            .skip(VISIBLE_PILLS)
            .filter(|pill| pill.state == PillState::Waiting)
            .map(|pill| pill.id.clone())
            .collect();
        Picked {
            pills: pills.into_iter().take(VISIBLE_PILLS).cloned().collect(),
            overflow,
            card: front.map(|offer| offer.card.clone()),
            shelf,
            held,
            interrupt: front.is_some_and(|offer| offer.urgency != Urgency::Info),
            urgency: front.map(|offer| offer.urgency),
        }
    }

    /// The id of the card in front when the board last picked.
    pub fn front(&self) -> Option<&str> {
        self.shown.as_ref().map(|(id, _, _)| id.as_str())
    }

    /// Every pill, most urgent first.
    pub fn pills(&self) -> Vec<Pill> {
        let mut pills: Vec<&Pill> = self.sources.values().flat_map(|c| &c.pills).collect();
        pills.sort_by_key(|pill| rank(pill.state));
        pills.into_iter().cloned().collect()
    }

    /// The next moment the picture can change without anyone publishing: a card expiring or a
    /// hold ending.
    pub fn next_change_ms(&self, now_ms: u64) -> Option<u64> {
        let expiries = self
            .sources
            .values()
            .flat_map(|c| &c.offers)
            .filter_map(|offer| offer.ttl_ms.map(|ttl| offer.at_ms.saturating_add(ttl)));
        let hold = self.shown.as_ref().map(|(_, _, since)| since.saturating_add(HOLD_MS));
        expiries.chain(hold).filter(|at| *at > now_ms).min()
    }

    /// How long the card `id` has been in front, as last picked. None when it isn't.
    pub fn in_front_ms(&self, id: &str, now_ms: u64) -> Option<u64> {
        self.shown.as_ref().filter(|(shown, _, _)| shown == id).map(|(_, _, since)| now_ms.saturating_sub(*since))
    }

    /// Something waits on the person: an approval or a question.
    pub fn waiting(&self, now_ms: u64) -> bool {
        self.sources.values().flat_map(|c| &c.offers).any(|offer| offer.urgency == Urgency::Waiting && offer.live(now_ms))
    }
}

/// Waiting first, then errors, work in progress, finished, idle.
fn rank(state: PillState) -> u8 {
    match state {
        PillState::Waiting => 0,
        PillState::Error => 1,
        PillState::Working => 2,
        PillState::Done => 3,
        PillState::Idle => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::cards::utilities::Vitals;
    use crate::surface::scene::CardBody;

    fn card(id: &str) -> Card {
        Card { id: id.into(), source: Source::Utilities, body: CardBody::Vitals(Vitals { line: id.into() }) }
    }

    fn offer(id: &str, urgency: Urgency, at_ms: u64, ttl_ms: Option<u64>) -> Offer {
        Offer { card: card(id), urgency, at_ms, ttl_ms }
    }

    fn pill(id: &str, state: PillState) -> Pill {
        Pill { id: id.into(), source: Source::Agents, label: id.into(), tint: "#888888".into(), state, badge: None, progress: None, card: None }
    }

    fn shown(board: &mut Board, now: u64) -> Option<String> {
        board.pick(now).card.map(|card| card.id)
    }

    #[test]
    fn nothing_offered_shows_nothing() {
        let picked = Board::default().pick(0);
        assert_eq!(picked, Picked::default());
    }

    #[test]
    fn a_waiting_card_comes_first_and_holds_the_island() {
        let mut board = Board::default();
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![offer("news", Urgency::Info, 0, None)] });
        assert_eq!(shown(&mut board, 0).as_deref(), Some("news"));
        assert!(!board.pick(0).held);
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("approve", Urgency::Waiting, 500, None)] });
        let picked = board.pick(500);
        assert_eq!(picked.card.map(|card| card.id).as_deref(), Some("approve"));
        assert!(picked.held && picked.interrupt);
        board.set(Source::Agents, Contribution::default());
        assert!(!board.pick(HOLD_MS * 2).interrupt, "news doesn't open the island");
    }

    #[test]
    fn a_card_s_time_in_front_counts_from_when_it_came_forward() {
        let mut board = Board::default();
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("first", Urgency::Waiting, 0, None), offer("second", Urgency::Waiting, 10, None)] });
        assert!(board.waiting(100));
        board.pick(100);
        assert_eq!(board.in_front_ms("first", 900), Some(800));
        // Queued behind the first, the second hasn't been in front at all.
        assert_eq!(board.in_front_ms("second", 900), None);
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("second", Urgency::Waiting, 10, None)] });
        board.pick(1_000);
        assert_eq!(board.in_front_ms("second", 1_200), Some(200), "old, but only just in front");
        board.set(Source::Agents, Contribution::default());
        assert!(!board.waiting(1_300));
    }

    #[test]
    fn waiting_cards_queue_oldest_first() {
        let mut board = Board::default();
        board.set(Source::Agents, Contribution {
            pills: vec![],
            offers: vec![offer("second", Urgency::Waiting, 200, None), offer("first", Urgency::Waiting, 100, None)],
        });
        assert_eq!(shown(&mut board, 300).as_deref(), Some("first"));
    }

    #[test]
    fn news_waits_for_the_shown_card_to_have_its_moment() {
        let mut board = Board::default();
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![offer("a", Urgency::Info, 0, None)] });
        assert_eq!(shown(&mut board, 0).as_deref(), Some("a"));
        board.set(Source::Utilities, Contribution {
            pills: vec![],
            offers: vec![offer("a", Urgency::Info, 0, None), offer("b", Urgency::Info, 1_000, None)],
        });
        assert_eq!(shown(&mut board, 1_000).as_deref(), Some("a"));
        assert_eq!(board.next_change_ms(1_000), Some(HOLD_MS));
        assert_eq!(shown(&mut board, HOLD_MS).as_deref(), Some("b"));
    }

    #[test]
    fn something_more_urgent_does_not_wait() {
        let mut board = Board::default();
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![offer("news", Urgency::Info, 0, None)] });
        shown(&mut board, 0);
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("done", Urgency::Alert, 100, Some(FINISH_MS))] });
        assert_eq!(shown(&mut board, 100).as_deref(), Some("done"));
    }

    #[test]
    fn a_card_goes_when_its_time_is_up() {
        let mut board = Board::default();
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("done", Urgency::Alert, 0, Some(FINISH_MS))] });
        assert_eq!(shown(&mut board, FINISH_MS - 1).as_deref(), Some("done"));
        assert_eq!(board.next_change_ms(FINISH_MS - 1), Some(FINISH_MS));
        assert_eq!(shown(&mut board, FINISH_MS), None);
    }

    #[test]
    fn a_card_updated_in_place_keeps_its_place_and_shows_the_new_content() {
        let mut board = Board::default();
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![offer("a", Urgency::Info, 0, None)] });
        shown(&mut board, 0);
        let mut changed = offer("a", Urgency::Info, 0, None);
        changed.card.body = CardBody::Vitals(Vitals { line: "hot".into() });
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![changed.clone()] });
        assert_eq!(board.pick(2_000).card, Some(changed.card));
        assert_eq!(board.next_change_ms(2_000), Some(HOLD_MS));
    }

    #[test]
    fn waiting_pills_that_do_not_fit_are_counted() {
        let mut board = Board::default();
        let mut pills: Vec<Pill> = (0..6).map(|n| pill(&format!("wait{n}"), PillState::Waiting)).collect();
        pills.insert(0, pill("idle", PillState::Idle));
        board.set(Source::Agents, Contribution { pills, offers: vec![] });
        let picked = board.pick(0);
        assert_eq!(picked.pills.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["wait0", "wait1", "wait2", "wait3"]);
        assert_eq!(picked.overflow, ["wait4", "wait5"]);
    }

    #[test]
    fn the_other_news_stays_on_the_shelf_to_switch_to() {
        let mut board = Board::default();
        board.set(Source::Media, Contribution { pills: vec![], offers: vec![offer("music", Urgency::Info, 10, None)] });
        board.set(Source::Utilities, Contribution { pills: vec![], offers: vec![offer("focus", Urgency::Info, 20, None), offer("gone", Urgency::Info, 0, Some(5))] });
        board.set(Source::Agents, Contribution { pills: vec![], offers: vec![offer("ask", Urgency::Waiting, 30, None), offer("done", Urgency::Alert, 40, Some(10_000))] });
        let picked = board.pick(100);
        assert_eq!(picked.card.map(|card| card.id).as_deref(), Some("ask"));
        // News only, newest first; neither what waits nor what has had its time.
        assert_eq!(picked.shelf.iter().map(|card| card.id.as_str()).collect::<Vec<_>>(), ["focus", "music"]);

        // The card in front isn't on the shelf as well.
        board.set(Source::Agents, Contribution::default());
        let picked = board.pick(100 + HOLD_MS);
        assert_eq!(picked.card.map(|card| card.id).as_deref(), Some("focus"));
        assert_eq!(picked.shelf.iter().map(|card| card.id.as_str()).collect::<Vec<_>>(), ["music"]);
    }

    #[test]
    fn publishing_nothing_removes_a_source() {
        let mut board = Board::default();
        board.set(Source::Mail, Contribution { pills: vec![pill("mail", PillState::Idle)], offers: vec![] });
        board.set(Source::Mail, Contribution::default());
        assert_eq!(board.pick(0), Picked::default());
    }
}
