//! The C entry points of native/surface/Sources/BuntiniSurface/Flow/FlowExports.swift: the
//! key watcher, the focused text field, pasting and copying. They run in Buntini's own
//! process, so they use the Accessibility permission Buntini already has.
use serde::{Deserialize, Serialize};

type Callback = extern "C" fn(*const u8, usize);

#[cfg(target_os = "macos")]
extern "C" {
    fn buntini_flow_watch(callback: Callback, config: *const u8, length: usize) -> bool;
    fn buntini_flow_unwatch();
    fn buntini_flow_focus(out: *mut u8, capacity: usize) -> isize;
    fn buntini_flow_paste(text: *const u8, length: usize) -> i32;
    fn buntini_flow_copy(text: *const u8, length: usize) -> bool;
}

/// Which key dictates, as the watcher reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchConfig {
    /// Watch the dictation key; otherwise only sleep, lock and Low Power Mode.
    pub keys: bool,
    /// `rightOption`, `rightCommand`, `rightControl`, `fn` or `chord`.
    pub key: String,
    pub chord: Option<super::Chord>,
}

/// Starts (or reconfigures) the watcher. Events arrive as JSON through `callback`, on the
/// main thread. False when the keys were asked for and Accessibility isn't allowed yet.
pub fn watch(callback: Callback, config: &WatchConfig) -> bool {
    let json = serde_json::to_vec(config).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the callback is a plain function that lives for the whole program, and the
        // bytes are a live slice Swift copies before returning.
        unsafe { buntini_flow_watch(callback, json.as_ptr(), json.len()) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (callback, json);
        false
    }
}

pub fn unwatch() {
    #[cfg(target_os = "macos")]
    // SAFETY: Swift moves the work to the main thread itself.
    unsafe {
        buntini_flow_unwatch()
    }
}

/// The text field in front, as the accessibility tree describes it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Focus {
    pub app: String,
    pub bundle: String,
    /// The app's process.
    pub pid: i64,
    /// Which element has the caret (its accessibility hash); 0 when none was found.
    pub field: u64,
    pub role: String,
    pub subrole: String,
    /// The value can be set.
    pub settable: bool,
    /// It reports a selection (a caret counts).
    pub selection: bool,
    pub secure: bool,
    /// Secure input is on somewhere (a password field, or a terminal's Secure Keyboard Entry).
    pub secure_input: bool,
    /// Buntini may use Accessibility.
    pub trusted: bool,
}

/// A password is being typed: dictated words are dropped, never typed or copied.
pub fn secure(focus: &Focus) -> bool {
    focus.secure || focus.secure_input || focus.subrole == "AXSecureTextField"
}

/// Whether the caret is still where it was: the same app, the same field and the same kind
/// of field. The words are typed only then.
pub fn same_field(before: &Focus, now: &Focus) -> bool {
    !before.bundle.is_empty() && before.bundle == now.bundle && before.pid == now.pid && before.field == now.field && before.role == now.role && before.subrole == now.subrole
}

/// What's in front. Call off the main thread: it asks the app over Accessibility.
pub fn focus() -> Focus {
    #[cfg(target_os = "macos")]
    {
        let mut buffer = vec![0u8; 16 << 10];
        // SAFETY: Swift writes at most `capacity` bytes into the buffer and returns how many.
        let written = unsafe { buntini_flow_focus(buffer.as_mut_ptr(), buffer.len()) };
        if written > 0 {
            buffer.truncate(written as usize);
            return serde_json::from_slice(&buffer).unwrap_or_default();
        }
    }
    Focus::default()
}

const TEXT_ROLES: [&str; 5] = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField", "AXWebArea"];

/// Whether the words should be typed where the caret is, or kept to paste by hand. A text
/// role, or a web or Electron editor that takes a value and shows a caret; never a password.
pub fn should_paste(focus: &Focus) -> bool {
    if !focus.trusted || secure(focus) {
        return false;
    }
    let texty = TEXT_ROLES.contains(&focus.role.as_str());
    (texty && (focus.settable || focus.selection)) || (focus.settable && focus.selection)
}

/// What became of a paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pasted {
    /// The field was seen to take the words; the clipboard is as it was.
    Arrived,
    /// The words are on the clipboard to paste by hand (they may have been typed too).
    OnClipboard,
    /// A password was being typed: nothing pasted, the clipboard untouched.
    Refused,
}

/// Pastes with ⌘V, keeping the clipboard as it was when the words are seen to arrive.
pub fn paste(text: &str) -> Pasted {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a live slice for the duration of the call; Swift copies it.
        match unsafe { buntini_flow_paste(text.as_ptr(), text.len()) } {
            1 => Pasted::Arrived,
            -1 => Pasted::Refused,
            _ => Pasted::OnClipboard,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        Pasted::Refused
    }
}

