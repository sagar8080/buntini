use crate::{controller::AppState, types::*};
use tauri::State;

#[tauri::command]
pub async fn list_voices() -> Result<Vec<VoiceChoice>, String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("/usr/bin/say")
            .args(["-v", "?"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Voice discovery timed out.".to_string())?
    .map_err(|_| "macOS voices could not be listed.".to_string())?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err("macOS voices could not be listed.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let left = line.split('#').next()?.trim();
            let split = left.rfind(char::is_whitespace)?;
            let name = left[..split].trim();
            let language = left[split..].trim();
            if name.is_empty() || language.is_empty() {
                return None;
            }
            Some(VoiceChoice {
                id: name.into(),
                name: name.into(),
                language: language.into(),
            })
        })
        .take(300)
        .collect())
}
pub async fn valid_voice(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Ok(());
    }
    if list_voices().await?.iter().any(|voice| voice.id == id) {
        Ok(())
    } else {
        Err("Choose a voice installed on this Mac.".into())
    }
}
#[tauri::command]
pub async fn preview_voice(
    voice_id: String,
    rate: u32,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let epoch = state.voice.reserve_speech();
    if !(100..=300).contains(&rate) || text.chars().count() > 500 {
        return Err("Voice previews allow up to 500 characters and a rate of 100–300.".into());
    }
    valid_voice(&voice_id).await?;
    state
        .voice
        .speak_with_voice(text, &voice_id, rate, epoch)
        .await
}
#[tauri::command]
pub async fn worker_diagnostics(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.store.lock().unwrap().settings()?;
    let response=state.worker.request(&settings,serde_json::json!({"type":"ping","model_path":settings.model_path,"whisper_model_path":settings.whisper_model,"vad_model_path":settings.vad_model_path}),std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),|_|{}).await?;
    serde_json::from_str(&response.text)
        .map_err(|_| "The local worker returned invalid diagnostics.".into())
}
#[tauri::command]
pub async fn check_os_updates(
    approved: bool,
    state: State<'_, AppState>,
) -> Result<OsUpdateCheck, String> {
    if !approved {
        return Err(
            "Checking OS updates contacts Apple's update servers. Explicit approval is required."
                .into(),
        );
    }
    state.audit(
        "os_updates.approved",
        "User explicitly approved a read-only check with Apple's update servers",
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("/usr/sbin/softwareupdate")
            .arg("--list")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        "The OS update check timed out after 60 seconds. No updates were installed.".to_string()
    })?
    .map_err(|_| "The macOS update service could not be started.".to_string())?;
    if output.stdout.len() > 65536 || output.stderr.len() > 65536 {
        return Err("The update service returned an oversized response.".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let errors = String::from_utf8_lossy(&output.stderr);
    let updates = text
        .lines()
        .filter_map(|line| {
            line.trim().strip_prefix("* ").map(|item| {
                item.strip_prefix("Label: ")
                    .unwrap_or(item)
                    .chars()
                    .take(300)
                    .collect::<String>()
            })
        })
        .take(30)
        .collect::<Vec<_>>();
    let (result, summary) = if !output.status.success() {
        (
            "error",
            "Apple's update service could not complete the check. No changes were made.".into(),
        )
    } else if !updates.is_empty() {
        ("available",format!("{} update(s) are available. Open System Settings → General → Software Update to review and install them.",updates.len()))
    } else if text.contains("No new software available")
        || errors.contains("No new software available")
    {
        (
            "upToDate",
            "Apple's update service reported no new software at the time of this check.".into(),
        )
    } else {
        ("unknown","The update service returned an unrecognized result. Review Software Update in System Settings; no up-to-date claim can be made.".into())
    };
    let check = OsUpdateCheck {
        state: result.into(),
        checked_at: now(),
        summary,
        updates,
    };
    state.store.lock().unwrap().save_update_check(&check)?;
    state.audit("os_updates.checked", &check.state);
    Ok(check)
}
