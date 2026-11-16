//! What holding and tapping the dictation key means. Pure: the native key watcher reports
//! presses with their times, and this decides when to open, lock, finish or drop a dictation.
//!
//! - The microphone opens on the first press, so the first word isn't lost.
//! - Held for 0.30 s or more, it's push-to-talk: letting go finishes.
//! - A short tap, then a second press within 0.40 s, goes hands-free until the next press.
//! - A single short tap with no second press is dropped.
//! - Another key while the dictation key is down (a shortcut using it) or Esc drops it.
//! - Finishing keeps the microphone open 600 ms more, for the end of the last word.

/// Pressed this long, it's push-to-talk.
pub const HOLD_MS: u64 = 300;
/// A second press this soon after a short tap goes hands-free.
pub const DOUBLE_TAP_MS: u64 = 400;
/// The microphone stays open this long after letting go.
pub const TAIL_MS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// The dictation key (or the whole custom chord) went down.
    Down,
    /// It came up.
    Up,
    /// Any other key went down.
    Other,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Open the microphone and start a dictation.
    Start,
    /// Hands-free: keep listening until the next press.
    Lock,
    /// Close the microphone and use what was said.
    Finish,
    /// Close the microphone and drop what was said.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Idle,
    /// Down and listening; a hold or a tap depending on when it comes up.
    Pressed { down: u64 },
    /// A short tap: listening, waiting to see whether a second press follows.
    Tapped { up: u64 },
    /// Hands-free, with the press that locked it still down.
    LockPressed,
    /// Hands-free.
    Locked,
    /// Let go; the microphone closes at `until`. A press before then carries on.
    Tail { until: u64 },
    /// Dropped while the key was down; waits for it to come up.
    Ignoring,
}

#[derive(Debug, Default)]
pub struct Machine {
    state: State,
}

