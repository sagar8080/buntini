//! Which of the person's apps a question reads, and whether its answer is worked out: a small
//! classifier over the bundled search model's sentence vectors (`embedding.rs`), trained on
//! written, labelled questions about made-up people (`scripts/router`). It runs on this Mac in
//! a few milliseconds, and nothing it sees leaves it.
use serde::Deserialize;
use std::{sync::OnceLock, time::Duration};

const APPS: [&str; 3] = ["read_mail", "read_calendar", "read_reminders"];
/// An app this likely is read even with nothing in the index to back it.
const SURE: f32 = 0.9;

/// What `router.bin` holds (`scripts/router/train.py`).
#[derive(Deserialize)]
struct About {
    model: String,
    heads: Vec<String>,
    inputs: usize,
    hidden: usize,
    thresholds: Thresholds,
}

struct Weights {
    thresholds: Thresholds,
    hidden: usize,
    mean: Vec<f32>,
    /// inputs × hidden, row by row.
    w1: Vec<f32>,
    b1: Vec<f32>,
    /// hidden × heads, row by row.
    w2: Vec<f32>,
    b2: Vec<f32>,
}

const HEADS: [&str; 5] = ["general", "mail", "calendar", "reminders", "work_out"];

#[derive(Deserialize, Debug, Clone, Copy)]
struct Thresholds {
    general: f32,
    app: f32,
    work_out: f32,
}

/// What the router makes of one question.
#[derive(Debug, Clone)]
pub struct Route {
    /// How likely it needs none of the person's records.
    pub general: f32,
    /// How likely each app is needed, in `APPS` order.
    pub apps: [f32; 3],
    /// How likely its answer has to be worked out rather than read off a record.
    pub work_out: f32,
    thresholds: Thresholds,
}

impl Route {
    pub fn is_general(&self) -> bool {
        self.general > self.thresholds.general
    }

    /// The apps to read: each one likely enough, or else the likeliest. None for a general question.
    pub fn reads(&self) -> Vec<String> {
        if self.is_general() {
            return vec![];
        }
        let chosen: Vec<String> = APPS.iter().zip(self.apps).filter(|(_, chance)| *chance > self.thresholds.app).map(|(tool, _)| tool.to_string()).collect();
        if !chosen.is_empty() {
            return chosen;
        }
        let likeliest = (0..APPS.len()).max_by(|a, b| self.apps[*a].total_cmp(&self.apps[*b])).unwrap_or(0);
        vec![APPS[likeliest].to_string()]
    }

    /// The apps to read whatever the records hold (the likeliest, and any nearly certain), then
    /// those likely enough to read once the index finds the question's own words in them: whether
    /// a calendar question also needs Mail depends on what this person's mail holds.
    pub fn sure_and_likely(&self) -> (Vec<String>, Vec<String>) {
        let reads = self.reads();
        let likeliest = (0..APPS.len()).max_by(|a, b| self.apps[*a].total_cmp(&self.apps[*b])).unwrap_or(0);
        reads.into_iter().partition(|tool| *tool == APPS[likeliest] || APPS.iter().zip(self.apps).any(|(app, chance)| app == tool && chance > SURE))
    }

    pub fn worked_out(&self) -> bool {
        self.work_out > self.thresholds.work_out
    }

    /// For the decision's notes.
    pub fn note(&self) -> String {
        format!("router general {:.2} mail {:.2} calendar {:.2} reminders {:.2} work out {:.2}", self.general, self.apps[0], self.apps[1], self.apps[2], self.work_out)
    }
}

fn weights() -> Option<&'static Weights> {
    static WEIGHTS: OnceLock<Option<Weights>> = OnceLock::new();
    WEIGHTS.get_or_init(|| read(include_str!("router.json"), include_bytes!("router.bin"))).as_ref()
}

fn read(about: &str, bytes: &[u8]) -> Option<Weights> {
    let about: About = serde_json::from_str(about).ok()?;
    let (inputs, hidden) = (about.inputs, about.hidden);
    if about.model != crate::embedding::MODEL_ID || about.heads != HEADS || inputs != crate::embedding::DIMENSION || bytes.len() != 4 * (inputs + inputs * hidden + hidden + hidden * HEADS.len() + HEADS.len()) {
        return None;
    }
    let mut values = bytes.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
    let mut take = |count: usize| -> Vec<f32> { values.by_ref().take(count).collect() };
    let weights = Weights { thresholds: about.thresholds, hidden, mean: take(inputs), w1: take(inputs * hidden), b1: take(hidden), w2: take(hidden * HEADS.len()), b2: take(HEADS.len()) };
    weights.b2.iter().chain(&weights.w1).all(|value| value.is_finite()).then_some(weights)
}

/// The router's reading of a question from its search-model vector.
pub fn route_vector(vector: &[i8]) -> Option<Route> {
    let weights = weights()?;
    if vector.len() != weights.mean.len() {
        return None;
    }
    let mut hidden = weights.b1.clone();
    for (at, (value, mean)) in vector.iter().zip(&weights.mean).enumerate() {
        let input = *value as f32 / 127.0 - mean;
        for (unit, weight) in hidden.iter_mut().zip(&weights.w1[at * weights.hidden..(at + 1) * weights.hidden]) {
            *unit += input * weight;
        }
    }
    let mut out = weights.b2.clone();
    for (at, unit) in hidden.iter().enumerate() {
        let unit = unit.max(0.0);
        for (head, weight) in out.iter_mut().zip(&weights.w2[at * HEADS.len()..(at + 1) * HEADS.len()]) {
            *head += unit * weight;
        }
    }
    let chance: Vec<f32> = out.iter().map(|z| 1.0 / (1.0 + (-z).exp())).collect();
    Some(Route { general: chance[0], apps: [chance[1], chance[2], chance[3]], work_out: chance[4], thresholds: weights.thresholds })
}

