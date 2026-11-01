//! "What needs a reply?": unreplied inbox email sorted into what likely needs an answer,
//! by fixed rules rather than a model, so every brain gets the same, explainable result.
//! A message needs a reply when a person (not an automated sender) wrote to the user
//! directly and asked something; flagged messages count too. Everything else from people
//! is listed as worth a look; automated mail is only counted.
//!
//! The emails an answer names are numbered and saved with that turn, so "reply to the
//! second one" or "reply to Ana's email" afterwards answers exactly that email.
use crate::apps::MailHeader;
use serde::{Deserialize, Serialize};
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

fn day(received: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(received, "%Y-%m-%d %H:%M")
        .map(|at| at.format("%a %-d %b").to_string())
        .unwrap_or_else(|_| received.to_string())
}

fn line(number: usize, item: &Item) -> String {
    let subject = if item.header.subject.is_empty() { "(no subject)" } else { &item.header.subject };
    format!("{number}. {} — “{}” ({}): {}", sender(&item.header), subject, day(&item.header.received), item.reason)
}

/// The emails an answer names, in the order it numbers them: those that need a reply,
/// then the ones it didn't open, then the rest from people.
fn shown(triage: &Triage) -> Vec<&Item> {
    let (unread, others): (Vec<&Item>, Vec<&Item>) =
        triage.look.iter().partition(|item| item.reason == UNOPENED);
    triage
        .reply
        .iter()
        .take(8)
        .chain(unread.into_iter().take(5))
        .chain(others.into_iter().take(5))
        .collect()
}

/// The answer as Buntini says it. Each email it names has a number, the same one
/// `listed` saves for a follow-up.
pub fn compose(triage: &Triage) -> String {
    let mut lines = Vec::new();
    let mut number = 0;
    let mut numbered = |item: &Item| {
        number += 1;
        line(number, item)
    };
    let unopened = match triage.unopened {
        1 => "1 email sent to you that I didn’t open".to_string(),
        n => format!("{n} emails sent to you that I didn’t open"),
    };
    // The ones past the reading limit are named, so "check them" says which.
    let (unread, others): (Vec<&Item>, Vec<&Item>) =
        triage.look.iter().partition(|item| item.reason == UNOPENED);
    let list_unread = |lines: &mut Vec<String>, numbered: &mut dyn FnMut(&Item) -> String| {
        lines.extend(unread.iter().take(5).map(|item| numbered(item)));
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
        list_unread(&mut lines, &mut numbered);
    }
    lines.extend(triage.reply.iter().take(8).map(&mut numbered));
    if triage.reply.len() > 8 {
        lines.push(format!("…and {} more.", triage.reply.len() - 8));
    }
    if !triage.reply.is_empty() && triage.unopened > 0 {
        lines.push(format!("I only open the newest {READ_AT_MOST} emails sent to you, so the {unopened} may need a reply too:"));
        list_unread(&mut lines, &mut numbered);
    }
    if !others.is_empty() {
        lines.push(format!(
            "{} from people that may not need an answer:",
            if others.len() == 1 { "One more".to_string() } else { format!("{} more", others.len()) }
        ));
        lines.extend(others.iter().take(5).map(|item| numbered(item)));
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

/// One email an answer named, by its number there. The ids are Mail's own, read with
/// the rest of the header; a reply to it is checked against all of them again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub number: usize,
    pub id: String,
    pub message_id: String,
    pub sender: String,
    pub subject: String,
    pub received: String,
}

impl Listed {
    /// "Ana Silva · “Lease” · Wed 21 Oct", for the approval sheet.
    pub fn describe(&self) -> String {
        about(&self.sender, &self.subject, &self.received)
    }
}

/// An email in a few words, as the approval sheet names the one a reply answers.
pub fn about(sender: &str, subject: &str, received: &str) -> String {
    let name = match sender.split_once('<') {
        Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"'),
        _ => sender.trim(),
    };
    let subject = if subject.is_empty() { "(no subject)" } else { subject };
    format!("{name} · “{subject}” · {}", day(received))
}

/// The emails `compose` numbered, with those numbers.
pub fn listed(triage: &Triage) -> Vec<Listed> {
    shown(triage)
        .into_iter()
        .enumerate()
        .map(|(index, item)| Listed {
            number: index + 1,
            id: item.header.id.clone(),
            message_id: item.header.message_id.clone(),
            sender: item.header.sender.clone(),
            subject: item.header.subject.clone(),
            received: item.header.received.clone(),
        })
        .collect()
}

/// The latest list, saved beside the turn that showed it. One at a time: a new list
/// replaces it, and deleting that turn or the history removes it (store.rs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub task_id: String,
    pub listed_at: String,
    pub emails: Vec<Listed>,
}

