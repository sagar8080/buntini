mod actions;
#[cfg(debug_assertions)]
pub mod acceptance_apps;
mod advisor;
mod approvals;
mod apps;
mod ax;
mod brief;
mod browser_workspace;
mod browser_session;
mod budget;
mod calculation;
mod structured_math;
mod writing_context;
mod source_fields;
mod edit_targets;
mod location_fields;
mod chat_threads;
mod chatgpt;
mod cloud;
mod completion;
mod control_actions;
mod controller;
mod conversation;
mod data;
mod delivery;
mod embedding;
mod explicit_fields;
mod focus;
mod goals;
mod indexer;
mod integrations;
mod local;
mod login;
mod mcp;
mod memory_index;
mod migration;
mod model;
mod monitor_actions;
mod notifications;
mod nudges;
mod overlay;
mod planning;
mod pricing;
mod privacy;
mod recipes;
mod research;
mod routines;
mod search_intent;
#[cfg(test)]
mod search_intent_tests;
mod search_pipeline;
#[cfg(test)]
mod search_pipeline_tests;
#[cfg(test)]
mod scenario_evals;
mod scheduler;
mod security;
mod skills;
mod store;
mod support;
mod task_contract;
mod tools;
mod triage;
mod types;
mod voice;
mod watches;
mod web;
mod worker;
mod workflows;
mod world;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
pub fn run_public_browser_smoke() {
    browser_workspace::run_public_smoke(app_context());
}

#[cfg(debug_assertions)]
pub fn run_public_desktop_browser_smoke() {
    browser_session::run_public_smoke();
}

