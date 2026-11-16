//! Ties dictation together: keys from the native watcher go through `keys`, the audio helper
//! listens and recognizes, `text` shapes the words, and they're pasted where the caret is.
//! Also runs call notes and the wake word. One task owns all of it; slow work (the brain,
//! pasting) runs beside it and reports back.
use super::{history, keys, models, native, notes, service::{self, Service}, text, wake, Choices, Op, Settings};
use crate::{
    controller::AppState,
    privacy::{Locality, Share},
    surface::{
        self,
        cards::flow::{FlowPill, FlowState},
        Action, Card, CardBody, Contribution, Offer, Pill, PillState, Source, Urgency,
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

/// How long Done or Copied stays on the pill.
const SHOWN_MS: u64 = 1_400;
/// Longest wait for the helper's words after the microphone closes.
const FINAL_WAIT: Duration = Duration::from_secs(20);
/// Longest wait for the brain's clean-up; after that the words go as they are.
const CLEAN_UP_WAIT: Duration = Duration::from_secs(8);
/// A hands-free dictation closes by itself after this long.
const MOST_MS: u64 = 10 * 60 * 1_000;
const CHOICES: &str = "flow-choices";
const MUTED: &str = "flow-muted-outputs";

enum Input {
    Native(native::Event),
    Pill(Action),
    Helper(Value),
    Call(Op, oneshot::Sender<Result<Value, String>>),
    /// Where the words of dictation `id` will go, found after it started.
    Target { id: u64, focus: native::Focus },
    Finished { outcome: Outcome },
    /// Takes Done or Copied off the pill.
    Settle { generation: u64 },
    NoteSaved { id: String },
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Pasted(String),
    Copied(String),
    /// Nothing was said.
    Empty,
    Failed(String),
}

static INPUT: OnceLock<mpsc::UnboundedSender<Input>> = OnceLock::new();

fn send(input: Input) {
    if let Some(sender) = INPUT.get() {
        let _ = sender.send(input);
    }
}

/// Called by the native key watcher on the main thread: copy and hand over, never block.
extern "C" fn watched(bytes: *const u8, length: usize) {
    // SAFETY: Swift passes a buffer it keeps alive until this returns.
    if let Some(event) = unsafe { native::read(bytes, length) } {
        send(Input::Native(event));
    }
}

/// Starts dictation's runtime. Nothing listens until dictation, the wake word or call notes
/// are switched on.
pub fn start(app: AppHandle) {
    let (sender, receiver) = mpsc::unbounded_channel();
    if INPUT.set(sender).is_err() {
        return;
    }
    let (events, mut helper_events) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = helper_events.recv().await {
            send(Input::Helper(event));
        }
    });
    let state = app.state::<AppState>();
    let service = Arc::new(Service::new(state.voice.helper().to_path_buf(), models::folder(&state.data_dir), events));
    surface::on_action(Source::Flow, |action| send(Input::Pill(action)));
    let mut runtime = Runtime::new(app.clone(), service);
    tauri::async_runtime::spawn(async move {
        runtime.begin().await;
        runtime.run(receiver).await;
    });
}

/// The command's work. Reads go straight to the database; anything that changes what is
/// listening goes through the runtime.
pub async fn call(app: AppHandle, op: Op) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let base = models::folder(&state.data_dir);
    match op {
        Op::Ping => Ok(Value::Bool(true)),
        Op::Language => Ok(Value::String(crate::i18n::language().into())),
        Op::Stats => {
            let today = chrono::Local::now().date_naive();
            let stats = history::stats(&state.store.lock().unwrap().db, today)?;
            Ok(json!(stats))
        }
        Op::History { limit } => Ok(json!(history::recent(&state.store.lock().unwrap().db, limit.unwrap_or(100).min(500))?)),
        Op::ForgetDictation { id } => history::forget(&state.store.lock().unwrap().db, id).map(|_| Value::Null),
        Op::Notes => Ok(json!(history::notes(&state.store.lock().unwrap().db)?)),
        Op::DeleteNote { id } => history::delete_note(&state.store.lock().unwrap().db, &id).map(|_| Value::Null),
        Op::Choices => Ok(json!(choices(&state))),
        Op::DownloadModels => {
            let free = crate::local_setup::free_bytes(&state.data_dir);
            let notify = app.clone();
            let tell: Arc<dyn Fn(models::Status) + Send + Sync> = Arc::new(move |status| {
                let _ = notify.emit("flow-models", status);
            });
            state.audit("speech_model.download", "Started downloading the speech model from Hugging Face");
            let result = models::download(base, free, tell).await;
            if result.is_ok() {
                // The helper picks Parakeet up without a restart.
                let (reply, _) = oneshot::channel();
                send(Input::Call(Op::Apply, reply));
            }
            result.map(|_| Value::Null)
        }
        Op::CancelDownload => {
            models::cancel();
            Ok(Value::Null)
        }
        Op::OpenLibrary => open_library(&app).map(|_| Value::Null),
        op => {
            let (reply, answer) = oneshot::channel();
            send(Input::Call(op, reply));
            answer.await.map_err(|_| "Dictation isn’t running.".to_string())?
        }
    }
}

