use crate::types::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    pub db: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS roots (path TEXT PRIMARY KEY);
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
        let store = Self { db };
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
        let stored = raw.as_ref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        let old_schema = stored.as_ref().map(|value| value.get("assistantName").is_none()).unwrap_or(false);
        // Saved before brains were selectable: keep a working local MLX setup, otherwise
        // start on Apple Intelligence like a fresh install.
        let needs_backend = stored.as_ref().is_some_and(|value| value.get("modelBackend").is_none());
        let mut settings: Settings = raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if needs_backend {
            settings.model_backend = if crate::model::mlx_model_present(&settings.model_path) { "mlx" } else { "apple" }.into();
            if !old_schema {
                self.db
                    .execute("INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)", [serde_json::to_string(&settings).map_err(|e| e.to_string())?])
                    .map_err(|e| e.to_string())?;
                self.audit("settings.migrated", &format!("Model choice set to {} for existing preferences", settings.model_backend))?;
            }
        }
        if old_schema {
            let defaults = Settings::default();
            if settings.whisper_model.is_empty() && !defaults.whisper_model.is_empty() {
                settings.whisper_model = defaults.whisper_model;
                let prior = std::env::var("HOME")
                    .map(|home| format!("{home}/projects/pulse-venv/bin/python"))
                    .unwrap_or_default();
                if settings.python_path == prior || settings.python_path == "python3" {
                    settings.python_path = defaults.python_path;
                }
            }
            self.db
                .execute(
                    "INSERT OR REPLACE INTO preferences(key,value) VALUES('settings',?1)",
                    [serde_json::to_string(&settings).map_err(|e| e.to_string())?],
                )
                .map_err(|e| e.to_string())?;
            self.audit("settings.migrated","Existing preferences preserved; profile defaults and authorized local voice paths initialized")?;
        }
        Ok(settings)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        settings.validate()?;
        let raw = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        self.db
            .execute(
                "INSERT OR REPLACE INTO preferences VALUES('settings',?1)",
                [raw],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
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
        self.audit("folder.revoked", root)
    }
    pub fn save_task(&self, task: &TaskRecord) -> Result<(), String> {
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
            .prepare("SELECT data FROM tasks ORDER BY created_at DESC LIMIT 100")
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
    pub fn clear_history(&self) -> Result<(), String> {
        self.db
            .execute("DELETE FROM tasks", [])
            .map_err(|e| e.to_string())?;
        self.db
            .execute("DELETE FROM file_index", [])
            .map_err(|e| e.to_string())?;
        self.audit("history.cleared", "Task history and file index cleared")
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
        let migrated: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM usage_monthly)", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if migrated {
            return Ok(());
        }
        let provider = self.settings().map(|settings| settings.search_provider).unwrap_or_else(|_| "brave".into());
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
            .query_row("SELECT used,estimated_spend FROM usage_monthly WHERE month=?1 AND provider=?2", params![now.format("%Y-%m").to_string(), provider], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, f64>(1)?)))
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or((0, 0.0));
        let today = self
            .db
            .query_row("SELECT used FROM usage_daily WHERE day=?1 AND provider=?2", params![now.format("%Y-%m-%d").to_string(), provider], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        Ok(Usage { today, month, spend })
    }
    /// Reserves one request for `provider` before it is sent: failed or cancelled requests
    /// may still cost the provider. Checks today's count, this month's count and this
    /// month's estimated spend together, on local dates.
    pub fn reserve(&self, provider: &str, limits: crate::budget::Limits) -> Result<Reservation, String> {
        if !limits.unit_cost.is_finite() || limits.unit_cost < 0.0 || limits.budget_usd.is_nan() || limits.budget_usd < 0.0 {
            return Err("Invalid search budget.".into());
        }
        let now = chrono::Local::now();
        let (month, day) = (now.format("%Y-%m").to_string(), now.format("%Y-%m-%d").to_string());
        let transaction = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let usage = self.usage(provider)?;
        if usage.today >= limits.daily {
            return Err(format!("Today’s limit of {} online lookups is used up. It resets at midnight, or you can raise it in Settings.", limits.daily));
        }
        if usage.month >= limits.monthly {
            return Err(format!("This month’s limit of {} lookups for this service is used up. Review the limits in Settings to search again.", limits.monthly));
        }
        if usage.spend + limits.unit_cost > limits.budget_usd + 0.000000001 {
            return Err(format!("This month’s estimated budget of ${:.2} is used up. Review the limits in Settings to search again.", limits.budget_usd));
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
        let ratio = (month_used as f64 / limits.monthly.max(1) as f64).max(if limits.budget_usd.is_finite() && limits.budget_usd > 0.0 { spend / limits.budget_usd } else { 0.0 });
        let key = format!("budget-warned-{month}-{provider}");
        let warned: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM preferences WHERE key=?1)", [&key], |r| r.get(0)).map_err(|e| e.to_string())?;
        let warning = (ratio >= 0.8 && !warned).then(|| crate::budget::warning(provider, month_used, limits, spend));
        if warning.is_some() {
            transaction.execute("INSERT OR REPLACE INTO preferences(key,value) VALUES(?1,'1')", [&key]).map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Reservation { warning })
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

#[cfg(test)]
mod tests {
    use super::*;
    fn limit(daily: u32, monthly: u32, budget_usd: f64, unit_cost: f64) -> crate::budget::Limits {
        crate::budget::Limits { daily, monthly, budget_usd, unit_cost }
    }
    #[test]
    fn daily_monthly_and_spend_limits_are_enforced_per_provider() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        for _ in 0..3 {
            s.reserve("tavily", limit(3, 100, f64::INFINITY, 0.0)).unwrap();
        }
        assert!(s.reserve("tavily", limit(3, 100, f64::INFINITY, 0.0)).unwrap_err().contains("Today’s limit of 3"));
        // eBay has its own counter, so it never uses up Tavily's allowance.
        s.reserve("ebay", limit(3, 100, f64::INFINITY, 0.0)).unwrap();
        assert_eq!(s.usage("tavily").unwrap(), Usage { today: 3, month: 3, spend: 0.0 });
        assert_eq!(s.usage("ebay").unwrap().month, 1);
        s.reserve("brave", limit(10, 10, 0.012, 0.005)).unwrap();
        s.reserve("brave", limit(10, 10, 0.012, 0.005)).unwrap();
        assert!(s.reserve("brave", limit(10, 10, 0.012, 0.005)).unwrap_err().contains("budget of $0.01"));
    }
    #[test]
    fn the_80_percent_warning_fires_once_a_month() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let limits = limit(100, 10, f64::INFINITY, 0.0);
        let warnings = (0..10).map(|_| s.reserve("tavily", limits).unwrap().warning).collect::<Vec<_>>();
        assert!(warnings[..7].iter().all(Option::is_none));
        assert_eq!(warnings[7].as_deref(), Some("Heads up: you’ve used 8 of this month’s 10 Tavily searches."));
        assert!(warnings[8..].iter().all(Option::is_none));
    }
    #[test]
    fn existing_settings_keep_a_working_local_model_and_others_get_apple() {
        let model = tempfile::tempdir().unwrap();
        std::fs::write(model.path().join("config.json"), "{}").unwrap();
        for (model_path, expected) in [(model.path().to_string_lossy().into_owned(), "mlx"), ("/no/longer/here".to_string(), "apple")] {
            let s = Store::open(Path::new(":memory:")).unwrap();
            let mut raw = serde_json::to_value(Settings::default()).unwrap();
            raw.as_object_mut().unwrap().remove("modelBackend");
            raw["modelPath"] = serde_json::json!(model_path);
            s.db.execute("INSERT INTO preferences VALUES('settings',?1)", [raw.to_string()]).unwrap();
            assert_eq!(s.settings().unwrap().model_backend, expected);
            // The choice is saved, so it never flips later.
            let saved: String = s.db.query_row("SELECT value FROM preferences WHERE key='settings'", [], |r| r.get(0)).unwrap();
            assert!(saved.contains(&format!("\"modelBackend\":\"{expected}\"")));
        }
        assert_eq!(Store::open(Path::new(":memory:")).unwrap().settings().unwrap().model_backend, "apple", "fresh installs start on Apple Intelligence");
    }
    #[test]
    fn the_old_monthly_counter_is_migrated_to_the_search_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE search_usage (month TEXT PRIMARY KEY, used INTEGER NOT NULL, estimated_spend REAL NOT NULL DEFAULT 0);").unwrap();
            db.execute("INSERT INTO search_usage VALUES(?1, 42, 0.21)", [chrono::Local::now().format("%Y-%m").to_string()]).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.usage("brave").unwrap(), Usage { today: 0, month: 42, spend: 0.21 });
    }
    #[test]
    fn quota_reservations_survive_reopen_and_cap_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.reserve("brave", limit(10, 1, f64::INFINITY, 0.0)).unwrap();
            assert!(s.reserve("brave", limit(10, 1, f64::INFINITY, 0.0)).is_err());
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.usage("brave").unwrap().month, 1);
        assert!(s.reserve("brave", limit(10, 1, f64::INFINITY, 0.0)).is_err());
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
        assert_eq!(settings.avatar, "kyuubi");
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