/// Puts the words on the clipboard for pasting by hand.
pub fn copy(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: as for `paste`.
        unsafe { buntini_flow_copy(text.as_ptr(), text.len()) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        false
    }
}

/// What the watcher reports.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    Key { key: Key },
    /// ⌃⌘V.
    PasteLast,
    /// The watcher lost track of the keys (its tap was switched off for a moment).
    Reset,
    System {
        asleep: bool,
        locked: bool,
        #[serde(rename = "lowPower")]
        low_power: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Key {
    Down,
    Up,
    Other,
    Escape,
}

impl From<Key> for super::keys::KeyEvent {
    fn from(key: Key) -> Self {
        match key {
            Key::Down => Self::Down,
            Key::Up => Self::Up,
            Key::Other => Self::Other,
            Key::Escape => Self::Escape,
        }
    }
}

/// Rebuilds a byte slice handed over by Swift.
///
/// # Safety
/// `bytes` must point to `length` readable bytes, or be null.
pub unsafe fn read(bytes: *const u8, length: usize) -> Option<Event> {
    if bytes.is_null() || length == 0 || length > 1 << 16 {
        return None;
    }
    serde_json::from_slice(std::slice::from_raw_parts(bytes, length)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(role: &str, settable: bool, selection: bool) -> Focus {
        Focus { role: role.into(), settable, selection, trusted: true, ..Focus::default() }
    }

    #[test]
    fn words_are_typed_into_text_fields_and_kept_otherwise() {
        assert!(should_paste(&field("AXTextArea", true, true)), "TextEdit");
        assert!(should_paste(&field("AXTextField", true, false)));
        // Chrome's and Electron's editors are groups that take a value and show a caret.
        assert!(should_paste(&field("AXGroup", true, true)));
        assert!(should_paste(&field("AXWebArea", false, true)));
        assert!(!should_paste(&field("AXGroup", true, false)));
        assert!(!should_paste(&field("AXButton", false, false)));
        assert!(!should_paste(&field("", false, false)), "nothing focused");
    }

    #[test]
    fn never_into_a_password_or_without_accessibility() {
        assert!(!should_paste(&Focus { secure: true, ..field("AXTextField", true, true) }));
        assert!(!should_paste(&Focus { subrole: "AXSecureTextField".into(), ..field("AXTextField", true, true) }));
        assert!(!should_paste(&Focus { secure_input: true, ..field("AXTextArea", true, true) }), "Secure Keyboard Entry");
        assert!(!should_paste(&Focus { trusted: false, ..field("AXTextArea", true, true) }));
        assert!(secure(&Focus { secure_input: true, ..Focus::default() }));
        assert!(!secure(&field("AXTextArea", true, true)));
    }

    #[test]
    fn the_caret_must_still_be_in_the_same_field() {
        let notes = Focus { bundle: "com.apple.Notes".into(), pid: 7, field: 42, ..field("AXTextArea", true, true) };
        assert!(same_field(&notes, &notes.clone()));
        assert!(!same_field(&notes, &Focus { field: 43, ..notes.clone() }), "another field");
        assert!(!same_field(&notes, &Focus { bundle: "com.apple.Terminal".into(), pid: 9, ..notes.clone() }), "another app");
        assert!(!same_field(&notes, &Focus { role: "AXTextField".into(), ..notes.clone() }));
        assert!(!same_field(&Focus::default(), &Focus::default()), "nothing known");
        let read: Focus = serde_json::from_str(r#"{"app":"Notes","bundle":"com.apple.Notes","pid":7,"field":42,"role":"AXTextArea","secureInput":true,"trusted":true}"#).unwrap();
        assert!(read.secure_input && read.field == 42 && read.pid == 7);
    }

    #[test]
    fn watcher_events_read() {
        let read = |text: &str| unsafe { super::read(text.as_ptr(), text.len()) };
        assert_eq!(read(r#"{"kind":"key","key":"down"}"#), Some(Event::Key { key: Key::Down }));
        assert_eq!(read(r#"{"kind":"pasteLast"}"#), Some(Event::PasteLast));
        assert_eq!(read(r#"{"kind":"system","asleep":false,"locked":true,"lowPower":false}"#), Some(Event::System { asleep: false, locked: true, low_power: false }));
        assert_eq!(read("nonsense"), None);
        assert_eq!(unsafe { super::read(std::ptr::null(), 4) }, None);
    }
}