/// The Library window: notes, dictation history and statistics. It may only call `flow_call`.
fn open_library(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("library") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(app, "library", tauri::WebviewUrl::App("index.html".into()))
        .title("Library")
        .inner_size(860.0, 620.0)
        .min_inner_size(560.0, 420.0)
        .on_navigation(|url| crate::security::ui_navigation_allowed(url, cfg!(dev)))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        .map(|window| {
            let _ = window.set_focus();
        })
        .map_err(|_| "The Library couldn’t open.".to_string())
}

fn choices(state: &AppState) -> Choices {
    state.store.lock().unwrap().preference(CHOICES).ok().flatten().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

fn save_choices(state: &AppState, choices: &Choices) -> Result<(), String> {
    choices.validate()?;
    let raw = serde_json::to_string(choices).map_err(|e| e.to_string())?;
    state.store.lock().unwrap().set_preference(CHOICES, &raw)
}

struct Dictation {
    id: u64,
    started: Instant,
    started_ms: u64,
    focus: native::Focus,
    locked: bool,
    text: Option<String>,
}

struct Recording {
    id: String,
    since: f64,
    stopping: bool,
}

struct Runtime {
    app: AppHandle,
    service: Arc<Service>,
    machine: keys::Machine,
    clock: Instant,
    next_id: u64,
    settings: Settings,
    dictation: Option<Dictation>,
    /// Finished dictations waiting for their words.
    waiting: HashMap<u64, oneshot::Sender<Value>>,
    generation: u64,
    watching: bool,
    /// When to try installing the key watcher again (Accessibility may have been allowed).
    retry_at: Option<Instant>,
    parakeet: bool,
    conditions: wake::Conditions,
    wake_on: bool,
    /// A wake word started a voice turn; listen again once it is over.
    wake_check_at: Option<Instant>,
    heard: u64,
    woke: u64,
    recording: Option<Recording>,
    /// The last thing typed, for ⌃⌘V.
    last: Option<String>,
    /// The app of the last dictation, for picking its style from the pill.
    last_bundle: Option<(String, String)>,
}

impl Runtime {
    fn new(app: AppHandle, service: Arc<Service>) -> Self {
        Self {
            app,
            service,
            machine: keys::Machine::new(),
            clock: Instant::now(),
            next_id: 0,
            settings: Settings::default(),
            dictation: None,
            waiting: HashMap::new(),
            generation: 0,
            watching: false,
            retry_at: None,
            parakeet: false,
            conditions: wake::Conditions::default(),
            wake_on: false,
            wake_check_at: None,
            heard: 0,
            woke: 0,
            recording: None,
            last: None,
            last_bundle: None,
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    async fn begin(&mut self) {
        // A crash while muted is undone now.
        let pending: Vec<String> = self.state().store.lock().unwrap().preference(MUTED).ok().flatten().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
        if !pending.is_empty() {
            let _ = self.service.send(json!({"op": "unmute", "devices": pending})).await;
        }
        // A call that was recording or being written up when Buntini closed won't finish.
        {
            let state = self.state();
            let store = state.store.lock().unwrap();
            for mut note in history::notes(&store.db).unwrap_or_default() {
                if note.state == "recording" || note.state == "processing" {
                    note.state = "failed".into();
                    let _ = history::save_note(&store.db, &note);
                }
            }
        }
        remove_audio(&self.notes_folder(), None);
        self.apply().await;
    }

    async fn run(&mut self, mut inputs: mpsc::UnboundedReceiver<Input>) {
        loop {
            let wake_at = [self.retry_at, self.wake_check_at].into_iter().flatten().min();
            let key_at = self.machine.next_deadline().map(|at| self.clock + Duration::from_millis(at));
            let limit_at = self.dictation.as_ref().filter(|dictation| dictation.locked).map(|dictation| dictation.started + Duration::from_millis(MOST_MS));
            let next = [wake_at, key_at, limit_at].into_iter().flatten().min();
            let input = match next {
                Some(at) => tokio::select! {
                    input = inputs.recv() => input,
                    _ = tokio::time::sleep_until(at.into()) => { self.timers().await; continue; }
                },
                None => inputs.recv().await,
            };
            let Some(input) = input else { break };
            self.handle(input).await;
        }
    }

    async fn timers(&mut self) {
        let now = self.now_ms();
        if let Some(command) = self.machine.tick(now) {
            self.command(command).await;
        }
        let instant = Instant::now();
        if self.retry_at.is_some_and(|at| at <= instant) {
            self.retry_at = None;
            self.watch();
        }
        if self.wake_check_at.is_some_and(|at| at <= instant) {
            self.wake_check_at = None;
            let busy = self.state().voice.busy();
            self.conditions.microphone_busy = busy || self.recording.is_some();
            if busy {
                self.wake_check_at = Some(instant + Duration::from_secs(2));
            }
            self.reconcile_wake().await;
        }
        if self.dictation.as_ref().is_some_and(|dictation| dictation.locked && dictation.started.elapsed() >= Duration::from_millis(MOST_MS)) {
            self.machine.ended();
            self.command(keys::Command::Finish).await;
        }
    }

    async fn handle(&mut self, input: Input) {
        match input {
            Input::Native(native::Event::Key { key }) => {
                let now = self.now_ms();
                for command in self.machine.event(key.into(), now) {
                    self.command(command).await;
                }
            }
            Input::Native(native::Event::PasteLast) => self.paste_last(),
            Input::Native(native::Event::Reset) => {
                if let Some(command) = self.machine.reset() {
                    self.command(command).await;
                }
            }
            Input::Native(native::Event::System { asleep, locked, low_power }) => {
                self.conditions.asleep = asleep;
                self.conditions.screen_locked = locked;
                self.conditions.low_power = low_power;
                if (asleep || locked) && self.machine.listening() {
                    if let Some(command) = self.machine.reset() {
                        self.command(command).await;
                    }
                }
                self.reconcile_wake().await;
            }
            Input::Pill(action) => self.pill_action(action).await,
            Input::Helper(event) => self.helper(event).await,
            Input::Call(op, reply) => {
                let answer = self.op(op).await;
                let _ = reply.send(answer);
            }
            Input::Target { id, focus } => {
                if let Some(dictation) = self.dictation.as_mut().filter(|dictation| dictation.id == id) {
                    dictation.focus = focus;
                    self.show_listening();
                }
            }
            Input::Finished { outcome } => self.finished(outcome),
            Input::Settle { generation } => {
                if generation == self.generation && self.dictation.is_none() {
                    self.show(self.idle_pill());
                }
            }
            Input::NoteSaved { id } => {
                let _ = self.app.emit("flow-notes", id);
            }
        }
    }

    // MARK: Settings

    async fn apply(&mut self) {
        let saved = self.state().store.lock().unwrap().settings().map(|settings| settings.flow).unwrap_or_default();
        self.settings = saved;
        if self.settings.enabled || self.settings.wake_word {
            self.watch();
        } else {
            native::unwatch();
            self.watching = false;
            self.retry_at = None;
        }
        if self.settings.enabled {
            // Loads Parakeet now, so the first dictation doesn't wait for it.
            let _ = self.service.send(json!({"op": "load"})).await;
        } else if let Some(command) = self.machine.reset() {
            self.command(command).await;
        }
        self.conditions.enabled = self.settings.wake_word;
        self.reconcile_wake().await;
        if !self.settings.enabled && !self.settings.wake_word && self.recording.is_none() && self.service.running().await {
            self.service.stop().await;
            self.wake_on = false;
        }
        if self.dictation.is_none() {
            self.show(self.idle_pill());
        }
    }

    fn watch(&mut self) {
        let config = native::WatchConfig { keys: self.settings.enabled, key: self.settings.key.id().into(), chord: self.settings.chord.clone() };
        let installed = native::watch(watched, &config);
        self.watching = installed && self.settings.enabled;
        // Accessibility isn't allowed yet: the key watcher goes in as soon as it is.
        self.retry_at = (!installed).then(|| Instant::now() + Duration::from_secs(3));
    }

    // MARK: Dictation

    async fn command(&mut self, command: keys::Command) {
        match command {
            keys::Command::Start => self.open().await,
            keys::Command::Lock => {
                if let Some(dictation) = self.dictation.as_mut() {
                    dictation.locked = true;
                }
                self.show_listening();
            }
            keys::Command::Finish => self.close().await,
            keys::Command::Cancel => self.cancel().await,
        }
    }

    async fn open(&mut self) {
        if self.dictation.is_some() {
            return;
        }
        self.next_id += 1;
        let id = self.next_id;
        let languages = self.settings.languages.clone();
        if let Err(message) = self.service.send(json!({"op": "dictate", "id": id, "languages": languages})).await {
            self.machine.ended();
            self.flash(FlowState::Error, Some(message));
            return;
        }
        if self.settings.mute {
            let _ = self.service.send(json!({"op": "mute"})).await;
        }
        self.dictation = Some(Dictation { id, started: Instant::now(), started_ms: self.now_ms(), focus: native::Focus::default(), locked: false, text: None });
        self.conditions.dictating = true;
        self.reconcile_wake().await;
        self.show_listening();
        tauri::async_runtime::spawn(async move {
            if let Ok(focus) = tokio::task::spawn_blocking(native::focus).await {
                send(Input::Target { id, focus });
            }
        });
    }

    async fn after_listening(&mut self) {
        if self.settings.mute {
            let _ = self.service.send(json!({"op": "unmute"})).await;
        }
        self.conditions.dictating = false;
        self.reconcile_wake().await;
    }

    async fn cancel(&mut self) {
        let Some(dictation) = self.dictation.take() else { return };
        let _ = self.service.send(json!({"op": "cancel", "id": dictation.id})).await;
        self.after_listening().await;
        self.show(self.idle_pill());
    }

    async fn close(&mut self) {
        let Some(dictation) = self.dictation.take() else { return };
        let (reply, words) = oneshot::channel();
        self.waiting.insert(dictation.id, reply);
        let _ = self.service.send(json!({"op": "stop", "id": dictation.id})).await;
        self.after_listening().await;
        let speaking_ms = self.now_ms().saturating_sub(dictation.started_ms);
        self.show(Some(FlowPill { state: FlowState::Working, app: Some(dictation.focus.app.clone()).filter(|app| !app.is_empty()), ..self.base_pill() }));
        self.last_bundle = Some((dictation.focus.bundle.clone(), dictation.focus.app.clone())).filter(|(bundle, _)| !bundle.is_empty());
        let app = self.app.clone();
        let flow = self.settings.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = match tokio::time::timeout(FINAL_WAIT, words).await {
                Ok(Ok(event)) => deliver(&app, &flow, event, dictation.focus, speaking_ms).await,
                _ => Outcome::Failed("The words didn’t come back. Try again.".into()),
            };
            send(Input::Finished { outcome });
        });
    }

    fn finished(&mut self, outcome: Outcome) {
        let (state, text) = match outcome {
            Outcome::Pasted(typed) => {
                self.last = Some(typed);
                (FlowState::Done, None)
            }
            Outcome::Copied(typed) => {
                self.last = Some(typed);
                (FlowState::Copied, None)
            }
            Outcome::Empty => (FlowState::Idle, None),
            Outcome::Failed(message) => (FlowState::Error, Some(message)),
        };
        if self.dictation.is_some() {
            return;
        }
        if state == FlowState::Idle {
            self.show(self.idle_pill());
        } else {
            self.flash(state, text);
        }
        let _ = self.app.emit("flow-history", ());
    }

    fn paste_last(&self) {
        let last = self.last.clone().or_else(|| history::last_text(&self.state().store.lock().unwrap().db).ok().flatten());
        let Some(text) = last else { return };
        std::thread::spawn(move || {
            let focus = native::focus();
            if native::should_paste(&focus) {
                native::paste(&text);
            } else {
                native::copy(&text);
            }
        });
    }

    // MARK: The pill

    fn base_pill(&self) -> FlowPill {
        let choices = choices(&self.state());
        let style = self.last_bundle.as_ref().map(|(bundle, _)| text::style_for(bundle, &choices.app_styles));
        FlowPill {
            state: FlowState::Idle,
            text: None,
            app: self.last_bundle.as_ref().map(|(_, name)| name.clone()),
            locked: false,
            style,
            mode: Some(choices.mode),
            recording: self.recording.as_ref().map(|recording| recording.since),
        }
    }

    /// The pill when nothing is happening: there while dictation is on or a call records.
    fn idle_pill(&self) -> Option<FlowPill> {
        (self.settings.enabled || self.recording.is_some()).then(|| self.base_pill())
    }

    fn show_listening(&mut self) {
        let Some(dictation) = self.dictation.as_ref() else { return };
        let pill = FlowPill {
            state: FlowState::Listening,
            text: dictation.text.clone(),
            app: Some(dictation.focus.app.clone()).filter(|app| !app.is_empty()),
            locked: dictation.locked,
            ..self.base_pill()
        };
        self.show(Some(pill));
    }

    fn show(&mut self, pill: Option<FlowPill>) {
        self.generation += 1;
        surface::set_flow(pill);
    }

    /// Shows a result for a moment, then the idle pill.
    fn flash(&mut self, state: FlowState, text: Option<String>) {
        let pill = FlowPill { state, text, ..self.base_pill() };
        self.show(Some(pill));
        let generation = self.generation;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(if state == FlowState::Error { SHOWN_MS * 3 } else { SHOWN_MS })).await;
            send(Input::Settle { generation });
        });
    }

    async fn pill_action(&mut self, action: Action) {
        match action.name.as_str() {
            // The toolbar's microphone: hands-free until pressed again.
            "dictate" => {
                if self.dictation.is_some() {
                    self.machine.ended();
                    self.close().await;
                } else {
                    self.open().await;
                    if let Some(dictation) = self.dictation.as_mut() {
                        dictation.locked = true;
                    }
                    self.show_listening();
                }
            }
            "cancel" => {
                self.machine.ended();
                self.cancel().await;
            }
            "record" => {
                let result = if self.recording.is_some() { self.stop_notes().await } else { self.start_notes().await };
                if let Err(message) = result {
                    self.flash(FlowState::Error, Some(message));
                }
            }
            "open" => {
                crate::focus::capture();
                if let Some(window) = self.app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = self.app.emit("companion-toggle", ());
                }
            }
            "library" => {
                let _ = open_library(&self.app);
            }
            "style" => {
                let Some((bundle, _)) = self.last_bundle.clone() else { return };
                let style = action.value.and_then(|value| serde_json::from_value::<text::Style>(value).ok());
                let _ = self.op(Op::SetStyle { bundle, style }).await;
            }
            "mode" => {
                if let Some(mode) = action.value.and_then(|value| serde_json::from_value::<text::Mode>(value).ok()) {
                    let _ = self.op(Op::Choose { mode, translate_to: None }).await;
                }
            }
            _ => {}
        }
    }

    // MARK: Commands from the UI

    async fn op(&mut self, op: Op) -> Result<Value, String> {
        match op {
            Op::Apply => {
                self.apply().await;
                Ok(Value::Null)
            }
            Op::Status => {
                let state = self.state();
                let base = models::folder(&state.data_dir);
                Ok(json!({
                    "enabled": self.settings.enabled,
                    "watching": self.watching,
                    "accessibility": crate::ax::trusted(false),
                    "engine": if models::ready(&base) { "parakeet" } else { "apple" },
                    "models": models::status(&base),
                    "recording": self.recording.as_ref().map(|recording| recording.since),
                    "wake": self.wake_on,
                }))
            }
            Op::StartNotes => self.start_notes().await.map(|_| Value::Null),
            Op::StopNotes => self.stop_notes().await.map(|_| Value::Null),
            Op::RemoveModels => {
                let base = models::folder(&self.state().data_dir);
                // The helper lets go of Parakeet first.
                if self.recording.is_none() && self.dictation.is_none() {
                    self.service.stop().await;
                    self.wake_on = false;
                }
                models::remove(&base)?;
                self.parakeet = false;
                self.apply().await;
                Ok(Value::Null)
            }
            Op::Choose { mode, translate_to } => {
                let state = self.state();
                let mut current = choices(&state);
                current.mode = mode;
                if let Some(language) = translate_to {
                    current.translate_to = language.trim().to_string();
                }
                save_choices(&state, &current)?;
                drop(state);
                if self.dictation.is_none() {
                    self.show(self.idle_pill());
                }
                Ok(json!(current))
            }
            Op::SetStyle { bundle, style } => {
                let state = self.state();
                let mut current = choices(&state);
                match style {
                    Some(style) => current.app_styles.insert(bundle, style),
                    None => current.app_styles.remove(&bundle),
                };
                save_choices(&state, &current)?;
                drop(state);
                if self.dictation.is_none() {
                    self.show(self.idle_pill());
                }
                Ok(json!(current))
            }
            _ => Err("Not here.".into()),
        }
    }

    // MARK: The helper

    async fn helper(&mut self, event: Value) {
        let id = event.get("id").and_then(Value::as_u64);
        match service::name(&event) {
            "ready" => {
                if event.get("protocolVersion").and_then(Value::as_u64) != Some(service::PROTOCOL) {
                    eprintln!("The audio helper speaks another protocol; dictation may not work");
                }
                self.parakeet = event.get("parakeet").and_then(Value::as_bool).unwrap_or(false);
                // A helper that started again isn't listening for the wake word yet.
                self.wake_on = false;
                self.reconcile_wake().await;
            }
            "partial" => {
                if let (Some(dictation), Some(text)) = (self.dictation.as_mut(), event.get("text").and_then(Value::as_str)) {
                    if Some(dictation.id) == id {
                        let tail: String = text.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
                        dictation.text = Some(tail);
                        self.show_listening();
                    }
                }
            }
            "final" | "cancelled" => {
                if let Some(reply) = id.and_then(|id| self.waiting.remove(&id)) {
                    let _ = reply.send(event);
                }
            }
            "error" => {
                if let Some(reply) = id.and_then(|id| self.waiting.remove(&id)) {
                    let _ = reply.send(event);
                } else if self.dictation.as_ref().is_some_and(|dictation| Some(dictation.id) == id) {
                    self.dictation = None;
                    self.machine.ended();
                    self.after_listening().await;
                    let message = event.get("message").and_then(Value::as_str).unwrap_or("The microphone didn’t open.").to_string();
                    self.flash(FlowState::Error, Some(message));
                }
            }
            "muted" => {
                let devices = event.get("devices").cloned().unwrap_or_else(|| json!([]));
                let empty = devices.as_array().is_none_or(Vec::is_empty);
                let state = self.state();
                let store = state.store.lock().unwrap();
                let _ = if empty { store.set_preference(MUTED, "[]") } else { store.set_preference(MUTED, &devices.to_string()) };
            }
            "unmuted" => {
                let state = self.state();
                let _ = state.store.lock().unwrap().set_preference(MUTED, "[]");
            }
            "wake.heard" => {
                self.heard += 1;
                let text = event.get("text").and_then(Value::as_str).unwrap_or("");
                if wake::heard(text) && wake::may_listen(self.conditions) {
                    self.woke += 1;
                    eprintln!("Wake word: {} of {} stretches of speech", self.woke, self.heard);
                    self.conditions.microphone_busy = true;
                    self.reconcile_wake().await;
                    self.wake_check_at = Some(Instant::now() + Duration::from_secs(4));
                    if let Some(window) = self.app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.emit("voice-wake", ());
                    }
                }
            }
            "notes.recording" => {}
            "notes.live" => {
                let _ = self.app.emit("flow-notes-live", &event);
            }
            "notes.done" => self.notes_done(event),
            "notes.error" => {
                if let Some(recording) = self.recording.take() {
                    self.fail_note(&recording.id);
                }
                self.after_notes().await;
                let message = event.get("message").and_then(Value::as_str).unwrap_or("The call couldn’t be recorded.").to_string();
                self.flash(FlowState::Error, Some(message));
            }
            "exited" => {
                self.wake_on = false;
                self.parakeet = false;
                self.waiting.clear();
                if self.dictation.take().is_some() {
                    self.machine.ended();
                    self.conditions.dictating = false;
                    self.flash(FlowState::Error, Some("The microphone stopped. Try again.".into()));
                }
                if let Some(recording) = self.recording.take() {
                    self.fail_note(&recording.id);
                    remove_audio(&self.notes_folder(), Some(&recording.id));
                    self.after_notes().await;
                }
                // The wake word comes back with the helper, unless it keeps stopping.
                if self.settings.wake_word {
                    self.wake_check_at = Some(Instant::now() + Duration::from_secs(5));
                }
            }
            _ => {}
        }
    }

    // MARK: Wake word

    async fn reconcile_wake(&mut self) {
        self.conditions.microphone_busy = self.conditions.microphone_busy || self.recording.is_some();
        let want = wake::may_listen(self.conditions);
        if want == self.wake_on {
            return;
        }
        let languages = self.settings.languages.clone();
        if self.service.send(json!({"op": "wake", "on": want, "languages": languages})).await.is_ok() {
            self.wake_on = want;
        }
    }

    // MARK: Call notes

    fn notes_folder(&self) -> PathBuf {
        self.state().cache_dir.join("notes")
    }

    async fn start_notes(&mut self) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("A call is already recording.".into());
        }
        if !self.settings.record_calls {
            return Err("Turn on Record calls in Settings first.".into());
        }
        let folder = self.notes_folder();
        std::fs::create_dir_all(&folder).map_err(|_| "Buntini couldn’t make a folder for the recording.".to_string())?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700));
        let id = uuid::Uuid::new_v4().to_string();
        self.service.send(json!({"op": "notes", "id": id, "folder": folder})).await?;
        let now = chrono::Utc::now();
        let title = format!("Call at {}", now.with_timezone(&chrono::Local).format("%-I:%M %p"));
        let note = history::Note { id: id.clone(), title, created_at: now.to_rfc3339(), seconds: 0.0, state: "recording".into(), lines: vec![], summary: String::new(), key_points: vec![], todos: vec![] };
        history::save_note(&self.state().store.lock().unwrap().db, &note)?;
        self.state().audit("microphone.start", "Recording a call: microphone and the Mac's sound");
        self.recording = Some(Recording { id, since: now.timestamp_millis() as f64 / 1000.0, stopping: false });
        self.conditions.microphone_busy = true;
        self.reconcile_wake().await;
        self.publish_recording();
        if self.dictation.is_none() {
            self.show(self.idle_pill());
        }
        let _ = self.app.emit("flow-notes", ());
        Ok(())
    }

    async fn stop_notes(&mut self) -> Result<(), String> {
        let Some(recording) = self.recording.as_mut() else { return Err("No call is recording.".into()) };
        if recording.stopping {
            return Ok(());
        }
        recording.stopping = true;
        let id = recording.id.clone();
        self.service.send(json!({"op": "notesStop", "id": id})).await?;
        let state = self.state();
        let store = state.store.lock().unwrap();
        if let Some(mut note) = history::notes(&store.db)?.into_iter().find(|note| note.id == id) {
            note.state = "processing".into();
            history::save_note(&store.db, &note)?;
        }
        drop(store);
        drop(state);
        self.publish_recording();
        let _ = self.app.emit("flow-notes", ());
        Ok(())
    }

    /// The island shows a call recording the whole time, and its note when ready.
    fn publish_recording(&self) {
        let pills = self
            .recording
            .as_ref()
            .map(|recording| Pill {
                id: "flow-notes".into(),
                source: Source::Flow,
                label: if recording.stopping { "Writing notes".into() } else { "Recording".into() },
                tint: "#E5484D".into(),
                state: if recording.stopping { PillState::Working } else { PillState::Waiting },
                badge: None,
                progress: None,
            })
            .into_iter()
            .collect();
        surface::publish(Source::Flow, Contribution { pills, offers: vec![] });
    }

    async fn after_notes(&mut self) {
        self.conditions.microphone_busy = self.state().voice.busy();
        self.reconcile_wake().await;
        self.publish_recording();
        if self.dictation.is_none() {
            self.show(self.idle_pill());
        }
    }

    fn fail_note(&self, id: &str) {
        let state = self.state();
        let store = state.store.lock().unwrap();
        if let Some(mut note) = history::notes(&store.db).unwrap_or_default().into_iter().find(|note| note.id == id) {
            note.state = "failed".into();
            let _ = history::save_note(&store.db, &note);
        }
        let _ = self.app.emit("flow-notes", ());
    }

    fn notes_done(&mut self, event: Value) {
        let Some(id) = event.get("id").and_then(Value::as_str).map(str::to_string) else { return };
        if self.recording.as_ref().is_some_and(|recording| recording.id == id) {
            self.recording = None;
        }
        let segments: Vec<notes::Segment> = event.get("segments").cloned().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
        let seconds = event.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
        let app = self.app.clone();
        let folder = self.notes_folder();
        tauri::async_runtime::spawn(async move {
            write_up(&app, &id, segments, seconds, &folder).await;
            send(Input::NoteSaved { id });
        });
        // Not awaited here: the runtime keeps answering keys meanwhile.
        let conditions_busy = self.state().voice.busy();
        self.conditions.microphone_busy = conditions_busy;
        self.publish_recording();
        if self.dictation.is_none() {
            self.show(self.idle_pill());
        }
        self.wake_check_at = Some(Instant::now());
    }
}

