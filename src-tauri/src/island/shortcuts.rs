//! Your shortcuts in the island: what's in a Shortcuts folder named "Buntini" shows as buttons
//! on a card when the island is opened, and a click runs that shortcut. The folder is the
//! setting: put a shortcut in it and it's there, Home scenes included. Read with Apple's
//! `shortcuts` command while Shortcuts is switched on in Settings → Apps.
//!
//! The folder is found by its exact name first, and its shortcuts are listed and run by their
//! ids: `shortcuts list --folder-name` quietly lists every shortcut when no folder has the name,
//! so a missing folder must show nothing, never all of them.
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tauri::AppHandle;
use tokio::process::Command;

use crate::i18n::t;
use crate::surface::cards::island::{ShortcutButton, Shortcuts};
use crate::surface::{Card, CardBody, Offer, Source, Urgency};

pub const CARD: &str = "shortcuts";
const FOLDER: &str = "Buntini";
/// How often the folder is read again.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// The most buttons a card holds.
const MOST: usize = 12;

/// "Name (ID)" lines, as `shortcuts list --show-identifiers` writes them.
pub fn parse(listing: &str) -> Vec<(String, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let (name, id) = line.trim().strip_suffix(')')?.rsplit_once(" (")?;
            let valid = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            (valid && !name.trim().is_empty()).then(|| (name.trim().to_string(), id.to_string()))
        })
        .collect()
}

static FOUND: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn found() -> MutexGuard<'static, Vec<(String, String)>> {
    FOUND.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn list(args: &[&str]) -> Option<String> {
    let mut command = Command::new("/usr/bin/shortcuts");
    command.arg("list").args(args);
    crate::actions::output(command, 15).await.ok()
}

/// The shortcuts in the Buntini folder, or none when there's no such folder.
async fn read() -> Vec<(String, String)> {
    let Some(folders) = list(&["--folders", "--show-identifiers"]).await else { return Vec::new() };
    let Some((_, folder)) = parse(&folders).into_iter().find(|(name, _)| name == FOLDER) else { return Vec::new() };
    let Some(listing) = list(&["--folder-name", &folder, "--show-identifiers"]).await else { return Vec::new() };
    parse(&listing).into_iter().take(MOST).collect()
}

/// Reads the folder now and then, while Shortcuts is switched on and the screen is on.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if !crate::media::screen_away() {
                let allowed = super::settings(&app).is_some_and(|settings| crate::apps::allowed(&settings, "shortcuts"));
                let now = if allowed { read().await } else { Vec::new() };
                let changed = *found() != now;
                if changed {
                    *found() = now;
                    super::refresh();
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// The card with a button for each shortcut, when the folder has any.
pub fn shown() -> Option<Offer> {
    let found = found();
    if found.is_empty() {
        return None;
    }
    let buttons = found.iter().map(|(name, id)| ShortcutButton { id: id.clone(), name: name.clone() }).collect();
    Some(Offer { card: Card { id: CARD.into(), source: Source::Island, body: CardBody::Shortcuts(Shortcuts { title: t("island.shortcuts.title"), buttons }) }, urgency: Urgency::Info, at_ms: 0, ttl_ms: None })
}

/// A button's shortcut, run by its id: only one the card offered.
pub fn run(id: &str) -> Result<(), String> {
    let id = found().iter().find(|(_, known)| known == id).map(|(_, id)| id.clone()).ok_or("That shortcut isn't in the Buntini folder.")?;
    tauri::async_runtime::spawn(async move {
        let mut command = Command::new("/usr/bin/shortcuts");
        command.args(["run", &id]);
        if let Err(problem) = crate::actions::output(command, 120).await {
            eprintln!("A shortcut didn't finish: {problem}");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ids_are_read_and_anything_else_is_not() {
        let listing = "Buntini (0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0)\nLights (Off) (11111111-2222-3333-4444-555555555555)\nNo id here\nBad (xyz)\n";
        assert_eq!(parse(listing), [("Buntini".to_string(), "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0".to_string()), ("Lights (Off)".into(), "11111111-2222-3333-4444-555555555555".into())]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn only_a_shortcut_the_card_offered_runs() {
        *found() = vec![("Lights".into(), "11111111-2222-3333-4444-555555555555".into())];
        assert!(run("99999999-2222-3333-4444-555555555555").is_err());
        let Some(Offer { card, .. }) = shown() else { panic!() };
        let CardBody::Shortcuts(body) = card.body else { panic!() };
        assert_eq!((body.buttons[0].name.as_str(), body.buttons.len()), ("Lights", 1));
        found().clear();
        assert!(shown().is_none());
    }

    /// The Buntini folder on this Mac, read the way the island reads it, running nothing:
    /// `cargo test --lib island::shortcuts::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let shortcuts = runtime.block_on(read());
        println!("{} in the Buntini folder: {:?}", shortcuts.len(), shortcuts.iter().map(|(name, _)| name).collect::<Vec<_>>());
    }
}
