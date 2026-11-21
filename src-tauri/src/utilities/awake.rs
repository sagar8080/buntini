//! Keeping the Mac awake: for as long as asked ("keep my Mac awake for an hour", or until
//! stopped), with its pill beside the notch and Stop on its card; and, without either, while
//! a coding agent works on a turn, so a long run isn't cut off by the Mac going to sleep. Both
//! are power assertions, as `caffeinate` makes: no permission, they show under Buntini's name
//! in Activity Monitor, and macOS ends them with Buntini.
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::i18n::{t, tf};
use crate::surface::cards::utilities::Awake as AwakeCard;
use crate::surface::{self, Card, CardBody, Offer, Pill, PillState, Source, Urgency};

pub const ID: &str = "utilities.awake";
const TINT: &str = "#FFD60A";
/// How often an agent's work is looked at again.
const AGENTS_EVERY: Duration = Duration::from_secs(30);
/// Asked for: the display stays on, and the Mac with it.
const DISPLAY: &str = "PreventUserIdleDisplaySleep";
/// For an agent: the Mac stays awake, the display may sleep.
const SYSTEM: &str = "PreventUserIdleSystemSleep";

#[derive(Debug, PartialEq)]
pub enum Command {
    /// For `seconds`, or until stopped.
    Start { seconds: Option<u64> },
    Stop,
}

/// "Keep my Mac awake", "stay awake for 2 hours", "don't let the Mac sleep"; "let my Mac
/// sleep", "stop keeping it awake". Anything more goes to the model.
pub fn recognise(text: &str) -> Option<Command> {
    const POLITE: [&str; 17] = ["buntini", "hey", "ok", "okay", "can", "you", "could", "would", "just", "i", "want", "to", "i'd", "like", "need", "go", "ahead"];
    let mut words: Vec<&str> = text.split_whitespace().filter(|word| *word != "please").collect();
    while words.first().is_some_and(|word| POLITE.contains(word)) {
        words.remove(0);
    }
    let line = words.join(" ");
    let thing = |middle: &str| ["", "my mac", "the mac", "mac", "my computer", "the computer", "my laptop", "the laptop", "my screen", "the screen", "the display", "it"].contains(&middle.trim());
    // Stopping.
    if let Some(middle) = line.strip_prefix("stop keeping").and_then(|rest| rest.strip_suffix("awake")) {
        return thing(middle).then_some(Command::Stop);
    }
    if let Some(rest) = line.strip_prefix("let ") {
        if ["go to sleep again", "go to sleep", "sleep again", "sleep now", "sleep"].iter().any(|end| rest.strip_suffix(end).is_some_and(thing)) {
            return Some(Command::Stop);
        }
    }
    if ["allow sleep", "allow sleeping", "stop caffeinate", "stop caffeinating"].contains(&line.as_str()) {
        return Some(Command::Stop);
    }
    // Starting, for a while or until stopped.
    let (head, seconds) = match line.find(" for ") {
        Some(at) => (&line[..at], Some(crate::skills::mac::duration(&line[at + 5..])?)),
        None => (line.as_str(), None),
    };
    let keep = head.strip_prefix("keep ").is_some_and(|rest| {
        rest.strip_suffix("awake").is_some_and(thing) || rest.strip_suffix("from sleeping").is_some_and(thing) || rest.strip_suffix("from going to sleep").is_some_and(thing)
    });
    let let_not = ["don't let ", "dont let ", "do not let "].iter().find_map(|start| head.strip_prefix(start)).is_some_and(|rest| {
        rest.strip_suffix("sleep").is_some_and(thing) || rest.strip_suffix("go to sleep").is_some_and(thing)
    });
    (keep || let_not || ["stay awake", "caffeinate", "no sleep"].contains(&head)).then_some(Command::Start { seconds })
}

/// Kept awake as asked.
#[derive(Debug, Clone, PartialEq)]
struct Asked {
    /// When it ends, in Unix milliseconds; `None` until stopped.
    until: Option<i64>,
    /// When it began, on the board's clock.
    since: u64,
}

#[derive(Default)]
struct State {
    asked: Option<Asked>,
    /// The power assertions held, by macOS's id for them.
    asked_hold: Option<u32>,
    agents_hold: Option<u32>,
    started: bool,
}

