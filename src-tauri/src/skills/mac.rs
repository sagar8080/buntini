//! Mac controls: volume, mute, dark mode, locking the screen and timers. Each change is
//! read back, so the reply says the Mac's actual state ("Volume is at 30%").
use super::{from_verb, osascript, Outcome};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};

#[derive(Debug, PartialEq)]
pub enum Command {
    Volume(Volume),
    DarkMode(Option<bool>),
    Lock,
    Timer { seconds: u64 },
    CancelTimers,
}

#[derive(Debug, PartialEq)]
pub enum Volume {
    Set(u8),
    Up,
    Down,
    Mute,
    Unmute,
}

/// Numbers people say, as digits or words.
fn number(word: &str) -> Option<u64> {
    word.trim_end_matches('%').parse().ok().or_else(|| {
        let words = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty"];
        match word {
            "a" | "an" => Some(1),
            "thirty" => Some(30),
            "forty" => Some(40),
            "fifty" => Some(50),
            "sixty" => Some(60),
            _ => words.iter().position(|candidate| *candidate == word).map(|index| index as u64),
        }
    })
}

/// "10 minutes", "a minute and a half", "half an hour", "90 seconds", "2 hours".
fn duration(text: &str) -> Option<u64> {
    if text.contains("half an hour") || text.contains("half hour") {
        return Some(30 * 60);
    }
    let words: Vec<&str> = text.split(' ').collect();
    let mut total = 0;
    for (index, word) in words.iter().enumerate() {
        let unit = match word.trim_end_matches('s') {
            "second" | "sec" => 1,
            "minute" | "min" => 60,
            "hour" | "hr" => 3600,
            _ => continue,
        };
        if let Some(amount) = index.checked_sub(1).and_then(|before| number(words[before])) {
            total += amount * unit;
        }
        if words.get(index + 1..index + 4).is_some_and(|after| after == ["and", "a", "half"]) {
            total += unit / 2;
        }
    }
    (total > 0 && total <= 24 * 3600).then_some(total)
}

pub fn recognise(text: &str) -> Option<Command> {
    // Volume.
    let louder = ["louder", "volume up", "turn it up", "turn up the volume", "turn the volume up", "increase the volume", "raise the volume", "increase volume"];
    let quieter = ["quieter", "softer", "volume down", "turn it down", "turn down the volume", "turn the volume down", "lower the volume", "decrease the volume", "decrease volume", "reduce the volume"];
    if text.contains("volume") || louder.iter().chain(&quieter).any(|phrase| text.contains(phrase)) || from_verb(text, &["mute", "unmute"]).is_some() {
        if from_verb(text, &["unmute"]).is_some() {
            return Some(Command::Volume(Volume::Unmute));
        }
        if from_verb(text, &["mute"]).is_some() {
            return Some(Command::Volume(Volume::Mute));
        }
        // "volume to 30%", "volume at 50", "volume 70": the word right after.
        let words: Vec<&str> = text.split(' ').collect();
        let level = words.windows(2).find_map(|pair| (["to", "at", "volume"].contains(&pair[0]) && !["a", "an"].contains(&pair[1])).then(|| number(pair[1])).flatten());
        if let Some(level) = level.filter(|level| *level <= 100 && text.contains("volume")) {
            return Some(Command::Volume(Volume::Set(level as u8)));
        }
        if louder.iter().any(|phrase| text.contains(phrase)) {
            return Some(Command::Volume(Volume::Up));
        }
        if quieter.iter().any(|phrase| text.contains(phrase)) {
            return Some(Command::Volume(Volume::Down));
        }
        return None;
    }
    // Dark and light mode.
    if text.contains("dark mode") || text.contains("light mode") {
        let wants_dark = text.contains("dark mode");
        let off = [" off", "disable", "turn off", "stop"].iter().any(|word| text.contains(word));
        if text.contains("toggle") {
            return Some(Command::DarkMode(None));
        }
        if from_verb(text, &["turn", "switch", "enable", "disable", "use", "go", "set", "change", "put", "toggle", "activate"]).is_some() || text.starts_with("dark mode") || text.starts_with("light mode") {
            return Some(Command::DarkMode(Some(wants_dark != off)));
        }
        return None;
    }
    // Lock the screen.
    if let Some((_, rest)) = from_verb(text, &["lock"]) {
        if ["screen", "mac", "computer", "laptop"].iter().any(|thing| rest.contains(thing)) && rest.split(' ').count() <= 3 {
            return Some(Command::Lock);
        }
    }
    // Timers.
    if text.contains("timer") {
        if from_verb(text, &["cancel", "stop", "clear", "delete", "remove"]).is_some() {
            return Some(Command::CancelTimers);
        }
        if let Some(seconds) = duration(text) {
            return Some(Command::Timer { seconds });
        }
    }
    None
}

const VOLUME: &str = r#"on run argv
  set act to item 1 of argv
  if act is "set" then set volume output volume ((item 2 of argv) as integer) without output muted
  if act is "up" then set volume output volume ((output volume of (get volume settings)) + 10) without output muted
  if act is "down" then set volume output volume ((output volume of (get volume settings)) - 10)
  if act is "mute" then set volume with output muted
  if act is "unmute" then set volume without output muted
  set settings to get volume settings
  return ((output volume of settings) as text) & tab & ((output muted of settings) as text)
end run
"#;
const DARK_MODE: &str = r#"on run argv
  tell application "System Events"
    tell appearance preferences
      if item 1 of argv is "toggle" then
        set dark mode to not dark mode
      else
        set dark mode to ((item 1 of argv) is "dark")
      end if
      return dark mode as text
    end tell
  end tell
