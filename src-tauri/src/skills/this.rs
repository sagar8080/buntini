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

fn target(rest: &str) -> Option<Command> {
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
        "this", "that", "this text", "this email", "this message", "this mail", "this paragraph",
        "this for me", "it",
    ];
    if vague.contains(&rest) {
        return Some(Command::About);
    }
    // "translate this into Portuguese", "translate this to plain English"
    let language = rest
        .strip_prefix("this into ")
        .or_else(|| rest.strip_prefix("this to "))?;
    (!language.is_empty() && language.split(' ').count() <= 3).then_some(Command::About)
}

/// `text` is the request in lowercase words (`super::words`).
pub fn recognise(text: &str) -> Option<Command> {
    if let Some((_, rest)) = from_verb(text, VERBS) {
        return target(rest);
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
    match crate::focus::referent(snapshot, context.settings).await? {
        Referent::Selection { app, text } => Ok(Outcome::Page {
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
            assert_eq!(recognise(&words(prompt)), Some(Command::About), "{prompt}");
        }
        for prompt in ["explain the selected text", "summarize what I selected"] {
            assert_eq!(recognise(&words(prompt)), Some(Command::Selection), "{prompt}");
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
        ] {
            assert_eq!(recognise(&words(prompt)), None, "{prompt}");
        }
    }
}
