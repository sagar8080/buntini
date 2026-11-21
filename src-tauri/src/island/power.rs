//! A line in the island when the Mac is plugged in or unplugged, and when its battery runs
//! low: "Charging · 82% · full in 40 min". macOS says when its power source changes (IOKit's
//! power source notification) and what it is now, to any app, without a permission. A Mac
//! without a battery says nothing, and nothing is said while the screen is away.
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

use crate::i18n::tf;
use crate::surface::cards::utilities::Vitals;
use crate::surface::{self, Card, CardBody, Offer, Source, Urgency};

const ID: &str = "power";
/// How long a line shows.
const SAID_MS: u64 = 5_000;
/// The battery is low at this, on battery.
const LOW: u8 = 20;
/// After a change, macOS takes a moment to work out how long charging takes.
const SETTLE: Duration = Duration::from_secs(3);

/// The battery as macOS has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Power {
    pub plugged: bool,
    pub percent: u8,
    pub charging: bool,
    /// Minutes, when macOS has worked them out.
    pub to_full: Option<u32>,
    pub to_empty: Option<u32>,
}

/// What's worth a line between two readings: plugged in, unplugged, or low on battery.
pub fn said(before: Option<&Power>, now: &Power) -> Option<String> {
    let before = before?;
    let mut parts = Vec::new();
    if now.plugged && !before.plugged {
        parts.push(tf(if now.charging { "island.power.charging" } else { "island.power.pluggedIn" }, &[("percent", &now.percent)]));
        if let Some(minutes) = now.to_full.filter(|_| now.charging) {
            parts.push(tf("island.power.fullIn", &[("time", &time(minutes))]));
        }
    } else if !now.plugged && before.plugged {
        parts.push(tf("island.power.onBattery", &[("percent", &now.percent)]));
        if let Some(minutes) = now.to_empty {
            parts.push(tf("island.power.left", &[("time", &time(minutes))]));
        }
    } else if !now.plugged && now.percent <= LOW && before.percent > LOW {
        parts.push(tf("island.power.low", &[("percent", &now.percent)]));
        if let Some(minutes) = now.to_empty {
            parts.push(tf("island.power.left", &[("time", &time(minutes))]));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// "40 min", "2 h", "1 h 20 min".
pub fn time(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => tf("glance.time.minutes", &[("minutes", &minutes)]),
        (hours, 0) => tf("glance.time.hours", &[("hours", &hours)]),
        (hours, minutes) => tf("glance.time.hoursMinutes", &[("hours", &hours), ("minutes", &minutes)]),
    }
}

static CHANGES: OnceLock<mpsc::Sender<()>> = OnceLock::new();

/// Watches the power source and says what changed.
pub fn start() {
    let Some(first) = read() else { return };
    let (sender, receiver) = mpsc::channel();
    if CHANGES.set(sender).is_err() {
        return;
    }
    watch();
    std::thread::spawn(move || {
        let mut last = first;
        while receiver.recv().is_ok() {
            // Several changes at once are one: the cable goes in, charging starts, the
            // estimate comes.
            std::thread::sleep(SETTLE);
            while receiver.try_recv().is_ok() {}
            let Some(now) = read() else { continue };
            let line = said(Some(&last), &now).filter(|_| !crate::media::screen_away());
            last = now;
            if let Some(line) = line {
                super::publish(vec![Offer {
                    card: Card { id: ID.into(), source: Source::Island, body: CardBody::Vitals(Vitals { line }) },
                    urgency: Urgency::Alert,
                    at_ms: surface::now_ms(),
                    ttl_ms: Some(SAID_MS),
                }]);
            }
        }
    });
}

extern "C" fn changed(_context: *mut std::ffi::c_void) {
    if let Some(changes) = CHANGES.get() {
        let _ = changes.send(());
    }
}

#[cfg(target_os = "macos")]
fn watch() {
    use std::ffi::c_void;
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSNotificationCreateRunLoopSource(callback: extern "C" fn(*mut c_void), context: *mut c_void) -> *mut c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRun();
        static kCFRunLoopDefaultMode: *const c_void;
    }
    std::thread::spawn(|| {
        // SAFETY: the source is added to this thread's own run loop, which runs for good.
        unsafe {
            let source = IOPSNotificationCreateRunLoopSource(changed, std::ptr::null_mut());
            if source.is_null() {
                return;
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
            CFRunLoopRun();
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn watch() {}

/// The internal battery now; `None` on a Mac without one.
#[cfg(target_os = "macos")]
pub fn read() -> Option<Power> {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *mut c_void;
        fn IOPSCopyPowerSourcesList(blob: *mut c_void) -> *mut NSArray<AnyObject>;
        fn IOPSGetPowerSourceDescription(blob: *mut c_void, source: *const AnyObject) -> *mut NSDictionary<NSString, AnyObject>;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(object: *const c_void);
    }

    // SAFETY: the snapshot is released once below; the list and descriptions are taken over
    // or retained before use, as their Copy and Get names say.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return None;
        }
        let found = Retained::from_raw(IOPSCopyPowerSourcesList(blob)).and_then(|list| {
            (0..list.count()).find_map(|index| {
                let source = list.objectAtIndex(index);
                let description = Retained::retain(IOPSGetPowerSourceDescription(blob, Retained::as_ptr(&source)))?;
                let value = |key: &str| description.objectForKey(&NSString::from_str(key));
                let number = |key: &str| value(key).and_then(|value| value.downcast::<NSNumber>().ok()).map(|number| number.intValue());
                let text = |key: &str| value(key).and_then(|value| value.downcast::<NSString>().ok()).map(|text| text.to_string());
                if text("Type").as_deref() != Some("InternalBattery") || number("Is Present") == Some(0) {
                    return None;
                }
                let current = number("Current Capacity")?;
                let max = number("Max Capacity").filter(|max| *max > 0).unwrap_or(100);
                let minutes = |key: &str| number(key).filter(|minutes| (1..24 * 60).contains(minutes)).map(|minutes| minutes as u32);
                Some(Power {
                    plugged: text("Power Source State").as_deref() == Some("AC Power"),
                    percent: (current * 100 / max).clamp(0, 100) as u8,
                    charging: number("Is Charging").is_some_and(|charging| charging != 0),
                    to_full: minutes("Time to Full Charge"),
                    to_empty: minutes("Time to Empty"),
                })
            })
        });
        CFRelease(blob);
        found
    }
}

#[cfg(not(target_os = "macos"))]
pub fn read() -> Option<Power> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power(plugged: bool, percent: u8, charging: bool, to_full: Option<u32>, to_empty: Option<u32>) -> Power {
        Power { plugged, percent, charging, to_full, to_empty }
    }

    #[test]
    fn plugging_in_says_how_long_until_full() {
        let before = power(false, 82, false, None, Some(310));
        let line = said(Some(&before), &power(true, 82, true, Some(40), None)).unwrap();
        assert_eq!(line, format!("{} · {}", tf("island.power.charging", &[("percent", &82)]), tf("island.power.fullIn", &[("time", &time(40))])));
        // Optimised charging holds it: plugged in, not charging.
        assert_eq!(said(Some(&before), &power(true, 80, false, None, None)), Some(tf("island.power.pluggedIn", &[("percent", &80)])));
    }

    #[test]
    fn unplugging_says_what_is_left_when_macos_knows() {
        let before = power(true, 100, false, None, None);
        assert_eq!(said(Some(&before), &power(false, 100, false, None, None)), Some(tf("island.power.onBattery", &[("percent", &100)])));
        let line = said(Some(&before), &power(false, 100, false, None, Some(310))).unwrap();
        assert!(line.ends_with(&tf("island.power.left", &[("time", &time(310))])));
    }

    #[test]
    fn a_low_battery_is_said_once_as_it_crosses() {
        let line = said(Some(&power(false, 21, false, None, None)), &power(false, 20, false, None, Some(42))).unwrap();
        assert!(line.starts_with(&tf("island.power.low", &[("percent", &20)])));
        assert_eq!(said(Some(&power(false, 20, false, None, None)), &power(false, 19, false, None, None)), None);
        assert_eq!(said(Some(&power(true, 21, true, None, None)), &power(true, 20, true, None, None)), None);
        // Nothing else is worth a line, and the first reading never is.
        assert_eq!(said(Some(&power(false, 60, false, None, None)), &power(false, 59, false, None, Some(200))), None);
        assert_eq!(said(None, &power(true, 82, true, Some(40), None)), None);
    }

    #[test]
    fn times_read_in_minutes_and_hours() {
        assert_eq!(time(40), tf("glance.time.minutes", &[("minutes", &40)]));
        assert_eq!(time(120), tf("glance.time.hours", &[("hours", &2)]));
        assert_eq!(time(310), tf("glance.time.hoursMinutes", &[("hours", &5), ("minutes", &10)]));
    }

    /// macOS's change notice arrives through the run loop: `cargo test --lib
    /// island::power::tests::notified -- --ignored --nocapture`. Waits up to
    /// `BUNTINI_LIVE_SECONDS` (300 unless set) for the battery to change by itself.
    #[test]
    #[ignore]
    fn notified() {
        let seconds: u64 = std::env::var("BUNTINI_LIVE_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(300);
        let (sender, receiver) = mpsc::channel();
        CHANGES.set(sender).unwrap();
        let before = read();
        watch();
        let started = std::time::Instant::now();
        receiver.recv_timeout(Duration::from_secs(seconds)).expect("a change notice");
        println!("notified after {:.0} s: {before:?} → {:?}", started.elapsed().as_secs_f64(), read());
    }

    /// The battery on this Mac now: `cargo test --lib island::power::tests::live -- --ignored
    /// --nocapture`. Plug in or unplug meanwhile to see the line it says.
    #[test]
    #[ignore]
    fn live() {
        let seconds: u64 = std::env::var("BUNTINI_LIVE_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(5);
        let mut last = read();
        println!("{last:?}");
        for _ in 0..seconds {
            std::thread::sleep(Duration::from_secs(1));
            let now = read();
            if now != last {
                println!("{now:?} → {:?}", now.as_ref().and_then(|now| said(last.as_ref(), now)));
                last = now;
            }
        }
    }
}
