//! "What needs a reply?": unreplied inbox email sorted into what likely needs an answer,
//! by fixed rules rather than a model, so every brain gets the same, explainable result.
//! A message needs a reply when a person (not an automated sender) wrote to the user
//! directly and asked something; flagged messages count too. Everything else from people
//! is listed as worth a look; automated mail is only counted.
use crate::apps::MailHeader;
use std::collections::HashMap;

/// How far back "what needs a reply" looks.
pub const DAYS: u32 = 7;
/// How many message openings are read to look for a question.
const READ_AT_MOST: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub header: MailHeader,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Triage {
    pub reply: Vec<Item>,
    pub look: Vec<Item>,
    pub automated: usize,
    /// (checked, total) when Mail had more unreplied messages than were checked.
    pub partial: Option<(usize, usize)>,
    /// Messages people sent the user directly whose opening wasn't read (past the
    /// first `READ_AT_MOST`, or Mail didn't return it), so a question in them was missed.
    pub unopened: usize,
}

const AUTOMATED: &[&str] = &[
    "noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "notification",
    "notifications", "newsletter", "newsletters", "mailer-daemon", "bounce", "alert",
    "alerts", "updates", "digest", "marketing", "news", "billing", "receipts", "support",
];

/// Whether a sender looks like a system rather than a person, from its address.
pub fn automated(sender: &str) -> bool {
    let address = crate::focus::sender_address(sender).unwrap_or_else(|| sender.to_lowercase());
    let local = address.split('@').next().unwrap_or("");
    AUTOMATED
        .iter()
        .any(|word| {
            local == *word
                || local.starts_with(&format!("{word}."))
                || local.starts_with(&format!("{word}-"))
                || local.starts_with(&format!("{word}+"))
        })
        || local.starts_with("noreply")
        || local.starts_with("no-reply")
}

