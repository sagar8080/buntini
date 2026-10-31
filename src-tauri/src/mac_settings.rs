//! Changes to this Mac's own settings: wallpaper, Dock, menu bar, appearance, Wi-Fi,
//! Finder and the screen saver. Each one reads its value before and after the change, so
//! the receipt says what is true now, not what was asked for. Security and privacy
//! settings (firewall, FileVault, Gatekeeper, passwords, app permissions) are never on
//! this list, whatever a request says.
use crate::actions::{ActionOutcome, ActionReceipt};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub const TOOL: &str = "change_mac_setting";

/// What a setting accepts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// "on" or "off".
    Toggle,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// Minutes, one of these (0 is never).
    Minutes(&'static [u64]),
    /// An image file: a built-in desktop picture or one in a shared folder.
    Picture,
}

pub struct Setting {
    pub id: &'static str,
    /// As the approval sheet names it: "Dock hiding".
    pub label: &'static str,
    pub kind: Kind,
}

pub const SETTINGS: &[Setting] = &[
    Setting { id: "wallpaper", label: "Wallpaper", kind: Kind::Picture },
    Setting { id: "appearance", label: "Appearance", kind: Kind::Choice(&["light", "dark"]) },
    Setting { id: "dock_autohide", label: "Hide the Dock", kind: Kind::Toggle },
    Setting { id: "dock_position", label: "Dock position", kind: Kind::Choice(&["left", "bottom", "right"]) },
    Setting { id: "dock_magnification", label: "Dock magnification", kind: Kind::Toggle },
    Setting { id: "menu_bar_autohide", label: "Hide the menu bar", kind: Kind::Toggle },
    Setting { id: "wifi", label: "Wi-Fi", kind: Kind::Toggle },
    Setting { id: "hidden_files", label: "Show hidden files", kind: Kind::Toggle },
    Setting { id: "desktop_icons", label: "Desktop icons", kind: Kind::Toggle },
    Setting { id: "screen_saver", label: "Screen saver after", kind: Kind::Minutes(&[0, 1, 2, 5, 10, 20, 30, 60]) },
];

pub fn setting(id: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.id == id)
}

/// Where built-in desktop pictures live; a wallpaper may also come from a shared folder.
const PICTURE_FOLDERS: [&str; 2] = ["/System/Library/Desktop Pictures", "/Library/Desktop Pictures"];
const PICTURE_TYPES: [&str; 6] = ["heic", "heif", "jpg", "jpeg", "png", "tiff"];

/// A wallpaper the user can have: an existing image in a built-in picture folder or a
/// folder they shared. Anything else, a symlink out of those folders included, is refused.
pub fn allowed_picture(path: &str, roots: &[String]) -> Result<PathBuf, String> {
    let real = Path::new(path).canonicalize().map_err(|_| "That picture isn’t on this Mac.".to_string())?;
    let picture = real
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| PICTURE_TYPES.contains(&ext.to_ascii_lowercase().as_str()));
    if !real.is_file() || !picture {
        return Err("That isn’t a picture I can use as wallpaper.".into());
    }
    let inside = PICTURE_FOLDERS.iter().map(PathBuf::from).chain(roots.iter().map(PathBuf::from)).any(|folder| {
        folder.canonicalize().is_ok_and(|folder| real.starts_with(folder))
    });
    if inside { Ok(real) } else { Err("Pick a built-in wallpaper or a picture in a folder you’ve shared.".into()) }
}

/// The built-in desktop pictures by name ("Sequoia Sunrise"), for the model to choose from.
pub fn built_in_pictures() -> Vec<(String, String)> {
    let mut pictures = Vec::new();
    for folder in PICTURE_FOLDERS {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let usable = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| PICTURE_TYPES.contains(&ext.to_ascii_lowercase().as_str()));
            if usable {
                if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
                    pictures.push((name.to_string(), path.to_string_lossy().into_owned()));
                }
            }
        }
    }
    pictures.sort();
    pictures
}

