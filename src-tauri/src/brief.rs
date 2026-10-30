//! The morning brief: today's calendar and reminders, read on this Mac and written up
//! without a model, so it is instant and nothing leaves the machine. Each part appears
//! only when the user allowed that app in Settings → Apps.
use crate::{controller::AppState, types::*};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default)]
pub struct Day {
    /// (time, title); None when Calendar isn't allowed or couldn't be read.
    pub events: Option<Vec<(String, String)>>,
    pub reminders: Option<Vec<String>>,
    /// Whether the user allowed Calendar, so the brief can say how to add it.
    pub calendar_allowed: bool,
    pub reminders_allowed: bool,
    /// A bounded read is useful, but must not imply that unlisted time is free.
    pub calendar_coverage: Option<(usize, usize)>,
    pub reminders_coverage: Option<(usize, usize)>,
    /// Unreplied email sorted by `triage`; None when Mail isn't allowed or couldn't be read.
    pub mail: Option<crate::triage::Triage>,
    pub mail_allowed: bool,
    /// Mail was closed and the brief ran on its own, so the inbox was left alone.
    pub mail_skipped: bool,
}

/// Reads today's parts. The automatic brief passes `Launch::IfRunning` so it never opens
/// Mail; "My day" asked for by the user passes `Launch::Allow`.
pub async fn gather(helper: &Path, settings: &crate::types::Settings, launch: crate::apps::Launch) -> Day {
    let calendar_allowed = crate::apps::allowed(settings, "calendar");
    let reminders_allowed = crate::apps::allowed(settings, "reminders");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let end_of_day = format!("{today} 23:59");
    let events = async {
        if !calendar_allowed {
            return None;
        }
        let events = crate::apps::events(helper, &today).await.ok()?;
        Some(events)
    };
    let reminders = async {
        if !reminders_allowed {
            return None;
        }
        let open = crate::apps::reminders(helper).await.ok()?;
        Some(open)
    };
    let mail_allowed = crate::apps::allowed(settings, "mail");
    let mail = async {
        if !mail_allowed {
            return (None, false);
        }
        match crate::triage::run(launch).await {
            Ok(triage) => (Some(triage), false),
            Err(error) => (None, crate::apps::is_not_running(&error)),
        }
    };
    let (events, reminders, (mail, mail_skipped)) = tokio::join!(events, reminders, mail);
    let calendar_coverage = events
        .as_ref()
        .filter(|read| !read.complete())
        .map(|read| (read.items.len(), read.total));
    let reminders_coverage = reminders
        .as_ref()
        .filter(|read| !read.complete())
        .map(|read| (read.items.len(), read.total));
    let events = events.map(|read| {
        read.items
            .into_iter()
            .map(|event| {
                (
                    if event.all_day {
                        String::new()
                    } else {
                        event.start.get(11..).unwrap_or("").to_string()
                    },
                    event.title,
                )
            })
            .collect()
    });
    let reminders = reminders.map(|read| {
        read.items
            .into_iter()
            .filter(|reminder| !reminder.due.is_empty() && reminder.due <= end_of_day)
            .map(|reminder| reminder.title)
            .collect()
    });
    Day {
        events,
        reminders,
        calendar_allowed,
        reminders_allowed,
        calendar_coverage,
        reminders_coverage,
        mail,
        mail_allowed,
        mail_skipped,
    }
}

fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The brief as Buntini would say it.
pub fn compose(
    name: &str,
    now: chrono::DateTime<chrono::Local>,
    day: &Day,
    battery_low: bool,
) -> String {
    let hello = match now.format("%H").to_string().parse::<u32>().unwrap_or(9) {
        0..=11 => "Good morning",
        12..=16 => "Good afternoon",
        _ => "Good evening",
    };
    let mut lines = vec![format!(
        "{hello}{}! It’s {}.",
        if name.trim().is_empty() {
            String::new()
        } else {
            format!(", {}", name.trim())
        },
        now.format("%A %-d %B")
    )];
    match &day.events {
        Some(events) if events.is_empty() && day.calendar_coverage.is_none() => lines.push("Your calendar is clear today.".into()),
        Some(events) => {
            let first = events.iter().take(5).map(|(time, title)| if time.is_empty() { title.clone() } else { format!("{title} at {time}") }).collect::<Vec<_>>();
            lines.push(format!("On your calendar: {}{}.", list(&first), if events.len() > 5 { format!(" and {} more", events.len() - 5) } else { String::new() }));
        }
        None if !day.calendar_allowed => lines.push("Allow Calendar in Settings → Apps and I’ll include your day here.".into()),
        None => lines.push("I couldn’t read your calendar. Check Calendar access in System Settings → Privacy & Security, then try again.".into()),
    }
    if let Some((returned, total)) = day.calendar_coverage {
        lines.push(format!("This only includes the first {returned} of {total} events; unlisted time may still be busy."));
    }
    match &day.reminders {
        Some(reminders) if reminders.is_empty() => {}
        Some(reminders) => lines.push(format!("Reminders due: {}{}.", list(&reminders.iter().take(5).cloned().collect::<Vec<_>>()), if reminders.len() > 5 { format!(" and {} more", reminders.len() - 5) } else { String::new() })),
        None if day.reminders_allowed => lines.push("I couldn’t read your reminders. Check Reminders access in System Settings → Privacy & Security, then try again.".into()),
        None => {}
    }
    if let Some((returned, total)) = day.reminders_coverage {
        lines.push(format!("I checked the first {returned} of {total} open reminders; other due items may be missing."));
    }
    match &day.mail {
        Some(triage) => lines.extend(crate::triage::brief_line(triage)),
        None if day.mail_skipped => lines.push("Mail isn’t open, so I skipped your inbox.".into()),
        None if day.mail_allowed => lines.push("I couldn’t check your email. Check Mail is open and allowed under System Settings → Privacy & Security → Automation.".into()),
        None => {}
    }
    if battery_low {
        lines.push("Your battery is low, so it might be time to plug in.".into());
    }
    lines.join(" ")
}

