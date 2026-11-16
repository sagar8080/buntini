//! Call notes: from what the helper heard on each side to lines speaker by speaker, and the
//! brain's summary of them. Pure.
use crate::surface::cards::flow::NoteLine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// This Mac's microphone: you.
    Mic,
    /// What the Mac played: everyone else.
    System,
}

/// One stretch of speech, as the helper hands it over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub side: Side,
    /// The diarizer's cluster on that side; the microphone is one person.
    #[serde(default)]
    pub speaker: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Same-speaker stretches closer than this read as one line.
const JOIN_GAP: f64 = 2.5;
/// Share of a microphone stretch's words also heard from the speakers at the same time,
/// above which it is the speakers' sound coming back in.
const ECHO_SHARE: f64 = 0.6;

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| word.chars().filter(|ch| ch.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect()
}

fn overlaps(a: &Segment, b: &Segment) -> bool {
    // A little slack: the room's echo arrives late.
    a.start < b.end + 0.5 && b.start < a.end + 0.5
}

/// Drops microphone stretches that are mostly the other side heard through the speakers,
/// for when echo removal on the audio left some behind.
pub fn drop_echo(segments: Vec<Segment>) -> Vec<Segment> {
    let system: Vec<&Segment> = segments.iter().filter(|segment| segment.side == Side::System).collect();
    let echoes: Vec<bool> = segments
        .iter()
        .map(|segment| {
            if segment.side != Side::Mic {
                return false;
            }
            let heard = words(&segment.text);
            if heard.is_empty() {
                return true;
            }
            let played: HashSet<String> = system.iter().filter(|other| overlaps(segment, other)).flat_map(|other| words(&other.text)).collect();
            let shared = heard.iter().filter(|word| played.contains(*word)).count();
            shared as f64 / heard.len() as f64 >= ECHO_SHARE
        })
        .collect();
    segments.into_iter().zip(echoes).filter(|(_, echo)| !echo).map(|(segment, _)| segment).collect()
}

/// Lines speaker by speaker: you, then the others numbered in the order they first spoke.
pub fn lines(segments: Vec<Segment>) -> Vec<NoteLine> {
    let mut segments: Vec<Segment> = drop_echo(segments).into_iter().filter(|segment| !segment.text.trim().is_empty()).collect();
    segments.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut out: Vec<(String, f64, String)> = Vec::new();
    for segment in segments {
        let name = match segment.side {
            Side::Mic => "You".to_string(),
            Side::System => {
                let next = names.len() + 1;
                names.entry(segment.speaker).or_insert_with(|| format!("Speaker {next}")).clone()
            }
        };
        let text = segment.text.trim().to_string();
        match out.last_mut() {
            Some((last, end, joined)) if *last == name && segment.start - *end <= JOIN_GAP => {
                joined.push(' ');
                joined.push_str(&text);
                *end = end.max(segment.end);
            }
            _ => out.push((name, segment.end, text)),
        }
    }
    // A call with just one person on the other side doesn't need numbers.
    let others = names.len();
    out.into_iter()
        .map(|(name, _, text)| NoteLine { speaker: Some(if others == 1 && name == "Speaker 1" { "Them".into() } else { name }), text })
        .collect()
}

/// The note as text for the brain, one line per turn.
pub fn transcript(lines: &[NoteLine]) -> String {
    lines.iter().map(|line| format!("{}: {}", line.speaker.as_deref().unwrap_or("Someone"), line.text)).collect::<Vec<_>>().join("\n")
}

/// What the brain is asked to make of a call.
pub fn summary_prompt() -> &'static str {
    "You write notes of a recorded call for the person who was on it (\"You\"). Reply with JSON only: {\"title\": a few words, \"summary\": two or three sentences, \"keyPoints\": [short strings], \"todos\": [short strings, each an action someone agreed to, naming who when known]}. Use the call's language. Leave a list empty rather than inventing."
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Summary {
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub todos: Vec<String>,
}

