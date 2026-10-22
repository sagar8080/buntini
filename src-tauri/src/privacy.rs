//! Who may see local data.
//!
//! On-device models see everything Buntini can see, because nothing leaves this Mac.
//! The strict line is egress: search providers and online integrations only receive
//! the exact request the user approved, and an external model backend (a cloud API,
//! Codex, Claude Code, Gemini, …) receives a category of local data only if the user
//! listed it in `Settings::external_shares`. Every model-facing path asks `allows`.
use crate::types::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    OnDevice,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Share {
    /// App timeline, active time and time of day.
    Activity,
    /// Battery, CPU, memory, storage and processes.
    System,
    /// File names and excerpts from approved folders.
    Files,
    /// Earlier conversations.
    History,
    /// Things the user asked Buntini to remember.
    Memory,
}

impl Share {
    pub const ALL: [Share; 5] = [Share::Activity, Share::System, Share::Files, Share::History, Share::Memory];
    pub fn id(self) -> &'static str {
        match self {
            Share::Activity => "activity",
            Share::System => "system",
            Share::Files => "files",
            Share::History => "history",
            Share::Memory => "memory",
        }
    }
}

pub fn allows(locality: Locality, settings: &Settings, share: Share) -> bool {
    match locality {
        Locality::OnDevice => true,
        Locality::External => settings.external_shares.iter().any(|id| id == share.id()),
    }
}

pub fn validate_shares(shares: &[String]) -> Result<(), String> {
    for (index, id) in shares.iter().enumerate() {
        if !Share::ALL.iter().any(|share| share.id() == id) || shares[..index].contains(id) {
            return Err("External model sharing must list activity, system, files, history or memory at most once each.".into());
        }
    }
    Ok(())
}

/// Whether a message asks about the user's own activity or Mac health, which the local
/// context answers directly. Apple's small model routes these to tools unreliably, so for
/// it Buntini answers them from the context alone.
pub fn asks_about_activity(prompt: &str) -> bool {
    let text = prompt.to_lowercase().replace(['’', '\''], "");
    const PHRASES: [&str; 17] = [
        "what am i doing", "what was i doing", "what im doing", "what i am doing", "what have i been", "what was i working", "what am i working",
        "how long have i been", "how long was i", "which apps", "what apps", "what app am i", "am i away", "time at my desk", "at my desk",
        "my battery", "battery level",
    ];
    PHRASES.iter().any(|phrase| text.contains(phrase))
}

/// Context given to a model with every chat: what the user is doing and how the Mac is
/// doing, limited to what this model may see. `None` when there is nothing to share.
pub fn context(locality: Locality, settings: &Settings) -> Option<String> {
    let activity = allows(locality, settings, Share::Activity).then(|| (chrono::Local::now(), crate::world::activity().ok()));
    let health = allows(locality, settings, Share::System).then(crate::world::health_now).flatten();
    describe(activity, health)
}

