//! Sorting by fixed rules, so every brain sorts the same way and each move can say why.
//! The order follows Coucou's MailTriage (MIT): a VIP is flagged and never moved; then a
//! banned word junks; then the person's rules in their order. The first that matches decides.
//! Matching ignores case and accents. Banned words match whole words or phrases, so "sale"
//! never catches "salesforce"; an entry with `@` or `.` matches as written.
use serde::{Deserialize, Serialize};

use super::{Mailbox, Message};

pub const MAX_RULES: usize = 100;
pub const MAX_WORDS: usize = 200;
const MAX_VALUE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    /// The sender's name or address.
    Sender,
    /// The sender's domain, subdomains included.
    Domain,
    Subject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleAction {
    Move,
    Flag,
    Read,
    Junk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub field: Field,
    pub value: String,
    pub action: RuleAction,
    /// Where `Move` puts it.
    #[serde(default)]
    pub to: Option<Mailbox>,
    #[serde(default = "on")]
    pub enabled: bool,
}

fn on() -> bool {
    true
}

/// What sorting would do to one email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Flag,
    Read,
    Junk,
    Move(Mailbox),
}

/// Why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Why {
    /// The person did it from the island or the app.
    Manual,
    Vip,
    Banned { word: String },
    Rule { id: String },
    Plan,
    Undo { of: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub verdict: Verdict,
    pub why: Why,
}

/// Lower case without accents, for matching.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' | 'đ' => 'd',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
            'ś' | 'š' | 'ş' => 's',
            'ť' | 'ţ' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            'ł' => 'l',
            'ß' => 's',
            other => other,
        })
        .collect()
}