/// The approved arguments in canonical form: `setting` and a `value` the setting accepts.
pub fn validate(args: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if args.keys().any(|key| key != "setting" && key != "value") {
        return Err("change_mac_setting takes only setting and value.".into());
    }
    let id = args.get("setting").and_then(Value::as_str).unwrap_or_default().trim();
    let setting = setting(id).ok_or("That setting isn’t one I can change.")?;
    let raw = match args.get("value") {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(on)) => if *on { "on".into() } else { "off".into() },
        _ => return Err("Say what to change it to.".into()),
    };
    let value = match setting.kind {
        // "Hide" turns the Dock's hiding on, but turns desktop icons off.
        Kind::Toggle => match (raw.to_lowercase().as_str(), setting.id.ends_with("autohide")) {
            ("on" | "true" | "yes" | "enable", _) | ("hide", true) | ("show", false) => "on".to_string(),
            ("off" | "false" | "no" | "disable", _) | ("show", true) | ("hide", false) => "off".to_string(),
            _ => return Err(format!("{} is either on or off.", setting.label)),
        },
        Kind::Choice(choices) => {
            let lower = raw.to_lowercase();
            if !choices.contains(&lower.as_str()) {
                return Err(format!("{} can be {}.", setting.label, choices.join(", ")));
            }
            lower
        }
        Kind::Minutes(allowed) => {
            let minutes: u64 = raw.trim_end_matches(" minutes").trim_end_matches(" min").parse().map_err(|_| "Say how many minutes.".to_string())?;
            if !allowed.contains(&minutes) {
                return Err(format!("Choose one of {} minutes (0 is never).", allowed.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")));
            }
            minutes.to_string()
        }
        Kind::Picture => {
            if raw.is_empty() || raw.chars().count() > 1024 || raw.chars().any(char::is_control) {
                return Err("Say which picture.".into());
            }
            raw
        }
    };
    let mut out = Map::new();
    out.insert("setting".into(), setting.id.into());
    out.insert("value".into(), value.into());
    Ok(out)
}

/// How a value reads on the approval sheet and in the receipt.
pub fn shown(id: &str, value: &str) -> String {
    match (setting(id).map(|s| s.kind), value) {
        (Some(Kind::Picture), path) => Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or(path).to_string(),
        (Some(Kind::Minutes(_)), "0") => "never".into(),
        (Some(Kind::Minutes(_)), minutes) => format!("{minutes} min"),
        (Some(Kind::Choice(_)), choice) => {
            let mut letters = choice.chars();
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
        }
        (_, "on") => "On".into(),
        (_, "off") => "Off".into(),
        (_, other) => other.into(),
    }
}

async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    crate::actions::output(command, 20).await.map(|out| out.trim().to_string())
}

async fn script(source: &str, argv: &[&str]) -> Result<String, String> {
    crate::skills::osascript(source, argv, 20).await
}

/// The Wi-Fi interface ("en0"), from the hardware port list.
async fn wifi_device() -> Result<String, String> {
    let ports = run("/usr/sbin/networksetup", &["-listallhardwareports"]).await?;
    let mut lines = ports.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "Hardware Port: Wi-Fi" {
            if let Some(device) = lines.next().and_then(|line| line.trim().strip_prefix("Device: ")) {
                return Ok(device.trim().to_string());
            }
        }
    }
    Err("This Mac has no Wi-Fi.".into())
}

fn on(value: bool) -> String {
    if value { "on".into() } else { "off".into() }
}

/// A Finder or Dock preference as on/off; an unset key means the macOS default.
fn defaults_bool(text: &str, default: bool) -> String {
    match text.trim() {
        "1" | "true" | "TRUE" | "YES" => on(true),
        "0" | "false" | "FALSE" | "NO" => on(false),
        _ => on(default),
    }
}

