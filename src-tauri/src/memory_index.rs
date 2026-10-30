//! Local memory: one searchable index over what Buntini may already read on this Mac
//! (remembered facts, earlier conversations, Notes, Mail, Calendar, Reminders and shared
//! folders), so "what did Ana say about the lease?" finds the note, email or chat it was in.
//!
//! Each record keeps the data categories it came from. Results are ranked by keyword match
//! (SQLite FTS5 with stemming, so "plans" finds "plan") and by meaning (sentence vectors
//! from the bundled model in `embedding`), merged by reciprocal rank. Before a result reaches a brain, its categories are checked against
//! that brain's privacy permissions and its app's current access. Indexing runs on the Mac
//! in the background, only for sources the user allowed, and never uses the network.
//! Background reads never open a closed Mail or Notes, and indexing waits while a chat is
//! running or the Mac is on battery.
//! Revoking a source, forgetting a fact, clearing history and Delete everything remove the
//! matching records, and a refresh already under way can't put them back: it reads the
//! apps first, then writes under the store lock only if nothing was deleted meanwhile.
use crate::{
    privacy::{self, Locality, Share},
    types::Settings,
};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// The most records kept, across every source.
pub const MAX_RECORDS: usize = 20_000;
const MAX_TEXT: usize = 1_500;
/// Reciprocal-rank fusion constant: dampens the advantage of the very top ranks.
const RRF: f64 = 60.0;
/// Keyword ranks count half as much as meaning ranks: on real notes, meaning found the
/// right note far more often, and exact words then break ties between close candidates.
const KEYWORD_WEIGHT: f64 = 0.5;
const CANDIDATES: usize = 50;

const FTS_SCHEMA: &str = "CREATE VIRTUAL TABLE memory_fts USING fts5(title, text, tokenize='porter unicode61 remove_diacritics 2')";
const MODEL_KEY: &str = "memory-embedding-model";

pub fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_records (
           id INTEGER PRIMARY KEY,
           source TEXT NOT NULL,
           source_id TEXT NOT NULL,
           scope TEXT NOT NULL DEFAULT '',
           shares TEXT NOT NULL,
           title TEXT NOT NULL,
           text TEXT NOT NULL,
           observed_at TEXT NOT NULL,
           indexed_at TEXT NOT NULL,
           vector BLOB,
           UNIQUE(source, source_id));",
    )
    .map_err(|e| format!("Cannot initialize the memory index: {e}"))?;
    // The keyword index stems words; an older unstemmed one is rebuilt from the records.
    let fts: Option<String> = db
        .query_row("SELECT sql FROM sqlite_master WHERE name='memory_fts'", [], |row| row.get(0))
        .ok();
    if fts.as_deref() != Some(FTS_SCHEMA) {
        db.execute_batch(&format!(
            "DROP TABLE IF EXISTS memory_fts; {FTS_SCHEMA}; INSERT INTO memory_fts(rowid, title, text) SELECT id, title, text FROM memory_records;"
        ))
        .map_err(|e| format!("Cannot initialize the memory index: {e}"))?;
    }
    // Vectors from another model don't compare with this one's, so they are made again.
    let model: Option<String> = db
        .query_row("SELECT value FROM preferences WHERE key=?1", [MODEL_KEY], |row| row.get(0))
        .ok();
    if model.as_deref() != Some(crate::embedding::MODEL_ID) {
        db.execute_batch("UPDATE memory_records SET vector=NULL WHERE vector IS NOT NULL")
            .map_err(|e| format!("Cannot initialize the memory index: {e}"))?;
        db.execute(
            "INSERT INTO preferences(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [MODEL_KEY, crate::embedding::MODEL_ID],
        )
        .map_err(|e| format!("Cannot initialize the memory index: {e}"))?;
    }
    Ok(())
}

/// One thing to remember, from one source.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub source: &'static str,
    pub source_id: String,
    /// What revoking removes along with it: the shared folder for files.
    pub scope: String,
    /// Every data category the text contains; a brain must be allowed all of them.
    pub shares: Vec<Share>,
    pub title: String,
    pub text: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub excerpt: String,
    pub observed_at: String,
    pub shares: Vec<String>,
    pub score: f64,
}

fn clip(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn shares_text(shares: &[Share]) -> String {
    let mut ids: Vec<&str> = shares.iter().map(|s| s.id()).collect();
    ids.sort();
    ids.dedup();
    ids.join(",")
}

/// Whether text looks like it holds a secret: an API key or token, a private key, or a
/// note about passwords or credentials. Such records are never indexed, so search can't
/// surface them, even though the source app still has them.
pub fn looks_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("-----begin") {
        return true;
    }
    let words = [
        "password", "passwords", "passcode", "passwd", "creds", "credentials", "api key", "apikey",
        "secret key", "access key", "recovery code", "recovery codes", "2fa code", "seed phrase",
        "private key", "pin code", "social security",
    ];
    let padded = format!(" {} ", lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "));
    if words.iter().any(|word| padded.contains(&format!(" {word} "))) {
        return true;
    }
    let token = |prefix: &str, minimum: usize| {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).any(|part| {
            part.starts_with(prefix) && part.len() >= prefix.len() + minimum
        })
    };
    [("sk-", 20), ("sk_live_", 16), ("sk_test_", 16), ("rk_live_", 16), ("ghp_", 30), ("gho_", 30), ("github_pat_", 30), ("glpat-", 20), ("xoxb-", 20), ("xoxp-", 20), ("AKIA", 16), ("ASIA", 16), ("AIza", 30), ("hf_", 30)]
        .iter()
        .any(|(prefix, minimum)| token(prefix, *minimum))
}

/// Replaces everything indexed from `source` with `records` (a complete read of that
/// source's window). Unchanged records keep their vectors; changed ones are re-embedded.
/// Records that look like secrets are left out, and dropped if indexed before.
pub fn replace_source(db: &Connection, source: &str, records: &[Record]) -> Result<usize, String> {
    write_source(db, source, records, true)
}

/// Like `replace_source`, but for a read that covered only part of the source: records
/// not in `records` stay as they were.
fn update_source(db: &Connection, source: &str, records: &[Record]) -> Result<usize, String> {
    write_source(db, source, records, false)
}