end run
"#;
const LOCK: &str = r#"tell application "System Events" to keystroke "q" using {control down, command down}"#;

/// Where finished timers are announced (set at launch).
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static TIMER_IDS: AtomicU64 = AtomicU64::new(0);
static TIMERS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

pub fn set_app(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

fn spoken(seconds: u64) -> String {
    let (hours, minutes, rest) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let part = |amount: u64, unit: &str| match amount {
        0 => None,
        1 => Some(format!("1 {unit}")),
        _ => Some(format!("{amount} {unit}s")),
    };
    [part(hours, "hour"), part(minutes, "minute"), part(rest, "second")].into_iter().flatten().collect::<Vec<_>>().join(" and ")
}

pub async fn run(command: Command) -> Result<Outcome, String> {
    let answer = match command {
        Command::Volume(change) => {
            let (action, level) = match change {
                Volume::Set(level) => ("set", level.to_string()),
                Volume::Up => ("up", String::new()),
                Volume::Down => ("down", String::new()),
                Volume::Mute => ("mute", String::new()),
                Volume::Unmute => ("unmute", String::new()),
            };
            let state = osascript(VOLUME, &[action, &level], 10).await?;
            match state.split('\t').collect::<Vec<_>>().as_slice() {
                [_, "true"] => "Your Mac is muted.".to_string(),
                [level, _] => format!("Volume is at {level}%."),
                _ => "Changed the volume.".to_string(),
            }
        }
        Command::DarkMode(wanted) => {
            let argument = match wanted {
                None => "toggle",
                Some(true) => "dark",
                Some(false) => "light",
            };
            let dark = osascript(DARK_MODE, &[argument], 15).await?;
            if dark == "true" { "Dark mode is on." } else { "Light mode is on." }.to_string()
        }
        Command::Lock => {
            if !tokio::task::spawn_blocking(|| crate::ax::trusted(true)).await.unwrap_or(false) {
                return Err("To lock the screen for you, allow Buntini under System Settings → Privacy & Security → Accessibility, then ask again.".into());
            }
            osascript(LOCK, &[], 10).await?;
            "Locked your screen.".to_string()
        }
        Command::Timer { seconds } => {
            let id = TIMER_IDS.fetch_add(1, Ordering::SeqCst) + 1;
            TIMERS.lock().unwrap().push(id);
            let length = spoken(seconds);
            let finished = format!("Your {length} timer is done.");
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
                let still_set = {
                    let mut timers = TIMERS.lock().unwrap();
                    let found = timers.contains(&id);
                    timers.retain(|timer| *timer != id);
                    found
                };
                if let (true, Some(app)) = (still_set, APP.get()) {
                    let _ = tauri::Emitter::emit(app, "timer-done", &finished);
                    crate::notifications::deliver(app, "Timer", &finished).await;
                }
            });
            let at = chrono::Local::now() + chrono::Duration::seconds(seconds as i64);
            format!("Timer set for {length}. I’ll tell you at {}.", at.format("%-H:%M"))
        }
        Command::CancelTimers => {
            let count = std::mem::take(&mut *TIMERS.lock().unwrap()).len();
            match count {
                0 => "There’s no timer running.".to_string(),
                1 => "Cancelled your timer.".to_string(),
                _ => format!("Cancelled your {count} timers."),
            }
        }
    };
    Ok(Outcome::Done { answer, share: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::words;

    fn ask(prompt: &str) -> Option<Command> {
        recognise(&words(prompt))
    }

    #[test]
    fn everyday_mac_requests_are_recognised() {
        assert_eq!(ask("Set the volume to 30%"), Some(Command::Volume(Volume::Set(30))));
        assert_eq!(ask("volume 70"), Some(Command::Volume(Volume::Set(70))));
        assert_eq!(ask("turn the volume down a bit"), Some(Command::Volume(Volume::Down)));
        assert_eq!(ask("louder please"), Some(Command::Volume(Volume::Up)));
        assert_eq!(ask("mute my mac"), Some(Command::Volume(Volume::Mute)));
        assert_eq!(ask("unmute"), Some(Command::Volume(Volume::Unmute)));
        assert_eq!(ask("turn on dark mode"), Some(Command::DarkMode(Some(true))));
        assert_eq!(ask("switch to light mode"), Some(Command::DarkMode(Some(false))));
        assert_eq!(ask("turn off dark mode"), Some(Command::DarkMode(Some(false))));
        assert_eq!(ask("lock my screen"), Some(Command::Lock));
        assert_eq!(ask("Set a timer for 10 minutes"), Some(Command::Timer { seconds: 600 }));
        assert_eq!(ask("start a two minute timer"), Some(Command::Timer { seconds: 120 }));
        assert_eq!(ask("timer for a minute and a half"), Some(Command::Timer { seconds: 90 }));
        assert_eq!(ask("set a timer for half an hour"), Some(Command::Timer { seconds: 1800 }));
        assert_eq!(ask("cancel the timer"), Some(Command::CancelTimers));
        assert_eq!(ask("what is dark mode good for?"), None, "questions go to the model");
        assert_eq!(ask("how do I lock my screen"), None);
        assert_eq!(ask("lock the door reminder at 9pm"), None);
        assert_eq!(ask("the volume of a sphere"), None);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(spoken(600), "10 minutes");
        assert_eq!(spoken(90), "1 minute and 30 seconds");
        assert_eq!(spoken(3600), "1 hour");
    }
}
