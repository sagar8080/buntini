//! Coding agents on the island: a pill per session, its step ticker, the requests waiting
//! on you, finished turns and the diffs you open. What you click comes back here.
use crate::surface::cards::agents as cards;
use crate::surface::{self as island, Action, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency, ERROR_MS, FINISH_MS};
use serde_json::Value;

use super::session::{Phase, Session};
use super::{answer_agent, diff, hub, recap, Agent, AgentRequest, Answer, Hub, ARMING_MS, RECAP};

/// How long last week's recap shows on Monday morning.
const RECAP_MS: u64 = 20_000;
/// A finished or failed turn keeps its mark beside the notch this long, as news; once the
/// island is opened it reads as any other session (BuntiniSurface's `IslandModel.pills`).
const NEWS_MS: u64 = 15 * 60_000;

/// Each agent's colour on the island.
pub fn tint(agent: &str) -> &'static str {
    match agent {
        "claude" => "#D97757",
        "codex" => "#10A37F",
        "copilot" => "#8957E5",
        "cursor" => "#A3A3A3",
        "gemini" => "#4285F4",
        "antigravity" => "#34A853",
        "opencode" => "#F5A623",
        "amp" => "#F25C54",
        "hermes" => "#A78BFA",
        _ => "#9CA3AF",
    }
}

fn name(agent: &str) -> String {
    Agent::from_id(agent).map_or_else(|| agent.to_string(), |agent| agent.name().to_string())
}

fn pill_id(session: &Session) -> String {
    format!("agent:{}:{}", session.agent, session.id)
}

fn ticker_id(session: &Session) -> String {
    format!("ticker:{}:{}", session.agent, session.id)
}

fn some(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

fn request_card(view: &AgentRequest) -> Card {
    let body = if view.questions.is_empty() {
        CardBody::Approval(cards::Approval {
            agent: view.agent.clone(),
            project: some(view.project.clone()),
            title: view.title.clone(),
            detail: some(view.detail.clone()),
            code: view.code,
            always: view.always,
            plan: view.plan,
            allow: view.allow,
            changes: view.changes.clone(),
            rules: view.rules.clone(),
        })
    } else {
        CardBody::Question(cards::QuestionCard {
            agent: view.agent.clone(),
            project: some(view.project.clone()),
            questions: view
                .questions
                .iter()
                .map(|question| cards::AgentQuestion {
                    header: some(question.header.clone()),
                    question: question.question.clone(),
                    options: question.options.iter().map(|option| cards::AgentChoice { label: option.label.clone(), description: some(option.description.clone()) }).collect(),
                    multi: question.multi_select,
                })
                .collect(),
        })
    };
    Card { id: view.id.clone(), source: Source::Agents, body }
}

fn ticker(session: &Session) -> Card {
    let steps = session
        .steps
        .iter()
        .map(|step| cards::TickerStep {
            label: step.label.clone(),
            detail: step.detail.clone().filter(|detail| !detail.is_empty()),
            added: step.diff.as_ref().map(|diff| diff.added as i64),
            removed: step.diff.as_ref().map(|diff| diff.removed as i64),
            failed: step.failed,
        })
        .collect();
    Card {
        id: ticker_id(session),
        source: Source::Agents,
        body: CardBody::Ticker(cards::Ticker { agent: name(&session.agent), project: some(session.project()), steps }),
    }
}

/// Everything the agents show, published whenever it changes.
pub(super) fn publish(hub: &Hub) {
    let now = island::now_ms();
    let mut offers: Vec<Offer> = hub
        .waiting
        .iter()
        .map(|waiting| Offer { card: request_card(&waiting.view), urgency: Urgency::Waiting, at_ms: waiting.at_ms, ttl_ms: None })
        .collect();
    let mut sessions: Vec<&Session> = hub.sessions.values().collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_ms));
    let mut pills = Vec::new();
    for session in sessions {
        let waiting = session.asking || hub.waiting.iter().any(|waiting| waiting.agent.id() == session.agent && waiting.session == session.id);
        let state = pill_state(session.phase, waiting, now.saturating_sub(session.last_ms));
        // Claude Code in the Claude app's Code tab.
        let label = if session.host.entrypoint == "claude-desktop" { "Claude".to_string() } else { name(&session.agent) };
        pills.push(Pill {
            id: pill_id(session),
            source: Source::Agents,
            label,
            tint: tint(&session.agent).into(),
            state,
            badge: some(session.project()),
            progress: None,
            card: (session.phase == Phase::Working && !session.steps.is_empty()).then(|| ticker_id(session)),
        });
        if session.phase == Phase::Working && !session.steps.is_empty() {
            offers.push(Offer { card: ticker(session), urgency: Urgency::Info, at_ms: session.last_ms, ttl_ms: None });
        }
        if session.announce && matches!(session.phase, Phase::Done | Phase::Error) {
            let failed = session.phase == Phase::Error;
            offers.push(Offer {
                card: Card {
                    id: format!("finished:{}:{}:{}", session.agent, session.id, session.last_ms),
                    source: Source::Agents,
                    body: CardBody::Finished(cards::Finished { agent: name(&session.agent), project: some(session.project()), message: some(session.message.clone()) }),
                },
                urgency: Urgency::Alert,
                at_ms: session.last_ms,
                ttl_ms: Some(if failed { ERROR_MS } else { FINISH_MS }),
            });
        }
    }
    if let Some((card, at_ms)) = &hub.diff {
        offers.push(Offer { card: card.clone(), urgency: Urgency::Info, at_ms: *at_ms, ttl_ms: Some(30_000) });
    }
    // Plan windows show when the island is opened with nothing newer to show.
    for (agent, usage) in &hub.usage {
        offers.push(Offer {
            card: Card { id: format!("usage:{}", agent.id()), source: Source::Agents, body: CardBody::Usage(usage.clone()) },
            urgency: Urgency::Info,
            at_ms: 0,
            ttl_ms: None,
        });
    }
    if let Some((card, at_ms)) = &hub.recap {
        offers.push(Offer { card: card.clone(), urgency: Urgency::Alert, at_ms: *at_ms, ttl_ms: Some(RECAP_MS) });
    }
    island::publish(Source::Agents, Contribution { pills, offers });
}

