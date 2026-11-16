//! A tour of the island: a scripted minute of what it does, with made-up sessions, mail and
//! music from the shared sample scenes. Nothing is sent, read or saved, and clicks on the
//! tour's cards only move it along.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::surface::{self, Card, Contribution, Mood, Offer, Pill, PillState, Source, Urgency};

const SAMPLES: &str = include_str!("../../../native/surface/Fixtures/contract.json");

static RUNNING: AtomicBool = AtomicBool::new(false);
static NEXT: AtomicBool = AtomicBool::new(false);

/// The cat's mood for the tour's current step, while it runs.
pub(super) static MOOD: std::sync::Mutex<Option<Mood>> = std::sync::Mutex::new(None);

/// One step: a sample card by kind, how long it shows, and the cat's mood.
const STEPS: &[(&str, u64, Option<Mood>)] = &[
    ("greeting", 3, None),
    ("ticker", 5, None),
    ("approval", 6, None),
    ("finished", 4, Some(Mood::Happy)),
    ("nowPlaying", 6, Some(Mood::Dancing)),
    ("mail", 5, None),
    ("service", 5, None),
    ("timer", 4, None),
    ("recap", 6, Some(Mood::Happy)),
];

/// Every card in the samples, by kind.
fn samples() -> Vec<(String, Card, Vec<Pill>)> {
    let Ok(root) = serde_json::from_str::<Value>(SAMPLES) else { return Vec::new() };
    root["scenes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|scene| {
            let card: Card = serde_json::from_value(scene["card"].clone()).ok()?;
            let kind = serde_json::to_value(&card.body).ok()?["kind"].as_str()?.to_string();
            let pills: Vec<Pill> = serde_json::from_value(scene["pills"].clone()).unwrap_or_default();
            Some((kind, card, pills))
        })
        .collect()
}

/// A click on a tour card: show the next step now.
pub fn advance() {
    NEXT.store(true, Ordering::SeqCst);
}

/// Plays the tour once. A second start while it runs does nothing.
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let samples = samples();
        let pills: Vec<Pill> = samples
            .iter()
            .find(|(kind, _, _)| kind == "approval")
            .map(|(_, _, pills)| pills.iter().cloned().map(|pill| Pill { source: Source::Island, id: format!("demo:{}", pill.id), state: PillState::Working, ..pill }).collect())
            .unwrap_or_default();
        for (kind, seconds, mood) in STEPS {
            let Some((_, card, _)) = samples.iter().find(|(name, _, _)| name == kind) else { continue };
            *MOOD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = *mood;
            let card = Card { id: format!("demo:{}", card.id), source: Source::Island, body: card.body.clone() };
            surface::publish(Source::Island, Contribution {
                pills: pills.clone(),
                offers: vec![Offer { card, urgency: Urgency::Alert, at_ms: surface::now_ms(), ttl_ms: None }],
            });
            NEXT.store(false, Ordering::SeqCst);
            for _ in 0..seconds * 10 {
                std::thread::sleep(Duration::from_millis(100));
                if NEXT.swap(false, Ordering::SeqCst) {
                    break;
                }
            }
        }
        *MOOD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        // Back to the island's own cards.
        super::publish(Vec::new());
        RUNNING.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_of_the_tour_has_a_sample() {
        let samples = samples();
        for (kind, _, _) in STEPS {
            assert!(samples.iter().any(|(name, _, _)| name == kind), "no sample {kind}");
        }
    }
}
