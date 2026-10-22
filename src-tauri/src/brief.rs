//! The morning brief: today's calendar and reminders, read on this Mac and written up
//! without a model, so it is instant and nothing leaves the machine.
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

// Today's events (time, then title) from every calendar; recurring events that
// Calendar's scripting doesn't expand may be missing.
const EVENTS: &str = r#"set startOfDay to current date
set time of startOfDay to 0
set endOfDay to startOfDay + 86400
set out to {}
tell application "Calendar"
  repeat with c in calendars
    repeat with e in (every event of c whose start date ≥ startOfDay and start date < endOfDay)
      set d to start date of e
      set end of out to (text -2 thru -1 of ("0" & (hours of d))) & ":" & (text -2 thru -1 of ("0" & (minutes of d))) & tab & (summary of e)
    end repeat
  end repeat
end tell
set AppleScript's text item delimiters to linefeed
return out as text"#;
// Open reminders due today or earlier.
const REMINDERS: &str = r#"set endOfDay to current date
set time of endOfDay to 0
set endOfDay to endOfDay + 86400
set out to {}
tell application "Reminders"
  repeat with r in (reminders whose completed is false and due date < endOfDay)
    set end of out to name of r
  end repeat
end tell
set AppleScript's text item delimiters to linefeed
return out as text"#;

async fn read(source: &str) -> Option<Vec<String>> {
    let mut command = Command::new("/usr/bin/osascript");
    command.arg("-e").arg(source).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(12), command.output()).await.ok()?.ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).take(20).map(str::to_string).collect())
}

pub struct Day {
    /// (time, title), or None when Calendar couldn't be read.
    pub events: Option<Vec<(String, String)>>,
    pub reminders: Option<Vec<String>>,
}

pub async fn gather() -> Day {
    let (events, reminders) = tokio::join!(read(EVENTS), read(REMINDERS));
    let mut events: Option<Vec<(String, String)>> = events.map(|lines| lines.into_iter().map(|line| match line.split_once('\t') { Some((time, title)) => (time.to_string(), title.to_string()), None => (String::new(), line) }).collect());
    if let Some(list) = events.as_mut() {
        list.sort();
    }
    Day { events, reminders }
}

fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The brief as Buntini would say it.
pub fn compose(name: &str, now: chrono::DateTime<chrono::Local>, day: &Day, battery_low: bool) -> String {
    let hello = match now.format("%H").to_string().parse::<u32>().unwrap_or(9) {
        0..=11 => "Good morning",
        12..=16 => "Good afternoon",
        _ => "Good evening",
    };
    let mut lines = vec![format!("{hello}{}! It’s {}.", if name.trim().is_empty() { String::new() } else { format!(", {}", name.trim()) }, now.format("%A %-d %B"))];
    match &day.events {
        Some(events) if events.is_empty() => lines.push("Your calendar is clear today.".into()),
        Some(events) => {
            let first = events.iter().take(5).map(|(time, title)| if time.is_empty() { title.clone() } else { format!("{title} at {time}") }).collect::<Vec<_>>();
            lines.push(format!("On your calendar: {}{}.", list(&first), if events.len() > 5 { format!(" and {} more", events.len() - 5) } else { String::new() }));
        }
        None => lines.push("I couldn’t read your calendar. Allow Buntini to use Calendar in System Settings → Privacy & Security → Automation.".into()),
    }
    match &day.reminders {
        Some(reminders) if reminders.is_empty() => {}
        Some(reminders) => lines.push(format!("Reminders due: {}{}.", list(&reminders.iter().take(5).cloned().collect::<Vec<_>>()), if reminders.len() > 5 { format!(" and {} more", reminders.len() - 5) } else { String::new() })),
        None => {}
    }
    if battery_low {
        lines.push("Your battery is low, so it might be time to plug in.".into());
    }
    lines.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn briefs_read_naturally() {
        let morning = chrono::Local.with_ymd_and_hms(2026, 10, 22, 8, 30, 0).unwrap();
        let day = Day { events: Some(vec![("09:00".into(), "Standup".into()), ("13:00".into(), "Lunch with Sam".into())]), reminders: Some(vec!["Call mum".into()]) };
        assert_eq!(compose("Sam", morning, &day, false), "Good morning, Sam! It’s Thursday 22 October. On your calendar: Standup at 09:00 and Lunch with Sam at 13:00. Reminders due: Call mum.");
        let quiet = Day { events: Some(vec![]), reminders: Some(vec![]) };
        assert_eq!(compose("", morning, &quiet, true), "Good morning! It’s Thursday 22 October. Your calendar is clear today. Your battery is low, so it might be time to plug in.");
        let blocked = Day { events: None, reminders: None };
        assert!(compose("", morning, &blocked, false).contains("Automation"));
    }
}
