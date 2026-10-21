//! One-time import of local data written under the app's former Nexon identity.
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub const DATABASE: &str = "buntini.sqlite3";
const LEGACY_ID: &str = "app.nexon.companion";
const LEGACY_DATABASE: &str = "nexon.sqlite3";
const MARKER: &str = "migrated-to-buntini";

/// Copies the former database into `data` before the store first opens it. The
/// legacy folder is never modified apart from a marker, so it stays recoverable.
/// Returns `None` when there is nothing to import, otherwise an audit detail.
pub fn import_legacy(data: &Path, cache: &Path) -> Option<Result<String, String>> {
    remove_legacy_recordings(cache);
    let legacy = data.parent()?.join(LEGACY_ID);
    let source = legacy.join(LEGACY_DATABASE);
    let target = data.join(DATABASE);
    if legacy == data || target.exists() || legacy.join(MARKER).exists() || !source.is_file() {
        return None;
    }
    Some(copy_database(&source, &target).map(|()| {
        let _ = std::fs::write(
            legacy.join(MARKER),
            format!("Imported into {}\n", target.display()),
        );
        format!("History and preferences imported from {}", source.display())
    }))
}

fn copy_database(source: &Path, target: &Path) -> Result<(), String> {
    let partial = target.with_extension("sqlite3.partial");
    let _ = std::fs::remove_file(&partial);
    let copied = (|| {
        // VACUUM INTO reads a consistent snapshot, including changes still in the WAL.
        let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        let path = partial
            .to_str()
            .ok_or("The data folder path is not valid UTF-8.")?;
        db.execute("VACUUM INTO ?1", [path])
            .map_err(|e| e.to_string())?;
        drop(db);
        // Only the former default name follows the rename; a chosen name is kept.
        Connection::open(&partial)
            .and_then(|copy| {
                copy.execute(
                    "UPDATE preferences SET value=json_set(value,'$.assistantName','Buntini')
                     WHERE key='settings' AND json_extract(value,'$.assistantName')='Nexon'",
                    [],
                )
            })
            .map_err(|e| e.to_string())?;
        std::fs::rename(&partial, target).map_err(|e| e.to_string())
    })();
    if copied.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    copied
}

/// Temporary recordings are private audio; never leave them behind in the old cache.
fn remove_legacy_recordings(cache: &Path) {
    let Some(legacy) = cache.parent().map(|parent| parent.join(LEGACY_ID)) else {
        return;
    };
    if legacy == cache {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(legacy) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("nexon-recording-") && name.ends_with(".wav") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::Store, types::*};

    fn legacy_store(root: &Path, assistant_name: &str) -> (Store, Connection) {
        let legacy = root.join(LEGACY_ID);
        std::fs::create_dir_all(&legacy).unwrap();
        let store = Store::open(&legacy.join(LEGACY_DATABASE)).unwrap();
        store
            .save_task(&TaskRecord {
                id: "legacy-task".into(),
                prompt: "before the rename".into(),
                answer: "kept".into(),
                state: "completed".into(),
                created_at: now(),
                sources: vec![],
                files: vec![],
                system: None,
                research: vec![],
                prices: vec![],
                cards: vec![],
            })
            .unwrap();
        let mut settings = serde_json::to_value(Settings::default()).unwrap();
        settings["assistantName"] = serde_json::json!(assistant_name);
        let raw = Connection::open(legacy.join(LEGACY_DATABASE)).unwrap();
        raw.execute(
            "INSERT OR REPLACE INTO preferences VALUES('settings',?1)",
            [settings.to_string()],
        )
        .unwrap();
        (store, raw)
    }

    #[test]
    fn imports_uncheckpointed_wal_changes_and_renames_the_default_name() {
        let dir = tempfile::tempdir().unwrap();
        // Both connections stay open, so the writes remain in the WAL like a running app.
        let (_store, _raw) = legacy_store(dir.path(), "Nexon");
        let wal = dir.path().join(LEGACY_ID).join("nexon.sqlite3-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);
        let data = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&data).unwrap();
        let detail = import_legacy(&data, &dir.path().join("cache"))
            .unwrap()
            .unwrap();
        assert!(detail.contains(LEGACY_DATABASE));
        let imported = Store::open(&data.join(DATABASE)).unwrap();
        assert_eq!(
            imported.task("legacy-task").unwrap().unwrap().answer,
            "kept"
        );
        assert_eq!(imported.settings().unwrap().assistant_name, "Buntini");
        assert!(dir.path().join(LEGACY_ID).join(MARKER).exists());
        assert!(dir.path().join(LEGACY_ID).join(LEGACY_DATABASE).exists());
        assert!(!data.join("buntini.sqlite3.partial").exists());
    }

    #[test]
    fn keeps_a_custom_assistant_name() {
        let dir = tempfile::tempdir().unwrap();
        drop(legacy_store(dir.path(), "Pip"));
        let data = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&data).unwrap();
        import_legacy(&data, &dir.path().join("cache"))
            .unwrap()
            .unwrap();
        let imported = Store::open(&data.join(DATABASE)).unwrap();
        assert_eq!(imported.settings().unwrap().assistant_name, "Pip");
    }

    #[test]
    fn runs_only_once() {
        let dir = tempfile::tempdir().unwrap();
        drop(legacy_store(dir.path(), "Nexon"));
        let data = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&data).unwrap();
        let cache = dir.path().join("cache");
        assert!(import_legacy(&data, &cache).is_some());
        assert!(import_legacy(&data, &cache).is_none());
        // A user who resets Buntini later does not get the old data re-imported.
        std::fs::remove_file(data.join(DATABASE)).unwrap();
        assert!(import_legacy(&data, &cache).is_none());
        assert!(!data.join(DATABASE).exists());
    }

    #[test]
    fn existing_buntini_data_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        drop(legacy_store(dir.path(), "Nexon"));
        let data = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&data).unwrap();
        drop(Store::open(&data.join(DATABASE)).unwrap());
        assert!(import_legacy(&data, &dir.path().join("cache")).is_none());
        let current = Store::open(&data.join(DATABASE)).unwrap();
        assert!(current.task("legacy-task").unwrap().is_none());
        assert!(!dir.path().join(LEGACY_ID).join(MARKER).exists());
    }

    #[test]
    fn missing_or_unreadable_legacy_data_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&data).unwrap();
        assert!(import_legacy(&data, &dir.path().join("cache")).is_none());
        let legacy = dir.path().join(LEGACY_ID);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(LEGACY_DATABASE), "not a database").unwrap();
        assert!(import_legacy(&data, &dir.path().join("cache"))
            .unwrap()
            .is_err());
        assert!(!data.join(DATABASE).exists());
        assert!(!data.join("buntini.sqlite3.partial").exists());
        assert!(!legacy.join(MARKER).exists());
    }

    #[test]
    fn removes_only_legacy_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_cache = dir.path().join(LEGACY_ID);
        std::fs::create_dir_all(&legacy_cache).unwrap();
        std::fs::write(legacy_cache.join("nexon-recording-1.wav"), "audio").unwrap();
        std::fs::write(legacy_cache.join("window-diagnostics.json"), "{}").unwrap();
        let cache = dir.path().join("app.buntini.companion");
        std::fs::create_dir_all(&cache).unwrap();
        remove_legacy_recordings(&cache);
        assert!(!legacy_cache.join("nexon-recording-1.wav").exists());
        assert!(legacy_cache.join("window-diagnostics.json").exists());
    }
}
