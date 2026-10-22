use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    LazyLock,
};
use tauri::Manager;
static EXPANDED: AtomicBool = AtomicBool::new(false);
pub fn is_expanded() -> bool {
    EXPANDED.load(Ordering::Relaxed)
}
static SCALE: AtomicU64 = AtomicU64::new(1.0f64.to_bits());
pub fn set_scale(scale: f64) {
    SCALE.store(scale.to_bits(), Ordering::Relaxed);
}
pub fn scale() -> f64 {
    f64::from_bits(SCALE.load(Ordering::Relaxed))
}
static LAYOUT_CHANGED: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// Keep the normal Tauri NSWindow, adding only supported overlay collection
/// behavior. NonactivatingPanel is deliberately reserved for real NSPanel.
pub fn configure(window: &tauri::WebviewWindow) {
    // The WebView is created transparent in config. Set the native window
    // explicitly too; the WebView color setter has no macOS implementation.
    let native_window = window.as_ref().window();
    let _ = native_window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
    #[cfg(target_os = "macos")]
    if let Ok(pointer) = window.ns_window() {
        // setup runs on the AppKit main thread; Tauri owns the NSWindow lifetime.
        unsafe {
            let native = &*(pointer as *const objc2_app_kit::NSWindow);
            native.setOpaque(false);
            native.setBackgroundColor(Some(&objc2_app_kit::NSColor::clearColor()));
            use objc2_app_kit::NSWindowCollectionBehavior as Behavior;
            native.setCollectionBehavior(
                native.collectionBehavior()
                    | Behavior::CanJoinAllSpaces
                    | Behavior::FullScreenAuxiliary
                    | Behavior::IgnoresCycle,
            );
        }
    }
}

/// Accessory windows must be ordered after AppKit finishes launch. This keeps
/// the idle companion visible while preserving the user's current key window.
pub fn order_idle_front(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    if let Ok(pointer) = window.ns_window() {
        unsafe {
            if let Some(main) = objc2::MainThreadMarker::new() {
                let app = objc2_app_kit::NSApplication::sharedApplication(main);
                if app.isHidden() {
                    app.unhideWithoutActivation();
                }
            }
            let native = &*(pointer as *const objc2_app_kit::NSWindow);
            native.setHidesOnDeactivate(false);
            native.orderFrontRegardless();
            if let Some(main) = objc2::MainThreadMarker::new() {
                let app = objc2_app_kit::NSApplication::sharedApplication(main);
                let frame = native.frame();
                let screen=native.screen().map(|screen|{let full=screen.frame();let visible=screen.visibleFrame();serde_json::json!({"frame":{"x":full.origin.x,"y":full.origin.y,"width":full.size.width,"height":full.size.height},"visibleFrame":{"x":visible.origin.x,"y":visible.origin.y,"width":visible.size.width,"height":visible.size.height}})});
                let diagnostic = serde_json::json!({"pid":std::process::id(),"recordedAt":crate::types::now(),"visible":native.isVisible(),"onActiveSpace":native.isOnActiveSpace(),"miniaturized":native.isMiniaturized(),"keyWindow":native.isKeyWindow(),"alpha":native.alphaValue(),"appHidden":app.isHidden(),"appActive":app.isActive(),"occlusionState":native.occlusionState().0,"frame":{"x":frame.origin.x,"y":frame.origin.y,"width":frame.size.width,"height":frame.size.height},"screen":screen});
                if let Ok(cache) = window.app_handle().path().app_cache_dir() {
                    let _ = std::fs::write(
                        cache.join("window-diagnostics.json"),
                        diagnostic.to_string(),
                    );
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window.show();
}

pub fn position_in_visible_screen(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    if let Ok(pointer) = window.ns_window() {
        unsafe {
            let native = &*(pointer as *const objc2_app_kit::NSWindow);
            if let Some(screen) = native.screen() {
                let frame = screen.visibleFrame();
                let size = native.frame().size;
                native.setFrameOrigin(objc2_foundation::NSPoint {
                    x: (frame.origin.x + frame.size.width - size.width - 24.0).max(frame.origin.x),
                    y: frame.origin.y + 48.0,
                });
            }
        }
    }
}

fn transparent_gap(x: f64, y: f64, width: f64, height: f64) -> bool {
    if height <= 180.0 || x < 0.0 || y < 0.0 || x > width || y > height {
        return false;
    }
    let scale = scale();
    let panel = x >= 10.0 && x <= width - 10.0 && y >= 6.0 && y <= height - 120.0 * scale;
    let pet = x >= width - 128.0 * scale && y >= height - 120.0 * scale;
    !panel && !pet
}

/// Global cursor polling continues even while the transparent gap ignores
/// mouse events. Native calls only change hit testing when the region changes.
pub fn start_hit_testing(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ignored = false;
        loop {
            if !EXPANDED.load(Ordering::Relaxed) {
                if ignored {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_ignore_cursor_events(false);
                    }
                    ignored = false;
                }
                LAYOUT_CHANGED.notified().await;
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            let Some(window) = app.get_webview_window("main") else {
                break;
            };
            let desired = (|| {
                if !window.is_visible().ok()? {
                    return Some(false);
                }
                let cursor = window.cursor_position().ok()?;
                let origin = window.outer_position().ok()?;
                let size = window.inner_size().ok()?;
                let scale = window.scale_factor().ok()?;
                Some(transparent_gap(
                    (cursor.x - f64::from(origin.x)) / scale,
                    (cursor.y - f64::from(origin.y)) / scale,
                    f64::from(size.width) / scale,
                    f64::from(size.height) / scale,
                ))
            })()
            .unwrap_or(false);
            if desired != ignored {
                if window.set_ignore_cursor_events(desired).is_ok() {
                    ignored = desired;
                }
            }
        }
    });
}

pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::Resized(size) = event {
        let scale = window.scale_factor().unwrap_or(1.0);
        let expanded = f64::from(size.height) / scale > 180.0;
        EXPANDED.store(expanded, Ordering::Relaxed);
        if !expanded {
            let _ = window.set_ignore_cursor_events(false);
        }
        LAYOUT_CHANGED.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_empty_overlay_regions_pass_through() {
        assert!(!transparent_gap(50.0, 50.0, 128.0, 136.0));
        assert!(!transparent_gap(127.0, 135.0, 128.0, 136.0));
        assert!(!transparent_gap(50.0, 50.0, 420.0, 420.0));
        assert!(!transparent_gap(350.0, 350.0, 420.0, 420.0));
        assert!(transparent_gap(40.0, 350.0, 420.0, 420.0));
        assert!(transparent_gap(260.0, 350.0, 420.0, 420.0));
        assert!(!transparent_gap(-5.0, 350.0, 420.0, 420.0));
    }
}
