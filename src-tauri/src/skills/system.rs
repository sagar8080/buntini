//! "How's my Mac?", "why is it slow?", "what's filling my disk?" and "what's using my
//! battery?" answered with the Mac at a glance card, instantly and with no model
//! (glance.rs). "Quit Chrome" proposes the approved `quit_app` action, which is also what
//! the card's Quit button sends. Anything else about the Mac goes to the model as before.
use super::{from_verb, one_of, without_politeness, Outcome};
use crate::glance::Focus;

#[derive(Debug, PartialEq)]
pub enum Command {
    Look(Focus),
    Quit { app: String },
}

const OVERVIEW: &[&str] = &[
    "how's my mac",
    "how is my mac",
    "how's my mac doing",
    "how is my mac doing",
    "is my mac ok",
    "is my mac okay",
    "is my mac alright",
    "is my mac all right",
    "is my mac fine",
    "is my mac healthy",
    "check my mac",
    "my mac's health",
    "mac health",
    "mac status",
    "show my mac",
    "system status",
];
const SLOW: &[&str] = &[
    "why is my mac slow",
    "why's my mac slow",
    "why is my mac so slow",
    "why's my mac so slow",
    "why is my mac running slow",
    "why is my mac running so slow",
    "why is my mac slow today",
    "why is it slow",
    "why is it so slow",
    "my mac is slow",
    "my mac is so slow",
    "my mac feels slow",
    "my mac is running slow",
    "what's slowing my mac",
    "what is slowing my mac",
    "what's slowing my mac down",
    "what is slowing my mac down",
    "what's slowing down my mac",
    "what is slowing down my mac",
    "what's using my memory",
    "what is using my memory",
    "what's using all my memory",
    "what's using my cpu",
    "what is using my cpu",
    "what's using all my cpu",
];
const STORAGE: &[&str] = &[
    "what's filling my disk",
    "what is filling my disk",
    "what's filling my storage",
    "what is filling my storage",
    "what's filling up my disk",
    "what is filling up my disk",
    "what's filling up my storage",
    "what is filling up my storage",
    "what's taking up my disk",
    "what's taking up my storage",
    "what's taking up space",
    "what's taking up space on my mac",
    "what is taking up space on my mac",
    "what's taking up all my space",
    "what's using my disk",
    "what is using my disk",
    "what's using my storage",
    "what is using my storage",
    "how much storage do i have",
    "how much storage do i have left",
    "how much space do i have",
    "how much space do i have left",
    "how much disk space do i have",
    "how much disk space do i have left",
    "how much free space do i have",
    "is my disk full",
    "is my storage full",
    "my disk is full",
    "my storage is full",
];
const BATTERY: &[&str] = &[
    "what's using my battery",
    "what is using my battery",
    "what's draining my battery",
    "what is draining my battery",
    "what's eating my battery",
    "why is my battery draining",
    "why is my battery draining so fast",
    "why's my battery draining",
    "how's my battery",
    "how is my battery",
    "how's my battery doing",
    "how is my battery doing",
    "how's my battery health",
    "how is my battery health",
    "what's my battery health",
    "what is my battery health",
    "what's my battery at",
    "how much battery do i have",
    "how much battery do i have left",
    "battery health",
];

/// `text` is the request in lowercase words (`super::words`). Quit matches the words only;
/// the controller then checks they name an open app, so "quit smoking" reaches the model.
pub fn recognise(text: &str) -> Option<Command> {
    let text = without_politeness(text);
    let text = text.strip_prefix("hey ").unwrap_or(text);
    let text = text.strip_prefix("buntini ").unwrap_or(text);
    let same_mac = text.replace("my macbook", "my mac").replace("my computer", "my mac").replace("my laptop", "my mac");
    let question = same_mac.strip_suffix(" right now").or_else(|| same_mac.strip_suffix(" now")).unwrap_or(&same_mac);
    for (focus, phrases) in [(Focus::Overview, OVERVIEW), (Focus::Slow, SLOW), (Focus::Storage, STORAGE), (Focus::Battery, BATTERY)] {
        if one_of(question, phrases) {
            return Some(Command::Look(focus));
        }
    }
    let (_, rest) = from_verb(text, &["quit"])?;
    let app = without_politeness(rest).trim_start_matches("the ").trim_end_matches(" app");
    let usable = !app.is_empty()
        && app.split_whitespace().count() <= 5
        && !["it", "this", "that", "buntini", "everything", "all", "all apps", "my apps"].contains(&app);
    usable.then(|| Command::Quit { app: app.to_string() })
}