fn write_source(db: &Connection, source: &str, records: &[Record], complete: bool) -> Result<usize, String> {
    let (records, secret): (Vec<&Record>, Vec<&Record>) = records
        .iter()
        .partition(|record| !looks_secret(&record.title) && !looks_secret(&record.text));
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let existing: HashMap<String, (i64, String, String)> = {
        let mut statement = tx
            .prepare("SELECT source_id, id, title, text FROM memory_records WHERE source=?1")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([source], |row| Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?, row.get(3)?))))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let kept: HashSet<&str> = records.iter().map(|r| r.source_id.as_str()).collect();
    let dropped: HashSet<&str> = secret.iter().map(|r| r.source_id.as_str()).collect();
    for (source_id, (id, _, _)) in &existing {
        let gone = if complete { !kept.contains(source_id.as_str()) } else { dropped.contains(source_id.as_str()) };
        if gone {
            tx.execute("DELETE FROM memory_records WHERE id=?1", [id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM memory_fts WHERE rowid=?1", [id]).map_err(|e| e.to_string())?;
        }
    }
    let now = chrono::Local::now().to_rfc3339();
    let mut changed = 0;
    for record in &records {
        let title = clip(&record.title, 200);
        let text = clip(&record.text, MAX_TEXT);
        match existing.get(&record.source_id) {
            Some((id, old_title, old_text)) if *old_title == title && *old_text == text => {
                tx.execute(
                    "UPDATE memory_records SET shares=?2, scope=?3, observed_at=?4 WHERE id=?1",
                    params![id, shares_text(&record.shares), record.scope, record.observed_at],
                )
                .map_err(|e| e.to_string())?;
            }
            Some((id, _, _)) => {
                tx.execute(
                    "UPDATE memory_records SET shares=?2, scope=?3, title=?4, text=?5, observed_at=?6, indexed_at=?7, vector=NULL WHERE id=?1",
                    params![id, shares_text(&record.shares), record.scope, title, text, record.observed_at, now],
                )
                .map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM memory_fts WHERE rowid=?1", [id]).map_err(|e| e.to_string())?;
                tx.execute("INSERT INTO memory_fts(rowid, title, text) VALUES (?1, ?2, ?3)", params![id, title, text])
                    .map_err(|e| e.to_string())?;
                changed += 1;
            }
            None => {
                tx.execute(
                    "INSERT INTO memory_records(source, source_id, scope, shares, title, text, observed_at, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![source, record.source_id, record.scope, shares_text(&record.shares), title, text, record.observed_at, now],
                )
                .map_err(|e| e.to_string())?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_fts(rowid, title, text) VALUES (?1, ?2, ?3)", params![id, title, text])
                    .map_err(|e| e.to_string())?;
                changed += 1;
            }
        }
    }
    let total: i64 = tx
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if total as usize > MAX_RECORDS {
        tx.rollback().map_err(|e| e.to_string())?;
        return Err(format!("The memory index is full ({MAX_RECORDS} records); {source} was left as it was."));
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

/// Removes everything from one source, or one scope of it (a shared folder), and says
/// how many records went.
pub fn purge(db: &Connection, source: &str, scope: Option<&str>) -> Result<usize, String> {
    // An empty scope means the whole source.
    let filter = "source=?1 AND (?2 = '' OR scope=?2)";
    let scope = scope.unwrap_or("");
    db.execute(
        &format!("DELETE FROM memory_fts WHERE rowid IN (SELECT id FROM memory_records WHERE {filter})"),
        params![source, scope],
    )
    .map_err(|e| e.to_string())?;
    db.execute(&format!("DELETE FROM memory_records WHERE {filter}"), params![source, scope])
        .map_err(|e| e.to_string())
}

/// Removes one record, such as a forgotten fact.
pub fn purge_record(db: &Connection, source: &str, source_id: &str) -> Result<(), String> {
    db.execute(
        "DELETE FROM memory_fts WHERE rowid IN (SELECT id FROM memory_records WHERE source=?1 AND source_id=?2)",
        params![source, source_id],
    )
    .map_err(|e| e.to_string())?;
    db.execute("DELETE FROM memory_records WHERE source=?1 AND source_id=?2", params![source, source_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The indexed sources that come from an app, each named like the app.
const APP_SOURCES: [&str; 4] = ["notes", "mail", "calendar", "reminders"];

/// Removes sources the user no longer allows (an app switched off in Settings → Apps),
/// and says whether any record went. Turning the app back on reads it again in full.
pub fn purge_disallowed(db: &Connection, settings: &Settings) -> Result<bool, String> {
    let mut removed = 0;
    for source in APP_SOURCES {
        if !crate::apps::allowed(settings, source) {
            removed += purge(db, source, None)?;
            db.execute("DELETE FROM preferences WHERE key=?1", [refreshed_key(source)])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(removed > 0)
}

fn refreshed_key(source: &str) -> String {
    format!("memory-refreshed-{source}")
}

/// The app sources read in full within `within`, which a refresh need not read again
/// (each launch would otherwise re-read 90 days of inbox).
fn recently_refreshed(db: &Connection, within: chrono::Duration) -> HashSet<&'static str> {
    let now = chrono::Local::now();
    APP_SOURCES
        .into_iter()
        .filter(|source| {
            db.query_row("SELECT value FROM preferences WHERE key=?1", [refreshed_key(source)], |row| row.get::<_, String>(0))
                .ok()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
                .is_some_and(|at| at <= now && now.signed_duration_since(at) < within)
        })
        .collect()
}

/// Records still waiting for a vector, oldest first.
pub fn unembedded(db: &Connection, limit: usize) -> Result<Vec<(i64, String)>, String> {
    let mut statement = db
        .prepare("SELECT id, title, text FROM memory_records WHERE vector IS NULL ORDER BY id LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, format!("{}. {}", row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Stores a vector, or an empty one for text with nothing to embed so it isn't retried.
pub fn set_vector(db: &Connection, id: i64, vector: Option<&[i8]>) -> Result<(), String> {
    let bytes: Vec<u8> = vector.unwrap_or(&[]).iter().map(|v| *v as u8).collect();
    db.execute("UPDATE memory_records SET vector=?2 WHERE id=?1", params![id, bytes])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// How many records each source holds, for status and support summaries.
pub fn counts(db: &Connection) -> Result<Vec<(String, usize)>, String> {
    let mut statement = db
        .prepare("SELECT source, COUNT(*) FROM memory_records GROUP BY source ORDER BY source")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as usize)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Words too common to say what a question is about; matching them only adds noise.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "am", "an", "and", "any", "are", "as", "at", "be", "before", "but",
    "by", "can", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "him", "his",
    "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "need", "next", "of", "on", "or",
    "our", "over", "said", "say", "she", "should", "so", "that", "the", "their", "them", "there",
    "they", "this", "to", "up", "us", "was", "we", "were", "what", "when", "where", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

/// The query as an FTS5 expression: each meaningful word quoted, any may match.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2 && !STOPWORDS.contains(&word.to_lowercase().as_str()))
        .take(16)
        .map(|word| format!("\"{}\"", word.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn dot(a: &[i8], b: &[i8]) -> i32 {
    a.iter().zip(b).map(|(x, y)| i32::from(*x) * i32::from(*y)).sum()
}

/// The best records for a query: keyword ranks and (with `vector`) meaning ranks, fused.
/// Brains search through `search_visible`; this unfiltered form is for tests.
#[cfg(test)]
pub fn search(db: &Connection, query: &str, vector: Option<&[i8]>, limit: usize) -> Result<Vec<Hit>, String> {
    let (best, _) = fused(db, query, vector, CANDIDATES)?;
    best.into_iter().take(limit).map(|(id, score)| hit(db, id, score)).collect()
}

/// The best `limit` records for a query that `keep` accepts (what a brain may see).
/// Records are filtered before the limit, and the pool of candidates widens until
/// `limit` visible ones are found or the rankings run out, so hidden ones never take the
/// places of visible ones and hide a match.
pub fn search_visible(
    db: &Connection,
    query: &str,
    vector: Option<&[i8]>,
    limit: usize,
    keep: impl Fn(&Hit) -> bool,
) -> Result<Vec<Hit>, String> {
    let mut pool = CANDIDATES * 4;
    loop {
        let (best, truncated) = fused(db, query, vector, pool)?;
        let mut hits = Vec::new();
        for (id, score) in best {
            let found = hit(db, id, score)?;
            if keep(&found) {
                hits.push(found);
                if hits.len() == limit {
                    return Ok(hits);
                }
            }
        }
        if !truncated {
            return Ok(hits);
        }
        pool *= 4;
    }
}

/// Record ids with their fused score, best first, from the top `pool` of each ranking,
/// and whether either ranking had more than `pool` records.
fn fused(db: &Connection, query: &str, vector: Option<&[i8]>, pool: usize) -> Result<(Vec<(i64, f64)>, bool), String> {
    let mut truncated = false;
    let mut score: HashMap<i64, f64> = HashMap::new();
    if let Some(expression) = fts_query(query) {
        let mut statement = db
            .prepare("SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?1 ORDER BY bm25(memory_fts, 2.0, 1.0) LIMIT ?2")
            .map_err(|e| e.to_string())?;
        let ids = statement
            .query_map(params![expression, pool as i64 + 1], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        truncated |= ids.len() > pool;
        for (rank, id) in ids.into_iter().take(pool).enumerate() {
            *score.entry(id).or_default() += KEYWORD_WEIGHT / (RRF + rank as f64 + 1.0);
        }
    }
    if let Some(vector) = vector.filter(|v| !v.is_empty()) {
        let mut statement = db
            .prepare("SELECT id, vector FROM memory_records WHERE length(vector) = ?1")
            .map_err(|e| e.to_string())?;
        let mut ranked: Vec<(i32, i64)> = statement
            .query_map([vector.len() as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|(id, bytes)| {
                let other: Vec<i8> = bytes.into_iter().map(|b| b as i8).collect();
                (dot(vector, &other), id)
            })
            .collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));
        truncated |= ranked.len() > pool;
        for (rank, (_, id)) in ranked.into_iter().take(pool).enumerate() {
            *score.entry(id).or_default() += 1.0 / (RRF + rank as f64 + 1.0);
        }
    }
    let mut best: Vec<(i64, f64)> = score.into_iter().collect();
    best.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok((best, truncated))
}

fn hit(db: &Connection, id: i64, score: f64) -> Result<Hit, String> {
    db.query_row(
        "SELECT source, source_id, title, text, observed_at, shares FROM memory_records WHERE id=?1",
        [id],
        |row| {
            Ok(Hit {
                source: row.get(0)?,
                source_id: row.get(1)?,
                title: row.get(2)?,
                excerpt: clip(&row.get::<_, String>(3)?, 400),
                observed_at: row.get(4)?,
                shares: row
                    .get::<_, String>(5)?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                score,
            })
        },
    )
    .map_err(|e| e.to_string())
}

/// The data categories a hit needs, as `Share`s.
pub fn hit_shares(hit: &Hit) -> Vec<Share> {
    hit.shares
        .iter()
        .filter_map(|id| Share::ALL.iter().copied().find(|share| share.id() == id))
        .collect()
}

/// Whether a brain may see a hit now: every category it contains is allowed for that
/// brain, and the app it came from is still allowed.
pub fn visible(hit: &Hit, locality: Locality, settings: &Settings, roots: &[String]) -> bool {
    let app_allowed = match hit.source.as_str() {
        "mail" | "notes" | "calendar" | "reminders" => crate::apps::allowed(settings, &hit.source),
        "files" => roots.iter().any(|root| {
            hit.source_id.starts_with(&format!("{}/", root.trim_end_matches('/')))
        }),
        _ => true,
    };
    app_allowed
        && hit_shares(hit).len() == hit.shares.len()
        && hit_shares(hit).into_iter().all(|share| privacy::allows(locality, settings, share))
}

/// What one refresh did, for the activity log.
#[derive(Debug, Default)]
pub struct Refresh {
    pub changed: usize,
    pub embedded: usize,
    pub failed: Vec<String>,
}

/// How far back each source is indexed.
const NOTES_DAYS: u32 = 365;
const MAIL_DAYS: u32 = 90;
const MAIL_MESSAGES: usize = 200;
const CALENDAR_PAST: i64 = 30;
const CALENDAR_AHEAD: i64 = 60;
pub(crate) const HISTORY_TURNS: usize = 1_000;
/// Records embedded per refresh, so one run stays a short background job.
const EMBED_PER_RUN: usize = 2_000;
/// Records embedded at a time: a short hold on the model, so a question asked meanwhile
/// waits little, and a quick check for deletions between batches.
const EMBED_BATCH: usize = 8;
/// An app source read in full this recently isn't read again (on launch, say).
const FRESH_HOURS: i64 = 3;
/// How often the index is brought up to date.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// Whether a finished turn is worth remembering: the brief is regenerated, not remembered.
pub(crate) fn remembered_turn(task: &crate::types::TaskRecord) -> bool {
    task.state == "completed"
        && task.backend != "brief"
        && !task.prompt.trim().is_empty()
        && !task.answer.trim().is_empty()
}

fn history_records(tasks: &[crate::types::TaskRecord]) -> Vec<Record> {
    tasks
        .iter()
        .filter(|task| remembered_turn(task))
        .take(HISTORY_TURNS)
        .map(|task| {
            let mut shares = vec![Share::History];
            shares.extend(task.touched.iter().filter_map(|id| Share::ALL.iter().copied().find(|s| s.id() == id)));
            Record {
                source: "history",
                source_id: task.id.clone(),
                scope: String::new(),
                shares,
                title: clip(&task.prompt, 120),
                text: format!("You asked: {}\nBuntini answered: {}", clip(&task.prompt, 400), clip(&task.answer, 1_000)),
                observed_at: task.created_at.clone(),
            }
        })
        .collect()
}

fn recent_records(source: &'static str, share: Share, rows: Vec<crate::apps::Recent>) -> Vec<Record> {
    rows.into_iter()
        .filter(|(_, id, _, _)| !id.is_empty())
        .map(|(at, id, title, text)| Record {
            source,
            source_id: id,
            scope: String::new(),
            shares: vec![share],
            title,
            text,
            observed_at: at,
        })
        .collect()
}

/// The calendar's window, read with one helper call rather than one per day.
async fn calendar_records(helper: &std::path::Path) -> Result<(Vec<Record>, Option<String>), String> {
    let today = chrono::Local::now().date_naive();
    let day = |offset: i64| (today + chrono::Duration::days(offset)).format("%Y-%m-%d").to_string();
    let found = crate::apps::events_between(helper, &day(-CALENDAR_PAST), &day(CALENDAR_AHEAD)).await?;
    Ok(event_records(found))
}

/// Index records for events read from Calendar, once each; a cut-off read says so.
fn event_records(found: crate::apps::Agenda<crate::apps::Event>) -> (Vec<Record>, Option<String>) {
    let partial = (!found.complete())
        .then(|| format!("only the first {} of {} events were read", found.items.len(), found.total));
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for event in found.items {
        let id = format!("{}@{}", event.id.clone().unwrap_or_else(|| event.title.clone()), event.start);
        if !seen.insert(id.clone()) {
            continue;
        }
        let when = if event.all_day {
            format!("{}, all day", event.start.get(..10).unwrap_or(&event.start))
        } else {
            format!("{} to {}", event.start, event.end)
        };
        let place = if event.location.is_empty() { String::new() } else { format!(" at {}", event.location) };
        records.push(Record {
            source: "calendar",
            source_id: id,
            scope: String::new(),
            shares: vec![Share::Calendar],
            title: event.title.clone(),
            text: format!("Calendar event “{}”, {when}{place}", event.title),
            observed_at: event.start.clone(),
        });
    }
    (records, partial)
}

/// What reading one app gave.
#[derive(Debug)]
pub(crate) enum SourceRead {
    /// A complete read of the source's window.
    Records(Vec<Record>),
    /// Part of the source, with what stopped the rest; records not read are kept.
    Partial(Vec<Record>, String),
    /// Not read: the app was closed, isn't allowed, was read recently, or the refresh
    /// was cancelled. What was indexed before stays.
    Skipped,
    /// The read failed; what was indexed before stays.
    Failed(String),
}

impl SourceRead {
    fn from(read: Result<(Vec<Record>, Option<String>), String>) -> Self {
        match read {
            Ok((records, None)) => Self::Records(records),
            Ok((records, Some(error))) => Self::Partial(records, error),
            // Left closed rather than opened for a background job.
            Err(error) if crate::apps::is_not_running(&error) => Self::Skipped,
            Err(error) => Self::Failed(error),
        }
    }
}

/// Waits while the user is busy with a chat or an action, so indexing steps aside.
async fn step_aside(busy: &(dyn Fn() -> bool + Sync), cancel: &AtomicBool) {
    while busy() && !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Reads the apps the user allows (Notes, Mail, Calendar, Reminders), without opening a
/// closed Mail or Notes. This is the slow part of a refresh and runs without the store.
async fn read_app_sources(
    settings: &Settings,
    models: &crate::model::ModelHub,
    cancel: &AtomicBool,
    fresh: &HashSet<&'static str>,
    busy: &(dyn Fn() -> bool + Sync),
) -> Vec<(&'static str, SourceRead)> {
    use crate::apps::Launch;
    let helper = models.apple_helper();
    let mut reads = Vec::new();
    for source in APP_SOURCES {
        step_aside(busy, cancel).await;
        if cancel.load(Ordering::Relaxed) || !crate::apps::allowed(settings, source) || fresh.contains(source) {
            reads.push((source, SourceRead::Skipped));
            continue;
        }
        let read = match source {
            "notes" => crate::apps::recent_notes(NOTES_DAYS, Launch::IfRunning)
                .await
                .map(|rows| (recent_records("notes", Share::Notes, rows), None)),
            "mail" => crate::apps::recent_mail(MAIL_DAYS, MAIL_MESSAGES, Launch::IfRunning)
                .await
                .map(|(rows, failed)| (recent_records("mail", Share::Mail, rows), failed)),
            "calendar" => calendar_records(helper).await,
            _ => crate::apps::reminders(helper).await.map(|read| {
                let records = read
                    .items
                    .into_iter()
                    .map(|reminder| Record {
                        source: "reminders",
                        source_id: reminder.id.clone().unwrap_or_else(|| reminder.title.clone()),
                        scope: String::new(),
                        shares: vec![Share::Calendar],
                        title: reminder.title.clone(),
                        text: if reminder.due.is_empty() {
                            format!("Open reminder “{}”", reminder.title)
                        } else {
                            format!("Open reminder “{}”, due {}", reminder.title, reminder.due)
                        },
                        observed_at: reminder.due,
                    })
                    .collect();
                (records, None)
            }),
        };
        reads.push((source, SourceRead::from(read)));
    }
    reads
}

/// What Buntini itself holds, read from the store as it is now: remembered facts, past
/// conversations and shared folders' text. A folder's text is what the file index read,
/// so PDF, Word, RTF and Pages documents count with the text the Apple helper extracted
/// (`local::read_content`); it is untrusted file content like any other.
fn store_sources(store: &crate::store::Store) -> Result<Vec<(&'static str, Vec<Record>)>, String> {
    let roots = store.roots()?;
    let files: Vec<Record> = {
        let mut statement = store
            .db
            .prepare("SELECT path, root, name, substr(content, 1, 1500) FROM file_index LIMIT 5000")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok)
            .filter(|(_, root, _, _)| roots.contains(root))
            .map(|(path, root, name, content)| Record {
                source: "files",
                source_id: path,
                scope: root,
                shares: vec![Share::Files],
                title: name,
                text: content,
                observed_at: String::new(),
            })
            .collect()
    };
    let memories = store
        .memories()?
        .into_iter()
        .map(|memory| Record {
            source: "memory",
            source_id: memory.id.to_string(),
            scope: String::new(),
            shares: vec![Share::Memory],
            title: "Something you asked me to remember".into(),
            text: memory.fact,
            observed_at: memory.created_at,
        })
        .collect();
    Ok(vec![
        ("memory", memories),
        ("history", history_records(&store.history_for_index(HISTORY_TURNS)?)),
        ("files", files),
    ])
}

/// Writes a refresh, all under the store lock. Buntini's own sources are rebuilt from the
/// store as it is now, so they are never stale. An app read is written only when nothing
/// was deleted or revoked since `epoch_at_start` and the app is still allowed; otherwise,
/// and when it was skipped or failed, what was indexed before stays.
pub(crate) fn apply(
    store: &crate::store::Store,
    epoch_at_start: u64,
    app_sources: Vec<(&'static str, SourceRead)>,
) -> Result<Refresh, String> {
    let settings = store.settings()?;
    let mut result = Refresh::default();
    for (source, records) in store_sources(store)? {
        match replace_source(&store.db, source, &records) {
            Ok(changed) => result.changed += changed,
            Err(error) => result.failed.push(format!("{source}: {error}")),
        }
    }
    // Something was deleted or revoked while the apps were read: their reads may hold it.
    let revoked = store.index_epoch() != epoch_at_start;
    for (source, read) in app_sources {
        if revoked || !crate::apps::allowed(&settings, source) {
            continue;
        }
        let written = match read {
            SourceRead::Skipped => continue,
            SourceRead::Failed(error) => {
                result.failed.push(format!("{source}: {error}"));
                continue;
            }
            SourceRead::Records(records) => replace_source(&store.db, source, &records).and_then(|changed| {
                store.set_preference(&refreshed_key(source), &chrono::Local::now().to_rfc3339())?;
                Ok(changed)
            }),
            SourceRead::Partial(records, error) => {
                result.failed.push(format!("{source} (partly read): {error}"));
                update_source(&store.db, source, &records)
            }
        };
        match written {
            Ok(changed) => result.changed += changed,
            Err(error) => result.failed.push(format!("{source}: {error}")),
        }
    }
    purge_disallowed(&store.db, &settings)?;
    Ok(result)
}

/// Reads every source the user allows and brings the index up to date, then embeds what
/// changed. A source that can't be read keeps what was indexed before. `cancel` (set by
/// Delete everything and Clear history) stops the run; `busy` says the user is in a chat.
pub async fn refresh(
    store: &std::sync::Arc<std::sync::Mutex<crate::store::Store>>,
    models: &crate::model::ModelHub,
    cancel: &AtomicBool,
    busy: &(dyn Fn() -> bool + Sync),
) -> Result<Refresh, String> {
    cancel.store(false, Ordering::SeqCst);
    let (settings, epoch, fresh) = {
        let store = store.lock().unwrap();
        (store.settings()?, store.index_epoch(), recently_refreshed(&store.db, chrono::Duration::hours(FRESH_HOURS)))
    };
    let app_sources = read_app_sources(&settings, models, cancel, &fresh, busy).await;
    if cancel.load(Ordering::SeqCst) {
        return Ok(Refresh::default());
    }
    let mut result = apply(&store.lock().unwrap(), epoch, app_sources)?;
    let mut attempted = 0;
    while attempted < EMBED_PER_RUN {
        step_aside(busy, cancel).await;
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let (epoch, batch) = {
            let store = store.lock().unwrap();
            (store.index_epoch(), unembedded(&store.db, EMBED_BATCH)?)
        };
        if batch.is_empty() {
            break;
        }
        attempted += batch.len();
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let embedded = tauri::async_runtime::spawn_blocking(move || crate::embedding::embed(&texts, false))
            .await
            .map_err(|_| "Memory search stopped unexpectedly.".to_string())
            .and_then(|result| result);
        let vectors = match embedded {
            Ok(vectors) => vectors,
            // Without on-device vectors the index still answers by keyword.
            Err(error) => {
                result.failed.push(format!("vectors: {error}"));
                break;
            }
        };
        let store = store.lock().unwrap();
        // Records deleted while this batch was embedding; their ids may be reused.
        if cancel.load(Ordering::SeqCst) || store.index_epoch() != epoch {
            continue;
        }
        for ((id, _), vector) in batch.iter().zip(&vectors) {
            set_vector(&store.db, *id, vector.as_deref())?;
        }
        result.embedded += batch.len();
    }
    Ok(result)
}

/// Whether this Mac is running on its battery; desktops never are.
fn on_battery() -> bool {
    std::process::Command::new("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .ok()
        .and_then(|output| crate::world::parse_battery(&String::from_utf8_lossy(&output.stdout)))
        .is_some_and(|(_, charging)| !charging)
}

/// `on_battery`, read again at most every 30 seconds, since each read runs pmset.
fn on_battery_lately(last: &std::sync::Mutex<Option<(std::time::Instant, bool)>>) -> bool {
    let mut last = last.lock().unwrap();
    match *last {
        Some((at, battery)) if at.elapsed() < std::time::Duration::from_secs(30) => battery,
        _ => {
            let battery = on_battery();
            *last = Some((std::time::Instant::now(), battery));
            battery
        }
    }
}

/// Keeps the index current: a first pass shortly after launch, then every six hours.
/// Each pass waits while a chat is running or the Mac is on battery, before it starts
/// and before each app read and embedding batch.
pub fn start(app: tauri::AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        let battery = std::sync::Mutex::new(None);
        loop {
            let state = app.state::<crate::controller::AppState>();
            let busy = || !state.active.lock().unwrap().is_empty() || on_battery_lately(&battery);
            while busy() {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
            match refresh(&state.store, &state.models, &state.index_cancel, &busy).await {
                Ok(done) => state.audit(
                    "memory.indexed",
                    &format!("{} changed, {} embedded{}", done.changed, done.embedded, if done.failed.is_empty() { String::new() } else { format!("; unavailable: {}", done.failed.join("; ")) }),
                ),
                Err(error) => state.audit("memory.index_failed", &error),
            }
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        initialize(&db).unwrap();
        db
    }

    fn record(source: &'static str, id: &str, title: &str, text: &str, shares: Vec<Share>) -> Record {
        Record {
            source,
            source_id: id.into(),
            scope: String::new(),
            shares,
            title: title.into(),
            text: text.into(),
            observed_at: "2026-10-29 09:00".into(),
        }
    }

    /// Indexes `notes` with the bundled model and counts, for each (question, title label),
    /// whether the labelled note is in the top three by keyword, by meaning and combined.
    fn recall(notes: &[Record], cases: &[(String, String)]) -> [usize; 3] {
        crate::embedding::set_model_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5"));
        let db = db();
        replace_source(&db, "notes", notes).unwrap();
        loop {
            let batch = unembedded(&db, 32).unwrap();
            if batch.is_empty() { break; }
            let vectors = crate::embedding::embed(&batch.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(), false).unwrap();
            for ((id, _), vector) in batch.iter().zip(&vectors) { set_vector(&db, *id, vector.as_deref()).unwrap(); }
        }
        let mut totals = [0; 3];
        for (query, label) in cases {
            let vector = crate::embedding::embed(std::slice::from_ref(query), true).unwrap().remove(0).unwrap();
            let hit = |hits: &[Hit]| hits.iter().take(3).any(|h| h.title.contains(label.as_str()));
            let keyword = hit(&search(&db, query, None, 3).unwrap());
            let fused = hit(&search(&db, query, Some(&vector), 3).unwrap());
            let mut ranked: Vec<(i32, String)> = db
                .prepare("SELECT title, vector FROM memory_records").unwrap()
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))).unwrap()
                .map(Result::unwrap)
                .map(|(title, bytes)| (bytes.iter().zip(&vector).map(|(b, q)| i32::from(*b as i8) * i32::from(*q)).sum(), title))
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0));
            let meaning = ranked.iter().take(3).any(|(_, title)| title.contains(label.as_str()));
            for (total, found) in totals.iter_mut().zip([keyword, meaning, fused]) { *total += usize::from(found); }
            println!("{query:<45} {label:<18} keyword {keyword:<5} meaning {meaning:<5} combined {fused}");
        }
        println!("found in top 3 of {}: keyword {}, meaning {}, combined {} ({} notes)", cases.len(), totals[0], totals[1], totals[2], notes.len());
        totals
    }

    /// The ten reworded questions, for a copy of a real index and for the fixture.
    fn recall_cases() -> Vec<(String, String)> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../evals/memory-recall.json")).unwrap();
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| (case["query"].as_str().unwrap().to_string(), case["label"].as_str().unwrap().to_string()))
            .collect()
    }

    /// Recall on the synthetic QA notes in a copy of a real index, with questions that
    /// avoid each note's own words. BUNTINI_EVAL_DB=/path/to/copy cargo test -- --ignored
    #[test]
    #[ignore]
    fn recall_on_a_copy_of_real_notes() {
        let path = std::env::var("BUNTINI_EVAL_DB").expect("set BUNTINI_EVAL_DB to a copy of buntini.sqlite3");
        let copy = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let notes: Vec<Record> = copy
            .prepare("SELECT source_id, title, text FROM memory_records WHERE source='notes' AND title LIKE 'BuntiniQA-%'")
            .unwrap()
            .query_map([], |row| Ok(Record { source: "notes", source_id: row.get(0)?, scope: String::new(), shares: vec![Share::Notes], title: row.get(1)?, text: row.get(2)?, observed_at: String::new() }))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        recall(&notes, &recall_cases());
    }

    /// Of the ten fixture questions, how many find their note in the top three. Measured
    /// with bge-small-en-v1.5: keyword 2, meaning 9, combined 9 (the handoff note is missed).
    const MEANING_FLOOR: usize = 9;
    const FUSED_FLOOR: usize = 9;

    /// The same ten questions over the committed synthetic notes in
    /// evals/memory-recall.json (8 targets among 40), with the bundled model. The floors
    /// are what this model and ranking measured; a change that lowers them is a regression.
    #[test]
    fn fused_recall_on_the_synthetic_fixture_meets_the_documented_floor() {
        if !std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5/model.safetensors").is_file() {
            eprintln!("skipped: run scripts/fetch-models.sh to test the bundled model");
            return;
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../evals/memory-recall.json")).unwrap();
        let notes: Vec<Record> = fixture["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| Record {
                source: "notes",
                source_id: note["id"].as_str().unwrap().into(),
                scope: String::new(),
                shares: vec![Share::Notes],
                title: note["title"].as_str().unwrap().into(),
                text: note["text"].as_str().unwrap().into(),
                observed_at: String::new(),
            })
            .collect();
        assert_eq!(notes.len(), 40);
        assert!(notes.iter().all(|note| !looks_secret(&note.text)), "every fixture note is indexed");
        let [keyword, meaning, fused] = recall(&notes, &recall_cases());
        assert!(meaning >= MEANING_FLOOR, "meaning found {meaning} of 10");
        assert!(fused >= FUSED_FLOOR, "combined found {fused} of 10");
        assert!(keyword <= fused, "combining never loses to keywords alone here");
    }

    fn store_allowing(apps: &[&str]) -> crate::store::Store {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        store
            .save_settings(&Settings { apps: apps.iter().map(|app| app.to_string()).collect(), ..Settings::default() })
            .unwrap();
        store
    }

    fn turn(id: &str, prompt: &str, answer: &str) -> crate::types::TaskRecord {
        serde_json::from_value(serde_json::json!({
            "id":id,"prompt":prompt,"answer":answer,"state":"completed","createdAt":"2026-10-29T09:00:00Z",
            "backend":"apple","touched":[],"sources":[],"files":[],"research":[],"prices":[],"cards":[]
        }))
        .unwrap()
    }

    /// What the slow phase of a refresh read from Notes before the user acted.
    fn notes_read() -> Vec<(&'static str, SourceRead)> {
        vec![("notes", SourceRead::Records(vec![record("notes", "n1", "Dinner", "Sam is vegetarian, so no fish", vec![Share::Notes])]))]
    }

    fn count(store: &crate::store::Store, source: &str) -> usize {
        counts(&store.db).unwrap().into_iter().find(|(name, _)| name == source).map_or(0, |(_, n)| n)
    }

    #[test]
    fn erase_during_refresh_is_not_undone() {
        let store = store_allowing(&["notes"]);
        store.remember("I'm vegetarian").unwrap();
        store.save_task(&turn("t1", "What do I eat?", "You're vegetarian.")).unwrap();
        apply(&store, store.index_epoch(), vec![]).unwrap();
        assert_eq!(count(&store, "memory") + count(&store, "history"), 2);
        let epoch = store.index_epoch();
        let read = notes_read();
        store.erase_everything().unwrap();
        apply(&store, epoch, read).unwrap();
        assert!(counts(&store.db).unwrap().is_empty());
        assert!(search(&store.db, "vegetarian", None, 5).unwrap().is_empty());
    }

    #[test]
    fn forget_during_refresh_is_not_undone() {
        let store = store_allowing(&["notes"]);
        store.remember("I'm vegetarian").unwrap();
        apply(&store, store.index_epoch(), vec![]).unwrap();
        let epoch = store.index_epoch();
        let read = notes_read();
        store.forget(store.memories().unwrap()[0].id).unwrap();
        apply(&store, epoch, read).unwrap();
        assert!(search(&store.db, "vegetarian", None, 5).unwrap().is_empty(), "neither the fact nor a read from before it went");
        // The next run, which reads after the change, indexes Notes as usual.
        apply(&store, store.index_epoch(), notes_read()).unwrap();
        assert_eq!(count(&store, "notes"), 1);
        assert_eq!(count(&store, "memory"), 0);
    }

    #[test]
    fn clear_history_during_refresh_is_not_undone() {
        let store = store_allowing(&[]);
        store.save_task(&turn("t1", "What do I eat?", "You're vegetarian.")).unwrap();
        apply(&store, store.index_epoch(), vec![]).unwrap();
        assert_eq!(count(&store, "history"), 1);
        let epoch = store.index_epoch();
        store.clear_history().unwrap();
        apply(&store, epoch, vec![]).unwrap();
        assert_eq!(count(&store, "history"), 0);
        assert!(search(&store.db, "vegetarian", None, 5).unwrap().is_empty());
        // Deleting one turn takes it out as well.
        store.save_task(&turn("t2", "Lease?", "Due Friday.")).unwrap();
        apply(&store, store.index_epoch(), vec![]).unwrap();
        store.delete_task("t2").unwrap();
        assert_eq!(count(&store, "history"), 0);
    }

    #[tokio::test]
    async fn a_refresh_stops_when_cancelled_and_otherwise_indexes_buntinis_own_data() {
        let store = std::sync::Arc::new(std::sync::Mutex::new(store_allowing(&[])));
        store.lock().unwrap().remember("I'm vegetarian").unwrap();
        let models = crate::model::ModelHub::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        let cancel = AtomicBool::new(false);
        // Delete everything arrives while the refresh waits for a chat to finish.
        let erased = || {
            cancel.store(true, Ordering::SeqCst);
            true
        };
        let done = refresh(&store, &models, &cancel, &erased).await.unwrap();
        assert_eq!((done.changed, done.embedded), (0, 0));
        assert!(counts(&store.lock().unwrap().db).unwrap().is_empty());
        // The next run clears the flag and goes ahead.
        let done = refresh(&store, &models, &cancel, &|| false).await.unwrap();
        assert!(!cancel.load(Ordering::SeqCst));
        assert_eq!(done.changed, 1);
        assert_eq!(count(&store.lock().unwrap(), "memory"), 1);
    }

    #[test]
    fn an_app_turned_off_before_apply_is_not_written() {
        let store = store_allowing(&["mail"]);
        let epoch = store.index_epoch();
        let read = vec![("mail", SourceRead::Records(vec![record("mail", "m1", "Lease", "Ana on the lease", vec![Share::Mail])]))];
        // Mail had no rows yet, so nothing was purged; the check at write time still holds.
        store.save_settings(&Settings::default()).unwrap();
        apply(&store, epoch, read).unwrap();
        assert_eq!(count(&store, "mail"), 0);
    }

    #[test]
    fn a_skipped_source_keeps_its_rows() {
        let store = store_allowing(&["mail"]);
        let mail = |id: &str, text: &str| record("mail", id, "Lease", text, vec![Share::Mail]);
        apply(&store, store.index_epoch(), vec![("mail", SourceRead::Records(vec![mail("m1", "Ana on the lease")]))]).unwrap();
        assert_eq!(count(&store, "mail"), 1);
        // Mail closed (left closed), or the read failed: what was indexed stays.
        apply(&store, store.index_epoch(), vec![("mail", SourceRead::Skipped)]).unwrap();
        let failed = apply(&store, store.index_epoch(), vec![("mail", SourceRead::Failed("Mail timed out.".into()))]).unwrap();
        assert_eq!(count(&store, "mail"), 1);
        assert_eq!(failed.failed, ["mail: Mail timed out."]);
        // A read that covered part of the inbox adds to it without dropping the rest.
        let partial = apply(&store, store.index_epoch(), vec![("mail", SourceRead::Partial(vec![mail("m2", "Sam on the deposit")], "Mail timed out.".into()))]).unwrap();
        assert_eq!(count(&store, "mail"), 2);
        assert!(partial.failed[0].contains("partly read"));
        apply(&store, store.index_epoch(), vec![("mail", SourceRead::Records(vec![mail("m2", "Sam on the deposit")]))]).unwrap();
        assert_eq!(count(&store, "mail"), 1, "a complete read replaces the window");
    }

    #[test]
    fn a_ranged_calendar_read_indexes_each_event_once_and_says_when_it_was_cut_off() {
        let event = |id: &str, start: &str, all_day: bool| crate::apps::Event {
            id: Some(id.into()),
            start: start.into(),
            end: start.into(),
            all_day,
            title: "Dentist".into(),
            ..Default::default()
        };
        let found = crate::apps::Agenda {
            items: vec![event("e1", "2026-11-02 09:00", false), event("e1", "2026-11-02 09:00", false), event("e2", "2026-11-03 00:00", true)],
            total: 3,
        };
        let (records, partial) = event_records(found);
        assert_eq!(records.len(), 2, "the same occurrence is indexed once");
        assert_eq!(records[1].text, "Calendar event “Dentist”, 2026-11-03, all day");
        assert!(partial.is_none());
        let (_, partial) = event_records(crate::apps::Agenda { items: vec![event("e1", "2026-11-02 09:00", false)], total: 2500 });
        assert_eq!(partial.as_deref(), Some("only the first 1 of 2500 events were read"));
    }

    #[test]
    fn a_source_read_in_full_is_not_read_again_soon() {
        let store = store_allowing(&["mail", "notes"]);
        let recent = |store: &crate::store::Store| recently_refreshed(&store.db, chrono::Duration::hours(FRESH_HOURS));
        apply(&store, store.index_epoch(), vec![("mail", SourceRead::Partial(vec![], "Mail timed out.".into()))]).unwrap();
        assert!(recent(&store).is_empty(), "a partial read is tried again");
        apply(&store, store.index_epoch(), vec![("mail", SourceRead::Records(vec![])), ("notes", SourceRead::Skipped)]).unwrap();
        assert_eq!(recent(&store), HashSet::from(["mail"]));
        assert!(recently_refreshed(&store.db, chrono::Duration::zero()).is_empty());
        // Turning Mail off and on again reads it in full next time.
        store.save_settings(&Settings { apps: vec!["notes".into()], ..Settings::default() }).unwrap();
        assert!(recent(&store).is_empty());
    }

    #[test]
    fn hidden_hits_do_not_crowd_out_visible_ones() {
        let mut db = db();
        // More hidden matches than the first pool of candidates holds, all ranked higher.
        let mail: Vec<Record> = (0..CANDIDATES * 4 + 50)
            .map(|n| record("mail", &format!("m{n}"), "Lease", "Lease renewal: the lease, the lease terms and the lease deposit", vec![Share::Mail]))
            .collect();
        replace_source(&mut db, "mail", &mail).unwrap();
        replace_source(&mut db, "notes", &[record("notes", "n1", "Flat", "Ana thinks the lease is fine", vec![Share::Notes])]).unwrap();
        let settings = Settings {
            apps: vec!["mail".into(), "notes".into()],
            external_shares: vec!["notes".into()],
            ..Settings::default()
        };
        let hits = search_visible(&db, "lease", None, 8, |hit| visible(hit, Locality::External, &settings, &[])).unwrap();
        assert_eq!(hits.iter().map(|h| h.source_id.as_str()).collect::<Vec<_>>(), ["n1"]);
        let on_device = search_visible(&db, "lease", None, 8, |hit| visible(hit, Locality::OnDevice, &settings, &[])).unwrap();
        assert_eq!(on_device.len(), 8, "an on-device brain sees everything local");
    }

    #[test]
    fn secrets_are_never_indexed_and_leave_if_they_were() {
        for text in [
            "OpenAI sk-proj-abcdefghijklmnopqrstuvwxyz0123",
            "Bank creds: user ana",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "My Wi-Fi password is hunter2",
            "AWS AKIAABCDEFGHIJKLMNOP",
            "Recovery codes for GitHub",
        ] {
            assert!(looks_secret(text), "{text}");
        }
        for text in ["Pick up the passport on Friday", "Ask Ana about the desk-pin", "sketch of the sk-8 park", "Lease renewal due Friday"] {
            assert!(!looks_secret(text), "{text}");
        }
        let mut db = db();
        replace_source(&mut db, "notes", &[record("notes", "n1", "Bank", "creds for the bank", vec![Share::Notes])]).unwrap();
        assert!(counts(&db).unwrap().is_empty());
        replace_source(&mut db, "notes", &[record("notes", "n1", "Bank", "visit the bank", vec![Share::Notes])]).unwrap();
        replace_source(&mut db, "notes", &[record("notes", "n1", "Bank", "visit the bank, creds below", vec![Share::Notes])]).unwrap();
        assert!(search(&db, "bank", None, 3).unwrap().is_empty(), "a note that gains a secret leaves the index");
    }

    #[test]
    fn stemming_matches_word_forms_and_a_new_model_redoes_vectors() {
        let mut db = db();
        replace_source(&mut db, "notes", &[record("notes", "n1", "Trip", "Planning the trips to Lisbon", vec![Share::Notes])]).unwrap();
        assert_eq!(search(&db, "plan trip", None, 3).unwrap().len(), 1, "plan finds planning, trip finds trips");
        let (id, _) = unembedded(&db, 1).unwrap()[0].clone();
        set_vector(&db, id, Some(&[1, 2, 3])).unwrap();
        initialize(&db).unwrap();
        assert!(unembedded(&db, 1).unwrap().is_empty(), "the same model keeps its vectors");
        db.execute("UPDATE preferences SET value='older-model' WHERE key=?1", [MODEL_KEY]).unwrap();
        initialize(&db).unwrap();
        assert_eq!(unembedded(&db, 1).unwrap().len(), 1, "another model's vectors are made again");
        // An index made before stemming is rebuilt with every record.
        db.execute_batch("DROP TABLE memory_fts; CREATE VIRTUAL TABLE memory_fts USING fts5(title, text);").unwrap();
        initialize(&db).unwrap();
        assert_eq!(search(&db, "plan", None, 3).unwrap().len(), 1);
    }

    #[test]
    fn conversations_carry_every_category_they_touched() {
        let mut task: crate::types::TaskRecord = serde_json::from_value(serde_json::json!({
            "id":"t1","prompt":"What did Ana email?","answer":"She asked about the lease.",
            "state":"completed","createdAt":"2026-10-29T09:00:00Z","backend":"apple","touched":["mail"],
            "sources":[],"files":[],"research":[],"prices":[],"cards":[]
        })).unwrap();
        let records = history_records(std::slice::from_ref(&task));
        assert_eq!(records[0].shares, [Share::History, Share::Mail]);
        assert!(records[0].text.contains("She asked about the lease."));
        task.backend = "brief".into();
        assert!(history_records(&[task]).is_empty(), "the brief is regenerated, not remembered");
    }

    #[test]
    fn replacing_a_source_keeps_unchanged_vectors_and_drops_missing_records() {
        let mut db = db();
        let lease = record("notes", "n1", "Lease", "Ana says the renewal is due Friday", vec![Share::Notes]);
        let bread = record("notes", "n2", "Bread", "Banana bread recipe", vec![Share::Notes]);
        assert_eq!(replace_source(&mut db, "notes", &[lease.clone(), bread.clone()]).unwrap(), 2);
        let pending = unembedded(&db, 10).unwrap();
        assert_eq!(pending.len(), 2);
        for (id, _) in &pending {
            set_vector(&db, *id, Some(&[1, 2, 3])).unwrap();
        }
        let mut changed = bread.clone();
        changed.text = "Banana bread recipe with walnuts".into();
        assert_eq!(replace_source(&mut db, "notes", &[lease, changed]).unwrap(), 1);
        assert_eq!(unembedded(&db, 10).unwrap().len(), 1, "only the changed note is re-embedded");
        assert_eq!(replace_source(&mut db, "notes", &[]).unwrap(), 0);
        assert!(counts(&db).unwrap().is_empty());
        assert!(search(&db, "lease", None, 5).unwrap().is_empty(), "keyword rows go with their records");
    }

    #[test]
    fn keyword_and_meaning_ranks_are_fused() {
        let mut db = db();
        replace_source(&mut db, "notes", &[
            record("notes", "n1", "Rental contract", "Sign by Friday", vec![Share::Notes]),
            record("notes", "n2", "Lease", "Banana bread for the lease party", vec![Share::Notes]),
            record("notes", "n3", "Groceries", "Milk and eggs", vec![Share::Notes]),
        ])
        .unwrap();
        for (id, _) in unembedded(&db, 10).unwrap() {
            let vector: &[i8] = match id { 1 => &[100, 0, 0], 2 => &[0, 100, 0], _ => &[0, 0, 100] };
            set_vector(&db, id, Some(vector)).unwrap();
        }
        // "lease" matches n2 by keyword; the query vector is closest to n1.
        let hits = search(&db, "lease", Some(&[90, 10, 0]), 3).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.source_id.as_str()).collect();
        assert_eq!(ids[0], "n2", "a match on both lists wins");
        assert!(ids.contains(&"n1"));
        assert!(search(&db, "", None, 3).unwrap().is_empty());
        assert_eq!(search(&db, "milk", None, 3).unwrap()[0].source_id, "n3");
    }

    #[test]
    fn shared_documents_are_indexed_by_their_text_only_while_shared() {
        let store = store_allowing(&[]);
        store.add_root("/shared").unwrap();
        let hit = |path: &str| crate::types::FileHit {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            size: 1,
            modified_at: String::new(),
            excerpt: None,
        };
        store.index_file("/shared", &hit("/shared/lease.pdf"), "Deposit returned 1200").unwrap();
        store.index_file("/revoked", &hit("/revoked/offer.docx"), "Private offer 900").unwrap();
        let sources = store_sources(&store).unwrap();
        let files = &sources.iter().find(|(source, _)| *source == "files").unwrap().1;
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].title.as_str(), files[0].text.as_str()), ("lease.pdf", "Deposit returned 1200"));
        assert_eq!(files[0].scope, "/shared");
    }

    #[test]
    fn purging_removes_sources_scopes_and_single_records() {
        let mut db = db();
        let mut a = record("files", "/a/lease.txt", "lease.txt", "lease", vec![Share::Files]);
        a.scope = "/a".into();
        let mut b = record("files", "/b/lease.txt", "lease.txt", "lease", vec![Share::Files]);
        b.scope = "/b".into();
        replace_source(&mut db, "files", &[a, b]).unwrap();
        replace_source(&mut db, "memory", &[record("memory", "7", "Remembered", "I'm vegetarian", vec![Share::Memory])]).unwrap();
        purge(&db, "files", Some("/a")).unwrap();
        assert_eq!(search(&db, "lease", None, 5).unwrap().len(), 1);
        purge_record(&db, "memory", "7").unwrap();
        assert!(search(&db, "vegetarian", None, 5).unwrap().is_empty());
        let mut settings = Settings::default();
        replace_source(&mut db, "mail", &[record("mail", "1", "Hi", "lease", vec![Share::Mail])]).unwrap();
        settings.apps = vec!["notes".into()];
        purge_disallowed(&db, &settings).unwrap();
        assert_eq!(counts(&db).unwrap(), [("files".to_string(), 1)]);
    }

    #[test]
    fn hits_are_visible_only_with_every_category_and_app_still_allowed() {
        let mut db = db();
        replace_source(&mut db, "history", &[record("history", "t1", "Chat", "lease from my email", vec![Share::History, Share::Mail])]).unwrap();
        replace_source(&mut db, "mail", &[record("mail", "m1", "Lease", "lease", vec![Share::Mail])]).unwrap();
        let hits = search(&db, "lease", None, 5).unwrap();
        let history = hits.iter().find(|h| h.source == "history").unwrap();
        let mail = hits.iter().find(|h| h.source == "mail").unwrap();
        let mut settings = Settings::default();
        assert!(visible(history, Locality::OnDevice, &settings, &[]));
        assert!(!visible(mail, Locality::OnDevice, &settings, &[]), "Mail is switched off");
        settings.apps = vec!["mail".into()];
        assert!(visible(mail, Locality::OnDevice, &settings, &[]));
        settings.external_shares = vec!["history".into()];
        assert!(!visible(history, Locality::External, &settings, &[]), "the chat contained email");
        settings.external_shares = vec!["history".into(), "mail".into()];
        assert!(visible(history, Locality::External, &settings, &[]));
        let file = Hit { source: "files".into(), source_id: "/a/x.txt".into(), title: String::new(), excerpt: String::new(), observed_at: String::new(), shares: vec!["files".into()], score: 0.0 };
        assert!(!visible(&file, Locality::OnDevice, &settings, &["/b".into()]));
        assert!(!visible(&file, Locality::OnDevice, &settings, &["/a/x".into()]));
        assert!(visible(&file, Locality::OnDevice, &settings, &["/a".into()]));
    }
}