static STATE: Mutex<State> = Mutex::new(State { asked: None, asked_hold: None, agents_hold: None, started: false });
static WAKE: Condvar = Condvar::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Keeps the holds in step with what's asked and what agents do, and the pill's minutes.
pub fn start() {
    // Tests never hold the real Mac awake.
    if cfg!(test) {
        return;
    }
    {
        let mut state = state();
        if state.started {
            return;
        }
        state.started = true;
    }
    std::thread::spawn(|| loop {
        let agents = crate::agents::working();
        let mut state = state();
        let at = now();
        let mut changed = false;
        if state.asked.as_ref().is_some_and(|asked| asked.until.is_some_and(|until| until <= at)) {
            state.asked = None;
            changed = true;
        }
        let asked = state.asked.is_some();
        hold(&mut state.asked_hold, asked, DISPLAY, "Buntini: keeping the Mac awake, as asked");
        hold(&mut state.agents_hold, agents && !asked, SYSTEM, "Buntini: a coding agent is working");
        // Wake when the pill's minutes change, when it ends, and to look at the agents again.
        let wait = state
            .asked
            .as_ref()
            .and_then(|asked| asked.until)
            .map(|until| Duration::from_millis(crate::utilities::focus::next_badge_change(until - at).clamp(20, 30_000) as u64))
            .unwrap_or(AGENTS_EVERY)
            .min(AGENTS_EVERY);
        drop(state);
        if changed || asked {
            super::publish();
        }
        let state = self::state();
        let _ = WAKE.wait_timeout(state, wait);
    });
}

fn hold(held: &mut Option<u32>, wanted: bool, kind: &str, reason: &str) {
    match (*held, wanted) {
        (None, true) => *held = power::hold(kind, reason),
        (Some(id), false) => {
            power::release(id);
            *held = None;
        }
        _ => {}
    }
}

/// Keeps the Mac awake for `seconds`, or until stopped, replacing what was asked before.
pub fn keep(seconds: Option<u64>) -> Option<i64> {
    let until = seconds.map(|seconds| now() + seconds as i64 * 1000);
    state().asked = Some(Asked { until, since: surface::now_ms() });
    start();
    WAKE.notify_all();
    super::publish();
    until
}

/// Lets the Mac sleep again. False when it wasn't kept awake as asked.
pub fn stop() -> bool {
    let was = state().asked.take().is_some();
    WAKE.notify_all();
    super::publish();
    was
}

/// The card's Stop.
pub fn act(name: &str) -> Result<(), String> {
    if name == "stop" {
        stop();
    }
    Ok(())
}

/// The pill and card while the Mac is kept awake as asked.
pub fn shown() -> (Option<Pill>, Option<Offer>) {
    let Some(asked) = state().asked.clone() else { return (None, None) };
    let left = asked.until.map(|until| until - now());
    let pill = Pill {
        id: ID.into(),
        source: Source::Utilities,
        label: t("utilities.awake.label"),
        tint: TINT.into(),
        state: PillState::Working,
        badge: left.map(|left| crate::utilities::focus::badge(left.max(0))),
        progress: None,
        card: None,
    };
    let card = Card { id: ID.into(), source: Source::Utilities, body: CardBody::Awake(AwakeCard { label: t("utilities.awake.label"), until: asked.until.map(|until| until / 1000) }) };
    (Some(pill), Some(Offer { card, urgency: Urgency::Info, at_ms: asked.since, ttl_ms: None }))
}

/// A request from the chat.
pub fn run(command: Command) -> Result<crate::skills::Outcome, String> {
    let answer = match command {
        Command::Start { seconds } => match keep(seconds) {
            Some(until) => {
                let until = chrono::DateTime::from_timestamp_millis(until).unwrap_or_default().with_timezone(&chrono::Local);
                tf("utilities.awake.chat.until", &[("time", &until.format("%-H:%M"))])
            }
            None => t("utilities.awake.chat.on"),
        },
        Command::Stop => {
            if !stop() {
                return Err(t("utilities.awake.chat.notOn"));
            }
            t("utilities.awake.chat.stopped")
        }
    };
    Ok(crate::skills::Outcome::Done { answer, shares: Vec::new() })
}