/// A session's pill: asking beats everything, then working; a turn that finished or failed
/// `since` milliseconds ago stays news for `NEWS_MS`.
fn pill_state(phase: Phase, waiting: bool, since: u64) -> PillState {
    match phase {
        _ if waiting => PillState::Waiting,
        Phase::Working => PillState::Working,
        Phase::Error if since < NEWS_MS => PillState::Error,
        Phase::Done if since < NEWS_MS => PillState::Done,
        _ => PillState::Idle,
    }
}

/// Hands what the person clicks on the island to the agents.
pub(super) fn listen() {
    island::on_action(Source::Agents, |action| {
        if let Err(problem) = handle(&action) {
            eprintln!("An agent action didn't go through: {problem}");
        }
    });
}

/// An answer clicked on the island counts only once its card has been in front of the
/// island for a moment, so a click aimed at the card before it doesn't land on this one.
fn answer_from_island(card: &str, answer: Answer) -> Result<(), String> {
    match island::shown_ms(card, island::now_ms()) {
        Some(ms) if ms >= ARMING_MS => answer_agent(card.to_string(), answer),
        Some(_) => Err("That card only just showed.".into()),
        None => Err("That card isn't showing.".into()),
    }
}

fn handle(action: &Action) -> Result<(), String> {
    let card = action.card.clone().unwrap_or_default();
    match action.name.as_str() {
        "allow" => answer_from_island(&card, Answer::Allow),
        "always" => answer_from_island(&card, Answer::Always),
        "deny" => answer_from_island(&card, Answer::Deny),
        "terminal" => answer_from_island(&card, Answer::Terminal),
        "answers" => match &action.value {
            Some(Value::Object(answers)) => answer_from_island(&card, Answer::Answers { answers: answers.clone() }),
            _ => Err("No answers came with it.".into()),
        },
        "diff" => {
            let index = action.value.as_ref().and_then(Value::as_u64).ok_or("No step came with it.")? as usize;
            open_diff(&card, index)
        }
        "pill" => match &action.value {
            Some(Value::String(pill)) => super::terminal::open_for_pill(pill),
            _ => Ok(()),
        },
        // Files dropped on a session: `{ "pill": …, "paths": [...] }`.
        "drop" => {
            let value = action.value.as_ref().ok_or("Nothing was dropped.")?;
            let pill = value.get("pill").and_then(Value::as_str).ok_or("No session came with it.")?;
            let paths: Vec<String> = value.get("paths").and_then(Value::as_array).map(|paths| paths.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            super::terminal::drop_files(pill, &paths)
        }
        "shareRecap" => super::share_recap().map(|_| ()),
        _ => Ok(()),
    }
}

/// Shows the diff of a ticker step.
fn open_diff(ticker: &str, index: usize) -> Result<(), String> {
    let mut hub = hub();
    let session = hub.sessions.values().find(|session| ticker_id(session) == ticker).ok_or("That session has gone.")?;
    let step = session.steps.get(index).ok_or("That step has gone.")?;
    let changes = step.diff.as_ref().ok_or("That step changed nothing.")?;
    let card = Card {
        id: format!("diff:{}:{}", ticker, index),
        source: Source::Agents,
        body: CardBody::Diff(cards::DiffCard {
            file: step.file.clone().unwrap_or_default(),
            added: changes.added as i64,
            removed: changes.removed as i64,
            lines: changes
                .lines
                .iter()
                .take(240)
                .map(|line| cards::DiffLine {
                    kind: match line.kind {
                        diff::Kind::Add => "add",
                        diff::Kind::Remove => "remove",
                        diff::Kind::Same => "same",
                        diff::Kind::Gap => "gap",
                    }
                    .into(),
                    text: line.text.chars().take(240).collect(),
                })
                .collect(),
        }),
    };
    hub.diff = Some((card, island::now_ms()));
    publish(&hub);
    Ok(())
}

/// Shows last week's recap on Monday morning, once.
pub(super) fn offer_recap() {
    if !recap::monday_morning() {
        return;
    }
    let due = RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).due(recap::today());
    let Some(week) = due else { return };
    if let Some(dir) = super::socket_dir() {
        RECAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).save(&recap::path(dir));
    }
    let mut hub = hub();
    hub.recap = Some((recap_card(&week), island::now_ms()));
    publish(&hub);
}

pub(super) fn recap_card(week: &recap::Week) -> Card {
    Card {
        id: format!("recap:{}", week.start),
        source: Source::Agents,
        body: CardBody::Recap(cards::Recap {
            week: recap::title(&week.start),
            stats: recap::stats(week, name, true).into_iter().map(|(label, value)| cards::RecapStat { label, value }).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_turn_stays_news_for_a_quarter_of_an_hour() {
        assert_eq!(pill_state(Phase::Done, false, 10 * 60_000), PillState::Done);
        assert_eq!(pill_state(Phase::Error, false, 14 * 60_000), PillState::Error);
        assert_eq!(pill_state(Phase::Done, false, NEWS_MS), PillState::Idle);
        assert_eq!(pill_state(Phase::Idle, false, 0), PillState::Idle);
        // Working or asking again comes first.
        assert_eq!(pill_state(Phase::Working, false, 0), PillState::Working);
        assert_eq!(pill_state(Phase::Done, true, 0), PillState::Waiting);
    }
}
