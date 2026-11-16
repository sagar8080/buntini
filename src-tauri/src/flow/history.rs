//! Dictation history, statistics and call notes, in Buntini's own database.
//!
//! The words of a dictation are kept 30 days for the Library, then only its counts stay.
//! Clear history removes the words and notes; Delete everything removes it all.
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// How fast people type, for "time saved".
pub const TYPING_WPM: f64 = 40.0;
/// Days a dictation's words are kept.
pub const KEEP_DAYS: i64 = 30;

pub fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS flow_dictations (id INTEGER PRIMARY KEY, at TEXT NOT NULL, day TEXT NOT NULL, app TEXT NOT NULL, bundle TEXT NOT NULL, words INTEGER NOT NULL, speaking_ms INTEGER NOT NULL, text TEXT NOT NULL, pasted INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS flow_dictations_day ON flow_dictations(day);
         CREATE TABLE IF NOT EXISTS flow_notes (id TEXT PRIMARY KEY, data TEXT NOT NULL, created_at TEXT NOT NULL);",
    )
    .map_err(|e| format!("Cannot prepare dictation history: {e}"))
}

/// Clear history: the words go, the counts stay.
pub fn clear(db: &Connection) -> Result<(), String> {
    db.execute_batch("UPDATE flow_dictations SET text=''; DELETE FROM flow_notes;").map_err(|e| e.to_string())
}

/// Delete everything.
pub fn erase(db: &Connection) -> Result<(), String> {
    db.execute_batch("DELETE FROM flow_dictations; DELETE FROM flow_notes;").map_err(|e| e.to_string())
}

pub fn count_words(text: &str) -> u32 {
    text.split_whitespace().filter(|word| word.chars().any(char::is_alphanumeric)).count() as u32
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dictation {
    pub id: i64,
    pub at: String,
    pub app: String,
    pub words: u32,
    pub text: String,
    /// False: it was copied rather than typed.
    pub pasted: bool,
}

pub struct NewDictation<'a> {
    pub app: &'a str,
    pub bundle: &'a str,
    pub text: &'a str,
    pub speaking_ms: u64,
    pub pasted: bool,
}

pub fn record(db: &Connection, entry: &NewDictation, now: DateTime<Utc>) -> Result<i64, String> {
    let words = count_words(entry.text);
    let day = now.with_timezone(&Local).date_naive().to_string();
    db.execute(
        "INSERT INTO flow_dictations(at,day,app,bundle,words,speaking_ms,text,pasted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![now.to_rfc3339(), day, entry.app, entry.bundle, words, entry.speaking_ms as i64, entry.text, entry.pasted],
    )
    .map_err(|e| e.to_string())?;
    let id = db.last_insert_rowid();
    // Old words go; their counts stay for the statistics.
    let cutoff = (now - Duration::days(KEEP_DAYS)).to_rfc3339();
    db.execute("UPDATE flow_dictations SET text='' WHERE at < ?1 AND text <> ''", [cutoff]).map_err(|e| e.to_string())?;
    Ok(id)
}