/// IOKit's power assertions, from Buntini's own process.
#[cfg(target_os = "macos")]
mod power {
    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    /// `kIOPMAssertionLevelOn`.
    const ON: u32 = 255;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(kind: *const NSString, level: u32, name: *const NSString, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    pub fn hold(kind: &str, reason: &str) -> Option<u32> {
        let kind = NSString::from_str(kind);
        let reason = NSString::from_str(reason);
        let mut id = 0u32;
        // SAFETY: both strings live through the call; only `id` is written.
        let status = unsafe { IOPMAssertionCreateWithName(Retained::as_ptr(&kind), ON, Retained::as_ptr(&reason), &mut id) };
        (status == 0).then_some(id)
    }

    pub fn release(id: u32) {
        // SAFETY: `id` came from `hold` and is released once.
        unsafe { IOPMAssertionRelease(id) };
    }
}

#[cfg(not(target_os = "macos"))]
mod power {
    pub fn hold(_: &str, _: &str) -> Option<u32> {
        None
    }
    pub fn release(_: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(text: &str) -> Option<Command> {
        recognise(&crate::skills::words(text))
    }

    #[test]
    fn keeping_awake_is_asked_for_in_plain_words() {
        for text in ["Keep my Mac awake", "keep awake", "Can you keep the Mac awake please?", "Stay awake", "Don't let my Mac sleep", "keep the computer from sleeping", "caffeinate"] {
            assert_eq!(ask(text), Some(Command::Start { seconds: None }), "{text}");
        }
        assert_eq!(ask("Keep my Mac awake for 2 hours"), Some(Command::Start { seconds: Some(7200) }));
        assert_eq!(ask("stay awake for half an hour"), Some(Command::Start { seconds: Some(1800) }));
        assert_eq!(ask("don't let the mac sleep for 45 minutes"), Some(Command::Start { seconds: Some(2700) }));
    }

    #[test]
    fn and_stopped_the_same_way() {
        for text in ["Let my Mac sleep", "let it sleep again", "stop keeping the Mac awake", "stop keeping awake", "you can let the mac go to sleep", "allow sleep"] {
            assert_eq!(ask(text), Some(Command::Stop), "{text}");
        }
    }

    #[test]
    fn anything_else_goes_to_the_model() {
        for text in [
            "keep my kids awake",
            "how do I keep my Mac awake",
            "keep the Mac awake for my download",
            "let the dough sleep",
            "stay awake during the meeting and take notes",
            "keep my Mac awake and play music",
            "awake",
        ] {
            assert_eq!(ask(text), None, "{text}");
        }
    }

    #[test]
    fn the_pill_counts_down_and_the_card_says_until_when() {
        keep(Some(45 * 60));
        let (pill, offer) = shown();
        let (pill, offer) = (pill.unwrap(), offer.unwrap());
        assert_eq!((pill.id.as_str(), pill.label, pill.badge), (ID, t("utilities.awake.label"), Some(tf("utilities.focus.minutes", &[("minutes", &45)]))));
        let CardBody::Awake(card) = offer.card.body else { panic!() };
        assert!(card.until.is_some_and(|until| (until - now() / 1000 - 45 * 60).abs() <= 1));
        keep(None);
        assert_eq!(shown().0.unwrap().badge, None);
        assert!(stop());
        assert!(!stop());
        assert_eq!(shown(), (None, None));
    }

    /// The holds reach macOS: `cargo test --lib utilities::awake::tests::live -- --ignored
    /// --nocapture` prints Buntini's lines from `pmset -g assertions` while each is held.
    #[test]
    #[ignore]
    fn live() {
        for (kind, reason) in [(DISPLAY, "Buntini test: kept awake as asked"), (SYSTEM, "Buntini test: an agent is working")] {
            let id = power::hold(kind, reason).expect("held");
            let listed = std::process::Command::new("/usr/bin/pmset").args(["-g", "assertions"]).output().unwrap();
            let listed = String::from_utf8_lossy(&listed.stdout);
            let line = listed.lines().find(|line| line.contains(reason)).expect("listed by macOS").trim().to_string();
            println!("{line}");
            assert!(line.contains(kind));
            power::release(id);
            let after = std::process::Command::new("/usr/bin/pmset").args(["-g", "assertions"]).output().unwrap();
            assert!(!String::from_utf8_lossy(&after.stdout).contains(reason), "released");
        }
    }
}
