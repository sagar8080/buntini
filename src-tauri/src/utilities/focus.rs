//! The focus timer: focus and rest in cycles, a long rest after the fourth round. Each phase
//! is anchored to a deadline, and the next one starts exactly at the previous deadline, so
//! the cycle never drifts however late the clock is read (a busy thread, a Mac asleep).
//! Buntini picks the lengths; a request can only say how long focus lasts.
use serde::{Deserialize, Serialize};

/// Focus rounds before the long rest.
pub const ROUNDS: u32 = 4;
/// 25 minutes.
pub const FOCUS_MS: i64 = 25 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Focus,
    Rest,
    LongRest,
}

impl Phase {
    pub fn id(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Rest => "rest",
            Self::LongRest => "longRest",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Rest => "Rest",
            Self::LongRest => "Long rest",
        }
    }
}

/// How long each phase lasts, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub focus: i64,
    pub rest: i64,
    pub long_rest: i64,
}

impl Default for Plan {
    fn default() -> Self {
        Self::for_focus(FOCUS_MS)
    }
}

impl Plan {
    /// Rest is a fifth of focus (5 minutes for 25), between 3 and 15 minutes; the long rest is
    /// three rests, between 15 and 30 minutes.
    pub fn for_focus(focus: i64) -> Self {
        let focus = focus.clamp(60_000, 3 * 3_600_000);
        let rest = (focus / 5).clamp(3 * 60_000, 15 * 60_000);
        Self { focus, rest, long_rest: (rest * 3).clamp(15 * 60_000, 30 * 60_000) }
    }

    pub fn length(&self, phase: Phase) -> i64 {
        match phase {
            Phase::Focus => self.focus,
            Phase::Rest => self.rest,
            Phase::LongRest => self.long_rest,
        }
    }
}

/// What reading the clock found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Still in the same phase.
    Same,
    /// A new phase began (maybe several passed while nobody looked; this is the one now).
    Turned,
    /// The long rest ended: the set is over.
    Over,
}

/// A running set. Times are Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub plan: Plan,
    pub phase: Phase,
    /// 1…ROUNDS: the focus round, or the one a rest follows.
    pub round: u32,
    /// When this phase ends; none while paused.
    pub ends_at: Option<i64>,
    /// While paused, what was left.
    pub left: i64,
}

impl Session {
    pub fn start(plan: Plan, now: i64) -> Self {
        Self { plan, phase: Phase::Focus, round: 1, ends_at: Some(now + plan.focus), left: plan.focus }
    }

    pub fn length(&self) -> i64 {
        self.plan.length(self.phase)
    }

    pub fn paused(&self) -> bool {
        self.ends_at.is_none()
    }

    pub fn remaining(&self, now: i64) -> i64 {
        match self.ends_at {
            Some(end) => (end - now).clamp(0, self.length()),
            None => self.left.clamp(0, self.length()),
        }
    }

    pub fn pause(&mut self, now: i64) {
        if let Some(end) = self.ends_at.take() {
            self.left = (end - now).clamp(0, self.length());
        }
    }

    pub fn resume(&mut self, now: i64) {
        if self.ends_at.is_none() {
            self.ends_at = Some(now + self.left);
        }
    }

    /// The phase after this one; none after the long rest.
    fn next(&self) -> Option<(Phase, u32)> {
        match self.phase {
            Phase::Focus if self.round >= ROUNDS => Some((Phase::LongRest, self.round)),
            Phase::Focus => Some((Phase::Rest, self.round)),
            Phase::Rest => Some((Phase::Focus, self.round + 1)),
            Phase::LongRest => None,
        }
    }

    /// Ends this phase at `at` and starts the next one there. False when the set is over.
    fn turn(&mut self, at: i64) -> bool {
        let Some((phase, round)) = self.next() else { return false };
        self.phase = phase;
        self.round = round;
        let length = self.length();
        self.ends_at = Some(at + length);
        self.left = length;
        true
    }

    /// Ends this phase now. A paused phase skipped to starts the next one paused too.
    pub fn skip(&mut self, now: i64) -> Step {
        let paused = self.paused();
        if !self.turn(now) {
            return Step::Over;
        }
        if paused {
            self.pause(now);
        }
        Step::Turned
    }