/// The preference that holds the latest `Listing`.
pub const LISTING: &str = "mail-listing";
/// How long a list stays what "the second one" means. Mail changes; every reply is
/// checked against the email again anyway.
const LISTING_HOURS: i64 = 12;

pub fn save_listing(store: &crate::store::Store, task_id: &str, emails: Vec<Listed>) -> Result<(), String> {
    let listing = Listing { task_id: task_id.into(), listed_at: chrono::Utc::now().to_rfc3339(), emails };
    store.set_preference(LISTING, &serde_json::to_string(&listing).map_err(|e| e.to_string())?)
}

/// The saved list, while it is recent and its turn is one of `turns` (the earlier turns of
/// this conversation), so a list from another conversation never answers "the second one".
pub fn recent_listing(store: &crate::store::Store, turns: &[crate::types::TaskRecord]) -> Option<Listing> {
    let listing: Listing = serde_json::from_str(&store.preference(LISTING).ok()??).ok()?;
    current(listing, turns, chrono::Utc::now())
}

fn current(listing: Listing, turns: &[crate::types::TaskRecord], now: chrono::DateTime<chrono::Utc>) -> Option<Listing> {
    let at = chrono::DateTime::parse_from_rfc3339(&listing.listed_at).ok()?;
    (now.signed_duration_since(at) < chrono::Duration::hours(LISTING_HOURS)
        && turns.iter().any(|turn| turn.id == listing.task_id))
        .then_some(listing)
}

/// Which listed email a reply means, in the user's own words.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// "the second one", "number 2", "2"
    Number(usize),
    /// "the last one"
    Last,
    /// "Ana's email", "the email from Ana", "Ana"
    Sender(String),
}

const ORDINALS: [&str; 10] = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
/// Words that end a sender's name ("reply to Ana saying yes").
const AFTER_NAME: &[&str] = &[
    "saying", "and", "with", "that", "telling", "to", "about", "asking", "please", "thanking",
    "then", "but", "so", "in", "for", "letting", "yes", "no", "email", "emails", "message",
    "mail", "one",
];
/// Words after "reply to" that are never a sender's name.
const NOT_A_NAME: &[&str] = &[
    "this", "that", "it", "her", "him", "them", "the", "my", "me", "these", "those", "all",
    "everyone", "everybody", "email", "message", "mail", "emails", "messages", "one", "a", "an",
    "back", "you", "us", "following",
];

fn number(word: &str) -> Option<usize> {
    let word = word.trim_end_matches(['.', ':']);
    ORDINALS
        .iter()
        .position(|ordinal| *ordinal == word)
        .map(|index| index + 1)
        .or_else(|| {
            let digits = word.trim_end_matches("st").trim_end_matches("nd").trim_end_matches("rd").trim_end_matches("th");
            digits.parse::<usize>().ok().filter(|n| (1..=99).contains(n))
        })
}

/// "Reply to the second one saying yes", "respond to number 3", "reply to Ana's email",
/// "write back to the email from Sam". None for "reply to this" or "reply to her",
/// which mean the email on screen.
pub fn pick(prompt: &str) -> Option<Pick> {
    let text = crate::skills::words(prompt);
    let (rest, true) = crate::focus::reply_object(&text)? else {
        return None;
    };
    let words: Vec<&str> = rest.split(' ').collect();
    let mut at = usize::from(words.first().is_some_and(|word| ["the", "this", "that"].contains(word)));
    if words.get(at) == Some(&"last") {
        return Some(Pick::Last);
    }
    if ["number", "no", "email", "message"].contains(words.get(at).unwrap_or(&"")) && words.get(at + 1).and_then(|w| number(w)).is_some() {
        at += 1;
    }
    if let Some(n) = words.get(at).and_then(|w| number(w)) {
        return Some(Pick::Number(n));
    }
    // "the email from Ana", "the one from Ana"
    if at == 1 && ["email", "message", "mail", "one"].contains(words.get(1).unwrap_or(&"")) && words.get(2) == Some(&"from") {
        at = 3;
    } else if at == 1 {
        return None;
    }
    let mut name = Vec::new();
    for word in &words[at..] {
        if let Some(owner) = word.strip_suffix("'s") {
            name.push(owner);
            break;
        }
        if AFTER_NAME.contains(word) || name.len() == 3 {
            break;
        }
        name.push(word);
    }
    match name.first() {
        Some(first) if !NOT_A_NAME.contains(first) && first.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'') => {
            Some(Pick::Sender(name.join(" ")))
        }
        _ => None,
    }
}

