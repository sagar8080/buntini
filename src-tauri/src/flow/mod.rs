//! Dictation anywhere, the pill, offline recognition, call notes and the wake word (stream C).
//!
//! - `keys`: what holding and tapping the dictation key means.
//! - `text`: your dictionary, replacements, snippets and the style per app.
//! - `native`: the key watcher, the focused text field and pasting, in Buntini's own process.
//! - `service`: the long-lived audio helper (microphone, Parakeet, call recording, muting).
//! - `runtime`: ties them together.
//! - `history`, `notes`, `wake`, `models`: statistics and history, call notes, "Hey Buntini",
//!   and the speech models downloaded on request.
pub mod history;
pub mod keys;
pub mod models;
pub mod native;
pub mod notes;
mod runtime;
pub mod service;
#[cfg(debug_assertions)]
pub mod smoke;
pub mod text;
pub mod wake;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use text::{Mode, Replacement, Snippet, Style};

pub use runtime::start;

/// The key that dictates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DictationKey {
    #[default]
    RightOption,
    RightCommand,
    RightControl,
    Fn,
    /// Your own combination (`Settings::chord`).
    Chord,
}

impl DictationKey {
    pub fn id(self) -> &'static str {
        match self {
            Self::RightOption => "rightOption",
            Self::RightCommand => "rightCommand",
            Self::RightControl => "rightControl",
            Self::Fn => "fn",
            Self::Chord => "chord",
        }
    }
}

/// A key combination of your own, held like the dictation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Chord {
    /// The macOS virtual key code.
    pub key_code: u16,
    /// `command`, `option`, `control`, `shift`.
    pub modifiers: Vec<String>,
    /// How it reads, such as ⌃⌥D.
    pub label: String,
}

/// Saved under `settings.flow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Settings {
    /// Dictate anywhere by holding the key.
    pub enabled: bool,
    pub key: DictationKey,
    pub chord: Option<Chord>,
    /// Up to three, such as `en-US`; none means the Mac's own language.
    pub languages: Vec<String>,
    /// Mute the Mac's sound while you speak.
    pub mute: bool,
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    /// "Hey Buntini".
    pub wake_word: bool,
    /// Agreed to record calls and meetings.
    pub record_calls: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            key: DictationKey::RightOption,
            chord: None,
            languages: Vec::new(),
            mute: true,
            dictionary: Vec::new(),
            replacements: Vec::new(),
            snippets: Vec::new(),
            wake_word: false,
            record_calls: false,
        }
    }
}

