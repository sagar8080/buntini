//! Shortcuts you can change. They are Carbon hot keys registered in Buntini's own process
//! (native/surface/Sources/BuntiniSurface/Utilities/HotKeys.swift), which need no permission.
//! What's checked here is pure: a combination must hold a modifier, must not be one macOS or
//! every app relies on, and must not already belong to another action or to dictation.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::i18n::{t, tf};

/// A key combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shortcut {
    /// The macOS virtual key code.
    pub key_code: u16,
    /// `control`, `option`, `shift`, `command`.
    pub modifiers: Vec<String>,
    /// How it reads, such as ⇧⌘2.
    pub label: String,
}

impl Shortcut {
    fn new(key_code: u16, modifiers: &[&str], label: &str) -> Self {
        Self { key_code, modifiers: modifiers.iter().map(|name| name.to_string()).collect(), label: label.into() }
    }

    /// Carbon's modifier mask, as RegisterEventHotKey and the system's own shortcuts use it.
    pub fn carbon(&self) -> u32 {
        self.modifiers.iter().fold(0, |mask, name| {
            mask | match name.as_str() {
                "command" => 0x100,
                "shift" => 0x200,
                "option" => 0x800,
                "control" => 0x1000,
                _ => 0,
            }
        })
    }

    fn same_keys(&self, other: &Shortcut) -> bool {
        self.key_code == other.key_code && self.carbon() == other.carbon()
    }
}

/// Something a shortcut can do.
pub struct Action {
    pub id: &'static str,
    pub default: Option<(u16, &'static [&'static str], &'static str)>,
}

/// Every action, in the order Settings lists them.
pub const ACTIONS: [Action; 8] = [
    Action { id: "companion", default: Some((49, &["shift", "command"], "⇧⌘Space")) },
    Action { id: "chat", default: Some((49, &["control", "option"], "⌃⌥Space")) },
    Action { id: "waiting", default: Some((0, &["control", "option"], "⌃⌥A")) },
    Action { id: "nextPill", default: Some((30, &["control", "option"], "⌃⌥]")) },
    Action { id: "mute", default: Some((46, &["control", "option"], "⌃⌥M")) },
    Action { id: "screenshot", default: Some((19, &["shift", "command"], "⇧⌘2")) },
    Action { id: "area", default: Some((1, &["control", "option"], "⌃⌥S")) },
    Action { id: "focus", default: Some((3, &["control", "option"], "⌃⌥F")) },
];

pub fn known(id: &str) -> bool {
    ACTIONS.iter().any(|action| action.id == id)
}

/// The action's name in Settings.
pub fn label(id: &str) -> String {
    match id {
        "companion" => t("utilities.shortcuts.companion"),
        "chat" => t("utilities.shortcuts.chat"),
        "waiting" => t("utilities.shortcuts.waiting"),
        "nextPill" => t("utilities.shortcuts.nextPill"),
        "mute" => t("utilities.shortcuts.mute"),
        "screenshot" => t("utilities.shortcuts.screenshot"),
        "area" => t("utilities.shortcuts.area"),
        "focus" => t("utilities.shortcuts.focus"),
        _ => String::new(),
    }
}

/// What differs from the defaults; `None` turns an action's shortcut off.
pub type Overrides = BTreeMap<String, Option<Shortcut>>;

/// Each action with the shortcut it has now.
pub fn bindings(overrides: &Overrides) -> Vec<(&'static str, Option<Shortcut>)> {
    ACTIONS
        .iter()
        .map(|action| {
            let shortcut = match overrides.get(action.id) {
                Some(chosen) => chosen.clone(),
                None => action.default.map(|(code, modifiers, label)| Shortcut::new(code, modifiers, label)),
            };
            (action.id, shortcut)
        })
        .collect()
}

/// Function keys work on their own; every other key needs ⌘, ⌃ or ⌥.
const FUNCTION_KEYS: [u16; 20] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90];

