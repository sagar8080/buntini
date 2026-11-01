//! Test observer copied only into a disposable source tree by the smoke script.
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

async fn evaluate(window: &WebviewWindow, source: &str) -> Result<Value, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(send));
    window.eval_with_callback(source, move |value| {
        if let Some(sender) = sender.lock().unwrap().take() { let _ = sender.send(value); }
    }).map_err(|error| error.to_string())?;
    let raw = tokio::time::timeout(Duration::from_secs(3), receive).await
        .map_err(|_| "JavaScript readback timeout".to_string())?
        .map_err(|_| "JavaScript readback closed".to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

async fn ready(window: &WebviewWindow) -> Result<(), String> {
    for _ in 0..150 {
        // Wry queues scripts submitted before its first finished navigation and
        // drops their callback; readiness probes must retry that startup race.
        if evaluate(window, "Boolean(window.__TAURI_INTERNALS__ && document.querySelector('#root')?.children.length)").await.is_ok_and(|value| value == true) { return Ok(()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("The bundled frontend did not mount".into())
}

async fn result(window: &WebviewWindow) -> Result<Value, String> {
    for _ in 0..150 {
        let value = evaluate(window, "window.__buntiniStartupSmoke || null").await?;
        if value["state"] == "done" { return Ok(value); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("The IPC smoke checks did not finish".into())
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let checked: Result<Value, String> = async {
            let main = app.get_webview_window("main").ok_or("Main window missing")?;
            ready(&main).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            main.eval(include_str!("startup_smoke_checks.js")).map_err(|error| error.to_string())?;
            let main_result = result(&main).await?;
            if main_result["passed"] != true { return Err(format!("Main IPC checks failed: {main_result}")); }
            // Same bundled frontend, with an unprivileged label and no capability.
            let other = WebviewWindowBuilder::new(&app, "smoke-untrusted", WebviewUrl::App("index.html".into()))
                .visible(false).incognito(true)
                .on_navigation(|url| crate::security::ui_navigation_allowed(url, false))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .on_download(|_, _| false)
                .build().map_err(|error| error.to_string())?;
            ready(&other).await?;
            other.eval("window.__buntiniStartupSmoke={state:'running'};window.__TAURI_INTERNALS__.invoke('bootstrap').then(()=>{window.__buntiniStartupSmoke={state:'done',passed:false,error:'Untrusted view invoked bootstrap'}}).catch(error=>{window.__buntiniStartupSmoke={state:'done',passed:/not allowed|denied/i.test(String(error)),denied:String(error)}})").map_err(|error| error.to_string())?;
            let untrusted = result(&other).await?;
            if untrusted["passed"] != true { return Err(format!("Untrusted IPC was allowed: {untrusted}")); }
            let before = main.url().map_err(|error| error.to_string())?;
            main.eval("window.location.assign('https://example.com/')").map_err(|error| error.to_string())?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let after = main.url().map_err(|error| error.to_string())?;
            if before != after { return Err(format!("Main navigated away: {after}")); }
            let window_count = app.webview_windows().len();
            main.eval("window.open('https://example.com/', '_blank')").map_err(|error| error.to_string())?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let popups_blocked = app.webview_windows().len() == window_count;
            other.destroy().map_err(|error| error.to_string())?;
            Ok(json!({"kind":"disposable_companion_startup_smoke", "passed":popups_blocked,
                "main":main_result,"untrustedView":untrusted,"externalNavigationBlocked":true,
                "popupsBlocked":popups_blocked,"nativeUrlAfterNavigationAttempt":after.as_str()}))
        }.await;
        let report = match checked { Ok(report) => report, Err(error) => json!({"kind":"disposable_companion_startup_smoke","passed":false,"error":error}) };
        println!("BUNTINI_STARTUP_SMOKE {}", report);
        app.exit(if report["passed"] == true { 0 } else { 1 });
    });
}
