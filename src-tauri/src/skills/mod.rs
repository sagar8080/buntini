//! Skills: everyday requests Buntini recognises and carries out itself, quickly and the same
//! way every time, instead of asking the model. Each acts on an app or the Mac directly and
//! checks the outcome, so the reply says what actually happened ("Playing “…” by …",
//! "Volume is at 30%"). Anything not recognised goes to the model as before.
//!
//! Recognition is deliberately narrow: a request that only looks similar ("open my lease
//! pdf", "stop reminding me…") falls through to the model. The phrase tests in each skill
//! are the contract.
pub mod browser;
pub mod mac;
pub mod music;

use crate::{privacy::Share, types::Settings};

#[derive(Debug, PartialEq)]
pub enum Skill {
    Music(music::Command),
    Browser(browser::Command),
    Mac(mac::Command),
}

pub enum Outcome {
    /// Done: what to tell the user, and the kind of data the reply shows.
    Done { answer: String, share: Option<Share> },
    /// A web page to summarise or answer from; the model writes the reply.
    Page { title: String, text: String },
}

pub struct Context<'a> {
    pub settings: &'a Settings,
    pub prompt: &'a str,
    /// The app that was in front before Buntini opened, if known.
    pub front_app: Option<String>,
}

/// A request in lowercase words, without punctuation (apostrophes, dots and percent signs
/// stay, for "what's", "github.com" and "30%").
pub fn words(prompt: &str) -> String {
    let text = prompt.to_lowercase().replace(['’', '‘'], "'");
    let kept: String = text.chars().map(|c| if c.is_alphanumeric() || matches!(c, '\'' | '.' | '%') { c } else { ' ' }).collect();
    kept.split_whitespace().map(|word| word.trim_matches('.')).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The rest of a request from its command word, if one of `verbs` comes within the first
/// few words ("hey buntini, could you please play …" → "play …"), unless it's negated.
pub fn from_verb<'a>(text: &'a str, verbs: &[&str]) -> Option<(&'a str, &'a str)> {
    let mut offset = 0;
    for (index, word) in text.split(' ').enumerate() {
        if index > 6 || ["don't", "dont", "didn't", "didnt", "doesn't", "doesnt", "isn't", "isnt", "wasn't", "wasnt", "won't", "wont", "can't", "cant", "couldn't", "not", "never", "how", "why", "when", "where", "who", "if"].contains(&word) {
            return None;
        }
        if verbs.contains(&word) {
            return Some((word, text[offset + word.len()..].trim_start()));
        }
        offset += word.len() + 1;
    }
    None
}

/// `recent`: the user's last few messages, newest first, for follow-ups such as "yes, play
/// it" or "that's the wrong song".
pub fn recognise(prompt: &str, recent: &[String]) -> Option<Skill> {
    let text = words(prompt);
    let recent: Vec<String> = recent.iter().map(|before| words(before)).collect();
    music::recognise(&text, &recent)
        .map(Skill::Music)
        .or_else(|| mac::recognise(&text).map(Skill::Mac))
        .or_else(|| browser::recognise(&text).map(Skill::Browser))
}

/// A short name for the activity line and the audit log.
pub fn label(skill: &Skill) -> &'static str {
    match skill {
        Skill::Music(_) => "music",
        Skill::Browser(_) => "browser",
        Skill::Mac(_) => "mac",
    }
}

pub async fn run(skill: Skill, context: &Context<'_>) -> Result<Outcome, String> {
    match skill {
        Skill::Music(command) => music::run(command, context).await,
        Skill::Browser(command) => browser::run(command, context).await,
        Skill::Mac(command) => mac::run(command).await,
    }
}

/// Runs an AppleScript with values passed as arguments (never as script text).
pub(crate) async fn osascript(source: &str, argv: &[&str], seconds: u64) -> Result<String, String> {
    let mut command = tokio::process::Command::new("/usr/bin/osascript");
    command.arg("-e").arg(source).args(argv);
    crate::actions::output(command, seconds).await.map(|output| output.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_reach_the_right_skill_or_the_model() {
        let skill = |prompt: &str| recognise(prompt, &[]).map(|skill| label(&skill));
        assert_eq!(skill("Hey Buntini, can you play Hey There Delilah on Spotify?"), Some("music"));
        assert_eq!(skill("turn the volume down a bit"), Some("mac"));
        assert_eq!(skill("open youtube"), Some("browser"));
        assert_eq!(skill("summarise this page"), Some("browser"));
        assert_eq!(skill("what's on my calendar tomorrow?"), None, "the model handles the rest");
        assert_eq!(skill("why don't you play some music"), None);
        assert_eq!(skill("how do I open a terminal"), None);
        assert_eq!(skill("I don't want you to open youtube"), None);
        assert_eq!(words("Hey, Buntini! Set the volume to 30%."), "hey buntini set the volume to 30%");
        assert_eq!(from_verb("hey buntini could you please play delilah", &["play"]), Some(("play", "delilah")));
    }
}
