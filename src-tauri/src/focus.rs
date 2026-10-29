//! What "this" means: the app, window and selection in front when the user opens Buntini.
//!
//! The app in front is tracked continuously (only its process id and names, in memory), so
//! a click on the pet still knows what the user was looking at after Buntini takes focus.
//! Opening Buntini takes a snapshot of that app's window title and selected text through
//! Accessibility. Anything app-specific (the email open in Mail, the page in a browser, the
//! files selected in Finder) is read only when a request actually refers to it, under that
//! app's own permission. Nothing here is written to disk; a task that uses it records the
//! data category it touched like any other read.
use crate::{privacy::Share, types::Settings};
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
    Selection { app: String, text: String },
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

/// Follows the app in front twice a second, ignoring Buntini itself.
pub fn start() {
    std::thread::spawn(|| loop {
        if let Some(front) = frontmost() {
            *LAST.lock().unwrap() = Some(front);
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

/// Takes a snapshot of the app the user was just in. Call it as Buntini opens, before
/// its window takes focus; reading happens on a background thread.
pub fn capture() {
    let target = frontmost().or_else(|| LAST.lock().unwrap().clone());
    if let Some(front) = &target {
        *LAST.lock().unwrap() = Some(front.clone());
    }
    let generation = STARTED.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let snapshot = target.map(|app| {
            let (window, selection, secure) = if crate::ax::trusted(false) {
                let read = crate::ax::selection(app.pid, MAX_SELECTION);
                (read.window, read.text, read.secure)
            } else {
                (String::new(), String::new(), false)
            };
            Snapshot { at: Instant::now(), app, window, selection, secure }
        });
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

/// The latest fresh snapshot. Waits briefly for one still being taken, so a request typed
/// straight after the shortcut sees it.
pub async fn current() -> Option<Snapshot> {
    let wanted = STARTED.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let done = SNAPSHOT.lock().unwrap().clone();
        match done {
            Some((generation, snapshot)) if generation >= wanted => {
                return snapshot.filter(|s| s.at.elapsed() < FRESH)
            }
            _ if wanted == 0 || Instant::now() >= deadline => return None,
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
}

/// Whether a request points at something on screen ("this", "these", "him", "her").
pub fn mentions_this(prompt: &str) -> bool {
    crate::skills::words(prompt)
        .split(' ')
        .any(|word| ["this", "these", "him", "her", "selection", "selected"].contains(&word))
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
    /// The allowed browser this snapshot was taken in, if it was one.
    pub fn browser(&self) -> Option<&'static str> {
        browser_id(&self.app.bundle)
    }
}

/// Resolves "this" against a snapshot: selected text first, then the app's own object,
/// then just the window. App reads need that app to be allowed in Settings → Apps.
pub async fn referent(snapshot: &Snapshot, settings: &Settings) -> Result<Referent, String> {
    if !snapshot.selection.is_empty() {
        return Ok(Referent::Selection {
            app: snapshot.app.name.clone(),
            text: snapshot.selection.clone(),
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
    /// The kind of data it shows, for what an external brain may see.
    pub fn share(&self) -> Share {
        match self {
            Referent::Selection { .. } | Referent::Window { .. } => Share::Selection,
            Referent::Mail(_) => Share::Mail,
            Referent::Page { .. } => Share::Browser,
            Referent::Finder { .. } => Share::Files,
        }
    }

    /// One line for the model about what "this" is. Quoted content is untrusted data.
    pub fn describe(&self) -> String {
        match self {
            Referent::Selection { app, text } => {
                format!("Text the user selected in {app}: “{}”", clip(text, 2_000))
            }
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
        let found = referent(&snapshot(MAIL, "Inbox", "Pick up keys at 6", false), &settings)
            .await
            .unwrap();
        assert_eq!(
            found,
            Referent::Selection { app: "Notes".into(), text: "Pick up keys at 6".into() }
        );
        assert_eq!(found.share(), Share::Selection);
    }

    #[tokio::test]
    async fn password_fields_and_unallowed_mail_are_refused_not_guessed() {
        let settings = Settings::default();
        let secure = referent(&snapshot("com.example.app", "Login", "", true), &settings).await;
        assert!(secure.unwrap_err().contains("password"));
        let mail = referent(&snapshot(MAIL, "Inbox", "", false), &settings).await;
        assert!(mail.unwrap_err().contains("Allow Mail"));
    }

    #[tokio::test]
    async fn browsers_become_pages_and_other_apps_their_window() {
        let settings = Settings::default();
        let page = referent(&snapshot("com.google.Chrome", "Lease FAQ", "", false), &settings)
            .await
            .unwrap();
        assert_eq!(page, Referent::Page { browser: "chrome", title: "Lease FAQ".into() });
        assert_eq!(page.share(), Share::Browser);
        let window = referent(&snapshot("com.apple.dt.Xcode", "main.rs", "", false), &settings)
            .await
            .unwrap();
        assert_eq!(window.describe(), "Notes, window “main.rs”");
        assert_eq!(window.reminder_title(), None);
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
            Referent::Selection { app: "Notes".into(), text: "\n  Call the bank\nabout fees".into() }
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
        let text = Referent::Selection { app: "Pages".into(), text: long }.describe();
        assert!(text.chars().count() < 2_100 && text.ends_with("…”"));
        assert_eq!(Referent::Mail(mail("Hi")).share(), Share::Mail);
        assert_eq!(Referent::Finder { paths: vec![] }.share(), Share::Files);
    }

    #[test]
    fn requests_that_point_at_the_screen_are_recognised() {
        for prompt in ["remind me about this tomorrow", "reply to him", "explain the selected text"] {
            assert!(mentions_this(prompt), "{prompt}");
        }
        for prompt in ["remind me to call Ana", "what's the weather", "thistle seeds"] {
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
        for prompt in ["reply to Ana", "don't reply to this", "how do I reply to this"] {
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
