//! Small, conservative metadata helpers for user-authored text. These do not
//! grant actions or treat a quoted invitation as instructions to the app.
use serde_json::{json, Value};

fn word(text: &str, prefix: &str) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
}

/// Whether the user asks for authored text, rather than an assistant persona
/// introduction or an explanation about writing. Quoted examples are inert.
pub fn authored(goal: &str) -> bool {
    let plain = crate::completion::unquoted(goal);
    plain.split([';', '\n', '.', '!', '?']).any(|sentence| {
        let request = crate::completion::request_clause(sentence);
        if crate::completion::non_action(request)
            || [
                "introduce yourself",
                "about yourself",
                "your self-introduction",
                "your self introduction",
                "your introduction",
            ]
            .iter()
            .any(|phrase| request.contains(phrase))
        {
            return false;
        }
        if ["rewrite", "rephrase", "reword"]
            .iter()
            .any(|verb| word(request, verb))
        {
            return true;
        }
        ["draft", "write", "compose"]
            .iter()
            .any(|verb| word(request, verb))
            && request
                .split(|ch: char| !ch.is_alphanumeric())
                .any(|target| ["reply", "email", "announcement"].contains(&target))
    })
}

fn quoted(goal: &str) -> Option<Vec<&str>> {
    if goal.len() > 64 * 1024 {
        return None;
    }
    let chars = goal.char_indices().collect::<Vec<_>>();
    let mut active = None;
    let mut begin = 0;
    let mut escaped = false;
    let mut result = Vec::new();
    for (position, (index, ch)) in chars.iter().copied().enumerate() {
        let preceding = position
            .checked_sub(1)
            .and_then(|i| chars.get(i))
            .map(|(_, ch)| *ch);
        let following = chars.get(position + 1).map(|(_, ch)| *ch);
        if let Some(end) = active {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == end
                && !(matches!(ch, '\'' | '’')
                    && preceding.is_some_and(char::is_alphanumeric)
                    && following.is_some_and(char::is_alphanumeric))
            {
                result.push(&goal[begin..index]);
                active = None;
            }
        } else {
            let end = match ch {
                '"' => Some('"'),
                '“' => Some('”'),
                '‘' => Some('’'),
                '`' => Some('`'),
                '\'' if !preceding.is_some_and(char::is_alphanumeric) => Some('\''),
                _ => None,
            };
            if let Some(end) = end {
                active = Some(end);
                begin = index + ch.len_utf8();
            }
        }
    }
    active.is_none().then_some(result)
}

fn name(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()
        && text.chars().count() <= 100
        && text.chars().any(char::is_alphabetic)
        && text
            .chars()
            .all(|ch| ch.is_alphabetic() || [' ', '-', '\'', '’', '.', '‑'].contains(&ch)))
    .then_some(text)
}

/// Explicit reply-role metadata from one quoted invitation. The source remains
/// untrusted text; ambiguous or absent greetings/signers produce no metadata.
pub fn roles(goal: &str, known_user: &str) -> Option<Value> {
    if !authored(goal)
        || role_override(goal)
        || !crate::completion::unquoted(goal)
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|token| token == "reply")
    {
        return None;
    }
    let sources = quoted(goal)?;
    if sources.len() != 1 {
        return None;
    }
    let source = sources[0].trim();
    let mut recipients = Vec::new();
    for (index, _) in source.char_indices() {
        if index != 0 && !source[..index].ends_with(char::is_whitespace) {
            continue;
        }
        let rest = &source[index..];
        let prefix = if rest
            .get(..3)
            .is_some_and(|head| head.eq_ignore_ascii_case("hi "))
        {
            3
        } else if rest
            .get(..6)
            .is_some_and(|head| head.eq_ignore_ascii_case("hello "))
        {
            6
        } else {
            continue;
        };
        let end = rest[prefix..].find(',')? + prefix;
        if let Some(recipient) = name(&rest[prefix..end]) {
            recipients.push((index, recipient));
        }
    }
    if recipients.len() != 1 || recipients[0].0 != 0 {
        return None;
    }
    let mut signers = Vec::new();
    for (index, ch) in source.char_indices() {
        if !['-', '—', '–'].contains(&ch)
            || (index != 0 && !source[..index].ends_with(char::is_whitespace))
            || !source[index + ch.len_utf8()..].starts_with(' ')
        {
            continue;
        }
        let rest = source[index + ch.len_utf8()..].trim_start();
        if let Some(signer) = name(rest) {
            signers.push((index, signer));
        } else if let Some(first_line) = rest.lines().next().and_then(name) {
            // Multiple name-only signature/bullet lines are ambiguous too.
            signers.push((index, first_line));
        }
    }
    if signers.len() != 1 {
        return None;
    }
    let (index, sender) = signers[0];
    if source[index..].contains('\n')
        || name(source[index + source[index..].chars().next()?.len_utf8()..].trim()) != Some(sender)
    {
        return None;
    }
    let recipient = recipients[0].1;
    let reply_sender = if known_user.trim().is_empty() {
        recipient
    } else {
        name(known_user)?
    };
    if ambiguous_names(recipient, sender) || ambiguous_names(reply_sender, sender) {
        return None;
    }
    Some(json!({
        "sourceRecipient":recipient,
        "sourceSender":sender,
        "replyRecipient":sender,
        "replySender":reply_sender
    }))
}

