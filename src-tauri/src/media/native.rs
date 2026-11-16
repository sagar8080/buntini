//! The C entry points of native/surface/Sources/BuntiniSurface/Media/MediaExports.swift:
//! the players' own "something changed" notifications, players opening and quitting, and
//! the screen sleeping or locking. Only that something happened arrives here, never what.
use serde::Deserialize;

type Callback = extern "C" fn(*const u8, usize);

#[cfg(target_os = "macos")]
extern "C" {
    fn buntini_media_watch(callback: Callback) -> bool;
}

/// What the watcher noticed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase", deny_unknown_fields)]
pub enum Event {
    /// A player said its track or state changed, or it opened or quit.
    Player { player: String },
    /// The screen slept or locked (`true`), or woke and unlocked (`false`).
    Asleep { value: bool },
}

/// Starts watching. Events arrive on the main thread through `callback`.
pub fn watch(callback: Callback) -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the callback is a plain function that lives for the whole program.
        unsafe { buntini_media_watch(callback) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = callback;
        false
    }
}

/// Rebuilds an event Swift handed over; Swift keeps the bytes alive until this returns.
///
/// # Safety
/// `bytes` must point to `length` readable bytes, or be null.
pub unsafe fn event(bytes: *const u8, length: usize) -> Option<Event> {
    if bytes.is_null() || length == 0 || length > 4096 {
        return None;
    }
    serde_json::from_slice(std::slice::from_raw_parts(bytes, length)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_and_nothing_else_does() {
        let read = |text: &str| unsafe { event(text.as_ptr(), text.len()) };
        assert_eq!(read(r#"{"event":"player","player":"spotify"}"#), Some(Event::Player { player: "spotify".into() }));
        assert_eq!(read(r#"{"event":"asleep","value":true}"#), Some(Event::Asleep { value: true }));
        assert_eq!(read(r#"{"event":"player","player":"music","title":"x"}"#), None);
        assert_eq!(read(""), None);
    }
}
