//! Your data: export it, erase it, and a support summary you can copy. Crashes are
//! written to a local log so the support summary can include them; nothing is sent.
use crate::{controller::AppState, store::Store};
use std::{io::Write, path::{Path, PathBuf}};
use tauri::State;

const CRASH_LOG_LIMIT: u64 = 256 * 1024;

/// Records panics to `logs/crash.log` (time, message and place only), then runs the
/// default hook. The log is trimmed once it passes 256 KB.
pub fn install_crash_log(data: &Path) {
    let path = data.join("logs").join("crash.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
        let _ = append_crash(&path, &format!("{} {} at {}\n", crate::types::now(), message.chars().take(500).collect::<String>(), place));
        previous(info);
    }));
}

fn append_crash(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).map(|m| m.len() > CRASH_LOG_LIMIT).unwrap_or(false) {
        std::fs::remove_file(path)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(line.as_bytes())
}

fn crash_tail(path: &Path, lines: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].iter().map(|line| line.to_string()).collect()
}

/// Replaces the home folder with ~ so shared summaries don't carry the account name.
fn tidy(text: &str, home: &str) -> String {
    if home.is_empty() { text.to_string() } else { text.replace(home, "~") }
}

/// A plain-text summary for bug reports: versions, which brain is set up and recent
/// errors. No messages, file contents or keys.
pub fn support_summary(store: &Store, data: &Path, models: &crate::model::ModelHub) -> Result<String, String> {
    let settings = store.settings()?;
    let os = std::process::Command::new("sw_vers").arg("-productVersion").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
    let mut lines = vec![
        format!("Buntini {}", env!("CARGO_PKG_VERSION")),
        format!("macOS {} · {}", os.trim(), std::env::consts::ARCH),
        format!("Brain: {} · {}", settings.model_backend, models.status(&settings)),
        format!("Apple Intelligence: {}", models.apple_status().map(|(_, reason)| reason).unwrap_or_else(|| "checking".into())),
        format!("Folders allowed: {}", store.roots()?.len()),
        format!("Online search: {} · free tier {}", settings.search_provider, if settings.free_tier_only { "on" } else { "off" }),
        String::new(),
        "Recent activity:".into(),
    ];
    lines.extend(store.audit_entries()?.into_iter().take(15).map(|entry| format!("  {} {} {}", entry.created_at, entry.action, entry.detail.chars().take(120).collect::<String>())));
    let crashes = crash_tail(&data.join("logs").join("crash.log"), 10);
    lines.push(String::new());
    lines.push(if crashes.is_empty() { "No crashes recorded.".into() } else { "Recent crashes:".into() });
    lines.extend(crashes.into_iter().map(|line| format!("  {line}")));
    Ok(tidy(&lines.join("\n"), &std::env::var("HOME").unwrap_or_default()))
}

#[tauri::command]
pub async fn export_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let snapshot = state.store.lock().unwrap().export_snapshot()?;
    let name = format!("Buntini export {}.json", chrono::Local::now().format("%Y-%m-%d"));
    let Some(file) = rfd::AsyncFileDialog::new().set_title("Export your Buntini data").set_file_name(&name).save_file().await else {
        return Ok(None);
    };
    let path: PathBuf = file.path().to_path_buf();
    let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(&path).map_err(|_| "Couldn’t write the export file.".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = out.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    out.write_all(text.as_bytes()).map_err(|_| "Couldn’t write the export file.".to_string())?;
    state.audit("data.exported", "Conversation, settings and lists exported");
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Erases conversations, settings, lists, the file index, usage, saved keys and
/// recordings. Buntini then starts over with setup.
#[tauri::command]
pub async fn delete_all_data(confirm: String, state: State<'_, AppState>) -> Result<(), String> {
    if confirm != "delete everything" {
        return Err("Confirm by choosing Delete everything.".into());
    }
    if !state.active.lock().unwrap().is_empty() {
        return Err("Stop the current task first.".into());
    }
    state.store.lock().unwrap().erase_everything()?;
    for provider in ["brave", "tavily", "ebay", "duffel"].into_iter().chain(crate::cloud::PROVIDERS.iter().map(|provider| provider.id)) {
        let _ = crate::web::set_credential(provider, "");
    }
    state.voice.clear_cache();
    state.models.reset().await;
    Ok(())
}

#[tauri::command]
pub fn support_info(state: State<'_, AppState>) -> Result<String, String> {
    support_summary(&state.store.lock().unwrap(), &state.data_dir, &state.models)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n", "an oversized log starts over");
    }

    #[test]
    fn summaries_hide_the_home_folder() {
        assert_eq!(tidy("folder.allowed /Users/someone/Documents", "/Users/someone"), "folder.allowed ~/Documents");
        assert_eq!(tidy("unchanged", ""), "unchanged");
    }
}
