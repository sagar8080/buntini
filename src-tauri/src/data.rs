//! Your data: export it, erase it, and a support summary you can copy. Crashes are
//! written to a local log; support summaries include only a count, and nothing is sent.
use crate::{controller::AppState, store::Store};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tauri::State;

const CRASH_LOG_LIMIT: u64 = 256 * 1024;

/// Records panics to `logs/crash.log` (time and source location only), then runs the
/// default hook. The log is trimmed once it passes 256 KB.
pub fn install_crash_log(data: &Path) {
    let path = data.join("logs").join("crash.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        // Panic payloads can contain private file contents, request bodies or
        // credentials. Keep only the code location in the persistent log.
        let _ = append_crash(
            &path,
            &format!("{} panic at {}\n", crate::types::now(), place),
        );
        previous(info);
    }));
}

fn private_output(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).append(append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Apply the creation mode before any data is visible, and refuse
        // symlinks so an export/log cannot overwrite a different file.
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    if !append {
        file.set_len(0)?;
    }
    Ok(file)
}

fn append_crash(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    if std::fs::metadata(path)
        .map(|m| m.len() > CRASH_LOG_LIMIT)
        .unwrap_or(false)
    {
        std::fs::remove_file(path)?;
    }
    private_output(path, true)?.write_all(line.as_bytes())
}

fn crash_tail(path: &Path, lines: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| line.to_string())
        .collect()
}

/// Replaces the home folder with ~ so shared summaries don't carry the account name.
fn tidy(text: &str, home: &str) -> String {
    if home.is_empty() {
        text.to_string()
    } else {
        text.replace(home, "~")
    }
}