/// The one listed email `pick` means, or what to tell the user when it means none or
/// several.
pub fn find<'a>(pick: &Pick, emails: &'a [Listed]) -> Result<&'a Listed, String> {
    match pick {
        Pick::Number(n) => emails.iter().find(|email| email.number == *n).ok_or_else(|| match emails.len() {
            1 => "I listed only one email.".to_string(),
            count => format!("I listed {count} emails, so there’s no number {n}."),
        }),
        Pick::Last => emails.last().ok_or_else(|| "I didn’t list any emails.".to_string()),
        Pick::Sender(name) => {
            let wanted: Vec<&str> = name.split(' ').collect();
            let from = |email: &&Listed| from_sender(name, &email.sender);
            let shown: String = wanted
                .iter()
                .map(|word| {
                    let mut letters = word.chars();
                    letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
                })
                .collect::<Vec<String>>()
                .join(" ");
            let found: Vec<&Listed> = emails.iter().filter(from).collect();
            match found[..] {
                [one] => Ok(one),
                [] => Err(format!("None of the emails I listed is from {shown}.")),
                _ => Err(format!("More than one email I listed is from {shown}. Say which one by its number.")),
            }
        }
    }
}

/// Whether `sender` ("Ana Silva <ana@example.com>") is who the user named ("ana", "ana
/// silva").
pub fn from_sender(name: &str, sender: &str) -> bool {
    let wanted: Vec<&str> = name.split_whitespace().collect();
    if wanted.is_empty() {
        return false;
    }
    let words = |text: &str| -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    // Only the name a sender shows counts, never their address's domain ("bob@ana.pt" is
    // not Ana), and a name relayed by a service ("Ana (via Google Docs)") is the service.
    let display = match sender.rsplit_once('<') {
        Some((name, _)) => words(name.trim().trim_matches('"')),
        None if crate::focus::sender_address(sender).is_some() => vec![],
        None => words(sender),
    };
    if display.iter().any(|word| word == "via") {
        return false;
    }
    let local = crate::focus::sender_address(sender)
        .and_then(|address| address.split('@').next().map(str::to_string))
        .unwrap_or_default();
    let local_words: Vec<String> = local.split(['.', '_', '-', '+']).filter(|w| !w.is_empty()).map(str::to_string).collect();
    wanted.iter().all(|word| display.iter().any(|shown| shown == word))
        || (display.is_empty() && wanted.iter().all(|word| local_words.iter().any(|part| part == word)))
        || (wanted.len() == 1 && local == wanted[0])
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
    fill(|| read(launch)).await
}

async fn read(launch: crate::apps::Launch) -> Result<Triage, String> {
    let (headers, total) = crate::apps::unreplied_mail(DAYS, launch).await?;
    let openings = crate::apps::mail_openings(&candidates(&headers), launch).await?;
    Ok(sort(headers, total, &openings))
}

/// Sorts with `read` and keeps the result, unless history was cleared or everything
/// deleted while it read: then what it read is not kept.
async fn fill<F, Fut>(read: F) -> Result<Triage, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Triage, String>>,
{
    let generation = GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    let triage = read().await?;
    let mut cache = CACHE.lock().unwrap();
    if GENERATION.load(std::sync::atomic::Ordering::SeqCst) == generation {
        *cache = Some((std::time::Instant::now(), triage.clone()));
    }
    Ok(triage)
}

/// The last sort, kept in memory only, so heads-ups about people waiting on a reply
/// reuse what the user (or an earlier check) already read instead of asking Mail again.
static CACHE: std::sync::Mutex<Option<(std::time::Instant, Triage)>> = std::sync::Mutex::new(None);
/// Moves on each time the cache is forgotten, so a read that began before never refills it.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The last sort while it is younger than `fresh`, else a new one read with `launch`.
pub async fn cached(launch: crate::apps::Launch, fresh: std::time::Duration) -> Result<Triage, String> {
    cached_with(fresh, || read(launch)).await
}

async fn cached_with<F, Fut>(fresh: std::time::Duration, read: F) -> Result<Triage, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Triage, String>>,
{
    if let Some((at, triage)) = CACHE.lock().unwrap().as_ref() {
        if at.elapsed() < fresh {
            return Ok(triage.clone());
        }
    }
    fill(read).await
}