    /// Moves past every phase whose deadline has gone by, each starting at the one before's.
    pub fn catch_up(&mut self, now: i64) -> Step {
        let mut step = Step::Same;
        while let Some(end) = self.ends_at.filter(|end| *end <= now) {
            if !self.turn(end) {
                return Step::Over;
            }
            step = Step::Turned;
        }
        step
    }
}

/// "25 min", or "<1 min" in the last minute, for the pill.
pub fn badge(remaining_ms: i64) -> String {
    if remaining_ms < 60_000 {
        "<1 min".into()
    } else {
        format!("{} min", (remaining_ms + 59_999) / 60_000)
    }
}

/// When the badge next reads differently: the next whole minute left.
pub fn next_badge_change(remaining_ms: i64) -> i64 {
    if remaining_ms <= 60_000 {
        remaining_ms
    } else {
        let over = (remaining_ms - 1) % 60_000 + 1;
        over.min(remaining_ms)
    }
}

/// What a chat request asks of the focus timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Focus, for this many minutes when said.
    Start { minutes: Option<u64> },
    Pause,
    Resume,
    Stop,
}

/// Narrow on purpose: "focus for 25 minutes", "start a focus timer", "pomodoro", "stop
/// focusing". "Focus mode" is the Mac's Focus, and "help me focus on…" is a question.
pub fn recognise(text: &str) -> Option<Command> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.iter().any(|word| ["don't", "dont", "not", "never", "mode", "how", "why", "what", "when", "can't", "cant"].contains(word)) {
        return None;
    }
    let mut start = 0;
    for polite in ["please", "buntini", "hey", "ok", "okay", "can", "you", "could", "would", "just", "let's", "lets", "i", "want", "to", "i'd", "like"] {
        if words.get(start) == Some(&polite) {
            start += 1;
        }
    }
    let words: Vec<&str> = words[start..].iter().copied().filter(|word| *word != "please").collect();
    let line = words.join(" ");
    let timer = |rest: &str| ["timer", "session", "block", "sprint", "cycle", ""].contains(&rest);
    let article = |line: &str| -> String {
        let mut line = line;
        for word in ["the ", "my ", "a ", "this ", "our "] {
            line = line.strip_prefix(word).unwrap_or(line);
        }
        line.to_string()
    };
    for (verbs, command) in [
        (&["stop", "end", "cancel", "finish", "quit"][..], Command::Stop),
        (&["pause", "hold"][..], Command::Pause),
        (&["resume", "continue", "unpause"][..], Command::Resume),
    ] {
        for verb in verbs {
            if let Some(rest) = line.strip_prefix(verb).map(str::trim) {
                let rest = article(rest);
                let thing = rest.strip_prefix("focus").or_else(|| rest.strip_prefix("pomodoro")).map(str::trim);
                if thing.is_some_and(timer) || (*verb == "stop" || *verb == "end") && rest == "focusing" {
                    return Some(command);
                }
            }
        }
    }
    let minutes = duration_minutes(&line);
    let started = ["start", "begin", "set", "run", "do", "go"].iter().find_map(|verb| line.strip_prefix(&format!("{verb} ")).map(article));
    let verb = started.is_some();
    let subject = started.unwrap_or_else(|| line.clone());
    if subject.contains(" on ") || subject == "focus on" {
        return None;
    }
    let amount = |word: &str| word.chars().all(|c| c.is_ascii_digit()) || ["a", "an", "half", "one", "two", "three", "four", "five", "ten", "fifteen", "twenty", "thirty", "forty", "fifty", "sixty", "ninety"].contains(&word);
    let focused = if subject == "pomodoro" || subject.starts_with("pomodoro ") {
        true
    } else if let Some(rest) = subject.strip_prefix("focusing").map(str::trim) {
        // "start focusing", "focusing for an hour"
        (verb && rest.is_empty()) || rest.starts_with("for ")
    } else if let Some(rest) = subject.strip_prefix("focus").map(str::trim) {
        let mut parts = rest.split_whitespace();
        match (parts.next(), parts.next()) {
            // "start focus", but never a bare "focus"
            (None, _) => verb,
            // "focus for 25 minutes", "focus 25 minutes"
            (Some("for"), _) => true,
            (Some(first), _) if amount(first) => minutes.is_some(),
            // "focus timer", "focus session for 45 minutes"
            (Some(first), next) => timer(first) && next.is_none_or(|next| next == "for"),
        }
    } else {
        // "a 50 minute focus session", "25 minutes of focus"
        subject.split_whitespace().next().is_some_and(amount) && minutes.is_some() && subject.split_whitespace().any(|word| word == "focus")
    };
    if !focused {
        return None;
    }
    // Something other than a length after "for" ("focus for my exam") isn't a timer.
    if let Some((_, after)) = subject.split_once(" for ") {
        if minutes.is_none() || !after.split_whitespace().next().is_some_and(amount) {
            return None;
        }
    }
    Some(Command::Start { minutes })
}

