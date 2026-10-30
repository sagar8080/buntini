//! "Summarise this", "explain the selection", "translate this into French": a read-only
//! request about whatever was in front when Buntini opened. The selected text comes first,
//! then the email open in Mail, the page in a browser or the files selected in Finder.
//! Nothing changes on the Mac; the model writes the reply from what was read.
use super::{browser, from_verb, without_politeness, Context, Outcome};
use crate::{focus::Referent, privacy::Share};

#[derive(Debug, PartialEq)]
pub enum Command {
    /// A question about "this", whatever it turns out to be.
    About,
    /// A question that names the selection, so a missing one is an error, not a page.
    Selection,
}

const VERBS: &[&str] = &[
    "summarise", "summarize", "explain", "translate", "simplify", "proofread",
];

/// Words that qualify a language ("plain English", "Brazilian Portuguese"). Anything else
/// after the language is text the user pasted, which the model should translate itself.
const QUALIFIERS: &[&str] = &[
    "plain", "simple", "simplified", "traditional", "formal", "informal", "casual", "british",
    "american", "brazilian", "european", "mexican", "swiss", "canadian", "modern",
];

fn target(rest: &str, follow_up: bool) -> Option<Command> {
    let rest = without_politeness(rest);
    let named = [
        "the selection",
        "my selection",
        "the selected text",
        "the text i selected",
        "what i selected",
        "what i've selected",
        "the highlighted text",
    ];
    if named.contains(&rest) {
        return Some(Command::Selection);
    }
    let vague = [
        "this", "this text", "this email", "this message", "this mail", "this paragraph",
        "this for me",
    ];
    if vague.contains(&rest) {
        return Some(Command::About);
    }
    // "That" and "it" point back at the last answer once there is one; the model has it.
    if ["that", "it"].contains(&rest) {
        return (!follow_up).then_some(Command::About);
    }
    // "translate this into Portuguese", "translate this to plain English"
    let language: Vec<&str> = rest
        .strip_prefix("this into ")
        .or_else(|| rest.strip_prefix("this to "))?
        .split(' ')
        .collect();
    match language[..] {
        [_] => Some(Command::About),
        [qualifier, _] if QUALIFIERS.contains(&qualifier) => Some(Command::About),
        ["latin", "american", _] => Some(Command::About),
        _ => None,
    }
}

/// `text` is the request in lowercase words (`super::words`); `recent` holds the user's
/// earlier messages, newest first, so "summarise that" can mean the last answer.
pub fn recognise(text: &str, recent: &[String]) -> Option<Command> {
    let follow_up = recent.iter().any(|before| !before.trim().is_empty());
    if let Some((_, rest)) = from_verb(text, VERBS) {
        return target(rest, follow_up);
    }
    [
        "what is this",
        "what's this",
        "what does this say",
        "what does this mean",
        "what is this about",
        "what's this about",
        "tldr",
        "tl dr",
    ]
    .contains(&text)
    .then_some(Command::About)
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    let Some(snapshot) = context.focus.as_ref() else {
        if command == Command::Selection {
            return Err("Select the text you mean, then open me with ⌘⇧Space and ask again.".into());
        }
        // Without a snapshot "this" keeps meaning the page in the browser.
        return browser::run(browser::Command::Page, context).await;
    };
    let done = |answer: String, share: Share| Ok(Outcome::Done { answer, shares: vec![share] });
    match crate::focus::referent(snapshot, context.settings, context.prompt).await? {
        Referent::Selection { app, text, .. } => Ok(Outcome::Page {
            kind: "selected text",
            title: format!("selected in {app}"),
            text,
            share: Share::Selection,
        }),
        _ if command == Command::Selection => Err(format!(
            "Nothing was selected in {}. Select the text you mean and ask again.",
            snapshot.app.name
        )),
        Referent::Mail(mail) => Ok(Outcome::Page {
            kind: "email",
            title: format!("{} — from {}, {}", mail.subject, mail.sender, mail.received),
            text: if mail.truncated {
                format!("{}\n(Only the opening of this email was read.)", mail.text)
            } else {
                mail.text
            },
            share: Share::Mail,
        }),
        Referent::Page { .. } => browser::run(browser::Command::Page, context).await,
        Referent::Finder { paths } => {
            let names: Vec<&str> = paths
                .iter()
                .map(|path| path.trim_end_matches('/').rsplit('/').next().unwrap_or(path))
                .collect();
            done(
                format!(
                    "You have {} selected in Finder: {}. I can read files in folders you share in Settings → Privacy → Folders; ask me to find one there.",
                    if names.len() == 1 { "one item".to_string() } else { format!("{} items", names.len()) },
                    names.join(", ")
                ),
                Share::Files,
            )
        }
        Referent::Window { app, title } => done(
            if title.is_empty() {
                format!("You were in {app}, with nothing selected. Select the text you mean and ask again.")
            } else {
                format!("You were in {app} (“{title}”), with nothing selected. Select the text you mean and ask again.")
            },
            Share::Selection,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::words;

    #[test]
    fn requests_about_this_are_recognised() {
        for prompt in [
            "summarise this",
            "Can you explain this please",
            "translate this into Portuguese",
            "what does this mean?",
            "TL;DR",
            "proofread this for me",
        ] {
            assert_eq!(recognise(&words(prompt), &[]), Some(Command::About), "{prompt}");
        }
        for prompt in ["explain the selected text", "summarize what I selected"] {
            assert_eq!(recognise(&words(prompt), &[]), Some(Command::Selection), "{prompt}");
        }
    }

    #[test]
    fn mentions_questions_and_other_targets_go_elsewhere() {
        for prompt in [
            "don't summarise this",
            "how do I translate this into French",
            "summarise this page",
            "summarise my unread email",
            "explain quantum computing",
            "translate this whole long thing into many different languages",
            "what is this week's weather",
            "Translate this to Spanish: good morning",
            "translate this into French: see you on Friday",
        ] {
            assert_eq!(recognise(&words(prompt), &[]), None, "{prompt}");
        }
        for prompt in ["translate this to plain English", "translate this into Brazilian Portuguese"] {
            assert_eq!(recognise(&words(prompt), &[]), Some(Command::About), "{prompt}");
        }
    }

    #[test]
    fn that_and_it_mean_the_last_answer_once_there_is_one() {
        let earlier = ["what's the history of the lease law".to_string()];
        for prompt in ["summarise that", "explain it", "simplify that for me"] {
            assert_eq!(recognise(&words(prompt), &earlier), None, "{prompt}");
            assert_eq!(recognise(&words(prompt), &[]), Some(Command::About), "{prompt}");
        }
        assert_eq!(recognise(&words("summarise this"), &earlier), Some(Command::About));
        // Through the router too, which passes the conversation's recent messages.
        assert_eq!(crate::skills::recognise("summarise that", &earlier.to_vec()), None);
    }
}