fn show_companion(app: &tauri::AppHandle) {
    // Before Buntini takes focus: what "this" means is whatever the user was just in.
    focus::capture();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit("companion-toggle", ());
    }
}
fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        show_companion(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir()?;
            // Dev builds under a custom CARGO_TARGET_DIR have no resource folder; the
            // controller then falls back to the project's worker/ and native/ folders.
            let resources = app.path().resource_dir().unwrap_or_default();
            data::install_crash_log(&data);
            app.manage(
                controller::AppState::new(data, cache, resources).map_err(std::io::Error::other)?,
            );
            // Construct the privileged UI with navigation guards in place before
            // it loads. External pages belong in the isolated browser workspace.
            let main = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or_else(|| std::io::Error::other("Missing main window configuration"))?;
            tauri::WebviewWindowBuilder::from_config(app, main)?
                .on_navigation(|url| security::ui_navigation_allowed(url, cfg!(dev)))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .on_download(|_, _| false)
                .build()?;
            let notices = app.handle().clone();
            app.state::<controller::AppState>()
                .models
                .on_notice(move |text| {
                    let _ = tauri::Emitter::emit(&notices, "budget-warning", text);
                });
            // Learn whether Apple Intelligence can run here before the first chat.
            let probe = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                probe
                    .state::<controller::AppState>()
                    .models
                    .probe_apple()
                    .await;
            });
            // Finished timers are announced through the app.
            skills::mac::set_app(app.handle().clone());
            browser_workspace::set_app(app.handle().clone());
            // Have the local model read its instructions now, so the first reply is quick.
            controller::warm_up_soon(app.handle());
            if let Some(window) = app.get_webview_window("main") {
                let profile = app
                    .state::<controller::AppState>()
                    .store
                    .lock()
                    .unwrap()
                    .settings()
                    .map_err(std::io::Error::other)?;
                overlay::set_scale(profile.companion_scale);
                let width = (128.0 * profile.companion_scale).max(128.0);
                let height = (136.0 * profile.companion_scale).max(136.0);
                let _ = window.set_size(tauri::LogicalSize::new(width, height));
                let _ = window.set_visible_on_all_workspaces(true);
                let _ = window.set_always_on_top(true);
                overlay::configure(&window);
                if let Some(monitor) = window.current_monitor()?.or(app.primary_monitor()?) {
                    let scale = monitor.scale_factor();
                    let origin = monitor.position();
                    let size = monitor.size();
                    let x = origin.x + (size.width as f64 - (width + 24.0) * scale) as i32;
                    let y = origin.y + (size.height as f64 - (height + 48.0) * scale) as i32;
                    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }
            overlay::start_hit_testing(app.handle().clone());
            world::start(app.handle().clone());
            focus::start();
            memory_index::start(app.handle().clone());
            // Before any timer or approval alert can be answered.
            notifications::install(app.handle());
            scheduler::observe_wake();
            scheduler::start(app.handle().clone());
            if let Err(error) = app
                .global_shortcut()
                .register("CommandOrControl+Shift+Space")
            {
                eprintln!("Global shortcut unavailable: {error}");
            }
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            let show = MenuItem::with_id(app, "show", "Show companion", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide companion", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Buntini", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
            TrayIconBuilder::with_id("buntini-tray")
                // A template silhouette, so macOS tints it for light and dark menu bars.
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray.png"
                ))?)
                .icon_as_template(true)
                .tooltip("Buntini · ⌘⇧Space")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_companion(app),
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_companion(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            overlay::window_event(window, event);
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            controller::bootstrap,
            focus::capture_focus,
            controller::save_settings,
            controller::set_api_key,
            controller::pick_folder,
            controller::add_root,
            controller::remove_root,
            controller::run_task,
            controller::new_topic,
            controller::finish_onboarding,
            controller::run_action,
            controller::resume_task,
            controller::task_progress,
            controller::pending_approvals,
            controller::open_approval,
            controller::plan_task_online,
            controller::continue_task_online,
            delivery::background_status,
            delivery::list_timers,
            delivery::cancel_timer,
            controller::quit_app,
            apps::app_list,
            apps::check_app,
            apps::accessibility,
            advisor::mac_profile,
            controller::choose_local_folder,
            chatgpt::chatgpt_sign_in,
            chatgpt::chatgpt_cancel_sign_in,
            chatgpt::chatgpt_sign_out,
            brief::morning_brief,
            controller::forget_memory,
            controller::cancel_action,
            data::export_data,
            data::delete_all_data,
            data::support_info,
            controller::restart_onboarding,
            controller::conversation,
            controller::conversations,
            controller::open_conversation,
            controller::cancel_task,
            controller::clear_history,
            controller::speak,
            controller::stop_speaking,
            controller::start_recording,
            controller::stop_recording,
            controller::cancel_recording,
            controller::open_source,
            approvals::plan_online_request,
            world::pet_world,
            world::pet_move,
            world::pet_stop,
            world::pet_perch,
            world::pet_sample,
            approvals::approve_web_approval,
            notifications::request_notification_permission,
            notifications::notification_permission_status,
            notifications::take_review_request,
            support::list_voices,
            support::preview_voice,
            support::worker_diagnostics,
            support::check_os_updates,
            scheduler::save_search,
            scheduler::update_saved_search,
            scheduler::delete_saved_search,
            scheduler::run_saved_search,
            watches::save_price_watch,
            watches::update_price_watch,
            watches::delete_price_watch,
            watches::run_price_watch,
            routines::update_routine,
            routines::delete_routine,
            mcp::added_tools,
            mcp::add_tool_server,
            mcp::pick_tool_config,
            mcp::add_tool_server_from_config,
            mcp::remove_tool_server,
            mcp::reconnect_tool_server,
            mcp::set_tool_safe,
            nudges::heads_ups,
            nudges::open_heads_up,
            nudges::dismiss_heads_up,
            notifications::take_heads_up_request,
            goals::update_goal,
            goals::delete_goal
        ])
        .build(app_context())
        .expect("Buntini could not start")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                if let Some(window) = app.get_webview_window("main") {
                    overlay::position_in_visible_screen(&window);
                    overlay::order_idle_front(&window);
                }
            }
            #[cfg(target_os = "macos")]
            if matches!(
                event,
                tauri::RunEvent::Reopen {
                    has_visible_windows: false,
                    ..
                }
            ) {
                if let Some(window) = app.get_webview_window("main") {
                    overlay::order_idle_front(&window);
                }
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
                    api.prevent_exit();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<controller::AppState>();
                        for cancel in state.active.lock().unwrap().values() {
                            cancel.store(true, Ordering::SeqCst);
                        }
                        state.voice.shutdown().await;
                        state.models.shutdown().await;
                        mcp::stop_all();
                        app.exit(0);
                    });
                }
            }
        });
}