/// The latest dictations whose words are still kept, newest first.
pub fn recent(db: &Connection, limit: usize) -> Result<Vec<Dictation>, String> {
    let mut statement = db
        .prepare("SELECT id,at,app,words,text,pasted FROM flow_dictations WHERE text <> '' ORDER BY id DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok(Dictation { id: row.get(0)?, at: row.get(1)?, app: row.get(2)?, words: row.get(3)?, text: row.get(4)?, pasted: row.get(5)? })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// The last thing dictated, for ⌃⌘V.
pub fn last_text(db: &Connection) -> Result<Option<String>, String> {
    db.query_row("SELECT text FROM flow_dictations WHERE text <> '' ORDER BY id DESC LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())
}

pub fn forget(db: &Connection, id: i64) -> Result<(), String> {
    db.execute("UPDATE flow_dictations SET text='' WHERE id=?1", [id]).map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppWords {
    pub app: String,
    pub words: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub words: u64,
    pub dictations: u64,
    /// Minutes saved against typing at 40 words a minute.
    pub minutes_saved: f64,
    pub words_per_minute: f64,
    /// Days in a row with a dictation, up to today (or yesterday, if nothing yet today).
    pub streak: u32,
    pub apps: Vec<AppWords>,
}

/// Days in a row ending today, or yesterday when today has none yet.
pub fn streak(days: &[NaiveDate], today: NaiveDate) -> u32 {
    let mut sorted: Vec<NaiveDate> = days.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut day = if sorted.binary_search(&today).is_ok() { today } else { today - Duration::days(1) };
    let mut count = 0;
    while sorted.binary_search(&day).is_ok() {
        count += 1;
        day -= Duration::days(1);
    }
    count
}

pub fn stats(db: &Connection, today: NaiveDate) -> Result<Stats, String> {
    let (words, dictations, speaking_ms): (i64, i64, i64) = db
        .query_row("SELECT COALESCE(SUM(words),0), COUNT(*), COALESCE(SUM(speaking_ms),0) FROM flow_dictations", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| e.to_string())?;
    // Saved per dictation, so a slow one doesn't take time back from the rest.
    let mut statement = db.prepare("SELECT words, speaking_ms FROM flow_dictations").map_err(|e| e.to_string())?;
    let saved_ms: f64 = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|(words, spoken)| (words as f64 / TYPING_WPM * 60_000.0 - spoken as f64).max(0.0))
        .sum();
    let mut statement = db.prepare("SELECT DISTINCT day FROM flow_dictations").map_err(|e| e.to_string())?;
    let days: Vec<NaiveDate> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|day| day.parse().ok())
        .collect();
    let mut statement = db
        .prepare("SELECT app, SUM(words) AS total FROM flow_dictations GROUP BY app ORDER BY total DESC, app LIMIT 5")
        .map_err(|e| e.to_string())?;
    let apps = statement
        .query_map([], |row| Ok(AppWords { app: row.get(0)?, words: row.get::<_, i64>(1)? as u64 }))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let minutes = speaking_ms as f64 / 60_000.0;
    Ok(Stats {
        words: words as u64,
        dictations: dictations as u64,
        minutes_saved: (saved_ms / 60_000.0 * 10.0).round() / 10.0,
        words_per_minute: if minutes > 0.0 { (words as f64 / minutes).round() } else { 0.0 },
        streak: streak(&days, today),
        apps,
    })
}

/// A recorded call or meeting, with what the brain made of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub seconds: f64,
    /// `recording`, `processing`, `ready` or `failed`.
    pub state: String,
    pub lines: Vec<crate::surface::cards::flow::NoteLine>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub todos: Vec<String>,
}

pub fn save_note(db: &Connection, note: &Note) -> Result<(), String> {
    let data = serde_json::to_string(note).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO flow_notes(id,data,created_at) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![note.id, data, note.created_at],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn notes(db: &Connection) -> Result<Vec<Note>, String> {
    let mut statement = db.prepare("SELECT data FROM flow_notes ORDER BY created_at DESC LIMIT 200").map_err(|e| e.to_string())?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).filter_map(|data| serde_json::from_str(&data).ok()).collect())
}

