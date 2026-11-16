//! Which brain a Mac uses when that's left to Buntini: Apple Intelligence when it's ready,
//! else the user's ChatGPT plan, else a local model. A brain the user picked in Settings is
//! never moved. A running task keeps its settings snapshot, and `save_settings_in` refuses
//! a change while one runs, so a brain only ever changes between tasks.
use crate::{controller::AppState, types::Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppleState {
    Ready,
    NotReady,
    /// Not asked yet this launch.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Not signed in.
    None,
    Usable,
    /// Signed in, but the account can't use its plan in other apps.
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brain {
    Apple,
    ChatGpt,
    Local,
}

/// Where a request from ChatGPT saying this account may not use its plan is remembered,
/// until the next sign-in or sign-out.
pub const PLAN_REFUSED: &str = "chatgpt-plan-refused";

/// The brain the rule gives, or None to leave things as they are: nothing usable yet, or
/// Apple Intelligence not asked yet.
pub fn pick(apple: AppleState, plan: Plan, local_ready: bool) -> Option<Brain> {
    match apple {
        AppleState::Ready => Some(Brain::Apple),
        AppleState::Unknown => None,
        AppleState::NotReady if plan == Plan::Usable => Some(Brain::ChatGpt),
        AppleState::NotReady if local_ready => Some(Brain::Local),
        AppleState::NotReady => None,
    }
}

/// The brain these settings use, if the rule could have chosen it (an API key it couldn't).
fn current(settings: &Settings) -> Option<Brain> {
    match settings.model_backend.as_str() {
        "apple" => Some(Brain::Apple),
        "mlx" => Some(Brain::Local),
        "cloud" if settings.cloud_provider == "chatgpt" => Some(Brain::ChatGpt),
        _ => None,
    }
}

pub(crate) fn with(settings: &Settings, brain: Brain) -> Settings {
    let mut next = settings.clone();
    match brain {
        Brain::Apple => next.model_backend = "apple".into(),
        Brain::Local => next.model_backend = "mlx".into(),
        Brain::ChatGpt => {
            next.model_backend = "cloud".into();
            if next.cloud_provider != "chatgpt" {
                next.cloud_provider = "chatgpt".into();
                next.cloud_model.clear();
                next.cloud_input_price = 0.0;
                next.cloud_output_price = 0.0;
            }
        }
    }
    next
}

fn announcement(from: Option<Brain>, to: Brain) -> String {
    match (from, to) {
        (_, Brain::Apple) => "Now using Apple Intelligence.".into(),
        (Some(Brain::Apple), Brain::ChatGpt) => "Apple Intelligence isn’t available, so I’m using ChatGPT.".into(),
        (Some(Brain::Apple), Brain::Local) => "Apple Intelligence isn’t available, so I’m using the model on this Mac.".into(),
        (_, Brain::ChatGpt) => "Now using ChatGPT.".into(),
        (_, Brain::Local) => "Now using the model on this Mac.".into(),
    }
}

/// What this Mac's sign-in allows, from what was noted when it was made. The sign-in itself
/// stays unread in the Keychain until it's used.
pub fn plan(state: &AppState) -> Plan {
    let store = state.store.lock().unwrap();
    let refused = store.preference(PLAN_REFUSED).ok().flatten().is_some_and(|value| !value.is_empty());
    plan_of(crate::chatgpt::saved().then(|| crate::chatgpt::plan_noted(&store)), refused)
}

fn plan_of(signed_in_with_plan_scope: Option<bool>, refused: bool) -> Plan {
    match signed_in_with_plan_scope {
        None => Plan::None,
        Some(true) if !refused => Plan::Usable,
        Some(_) => Plan::Refused,
    }
}

/// Apple Intelligence still downloading, or its helper failing this once, says nothing
/// about whether this Mac has it.
fn passing(reason: &str) -> bool {
    ["modelNotReady", "helperFailed"].contains(&reason)
}

fn apple_state(state: &AppState) -> AppleState {
    match state.models.apple_status() {
        None => AppleState::Unknown,
        Some((true, _)) => AppleState::Ready,
        // A passing problem doesn't send anyone's words to a cloud model.
        Some((false, reason)) if passing(&reason) => AppleState::Unknown,
        Some((false, _)) => AppleState::NotReady,
    }
}

/// The rule applied to these settings: unchanged when the user chose the brain, before
/// setup has finished (it chooses), or when the rule has nothing better.
pub fn resolved(state: &AppState, settings: Settings) -> Settings {
    if !settings.brain_auto || !state.store.lock().unwrap().onboarded().unwrap_or(false) {
        return settings;
    }
    let apple = apple_state(state);
    let plan = if apple == AppleState::NotReady { plan(state) } else { Plan::None };
    match pick(apple, plan, crate::model::local_ready(&settings)) {
        Some(brain) if current(&settings) != Some(brain) => with(&settings, brain),
        _ => settings,
    }
}

/// Moves to the brain the rule gives now. Returns what to tell the user when it did. A task
/// holding the settings leaves things as they are until the next chance.
pub async fn settle(state: &AppState) -> Option<String> {
    let settings = state.store.lock().unwrap().settings().ok()?;
    let next = resolved(state, settings.clone());
    if next.model_backend == settings.model_backend && next.cloud_provider == settings.cloud_provider {
        return None;
    }
    let (from, to) = (current(&settings), current(&next)?);
    crate::controller::save_settings_in(state, next).await.ok()?;
    state.audit("brain.switched", &format!("{to:?}"));
    Some(announcement(from, to))
}

/// Signing in is the user saying ChatGPT may answer, so while Apple Intelligence isn't
/// ready, for whatever reason, it does.
pub async fn settle_after_sign_in(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let state = app.state::<AppState>();
    let Ok(settings) = state.store.lock().unwrap().settings() else { return };
    let waiting = matches!(state.models.apple_status(), Some((false, reason)) if passing(&reason));
    let onboarded = state.store.lock().unwrap().onboarded().unwrap_or(false);
    if waiting && onboarded && settings.brain_auto && plan(&state) == Plan::Usable && current(&settings) != Some(Brain::ChatGpt) {
        let from = current(&settings);
        if crate::controller::save_settings_in(&state, with(&settings, Brain::ChatGpt)).await.is_ok() {
            state.audit("brain.switched", "ChatGpt");
            let _ = app.emit("brain-changed", announcement(from, Brain::ChatGpt));
        }
        return;
    }
    settle_and_tell(app).await;
}

/// `settle`, telling the app about any change.
pub async fn settle_and_tell(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    if let Some(notice) = settle(&app.state::<AppState>()).await {
        let _ = app.emit("brain-changed", notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppleState::{NotReady, Ready, Unknown};

    #[test]
    fn apple_intelligence_then_chatgpt_then_a_local_model() {
        for plan in [Plan::None, Plan::Usable, Plan::Refused] {
            for local in [false, true] {
                assert_eq!(pick(Ready, plan, local), Some(Brain::Apple), "{plan:?} {local}");
                assert_eq!(pick(Unknown, plan, local), None, "not asked yet: nothing moves");
            }
        }
        assert_eq!(pick(NotReady, Plan::Usable, false), Some(Brain::ChatGpt));
        assert_eq!(pick(NotReady, Plan::Usable, true), Some(Brain::ChatGpt), "the plan comes before the local model");
        assert_eq!(pick(NotReady, Plan::None, true), Some(Brain::Local));
        assert_eq!(pick(NotReady, Plan::Refused, true), Some(Brain::Local));
        assert_eq!(pick(NotReady, Plan::None, false), None);
        assert_eq!(pick(NotReady, Plan::Refused, false), None);
    }

    #[test]
    fn what_sign_in_returned_decides_the_plan() {
        assert_eq!(plan_of(None, false), Plan::None);
        assert_eq!(plan_of(None, true), Plan::None, "signed out, nothing is refused");
        assert_eq!(plan_of(Some(true), false), Plan::Usable);
        assert_eq!(plan_of(Some(false), false), Plan::Refused, "the plan scope wasn't granted");
        assert_eq!(plan_of(Some(true), true), Plan::Refused, "ChatGPT said no to a request");
    }

    #[test]
    fn moving_to_chatgpt_uses_the_plan_and_keeps_what_else_was_set() {
        let settings = Settings { cloud_provider: "anthropic".into(), cloud_model: "claude-old".into(), cloud_budget_usd: 9.0, ..Settings::default() };
        let next = with(&settings, Brain::ChatGpt);
        assert_eq!((next.model_backend.as_str(), next.cloud_provider.as_str(), next.cloud_model.as_str()), ("cloud", "chatgpt", ""));
        assert_eq!(next.cloud_budget_usd, 9.0);
        assert_eq!(current(&next), Some(Brain::ChatGpt));
        assert_eq!(with(&next, Brain::Apple).cloud_provider, "chatgpt", "moving back leaves the sign-in's provider alone");
        assert_eq!(current(&Settings { model_backend: "cloud".into(), cloud_provider: "openai".into(), ..Settings::default() }), None, "an API key isn't the rule's choice");
    }

    #[test]
    fn each_move_is_said_once_in_plain_words() {
        assert_eq!(announcement(Some(Brain::ChatGpt), Brain::Apple), "Now using Apple Intelligence.");
        assert_eq!(announcement(Some(Brain::Apple), Brain::ChatGpt), "Apple Intelligence isn’t available, so I’m using ChatGPT.");
        assert_eq!(announcement(Some(Brain::Apple), Brain::Local), "Apple Intelligence isn’t available, so I’m using the model on this Mac.");
        assert_eq!(announcement(Some(Brain::Local), Brain::ChatGpt), "Now using ChatGPT.");
    }

    fn app(dir: &std::path::Path, settings: Settings, onboarded: bool) -> AppState {
        let state = AppState::new(dir.join("data"), dir.join("cache"), dir.into()).unwrap();
        state.store.lock().unwrap().save_settings(&settings).unwrap();
        state.store.lock().unwrap().set_onboarded(onboarded).unwrap();
        state
    }
    fn saved(state: &AppState) -> String {
        state.store.lock().unwrap().settings().unwrap().model_backend
    }
    fn elsewhere() -> Settings {
        Settings { model_backend: "mlx".into(), ..Settings::default() }
    }

    #[tokio::test]
    async fn apple_intelligence_takes_over_between_tasks_and_says_so_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = app(dir.path(), elsewhere(), true);
        assert_eq!(settle(&state).await, None, "not asked yet: nothing moves");
        assert_eq!(saved(&state), "mlx");

        state.models.set_apple_status(true, "available".into());
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        state.active.lock().unwrap().insert("task".into(), running);
        assert_eq!(settle(&state).await, None, "never during a task");
        assert_eq!(saved(&state), "mlx");

        state.active.lock().unwrap().clear();
        assert_eq!(settle(&state).await.as_deref(), Some("Now using Apple Intelligence."));
        assert_eq!(saved(&state), "apple");
        assert_eq!(settle(&state).await, None, "said once");
    }

    #[tokio::test]
    async fn a_brain_the_user_picked_and_setup_itself_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let state = app(dir.path(), Settings { brain_auto: false, ..elsewhere() }, true);
        state.models.set_apple_status(true, "available".into());
        assert_eq!(settle(&state).await, None);
        assert_eq!(saved(&state), "mlx");
        assert_eq!(resolved(&state, Settings { brain_auto: false, ..elsewhere() }).model_backend, "mlx");

        let dir = tempfile::tempdir().unwrap();
        let state = app(dir.path(), elsewhere(), false);
        state.models.set_apple_status(true, "available".into());
        assert_eq!(settle(&state).await, None, "setup chooses until it's finished");
        assert_eq!(saved(&state), "mlx");
    }

    #[tokio::test]
    async fn saving_a_stale_screen_keeps_the_brain_the_rule_chose() {
        let dir = tempfile::tempdir().unwrap();
        let state = app(dir.path(), Settings::default(), true);
        state.models.set_apple_status(true, "available".into());
        assert_eq!(resolved(&state, elsewhere()).model_backend, "apple", "left to Buntini");
        assert_eq!(resolved(&state, Settings { brain_auto: false, ..elsewhere() }).model_backend, "mlx", "the pick that turned it off");
    }

    #[test]
    fn only_a_lasting_reason_counts_as_no_apple_intelligence() {
        assert!(passing("modelNotReady") && passing("helperFailed"));
        assert!(!passing("deviceNotEligible") && !passing("appleIntelligenceNotEnabled") && !passing("unavailable"));
    }

    #[test]
    fn a_new_install_is_automatic_and_an_existing_one_keeps_its_brain() {
        assert!(Settings::default().brain_auto);
        let existing: Settings = serde_json::from_str(r#"{"modelBackend":"cloud","cloudProvider":"openai"}"#).unwrap();
        assert!(!existing.brain_auto, "settings saved before this field was added");
        let saved = serde_json::to_string(&Settings::default()).unwrap();
        assert!(serde_json::from_str::<Settings>(&saved).unwrap().brain_auto, "it survives saving");
    }
}