fn role_override(goal: &str) -> bool {
    let Some(sources) = quoted(goal) else {
        return true;
    };
    let mut instruction = goal.to_string();
    for source in sources {
        // Mask exact quoted content first: the shared prose classifier is
        // intentionally simpler about apostrophes inside quoted names.
        if source.is_empty() || goal.match_indices(source).count() != 1 {
            return true;
        }
        instruction = instruction.replacen(source, " ", 1);
    }
    let plain = crate::completion::unquoted(&instruction);
    if plain.split(|ch: char| !ch.is_alphabetic()).any(|word| {
        [
            "sign",
            "signed",
            "signature",
            "recipient",
            "sender",
            "address",
        ]
        .contains(&word)
    }) {
        return true;
    }
    [
        "reply as",
        "respond as",
        "write as",
        "writing as",
        "reply from",
        "reply for",
        "on behalf of",
        "from the perspective",
        "in the voice of",
        "address it to",
        "address this",
        "address the",
        "as the recipient",
        "as recipient",
        "as the sender",
        "as sender",
        "recipient is",
        "sender is",
        "signed as",
        "sign as",
        "sign off as",
        "sign it as",
        "sign the reply",
        "signature",
        "use the greeting",
        "keep the greeting",
        "keep the sender",
        "keep the recipient",
        "do not change the greeting",
        "don't change the greeting",
        "start with",
        "begin with",
    ]
    .iter()
    .any(|phrase| plain.contains(phrase))
        || ["reply to ", "respond to "].iter().any(|marker| {
            plain.match_indices(marker).any(|(index, found)| {
                let tail = plain[index + found.len()..].trim_start();
                // Removing the quoted invitation leaves punctuation here;
                // that is a source boundary, not an explicit named override.
                if tail.is_empty() || tail.starts_with(['.', ';', '!', '?']) {
                    return false;
                }
                ![
                    "this", "that", "the", "a", "an", "my", "these", "pasted", "quoted",
                ]
                .iter()
                .any(|prefix| word(tail, prefix))
            })
        })
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

fn ambiguous_names(left: &str, right: &str) -> bool {
    let parts = |name: &str| {
        name.split_whitespace()
            .map(|part| part.trim_matches('.').to_lowercase())
            .collect::<Vec<_>>()
    };
    let left_parts = parts(left);
    let right_parts = parts(right);
    same_name(left, right)
        || left_parts.iter().all(|part| right_parts.contains(part))
        || right_parts.iter().all(|part| left_parts.contains(part))
        || left
            .split_whitespace()
            .next()
            .zip(right.split_whitespace().next())
            .is_some_and(|(left, right)| same_name(left, right))
}

fn contains_name(text: &str, name: &str) -> bool {
    let text = text.to_lowercase();
    let name = name.to_lowercase();
    text.match_indices(&name).any(|(index, matched)| {
        !text[..index]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
            && !text[index + matched.len()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
    })
}

fn signature_line(line: &str) -> &str {
    let mut text = line.trim();
    while let Some(rest) = text.strip_prefix('>') {
        text = rest.trim_start();
    }
    text.trim_matches(['*', '_', '`']).trim()
}

fn remove_signature(answer: &str, closing_name: &str) -> String {
    let lines = answer.lines().collect::<Vec<_>>();
    let Some(last) = lines
        .iter()
        .rposition(|line| !signature_line(line).is_empty())
    else {
        return answer.to_string();
    };
    let signature = signature_line(lines[last]);
    let signature = signature
        .strip_prefix("- ")
        .or_else(|| signature.strip_prefix("— "))
        .or_else(|| signature.strip_prefix("– "))
        .unwrap_or(signature);
    if !same_name(signature, closing_name) {
        return answer.to_string();
    }
    let mut start = last;
    while start > 0 && signature_line(lines[start - 1]).is_empty() {
        start -= 1;
    }
    if start > 0
        && [
            "best,",
            "best regards,",
            "kind regards,",
            "regards,",
            "sincerely,",
            "cheers,",
            "warm regards,",
            "thanks,",
            "thank you,",
        ]
        .contains(&signature_line(lines[start - 1]).to_lowercase().as_str())
    {
        start -= 1;
    }
    while start > 0 && signature_line(lines[start - 1]).is_empty() {
        start -= 1;
    }
    lines[..start].join("\n").trim_end().to_string()
}

fn greeting(line: &str) -> Option<(&str, std::ops::Range<usize>)> {
    let clean = signature_line(line);
    let prefix = if clean
        .get(..3)
        .is_some_and(|s| s.eq_ignore_ascii_case("hi "))
    {
        3
    } else if clean
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("hello "))
    {
        6
    } else {
        return None;
    };
    let comma = clean[prefix..].find(',')? + prefix;
    let raw = &clean[prefix..comma];
    let recipient = name(raw)?;
    let begin = line.find(clean)? + prefix + raw.len() - raw.trim_start().len();
    Some((recipient, begin..begin + recipient.len()))
}

/// Correct only explicit reply boundaries and unsupported closing signatures.
/// The drafted body's facts and all ambiguous/overridden identities stay intact.
pub fn normalize(goal: &str, answer: &str, assistant_name: &str, known_user: &str) -> String {
    if !authored(goal) {
        return answer.to_string();
    }
    let mut result = answer.to_string();
    if let Some(roles) = roles(goal, known_user) {
        let source_recipient = roles["sourceRecipient"].as_str().unwrap();
        let source_sender = roles["sourceSender"].as_str().unwrap();
        let reply_sender = roles["replySender"].as_str().unwrap();
        let mut boundaries = Vec::new();
        let mut offset = 0;
        let mut preface = true;
        let mut original_section = false;
        for line in result.split_inclusive('\n') {
            if let Some((recipient, range)) = greeting(line.trim_end_matches(['\r', '\n'])) {
                boundaries.push((
                    recipient.to_string(),
                    offset + range.start..offset + range.end,
                    preface,
                ));
            }
            let clean = signature_line(line).to_lowercase();
            original_section |= ["original:", "source:", "invitation:"].contains(&clean.as_str());
            preface &= clean.is_empty()
                || clean.starts_with("subject:")
                || (clean.ends_with(':')
                    && !original_section
                    && ["draft", "reply", "text for review"]
                        .iter()
                        .any(|marker| clean.contains(marker)));
            offset += line.len();
        }
        if boundaries.len() == 1 && boundaries[0].2 && !original_section {
            let (recipient, range, _) = &boundaries[0];
            if !same_name(recipient, source_sender)
                && (same_name(recipient, source_recipient) || same_name(recipient, reply_sender))
            {
                result.replace_range(range.clone(), source_sender);
            }
        }
        if boundaries.len() <= 1 && !original_section && !same_name(source_sender, reply_sender) {
            result = remove_signature(&result, source_sender);
        }
    }
    let assistant_name = assistant_name.trim();
    if !assistant_name.is_empty()
        && !same_name(assistant_name, known_user)
        && !contains_name(goal, assistant_name)
    {
        result = remove_signature(&result, assistant_name);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_intent_is_positive_and_quoted_examples_are_inert() {
        for goal in [
            "Rewrite this message politely: 'Hello'.",
            "Please rephrase my text.",
            "Draft a friendly reply to this invitation.",
            "Write an email introducing me.",
            "Compose an announcement for my group.",
        ] {
            assert!(authored(goal), "{goal}");
        }
        for goal in [
            "Who are you?",
            "Introduce yourself.",
            "Draft your self-introduction.",
            "Write an email about yourself.",
            "Explain how to rewrite an email.",
            "Do not rewrite this and draft a reply.",
            "What does 'Draft a reply' mean?",
            "For example, draft a reply.",
        ] {
            assert!(!authored(goal), "{goal}");
        }
    }

    #[test]
    fn explicit_invitation_roles_preserve_names_and_prefer_the_known_user() {
        let goal = "Draft a friendly reply to this invitation: 'Hi Rowan, can you join lunch on Tuesday? - Morgan'. Do not send it.";
        assert_eq!(
            roles(goal, ""),
            Some(
                json!({"sourceRecipient":"Rowan","sourceSender":"Morgan","replyRecipient":"Morgan","replySender":"Rowan"})
            )
        );
        assert_eq!(roles(goal, "Avery").unwrap()["replySender"], "Avery");
        let goal = "Draft a reply to ‘Hello D'Arcy,\nPlease join us at noon.\n- Zoë O’Neil’.";
        assert_eq!(roles(goal, "").unwrap()["replyRecipient"], "Zoë O’Neil");
        assert_eq!(roles(goal, "").unwrap()["replySender"], "D'Arcy");
    }

    #[test]
    fn missing_or_ambiguous_role_sources_do_not_get_guessed() {
        for goal in [
            "Draft a reply to 'Hello Rowan, lunch Tuesday?'.",
            "Draft a reply to 'Hi Rowan, lunch? - Morgan' and 'Hi Quinn, dinner? - Avery'.",
            "Draft a reply to 'Hi Rowan, lunch?\nHi Quinn, dinner?\n- Morgan'.",
            "Draft a reply to 'Hi Rowan, lunch?\n- Morgan\n- Avery'.",
            "Draft a reply to 'Hi Rowan, lunch? - Morgan",
            "Explain 'Draft a reply to Hi Rowan, lunch? - Morgan'.",
        ] {
            assert!(roles(goal, "").is_none(), "{goal}");
        }
    }

    #[test]
    fn unsupported_signatures_are_removed_without_body_replacements() {
        let goal = "Rewrite my message as a professional email: 'The delivery arrives Tuesday.'";
        let body = "Text for review:\n\n> Hello,\n> The delivery arrives Tuesday.\n> \n> Best regards,\n> \n> Orion";
        assert_eq!(
            normalize(goal, body, "Orion", ""),
            "Text for review:\n\n> Hello,\n> The delivery arrives Tuesday."
        );
        assert_eq!(
            normalize(goal, "Delivery Tuesday.\n\n- Orion", "Orion", ""),
            "Delivery Tuesday."
        );
        assert_eq!(
            normalize(
                goal,
                "Orion appears in this body.\nBest,\nOrion",
                "Orion",
                ""
            ),
            "Orion appears in this body."
        );
        assert_eq!(
            normalize(goal, "Delivery Tuesday.\nBest,\nRowan", "Orion", ""),
            "Delivery Tuesday.\nBest,\nRowan"
        );
    }

    #[test]
    fn legitimate_source_brand_user_and_assistant_names_are_preserved() {
        let answer = "Hello,\nDelivery Tuesday.\nBest,\nOrion";
        assert_eq!(
            normalize("Rewrite the Orion announcement.", answer, "Orion", ""),
            answer
        );
        assert_eq!(
            normalize("Rewrite my email.", answer, "Orion", "Orion"),
            answer
        );
        assert_eq!(
            normalize("Introduce yourself.", answer, "Orion", ""),
            answer
        );
        // A name embedded inside another word is not source attribution.
        assert_eq!(
            normalize("Rewrite this about Canada.", "Hello.\nAda", "Ada", ""),
            "Hello."
        );
    }

    #[test]
    fn reply_greeting_uses_explicit_source_sender_without_changing_body_or_correct_author() {
        let goal = "Draft a friendly reply to this invitation: 'Hi Rowan, can you join on March 9? - Morgan'.";
        let answer = "Text for review:\n\n> Hi Rowan,\n> I can join on March 9. Rowan and Morgan can check parking.\n> Best,\n> Rowan";
        assert_eq!(
            normalize(goal, answer, "Orion", ""),
            answer.replacen("Hi Rowan,", "Hi Morgan,", 1)
        );
        let correct = "Hello Morgan,\nI can join March 9.\nBest,\nRowan";
        assert_eq!(normalize(goal, correct, "Orion", ""), correct);
        let wrong_author = "Hi Rowan,\nI can join March 9.\nBest,\nMorgan";
        assert_eq!(
            normalize(goal, wrong_author, "Orion", ""),
            "Hi Morgan,\nI can join March 9."
        );
        let self_greeting = "Hi Avery,\nI can join March 9.\nBest,\nAvery";
        assert_eq!(
            normalize(goal, self_greeting, "Orion", "Avery"),
            self_greeting.replacen("Hi Avery,", "Hi Morgan,", 1)
        );
    }

    #[test]
    fn explicit_identity_overrides_and_ambiguous_names_disable_reply_corrections() {
        let source = "Draft a reply to this invitation: 'Hi Rowan, lunch March 9? - Morgan'.";
        let answer = "Hi Rowan,\nMarch 9 works.\nBest,\nMorgan";
        for extra in [
            " Address the reply to Rowan.",
            " Reply as Morgan.",
            " Use Rowan as the recipient.",
            " Keep the greeting unchanged.",
            " Sign off as Morgan.",
            " Sign the email as Morgan.",
            " Address Rowan directly.",
        ] {
            assert_eq!(
                normalize(&format!("{source}{extra}"), answer, "Orion", ""),
                answer,
                "{extra}"
            );
        }
        let direct = "Draft a reply to Rowan: 'Hi Rowan, lunch March 9? - Morgan'.";
        assert_eq!(normalize(direct, answer, "Orion", ""), answer);
        for goal in [
            "Draft a reply to this invitation: 'Hi Alex, lunch? - Alex'.",
            "Draft a reply to this invitation: 'Hi Alex Chen, lunch? - Alex Rivera'.",
            "Draft a reply to this invitation: 'Hi Dr. Alex, lunch? - Alex'.",
            "Draft a reply to these invitations: 'Hi Rowan, lunch? - Morgan' and 'Hi Quinn, dinner? - Avery'.",
        ] {
            assert!(roles(goal, "").is_none(), "{goal}");
            assert_eq!(normalize(goal, answer, "Orion", ""), answer);
        }
        assert!(roles(source, "Morgan").is_none());
        let two_greetings = "Hi Rowan,\nOriginal:\nHi Rowan,\nMarch 9 works.\nBest,\nMorgan";
        assert_eq!(normalize(source, two_greetings, "Orion", ""), two_greetings);
        let original = "Original:\nHi Rowan,\nLunch March 9?\n- Morgan";
        assert_eq!(normalize(source, original, "Orion", ""), original);
    }
}

/// A chat reply without what makes it read like a chatbot: closing offers ("Let me know
/// if…", "I hope this helps"), sign-offs ("Best, Buntini") and "That's all for now".
/// Writing the user asked for (a draft, a letter) is theirs and stays as written.
pub fn without_filler(goal: &str, answer: &str, assistant_name: &str) -> String {
    if authored(goal) {
        return answer.to_string();
    }
    const OPENERS: [&str; 14] = [
        "i hope this helps", "i hope that helps", "hope this helps", "hope that helps", "let me know if",
        "feel free to", "if you have any other questions", "if you have any more questions", "if you need anything else",
        "if you need any more", "is there anything else", "that's all for now", "happy to help", "i'm here to help you with anything",
    ];
    let name = assistant_name.trim().to_lowercase();
    let mut lines: Vec<String> = Vec::new();
    for line in answer.lines() {
        let plain = line.trim().trim_end_matches(['.', '!', ',']).to_lowercase().replace('’', "'");
        // A sign-off: "Best," / "Cheers!" / the assistant's own name on its own line.
        if ["best", "cheers", "warmly", "regards", "kind regards", "best wishes", "sincerely"].contains(&plain.as_str())
            || (!name.is_empty() && plain == name)
            || (!name.is_empty() && plain.ends_with(&format!(", {name}")) && plain.len() < name.len() + 16)
        {
            continue;
        }
        let kept: String = crate::completion::sentences(line)
            .into_iter()
            .filter(|sentence| {
                let start = sentence.trim().to_lowercase().replace('’', "'");
                !OPENERS.iter().any(|opener| start.starts_with(opener))
            })
            .collect();
        if kept.trim().is_empty() && !line.trim().is_empty() {
            continue;
        }
        lines.push(kept.trim_end().to_string());
    }
    let joined = lines.join("\n");
    let trimmed = joined.trim();
    if trimmed.chars().any(char::is_alphabetic) { trimmed.to_string() } else { answer.trim().to_string() }
}

#[cfg(test)]
mod filler_tests {
    use super::without_filler;

    #[test]
    fn closing_offers_and_sign_offs_go_but_the_answer_stays() {
        let reply = "Hey Sagar!\n\nTea and a book is a lovely start. I hope this helps!\n\nLet me know if you need anything else.\n\nBest,\nBuntini";
        assert_eq!(without_filler("cozy ideas?", reply, "Buntini"), "Hey Sagar!\n\nTea and a book is a lovely start.");
        assert_eq!(without_filler("hi", "That's all for now! Feel free to ask.", "Buntini"), "That's all for now! Feel free to ask.", "nothing else left: keep it");
        let draft = "Hi Ana,\n\nLet me know if Friday works.\n\nBest,\nSagar";
        assert_eq!(without_filler("draft an email to Ana asking if Friday works", draft, "Buntini"), draft);
    }
}
