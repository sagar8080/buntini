//! Skills: everyday requests Buntini recognises and carries out itself, quickly and the same
//! way every time, instead of asking the model. Each acts on an app or the Mac directly and
//! checks the outcome, so the reply says what actually happened ("Playing “…” by …",
//! "Volume is at 30%"). Anything not recognised goes to the model as before.
//!
//! Recognition is deliberately narrow: a request that only looks similar ("open my lease
//! pdf", "stop reminding me…") falls through to the model. The phrase tests in each skill
//! are the contract.
pub mod browser;
pub mod day;
pub mod mac;
pub mod music;
pub mod this;

use crate::{privacy::Share, types::Settings};

#[derive(Debug, PartialEq)]
pub enum Skill {
    Music(music::Command),
    Browser(browser::Command),
    Mac(mac::Command),
    This(this::Command),
    Day(day::Command),
}

pub enum Outcome {
    /// Done: what to tell the user, and every kind of data the reply shows, so an
    /// external brain is never replayed a turn it may not see.
    Done {
        answer: String,
        shares: Vec<Share>,
    },
    /// Text to summarise or answer from (a web page, a selection, an email); the model
    /// writes the reply. `kind` names it for the model; `share` is the data it shows.
    Page {
        kind: &'static str,
        title: String,
        text: String,
        share: Share,
    },
}

pub struct Context<'a> {
    pub settings: &'a Settings,
    pub prompt: &'a str,
    /// The app that was in front before Buntini opened, if known.
    pub front_app: Option<String>,
    /// What was in front when Buntini opened, for requests about "this".
    pub focus: Option<crate::focus::Snapshot>,
    /// The Apple helper, which reads Calendar and Reminders.
    pub helper: &'a std::path::Path,
}