/// Whether the opening of a message asks the reader for something.
pub fn asks(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains('?')
        || [
            "can you", "could you", "would you", "will you", "are you", "do you", "please",
            "let me know", "rsvp", "confirm", "get back to me", "your thoughts",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// Whether a person wrote to the user directly (in To, not one of many).
fn direct(header: &MailHeader) -> bool {
    match header.to_me {
        Some(to_me) => to_me && header.to_count <= 5,
        None => header.to_count <= 3,
    }
}

/// The people's messages worth opening for a question, newest first.
pub fn candidates(headers: &[MailHeader]) -> Vec<String> {
    headers
        .iter()
        .filter(|h| !automated(&h.sender) && direct(h))
        .take(READ_AT_MOST)
        .map(|h| h.id.clone())
        .collect()
}

/// Why a message sent to the user is listed although Buntini never read it.
const UNOPENED: &str = "sent to you, not opened";

pub fn sort(headers: Vec<MailHeader>, total: usize, openings: &HashMap<String, String>) -> Triage {
    let mut triage = Triage {
        partial: (total > headers.len()).then_some((headers.len(), total)),
        ..Default::default()
    };
    for header in headers {
        if automated(&header.sender) {
            if header.flagged {
                triage.look.push(Item { header, reason: "flagged" });
            } else {
                triage.automated += 1;
            }
            continue;
        }
        let opening = openings.get(&header.id).map(String::as_str);
        let reason = if header.flagged {
            Some("flagged")
        } else if direct(&header) && opening.is_some_and(asks) {
            Some("asks you something")
        } else {
            None
        };
        match reason {
            Some(reason) => triage.reply.push(Item { header, reason }),
            None if direct(&header) && opening.is_none() => {
                triage.unopened += 1;
                triage.look.push(Item { header, reason: UNOPENED });
            }
            None => triage.look.push(Item {
                reason: if direct(&header) { "sent to you" } else { "you’re one of several recipients" },
                header,
            }),
        }
    }
    triage
}

fn sender(header: &MailHeader) -> String {
    match header.sender.split_once('<') {
        Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"').to_string(),
        _ => header.sender.trim().to_string(),
    }
}

fn line(item: &Item) -> String {
    let when = chrono::NaiveDateTime::parse_from_str(&item.header.received, "%Y-%m-%d %H:%M")
        .map(|at| at.format("%a %-d %b").to_string())
        .unwrap_or_else(|_| item.header.received.clone());
    let subject = if item.header.subject.is_empty() { "(no subject)" } else { &item.header.subject };
    format!("• {} — “{}” ({when}): {}", sender(&item.header), subject, item.reason)
}

/// The answer as Buntini says it.
pub fn compose(triage: &Triage) -> String {
    let mut lines = Vec::new();
    let unopened = match triage.unopened {
        1 => "1 email sent to you that I didn’t open".to_string(),
        n => format!("{n} emails sent to you that I didn’t open"),
    };
    // The ones past the reading limit are named, so "check them" says which.
    let (unread, others): (Vec<&Item>, Vec<&Item>) =
        triage.look.iter().partition(|item| item.reason == UNOPENED);
    let list_unread = |lines: &mut Vec<String>| {
        lines.extend(unread.iter().take(5).map(|item| line(item)));
        if unread.len() > 5 {
            lines.push(format!("…and {} more.", unread.len() - 5));
        }
    };
    match triage.reply.len() {
        // "Nothing" would claim more than was read.
        0 if triage.unopened > 0 => lines.push(format!(
            "None of the emails I opened looks like it needs a reply, but I only open the newest {READ_AT_MOST} sent to you, so check the {unopened}:"
        )),
        0 => lines.push(format!("Nothing in your inbox from the last {DAYS} days looks like it needs a reply.")),
        1 => lines.push("One email looks like it needs a reply:".into()),
        n => lines.push(format!("{n} emails look like they need a reply:")),
    }
    if triage.reply.is_empty() {
        list_unread(&mut lines);
    }
    lines.extend(triage.reply.iter().take(8).map(line));
    if triage.reply.len() > 8 {
        lines.push(format!("…and {} more.", triage.reply.len() - 8));
    }
    if !triage.reply.is_empty() && triage.unopened > 0 {
        lines.push(format!("I only open the newest {READ_AT_MOST} emails sent to you, so the {unopened} may need a reply too:"));
        list_unread(&mut lines);
    }
    if !others.is_empty() {
        lines.push(format!(
            "{} from people that may not need an answer:",
            if others.len() == 1 { "One more".to_string() } else { format!("{} more", others.len()) }
        ));
        lines.extend(others.iter().take(5).map(|item| line(item)));
    }
    if triage.automated > 0 {
        lines.push(format!(
            "I skipped {} automated {}.",
            triage.automated,
            if triage.automated == 1 { "message" } else { "messages" }
        ));
    }
    if let Some((checked, total)) = triage.partial {
        lines.push(format!("I checked the newest {checked} of {total} unreplied messages, so older ones may be missing."));
    }
    lines.push(format!("This covers unreplied messages in your inbox from the last {DAYS} days."));
    lines.join("\n")
}

/// The count for the morning brief, e.g. "2 emails look like they need a reply: Ana, Sam."
pub fn brief_line(triage: &Triage) -> Option<String> {
    if triage.reply.is_empty() {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    for item in &triage.reply {
        let name = sender(&item.header);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    Some(format!(
        "{} look{} like {} a reply, from {shown}{}.",
        if triage.reply.len() == 1 { "One email".to_string() } else { format!("{} emails", triage.reply.len()) },
        if triage.reply.len() == 1 { "s" } else { "" },
        if triage.reply.len() == 1 { "it needs" } else { "they need" },
        if names.len() > 3 { " and others" } else { "" }
    ))
}

/// Reads Mail and sorts it. Mail must be allowed in Settings → Apps. With
/// `Launch::IfRunning` a closed Mail stays closed and the read fails with
/// `apps::NOT_RUNNING`.
pub async fn run(launch: crate::apps::Launch) -> Result<Triage, String> {
    let (headers, total) = crate::apps::unreplied_mail(DAYS, launch).await?;
    let openings = crate::apps::mail_openings(&candidates(&headers), launch).await?;
    Ok(sort(headers, total, &openings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(id: &str, sender: &str, to_me: Option<bool>, to_count: usize, flagged: bool) -> MailHeader {
        MailHeader {
            received: format!("2026-10-2{id} 09:00"),
            id: id.into(),
            read: false,
            flagged,
            to_me,
            to_count,
            sender: sender.into(),
            subject: format!("Subject {id}"),
        }
    }

    #[test]
    fn automated_senders_are_recognised_by_address_not_name() {
        for sender in [
            "Shop <noreply@shop.com>",
            "no-reply@accounts.example.com",
            "GitHub <notifications@github.com>",
            "news@paper.com",
        ] {
            assert!(automated(sender), "{sender}");
        }
        for sender in ["Ana Silva <ana@example.com>", "Newsletter Fan <sam@example.com>", "noah@example.com"] {
            assert!(!automated(sender), "{sender}");
        }
    }

    #[test]
    fn people_who_ask_need_a_reply_and_the_rest_is_explained() {
        let headers = vec![
            header("1", "Ana <ana@x.com>", Some(true), 1, false),
            header("2", "Sam <sam@x.com>", Some(true), 1, false),
            header("3", "Team <team@x.com>", Some(false), 40, false),
            header("4", "noreply@shop.com", None, 1, false),
            header("5", "Lee <lee@x.com>", Some(false), 1, true),
        ];
        assert_eq!(candidates(&headers), ["1", "2"]);
        let openings = HashMap::from([
            ("1".to_string(), "Could you sign the lease by Friday".to_string()),
            ("2".to_string(), "Here are the photos from the trip.".to_string()),
        ]);
        let triage = sort(headers, 9, &openings);
        let ids = |items: &[Item]| items.iter().map(|i| i.header.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&triage.reply), ["1", "5"]);
        assert_eq!(ids(&triage.look), ["2", "3"]);
        assert_eq!(triage.automated, 1);
        assert_eq!(triage.partial, Some((5, 9)));
        let text = compose(&triage);
        assert!(text.starts_with("2 emails look like they need a reply:"));
        assert!(text.contains("• Ana — “Subject 1” (Wed 21 Oct): asks you something"));
        assert!(text.contains("I skipped 1 automated message."));
        assert!(text.contains("newest 5 of 9"));
        assert_eq!(
            brief_line(&triage).as_deref(),
            Some("2 emails look like they need a reply, from Ana, Lee.")
        );
    }

    #[test]
    fn emails_past_the_reading_limit_are_never_called_nothing() {
        // Thirteen people wrote directly; only the oldest, which is past the limit, asks.
        let headers: Vec<MailHeader> = (1..=13)
            .map(|n| MailHeader {
                received: format!("2026-10-22 {:02}:00", 23 - n),
                ..header(&n.to_string(), &format!("P{n} <p{n}@x.com>"), Some(true), 1, false)
            })
            .collect();
        let read = candidates(&headers);
        assert_eq!(read.len(), READ_AT_MOST);
        assert!(!read.contains(&"13".to_string()));
        let openings: HashMap<String, String> = read.iter().map(|id| (id.clone(), "Photos from the trip.".to_string())).collect();
        let triage = sort(headers, 13, &openings);
        assert!(triage.reply.is_empty());
        assert_eq!(triage.unopened, 1);
        assert_eq!(triage.partial, None, "Mail returned every unreplied message");
        let text = compose(&triage);
        assert!(!text.starts_with("Nothing"), "{text}");
        assert!(text.contains("newest 12") && text.contains("1 email sent to you that I didn’t open"), "{text}");
        assert_eq!(triage.look.last().unwrap().reason, "sent to you, not opened");
        // The one to check is named, and not counted among those that need no answer.
        assert!(text.contains("P13"), "{text}");
        assert!(text.contains("12 more from people that may not need an answer"), "{text}");
        // With a reply found as well, the limit is still stated, and the email named.
        let mut both = triage.clone();
        both.reply.push(both.look.remove(0));
        let text = compose(&both);
        assert!(text.contains("may need a reply too") && text.contains("P13"), "{text}");
    }

    #[test]
    fn an_empty_inbox_says_so_and_adds_nothing_to_the_brief() {
        let triage = sort(vec![], 0, &HashMap::new());
        assert!(compose(&triage).starts_with("Nothing in your inbox from the last 7 days"));
        assert_eq!(brief_line(&triage), None);
    }

    #[test]
    fn unknown_recipients_fall_back_to_a_small_to_list() {
        assert!(direct(&header("1", "a@x.com", None, 2, false)));
        assert!(!direct(&header("1", "a@x.com", None, 8, false)));
        assert!(!direct(&header("1", "a@x.com", Some(true), 30, false)));
    }
}
