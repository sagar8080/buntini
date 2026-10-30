//! What "this" means: the app, window and selection in front when the user opens Buntini.
//!
//! The app in front is tracked continuously (only its process id and names, in memory), so
//! a click on the pet still knows what the user was looking at after Buntini takes focus.
//! Opening Buntini takes a snapshot of that app's window title and selected text through
//! Accessibility. Anything app-specific (the email open in Mail, the page in a browser, the
//! files selected in Finder) is read only when a request actually refers to it, under that
//! app's own permission. Nothing here is written to disk; a task that uses it records the
//! data category it touched like any other read.
use crate::{
    privacy::{self, Locality, Share},
    types::Settings,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

/// A snapshot older than this no longer says what the user means by "this".
const FRESH: Duration = Duration::from_secs(600);
const MAX_SELECTION: usize = 8_000;
const MAIL: &str = "com.apple.mail";
const FINDER: &str = "com.apple.finder";

#[derive(Debug, Clone, PartialEq)]
pub struct Front {
    pub pid: i32,
    pub name: String,
    pub bundle: String,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub at: Instant,
    pub app: Front,
    pub window: String,
    pub selection: String,
    /// The focused field was a password field, so nothing was read from it.
    pub secure: bool,
}

/// What a request's "this" resolves to, most specific first.
#[derive(Debug, Clone, PartialEq)]
pub enum Referent {
    Selection {
        app: String,
        text: String,
        /// The email the text was selected in, read only for a reply to it, so "reply to
        /// her" still knows who "her" is.
        mail: Option<crate::apps::SelectedMail>,
    },
    Mail(crate::apps::SelectedMail),
    /// A browser page; the browser skill reads it, honouring private windows.
    Page { browser: &'static str, title: String },
    Finder { paths: Vec<String> },
    Window { app: String, title: String },
}

static LAST: Mutex<Option<Front>> = Mutex::new(None);
static SNAPSHOT: Mutex<Option<(u64, Option<Snapshot>)>> = Mutex::new(None);
static STARTED: AtomicU64 = AtomicU64::new(0);

fn frontmost() -> Option<Front> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    (pid != std::process::id() as i32).then(|| Front {
        pid,
        name: app.localizedName().map(|n| n.to_string()).unwrap_or_default(),
        bundle: app.bundleIdentifier().map(|b| b.to_string()).unwrap_or_default(),
    })
}

/// Follows the app in front twice a second, ignoring Buntini itself. Each check drains
/// its own autorelease pool: this thread never returns to a run loop that would.
pub fn start() {
    std::thread::spawn(|| loop {
        if let Some(front) = objc2::rc::autoreleasepool(|_| frontmost()) {
            *LAST.lock().unwrap() = Some(front);
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

/// Takes a snapshot of the app the user was just in. Call it as Buntini opens, before
/// its window takes focus; reading happens on a background thread.
pub fn capture() {
    let target = objc2::rc::autoreleasepool(|_| frontmost()).or_else(|| LAST.lock().unwrap().clone());
    if let Some(front) = &target {
        *LAST.lock().unwrap() = Some(front.clone());
    }
    let generation = STARTED.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        // Accessibility reads on a short-lived thread still leave autoreleased objects.
        let snapshot = objc2::rc::autoreleasepool(|_| target.map(|app| {
            let (window, selection, secure) = if crate::ax::trusted(false) {
                let read = crate::ax::selection(app.pid, MAX_SELECTION);
                (read.window, read.text, read.secure)
            } else {
                (String::new(), String::new(), false)
            };
            Snapshot { at: Instant::now(), app, window, selection, secure }
        }));
        let mut slot = SNAPSHOT.lock().unwrap();
        if slot.as_ref().is_none_or(|(done, _)| *done < generation) {
            *slot = Some((generation, snapshot));
        }
    });
}

/// The UI asks for a snapshot when the chat opens from the pet itself.
#[tauri::command]
pub fn capture_focus() {
    capture();
}

/// Whether a snapshot still says what "this" is: taken recently, and of the app the user
/// was in last. After a switch from Mail to Safari, the Mail snapshot no longer applies.
fn still_current(snapshot: &Snapshot, last: Option<&Front>) -> bool {
    snapshot.at.elapsed() < FRESH && last.is_none_or(|front| front.pid == snapshot.app.pid)
}

/// The latest fresh snapshot. Waits briefly for one still being taken, so a request typed
/// straight after the shortcut sees it.
pub async fn current() -> Option<Snapshot> {
    let wanted = STARTED.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let done = SNAPSHOT.lock().unwrap().clone();
        match done {
            Some((generation, snapshot)) if generation >= wanted => {
                let last = LAST.lock().unwrap().clone();
                return snapshot.filter(|s| still_current(s, last.as_ref()));
            }
            _ if wanted == 0 || Instant::now() >= deadline => return None,
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
}

/// Words after "this" that make it a time ("this weekend"), not something on screen.
const TIMES: &[&str] = &[
    "morning", "afternoon", "evening", "week", "weekend", "month", "year", "time", "season",
    "spring", "summer", "autumn", "fall", "winter", "quarter", "term", "semester", "monday",
    "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "week's", "month's",
    "year's", "weekend's",
];

/// Words before "him" or "her" that address the person on screen ("reply to her", "email
/// him"). Elsewhere "her" is usually possessive ("her birthday") or a third party.
const ADDRESSING: &[&str] = &[
    "reply", "respond", "answer", "email", "message", "text", "thank", "forward", "ping",
];

/// Whether a request points at something on screen ("this email", "these", "reply to
/// her"), not at a time ("this weekend") or someone in passing ("her birthday").
pub fn mentions_this(prompt: &str) -> bool {
    let text = crate::skills::words(prompt);
    let words: Vec<&str> = text.split(' ').collect();
    words.iter().enumerate().any(|(index, word)| {
        let before = |back: usize| index.checked_sub(back).map(|at| words[at]);
        match *word {
            "selection" | "selected" => true,
            // "this morning's emails" is a time too.
            "this" | "these" => words
                .get(index + 1)
                .is_none_or(|next| !TIMES.contains(&next.trim_end_matches("'s"))),
            "him" | "her" => {
                before(1).is_some_and(|word| ADDRESSING.contains(&word))
                    || (before(1) == Some("to")
                        && before(2).is_some_and(|word| ADDRESSING.contains(&word) || word == "back"))
            }
            _ => false,
        }
    })
}

/// Nouns after "this" that name something on screen ("this email", "this page").
const ON_SCREEN: &[&str] = &[
    "email", "mail", "message", "page", "tab", "site", "website", "article", "window", "file",
    "files", "folder", "document", "doc", "pdf", "screen", "selection", "image", "picture",
    "photo", "screenshot", "note", "link", "video", "thread", "chart", "table", "spreadsheet",
];

/// Words that may follow a bare "this" without giving it a meaning ("explain this please").
const FILLER: &[&str] = &["please", "for", "me", "now", "again", "quickly", "briefly", "up"];

/// Whether a request is plainly about what is on screen ("summarise this", "reply to
/// her", "what's on this page"), so it can't be answered without reading it. "Is this
/// sentence correct" or "how do I do this in Python" are not.
pub fn plainly_on_screen(prompt: &str) -> bool {
    if replies_to_this(prompt) || reminds_about_this(prompt) {
        return true;
    }
    let text = crate::skills::words(prompt);
    let words: Vec<&str> = text.split(' ').collect();
    words.iter().enumerate().any(|(index, word)| match *word {
        "selection" | "selected" => true,
        "this" | "these" => {
            words.get(index + 1).is_some_and(|next| ON_SCREEN.contains(next))
                || words[index + 1..].iter().all(|word| FILLER.contains(word))
        }
        _ => false,
    })
}

/// Whether the request itself holds what "this" means: text after a colon or in quotes
/// ("Translate this to Spanish: good morning").
fn carries_its_own_text(prompt: &str) -> bool {
    prompt
        .split_once(':')
        .is_some_and(|(_, rest)| rest.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        || prompt.chars().filter(|c| matches!(c, '"' | '“' | '”')).count() >= 2
}

/// Whether a request asks for a reminder about the thing on screen itself
/// ("remind me about this tomorrow at 9"), rather than one that merely mentions it.
pub fn reminds_about_this(prompt: &str) -> bool {
    let text = crate::skills::words(prompt);
    [
        "remind me about this",
        "remind me of this",
        "remind me to look at this",
        "remind me to read this",
        "add a reminder about this",
        "set a reminder about this",
        "add this to my reminders",
        "add this to reminders",
    ]
    .iter()
    .any(|phrase| {
        crate::skills::from_verb(&text, &["remind", "add", "set"]).is_some()
            && (text.starts_with(phrase) || text.contains(&format!(" {phrase}")))
    })
}

/// Whether a request asks to reply to the thing on screen ("reply to this saying yes",
/// "write back to her"), rather than to someone it names.
pub fn replies_to_this(prompt: &str) -> bool {
    let text = crate::skills::words(prompt);
    let Some((_, rest)) = crate::skills::from_verb(&text, &["reply", "respond", "write"]) else {
        return false;
    };
    let rest = rest.strip_prefix("back").map(str::trim_start).unwrap_or(rest);
    let rest = rest.strip_prefix("to ").unwrap_or(rest);
    // "Respond to this question in one sentence" is answered in chat, not replied to;
    // the same test decides whether the request needs a Mail draft at all.
    if crate::completion::answered_in_chat(&rest.split(' ').collect::<Vec<_>>()) {
        return false;
    }
    [
        "this", "this email", "this message", "this mail", "him", "her", "them", "it",
        "the email", "that email",
    ]
    .iter()
    .any(|target| rest == *target || rest.starts_with(&format!("{target} ")))
}

/// "Ana Silva <ana@example.com>" as "ana@example.com"; a bare address as itself.
pub fn sender_address(sender: &str) -> Option<String> {
    let address = match sender.rsplit_once('<') {
        Some((_, rest)) => rest.trim_end().trim_end_matches('>').trim(),
        None => sender.trim(),
    };
    let (user, domain) = address.split_once('@')?;
    (!user.is_empty() && domain.contains('.') && !address.contains(char::is_whitespace))
        .then(|| address.to_lowercase())
}

fn browser_id(bundle: &str) -> Option<&'static str> {
    match bundle {
        "com.apple.Safari" => Some("safari"),
        "com.google.Chrome" => Some("chrome"),
        "com.brave.Browser" => Some("brave"),
        "com.microsoft.edgemac" => Some("edge"),
        "company.thebrowser.Browser" => Some("arc"),
        _ => None,
    }
}

impl Snapshot {
    /// The browser this snapshot was taken in, if it was one.
    pub fn browser(&self) -> Option<&'static str> {
        browser_id(&self.app.bundle)
    }
}

/// A browser's window title and selection come from its page, so reading them needs the
/// browser allowed in Settings → Apps, with the same message the browser skill gives.
fn allowed_browser(id: &str, settings: &Settings) -> Result<&'static crate::apps::App, String> {
    let app = crate::apps::app(id).ok_or("Unknown browser.")?;
    if !crate::apps::allowed(settings, id) {
        return Err(format!("Allow {} in Settings → Apps so I can read its tabs.", app.name));
    }
    Ok(app)
}

/// Resolves "this" against a snapshot: selected text first, then the app's own object,
/// then just the window. App reads need that app to be allowed in Settings → Apps, and a
/// browser's private window is never read. `prompt` is the request, so a reply to a
/// selection in Mail can also read the email it belongs to.
pub async fn referent(
    snapshot: &Snapshot,
    settings: &Settings,
    prompt: &str,
) -> Result<Referent, String> {
    if let Some(browser) = snapshot.browser() {
        crate::apps::ensure_public_window(allowed_browser(browser, settings)?).await?;
    }
    if !snapshot.selection.is_empty() {
        let mail = if snapshot.app.bundle == MAIL
            && replies_to_this(prompt)
            && crate::apps::allowed(settings, "mail")
        {
            crate::apps::selected_mail().await?
        } else {
            None
        };
        return Ok(Referent::Selection {
            app: snapshot.app.name.clone(),
            text: snapshot.selection.clone(),
            mail,
        });
    }
    if snapshot.secure {
        return Err("That’s a password field, so I didn’t read it.".into());
    }
    match snapshot.app.bundle.as_str() {
        MAIL => {
            if !crate::apps::allowed(settings, "mail") {
                return Err("Allow Mail in Settings → Apps so I can read the email you have open.".into());
            }
            if let Some(mail) = crate::apps::selected_mail().await? {
                return Ok(Referent::Mail(mail));
            }
        }
        FINDER => {
            let paths = crate::apps::selected_finder_items().await?;
            if !paths.is_empty() {
                return Ok(Referent::Finder { paths });
            }
        }
        bundle => {
            if let Some(browser) = browser_id(bundle) {
                return Ok(Referent::Page { browser, title: snapshot.window.clone() });
            }
        }
    }
    Ok(Referent::Window {
        app: snapshot.app.name.clone(),
        title: snapshot.window.clone(),
    })
}

/// What "this" adds to a request that goes to the model.
#[derive(Debug, PartialEq)]
pub enum ThisContext {
    /// The request doesn't point at the screen, or nothing was captured.
    Nothing,
    /// What "this" is, for a brain allowed every kind of data it shows.
    Seen(Referent),
    /// This brain may not see it, so it is told that instead of guessing.
    Withheld,
    /// It couldn't be read: an app to allow, a private window or a password field. The
    /// user sees why, as they do when a skill asks about "this".
    Refused(String),
    /// It couldn't be read, but the request may not need it ("is this a good idea?"): the
    /// model is told why and answers from what the user wrote.
    Unread(String),
}

/// Resolves "this" for the model path, under the same app checks as the skill path and
/// the privacy rules of the brain that will see it.
pub async fn this_context(
    prompt: &str,
    snapshot: Option<&Snapshot>,
    settings: &Settings,
    locality: Locality,
) -> ThisContext {
    let on_screen = plainly_on_screen(prompt);
    let Some(snapshot) = snapshot
        .filter(|_| mentions_this(prompt) && (on_screen || !carries_its_own_text(prompt)))
    else {
        return ThisContext::Nothing;
    };
    match referent(snapshot, settings, prompt).await {
        Err(message) if on_screen => ThisContext::Refused(message),
        Err(message) => ThisContext::Unread(message),
        Ok(found)
            if found
                .shares()
                .into_iter()
                .all(|share| privacy::allows(locality, settings, share)) =>
        {
            ThisContext::Seen(found)
        }
        Ok(_) => ThisContext::Withheld,
    }
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let mut kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

impl Referent {
    /// Every kind of data it shows, for what an external brain may see.
    pub fn shares(&self) -> Vec<Share> {
        match self {
            Referent::Selection { mail: Some(_), .. } => vec![Share::Selection, Share::Mail],
            Referent::Selection { .. } | Referent::Window { .. } => vec![Share::Selection],
            Referent::Mail(_) => vec![Share::Mail],
            Referent::Page { .. } => vec![Share::Browser],
            Referent::Finder { .. } => vec![Share::Files],
        }
    }

    /// The email a reply to "this" answers, if there is one.
    pub fn mail(&self) -> Option<&crate::apps::SelectedMail> {
        match self {
            Referent::Mail(mail) | Referent::Selection { mail: Some(mail), .. } => Some(mail),
            _ => None,
        }
    }

    /// One line for the model about what "this" is. Quoted content is untrusted data.
    pub fn describe(&self) -> String {
        match self {
            Referent::Selection { app, text, mail: None } => {
                format!("Text the user selected in {app}: “{}”", clip(text, 2_000))
            }
            Referent::Selection { text, mail: Some(mail), .. } => format!(
                "Text the user selected in an email in Mail, from {}, subject “{}”, received {}: “{}”",
                mail.sender,
                mail.subject,
                mail.received,
                clip(text, 2_000)
            ),
            Referent::Mail(mail) => format!(
                "The email open in Mail, from {}, subject “{}”, received {}: “{}”",
                mail.sender,
                mail.subject,
                mail.received,
                clip(&mail.text, 1_500)
            ),
            Referent::Page { browser, title } => {
                let name = crate::apps::app(browser).map_or(*browser, |app| app.name);
                format!("The web page showing in {name}: “{title}”")
            }
            Referent::Finder { paths } => format!(
                "Files selected in Finder: {}",
                paths.iter().map(|p| file_name(p)).collect::<Vec<_>>().join(", ")
            ),
            Referent::Window { app, title } if title.is_empty() => format!("The app {app}"),
            Referent::Window { app, title } => format!("{app}, window “{title}”"),
        }
    }

    /// A reminder title for "remind me about this", when "this" has a name of its own.
    pub fn reminder_title(&self) -> Option<String> {
        let title = match self {
            Referent::Selection { text, .. } => clip(text.lines().find(|l| !l.trim().is_empty())?, 120),
            Referent::Mail(mail) if mail.subject.is_empty() => format!("Email from {}", mail.sender),
            Referent::Mail(mail) => clip(&format!("{} (email from {})", mail.subject, sender_name(&mail.sender)), 200),
            Referent::Page { title, .. } if !title.trim().is_empty() => clip(title, 200),
            Referent::Finder { paths } if paths.len() == 1 => file_name(&paths[0]).to_string(),
            _ => return None,
        };
        (!title.trim().is_empty()).then_some(title)
    }
}

/// "Ana Silva <ana@example.com>" as "Ana Silva".
fn sender_name(sender: &str) -> &str {
    match sender.split_once('<') {
        Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"'),
        _ => sender.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(bundle: &str, window: &str, selection: &str, secure: bool) -> Snapshot {
        Snapshot {
            at: Instant::now(),
            app: Front { pid: 1, name: "Notes".into(), bundle: bundle.into() },
            window: window.into(),
            selection: selection.into(),
            secure,
        }
    }

    fn mail(subject: &str) -> crate::apps::SelectedMail {
        crate::apps::SelectedMail {
            id: "12".into(),
            message_id: "<a@b>".into(),
            received: "2026-10-29 09:15".into(),
            sender: "Ana Silva <ana@example.com>".into(),
            subject: subject.into(),
            text: "Can you sign the lease renewal by Friday?".into(),
            truncated: false,
            selected: 1,
        }
    }

    #[tokio::test]
    async fn selected_text_wins_over_the_app_and_its_window() {
        let settings = Settings::default();
        let found = referent(&snapshot(MAIL, "Inbox", "Pick up keys at 6", false), &settings, "summarise this")
            .await
            .unwrap();
        assert_eq!(
            found,
            Referent::Selection { app: "Notes".into(), text: "Pick up keys at 6".into(), mail: None }
        );
        assert_eq!(found.shares(), [Share::Selection]);
        // Mail isn't allowed, so a reply to the selection can't read the email behind it.
        let reply = referent(&snapshot(MAIL, "Inbox", "Pick up keys at 6", false), &settings, "reply to her saying yes")
            .await
            .unwrap();
        assert_eq!(reply.mail(), None);
    }

    #[tokio::test]
    async fn password_fields_and_unallowed_mail_are_refused_not_guessed() {
        let settings = Settings::default();
        let secure = referent(&snapshot("com.example.app", "Login", "", true), &settings, "").await;
        assert!(secure.unwrap_err().contains("password"));
        let mail = referent(&snapshot(MAIL, "Inbox", "", false), &settings, "").await;
        assert!(mail.unwrap_err().contains("Allow Mail"));
    }

    #[tokio::test]
    async fn browsers_need_their_permission_before_anything_is_read() {
        let mut settings = Settings::default();
        // Not even the window title or a selection is read from a browser that isn't allowed.
        for selection in ["", "Rent is due on the 1st"] {
            let refused = referent(&snapshot("com.google.Chrome", "Lease FAQ", selection, false), &settings, "")
                .await
                .unwrap_err();
            assert_eq!(refused, "Allow Google Chrome in Settings → Apps so I can read its tabs.");
        }
        // Safari has no private-window flag to check (Settings → Apps says so).
        settings.apps = vec!["safari".into()];
        let page = referent(&snapshot("com.apple.Safari", "Lease FAQ", "", false), &settings, "")
            .await
            .unwrap();
        assert_eq!(page, Referent::Page { browser: "safari", title: "Lease FAQ".into() });
        assert_eq!(page.shares(), [Share::Browser]);
        let window = referent(&snapshot("com.apple.dt.Xcode", "main.rs", "", false), &settings, "")
            .await
            .unwrap();
        assert_eq!(window.describe(), "Notes, window “main.rs”");
        assert_eq!(window.reminder_title(), None);
    }

    #[test]
    fn a_snapshot_of_an_app_the_user_has_left_is_stale() {
        let taken = snapshot(MAIL, "Inbox", "", false);
        let mail = taken.app.clone();
        let safari = Front { pid: 2, name: "Safari".into(), bundle: "com.apple.Safari".into() };
        assert!(still_current(&taken, Some(&mail)));
        assert!(still_current(&taken, None));
        assert!(!still_current(&taken, Some(&safari)));
        if let Some(at) = Instant::now().checked_sub(FRESH + Duration::from_secs(1)) {
            assert!(!still_current(&Snapshot { at, ..taken }, Some(&mail)));
        }
    }

    #[tokio::test]
    async fn the_model_path_shows_refusals_and_keeps_what_it_may_not_see() {
        let settings = Settings::default();
        let selected = snapshot("com.apple.Notes", "Groceries", "Oat milk, eggs", false);
        let seen = this_context("summarise this", Some(&selected), &settings, Locality::OnDevice).await;
        assert_eq!(
            seen,
            ThisContext::Seen(Referent::Selection { app: "Notes".into(), text: "Oat milk, eggs".into(), mail: None })
        );
        let mut external = Settings { external_shares: vec!["history".into()], ..Settings::default() };
        assert_eq!(
            this_context("summarise this", Some(&selected), &external, Locality::External).await,
            ThisContext::Withheld
        );
        external.external_shares.push("selection".into());
        assert!(matches!(
            this_context("summarise this", Some(&selected), &external, Locality::External).await,
            ThisContext::Seen(_)
        ));
        let open_mail = snapshot(MAIL, "Inbox", "", false);
        assert_eq!(
            this_context("reply to her saying yes", Some(&open_mail), &settings, Locality::OnDevice).await,
            ThisContext::Refused("Allow Mail in Settings → Apps so I can read the email you have open.".into())
        );
        let password = snapshot("com.example.app", "Login", "", true);
        assert!(matches!(
            this_context("what is this", Some(&password), &settings, Locality::OnDevice).await,
            ThisContext::Refused(message) if message.contains("password")
        ));
        // Requests that don't point at the screen never read it.
        assert_eq!(
            this_context("weather this weekend", Some(&open_mail), &settings, Locality::OnDevice).await,
            ThisContext::Nothing
        );
        assert_eq!(this_context("summarise this", None, &settings, Locality::OnDevice).await, ThisContext::Nothing);
        // A browser that isn't allowed is still in front: a request that carries its own
        // text never reads it, and one that may not need it is answered with a note.
        let chrome = snapshot("com.google.Chrome", "News", "", false);
        for prompt in [
            "Translate this to Spanish: good morning",
            "Is this sentence correct: \"me and him went\"",
            "how do I do this in Python: read a file line by line",
            "Summarise this morning's emails",
        ] {
            assert_eq!(this_context(prompt, Some(&chrome), &settings, Locality::OnDevice).await, ThisContext::Nothing, "{prompt}");
        }
        assert!(matches!(
            this_context("is this a good time to refinance", Some(&chrome), &settings, Locality::OnDevice).await,
            ThisContext::Unread(message) if message.contains("Google Chrome")
        ));
        for prompt in ["summarise this", "what's on this page", "explain this please"] {
            assert!(matches!(
                this_context(prompt, Some(&chrome), &settings, Locality::OnDevice).await,
                ThisContext::Refused(_)
            ), "{prompt}");
        }
    }

    #[test]
    fn reminder_titles_name_the_thing_on_screen() {
        assert_eq!(
            Referent::Mail(mail("Lease renewal")).reminder_title().as_deref(),
            Some("Lease renewal (email from Ana Silva)")
        );
        assert_eq!(
            Referent::Mail(mail("")).reminder_title().as_deref(),
            Some("Email from Ana Silva <ana@example.com>")
        );
        assert_eq!(
            Referent::Selection { app: "Notes".into(), text: "\n  Call the bank\nabout fees".into(), mail: None }
                .reminder_title()
                .as_deref(),
            Some("Call the bank")
        );
        assert_eq!(
            Referent::Finder { paths: vec!["/Users/a/Lease.pdf".into()] }.reminder_title().as_deref(),
            Some("Lease.pdf")
        );
        assert_eq!(Referent::Finder { paths: vec!["/a".into(), "/b".into()] }.reminder_title(), None);
    }

    #[test]
    fn descriptions_are_bounded_and_mail_is_its_own_share() {
        let long = "word ".repeat(1_000);
        let text = Referent::Selection { app: "Pages".into(), text: long, mail: None }.describe();
        assert!(text.chars().count() < 2_100 && text.ends_with("…”"));
        assert_eq!(Referent::Mail(mail("Hi")).shares(), [Share::Mail]);
        assert_eq!(Referent::Finder { paths: vec![] }.shares(), [Share::Files]);
        // A selection inside an email shows both, and names the email it came from.
        let quoted = Referent::Selection { app: "Mail".into(), text: "Friday works".into(), mail: Some(mail("Lunch")) };
        assert_eq!(quoted.shares(), [Share::Selection, Share::Mail]);
        assert!(quoted.describe().contains("subject “Lunch”") && quoted.describe().contains("Friday works"));
    }

    #[test]
    fn requests_that_point_at_the_screen_are_recognised() {
        for prompt in ["remind me about this tomorrow", "reply to him", "explain the selected text"] {
            assert!(mentions_this(prompt), "{prompt}");
        }
        for prompt in ["remind me to call Ana", "what's the weather", "thistle seeds"] {
            assert!(!mentions_this(prompt), "{prompt}");
        }
        for prompt in ["Summarise this email", "reply to her saying yes", "write back to him", "what are these?"] {
            assert!(mentions_this(prompt), "{prompt}");
        }
        for prompt in [
            "weather this weekend",
            "gift for her birthday",
            "what's on this Friday",
            "I told her about the lease",
        ] {
            assert!(!mentions_this(prompt), "{prompt}");
        }
        for prompt in [
            "remind me about this tomorrow at 9",
            "please add this to my reminders for Friday",
            "can you remind me to read this tonight",
        ] {
            assert!(reminds_about_this(prompt), "{prompt}");
        }
        for prompt in ["reply to this saying yes", "Please write back to her", "respond to him: no"] {
            assert!(replies_to_this(prompt), "{prompt}");
        }
        // Whenever "this" needs a threaded reply, it is the email on screen that it answers,
        // so that email and its sender are bound rather than left to the model.
        for prompt in ["Reply to her question saying yes", "reply to this saying yes", "respond to this question in one sentence"] {
            let replies = crate::completion::requested_actions(prompt).0.contains(&"reply_email");
            assert_eq!(replies_to_this(prompt), replies, "{prompt}");
        }
        for prompt in [
            "reply to Ana",
            "don't reply to this",
            "how do I reply to this",
            "respond to this question in one sentence",
        ] {
            assert!(!replies_to_this(prompt), "{prompt}");
        }
        assert_eq!(sender_address("Ana Silva <Ana@Example.com>").as_deref(), Some("ana@example.com"));
        assert_eq!(sender_address("ana@example.com").as_deref(), Some("ana@example.com"));
        assert_eq!(sender_address("Ana Silva"), None);
        for prompt in [
            "don't remind me about this",
            "remind me to reply to him",
            "how do I add this to my reminders",
        ] {
            assert!(!reminds_about_this(prompt), "{prompt}");
        }
    }
}