/// Today's brief, written from the calendar and reminders on this Mac. The automatic
/// once-a-day brief is the built-in routine (`routines::MORNING_BRIEF`): it never opens
/// Mail; the "My day" button may. The window passes its own id so Stop can cancel the
/// brief with `cancel_task` like any other turn.
#[tauri::command]
pub async fn morning_brief(
    automatic: Option<bool>,
    id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<TaskRecord, String> {
    let launch = if automatic.unwrap_or(false) {
        crate::apps::Launch::IfRunning
    } else {
        crate::apps::Launch::Allow
    };
    let id = id
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let cancel = Arc::new(AtomicBool::new(false));
    let settings = {
        let mut active = state.active.lock().unwrap();
        if !active.is_empty() {
            return Err("Finish the current task before preparing the brief.".into());
        }
        // Stop pressed before this brief had started: it doesn't start.
        if state.stopped_early.lock().unwrap().remove(&id) {
            return Err("Morning brief cancelled.".into());
        }
        let settings = state.store.lock().unwrap().settings()?;
        active.insert(id.clone(), cancel.clone());
        settings
    };
    let _claim = crate::controller::ActionClaim {
        state: &state,
        id: id.clone(),
    };
    // Stop ends the reads themselves (their helper processes are killed when dropped),
    // rather than letting Mail, Calendar and Reminders run on and discarding the result.
    let stopped = async {
        while !cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let day = tokio::select! {
        day = crate::brief::gather(state.models.apple_helper(), &settings, launch) => day,
        () = stopped => return Err("Morning brief cancelled.".into()),
    };
    if cancel.load(Ordering::Relaxed) {
        return Err("Morning brief cancelled.".into());
    }
    let battery_low = crate::world::health_now().is_some_and(|health| {
        !health.charging && health.battery_percent.is_some_and(|level| level <= 20)
    });
    let task = TaskRecord {
        id,
        prompt: String::new(),
        answer: crate::brief::compose(&settings.user_name, chrono::Local::now(), &day, battery_low),
        state: "completed".into(),
        created_at: now(),
        sources: vec![],
        files: vec![],
        system: None,
        research: vec![],
        research_report: None,
        prices: vec![],
        cards: vec![],
        backend: "brief".into(),
        // The brief is indexed like any turn, so it records everything it shows.
        touched: crate::skills::day::shares(&day, battery_low)
            .into_iter()
            .map(|share| share.id().to_string())
            .collect(),
        handoff: None,
        action: None,
    };
    let store = state.store.lock().unwrap();
    store.save_task(&task)?;
    crate::routines::record_brief(&store, &task.id)?;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_brief_names_who_is_waiting_for_a_reply_or_says_mail_was_unreadable() {
        let morning = chrono::Local
            .with_ymd_and_hms(2026, 10, 22, 8, 30, 0)
            .unwrap();
        let header = crate::apps::MailHeader {
            received: "2026-10-21 09:00".into(),
            id: "1".into(),
            read: false,
            flagged: true,
            to_me: Some(true),
            to_count: 1,
            sender: "Ana Silva <ana@example.com>".into(),
            subject: "Lease".into(),
        };
        let triage = crate::triage::sort(vec![header], 1, &Default::default());
        let mut day = Day {
            events: Some(vec![]),
            reminders: Some(vec![]),
            calendar_allowed: true,
            reminders_allowed: true,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: Some(triage),
            mail_allowed: true,
            mail_skipped: false,
        };
        assert!(compose("", morning, &day, false)
            .ends_with("Your calendar is clear today. One email looks like it needs a reply, from Ana Silva."));
        day.mail = None;
        assert!(compose("", morning, &day, false).contains("couldn’t check your email"));
        day.mail_allowed = false;
        assert!(!compose("", morning, &day, false).contains("email"));
    }

    #[test]
    fn a_skipped_inbox_is_stated() {
        let morning = chrono::Local
            .with_ymd_and_hms(2026, 10, 22, 8, 30, 0)
            .unwrap();
        let day = Day {
            events: Some(vec![]),
            reminders: Some(vec![]),
            calendar_allowed: true,
            reminders_allowed: true,
            mail_allowed: true,
            mail_skipped: true,
            ..Default::default()
        };
        let answer = compose("", morning, &day, false);
        assert!(answer.ends_with("Mail isn’t open, so I skipped your inbox."), "{answer}");
        assert!(!answer.contains("couldn’t check your email"), "a closed Mail is not a failure");
    }

    #[test]
    fn briefs_read_naturally() {
        let morning = chrono::Local
            .with_ymd_and_hms(2026, 10, 22, 8, 30, 0)
            .unwrap();
        let day = Day {
            events: Some(vec![
                ("09:00".into(), "Standup".into()),
                ("13:00".into(), "Lunch with Sam".into()),
            ]),
            reminders: Some(vec!["Call mum".into()]),
            calendar_allowed: true,
            reminders_allowed: true,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: None,
            mail_allowed: false,
            mail_skipped: false,
        };
        assert_eq!(compose("Sam", morning, &day, false), "Good morning, Sam! It’s Thursday 22 October. On your calendar: Standup at 09:00 and Lunch with Sam at 13:00. Reminders due: Call mum.");
        let quiet = Day {
            events: Some(vec![]),
            reminders: Some(vec![]),
            calendar_allowed: true,
            reminders_allowed: true,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: None,
            mail_allowed: false,
            mail_skipped: false,
        };
        assert_eq!(compose("", morning, &quiet, true), "Good morning! It’s Thursday 22 October. Your calendar is clear today. Your battery is low, so it might be time to plug in.");
        let blocked = Day {
            events: None,
            reminders: None,
            calendar_allowed: true,
            reminders_allowed: true,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: None,
            mail_allowed: false,
            mail_skipped: false,
        };
        assert!(compose("", morning, &blocked, false).contains("Calendar"));
        assert!(compose("", morning, &blocked, false).contains("couldn’t read your reminders"));
        let not_allowed = Day {
            events: None,
            reminders: None,
            calendar_allowed: false,
            reminders_allowed: false,
            calendar_coverage: None,
            reminders_coverage: None,
            mail: None,
            mail_allowed: false,
            mail_skipped: false,
        };
        assert!(compose("", morning, &not_allowed, false).contains("Settings → Apps"));
        assert!(!compose("", morning, &not_allowed, false).contains("couldn’t read your reminders"));
        let partial = Day {
            events: Some(vec![("09:00".into(), "Standup".into())]),
            reminders: Some(vec!["Call mum".into()]),
            calendar_allowed: true,
            reminders_allowed: true,
            calendar_coverage: Some((40, 47)),
            reminders_coverage: Some((50, 63)),
            mail: None,
            mail_allowed: false,
            mail_skipped: false,
        };
        let answer = compose("", morning, &partial, false);
        assert!(
            answer.contains("first 40 of 47 events")
                && answer.contains("unlisted time may still be busy")
        );
        assert!(
            answer.contains("first 50 of 63 open reminders")
                && answer.contains("other due items may be missing")
        );
    }
}
