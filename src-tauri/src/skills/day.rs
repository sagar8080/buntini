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
                shares: vec![Share::Mail],
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
            Ok(Outcome::Done { answer, shares: shares(&day, battery_low) })
        }
    }
}

/// Every kind of data a day summary shows: events and reminders are Calendar, the inbox
/// is Mail and the battery line is System. A turn is only replayed to an external brain
/// allowed all of them, so none may be left out.
pub fn shares(day: &crate::brief::Day, battery_low: bool) -> Vec<Share> {
    let mut shares = Vec::new();
    if day.events.is_some() || day.reminders.is_some() {
        shares.push(Share::Calendar);
    }
    if day.mail.is_some() {
        shares.push(Share::Mail);
    }
    if battery_low {
        shares.push(Share::System);
    }
    shares
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

    fn day(events: bool, reminders: bool, mail: bool) -> crate::brief::Day {
        crate::brief::Day {
            events: events.then(|| vec![("09:00".into(), "Standup".into())]),
            reminders: reminders.then(|| vec!["Call mum".into()]),
            calendar_allowed: events,
            reminders_allowed: reminders,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: mail.then(crate::triage::Triage::default),
            mail_allowed: mail,
            mail_skipped: false,
        }
    }

    #[test]
    fn my_day_touches_every_category_it_read() {
        assert_eq!(shares(&day(true, false, true), false), [Share::Calendar, Share::Mail]);
        assert_eq!(shares(&day(false, true, false), false), [Share::Calendar]);
        assert_eq!(shares(&day(false, false, false), false), []);
        assert_eq!(
            shares(&day(true, true, true), true),
            [Share::Calendar, Share::Mail, Share::System]
        );
        assert_eq!(shares(&day(false, false, false), true), [Share::System]);
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
