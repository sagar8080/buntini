use crate::types::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Store {
    pub db: Connection,
    /// Moves on whenever indexed data is deleted or revoked, so a memory refresh that
    /// read before the change can tell and never writes the old data back.
    index_epoch: AtomicU64,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;
          CREATE TABLE IF NOT EXISTS preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS roots (path TEXT PRIMARY KEY);
          CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, fact TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, data TEXT NOT NULL, created_at TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS audit (id INTEGER PRIMARY KEY, action TEXT NOT NULL, detail TEXT NOT NULL, created_at TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS search_usage (month TEXT PRIMARY KEY, used INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS usage_monthly (month TEXT NOT NULL, provider TEXT NOT NULL, used INTEGER NOT NULL, estimated_spend REAL NOT NULL, PRIMARY KEY (month, provider));
          CREATE TABLE IF NOT EXISTS usage_daily (day TEXT NOT NULL, provider TEXT NOT NULL, used INTEGER NOT NULL, PRIMARY KEY (day, provider));
          CREATE TABLE IF NOT EXISTS search_cache (key TEXT PRIMARY KEY, data TEXT NOT NULL, expires INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS saved_searches (id TEXT PRIMARY KEY, data TEXT NOT NULL, next_run_at TEXT NOT NULL, enabled INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS price_watches (id TEXT PRIMARY KEY, data TEXT NOT NULL, next_run_at TEXT NOT NULL, enabled INTEGER NOT NULL);
          CREATE VIRTUAL TABLE IF NOT EXISTS file_index USING fts5(path UNINDEXED, root UNINDEXED, name, content);")
            .map_err(|e| format!("Cannot initialize local database: {e}"))?;
        // A prior interruption is recoverable and never silently shown as still running.
        let store = Self { db, index_epoch: AtomicU64::new(0) };
        crate::workflows::initialize(&store.db)?;
        crate::delivery::initialize(&store.db)?;
        crate::chat_threads::initialize(&store)?;
        crate::memory_index::initialize(&store.db)?;
        crate::routines::initialize(&store)?;
        crate::mcp::initialize(&store.db)?;
        crate::nudges::initialize(&store.db)?;
        crate::goals::initialize(&store)?;
        let has_spend: bool = {
            let mut statement = store
                .db
                .prepare("PRAGMA table_info(search_usage)")
                .map_err(|e| e.to_string())?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| e.to_string())?;
            let found = columns
                .filter_map(Result::ok)
                .any(|name| name == "estimated_spend");
            found
        };
        if !has_spend {
            store
                .db
                .execute(
                    "ALTER TABLE search_usage ADD COLUMN estimated_spend REAL NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|e| e.to_string())?;
        }
        store.migrate_search_usage()?;
        store.recover_workflows()?;
        for mut task in store.tasks()? {
            if task.state == "running" {
                task.state = "error".into();
                task.answer =
                    "This task was interrupted when Buntini closed. You can run it again.".into();
                store.save_task(&task)?;
            }
        }
        Ok(store)
    }
    pub fn settings(&self) -> Result<Settings, String> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let stored = raw
            .as_ref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        let old_schema = stored
            .as_ref()
            .map(|value| value.get("assistantName").is_none())
            .unwrap_or(false);
        // Saved before brains were selectable: keep a working local MLX setup, otherwise
        // start on Apple Intelligence like a fresh install.
        let needs_backend = stored
            .as_ref()
            .is_some_and(|value| value.get("modelBackend").is_none());
        let mut settings: Settings = raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // The fox and nine-tailed fox were retired; anyone who chose one gets the cat.
        if settings.avatar == "fox" || settings.avatar == "kyuubi" {
            settings.avatar = "cat".into();
        }
        // Settings no longer show the Whisper folder or speech filter. One that has since
        // moved or been deleted can't be cleared by the user, so it is forgotten here
        // instead of failing every later save; voice then uses Apple's speech.
        if !settings.whisper_model.is_empty() && !Path::new(&settings.whisper_model).is_dir() {
            settings.whisper_model.clear();
        }
        let vad = Path::new(&settings.vad_model_path);
        if !settings.vad_model_path.is_empty() && !(vad.is_absolute() && vad.is_file()) {
            settings.vad_model_path.clear();
        }
        // The status line has no setting any more either; Buntini words it itself.
        settings.status_text.clear();
        if needs_backend {
            settings.model_backend = if crate::model::mlx_model_present(&settings.model_path) {
                "mlx"
            } else {
                "apple"
            }
            .into();
            if !old_schema {
                self.db
                    .execute(
                        "INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)",
                        [serde_json::to_string(&settings).map_err(|e| e.to_string())?],
                    )
                    .map_err(|e| e.to_string())?;
                self.audit(
                    "settings.migrated",
                    &format!(
                        "Model choice set to {} for existing preferences",
                        settings.model_backend
                    ),
                )?;
            }
        }
        if old_schema {
            if settings.python_path == "python3" {
                settings.python_path.clear();
            }
            self.db
                .execute(
                    "INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)",
                    [serde_json::to_string(&settings).map_err(|e| e.to_string())?],
                )
                .map_err(|e| e.to_string())?;
            self.audit(
                "settings.migrated",
                "Existing preferences preserved; new profile settings use their defaults",
            )?;
        }
        Ok(settings)
    }
    /// Which deletion or revocation the memory index has seen; see `index_epoch`.
    pub fn index_epoch(&self) -> u64 {
        self.index_epoch.load(Ordering::SeqCst)
    }
    fn bump_index(&self) {
        self.index_epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        // An app switched off in Settings → Apps takes its records out of the memory index.
        if crate::memory_index::purge_disallowed(&self.db, settings)? {
            self.bump_index();
        }
        settings.validate()?;
        let raw = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences VALUES('settings',?1)",
                [raw],
            )
            .map_err(|e| e.to_string())?;
        // The built-in morning brief routine is on exactly when its setting is.
        crate::routines::follow_setting(self, settings.morning_brief)
    }
    pub fn roots(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT path FROM roots ORDER BY path")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    pub fn add_root(&self, root: &str) -> Result<(), String> {
        self.db
            .execute("INSERT OR IGNORE INTO roots VALUES (?1)", [root])
            .map_err(|e| e.to_string())?;
        self.audit("folder.allowed", root)
    }
    pub fn remove_root(&self, root: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM roots WHERE path=?1", [root])
            .map_err(|e| e.to_string())?;
        self.db
            .execute("DELETE FROM file_index WHERE root=?1", [root])
            .map_err(|e| e.to_string())?;
        crate::memory_index::purge(&self.db, "files", Some(root))?;
        self.bump_index();
        self.audit("folder.revoked", root)
    }
    /// Remembers a fact; repeating one already known changes nothing. Keeps at most 200.
    pub fn remember(&self, fact: &str) -> Result<bool, String> {
        let fact = fact.split_whitespace().collect::<Vec<_>>().join(" ");
        if fact.chars().count() < 3 || fact.chars().count() > 300 {
            return Err("Memories are 3–300 characters.".into());
        }
        let exists: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE fact=?1)",
                [&fact],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Ok(false);
        }
        let count: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count >= 200 {
            return Err("Memory is full. Remove a few things in Settings → Privacy.".into());
        }
        let added = self
            .db
            .execute(
                "INSERT OR IGNORE INTO memories(fact,created_at) VALUES(?1,?2)",
                params![fact, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(added > 0)
    }
    pub fn memories(&self) -> Result<Vec<Memory>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT id,fact,created_at FROM memories ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Memory {
                    id: r.get(0)?,
                    fact: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    pub fn forget(&self, id: i64) -> Result<(), String> {
        self.db
            .execute("DELETE FROM memories WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        crate::memory_index::purge_record(&self.db, "memory", &id.to_string())?;
        self.bump_index();
        Ok(())
    }
    pub fn preference(&self, key: &str) -> Result<Option<String>, String> {
        self.db
            .query_row("SELECT value FROM preferences WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn set_preference(&self, key: &str, value: &str) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences(key,value) VALUES(?1,?2)",
                [key, value],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn delete_task(&self, id: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM tasks WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        self.db
            .execute("DELETE FROM conversation_turns WHERE task_id=?1", [id])
            .map_err(|e| e.to_string())?;
        // The emails a "What needs a reply?" turn listed go with that turn.
        self.db
            .execute(
                "DELETE FROM preferences WHERE key=?1 AND json_extract(value,'$.taskId')=?2",
                [crate::triage::LISTING, id],
            )
            .map_err(|e| e.to_string())?;
        crate::memory_index::purge_record(&self.db, "history", id)?;
        self.bump_index();
        Ok(())
    }
    pub fn save_task(&self, task: &TaskRecord) -> Result<(), String> {
        self.active_conversation_id()?;
        self.db
            .execute(
                "INSERT OR REPLACE INTO tasks VALUES(?1,?2,?3)",
                params![
                    task.id,
                    serde_json::to_string(task).map_err(|e| e.to_string())?,
                    task.created_at
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn tasks(&self) -> Result<Vec<TaskRecord>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM tasks WHERE id IN (SELECT id FROM tasks ORDER BY created_at DESC LIMIT 100) OR id IN (SELECT id FROM workflows WHERE state IN ('paused','waiting_action','waiting_online','unknown')) ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            r.map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        })
        .collect()
    }
    /// The newest finished conversation turns worth remembering (at most `limit`), for
    /// the memory index. Unlike `tasks`, this isn't capped at the 100 shown in the app.
    pub fn history_for_index(&self, limit: usize) -> Result<Vec<TaskRecord>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM tasks ORDER BY created_at DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([(limit * 2) as i64], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut turns = Vec::new();
        for row in rows {
            let task: TaskRecord = serde_json::from_str(&row.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if crate::memory_index::remembered_turn(&task) {
                turns.push(task);
            }
            if turns.len() == limit {
                break;
            }
        }
        Ok(turns)
    }
    pub fn conversation_start(&self) -> Result<Option<String>, String> {
        self.db
            .query_row(
                "SELECT value FROM preferences WHERE key='conversation-start'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    /// Setup counts as done once finished, or for anyone who already chatted before
    /// onboarding existed. "Run setup again" stores an explicit "no".
    pub fn onboarded(&self) -> Result<bool, String> {
        let state: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM preferences WHERE key='onboarding'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match state.as_deref() {
            Some("pending") => Ok(false),
            Some(_) => Ok(true),
            None => self
                .db
                .query_row("SELECT EXISTS(SELECT 1 FROM tasks)", [], |r| r.get(0))
                .map_err(|e| e.to_string()),
        }
    }
    pub fn set_onboarded(&self, done: bool) -> Result<(), String> {
        let value = if done { now() } else { "pending".into() };
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences(key,value) VALUES('onboarding',?1)",
                [&value],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn task(&self, id: &str) -> Result<Option<TaskRecord>, String> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT data FROM tasks WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM saved_searches ORDER BY next_run_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            r.map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        })
        .collect()
    }
    pub fn saved_search(&self, id: &str) -> Result<Option<SavedSearch>, String> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT data FROM saved_searches WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn save_search(&self, search: &SavedSearch) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO saved_searches VALUES(?1,?2,?3,?4)",
                params![
                    search.id,
                    serde_json::to_string(search).map_err(|e| e.to_string())?,
                    search.next_run_at,
                    search.enabled
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn delete_search(&self, id: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM saved_searches WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        self.audit("saved_search.deleted", id)
    }
    pub fn price_watches(&self) -> Result<Vec<PriceWatch>, String> {
        let mut statement = self
            .db
            .prepare("SELECT data FROM price_watches ORDER BY next_run_at")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            row.map_err(|e| e.to_string())
                .and_then(|data| serde_json::from_str(&data).map_err(|e| e.to_string()))
        })
        .collect()
    }
    pub fn price_watch(&self, id: &str) -> Result<Option<PriceWatch>, String> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT data FROM price_watches WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|data| serde_json::from_str(&data).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn save_price_watch(&self, watch: &PriceWatch) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO price_watches VALUES(?1,?2,?3,?4)",
                params![
                    watch.id,
                    serde_json::to_string(watch).map_err(|e| e.to_string())?,
                    watch.next_run_at,
                    watch.enabled
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn delete_price_watch(&self, id: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM price_watches WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        self.audit("price_watch.deleted", id)
    }
    /// The built-in morning brief first, then routines in the order they were made.
    pub fn routines(&self) -> Result<Vec<Routine>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM routines ORDER BY json_extract(data,'$.builtIn') DESC, created_at, id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn routine(&self, id: &str) -> Result<Option<Routine>, String> {
        let data: Option<String> = self
            .db
            .query_row("SELECT data FROM routines WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        data.map(|data| serde_json::from_str(&data).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn save_routine(&self, routine: &Routine) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO routines(id,data,next_run_at,enabled,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![
                    routine.id,
                    serde_json::to_string(routine).map_err(|e| e.to_string())?,
                    routine.next_run_at,
                    routine.enabled,
                    routine.created_at
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn delete_routine(&self, id: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM routines WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        self.audit("routine.deleted", id)
    }
    /// Everything Buntini keeps about you, for "Export my data". Keys stay in the Keychain
    /// and are never included.
    pub fn export_snapshot(&self) -> Result<serde_json::Value, String> {
        let rows = |sql: &str| -> Result<Vec<serde_json::Value>, String> {
            let mut stmt = self.db.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.map(|row| {
                row.map_err(|e| e.to_string())
                    .map(|raw| serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw)))
            })
            .collect()
        };
        Ok(serde_json::json!({
            "exportedAt": now(),
            "settings": self.settings()?,
            "folders": self.roots()?,
            "conversation": rows("SELECT data FROM tasks ORDER BY created_at")?,
            "conversations": rows("SELECT json_object('id',id,'title',title,'createdAt',created_at,'updatedAt',updated_at,'legacy',json(CASE WHEN legacy THEN 'true' ELSE 'false' END)) FROM chat_threads ORDER BY created_at,id")?,
            "conversationMembership": rows("SELECT json_object('taskId',task_id,'conversationId',conversation_id) FROM conversation_turns ORDER BY conversation_id,task_id")?,
            "activeConversationId": self.preference("active-conversation")?,
            "savedSearches": rows("SELECT data FROM saved_searches")?,
            "priceWatches": rows("SELECT data FROM price_watches")?,
            "routines": rows("SELECT data FROM routines ORDER BY created_at")?,
            "toolsYouAdded": self.mcp_export()?,
            "headsUps": rows("SELECT data FROM nudges ORDER BY created_at")?,
            "goals": rows("SELECT data FROM goals ORDER BY created_at")?,
            "memories": self.memories()?,
            "workflows": self.workflows()?,
            "timers": rows("SELECT data FROM timers")?,
            "monitorRuns": rows("SELECT data FROM monitor_runs")?,
            "controlIntents": rows("SELECT json_object('id',id,'tool',tool,'arguments',json(args),'objectId',object_id) FROM control_intents")?,
            "monitorIntents": rows("SELECT json_object('id',id,'tool',tool,'arguments',json(args),'monitorId',monitor_id) FROM monitor_intents")?,
            "activityLog": self.audit_entries()?,
        }))
    }
    /// "Delete everything": every table back to empty, as on a fresh install.
    pub fn erase_everything(&self) -> Result<(), String> {
        // Tokens of the tools the user added live in the Keychain, not in these tables.
        let tool_servers = self.mcp_server_ids()?;
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        tx
            .execute_batch(
                "DELETE FROM control_intents; DELETE FROM monitor_intents; DELETE FROM timers; DELETE FROM monitor_runs; DELETE FROM workflows; DELETE FROM tasks; DELETE FROM conversation_turns; DELETE FROM chat_threads; DELETE FROM file_index; DELETE FROM saved_searches; DELETE FROM price_watches; DELETE FROM routines; DELETE FROM goals;
                 DELETE FROM mcp_servers; DELETE FROM roots; DELETE FROM search_cache; DELETE FROM usage_monthly; DELETE FROM usage_daily;
                 DELETE FROM search_usage; DELETE FROM preferences; DELETE FROM audit; DELETE FROM memories;
                 DELETE FROM memory_records; DELETE FROM memory_fts; DELETE FROM nudges; DELETE FROM nudge_marks; DELETE FROM nudge_sent;",
            )
            .map_err(|e| e.to_string())?;
        self.bump_index();
        crate::triage::forget_cache();
        // The empty database already uses the new thread schema. A task saved before
        // the next restart must not be mistaken for an unmigrated legacy turn.
        self.set_preference("conversation-migration-v1", "1")?;
        self.set_onboarded(false)?;
        // The morning brief comes back as on a fresh install: there, but off.
        crate::routines::initialize(self)?;
        tx.commit().map_err(|e| e.to_string())?;
        crate::mcp::forget_secrets(&tool_servers)?;
        // DELETE alone leaves private text in free pages and older WAL frames.
        // Rebuild the emptied database and truncate the WAL before reporting
        // successful erasure. This is logical removal, not disk sanitization.
        self.db
            .execute_batch("VACUUM;")
            .map_err(|e| e.to_string())?;
        let (busy, _, _): (i64, i64, i64) = self
            .db
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| e.to_string())?;
        if busy != 0 {
            return Err("Data was removed, but a database reader prevented final cleanup. Close and reopen Buntini, then delete everything again.".into());
        }
        Ok(())
    }
    pub fn clear_history(&self) -> Result<(), String> {
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("DELETE FROM workflows; DELETE FROM tasks; DELETE FROM conversation_turns; DELETE FROM chat_threads; DELETE FROM file_index; DELETE FROM preferences WHERE key IN ('active-conversation','conversation-start','mail-listing');
            UPDATE routines SET data=json_remove(data,'$.lastTaskId','$.lastError');
            DELETE FROM nudges;
            DELETE FROM goals WHERE json_extract(data,'$.state') IN ('done','ended');
            UPDATE goals SET data=json_remove(data,'$.lastTaskId','$.note');").map_err(|e| e.to_string())?;
        // Heads-ups name events, reminders and emails, so they go with the history. What
        // was already said (hashed keys and send times, no text) stays, so none repeats.
        crate::triage::forget_cache();
        // Earlier conversations and folder text leave the memory index with them.
        crate::memory_index::purge(&tx, "history", None)?;
        crate::memory_index::purge(&tx, "files", None)?;
        self.bump_index();
        self.active_conversation_id()?;
        self.audit("history.cleared", "Task history and file index cleared")?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn audit(&self, action: &str, detail: &str) -> Result<(), String> {
        self.db
            .execute(
                "INSERT INTO audit(action,detail,created_at) VALUES(?1,?2,?3)",
                params![action, detail, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn audit_entries(&self) -> Result<Vec<AuditEntry>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT id,action,detail,created_at FROM audit ORDER BY id DESC LIMIT 100")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    action: r.get(1)?,
                    detail: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    /// The text the file index holds for one file, if it has it.
    pub fn indexed_content(&self, root: &str, path: &str) -> Option<String> {
        self.db
            .query_row("SELECT content FROM file_index WHERE path=?1 AND root=?2", params![path, root], |row| row.get(0))
            .ok()
    }
    pub fn index_file(&self, root: &str, hit: &FileHit, content: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM file_index WHERE path=?1", [&hit.path])
            .map_err(|e| e.to_string())?;
        self.db
            .execute(
                "INSERT INTO file_index(path,root,name,content) VALUES(?1,?2,?3,?4)",
                params![hit.path, root, hit.name, content],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Moves the old single monthly counter into the per-provider tables, once.
    fn migrate_search_usage(&self) -> Result<(), String> {
        let migrated: bool = self
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM usage_monthly)", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if migrated {
            return Ok(());
        }
        let provider = self
            .settings()
            .map(|settings| settings.search_provider)
            .unwrap_or_else(|_| "brave".into());
        self.db
            .execute(
                "INSERT OR IGNORE INTO usage_monthly(month,provider,used,estimated_spend) SELECT month,?1,used,estimated_spend FROM search_usage",
                [provider],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn usage(&self, provider: &str) -> Result<Usage, String> {
        let now = chrono::Local::now();
        let (month, spend) = self
            .db
            .query_row(
                "SELECT used,estimated_spend FROM usage_monthly WHERE month=?1 AND provider=?2",
                params![now.format("%Y-%m").to_string(), provider],
                |r| Ok((r.get::<_, u32>(0)?, r.get::<_, f64>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or((0, 0.0));
        let today = self
            .db
            .query_row(
                "SELECT used FROM usage_daily WHERE day=?1 AND provider=?2",
                params![now.format("%Y-%m-%d").to_string(), provider],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        Ok(Usage {
            today,
            month,
            spend,
        })
    }
    /// Reserves one request for `provider` before it is sent: failed or cancelled requests
    /// may still cost the provider. Checks today's count, this month's count and this
    /// month's estimated spend together, on local dates.
    pub fn reserve(
        &self,
        provider: &str,
        limits: crate::budget::Limits,
    ) -> Result<Reservation, String> {
        if !limits.unit_cost.is_finite()
            || limits.unit_cost < 0.0
            || limits.budget_usd.is_nan()
            || limits.budget_usd < 0.0
        {
            return Err("Invalid search budget.".into());
        }
        let now = chrono::Local::now();
        let (month, day) = (
            now.format("%Y-%m").to_string(),
            now.format("%Y-%m-%d").to_string(),
        );
        let transaction = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let usage = self.usage(provider)?;
        if usage.today >= limits.daily {
            return Err(format!("Today’s limit of {} online lookups is used up. It resets at midnight.", limits.daily));
        }
        if usage.month >= limits.monthly {
            return Err(format!("This month’s limit of {} lookups for this service is used up. It resets next month.", limits.monthly));
        }
        if usage.spend + limits.unit_cost > limits.budget_usd + 0.000000001 {
            return Err(format!("This month’s estimated {} budget of ${:.2} is used up. You can raise it in Settings, or wait for next month.", crate::budget::display_name(provider), limits.budget_usd));
        }
        transaction
            .execute(
                "INSERT INTO usage_monthly(month,provider,used,estimated_spend) VALUES(?1,?2,1,?3) ON CONFLICT(month,provider) DO UPDATE SET used=used+1,estimated_spend=estimated_spend+?3",
                params![month, provider, limits.unit_cost],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute("INSERT INTO usage_daily(day,provider,used) VALUES(?1,?2,1) ON CONFLICT(day,provider) DO UPDATE SET used=used+1", params![day, provider])
            .map_err(|e| e.to_string())?;
        let (month_used, spend) = (usage.month + 1, usage.spend + limits.unit_cost);
        let ratio = (month_used as f64 / limits.monthly.max(1) as f64).max(
            if limits.budget_usd.is_finite() && limits.budget_usd > 0.0 {
                spend / limits.budget_usd
            } else {
                0.0
            },
        );
        let key = format!("budget-warned-{month}-{provider}");
        let warned: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM preferences WHERE key=?1)",
                [&key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let warning = (ratio >= 0.8 && !warned)
            .then(|| crate::budget::warning(provider, month_used, limits, spend));
        if warning.is_some() {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO preferences(key,value) VALUES(?1,'1')",
                    [&key],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Reservation { warning })
    }
    /// Replaces a reservation's estimate with what the provider actually reported.
    pub fn settle(&self, provider: &str, reserved: f64, actual: f64) -> Result<(), String> {
        let month = chrono::Local::now().format("%Y-%m").to_string();
        self.db
            .execute("UPDATE usage_monthly SET estimated_spend=MAX(0,estimated_spend+?3-?4) WHERE month=?1 AND provider=?2", params![month, provider, actual, reserved])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn save_update_check(&self, check: &OsUpdateCheck) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences(key,value) VALUES('os_update_check',?1)",
                [serde_json::to_string(check).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn index_metadata(&self) -> Result<Option<IndexMetadata>, String> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM preferences WHERE key='file_index_status'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|data| serde_json::from_str(&data).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn save_index_metadata(&self, metadata: &IndexMetadata) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences(key,value) VALUES('file_index_status',?1)",
                [serde_json::to_string(metadata).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn update_check(&self) -> Result<Option<OsUpdateCheck>, String> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM preferences WHERE key='os_update_check'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn cached(&self, key: &str) -> Result<Option<Vec<Source>>, String> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM search_cache WHERE key=?1 AND expires>?2",
                params![key, chrono::Utc::now().timestamp()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        data.map(|d| serde_json::from_str(&d).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn cache(&self, key: &str, sources: &[Source]) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO search_cache VALUES(?1,?2,?3)",
                params![
                    key,
                    serde_json::to_string(sources).map_err(|e| e.to_string())?,
                    chrono::Utc::now().timestamp() + 900
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// One proposal waiting for the person, as the approval inbox lists it. The
/// description names the kind of work only, never the proposal's field values.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApprovalSummary {
    pub task_id: String,
    pub approval_id: String,
    pub tool: String,
    pub description: String,
    /// The person's own request, shown in Buntini only.
    pub goal: String,
    pub created_at: String,
}

impl Store {
    /// Proposals waiting for approval, newest first and one per task. They are
    /// journal rows, not memory, so they survive a restart; the in-memory
    /// approval map only ever held the older one-shot sheets.
    /// The id of every saved proposal still waiting, with no limit, so their alerts can
    /// be withdrawn when they are cleared.
    pub fn waiting_approval_ids(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM workflows WHERE state IN ('waiting_action','waiting_online')")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut ids = vec![];
        for row in rows {
            let flow: crate::workflows::Workflow =
                serde_json::from_str(&row.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(step) = flow.steps.last().filter(|step| step.state == "waiting") {
                ids.push(step.id.clone());
            }
        }
        Ok(ids)
    }
    pub fn pending_approvals(&self) -> Result<Vec<PendingApprovalSummary>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM workflows WHERE state IN ('waiting_action','waiting_online') ORDER BY rowid DESC LIMIT 20")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut pending = vec![];
        for row in rows {
            let flow: crate::workflows::Workflow =
                serde_json::from_str(&row.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            // Only the last step can be waiting; anything else is already used.
            let Some(step) = flow.steps.last().filter(|step| step.state == "waiting") else {
                continue;
            };
            pending.push(PendingApprovalSummary {
                task_id: flow.id.clone(),
                approval_id: step.id.clone(),
                tool: step.tool.clone(),
                description: crate::notifications::approval_body(&step.tool).into(),
                goal: flow.goal.clone(),
                created_at: step.proposed_at.clone(),
            });
        }
        Ok(pending)
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_approvals_lists_paused_proposals_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        let mut task: TaskRecord = serde_json::from_value(json!({"id":"inbox","prompt":"Email Ana the plan","answer":"","state":"running","createdAt":now(),"sources":[],"files":[]})).unwrap();
        let mut flow = crate::workflows::Workflow::new(
            &task,
            "auto",
            &Settings::default(),
            vec![json!({"role":"user","content":task.prompt})],
            None,
            &["draft_email"],
        );
        let args = json!({"to":"ana@example.com","subject":"Plan","body":"Secret details"});
        let id = flow
            .begin(
                "draft_email",
                args.clone(),
                json!({"role":"assistant"}),
                true,
                true,
            )
            .unwrap();
        task.state = "waiting_action".into();
        // Saved twice, as a proposal is when the turn and journal both commit.
        store.checkpoint(&mut flow, &task).unwrap();
        store.checkpoint(&mut flow, &task).unwrap();
        drop(store);
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        let pending = store.pending_approvals().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "one entry per waiting task, after a restart"
        );
        assert_eq!(pending[0].approval_id, id);
        assert_eq!(pending[0].task_id, "inbox");
        assert_eq!(pending[0].tool, "draft_email");
        for value in ["ana@example.com", "Plan", "Secret details"] {
            assert!(!pending[0].description.contains(value));
        }
        let mut flow = store.workflow(&id).unwrap().unwrap();
        flow.approve(&id, args).unwrap();
        task.state = "running".into();
        store.checkpoint(&mut flow, &task).unwrap();
        assert!(
            store.pending_approvals().unwrap().is_empty(),
            "a used approval leaves the inbox"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limit(daily: u32, monthly: u32, budget_usd: f64, unit_cost: f64) -> crate::budget::Limits {
        crate::budget::Limits {
            daily,
            monthly,
            budget_usd,
            unit_cost,
        }
    }
    #[test]
    fn settings_no_screen_shows_any_more_never_block_a_save() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let folder = tempfile::tempdir().unwrap();
        let mut settings = s.settings().unwrap();
        settings.whisper_model = folder.path().join("whisper").to_string_lossy().into();
        settings.vad_model_path = folder.path().join("silero.onnx").to_string_lossy().into();
        settings.status_text = "On vacation".into();
        // Saved by an earlier version, when the files were there.
        s.db.execute(
            "INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
        let loaded = s.settings().unwrap();
        assert!(loaded.whisper_model.is_empty() && loaded.vad_model_path.is_empty());
        assert!(loaded.status_text.is_empty());
        s.save_settings(&loaded).unwrap();
        // A Whisper folder that is still there is kept.
        std::fs::create_dir(folder.path().join("whisper")).unwrap();
        s.db.execute(
            "INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
        assert_eq!(s.settings().unwrap().whisper_model, settings.whisper_model);
    }

    #[test]
    fn daily_monthly_and_spend_limits_are_enforced_per_provider() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        for _ in 0..3 {
            s.reserve("tavily", limit(3, 100, f64::INFINITY, 0.0))
                .unwrap();
        }
        assert!(s
            .reserve("tavily", limit(3, 100, f64::INFINITY, 0.0))
            .unwrap_err()
            .contains("Today’s limit of 3"));
        // eBay has its own counter, so it never uses up Tavily's allowance.
        s.reserve("ebay", limit(3, 100, f64::INFINITY, 0.0))
            .unwrap();
        assert_eq!(
            s.usage("tavily").unwrap(),
            Usage {
                today: 3,
                month: 3,
                spend: 0.0
            }
        );
        assert_eq!(s.usage("ebay").unwrap().month, 1);
        s.reserve("brave", limit(10, 10, 0.012, 0.005)).unwrap();
        s.reserve("brave", limit(10, 10, 0.012, 0.005)).unwrap();
        assert!(s
            .reserve("brave", limit(10, 10, 0.012, 0.005))
            .unwrap_err()
            .contains("budget of $0.01"));
    }
    #[test]
    fn export_has_your_data_and_erase_starts_over() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        s.add_root("/tmp/docs").unwrap();
        s.save_settings(&Settings {
            assistant_name: "Miso".into(),
            ..Settings::default()
        })
        .unwrap();
        s.set_onboarded(true).unwrap();
        let export = s.export_snapshot().unwrap();
        assert_eq!(export["settings"]["assistantName"], "Miso");
        assert_eq!(export["folders"][0], "/tmp/docs");
        assert!(
            !export.to_string().to_lowercase().contains("apikey"),
            "keys never leave the Keychain"
        );
        s.erase_everything().unwrap();
        assert!(s.roots().unwrap().is_empty());
        assert_eq!(s.settings().unwrap().assistant_name, "Buntini");
        assert!(!s.onboarded().unwrap(), "setup runs again after erasing");
    }

    #[test]
    fn deleting_everything_removes_text_from_database_and_wal_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.sqlite3");
        let store = Store::open(&path).unwrap();
        let secret = "private-sentinel-erasure-test";
        store.remember(secret).unwrap();
        store
            .save_routine(&Routine {
                id: "action-routine".into(),
                task: format!("Tell me about {secret}-routine"),
                days: "weekdays".into(),
                time: "08:00".into(),
                enabled: true,
                built_in: false,
                created_at: now(),
                next_run_at: now(),
                last_run_at: Some(now()),
                last_task_id: Some(format!("{secret}-run")),
                last_error: Some(format!("{secret}-error")),
            })
            .unwrap();
        let goal = |id: &str, state: &str| -> crate::goals::Goal {
            serde_json::from_value(serde_json::json!({
                "id": id, "aim": format!("Prepare for {secret}-trip"), "checkIn": format!("Help me prepare for {secret}-trip"),
                "style": "run", "days": "every_day", "time": "09:00", "until": "2026-11-06", "doneWhen": "reply",
                "person": format!("{secret}-person"), "state": state, "createdAt": now(),
                "outcome": format!("{secret}-outcome"), "lastTaskId": format!("{secret}-check-in"), "note": format!("{secret}-note"),
            }))
            .unwrap()
        };
        crate::goals::save(&store, &goal("goal-sentinel", "active")).unwrap();
        crate::goals::save(&store, &goal("goal-ended", "done")).unwrap();
        store
            .db
            .execute(
                "INSERT INTO file_index(path,root,name,content) VALUES(?1,?1,?1,?1)",
                [secret],
            )
            .unwrap();
        // A tool the user added, with the token they gave it.
        store
            .save_mcp_server(&crate::mcp::McpServer {
                id: "tool".into(),
                name: "GitHub".into(),
                program: "npx".into(),
                env: [("GITHUB_TOKEN".to_string(), format!("{secret}-token"))].into(),
                created_at: now(),
                ..Default::default()
            })
            .unwrap();
        // A heads-up names the event it is about.
        store
            .db
            .execute(
                "INSERT INTO nudges(id,data,created_at,expires_at,state) VALUES('n1',?1,?2,?2,'new')",
                params![format!("{{\"title\":\"{secret}-meeting\"}}"), now()],
            )
            .unwrap();
        let mut db = Connection::open(&path).unwrap();
        crate::memory_index::replace_source(
            &mut db,
            "notes",
            &[crate::memory_index::Record {
                source: "notes",
                source_id: "n1".into(),
                scope: String::new(),
                shares: vec![crate::privacy::Share::Notes],
                title: format!("{secret}-title"),
                text: format!("{secret}-note"),
                observed_at: "2026-10-29 09:00".into(),
            }],
        )
        .unwrap();
        drop(db);
        store
            .db
            .execute_batch("PRAGMA wal_checkpoint(FULL)")
            .unwrap();
        let contains = |bytes: &[u8]| {
            bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        };
        assert!(contains(&std::fs::read(&path).unwrap()));
        store.erase_everything().unwrap();
        assert!(!contains(&std::fs::read(&path).unwrap()));
        assert!(crate::goals::all(&store).unwrap().is_empty());
        // Only the built-in morning brief is left, off, as on a fresh install.
        let routines = store.routines().unwrap();
        assert_eq!(routines.len(), 1);
        assert!(routines[0].built_in && !routines[0].enabled);
        let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
        if wal.exists() {
            assert_eq!(std::fs::metadata(wal).unwrap().len(), 0);
        }
    }
    #[test]
    fn clearing_history_keeps_routines_but_not_their_results() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let routine = Routine {
            id: "action-brief".into(),
            task: "Brief me".into(),
            days: "weekdays".into(),
            time: "08:00".into(),
            enabled: true,
            built_in: false,
            created_at: now(),
            next_run_at: now(),
            last_run_at: Some(now()),
            last_task_id: Some("run".into()),
            last_error: Some("The last run didn’t finish.".into()),
        };
        s.save_routine(&routine).unwrap();
        assert_eq!(s.export_snapshot().unwrap()["routines"].as_array().unwrap().len(), 2);
        s.clear_history().unwrap();
        let kept = s.routine("action-brief").unwrap().unwrap();
        assert_eq!(kept.task, "Brief me");
        assert!(kept.enabled);
        assert_eq!(kept.last_task_id, None);
        assert_eq!(kept.last_error, None);
        assert_eq!(kept.last_run_at, routine.last_run_at);
    }
    #[test]
    fn clearing_history_keeps_open_goals_but_not_their_results_or_finished_goals() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let goal = |id: &str, state: &str| -> crate::goals::Goal {
            serde_json::from_value(serde_json::json!({
                "id": id, "aim": "Follow up with Ana", "checkIn": "Draft a short follow-up email to Ana", "style": "run",
                "days": "weekdays", "time": "09:00", "until": "2026-11-13", "doneWhen": "reply", "person": "Ana",
                "state": state, "createdAt": now(), "outcome": "An email from Ana arrived.", "lastTaskId": "run",
                "note": "Mail wasn’t open, so I couldn’t check for a reply from Ana.",
            }))
            .unwrap()
        };
        crate::goals::save(&s, &goal("goal-open", "active")).unwrap();
        crate::goals::save(&s, &goal("goal-done", "done")).unwrap();
        assert_eq!(s.export_snapshot().unwrap()["goals"].as_array().unwrap().len(), 2);
        s.clear_history().unwrap();
        let kept = crate::goals::all(&s).unwrap();
        assert_eq!(kept.len(), 1, "a finished goal is only history");
        assert_eq!((kept[0].id.as_str(), kept[0].person.as_deref()), ("goal-open", Some("Ana")));
        assert_eq!((kept[0].last_task_id.as_deref(), kept[0].note.as_deref()), (None, None));
    }
    #[test]
    fn memory_history_source_reads_up_to_history_turns() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        for n in 0..150 {
            let task: TaskRecord = serde_json::from_value(serde_json::json!({
                "id":format!("t{n}"),"prompt":format!("Question {n}"),"answer":format!("Answer {n}"),
                "state":"completed","createdAt":format!("2026-10-01T09:{:02}:{:02}Z", n / 60, n % 60),
                "backend":"apple","sources":[],"files":[]
            }))
            .unwrap();
            s.save_task(&task).unwrap();
        }
        assert_eq!(s.tasks().unwrap().len(), 100, "the app shows the newest 100");
        let turns = s.history_for_index(crate::memory_index::HISTORY_TURNS).unwrap();
        assert_eq!(turns.len(), 150);
        assert_eq!(turns[0].id, "t149", "newest first");
        assert_eq!(s.history_for_index(10).unwrap().len(), 10);
        let history = |s: &Store| {
            crate::memory_index::counts(&s.db)
                .unwrap()
                .into_iter()
                .find(|(source, _)| source == "history")
                .map_or(0, |(_, n)| n)
        };
        crate::memory_index::apply(&s, s.index_epoch(), vec![]).unwrap();
        assert_eq!(history(&s), 150);
        for (id, _) in crate::memory_index::unembedded(&s.db, 200).unwrap() {
            crate::memory_index::set_vector(&s.db, id, Some(&[1, 2, 3])).unwrap();
        }
        crate::memory_index::apply(&s, s.index_epoch(), vec![]).unwrap();
        assert_eq!(history(&s), 150, "older turns aren't dropped on the next refresh");
        assert!(
            crate::memory_index::unembedded(&s.db, 200).unwrap().is_empty(),
            "and keep their vectors"
        );
    }
    #[test]
    fn memories_are_kept_once_and_can_be_forgotten() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        assert!(s.remember("Sam is  vegetarian").unwrap());
        assert!(
            !s.remember("Sam is vegetarian").unwrap(),
            "the same fact isn't stored twice"
        );
        let saved = s.memories().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].fact, "Sam is vegetarian");
        s.forget(saved[0].id).unwrap();
        assert!(s.memories().unwrap().is_empty());
        assert!(s.remember("no").is_err());
    }

    #[test]
    fn repeating_a_saved_memory_is_idempotent_even_when_capacity_is_full() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        for number in 0..200 {
            assert!(store
                .remember(&format!("Preference number {number}"))
                .unwrap());
        }
        assert!(!store.remember("Preference   number 0").unwrap());
        assert_eq!(store.memories().unwrap().len(), 200);
        assert!(store
            .remember("A new preference")
            .unwrap_err()
            .contains("Memory is full"));
    }
    #[test]
    fn onboarding_is_needed_once_unless_asked_again() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        assert!(!s.onboarded().unwrap(), "a fresh install starts with setup");
        s.set_onboarded(true).unwrap();
        assert!(s.onboarded().unwrap());
        s.set_onboarded(false).unwrap();
        assert!(!s.onboarded().unwrap(), "Run setup again");
    }
    #[test]
    fn cloud_reservations_settle_to_reported_usage() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let settings = Settings {
            cloud_budget_usd: 1.0,
            ..Settings::default()
        };
        s.reserve("cloud-openai", crate::budget::cloud_limits(&settings, 0.6))
            .unwrap();
        assert!(s
            .reserve("cloud-openai", crate::budget::cloud_limits(&settings, 0.6))
            .unwrap_err()
            .contains("OpenAI budget of $1.00"));
        // The reply cost far less than the cautious estimate, which frees the rest of the cap.
        s.settle("cloud-openai", 0.6, 0.01).unwrap();
        assert!((s.usage("cloud-openai").unwrap().spend - 0.01).abs() < 1e-9);
        s.reserve("cloud-openai", crate::budget::cloud_limits(&settings, 0.6))
            .unwrap();
        assert_eq!(
            s.usage("openai").unwrap().month,
            0,
            "cloud spending is kept apart from search counts"
        );
    }
    #[test]
    fn the_80_percent_warning_fires_once_a_month() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let limits = limit(100, 10, f64::INFINITY, 0.0);
        let warnings = (0..10)
            .map(|_| s.reserve("tavily", limits).unwrap().warning)
            .collect::<Vec<_>>();
        assert!(warnings[..7].iter().all(Option::is_none));
        assert_eq!(
            warnings[7].as_deref(),
            Some("Heads up: you’ve used 8 of this month’s 10 Tavily searches.")
        );
        assert!(warnings[8..].iter().all(Option::is_none));
    }
    #[test]
    fn existing_settings_keep_a_working_local_model_and_others_get_apple() {
        let model = tempfile::tempdir().unwrap();
        std::fs::write(model.path().join("config.json"), "{}").unwrap();
        for (model_path, expected) in [
            (model.path().to_string_lossy().into_owned(), "mlx"),
            ("/no/longer/here".to_string(), "apple"),
        ] {
            let s = Store::open(Path::new(":memory:")).unwrap();
            let mut raw = serde_json::to_value(Settings::default()).unwrap();
            raw.as_object_mut().unwrap().remove("modelBackend");
            raw["modelPath"] = serde_json::json!(model_path);
            s.db.execute(
                "INSERT INTO preferences VALUES('settings',?1)",
                [raw.to_string()],
            )
            .unwrap();
            assert_eq!(s.settings().unwrap().model_backend, expected);
            // The choice is saved, so it never flips later.
            let saved: String =
                s.db.query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(saved.contains(&format!("\"modelBackend\":\"{expected}\"")));
        }
        assert_eq!(
            Store::open(Path::new(":memory:"))
                .unwrap()
                .settings()
                .unwrap()
                .model_backend,
            "apple",
            "fresh installs start on Apple Intelligence"
        );
    }
    #[test]
    fn the_old_monthly_counter_is_migrated_to_the_search_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE search_usage (month TEXT PRIMARY KEY, used INTEGER NOT NULL, estimated_spend REAL NOT NULL DEFAULT 0);").unwrap();
            db.execute(
                "INSERT INTO search_usage VALUES(?1, 42, 0.21)",
                [chrono::Local::now().format("%Y-%m").to_string()],
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(
            s.usage("brave").unwrap(),
            Usage {
                today: 0,
                month: 42,
                spend: 0.21
            }
        );
    }
    #[test]
    fn quota_reservations_survive_reopen_and_cap_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.reserve("brave", limit(10, 1, f64::INFINITY, 0.0))
                .unwrap();
            assert!(s
                .reserve("brave", limit(10, 1, f64::INFINITY, 0.0))
                .is_err());
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.usage("brave").unwrap().month, 1);
        assert!(s
            .reserve("brave", limit(10, 1, f64::INFINITY, 0.0))
            .is_err());
    }
    #[test]
    fn roots_are_data_and_revocation_erases_index() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        s.add_root("a' OR 1=1 --").unwrap();
        assert_eq!(s.roots().unwrap().len(), 1);
        let hit = FileHit {
            name: "hello".into(),
            path: "x".into(),
            size: 1,
            modified_at: now(),
            excerpt: None,
        };
        s.index_file("a' OR 1=1 --", &hit, "world").unwrap();
        let n: i64 =
            s.db.query_row(
                "SELECT count(*) FROM file_index WHERE file_index MATCH 'world'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        s.remove_root("a' OR 1=1 --").unwrap();
        let n: i64 =
            s.db.query_row("SELECT count(*) FROM file_index", [], |r| r.get(0))
                .unwrap();
        assert_eq!(n, 0);
    }
    #[test]
    fn saved_search_state_survives_restart_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let search = SavedSearch {
            id: "test".into(),
            query: "local AI news".into(),
            interval_minutes: 60,
            enabled: false,
            created_at: now(),
            next_run_at: now(),
            last_result_task_id: Some("last-task".into()),
            last_error: Some("Quota reached".into()),
        };
        {
            let s = Store::open(&path).unwrap();
            s.save_search(&search).unwrap();
        }
        let s = Store::open(&path).unwrap();
        let found = s.saved_search("test").unwrap().unwrap();
        assert!(!found.enabled);
        assert_eq!(found.last_error.as_deref(), Some("Quota reached"));
        assert_eq!(found.last_result_task_id.as_deref(), Some("last-task"));
        s.delete_search("test").unwrap();
        assert!(s.saved_search("test").unwrap().is_none());
    }
    #[test]
    fn settings_saved_before_pet_customization_get_cat_defaults() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let mut raw = serde_json::to_value(Settings::default()).unwrap();
        for key in ["catCoat", "catEyes", "catAccessory", "petMotion"] {
            raw.as_object_mut().unwrap().remove(key);
        }
        raw["avatar"] = serde_json::json!("kyuubi");
        s.db.execute(
            "INSERT INTO preferences VALUES('settings',?1)",
            [raw.to_string()],
        )
        .unwrap();
        let settings = s.settings().unwrap();
        assert_eq!(
            settings.avatar, "cat",
            "the retired nine-tailed fox becomes the cat"
        );
        assert_eq!(settings.cat_coat, "ginger");
        assert_eq!(settings.pet_motion, "playful");
        let mut invalid = settings.clone();
        invalid.cat_accessory = "crown<script>".into();
        assert!(invalid.validate().is_err());
        invalid.cat_accessory = "flowers".into();
        invalid.cat_coat = "tiger".into();
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn migration_preserves_unavailable_custom_assets_and_opens_settings() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let mut raw = serde_json::to_value(Settings::default()).unwrap();
        raw.as_object_mut().unwrap().remove("assistantName");
        raw["modelPath"] = serde_json::json!("/moved/unavailable/model");
        raw["pythonPath"] = serde_json::json!("/custom/python");
        s.db.execute(
            "INSERT INTO preferences VALUES('settings',?1)",
            [raw.to_string()],
        )
        .unwrap();
        let migrated = s.settings().unwrap();
        assert_eq!(migrated.model_path, "/moved/unavailable/model");
        assert_eq!(migrated.python_path, "/custom/python");
        assert_eq!(migrated.assistant_name, "Buntini");
        assert!(s.settings().is_ok());
    }
    #[test]
    fn price_history_and_estimated_spend_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.reserve("brave", limit(100, 10, 0.02, 0.01)).unwrap();
            let watch = PriceWatch {
                id: "watch".into(),
                url: "https://shop.example.com/product".into(),
                label: "Widget".into(),
                currency: "USD".into(),
                threshold: "9007199254740993.01".into(),
                interval_minutes: 60,
                enabled: false,
                created_at: now(),
                next_run_at: now(),
                last_checked_at: Some(now()),
                last_price: Some("9007199254740993".into()),
                last_error: None,
                last_availability: None,
                history: vec![PricePoint {
                    price: "9007199254740993".into(),
                    currency: "USD".into(),
                    observed_at: now(),
                    url: "https://shop.example.com/product".into(),
                }],
            };
            s.save_price_watch(&watch).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!((s.usage("brave").unwrap().spend - 0.01).abs() < 0.00001);
        let watch = s.price_watch("watch").unwrap().unwrap();
        assert!(!watch.enabled);
        assert_eq!(watch.history[0].price, "9007199254740993");
        s.delete_price_watch("watch").unwrap();
        assert!(s.price_watch("watch").unwrap().is_none());
    }
}
