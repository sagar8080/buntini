//! The morning brief: today's calendar and reminders, read on this Mac and written up
//! without a model, so it is instant and nothing leaves the machine. Each part appears
//! only when the user allowed that app in Settings → Apps.
use std::path::Path;

pub struct Day {
    /// (time, title); None when Calendar isn't allowed or couldn't be read.
    pub events: Option<Vec<(String, String)>>,
    pub reminders: Option<Vec<String>>,
    /// Whether the user allowed Calendar, so the brief can say how to add it.
    pub calendar_allowed: bool,
}

pub async fn gather(helper: &Path, settings: &crate::types::Settings) -> Day {
    let calendar_allowed = crate::apps::allowed(settings, "calendar");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let end_of_day = format!("{today} 23:59");
    let events = async {
        if !calendar_allowed {
            return None;
        }
        let events = crate::apps::events(helper, &today).await.ok()?;
        Some(events.into_iter().take(20).map(|event| (if event.all_day { String::new() } else { event.start.get(11..).unwrap_or("").to_string() }, event.title)).collect())
    };
    let reminders = async {
        if !crate::apps::allowed(settings, "reminders") {
            return None;
        }
        let open = crate::apps::reminders(helper).await.ok()?;
        Some(open.into_iter().filter(|reminder| !reminder.due.is_empty() && reminder.due <= end_of_day).take(20).map(|reminder| reminder.title).collect())
    };
    let (events, reminders) = tokio::join!(events, reminders);
    Day { events, reminders, calendar_allowed }
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
        None if !day.calendar_allowed => lines.push("Allow Calendar in Settings → Apps and I’ll include your day here.".into()),
        None => lines.push("I couldn’t read your calendar. Allow Buntini in System Settings → Privacy & Security → Calendars.".into()),
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
        let day = Day { events: Some(vec![("09:00".into(), "Standup".into()), ("13:00".into(), "Lunch with Sam".into())]), reminders: Some(vec!["Call mum".into()]), calendar_allowed: true };
        assert_eq!(compose("Sam", morning, &day, false), "Good morning, Sam! It’s Thursday 22 October. On your calendar: Standup at 09:00 and Lunch with Sam at 13:00. Reminders due: Call mum.");
        let quiet = Day { events: Some(vec![]), reminders: Some(vec![]), calendar_allowed: true };
        assert_eq!(compose("", morning, &quiet, true), "Good morning! It’s Thursday 22 October. Your calendar is clear today. Your battery is low, so it might be time to plug in.");
        let blocked = Day { events: None, reminders: None, calendar_allowed: true };
        assert!(compose("", morning, &blocked, false).contains("Calendars"));
        let not_allowed = Day { events: None, reminders: None, calendar_allowed: false };
        assert!(compose("", morning, &not_allowed, false).contains("Settings → Apps"));
    }
}