fn language_tag(tag: &str) -> bool {
    let mut parts = tag.split(['-', '_']);
    let first = parts.next().unwrap_or("");
    (2..=3).contains(&first.len()) && first.chars().all(|ch| ch.is_ascii_alphabetic()) && parts.all(|part| (2..=8).contains(&part.len()) && part.chars().all(|ch| ch.is_ascii_alphanumeric()))
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        let short = |text: &str, most: usize| !text.trim().is_empty() && text.chars().count() <= most;
        if self.languages.len() > 3 || !self.languages.iter().all(|tag| language_tag(tag)) {
            return Err("Choose up to three dictation languages.".into());
        }
        if self.dictionary.len() > 500 || !self.dictionary.iter().all(|word| short(word, 64)) {
            return Err("Dictionary words must be 1–64 characters, at most 500 of them.".into());
        }
        if self.replacements.len() > 200 || !self.replacements.iter().all(|item| short(&item.from, 64) && item.to.chars().count() <= 1000) {
            return Err("Each replacement needs a phrase of up to 64 characters, at most 200 of them.".into());
        }
        if self.snippets.len() > 100 || !self.snippets.iter().all(|item| short(&item.trigger, 64) && short(&item.text, 4000)) {
            return Err("Each snippet needs a phrase and its text, at most 100 of them.".into());
        }
        match (&self.key, &self.chord) {
            (DictationKey::Chord, None) => return Err("Record a key combination first.".into()),
            (_, Some(chord)) => {
                let known = ["command", "option", "control", "shift"];
                if chord.key_code > 127 || chord.modifiers.is_empty() || !chord.modifiers.iter().all(|modifier| known.contains(&modifier.as_str())) || chord.label.chars().count() > 20 {
                    return Err("That key combination can’t be used.".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// What you pick from the pill: a style per app, and the mode. Kept apart from `Settings`,
/// which the settings sheet saves whole, so a pick isn't overwritten by an older copy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Choices {
    /// By bundle id.
    pub app_styles: BTreeMap<String, Style>,
    pub mode: Mode,
    /// For translate mode, such as `French`.
    pub translate_to: String,
}

impl Choices {
    pub fn validate(&self) -> Result<(), String> {
        let bundle = |id: &str| !id.is_empty() && id.len() <= 255 && id.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
        if self.app_styles.len() > 200 || !self.app_styles.keys().all(|id| bundle(id)) {
            return Err("An app style names an unknown app.".into());
        }
        if self.translate_to.chars().count() > 40 {
            return Err("Name the language to translate into in a few letters.".into());
        }
        Ok(())
    }
}

/// What the UI asks of this part, through its one command.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    /// Answers true: the command is wired.
    Ping,
    /// Whether dictation can run, which engine, and the model download.
    Status,
    /// Settings were saved: apply the key, the wake word and the pill.
    Apply,
    Stats,
    History {
        #[serde(default)]
        limit: Option<usize>,
    },
    ForgetDictation {
        id: i64,
    },
    Notes,
    DeleteNote {
        id: String,
    },
    StartNotes,
    StopNotes,
    Choices,
    /// The mode, and the language for translate mode.
    Choose {
        mode: Mode,
        #[serde(default)]
        translate_to: Option<String>,
    },
    /// A style for one app; none goes back to the app's kind.
    SetStyle {
        bundle: String,
        style: Option<Style>,
    },
    DownloadModels,
    CancelDownload,
    RemoveModels,
    OpenLibrary,
    /// The language Buntini speaks now, for the Library window.
    Language,
}

#[tauri::command]
pub async fn flow_call(app: tauri::AppHandle, op: Op) -> Result<serde_json::Value, String> {
    runtime::call(app, op).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_with_the_right_option_key_and_muting() {
        let settings = Settings::default();
        assert!(!settings.enabled && !settings.wake_word && !settings.record_calls);
        assert_eq!(settings.key, DictationKey::RightOption);
        assert!(settings.mute);
        assert!(settings.validate().is_ok());
        // An older save without these fields reads as the defaults.
        let read: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(read.enabled && read.mute);
    }

    #[test]
    fn settings_are_checked() {
        let with = |edit: fn(&mut Settings)| {
            let mut settings = Settings::default();
            edit(&mut settings);
            settings.validate()
        };
        assert!(with(|s| s.languages = vec!["en-US".into(), "fr".into(), "pt-BR".into()]).is_ok());
        assert!(with(|s| s.languages = vec!["en".into(); 4]).is_err());
        assert!(with(|s| s.languages = vec!["english please".into()]).is_err());
        assert!(with(|s| s.key = DictationKey::Chord).is_err(), "a chord needs its keys");
        assert!(with(|s| {
            s.key = DictationKey::Chord;
            s.chord = Some(Chord { key_code: 2, modifiers: vec!["control".into(), "option".into()], label: "⌃⌥D".into() });
        })
        .is_ok());
        assert!(with(|s| s.chord = Some(Chord { key_code: 2, modifiers: vec![], label: "D".into() })).is_err(), "a bare key would type");
        assert!(with(|s| s.snippets = vec![Snippet { trigger: " ".into(), text: "x".into() }]).is_err());
        assert!(serde_json::from_str::<Settings>(r#"{"surprise":1}"#).is_err());
    }

    #[test]
    fn choices_are_checked() {
        let mut choices = Choices::default();
        choices.app_styles.insert("com.apple.Terminal".into(), Style::Natural);
        assert!(choices.validate().is_ok());
        choices.app_styles.insert("not an id!".into(), Style::Natural);
        assert!(choices.validate().is_err());
        assert!(Choices { translate_to: "x".repeat(41), ..Choices::default() }.validate().is_err());
    }
}
