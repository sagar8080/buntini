//! Unread messages in chat apps, beside the notch: the count each one's Dock icon shows
//! (WhatsApp, Telegram, Signal, Discord, Slack, Messages), for the apps switched on in
//! Settings → Apps. Launch Services keeps every running app's badge and tells any app, so
//! nothing is asked of the chat apps and no permission is needed; the messages themselves are
//! never read. Opening one brings its app to the front.
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tauri::AppHandle;

use crate::i18n::{t, tf};
use crate::surface::cards::island::Unread as UnreadCard;
use crate::surface::{Card, CardBody, Offer, Pill, PillState, Source, Urgency};

/// The start of each chat's pill and card id.
pub const PREFIX: &str = "unread.";
/// How often the badges are read.
const EVERY: Duration = Duration::from_secs(5);

/// A chat app, by its id in Settings → Apps.
pub struct Chat {
    pub id: &'static str,
    pub name: &'static str,
    pub bundles: &'static [&'static str],
    pub tint: &'static str,
}

pub const CHATS: [Chat; 6] = [
    Chat { id: "messages", name: "Messages", bundles: &["com.apple.MobileSMS"], tint: "#34C759" },
    Chat { id: "whatsapp", name: "WhatsApp", bundles: &["net.whatsapp.WhatsApp"], tint: "#25D366" },
    Chat { id: "telegram", name: "Telegram", bundles: &["ru.keepcoder.Telegram", "org.telegram.desktop"], tint: "#2AABEE" },
    Chat { id: "signal", name: "Signal", bundles: &["org.whispersystems.signal-desktop"], tint: "#3A76F0" },
    Chat { id: "discord", name: "Discord", bundles: &["com.hnc.Discord"], tint: "#5865F2" },
    Chat { id: "slack", name: "Slack", bundles: &["com.tinyspeck.slackmacgap"], tint: "#E01E5A" },
];

/// A chat's badge, as its Dock icon shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub chat: &'static str,
    pub bundle: String,
    pub label: String,
}

/// What a badge says, kept: "3", "99+" or a dot. Nothing for an empty one or a zero.
pub fn kept(label: &str) -> Option<String> {
    let label = label.trim();
    (!label.is_empty() && label != "0" && label.chars().count() <= 9).then(|| label.to_string())
}

/// The card's line: how many, or that there are new messages when the badge is a dot.
pub fn line(label: &str) -> String {
    let counted = label.trim_end_matches('+');
    if !counted.is_empty() && counted.chars().all(|c| c.is_ascii_digit()) {
        tf("island.unread.count", &[("count", &label)])
    } else {
        t("island.unread.some")
    }
}

/// A chat's pill and card.
pub fn view(badge: &Badge) -> Option<(Pill, Card)> {
    let chat = CHATS.iter().find(|chat| chat.id == badge.chat)?;
    let id = format!("{PREFIX}{}", chat.id);
    let pill = Pill {
        id: id.clone(),
        source: Source::Island,
        label: chat.name.into(),
        tint: chat.tint.into(),
        state: PillState::Idle,
        badge: Some(badge.label.clone()),
        progress: None,
        card: None,
    };
    let card = Card { id, source: Source::Island, body: CardBody::Unread(UnreadCard { name: chat.name.into(), line: line(&badge.label) }) };
    Some((pill, card))
}

static SHOWN: Mutex<Vec<Badge>> = Mutex::new(Vec::new());

fn shown_now() -> MutexGuard<'static, Vec<Badge>> {
    SHOWN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads the badges of the chat apps switched on, every few seconds while the screen is on.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(EVERY).await;
            if crate::media::screen_away() {
                continue;
            }
            let allowed: Vec<&'static str> = super::settings(&app).map(|settings| CHATS.iter().map(|chat| chat.id).filter(|id| crate::apps::allowed(&settings, id)).collect()).unwrap_or_default();
            let found = tokio::task::spawn_blocking(move || read(&allowed)).await.unwrap_or_default();
            let mut shown = shown_now();
            if *shown != found {
                *shown = found;
                drop(shown);
                super::refresh();
            }
        }
    });
}

/// The badges of the chats in `allowed` that are open now.
fn read(allowed: &[&'static str]) -> Vec<Badge> {
    CHATS
        .iter()
        .filter(|chat| allowed.contains(&chat.id))
        .filter_map(|chat| {
            chat.bundles.iter().find_map(|bundle| {
                let pid = running(bundle)?;
                Some(Badge { chat: chat.id, bundle: bundle.to_string(), label: kept(&launch::label(pid)?)? })
            })
        })
        .collect()
}

fn running(bundle: &str) -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSRunningApplication;
        use objc2_foundation::NSString;
        let found = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(bundle)).firstObject()?;
        Some(found.processIdentifier()).filter(|pid| *pid > 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle;
        None
    }
}

/// The chats with unread messages: their pills and cards.
pub fn shown() -> (Vec<Pill>, Vec<Offer>) {
    let mut pills = Vec::new();
    let mut offers = Vec::new();
    for (pill, card) in shown_now().iter().filter_map(view) {
        pills.push(pill);
        offers.push(Offer { card, urgency: Urgency::Info, at_ms: 0, ttl_ms: None });
    }
    (pills, offers)
}