/// Plain sentences rather than JSON: small local models mix up similarly named fields.
pub fn describe(activity: Option<(chrono::DateTime<chrono::Local>, Option<crate::world::Activity>)>, health: Option<crate::world::Health>) -> Option<String> {
    let mut lines = Vec::new();
    if let Some((now, activity)) = activity {
        lines.push(format!("Time: {}.", now.format("%A %-d %B %Y, %H:%M")));
        if let Some(activity) = activity {
            if let Some(app) = &activity.current_app {
                let doing = match activity.current_category {
                    Some("code") => ", coding",
                    Some("browser") => ", browsing",
                    Some("writing") => ", writing",
                    Some("design") => ", designing",
                    Some("chat") => ", chatting",
                    Some("media") => ", listening or watching",
                    Some("files") => ", managing files",
                    Some("meeting") => ", in a meeting",
                    _ => "",
                };
                lines.push(format!("Right now (the app in front before Buntini was opened): {app}{doing}, for {} minutes in a row.", activity.minutes_in_current_app));
            }
            match (activity.seconds_since_last_input, activity.minutes_active) {
                (Some(idle), _) if idle >= 180 => lines.push(format!("Away from the keyboard for {} minutes.", idle / 60)),
                (_, Some(active)) => lines.push(format!("Active at the Mac for {active} minutes without a break.")),
                _ => {}
            }
            if !activity.apps_last_two_hours.is_empty() {
                let apps = activity.apps_last_two_hours.iter().map(|app| format!("{} {} min", app.name, app.minutes)).collect::<Vec<_>>().join(", ");
                lines.push(format!("Total time per app over the last two hours: {apps}."));
            }
        }
    }
    if let Some(health) = health {
        let battery = match health.battery_percent {
            Some(percent) => format!("battery {percent}% ({})", if health.charging { "charging or plugged in" } else { "not charging" }),
            None => "no battery (desktop Mac)".into(),
        };
        lines.push(format!("Mac: {battery}, CPU {:.0}%, disk {:.0}% free.", health.cpu_percent, health.disk_free_percent));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_device_models_see_everything() {
        let settings = Settings::default();
        assert!(Share::ALL.iter().all(|share| allows(Locality::OnDevice, &settings, *share)));
        assert!(context(Locality::OnDevice, &settings).unwrap().starts_with("Time: "));
    }

    #[test]
    fn external_models_see_only_what_the_user_approved() {
        let mut settings = Settings::default();
        assert!(Share::ALL.iter().all(|share| !allows(Locality::External, &settings, *share)));
        assert_eq!(context(Locality::External, &settings), None);
        settings.external_shares = vec!["activity".into()];
        assert!(allows(Locality::External, &settings, Share::Activity));
        assert!(!allows(Locality::External, &settings, Share::Files));
        assert!(!allows(Locality::External, &settings, Share::System));
        assert!(context(Locality::External, &settings).unwrap().starts_with("Time: "));
    }

    #[test]
    fn context_reads_as_plain_sentences() {
        use crate::world::{Activity, AppTime, Health};
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 21, 18, 40, 0).unwrap();
        let activity = Activity {
            local_time: String::new(),
            current_app: Some("Visual Studio Code".into()),
            current_category: Some("code"),
            minutes_in_current_app: 34,
            seconds_since_last_input: Some(3),
            minutes_active: Some(71),
            apps_last_two_hours: vec![AppTime { name: "Visual Studio Code".into(), category: "code", minutes: 52 }, AppTime { name: "Slack".into(), category: "chat", minutes: 5 }],
            battery_percent: None,
            charging: None,
        };
        let health = Health { battery_percent: Some(41), charging: false, cpu_percent: 23.5, disk_free_percent: 20.7 };
        assert_eq!(
            describe(Some((now, Some(activity.clone()))), Some(health)).unwrap(),
            "Time: Wednesday 21 October 2026, 18:40. Right now (the app in front before Buntini was opened): Visual Studio Code, coding, for 34 minutes in a row. Active at the Mac for 71 minutes without a break. Total time per app over the last two hours: Visual Studio Code 52 min, Slack 5 min. Mac: battery 41% (not charging), CPU 24%, disk 21% free."
        );
        let away = Activity { seconds_since_last_input: Some(600), ..activity };
        assert!(describe(Some((now, Some(away))), None).unwrap().contains("Away from the keyboard for 10 minutes."));
        assert_eq!(describe(None, None), None);
        for question in ["hi buntini what am i doing now?", "What have I been working on today?", "how long have I been at my desk?", "Which apps did I use?", "How's my battery?", "What I’m doing right now?"] {
            assert!(asks_about_activity(question), "{question}");
        }
        for other in ["find my lease pdf", "remind me to stretch", "how much memory am I using?", "tell me a joke"] {
            assert!(!asks_about_activity(other), "{other}");
        }
    }

    #[test]
    fn share_lists_are_validated() {
        assert!(validate_shares(&[]).is_ok());
        assert!(validate_shares(&["files".into(), "history".into()]).is_ok());
        assert!(validate_shares(&["files".into(), "files".into()]).is_err());
        assert!(validate_shares(&["passwords".into()]).is_err());
    }
}