/// Whether the cache was forgotten since `generation`.
pub fn forgotten_since(generation: u64) -> bool {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation
}
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// Erasing data or clearing history forgets the cached sort with it.
pub fn forget_cache() {
    let mut cache = CACHE.lock().unwrap();
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *cache = None;
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
            message_id: format!("m-{id}@x.com"),
        }
    }

    #[test]
    fn a_name_matches_what_a_sender_shows_never_their_domain_or_a_relay() {
        assert!(from_sender("ana", "Ana Silva <ana@example.com>"));
        assert!(from_sender("ana silva", "\"Ana Silva\" <a@example.com>"));
        assert!(from_sender("ana", "ana@example.com"));
        assert!(from_sender("ana silva", "ana.silva@example.com"));
        assert!(!from_sender("ana", "Pedro <pedro@ana.pt>"));
        assert!(!from_sender("mark", "Deals <offers@mark.shop>"));
        assert!(!from_sender("ana", "pedro@ana.pt"));
        assert!(!from_sender("ana", "Ana Silva (via Google Docs) <drive-shares-dm-noreply@google.com>"));
        assert!(!from_sender("ana", "Ana Silva via LinkedIn <invitations@linkedin.com>"));
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
        assert!(text.contains("1. Ana — “Subject 1” (Wed 21 Oct): asks you something"));
        // Every email named has the number a follow-up can use, and the same is saved.
        assert!(text.contains("2. Lee — “Subject 5”") && text.contains("3. Sam — “Subject 2”") && text.contains("4. Team — “Subject 3”"), "{text}");
        let listed = listed(&triage);
        assert_eq!(
            listed.iter().map(|email| (email.number, email.id.as_str())).collect::<Vec<_>>(),
            [(1, "1"), (2, "5"), (3, "2"), (4, "3")]
        );
        assert_eq!(listed[0].message_id, "m-1@x.com");
        assert_eq!(listed[0].describe(), "Ana · “Subject 1” · Wed 21 Oct");
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
    fn unopened_emails_are_numbered_where_they_are_shown() {
        let headers = vec![
            header("1", "Ana <ana@x.com>", Some(true), 1, false),
            header("2", "Team <team@x.com>", Some(false), 40, false),
        ];
        // Nothing was opened, so Ana's email comes first, before the one to several people.
        let triage = sort(headers, 2, &HashMap::new());
        let text = compose(&triage);
        assert!(text.contains("1. Ana — ") && text.contains("2. Team — "), "{text}");
        assert_eq!(listed(&triage).iter().map(|email| email.id.as_str()).collect::<Vec<_>>(), ["1", "2"]);
    }

    fn email(number: usize, sender: &str) -> Listed {
        Listed {
            number,
            id: format!("{number}0"),
            message_id: format!("m-{number}@x.com"),
            sender: sender.into(),
            subject: format!("Subject {number}"),
            received: "2026-10-21 09:00".into(),
        }
    }

    #[test]
    fn follow_ups_pick_a_listed_email_by_number_or_sender() {
        for (prompt, expected) in [
            ("Reply to the second one saying yes", Pick::Number(2)),
            ("reply to number 3: thanks", Pick::Number(3)),
            ("Please respond to the first email and say no", Pick::Number(1)),
            ("reply to 2 saying I'll call", Pick::Number(2)),
            ("reply to the 4th one", Pick::Number(4)),
            ("reply to the last one", Pick::Last),
            ("Reply to Ana's email saying yes", Pick::Sender("ana".into())),
            ("reply to ana silva's message", Pick::Sender("ana silva".into())),
            ("write back to the email from Sam saying thanks", Pick::Sender("sam".into())),
            ("reply to Lee saying I'll be there", Pick::Sender("lee".into())),
        ] {
            assert_eq!(pick(prompt), Some(expected), "{prompt}");
        }
        for prompt in [
            "reply to this saying yes",
            "reply to her saying yes",
            "Reply to the email saying yes",
            "reply to it",
            "don't reply to the second one",
            "how do I reply to Ana",
            "reply in French",
            "what needs a reply?",
        ] {
            assert_eq!(pick(prompt), None, "{prompt}");
        }
        let emails = vec![email(1, "Ana Silva <ana@x.com>"), email(2, "Sam <sam@x.com>"), email(3, "Ana Lopes <al@y.com>"), email(4, "lee@z.com")];
        assert_eq!(find(&Pick::Number(2), &emails).unwrap().id, "20");
        assert_eq!(find(&Pick::Last, &emails).unwrap().id, "40");
        assert_eq!(find(&Pick::Sender("sam".into()), &emails).unwrap().id, "20");
        assert_eq!(find(&Pick::Sender("ana silva".into()), &emails).unwrap().id, "10");
        assert_eq!(find(&Pick::Sender("lee".into()), &emails).unwrap().id, "40");
        assert_eq!(find(&Pick::Number(9), &emails).unwrap_err(), "I listed 4 emails, so there’s no number 9.");
        assert_eq!(find(&Pick::Sender("ana".into()), &emails).unwrap_err(), "More than one email I listed is from Ana. Say which one by its number.");
        assert_eq!(find(&Pick::Sender("noah".into()), &emails).unwrap_err(), "None of the emails I listed is from Noah.");
    }

    #[test]
    fn a_list_counts_only_in_its_own_recent_conversation() {
        let now = chrono::Utc::now();
        let listing = Listing { task_id: "triage".into(), listed_at: now.to_rfc3339(), emails: vec![email(1, "ana@x.com")] };
        let turn = |id: &str| -> crate::types::TaskRecord {
            serde_json::from_value(serde_json::json!({"id":id,"prompt":"What needs a reply?","answer":"1. Ana","state":"completed","createdAt":"2026-10-30T09:00:00Z","sources":[],"files":[]})).unwrap()
        };
        assert!(current(listing.clone(), &[turn("triage")], now).is_some());
        assert!(current(listing.clone(), &[turn("other")], now).is_none(), "another conversation's list");
        assert!(current(listing, &[turn("triage")], now + chrono::Duration::hours(13)).is_none(), "an old list");
        // The list goes with its turn and with the history.
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        save_listing(&store, "triage", vec![email(1, "ana@x.com")]).unwrap();
        assert!(recent_listing(&store, &[turn("triage")]).is_some());
        store.delete_task("other").unwrap();
        assert!(store.preference(LISTING).unwrap().is_some());
        store.delete_task("triage").unwrap();
        assert!(store.preference(LISTING).unwrap().is_none());
        save_listing(&store, "triage", vec![email(1, "ana@x.com")]).unwrap();
        store.clear_history().unwrap();
        assert!(store.preference(LISTING).unwrap().is_none());
    }

    #[test]
    fn an_empty_inbox_says_so_and_adds_nothing_to_the_brief() {
        let triage = sort(vec![], 0, &HashMap::new());
        assert!(compose(&triage).starts_with("Nothing in your inbox from the last 7 days"));
        assert_eq!(brief_line(&triage), None);
    }

    #[tokio::test]
    async fn the_cached_sort_is_reused_while_fresh_and_forgotten_on_erase() {
        // Never reads Mail: the read is a stand-in, so other tests clearing history at
        // the same time can't send this one to the real inbox.
        let triage = sort(vec![header("1", "Ana <ana@x.com>", Some(true), 1, true)], 1, &HashMap::new());
        let before = generation();
        *CACHE.lock().unwrap() = Some((std::time::Instant::now(), triage.clone()));
        let called = std::sync::atomic::AtomicBool::new(false);
        let reused = cached_with(std::time::Duration::from_secs(60), || async {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(triage.clone())
        })
        .await
        .unwrap();
        assert_eq!(reused, triage);
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst) || forgotten_since(before), "a fresh sort is reused");
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        store.clear_history().unwrap();
        assert!(CACHE.lock().unwrap().is_none(), "clearing history forgets the sort");
        *CACHE.lock().unwrap() = Some((std::time::Instant::now(), triage.clone()));
        store.erase_everything().unwrap();
        assert!(CACHE.lock().unwrap().is_none(), "erasing forgets the sort");
        // A read that was running while everything was deleted doesn't bring the sort back.
        let read = fill(|| async {
            forget_cache();
            Ok(triage.clone())
        })
        .await
        .unwrap();
        assert_eq!(read, triage);
        assert!(CACHE.lock().unwrap().is_none(), "what was read before the erase isn't kept");
    }

    #[test]
    fn unknown_recipients_fall_back_to_a_small_to_list() {
        assert!(direct(&header("1", "a@x.com", None, 2, false)));
        assert!(!direct(&header("1", "a@x.com", None, 8, false)));
        assert!(!direct(&header("1", "a@x.com", Some(true), 30, false)));
    }
}