/// A request in lowercase words. Apostrophes, dots, percent signs and numeric signs
/// stay, for "what's", "github.com", "30%" and rejecting negative timer durations.
pub fn words(prompt: &str) -> String {
    let text = prompt.to_lowercase().replace(['’', '‘'], "'");
    let characters: Vec<char> = text.chars().collect();
    let kept: String = characters
        .iter()
        .enumerate()
        .map(|(index, &c)| {
            let signed_number = matches!(c, '-' | '+')
                && characters.get(index + 1).is_some_and(char::is_ascii_digit)
                && (index == 0 || characters[index - 1].is_whitespace());
            if c.is_alphanumeric() || matches!(c, '\'' | '.' | '%') || signed_number {
                c
            } else {
                ' '
            }
        })
        .collect();
    kept.split_whitespace()
        .map(|word| word.trim_matches('.'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the words before a command are a direct, polite request.
fn request_prefix(mut prefix: &[&str]) -> bool {
    if prefix.iter().any(|word| {
        [
            "no", "don't", "dont", "didn't", "didnt", "doesn't", "doesnt", "isn't", "isnt",
            "wasn't", "wasnt", "won't", "wont", "can't", "cant", "couldn't", "not", "never", "how",
            "why", "when", "where", "who", "what", "which", "if", "explain", "tell", "whether",
            "should",
        ]
        .contains(word)
    }) {
        return false;
    }
    // A direct request can have a greeting, the assistant's name and a polite wrapper.
    // Arbitrary prose before a command ("explain how to ...", "I used to ...") is
    // deliberately left to the model instead of being mistaken for permission to act.
    if prefix
        .first()
        .is_some_and(|word| ["hey", "hi", "hello"].contains(word))
    {
        prefix = &prefix[1..];
        if prefix.first().is_some_and(|word| {
            !["can", "could", "would", "will", "please", "kindly", "just"].contains(word)
        }) {
            prefix = &prefix[1..];
        }
    } else if prefix
        .first()
        .is_some_and(|word| ["buntini", "nexon"].contains(word))
    {
        prefix = &prefix[1..];
    }
    if prefix.starts_with(&["i", "want", "you", "to"])
        || prefix.starts_with(&["i'd", "like", "you", "to"])
    {
        prefix = &prefix[4..];
    } else if prefix
        .first()
        .is_some_and(|word| ["can", "could", "would", "will"].contains(word))
    {
        if prefix.get(1) != Some(&"you") {
            return false;
        }
        prefix = &prefix[2..];
    }
    prefix
        .iter()
        .all(|word| ["please", "kindly", "just"].contains(word))
}

/// The rest of a direct request from its command word, unless negated or informational.
pub fn from_verb<'a>(text: &'a str, verbs: &[&str]) -> Option<(&'a str, &'a str)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut offset = 0;
    for (index, word) in words.iter().enumerate() {
        if index > 6 {
            return None;
        }
        if verbs.contains(word) {
            return request_prefix(&words[..index])
                .then(|| (*word, text[offset + word.len()..].trim_start()));
        }
        offset += word.len() + 1;
    }
    None
}

/// Optional politeness at the end of a direct request, without changing its target.
pub(crate) fn without_politeness(mut text: &str) -> &str {
    loop {
        let shortened = text
            .strip_suffix(" please")
            .or_else(|| text.strip_suffix(" for me"));
        match shortened {
            Some(rest) => text = rest,
            None => return text,
        }
    }
}

/// A fast skill must account for the whole request. Compound actions belong to
/// the durable controller, even when the first clause happens to be recognisable.
/// Conjunctions inside a duration or title ("one minute and thirty seconds",
/// "rock and roll") stay eligible because the following words are not commands.
fn compound_request(prompt: &str) -> bool {
    const ACTIONS: &[&str] = &[
        "open",
        "launch",
        "close",
        "play",
        "pause",
        "stop",
        "skip",
        "resume",
        "start",
        "set",
        "create",
        "make",
        "prepare",
        "put",
        "add",
        "draft",
        "write",
        "send",
        "reply",
        "forward",
        "remind",
        "search",
        "look",
        "find",
        "read",
        "summarize",
        "summarise",
        "save",
        "turn",
        "mute",
        "unmute",
        "switch",
        "run",
        "cancel",
        "delete",
        "lock",
        "enable",
        "disable",
        "schedule",
        "book",
        "buy",
        "list",
        "show",
        "tell",
        "keep",
        "check",
        "compare",
        "monitor",
        "watch",
        "download",
        "upload",
    ];
    let command = |tail: &str| {
        let normalized = words(tail);
        let tail = normalized
            .trim_start_matches("then ")
            .trim_start_matches("also ");
        let tail = tail
            .strip_prefix("don't ")
            .or_else(|| tail.strip_prefix("do not "))
            .or_else(|| tail.strip_prefix("never "))
            .or_else(|| tail.strip_prefix("no "))
            .unwrap_or(tail);
        from_verb(tail, ACTIONS).is_some()
            || [
                "timer",
                "timers",
                "a timer",
                "the timer",
                "a reminder",
                "a note",
                "an event",
                "volume",
                "the volume",
                "dark mode",
                "light mode",
            ]
            .iter()
            .any(|phrase| tail == *phrase || tail.starts_with(&format!("{phrase} ")))
    };
    let normalized = words(prompt);
    for separator in [
        " and ",
        " but ",
        " then ",
        " also ",
        " afterwards ",
        " after that ",
    ] {
        for (at, _) in normalized.match_indices(separator) {
            if command(&normalized[at + separator.len()..]) {
                return true;
            }
        }
    }
    // Punctuation is lost by `words`, so inspect clause boundaries before routing.
    for separator in [";", "\n", ",", ". "] {
        for (at, _) in prompt.match_indices(separator) {
            if from_verb(&words(&prompt[..at]), ACTIONS).is_some()
                && command(&prompt[at + separator.len()..])
            {
                return true;
            }
        }
    }
    false
}

/// `recent`: the user's last few messages, newest first, for follow-ups such as "yes, play
/// it" or "that's the wrong song".
pub fn recognise(prompt: &str, recent: &[String]) -> Option<Skill> {
    if compound_request(prompt) {
        return None;
    }
    let text = words(prompt);
    let recent: Vec<String> = recent
        .iter()
        .take_while(|before| !compound_request(before))
        .map(|before| words(before))
        .collect();
    music::recognise(&text, &recent)
        .map(Skill::Music)
        .or_else(|| mac::recognise(&text).map(Skill::Mac))
        .or_else(|| this::recognise(&text, &recent).map(Skill::This))
        .or_else(|| day::recognise(&text).map(Skill::Day))
        .or_else(|| browser::recognise_prompt(prompt).map(Skill::Browser))
}

/// A short name for the activity line and the audit log.
pub fn label(skill: &Skill) -> &'static str {
    match skill {
        Skill::Music(_) => "music",
        Skill::Browser(_) => "browser",
        Skill::Mac(_) => "mac",
        Skill::This(_) => "this",
        Skill::Day(_) => "day",
    }
}

