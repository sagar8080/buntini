//! The week with the coding agents, counted on this Mac from their hook events and never
//! sent anywhere: time working, turns, sessions, lines changed, commands run, what was
//! allowed or denied, the top agent and project, the busiest day and the longest session.
//! Twelve weeks are kept. After Coucou's RecapStore (MIT).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use crate::i18n::{t, tf};

const WEEKS_KEPT: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Week {
    /// The Monday it starts on, `YYYY-MM-DD`.
    pub start: String,
    pub turns: u64,
    pub seconds: u64,
    pub sessions: BTreeSet<String>,
    pub files: BTreeSet<String>,
    pub added: u64,
    pub removed: u64,
    pub commands: u64,
    pub allowed: u64,
    pub denied: u64,
    pub agents: BTreeMap<String, u64>,
    pub projects: BTreeMap<String, u64>,
    /// Seconds per weekday, 0 = Monday.
    pub days: [u64; 7],
    /// Each session's working seconds, for the longest one.
    pub session_seconds: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Recap {
    pub weeks: Vec<Week>,
    /// The Monday whose week was last shown, so it shows once.
    pub shown: String,
}

/// The Monday of the week `date` falls in.
pub fn monday(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

impl Recap {
    pub fn load(path: &Path) -> Recap {
        std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let temp = path.with_extension("tmp");
            if std::fs::write(&temp, bytes).is_ok() {
                let _ = std::fs::rename(&temp, path);
            }
        }
    }

    fn week(&mut self, today: NaiveDate) -> &mut Week {
        let start = monday(today).format("%Y-%m-%d").to_string();
        if self.weeks.last().is_none_or(|week| week.start != start) {
            self.weeks.push(Week { start, ..Week::default() });
            let extra = self.weeks.len().saturating_sub(WEEKS_KEPT);
            self.weeks.drain(..extra);
        }
        self.weeks.last_mut().unwrap()
    }

    pub fn turn(&mut self, today: NaiveDate, agent: &str, session: &str, project: &str, seconds: u64) {
        let week = self.week(today);
        week.turns += 1;
        week.seconds += seconds;
        if week.sessions.len() < 2_000 {
            week.sessions.insert(format!("{agent}:{session}"));
        }
        *week.agents.entry(agent.into()).or_default() += 1;
        if !project.is_empty() && (week.projects.len() < 200 || week.projects.contains_key(project)) {
            *week.projects.entry(project.into()).or_default() += seconds;
        }
        week.days[today.weekday().num_days_from_monday() as usize] += seconds;
        if week.session_seconds.len() < 2_000 {
            *week.session_seconds.entry(format!("{agent}:{session}")).or_default() += seconds;
        }
    }

    pub fn edit(&mut self, today: NaiveDate, file: &str, added: u64, removed: u64) {
        let week = self.week(today);
        if !file.is_empty() && week.files.len() < 5_000 {
            week.files.insert(file.into());
        }
        week.added += added;
        week.removed += removed;
    }

    pub fn command(&mut self, today: NaiveDate) {
        self.week(today).commands += 1;
    }

    pub fn decision(&mut self, today: NaiveDate, allowed: bool) {
        let week = self.week(today);
        if allowed { week.allowed += 1 } else { week.denied += 1 }
    }

    /// The finished week to show on a Monday, once. None before any work or once shown.
    pub fn due(&mut self, today: NaiveDate) -> Option<Week> {
        let this = monday(today).format("%Y-%m-%d").to_string();
        let last = monday(today - chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
        if self.shown == this {
            return None;
        }
        let week = self.weeks.iter().find(|week| week.start == last && week.turns > 0)?.clone();
        self.shown = this;
        Some(week)
    }
}

/// A day of the week by name, 0 = Monday.
fn weekday(day: usize) -> String {
    match day {
        0 => t("common.weekday.monday"),
        1 => t("common.weekday.tuesday"),
        2 => t("common.weekday.wednesday"),
        3 => t("common.weekday.thursday"),
        4 => t("common.weekday.friday"),
        5 => t("common.weekday.saturday"),
        _ => t("common.weekday.sunday"),
    }
}

/// A month, short: `Nov` for 11. Service cards use it too.
pub fn month_short(month: u32) -> String {
    match month {
        1 => t("common.monthShort.jan"),
        2 => t("common.monthShort.feb"),
        3 => t("common.monthShort.mar"),
        4 => t("common.monthShort.apr"),
        5 => t("common.monthShort.may"),
        6 => t("common.monthShort.jun"),
        7 => t("common.monthShort.jul"),
        8 => t("common.monthShort.aug"),
        9 => t("common.monthShort.sep"),
        10 => t("common.monthShort.oct"),
        11 => t("common.monthShort.nov"),
        _ => t("common.monthShort.dec"),
    }
}

/// The week as a few labelled numbers, for the card and the picture.
pub fn stats(week: &Week, names: impl Fn(&str) -> String, projects: bool) -> Vec<(String, String)> {
    let mut out = vec![(t("agents.recap.time"), duration(week.seconds)), (t("agents.recap.sessions"), week.sessions.len().to_string()), (t("agents.recap.turns"), week.turns.to_string())];
    out.push((t("agents.recap.lines"), format!("+{} −{}", week.added, week.removed)));
    out.push((t("agents.recap.files"), week.files.len().to_string()));
    out.push((t("agents.recap.commands"), week.commands.to_string()));
    if week.allowed + week.denied > 0 {
        out.push((t("agents.recap.allowed"), tf("agents.recap.allowedDenied", &[("allowed", &week.allowed), ("denied", &week.denied)])));
    }
    if let Some((agent, _)) = week.agents.iter().max_by_key(|(_, turns)| **turns) {
        out.push((t("agents.recap.topAgent"), names(agent)));
    }
    if projects {
        if let Some((project, _)) = week.projects.iter().max_by_key(|(_, seconds)| **seconds) {
            out.push((t("agents.recap.topProject"), project.clone()));
        }
    }
    if let Some((day, _)) = week.days.iter().enumerate().filter(|(_, seconds)| **seconds > 0).max_by_key(|(_, seconds)| **seconds) {
        out.push((t("agents.recap.busiestDay"), weekday(day)));
    }
    if let Some(longest) = week.session_seconds.values().max().filter(|seconds| **seconds > 0) {
        out.push((t("agents.recap.longestSession"), duration(*longest)));
    }
    out
}

pub fn duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    if minutes < 60 {
        tf("agents.recap.minutes", &[("minutes", &minutes)])
    } else {
        tf("agents.recap.hoursMinutes", &[("hours", &(minutes / 60)), ("minutes", &format!("{:02}", minutes % 60))])
    }
}

/// "10–16 Nov" for a week starting on `start`.
pub fn title(start: &str) -> String {
    let Ok(monday) = NaiveDate::parse_from_str(start, "%Y-%m-%d") else { return start.into() };
    let sunday = monday + chrono::Duration::days(6);
    if monday.month() == sunday.month() {
        tf("agents.recap.weekInMonth", &[("first", &monday.day()), ("last", &sunday.day()), ("month", &month_short(sunday.month()))])
    } else {
        tf("agents.recap.weekAcrossMonths", &[("first", &monday.day()), ("firstMonth", &month_short(monday.month())), ("last", &sunday.day()), ("lastMonth", &month_short(sunday.month()))])
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Where the recap is kept.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("agents-recap.json")
}

/// Monday from six in the morning, local time.
pub fn monday_morning() -> bool {
    let now = Local::now();
    now.weekday() == chrono::Weekday::Mon && Local.from_local_datetime(&now.date_naive().and_hms_opt(6, 0, 0).unwrap()).single().is_some_and(|six| now >= six)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn weeks_start_on_monday_and_only_twelve_are_kept() {
        assert_eq!(monday(day("2026-11-16")), day("2026-11-16"));
        assert_eq!(monday(day("2026-11-22")), day("2026-11-16"));
        let mut recap = Recap::default();
        for week in 0..15 {
            recap.command(day("2026-01-05") + chrono::Duration::days(7 * week));
        }
        assert_eq!(recap.weeks.len(), WEEKS_KEPT);
        assert_eq!(recap.weeks[0].start, "2026-01-26");
    }

    #[test]
    fn a_week_adds_up() {
        let mut recap = Recap::default();
        let tuesday = day("2026-11-10");
        recap.turn(tuesday, "claude", "s1", "shop", 600);
        recap.turn(tuesday, "claude", "s1", "shop", 1_200);
        recap.turn(day("2026-11-12"), "codex", "s2", "blog", 300);
        recap.edit(tuesday, "cart.rs", 12, 3);
        recap.edit(tuesday, "cart.rs", 1, 1);
        recap.command(tuesday);
        recap.decision(tuesday, true);
        recap.decision(tuesday, false);
        let week = &recap.weeks[0];
        assert_eq!((week.turns, week.seconds, week.sessions.len(), week.files.len(), week.added, week.removed), (3, 2_100, 2, 1, 13, 4));
        let stats = stats(week, |id| id.to_uppercase(), false);
        let get = |label: &str| stats.iter().find(|(name, _)| name == label).map(|(_, value)| value.clone());
        assert_eq!(get("Time").as_deref(), Some("35 min"));
        assert_eq!(get("Top agent").as_deref(), Some("CLAUDE"));
        assert_eq!(get("Busiest day").as_deref(), Some("Tuesday"));
        assert_eq!(get("Longest session").as_deref(), Some("30 min"));
        assert_eq!(get("Allowed").as_deref(), Some("1 · 1 denied"));
        assert_eq!(get("Top project"), None, "projects only when asked for");
    }

    #[test]
    fn last_week_shows_once_on_monday() {
        let mut recap = Recap::default();
        recap.turn(day("2026-11-12"), "claude", "s1", "", 60);
        assert!(recap.due(day("2026-11-13")).is_none(), "not before the week is over");
        let week = recap.due(day("2026-11-16")).unwrap();
        assert_eq!(week.start, "2026-11-09");
        assert!(recap.due(day("2026-11-16")).is_none());
        assert_eq!(title("2026-11-09"), "9–15 Nov");
        assert_eq!(title("2026-11-30"), "30 Nov – 6 Dec");
    }

    #[test]
    fn it_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut recap = Recap::default();
        recap.command(day("2026-11-10"));
        recap.save(&path(dir.path()));
        assert_eq!(Recap::load(&path(dir.path())), recap);
        std::fs::write(path(dir.path()), b"not json").unwrap();
        assert_eq!(Recap::load(&path(dir.path())), Recap::default());
    }
}