fn words(text: &str) -> Vec<String> {
    fold(text).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Whether `phrase` appears in `text` as whole words, in order.
pub fn has_phrase(text: &str, phrase: &str) -> bool {
    let wanted = words(phrase);
    if wanted.is_empty() {
        return false;
    }
    words(text).windows(wanted.len()).any(|window| window == wanted.as_slice())
}

/// A banned entry with `@` or `.` is an address or domain, matched as written.
fn banned_hit(entry: &str, sender: &str, subject: &str) -> bool {
    if entry.contains(['@', '.']) {
        let entry = fold(entry.trim());
        !entry.is_empty() && (fold(sender).contains(&entry) || fold(subject).contains(&entry))
    } else {
        has_phrase(sender, entry) || has_phrase(subject, entry)
    }
}

/// The sender's address, lower case.
pub fn address(sender: &str) -> String {
    crate::focus::sender_address(sender).unwrap_or_else(|| fold(sender.trim()))
}

fn display_name(sender: &str) -> String {
    match sender.rsplit_once('<') {
        Some((name, _)) => fold(name.trim().trim_matches('"').trim()),
        None => String::new(),
    }
}

/// Whether the sender's address is at `domain` or one of its subdomains.
fn at_domain(sender: &str, domain: &str) -> bool {
    let domain = fold(domain.trim().trim_start_matches('@'));
    if domain.is_empty() {
        return false;
    }
    let address = address(sender);
    let host = address.rsplit_once('@').map_or("", |(_, host)| host);
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// A VIP is a full address, a domain (`@example.com` or `example.com`) or a name.
pub fn is_vip(vips: &[String], sender: &str) -> bool {
    vips.iter().any(|vip| {
        let vip = vip.trim();
        if vip.is_empty() {
            false
        } else if vip.contains('@') && !vip.starts_with('@') {
            address(sender) == fold(vip)
        } else if vip.contains('.') || vip.starts_with('@') {
            at_domain(sender, vip)
        } else {
            display_name(sender) == fold(vip)
        }
    })
}

fn rule_hit(rule: &Rule, sender: &str, subject: &str) -> bool {
    let value = fold(rule.value.trim());
    if !rule.enabled || value.is_empty() {
        return false;
    }
    match rule.field {
        Field::Sender => fold(sender).contains(&value),
        Field::Domain => at_domain(sender, &value),
        Field::Subject => fold(subject).contains(&value),
    }
}

/// What sorting does to an email from `sender` about `subject`, if anything.
pub fn decide(settings: &super::Settings, sender: &str, subject: &str) -> Option<Decision> {
    if is_vip(&settings.vips, sender) {
        return Some(Decision { verdict: Verdict::Flag, why: Why::Vip });
    }
    if let Some(word) = settings.banned.iter().find(|word| banned_hit(word, sender, subject)) {
        return Some(Decision { verdict: Verdict::Junk, why: Why::Banned { word: word.trim().to_string() } });
    }
    let rule = settings.rules.iter().find(|rule| rule_hit(rule, sender, subject))?;
    let verdict = match rule.action {
        RuleAction::Flag => Verdict::Flag,
        RuleAction::Read => Verdict::Read,
        RuleAction::Junk => Verdict::Junk,
        RuleAction::Move => Verdict::Move(rule.to.clone()?),
    };
    Some(Decision { verdict, why: Why::Rule { id: rule.id.clone() } })
}

/// What sorting would change among `messages`, leaving out what is already so.
pub fn sort(settings: &super::Settings, messages: &[Message]) -> Vec<(Message, Decision)> {
    messages
        .iter()
        .filter_map(|message| {
            let decision = decide(settings, &message.sender, &message.subject)?;
            let already = match &decision.verdict {
                Verdict::Flag => message.flagged,
                Verdict::Read => message.read,
                Verdict::Junk => message.junk,
                Verdict::Move(to) => same_box(to, &message.mailbox),
            };
            (!already).then(|| (message.clone(), decision))
        })
        .collect()
}

pub fn same_box(a: &Mailbox, b: &Mailbox) -> bool {
    a.account == b.account && a.path.eq_ignore_ascii_case(&b.path)
}

/// Mailboxes nothing is ever moved into: mail there is sent, waits to be sent, is a draft, or
/// is on its way to being deleted.
const NOT_A_DESTINATION: &[&str] = &[
    "outbox", "drafts", "draft", "sent", "sent messages", "sent mail", "sent items", "sendlater", "send later", "trash",
    "deleted messages", "deleted items", "bin", "recently deleted", "notes",
];

/// Whether moving an email into `mailbox` would send it, make it a draft or delete it.
pub fn forbidden_destination(mailbox: &Mailbox) -> bool {
    let leaf = fold(super::log::leaf(&mailbox.path));
    NOT_A_DESTINATION.contains(&leaf.trim())
}

fn plain(value: &str, what: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > MAX_VALUE || value.chars().any(char::is_control) {
        return Err(format!("Each {what} needs 1 to {MAX_VALUE} characters on one line."));
    }
    Ok(())
}

pub fn validate(settings: &super::Settings) -> Result<(), String> {
    if settings.rules.len() > MAX_RULES {
        return Err(format!("Mail keeps at most {MAX_RULES} rules."));
    }
    if settings.banned.len() > MAX_WORDS || settings.vips.len() > MAX_WORDS {
        return Err(format!("Mail keeps at most {MAX_WORDS} banned words and {MAX_WORDS} VIPs."));
    }
    for word in &settings.banned {
        plain(word, "banned word")?;
    }
    for vip in &settings.vips {
        plain(vip, "VIP")?;
    }
    let mut ids = std::collections::HashSet::new();
    for rule in &settings.rules {
        if rule.id.is_empty() || rule.id.len() > 64 || !ids.insert(rule.id.as_str()) {
            return Err("Each mail rule needs its own id.".into());
        }
        plain(&rule.value, "rule")?;
        match (&rule.action, &rule.to) {
            (RuleAction::Move, Some(to)) => {
                plain(&to.path, "mailbox")?;
                if forbidden_destination(to) {
                    return Err(format!("Mail can’t be sorted into {}.", to.path));
                }
                if !to.account.is_empty() {
                    plain(&to.account, "account")?;
                }
            }
            (RuleAction::Move, None) => return Err("A rule that moves mail needs a mailbox.".into()),
            (_, Some(_)) => return Err("Only a rule that moves mail has a mailbox.".into()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::Settings;

    fn rule(id: &str, field: Field, value: &str, action: RuleAction, to: Option<&str>) -> Rule {
        Rule {
            id: id.into(),
            field,
            value: value.into(),
            action,
            to: to.map(|path| Mailbox { account: "iCloud".into(), path: path.into() }),
            enabled: true,
        }
    }

    fn message(sender: &str, subject: &str) -> Message {
        Message {
            id: 1,
            header: "h@x".into(),
            at: 0,
            read: false,
            flagged: false,
            junk: false,
            sender: sender.into(),
            subject: subject.into(),
            mailbox: Mailbox { account: "iCloud".into(), path: "INBOX".into() },
        }
    }

    fn settings() -> Settings {
        Settings {
            vips: vec!["boss@work.example.com".into(), "@family.example".into(), "Ana Ruiz".into()],
            banned: vec!["lottery".into(), "free money".into(), "spam.example.net".into()],
            rules: vec![
                rule("news", Field::Domain, "news.example.com", RuleAction::Move, Some("Newsletters")),
                rule("receipts", Field::Subject, "receipt", RuleAction::Read, None),
                rule("bank", Field::Sender, "Bank Alerts", RuleAction::Flag, None),
                rule("all-news", Field::Subject, "this week", RuleAction::Junk, None),
            ],
        }
    }

    #[test]
    fn a_vip_is_flagged_and_never_moved_or_junked() {
        let settings = settings();
        for sender in ["Boss <BOSS@work.example.com>", "Mum <mum@home.family.example>", "Ana Ruiz <ana@example.org>"] {
            let decision = decide(&settings, sender, "You won the lottery, this week").unwrap();
            assert_eq!(decision, Decision { verdict: Verdict::Flag, why: Why::Vip }, "{sender}");
        }
        // A name is matched exactly, not as part of another.
        assert!(!is_vip(&settings.vips, "Ana Ruizz <x@y.com>"));
        assert!(!is_vip(&settings.vips, "Boss <boss@work.example.com.evil.io>"));
        assert!(!is_vip(&settings.vips, "<someone@notfamily.example>"));
    }

    #[test]
    fn banned_words_match_whole_words_and_ignore_case_and_accents() {
        let settings = settings();
        let junk = |sender: &str, subject: &str| decide(&settings, sender, subject).map(|d| d.verdict);
        assert_eq!(junk("Prize <a@b.com>", "You won the LOTTERY!!!"), Some(Verdict::Junk));
        assert_eq!(junk("Prize <a@b.com>", "Frée   Money inside"), Some(Verdict::Junk));
        assert_eq!(junk("Prize <winner@spam.example.net>", "Hello"), Some(Verdict::Junk));
        assert_eq!(junk("Lotteryville Council <a@b.com>", "Meeting"), None);
        assert_eq!(junk("Shop <a@b.com>", "freemoney"), None);
        let why = decide(&settings, "a@b.com", "lottery").unwrap().why;
        assert_eq!(why, Why::Banned { word: "lottery".into() });
    }

    #[test]
    fn rules_apply_in_order_and_the_first_match_decides() {
        let settings = settings();
        // Matches both "news" (domain) and "all-news" (subject): the first rule wins.
        let decision = decide(&settings, "Weekly <newsletter@mail.news.example.com>", "This week in tech").unwrap();
        assert_eq!(decision.verdict, Verdict::Move(Mailbox { account: "iCloud".into(), path: "Newsletters".into() }));
        assert_eq!(decision.why, Why::Rule { id: "news".into() });
        assert_eq!(decide(&settings, "Shop <a@shop.com>", "Your Receipt #42").unwrap().verdict, Verdict::Read);
        assert_eq!(decide(&settings, "Bank Alerts <alerts@bank.com>", "Statement").unwrap().verdict, Verdict::Flag);
        // A domain rule never matches a look-alike domain.
        assert_eq!(decide(&settings, "x <a@fakenews.example.com.io>", "hi"), None);
        assert_eq!(decide(&settings, "Someone <a@b.com>", "Hello"), None);
    }

    #[test]
    fn a_disabled_or_empty_rule_never_matches() {
        let mut settings = settings();
        settings.rules[0].enabled = false;
        settings.rules[3].value = "  ".into();
        assert_eq!(decide(&settings, "Weekly <newsletter@news.example.com>", "This week in tech"), None);
    }

    #[test]
    fn sorting_leaves_out_what_is_already_so() {
        let settings = settings();
        let mut flagged = message("Boss <boss@work.example.com>", "Plan");
        flagged.flagged = true;
        let mut sorted = message("Weekly <newsletter@news.example.com>", "Tech");
        sorted.mailbox.path = "newsletters".into();
        let fresh = message("Weekly <newsletter@news.example.com>", "Tech");
        let changes = sort(&settings, &[flagged, sorted, fresh.clone(), message("x <a@b.com>", "hi")]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, fresh);
    }

    #[test]
    fn mail_is_never_moved_where_it_would_be_sent_drafted_or_deleted() {
        let at = |account: &str, path: &str| Mailbox { account: account.into(), path: path.into() };
        for path in ["Outbox", "Drafts", "Sent Messages", "[Gmail]/Sent Mail", "Trash", "[Gmail]/Trash", "Deleted Items", "SendLater", "Bin"] {
            assert!(forbidden_destination(&at("Work", path)), "{path}");
        }
        for path in ["INBOX", "Junk", "Archive", "Newsletters", "Work/Clients", "Sentences", "[Gmail]/All Mail"] {
            assert!(!forbidden_destination(&at("Work", path)), "{path}");
        }
        let mut settings = settings();
        settings.rules[0].to = Some(at("", "Outbox"));
        assert!(validate(&settings).is_err());
    }

    #[test]
    fn settings_are_checked() {
        let mut settings = settings();
        assert!(validate(&settings).is_ok());
        settings.rules[1].to = Some(Mailbox { account: String::new(), path: "X".into() });
        assert!(validate(&settings).is_err());
        let mut settings = self::settings();
        settings.rules[0].to = None;
        assert!(validate(&settings).is_err());
        let mut settings = self::settings();
        settings.rules[1].id = "news".into();
        assert!(validate(&settings).is_err());
        let mut settings = self::settings();
        settings.banned.push("two\nlines".into());
        assert!(validate(&settings).is_err());
    }

    #[test]
    fn settings_read_from_the_app() {
        let raw = serde_json::json!({"rules":[{"id":"r1","field":"domain","value":"news.example.com","action":"move","to":{"account":"","path":"Newsletters"}}],"banned":["lottery"],"vips":["ana@example.org"]});
        let settings: Settings = serde_json::from_value(raw).unwrap();
        assert!(settings.rules[0].enabled);
        assert!(validate(&settings).is_ok());
        assert!(serde_json::from_value::<Settings>(serde_json::json!({"rules":[{"id":"r","field":"body","value":"x","action":"flag"}]})).is_err());
    }
}