pub async fn run(skill: Skill, context: &Context<'_>) -> Result<Outcome, String> {
    match skill {
        Skill::Music(command) => music::run(command, context).await,
        Skill::Browser(command) => browser::run(command, context).await,
        Skill::Mac(command) => mac::run(command).await,
        Skill::This(command) => this::run(command, context).await,
        Skill::Day(command) => day::run(command, context).await,
    }
}

/// Runs an AppleScript with values passed as arguments (never as script text).
pub(crate) async fn osascript(source: &str, argv: &[&str], seconds: u64) -> Result<String, String> {
    let mut command = crate::actions::osascript_command("AppleScript", source);
    command.args(argv);
    crate::actions::output(command, seconds)
        .await
        .map(|output| output.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_negations_and_other_targets_do_not_execute_controls() {
        for prompt in [
            "don't close this tab",
            "never close this tab",
            "can you not close this tab",
            "don't summarise this page",
            "I said close this tab",
            "tell me how to close this tab",
            "don't turn the volume to 50",
            "why is the volume down",
            "how do I set a timer for 10 minutes",
            "the timer for 10 minutes is useful",
            "don't set a timer for 10 minutes",
            "don't toggle dark mode",
            "pause my subscription",
            "resume my download",
            "skip my meeting",
            "stop reminding me",
            "play a video of cats",
        ] {
            assert_eq!(
                recognise(prompt, &[]),
                None,
                "{prompt} must go to the model"
            );
        }
        assert_eq!(
            recognise("stop the timer", &[]),
            Some(Skill::Mac(mac::Command::CancelTimers))
        );
        assert_eq!(
            recognise("stop the music", &[]).map(|skill| label(&skill)),
            Some("music")
        );
        assert_eq!(
            recognise("play Don't Stop Believin'", &[]),
            Some(Skill::Music(music::Command {
                action: "play",
                query: Some("don't stop believin'".into()),
                app: None,
                retry: false,
            }))
        );
    }

    #[test]
    fn requests_reach_the_right_skill_or_the_model() {
        let skill = |prompt: &str| recognise(prompt, &[]).map(|skill| label(&skill));
        assert_eq!(
            skill("Hey Buntini, can you play Hey There Delilah on Spotify?"),
            Some("music")
        );
        assert_eq!(skill("turn the volume down a bit"), Some("mac"));
        assert_eq!(skill("open youtube"), Some("browser"));
        assert_eq!(skill("summarise this page"), Some("browser"));
        assert_eq!(
            skill("what's on my calendar tomorrow?"),
            None,
            "the model handles the rest"
        );
        assert_eq!(skill("why don't you play some music"), None);
        assert_eq!(skill("how do I open a terminal"), None);
        assert_eq!(skill("I don't want you to open youtube"), None);
        assert_eq!(
            words("Hey, Buntini! Set the volume to 30%."),
            "hey buntini set the volume to 30%"
        );
        assert_eq!(
            from_verb("hey buntini could you please play delilah", &["play"]),
            Some(("play", "delilah"))
        );
    }

    #[test]
    fn separate_timer_requests_are_not_combined_into_one_fast_timer() {
        for prompt in [
            "set two timers for 5 minutes and 10 minutes",
            "set timers for 5 and 10 minutes",
            "set a timer for 5 minutes and another for 10 minutes",
            "set a timer for 5 minutes, another for 10 minutes",
            "set a timer for 5 minutes and a second for 10 minutes",
            "set a 5 minute timer and one for 10 minutes",
            "set a timer for 5 minutes plus an additional 10 minute timer",
            "start a timer for 5 minutes then one more for 10 minutes",
            "start a timer for 5 minutes also a separate one for 10 minutes",
        ] {
            assert_eq!(
                recognise(prompt, &[]),
                None,
                "{prompt} needs separate verified timers"
            );
        }
        for (prompt, seconds) in [
            ("set a timer for one minute and thirty seconds", 90),
            ("set a timer for five minutes and a second", 301),
            ("set a timer for a minute and a half", 90),
            ("start a timer for 1 hour and half an hour", 5400),
            ("set another timer for 10 minutes", 600),
        ] {
            assert_eq!(
                recognise(prompt, &[]),
                Some(Skill::Mac(mac::Command::Timer { seconds }))
            );
        }
        assert_eq!(
            recognise("cancel all timers", &[]),
            Some(Skill::Mac(mac::Command::CancelAllTimers))
        );
    }
}
