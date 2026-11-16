//! "Hey Buntini": whether short stretches of speech hold the phrase, and when listening
//! for it is allowed at all. Pure. The helper only recognizes while someone is speaking.

/// What can be heard for "hey".
const GREETINGS: [&str; 7] = ["hey", "hi", "hay", "a", "okay", "ok", "hello"];
const NAME: &str = "buntini";

fn edits(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            current[j + 1] = (previous[j] + usize::from(ca != cb)).min(previous[j + 1] + 1).min(current[j] + 1);
        }
        previous = current;
    }
    previous[b.len()]
}

/// Spellings that sound the same as the name's "i": "ee", "ea", "ie" and a final "y".
fn sounds(word: &str) -> String {
    let mut out = word.replace("ee", "i").replace("ea", "i").replace("ie", "i");
    if out.ends_with('y') {
        out.pop();
        out.push('i');
    }
    out
}

/// Whether `text` holds "hey Buntini", as a recognizer may write it: "Hey Bunteeny",
/// "a bun tini", "hey, Buntini!". The name must follow a greeting, so talking *about*
/// Buntini doesn't wake it.
pub fn heard(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    for (index, word) in words.iter().enumerate() {
        if !GREETINGS.contains(&word.as_str()) {
            continue;
        }
        for take in 1..=3 {
            let Some(parts) = words.get(index + 1..index + 1 + take) else { break };
            let joined = sounds(&parts.concat());
            if joined.len().abs_diff(NAME.len()) <= 1 && edits(&joined, NAME) <= 2 {
                return true;
            }
        }
    }
    false
}

/// What decides whether the wake word may listen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conditions {
    pub enabled: bool,
    pub asleep: bool,
    pub screen_locked: bool,
    pub low_power: bool,
    /// A dictation is open.
    pub dictating: bool,
    /// A voice turn in the panel, or a call being recorded, has the microphone.
    pub microphone_busy: bool,
}

pub fn may_listen(conditions: Conditions) -> bool {
    conditions.enabled && !conditions.asleep && !conditions.screen_locked && !conditions.low_power && !conditions.dictating && !conditions.microphone_busy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phrase_is_heard_however_it_is_written() {
        for text in ["Hey Buntini", "hey, Buntini!", "Hey Bunteeny what's the time", "a bun tini", "Hi Buntiny.", "okay buntini open mail", "Hey bun tee knee"] {
            assert!(heard(text), "{text}");
        }
    }

    /// What Parakeet and Apple's recognizer wrote for "Hey Buntini…" said by four macOS voices
    /// (`say` into files, then the helper's transcribe check).
    #[test]
    fn what_the_recognizers_really_write_is_heard() {
        for text in [
            "Hey Buntany, what's on my calendar today?",
            "Hey Buntany.",
            "Hey Buntani, start a timer for 10 minutes.",
            "Hi Bontani.",
            "Hey Bontini, what's on my calendar today?",
            "Hey, Buntiny.",
            "Hey, Bantini, start a timer for 10 minutes.",
            "Hi, Bantani.",
        ] {
            assert!(heard(text), "{text}");
        }
        for text in ["Buntany is my cat.", "Montoney is my cat.", "Hey Martini, are you there?", "Hey, Martini, are you there?"] {
            assert!(!heard(text), "{text}");
        }
    }

    #[test]
    fn talk_about_buntini_or_near_misses_do_not_wake_it() {
        for text in ["Buntini is my cat", "I asked Buntini", "hey martini", "hey bikini", "hey bunny", "they wanted a tiny bun", ""] {
            assert!(!heard(text), "{text}");
        }
    }

    #[test]
    fn listening_stops_while_asleep_locked_on_low_power_or_dictating() {
        let on = Conditions { enabled: true, ..Conditions::default() };
        assert!(may_listen(on));
        assert!(!may_listen(Conditions::default()), "off unless turned on");
        assert!(!may_listen(Conditions { asleep: true, ..on }));
        assert!(!may_listen(Conditions { screen_locked: true, ..on }));
        assert!(!may_listen(Conditions { low_power: true, ..on }));
        assert!(!may_listen(Conditions { dictating: true, ..on }));
        assert!(!may_listen(Conditions { microphone_busy: true, ..on }));
    }
}