/// The setting's current value, in the same form `validate` gives.
pub async fn read(id: &str) -> Result<String, String> {
    match id {
        "wallpaper" => script(r#"tell application "System Events" to get picture of desktop 1"#, &[]).await,
        "appearance" => script(r#"tell application "System Events" to tell appearance preferences to get dark mode"#, &[])
            .await
            .map(|dark| if dark == "true" { "dark".into() } else { "light".into() }),
        "dock_autohide" => script(r#"tell application "System Events" to get autohide of dock preferences"#, &[]).await.map(|v| on(v == "true")),
        "dock_magnification" => script(r#"tell application "System Events" to get magnification of dock preferences"#, &[]).await.map(|v| on(v == "true")),
        "menu_bar_autohide" => script(r#"tell application "System Events" to get autohide menu bar of dock preferences"#, &[]).await.map(|v| on(v == "true")),
        "dock_position" => script(r#"tell application "System Events" to get (screen edge of dock preferences) as text"#, &[]).await.map(|edge| edge.to_lowercase()),
        "wifi" => {
            let device = wifi_device().await?;
            run("/usr/sbin/networksetup", &["-getairportpower", &device]).await.map(|power| on(power.trim_end().ends_with("On")))
        }
        "hidden_files" => Ok(defaults_bool(&run("/usr/bin/defaults", &["read", "com.apple.finder", "AppleShowAllFiles"]).await.unwrap_or_default(), false)),
        "desktop_icons" => Ok(defaults_bool(&run("/usr/bin/defaults", &["read", "com.apple.finder", "CreateDesktop"]).await.unwrap_or_default(), true)),
        "screen_saver" => {
            let seconds: u64 = run("/usr/bin/defaults", &["-currentHost", "read", "com.apple.screensaver", "idleTime"]).await.ok().and_then(|text| text.parse().ok()).unwrap_or(1200);
            Ok((seconds / 60).to_string())
        }
        _ => Err("That setting isn’t one I can change.".into()),
    }
}

async fn write(id: &str, value: &str) -> Result<(), String> {
    let truth = if value == "on" { "true" } else { "false" };
    match id {
        "wallpaper" => script("on run argv\ntell application \"System Events\" to tell every desktop to set picture to (item 1 of argv)\nend run", &[value]).await.map(drop),
        "appearance" => script("on run argv\ntell application \"System Events\" to tell appearance preferences to set dark mode to ((item 1 of argv) is \"dark\")\nend run", &[value]).await.map(drop),
        "dock_autohide" => script("on run argv\ntell application \"System Events\" to set autohide of dock preferences to ((item 1 of argv) is \"true\")\nend run", &[truth]).await.map(drop),
        "dock_magnification" => script("on run argv\ntell application \"System Events\" to set magnification of dock preferences to ((item 1 of argv) is \"true\")\nend run", &[truth]).await.map(drop),
        "menu_bar_autohide" => script("on run argv\ntell application \"System Events\" to set autohide menu bar of dock preferences to ((item 1 of argv) is \"true\")\nend run", &[truth]).await.map(drop),
        "dock_position" => script("on run argv\ntell application \"System Events\" to tell dock preferences\nif item 1 of argv is \"left\" then\nset screen edge to left\nelse if item 1 of argv is \"right\" then\nset screen edge to right\nelse\nset screen edge to bottom\nend if\nend tell\nend run", &[value]).await.map(drop),
        "wifi" => {
            let device = wifi_device().await?;
            run("/usr/sbin/networksetup", &["-setairportpower", &device, value]).await.map(drop)
        }
        "hidden_files" | "desktop_icons" => {
            let key = if id == "hidden_files" { "AppleShowAllFiles" } else { "CreateDesktop" };
            run("/usr/bin/defaults", &["write", "com.apple.finder", key, "-bool", truth]).await?;
            // Finder reads these when it starts.
            let _ = run("/usr/bin/killall", &["Finder"]).await;
            Ok(())
        }
        "screen_saver" => {
            let seconds = (value.parse::<u64>().map_err(|_| "Say how many minutes.".to_string())? * 60).to_string();
            run("/usr/bin/defaults", &["-currentHost", "write", "com.apple.screensaver", "idleTime", "-int", &seconds]).await.map(drop)
        }
        _ => Err("That setting isn’t one I can change.".into()),
    }
}

fn receipt(outcome: ActionOutcome, id: &str, message: String, evidence: Value, error: Option<String>) -> ActionReceipt {
    ActionReceipt {
        outcome,
        tool: TOOL.into(),
        object_id: Some(format!("mac-setting:{id}")),
        destination: Some("This Mac".into()),
        message,
        evidence,
        error,
    }
}

/// Changes one setting the user approved and reads it back. A value that didn't change
/// is a failure; one that couldn't be read back is unknown, never a success.
pub async fn execute(args: &Map<String, Value>, roots: &[String]) -> ActionReceipt {
    let args = match validate(args) {
        Ok(args) => args,
        Err(error) => return receipt(ActionOutcome::Failed, "", error.clone(), json!({}), Some(error)),
    };
    let id = args["setting"].as_str().unwrap_or_default().to_string();
    let mut value = args["value"].as_str().unwrap_or_default().to_string();
    if id == "wallpaper" {
        // A bare name is a built-in picture ("Sonoma"); anything else is a path.
        if !value.contains('/') {
            let wanted = value.to_lowercase();
            match built_in_pictures().into_iter().find(|(name, _)| name.to_lowercase() == wanted) {
                Some((_, path)) => value = path,
                None => {
                    let error = format!("There’s no built-in wallpaper called {value}.");
                    return receipt(ActionOutcome::Failed, &id, error.clone(), json!({}), Some(error));
                }
            }
        }
        value = match allowed_picture(&value, roots) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => return receipt(ActionOutcome::Failed, &id, error.clone(), json!({}), Some(error)),
        };
    }
    let label = setting(&id).map(|s| s.label).unwrap_or("Setting");
    let before = read(&id).await.ok();
    if let Err(error) = write(&id, &value).await {
        let message = if error.contains("-1743") || error.contains("Not authorized") || error.contains("Automation") {
            "macOS didn’t allow Buntini to change that. Allow it in System Settings → Privacy & Security → Automation.".to_string()
        } else {
            error
        };
        return receipt(ActionOutcome::Failed, &id, message.clone(), json!({"before":before}), Some(message));
    }
    // Finder and the Dock can take a moment to pick a change up.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let after = read(&id).await;
    let evidence = json!({"setting":id, "before":before, "requested":value, "after":after.as_ref().ok()});
    match after {
        Ok(after) if same(&id, &after, &value) => receipt(ActionOutcome::Succeeded, &id, format!("{label} · {}", shown(&id, &value)), evidence, None),
        Ok(after) => {
            let message = format!("{label} is still {}.", shown(&id, &after));
            receipt(ActionOutcome::Failed, &id, message.clone(), evidence, Some(message))
        }
        Err(error) => receipt(ActionOutcome::Unknown, &id, format!("I couldn’t check {} afterwards. Look before asking again.", label.to_lowercase()), evidence, Some(error)),
    }
}

/// Whether a read-back value is the one approved. Wallpaper paths compare as files.
fn same(id: &str, after: &str, value: &str) -> bool {
    if id == "wallpaper" {
        return Path::new(after).canonicalize().ok() == Path::new(value).canonicalize().ok();
    }
    after == value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(setting: &str, value: Value) -> Map<String, Value> {
        json!({"setting":setting, "value":value}).as_object().unwrap().clone()
    }

    #[test]
    fn only_listed_settings_with_accepted_values_pass() {
        assert_eq!(validate(&args("dock_autohide", json!("Hide"))).unwrap()["value"], "on");
        assert_eq!(validate(&args("dock_position", json!("Left"))).unwrap()["value"], "left");
        assert_eq!(validate(&args("screen_saver", json!(10))).unwrap()["value"], "10");
        assert_eq!(validate(&args("wifi", json!(false))).unwrap()["value"], "off");
        assert_eq!(validate(&args("desktop_icons", json!("hide"))).unwrap()["value"], "off");
        for (setting, value) in [("firewall", json!("off")), ("filevault", json!("off")), ("dock_position", json!("top")), ("screen_saver", json!(7)), ("wifi", json!("maybe")), ("wallpaper", json!(""))] {
            assert!(validate(&args(setting, value.clone())).is_err(), "{setting} = {value}");
        }
        let mut extra = args("wifi", json!("on"));
        extra.insert("script".into(), json!("do shell script \"rm\""));
        assert!(validate(&extra).is_err(), "nothing but setting and value");
    }

    #[test]
    fn a_wallpaper_comes_only_from_built_in_pictures_or_a_shared_folder() {
        let shared = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let photo = shared.path().join("beach.jpg");
        std::fs::write(&photo, b"jpg").unwrap();
        let outside = elsewhere.path().join("other.png");
        std::fs::write(&outside, b"png").unwrap();
        let notes = shared.path().join("notes.txt");
        std::fs::write(&notes, b"text").unwrap();
        let roots = [shared.path().canonicalize().unwrap().to_string_lossy().into_owned()];
        assert!(allowed_picture(photo.to_str().unwrap(), &roots).is_ok());
        assert!(allowed_picture(outside.to_str().unwrap(), &roots).is_err(), "not a shared folder");
        assert!(allowed_picture(notes.to_str().unwrap(), &roots).is_err(), "not a picture");
        #[cfg(unix)]
        {
            let link = shared.path().join("sneaky.png");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(allowed_picture(link.to_str().unwrap(), &roots).is_err(), "a link out of the folder");
        }
    }

    #[test]
    fn values_read_as_people_say_them() {
        assert_eq!(shown("screen_saver", "0"), "never");
        assert_eq!(shown("dock_position", "left"), "Left");
        assert_eq!(shown("wifi", "off"), "Off");
        assert_eq!(shown("wallpaper", "/System/Library/Desktop Pictures/Sonoma.heic"), "Sonoma");
    }
}