pub fn delete_note(db: &Connection, id: &str) -> Result<(), String> {
    db.execute("DELETE FROM flow_notes WHERE id=?1", [id]).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        db
    }

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn streaks_count_days_in_a_row_up_to_today_or_yesterday() {
        let days = [day("2026-11-10"), day("2026-11-12"), day("2026-11-13"), day("2026-11-14"), day("2026-11-14")];
        assert_eq!(streak(&days, day("2026-11-14")), 3);
        assert_eq!(streak(&days, day("2026-11-15")), 3, "nothing yet today keeps yesterday's streak");
        assert_eq!(streak(&days, day("2026-11-16")), 0);
        assert_eq!(streak(&[], day("2026-11-16")), 0);
    }

    #[test]
    fn statistics_add_up_words_time_saved_speed_and_apps() {
        let db = db();
        let now = Utc.with_ymd_and_hms(2026, 11, 16, 15, 0, 0).unwrap();
        // 40 words spoken in 15 s: typing them takes a minute, so 45 s saved, 160 wpm.
        let forty = vec!["word"; 40].join(" ");
        record(&db, &NewDictation { app: "Notes", bundle: "com.apple.Notes", text: &forty, speaking_ms: 15_000, pasted: true }, now).unwrap();
        record(&db, &NewDictation { app: "Slack", bundle: "com.tinyspeck.slackmacgap", text: "on my way", speaking_ms: 1_000, pasted: true }, now).unwrap();
        // A slow dictation saves nothing, and takes nothing back from the others.
        record(&db, &NewDictation { app: "Slack", bundle: "com.tinyspeck.slackmacgap", text: "ok", speaking_ms: 60_000, pasted: false }, now).unwrap();
        let today = now.with_timezone(&Local).date_naive();
        let stats = stats(&db, today).unwrap();
        assert_eq!(stats.words, 44);
        assert_eq!(stats.dictations, 3);
        assert_eq!(stats.minutes_saved, 0.8);
        assert_eq!(stats.words_per_minute, (44.0_f64 / (76.0 / 60.0)).round());
        assert_eq!(stats.streak, 1);
        assert_eq!(stats.apps[0], AppWords { app: "Notes".into(), words: 40 });
        assert_eq!(stats.apps[1], AppWords { app: "Slack".into(), words: 4 });
    }

    #[test]
    fn words_are_kept_a_month_then_only_counts_remain() {
        let db = db();
        let then = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        record(&db, &NewDictation { app: "Notes", bundle: "b", text: "an old thought", speaking_ms: 900, pasted: true }, then).unwrap();
        assert_eq!(last_text(&db).unwrap().as_deref(), Some("an old thought"));
        let now = Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap();
        record(&db, &NewDictation { app: "Notes", bundle: "b", text: "a new one", speaking_ms: 900, pasted: true }, now).unwrap();
        let kept = recent(&db, 10).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "a new one");
        assert_eq!(stats(&db, now.date_naive()).unwrap().words, 6);
        forget(&db, kept[0].id).unwrap();
        assert_eq!(last_text(&db).unwrap(), None);
        clear(&db).unwrap();
        assert_eq!(stats(&db, now.date_naive()).unwrap().dictations, 2);
        erase(&db).unwrap();
        assert_eq!(stats(&db, now.date_naive()).unwrap().dictations, 0);
    }

    #[test]
    fn notes_are_saved_updated_and_listed_newest_first() {
        let db = db();
        let mut note = Note { id: "n1".into(), title: "Standup".into(), created_at: "2026-11-16T10:00:00Z".into(), seconds: 60.0, state: "processing".into(), lines: vec![], summary: String::new(), key_points: vec![], todos: vec![] };
        save_note(&db, &note).unwrap();
        note.state = "ready".into();
        note.todos = vec!["Send the deck".into()];
        save_note(&db, &note).unwrap();
        save_note(&db, &Note { id: "n2".into(), created_at: "2026-11-16T11:00:00Z".into(), ..note.clone() }).unwrap();
        let listed = notes(&db).unwrap();
        assert_eq!(listed.iter().map(|note| note.id.as_str()).collect::<Vec<_>>(), ["n2", "n1"]);
        assert_eq!(listed[1].todos, ["Send the deck"]);
        delete_note(&db, "n2").unwrap();
        clear(&db).unwrap();
        assert!(notes(&db).unwrap().is_empty());
    }

    #[test]
    fn only_words_are_counted() {
        assert_eq!(count_words("Hello, world — it's 3 pm!"), 5);
        assert_eq!(count_words("  "), 0);
    }
}