/// The router's reading of a question, when its words are English (the search model's language)
/// and the model is free within a moment. None otherwise; the rules route it then.
pub fn route(goal: &str) -> Option<Route> {
    // `BUNTINI_ROUTER_OFF`: the same build routed by the rules alone, for a baseline.
    #[cfg(test)]
    if std::env::var_os("BUNTINI_ROUTER_OFF").is_some() {
        return None;
    }
    let letters: Vec<char> = goal.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 3 || letters.iter().filter(|c| c.is_ascii()).count() * 10 < letters.len() * 9 {
        return None;
    }
    let vector = crate::embedding::embed_within(&[goal.to_string()], false, Duration::from_secs(2)).ok()?.pop()??;
    route_vector(&vector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weights_load_and_fit_the_search_model() {
        assert!(weights().is_some(), "router.bin doesn't fit the bundled search model");
        assert!(read(include_str!("router.json"), &include_bytes!("router.bin")[4..]).is_none());
        let route = route_vector(&vec![0; crate::embedding::DIMENSION]).unwrap();
        for chance in [route.general, route.apps[0], route.apps[1], route.apps[2], route.work_out] {
            assert!((0.0..=1.0).contains(&chance));
        }
        assert!(route_vector(&[0; 3]).is_none());
    }

    #[test]
    fn a_second_app_is_sure_only_when_nearly_certain() {
        let thresholds = Thresholds { general: 0.5, app: 0.5, work_out: 0.5 };
        let route = Route { general: 0.0, apps: [0.81, 0.99, 0.2], work_out: 0.1, thresholds };
        assert_eq!(route.sure_and_likely(), (vec!["read_calendar".to_string()], vec!["read_mail".to_string()]));
        let route = Route { general: 0.0, apps: [0.95, 0.99, 0.6], work_out: 0.1, thresholds };
        assert_eq!(route.sure_and_likely(), (vec!["read_mail".to_string(), "read_calendar".to_string()], vec!["read_reminders".to_string()]));
        let route = Route { general: 0.0, apps: [0.3, 0.2, 0.1], work_out: 0.1, thresholds };
        assert_eq!(route.sure_and_likely(), (vec!["read_mail".to_string()], vec![]));
        let route = Route { general: 0.9, apps: [0.9, 0.2, 0.1], work_out: 0.1, thresholds };
        assert_eq!(route.sure_and_likely(), (vec![], vec![]));
    }

    #[test]
    fn a_question_in_another_script_is_left_to_the_rules() {
        assert!(route("明日の予定は何ですか").is_none());
        assert!(route("ok").is_none());
    }

    /// Writes the search model's vector for every text in `BUNTINI_ROUTER_TEXTS` (a JSON array
    /// of strings) to `BUNTINI_ROUTER_VECTORS`, for `scripts/router/train.py`.
    #[test]
    #[ignore = "reads the bundled search model; run by scripts/router/embed.py"]
    fn dump_router_vectors() {
        let texts: Vec<String> = serde_json::from_str(&std::fs::read_to_string(std::env::var("BUNTINI_ROUTER_TEXTS").unwrap()).unwrap()).unwrap();
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(64) {
            for vector in crate::embedding::embed(chunk, false).unwrap() {
                vectors.push(vector.unwrap_or_default());
            }
        }
        std::fs::write(std::env::var("BUNTINI_ROUTER_VECTORS").unwrap(), serde_json::to_string(&vectors).unwrap()).unwrap();
    }

    /// Writes what the rules make of every text in `BUNTINI_ROUTER_TEXTS` to `BUNTINI_ROUTER_RULES`:
    /// whether it asks to do something or to read elsewhere, the apps its words name, and whether
    /// it's shaped as a question about the person's own things. `train.py` uses it to score the
    /// router as the controller combines it with the rules.
    #[test]
    #[ignore = "reads the bundled search model; run by scripts/router/train.py"]
    fn dump_router_rules() {
        let texts: Vec<String> = serde_json::from_str(&std::fs::read_to_string(std::env::var("BUNTINI_ROUTER_TEXTS").unwrap()).unwrap()).unwrap();
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        crate::intent::enable();
        let rows: Vec<serde_json::Value> = texts
            .iter()
            .map(|goal| {
                let (_, goal) = crate::decision::tagged(goal);
                let mut contract = crate::task_contract::TaskContract::default();
                contract.refresh(&goal, &[]);
                let writes = contract.outcomes.iter().any(|outcome| outcome.kind == "write" || outcome.kind == "unsupported");
                serde_json::json!({
                    "writes": writes,
                    "elsewhere": crate::personal_answer::reads_elsewhere(&contract),
                    "named": crate::personal_answer::source_requests(&goal, None, &contract).0,
                    "own": crate::personal_answer::about_own_things(&goal, &contract),
                })
            })
            .collect();
        std::fs::write(std::env::var("BUNTINI_ROUTER_RULES").unwrap(), serde_json::to_string(&rows).unwrap()).unwrap();
    }

    /// Plain questions go where they obviously belong, through the real search model.
    #[test]
    #[ignore = "reads the bundled search model"]
    fn plain_questions_route_where_they_belong() {
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        for (asked, apps) in [("how many unread emails came in today", "read_mail"), ("what's the capital of portugal", ""), ("anything on my calendar thursday evening", "read_calendar")] {
            let route = route(asked).unwrap();
            eprintln!("{asked}: {}", route.note());
            assert_eq!(route.reads().join(","), apps, "{asked}");
        }
    }
}