/// Reads the brain's answer, also when it wrapped the JSON in a code fence or words.
pub fn read_summary(answer: &str) -> Option<Summary> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    let mut summary: Summary = serde_json::from_str(answer.get(start..=end)?).ok()?;
    let clean = |items: &mut Vec<String>| {
        items.iter_mut().for_each(|item| *item = item.trim().trim_start_matches(['-', '•', '*']).trim().to_string());
        items.retain(|item| !item.is_empty());
        items.truncate(12);
    };
    clean(&mut summary.key_points);
    clean(&mut summary.todos);
    summary.title = summary.title.trim().chars().take(80).collect();
    (!summary.summary.trim().is_empty() || !summary.key_points.is_empty() || !summary.todos.is_empty()).then_some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(side: Side, speaker: u32, start: f64, end: f64, text: &str) -> Segment {
        Segment { side, speaker, start, end, text: text.into() }
    }

    fn said(lines: &[NoteLine]) -> Vec<(String, String)> {
        lines.iter().map(|line| (line.speaker.clone().unwrap(), line.text.clone())).collect()
    }

    #[test]
    fn turns_merge_by_speaker_in_time_order() {
        let segments = vec![
            seg(Side::System, 7, 0.0, 2.0, "Hi, thanks for joining."),
            seg(Side::System, 7, 2.5, 4.0, "Can you hear me?"),
            seg(Side::Mic, 0, 4.2, 5.0, "Yes, loud and clear."),
            seg(Side::System, 3, 5.5, 8.0, "Great, let's start with the budget."),
            seg(Side::Mic, 0, 8.5, 9.0, "Sure."),
            seg(Side::Mic, 0, 20.0, 21.0, "Anything else?"),
        ];
        assert_eq!(
            said(&lines(segments)),
            [
                ("Speaker 1".into(), "Hi, thanks for joining. Can you hear me?".into()),
                ("You".into(), "Yes, loud and clear.".into()),
                ("Speaker 2".into(), "Great, let's start with the budget.".into()),
                ("You".into(), "Sure.".into()),
                ("You".into(), "Anything else?".into()),
            ]
        );
    }

    #[test]
    fn one_person_on_the_other_side_is_them() {
        let segments = vec![seg(Side::System, 4, 0.0, 1.0, "Hello?"), seg(Side::Mic, 0, 1.2, 2.0, "Hi!")];
        assert_eq!(said(&lines(segments)), [("Them".into(), "Hello?".into()), ("You".into(), "Hi!".into())]);
    }

    #[test]
    fn the_speakers_heard_by_the_microphone_are_dropped() {
        let segments = vec![
            seg(Side::System, 1, 10.0, 14.0, "The launch moves to Thursday because of the review."),
            // The laptop's microphone picked the speakers up, a little late and misheard.
            seg(Side::Mic, 0, 10.3, 14.4, "the launch moves to thursday because of review"),
            // Your own answer, said over the end of theirs, stays.
            seg(Side::Mic, 0, 13.8, 15.0, "Okay, I'll tell design."),
            // Silence the recognizer made nothing of goes too.
            seg(Side::Mic, 0, 30.0, 31.0, "   "),
        ];
        assert_eq!(
            said(&lines(segments)),
            [("Them".into(), "The launch moves to Thursday because of the review.".into()), ("You".into(), "Okay, I'll tell design.".into())]
        );
    }

    #[test]
    fn the_same_words_at_another_time_are_not_echo() {
        let segments = vec![seg(Side::System, 1, 0.0, 2.0, "sounds good"), seg(Side::Mic, 0, 60.0, 61.0, "sounds good")];
        assert_eq!(lines(segments).len(), 2);
    }

    /// The helper's real output for a recorded call (diarized, echo removed on the audio) reads
    /// as alternating turns, the two voices on the far side kept apart.
    #[test]
    fn a_recorded_two_voice_call_reads_turn_by_turn() {
        let raw: serde_json::Value = serde_json::from_str(include_str!("fixtures/two-voice-call.json")).unwrap();
        let segments: Vec<Segment> = serde_json::from_value(raw["segments"].clone()).unwrap();
        let lines = lines(segments);
        let speakers: Vec<&str> = lines.iter().map(|line| line.speaker.as_deref().unwrap()).collect();
        assert_eq!(speakers, ["Speaker 1", "Speaker 2", "You", "Speaker 1", "Speaker 2", "You", "Speaker 1"]);
        assert!(lines[0].text.starts_with("Hi everyone"));
        assert!(lines[2].text.contains("Wednesday"));
        assert!(lines[6].text.contains("Thursday"));
        // The far side heard through the microphone isn't written twice.
        assert!(!lines.iter().any(|line| line.speaker.as_deref() == Some("You") && line.text.contains("launch plan")));
    }

    #[test]
    fn the_brains_summary_is_read_even_when_wrapped() {
        let answer = "Here you go:\n```json\n{\"title\":\"Budget sync\",\"summary\":\"We agreed the budget.\",\"keyPoints\":[\"- Launch Thursday\",\"\"],\"todos\":[\"Sam sends the deck\"]}\n```";
        let summary = read_summary(answer).unwrap();
        assert_eq!(summary.title, "Budget sync");
        assert_eq!(summary.key_points, ["Launch Thursday"]);
        assert_eq!(summary.todos, ["Sam sends the deck"]);
        assert_eq!(read_summary("I can't help with that."), None);
        assert_eq!(read_summary("{\"title\":\"x\"}"), None, "nothing worth keeping");
    }

    #[test]
    fn the_transcript_names_each_turn() {
        let text = transcript(&[NoteLine { speaker: Some("You".into()), text: "Hi".into() }, NoteLine { speaker: None, text: "Hey".into() }]);
        assert_eq!(text, "You: Hi\nSomeone: Hey");
    }
}