/// The card's Open: the chat app, in front.
pub fn open(card: &str) {
    let bundle = shown_now().iter().find(|badge| card.strip_prefix(PREFIX) == Some(badge.chat)).map(|badge| badge.bundle.clone());
    if let Some(bundle) = bundle {
        let _ = std::process::Command::new("/usr/bin/open").args(["-b", &bundle]).spawn();
    }
}

/// Launch Services, which keeps each running app's Dock badge. Its calls aren't public, so
/// they're looked up when first needed: a macOS without them shows no counts, and nothing
/// else changes.
#[cfg(target_os = "macos")]
mod launch {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    type CreateAsn = unsafe extern "C" fn(*const c_void, i32) -> *const c_void;
    type CopyItem = unsafe extern "C" fn(i32, *const c_void, *const c_void) -> *const c_void;
    /// `kLSDefaultSessionID`.
    const SESSION: i32 = -2;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(object: *const c_void);
        fn CFGetTypeID(object: *const c_void) -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    }

    static CALLS: OnceLock<Option<(CreateAsn, CopyItem)>> = OnceLock::new();

    fn calls() -> Option<(CreateAsn, CopyItem)> {
        *CALLS.get_or_init(|| {
            // SAFETY: looking up two functions by name in a system framework; a missing one is None.
            unsafe {
                let library = libc::dlopen(c"/System/Library/Frameworks/CoreServices.framework/CoreServices".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
                if library.is_null() {
                    return None;
                }
                let create = libc::dlsym(library, c"_LSASNCreateWithPid".as_ptr());
                let copy = libc::dlsym(library, c"_LSCopyApplicationInformationItem".as_ptr());
                if create.is_null() || copy.is_null() {
                    return None;
                }
                Some((std::mem::transmute::<*mut c_void, CreateAsn>(create), std::mem::transmute::<*mut c_void, CopyItem>(copy)))
            }
        })
    }

    /// The badge on the Dock icon of the app with `pid`, if it has one.
    pub fn label(pid: i32) -> Option<String> {
        let (create, copy) = calls()?;
        let key = NSString::from_str("StatusLabel");
        let label_key = NSString::from_str("label");
        // SAFETY: each object Launch Services returns is checked for its type before use and
        // released once; the keys live through the calls.
        unsafe {
            let asn = create(std::ptr::null(), pid);
            if asn.is_null() {
                return None;
            }
            let item = copy(SESSION, asn, Retained::as_ptr(&key) as *const c_void);
            CFRelease(asn);
            if item.is_null() {
                return None;
            }
            let mut label = None;
            if CFGetTypeID(item) == CFDictionaryGetTypeID() {
                let value = CFDictionaryGetValue(item, Retained::as_ptr(&label_key) as *const c_void);
                if !value.is_null() && CFGetTypeID(value) == CFStringGetTypeID() {
                    label = Retained::retain(value as *mut NSString).map(|text| text.to_string());
                }
            }
            CFRelease(item);
            label
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod launch {
    pub fn label(_: i32) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_badge_is_kept_as_it_reads_and_empty_ones_are_not() {
        assert_eq!(kept("3").as_deref(), Some("3"));
        assert_eq!(kept(" 99+ ").as_deref(), Some("99+"));
        assert_eq!(kept("•").as_deref(), Some("•"));
        assert_eq!(kept(""), None);
        assert_eq!(kept("0"), None);
        assert_eq!(kept("a very long badge"), None);
    }

    #[test]
    fn the_card_says_how_many_or_that_there_are_new_messages() {
        assert_eq!(line("3"), tf("island.unread.count", &[("count", &"3")]));
        assert_eq!(line("99+"), tf("island.unread.count", &[("count", &"99+")]));
        assert_eq!(line("•"), t("island.unread.some"));
        let (pill, card) = view(&Badge { chat: "whatsapp", bundle: "net.whatsapp.WhatsApp".into(), label: "3".into() }).unwrap();
        assert_eq!((pill.id.as_str(), pill.label.as_str(), pill.badge.as_deref(), pill.tint.as_str()), ("unread.whatsapp", "WhatsApp", Some("3"), "#25D366"));
        let CardBody::Unread(body) = card.body else { panic!() };
        assert_eq!(body.name, "WhatsApp");
        assert!(view(&Badge { chat: "fax", bundle: String::new(), label: "1".into() }).is_none());
    }

    #[test]
    fn every_chat_is_an_app_in_settings() {
        for chat in &CHATS {
            assert!(crate::apps::app(chat.id).is_some(), "{}", chat.id);
        }
    }

    /// The chat apps' badges on this Mac now, whatever Settings says: `cargo test --lib
    /// island::unread::tests::live -- --ignored --nocapture`. `BUNTINI_LIVE_PID` reads any
    /// other app's badge the same way.
    #[test]
    #[ignore]
    fn live() {
        if let Some(pid) = std::env::var("BUNTINI_LIVE_PID").ok().and_then(|pid| pid.parse().ok()) {
            println!("pid {pid}: {:?}", launch::label(pid));
        }
        let every: Vec<&'static str> = CHATS.iter().map(|chat| chat.id).collect();
        for chat in &CHATS {
            let pid = chat.bundles.iter().find_map(|bundle| running(bundle));
            println!("{}: {pid:?} → {:?}", chat.name, pid.and_then(launch::label));
        }
        println!("{:?}", read(&every));
    }
}
