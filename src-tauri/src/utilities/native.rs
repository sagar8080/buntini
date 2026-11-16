//! The C entry points of native/surface/Sources/BuntiniSurface/Utilities/UtilityExports.swift:
//! hot keys, the system's own shortcuts, Screen Recording, and putting pictures on the
//! clipboard or pasting them. They run in Buntini's own process.
use std::path::Path;

use serde::Serialize;

#[cfg(target_os = "macos")]
extern "C" {
    fn buntini_hotkeys_set(bytes: *const u8, length: usize, out: *mut u8, capacity: usize) -> isize;
    fn buntini_hotkeys_pause(paused: bool);
    fn buntini_hotkeys_system(key_code: u16, modifiers: u32) -> bool;
    fn buntini_capture_allowed(ask: bool) -> bool;
    fn buntini_capture_screen(out: *mut f64) -> bool;
    fn buntini_capture_copy(path: *const u8, length: usize) -> bool;
    fn buntini_capture_paste(path: *const u8, length: usize) -> bool;
    fn buntini_capture_annotate(input: *const u8, input_length: usize, output: *const u8, output_length: usize) -> bool;
}

/// One hot key to register.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotKey {
    pub id: String,
    pub key_code: u16,
    /// Carbon's modifier mask.
    pub modifiers: u32,
}

/// Replaces every hot key. Returns the ids macOS refused (another app holds them), or none
/// at all when the native side isn't there.
pub fn set_hot_keys(keys: &[HotKey]) -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        let json = serde_json::to_vec(keys).ok()?;
        let mut out = vec![0u8; 4096];
        // SAFETY: both are live slices for the call; Swift copies the input and writes at most
        // `capacity` bytes, returning how many.
        let written = unsafe { buntini_hotkeys_set(json.as_ptr(), json.len(), out.as_mut_ptr(), out.len()) };
        if written < 0 {
            return None;
        }
        out.truncate(written as usize);
        serde_json::from_slice(&out).ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = keys;
        None
    }
}

/// While a shortcut is being recorded, Buntini's own don't fire.
pub fn pause_hot_keys(paused: bool) {
    #[cfg(target_os = "macos")]
    // SAFETY: Swift moves the work to the main thread itself.
    unsafe {
        buntini_hotkeys_pause(paused)
    }
    #[cfg(not(target_os = "macos"))]
    let _ = paused;
}

/// Whether macOS has this combination switched on for itself (Mission Control, Spotlight,
/// input sources, screenshots…).
pub fn system_uses(key_code: u16, modifiers: u32) -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: plain values.
    unsafe {
        buntini_hotkeys_system(key_code, modifiers)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (key_code, modifiers);
        false
    }
}

/// Whether Buntini may record the screen; `ask` shows macOS's request (only ever once).
pub fn capture_allowed(ask: bool) -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: a plain value.
    unsafe {
        buntini_capture_allowed(ask)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ask;
        false
    }
}

/// The screen with the pointer, as `[x, y, width, height]` in points from the main screen's
/// top left (screencapture's `-R`).
pub fn pointer_screen() -> Option<[f64; 4]> {
    #[cfg(target_os = "macos")]
    {
        let mut rect = [0f64; 4];
        // SAFETY: Swift writes exactly four doubles.
        let found = unsafe { buntini_capture_screen(rect.as_mut_ptr()) };
        found.then_some(rect)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

fn with_path<T>(path: &Path, call: impl FnOnce(*const u8, usize) -> T) -> T {
    let text = path.to_string_lossy();
    call(text.as_ptr(), text.len())
}

/// Puts a picture on the clipboard, for pasting by hand.
pub fn copy_image(path: &Path) -> bool {
    // SAFETY: a live slice for the call; Swift copies it.
    #[cfg(target_os = "macos")]
    let copied = with_path(path, |bytes, length| unsafe { buntini_capture_copy(bytes, length) });
    #[cfg(not(target_os = "macos"))]
    let copied = with_path(path, |_, _| false);
    copied
}

/// Pastes a picture where the caret is with ⌘V, putting the clipboard back after. Blocks for
/// most of a second; call it off the main thread. False without Accessibility.
pub fn paste_image(path: &Path) -> bool {
    // SAFETY: as for `copy_image`.
    #[cfg(target_os = "macos")]
    let pasted = with_path(path, |bytes, length| unsafe { buntini_capture_paste(bytes, length) });
    #[cfg(not(target_os = "macos"))]
    let pasted = with_path(path, |_, _| false);
    pasted
}

/// Opens the mark-up editor on `input`. Done saves to `output` and comes back as the
/// `annotated` action.
pub fn annotate(input: &Path, output: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let (input, output) = (input.to_string_lossy(), output.to_string_lossy());
        // SAFETY: live slices for the call; Swift copies them.
        unsafe { buntini_capture_annotate(input.as_ptr(), input.len(), output.as_ptr(), output.len()) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (input, output);
        false
    }
}

/// System Settings → Privacy & Security → Screen & System Audio Recording.
pub fn open_recording_settings() {
    let _ = std::process::Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture").status();
}