/// Combinations macOS keeps for itself. Most are also in the system's own list, which the
/// native side checks; these are refused even when that list says they're switched off.
const SYSTEM: [(u16, u32); 18] = [
    (49, 0x100),          // ⌘Space, Spotlight
    (49, 0x1100),         // ⌃⌘Space, emoji
    (48, 0x100),          // ⌘Tab
    (48, 0x300),          // ⇧⌘Tab
    (50, 0x100),          // ⌘`
    (53, 0x900),          // ⌥⌘Esc, Force Quit
    (20, 0x300),          // ⇧⌘3
    (21, 0x300),          // ⇧⌘4
    (23, 0x300),          // ⇧⌘5
    (22, 0x300),          // ⇧⌘6
    (20, 0x1300),         // ⌃⇧⌘3
    (21, 0x1300),         // ⌃⇧⌘4
    (12, 0x1100),         // ⌃⌘Q, lock
    (12, 0x300),          // ⇧⌘Q, log out
    (3, 0x1100),          // ⌃⌘F, full screen
    (2, 0x900),           // ⌥⌘D, Dock
    (9, 0x1100),          // ⌃⌘V, Buntini pastes the last dictation
    (44, 0x300),          // ⇧⌘/, Help
];

/// ⌘ and one of these is every app's: quit, close, hide, minimise, copy, paste, cut, undo,
/// select all, save, new, open, print, find, new tab, settings.
const EVERY_APP: [u16; 16] = [12, 13, 4, 46, 8, 9, 7, 6, 0, 1, 45, 31, 35, 3, 17, 43];

/// ⇧⌘ and one of these is in most apps: Save As or Duplicate, redo, a new window or folder,
/// reopen the closed tab, find the previous one, close the window, and the tab before or after.
const EVERY_APP_SHIFT: [u16; 8] = [1, 6, 45, 17, 5, 13, 33, 30];