/// Deletes a call's audio (or every call's, with no id).
fn remove_audio(folder: &std::path::Path, id: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if id.is_none_or(|id| name.starts_with(id)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Lines from the segments, saved; the audio deleted as soon as they are; then the brain's
/// summary, key points and to-dos.
async fn write_up(app: &AppHandle, id: &str, segments: Vec<notes::Segment>, seconds: f64, folder: &std::path::Path) {
    let state = app.state::<AppState>();
    let lines = notes::lines(segments);
    let Some(mut note) = history::notes(&state.store.lock().unwrap().db).unwrap_or_default().into_iter().find(|note| note.id == id) else {
        remove_audio(folder, Some(id));
        return;
    };
    note.lines = lines;
    note.seconds = seconds;
    note.state = if note.lines.is_empty() { "ready".into() } else { "processing".into() };
    let saved = history::save_note(&state.store.lock().unwrap().db, &note);
    remove_audio(folder, Some(id));
    if saved.is_err() || note.lines.is_empty() {
        return;
    }
    let _ = app.emit("flow-notes", id);
    let settings = state.store.lock().unwrap().settings().unwrap_or_default();
    if let Some(brain) = brain_for(&state, &settings) {
        let payload = json!({"messages": [{"role": "system", "content": notes::summary_prompt()}, {"role": "user", "content": notes::transcript(&note.lines)}], "tools": [], "max_tokens": 900});
        let answer = tokio::time::timeout(Duration::from_secs(120), state.models.generate(&brain, payload, Arc::new(AtomicBool::new(false)), |_| {})).await;
        if let Ok(Ok(turn)) = answer {
            if let Some(summary) = notes::read_summary(&turn.text) {
                if !summary.title.is_empty() {
                    note.title = summary.title;
                }
                note.summary = summary.summary;
                note.key_points = summary.key_points;
                note.todos = summary.todos;
            }
        }
    }
    note.state = "ready".into();
    let _ = history::save_note(&state.store.lock().unwrap().db, &note);
    let card = Card {
        id: format!("note-{id}"),
        source: Source::Flow,
        body: CardBody::Note(crate::surface::cards::flow::Note { id: note.id.clone(), title: note.title.clone(), state: note.state.clone(), seconds: Some(note.seconds), lines: note.lines.iter().take(3).cloned().collect() }),
    };
    surface::publish(Source::Flow, Contribution { pills: vec![], offers: vec![Offer { card, urgency: Urgency::Info, at_ms: surface::now_ms(), ttl_ms: Some(30_000) }] });
}

/// The brain that tidies words: on the Mac first. One elsewhere only sees dictation when
/// you let it in Privacy.
fn brain_for(state: &AppState, settings: &crate::types::Settings) -> Option<crate::types::Settings> {
    if state.models.locality(settings) == Locality::OnDevice && state.models.ready(settings).is_ok() {
        return Some(settings.clone());
    }
    if matches!(state.models.apple_status(), Some((true, _))) {
        return Some(crate::types::Settings { model_backend: "apple".into(), ..settings.clone() });
    }
    if crate::model::local_ready(settings) {
        return Some(crate::types::Settings { model_backend: "mlx".into(), ..settings.clone() });
    }
    (crate::privacy::allows(Locality::External, settings, Share::Dictation) && state.models.ready(settings).is_ok()).then(|| settings.clone())
}

/// Turns what was heard into what's typed, then types it.
async fn deliver(app: &AppHandle, flow: &Settings, event: Value, started: native::Focus, speaking_ms: u64) -> Outcome {
    if super::service::name(&event) != "final" {
        return match event.get("message").and_then(Value::as_str) {
            Some(message) => Outcome::Failed(message.to_string()),
            None => Outcome::Empty,
        };
    }
    let heard = event.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if heard.is_empty() {
        return Outcome::Empty;
    }
    let state = app.state::<AppState>();
    // Where the caret is now; the app it started in if the check finds nothing.
    let focus = tokio::task::spawn_blocking(native::focus).await.unwrap_or_default();
    let target = if focus.bundle.is_empty() { started } else { focus };
    let choices = choices(&state);
    let style = text::style_for(&target.bundle, &choices.app_styles);
    let shaped = shape(&heard, flow, style);
    let mut typed = None;
    if let Some(request) = text::request(&shaped.text, style, choices.mode, &choices.translate_to) {
        let settings = state.store.lock().unwrap().settings().unwrap_or_default();
        if let Some(brain) = brain_for(&state, &settings) {
            let payload = json!({"messages": [{"role": "system", "content": request.system}, {"role": "user", "content": request.user}], "tools": [], "max_tokens": (request.limit / 2).max(96)});
            let cancel = Arc::new(AtomicBool::new(false));
            let answer = tokio::time::timeout(CLEAN_UP_WAIT, state.models.generate(&brain, payload, cancel.clone(), |_| {})).await;
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Ok(Ok(turn)) = answer {
                typed = text::accept(&turn.text, &request, &shaped.kept);
            }
        }
    }
    let typed = complete(typed.as_deref().unwrap_or(&shaped.text), &shaped.kept, flow, style);
    if typed.is_empty() {
        return Outcome::Empty;
    }
    let paste = native::should_paste(&target);
    // A capture on screen while you spoke goes in first, then the words.
    let picture = paste.then(|| crate::utilities::capture::for_dictation(Duration::from_millis(speaking_ms + 15_000))).flatten();
    let words = typed.clone();
    let pasted = tokio::task::spawn_blocking(move || {
        if let Some(picture) = picture {
            crate::utilities::capture::paste(&picture);
        }
        if paste && native::paste(&words) {
            return true;
        }
        // Not a text field (or the paste didn't go through): kept on the clipboard instead.
        native::copy(&words);
        false
    })
    .await
    .unwrap_or(false);
    {
        let store = state.store.lock().unwrap();
        let _ = history::record(&store.db, &history::NewDictation { app: &target.app, bundle: &target.bundle, text: &typed, speaking_ms, pasted }, chrono::Utc::now());
    }
    if pasted {
        Outcome::Pasted(typed)
    } else {
        Outcome::Copied(typed)
    }
}

pub(super) struct Shaped {
    pub(super) text: String,
    pub(super) kept: Vec<String>,
}

/// Before the brain: fillers, your spellings and replacements, snippets set aside, spoken
/// line breaks.
pub(super) fn shape(heard: &str, flow: &Settings, style: text::Style) -> Shaped {
    let text = text::tidy(heard);
    let text = text::apply_dictionary(&text, &flow.dictionary);
    let text = text::apply_replacements(&text, &flow.replacements);
    let (text, kept) = text::protect_snippets(&text, &flow.snippets);
    Shaped { text: text::line_breaks(&text, style), kept }
}

/// After the brain: snippets back, your spellings again (the brain may have "corrected"
/// a name), and the style's finish.
pub(super) fn complete(text: &str, kept: &[String], flow: &Settings, style: text::Style) -> String {
    // Finished around the placeholders, so a snippet never gets a capital or full stop added;
    // then your spellings, which win over the style's capitals ("iPhone", not "IPhone").
    let finished = text::finish(text, style);
    let spelled = text::apply_dictionary(&finished, &flow.dictionary);
    text::restore(&spelled, kept).unwrap_or(spelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::{Replacement, Snippet, Style};

    fn flow() -> Settings {
        Settings {
            dictionary: vec!["Buntini".into()],
            replacements: vec![Replacement { from: "my email".into(), to: "sam@example.com".into() }],
            snippets: vec![Snippet { trigger: "my signature".into(), text: "— Sam\nBuntini team".into() }],
            ..Settings::default()
        }
    }

    #[test]
    fn a_dictation_is_shaped_for_its_app_without_a_brain() {
        let flow = flow();
        let shaped = shape("um send it to my email new line thanks my signature", &flow, Style::Polished);
        assert_eq!(shaped.text, "send it to sam@example.com\nThanks ⟦1⟧");
        assert_eq!(complete(&shaped.text, &shaped.kept, &flow, Style::Polished), "Send it to sam@example.com\nThanks — Sam\nBuntini team");
        let terminal = shape("git commit dash m fix buntiny.", &flow, Style::AsSaid);
        assert_eq!(complete(&terminal.text, &terminal.kept, &flow, Style::AsSaid), "git commit dash m fix Buntini");
        let phone = Settings { dictionary: vec!["iPhone".into()], ..Settings::default() };
        let said = shape("iphone notes are synced", &phone, Style::Natural);
        assert_eq!(complete(&said.text, &said.kept, &phone, Style::Natural), "iPhone notes are synced");
    }

    #[test]
    fn the_brains_version_keeps_your_spelling_and_snippets() {
        let flow = flow();
        let shaped = shape("thanks bun tini my signature", &flow, Style::Natural);
        assert_eq!(shaped.text, "thanks Buntini ⟦1⟧");
        // The brain "corrected" the name; your spelling wins.
        assert_eq!(complete("Thanks, Buntiny! ⟦1⟧", &shaped.kept, &flow, Style::Natural), "Thanks, Buntini! — Sam\nBuntini team");
    }

    #[test]
    fn leftover_call_audio_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["abc-mic.caf", "abc-system.caf", "def-mic.caf"] {
            std::fs::write(dir.path().join(name), b"audio").unwrap();
        }
        remove_audio(dir.path(), Some("abc"));
        assert!(dir.path().join("def-mic.caf").exists() && !dir.path().join("abc-mic.caf").exists());
        remove_audio(dir.path(), None);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
