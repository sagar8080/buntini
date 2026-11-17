//! Dictation on this Mac without building Buntini or speaking (debug builds only): the real
//! key watcher, key logic, audio helper, text steps, text-field check, paste and pill, with a
//! recorded file played in place of the microphone. Run by `examples/flow_host.rs`; each
//! dictation prints one JSON line with where the words went.
use super::{keys, native, runtime, service::Service, text, Settings};
use crate::surface::{
    self,
    cards::flow::{FlowPill, FlowState},
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::OnceLock, time::Instant};
use tokio::sync::mpsc;

static KEYS: OnceLock<mpsc::UnboundedSender<native::Event>> = OnceLock::new();

extern "C" fn watched(bytes: *const u8, length: usize) {
    // SAFETY: Swift passes a buffer it keeps alive until this returns.
    if let (Some(event), Some(sender)) = (unsafe { native::read(bytes, length) }, KEYS.get()) {
        let _ = sender.send(event);
    }
}

fn pill(state: FlowState, app: Option<String>, text: Option<String>) {
    surface::set_flow(Some(FlowPill { state, text, app, locked: false, style: None, mode: Some(text::Mode::Dictate), recording: None }));
}

/// Runs `rounds` dictations on its own thread, then exits the process. `settings` is a
/// `settings.flow` object (dictionary, replacements, snippets, languages).
pub fn run(helper: PathBuf, models: PathBuf, input: PathBuf, settings: Value, rounds: usize) {
    let flow: Settings = serde_json::from_value(settings).expect("valid flow settings");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
        runtime.block_on(check(helper, models, input, flow, rounds));
        std::process::exit(0);
    });
}

async fn check(helper: PathBuf, models: PathBuf, input: PathBuf, flow: Settings, rounds: usize) {
    let (keys_sender, mut keys_events) = mpsc::unbounded_channel();
    let _ = KEYS.set(keys_sender);
    let (helper_sender, mut helper_events) = mpsc::unbounded_channel();
    let service = Service::new(helper, models, helper_sender);
    let config = native::WatchConfig { keys: true, key: "rightOption".into(), chord: None };
    println!("{}", json!({"watching": native::watch(watched, &config)}));
    pill(FlowState::Idle, None, None);
    let clock = Instant::now();
    let mut machine = keys::Machine::new();
    let mut next_id = 0;
    let mut done = 0;
    let mut open: Option<(u64, Instant)> = None;
    while done < rounds {
        let deadline = machine.next_deadline().map(|at| tokio::time::Instant::from_std(clock + std::time::Duration::from_millis(at)));
        let commands = tokio::select! {
            Some(event) = keys_events.recv() => match event {
                native::Event::Key { key } => machine.event(key.into(), clock.elapsed().as_millis() as u64),
                _ => vec![],
            },
            Some(event) = helper_events.recv() => {
                if super::service::name(&event) == "final" {
                    let (_, started) = open.take().unwrap_or((0, Instant::now()));
                    let heard = event["text"].as_str().unwrap_or("").to_string();
                    let focus = tokio::task::spawn_blocking(native::focus).await.unwrap_or_default();
                    let style = text::style_for(&focus.bundle, &Default::default());
                    let shaped = runtime::shape(&heard, &flow, style);
                    let typed = runtime::complete(&shaped.text, &shaped.kept, &flow, style);
                    let paste = native::should_paste(&focus);
                    let words = typed.clone();
                    let pasted = tokio::task::spawn_blocking(move || {
                        if paste && native::paste(&words) == native::Pasted::Arrived {
                            return true;
                        }
                        native::copy(&words);
                        false
                    })
                    .await
                    .unwrap_or(false);
                    pill(if pasted { FlowState::Done } else { FlowState::Copied }, Some(focus.app.clone()), None);
                    println!("{}", json!({"app": focus.app, "bundle": focus.bundle, "role": focus.role, "settable": focus.settable, "selection": focus.selection, "style": style, "heard": heard, "typed": typed, "pasted": pasted, "engine": event["engine"], "seconds": started.elapsed().as_secs_f64()}));
                    done += 1;
                } else if matches!(super::service::name(&event), "error" | "cancelled") {
                    println!("{}", json!({"helper": event}));
                    open = None;
                    done += 1;
                    pill(FlowState::Idle, None, None);
                }
                vec![]
            }
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {
                machine.tick(clock.elapsed().as_millis() as u64).into_iter().collect()
            }
        };
        for command in commands {
            println!("{}", json!({"command": format!("{command:?}")}));
            match command {
                keys::Command::Start => {
                    next_id += 1;
                    open = Some((next_id, Instant::now()));
                    // As the runtime does: where the words go is looked up when dictation starts.
                    tokio::task::spawn_blocking(native::focus);
                    let _ = service.send(json!({"op": "dictate", "id": next_id, "languages": flow.languages, "input": input})).await;
                    pill(FlowState::Listening, None, None);
                }
                keys::Command::Lock => {}
                keys::Command::Finish => {
                    if let Some((id, _)) = open {
                        let _ = service.send(json!({"op": "stop", "id": id})).await;
                        pill(FlowState::Working, None, None);
                    }
                }
                keys::Command::Cancel => {
                    if let Some((id, _)) = open.take() {
                        let _ = service.send(json!({"op": "cancel", "id": id})).await;
                    }
                }
            }
        }
    }
    // Long enough for the clipboard to be put back and seen.
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    service.stop().await;
}