/// "25 minutes", "an hour", "half an hour", "90 min", "a 50 minute focus session".
fn duration_minutes(text: &str) -> Option<u64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let number = |word: &str| -> Option<u64> {
        word.parse().ok().or(match word {
            "a" | "an" | "one" => Some(1),
            "two" => Some(2),
            "three" => Some(3),
            "four" => Some(4),
            "five" => Some(5),
            "ten" => Some(10),
            "fifteen" => Some(15),
            "twenty" => Some(20),
            "thirty" => Some(30),
            "forty" => Some(40),
            "fifty" => Some(50),
            "sixty" => Some(60),
            "ninety" => Some(90),
            _ => None,
        })
    };
    for (index, word) in words.iter().enumerate() {
        let unit = match word.trim_end_matches('s') {
            "minute" | "min" => 1,
            "hour" | "hr" => 60,
            _ => continue,
        };
        let before = index.checked_sub(1).map(|at| words[at]);
        if before == Some("an") && index >= 2 && words[index - 2] == "half" || before == Some("half") {
            return Some(unit / 2).filter(|minutes| *minutes > 0);
        }
        let amount = before.and_then(number)?;
        let minutes = amount.checked_mul(unit)?;
        return (1..=180).contains(&minutes).then_some(minutes);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn buntini_picks_rest_from_focus() {
        assert_eq!(Plan::default(), Plan { focus: 25 * MIN, rest: 5 * MIN, long_rest: 15 * MIN });
        assert_eq!(Plan::for_focus(50 * MIN), Plan { focus: 50 * MIN, rest: 10 * MIN, long_rest: 30 * MIN });
        assert_eq!(Plan::for_focus(10 * MIN).rest, 3 * MIN, "never a rest too short to stand up");
        assert_eq!(Plan::for_focus(120 * MIN).rest, 15 * MIN);
        assert_eq!(Plan::for_focus(0).focus, MIN);
    }

    #[test]
    fn a_whole_set_runs_focus_rest_four_times_then_a_long_rest() {
        let plan = Plan::default();
        let mut session = Session::start(plan, 0);
        let mut seen = vec![(session.phase, session.round)];
        let now = loop {
            let now = session.ends_at.unwrap();
            match session.catch_up(now) {
                Step::Turned => seen.push((session.phase, session.round)),
                Step::Over => break now,
                Step::Same => panic!("a deadline passed without a turn"),
            }
        };
        use Phase::*;
        assert_eq!(seen, [(Focus, 1), (Rest, 1), (Focus, 2), (Rest, 2), (Focus, 3), (Rest, 3), (Focus, 4), (LongRest, 4)]);
        assert_eq!(now, 4 * 25 * MIN + 3 * 5 * MIN + 15 * MIN);
    }

    #[test]
    fn deadlines_never_drift_however_late_the_clock_is_read() {
        let mut session = Session::start(Plan::default(), 1_000);
        // Read 40 seconds late: rest still ends 5 minutes after focus was due, not after now.
        assert_eq!(session.catch_up(1_000 + 25 * MIN + 40_000), Step::Turned);
        assert_eq!(session.ends_at, Some(1_000 + 30 * MIN));
        assert_eq!(session.remaining(1_000 + 25 * MIN + 40_000), 5 * MIN - 40_000);
    }

    #[test]
    fn asleep_through_several_phases_lands_in_the_right_one() {
        let mut session = Session::start(Plan::default(), 0);
        // 62 minutes later: focus 25, rest 5, focus 25 (ends 55), rest (ends 60), focus 3 since 60.
        assert_eq!(session.catch_up(62 * MIN), Step::Turned);
        assert_eq!((session.phase, session.round), (Phase::Focus, 3));
        assert_eq!(session.remaining(62 * MIN), 23 * MIN);
        // Asleep all night: the set is over.
        assert_eq!(session.catch_up(12 * 60 * MIN), Step::Over);
    }

    #[test]
    fn pausing_holds_what_was_left_and_resuming_sets_a_new_deadline() {
        let mut session = Session::start(Plan::default(), 0);
        session.pause(10 * MIN);
        assert!(session.paused());
        assert_eq!(session.remaining(99 * MIN), 15 * MIN, "time stands still while paused");
        assert_eq!(session.catch_up(99 * MIN), Step::Same);
        session.resume(100 * MIN);
        assert_eq!(session.ends_at, Some(115 * MIN));
        session.resume(101 * MIN);
        assert_eq!(session.ends_at, Some(115 * MIN), "resuming twice changes nothing");
    }

    #[test]
    fn skipping_moves_on_and_keeps_a_pause() {
        let mut session = Session::start(Plan::default(), 0);
        assert_eq!(session.skip(3 * MIN), Step::Turned);
        assert_eq!((session.phase, session.ends_at), (Phase::Rest, Some(8 * MIN)));
        session.pause(4 * MIN);
        assert_eq!(session.skip(5 * MIN), Step::Turned);
        assert_eq!((session.phase, session.round, session.paused(), session.remaining(9 * MIN)), (Phase::Focus, 2, true, 25 * MIN));
        let mut last = Session { phase: Phase::LongRest, round: 4, ..Session::start(Plan::default(), 0) };
        assert_eq!(last.skip(0), Step::Over);
    }

    #[test]
    fn the_pill_counts_whole_minutes_and_wakes_when_they_change() {
        assert_eq!(badge(25 * MIN), "25 min");
        assert_eq!(badge(24 * MIN + 1), "25 min");
        assert_eq!(badge(24 * MIN), "24 min");
        assert_eq!(badge(59_999), "<1 min");
        assert_eq!(next_badge_change(25 * MIN), MIN, "25 min becomes 24 min a minute later");
        assert_eq!(next_badge_change(24 * MIN + 500), 500);
        assert_eq!(next_badge_change(30_000), 30_000);
    }

    #[test]
    fn a_session_survives_being_saved() {
        let mut session = Session::start(Plan::for_focus(50 * MIN), 7);
        session.pause(MIN);
        let saved = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&saved).unwrap(), session);
    }

    fn ask(text: &str) -> Option<Command> {
        recognise(&crate::skills::words(text))
    }

    #[test]
    fn focus_requests_are_recognised() {
        assert_eq!(ask("Focus for 25 minutes"), Some(Command::Start { minutes: Some(25) }));
        assert_eq!(ask("focus for an hour"), Some(Command::Start { minutes: Some(60) }));
        assert_eq!(ask("Start a focus timer"), Some(Command::Start { minutes: None }));
        assert_eq!(ask("start a 50 minute focus session"), Some(Command::Start { minutes: Some(50) }));
        assert_eq!(ask("start focusing"), Some(Command::Start { minutes: None }));
        assert_eq!(ask("pomodoro"), Some(Command::Start { minutes: None }));
        assert_eq!(ask("please start a pomodoro"), Some(Command::Start { minutes: None }));
        assert_eq!(ask("can you start a focus session for 45 minutes?"), Some(Command::Start { minutes: Some(45) }));
        assert_eq!(ask("Stop the focus timer"), Some(Command::Stop));
        assert_eq!(ask("stop focusing"), Some(Command::Stop));
        assert_eq!(ask("pause focus"), Some(Command::Pause));
        assert_eq!(ask("resume the pomodoro"), Some(Command::Resume));
    }

    #[test]
    fn things_that_only_sound_like_focus_go_to_the_model() {
        for text in [
            "turn on focus mode",
            "help me focus on my essay",
            "focus on the budget first",
            "how do I focus better",
            "I can't focus",
            "focus for my exam",
            "don't start a focus timer",
            "what is a pomodoro",
            "set a timer for 10 minutes",
            "stop the timer",
            "focus",
        ] {
            assert_eq!(ask(text), None, "{text}");
        }
    }
}
