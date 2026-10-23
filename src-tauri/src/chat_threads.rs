//! Durable conversation membership, independent of task and workflow lifecycles.
//! Legacy storage retained only the latest topic boundary: older boundaries cannot be recovered.
use crate::{store::Store, types::*};
use rusqlite::{params, OptionalExtension};

const SUMMARY: &str = "SELECT c.id,c.title,c.created_at,c.updated_at,c.legacy,COUNT(t.id) FROM chat_threads c LEFT JOIN conversation_turns m ON m.conversation_id=c.id LEFT JOIN tasks t ON t.id=m.task_id";
fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationSummary> {
    Ok(ConversationSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
        legacy: row.get(4)?,
        turn_count: row.get::<_, i64>(5)? as usize,
    })
}

pub fn initialize(store: &Store) -> Result<(), String> {
    store.db.execute_batch("CREATE TABLE IF NOT EXISTS chat_threads(id TEXT PRIMARY KEY,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,legacy INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS conversation_turns(task_id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES chat_threads(id));
        CREATE INDEX IF NOT EXISTS conversation_turns_thread ON conversation_turns(conversation_id);
        CREATE INDEX IF NOT EXISTS tasks_chronological ON tasks(created_at,id);").map_err(|e| e.to_string())?;
    // Savepoint makes migration atomic, including a process exit halfway through.
    store
        .db
        .execute_batch("SAVEPOINT conversation_migration")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        if store.preference("conversation-migration-v1")?.is_none() {
            let boundary = store.conversation_start()?;
            let old: i64 = store
                .db
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE ?1 IS NULL OR created_at<=?1",
                    [&boundary],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut active = None;
            if old > 0 {
                let id = store.insert_conversation("Earlier conversations (legacy)", true)?;
                store.db.execute("INSERT OR IGNORE INTO conversation_turns SELECT id,?1 FROM tasks WHERE ?2 IS NULL OR created_at<=?2", params![id,boundary]).map_err(|e| e.to_string())?;
                active = Some(id);
            }
            if boundary.is_some() {
                let id = store.insert_conversation("Current conversation (imported)", true)?;
                store.db.execute("INSERT OR IGNORE INTO conversation_turns SELECT id,?1 FROM tasks WHERE created_at>?2", params![id,boundary]).map_err(|e| e.to_string())?;
                active = Some(id);
            }
            if let Some(id) = active {
                store.set_preference("active-conversation", &id)?;
            }
            store.set_preference("conversation-migration-v1", "1")?;
        }
        store.active_conversation_id()?;
        // Membership survives INSERT OR REPLACE used by workflow checkpoints. NOT EXISTS
        // is essential: SQLite's outer REPLACE policy overrides a trigger's OR IGNORE.
        // There is
        // intentionally no task FK with ON DELETE CASCADE: REPLACE would erase it.
        store.db.execute_batch("DROP TRIGGER IF EXISTS assign_conversation;
            CREATE TRIGGER assign_conversation AFTER INSERT ON tasks BEGIN
            INSERT INTO conversation_turns(task_id,conversation_id) SELECT NEW.id,value FROM preferences WHERE key='active-conversation' AND NOT EXISTS(SELECT 1 FROM conversation_turns WHERE task_id=NEW.id);
            UPDATE chat_threads SET title=CASE WHEN title='New conversation' THEN COALESCE(NULLIF(substr(json_extract(NEW.data,'$.prompt'),1,100),''),'Morning brief') ELSE title END,
                updated_at=MAX(updated_at,NEW.created_at)
                WHERE id=(SELECT conversation_id FROM conversation_turns WHERE task_id=NEW.id);
            END;").map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_ok() {
        store
            .db
            .execute_batch("RELEASE conversation_migration")
            .map_err(|e| e.to_string())?;
    } else {
        let _ = store
            .db
            .execute_batch("ROLLBACK TO conversation_migration; RELEASE conversation_migration");
    }
    result
}
impl Store {
    /// Called inside the scheduler's task/run transaction, before its first task
    /// insert. Background checks have a stable thread without changing selection.
    pub fn assign_scheduled_search_conversation(
        &self,
        task: &TaskRecord,
        search: &SavedSearch,
    ) -> Result<(), String> {
        let id = format!("saved-search:{}", search.id);
        let title = format!(
            "Saved search: {}",
            search.query.chars().take(85).collect::<String>()
        );
        self.db.execute("INSERT INTO chat_threads(id,title,created_at,updated_at,legacy) VALUES(?1,?2,?3,?3,0) ON CONFLICT(id) DO NOTHING", params![id,title,now()]).map_err(|e| e.to_string())?;
        self.db.execute("INSERT INTO conversation_turns(task_id,conversation_id) VALUES(?1,?2) ON CONFLICT(task_id) DO NOTHING", params![task.id,id]).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn insert_conversation(&self, title: &str, legacy: bool) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.db
            .execute(
                "INSERT INTO chat_threads VALUES(?1,?2,?3,?3,?4)",
                params![id, title, now(), legacy],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn active_conversation_id(&self) -> Result<String, String> {
        if let Some(id) = self.preference("active-conversation")? {
            if self.conversation_summary(&id).is_ok() {
                return Ok(id);
            }
        }
        let id = self.insert_conversation("New conversation", false)?;
        self.set_preference("active-conversation", &id)?;
        Ok(id)
    }
    pub fn new_conversation(&self) -> Result<ConversationPage, String> {
        let id = self.insert_conversation("New conversation", false)?;
        self.set_preference("active-conversation", &id)?;
        self.conversation_page(&id, None, 100)
    }
    pub fn task_conversation(&self, task_id: &str) -> Result<String, String> {
        self.db.query_row("SELECT conversation_id FROM conversation_turns m JOIN tasks t ON t.id=m.task_id WHERE task_id=?1", [task_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?.ok_or_else(|| "This task has no saved conversation.".into())
    }
    pub fn conversation_summary(&self, id: &str) -> Result<ConversationSummary, String> {
        self.db
            .query_row(
                &format!("{SUMMARY} WHERE c.id=?1 GROUP BY c.id"),
                [id],
                summary,
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Conversation not found.".into())
    }
    pub fn open_conversation(&self, id: &str) -> Result<ConversationPage, String> {
        let page = self.conversation_page(id, None, 100)?;
        self.set_preference("active-conversation", id)?;
        Ok(page)
    }
    pub fn conversation_page(
        &self,
        id: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ConversationPage, String> {
        let conversation = self.conversation_summary(id)?;
        let at = if let Some(cursor) = cursor {
            Some(self.db.query_row("SELECT t.created_at FROM tasks t JOIN conversation_turns m ON m.task_id=t.id WHERE m.conversation_id=?1 AND t.id=?2", params![id,cursor], |r| r.get::<_, String>(0)).optional().map_err(|e| e.to_string())?.ok_or("Invalid conversation cursor.")?)
        } else {
            None
        };
        let limit = limit.clamp(1, 100) as usize;
        let mut stmt = self.db.prepare("SELECT t.data FROM tasks t JOIN conversation_turns m ON m.task_id=t.id WHERE m.conversation_id=?1 AND (?2 IS NULL OR t.created_at<?2 OR (t.created_at=?2 AND t.id<?3)) ORDER BY t.created_at DESC,t.id DESC LIMIT ?4").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![id, at, cursor, (limit + 1) as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        let mut turns: Vec<TaskRecord> = rows
            .map(|r| {
                r.map_err(|e| e.to_string())
                    .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let more = turns.len() > limit;
        turns.truncate(limit);
        let next_cursor = if more {
            turns.last().map(|t| t.id.clone())
        } else {
            None
        };
        Ok(ConversationPage {
            conversation,
            turns,
            next_cursor,
        })
    }
    /// A task always uses its own thread, even when resumed after another was opened.
    pub fn conversation_history_for_task(&self, task_id: &str) -> Result<Vec<TaskRecord>, String> {
        let id = self.task_conversation(task_id)?;
        let mut stmt = self.db.prepare("SELECT t.data FROM tasks t JOIN conversation_turns m ON m.task_id=t.id WHERE m.conversation_id=?1 AND t.id<>?2 AND json_extract(t.data,'$.state')='completed' ORDER BY t.created_at DESC,t.id DESC LIMIT 100").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![id, task_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            r.map_err(|e| e.to_string())
                .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
        })
        .collect()
    }
    pub fn conversation_list(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ConversationList, String> {
        // Cursor encodes the sort tuple instead of looking up a mutable updated_at.
        let tuple: Option<(String, String)> = cursor
            .map(|raw| {
                serde_json::from_str(raw)
                    .map_err(|_| "Invalid conversation list cursor.".to_string())
            })
            .transpose()?;
        let (at, id) = tuple
            .as_ref()
            .map(|(at, id)| (Some(at), Some(id)))
            .unwrap_or((None, None));
        let limit = limit.clamp(1, 100) as usize;
        let mut stmt = self.db.prepare(&format!("{SUMMARY} WHERE (?1 IS NULL OR c.updated_at<?1 OR (c.updated_at=?1 AND c.id<?2)) GROUP BY c.id HAVING COUNT(t.id)>0 OR c.id=(SELECT value FROM preferences WHERE key='active-conversation') ORDER BY c.updated_at DESC,c.id DESC LIMIT ?3")).map_err(|e| e.to_string())?;
        let mut items = stmt
            .query_map(params![at, id, (limit + 1) as i64], summary)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = if more {
            items
                .last()
                .map(|s| serde_json::to_string(&(&s.updated_at, &s.id)).unwrap())
        } else {
            None
        };
        Ok(ConversationList { items, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{privacy::Locality, workflows::Workflow};
    use serde_json::json;
    fn turn(id: &str, at: &str) -> TaskRecord {
        serde_json::from_value(json!({"id":id,"prompt":format!("Question {id}"),"answer":format!("Answer {id}"),"state":"completed","createdAt":at,"sources":[],"files":[]})).unwrap()
    }
    fn legacy_db(path: &std::path::Path, boundary: Option<&str>) {
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE tasks(id TEXT PRIMARY KEY,data TEXT NOT NULL,created_at TEXT NOT NULL);").unwrap();
        for (id, at) in [
            ("old-a", "2026-10-01T00:00:00Z"),
            ("old-b", "2026-10-02T00:00:00Z"),
            ("current", "2026-10-03T00:00:00Z"),
        ] {
            db.execute(
                "INSERT INTO tasks VALUES(?1,?2,?3)",
                params![id, serde_json::to_string(&turn(id, at)).unwrap(), at],
            )
            .unwrap();
        }
        if let Some(at) = boundary {
            db.execute(
                "INSERT INTO preferences VALUES('conversation-start',?1)",
                [at],
            )
            .unwrap();
        }
    }
    #[test]
    fn legacy_migration_uses_only_known_boundary_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        legacy_db(&path, Some("2026-10-02T00:00:00Z"));
        let store = Store::open(&path).unwrap();
        let old = store.task_conversation("old-a").unwrap();
        assert_eq!(old, store.task_conversation("old-b").unwrap());
        assert!(store
            .conversation_summary(&old)
            .unwrap()
            .title
            .contains("legacy"));
        let active = store.active_conversation_id().unwrap();
        assert_ne!(old, active);
        assert_eq!(active, store.task_conversation("current").unwrap());
        assert_eq!(store.conversation_list(None, 100).unwrap().items.len(), 2);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(active, store.active_conversation_id().unwrap());
        assert_eq!(old, store.task_conversation("old-a").unwrap());
        assert_eq!(store.conversation_list(None, 100).unwrap().items.len(), 2);
    }
    #[test]
    fn absent_legacy_boundary_does_not_invent_past_threads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        legacy_db(&path, None);
        let store = Store::open(&path).unwrap();
        let list = store.conversation_list(None, 100).unwrap();
        assert_eq!(list.items.len(), 1);
        assert!(list.items[0].legacy);
        assert_eq!(list.items[0].turn_count, 3);
    }
    #[test]
    fn paginates_more_than_100_turns_with_identical_timestamps_without_leaks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("threads.db")).unwrap();
        let id = store.active_conversation_id().unwrap();
        for n in 0..235 {
            store
                .save_task(&turn(&format!("turn-{n:03}"), "2026-10-01T00:00:00Z"))
                .unwrap();
        }
        let other = store.new_conversation().unwrap().conversation.id;
        store
            .save_task(&turn("unrelated", "2026-10-01T00:00:00Z"))
            .unwrap();
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page = store
                .conversation_page(&id, cursor.as_deref(), 100)
                .unwrap();
            ids.extend(page.turns.iter().map(|t| t.id.clone()));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 235);
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 235);
        assert_eq!(ids.first().unwrap(), "turn-234");
        assert_eq!(ids.last().unwrap(), "turn-000");
        assert!(store
            .conversation_page(&id, Some("unrelated"), 100)
            .is_err());
        assert_eq!(
            store
                .conversation_page(&other, None, 100)
                .unwrap()
                .turns
                .len(),
            1
        );
    }
    #[test]
    fn reopening_and_continuing_keeps_history_and_checkpoint_receipts_in_original_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        let store = Store::open(&path).unwrap();
        let first = store.active_conversation_id().unwrap();
        store
            .save_task(&turn("context", "2026-10-01T00:00:00Z"))
            .unwrap();
        let mut task = turn("workflow", "2026-10-02T00:00:00Z");
        task.state = "running".into();
        let settings = Settings::default();
        let mut flow = Workflow::new(&task, "auto", &settings, vec![], None, &["create_note"]);
        let approval = flow
            .begin(
                "create_note",
                json!({"title":"Trip","body":"Pack"}),
                json!({}),
                true,
                true,
            )
            .unwrap();
        flow.approve(&approval, json!({"title":"Trip","body":"Pack"}))
            .unwrap();
        flow.finish_step(
            "succeeded",
            json!({"objectId":"synthetic-note","verified":true}),
        )
        .unwrap();
        store.checkpoint(&mut flow, &task).unwrap(); // No prior save_task: trigger must assign it.
        let second = store.new_conversation().unwrap().conversation.id;
        store
            .save_task(&turn("unrelated", "2026-10-03T00:00:00Z"))
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap(); // Recovery checkpoints while another thread is active.
        assert_eq!(store.task("workflow").unwrap().unwrap().state, "paused");
        assert_eq!(store.task_conversation("workflow").unwrap(), first);
        assert_eq!(store.active_conversation_id().unwrap(), second);
        let history = store.conversation_history_for_task("workflow").unwrap();
        let messages =
            crate::conversation::history(&history, None, 2000, Locality::OnDevice, &settings);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "Question context");
        let mut resumed = store.workflow(&approval).unwrap().unwrap();
        task.state = "completed".into();
        resumed.state = "completed".into();
        store.checkpoint(&mut resumed, &task).unwrap();
        assert_eq!(store.task_conversation("workflow").unwrap(), first);
        assert_eq!(
            store.workflow(&approval).unwrap().unwrap().steps[0]
                .evidence
                .as_ref()
                .unwrap()["objectId"],
            "synthetic-note"
        );
        store.open_conversation(&first).unwrap();
        store
            .save_task(&turn("followup", "2026-10-04T00:00:00Z"))
            .unwrap();
        assert_eq!(store.task_conversation("followup").unwrap(), first);
        assert_eq!(
            store
                .conversation_history_for_task("followup")
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn conversation_list_pagination_and_export_clear_cover_all_threads() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("threads.db")).unwrap();
        for n in 0..105 {
            store.new_conversation().unwrap();
            store
                .save_task(&turn(&format!("task-{n}"), "2026-10-01T00:00:00Z"))
                .unwrap();
        }
        let mut ids = std::collections::HashSet::new();
        let mut cursor = None;
        loop {
            let page = store.conversation_list(cursor.as_deref(), 30).unwrap();
            for item in page.items {
                assert!(ids.insert(item.id));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 105);
        let export = store.export_snapshot().unwrap();
        assert_eq!(
            export["conversationMembership"].as_array().unwrap().len(),
            105
        );
        store.clear_history().unwrap();
        assert!(store.tasks().unwrap().is_empty());
        assert!(store.workflows().unwrap().is_empty());
        let cleared = store.conversation_list(None, 100).unwrap();
        assert_eq!(cleared.items.len(), 1);
        assert_eq!(cleared.items[0].turn_count, 0);
        assert!(store.export_snapshot().unwrap()["conversationMembership"]
            .as_array()
            .unwrap()
            .is_empty());
        store
            .save_task(&turn("fresh", "2026-10-05T00:00:00Z"))
            .unwrap();
        assert_eq!(
            store.task_conversation("fresh").unwrap(),
            cleared.items[0].id
        );
        store.erase_everything().unwrap();
        assert!(store.export_snapshot().unwrap()["conversations"]
            .as_array()
            .unwrap()
            .is_empty());
        store
            .save_task(&turn("after-erase", "2026-10-05T00:00:00Z"))
            .unwrap();
        let fresh_id = store.task_conversation("after-erase").unwrap();
        drop(store);
        let store = Store::open(&dir.path().join("threads.db")).unwrap();
        assert_eq!(store.active_conversation_id().unwrap(), fresh_id);
        assert_eq!(store.task_conversation("after-erase").unwrap(), fresh_id);
        assert!(!store.conversation_summary(&fresh_id).unwrap().legacy);
    }

    #[test]
    fn history_and_data_deletion_roll_back_membership_on_storage_failure() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("threads.db")).unwrap();
        store
            .save_task(&turn("keep", "2026-10-05T00:00:00Z"))
            .unwrap();
        let original = store.task_conversation("keep").unwrap();
        store.db.execute_batch("CREATE TRIGGER fail_thread_delete BEFORE DELETE ON chat_threads BEGIN SELECT RAISE(ABORT, 'storage failure fixture'); END;").unwrap();
        assert!(store.clear_history().is_err());
        assert!(store.db.is_autocommit());
        assert_eq!(store.task_conversation("keep").unwrap(), original);
        assert!(store.erase_everything().is_err());
        assert!(store.db.is_autocommit());
        assert_eq!(store.task_conversation("keep").unwrap(), original);
        assert_eq!(store.active_conversation_id().unwrap(), original);
        store
            .db
            .execute_batch("DROP TRIGGER fail_thread_delete;")
            .unwrap();
        store.clear_history().unwrap();
        assert!(store.tasks().unwrap().is_empty());
    }
}
