use crate::{controller::AppState, types::*};
use tauri::State;

/// The voices worth offering: natural ones that speak the Mac's language, its own
/// accent first. macOS also installs novelty voices (Bad News, Bells, Boing) and older
/// synthesised ones; a picker of 184 names is a chore, not a choice.
#[tauri::command]
pub async fn list_voices() -> Result<Vec<VoiceChoice>, String> {
    Ok(curated(installed_voices().await?, &mac_locale().await))
}

/// The Mac's locale, such as "en_US"; English (US) when it can't be read.
async fn mac_locale() -> String {
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLocale"]).kill_on_drop(true).output(),
    )
    .await;
    match read {
        Ok(Ok(output)) if output.status.success() => {
            let locale = String::from_utf8_lossy(&output.stdout).trim().split('@').next().unwrap_or_default().to_string();
            if locale.contains('_') { locale } else { "en_US".into() }
        }
        _ => "en_US".into(),
    }
}

/// Novelty and older synthesised voices, by their base name.
const NOT_OFFERED: [&str; 27] = [
    "Albert", "Bad News", "Bahh", "Bells", "Boing", "Bubbles", "Cellos", "Wobble", "Fred", "Good News", "Jester",
    "Junior", "Kathy", "Organ", "Superstar", "Ralph", "Trinoids", "Whisper", "Zarvox", "Eddy", "Flo", "Grandma",
    "Grandpa", "Reed", "Rocko", "Sandy", "Shelley",
];

/// What a region sounds like, for English voices; other languages show the region code.
fn accent(region: &str) -> String {
    match region {
        "US" => "American", "GB" => "British", "AU" => "Australian", "IE" => "Irish", "ZA" => "South African",
        "IN" => "Indian", "NZ" => "New Zealand", "CA" => "Canadian", "SC" => "Scottish",
        other => other,
    }
    .to_string()
}

/// Every voice in the Mac's own accent, but only one of each other accent: a dozen
/// Indian or Scottish voices crowd out the one that sounds like home. Premium and
/// Enhanced voices were downloaded on purpose, so they're always offered.
pub(crate) fn curated(voices: Vec<VoiceChoice>, locale: &str) -> Vec<VoiceChoice> {
    let (language, region) = locale.split_once('_').unwrap_or((locale, ""));
    let mut installed: Vec<(u8, bool, String, String, VoiceChoice)> = voices
        .into_iter()
        .filter_map(|voice| {
            let (spoken, from) = voice.language.split_once('_').unwrap_or((voice.language.as_str(), ""));
            let base = voice.name.split(" (").next().unwrap_or(&voice.name).to_string();
            if spoken != language || NOT_OFFERED.contains(&base.as_str()) {
                return None;
            }
            let quality = if voice.name.contains("(Premium)") { 2 } else if voice.name.contains("(Enhanced)") { 1 } else { 0 };
            Some((quality, from == region, base, from.to_string(), voice))
        })
        .collect();
    // Higher-quality downloads first, then the Mac's own accent, then by name.
    installed.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    let mut offered: Vec<(String, String)> = Vec::new();
    let mut choices = Vec::new();
    for (quality, local, base, from, voice) in installed {
        // A voice installed in two qualities, or listed twice for Siri, is offered once.
        let repeat = offered.iter().any(|(name, accent)| *name == base && *accent == from);
        let crowded = !local && quality == 0 && offered.iter().any(|(_, accent)| *accent == from);
        if repeat || crowded {
            continue;
        }
        offered.push((base.clone(), from.clone()));
        choices.push(VoiceChoice { name: base, language: accent(&from), ..voice });
    }
    choices.truncate(10);
    choices
}

/// Every voice installed on this Mac, as `say` lists them.
pub async fn installed_voices() -> Result<Vec<VoiceChoice>, String> {
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
    if installed_voices().await?.iter().any(|voice| voice.id == id) {
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
    if !(100..=300).contains(&rate) || text.chars().count() > 500 {
        return Err("Voice previews allow up to 500 characters and a rate of 100–300.".into());
    }
    let epoch = {
        let _gate = state.config_gate.lock().await;
        state.voice.reserve_speech()
    };
    valid_voice(&voice_id).await?;
    state
        .voice
        .speak_with_voice(text, &voice_id, rate, epoch)
        .await
}
#[tauri::command]
pub async fn worker_diagnostics(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.store.lock().unwrap().settings()?;
    let response=state.models.mlx_request(&settings,serde_json::json!({"type":"ping","model_path":settings.model_path,"whisper_model_path":settings.whisper_model,"vad_model_path":settings.vad_model_path}),std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))).await?;
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

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn only_natural_voices_in_the_macs_language_are_offered() {
        let voice = |name: &str, language: &str| VoiceChoice { id: name.into(), name: name.into(), language: language.into() };
        let installed = vec![
            voice("Bad News", "en_US"), voice("Daniel", "en_GB"), voice("Samantha", "en_US"), voice("Shelley (English (US))", "en_US"),
            voice("Amélie", "fr_CA"), voice("Karen", "en_AU"), voice("Ava (Premium)", "en_US"),
        ];
        let offered: Vec<(String, String)> = curated(installed, "en_US").into_iter().map(|v| (v.name, v.language)).collect();
        assert_eq!(offered, [
            ("Ava".into(), "American".into()),
            ("Samantha".into(), "American".into()),
            ("Daniel".into(), "British".into()),
            ("Karen".into(), "Australian".into()),
        ]);
    }

    #[test]
    fn other_accents_get_one_voice_each_and_none_is_listed_twice() {
        let voice = |name: &str, language: &str| VoiceChoice { id: name.into(), name: name.into(), language: language.into() };
        let installed = vec![
            voice("Aman (English (India))", "en_IN"), voice("Aman (English (India))", "en_IN"), voice("Rishi", "en_IN"),
            voice("Tara (English (India))", "en_IN"), voice("Tara (English (India))", "en_IN"), voice("Samantha", "en_US"),
            voice("Daniel", "en_GB"), voice("Serena (Premium)", "en_GB"), voice("Kate (Enhanced)", "en_GB"),
            voice("Ava (Enhanced)", "en_US"), voice("Ava (Premium)", "en_US"), voice("Moira", "en_IE"),
        ];
        let offered: Vec<(String, String, String)> = curated(installed, "en_US").into_iter().map(|v| (v.id, v.name, v.language)).collect();
        assert_eq!(offered, [
            ("Ava (Premium)".into(), "Ava".into(), "American".into()),
            ("Serena (Premium)".into(), "Serena".into(), "British".into()),
            ("Kate (Enhanced)".into(), "Kate".into(), "British".into()),
            ("Samantha".into(), "Samantha".into(), "American".into()),
            ("Aman (English (India))".into(), "Aman".into(), "Indian".into()),
            ("Moira".into(), "Moira".into(), "Irish".into()),
        ]);
        // At home, every voice of the accent is offered.
        let home: Vec<String> = curated(vec![voice("Rishi", "en_IN"), voice("Aman (English (India))", "en_IN"), voice("Samantha", "en_US")], "en_IN")
            .into_iter().map(|v| v.name).collect();
        assert_eq!(home, ["Aman", "Rishi", "Samantha"]);
    }
}