/// A plain-text summary for bug reports: versions, which brain is set up and recent
/// errors. No messages, file contents or keys.
pub fn support_summary(
    store: &Store,
    data: &Path,
    models: &crate::model::ModelHub,
) -> Result<String, String> {
    let settings = store.settings()?;
    let os = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let mut lines = vec![
        format!("Buntini {}", env!("CARGO_PKG_VERSION")),
        format!("macOS {} · {}", os.trim(), std::env::consts::ARCH),
        format!(
            "Brain: {} · {}",
            settings.model_backend,
            models.status(&settings)
        ),
        format!(
            "Apple Intelligence: {}",
            models
                .apple_status()
                .map(|(_, reason)| reason)
                .unwrap_or_else(|| "checking".into())
        ),
        format!("Folders allowed: {}", store.roots()?.len()),
        format!(
            "Memory index: {}",
            match crate::memory_index::counts(&store.db)? {
                counts if counts.is_empty() => "empty".to_string(),
                counts => counts
                    .iter()
                    .map(|(source, count)| format!("{source} {count}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            }
        ),
        format!(
            "Online search: {} · free tier {}",
            settings.search_provider,
            if settings.free_tier_only { "on" } else { "off" }
        ),
        String::new(),
        "Recent activity:".into(),
    ];
    // Event details include user-chosen URLs, folders and error messages. A
    // shareable support summary must not copy that free-form private data.
    lines.extend(
        store
            .audit_entries()?
            .into_iter()
            .take(15)
            .map(|entry| format!("  {} {}", entry.created_at, entry.action)),
    );
    let crashes = crash_tail(&data.join("logs").join("crash.log"), 10);
    lines.push(String::new());
    lines.push(if crashes.is_empty() {
        "No crashes recorded.".into()
    } else {
        format!("Recent crashes recorded: {}.", crashes.len())
    });
    Ok(tidy(
        &lines.join("\n"),
        &std::env::var("HOME").unwrap_or_default(),
    ))
}

#[tauri::command]
pub async fn export_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let name = format!(
        "Buntini export {}.json",
        chrono::Local::now().format("%Y-%m-%d")
    );
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export your Buntini data")
        .set_file_name(&name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path: PathBuf = file.path().to_path_buf();
    write_export(&state, &path).await?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

async fn write_export(state: &AppState, path: &Path) -> Result<(), String> {
    // The save dialog may stay open while erasure runs. Read a fresh snapshot
    // only after choosing the destination; never write a pre-erasure snapshot.
    let _gate = state.config_gate.lock().await;
    let snapshot = state.store.lock().unwrap().export_snapshot()?;
    let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
    let mut out =
        private_output(path, false).map_err(|_| "Couldn’t write the export file.".to_string())?;
    out.write_all(text.as_bytes())
        .map_err(|_| "Couldn’t write the export file.".to_string())?;
    state.audit("data.exported", "Conversation, settings and lists exported");
    Ok(())
}

/// Erases conversations, settings, lists, the file index, usage, saved keys and
/// recordings. Buntini then starts over with setup.
#[tauri::command]
pub async fn delete_all_data(confirm: String, state: State<'_, AppState>) -> Result<(), String> {
    erase_with_services(
        &confirm,
        &state,
        crate::chatgpt::sign_out,
        crate::browser_workspace::clear_profiles,
        |provider| crate::web::set_credential(provider, ""),
    )
    .await
}

/// The production erasure flow, with only OS/account services replaceable by
/// disposable fixtures. Store, voice, locks, approvals and files stay real.
async fn erase_with_services<SignOut, SignOutFuture, Profiles, ProfilesFuture, Credential>(
    confirm: &str,
    state: &AppState,
    sign_out: SignOut,
    clear_profiles: Profiles,
    mut clear_credential: Credential,
) -> Result<(), String>
where
    SignOut: FnOnce() -> SignOutFuture,
    SignOutFuture: std::future::Future<Output = Result<bool, String>>,
    Profiles: FnOnce() -> ProfilesFuture,
    ProfilesFuture: std::future::Future<Output = Result<(), String>>,
    Credential: FnMut(&str) -> Result<(), String>,
{
    if confirm != "delete everything" {
        return Err("Confirm by choosing Delete everything.".into());
    }
    let _gate = state.config_gate.lock().await;
    let reservation = format!("erase-{}", uuid::Uuid::new_v4());
    {
        let mut active = state.active.lock().unwrap();
        if !active.is_empty() {
            return Err("Stop the current task first.".into());
        }
        active.insert(
            reservation.clone(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
    }
    let _claim = crate::controller::ActionClaim {
        state,
        id: reservation,
    };
    let mut failures = Vec::new();
    state.approvals.lock().unwrap().clear();
    state.pending_actions.lock().unwrap().clear();
    // Tools the user added stop before their settings and values are erased.
    crate::mcp::stop_servers(&state.store.lock().unwrap());
    if let Err(error) = state.voice.cancel().await {
        failures.push(error);
    }
    if let Err(error) = state.voice.stop_speaking().await {
        failures.push(error);
    }
    if let Err(error) = sign_out().await {
        failures.push(error);
    }
    if let Err(error) = clear_profiles().await {
        failures.push(error);
    }
    // A memory refresh under way stops; its writes would be refused anyway.
    state
        .index_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    if let Err(error) = state.store.lock().unwrap().erase_everything() {
        failures.push(error);
    }
    // Saves that waited behind the erase carry the old settings; they are refused.
    state
        .erasures
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    crate::delivery::erase_os_requests();
    // Settings go back to Open at login off, so macOS stops opening Buntini at login too.
    if crate::login::status().registered() {
        if let Err(error) = crate::login::apply(false) {
            failures.push(error);
        }
    }
    for provider in ["brave", "tavily", "ebay", "duffel"]
        .into_iter()
        .chain(crate::cloud::PROVIDERS.iter().map(|provider| provider.id))
    {
        if let Err(error) = clear_credential(provider) {
            failures.push(error);
        }
    }
    if let Err(error) = state.voice.clear_cache() {
        failures.push(error);
    }
    state.models.reset().await;
    if let Err(error) = std::fs::remove_dir_all(state.data_dir.join("logs")) {
        if error.kind() != std::io::ErrorKind::NotFound {
            failures.push("Couldn’t remove the local crash logs.".into());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Some data could not be removed. {}",
            failures.join(" ")
        ))
    }
}

#[tauri::command]
pub fn support_info(state: State<'_, AppState>) -> Result<String, String> {
    support_summary(&state.store.lock().unwrap(), &state.data_dir, &state.models)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path().join("data"),
            dir.path().join("cache"),
            dir.path().join("resources"),
        )
        .unwrap();
        (dir, state)
    }

    fn seed_private_state(state: &AppState) -> PathBuf {
        let store = state.store.lock().unwrap();
        store.remember("private-erasure-sentinel").unwrap();
        store.add_root("/disposable/fixture").unwrap();
        store.set_onboarded(true).unwrap();
        drop(store);
        let approval = crate::types::PendingApproval {
            id: "fixture-approval".into(),
            prompt: "private-erasure-sentinel".into(),
            provider: "brave".into(),
            expires_at: crate::types::now(),
            approved: false,
            tool: "web_search".into(),
            arguments: serde_json::json!({"query":"private-erasure-sentinel"}),
            service: "fixture".into(),
            summary: "fixture".into(),
            sends: vec![],
            host: "example.invalid".into(),
            uses_key: false,
            note: None,
        };
        state
            .approvals
            .lock()
            .unwrap()
            .insert(approval.id.clone(), approval);
        append_crash(
            &state.data_dir.join("logs/crash.log"),
            "private-erasure-sentinel\n",
        )
        .unwrap();
        let path = state
            .data_dir
            .parent()
            .unwrap()
            .join("cache/buntini-recording-fixture.wav");
        std::fs::write(&path, b"private-erasure-sentinel").unwrap();
        std::fs::write(path.with_file_name("unrelated-file.txt"), b"keep").unwrap();
        path
    }

    #[tokio::test]
    async fn full_erasure_clears_disposable_state_and_attempts_every_credential() {
        let (_dir, state) = fixture_state();
        let recording = seed_private_state(&state);
        let calls = std::cell::RefCell::new(Vec::new());
        erase_with_services(
            "delete everything",
            &state,
            || {
                calls.borrow_mut().push("sign_out".to_string());
                std::future::ready(Ok(true))
            },
            || {
                calls.borrow_mut().push("profiles".to_string());
                std::future::ready(Ok(()))
            },
            |provider| {
                assert!(
                    state.config_gate.try_lock().is_err(),
                    "erasure retains its lifecycle gate"
                );
                assert_eq!(
                    state.active.lock().unwrap().len(),
                    1,
                    "erasure retains its execution claim"
                );
                calls.borrow_mut().push(provider.to_string());
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                "sign_out",
                "profiles",
                "brave",
                "tavily",
                "ebay",
                "duffel",
                "anthropic",
                "openai",
                "gemini",
                "chatgpt"
            ]
        );
        assert!(state.store.lock().unwrap().memories().unwrap().is_empty());
        assert!(state.store.lock().unwrap().roots().unwrap().is_empty());
        assert!(!state.store.lock().unwrap().onboarded().unwrap());
        assert!(state.approvals.lock().unwrap().is_empty());
        assert!(state.active.lock().unwrap().is_empty());
        assert!(state.config_gate.try_lock().is_ok());
        assert!(!recording.exists());
        assert!(recording.with_file_name("unrelated-file.txt").exists());
        assert!(!state.data_dir.join("logs").exists());
    }

    #[tokio::test]
    async fn a_settings_save_queued_behind_the_erase_is_refused() {
        let (_dir, state) = fixture_state();
        let mut before = state.store.lock().unwrap().settings().unwrap();
        before.user_name = "Before the erase".into();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        // The erase holds the settings gate while it signs out; the save arrives meanwhile.
        let erase = erase_with_services(
            "delete everything",
            &state,
            || async {
                let _ = released.await;
                Ok(true)
            },
            || std::future::ready(Ok(())),
            |_| Ok(()),
        );
        let save = crate::controller::save_settings_in(&state, before);
        let finish = async {
            tokio::task::yield_now().await;
            let _ = release.send(());
        };
        let (erased, saved, ()) = tokio::join!(erase, save, finish);
        erased.unwrap();
        assert!(saved.unwrap_err().contains("erased"));
        assert_eq!(state.store.lock().unwrap().settings().unwrap().user_name, crate::types::Settings::default().user_name);
        // A save made after the erase goes through.
        let mut after = state.store.lock().unwrap().settings().unwrap();
        after.user_name = "After".into();
        assert!(crate::controller::save_settings_in(&state, after).await.is_ok());
    }

    #[tokio::test]
    async fn erase_clears_pending_os_requests() {
        let (_dir, state) = fixture_state();
        let timer = {
            let store = state.store.lock().unwrap();
            let mut settings = store.settings().unwrap();
            settings.notifications_enabled = true;
            store.save_settings(&settings).unwrap();
            crate::delivery::schedule(&store, 600, chrono::Utc::now()).unwrap()
        };
        let identifier = crate::delivery::os_identifier(&timer.id);
        assert!(crate::delivery::fake::pending().contains_key(&identifier));
        erase_with_services(
            "delete everything",
            &state,
            || std::future::ready(Ok(true)),
            || std::future::ready(Ok(())),
            |_| Ok(()),
        )
        .await
        .unwrap();
        assert!(
            crate::delivery::fake::pending().is_empty(),
            "an erased timer must not alert from macOS later"
        );
        assert!(crate::delivery::timers(&state.store.lock().unwrap())
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn erasure_reports_service_and_cache_failures_after_finishing_other_cleanup() {
        let (_dir, state) = fixture_state();
        let recording = seed_private_state(&state);
        let malformed_recording = recording.with_file_name("buntini-recording-unremovable.wav");
        std::fs::create_dir(&malformed_recording).unwrap();
        let calls = std::cell::RefCell::new(Vec::new());
        let error = erase_with_services(
            "delete everything",
            &state,
            || std::future::ready(Err("fixture sign-out failure".into())),
            || std::future::ready(Err("fixture profile failure".into())),
            |provider| {
                calls.borrow_mut().push(provider.to_string());
                if provider == "brave" {
                    Err("fixture Keychain failure".into())
                } else {
                    Ok(())
                }
            },
        )
        .await
        .unwrap_err();
        for expected in [
            "fixture sign-out failure",
            "fixture profile failure",
            "fixture Keychain failure",
            "temporary recordings",
        ] {
            assert!(error.contains(expected), "{error}");
        }
        assert_eq!(
            calls.borrow().len(),
            8,
            "a failed credential never skips the remaining services"
        );
        assert!(state.store.lock().unwrap().memories().unwrap().is_empty());
        assert!(state.approvals.lock().unwrap().is_empty());
        assert!(state.active.lock().unwrap().is_empty());
        assert!(state.config_gate.try_lock().is_ok());
        assert!(!recording.exists(), "other recordings are still removed");
        assert!(
            !state.data_dir.join("logs").exists(),
            "cleanup continues after cache failure"
        );
    }

    #[tokio::test]
    async fn rejected_erasure_never_calls_external_services_or_changes_data() {
        let (_dir, state) = fixture_state();
        let recording = seed_private_state(&state);
        for busy in [false, true] {
            if busy {
                state.active.lock().unwrap().insert(
                    "fixture-running".into(),
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                );
            }
            let error = erase_with_services(
                if busy {
                    "delete everything"
                } else {
                    "wrong confirmation"
                },
                &state,
                || -> std::future::Ready<Result<bool, String>> {
                    panic!("rejected erasure must not touch sign-in")
                },
                || -> std::future::Ready<Result<(), String>> {
                    panic!("rejected erasure must not touch profiles")
                },
                |_| panic!("rejected erasure must not touch Keychain"),
            )
            .await
            .unwrap_err();
            assert!(error.contains(if busy {
                "Stop the current task"
            } else {
                "Confirm"
            }));
            assert_eq!(state.store.lock().unwrap().memories().unwrap().len(), 1);
            assert!(recording.exists());
            assert_eq!(state.approvals.lock().unwrap().len(), 1);
        }
        assert!(state.active.lock().unwrap().contains_key("fixture-running"));
    }

    #[tokio::test]
    async fn delayed_export_reads_current_data_after_erasure() {
        let (dir, state) = fixture_state();
        seed_private_state(&state);
        let path = dir.path().join("export.json");
        write_export(&state, &path).await.unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("private-erasure-sentinel"));
        // Simulate a destination being selected only after Delete everything
        // completes, with the same store and production export writer.
        erase_with_services(
            "delete everything",
            &state,
            || std::future::ready(Ok(true)),
            || std::future::ready(Ok(())),
            |_| Ok(()),
        )
        .await
        .unwrap();
        write_export(&state, &path).await.unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(snapshot["memories"], serde_json::json!([]));
        assert!(!text.contains("private-erasure-sentinel"));
    }

    #[test]
    fn crash_log_keeps_recent_lines_and_stays_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("crash.log");
        for index in 0..20 {
            append_crash(&path, &format!("line {index}\n")).unwrap();
        }
        assert_eq!(crash_tail(&path, 3), ["line 17", "line 18", "line 19"]);
        std::fs::write(&path, vec![b'x'; CRASH_LOG_LIMIT as usize + 1]).unwrap();
        append_crash(&path, "fresh\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fresh\n",
            "an oversized log starts over"
        );
    }

    #[test]
    fn summaries_hide_the_home_folder() {
        assert_eq!(
            tidy("folder.allowed /Users/someone/Documents", "/Users/someone"),
            "folder.allowed ~/Documents"
        );
        assert_eq!(tidy("unchanged", ""), "unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn exports_are_private_and_refuse_symlink_targets() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.json");
        private_output(&path, false)
            .unwrap()
            .write_all(b"private")
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        private_output(&path, false)
            .unwrap()
            .write_all(b"new")
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(private_output(&link, false).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"new");
    }

    #[test]
    fn shared_support_summary_omits_free_form_audit_and_legacy_crash_contents() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Path::new(":memory:")).unwrap();
        store
            .audit(
                "research.public_html_read",
                "https://example.com/?private=private-sentinel",
            )
            .unwrap();
        append_crash(
            &dir.path().join("logs/crash.log"),
            "old panic private-sentinel\n",
        )
        .unwrap();
        let models = crate::model::ModelHub::new(PathBuf::new(), PathBuf::new());
        let summary = support_summary(&store, dir.path(), &models).unwrap();
        assert!(summary.contains("research.public_html_read"));
        assert!(summary.contains("Recent crashes recorded: 1."));
        assert!(!summary.contains("private-sentinel"));
        assert!(!summary.contains("example.com"));
    }
}
