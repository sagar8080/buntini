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
        let old_schema = raw
            .as_ref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .map(|value| value.get("assistantName").is_none())
            .unwrap_or(false);
        let mut settings: Settings = raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
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
    pub fn used(&self) -> Result<u32, String> {
        self.db
            .query_row(
                "SELECT used FROM search_usage WHERE month=?1",
                [chrono::Utc::now().format("%Y-%m").to_string()],
                |r| r.get(0),
            )
            .optional()
            .map(|x| x.unwrap_or(0))
            .map_err(|e| e.to_string())
    }
    #[cfg(test)]
    pub fn reserve_search(&self, limit: u32) -> Result<(), String> {
        self.reserve_search_budget(limit, 100000.0, 0.0)
    }
    pub fn estimated_spend(&self) -> Result<f64, String> {
        self.db
            .query_row(
                "SELECT estimated_spend FROM search_usage WHERE month=?1",
                [chrono::Utc::now().format("%Y-%m").to_string()],
                |r| r.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(0.0))
            .map_err(|e| e.to_string())
    }
    pub fn reserve_search_budget(
        &self,
        limit: u32,
        budget: f64,
        unit_cost: f64,
    ) -> Result<(), String> {
        if !budget.is_finite() || !unit_cost.is_finite() || budget < 0.0 || unit_cost < 0.0 {
            return Err("Invalid search budget.".into());
        }
        // A reservation is charged before dispatch: failed/cancelled requests may still cost the provider.
        let month = chrono::Utc::now().format("%Y-%m").to_string();
        self.db
            .execute(
                "INSERT OR IGNORE INTO search_usage(month,used) VALUES(?1,0)",
                [&month],
            )
            .map_err(|e| e.to_string())?;
        let changed = self
            .db
            .execute(
            "UPDATE search_usage SET used=used+1,estimated_spend=estimated_spend+?3 WHERE month=?1 AND used<?2 AND estimated_spend+?3<=?4+0.000000001",
            params![month, limit,unit_cost,budget],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            Err("Your monthly search count or estimated spending budget has been reached. Review the limits in Settings to search again.".into())
        } else {
            Ok(())
        }
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
    #[test]
    fn quota_reservations_survive_reopen_and_cap_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.reserve_search(1).unwrap();
            assert!(s.reserve_search(1).is_err());
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.used().unwrap(), 1);
        assert!(s.reserve_search(1).is_err());
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
            s.reserve_search_budget(10, 0.02, 0.01).unwrap();
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
        assert!((s.estimated_spend().unwrap() - 0.01).abs() < 0.00001);
        let watch = s.price_watch("watch").unwrap().unwrap();
        assert!(!watch.enabled);
        assert_eq!(watch.history[0].price, "9007199254740993");
        s.delete_price_watch("watch").unwrap();
        assert!(s.price_watch("watch").unwrap().is_none());
    }
}
