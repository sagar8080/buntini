//! "What needs a reply?" and "My day": read-only summaries of Mail, Calendar and Reminders
//! written by fixed rules on this Mac, without the model. Each part needs its app allowed
//! in Settings → Apps; a part that couldn't be read says so.
use super::{from_verb, without_politeness, Context, Outcome};
use crate::privacy::Share;

#[derive(Debug, PartialEq)]
pub enum Command {
    NeedsReply,
    MyDay,
}

/// `text` is the request in lowercase words (`super::words`).
pub fn recognise(text: &str) -> Option<Command> {
    let text = without_politeness(text);
    let text = text.strip_prefix("buntini ").unwrap_or(text);
    if [
        "what needs a reply",
        "what needs replying to",
        "what do i need to reply to",
        "what should i reply to",
        "who do i need to reply to",
        "who should i reply to",
        "who's waiting for a reply",
        "who is waiting for a reply",
        "which emails need a reply",
        "what emails need a reply",
        "which emails need replies",
        "what emails need replies",
        "which emails should i reply to",
        "what emails should i reply to",
        "any emails i need to reply to",
        "any emails that need a reply",
        "do i need to reply to anything",
        "do any emails need a reply",
    ]
    .contains(&text)
        || from_verb(text, &["triage"]).is_some_and(|(_, rest)| {
            ["my email", "my emails", "my mail", "my inbox", "email", "the inbox"]
                .contains(&without_politeness(rest))
        })
    {
        return Some(Command::NeedsReply);
    }
    if [
        "my day",
        "what's my day like",
        "what is my day like",
        "what does my day look like",
        "how does my day look",
        "how's my day looking",
        "plan my day",
        "brief me",
        "my brief",
        "morning brief",
        "give me my brief",
        "give me my morning brief",
    ]
    .contains(&text)
    {
        return Some(Command::MyDay);
    }
    None
}

pub async fn run(command: Command, context: &Context<'_>) -> Result<Outcome, String> {
    let mail_allowed = crate::apps::allowed(context.settings, "mail");
    match command {
        Command::NeedsReply => {
            if !mail_allowed {
                return Err("Allow Mail in Settings → Apps so I can check what needs a reply.".into());
            }
            let triage = crate::triage::run(crate::apps::Launch::Allow).await?;
            Ok(Outcome::Done {
                answer: crate::triage::compose(&triage),
                share: Some(Share::Mail),
            })
        }
        Command::MyDay => {
            let day = crate::brief::gather(context.helper, context.settings, crate::apps::Launch::Allow).await;
            let battery_low = crate::world::health_now().is_some_and(|health| {
                !health.charging && health.battery_percent.is_some_and(|level| level <= 20)
            });
            let answer = crate::brief::compose(
                &context.settings.user_name,
                chrono::Local::now(),
                &day,
                battery_low,
            );
            let share = if day.mail.is_some() {
                Share::Mail
            } else {
                Share::Calendar
            };
            Ok(Outcome::Done { answer, share: Some(share) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::words;

    #[test]
    fn everyday_inbox_and_day_questions_are_recognised() {
        for prompt in ["What needs a reply?", "Which emails need a reply", "triage my inbox please", "Who's waiting for a reply?"] {
            assert_eq!(recognise(&words(prompt)), Some(Command::NeedsReply), "{prompt}");
        }
        for prompt in ["My day", "What does my day look like?", "brief me"] {
            assert_eq!(recognise(&words(prompt)), Some(Command::MyDay), "{prompt}");
        }
    }

    #[test]
    fn writes_and_other_questions_go_elsewhere() {
        for prompt in [
            "reply to this saying yes",
            "what needs a reply from Ana about the lease",
            "don't triage my inbox",
            "what's on my calendar tomorrow",
            "plan my day around the dentist",
        ] {
            assert_eq!(recognise(&words(prompt)), None, "{prompt}");
        }
    }
}