pub async fn run(command: Command) -> Result<Outcome, String> {
    match command {
        Command::Look(focus) => Ok(Outcome::Look(focus)),
        Command::Quit { app } => {
            let app = crate::glance::running_app_named(&app).ok_or(format!("{app} isn’t open."))?;
            Ok(Outcome::Quit { app })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{recognise as route, words, Skill};

    fn look(prompt: &str) -> Option<Focus> {
        match route(prompt, &[]) {
            Some(Skill::System(Command::Look(focus))) => Some(focus),
            _ => None,
        }
    }

    #[test]
    fn each_phrase_reaches_the_card() {
        for prompt in ["How's my Mac?", "How is my Mac doing?", "how’s my mac", "Is my MacBook OK?", "Hey Buntini, how's my computer?", "Check my Mac", "system status", "how's my mac please"] {
            assert_eq!(look(prompt), Some(Focus::Overview), "{prompt}");
        }
        for prompt in ["Why is my Mac slow?", "why is it slow?", "What's slowing my Mac down?", "My laptop is so slow", "what's using my memory?", "What’s using my CPU"] {
            assert_eq!(look(prompt), Some(Focus::Slow), "{prompt}");
        }
        for prompt in ["What's filling my disk?", "what is filling up my storage", "How much storage do I have left?", "Is my disk full?", "What's taking up space on my Mac?"] {
            assert_eq!(look(prompt), Some(Focus::Storage), "{prompt}");
        }
        for prompt in ["What's using my battery?", "what's draining my battery", "How's my battery?", "battery health", "how much battery do I have left?"] {
            assert_eq!(look(prompt), Some(Focus::Battery), "{prompt}");
        }
    }

    #[test]
    fn a_missing_apostrophe_or_a_right_now_changes_nothing() {
        assert_eq!(look("whats using my battery"), Some(Focus::Battery));
        assert_eq!(look("Whats draining my battery right now?"), Some(Focus::Battery));
        assert_eq!(look("hows my mac"), Some(Focus::Overview));
        assert_eq!(look("whys my mac so slow"), Some(Focus::Slow));
        assert_eq!(look("what's slowing my mac down now"), Some(Focus::Slow));
        assert_eq!(look("whats filling my disk"), Some(Focus::Storage));
    }

    #[test]
    fn the_photos_skill_leaves_these_alone() {
        for prompt in ["show my mac", "check my mac", "mac status", "how much storage do i have"] {
            assert!(look(prompt).is_some(), "{prompt}");
        }
    }

    #[test]
    fn other_questions_about_a_mac_go_to_the_model() {
        for prompt in [
            "is 16 GB enough for video editing?",
            "why is the wifi slow",
            "how much memory does a Mac mini have",
            "how do I check my battery health",
            "what's using my battery in Figma and how do I stop it",
            "remember that my mac is slow on Mondays",
            "don't check my mac",
            "how's my mac and play some music",
        ] {
            assert_eq!(look(prompt), None, "{prompt}");
        }
    }

    #[test]
    fn quitting_needs_a_direct_request_for_one_app() {
        let quit = |prompt: &str| match recognise(&words(prompt)) {
            Some(Command::Quit { app }) => Some(app),
            _ => None,
        };
        assert_eq!(quit("Quit Google Chrome").as_deref(), Some("google chrome"));
        assert_eq!(quit("please quit Slack").as_deref(), Some("slack"));
        assert_eq!(quit("can you quit the Photos app for me").as_deref(), Some("photos"));
        for prompt in ["don't quit Chrome", "how do I quit Chrome", "why did Chrome quit", "quit", "quit it", "quit everything", "quit Buntini", "force quit Chrome", "I want to quit my job at the bank today now"] {
            assert_eq!(quit(prompt), None, "{prompt}");
        }
        assert_eq!(route("quit Chrome and open Safari", &[]), None, "more than one thing is the controller's");
    }
}