/// Why `shortcut` can't be used for `id`, in a few words; none when it can.
pub fn problem(id: &str, shortcut: &Shortcut, current: &[(&'static str, Option<Shortcut>)], dictation: Option<&Shortcut>) -> Option<String> {
    let known_names = ["command", "option", "control", "shift"];
    if shortcut.key_code > 127 || !shortcut.modifiers.iter().all(|name| known_names.contains(&name.as_str())) || shortcut.label.is_empty() || shortcut.label.chars().count() > 20 {
        return Some(t("utilities.shortcuts.cantUse"));
    }
    let mask = shortcut.carbon();
    if mask & 0x1900 == 0 && !FUNCTION_KEYS.contains(&shortcut.key_code) {
        return Some(t("utilities.shortcuts.addModifier"));
    }
    if SYSTEM.contains(&(shortcut.key_code, mask)) {
        return Some(t("utilities.shortcuts.macosUses"));
    }
    if mask == 0x100 && EVERY_APP.contains(&shortcut.key_code) || mask == 0x300 && EVERY_APP_SHIFT.contains(&shortcut.key_code) {
        return Some(t("utilities.shortcuts.everyAppUses"));
    }
    if dictation.is_some_and(|chord| chord.same_keys(shortcut)) {
        return Some(t("utilities.shortcuts.dictationUses"));
    }
    current
        .iter()
        .find(|(other, chosen)| *other != id && chosen.as_ref().is_some_and(|chosen| chosen.same_keys(shortcut)))
        .map(|(other, _)| tf("utilities.shortcuts.usedBy", &[("action", &label(other))]))
}

/// What's saved: overrides only for actions that exist, so a removed action leaves nothing.
pub fn tidy(mut overrides: Overrides) -> Overrides {
    overrides.retain(|id, _| known(id));
    let defaults = bindings(&Overrides::new());
    overrides.retain(|id, chosen| defaults.iter().find(|(other, _)| other == id).is_none_or(|(_, default)| default != chosen));
    overrides
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(code: u16, modifiers: &[&str]) -> Shortcut {
        Shortcut::new(code, modifiers, "x")
    }

    #[test]
    fn the_defaults_are_the_familiar_ones_and_leave_the_rest_free() {
        let all = bindings(&Overrides::new());
        let set: Vec<_> = all.iter().filter_map(|(id, shortcut)| shortcut.as_ref().map(|s| (*id, s.label.as_str()))).collect();
        assert_eq!(
            set,
            [("companion", "⇧⌘Space"), ("chat", "⌃⌥Space"), ("waiting", "⌃⌥A"), ("nextPill", "⌃⌥]"), ("mute", "⌃⌥M"), ("screenshot", "⇧⌘2"), ("area", "⌃⌥S"), ("focus", "⌃⌥F")]
        );
        // Every default passes its own checks.
        for (id, shortcut) in &all {
            if let Some(shortcut) = shortcut {
                assert_eq!(problem(id, shortcut, &all, None), None, "{id}");
            }
        }
    }

    #[test]
    fn carbon_reads_the_modifiers() {
        assert_eq!(keys(1, &["shift", "command"]).carbon(), 0x300);
        assert_eq!(keys(1, &["control", "option"]).carbon(), 0x1800);
    }

    #[test]
    fn a_combination_macos_or_every_app_needs_is_refused() {
        let all = bindings(&Overrides::new());
        let check = |code, modifiers: &[&str]| problem("chat", &keys(code, modifiers), &all, None);
        assert_eq!(check(49, &["command"]).as_deref(), Some("macOS uses it"), "Spotlight");
        assert_eq!(check(20, &["shift", "command"]).as_deref(), Some("macOS uses it"), "screenshots");
        assert_eq!(check(9, &["control", "command"]).as_deref(), Some("macOS uses it"), "paste the last dictation");
        assert_eq!(check(8, &["command"]).as_deref(), Some("Every app uses it"), "copy");
        assert_eq!(check(6, &["shift", "command"]).as_deref(), Some("Every app uses it"), "redo");
        assert_eq!(check(1, &["shift", "command"]).as_deref(), Some("Every app uses it"), "Save As, Duplicate");
        assert_eq!(check(17, &["shift", "command"]).as_deref(), Some("Every app uses it"), "reopen the closed tab");
        assert_eq!(check(1, &["shift"]).as_deref(), Some("Add ⌘, ⌃ or ⌥"), "it would type a capital");
        assert_eq!(check(1, &[]).as_deref(), Some("Add ⌘, ⌃ or ⌥"));
        assert_eq!(check(200, &["command"]).as_deref(), Some("Can’t be used"));
        assert_eq!(check(122, &[]), None, "F1 alone is fine");
        assert_eq!(check(8, &["option", "command"]), None);
    }

    #[test]
    fn two_actions_or_dictation_cannot_share_one() {
        let all = bindings(&Overrides::new());
        let screenshot = keys(19, &["shift", "command"]);
        assert_eq!(problem("focus", &screenshot, &all, None).as_deref(), Some("Screenshot uses it"));
        assert_eq!(problem("screenshot", &screenshot, &all, None), None, "an action keeps its own");
        let chord = keys(2, &["control", "option"]);
        assert_eq!(problem("focus", &chord, &all, Some(&chord)).as_deref(), Some("Dictation uses it"));
    }

    #[test]
    fn overrides_keep_only_what_differs() {
        let mut overrides = Overrides::new();
        overrides.insert("screenshot".into(), Some(Shortcut::new(19, &["shift", "command"], "⇧⌘2")));
        overrides.insert("area".into(), None);
        overrides.insert("gone".into(), None);
        let kept = tidy(overrides);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["area"]);
        assert_eq!(bindings(&kept).iter().find(|(id, _)| *id == "area").unwrap().1, None, "turned off");
    }
}