impl Machine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a dictation is open.
    pub fn listening(&self) -> bool {
        !matches!(self.state, State::Idle | State::Ignoring)
    }

    #[cfg(test)]
    fn hands_free(&self) -> bool {
        matches!(self.state, State::Locked | State::LockPressed)
    }

    /// Feeds one key event at `now` (milliseconds on any steady clock). A timer that should
    /// already have fired comes first in what's returned.
    pub fn event(&mut self, event: KeyEvent, now: u64) -> Vec<Command> {
        let late = self.tick(now);
        let (next, command) = match (self.state, event) {
            (State::Idle, KeyEvent::Down) => (State::Pressed { down: now }, Some(Command::Start)),
            (State::Idle, _) => (State::Idle, None),

            (State::Pressed { down }, KeyEvent::Up) if now.saturating_sub(down) >= HOLD_MS => {
                (State::Tail { until: now + TAIL_MS }, None)
            }
            (State::Pressed { .. }, KeyEvent::Up) => (State::Tapped { up: now }, None),
            (State::Pressed { .. }, KeyEvent::Other | KeyEvent::Escape) => (State::Ignoring, Some(Command::Cancel)),
            (State::Pressed { down }, KeyEvent::Down) => (State::Pressed { down }, None),

            (State::Tapped { .. }, KeyEvent::Down) => (State::LockPressed, Some(Command::Lock)),
            (State::Tapped { .. }, KeyEvent::Other | KeyEvent::Escape) => (State::Idle, Some(Command::Cancel)),
            (State::Tapped { up }, KeyEvent::Up) => (State::Tapped { up }, None),

            (State::LockPressed, KeyEvent::Up) => (State::Locked, None),
            (State::LockPressed | State::Locked, KeyEvent::Escape) => (State::Idle, Some(Command::Cancel)),
            (State::LockPressed, _) => (State::LockPressed, None),
            (State::Locked, KeyEvent::Down) => (State::Tail { until: now + TAIL_MS }, None),
            // Typing while hands-free is fine; only Esc or the key itself ends it.
            (State::Locked, _) => (State::Locked, None),

            // Pressed again before the microphone closed: the same dictation carries on, as a hold.
            (State::Tail { .. }, KeyEvent::Down) => (State::Pressed { down: now.saturating_sub(HOLD_MS) }, None),
            (State::Tail { .. }, KeyEvent::Escape) => (State::Idle, Some(Command::Cancel)),
            (State::Tail { until }, _) => (State::Tail { until }, None),

            (State::Ignoring, KeyEvent::Up) => (State::Idle, None),
            (State::Ignoring, _) => (State::Ignoring, None),
        };
        self.state = next;
        late.into_iter().chain(command).collect()
    }

    /// Moves time on. Call at `next_deadline`, or whenever convenient.
    pub fn tick(&mut self, now: u64) -> Option<Command> {
        match self.state {
            State::Tapped { up } if now >= up + DOUBLE_TAP_MS => {
                self.state = State::Idle;
                Some(Command::Cancel)
            }
            State::Tail { until } if now >= until => {
                self.state = State::Idle;
                Some(Command::Finish)
            }
            _ => None,
        }
    }

    /// When `tick` next has something to do.
    pub fn next_deadline(&self) -> Option<u64> {
        match self.state {
            State::Tapped { up } => Some(up + DOUBLE_TAP_MS),
            State::Tail { until } => Some(until),
            _ => None,
        }
    }

    /// Forgets everything, as when the watcher lost track of the keys (its tap was switched
    /// off, or the Mac slept). An open dictation is dropped.
    pub fn reset(&mut self) -> Option<Command> {
        let was = self.listening();
        self.state = State::Idle;
        was.then_some(Command::Cancel)
    }

    /// The dictation ended elsewhere (the pill's button, a time limit).
    pub fn ended(&mut self) {
        if !matches!(self.state, State::Pressed { .. } | State::LockPressed) {
            self.state = State::Idle;
        } else {
            self.state = State::Ignoring;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Command::*;
    use KeyEvent::*;

    /// Runs events at the given times, ticking at each deadline in between, and returns
    /// every command with its time.
    fn run(steps: &[(u64, KeyEvent)], until: u64) -> Vec<(u64, Command)> {
        let mut machine = Machine::new();
        let mut out = Vec::new();
        for &(at, event) in steps {
            while let Some(deadline) = machine.next_deadline().filter(|deadline| *deadline <= at) {
                if let Some(command) = machine.tick(deadline) {
                    out.push((deadline, command));
                }
            }
            out.extend(machine.event(event, at).into_iter().map(|command| (at, command)));
        }
        while let Some(deadline) = machine.next_deadline().filter(|deadline| *deadline <= until) {
            if let Some(command) = machine.tick(deadline) {
                out.push((deadline, command));
            }
        }
        out
    }

    #[test]
    fn the_microphone_opens_on_the_first_press() {
        let mut machine = Machine::new();
        assert_eq!(machine.event(Down, 1_000), [Start]);
        assert!(machine.listening());
    }

    #[test]
    fn holding_is_push_to_talk_and_the_microphone_stays_open_after_letting_go() {
        assert_eq!(run(&[(0, Down), (300, Up)], 5_000), [(0, Start), (900, Finish)]);
        assert_eq!(run(&[(0, Down), (4_000, Up)], 9_000), [(0, Start), (4_600, Finish)]);
    }

    #[test]
    fn just_under_the_hold_time_is_a_tap() {
        // A lone tap is dropped once the double-tap window has passed.
        assert_eq!(run(&[(0, Down), (299, Up)], 5_000), [(0, Start), (699, Cancel)]);
    }

    #[test]
    fn a_double_tap_goes_hands_free_until_the_next_press() {
        let steps = [(0, Down), (80, Up), (300, Down), (380, Up), (20_000, Down), (20_100, Up)];
        assert_eq!(run(&steps, 30_000), [(0, Start), (300, Lock), (20_600, Finish)]);
    }

    #[test]
    fn the_second_tap_must_come_within_the_window() {
        assert_eq!(run(&[(0, Down), (80, Up), (480, Down)], 600), [(0, Start), (480, Cancel), (480, Start)]);
        assert_eq!(run(&[(0, Down), (80, Up), (479, Down)], 600), [(0, Start), (479, Lock)]);
    }

    #[test]
    fn hands_free_ignores_typing_but_esc_drops_it() {
        let steps = [(0, Down), (50, Up), (100, Down), (150, Up), (1_000, Other), (2_000, Other), (3_000, Escape)];
        assert_eq!(run(&steps, 9_000), [(0, Start), (100, Lock), (3_000, Cancel)]);
    }

    #[test]
    fn another_key_while_held_is_a_shortcut_and_drops_the_dictation() {
        let steps = [(0, Down), (500, Other), (700, Up), (2_000, Down), (2_500, Up)];
        assert_eq!(run(&steps, 9_000), [(0, Start), (500, Cancel), (2_000, Start), (3_100, Finish)]);
    }

    #[test]
    fn esc_drops_a_hold_and_its_release_does_nothing() {
        assert_eq!(run(&[(0, Down), (1_000, Escape), (1_200, Up)], 9_000), [(0, Start), (1_000, Cancel)]);
    }

    #[test]
    fn esc_during_the_tail_drops_it() {
        assert_eq!(run(&[(0, Down), (1_000, Up), (1_300, Escape)], 9_000), [(0, Start), (1_000 + 300, Cancel)]);
    }

    #[test]
    fn pressing_again_during_the_tail_carries_on_the_same_dictation() {
        let steps = [(0, Down), (1_000, Up), (1_200, Down), (1_250, Up)];
        // The second press counts as part of the hold: letting go finishes after the tail.
        assert_eq!(run(&steps, 9_000), [(0, Start), (1_850, Finish)]);
    }

    #[test]
    fn esc_or_other_keys_with_nothing_open_do_nothing() {
        assert_eq!(run(&[(0, Escape), (10, Other), (20, Up)], 1_000), []);
    }

    #[test]
    fn a_lost_watcher_drops_an_open_dictation() {
        let mut machine = Machine::new();
        machine.event(Down, 0);
        assert_eq!(machine.reset(), Some(Cancel));
        assert_eq!(machine.reset(), None);
        assert_eq!(machine.event(Up, 10), []);
    }

    #[test]
    fn ending_elsewhere_waits_for_a_held_key_to_come_up() {
        let mut machine = Machine::new();
        machine.event(Down, 0);
        machine.ended();
        assert!(!machine.listening());
        assert_eq!(machine.event(Up, 900), []);
        assert_eq!(machine.event(Down, 1_000), [Start]);
        let mut locked = Machine::new();
        locked.event(Down, 0);
        locked.event(Up, 50);
        locked.event(Down, 100);
        locked.event(Up, 150);
        assert!(locked.hands_free());
        locked.ended();
        assert_eq!(locked.event(Down, 500), [Start]);
    }

    #[test]
    fn a_timer_that_should_have_fired_is_reported_with_the_next_event() {
        let mut machine = Machine::new();
        machine.event(Down, 0);
        machine.event(Up, 1_000);
        // Nobody ticked at 1 600; an unrelated key at 5 000 still finishes it.
        assert_eq!(machine.event(Other, 5_000), [Finish]);
        assert!(!machine.listening());
        // And a press long after a tail both finishes the old one and starts anew.
        machine.event(Down, 6_000);
        machine.event(Up, 7_000);
        assert_eq!(machine.event(Down, 9_000), [Finish, Start]);
    }
}
